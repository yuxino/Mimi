//! Publication behavior regressions.

use super::*;
use std::future::Future;

#[test]
fn latency_payload_ignores_stopped_paused_reconnecting_and_failed_sessions() {
    let health = Some(HealthCheckLatency {
        generation: 41,
        task_id: 9,
        milliseconds: 80,
    });
    let translation = Some(TranslationLatency {
        milliseconds: 120,
        kind: TranslationLatencyKind::Request,
    });
    assert_eq!(
        visible_session_latencies(
            &SessionStatus::Listening,
            false,
            false,
            41,
            9,
            health,
            translation,
        ),
        (Some(80), Some(120), Some(TranslationLatencyKind::Request))
    );
    for (status, paused, recovering, generation) in [
        (SessionStatus::Idle, false, false, 41),
        (SessionStatus::Connecting, false, false, 41),
        (SessionStatus::Stopping, false, false, 41),
        (
            SessionStatus::Error("synthetic failure".into()),
            false,
            false,
            41,
        ),
        (SessionStatus::Listening, true, false, 41),
        (SessionStatus::Listening, false, true, 41),
        (SessionStatus::Listening, false, false, NO_GENERATION),
    ] {
        assert_eq!(
            visible_session_latencies(
                &status,
                paused,
                recovering,
                generation,
                9,
                health,
                translation,
            ),
            (None, None, None)
        );
    }
}

#[test]
fn bootstrapping_state_serializes_unknown_latencies_as_nullable_fields() {
    let state = TranslationSessionState::default();
    let payload = serde_json::to_value(SessionStateEvent::from(&state)).unwrap();
    for field in [
        "apiLatencyMs",
        "translationLatencyMs",
        "translationLatencyKind",
        "translationRecovery",
    ] {
        assert_eq!(payload.get(field), Some(&serde_json::Value::Null));
    }
}

#[test]
fn terminal_error_remains_visible_but_is_not_an_active_session() {
    let status = SessionStatus::Error("capture failed".into());

    assert!(!status.is_active());
    assert!(status_should_show_overlay(&status));
    assert!(!status_should_show_overlay(&SessionStatus::Idle));
}

#[tokio::test]
async fn idle_state_broadcast_publishes_immediately_and_batches_pending_updates() {
    let broadcasts = StateBroadcasts::default();
    let published = std::cell::RefCell::new(Vec::new());
    let latest = std::cell::Cell::new(1);
    assert!(broadcasts.request());
    latest.set(2);
    assert!(!broadcasts.request());

    let publisher = broadcasts.drain(|| {
        published.borrow_mut().push(latest.get());
        std::future::ready(())
    });
    tokio::pin!(publisher);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(publisher.as_mut().poll(&mut context).is_pending());
    assert_eq!(*published.borrow(), [2]);

    latest.set(3);
    assert!(!broadcasts.request());
    latest.set(4);
    assert!(!broadcasts.request());

    tokio::time::timeout(Duration::from_secs(2), publisher)
        .await
        .unwrap();
    assert_eq!(*published.borrow(), [2, 4]);
    assert!(broadcasts.request());
}

#[tokio::test]
async fn stop_publication_waits_until_the_last_accepted_tail_is_published() {
    let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
    controller.lock().unwrap().did_connect();
    controller.lock().unwrap().begin_stopping();
    let published = Arc::new(Mutex::new(vec![SessionStateEvent::from(
        &controller.lock().unwrap().state,
    )]));
    assert!(published.lock().unwrap()[0].subtitles.history.is_empty());
    let tail = LiveTranslateServerEvent::SubtitleConfirmedPair {
        source_utterance_id: Some(7),
        utterance_id: 1,
        source: "Synthetic stop tail".into(),
        translation: "Synthetic final translation".into(),
        language: Some("en".into()),
    };
    assert!(generation_accepts_event(NO_GENERATION, 9, 9, &tail));
    controller
        .lock()
        .unwrap()
        .handle_from(AudioSource::System, tail);
    controller.lock().unwrap().did_stop();

    // Hold the same kind of content gate used by publish_state_now. A
    // coalesced request alone cannot complete stop while delivery is gated.
    let content = Arc::new(TokioMutex::new(()));
    let held = Arc::clone(&content).lock_owned().await;
    let (requested, request_seen) = tokio::sync::oneshot::channel();
    let publication_controller = Arc::clone(&controller);
    let publication_output = Arc::clone(&published);
    let mut stopped = tokio::spawn(async move {
        publish_stop_boundary(|| requested.send(()).unwrap(), async move {
            let _content = content.lock().await;
            publication_output
                .lock()
                .unwrap()
                .push(SessionStateEvent::from(
                    &publication_controller.lock().unwrap().state,
                ));
        })
        .await;
    });
    request_seen.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut stopped)
            .await
            .is_err(),
        "stop returned while only the old stopping/history=0 snapshot was published"
    );
    // No std controller guard is held while waiting for publication.
    assert!(controller.try_lock().is_ok());
    drop(held);
    tokio::time::timeout(Duration::from_millis(200), stopped)
        .await
        .unwrap()
        .unwrap();
    let output = published.lock().unwrap();
    assert_eq!(output.len(), 2);
    let final_state = &output[1];
    assert!(matches!(final_state.status, StatusPayload::Idle));
    assert_eq!(final_state.subtitles.history.len(), 1);
    assert_eq!(
        final_state.subtitles.history[0].source,
        "Synthetic stop tail"
    );
    assert_eq!(
        final_state.subtitles.history[0].translation,
        "Synthetic final translation"
    );
}
