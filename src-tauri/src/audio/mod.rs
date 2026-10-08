//! Explicitly selected platform audio capture (system audio by default) and the
//! bounded PCM send pipeline.

pub(crate) mod echo_pipeline;
pub mod send_pipeline;

/// Local presentation only. Device names never enter support diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDetails {
    pub kind: &'static str,
    pub strategy: &'static str,
    pub actual_device_name: Option<String>,
    /// macOS's current default playback destination. The system mix captures
    /// independently; this is not a selected capture device.
    pub system_output_device_name: Option<String>,
    pub observation: Option<CaptureSignal>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCaptureStatus {
    pub audio_source: crate::core::audio_input::AudioSource,
    #[serde(flatten)]
    pub details: CaptureDetails,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    /// Retain the aggregate wire fields for older presentation consumers.
    #[serde(flatten)]
    pub details: CaptureDetails,
    pub sources: Vec<SourceCaptureStatus>,
}

impl CaptureStatus {
    pub fn for_input(
        input: crate::core::audio_input::AudioInput,
        mut details_for_source: impl FnMut(crate::core::audio_input::AudioSource) -> CaptureDetails,
    ) -> Self {
        use crate::core::audio_input::{AudioInput, AudioSource};
        let sources: Vec<_> = input
            .sources()
            .iter()
            .map(|&audio_source| SourceCaptureStatus {
                audio_source,
                details: details_for_source(audio_source),
            })
            .collect();
        let details = if input == AudioInput::Both {
            CaptureDetails {
                kind: "both",
                strategy: "independent_inputs",
                actual_device_name: sources
                    .iter()
                    .find(|source| source.audio_source == AudioSource::Microphone)
                    .and_then(|source| source.details.actual_device_name.clone()),
                system_output_device_name: sources
                    .iter()
                    .find(|source| source.audio_source == AudioSource::System)
                    .and_then(|source| source.details.system_output_device_name.clone()),
                observation: sources
                    .iter()
                    .filter_map(|source| source.details.observation)
                    .reduce(|left, right| CaptureSignal {
                        pcm_data_recent: left.pcm_data_recent || right.pcm_data_recent,
                        sound_recent: left.sound_recent || right.sound_recent,
                    }),
            }
        } else {
            // AudioInput always contains at least one explicitly selected source.
            sources[0].details.clone()
        };
        Self { details, sources }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSignal {
    pub pcm_data_recent: bool,
    pub sound_recent: bool,
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
mod streaming_resampler;

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod microphone;

pub mod applications;
pub mod census;
#[cfg(target_os = "windows")]
mod windows_application;
#[cfg(target_os = "windows")]
mod windows_application_icons;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "macos")]
mod macos_block_buffer;

#[cfg(target_os = "macos")]
mod macos_tap;

#[cfg(target_os = "macos")]
pub mod macos_output;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub mod unsupported;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSourceDevice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSourceSnapshot {
    pub devices: Vec<AudioSourceDevice>,
    pub current_device: Option<String>,
    pub receiving_sound: bool,
    pub receiving_audio_data: bool,
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SystemAudioCaptureError {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[error("application_audio_unavailable")]
    ApplicationUnavailable,
    #[error("application_audio_unsupported")]
    ApplicationUnsupported,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[error("application_audio_list_failed")]
    ApplicationListFailed,
    #[error("Audio capture is already running.")]
    AlreadyRunning,
    #[cfg(target_os = "macos")]
    #[error("mimi could not find a display to use for system audio capture.")]
    NoDisplay,
    #[error("The system returned an unsupported audio format.")]
    UnsupportedAudioFormat,
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[error("No default playback device is available for system audio capture.")]
    NoPlaybackDevice,
    #[cfg(target_os = "macos")]
    #[error("System audio capture permission was denied.")]
    PermissionDenied,
    #[cfg(target_os = "macos")]
    #[error("System audio capture was stopped by the user.")]
    UserStopped,
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[error("Audio capture setup timed out.")]
    StartTimedOut,
    #[error("Audio capture start was cancelled.")]
    StartCancelled,
    #[error("The previous audio capture is still stopping.")]
    PreviousCaptureStopping,
    #[cfg(target_os = "windows")]
    #[error("The selected sound output is unavailable. Stop subtitles and choose another sound source in Settings.")]
    SelectedPlaybackDeviceUnavailable,
    #[error("No default microphone is available.")]
    NoMicrophoneDevice,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[error("Microphone capture permission was denied.")]
    MicrophonePermissionDenied,
    #[error("Microphone capture could not be started.")]
    MicrophoneStartFailed,
    #[error("System audio capture could not be started.")]
    NativeStartFailed,
    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    #[error("Audio capture could not process the device audio format.")]
    AudioProcessingFailed,
    #[cfg(target_os = "linux")]
    #[error("Connect to PulseAudio or PipeWire with PulseAudio support to capture audio.")]
    AudioServerUnavailable,
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    #[error("System audio capture is not supported on this platform.")]
    UnsupportedPlatform,
}

/// Fatal failures reported after a native capture session has started.
///
/// The variants deliberately contain no platform or provider free text so the
/// same value is safe to use for both recovery decisions and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SystemAudioCaptureFailure {
    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    #[error("application_audio_unavailable")]
    ApplicationUnavailable,
    #[error("Audio capture stopped unexpectedly.")]
    NativeStopped,
    #[cfg(any(target_os = "macos", test))]
    #[error("System audio capture permission was denied.")]
    PermissionDenied,
    #[cfg(any(target_os = "macos", test))]
    #[error("System audio capture was stopped by the user.")]
    UserStopped,
    #[error("Audio capture could not process the device audio format.")]
    AudioProcessingFailed,
    #[error("Audio streaming fell behind. mimi is reconnecting.")]
    Backpressure,
}

impl SystemAudioCaptureFailure {
    pub fn is_recoverable(self) -> bool {
        match self {
            #[cfg(any(target_os = "macos", target_os = "windows", test))]
            Self::ApplicationUnavailable => false,
            #[cfg(any(target_os = "macos", test))]
            Self::PermissionDenied | Self::UserStopped => false,
            Self::NativeStopped | Self::AudioProcessingFailed | Self::Backpressure => true,
        }
    }

