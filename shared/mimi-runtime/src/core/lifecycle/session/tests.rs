use super::*;

#[test]
fn independent_source_projection_matches_desktop_and_bounds_combined_history() {
    let mut desktop = TranslationSessionController::default();
    desktop.set_audio_input(AudioInput::Both);
    desktop.begin_session();
    desktop.did_connect();
    for source in [AudioSource::System, AudioSource::Microphone] {
        desktop.handle_from(
            source,
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id: "same-provider-id".into(),
                source: format!("Synthetic {source:?}"),
                translation: format!("Translated {source:?}"),
                language: None,
            },
        );
    }
    let lanes: Vec<_> = [AudioSource::System, AudioSource::Microphone]
        .into_iter()
        .map(|source| (source, desktop.sources[source_index(source)].state.clone()))
        .collect();
    let merged = merge_independent_states(lanes.clone(), 6, false);
    assert_eq!(merged.subtitles.tracks, desktop.state.subtitles.tracks);
    assert_eq!(merged.subtitles.source, desktop.state.subtitles.source);
    assert_eq!(merged.status, desktop.state.status);
    let bounded = merge_independent_states(lanes, 1, false);
    assert!(bounded.subtitles.history.len() <= 1);
    assert_eq!(bounded.subtitles.tracks.len(), 2);
    assert_eq!(
        bounded.subtitles.tracks[1].audio_source,
        AudioSource::Microphone
    );
    assert!(bounded.subtitles.tracks[0].source.text.contains("System"));
    assert!(bounded.subtitles.tracks[1]
        .source
        .text
        .contains("Microphone"));
}

#[test]
fn stopped_independent_projection_keeps_confirmed_pairs_and_retires_activity() {
    let mut controller = TranslationSessionController::default();
    controller.begin_session();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::SubtitleFinalPair {
        source: "Synthetic source".into(),
        translation: "Synthetic translation".into(),
        language: None,
    });
    controller.state.is_translation_pending = true;
    controller.state.is_translation_timed_out = true;
    let merged = merge_independent_states(vec![(AudioSource::System, controller.state)], 0, true);
    assert_eq!(merged.status, SessionStatus::Idle);
    assert!(merged.subtitles.display_pair_final);
    assert_eq!(merged.subtitles.source.text, "Synthetic source");
    assert!(merged.subtitles.realtime_preview.is_none());
    assert!(!merged.is_translation_pending);
    assert!(!merged.is_translation_timed_out);
    assert!(merged.subtitles.history.is_empty());
}

#[test]
fn stopping_zero_history_keeps_the_final_before_a_newer_preview() {
    let mut controller = TranslationSessionController::default();
    controller.begin_session();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::SubtitleConfirmedPair {
        utterance_id: 1,
        source_utterance_id: None,
        source: "Accepted source".into(),
        translation: "Accepted translation".into(),
        language: None,
    });
    controller.handle(LiveTranslateServerEvent::SubtitlePreviewPair {
        source_utterance_id: Some(2),
        source: "New preview".into(),
        translation: "Preview translation".into(),
        language: None,
    });
    assert!(!controller.state.subtitles.display_pair_final);
    let stopped = merge_independent_states(vec![(AudioSource::System, controller.state)], 0, true);
    assert_eq!(stopped.subtitles.source.text, "Accepted source");
    assert_eq!(stopped.subtitles.translation.text, "Accepted translation");
    assert!(stopped.subtitles.display_pair_final);
    assert!(stopped.subtitles.history.is_empty());
}

fn identified_source(id: &str, is_final: bool) -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::UtteranceText {
        utterance_id: id.into(),
        role: UtteranceRole::Source,
        text: format!("Synthetic {id} source."),
        is_final,
        language: None,
    }
}

