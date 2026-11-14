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
use crate::core::audio_source::{
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
    let selected = if source.is_empty() {
        host.default_output_device()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string())
            .ok_or(SystemAudioCaptureError::NoPlaybackDevice)?
    } else if let Some(role) = parse_default_role(source) {
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
    } else {
        source.to_string()
    };
    // Resolve to a specific render endpoint for persisted ids. CPAL's
    // default-device handle is dynamic and emits DeviceChanged errors itself;
    // our generation-scoped monitor owns rebinding instead. Enumerating only
    // render endpoints also rejects persisted microphone IDs.
    host.output_devices()
        .map_err(|_| SystemAudioCaptureError::SelectedPlaybackDeviceUnavailable)?
        .find(|device| device.id().is_ok_and(|id| id.to_string() == selected))
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
mod tests {
    use super::*;
    use crate::audio::send_pipeline::AudioSendPipeline;

    fn current_com_apartment() -> windows::core::Result<(
        windows::Win32::System::Com::APTTYPE,
        windows::Win32::System::Com::APTTYPEQUALIFIER,
    )> {
        use windows::Win32::System::Com::{
            CoGetApartmentType, APTTYPEQUALIFIER_NONE, APTTYPE_CURRENT,
        };
        let mut apartment = APTTYPE_CURRENT;
        let mut qualifier = APTTYPEQUALIFIER_NONE;
        unsafe { CoGetApartmentType(&mut apartment, &mut qualifier) }?;
        Ok((apartment, qualifier))
    }

    fn assert_no_explicit_com_apartment() {
        use windows::Win32::System::Com::{APTTYPEQUALIFIER_IMPLICIT_MTA, APTTYPE_MTA};
        match current_com_apartment() {
            // Other parallel tests may keep the process MTA alive. That does
            // not mean this fresh thread owns an initialization reference.
            Ok(state) => assert_eq!(state, (APTTYPE_MTA, APTTYPEQUALIFIER_IMPLICIT_MTA)),
            Err(error) => assert_eq!(
                error.code(),
                windows::Win32::Foundation::CO_E_NOTINITIALIZED
            ),
        }
    }

    #[test]
    fn nested_com_initialization_releases_each_successful_reference() {
        std::thread::spawn(|| {
            use windows::Win32::System::Com::{
                APTTYPEQUALIFIER_NONE, APTTYPE_MTA, COINIT_MULTITHREADED,
            };
            assert_no_explicit_com_apartment();
            let first = ComApartment::new(COINIT_MULTITHREADED);
            assert!(first.result.is_ok());
            let nested = ComApartment::new(COINIT_MULTITHREADED);
            assert_eq!(nested.result, windows::Win32::Foundation::S_FALSE);
            drop(nested);
            assert_eq!(
                current_com_apartment().unwrap(),
                (APTTYPE_MTA, APTTYPEQUALIFIER_NONE)
            );
            drop(first);
            assert_no_explicit_com_apartment();
        })
        .join()
        .unwrap();
    }

    #[test]
    fn failed_mta_initialization_does_not_release_existing_sta() {
        std::thread::spawn(|| {
            use windows::Win32::System::Com::{
                APTTYPE_MAINSTA, APTTYPE_STA, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
            };
            assert_no_explicit_com_apartment();
            let sta = ComApartment::new(COINIT_APARTMENTTHREADED);
            assert!(sta.result.is_ok());
            let original = current_com_apartment().unwrap();
            assert!(matches!(original.0, APTTYPE_STA | APTTYPE_MAINSTA));
            let incompatible = ComApartment::new(COINIT_MULTITHREADED);
            assert_eq!(
                incompatible.result,
                windows::Win32::Foundation::RPC_E_CHANGED_MODE
            );
            drop(incompatible);
            assert_eq!(current_com_apartment().unwrap(), original);
            drop(sta);
            assert_no_explicit_com_apartment();
        })
        .join()
        .unwrap();
    }

    /// The backend (WASAPI) id string, comparable with `IMMDevice::GetId`.
    fn endpoint_id(device: &cpal::Device) -> Option<String> {
        device.id().ok().map(|id| id.id().to_string())
    }

    #[test]
    fn role_sources_resolve_to_a_render_endpoint_or_report_unavailable() {
        // A real machine resolves each role; a headless CI machine may have no
        // render endpoints at all, which must surface as an error, not a panic.
        for source in ["role:communications", "role:multimedia", "role:console"] {
            match resolve_output(source) {
                Ok(device) => assert!(endpoint_id(&device).is_some()),
                Err(SystemAudioCaptureError::NoPlaybackDevice)
                | Err(SystemAudioCaptureError::SelectedPlaybackDeviceUnavailable) => {}
                Err(other) => panic!("unexpected error for {source}: {other:?}"),
            }
        }
    }

