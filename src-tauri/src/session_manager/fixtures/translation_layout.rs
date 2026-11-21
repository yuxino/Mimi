//! Fixed pending-only transitions followed by one synthetic translation.

use super::super::ownership::SESSION_START_CANCELLED;
use super::{
    AudioInput, AudioSource, LiveTranslateServerEvent, SessionManager, SessionStateEvent,
    StatusPayload, TranslationSessionController, UtteranceRole,
};
use serde::Serialize;
use std::{io::Write, path::PathBuf, sync::Arc, time::Duration};

const INITIAL_CAPTIONS: usize = 6;
const DRAFT_ID: u64 = 10_001;
const DRAFT_SOURCE: &str = "The next boat leaves at noon.";
const DRAFT_TRANSLATION: &str = "下一班船在中午出发。";
const STATE_FILE: &str = "translation-layout-state.json";
const REQUESTS: [&str; 5] = [
    "translation-layout-step-1",
    "translation-layout-step-2",
    "translation-layout-step-3",
    "translation-layout-step-4",
    "translation-layout-step-5",
];
const FIXTURE_FAILED: &str = "ui_test_translation_layout_fixture_failed";

pub(super) fn requested_directory() -> Result<Option<PathBuf>, String> {
    if std::env::var("MIMI_UI_TEST_TRANSLATION_LAYOUT").as_deref() != Ok("1") {
        return Ok(None);
    }
    // This entry is called only after the credential-free UI-test start gate.
    // Startup validates this same directory with load_ui_test_preferences:
    // private, not a symlink, and named mimi-ui-test- beneath the temp root.
    std::env::var_os("MIMI_UI_TEST_PREFERENCES_DIR")
        .map(PathBuf::from)
        .map(Some)
        .ok_or_else(|| FIXTURE_FAILED.into())
}

struct TranslationLayout {
    directory: PathBuf,
    step: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FixtureState {
    step: usize,
    pending: bool,
    phase: &'static str,
    history_count: usize,
    has_translation: bool,
    confirmed: bool,
}

impl TranslationLayout {
    fn prepare(directory: PathBuf) -> Result<Self, String> {
        for name in REQUESTS.into_iter().chain([STATE_FILE]) {
            match std::fs::remove_file(directory.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(FIXTURE_FAILED.into()),
            }
        }
        Ok(Self { directory, step: 0 })
    }

    fn next_is_marked(&self) -> Result<bool, String> {
        let Some(name) = REQUESTS.get(self.step) else {
            return Ok(false);
        };
        match std::fs::symlink_metadata(self.directory.join(name)) {
            Ok(metadata) if metadata.is_file() && metadata.len() == 0 => Ok(true),
            Ok(_) => Err(FIXTURE_FAILED.into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(FIXTURE_FAILED.into()),
        }
    }

    fn record_state(&self, state: &FixtureState) -> Result<(), String> {
        // Atomic JSON acceptance marker only. Native tests must independently
        // wait for WebKit pixels/AX bounds, not treat this as a paint barrier.
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)
            .map_err(|_| FIXTURE_FAILED.to_string())?;
        serde_json::to_writer(&mut temporary, state).map_err(|_| FIXTURE_FAILED.to_string())?;
        temporary.flush().map_err(|_| FIXTURE_FAILED.to_string())?;
        temporary
            .persist(self.directory.join(STATE_FILE))
            .map_err(|_| FIXTURE_FAILED.to_string())?;
        Ok(())
    }

    fn state(&self, controller: &TranslationSessionController) -> FixtureState {
        let event = SessionStateEvent::from(&controller.state);
        FixtureState {
            step: self.step,
            pending: event.is_translation_pending,
            phase: match event.status {
                StatusPayload::Idle => "idle",
                StatusPayload::Connecting => "connecting",
                StatusPayload::Listening => "listening",
                StatusPayload::Stopping => "stopping",
                StatusPayload::Error { .. } => "error",
            },
            history_count: event.subtitles.history.len(),
            // Realtime snapshots can retain the preceding final in the raw
            // lane. This field describes the new, fixed utterance only.
            has_translation: event.subtitles.translation.text == DRAFT_TRANSLATION,
            confirmed: event.subtitles.history.last().is_some_and(|pair| {
                pair.source == DRAFT_SOURCE && pair.translation == DRAFT_TRANSLATION
            }),
        }
    }

