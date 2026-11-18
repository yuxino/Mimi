//! Events behavior regressions.

use super::*;

#[test]
fn translation_timeout_captures_the_first_and_replacement_deadline_per_source() {
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.did_connect();
    for source in [AudioSource::System, AudioSource::Microphone] {
        assert!(controller
            .translation_pending_deadline_from(source)
            .is_none());
        let first = apply_provider_event(
            &mut controller,
            source,
            &LiveTranslateServerEvent::TranslationStarted,
        )
        .expect("the first timer must observe the newly applied pending request");
        assert_eq!(
            controller.translation_pending_deadline_from(source),
            Some(first)
        );

        // Give the previous request a distinct deadline without sleeping.
        controller.handle_from_at(
            source,
            LiveTranslateServerEvent::TranslationStarted,
            Instant::now() - Duration::from_secs(1),
        );
        let previous = controller
            .translation_pending_deadline_from(source)
            .unwrap();
        let replacement = apply_provider_event(
            &mut controller,
            source,
            &LiveTranslateServerEvent::TranslationStarted,
        )
        .unwrap();
        assert!(replacement > previous);
        assert!(!controller.expire_translation_pending_from(source, previous));
        assert!(controller.expire_translation_pending_from(source, replacement));
        assert!(controller
            .translation_pending_deadline_from(source)
            .is_none());
    }
}

#[test]
fn translation_timeout_deadline_is_not_rearmed_by_final_or_cancelled_content() {
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.did_connect();
    let system = apply_provider_event(
        &mut controller,
        AudioSource::System,
        &LiveTranslateServerEvent::TranslationStarted,
    )
    .unwrap();
    let microphone = apply_provider_event(
        &mut controller,
        AudioSource::Microphone,
        &LiveTranslateServerEvent::TranslationStarted,
    )
    .unwrap();
    assert!(apply_provider_event(
        &mut controller,
        AudioSource::System,
        &LiveTranslateServerEvent::TranslationFinal("synthetic translation".into()),
    )
    .is_none());
    assert!(!controller.expire_translation_pending_from(AudioSource::System, system));
    assert_eq!(
        controller.translation_pending_deadline_from(AudioSource::Microphone),
        Some(microphone)
    );
    controller.clear_subtitles();
    assert!(!controller.expire_translation_pending_from(AudioSource::Microphone, microphone));
}

#[test]
fn provider_application_keeps_the_current_timer_when_an_old_pair_enters_history() {
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.did_connect();
    for source in [AudioSource::System, AudioSource::Microphone] {
        for (id, source_order) in [("previous", 1), ("current", 2)] {
            apply_provider_event(
                &mut controller,
                source,
                &LiveTranslateServerEvent::SubtitleSourceBoundary {
                    utterance_id: id.into(),
                    source_order,
                },
            );
        }
        apply_provider_event(
            &mut controller,
            source,
            &LiveTranslateServerEvent::UtteranceText {
                utterance_id: "current".into(),
                role: UtteranceRole::Source,
                text: "Synthetic current source.".into(),
                is_final: true,
                language: None,
            },
        );
        let deadline = apply_provider_event(
            &mut controller,
            source,
            &LiveTranslateServerEvent::TranslationStarted,
        )
        .unwrap();
        let pair = |id: &str| LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
            utterance_id: id.into(),
            source: format!("Synthetic {id} source."),
            translation: format!("Synthetic {id} translation."),
            language: None,
        };
        let old = apply_provider_envelope(
            &mut controller,
            source,
            &ProviderEvent::unsequenced(pair("previous"), 0),
        );
        assert!(!old.resolves_translation_timeout);
        assert!(old.translation_pending_deadline.is_none());
        assert_eq!(
            controller.translation_pending_deadline_from(source),
            Some(deadline)
        );
        assert_eq!(
            controller
                .state
                .subtitles
                .tracks
                .iter()
                .find(|track| track.audio_source == source)
                .unwrap()
                .source
                .utterance_id
                .as_deref(),
            Some("current")
        );
        let current = ProviderEvent::unsequenced(pair("current"), 0);
        assert!(
            apply_provider_envelope(&mut controller, source, &current).resolves_translation_timeout
        );
        let restarted = apply_provider_event(
            &mut controller,
            source,
            &LiveTranslateServerEvent::TranslationStarted,
        )
        .unwrap();
        assert!(
            !apply_provider_envelope(&mut controller, source, &current)
                .resolves_translation_timeout
        );
        assert_eq!(
            controller.translation_pending_deadline_from(source),
            Some(restarted)
        );
    }
    assert_eq!(controller.state.subtitles.history.len(), 4);
}

#[test]
fn source_text_without_runtime_activity_metadata_cannot_replace_the_native_timer() {
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.did_connect();
    for source in [AudioSource::System, AudioSource::Microphone] {
        let deadline = apply_provider_event(
            &mut controller,
            source,
            &LiveTranslateServerEvent::TranslationStarted,
        )
        .unwrap();
        let final_ = ProviderEvent::unsequenced(
            LiveTranslateServerEvent::UtteranceText {
                utterance_id: "synthetic-original-only".into(),
                role: UtteranceRole::Source,
                text: "Synthetic source without translation activity.".into(),
                is_final: true,
                language: None,
            },
            0,
        );
        let application = apply_provider_envelope(&mut controller, source, &final_);
        assert!(application.translation_pending_deadline.is_none());
        assert!(!application.resolves_translation_timeout);
        assert_eq!(
            controller.translation_pending_deadline_from(source),
            Some(deadline)
        );
    }
}

