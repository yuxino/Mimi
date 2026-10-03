//! WASAPI process loopback. No playback rerouting, muting, or mix fallback.
use super::applications::Process;
use super::send_pipeline::{AudioIngress, AudioIngressError};
use super::{
    AudioCaptureFormat, CaptureFailureSender, SystemAudioCaptureError, SystemAudioCaptureFailure,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};
use windows::core::{implement, AgileReference, Interface, Ref, HRESULT};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, BLOB, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VT_BLOB;

const POLL: Duration = Duration::from_millis(5);
const START_TIMEOUT: Duration = Duration::from_secs(10);

pub fn supported() -> bool {
    // RtlGetVersion reports the actual build, independent of manifest compatibility.
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(
            version: *mut windows::Win32::System::SystemInformation::OSVERSIONINFOW,
        ) -> i32;
    }
    let mut version = windows::Win32::System::SystemInformation::OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<
            windows::Win32::System::SystemInformation::OSVERSIONINFOW,
        >() as u32,
        ..Default::default()
    };
    unsafe { RtlGetVersion(&mut version) == 0 && version.dwBuildNumber >= 20348 }
}

#[derive(Default)]
struct Worker {
    cancelled: AtomicBool,
    finished: AtomicBool,
}
#[derive(Default)]
struct State {
    worker: Mutex<Option<Arc<Worker>>>,
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.get_mut().unwrap() {
            worker.cancelled.store(true, Ordering::SeqCst);
        }
    }
}
#[derive(Clone, Default)]
pub struct WindowsApplicationCapture {
    state: Arc<State>,
}
struct Cancel(Option<Arc<Worker>>);
impl Drop for Cancel {
    fn drop(&mut self) {
        if let Some(worker) = &self.0 {
            worker.cancelled.store(true, Ordering::SeqCst);
        }
    }
}
struct Finished(Arc<Worker>);
impl Drop for Finished {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::SeqCst);
    }
}
struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
struct Client(IAudioClient);
impl Drop for Client {
    fn drop(&mut self) {
        let _ = unsafe { self.0.Stop() };
    }
}

