use super::*;
use crate::core::models::SubtitlePair;

fn final_pair(id: u64, source: &str, translation: &str) -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::SubtitleConfirmedPair {
        utterance_id: id,
        source_utterance_id: Some(id),
        source: source.into(),
        translation: translation.into(),
        language: Some("en".into()),
    }
}

#[test]
fn refresh_keeps_the_complete_primary_projection_and_independent_histories() {
    use mimi_core::{PreviewSubtitlePair, RealtimeSubtitlePreview, SubtitleLine};

    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.set_atomic_preview(true);
    controller.did_connect();
    for source in [AudioSource::System, AudioSource::Microphone] {
        controller.handle_from(source, final_pair(1, "Synthetic final", "Synthetic result"));
    }
    let aggregate_history = serde_json::to_value(&controller.state.subtitles.history).unwrap();
    for (index, lane) in controller.sources.iter_mut().enumerate() {
        let owner = format!("synthetic-{index}");
        lane.state.subtitles.source =
            SubtitleLine::for_utterance(format!("Source {index}"), false, owner.clone());
        lane.state.subtitles.translation =
            SubtitleLine::for_utterance(format!("Translation {index}"), false, owner.clone());
        let pair = PreviewSubtitlePair {
            utterance_id: Some(owner),
            source: format!("Paired source {index}"),
            translation: format!("Paired result {index}"),
        };
        lane.state.subtitles.preview_pair = Some(pair.clone());
        lane.state.subtitles.display_pair = Some(pair);
        lane.state.subtitles.realtime_preview = Some(RealtimeSubtitlePreview {
            source: lane.state.subtitles.source.clone(),
            translation: lane.state.subtitles.translation.clone(),
        });
        lane.state.subtitles.display_pair_final = index == 0;
        lane.state.subtitles.display_pair_created_at_ms = Some(100 + index as u64);
        lane.state.detected_language = Some(DetectedLanguage {
            code: if index == 0 { "en" } else { "ja" }.into(),
        });
        lane.state.is_translation_pending = index == 0;
        lane.state.is_translation_preview_pending = index == 1;
        lane.state.is_translation_timed_out = index == 1;
    }

    for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
        // Exercise projection without a capture reconfiguration, which
        // deliberately clears transient previews before refreshing.
        controller.audio_input = input;
        controller.refresh();
        let wire = serde_json::to_value(&controller.state.subtitles).unwrap();
        let primary = &controller.sources[source_index(input.sources()[0])].state;
        let primary_wire = serde_json::to_value(&primary.subtitles).unwrap();
        for (field, value) in primary_wire.as_object().unwrap() {
            if field != "history" && field != "tracks" {
                assert_eq!(wire.get(field), Some(value), "primary field {field}");
            }
        }
        assert_eq!(wire["history"], aggregate_history);
        assert_eq!(
            wire["tracks"].as_array().unwrap().len(),
            input.sources().len()
        );
        for (track, source) in wire["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .zip(input.sources())
        {
            let lane = &controller.sources[source_index(*source)].state;
            let lane_wire = serde_json::to_value(&lane.subtitles).unwrap();
            for (field, value) in lane_wire.as_object().unwrap() {
                if field != "tracks" {
                    assert_eq!(track.get(field), Some(value), "track field {field}");
                }
            }
            assert_eq!(track["audioSource"], serde_json::to_value(source).unwrap());
        }
        assert_eq!(
            controller.state.detected_language,
            primary.detected_language
        );
        assert_eq!(controller.state.atomic_preview, Some(true));
        assert_eq!(
            controller.state.is_translation_pending,
            input.sources().contains(&AudioSource::System)
        );
        assert_eq!(
            controller.state.is_translation_preview_pending,
            input.sources().contains(&AudioSource::Microphone)
        );
        assert_eq!(
            controller.state.is_translation_timed_out,
            input.sources().contains(&AudioSource::Microphone)
        );
    }
}