    fn advance(&mut self, controller: &mut TranslationSessionController) {
        self.step += 1;
        match self.step {
            1 | 3 => controller.handle_from(
                AudioSource::System,
                LiveTranslateServerEvent::TranslationStarted,
            ),
            2 | 4 => {
                // Reuse the controller's existing transient-work cleanup. It
                // retains text/status and does not change SessionManager's
                // paused flag or issue any native pause/capture operation.
                controller.did_pause();
                if self.step == 4 {
                    let owner = controller
                        .state
                        .subtitles
                        .source
                        .utterance_id
                        .clone()
                        .expect("the fixed source draft owns this fixture");
                    controller.handle_from(
                        AudioSource::System,
                        LiveTranslateServerEvent::UtteranceText {
                            utterance_id: owner,
                            role: UtteranceRole::Translation,
                            text: DRAFT_TRANSLATION.into(),
                            is_final: false,
                            language: None,
                        },
                    );
                }
            }
            5 => controller.handle_from(
                AudioSource::System,
                LiveTranslateServerEvent::SubtitleConfirmedPair {
                    utterance_id: DRAFT_ID,
                    source_utterance_id: Some(DRAFT_ID),
                    source: DRAFT_SOURCE.into(),
                    language: Some("en".into()),
                    translation: DRAFT_TRANSLATION.into(),
                },
            ),
            _ => unreachable!("the fixed request list bounds the scene"),
        }
    }
}

fn seed(controller: &mut TranslationSessionController) {
    controller.clear_subtitles();
    controller.set_audio_input(AudioInput::System);
    controller.set_atomic_preview(false);
    for index in 1..=INITIAL_CAPTIONS {
        controller.handle_from(
            AudioSource::System,
            LiveTranslateServerEvent::SubtitleConfirmedPair {
                utterance_id: index as u64,
                source_utterance_id: Some(index as u64),
                source: format!("History {index:02} rests beside the quiet river."),
                language: Some("en".into()),
                translation: format!("旧句 {index:02} 静静地停在河边。"),
            },
        );
    }
    controller.handle_from(
        AudioSource::System,
        LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: DRAFT_ID,
            text: DRAFT_SOURCE.into(),
            language: Some("en".into()),
        },
    );
}