#[test]
fn old_source_activity_preserves_current_preview_recovery_timeout_and_completion() {
    use crate::core::diagnostics::latency::{TranslationRecovery, TranslationRecoveryReason};
    let now = Instant::now();
    for mode in ["preview", "recovery", "timeout", "completed"] {
        let mut controller = TranslationSessionController::default();
        controller.did_connect();
        controller.handle_ordered_from_at(
            AudioSource::System,
            identified_source("old", false),
            now,
            Some(0),
            false,
        );
        let started = controller.handle_ordered_from_at(
            AudioSource::System,
            identified_source("current", true),
            now,
            Some(2),
            true,
        );
        assert_eq!(
            started.translation_pending_deadline,
            Some(now + TRANSLATION_PENDING_TIMEOUT)
        );
        match mode {
            "preview" => controller
                .handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 7 }),
            "recovery" => controller.handle(LiveTranslateServerEvent::TranslationDeferred(
                TranslationRecovery {
                    reason: TranslationRecoveryReason::RateLimited,
                    retry_after_ms: 4_000,
                    retry_scheduled: true,
                },
            )),
            "timeout" => {
                assert!(controller.tick(now + TRANSLATION_PENDING_TIMEOUT));
            }
            "completed" => {
                controller.handle_ordered_from_at(
                    AudioSource::System,
                    LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                        utterance_id: "current".into(),
                        source: "Synthetic current source.".into(),
                        translation: "Synthetic current translation.".into(),
                        language: None,
                    },
                    now,
                    Some(2),
                    false,
                );
            }
            _ => unreachable!(),
        }
        let before = controller.state.clone();
        let deadline = controller.translation_pending_deadline_from(AudioSource::System);
        for (id, order) in [("old", 0), ("unseen-old", 1)] {
            let application = controller.handle_ordered_from_at(
                AudioSource::System,
                identified_source(id, true),
                now + Duration::from_secs(7),
                Some(order),
                true,
            );
            assert!(application.translation_pending_deadline.is_none());
            assert!(!application.resolves_translation_timeout);
            assert_eq!(controller.state, before);
            assert_eq!(
                controller.translation_pending_deadline_from(AudioSource::System),
                deadline
            );
        }
        if mode == "completed" {
            let duplicate = controller.handle_ordered_from_at(
                AudioSource::System,
                identified_source("current", true),
                now + Duration::from_secs(7),
                Some(2),
                true,
            );
            assert!(duplicate.translation_pending_deadline.is_none());
            assert_eq!(controller.state, before);
        }
        let next = controller.handle_ordered_from_at(
            AudioSource::System,
            identified_source("next", true),
            now + Duration::from_secs(8),
            Some(3),
            true,
        );
        assert_eq!(
            next.translation_pending_deadline,
            Some(now + Duration::from_secs(8) + TRANSLATION_PENDING_TIMEOUT)
        );
        assert!(controller.state.is_translation_pending);
        assert!(!controller.state.is_translation_preview_pending);
        assert!(!controller.state.is_translation_timed_out);
        assert!(controller.state.translation_recovery.is_none());
    }
}

#[test]
fn source_final_activity_obeys_source_selection_stopping_and_original_only() {
    let now = Instant::now();
    let mut controller = TranslationSessionController::default();
    controller.did_connect();
    let before = controller.state.clone();
    let disabled = controller.handle_ordered_from_at(
        AudioSource::Microphone,
        identified_source("disabled", true),
        now,
        Some(0),
        true,
    );
    assert!(disabled.translation_pending_deadline.is_none());
    assert_eq!(controller.state, before);
    let original_only = controller.handle_ordered_from_at(
        AudioSource::System,
        identified_source("original-only", true),
        now,
        Some(0),
        false,
    );
    assert!(original_only.translation_pending_deadline.is_none());
    assert!(!controller.state.is_translation_pending);
    assert_eq!(
        controller.state.subtitles.source.utterance_id.as_deref(),
        Some("original-only")
    );
    controller.begin_stopping();
    let before = controller.state.clone();
    let ignored = controller.handle_ordered_from_at(
        AudioSource::System,
        identified_source("stopping", true),
        now,
        Some(1),
        true,
    );
    assert!(ignored.translation_pending_deadline.is_none());
    assert_eq!(controller.state, before);
}