#[test]
fn stopped_session_restart_keeps_confirmations_and_starts_a_separate_archive() {
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.archive_mut().begin(true, 0);
    controller.begin_session();
    controller.did_connect();
    for source in [AudioSource::System, AudioSource::Microphone] {
        controller.handle_from(source, final_pair(1, "Old confirmation", "Old translation"));
        controller.handle_from(
            source,
            LiveTranslateServerEvent::SourceDraft {
                text: "Obsolete draft".into(),
                language: Some("ja".into()),
            },
        );
        controller.handle_from(
            source,
            LiveTranslateServerEvent::SubtitlePreviewPair {
                source_utterance_id: Some(2),
                source: "Obsolete draft".into(),
                language: Some("ja".into()),
                translation: "Obsolete preview".into(),
            },
        );
        controller.handle_from(source, LiveTranslateServerEvent::TranslationStarted);
        controller.handle_from(
            source,
            LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 2 },
        );
    }
    controller.clear_translation_pending_from(AudioSource::Microphone);
    let confirmed = controller.state.subtitles.history.clone();
    assert_eq!(controller.archive().count(), 2);
    controller.begin_stopping();
    controller.did_stop();

    controller.set_audio_input(AudioInput::Both);
    controller.archive_mut().begin(true, 1);
    controller.begin_session();
    assert_eq!(controller.state.status, SessionStatus::Connecting);
    assert_eq!(controller.state.subtitles.history, confirmed);
    assert_eq!(controller.archive().count(), 0);
    assert!(controller.state.subtitles.tracks.iter().all(|track| {
        !track.is_translation_pending
            && !track.is_translation_preview_pending
            && !track.is_translation_timed_out
            && track.preview_pair.is_none()
            && track.source.text != "Obsolete draft"
            && track.translation.text != "Obsolete preview"
            && track.detected_language.is_none()
    }));

    controller.did_connect();
    for source in [AudioSource::System, AudioSource::Microphone] {
        controller.handle_from(source, final_pair(1, "New confirmation", "New translation"));
    }
    assert_eq!(controller.state.subtitles.history.len(), 4);
    assert_eq!(&controller.state.subtitles.history[..2], &confirmed);
    assert_eq!(controller.archive().count(), 2);
    let exported = controller.archive().export().unwrap();
    assert!(!exported.contains("Old confirmation"));
    assert_eq!(exported.matches("New confirmation").count(), 2);
    assert_eq!(
        controller.state.subtitles.history[2].audio_source,
        AudioSource::System
    );
    assert_eq!(
        controller.state.subtitles.history[3].audio_source,
        AudioSource::Microphone
    );

    controller.clear_subtitles();
    assert!(controller.state.subtitles.history.is_empty());
    assert!(controller
        .state
        .subtitles
        .tracks
        .iter()
        .all(|track| track.history.is_empty()));
    assert_eq!(controller.archive().count(), 0);
}

#[test]
fn stopped_session_source_change_keeps_history_and_filters_inactive_events() {
    let mut controller = TranslationSessionController::default();
    controller.begin_session();
    controller.did_connect();
    controller.handle_from(
        AudioSource::System,
        final_pair(1, "Previous system confirmation", "Previous translation"),
    );
    controller.begin_stopping();
    controller.did_stop();
    let confirmed = controller.state.subtitles.history.clone();

    controller.set_audio_input(AudioInput::Microphone);
    assert_eq!(controller.state.status, SessionStatus::Idle);
    assert_eq!(controller.state.subtitles.history, confirmed);
    assert_eq!(controller.state.subtitles.tracks.len(), 1);
    assert_eq!(
        controller.state.subtitles.tracks[0].audio_source,
        AudioSource::Microphone
    );
    controller.begin_session();
    controller.did_connect();
    controller.handle_from(
        AudioSource::System,
        final_pair(2, "Unselected system", "Ignored translation"),
    );
    assert_eq!(controller.state.subtitles.history, confirmed);
    controller.handle_from(
        AudioSource::Microphone,
        final_pair(1, "New microphone confirmation", "New translation"),
    );
    assert_eq!(controller.state.subtitles.history.len(), 2);
    assert_eq!(
        controller.state.subtitles.history[0].audio_source,
        AudioSource::System
    );
    assert_eq!(
        controller.state.subtitles.history[1].audio_source,
        AudioSource::Microphone
    );
}

