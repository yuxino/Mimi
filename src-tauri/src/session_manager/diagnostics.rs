//! Content-free observations and support snapshots for the current native session.

use super::publication::visible_session_latencies;
use super::SessionManager;
use crate::audio::send_pipeline::AudioSendPipeline;
use crate::core::audio_input::{AudioInput, AudioSource};
use crate::core::diagnostics::milliseconds;
use crate::core::models::{SessionStatus, UtteranceRole};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::provider::ProviderKind;
use crate::core::support_diagnostics::{
    Availability, CaptureObservation, DiagnosticEvent, DiagnosticFacts, DiagnosticStatus,
    OutputSelection, RecoveryAction, SafeFailure, TextEventKind,
};
use mimi_runtime::core::recovery::NO_GENERATION;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Capture presentation never borrows a retired generation's recent activity.
/// Unlike diagnostics, this represents currently selected live input only.
pub(super) fn source_capture_observation(
    pipeline_slot: &Mutex<Option<Arc<AudioSendPipeline>>>,
    pipeline_generation: &AtomicU64,
    active_generation: u64,
) -> Option<crate::audio::CaptureSignal> {
    let pipeline = pipeline_slot.lock().unwrap();
    if active_generation == NO_GENERATION
        || pipeline_generation.load(Ordering::SeqCst) != active_generation
    {
        return None;
    }
    pipeline.as_ref().map(|pipeline| {
        let (pcm_data_recent, sound_recent) = pipeline.input_activity();
        crate::audio::CaptureSignal {
            pcm_data_recent,
            sound_recent,
        }
    })
}

impl SessionManager {
    pub(super) fn record_diagnostic_failure(&self, error: &str) {
        self.record_safe_failure(SafeFailure::from_error(error));
    }

    pub(super) fn record_safe_failure(&self, classification: SafeFailure) {
        self.record_source_failure(None, classification);
    }

    pub(super) fn record_source_failure(
        &self,
        audio_source: Option<AudioSource>,
        classification: SafeFailure,
    ) {
        *self.diagnostic_error.lock().unwrap() = Some((classification, Instant::now()));
        self.record_diagnostic_event(DiagnosticEvent::Failure {
            classification,
            audio_source,
        });
    }

    pub(super) fn record_diagnostic_event(&self, event: DiagnosticEvent) {
        self.diagnostic_journal
            .lock()
            .unwrap()
            .record(event, milliseconds(self.diagnostic_epoch, Instant::now()));
    }

    /// Recheck event admission under the generation gate before recording.
    /// No journal or generation lock crosses an await.
    pub(super) fn record_accepted_provider_event(
        &self,
        source: AudioSource,
        generation: u64,
        event: &LiveTranslateServerEvent,
    ) -> bool {
        let _transition = self.generation_transition.lock().unwrap();
        if !self.accepts_event(generation, event) || self.is_paused() {
            return false;
        }
        if let LiveTranslateServerEvent::Error { code, message } = event {
            self.record_source_failure(
                Some(source),
                SafeFailure::from_provider_error(code, message),
            );
        }
        let mut journal = self.diagnostic_journal.lock().unwrap();
        let text_kind = match event {
            LiveTranslateServerEvent::SourceDraft { .. }
            | LiveTranslateServerEvent::SourceUtteranceDraft { .. } => {
                Some(TextEventKind::SourceDraft)
            }
            LiveTranslateServerEvent::SourceFinal { .. }
            | LiveTranslateServerEvent::SourceUtteranceFinal { .. } => {
                Some(TextEventKind::SourceFinal)
            }
            LiveTranslateServerEvent::TranslationDraft(_) => Some(TextEventKind::TranslationDraft),
            LiveTranslateServerEvent::TranslationFinal(_) => Some(TextEventKind::TranslationFinal),
            LiveTranslateServerEvent::SubtitleFinalPair { .. }
            | LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
            | LiveTranslateServerEvent::SubtitleConfirmedPair { .. } => {
                Some(TextEventKind::ConfirmedPair)
            }
            LiveTranslateServerEvent::UtteranceText { role, is_final, .. } => {
                Some(match (role, is_final) {
                    (UtteranceRole::Source, false) => TextEventKind::SourceDraft,
                    (UtteranceRole::Source, true) => TextEventKind::SourceFinal,
                    (UtteranceRole::Translation, false) => TextEventKind::TranslationDraft,
                    (UtteranceRole::Translation, true) => TextEventKind::TranslationFinal,
                })
            }
            _ => None,
        };
        if let Some(kind) = text_kind {
            journal.observe_text(kind);
        }
        if let LiveTranslateServerEvent::TranslationDeferred(recovery) = event {
            journal.record(
                DiagnosticEvent::TranslationBackoff {
                    recovery: *recovery,
                },
                milliseconds(self.diagnostic_epoch, Instant::now()),
            );
        }
        true
    }