impl SessionManager {
    pub(super) fn start_ui_test_translation_layout(
        self: &Arc<Self>,
        directory: PathBuf,
        generation: u64,
        attempt: u64,
    ) -> Result<(), String> {
        let mut scene = {
            let _transition = self.generation_transition.lock().unwrap();
            self.ensure_generation_current(generation)?;
            if *self.audio_attempt.lock().unwrap() != Some((generation, attempt)) {
                return Err(SESSION_START_CANCELLED.into());
            }
            let scene = TranslationLayout::prepare(directory)?;
            seed(&mut self.controller.lock().unwrap());
            scene
        };
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let drive = async {
                loop {
                    manager.publish_lifecycle_state().await;
                    {
                        let _transition = manager.generation_transition.lock().unwrap();
                        if !manager.is_generation_current(generation)
                            || *manager.audio_attempt.lock().unwrap() != Some((generation, attempt))
                            || manager.is_paused()
                        {
                            return Ok::<(), String>(());
                        }
                        let controller = manager.controller.lock().unwrap();
                        scene.record_state(&scene.state(&controller))?;
                    }
                    if scene.step == REQUESTS.len() {
                        return Ok(());
                    }
                    loop {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        let _transition = manager.generation_transition.lock().unwrap();
                        if !manager.is_generation_current(generation)
                            || *manager.audio_attempt.lock().unwrap() != Some((generation, attempt))
                        {
                            return Ok(());
                        }
                        if !manager.is_paused() && scene.next_is_marked()? {
                            scene.advance(&mut manager.controller.lock().unwrap());
                            break;
                        }
                    }
                }
            };
            let result = manager
                .run_while_generation_current(
                    generation,
                    tokio::time::timeout(Duration::from_secs(120), drive),
                )
                .await;
            match result {
                Ok(Ok(Err(_))) => tracing::warn!("translation layout fixture marker failed"),
                Ok(Err(_)) => tracing::warn!("translation layout fixture timed out"),
                _ => {}
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::SessionStatus;

    #[test]
    fn pending_only_steps_preserve_all_captions_then_translate_and_confirm_the_same_source() {
        let directory = tempfile::tempdir().unwrap();
        let mut scene = TranslationLayout::prepare(directory.path().to_owned()).unwrap();
        let mut controller = TranslationSessionController::default();
        controller.did_connect();
        seed(&mut controller);
        let initial = controller.state.clone();
        assert_eq!(initial.subtitles.history.len(), INITIAL_CAPTIONS);
        assert_eq!(initial.subtitles.source.text, DRAFT_SOURCE);
        assert_eq!(
            initial.subtitles.translation.text,
            initial.subtitles.history.last().unwrap().translation
        );
        assert!(!scene.state(&controller).has_translation);
        let history = initial.subtitles.history.clone();
        let source = initial.subtitles.source.clone();
        for pending in [true, false, true] {
            scene.advance(&mut controller);
            let mut current = controller.state.clone();
            assert_eq!(current.is_translation_pending, pending);
            assert!(current
                .subtitles
                .tracks
                .iter()
                .all(|track| track.is_translation_pending == pending));
            current.is_translation_pending = false;
            for track in &mut current.subtitles.tracks {
                track.is_translation_pending = false;
            }
            assert_eq!(current, initial);
            let event = SessionStateEvent::from(&controller.state);
            assert!(!event.is_paused);
            assert!(matches!(event.status, StatusPayload::Listening));
        }
        scene.advance(&mut controller);
        assert!(!controller.state.is_translation_pending);
        assert_eq!(controller.state.subtitles.source, source);
        assert_eq!(
            controller.state.subtitles.translation.text,
            DRAFT_TRANSLATION
        );
        assert!(!controller.state.subtitles.translation.is_final);
        assert_eq!(
            controller.state.subtitles.source.utterance_id,
            controller.state.subtitles.translation.utterance_id
        );
        assert_eq!(controller.state.subtitles.history, history);
        scene.advance(&mut controller);
        assert_eq!(controller.state.status, SessionStatus::Listening);
        assert!(!controller.state.is_translation_pending);
        assert_eq!(
            controller.state.subtitles.history.len(),
            INITIAL_CAPTIONS + 1
        );
        assert_eq!(
            &controller.state.subtitles.history[..INITIAL_CAPTIONS],
            history.as_slice()
        );
        let final_pair = controller.state.subtitles.history.last().unwrap();
        assert_eq!(final_pair.source, DRAFT_SOURCE);
        assert_eq!(final_pair.translation, DRAFT_TRANSLATION);
        assert_eq!(final_pair.audio_source, AudioSource::System);
        let state = scene.state(&controller);
        assert_eq!(state.step, 5);
        assert!(state.has_translation);
        assert!(state.confirmed);
        assert_eq!(controller.archive().page("", 0).total, 0);
    }

    #[test]
    fn requests_are_empty_files_in_fixed_order_and_restarts_reset_them() {
        let directory = tempfile::tempdir().unwrap();
        let mut scene = TranslationLayout::prepare(directory.path().to_owned()).unwrap();
        std::fs::write(directory.path().join(REQUESTS[2]), "").unwrap();
        assert!(!scene.next_is_marked().unwrap());
        std::fs::write(directory.path().join(REQUESTS[0]), "not empty").unwrap();
        assert!(scene.next_is_marked().is_err());
        std::fs::write(directory.path().join(REQUESTS[0]), "").unwrap();
        assert!(scene.next_is_marked().unwrap());
        let mut controller = TranslationSessionController::default();
        controller.did_connect();
        seed(&mut controller);
        scene.advance(&mut controller);
        scene.record_state(&scene.state(&controller)).unwrap();
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.path().join(STATE_FILE)).unwrap())
                .unwrap();
        assert_eq!(state["step"], 1);
        assert_eq!(state["pending"], true);
        assert_eq!(state["phase"], "listening");
        assert_eq!(state["historyCount"], INITIAL_CAPTIONS);
        assert!(!scene.next_is_marked().unwrap());
        TranslationLayout::prepare(directory.path().to_owned()).unwrap();
        assert!(!directory.path().join(REQUESTS[0]).exists());
        assert!(!directory.path().join(REQUESTS[2]).exists());
        assert!(!directory.path().join(STATE_FILE).exists());
    }
}
