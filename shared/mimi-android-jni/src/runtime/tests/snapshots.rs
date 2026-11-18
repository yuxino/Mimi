//! Native snapshots regressions.

use super::*;

#[test]
fn conditional_poll_preserves_legacy_and_first_full_response() {
    let fixture = PollSession::new();
    let expected = legacy_full_response(&fixture.native);
    assert_eq!(fixture.poll(None).unwrap(), expected);
    assert_eq!(fixture.poll(Some(json!(0))).unwrap(), expected);
    assert_eq!(fixture.poll(Some(json!(u64::MAX))).unwrap(), expected);
    assert_eq!(
        fixture.poll(Some(json!(1))).unwrap(),
        json!({"unchanged":true,"version":1,"finished":false,"errorCode":null,"historySaveError":false,"historyLimited":false})
    );
}

#[test]
fn conditional_poll_refreshes_terminal_fields_without_a_controller_revision() {
    let fixture = PollSession::new();
    let first = fixture.poll(None).unwrap();
    let version = first["version"].clone();
    assert_eq!(
        fixture.poll(Some(version.clone())).unwrap()["finished"],
        false
    );
    *fixture.native.error_code.lock().unwrap() = Some("transport_error".into());
    let pending = fixture.poll(Some(version.clone())).unwrap();
    assert_eq!(pending["errorCode"], "transport_error");
    assert_eq!(pending["finished"], false);
    fixture.native.finished.store(true, Ordering::SeqCst);
    {
        let mut history = fixture.native.history_status.lock().unwrap();
        history.save_error = true;
        history.limited = true;
    }
    assert_eq!(
        fixture.poll(Some(version.clone())).unwrap(),
        json!({"unchanged":true,"version":version,"finished":true,"errorCode":"transport_error","historySaveError":true,"historyLimited":true})
    );
}

#[test]
fn conditional_poll_returns_complete_current_state_after_a_change() {
    let fixture = PollSession::new();
    let first = fixture.poll(None).unwrap();
    fixture.native.publish(|controller| {
        controller.did_connect();
        controller.handle_from(
            AudioSource::System,
            LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: Some(1),
                utterance_id: 1,
                source: "Synthetic conditional source".into(),
                translation: "Synthetic conditional translation".into(),
                language: Some("en".into()),
            },
        );
        controller.state.atomic_preview = Some(true);
        controller.state.is_translation_pending = true;
        controller.state.is_translation_preview_pending = true;
        controller.state.is_translation_timed_out = true;
        controller.state.translation_recovery = Some(TranslationRecovery {
            reason: mimi_runtime::core::diagnostics::TranslationRecoveryReason::RateLimited,
            retry_after_ms: 4000,
            retry_scheduled: true,
        });
    });
    let changed = fixture.poll(Some(first["version"].clone())).unwrap();
    assert_eq!(changed, legacy_full_response(&fixture.native));
    assert!(changed.get("unchanged").is_none());
    assert_eq!(changed["snapshot"]["history"].as_array().unwrap().len(), 1);
    assert_eq!(
        fixture.poll(Some(changed["version"].clone())).unwrap()["unchanged"],
        true
    );
}

#[test]
fn conditional_poll_observes_tick_changes_without_republishing_unchanged_ticks() {
    let fixture = PollSession::new();
    fixture.native.publish(|controller| {
        controller.handle_from(
            AudioSource::System,
            LiveTranslateServerEvent::TranslationStarted,
        );
    });
    let before = fixture.poll(None).unwrap();
    assert_eq!(before["isTranslationPending"], true);
    let now = std::time::Instant::now();
    fixture.native.tick(now + Duration::from_secs(7));
    let expired = fixture.poll(Some(before["version"].clone())).unwrap();
    assert_eq!(expired["isTranslationPending"], false);
    assert_eq!(expired["isTranslationTimedOut"], true);
    assert_eq!(
        expired["version"].as_u64().unwrap(),
        before["version"].as_u64().unwrap() + 1
    );
    fixture.native.tick(now + Duration::from_secs(8));
    assert_eq!(
        fixture.poll(Some(expired["version"].clone())).unwrap()["unchanged"],
        true
    );
}

#[test]
fn conditional_poll_rejects_invalid_versions_and_stale_handles() {
    let fixture = PollSession::new();
    for since in [json!(-1), Value::Null, json!(true), json!("1"), json!(1.5)] {
        assert_eq!(
            fixture.poll(Some(since)),
            Err("native_runtime_invalid_request")
        );
    }
    sessions().lock().unwrap().remove(&fixture.handle);
    assert_eq!(
        fixture.poll(Some(json!(1))),
        Err("native_runtime_stale_handle")
    );
}