    #[test]
    fn audible_source_resolves_or_falls_back_without_panicking() {
        let follow = Mutex::new(FollowAudible::default());
        for source in ["", FOLLOW_AUDIBLE] {
            match resolve_capture_source(source, &follow) {
                Ok(device) => assert!(endpoint_id(&device).is_some()),
                Err(SystemAudioCaptureError::NoPlaybackDevice)
                | Err(SystemAudioCaptureError::SelectedPlaybackDeviceUnavailable) => {}
                Err(other) => panic!("unexpected error: {other:?}"),
            }
        }
    }

    #[test]
    fn raw_census_ids_resolve_the_exact_cpal_render_endpoint() {
        for (id, _) in crate::audio::census::endpoint_levels() {
            let resolved = resolve_backend_endpoint(&id).expect("raw WASAPI id must resolve");
            assert_eq!(endpoint_id(&resolved), Some(id));
        }
    }

    #[test]
    fn console_role_matches_the_cpal_default_device() {
        // The role resolver must agree with CPAL for the role CPAL itself asks
        // for, otherwise "follow system" and "follow console" would capture
        // different endpoints on the same machine.
        let (Ok(role), Ok(cpal_default)) = (resolve_output("role:console"), resolve_output(""))
        else {
            return; // headless machine without render endpoints
        };
        assert_eq!(endpoint_id(&role), endpoint_id(&cpal_default));
    }
    use std::mem::size_of;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc::error::TryRecvError;

    const TEST_TONE_FREQUENCY_HZ: f32 = 997.0;
    const TEST_TONE_AMPLITUDE: f32 = 0.2;
    const LOOPBACK_SAMPLE_RATE_HZ: f64 = 16_000.0;
    const TEST_TONE_WINDOW_SAMPLES: usize = 4_096;
    const TEST_TONE_MIN_RMS: f64 = 0.01;
    const TEST_TONE_MAX_RMS: f64 = 0.6;
    const TEST_TONE_MAX_PEAK: f64 = 0.95;
    const TEST_TONE_MAX_SATURATION_RATIO: f64 = 0.01;
    const TEST_TONE_MIN_ENERGY_RATIO: f64 = 0.5;

    #[derive(Default)]
    struct TestToneDetector {
        samples_in_window: usize,
        sine_projection: f64,
        cosine_projection: f64,
        square_sum: f64,
        peak: f64,
        saturated_samples: usize,
    }

    impl TestToneDetector {
        fn push_pcm16_le(&mut self, pcm: &[u8]) -> bool {
            for sample_bytes in pcm.as_chunks::<2>().0 {
                let sample = f64::from(i16::from_le_bytes(*sample_bytes)) / 32_768.0;
                let phase = std::f64::consts::TAU
                    * f64::from(TEST_TONE_FREQUENCY_HZ)
                    * self.samples_in_window as f64
                    / LOOPBACK_SAMPLE_RATE_HZ;
                let (phase_sine, phase_cosine) = phase.sin_cos();
                self.sine_projection += sample * phase_sine;
                self.cosine_projection += sample * phase_cosine;
                self.square_sum += sample * sample;
                self.peak = self.peak.max(sample.abs());
                if sample.abs() >= 0.999 {
                    self.saturated_samples += 1;
                }
                self.samples_in_window += 1;

                if self.samples_in_window == TEST_TONE_WINDOW_SAMPLES {
                    let detected = self.window_matches_test_tone();
                    self.reset();
                    if detected {
                        return true;
                    }
                }
            }
            false
        }

        fn window_matches_test_tone(&self) -> bool {
            let sample_count = self.samples_in_window as f64;
            let mean_square = self.square_sum / sample_count;
            let rms = mean_square.sqrt();
            let saturation_ratio = self.saturated_samples as f64 / sample_count;
            if !(TEST_TONE_MIN_RMS..=TEST_TONE_MAX_RMS).contains(&rms)
                || self.peak > TEST_TONE_MAX_PEAK
                || saturation_ratio > TEST_TONE_MAX_SATURATION_RATIO
            {
                return false;
            }

            let projected_mean_square = 2.0
                * (self.sine_projection * self.sine_projection
                    + self.cosine_projection * self.cosine_projection)
                / (sample_count * sample_count);
            projected_mean_square / mean_square >= TEST_TONE_MIN_ENERGY_RATIO
        }

