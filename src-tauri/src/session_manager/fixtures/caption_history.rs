//! A bounded, marker-driven history scene for native scrolling acceptance.

use super::super::ownership::SESSION_START_CANCELLED;
use super::{AudioSource, LiveTranslateServerEvent, SessionManager, TranslationSessionController};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const INITIAL_CAPTIONS: usize = 18;
const FINAL_CAPTION: usize = 20;
const COUNT_FILE: &str = "caption-history-count";
const FIXTURE_FAILED: &str = "ui_test_caption_history_fixture_failed";

pub(super) fn requested_directory() -> Result<Option<PathBuf>, String> {
    if std::env::var("MIMI_UI_TEST_CAPTION_HISTORY").as_deref() != Ok("1") {
        return Ok(None);
    }
    // Called only by the credential-free UI-test session path. Startup accepts
    // this variable only through load_ui_test_preferences,
    // which requires a private, non-symlinked mimi-ui-test- directory in temp.
    std::env::var_os("MIMI_UI_TEST_PREFERENCES_DIR")
        .map(PathBuf::from)
        .map(Some)
        .ok_or_else(|| FIXTURE_FAILED.into())
}

struct CaptionHistory {
    directory: PathBuf,
    next: usize,
}

impl CaptionHistory {
    fn prepare(directory: PathBuf) -> Result<Self, String> {
        // A stopped/restarted scene must not consume an earlier scene's input.
        for name in ["append-caption-19", "append-caption-20", COUNT_FILE] {
            match std::fs::remove_file(directory.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(FIXTURE_FAILED.into()),
            }
        }
        Ok(Self {
            directory,
            next: INITIAL_CAPTIONS + 1,
        })
    }