#[test]
fn repeated_session_restarts_keep_display_history_bounded_without_retention() {
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    for session in 0..25 {
        controller.archive_mut().begin(false, session);
        controller.begin_session();
        controller.did_connect();
        for source in [AudioSource::System, AudioSource::Microphone] {
            controller.handle_from(
                source,
                final_pair(1, "Synthetic final", "Synthetic translation"),
            );
        }
        controller.begin_stopping();
        controller.did_stop();
        assert!(controller.state.subtitles.history.len() <= 20);
        assert!(controller
            .state
            .subtitles
            .tracks
            .iter()
            .all(|track| track.history.len() <= 20));
        assert_eq!(controller.archive().count(), 0);
    }
    assert_eq!(controller.state.subtitles.history.len(), 20);
    assert!(controller
        .state
        .subtitles
        .tracks
        .iter()
        .all(|track| track.history.len() == 20));
}

#[test]
fn manual_restart_accepts_an_identical_idless_final_once_in_the_new_archive() {
    let mut controller = TranslationSessionController::default();
    let final_event = LiveTranslateServerEvent::SubtitleFinalPair {
        source: "Repeated confirmation".into(),
        language: Some("en".into()),
        translation: "Repeated translation".into(),
    };
    controller.archive_mut().begin(true, 0);
    controller.begin_session();
    controller.did_connect();
    controller.handle_from(AudioSource::System, final_event.clone());
    controller.begin_stopping();
    controller.did_stop();
    let previous_time = controller.state.subtitles.history[0].created_at_ms;

    controller.archive_mut().begin(true, 1);
    controller.begin_session();
    // Connecting can be retried before the new session produces any text.
    controller.begin_connecting();
    controller.begin_connecting();
    controller.did_connect();
    controller.handle_from(AudioSource::System, final_event.clone());
    controller.handle_from(AudioSource::System, final_event.clone());
    assert_eq!(controller.state.subtitles.history.len(), 2);
    assert!(controller.state.subtitles.history[1].created_at_ms > previous_time);
    assert_eq!(controller.archive().count(), 1);
    assert_eq!(
        controller
            .archive()
            .export()
            .unwrap()
            .matches("Repeated confirmation")
            .count(),
        1
    );

    // A reconnect within that session still suppresses duplicate finals.
    controller.begin_connecting();
    controller.did_connect();
    controller.handle_from(AudioSource::System, final_event);
    assert_eq!(controller.state.subtitles.history.len(), 2);
    assert_eq!(controller.archive().count(), 1);
}

#[test]
fn enabling_a_dormant_source_uses_the_current_manual_session_boundary() {
    let mut controller = TranslationSessionController::default();
    let final_event = LiveTranslateServerEvent::SubtitleFinalPair {
        source: "Repeated microphone confirmation".into(),
        language: Some("en".into()),
        translation: "Repeated microphone translation".into(),
    };
    controller.set_audio_input(AudioInput::Both);
    controller.begin_session();
    controller.did_connect();
    controller.handle_from(AudioSource::Microphone, final_event.clone());
    controller.begin_stopping();
    controller.did_stop();

    controller.set_audio_input(AudioInput::System);
    controller.archive_mut().begin(true, 1);
    controller.begin_session();
    controller.did_connect();
    assert_eq!(controller.state.subtitles.tracks.len(), 1);
    assert_eq!(controller.state.subtitles.history.len(), 1);

    controller.reconfigure_audio_input(AudioInput::Both);
    controller.begin_connecting();
    controller.did_connect();
    controller.handle_from(AudioSource::Microphone, final_event.clone());
    controller.handle_from(AudioSource::Microphone, final_event);
    assert_eq!(controller.state.subtitles.history.len(), 2);
    assert!(controller
        .state
        .subtitles
        .history
        .iter()
        .all(|pair| pair.audio_source == AudioSource::Microphone));
    assert_eq!(controller.archive().count(), 1);
}

