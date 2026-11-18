//! Explicitly selected default-input capture on macOS and Windows.
//!
//! A single worker owns the native stream, including its destruction. Dropping
//! a pending start cancels that worker; a new start cannot overlap teardown.
//! Callbacks never block on network work and retain only bounded resampler data.

use super::send_pipeline::{AudioIngress, AudioIngressError};
use super::streaming_resampler::StreamingPcm16Resampler;
use super::{
    AudioCaptureFormat, CaptureFailureSender, SystemAudioCaptureError, SystemAudioCaptureFailure,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

const POLL_INTERVAL: Duration = Duration::from_millis(5);
const MAX_CALLBACK_MS: usize = 250;

#[derive(Clone, Default)]
pub struct MicrophoneCapture {
    state: Arc<CaptureState>,
}

#[derive(Default)]
struct CaptureState {
    worker: Mutex<Option<Arc<WorkerControl>>>,
}

impl Drop for CaptureState {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.get_mut().unwrap().as_ref() {
            worker.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

#[derive(Default)]
struct WorkerControl {
    cancelled: AtomicBool,
    finished: AtomicBool,
    device_name: Mutex<Option<String>>,
}

struct CancelStartOnDrop(Option<Arc<WorkerControl>>);
impl Drop for CancelStartOnDrop {
    fn drop(&mut self) {
        if let Some(worker) = &self.0 {
            worker.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

struct FinishWorkerOnDrop(Arc<WorkerControl>);
impl Drop for FinishWorkerOnDrop {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::SeqCst);
    }
}

impl MicrophoneCapture {
    pub async fn start(
        &self,
        ingress: AudioIngress,
        failure: CaptureFailureSender,
        format: AudioCaptureFormat,
    ) -> Result<(), SystemAudioCaptureError> {
        AudioCaptureFormat::pcm16_mono(format.sample_rate_hz)?;
        let control = self.reserve_worker()?;
        let mut cancel_start = CancelStartOnDrop(Some(Arc::clone(&control)));
        let (ready_tx, ready_rx) = oneshot::channel();
        let worker = Arc::clone(&control);
        if std::thread::Builder::new()
            .name("mimi-microphone".into())
            .spawn(move || {
                // Declare first so finished is published after the stream drops.
                let _finished = FinishWorkerOnDrop(Arc::clone(&worker));
                let stream = match open_stream(&worker, ingress, failure.clone(), format) {
                    Ok(stream) => stream,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if worker.cancelled.load(Ordering::SeqCst) {
                    let _ = ready_tx.send(Err(SystemAudioCaptureError::StartCancelled));
                    return;
                }
                if let Err(error) = stream.play() {
                    let _ = ready_tx.send(Err(map_native_error(error)));
                    return;
                }
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                while !worker.cancelled.load(Ordering::SeqCst) && !failure.has_reported() {
                    std::thread::sleep(POLL_INTERVAL);
                }
                // Drop closes the input even after unplugging, callback failure,
                // cancellation, or a silent device. No new audio is needed.
                drop(stream);
            })
            .is_err()
        {
            control.finished.store(true, Ordering::SeqCst);
            return Err(SystemAudioCaptureError::MicrophoneStartFailed);
        }
        ready_rx
            .await
            .map_err(|_| SystemAudioCaptureError::MicrophoneStartFailed)??;
        if control.cancelled.load(Ordering::SeqCst) {
            return Err(SystemAudioCaptureError::StartCancelled);
        }
        cancel_start.0 = None;
        Ok(())
    }

    fn reserve_worker(&self) -> Result<Arc<WorkerControl>, SystemAudioCaptureError> {
        let mut slot = self.state.worker.lock().unwrap();
        if let Some(worker) = slot.as_ref() {
            if !worker.finished.load(Ordering::SeqCst) {
                return Err(if worker.cancelled.load(Ordering::SeqCst) {
                    SystemAudioCaptureError::PreviousCaptureStopping
                } else {
                    SystemAudioCaptureError::AlreadyRunning
                });
            }
        }
        let worker = Arc::new(WorkerControl::default());
        *slot = Some(Arc::clone(&worker));
        Ok(worker)
    }

    pub fn device_name(&self) -> Option<String> {
        let worker = self.state.worker.lock().unwrap().clone()?;
        if worker.cancelled.load(Ordering::SeqCst) || worker.finished.load(Ordering::SeqCst) {
            return None;
        }
        let name = worker.device_name.lock().unwrap().clone();
        name
    }

    pub async fn stop(&self) {
        let worker = self.state.worker.lock().unwrap().clone();
        if let Some(worker) = worker {
            worker.cancelled.store(true, Ordering::SeqCst);
            while !worker.finished.load(Ordering::SeqCst) {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
    }
}

fn map_native_error(error: cpal::Error) -> SystemAudioCaptureError {
    match error.kind() {
        cpal::ErrorKind::PermissionDenied => SystemAudioCaptureError::MicrophonePermissionDenied,
        cpal::ErrorKind::DeviceNotAvailable => SystemAudioCaptureError::NoMicrophoneDevice,
        _ => SystemAudioCaptureError::MicrophoneStartFailed,
    }
}

fn open_stream(
    control: &Arc<WorkerControl>,
    ingress: AudioIngress,
    failure: CaptureFailureSender,
    format: AudioCaptureFormat,
) -> Result<cpal::Stream, SystemAudioCaptureError> {
    #[cfg(target_os = "macos")]
    request_microphone_permission(control)?;
    if control.cancelled.load(Ordering::SeqCst) {
        return Err(SystemAudioCaptureError::StartCancelled);
    }
    // Never resolve an output endpoint or retry with system loopback. A
    // missing/denied microphone must remain a visible microphone error.
    let device = cpal::default_host()
        .default_input_device()
        .ok_or(SystemAudioCaptureError::NoMicrophoneDevice)?;
    let config = device.default_input_config().map_err(map_native_error)?;
    *control.device_name.lock().unwrap() = device.description().ok().map(|d| d.name().to_owned());
    let sample_format = config.sample_format();
    let processor =
        MicrophoneProcessor::new(config.sample_rate(), config.channels() as usize, format)?;
    let config = config.into();
    macro_rules! stream {
        ($sample:ty) => {
            build_stream::<$sample>(
                &device,
                config,
                Arc::clone(control),
                ingress,
                failure,
                processor,
                |sample| f32::from_sample(sample),
            )
        };
    }
    match sample_format {
        cpal::SampleFormat::F32 => stream!(f32),
        cpal::SampleFormat::F64 => stream!(f64),
        cpal::SampleFormat::I8 => stream!(i8),
        cpal::SampleFormat::I16 => stream!(i16),
        cpal::SampleFormat::I24 => build_stream::<cpal::I24>(
            &device,
            config,
            Arc::clone(control),
            ingress,
            failure,
            processor,
            normalize_i24,
        ),
        cpal::SampleFormat::I32 => stream!(i32),
        cpal::SampleFormat::I64 => stream!(i64),
        cpal::SampleFormat::U8 => stream!(u8),
        cpal::SampleFormat::U16 => stream!(u16),
        cpal::SampleFormat::U24 => stream!(cpal::U24),
        cpal::SampleFormat::U32 => stream!(u32),
        cpal::SampleFormat::U64 => stream!(u64),
        _ => Err(SystemAudioCaptureError::UnsupportedAudioFormat),
    }
}

fn normalize_i24(sample: cpal::I24) -> f32 {
    #[cfg(target_os = "windows")]
    {
        (sample.inner() >> 8) as f32 / 8_388_608.0
    }
    #[cfg(not(target_os = "windows"))]
    {
        f32::from_sample(sample)
    }
}

struct MicrophoneProcessor {
    resampler: StreamingPcm16Resampler,
    normalized: Vec<f32>,
    max_samples: usize,
}

impl MicrophoneProcessor {
    fn new(
        rate: u32,
        channels: usize,
        format: AudioCaptureFormat,
    ) -> Result<Self, SystemAudioCaptureError> {
        Ok(Self {
            resampler: StreamingPcm16Resampler::new(rate, format.sample_rate_hz, channels)?,
            normalized: Vec::new(),
            max_samples: rate as usize * channels * MAX_CALLBACK_MS / 1000,
        })
    }

    fn process<T: Sample>(
        &mut self,
        data: &[T],
        normalize: &impl Fn(T) -> f32,
    ) -> Result<Vec<Vec<u8>>, SystemAudioCaptureFailure> {
        if data.len() > self.max_samples {
            return Err(SystemAudioCaptureFailure::Backpressure);
        }
        self.normalized.extend(data.iter().copied().map(normalize));
        let buffers = self
            .resampler
            .push_interleaved(&self.normalized)
            .map_err(|_| SystemAudioCaptureFailure::AudioProcessingFailed);
        self.normalized.clear();
        buffers
    }
}

fn build_stream<T: cpal::SizedSample>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    control: Arc<WorkerControl>,
    ingress: AudioIngress,
    failure: CaptureFailureSender,
    mut processor: MicrophoneProcessor,
    normalize: impl Fn(T) -> f32 + Send + 'static,
) -> Result<cpal::Stream, SystemAudioCaptureError> {
    let error_failure = failure.clone();
    let error_control = Arc::clone(&control);
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if control.cancelled.load(Ordering::SeqCst) || failure.has_reported() {
                    return;
                }
                let buffers = match processor.process(data, &normalize) {
                    Ok(buffers) => buffers,
                    Err(error) => {
                        failure.report(error);
                        return;
                    }
                };
                for pcm in buffers {
                    if control.cancelled.load(Ordering::SeqCst) {
                        return;
                    }
                    match ingress.try_send(pcm) {
                        Ok(()) => {}
                        Err(AudioIngressError::Backpressure) => {
                            failure.report(SystemAudioCaptureFailure::Backpressure);
                            return;
                        }
                        Err(AudioIngressError::Closed) => {
                            control.cancelled.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                }
            },
            move |_error| {
                if !error_control.cancelled.load(Ordering::SeqCst) {
                    error_failure.report(SystemAudioCaptureFailure::NativeStopped);
                }
            },
            None,
        )
        .map_err(map_native_error)
}

#[cfg(target_os = "macos")]
fn request_microphone_permission(control: &WorkerControl) -> Result<(), SystemAudioCaptureError> {
    use objc2::runtime::Bool;
    use objc2::{class, msg_send};
    use objc2_foundation::NSString;
    // Loading AVFoundation registers AVCaptureDevice. The media type value is
    // Apple's AVMediaTypeAudio constant; no camera access is requested.
    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        static AVMediaTypeAudio: &'static NSString;
    }
    // SAFETY: AVFoundation's documented class methods accept NSString media
    // types, return NSInteger authorization status, and copy the completion block.
    unsafe {
        let media_type = AVMediaTypeAudio;
        let status: isize =
            msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: media_type];
        match status {
            3 => return Ok(()), // AVAuthorizationStatusAuthorized
            1 | 2 => return Err(SystemAudioCaptureError::MicrophonePermissionDenied),
            0 => {} // AVAuthorizationStatusNotDetermined
            _ => return Err(SystemAudioCaptureError::MicrophonePermissionDenied),
        }
        if control.cancelled.load(Ordering::SeqCst) {
            return Err(SystemAudioCaptureError::StartCancelled);
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let completion = block2::RcBlock::new(move |granted: Bool| {
            let _ = tx.try_send(granted.as_bool());
        });
        let _: () = msg_send![class!(AVCaptureDevice), requestAccessForMediaType: media_type, completionHandler: &*completion];
        loop {
            if control.cancelled.load(Ordering::SeqCst) {
                return Err(SystemAudioCaptureError::StartCancelled);
            }
            match rx.recv_timeout(POLL_INTERVAL) {
                Ok(true) => return Ok(()),
                Ok(false) => return Err(SystemAudioCaptureError::MicrophonePermissionDenied),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(SystemAudioCaptureError::MicrophoneStartFailed),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_start_keeps_the_microphone_reserved_until_released() {
        let capture = MicrophoneCapture::default();
        let first = capture.reserve_worker().unwrap();
        assert!(matches!(
            capture.reserve_worker(),
            Err(SystemAudioCaptureError::AlreadyRunning)
        ));
        drop(CancelStartOnDrop(Some(first.clone())));
        assert!(first.cancelled.load(Ordering::SeqCst));
        assert!(matches!(
            capture.reserve_worker(),
            Err(SystemAudioCaptureError::PreviousCaptureStopping)
        ));
        drop(FinishWorkerOnDrop(first.clone()));
        let second = capture.reserve_worker().unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        drop(capture);
        assert!(second.cancelled.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn stop_cancels_immediately_but_waits_until_native_resources_are_released() {
        let capture = MicrophoneCapture::default();
        let worker = capture.reserve_worker().unwrap();
        let stop = capture.stop();
        tokio::pin!(stop);
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut stop)
            .await
            .is_err());
        assert!(worker.cancelled.load(Ordering::SeqCst));
        assert!(!worker.finished.load(Ordering::SeqCst));
        drop(FinishWorkerOnDrop(worker));
        tokio::time::timeout(Duration::from_millis(100), stop)
            .await
            .unwrap();
        assert!(capture.reserve_worker().is_ok());
    }

    #[test]
    fn microphone_converts_stereo_integer_input_to_provider_pcm() {
        let mut processor =
            MicrophoneProcessor::new(24_000, 2, AudioCaptureFormat::pcm16_mono(24_000).unwrap())
                .unwrap();
        let pcm = processor
            .process(&[16_384_i16, 0, -16_384, 0], &f32::from_sample)
            .unwrap();
        let samples: Vec<_> = pcm
            .concat()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| i16::from_le_bytes(*p))
            .collect();
        assert_eq!(samples.len(), 2);
        assert!((samples[0] - 8_192).abs() <= 1);
        assert!((samples[1] + 8_192).abs() <= 1);
        assert!(processor.normalized.is_empty());
    }

    #[test]
    fn oversized_native_callback_is_rejected_before_copying() {
        let mut processor =
            MicrophoneProcessor::new(24_000, 1, AudioCaptureFormat::pcm16_mono(24_000).unwrap())
                .unwrap();
        assert_eq!(
            processor.process(&vec![0.0_f32; 6_001], &|s| s),
            Err(SystemAudioCaptureFailure::Backpressure)
        );
        assert_eq!(processor.normalized.capacity(), 0);
    }

    #[test]
    fn native_errors_remain_microphone_specific_and_content_free() {
        assert_eq!(
            map_native_error(cpal::Error::new(cpal::ErrorKind::PermissionDenied)),
            SystemAudioCaptureError::MicrophonePermissionDenied
        );
        assert_eq!(
            map_native_error(cpal::Error::new(cpal::ErrorKind::DeviceNotAvailable)),
            SystemAudioCaptureError::NoMicrophoneDevice
        );
        assert_eq!(
            map_native_error(cpal::Error::new(cpal::ErrorKind::BackendError)),
            SystemAudioCaptureError::MicrophoneStartFailed
        );
    }
}