#[test]
fn pending_deadline_is_shared_and_drafts_do_not_extend_it() {
    let now = Instant::now();
    let mut controller = TranslationSessionController::default();
    controller.did_connect();
    controller.handle_from_at(
        AudioSource::System,
        LiveTranslateServerEvent::TranslationStarted,
        now,
    );
    let deadline = now + TRANSLATION_PENDING_TIMEOUT;
    assert_eq!(
        controller.translation_pending_deadline_from(AudioSource::System),
        Some(deadline)
    );
    controller.handle_from_at(
        AudioSource::System,
        LiveTranslateServerEvent::TranslationDraft("草稿".into()),
        now + Duration::from_secs(5),
    );
    let mut before = controller.state.subtitles.clone();
    before.tracks.clear();
    assert!(!controller.tick(deadline - Duration::from_nanos(1)));
    assert!(controller.tick(deadline));
    assert!(!controller.state.is_translation_pending);
    assert!(controller.state.is_translation_timed_out);
    assert_eq!(controller.state.status, SessionStatus::Listening);
    let mut after = controller.state.subtitles.clone();
    after.tracks.clear();
    assert_eq!(after, before);
    assert!(!controller.tick(deadline + Duration::from_secs(1)));
}

#[test]
fn a_new_request_replaces_the_deadline_and_final_or_stop_cancels_it() {
    let now = Instant::now();
    let mut controller = TranslationSessionController::default();
    controller.handle_from_at(
        AudioSource::System,
        LiveTranslateServerEvent::TranslationStarted,
        now,
    );
    controller.handle_from_at(
        AudioSource::System,
        LiveTranslateServerEvent::TranslationStarted,
        now + Duration::from_secs(5),
    );
    assert!(!controller
        .expire_translation_pending_from(AudioSource::System, now + TRANSLATION_PENDING_TIMEOUT));
    controller.handle_from_at(
        AudioSource::System,
        LiveTranslateServerEvent::TranslationFinal("译文".into()),
        now + Duration::from_secs(7),
    );
    assert_eq!(
        controller.translation_pending_deadline_from(AudioSource::System),
        None
    );
    assert!(!controller.tick(now + Duration::from_secs(12)));
    controller.handle_from_at(
        AudioSource::System,
        LiveTranslateServerEvent::TranslationStarted,
        now + Duration::from_secs(13),
    );
    controller.did_stop();
    assert_eq!(
        controller.translation_pending_deadline_from(AudioSource::System),
        None
    );
}

#[test]
fn pending_deadlines_remain_independent_per_audio_source() {
    let now = Instant::now();
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.handle_from_at(
        AudioSource::System,
        LiveTranslateServerEvent::TranslationStarted,
        now,
    );
    controller.handle_from_at(
        AudioSource::Microphone,
        LiveTranslateServerEvent::TranslationStarted,
        now + Duration::from_secs(2),
    );
    assert!(controller.tick(now + TRANSLATION_PENDING_TIMEOUT));
    assert!(controller.state.is_translation_pending);
    assert_eq!(
        controller.translation_pending_deadline_from(AudioSource::System),
        None
    );
    assert_eq!(
        controller.translation_pending_deadline_from(AudioSource::Microphone),
        Some(now + Duration::from_secs(8))
    );
    controller.clear_subtitles();
    assert!(!controller.tick(now + Duration::from_secs(20)));
}

#[test]
fn identified_late_final_keeps_newer_text_language_and_pending_owner() {
    let mut controller = SourceSessionController::default();
    let source = |id: &str, text: &str, language: &str| LiveTranslateServerEvent::UtteranceText {
        utterance_id: id.into(),
        role: UtteranceRole::Source,
        text: text.into(),
        is_final: true,
        language: Some(language.into()),
    };
    let pair = |id: &str, text: &str, language: &str| {
        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
            utterance_id: id.into(),
            source: text.into(),
            translation: format!("译文 {text}"),
            language: Some(language.into()),
        }
    };
    controller.handle(source("A", "Older sentence", "en"));
    controller.handle(source("B", "새로운 문장", "ko"));
    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    controller.handle(pair("A", "Older sentence", "en"));
    assert!(controller.state.is_translation_pending);
    assert_eq!(
        controller.state.subtitles.source.utterance_id.as_deref(),
        Some("B")
    );
    assert_eq!(
        controller.state.detected_language.as_ref().unwrap().code,
        "ko"
    );
    controller.handle(pair("B", "새로운 문장", "ko"));
    let expected = controller.state.clone();
    controller.handle(source("A", "Older sentence", "en"));
    controller.handle(pair("A", "Older sentence", "en"));
    assert_eq!(controller.state, expected);
    assert_eq!(
        controller
            .state
            .subtitles
            .display_pair
            .as_ref()
            .unwrap()
            .utterance_id
            .as_deref(),
        Some("B")
    );
    assert_eq!(controller.state.subtitles.history.len(), 2);
    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    let pending = controller.state.clone();
    controller.handle(pair("A", "Older sentence", "en"));
    assert_eq!(controller.state, pending);
}