impl WindowsApplicationCapture {
    pub async fn start(
        &self,
        id: String,
        ingress: AudioIngress,
        failure: CaptureFailureSender,
        format: AudioCaptureFormat,
    ) -> Result<(), SystemAudioCaptureError> {
        if !supported() {
            return Err(SystemAudioCaptureError::ApplicationUnsupported);
        }
        let worker = {
            let mut slot = self.state.worker.lock().unwrap();
            if slot
                .as_ref()
                .is_some_and(|worker| !worker.finished.load(Ordering::SeqCst))
            {
                return Err(SystemAudioCaptureError::PreviousCaptureStopping);
            }
            let worker = Arc::new(Worker::default());
            *slot = Some(Arc::clone(&worker));
            worker
        };
        let mut cancel = Cancel(Some(Arc::clone(&worker)));
        let (tx, rx) = tokio::sync::oneshot::channel();
        let control = Arc::clone(&worker);
        if std::thread::Builder::new()
            .name("mimi-application-audio".into())
            .spawn(move || {
                let _finished = Finished(Arc::clone(&control));
                let result = open(&id, &control, format);
                let native = match result {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        return;
                    }
                };
                if control.cancelled.load(Ordering::SeqCst) {
                    let _ = tx.send(Err(SystemAudioCaptureError::StartCancelled));
                    return;
                }
                if unsafe { native.client.0.Start() }.is_err() {
                    let _ = tx.send(Err(SystemAudioCaptureError::NativeStartFailed));
                    return;
                }
                if tx.send(Ok(())).is_err() {
                    return;
                }
                while !control.cancelled.load(Ordering::SeqCst) && !failure.has_reported() {
                    if !native.process.running() {
                        failure.report(SystemAudioCaptureFailure::ApplicationUnavailable);
                        break;
                    }
                    if read_packets(&native.capture, &control, &ingress, &failure).is_err() {
                        failure.report(SystemAudioCaptureFailure::NativeStopped);
                        break;
                    }
                    std::thread::sleep(POLL);
                }
                // COM objects must be released before the worker's apartment.
                drop(native);
            })
            .is_err()
        {
            worker.finished.store(true, Ordering::SeqCst);
            return Err(SystemAudioCaptureError::NativeStartFailed);
        }
        tokio::time::timeout(START_TIMEOUT, rx)
            .await
            .map_err(|_| SystemAudioCaptureError::StartTimedOut)?
            .map_err(|_| SystemAudioCaptureError::NativeStartFailed)??;
        if worker.cancelled.load(Ordering::SeqCst) {
            return Err(SystemAudioCaptureError::StartCancelled);
        }
        cancel.0 = None;
        Ok(())
    }
    pub async fn stop(&self) {
        let worker = self.state.worker.lock().unwrap().clone();
        if let Some(worker) = worker {
            worker.cancelled.store(true, Ordering::SeqCst);
            while !worker.finished.load(Ordering::SeqCst) {
                tokio::time::sleep(POLL).await;
            }
        }
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activation {
    // Windows retains the completion handler until activation finishes, so
    // cancellation cannot outlive the process-loopback parameter storage.
    params: AUDIOCLIENT_ACTIVATION_PARAMS,
    tx: Mutex<Option<mpsc::Sender<windows::core::Result<AgileReference<IAudioClient>>>>>,
}
impl IActivateAudioInterfaceCompletionHandler_Impl for Activation_Impl {
    fn ActivateCompleted(
        &self,
        operation: Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let result = (|| {
            let mut status = HRESULT(0);
            let mut unknown = None;
            unsafe {
                operation
                    .ok()?
                    .GetActivateResult(&mut status, &mut unknown)?;
            }
            status.ok()?;
            let client: IAudioClient = unknown
                .ok_or_else(|| windows::core::Error::from_hresult(HRESULT(0x80004005u32 as i32)))?
                .cast()?;
            // Use COM's agile reference to marshal the result to our worker.
            AgileReference::new(&client)
        })();
        if let Some(tx) = self.tx.lock().unwrap().take() {
            let _ = tx.send(result);
        }
        Ok(())
    }
}

struct OpenCapture {
    client: Client,
    capture: IAudioCaptureClient,
    process: Process,
    _com: Com,
}
fn open(
    id: &str,
    worker: &Worker,
    format: AudioCaptureFormat,
) -> Result<OpenCapture, SystemAudioCaptureError> {
    AudioCaptureFormat::pcm16_mono(format.sample_rate_hz)?;
    let process = Process::selected(id)?;
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|_| SystemAudioCaptureError::NativeStartFailed)?;
    let com = Com;
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: process.pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let (tx, rx) = mpsc::channel();
    let activation = windows::core::ComObject::new(Activation {
        params,
        tx: Mutex::new(Some(tx)),
    });
    let params = &activation.get().params;
    let variant = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: std::mem::size_of_val(params) as u32,
                        pBlobData: params as *const _ as *mut u8,
                    },
                },
                ..Default::default()
            }),
        },
    };
    let handler = activation.to_interface::<IActivateAudioInterfaceCompletionHandler>();
    let _operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&variant),
            &handler,
        )
    }
    .map_err(|_| SystemAudioCaptureError::NativeStartFailed)?;
    let deadline = Instant::now() + START_TIMEOUT;
    let client = loop {
        if worker.cancelled.load(Ordering::SeqCst) {
            return Err(SystemAudioCaptureError::StartCancelled);
        }
        if Instant::now() >= deadline {
            return Err(SystemAudioCaptureError::NativeStartFailed);
        }
        match rx.recv_timeout(POLL) {
            Ok(result) => {
                break result
                    .and_then(|client| client.resolve())
                    .map_err(|_| SystemAudioCaptureError::NativeStartFailed)?
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return Err(SystemAudioCaptureError::NativeStartFailed),
        }
    };
    // The audio engine converts directly to provider-rate mono PCM16, avoiding
    // an extra resampler and keeping packet memory bounded by the native buffer.
    let wave = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 1,
        nSamplesPerSec: format.sample_rate_hz,
        nAvgBytesPerSec: format.sample_rate_hz * 2,
        nBlockAlign: 2,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            200_000,
            0,
            &wave,
            None,
        )
    }
    .map_err(|_| SystemAudioCaptureError::NativeStartFailed)?;
    let capture = unsafe { client.GetService::<IAudioCaptureClient>() }
        .map_err(|_| SystemAudioCaptureError::NativeStartFailed)?;
    Ok(OpenCapture {
        client: Client(client),
        capture,
        process,
        _com: com,
    })
}