#[test]
fn audio_input_switch_preserves_confirmed_history_and_disabled_source_labels() {
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.did_connect();
    controller.archive_mut().begin(true, 0);
    controller.handle_from(
        AudioSource::System,
        final_pair(1, "Synthetic system", "System translation"),
    );
    controller.handle_from(
        AudioSource::Microphone,
        final_pair(1, "Synthetic microphone", "Microphone translation"),
    );
    let confirmed = controller.state.subtitles.history.clone();
    let transcript = controller.archive().export();
    controller.handle_from(
        AudioSource::Microphone,
        LiveTranslateServerEvent::TranslationStarted,
    );
    controller.handle_from(
        AudioSource::Microphone,
        LiveTranslateServerEvent::SourceDraft {
            text: "Discard this synthetic draft".into(),
            language: Some("ja".into()),
        },
    );
    controller.reconfigure_audio_input(AudioInput::System);
    assert_eq!(controller.state.status, SessionStatus::Listening);
    assert_eq!(controller.state.subtitles.history, confirmed);
    assert_eq!(controller.archive().export(), transcript);
    assert_eq!(controller.state.subtitles.tracks.len(), 1);
    assert_eq!(
        controller.state.subtitles.tracks[0].audio_source,
        AudioSource::System
    );
    controller.handle_from(
        AudioSource::Microphone,
        final_pair(2, "Late microphone", "Late translation"),
    );
    assert_eq!(controller.state.subtitles.history, confirmed);
    controller.reconfigure_audio_input(AudioInput::Both);
    assert_eq!(controller.state.subtitles.history, confirmed);
    assert!(controller
        .state
        .subtitles
        .tracks
        .iter()
        .all(|track| !track.is_translation_pending && track.preview_pair.is_none()));
    assert!(!controller.state.subtitles.tracks[1]
        .source
        .text
        .contains("Discard"));
    // Fresh client IDs restart at one; old confirmed history remains intact.
    controller.handle_from(
        AudioSource::Microphone,
        final_pair(1, "New microphone", "New translation"),
    );
    assert_eq!(controller.state.subtitles.history.len(), 3);
    assert_eq!(
        controller.state.subtitles.history[2].audio_source,
        AudioSource::Microphone
    );
}

#[test]
fn application_capture_restart_preserves_history_and_reaccepts_fresh_provider_ids() {
    let mut controller = TranslationSessionController::default();
    controller.did_connect();
    controller.archive_mut().begin(true, 0);
    let pair = |text: &str| LiveTranslateServerEvent::SubtitleConfirmedPair {
        source_utterance_id: Some(1),
        utterance_id: 1,
        source: text.into(),
        translation: "Synthetic translation".into(),
        language: Some("en".into()),
    };
    controller.handle_from(AudioSource::System, pair("Synthetic first application"));
    let history = controller.state.subtitles.history.clone();
    let transcript = controller.archive().export();
    controller.handle_from(
        AudioSource::System,
        LiveTranslateServerEvent::SourceDraft {
            text: "Synthetic obsolete draft".into(),
            language: Some("en".into()),
        },
    );
    // App A -> B retains AudioInput::System, but starts a fresh provider.
    controller.reconfigure_audio_input(AudioInput::System);
    assert_eq!(controller.state.status, SessionStatus::Listening);
    assert_eq!(controller.state.subtitles.history, history);
    assert_eq!(controller.archive().export(), transcript);
    assert!(!controller.state.subtitles.source.text.contains("obsolete"));
    controller.handle_from(AudioSource::System, pair("Synthetic second application"));
    assert_eq!(controller.state.subtitles.history.len(), 2);
    assert_eq!(
        controller.state.subtitles.history[1].source,
        "Synthetic second application"
    );
}