#[tokio::test]
async fn popped_pre_clear_event_cannot_restore_overlay_or_private_history() {
    let directory = tempfile::tempdir().unwrap();
    let history = SessionHistory::new(directory.path().to_path_buf(), false);
    let id = uuid::Uuid::new_v4().to_string();
    history.begin(&id, 0).unwrap();
    let mut controller = TranslationSessionController::default();
    controller.did_connect();
    let (sender, mut receiver) = provider_event_channel();
    let final_event = |source: &str| LiveTranslateServerEvent::SubtitleFinalPair {
        source: source.into(),
        translation: "synthetic translation".into(),
        language: None,
    };
    sender.send(final_event("synthetic old pair")).unwrap();
    // This is the race a receiver-only filter cannot fix: already popped.
    let popped = receiver.recv_with_revision().await.unwrap();
    let generation = 3;
    let content = TokioMutex::new((generation, sender.content_revision()));
    {
        let mut boundary = content.lock().await;
        history.clear_text(&id).unwrap();
        boundary.1 = sender.advance_content_revision();
        controller.clear_subtitles();
    }
    let boundary = content.lock().await;
    assert!(!subtitle_content_is_current(generation, &popped, *boundary));
    assert!(generation_accepts_event(
        NO_GENERATION,
        generation,
        generation,
        &popped.event
    ));
    // A valid stopping tail still needs the retained content revision.
    assert!(!subtitle_content_is_current(generation, &popped, *boundary));
    assert!(controller.state.subtitles.history.is_empty());
    drop(boundary);
    sender.send(final_event("synthetic next sentence")).unwrap();
    let next = receiver.recv_with_revision().await.unwrap();
    {
        let boundary = content.lock().await;
        assert!(subtitle_content_is_current(generation, &next, *boundary));
        controller.handle(next.event);
        history
            .append_pair(&id, controller.state.subtitles.history.last().unwrap())
            .unwrap();
    }
    assert_eq!(controller.state.subtitles.history.len(), 1);
    let stored = std::fs::read_to_string(directory.path().join(format!("{id}.jsonl"))).unwrap();
    assert!(!stored.contains("synthetic old pair"));
    assert!(stored.contains("synthetic next sentence"));
    // An older session's revision cannot match a new session by accident.
    assert!(!subtitle_content_is_current(
        generation + 1,
        &popped,
        (generation, 0)
    ));
    for event in [
        LiveTranslateServerEvent::SessionFinished,
        LiveTranslateServerEvent::Error {
            code: "authentication_error".into(),
            message: "fixed label".into(),
        },
    ] {
        assert!(subtitle_content_is_current(
            generation,
            &ProviderEvent::unsequenced(event, 0),
            (generation, sender.content_revision())
        ));
    }
}

#[test]
fn preview_pending_is_serialized_separately_from_final_pending() {
    let mut controller = TranslationSessionController::default();
    controller.did_connect();
    controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 42 });
    let payload = serde_json::to_value(SessionStateEvent::from(&controller.state)).unwrap();
    assert_eq!(payload["isTranslationPreviewPending"], true);
    assert_eq!(payload["isTranslationPending"], false);
    assert!(payload.get("requestId").is_none());
    controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 42 });
    let payload = serde_json::to_value(SessionStateEvent::from(&controller.state)).unwrap();
    assert_eq!(payload["isTranslationPreviewPending"], false);
}

#[test]
fn stop_rejects_old_events_but_keeps_atomic_confirmed_tail() {
    let generation = 9;
    let final_pair = LiveTranslateServerEvent::SubtitleFinalPair {
        source: "tail".into(),
        language: Some("en".into()),
        translation: "尾句".into(),
    };
    assert!(generation_accepts_event(
        NO_GENERATION,
        generation,
        generation,
        &final_pair,
    ));
    assert!(!generation_accepts_event(
        NO_GENERATION,
        generation,
        generation,
        &LiveTranslateServerEvent::TranslationDraft("stale".into()),
    ));
    let identified_pair = LiveTranslateServerEvent::SubtitleConfirmedPair {
        utterance_id: 1,
        source_utterance_id: None,
        source: "Synthetic tail".into(),
        language: Some("en".into()),
        translation: "Synthetic translation".into(),
    };
    assert!(generation_accepts_event(
        NO_GENERATION,
        generation,
        generation,
        &identified_pair
    ));
    assert!(!generation_accepts_event(
        generation + 1,
        NO_GENERATION,
        generation,
        &identified_pair
    ));
    let dashscope_pair = LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
        utterance_id: "provider-source".into(),
        source: "Synthetic identified tail".into(),
        language: Some("en".into()),
        translation: "合成有身份尾句".into(),
    };
    assert!(generation_accepts_event(
        NO_GENERATION,
        generation,
        generation,
        &dashscope_pair
    ));
    assert!(!generation_accepts_event(
        generation + 1,
        NO_GENERATION,
        generation,
        &dashscope_pair
    ));
    let registration = LiveTranslateServerEvent::UtteranceRegistered {
        utterance_id: "tencent-sentence".into(),
    };
    assert!(generation_accepts_event(
        NO_GENERATION,
        generation,
        generation,
        &registration
    ));
    assert!(!generation_accepts_event(
        generation + 1,
        NO_GENERATION,
        generation,
        &registration
    ));
}