#[test]
fn registered_complete_previews_keep_late_finals_out_of_the_current_pair() {
    let mut controller = SourceSessionController::default();
    let before = controller.state.clone();
    for id in ["A", "B"] {
        controller.handle(LiveTranslateServerEvent::UtteranceRegistered {
            utterance_id: id.into(),
        });
    }
    assert_eq!(controller.state, before);
    let preview =
        |source: &str, translation: &str| LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair {
            utterance_id: "B".into(),
            source: source.into(),
            translation: translation.into(),
            language: Some("ko".into()),
        };
    controller.handle(preview("Synthetic B", "Synthetic translated B"));
    controller.handle(LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
        utterance_id: "A".into(),
        source: "Synthetic A".into(),
        translation: "Synthetic translated A".into(),
        language: Some("en".into()),
    });
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert_eq!(controller.state.subtitles.history[0].source, "Synthetic A");
    assert_eq!(controller.state.subtitles.source.text, "Synthetic B");
    assert_eq!(
        controller.state.subtitles.translation.text,
        "Synthetic translated B"
    );
    let current = controller
        .state
        .subtitles
        .realtime_preview
        .as_ref()
        .unwrap();
    assert_eq!(current.source.text, "Synthetic B");
    assert_eq!(current.translation.text, "Synthetic translated B");
    assert_eq!(
        current.source.utterance_id,
        current.translation.utterance_id
    );
    assert_eq!(
        controller.state.detected_language.as_ref().unwrap().code,
        "ko"
    );
    controller.handle(preview(
        "Synthetic B revised",
        "Synthetic translation revised",
    ));
    assert_eq!(
        controller.state.subtitles.source.text,
        "Synthetic B revised"
    );
    assert_eq!(
        controller.state.subtitles.translation.text,
        "Synthetic translation revised"
    );
    let revised = controller
        .state
        .subtitles
        .realtime_preview
        .as_ref()
        .unwrap();
    assert_eq!(revised.source.text, "Synthetic B revised");
    assert_eq!(revised.translation.text, "Synthetic translation revised");
    controller.handle(LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
        utterance_id: "B".into(),
        source: "Synthetic B final".into(),
        translation: "Synthetic translation final".into(),
        language: Some("ko".into()),
    });
    let confirmed = controller.state.clone();
    controller.handle(LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair {
        utterance_id: "B".into(),
        source: "Obsolete source".into(),
        translation: "Obsolete translation".into(),
        language: Some("en".into()),
    });
    assert_eq!(controller.state, confirmed);
    controller.clear_subtitles();
    let cleared = controller.state.clone();
    controller.handle(LiveTranslateServerEvent::UtteranceRegistered {
        utterance_id: "B".into(),
    });
    controller.handle(preview("Cleared source", "Cleared translation"));
    controller.handle(LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
        utterance_id: "B".into(),
        source: "Cleared source".into(),
        translation: "Cleared translation".into(),
        language: Some("en".into()),
    });
    assert_eq!(controller.state, cleared);
}

#[test]
fn identified_pair_is_atomic_while_stopping_and_repeated_ids_remain_distinct() {
    let mut controller = TranslationSessionController::default();
    for id in ["first", "second"] {
        controller.handle(LiveTranslateServerEvent::UtteranceText {
            utterance_id: id.into(),
            role: UtteranceRole::Source,
            text: "Repeat sentence".into(),
            is_final: true,
            language: Some("en".into()),
        });
        controller.begin_stopping();
        controller.handle(LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
            utterance_id: id.into(),
            source: "Repeat sentence".into(),
            translation: "重复句子".into(),
            language: Some("en".into()),
        });
        controller.did_connect();
    }
    assert_eq!(controller.state.subtitles.history.len(), 2);
    assert!(
        controller.state.subtitles.history[0].created_at_ms
            < controller.state.subtitles.history[1].created_at_ms
    );
}