#[test]
fn audio_input_switch_preserves_paused_or_error_controller_status() {
    for status in [
        SessionStatus::Listening,
        SessionStatus::Error("synthetic permission error".into()),
        SessionStatus::Idle,
    ] {
        let mut controller = TranslationSessionController::default();
        controller.sources[0].state.status = status.clone();
        controller.refresh();
        controller.reconfigure_audio_input(AudioInput::Both);
        assert_eq!(controller.state.status, status);
        assert!(controller
            .state
            .subtitles
            .tracks
            .iter()
            .all(|track| !track.is_translation_pending));
    }
}

#[test]
fn simultaneous_drafts_and_same_provider_ids_stay_with_their_inputs() {
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    for (input, text, language) in [
        (AudioSource::System, "Synthetic system draft", "en"),
        (AudioSource::Microphone, "Synthetic microphone draft", "ja"),
    ] {
        controller.handle_from(
            input,
            LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: 1,
                text: text.into(),
                language: Some(language.into()),
            },
        );
    }
    let tracks = &controller.state.subtitles.tracks;
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].source.text, "Synthetic system draft");
    assert_eq!(tracks[1].source.text, "Synthetic microphone draft");
    assert_eq!(tracks[0].detected_language.as_ref().unwrap(), "en");
    assert_eq!(tracks[1].detected_language.as_ref().unwrap(), "ja");
    assert_ne!(tracks[0].source.utterance_id, tracks[1].source.utterance_id);
    let wire = serde_json::to_value(&controller.state.subtitles).unwrap();
    assert_eq!(wire["tracks"][0]["detectedLanguage"], "en");
    assert_eq!(wire["tracks"][1]["detectedLanguage"], "ja");
    assert_eq!(wire["tracks"][0]["audioSource"], "system");
    assert_eq!(wire["tracks"][1]["audioSource"], "microphone");

    controller.handle_from(
        AudioSource::System,
        final_pair(1, "System final", "System translation"),
    );
    // One input's provider ID does not consume the other's ID or draft.
    assert!(controller.accepts_confirmed_pair_from(
        AudioSource::Microphone,
        1,
        "Mic final",
        "Mic translation"
    ));
    assert_eq!(
        controller.state.subtitles.tracks[1].source.text,
        "Synthetic microphone draft"
    );
    controller.handle_from(
        AudioSource::Microphone,
        final_pair(1, "Mic final", "Mic translation"),
    );
    assert_eq!(controller.state.subtitles.history.len(), 2);
    assert_eq!(
        controller.state.subtitles.history[0].audio_source,
        AudioSource::System
    );
    assert_eq!(
        controller.state.subtitles.history[1].audio_source,
        AudioSource::Microphone
    );
    assert!(
        controller.state.subtitles.history[0].created_at_ms
            < controller.state.subtitles.history[1].created_at_ms
    );
    controller.handle_from(AudioSource::System, final_pair(1, "Replay", "Replay"));
    controller.handle_from(AudioSource::Microphone, final_pair(1, "Replay", "Replay"));
    assert_eq!(controller.state.subtitles.history.len(), 2);
}