    pub fn diagnostic_label(self) -> &'static str {
        match self {
            Self::NativeStopped => "capture.native_stopped",
            #[cfg(any(target_os = "macos", test))]
            Self::PermissionDenied => "capture.permission_denied",
            #[cfg(any(target_os = "macos", test))]
            Self::UserStopped => "capture.user_stopped",
            #[cfg(any(target_os = "macos", target_os = "windows", test))]
            Self::ApplicationUnavailable => "capture.application_unavailable",
            Self::AudioProcessingFailed => "capture.audio_processing_failed",
            Self::Backpressure => "capture.backpressure",
        }
    }
}

/// Cloneable, non-blocking, exactly-once failure reporter for native audio
/// callbacks. The bounded channel holds one fatal failure because a capture
/// generation is torn down after the first one.
#[derive(Clone)]
pub struct CaptureFailureSender {
    tx: mpsc::Sender<SystemAudioCaptureFailure>,
    reported: Arc<AtomicBool>,
}

impl CaptureFailureSender {
    pub fn channel() -> (Self, mpsc::Receiver<SystemAudioCaptureFailure>) {
        let (tx, rx) = mpsc::channel(1);
        (
            Self {
                tx,
                reported: Arc::new(AtomicBool::new(false)),
            },
            rx,
        )
    }

    /// Reports the first fatal failure without ever blocking the native audio
    /// callback. Returns true only for the caller that claimed the report.
    pub fn report(&self, failure: SystemAudioCaptureFailure) -> bool {
        if self
            .reported
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        let _ = self.tx.try_send(failure);
        true
    }

    pub fn has_reported(&self) -> bool {
        self.reported.load(Ordering::SeqCst)
    }
}

/// Provider-requested wire format. All supported backends capture one selected
/// audio source, mix to mono, resample to this rate, and encode little-endian
/// PCM16 before emitting buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioCaptureFormat {
    pub sample_rate_hz: u32,
}