#[test]
fn replayed_confirmation_cannot_clear_pending_work_or_rewind_a_live_pair() {
    let mut controller = SourceSessionController::default();
    let replay = LiveTranslateServerEvent::SubtitleConfirmedPair {
        source_utterance_id: None,
        utterance_id: 1,
        source: "Synthetic final".into(),
        language: Some("en".into()),
        translation: "Synthetic final translation".into(),
    };
    controller.handle(replay.clone());
    controller.handle(LiveTranslateServerEvent::SubtitlePreviewPair {
        source_utterance_id: None,
        source: "New synthetic source".into(),
        language: Some("ja".into()),
        translation: "New synthetic translation".into(),
    });
    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    let expected = controller.state.clone();
    controller.handle(replay);
    assert_eq!(controller.state, expected);
    controller.handle(LiveTranslateServerEvent::SubtitleConfirmedPair {
        source_utterance_id: None,
        utterance_id: 2,
        source: " ".into(),
        language: Some("en".into()),
        translation: "Synthetic incomplete result".into(),
    });
    assert_eq!(controller.state, expected);
}

#[test]
fn oversized_events_cannot_clear_pending_recovery_or_claim_a_confirmation() {
    let mut controller = SourceSessionController::default();
    controller.handle(LiveTranslateServerEvent::SubtitlePreviewPair {
        source_utterance_id: None,
        source: "Synthetic complete source".into(),
        language: Some("ja".into()),
        translation: "Synthetic complete translation".into(),
    });
    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 7 });
    controller.state.translation_recovery =
        Some(crate::core::diagnostics::latency::TranslationRecovery {
            reason: crate::core::diagnostics::latency::TranslationRecoveryReason::RateLimited,
            retry_after_ms: 4_000,
            retry_scheduled: true,
        });
    let expected = controller.state.clone();
    let oversized = "x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1);
    for event in [
        LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 1,
            text: oversized.clone(),
            language: Some("en".into()),
        },
        LiveTranslateServerEvent::TranslationDraft(oversized.clone()),
        LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: None,
            utterance_id: 1,
            source: "valid".into(),
            language: Some("en".into()),
            translation: oversized,
        },
    ] {
        controller.handle(event);
        assert_eq!(controller.state, expected);
    }
    controller.handle(LiveTranslateServerEvent::SubtitleConfirmedPair {
        source_utterance_id: None,
        utterance_id: 1,
        source: "Synthetic valid final".into(),
        language: Some("en".into()),
        translation: "Synthetic valid translation".into(),
    });
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert!(!controller.state.is_translation_pending);
    assert!(!controller.state.is_translation_preview_pending);
}

#[test]
fn preview_http_pending_is_owner_matched_and_does_not_change_final_pairing() {
    let mut controller = SourceSessionController::default();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 1 });
    assert!(controller.state.is_translation_preview_pending);
    assert!(!controller.state.is_translation_pending);
    controller.handle(LiveTranslateServerEvent::SourceDraft {
        text: "Synthetic source".into(),
        language: Some("en".into()),
    });
    controller.handle(LiveTranslateServerEvent::TranslationDraft(
        "Synthetic partial".into(),
    ));
    assert!(controller.state.is_translation_preview_pending);
    assert!(controller.state.subtitles.history.is_empty());
    controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 2 });
    controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 1 });
    assert!(controller.state.is_translation_preview_pending);
    controller.clear_translation_pending();
    assert!(controller.state.is_translation_preview_pending);
    controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 2 });
    assert!(!controller.state.is_translation_preview_pending);
    assert!(!controller.state.is_translation_pending);
    assert!(controller.state.subtitles.history.is_empty());
}

#[test]
fn preview_cleanup_cannot_clear_a_final_and_final_start_clears_old_preview() {
    let mut controller = SourceSessionController::default();
    controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 1 });
    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    assert!(!controller.state.is_translation_preview_pending);
    assert!(controller.state.is_translation_pending);
    controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 1 });
    assert!(controller.state.is_translation_pending);
    controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 2 });
    controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 2 });
    assert!(!controller.state.is_translation_preview_pending);
    assert!(controller.state.is_translation_pending);
}