        fn reset(&mut self) {
            self.samples_in_window = 0;
            self.sine_projection = 0.0;
            self.cosine_projection = 0.0;
            self.square_sum = 0.0;
            self.peak = 0.0;
            self.saturated_samples = 0;
        }
    }

    fn sine_pcm16(frequency_hz: f64, amplitude: f64) -> Vec<u8> {
        let mut pcm = Vec::with_capacity(TEST_TONE_WINDOW_SAMPLES * size_of::<i16>());
        for sample_index in 0..TEST_TONE_WINDOW_SAMPLES {
            let phase = std::f64::consts::TAU * frequency_hz * sample_index as f64
                / LOOPBACK_SAMPLE_RATE_HZ;
            let sample = (phase.sin() * amplitude * f64::from(i16::MAX)).round() as i16;
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        pcm
    }

    fn clipped_square_pcm16(frequency_hz: f64) -> Vec<u8> {
        let mut pcm = Vec::with_capacity(TEST_TONE_WINDOW_SAMPLES * size_of::<i16>());
        for sample_index in 0..TEST_TONE_WINDOW_SAMPLES {
            let phase = std::f64::consts::TAU * frequency_hz * sample_index as f64
                / LOOPBACK_SAMPLE_RATE_HZ;
            let sample = if phase.sin().is_sign_negative() {
                i16::MIN
            } else {
                i16::MAX
            };
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        pcm
    }

    fn build_test_tone_stream<T>(
        output_device: &cpal::Device,
        stream_config: cpal::StreamConfig,
        output_failed: Arc<AtomicBool>,
        mut encode_sample: impl FnMut(f32) -> T + Send + 'static,
    ) -> Result<cpal::Stream, cpal::Error>
    where
        T: cpal::SizedSample + Copy,
    {
        let channel_count = stream_config.channels as usize;
        let phase_step = TEST_TONE_FREQUENCY_HZ / stream_config.sample_rate as f32;
        let mut phase = 0.0_f32;

        output_device.build_output_stream(
            stream_config,
            move |data: &mut [T], _info| {
                for frame in data.chunks_mut(channel_count) {
                    let value = (std::f32::consts::TAU * phase).sin() * TEST_TONE_AMPLITUDE;
                    phase = (phase + phase_step).fract();
                    let sample = encode_sample(value);
                    frame.fill(sample);
                }
            },
            move |_error| {
                output_failed.store(true, Ordering::SeqCst);
            },
            None,
        )
    }

    fn build_output_test_tone(
        output_device: &cpal::Device,
        output_failed: Arc<AtomicBool>,
    ) -> Result<cpal::Stream, &'static str> {
        let output_config = output_device
            .default_output_config()
            .map_err(|_| "default playback format unavailable for the native tone")?;
        let sample_format = output_config.sample_format();
        let stream_config = output_config.into();

        macro_rules! build_converting_tone {
            ($sample_type:ty) => {
                build_test_tone_stream::<$sample_type>(
                    output_device,
                    stream_config,
                    output_failed,
                    <$sample_type>::from_sample,
                )
            };
        }

        let stream = match sample_format {
            cpal::SampleFormat::I8 => build_converting_tone!(i8),
            cpal::SampleFormat::I16 => build_converting_tone!(i16),
            cpal::SampleFormat::I24 => build_test_tone_stream::<cpal::I24>(
                output_device,
                stream_config,
                output_failed,
                |value| {
                    // Match the native 24-valid-in-32 representation expected
                    // by the vendored WASAPI output backend.
                    let logical = cpal::I24::from_sample(value).inner();
                    cpal::I24::new_unchecked(logical << (i32::BITS - 24))
                },
            ),
            cpal::SampleFormat::I32 => build_converting_tone!(i32),
            cpal::SampleFormat::I64 => build_converting_tone!(i64),
            cpal::SampleFormat::U8 => build_converting_tone!(u8),
            cpal::SampleFormat::U16 => build_converting_tone!(u16),
            cpal::SampleFormat::U24 => build_converting_tone!(cpal::U24),
            cpal::SampleFormat::U32 => build_converting_tone!(u32),
            cpal::SampleFormat::U64 => build_converting_tone!(u64),
            cpal::SampleFormat::F32 => build_converting_tone!(f32),
            cpal::SampleFormat::F64 => build_converting_tone!(f64),
            _ => return Err("default playback format cannot carry the native test tone"),
        }
        .map_err(|_| "native test tone stream could not be built")?;

        Ok(stream)
    }

    fn assert_capture_failure_channel_empty(
        failure_rx: &mut tokio::sync::mpsc::Receiver<SystemAudioCaptureFailure>,
    ) {
        match failure_rx.try_recv() {
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                panic!("the native loopback failure channel disconnected unexpectedly")
            }
            Ok(failure) => panic!(
                "the native loopback stream reported an early fatal failure: {}",
                failure.diagnostic_label()
            ),
        }
    }