#[test]
fn published_controller_state_and_version_stay_together_under_contention() {
    let native = fixture(configuration());
    native
        .controller
        .lock()
        .unwrap()
        .state
        .subtitles
        .source
        .text = "1".into();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let writers: Vec<_> = (0..2)
        .map(|_| {
            let native = native.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..10_000 {
                    native.publish(|controller| {
                        let next = native.version.load(Ordering::SeqCst) + 1;
                        controller.state.subtitles.source.text = next.to_string();
                        std::thread::yield_now();
                    });
                }
            })
        })
        .collect();
    barrier.wait();
    for _ in 0..2_000 {
        let snapshot = native.snapshot();
        assert_eq!(
            snapshot["snapshot"]["source"]["text"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap(),
            snapshot["version"].as_u64().unwrap(),
            "a snapshot must carry the revision of its controller state"
        );
    }
    for writer in writers {
        writer.join().unwrap();
    }
    let final_snapshot = native.snapshot();
    assert_eq!(final_snapshot["snapshot"]["source"]["text"], "20001");
    assert_eq!(final_snapshot["version"], 20_001);
}

#[test]
fn native_snapshot_preserves_shared_connection_and_translation_recovery() {
    use mimi_runtime::core::diagnostics::{TranslationRecovery, TranslationRecoveryReason};
    let native = fixture(configuration());
    native.publish(TranslationSessionController::begin_session);
    assert_eq!(native.snapshot()["connectionStatus"], "connecting");
    native.publish(TranslationSessionController::did_connect);
    native.publish(|controller| {
        controller.handle_from(
            AudioSource::System,
            LiveTranslateServerEvent::TranslationDeferred(TranslationRecovery {
                reason: TranslationRecoveryReason::RateLimited,
                retry_after_ms: 8000,
                retry_scheduled: false,
            }),
        );
    });
    let snapshot = native.snapshot();
    assert_eq!(snapshot["connectionStatus"], "listening");
    assert_eq!(snapshot["translationRecovery"]["reason"], "rateLimited");
    assert_eq!(snapshot["translationRecovery"]["retryAfterMs"], 8000);
    assert_eq!(snapshot["translationRecovery"]["retryScheduled"], false);
    native.publish(|controller| {
        controller.handle_from(
            AudioSource::System,
            LiveTranslateServerEvent::TranslationStarted,
        )
    });
    assert!(native.snapshot()["translationRecovery"].is_null());
    native.publish(TranslationSessionController::begin_stopping);
    assert_eq!(native.snapshot()["connectionStatus"], "stopping");
    native.publish(TranslationSessionController::did_stop);
    assert_eq!(native.snapshot()["connectionStatus"], "idle");
}

#[test]
fn native_snapshot_uses_the_desktop_controller_for_identified_drafts() {
    let native = fixture(configuration());
    let pair = |id: &str, source: &str, translation: &str| {
        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
            utterance_id: id.into(),
            source: source.into(),
            translation: translation.into(),
            language: Some("en".into()),
        }
    };
    native.publish(TranslationSessionController::begin_session);
    native.publish(TranslationSessionController::did_connect);
    native.publish(|controller| {
        controller.handle_from(AudioSource::System, pair("A", "Complete A", "完整 A"))
    });
    native.publish(|controller| {
        controller.handle_from(
            AudioSource::System,
            LiveTranslateServerEvent::UtteranceText {
                utterance_id: "B".into(),
                role: mimi_runtime::core::models::UtteranceRole::Source,
                text: "Draft B".into(),
                is_final: false,
                language: Some("en".into()),
            },
        )
    });
    let snapshot =
        serde_json::to_value(&native.controller.lock().unwrap().state.subtitles).unwrap();
    assert_eq!(snapshot["displayPair"]["source"], "Complete A");
    assert_eq!(snapshot["realtimePreview"]["source"]["text"], "Draft B");
    assert_eq!(snapshot["realtimePreview"]["translation"]["text"], "");
    native.publish(|controller| {
        controller.handle_from(
            AudioSource::System,
            LiveTranslateServerEvent::UtteranceText {
                utterance_id: "B".into(),
                role: mimi_runtime::core::models::UtteranceRole::Translation,
                text: "草稿 B".into(),
                is_final: false,
                language: None,
            },
        )
    });
    let snapshot =
        serde_json::to_value(&native.controller.lock().unwrap().state.subtitles).unwrap();
    assert_eq!(snapshot["realtimePreview"]["translation"]["text"], "草稿 B");
    native.publish(|controller| {
        controller.handle_from(AudioSource::System, pair("B", "Complete B", "完整 B"))
    });
    let snapshot =
        serde_json::to_value(&native.controller.lock().unwrap().state.subtitles).unwrap();
    assert_eq!(snapshot["displayPair"]["source"], "Complete B");
    assert!(snapshot["realtimePreview"].is_null());
}