#[test]
fn preview_pending_is_cleared_on_each_session_lifecycle_boundary() {
    let boundaries: [fn(&mut SourceSessionController); 6] = [
        SourceSessionController::begin_connecting,
        SourceSessionController::did_connect,
        SourceSessionController::did_pause,
        SourceSessionController::begin_stopping,
        SourceSessionController::did_stop,
        |controller| controller.did_fail("synthetic failure"),
    ];
    for boundary in boundaries {
        let mut controller = SourceSessionController::default();
        controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 1 });
        boundary(&mut controller);
        assert!(!controller.state.is_translation_preview_pending);
        assert!(controller.preview_pending_id.is_none());
    }
}

#[test]
fn translation_backoff_is_nonterminal_and_lifecycle_or_valid_text_clears_it() {
    use crate::core::diagnostics::latency::{TranslationRecovery, TranslationRecoveryReason};
    let recovery = TranslationRecovery {
        reason: TranslationRecoveryReason::RateLimited,
        retry_after_ms: 4_000,
        retry_scheduled: true,
    };
    let mut controller = SourceSessionController::default();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    controller.handle(LiveTranslateServerEvent::TranslationDeferred(recovery));
    assert_eq!(controller.state.status, SessionStatus::Listening);
    assert!(controller.state.status.is_active());
    assert!(!controller.state.is_translation_pending);
    assert!(!controller.state.is_translation_timed_out);
    assert_eq!(controller.state.translation_recovery, Some(recovery));
    let idle_recovery = TranslationRecovery {
        retry_scheduled: false,
        ..recovery
    };
    controller.handle(LiveTranslateServerEvent::TranslationDeferred(idle_recovery));
    controller.handle(LiveTranslateServerEvent::SourceDraft {
        text: "recognition keeps running without a scheduled retry".into(),
        language: Some("en".into()),
    });
    assert_eq!(controller.state.translation_recovery, Some(idle_recovery));
    assert_eq!(controller.state.status, SessionStatus::Listening);
    assert!(!controller.state.is_translation_pending);
    controller.handle(LiveTranslateServerEvent::TranslationDeferred(recovery));
    controller.handle(LiveTranslateServerEvent::SourceDraft {
        text: "new synthetic recognition".into(),
        language: Some("en".into()),
    });
    assert_eq!(controller.state.translation_recovery, Some(recovery));
    controller.handle(LiveTranslateServerEvent::TranslationDraft(
        "valid synthetic preview".into(),
    ));
    assert_eq!(controller.state.translation_recovery, None);
    for finish in [
        SourceSessionController::did_pause,
        SourceSessionController::begin_stopping,
        SourceSessionController::did_stop,
        SourceSessionController::begin_connecting,
    ] {
        controller.handle(LiveTranslateServerEvent::TranslationDeferred(recovery));
        finish(&mut controller);
        assert_eq!(controller.state.translation_recovery, None);
    }
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::TranslationDeferred(recovery));
    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    assert_eq!(controller.state.translation_recovery, None);
    controller.handle(LiveTranslateServerEvent::TranslationDeferred(recovery));
    controller.did_fail("credential_authentication_failed");
    assert_eq!(controller.state.translation_recovery, None);
    assert!(!controller.state.status.is_active());
}

#[test]
fn session_follows_the_happy_path_lifecycle() {
    let mut controller = SourceSessionController::default();

    controller.begin_connecting();
    assert_eq!(controller.state.status, SessionStatus::Connecting);

    controller.did_connect();
    assert_eq!(controller.state.status, SessionStatus::Listening);

    controller.begin_stopping();
    assert_eq!(controller.state.status, SessionStatus::Stopping);

    controller.did_stop();
    assert_eq!(controller.state.status, SessionStatus::Idle);
}