    pub(super) fn record_recovery_action(&self, action: RecoveryAction) {
        *self.diagnostic_recovery.lock().unwrap() = Some((action, Instant::now()));
        self.record_diagnostic_event(DiagnosticEvent::Recovery { action });
    }

    pub(super) fn record_recovery_if_current(&self, action: RecoveryAction, epoch: u64) {
        let _transition = self.generation_transition.lock().unwrap();
        if self.is_lifecycle_request_current(epoch) {
            self.record_recovery_action(action);
        }
    }

    pub(super) fn current_capture_observation(&self) -> Option<CaptureObservation> {
        self.lanes
            .iter()
            .filter_map(|lane| {
                lane.audio_pipeline
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|pipeline| pipeline.input_activity())
            })
            .fold(None, |previous, (pcm_data_recent, sound_recent)| {
                let mut observation = previous.unwrap_or(CaptureObservation {
                    pcm_data_recent: false,
                    sound_recent: false,
                });
                observation.pcm_data_recent |= pcm_data_recent;
                observation.sound_recent |= sound_recent;
                Some(observation)
            })
    }

    pub async fn audio_applications(
        &self,
        cached_only: bool,
    ) -> Result<
        crate::audio::applications::ApplicationSnapshot,
        crate::audio::SystemAudioCaptureError,
    > {
        let capture = self.lane(AudioSource::System).audio.lock().unwrap().clone();
        capture
            .audio_applications(
                self.settings
                    .preferences()
                    .ui_language
                    .unwrap_or_else(|| "system".into()),
                cached_only,
            )
            .await
    }

    pub fn capture_status(&self) -> crate::audio::CaptureStatus {
        let preferences = self.settings.preferences();
        let generation = self.active_generation.load(Ordering::SeqCst);
        crate::audio::CaptureStatus::for_input(preferences.audio_input, |source| {
            let lane = self.lane(source);
            let observation = source_capture_observation(
                &lane.audio_pipeline,
                &lane.audio_pipeline_generation,
                generation,
            );
            if source == AudioSource::System {
                if let Some(name) = preferences.system_audio_target.application_name() {
                    return crate::audio::CaptureDetails {
                        kind: "application",
                        strategy: "selected_application",
                        actual_device_name: Some(name.to_string()),
                        system_output_device_name: None,
                        observation,
                    };
                }
            }
            self.capture_details(source, observation, &preferences.windows_audio_source)
        })
    }

    pub(super) fn capture_details(
        &self,
        source: AudioSource,
        observation: Option<crate::audio::CaptureSignal>,
        windows_audio_source: &str,
    ) -> crate::audio::CaptureDetails {
        if source == AudioSource::Microphone {
            return crate::audio::CaptureDetails {
                kind: "microphone",
                strategy: "default_input",
                actual_device_name: self
                    .lane(source)
                    .audio
                    .lock()
                    .unwrap()
                    .microphone_device_name(),
                system_output_device_name: None,
                observation,
            };
        }
        #[cfg(target_os = "windows")]
        {
            let snapshot = self.windows_audio_status().ok().flatten();
            let actual_device_name = snapshot.and_then(|snapshot| {
                snapshot.current_device.and_then(|id| {
                    snapshot
                        .devices
                        .into_iter()
                        .find(|device| device.id == id)
                        .map(|device| device.name)
                })
            });
            crate::audio::CaptureDetails {
                kind: "windows_output",
                strategy: if windows_audio_source.is_empty() {
                    "follow_system"
                } else {
                    "manual_output"
                },
                actual_device_name,
                system_output_device_name: None,
                observation,
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = windows_audio_source;
            crate::audio::CaptureDetails {
                kind: if cfg!(target_os = "macos") {
                    "macos_system_mix"
                } else if cfg!(target_os = "linux") {
                    "linux_output_monitor"
                } else {
                    "unknown"
                },
                strategy: "platform_capture",
                actual_device_name: None,
                system_output_device_name: {
                    #[cfg(target_os = "macos")]
                    {
                        crate::audio::macos_output::default_output_device_name()
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        None
                    }
                },
                observation,
            }
        }
    }

    pub fn support_diagnostics(&self) -> String {
        crate::core::support_diagnostics::render(self.diagnostic_facts())
    }

    pub fn support_issue(&self) -> crate::core::support_diagnostics::SupportIssue {
        crate::core::support_diagnostics::issue_link(self.diagnostic_facts())
    }

    pub(super) fn diagnostic_facts(&self) -> DiagnosticFacts {
        let now = Instant::now();
        let prefs = self.settings.preferences();
        let profile = self.settings.active_profile().ok();
        let provider = profile.as_ref().map(|profile| profile.effective_provider());
        let alibaba_realtime_mode = profile.as_ref().and_then(|profile| {
            (profile.effective_provider() == ProviderKind::AlibabaCloud)
                .then_some(profile.alibaba_realtime_mode)
        });
        let (provider, mode, alibaba_realtime_mode) = self
            .active_settings
            .lock()
            .unwrap()
            .as_ref()
            .map(|config| {
                (
                    Some(config.provider),
                    config.effective_translation_mode(),
                    (config.provider == ProviderKind::AlibabaCloud)
                        .then_some(config.alibaba_realtime_mode),
                )
            })
            .unwrap_or((provider, prefs.translation_mode, alibaba_realtime_mode));
        let generation = self.active_generation.load(Ordering::SeqCst);
        let (
            status,
            translation_pending,
            translation_preview_pending,
            translation_timed_out,
            session_status,
            translation_recovery,
        ) = {
            let controller = self.controller.lock().unwrap();
            let status = match controller.state.status {
                SessionStatus::Idle => DiagnosticStatus::Idle,
                SessionStatus::Connecting => DiagnosticStatus::Connecting,
                SessionStatus::Stopping => DiagnosticStatus::Stopping,
                SessionStatus::Listening if self.is_paused() => DiagnosticStatus::Paused,
                SessionStatus::Listening => DiagnosticStatus::Listening,
                SessionStatus::Error(_) => DiagnosticStatus::Error,
            };
            (
                status,
                controller.state.is_translation_pending,
                controller.state.is_translation_preview_pending,
                controller.state.is_translation_timed_out,
                controller.state.status.clone(),
                controller.state.translation_recovery,
            )
        };
        let capture = self
            .current_capture_observation()
            .map(|capture| (capture, 0))
            .or_else(|| {
                self.diagnostic_capture
                    .lock()
                    .unwrap()
                    .map(|(capture, at)| (capture, crate::core::diagnostics::milliseconds(at, now)))
            });
        #[cfg(target_os = "windows")]
        let (output_selection, output_availability) = {
            let selected = prefs.windows_audio_source;
            let availability = self
                .windows_audio_status()
                .ok()
                .flatten()
                .map(|snapshot| {
                    if selected.is_empty() {
                        // An active bound endpoint is known; before start, the
                        // default role cannot be inferred from the device list.
                        if snapshot.current_device.is_some() {
                            Availability::Available
                        } else {
                            Availability::Unknown
                        }
                    } else if snapshot.devices.iter().any(|device| device.id == selected) {
                        Availability::Available
                    } else {
                        Availability::Unavailable
                    }
                })
                .unwrap_or(Availability::Unknown);
            (
                if selected.is_empty() {
                    OutputSelection::SystemDefault
                } else {
                    OutputSelection::ManualOutput
                },
                availability,
            )
        };
        #[cfg(not(target_os = "windows"))]
        let (output_selection, output_availability) =
            (OutputSelection::PlatformSystemAudio, Availability::Unknown);
        let (output_selection, output_availability) = if prefs.audio_input == AudioInput::Microphone
        {
            (OutputSelection::DefaultMicrophone, Availability::Unknown)
        } else if prefs.audio_input == AudioInput::Both {
            (OutputSelection::SystemAndMicrophone, Availability::Unknown)
        } else if prefs.system_audio_target.application_id().is_some() {
            (OutputSelection::SelectedApplication, Availability::Unknown)
        } else {
            (output_selection, output_availability)
        };
        let translation_latency = self
            .sources()
            .iter()
            .filter_map(|&source| {
                self.client_for_generation(source, generation)
                    .and_then(|client| client.translation_latency())
            })
            .max_by_key(|latency| latency.milliseconds);
        let (mut api_latency_ms, mut translation_latency_ms, mut translation_latency_kind) =
            visible_session_latencies(
                &session_status,
                self.is_paused(),
                self.is_recovering.load(Ordering::SeqCst),
                generation,
                self.health_task_id.load(Ordering::SeqCst),
                *self.health_latency.lock().unwrap(),
                translation_latency,
            );
        let translation_recovery = if self.is_generation_current(generation) {
            translation_recovery
        } else {
            api_latency_ms = None;
            translation_latency_ms = None;
            translation_latency_kind = None;
            None
        };
        let journal = self.diagnostic_journal.lock().unwrap().snapshot();
        DiagnosticFacts {
            provider,
            mode,
            alibaba_realtime_mode,
            status,
            output_selection,
            output_availability,
            capture,
            translation_pending,
            translation_preview_pending,
            translation_timed_out,
            last_error: self
                .diagnostic_error
                .lock()
                .unwrap()
                .map(|(error, at)| (error, crate::core::diagnostics::milliseconds(at, now))),
            recovery: self
                .diagnostic_recovery
                .lock()
                .unwrap()
                .map(|(action, at)| (action, crate::core::diagnostics::milliseconds(at, now))),
            api_latency_ms,
            translation_latency_ms,
            translation_latency_kind,
            translation_recovery,
            journal,
            elapsed_ms: crate::core::diagnostics::milliseconds(
                self.diagnostic_epoch,
                Instant::now(),
            ),
        }
    }

    pub fn windows_audio_status(
        &self,
    ) -> Result<Option<crate::audio::AudioSourceSnapshot>, String> {
        if self.settings.preferences().audio_input == AudioInput::Microphone {
            return Ok(None);
        }
        #[cfg(target_os = "windows")]
        {
            let mut snapshot = self
                .lane(AudioSource::System)
                .audio
                .lock()
                .unwrap()
                .snapshot()?;
            // Sound reaching the provider is measured after mono/resampling,
            // independently of raw callback arrival or synthetic keepalive.
            let system = self.lane(AudioSource::System);
            snapshot.receiving_sound = snapshot.current_device.is_some()
                && source_capture_observation(
                    &system.audio_pipeline,
                    &system.audio_pipeline_generation,
                    self.active_generation.load(Ordering::SeqCst),
                )
                .is_some_and(|capture| capture.sound_recent);
            Ok(Some(snapshot))
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok(None)
        }
    }

    pub(super) fn observe_diagnostic_status(&self) {
        let status = {
            let controller = self.controller.lock().unwrap();
            match controller.state.status {
                SessionStatus::Idle => DiagnosticStatus::Idle,
                SessionStatus::Connecting => DiagnosticStatus::Connecting,
                SessionStatus::Stopping => DiagnosticStatus::Stopping,
                SessionStatus::Listening if self.is_paused() => DiagnosticStatus::Paused,
                SessionStatus::Listening => DiagnosticStatus::Listening,
                SessionStatus::Error(_) => DiagnosticStatus::Error,
            }
        };
        self.diagnostic_journal
            .lock()
            .unwrap()
            .observe_status(status, milliseconds(self.diagnostic_epoch, Instant::now()));
    }
}