    fn next_is_marked(&self) -> Result<bool, String> {
        match std::fs::symlink_metadata(
            self.directory.join(format!("append-caption-{}", self.next)),
        ) {
            Ok(metadata) if metadata.is_file() => Ok(true),
            Ok(_) => Err(FIXTURE_FAILED.into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(FIXTURE_FAILED.into()),
        }
    }

    fn record_count(&self, count: usize) -> Result<(), String> {
        // Controller acceptance only; the native test must still observe UIA
        // bounds/paint before declaring the caption visible.
        std::fs::write(self.directory.join(COUNT_FILE), count.to_string())
            .map_err(|_| FIXTURE_FAILED.into())
    }

    fn append(&mut self, controller: &mut TranslationSessionController, source: AudioSource) {
        append_caption(controller, source, self.next);
        self.next += 1;
    }
}

fn append_caption(
    controller: &mut TranslationSessionController,
    source: AudioSource,
    index: usize,
) {
    controller.handle_from(source, LiveTranslateServerEvent::SubtitleFinalPair {
        source: format!("Original {index:02} follows the riverside path past the old station and reaches the bridge."),
        language: Some("en".into()),
        translation: format!("译文 {index:02} 沿着河边的小路，经过老车站，走到了桥边。"),
    });
}

impl SessionManager {
    pub(super) fn start_ui_test_caption_history(
        self: &Arc<Self>,
        directory: PathBuf,
        generation: u64,
        attempt: u64,
        source: AudioSource,
    ) -> Result<(), String> {
        let mut scene = {
            let _transition = self.generation_transition.lock().unwrap();
            self.ensure_generation_current(generation)?;
            if *self.audio_attempt.lock().unwrap() != Some((generation, attempt)) {
                return Err(SESSION_START_CANCELLED.into());
            }
            // Claim input before clearing markers: a superseded reconnect must
            // not erase the newest scene's commands or replace its first rows.
            let scene = CaptionHistory::prepare(directory)?;
            let mut controller = self.controller.lock().unwrap();
            controller.clear_subtitles();
            for index in 1..=INITIAL_CAPTIONS {
                append_caption(&mut controller, source, index);
            }
            scene.record_count(INITIAL_CAPTIONS)?;
            scene
        };
        self.publish_state();
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let append_marked_captions = async {
                while scene.next <= FINAL_CAPTION {
                    let marked = scene.next_is_marked()?;
                    let appended = {
                        // A configuration reconnect may replace its audio attempt
                        // within the same generation. Only its newest scene owns input.
                        let _transition = manager.generation_transition.lock().unwrap();
                        if !manager.is_generation_current(generation)
                            || *manager.audio_attempt.lock().unwrap() != Some((generation, attempt))
                        {
                            return Ok::<(), String>(());
                        }
                        if marked && !manager.is_paused() {
                            scene.append(&mut manager.controller.lock().unwrap(), source);
                            scene.record_count(scene.next - 1)?;
                            true
                        } else {
                            false
                        }
                    };
                    if appended {
                        manager.publish_state();
                    }
                    if scene.next <= FINAL_CAPTION {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                }
                Ok(())
            };
            // Existing lifecycle notification cancels immediately on Stop/new
            // generation; an idle test cannot leave a polling task indefinitely.
            let result = manager
                .run_while_generation_current(
                    generation,
                    tokio::time::timeout(Duration::from_secs(120), append_marked_captions),
                )
                .await;
            match result {
                Ok(Ok(Err(_))) => tracing::warn!("caption history fixture marker failed"),
                Ok(Err(_)) => tracing::warn!("caption history fixture timed out"),
                _ => {}
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_history_preserves_existing_rows_and_selected_source_through_twenty() {
        for source in [AudioSource::System, AudioSource::Microphone] {
            let mut controller = TranslationSessionController::default();
            controller.set_audio_input(match source {
                AudioSource::System => super::super::AudioInput::System,
                AudioSource::Microphone => super::super::AudioInput::Microphone,
            });
            controller.did_connect();
            for index in 1..=INITIAL_CAPTIONS {
                append_caption(&mut controller, source, index);
            }
            let original = controller.state.subtitles.history.clone();
            assert_eq!(original.len(), INITIAL_CAPTIONS);
            for index in 19..=20 {
                append_caption(&mut controller, source, index);
            }
            let history = &controller.state.subtitles.history;
            assert_eq!(history.len(), FINAL_CAPTION);
            assert_eq!(&history[..INITIAL_CAPTIONS], original.as_slice());
            assert!(history
                .windows(2)
                .all(|pair| pair[0].created_at_ms < pair[1].created_at_ms));
            for (index, pair) in history.iter().enumerate() {
                assert!(pair
                    .source
                    .starts_with(&format!("Original {:02} ", index + 1)));
                assert!(pair
                    .translation
                    .starts_with(&format!("译文 {:02} ", index + 1)));
                assert_eq!(pair.audio_source, source);
            }
            assert_eq!(controller.archive().page("", 0).total, 0);
        }
    }

    #[test]
    fn caption_history_waits_for_nineteen_before_twenty_and_resets_old_markers() {
        let directory = tempfile::tempdir().unwrap();
        let mut scene = CaptionHistory::prepare(directory.path().to_owned()).unwrap();
        std::fs::write(directory.path().join("append-caption-20"), "").unwrap();
        assert!(!scene.next_is_marked().unwrap());
        std::fs::write(directory.path().join("append-caption-19"), "").unwrap();
        assert!(scene.next_is_marked().unwrap());
        let mut controller = TranslationSessionController::default();
        controller.did_connect();
        scene.append(&mut controller, AudioSource::System);
        scene.record_count(19).unwrap();
        assert!(scene.next_is_marked().unwrap());
        assert_eq!(
            std::fs::read_to_string(directory.path().join(COUNT_FILE)).unwrap(),
            "19"
        );
        CaptionHistory::prepare(directory.path().to_owned()).unwrap();
        assert!(!directory.path().join("append-caption-19").exists());
        assert!(!directory.path().join("append-caption-20").exists());
        assert!(!directory.path().join(COUNT_FILE).exists());
    }
}