fn read_packets(
    capture: &IAudioCaptureClient,
    worker: &Worker,
    ingress: &AudioIngress,
    failure: &CaptureFailureSender,
) -> windows::core::Result<()> {
    // Limit work per poll as well as allocation; cancellation cannot wait for
    // an unbounded native packet drain when a driver floods the queue.
    for _ in 0..128 {
        if worker.cancelled.load(Ordering::SeqCst)
            || failure.has_reported()
            || unsafe { capture.GetNextPacketSize()? } == 0
        {
            break;
        }
        let mut data = std::ptr::null_mut();
        let mut frames = 0;
        let mut flags = 0;
        unsafe {
            capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
        }
        // SAFETY: WASAPI owns a frames-long mono PCM16 packet until ReleaseBuffer.
        let pcm = unsafe { copy_packet(data, frames, flags) };
        // Release even a malformed packet before reporting the fatal error.
        unsafe {
            capture.ReleaseBuffer(frames)?;
        }
        if let Some(pcm) = pcm {
            if pcm.is_empty() || worker.cancelled.load(Ordering::SeqCst) {
                continue;
            }
            match ingress.try_send(pcm) {
                Ok(()) => {}
                Err(AudioIngressError::Closed) => break,
                Err(AudioIngressError::Backpressure) => {
                    failure.report(SystemAudioCaptureFailure::Backpressure);
                    break;
                }
            }
        } else {
            failure.report(SystemAudioCaptureFailure::AudioProcessingFailed);
            break;
        }
    }
    Ok(())
}

/// Never inspect SILENT packet pointers, even when the driver supplies one.
unsafe fn copy_packet(data: *const u8, frames: u32, flags: u32) -> Option<Vec<u8>> {
    if frames > 48_000 {
        None
    } else if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
        Some(vec![0; frames as usize * 2])
    } else if frames == 0 {
        Some(Vec::new())
    } else if data.is_null() {
        None
    } else {
        Some(unsafe { std::slice::from_raw_parts(data, frames as usize * 2) }.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silent_packets_do_not_read_undefined_native_bytes() {
        let invalid = std::ptr::dangling::<u8>();
        assert_eq!(
            unsafe { copy_packet(invalid, 4, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) },
            Some(vec![0; 8])
        );
        assert_eq!(unsafe { copy_packet(std::ptr::null(), 4, 0) }, None);
        assert_eq!(unsafe { copy_packet(invalid, 48_001, 0) }, None);
        assert_eq!(unsafe { copy_packet(std::ptr::null(), 0, 0) }, Some(vec![]));
        let pcm = [1, 0, 2, 0];
        assert_eq!(
            unsafe { copy_packet(pcm.as_ptr(), 2, 0) },
            Some(pcm.to_vec())
        );
    }
    #[test]
    fn malformed_or_self_targets_cannot_activate_capture() {
        for id in ["", "windows:0:0", "com.example.player", "windows:garbage:0"] {
            assert!(Process::selected(id).is_err());
        }
        assert!(Process::selected(&format!("windows:{}:0", std::process::id())).is_err());
    }
}