    fn assert_normalized<T>(samples: &[T])
    where
        T: Sample,
        f32: FromSample<T>,
    {
        let mut output = Vec::new();
        normalize_samples(samples, &mut output);

        assert_eq!(output.len(), 3);
        assert!((output[0] + 1.0).abs() < 0.000_001, "{output:?}");
        assert!(output[1].abs() < f32::EPSILON, "{output:?}");
        assert!(output[2] > 0.99 && output[2] <= 1.0, "{output:?}");
    }

    #[test]
    fn every_cpal_pcm_and_float_format_is_supported() {
        for format in [
            cpal::SampleFormat::I8,
            cpal::SampleFormat::I16,
            cpal::SampleFormat::I24,
            cpal::SampleFormat::I32,
            cpal::SampleFormat::I64,
            cpal::SampleFormat::U8,
            cpal::SampleFormat::U16,
            cpal::SampleFormat::U24,
            cpal::SampleFormat::U32,
            cpal::SampleFormat::U64,
            cpal::SampleFormat::F32,
            cpal::SampleFormat::F64,
        ] {
            assert!(supports_sample_format(format), "unsupported {format}");
        }
    }

    #[test]
    fn dsd_formats_are_rejected_before_stream_creation() {
        for format in [
            cpal::SampleFormat::DsdU8,
            cpal::SampleFormat::DsdU16,
            cpal::SampleFormat::DsdU32,
        ] {
            assert!(!supports_sample_format(format), "accepted {format}");
        }
    }

    #[test]
    fn integer_samples_are_normalized_around_their_equilibrium() {
        assert_normalized(&[i8::MIN, 0, i8::MAX]);
        assert_normalized(&[i16::MIN, 0, i16::MAX]);
        assert_normalized(&[i32::MIN, 0, i32::MAX]);
        assert_normalized(&[i64::MIN, 0, i64::MAX]);
        assert_normalized(&[u8::MIN, 128, u8::MAX]);
        assert_normalized(&[u16::MIN, 32_768, u16::MAX]);
        assert_normalized(&[
            cpal::U24::new(0).unwrap(),
            cpal::U24::new(8_388_608).unwrap(),
            cpal::U24::new(16_777_215).unwrap(),
        ]);
        assert_normalized(&[u32::MIN, 2_147_483_648, u32::MAX]);
        assert_normalized(&[u64::MIN, 9_223_372_036_854_775_808, u64::MAX]);
    }

    #[test]
    fn wasapi_i24_left_aligned_container_is_shifted_before_normalization() {
        let samples = [
            cpal::I24::new_unchecked(i32::MIN),
            cpal::I24::new_unchecked(-1_073_741_824),
            cpal::I24::new_unchecked(0),
            cpal::I24::new_unchecked(1_073_741_824),
            cpal::I24::new_unchecked(0x4000_00ff),
            cpal::I24::new_unchecked(2_147_483_392),
        ];
        let mut output = Vec::new();

        normalize_wasapi_i24_samples(&samples, &mut output);

        assert_eq!(output.len(), samples.len());
        assert!((output[0] + 1.0).abs() < f32::EPSILON, "{output:?}");
        assert!((output[1] + 0.5).abs() < f32::EPSILON, "{output:?}");
        assert!(output[2].abs() < f32::EPSILON, "{output:?}");
        assert!((output[3] - 0.5).abs() < f32::EPSILON, "{output:?}");
        assert!((output[4] - 0.5).abs() < f32::EPSILON, "{output:?}");
        assert!(output[5] > 0.99 && output[5] <= 1.0, "{output:?}");
    }

    #[test]
    fn floating_point_samples_keep_their_amplitude() {
        assert_normalized(&[-1.0_f32, 0.0, 1.0]);
        assert_normalized(&[-1.0_f64, 0.0, 1.0]);
    }

    #[test]
    fn converted_callbacks_reuse_the_normalization_buffer() {
        let mut output = Vec::new();
        normalize_samples(&[i16::MIN, 0, i16::MAX], &mut output);
        let allocation = output.as_ptr();
        let capacity = output.capacity();

        normalize_samples(&[0_i16, 1, 2], &mut output);

        assert_eq!(output.as_ptr(), allocation);
        assert_eq!(output.capacity(), capacity);
    }