#[test]
fn finals_in_one_lane_do_not_clear_pending_or_preview_in_the_other() {
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.begin_connecting();
    controller.handle_from(
        AudioSource::System,
        LiveTranslateServerEvent::SessionUpdated,
    );
    assert_eq!(controller.state.status, SessionStatus::Connecting);
    controller.handle_from(
        AudioSource::Microphone,
        LiveTranslateServerEvent::SessionUpdated,
    );
    assert_eq!(controller.state.status, SessionStatus::Listening);
    for source in [AudioSource::System, AudioSource::Microphone] {
        controller.handle_from(source, LiveTranslateServerEvent::TranslationStarted);
        controller.handle_from(
            source,
            LiveTranslateServerEvent::SubtitlePreviewPair {
                source_utterance_id: Some(2),
                source: "Synthetic preview".into(),
                language: None,
                translation: "Synthetic translation".into(),
            },
        );
    }
    controller.handle_from(
        AudioSource::Microphone,
        final_pair(1, "Mic final", "Mic translation"),
    );
    assert!(controller.state.is_translation_pending);
    assert!(controller.state.subtitles.tracks[0].is_translation_pending);
    assert!(!controller.state.subtitles.tracks[1].is_translation_pending);
    assert!(controller.state.subtitles.tracks[0].preview_pair.is_some());
    controller.clear_translation_pending_from(AudioSource::System);
    assert!(controller.state.subtitles.tracks[0].is_translation_timed_out);
    assert!(!controller.state.subtitles.tracks[1].is_translation_timed_out);
    assert!(!controller.state.is_translation_pending);
}

#[test]
fn both_histories_and_the_aggregate_are_bounded_and_export_preserves_sources() {
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.archive_mut().begin(true, 0);
    for id in 1..=40 {
        for input in [AudioSource::System, AudioSource::Microphone] {
            controller.handle_from(
                input,
                final_pair(id, "Synthetic final", "Synthetic translation"),
            );
        }
    }
    assert_eq!(controller.state.subtitles.history.len(), 20);
    assert!(controller
        .state
        .subtitles
        .tracks
        .iter()
        .all(|track| track.history.len() == 20));
    assert_eq!(controller.archive().count(), 80);
    let exported = controller.archive().export().unwrap();
    assert_eq!(exported.matches("System audio").count(), 40);
    assert_eq!(exported.matches("Microphone").count(), 40);
    controller.clear_subtitles();
    assert!(controller.state.subtitles.history.is_empty());
    assert!(controller
        .state
        .subtitles
        .tracks
        .iter()
        .all(|track| track.history.is_empty() && track.preview_pair.is_none()));
    assert_eq!(controller.archive().count(), 0);
    controller.handle_from(
        AudioSource::Microphone,
        final_pair(40, "Replay after clear", "Replay"),
    );
    assert!(controller.state.subtitles.history.is_empty());
    controller.begin_connecting();
    controller.handle_from(
        AudioSource::Microphone,
        final_pair(1, "New generation", "New translation"),
    );
    assert_eq!(controller.state.subtitles.history.len(), 1);
}

#[test]
fn unselected_inputs_cannot_produce_content_and_legacy_pairs_default_to_system() {
    let mut controller = TranslationSessionController::default();
    controller.handle_from(
        AudioSource::Microphone,
        final_pair(1, "Unexpected microphone", "Unexpected microphone"),
    );
    assert!(controller.state.subtitles.history.is_empty());
    controller.set_audio_input(AudioInput::Microphone);
    controller.handle_from(
        AudioSource::System,
        final_pair(1, "Unexpected system", "Unexpected system"),
    );
    assert!(controller.state.subtitles.history.is_empty());
    controller.handle(final_pair(
        1,
        "Synthetic microphone",
        "Synthetic translation",
    ));
    assert_eq!(
        controller.state.subtitles.history[0].audio_source,
        AudioSource::Microphone
    );
    assert_eq!(controller.state.subtitles.tracks.len(), 1);
    let legacy: SubtitlePair = serde_json::from_str(
        r#"{"source":"Legacy synthetic","translation":"Legacy translation","createdAt":100}"#,
    )
    .unwrap();
    assert_eq!(legacy.audio_source, AudioSource::System);
    let mut microphone = legacy.clone();
    microphone.audio_source = AudioSource::Microphone;
    assert_ne!(legacy, microphone);
}