#[test]
fn server_events_update_subtitle_state() {
    let mut controller = SourceSessionController::default();
    controller.handle(LiveTranslateServerEvent::SourceDraft {
        text: "Hello wor".into(),
        language: Some("en".into()),
    });
    controller.handle(LiveTranslateServerEvent::TranslationDraft(
        "你好，世".into(),
    ));

    assert_eq!(controller.state.subtitles.source.text, "Hello wor");
    assert!(!controller.state.subtitles.source.is_final);
    assert_eq!(controller.state.subtitles.translation.text, "你好，世");

    controller.handle(LiveTranslateServerEvent::SourceFinal {
        text: "Hello world.".into(),
        language: Some("en".into()),
    });
    controller.handle(LiveTranslateServerEvent::TranslationFinal(
        "你好，世界。".into(),
    ));

    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert!(controller.state.subtitles.translation.is_final);
    assert_eq!(
        controller.state.detected_language.as_ref().unwrap().code,
        "en"
    );
}

#[test]
fn a_new_connection_clears_the_previously_detected_language() {
    let mut controller = SourceSessionController::default();
    controller.handle(LiveTranslateServerEvent::SourceDraft {
        text: "こんにちは".into(),
        language: Some("ja".into()),
    });
    assert_eq!(
        controller.state.detected_language.as_ref().unwrap().code,
        "ja"
    );

    controller.begin_connecting();
    assert_eq!(controller.state.detected_language, None);
}

#[test]
fn service_errors_move_the_session_to_error() {
    let mut controller = SourceSessionController::default();
    controller.begin_connecting();
    controller.handle(LiveTranslateServerEvent::Error {
        code: "invalid_value".into(),
        message: "Bad language".into(),
    });

    assert_eq!(
        controller.state.status,
        SessionStatus::Error("Bad language".into())
    );
}

#[test]
fn translation_activity_follows_the_real_plus_request_lifecycle() {
    let mut controller = SourceSessionController::default();
    controller.did_connect();

    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    assert!(controller.state.is_translation_pending);

    controller.handle(LiveTranslateServerEvent::TranslationFinal(
        "翻译完成。".into(),
    ));
    assert!(!controller.state.is_translation_pending);

    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    controller.did_fail("Request failed");
    assert!(!controller.state.is_translation_pending);
}

#[test]
fn pausing_clears_translation_activity_without_discarding_subtitles() {
    let mut controller = SourceSessionController::default();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::SourceFinal {
        text: "Please wait.".into(),
        language: Some("en".into()),
    });
    controller.handle(LiveTranslateServerEvent::TranslationFinal(
        "请稍等。".into(),
    ));
    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    let subtitles_before_pause = controller.state.subtitles.clone();

    controller.did_pause();

    assert_eq!(controller.state.status, SessionStatus::Listening);
    assert!(!controller.state.is_translation_pending);
    assert_eq!(controller.state.subtitles, subtitles_before_pause);
}

#[test]
fn clearing_translation_pending_keeps_status_and_subtitles() {
    let mut controller = SourceSessionController::default();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::SourceFinal {
        text: "Hello.".into(),
        language: Some("en".into()),
    });
    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    assert!(controller.state.is_translation_pending);
    let subtitles_before = controller.state.subtitles.clone();

    // The timeout guard clears only the waiting-for-final flag.
    controller.clear_translation_pending();

    assert_eq!(controller.state.status, SessionStatus::Listening);
    assert!(!controller.state.is_translation_pending);
    assert!(controller.state.is_translation_timed_out);
    assert_eq!(controller.state.subtitles, subtitles_before);

    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    assert!(!controller.state.is_translation_timed_out);
}

#[test]
fn clearing_subtitles_does_not_change_session_status() {
    let mut controller = SourceSessionController::default();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::SourceFinal {
        text: "Hello.".into(),
        language: Some("en".into()),
    });
    controller.handle(LiveTranslateServerEvent::TranslationFinal("你好。".into()));

    controller.clear_subtitles();

    assert_eq!(controller.state.status, SessionStatus::Listening);
    assert_eq!(controller.state.subtitles, SubtitleSnapshot::empty());
}