impl AudioCaptureFormat {
    pub fn pcm16_mono(sample_rate_hz: u32) -> Result<Self, SystemAudioCaptureError> {
        if !matches!(sample_rate_hz, 16_000 | 24_000) {
            return Err(SystemAudioCaptureError::UnsupportedAudioFormat);
        }
        Ok(Self { sample_rate_hz })
    }
}

/// Cross-platform handle for an active capture session.
#[cfg(target_os = "macos")]
pub type SystemAudioCapture = macos::MacSystemAudioCapture;

#[cfg(target_os = "windows")]
pub type SystemAudioCapture = windows::WindowsSystemAudioCapture;

#[cfg(target_os = "linux")]
pub type SystemAudioCapture = linux::LinuxSystemAudioCapture;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub type SystemAudioCapture = unsupported::UnsupportedSystemAudioCapture;

impl SystemAudioCapture {
    /// Creates the platform capture handle for this app. On macOS the
    /// ScreenCaptureKit objects are confined to the main thread through the
    /// app handle's run-on-main-thread executor.
    pub fn for_app(app: &tauri::AppHandle) -> Self {
        #[cfg(target_os = "macos")]
        {
            let app = app.clone();
            macos::MacSystemAudioCapture::new(std::sync::Arc::new(
                move |task: Box<dyn FnOnce() + Send>| {
                    // A dropped dispatch silently leaves the capture running;
                    // surface the failure instead of ignoring it.
                    if app.run_on_main_thread(task).is_err() {
                        tracing::error!("main-thread dispatch failed label=tauri_dispatch_failed");
                    }
                },
            ))
        }
        #[cfg(target_os = "windows")]
        {
            // The AppHandle is macOS-only (ScreenCaptureKit main-thread
            // dispatch); Windows WASAPI needs no app handle.
            let _ = app;
            windows::WindowsSystemAudioCapture::new()
        }
        #[cfg(target_os = "linux")]
        {
            let _ = app;
            linux::LinuxSystemAudioCapture::new()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = app;
            unsupported::UnsupportedSystemAudioCapture::new()
        }
    }
}

/// One independently owned input for a capture lane. Separate handles capture
/// system audio and microphone concurrently without sharing native workers.
#[derive(Clone)]
pub struct AudioCapture {
    system: SystemAudioCapture,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    microphone: microphone::MicrophoneCapture,
    #[cfg(target_os = "windows")]
    application: windows_application::WindowsApplicationCapture,
    state: Arc<std::sync::Mutex<SelectionState>>,
}

#[derive(Default)]
struct SelectionState {
    next_token: u64,
    active: Option<u64>,
    stopping: usize,
    starting: usize,
}

struct CancelSelectionOnDrop {
    state: Arc<std::sync::Mutex<SelectionState>>,
    token: u64,
    armed: bool,
}

impl Drop for CancelSelectionOnDrop {
    fn drop(&mut self) {
        {
            let mut state = self.state.lock().unwrap();
            state.starting -= 1;
            if self.armed && state.active == Some(self.token) {
                state.active = None;
            }
        }
    }
}

impl AudioCapture {
    pub fn for_app(app: &tauri::AppHandle) -> Self {
        Self {
            system: SystemAudioCapture::for_app(app),
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            microphone: microphone::MicrophoneCapture::default(),
            #[cfg(target_os = "windows")]
            application: Default::default(),
            state: Arc::new(std::sync::Mutex::new(SelectionState::default())),
        }
    }

