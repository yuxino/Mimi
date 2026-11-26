//! Windows system-audio capture via WASAPI loopback (cpal): opening an input
//! stream on the default output device transparently enables loopback mode,
//! capturing the mix of everything playing. The mix format (typically 48 kHz
//! stereo f32) is resampled to the provider rate, mixed to mono, and
//! quantized to PCM16.

use super::windows_capture_worker::{Worker, WorkerSlot};
use crate::audio::send_pipeline::{AudioIngress, AudioIngressError};
use crate::audio::streaming_resampler::StreamingPcm16Resampler;
use crate::audio::{
    AudioCaptureFormat, CaptureFailureSender, SystemAudioCaptureError, SystemAudioCaptureFailure,
};
use crate::core::audio::audio_source::{
    parse_default_role, source_action, AudioActivity, DefaultRole, FollowAudible, FollowDecision,
    SourceAction,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub struct WindowsSystemAudioCapture {
    workers: Arc<WorkerSlot<WindowsWorkerState>>,
    source: Arc<Mutex<String>>,
}

#[derive(Default)]
struct WindowsWorkerState {
    active: Arc<AtomicBool>,
    current_device: Mutex<Option<String>>,
    activity: Arc<Mutex<AudioActivity>>,
}

type CaptureWorker = Worker<WindowsWorkerState>;

struct WindowsAudioProcessor {
    resampler: StreamingPcm16Resampler,
    normalized_samples: Vec<f32>,
    activity: Arc<Mutex<AudioActivity>>,
}

struct WindowsStreamContext {
    worker: Arc<CaptureWorker>,
    live: Arc<AtomicBool>,
    processor: Arc<Mutex<WindowsAudioProcessor>>,
    audio_ingress: AudioIngress,
    failure_tx: CaptureFailureSender,
}

/// The owner thread drops this before publishing `finished`, including on
/// cancelled build, failed play, device rebind, and normal stop. CPAL's WASAPI
/// destructor synchronously joins its callback thread; it must never run on a
/// Tokio worker or while holding the capture slot lock.
struct NativeStream {
    stream: cpal::Stream,
    live: Arc<AtomicBool>,
    worker: Arc<CaptureWorker>,
}

impl Drop for NativeStream {
    fn drop(&mut self) {
        self.live.store(false, Ordering::SeqCst);
        self.worker.data.active.store(false, Ordering::SeqCst);
        *self.worker.data.current_device.lock().unwrap() = None;
    }
}

impl WindowsSystemAudioCapture {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start(
        &self,
        ingress: AudioIngress,
        failure: CaptureFailureSender,
        format: AudioCaptureFormat,
    ) -> Result<(), SystemAudioCaptureError> {
        let source = self.source.lock().unwrap().clone();
        // Do not capture `self`/WorkerSlot in the thread: dropping the last
        // external capture handle must still cancel its owner.
        self.workers
            .start(move |worker, ready| {
                run_capture(worker, source, ingress, failure, format, ready);
            })
            .await
    }

    pub fn set_source(&self, source: String) {
        *self.source.lock().unwrap() = source;
    }

    pub fn snapshot(&self) -> Result<crate::audio::AudioSourceSnapshot, String> {
        let devices = cpal::default_host()
            .output_devices()
            .map_err(|_| "Sound outputs could not be loaded.".to_string())?
            .filter_map(|device| {
                Some(crate::audio::AudioSourceDevice {
                    id: device.id().ok()?.to_string(),
                    name: device.description().ok()?.name().to_string(),
                })
            })
            .collect();
        let worker = self.workers.current();
        let (current_device, receiving_audio_data, receiving_sound) = worker
            .filter(|worker| !worker.cancelled() && !worker.finished())
            .map(|worker| {
                let (data, sound) = worker.data.activity.lock().unwrap().state(Instant::now());
                let active = worker.data.active.load(Ordering::SeqCst);
                let device = worker.data.current_device.lock().unwrap().clone();
                (device, active && data, active && sound)
            })
            .unwrap_or((None, false, false));
        Ok(crate::audio::AudioSourceSnapshot {
            devices,
            current_device,
            receiving_sound,
            receiving_audio_data,
        })
    }

    pub async fn stop(&self) {
        self.workers.stop().await;
    }
}

fn open_stream(
    worker: &Arc<CaptureWorker>,
    device: &cpal::Device,
    ingress: AudioIngress,
    failure: CaptureFailureSender,
    format: AudioCaptureFormat,
) -> Result<NativeStream, SystemAudioCaptureError> {
    if worker.cancelled() {
        return Err(SystemAudioCaptureError::StartCancelled);
    }
    let id = device
        .id()
        .map_err(|_| SystemAudioCaptureError::NativeStartFailed)?
        .to_string();
    *worker.data.activity.lock().unwrap() = AudioActivity::default();
    // Errors during native construction are still current. Only destruction
    // disables this stream's error callbacks; `active` gates PCM after play.
    let live = Arc::new(AtomicBool::new(true));
    let stream = NativeStream {
        stream: build_stream_on_output_device(
            device,
            ingress,
            failure,
            format,
            Arc::clone(worker),
            Arc::clone(&live),
        )?,
        live,
        worker: Arc::clone(worker),
    };
    if worker.cancelled() {
        return Err(SystemAudioCaptureError::StartCancelled);
    }
    worker.data.active.store(true, Ordering::SeqCst);
    stream
        .stream
        .play()
        .map_err(|_| SystemAudioCaptureError::NativeStartFailed)?;
    *worker.data.current_device.lock().unwrap() = Some(id);
    Ok(stream)
}

fn run_capture(
    worker: Arc<CaptureWorker>,
    source: String,
    ingress: AudioIngress,
    failure: CaptureFailureSender,
    format: AudioCaptureFormat,
    ready: tokio::sync::oneshot::Sender<Result<(), SystemAudioCaptureError>>,
) {
    let follow = Mutex::new(FollowAudible::default());
    let first = resolve_capture_source(&source, &follow)
        .and_then(|device| open_stream(&worker, &device, ingress.clone(), failure.clone(), format));
    let mut stream = match first {
        Ok(stream) => stream,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    while !worker.cancelled() && !failure.has_reported() {
        // The old async monitor scanned once a second. Keep that cadence,
        // with short cancellation checks instead of a one-second stop delay.
        let next_scan = Instant::now() + Duration::from_secs(1);
        while Instant::now() < next_scan && !worker.cancelled() && !failure.has_reported() {
            std::thread::sleep(Duration::from_millis(5));
        }
        if worker.cancelled() || failure.has_reported() {
            break;
        }
        let device = resolve_capture_source(&source, &follow).ok();
        let id = device
            .as_ref()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string());
        let bound = worker.data.current_device.lock().unwrap().clone();
        match source_action(!worker.cancelled(), bound.as_deref(), id.as_deref()) {
            SourceAction::StopMonitor => break,
            SourceAction::Keep => {}
            SourceAction::Unavailable => {
                failure.report(SystemAudioCaptureFailure::NativeStopped);
                break;
            }
            SourceAction::Rebind => {
                // Serial ownership also prevents the old callback from
                // observing a new active flag before its stream has exited.
                drop(stream);
                if worker.cancelled() || failure.has_reported() {
                    return;
                }
                stream = match open_stream(
                    &worker,
                    &device.unwrap(),
                    ingress.clone(),
                    failure.clone(),
                    format,
                ) {
                    Ok(stream) => stream,
                    Err(_) => {
                        if !worker.cancelled() {
                            failure.report(SystemAudioCaptureFailure::NativeStopped);
                        }
                        return;
                    }
                };
            }
        }
    }
}

fn build_stream_on_output_device(
    output_device: &cpal::Device,
    audio_ingress: AudioIngress,
    failure_tx: CaptureFailureSender,
    format: AudioCaptureFormat,
    worker: Arc<CaptureWorker>,
    live: Arc<AtomicBool>,
) -> Result<cpal::Stream, SystemAudioCaptureError> {
    // CPAL enables WASAPI loopback when an input stream is built on an
    // output device. Its configuration must still come from that output
    // endpoint: `default_input_config()` rejects render devices before a
    // loopback stream can be created.
    let input_config = output_device
        .default_output_config()
        .map_err(|_| SystemAudioCaptureError::NativeStartFailed)?;
    let sample_format = input_config.sample_format();
    let channel_count = input_config.channels() as usize;
    // cpal 0.18: `SampleRate` is a `u32` type alias (not a tuple struct).
    let sample_rate = input_config.sample_rate();

    if !supports_sample_format(sample_format) {
        return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
    }

    let processor = Arc::new(Mutex::new(WindowsAudioProcessor {
        resampler: StreamingPcm16Resampler::new(sample_rate, format.sample_rate_hz, channel_count)?,
        normalized_samples: Vec::new(),
        activity: Arc::clone(&worker.data.activity),
    }));
    let stream_context = WindowsStreamContext {
        worker,
        live,
        processor,
        audio_ingress,
        failure_tx,
    };
    let stream_config: cpal::StreamConfig = input_config.into();

    match sample_format {
        cpal::SampleFormat::F32 => {
            build_f32_input_stream(output_device, stream_config, stream_context)
        }
        cpal::SampleFormat::I8 => {
            build_converting_input_stream::<i8>(output_device, stream_config, stream_context)
        }
        cpal::SampleFormat::I16 => {
            build_converting_input_stream::<i16>(output_device, stream_config, stream_context)
        }
        cpal::SampleFormat::I24 => {
            build_i24_input_stream(output_device, stream_config, stream_context)
        }
        cpal::SampleFormat::I32 => {
            build_converting_input_stream::<i32>(output_device, stream_config, stream_context)
        }
        cpal::SampleFormat::I64 => {
            build_converting_input_stream::<i64>(output_device, stream_config, stream_context)
        }
        cpal::SampleFormat::U8 => {
            build_converting_input_stream::<u8>(output_device, stream_config, stream_context)
        }
        cpal::SampleFormat::U16 => {
            build_converting_input_stream::<u16>(output_device, stream_config, stream_context)
        }
        cpal::SampleFormat::U24 => {
            build_converting_input_stream::<cpal::U24>(output_device, stream_config, stream_context)
        }
        cpal::SampleFormat::U32 => {
            build_converting_input_stream::<u32>(output_device, stream_config, stream_context)
        }
        cpal::SampleFormat::U64 => {
            build_converting_input_stream::<u64>(output_device, stream_config, stream_context)
        }
        cpal::SampleFormat::F64 => {
            build_converting_input_stream::<f64>(output_device, stream_config, stream_context)
        }
        _ => Err(SystemAudioCaptureError::UnsupportedAudioFormat),
    }
}

fn supports_sample_format(sample_format: cpal::SampleFormat) -> bool {
    matches!(
        sample_format,
        cpal::SampleFormat::I8
            | cpal::SampleFormat::I16
            | cpal::SampleFormat::I24
            | cpal::SampleFormat::I32
            | cpal::SampleFormat::I64
            | cpal::SampleFormat::U8
            | cpal::SampleFormat::U16
            | cpal::SampleFormat::U24
            | cpal::SampleFormat::U32
            | cpal::SampleFormat::U64
            | cpal::SampleFormat::F32
            | cpal::SampleFormat::F64
    )
}

fn build_f32_input_stream(
    output_device: &cpal::Device,
    stream_config: cpal::StreamConfig,
    context: WindowsStreamContext,
) -> Result<cpal::Stream, SystemAudioCaptureError> {
    let WindowsStreamContext {
        worker,
        live,
        processor,
        audio_ingress,
        failure_tx,
    } = context;
    let failure_tx_for_error = failure_tx.clone();
    let worker_for_error = Arc::clone(&worker);
    output_device
        .build_input_stream(
            stream_config,
            move |data: &[f32], _info| {
                if worker.cancelled() || !worker.data.active.load(Ordering::SeqCst) {
                    return;
                }
                process_frames_f32(data, &processor, &audio_ingress, &failure_tx);
            },
            move |_error| {
                if !worker_for_error.cancelled() && live.load(Ordering::SeqCst) {
                    failure_tx_for_error.report(SystemAudioCaptureFailure::NativeStopped);
                }
            },
            None,
        )
        .map_err(|_| SystemAudioCaptureError::NativeStartFailed)
}

fn build_converting_input_stream<T>(
    output_device: &cpal::Device,
    stream_config: cpal::StreamConfig,
    context: WindowsStreamContext,
) -> Result<cpal::Stream, SystemAudioCaptureError>
where
    T: cpal::SizedSample,
    f32: FromSample<T>,
{
    let WindowsStreamContext {
        worker,
        live,
        processor,
        audio_ingress,
        failure_tx,
    } = context;
    let failure_tx_for_error = failure_tx.clone();
    let worker_for_error = Arc::clone(&worker);
    output_device
        .build_input_stream(
            stream_config,
            move |data: &[T], _info| {
                if worker.cancelled() || !worker.data.active.load(Ordering::SeqCst) {
                    return;
                }
                process_frames(data, &processor, &audio_ingress, &failure_tx);
            },
            move |_error| {
                if !worker_for_error.cancelled() && live.load(Ordering::SeqCst) {
                    failure_tx_for_error.report(SystemAudioCaptureFailure::NativeStopped);
                }
            },
            None,
        )
        .map_err(|_| SystemAudioCaptureError::NativeStartFailed)
}

fn build_i24_input_stream(
    output_device: &cpal::Device,
    stream_config: cpal::StreamConfig,
    context: WindowsStreamContext,
) -> Result<cpal::Stream, SystemAudioCaptureError> {
    let WindowsStreamContext {
        worker,
        live,
        processor,
        audio_ingress,
        failure_tx,
    } = context;
    let failure_tx_for_error = failure_tx.clone();
    let worker_for_error = Arc::clone(&worker);
    output_device
        .build_input_stream(
            stream_config,
            move |data: &[cpal::I24], _info| {
                if worker.cancelled() || !worker.data.active.load(Ordering::SeqCst) {
                    return;
                }
                process_frames_i24(data, &processor, &audio_ingress, &failure_tx);
            },
            move |_error| {
                if !worker_for_error.cancelled() && live.load(Ordering::SeqCst) {
                    failure_tx_for_error.report(SystemAudioCaptureFailure::NativeStopped);
                }
            },
            None,
        )
        .map_err(|_| SystemAudioCaptureError::NativeStartFailed)
}

/// Buffers interleaved frames across callbacks, resamples complete windows,
/// and emits mono PCM16.
fn process_frames_f32(
    data: &[f32],
    processor: &Arc<Mutex<WindowsAudioProcessor>>,
    audio_ingress: &AudioIngress,
    failure_tx: &CaptureFailureSender,
) {
    if failure_tx.has_reported() {
        return;
    }
    let Ok(mut processor) = processor.lock() else {
        failure_tx.report(SystemAudioCaptureFailure::AudioProcessingFailed);
        return;
    };
    processor
        .activity
        .lock()
        .unwrap()
        .observe(data, Instant::now());
    let Ok(buffers) = processor.resampler.push_interleaved(data) else {
        failure_tx.report(SystemAudioCaptureFailure::AudioProcessingFailed);
        return;
    };
    drop(processor);
    emit_buffers(buffers, audio_ingress, failure_tx);
}

fn process_frames<T>(
    data: &[T],
    processor: &Arc<Mutex<WindowsAudioProcessor>>,
    audio_ingress: &AudioIngress,
    failure_tx: &CaptureFailureSender,
) where
    T: Sample,
    f32: FromSample<T>,
{
    process_converted_frames(
        data,
        processor,
        audio_ingress,
        failure_tx,
        normalize_samples::<T>,
    );
}

fn process_frames_i24(
    data: &[cpal::I24],
    processor: &Arc<Mutex<WindowsAudioProcessor>>,
    audio_ingress: &AudioIngress,
    failure_tx: &CaptureFailureSender,
) {
    process_converted_frames(
        data,
        processor,
        audio_ingress,
        failure_tx,
        normalize_wasapi_i24_samples,
    );
}

fn process_converted_frames<T>(
    data: &[T],
    processor: &Arc<Mutex<WindowsAudioProcessor>>,
    audio_ingress: &AudioIngress,
    failure_tx: &CaptureFailureSender,
    normalize: impl FnOnce(&[T], &mut Vec<f32>),
) {
    if failure_tx.has_reported() {
        return;
    }
    let Ok(mut processor) = processor.lock() else {
        failure_tx.report(SystemAudioCaptureFailure::AudioProcessingFailed);
        return;
    };
    let WindowsAudioProcessor {
        resampler,
        normalized_samples,
        activity,
    } = &mut *processor;
    normalize(data, normalized_samples);
    activity
        .lock()
        .unwrap()
        .observe(normalized_samples, Instant::now());
    let buffers = resampler.push_interleaved(normalized_samples);
    // Keep the reusable allocation, but do not retain a logical copy of the
    // most recent system-audio callback after it enters the PCM pipeline.
    normalized_samples.clear();
    let Ok(buffers) = buffers else {
        failure_tx.report(SystemAudioCaptureFailure::AudioProcessingFailed);
        return;
    };
    drop(processor);
    emit_buffers(buffers, audio_ingress, failure_tx);
}

fn normalize_samples<T>(data: &[T], output: &mut Vec<f32>)
where
    T: Sample,
    f32: FromSample<T>,
{
    output.clear();
    output.extend(data.iter().copied().map(f32::from_sample));
}

/// CPAL exposes WASAPI's 24-valid-bit PCM as `I24`, but the native shared-mode
/// buffer uses a 32-bit `WAVEFORMATEXTENSIBLE` container. Windows left-aligns
/// valid PCM bits in that container, so recover the signed 24-bit value before
/// applying CPAL's normalized-sample conversion contract.
fn normalize_wasapi_i24_samples(data: &[cpal::I24], output: &mut Vec<f32>) {
    const CONTAINER_PADDING_BITS: u32 = i32::BITS - 24;
    const I24_SCALE: f32 = 8_388_608.0;

    output.clear();
    output.extend(
        data.iter()
            .map(|sample| (sample.inner() >> CONTAINER_PADDING_BITS) as f32 / I24_SCALE),
    );
}

fn emit_buffers(
    buffers: Vec<Vec<u8>>,
    audio_ingress: &AudioIngress,
    failure_tx: &CaptureFailureSender,
) {
    for pcm in buffers {
        match audio_ingress.try_send(pcm) {
            Ok(()) => {}
            Err(AudioIngressError::Backpressure) => {
                failure_tx.report(SystemAudioCaptureFailure::Backpressure);
                return;
            }
            Err(AudioIngressError::Closed) => return,
        }
    }
}

fn resolve_output(source: &str) -> Result<cpal::Device, SystemAudioCaptureError> {
    let host = cpal::default_host();
    if let Some(role) = parse_default_role(source) {
        // CPAL always asks Windows for the *console* default endpoint, but a
        // call headset is usually the *communications* default while media
        // keeps playing to the speakers. Resolve the requested role ourselves
        // and fall back to the console default when it cannot be resolved.
        // Match the role endpoint against CPAL's devices by the *backend* id
        // string (`IMMDevice::GetId`), which is the same value on both sides;
        // the `Display` form adds a host prefix and would never compare equal.
        let matched = default_endpoint_id(role).and_then(|wanted| {
            host.output_devices()
                .ok()?
                .find(|device| device.id().is_ok_and(|id| id.id() == wanted))
        });
        return matched
            .or_else(|| host.default_output_device())
            .ok_or(SystemAudioCaptureError::SelectedPlaybackDeviceUnavailable);
    }
    // Resolve to a specific render endpoint for persisted ids. CPAL's
    // default-device handle is dynamic and emits DeviceChanged errors itself;
    // our generation-scoped monitor owns rebinding instead. Enumerating only
    // render endpoints also rejects persisted microphone IDs.
    host.output_devices()
        .map_err(|_| SystemAudioCaptureError::SelectedPlaybackDeviceUnavailable)?
        .find(|device| device.id().is_ok_and(|id| id.to_string() == source))
        .ok_or(SystemAudioCaptureError::SelectedPlaybackDeviceUnavailable)
}

/// The sentinel for "capture whichever render endpoint is currently audible".
pub const FOLLOW_AUDIBLE: &str = "follow:audible";

/// The normal default and legacy audible sentinel share the same policy.
/// Role ids and census ids are raw WASAPI ids, not CPAL's display/persisted ids.
fn resolve_capture_source(
    source: &str,
    follow: &Mutex<FollowAudible>,
) -> Result<cpal::Device, SystemAudioCaptureError> {
    if !source.is_empty() && source != FOLLOW_AUDIBLE {
        return resolve_output(source);
    }
    smart_default_device(follow)
}

/// Follow audible role-ranked outputs, falling back through the role chain
/// while quiet. Census ids must resolve as raw WASAPI ids, not persisted ids.
fn smart_default_device(
    follow: &Mutex<FollowAudible>,
) -> Result<cpal::Device, SystemAudioCaptureError> {
    let roles: Vec<String> = [
        DefaultRole::Communications,
        DefaultRole::Multimedia,
        DefaultRole::Console,
    ]
    .into_iter()
    .filter_map(default_endpoint_id)
    .collect();
    let candidates = crate::audio::census::endpoint_levels();
    let chosen = {
        let mut state = follow.lock().unwrap();
        match state.decide_with_roles(&candidates, &roles) {
            FollowDecision::SwitchTo(id) => Some(id),
            FollowDecision::Keep => state.bound().map(str::to_string),
            FollowDecision::NothingAudible => None,
        }
    };
    chosen
        .as_deref()
        .and_then(resolve_backend_endpoint)
        .or_else(|| roles.iter().find_map(|id| resolve_backend_endpoint(id)))
        .or_else(|| cpal::default_host().output_devices().ok()?.next())
        .ok_or(SystemAudioCaptureError::NoPlaybackDevice)
}

fn resolve_backend_endpoint(wanted: &str) -> Option<cpal::Device> {
    cpal::default_host()
        .output_devices()
        .ok()?
        .find(|device| device.id().is_ok_and(|id| id.id() == wanted))
}

/// IMMDevice::GetId owns a COM allocation; copy then release it on every scan.
pub(crate) unsafe fn com_endpoint_id(
    device: &windows::Win32::Media::Audio::IMMDevice,
) -> Option<String> {
    let id = device.GetId().ok()?;
    let value = id.to_string().ok();
    windows::Win32::System::Com::CoTaskMemFree(Some(id.0.cast()));
    value
}

/// Windows' default render endpoint for the requested role, by endpoint id.
fn default_endpoint_id(role: DefaultRole) -> Option<String> {
    with_com(|| unsafe {
        use windows::Win32::Media::Audio::{
            eCommunications, eConsole, eMultimedia, eRender, IMMDeviceEnumerator,
            MMDeviceEnumerator,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
        let role = match role {
            DefaultRole::Console => eConsole,
            DefaultRole::Multimedia => eMultimedia,
            DefaultRole::Communications => eCommunications,
        };
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, role).ok()?;
        com_endpoint_id(&device)
    })
}

/// Own exactly the COM initialization reference acquired on this thread.
/// S_FALSE also increments the count; RPC_E_CHANGED_MODE does not.
struct ComApartment {
    result: windows::core::HRESULT,
    _thread: std::marker::PhantomData<*mut ()>,
}

impl ComApartment {
    fn new(mode: windows::Win32::System::Com::COINIT) -> Self {
        Self {
            result: unsafe { windows::Win32::System::Com::CoInitializeEx(None, mode) },
            _thread: std::marker::PhantomData,
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.result.is_ok() {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

/// Capture owners and presentation queries initialize COM once per thread.
/// Release this reference when that thread exits, after its native locals.
/// An existing incompatible apartment remains owned by its original caller.
pub(crate) fn with_com<R>(run: impl FnOnce() -> R) -> R {
    use windows::Win32::System::Com::COINIT_MULTITHREADED;
    thread_local! {
        static COM_APARTMENT: ComApartment = ComApartment::new(COINIT_MULTITHREADED);
    }
    COM_APARTMENT.with(|_| run())
}

#[cfg(test)]
mod tests;