#[test]
fn clear_resets_pending_work_without_stopping_and_new_preview_remains_owned() {
    let mut controller = SourceSessionController::default();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::TranslationStarted);
    controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 1 });
    controller.state.is_translation_timed_out = true;
    controller.state.translation_recovery =
        Some(crate::core::diagnostics::latency::TranslationRecovery {
            reason: crate::core::diagnostics::latency::TranslationRecoveryReason::RateLimited,
            retry_after_ms: 4_000,
            retry_scheduled: true,
        });
    controller.clear_subtitles();
    assert_eq!(controller.state.status, SessionStatus::Listening);
    assert!(!controller.state.is_translation_pending);
    assert!(!controller.state.is_translation_preview_pending);
    assert!(!controller.state.is_translation_timed_out);
    assert!(controller.state.translation_recovery.is_none());
    controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 2 });
    controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 1 });
    assert!(controller.state.is_translation_preview_pending);
    controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 2 });
    assert!(!controller.state.is_translation_preview_pending);
}

#[test]
fn stopping_ignores_flushed_tail_subtitles() {
    let mut controller = SourceSessionController::default();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::SourceFinal {
        text: "Last real line.".into(),
        language: Some("en".into()),
    });
    controller.handle(LiveTranslateServerEvent::TranslationFinal(
        "最后一句正常字幕。".into(),
    ));
    let subtitles_before_stopping = controller.state.subtitles.clone();

    controller.begin_stopping();
    controller.handle(LiveTranslateServerEvent::SourceFinal {
        text: "Translation mode ended.".into(),
        language: Some("en".into()),
    });
    controller.handle(LiveTranslateServerEvent::TranslationFinal(
        "翻译模式已结束。".into(),
    ));
    controller.handle(LiveTranslateServerEvent::SessionFinished);

    assert_eq!(controller.state.status, SessionStatus::Stopping);
    assert_eq!(controller.state.subtitles, subtitles_before_stopping);
}

#[test]
fn stopping_accepts_a_provider_confirmed_atomic_tail_pair() {
    let mut controller = SourceSessionController::default();
    controller.did_connect();
    controller.begin_stopping();

    controller.handle(LiveTranslateServerEvent::SubtitleFinalPair {
        source: "Confirmed tail".into(),
        language: Some("en".into()),
        translation: "已确认的尾句".into(),
    });

    assert_eq!(controller.state.status, SessionStatus::Stopping);
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert_eq!(
        controller.state.subtitles.history[0].source,
        "Confirmed tail"
    );
    assert_eq!(
        controller.state.subtitles.history[0].translation,
        "已确认的尾句"
    );
}

#[test]
fn unknown_server_events_leave_state_unchanged() {
    let mut controller = SourceSessionController::default();
    let before = controller.state.clone();
    controller.handle(LiveTranslateServerEvent::Ignored {
        kind: "response.created".into(),
    });
    assert_eq!(controller.state, before);
}

#[test]
fn atomic_pair_updates_history_and_detected_language_together() {
    let mut controller = SourceSessionController::default();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::SubtitleFinalPair {
        source: "Hello.".into(),
        language: Some("en".into()),
        translation: "你好。".into(),
    });

    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert_eq!(controller.state.subtitles.history[0].source, "Hello.");
    assert_eq!(controller.state.subtitles.history[0].translation, "你好。");
    assert_eq!(
        controller
            .state
            .detected_language
            .as_ref()
            .map(|value| value.code.as_str()),
        Some("en")
    );
}

#[test]
fn stamped_utterance_text_reaches_the_snapshot_with_its_identity() {
    let mut controller = SourceSessionController::default();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::UtteranceText {
        utterance_id: "item_source".into(),
        role: UtteranceRole::Source,
        text: "Hello.".into(),
        is_final: true,
        language: Some("en".into()),
    });
    controller.handle(LiveTranslateServerEvent::UtteranceText {
        utterance_id: "item_source".into(),
        role: UtteranceRole::Translation,
        text: "你好。".into(),
        is_final: false,
        language: None,
    });

    assert_eq!(
        controller.state.subtitles.source.utterance_id.as_deref(),
        Some("item_source")
    );
    assert_eq!(
        controller
            .state
            .subtitles
            .translation
            .utterance_id
            .as_deref(),
        Some("item_source")
    );
    assert_eq!(
        controller
            .state
            .detected_language
            .as_ref()
            .map(|value| value.code.as_str()),
        Some("en")
    );
}