    #[test]
    fn test_tone_detector_accepts_997_hz_pcm16_across_callbacks() {
        let pcm = sine_pcm16(f64::from(TEST_TONE_FREQUENCY_HZ), 0.2);
        let split_at = 1_000 * size_of::<i16>();
        let mut detector = TestToneDetector::default();

        assert!(!detector.push_pcm16_le(&pcm[..split_at]));
        assert!(detector.push_pcm16_le(&pcm[split_at..]));
    }

    #[test]
    fn test_tone_detector_rejects_silence() {
        let silence = vec![0; TEST_TONE_WINDOW_SAMPLES * size_of::<i16>()];
        let mut detector = TestToneDetector::default();

        assert!(!detector.push_pcm16_le(&silence));
    }

    #[test]
    fn test_tone_detector_rejects_clearly_different_frequency() {
        let pcm = sine_pcm16(440.0, 0.2);
        let mut detector = TestToneDetector::default();

        assert!(!detector.push_pcm16_le(&pcm));
    }

    #[test]
    fn test_tone_detector_rejects_a_clipped_997_hz_square_wave() {
        let pcm = clipped_square_pcm16(f64::from(TEST_TONE_FREQUENCY_HZ));
        let mut detector = TestToneDetector::default();

        assert!(!detector.push_pcm16_le(&pcm));
    }

    #[test]
    fn test_tone_detector_continues_after_a_failed_window() {
        let off_frequency = sine_pcm16(440.0, 0.2);
        let test_tone = sine_pcm16(f64::from(TEST_TONE_FREQUENCY_HZ), 0.2);
        let mut detector = TestToneDetector::default();

        assert!(!detector.push_pcm16_le(&off_frequency));
        assert!(detector.push_pcm16_le(&test_tone));
    }

    /// Native acceptance probe for the real default render endpoint. This is
    /// ignored in the portable suite because CI runners are not guaranteed to
    /// expose an active playback device.
    #[tokio::test]
    #[ignore = "requires a Windows machine with a default playback endpoint"]
    async fn native_default_output_opens_as_a_wasapi_loopback_stream() {
        let output_device = cpal::default_host()
            .default_output_device()
            .expect("the native probe requires a default playback device");
        let output_failed = Arc::new(AtomicBool::new(false));
        let tone_stream = build_output_test_tone(&output_device, Arc::clone(&output_failed))
            .expect("the in-memory native test tone must use the default output");
        let (pcm_tx, mut pcm_rx) = tokio::sync::mpsc::channel(1);
        let detector = Arc::new(Mutex::new(TestToneDetector::default()));
        let detector_for_pipeline = Arc::clone(&detector);
        let pipeline = AudioSendPipeline::spawn(
            move |pcm| {
                let pcm_tx = pcm_tx.clone();
                let detected = detector_for_pipeline.lock().unwrap().push_pcm16_le(&pcm);
                async move {
                    if detected {
                        let _ = pcm_tx.try_send(());
                    }
                    Ok::<(), ()>(())
                }
            },
            |_failure| {},
        );
        let ingress = pipeline.ingress().expect("pipeline ingress");
        let (failure_tx, mut failure_rx) = CaptureFailureSender::channel();
        let capture = WindowsSystemAudioCapture::new();

        tone_stream
            .play()
            .expect("the in-memory native test tone must start");
        capture.set_source(output_device.id().unwrap().to_string());
        let capture_requested = Instant::now();
        capture
            .start(
                ingress,
                failure_tx,
                AudioCaptureFormat::pcm16_mono(16_000).unwrap(),
            )
            .await
            .expect("default output must open as a loopback input stream");
        let capture_ready = capture_requested.elapsed();
        assert_capture_failure_channel_empty(&mut failure_rx);

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert_capture_failure_channel_empty(&mut failure_rx);
            match pcm_rx.try_recv() {
                Ok(()) => break,
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    panic!("the native loopback PCM observation channel disconnected")
                }
            }
            assert!(
                !output_failed.load(Ordering::SeqCst),
                "the in-memory native test tone stopped unexpectedly"
            );
            assert!(
                Instant::now() < deadline,
                "WASAPI loopback did not deliver PCM for the in-memory native test tone"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_capture_failure_channel_empty(&mut failure_rx);
        let tone_observed = capture_requested.elapsed();

        drop(tone_stream);
        let stop_requested = Instant::now();
        capture.stop().await;
        eprintln!(
            "WASAPI probe ready_ms={} tone_observed_ms={} stop_ms={}",
            capture_ready.as_millis(),
            tone_observed.as_millis(),
            stop_requested.elapsed().as_millis()
        );
        pipeline.stop();
    }
}
