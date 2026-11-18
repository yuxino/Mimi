//! Credential-free native UI fixtures. These paths never start capture or provider connections.

use super::publication::{SessionStateEvent, StatusPayload};
use super::SessionManager;
use crate::core::audio_input::{AudioInput, AudioSource};
use crate::core::models::UtteranceRole;
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::session::TranslationSessionController;
use crate::pipeline_log;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// UI-only buffers contain synthetic fixtures. An explicit export fixture keeps
/// them in memory after stop so QA can reach the real picker and write path.
/// This helper is never used by production history finalization.
pub(super) fn finish_ui_test_archive(
    controller: &mut TranslationSessionController,
    recording: &mut crate::core::session_archive::AudioRecording,
    retain_for_export: bool,
) {
    if !retain_for_export {
        controller.archive_mut().clear();
        recording.clear();
    }
}

pub(super) fn ui_test_audio_samples(source: AudioSource, retain_for_export: bool) -> Vec<u8> {
    let frequency = match source {
        AudioSource::System => 440.0,
        AudioSource::Microphone => 613.0,
    };
    let sample_count = 16_000 * if retain_for_export { 8 } else { 1 };
    (0..sample_count)
        .flat_map(|index| {
            let phase = index as f32 * std::f32::consts::TAU * frequency / 16_000.0;
            ((phase.sin() * 1_000.0) as i16).to_le_bytes()
        })
        .collect()
}

/// Only the credential-free UI-test start path calls this synthetic fixture.
/// Preserve the selected sources and publish both raw and complete paired
/// previews so native geometry QA works for independent and atomic routes.
pub(super) fn seed_ui_test_live_subtitles(
    controller: &mut TranslationSessionController,
    audio_input: AudioInput,
) {
    for &audio_source in audio_input.sources() {
        let (source_id, source, translation) = match audio_source {
            AudioSource::System => (10_001, "System live test", "系统字幕测试"),
            AudioSource::Microphone => (10_002, "Microphone live test", "麦克风字幕测试"),
        };
        controller.handle_from(
            audio_source,
            LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: source_id,
                text: source.into(),
                language: Some("en".into()),
            },
        );
        // Use the actual reducer stamp, including its current display epoch.
        let Some(owner) = controller
            .state
            .subtitles
            .tracks
            .iter()
            .find(|track| track.audio_source == audio_source)
            .and_then(|track| track.source.utterance_id.clone())
        else {
            continue;
        };
        controller.handle_from(
            audio_source,
            LiveTranslateServerEvent::UtteranceText {
                utterance_id: owner,
                role: UtteranceRole::Translation,
                text: translation.into(),
                is_final: false,
                language: None,
            },
        );
        controller.handle_from(
            audio_source,
            LiveTranslateServerEvent::SubtitlePreviewPair {
                source_utterance_id: Some(source_id),
                source: source.into(),
                language: Some("en".into()),
                translation: translation.into(),
            },
        );
    }
}

impl SessionManager {
    pub(super) fn write_ui_test_session_state(&self, event: &SessionStateEvent) {
        if !self.is_ui_test() {
            return;
        }
        let Some(path) = std::env::var_os("MIMI_UI_TEST_SESSION_STATE_FILE") else {
            return;
        };
        let state = match &event.status {
            StatusPayload::Idle => "idle",
            StatusPayload::Connecting => "connecting",
            StatusPayload::Listening if event.is_paused => "paused",
            StatusPayload::Listening => "listening",
            StatusPayload::Stopping => "stopping",
            StatusPayload::Error { .. } => "error",
        };
        if let Err(error) = std::fs::write(path, state) {
            tracing::warn!(error = %error, "could not write UI-test session state marker");
        }
    }

    pub(super) fn establish_ui_test_session(self: &Arc<Self>) -> Result<(), String> {
        self.cancel_translation_timeout();
        self.clear_active_settings();
        let generation = self.active_generation.load(Ordering::SeqCst);
        let attempt = self.next_background_task_id();
        {
            let _transition = self.generation_transition.lock().unwrap();
            self.ensure_generation_current(generation)?;
            *self.audio_attempt.lock().unwrap() = Some((generation, attempt));
            self.history_stats
                .lock()
                .unwrap()
                .audio_evidence
                .begin_attempt(generation, attempt);
        }
        self.controller.lock().unwrap().begin_connecting();
        self.publish_state();
        #[cfg(all(target_os = "macos", feature = "development-debugger"))]
        if std::env::var("MIMI_UI_TEST_AUDIO_PERMISSION_DENIED").as_deref() == Ok("1") {
            // Only this credential-free UI-test path may synthesize an error.
            // No actual grant, capture or service configuration is touched.
            let generation = self.active_generation.load(Ordering::SeqCst);
            if self.invalidate_generation(generation) {
                self.controller
                    .lock()
                    .unwrap()
                    .did_fail(crate::audio::SystemAudioCaptureError::PermissionDenied.to_string());
                self.publish_state();
            }
            return Err(crate::audio::SystemAudioCaptureError::PermissionDenied.to_string());
        }
        self.controller.lock().unwrap().did_connect();
        // Exercise the real archive and native save dialog in credential-free
        // UI QA. These are explicitly synthetic samples, never captured audio.
        let preferences = self.settings.preferences();
        let dual_fixture = std::env::var("MIMI_UI_TEST_DUAL_SUBTITLES").as_deref() == Ok("1");
        let dual_live_fixture =
            std::env::var("MIMI_UI_TEST_DUAL_LIVE_SUBTITLES").as_deref() == Ok("1");
        self.controller
            .lock()
            .unwrap()
            .set_audio_input(preferences.audio_input);
        if preferences.retain_session_history || dual_fixture {
            for &source in preferences.audio_input.sources() {
                let (original, translation) = match source {
                    AudioSource::System => (
                        "Mimi system audio test: can you see the shared screen?",
                        "Mimi 系统声音测试：你能看到共享屏幕吗？",
                    ),
                    AudioSource::Microphone => (
                        "Mimi microphone test: yes, I can see it.",
                        "Mimi 麦克风测试：可以，我能看到。",
                    ),
                };
                self.controller.lock().unwrap().handle_from(
                    source,
                    LiveTranslateServerEvent::SubtitleFinalPair {
                        source: original.into(),
                        language: Some("en".into()),
                        translation: translation.into(),
                    },
                );
            }
        }
        if dual_live_fixture {
            seed_ui_test_live_subtitles(
                &mut self.controller.lock().unwrap(),
                preferences.audio_input,
            );
        }
        if preferences.record_session_audio && !dual_live_fixture {
            let retain_for_export = std::env::var("MIMI_UI_TEST_EXPORT").as_deref() == Ok("1");
            for &source in preferences.audio_input.sources() {
                let slot = match source {
                    AudioSource::System => 0,
                    AudioSource::Microphone => 1,
                };
                let samples = ui_test_audio_samples(source, retain_for_export);
                let mut recordings = self.recording.lock().unwrap();
                recordings[slot].begin(true);
                recordings[slot].append(16_000, &samples);
                if !recordings[slot].is_empty() {
                    self.history_stats
                        .lock()
                        .unwrap()
                        .audio_evidence
                        .record(generation, attempt, source, true);
                }
            }
        }
        self.publish_state();
        pipeline_log!("ui-test synthetic session listening");
        Ok(())
    }
}