    pub async fn start(
        &self,
        ingress: send_pipeline::AudioIngress,
        failure: CaptureFailureSender,
        format: AudioCaptureFormat,
        input: crate::core::audio_input::AudioSource,
        target: crate::core::system_audio_target::SystemAudioTarget,
    ) -> Result<(), SystemAudioCaptureError> {
        let token = {
            let mut state = self.state.lock().unwrap();
            if state.stopping > 0 {
                return Err(SystemAudioCaptureError::PreviousCaptureStopping);
            }
            if state.active.is_some() || state.starting > 0 {
                return Err(SystemAudioCaptureError::AlreadyRunning);
            }
            let token = state.next_token;
            state.next_token = state.next_token.wrapping_add(1);
            state.active = Some(token);
            state.starting += 1;
            token
        };
        let mut reservation = CancelSelectionOnDrop {
            state: Arc::clone(&self.state),
            token,
            armed: true,
        };
        // A dropped prior start has cancelled its native worker. Wait for it
        // to release the device before allowing a different source to open.
        self.stop_backends().await;
        #[cfg(target_os = "macos")]
        self.system.wait_until_idle().await?;
        if self.state.lock().unwrap().active != Some(token) {
            return Err(SystemAudioCaptureError::StartCancelled);
        }
        match input {
            crate::core::audio_input::AudioSource::System => {
                #[cfg(target_os = "macos")]
                self.system.start(ingress, failure, format, target).await?;
                #[cfg(target_os = "windows")]
                if let Some(id) = target.application_id() {
                    self.application
                        .start(id.to_string(), ingress, failure, format)
                        .await?;
                } else {
                    self.system.start(ingress, failure, format).await?;
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                if target.application_id().is_some() {
                    return Err(SystemAudioCaptureError::ApplicationUnsupported);
                } else {
                    self.system.start(ingress, failure, format).await?;
                }
            }
            crate::core::audio_input::AudioSource::Microphone => {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                self.microphone.start(ingress, failure, format).await?;
                #[cfg(target_os = "linux")]
                self.system
                    .start_input(ingress, failure, format, input)
                    .await?;
                #[cfg(not(any(
                    target_os = "macos",
                    target_os = "windows",
                    target_os = "linux"
                )))]
                return Err(SystemAudioCaptureError::UnsupportedPlatform);
            }
        }
        if self.state.lock().unwrap().active != Some(token) {
            return Err(SystemAudioCaptureError::StartCancelled);
        }
        reservation.armed = false;
        Ok(())
    }

    async fn stop_backends(&self) {
        self.system.stop().await;
        #[cfg(target_os = "windows")]
        self.application.stop().await;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        self.microphone.stop().await;
    }

    pub async fn stop(&self) {
        {
            let mut state = self.state.lock().unwrap();
            state.active = None;
            state.stopping += 1;
        }
        struct FinishStop(Arc<std::sync::Mutex<SelectionState>>);
        impl Drop for FinishStop {
            fn drop(&mut self) {
                self.0.lock().unwrap().stopping -= 1;
            }
        }
        let _finish = FinishStop(Arc::clone(&self.state));
        loop {
            self.stop_backends().await;
            if self.state.lock().unwrap().starting == 0 {
                break;
            }
            // A start may have crossed its source-reservation check when
            // stop invalidated it. Keep cancelling until that future exits.
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    /// An explicit source switch-off must await native release, including
    /// ScreenCaptureKit's asynchronous completion after stop acknowledgement.
    pub async fn stop_and_wait(&self) -> Result<(), SystemAudioCaptureError> {
        self.stop().await;
        #[cfg(target_os = "macos")]
        self.system.wait_until_idle().await?;
        Ok(())
    }

    pub async fn audio_applications(
        &self,
    ) -> Result<applications::ApplicationSnapshot, SystemAudioCaptureError> {
        #[cfg(target_os = "macos")]
        {
            self.system.audio_applications().await
        }
        #[cfg(target_os = "windows")]
        {
            tokio::task::spawn_blocking(applications::windows_applications)
                .await
                .map_err(|_| SystemAudioCaptureError::ApplicationListFailed)?
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Ok(applications::ApplicationSnapshot {
                supported: false,
                applications: Vec::new(),
            })
        }
    }

    pub fn microphone_device_name(&self) -> Option<String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            self.microphone.device_name()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            None
        }
    }

    #[cfg(target_os = "windows")]
    pub fn set_source(&self, source: String) {
        self.system.set_source(source);
    }

    #[cfg(target_os = "windows")]
    pub fn snapshot(&self) -> Result<AudioSourceSnapshot, String> {
        self.system.snapshot()
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;

    fn capture_fixture(
        source: crate::core::audio_input::AudioSource,
        observation: Option<CaptureSignal>,
    ) -> CaptureDetails {
        use crate::core::audio_input::AudioSource;
        CaptureDetails {
            kind: if source == AudioSource::System {
                "macos_system_mix"
            } else {
                "microphone"
            },
            strategy: if source == AudioSource::System {
                "platform_capture"
            } else {
                "default_input"
            },
            actual_device_name: (source == AudioSource::Microphone)
                .then(|| "Synthetic microphone".into()),
            system_output_device_name: (source == AudioSource::System)
                .then(|| "Synthetic speaker".into()),
            observation,
        }
    }

    #[test]
    fn explicit_capture_stops_never_enter_automatic_recovery() {
        for failure in [
            SystemAudioCaptureFailure::PermissionDenied,
            SystemAudioCaptureFailure::UserStopped,
            SystemAudioCaptureFailure::ApplicationUnavailable,
        ] {
            assert!(!failure.is_recoverable());
        }
        for failure in [
            SystemAudioCaptureFailure::NativeStopped,
            SystemAudioCaptureFailure::AudioProcessingFailed,
            SystemAudioCaptureFailure::Backpressure,
        ] {
            assert!(failure.is_recoverable());
        }
    }

    #[test]
    fn capture_status_keeps_each_selected_source_and_legacy_aggregate_fields() {
        use crate::core::audio_input::{AudioInput, AudioSource};
        let status = CaptureStatus::for_input(AudioInput::Both, |source| {
            capture_fixture(
                source,
                Some(CaptureSignal {
                    pcm_data_recent: source == AudioSource::System,
                    sound_recent: source == AudioSource::System,
                }),
            )
        });
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["kind"], "both");
        assert_eq!(value["strategy"], "independent_inputs");
        assert_eq!(value["observation"]["soundRecent"], true);
        assert_eq!(value["sources"][0]["audioSource"], "system");
        assert_eq!(
            value["sources"][0]["systemOutputDeviceName"],
            "Synthetic speaker"
        );
        assert_eq!(value["sources"][0]["observation"]["soundRecent"], true);
        assert_eq!(value["sources"][1]["audioSource"], "microphone");
        assert_eq!(
            value["sources"][1]["actualDeviceName"],
            "Synthetic microphone"
        );
        assert_eq!(value["sources"][1]["observation"]["pcmDataRecent"], false);
        assert_eq!(value["sources"][1]["observation"]["soundRecent"], false);
        assert!(value["sources"][1]["systemOutputDeviceName"].is_null());
        assert!(value.get("details").is_none());
        assert!(value["sources"][0].get("details").is_none());
    }

    #[test]
    fn capture_status_does_not_read_or_include_an_unselected_source() {
        use crate::core::audio_input::{AudioInput, AudioSource};
        for (input, selected) in [
            (AudioInput::System, AudioSource::System),
            (AudioInput::Microphone, AudioSource::Microphone),
        ] {
            let mut requested = Vec::new();
            let status = CaptureStatus::for_input(input, |source| {
                requested.push(source);
                capture_fixture(source, None)
            });
            assert_eq!(requested, vec![selected]);
            assert_eq!(status.sources.len(), 1);
            assert_eq!(status.sources[0].audio_source, selected);
            assert_eq!(status.details, status.sources[0].details);
            assert_eq!(status.details.observation, None);
        }
    }

    #[test]
    fn provider_sample_rates_are_supported() {
        assert_eq!(
            AudioCaptureFormat::pcm16_mono(16_000)
                .unwrap()
                .sample_rate_hz,
            16_000
        );
        assert_eq!(
            AudioCaptureFormat::pcm16_mono(24_000)
                .unwrap()
                .sample_rate_hz,
            24_000
        );
        assert!(AudioCaptureFormat::pcm16_mono(48_000).is_err());
    }

    #[tokio::test]
    async fn capture_failure_sender_is_bounded_and_reports_once() {
        let (sender, mut receiver) = CaptureFailureSender::channel();

        assert!(sender.report(SystemAudioCaptureFailure::NativeStopped));
        assert!(!sender.report(SystemAudioCaptureFailure::Backpressure));
        assert!(sender.has_reported());
        assert_eq!(
            receiver.recv().await,
            Some(SystemAudioCaptureFailure::NativeStopped)
        );
        assert!(receiver.try_recv().is_err());
    }
}
