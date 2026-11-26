#[tokio::test]
async fn handshake_authentication_rejections_preserve_transport_failures() {
    for (status, expected_auth_label) in
        crate::clients::transport::connection_diagnostics::handshake_failure_cases()
    {
        let (endpoint, server) =
            crate::clients::transport::connection_diagnostics::rejected_websocket_endpoint(status)
                .await;
        let (events, _receiver) = provider_event_channel();
        let mut client = XAIRealtimeClient::with_endpoint(
            "test-key-not-real",
            TargetLanguage::Japanese,
            events,
            endpoint,
        )
        .unwrap();
        client.network = crate::clients::transport::provider_network::ProviderNetwork::resolve(
            &crate::core::services::network_proxy::ProxyConfig {
                mode: crate::core::services::network_proxy::ProxyMode::Direct,
                url: None,
            },
        )
        .unwrap();
        let error = client.connect().await.unwrap_err();
        server.await.unwrap();
        if let Some(expected_label) = expected_auth_label {
            assert_eq!(error, XAIRealtimeClientError::AuthenticationFailed);
            assert_eq!(error.to_string(), expected_label);
        } else {
            assert_eq!(error, XAIRealtimeClientError::TransportFailure);
        }
        assert!(!error.to_string().contains("private-handshake-body"));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn clear_waits_for_real_response_boundary_then_accepts_the_next_item() {
    let (sender, mut receiver) = provider_event_channel();
    let endpoint = url::Url::parse("ws://127.0.0.1:1/realtime").unwrap();
    let client = XAIRealtimeClient::with_endpoint(
        "test-key-not-real",
        TargetLanguage::Japanese,
        sender.clone(),
        endpoint,
    )
    .unwrap();
    {
        let mut turn = client.inner.turn.lock().await;
        turn.update_source("old source".into(), Some("old-item".into()), None, true);
        turn.response_started(Some("old-response".into()));
        turn.append_translation("old translation".into(), Some("old-response".into()));
    }
    sender
        .send(LiveTranslateServerEvent::Error {
            code: "transport_error".into(),
            message: GENERIC_TRANSPORT_ERROR.into(),
        })
        .unwrap();
    assert_eq!(client.clear_content().await, 1);
    assert_eq!(client.content_revision(), 1);
    let mut turn = client.inner.turn.lock().await;
    assert!(turn.source.is_empty() && turn.translation.is_empty());
    assert!(turn
        .append_translation("late old".into(), Some("old-response".into()))
        .is_empty());
    assert!(turn
        .complete_translation(Some("late old final".into()), Some("old-response".into()))
        .is_empty());
    turn.update_source("new source".into(), Some("new-item".into()), None, true);
    assert!(turn.response_done(Some("old-response".into())).is_empty());
    assert!(!turn.discard_current_turn);
    turn.response_started(Some("new-response".into()));
    let previews =
        turn.complete_translation(Some("new translation".into()), Some("new-response".into()));
    assert!(!previews.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
    )));
    let events = turn.response_done(Some("new-response".into()));
    assert!(events.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {source,translation,..}
        if source == "new source" && translation == "new translation")));
    drop(turn);
    assert!(matches!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::Error { .. })
    ));
}

use super::*;
use crate::clients::transport::provider_events::{provider_event_channel, ProviderEventReceiver};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use tokio::net::TcpListener;

async fn test_client(
    server: impl FnOnce(
            WebSocketStream<TcpStream>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + 'static,
) -> (XAIRealtimeClient, ProviderEventReceiver) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        server(socket).await;
    });
    let (events, receiver) = provider_event_channel();
    let endpoint = url::Url::parse(&format!("ws://{address}/realtime")).unwrap();
    let client = XAIRealtimeClient::with_endpoint(
        "xai-test-key-not-real",
        TargetLanguage::Japanese,
        events,
        endpoint,
    )
    .unwrap();
    (client, receiver)
}

fn setup_ack() -> Message {
    Message::Text(
        r#"{"type":"session.updated","session":{"reasoning":{"effort":"none"},"turn_detection":{"type":"server_vad"},"audio":{"input":{"format":{"type":"audio/pcm","rate":24000},"transcription":{"model":"grok-transcribe"}}}}}"#
            .into(),
    )
}

#[tokio::test]
async fn pre_ready_provider_errors_keep_typed_failure() {
    let mut observed = Vec::new();
    let mut expected = Vec::new();
    for (kind, code, related, failure) in [
        (
            "authentication_error",
            "invalid_api_key",
            false,
            XAIRealtimeClientError::AuthenticationFailed,
        ),
        (
            "authorization_error",
            "permission_denied",
            false,
            XAIRealtimeClientError::AuthenticationFailed,
        ),
        (
            "authentication_error",
            "key_expired",
            false,
            XAIRealtimeClientError::AuthenticationFailed,
        ),
        (
            "authorization_error",
            "invalid_event",
            true,
            XAIRealtimeClientError::AuthenticationFailed,
        ),
        (
            "invalid_request_error",
            "insufficient_quota",
            false,
            XAIRealtimeClientError::SessionSetupRejected,
        ),
        (
            "invalid_request_error",
            "invalid_event",
            false,
            XAIRealtimeClientError::SessionSetupRejected,
        ),
        (
            "server_error",
            "unknown_service_code",
            false,
            XAIRealtimeClientError::SessionSetupRejected,
        ),
        (
            "server_error",
            "unknown_service_code",
            true,
            XAIRealtimeClientError::SessionSetupRejected,
        ),
    ] {
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let update = socket.next().await.unwrap().unwrap();
                let update: Value = serde_json::from_str(update.to_text().unwrap()).unwrap();
                let mut error = serde_json::json!({
                    "type": kind,
                    "code": code,
                    "message": "private provider details",
                });
                if related {
                    error["event_id"] = update["event_id"].clone();
                }
                socket
                    .send(Message::Text(
                        serde_json::json!({"type": "error", "error": error})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                while socket.next().await.is_some() {}
            })
        })
        .await;
        observed.push(
            client
                .connect_with_timeout(Duration::from_millis(100))
                .await,
        );
        expected.push(Err(failure));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
        assert!(events.try_recv().is_err());
        client.disconnect().await;
    }
    assert_eq!(observed, expected);
}

#[tokio::test]
async fn active_authentication_errors_keep_the_fixed_terminal_label() {
    for (kind, code, expected) in [
        (
            "invalid_request_error",
            "invalid_api_key",
            "credential_authentication_failed",
        ),
        (
            "authentication_error",
            "key_expired",
            "credential_authentication_failed",
        ),
        (
            "authorization_error",
            "invalid_event",
            "credential_authentication_failed",
        ),
        (
            "authentication_error!",
            "unknown_code",
            "xai_provider_error.unknown_code",
        ),
    ] {
        let (release, released) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                released.await.unwrap();
                socket.send(Message::Text(serde_json::json!({
                    "type": "error", "error": {
                        "type": kind, "code": code,
                        "message": "private provider detail authentication_error"
                    }
                }).to_string().into())).await.unwrap();
                socket.send(Message::Text(
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"late","transcript":"must not be processed"}"#.into()
                )).await.unwrap();
                while socket.next().await.is_some() {}
            })
        }).await;
        client.connect().await.unwrap();
        release.send(()).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        let receive_task = client.inner.receive_task.lock().await.take().unwrap();
        tokio::time::timeout(Duration::from_secs(1), receive_task)
            .await
            .unwrap()
            .unwrap();
        assert!(events.try_recv().is_err());
        client.disconnect().await;
        assert_eq!(
            event,
            LiveTranslateServerEvent::Error {
                code: expected.into(),
                message: GENERIC_PROVIDER_ERROR.into(),
            },
            "{kind}/{code}"
        );
        assert!(!crate::core::lifecycle::recovery::provider_error_is_retryable(expected));
    }
}

#[tokio::test]
async fn pre_ready_recoverable_error_for_other_request_keeps_waiting() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await.unwrap().unwrap();
            socket.send(Message::Text(r#"{"type":"error","error":{"type":"invalid_request_error","code":"invalid_event","event_id":"unrelated-fixture-request"}}"#.into())).await.unwrap();
            socket.send(setup_ack()).await.unwrap();
            while socket.next().await.is_some() {}
        })
    }).await;
    assert_eq!(
        client.connect_with_timeout(Duration::from_secs(1)).await,
        Ok(())
    );
    assert!(client.inner.ready.load(Ordering::SeqCst));
    assert!(events.try_recv().is_err());
    client.disconnect().await;
}

#[tokio::test]
async fn pre_ready_socket_close_remains_a_transport_failure() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.close(None).await.unwrap();
        })
    })
    .await;
    assert_eq!(
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await,
        Err(XAIRealtimeClientError::TransportFailure),
    );
    assert!(events.try_recv().is_err());
    client.disconnect().await;
}

#[tokio::test]
async fn configuration_mismatch_after_ready_emits_a_terminal_error() {
    let (release, released) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            released.await.unwrap();
            let mut mismatch: Value = serde_json::from_str(setup_ack().to_text().unwrap()).unwrap();
            mismatch["session"]["audio"]["input"]["format"]["rate"] = 16_000.into();
            socket
                .send(Message::Text(mismatch.to_string().into()))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        })
    })
    .await;
    client.connect().await.unwrap();
    release.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_millis(200), events.recv()).await;
    client.disconnect().await;
    assert!(matches!(
        result,
        Ok(Some(LiveTranslateServerEvent::Error { code, message }))
            if code == "xai_session_configuration_mismatch"
                && message == GENERIC_PROVIDER_ERROR
    ));
}

#[tokio::test]
async fn interrupted_response_cannot_pair_its_translation_with_the_next_source() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            for event in [
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"First source."}"#,
                r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                r#"{"type":"response.output_audio_transcript.delta","response_id":"response_a","delta":"First translation."}"#,
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_b","transcript":"Second source."}"#,
                r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"First translation."}"#,
                r#"{"type":"response.done","response":{"id":"response_a","status":"cancelled"}}"#,
                r#"{"type":"response.created","response":{"id":"response_b"}}"#,
                r#"{"type":"response.output_audio_transcript.done","response_id":"response_b","transcript":"Second translation."}"#,
                r#"{"type":"response.done","response":{"id":"response_b","status":"completed"}}"#,
            ] {
                socket.send(Message::Text(event.into())).await.unwrap();
            }
            while socket.next().await.is_some() {}
        })
    }).await;
    client.connect().await.unwrap();
    let mut pairs = Vec::new();
    let mut errors = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await
    {
        match event {
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                source,
                translation,
                ..
            } => {
                pairs.push((source, translation));
            }
            LiveTranslateServerEvent::Error { code, .. } => errors.push(code),
            _ => {}
        }
    }
    client.disconnect().await;
    assert_eq!(
        pairs,
        [("Second source.".into(), "Second translation.".into())]
    );
    assert!(
        errors.is_empty(),
        "normal VAD cancellation must not fail the session"
    );
}

#[tokio::test]
async fn failed_response_does_not_confirm_its_completed_transcript() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            for event in [
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"First source."}"#,
                r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"Unconfirmed translation."}"#,
                r#"{"type":"response.done","response":{"id":"response_a","status":"failed"}}"#,
            ] {
                socket.send(Message::Text(event.into())).await.unwrap();
            }
            while socket.next().await.is_some() {}
        })
    }).await;
    client.connect().await.unwrap();
    let mut pairs = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await
    {
        if let LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. } = event {
            pairs.push(event);
        }
    }
    client.disconnect().await;
    assert!(pairs.is_empty());
}

#[tokio::test]
async fn silence_only_audio_finishes_without_a_response_timeout_error() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            while socket.next().await.is_some() {}
        })
    })
    .await;
    client.connect().await.unwrap();
    client
        .send_audio(&vec![0; XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    client.finish(Duration::from_millis(40)).await;
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
}

#[test]
fn transcript_safety_limit_discards_same_turn_residue_without_cross_pairing() {
    let mut turn = GrokTurnState::default();
    let source_error = turn.update_source(
        "s".repeat(MAXIMUM_TRANSCRIPT_BYTES + 1),
        Some("oversized_source".into()),
        None,
        false,
    );
    assert!(matches!(
        source_error.as_slice(),
        [LiveTranslateServerEvent::Error { code, message }]
            if code == "xai_transcript_safety_limit"
                && message == "xAI Grok Voice transcript buffering exceeded its safety limit."
    ));
    assert!(turn.source.is_empty());
    assert!(turn.translation.is_empty());
    assert!(turn.item_id.is_none());
    assert!(turn.discard_current_turn);
    assert_eq!(turn.discarded_item_id.as_deref(), Some("oversized_source"));

    assert!(turn
        .update_source(
            "stale source tail".into(),
            Some("oversized_source".into()),
            None,
            true,
        )
        .is_empty());

    let source_events = turn.update_source(
        "Next sentence.".into(),
        Some("next_source".into()),
        Some("en".into()),
        true,
    );
    assert!(matches!(
        source_events.as_slice(),
        [LiveTranslateServerEvent::UtteranceText { text, language, utterance_id, role: UtteranceRole::Source, is_final: false }]
            if text == "Next sentence." && language.as_deref() == Some("en")
                && utterance_id == "next_source"
    ));
    assert!(turn.discard_current_turn);
    assert!(turn.source.is_empty());

    turn.response_started(Some("old_response".into()));
    assert!(turn
        .append_translation("stale translation tail".into(), Some("old_response".into()))
        .is_empty());
    assert!(turn
        .complete_translation(
            Some("stale final translation".into()),
            Some("old_response".into()),
        )
        .is_empty());
    assert!(turn.response_done(Some("old_response".into())).is_empty());
    assert!(!turn.discard_current_turn);
    assert_eq!(turn.item_id.as_deref(), Some("next_source"));
    assert_eq!(turn.source, "Next sentence.");
    assert!(turn.has_unfinished_turn());

    turn.response_started(Some("next_response".into()));
    let previews = turn.complete_translation(Some("次の文。".into()), Some("next_response".into()));
    assert!(
        matches!(previews.as_slice(), [LiveTranslateServerEvent::UtteranceText { text, utterance_id, role: UtteranceRole::Translation, is_final: false, .. }] if text == "次の文。" && utterance_id == "next_source")
    );
    let pair = turn.response_done(Some("next_response".into()));
    assert!(matches!(
        pair.as_slice(),
        [LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { source, language, translation: final_translation, .. }]
            if source == "Next sentence."
                && language.as_deref() == Some("en")
                && final_translation == "次の文。"
    ));

    let source_bytes = MAXIMUM_TRANSCRIPT_BYTES / 2;
    let _ = turn.update_source(
        "s".repeat(source_bytes),
        Some("combined_turn".into()),
        None,
        true,
    );
    let combined_error = turn.append_translation(
        "t".repeat(MAXIMUM_TRANSCRIPT_BYTES - source_bytes + 1),
        Some("combined_response".into()),
    );
    assert!(matches!(
        combined_error.as_slice(),
        [LiveTranslateServerEvent::Error { code, .. }]
            if code == "xai_transcript_safety_limit"
    ));
    assert!(turn.source.is_empty());
    assert!(turn.translation.is_empty());
    assert_eq!(turn.response_id.as_deref(), Some("combined_response"));
    assert!(turn.discard_current_turn);
    assert_eq!(turn.discarded_item_id.as_deref(), Some("combined_turn"));

    let mut unidentified_turn = GrokTurnState::default();
    let _ = unidentified_turn.update_source(
        "s".repeat(MAXIMUM_TRANSCRIPT_BYTES + 1),
        None,
        None,
        false,
    );
    assert!(unidentified_turn
        .update_source(
            "cannot prove this is a new turn".into(),
            Some("candidate_item".into()),
            None,
            true,
        )
        .is_empty());
    assert!(unidentified_turn
        .response_done(Some("discarded_response".into()))
        .is_empty());
    assert!(!unidentified_turn.discard_current_turn);
    assert!(!unidentified_turn
        .update_source(
            "Safe after response.done.".into(),
            Some("confirmed_new_item".into()),
            None,
            true,
        )
        .is_empty());
}

#[test]
fn successful_response_waits_for_its_source_and_cannot_clear_the_next_turn() {
    let mut turn = GrokTurnState::default();
    turn.input_boundary(Some("source_a".into()), true);
    turn.response_started(Some("response_a".into()));
    assert!(turn
        .complete_translation(Some("Translation A.".into()), Some("response_a".into()))
        .iter()
        .all(|event| !matches!(
            event,
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
        )));
    assert!(turn.response_done(Some("response_a".into())).is_empty());
    turn.input_boundary(Some("source_b".into()), false);
    let events = turn.update_source("Source A.".into(), Some("source_a".into()), None, true);
    assert!(events.iter().any(|event| matches!(event,
        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {source, translation, ..}
            if source == "Source A." && translation == "Translation A.")));
    assert_eq!(turn.item_id.as_deref(), Some("source_b"));
    assert!(turn.has_unfinished_turn());
    assert!(turn.response_done(Some("response_a".into())).is_empty());
    assert!(!turn.response_started(Some("response_a".into())));
    assert_eq!(turn.item_id.as_deref(), Some("source_b"));
    assert!(turn.source.is_empty() && turn.translation.is_empty());
    assert!(turn.has_unfinished_turn());
}

#[test]
fn a_response_without_a_known_input_boundary_cannot_borrow_a_later_source() {
    let mut turn = GrokTurnState::default();
    assert!(turn
        .append_translation("Unknown translation.".into(), Some("response_a".into()))
        .is_empty());
    assert!(turn
        .update_source("Later source.".into(), Some("source_b".into()), None, true)
        .is_empty());
    assert!(turn.response_done(Some("response_a".into())).is_empty());
    assert!(turn.source.is_empty() && turn.translation.is_empty());
}

#[test]
fn identified_lanes_project_early_previews_without_crossing_overlapping_turns() {
    let mut turn = GrokTurnState::default();
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    controller.set_atomic_preview(false);
    controller.did_connect();
    let apply = |controller: &mut crate::core::lifecycle::session::TranslationSessionController,
                 events: Vec<LiveTranslateServerEvent>| {
        for event in events {
            controller.handle(event);
        }
    };
    apply(
        &mut controller,
        turn.update_source(
            "Source A.".into(),
            Some("source_a".into()),
            Some("en".into()),
            true,
        ),
    );
    turn.response_started(Some("response_a".into()));
    apply(
        &mut controller,
        turn.append_translation("Translation A".into(), Some("response_a".into())),
    );
    let early = controller
        .state
        .subtitles
        .realtime_preview
        .as_ref()
        .unwrap();
    assert_eq!(early.source.text, "Source A.");
    assert_eq!(early.translation.text, "Translation A");
    assert!(controller.state.subtitles.history.is_empty());

    apply(
        &mut controller,
        turn.update_source(
            "Source B growing".into(),
            Some("source_b".into()),
            Some("ko".into()),
            false,
        ),
    );
    let overlapping = controller
        .state
        .subtitles
        .realtime_preview
        .as_ref()
        .unwrap();
    assert_eq!(overlapping.source.text, "Source B growing");
    assert!(
        overlapping.translation.text.is_empty(),
        "A translation cannot accompany B source"
    );
    apply(
        &mut controller,
        turn.complete_translation(Some("Translation A.".into()), Some("response_a".into())),
    );
    assert_eq!(controller.state.subtitles.source.text, "Source B growing");
    apply(
        &mut controller,
        turn.response_done(Some("response_a".into())),
    );
    assert_eq!(
        controller.state.subtitles.source.text, "Source B growing",
        "late A final must preserve B's current lane"
    );
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert_eq!(
        controller.state.detected_language.as_ref().unwrap().code,
        "ko"
    );

    apply(
        &mut controller,
        turn.update_source(
            "Source B.".into(),
            Some("source_b".into()),
            Some("ko".into()),
            true,
        ),
    );
    turn.response_started(Some("response_b".into()));
    apply(
        &mut controller,
        turn.append_translation("Translation B".into(), Some("response_b".into())),
    );
    let early_b = controller
        .state
        .subtitles
        .realtime_preview
        .as_ref()
        .unwrap();
    assert_eq!(early_b.source.utterance_id.as_deref(), Some("source_b"));
    assert_eq!(
        early_b.translation.utterance_id.as_deref(),
        Some("source_b")
    );
    assert_eq!(early_b.translation.text, "Translation B");
    assert_eq!(controller.state.subtitles.history.len(), 1);
    apply(
        &mut controller,
        turn.complete_translation(Some("Translation B.".into()), Some("response_b".into())),
    );
    apply(
        &mut controller,
        turn.response_done(Some("response_b".into())),
    );
    assert_eq!(controller.state.subtitles.history.len(), 2);
    let pair = controller.state.subtitles.display_pair.as_ref().unwrap();
    assert_eq!(pair.utterance_id.as_deref(), Some("source_b"));
    assert_eq!(pair.source, "Source B.");
    assert_eq!(pair.translation, "Translation B.");
}

#[test]
fn clear_retires_pending_input_before_late_completions_reach_the_real_projection() {
    let mut turn = GrokTurnState::default();
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    controller.set_atomic_preview(false);
    controller.did_connect();
    let apply = |controller: &mut crate::core::lifecycle::session::TranslationSessionController,
                 events: Vec<LiveTranslateServerEvent>| {
        for event in events {
            controller.handle(event);
        }
    };
    apply(
        &mut controller,
        turn.update_source("Source A.".into(), Some("source_a".into()), None, true),
    );
    turn.response_started(Some("response_a".into()));
    apply(
        &mut controller,
        turn.append_translation("Translation A".into(), Some("response_a".into())),
    );
    turn.input_boundary(Some("source_b".into()), false);
    apply(
        &mut controller,
        turn.update_source(
            "Source B growing".into(),
            Some("source_b".into()),
            None,
            false,
        ),
    );
    turn.clear_content();
    controller.clear_subtitles();

    assert!(
        turn.update_source("Source B.".into(), Some("source_b".into()), None, true)
            .is_empty(),
        "cleared pending B must not be admitted again"
    );
    apply(
        &mut controller,
        turn.response_done(Some("response_a".into())),
    );
    turn.response_started(Some("response_b".into()));
    apply(
        &mut controller,
        turn.complete_translation(Some("Translation B.".into()), Some("response_b".into())),
    );
    apply(
        &mut controller,
        turn.response_done(Some("response_b".into())),
    );
    assert!(controller.state.subtitles.history.is_empty());
    assert!(controller.state.subtitles.source.text.is_empty());
    assert!(controller.state.subtitles.translation.text.is_empty());

    apply(
        &mut controller,
        turn.update_source("Source C.".into(), Some("source_c".into()), None, true),
    );
    turn.response_started(Some("response_c".into()));
    apply(
        &mut controller,
        turn.complete_translation(Some("Translation C.".into()), Some("response_c".into())),
    );
    apply(
        &mut controller,
        turn.response_done(Some("response_c".into())),
    );
    assert_eq!(controller.state.subtitles.history.len(), 1);
    let pair = controller.state.subtitles.display_pair.as_ref().unwrap();
    assert_eq!(pair.source, "Source C.");
    assert_eq!(pair.translation, "Translation C.");
    assert!(
        turn.source.is_empty() && turn.translation.is_empty() && turn.pending_next_source.is_none()
    );
    assert!(turn.retired_source_ids.len() <= 4 && turn.retired_response_ids.len() <= 4);
}

#[tokio::test]
async fn discarded_pending_input_never_lends_its_response_after_a_safety_limit() {
    for scenario in [
        "current_translation",
        "pending_source",
        "extra_pending_source",
    ] {
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                for event in [
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Source A."}"#,
                    r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                    r#"{"type":"conversation.item.input_audio_transcription.updated","item_id":"source_b","transcript":"Source B growing"}"#,
                ] { socket.send(Message::Text(event.into())).await.unwrap(); }
                let oversized = "s".repeat(MAXIMUM_TRANSCRIPT_BYTES + 1);
                let oversized_event = if scenario == "current_translation" {
                    serde_json::json!({"type":"response.output_audio_transcript.delta", "response_id":"response_a", "delta":oversized})
                } else {
                    let id = if scenario == "pending_source" {"source_b"} else {"source_c"};
                    serde_json::json!({"type":"conversation.item.input_audio_transcription.updated", "item_id":id, "transcript":oversized})
                };
                socket.send(Message::Text(oversized_event.to_string().into())).await.unwrap();
                for event in [
                    r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#,
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_c","transcript":"Source C."}"#,
                    r#"{"type":"response.created","response":{"id":"response_b"}}"#,
                    r#"{"type":"response.output_audio_transcript.done","response_id":"response_b","transcript":"Translation B."}"#,
                    r#"{"type":"response.done","response":{"id":"response_b","status":"completed"}}"#,
                ] { if socket.send(Message::Text(event.into())).await.is_err() { break; } }
                while socket.next().await.is_some() {}
            })
        }).await;
        client.connect().await.unwrap();
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        controller.did_connect();
        let mut recovery_errors = 0;
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), events.recv()).await
        {
            if let LiveTranslateServerEvent::Error { code, .. } = &event {
                if code == "transport_error" {
                    recovery_errors += 1;
                } else {
                    assert_eq!(code, "xai_transcript_safety_limit");
                }
            }
            controller.handle(event);
        }
        assert!(
            controller.state.subtitles.history.is_empty(),
            "{scenario}: C must never borrow discarded B's response"
        );
        assert_eq!(
            recovery_errors, 1,
            "{scenario}: recover once when response ownership is ambiguous"
        );
        let turn = client.inner.turn.lock().await;
        assert!(
            turn.source.is_empty()
                && turn.translation.is_empty()
                && turn.pending_next_source.is_none()
        );
        drop(turn);
        client.disconnect().await;
    }
}

#[tokio::test]
async fn progress_transcripts_wait_for_actual_completion_after_response_done() {
    let (complete_sent, complete_received) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            for event in [
                r#"{"type":"input_audio_buffer.committed","item_id":"source_a"}"#,
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Partial","status":"in_progress"}"#,
                r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"Complete translation."}"#,
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Partial source","status":"in_progress"}"#,
                r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#,
            ] { socket.send(Message::Text(event.into())).await.unwrap(); }
            complete_received.await.unwrap();
            socket.send(Message::Text(r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Actual complete source.","status":"completed"}"#.into())).await.unwrap();
            while socket.next().await.is_some() {}
        })
    }).await;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    controller.set_atomic_preview(false);
    controller.did_connect();
    client.connect().await.unwrap();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await
    {
        controller.handle(event);
    }
    assert!(
        controller.state.subtitles.history.is_empty(),
        "in_progress source must not confirm at response.done"
    );
    assert!(!client.inner.turn.lock().await.source_complete);
    complete_sent.send(()).unwrap();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await
    {
        controller.handle(event);
    }
    assert_eq!(controller.state.subtitles.history.len(), 1);
    let pair = controller.state.subtitles.display_pair.as_ref().unwrap();
    assert_eq!(pair.source, "Actual complete source.");
    assert_eq!(pair.translation, "Complete translation.");
    client.disconnect().await;
}

#[tokio::test]
async fn progress_only_sources_never_become_durable_when_finishing() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            for event in [
                r#"{"type":"input_audio_buffer.committed","item_id":"source_a"}"#,
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Partial","status":"in_progress"}"#,
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Partial source","status":"in_progress"}"#,
                r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"Unconfirmed translation."}"#,
                r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#,
            ] { socket.send(Message::Text(event.into())).await.unwrap(); }
            while socket.next().await.is_some() {}
        })
    }).await;
    client.connect().await.unwrap();
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    controller.did_connect();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await
    {
        controller.handle(event);
    }
    client.finish(Duration::from_millis(40)).await;
    while let Ok(event) = events.try_recv() {
        controller.handle(event);
    }
    assert!(controller.state.subtitles.history.is_empty());
    assert!(matches!(
        controller.state.status,
        crate::core::models::SessionStatus::Error(_)
    ));
}

#[tokio::test]
async fn clear_with_a_new_source_before_the_cleared_response_recovers_without_cross_pairing() {
    let (clear_sent, clear_received) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            for event in [
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Source A."}"#,
                r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                r#"{"type":"input_audio_buffer.speech_started","item_id":"source_b"}"#,
                r#"{"type":"conversation.item.input_audio_transcription.updated","item_id":"source_b","transcript":"Source B growing"}"#,
            ] { socket.send(Message::Text(event.into())).await.unwrap(); }
            clear_received.await.unwrap();
            for event in [
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_c","transcript":"Source C."}"#,
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_b","transcript":"Source B."}"#,
                r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#,
                r#"{"type":"response.created","response":{"id":"response_b"}}"#,
                r#"{"type":"response.output_audio_transcript.done","response_id":"response_b","transcript":"Translation B."}"#,
                r#"{"type":"response.done","response":{"id":"response_b","status":"completed"}}"#,
            ] { if socket.send(Message::Text(event.into())).await.is_err() { break; } }
            while socket.next().await.is_some() {}
        })
    }).await;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    controller.set_atomic_preview(false);
    controller.did_connect();
    client.connect().await.unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        let pending_b = matches!(&event, LiveTranslateServerEvent::UtteranceText { utterance_id, role:UtteranceRole::Source, .. } if utterance_id == "source_b");
        controller.handle(event);
        if pending_b {
            break;
        }
    }
    client.clear_content().await;
    controller.clear_subtitles();
    clear_sent.send(()).unwrap();
    let mut recovery_errors = 0;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await
    {
        if let LiveTranslateServerEvent::Error { code, message } = &event {
            assert_eq!(code, "transport_error");
            assert_eq!(message, GENERIC_PROTOCOL_ERROR);
            recovery_errors += 1;
        }
        controller.handle(event);
    }
    assert!(
        controller.state.subtitles.history.is_empty(),
        "C cannot borrow the cleared B response"
    );
    assert_eq!(recovery_errors, 1);
    client.disconnect().await;

    // The normal reconnect resets only bounded provider state. The same
    // real C item can then preview and confirm with its own response.
    let (reconnected, mut new_events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            for event in [
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_c","transcript":"Source C."}"#,
                r#"{"type":"response.created","response":{"id":"response_c"}}"#,
                r#"{"type":"response.output_audio_transcript.done","response_id":"response_c","transcript":"Translation C."}"#,
                r#"{"type":"response.done","response":{"id":"response_c","status":"completed"}}"#,
            ] { socket.send(Message::Text(event.into())).await.unwrap(); }
            while socket.next().await.is_some() {}
        })
    }).await;
    controller.begin_connecting();
    controller.did_connect();
    reconnected.connect().await.unwrap();
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(50), new_events.recv()).await
    {
        controller.handle(event);
    }
    assert_eq!(controller.state.subtitles.history.len(), 1);
    let pair = controller.state.subtitles.display_pair.as_ref().unwrap();
    assert_eq!(pair.utterance_id.as_deref(), Some("source_c"));
    assert_eq!(pair.source, "Source C.");
    assert_eq!(pair.translation, "Translation C.");
    reconnected.disconnect().await;
}

#[tokio::test]
async fn ambiguous_following_items_trigger_one_recovery_without_publishing_a_wrong_final() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            for event in [
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"First source."}"#,
                r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_b","transcript":"Second source."}"#,
                r#"{"type":"input_audio_buffer.speech_started","item_id":"source_c"}"#,
                r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"First translation."}"#,
                r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#,
            ] {
                if socket.send(Message::Text(event.into())).await.is_err() { break; }
            }
            while socket.next().await.is_some() {}
        })
    }).await;
    client.connect().await.unwrap();
    let mut recovery_errors = 0;
    let mut pairs = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await
    {
        match event {
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. } => pairs.push(event),
            LiveTranslateServerEvent::Error { code, message } => {
                assert_eq!(code, "transport_error");
                assert_eq!(message, GENERIC_PROTOCOL_ERROR);
                assert!(crate::core::lifecycle::recovery::provider_error_is_retryable(&code));
                recovery_errors += 1;
            }
            _ => {}
        }
    }
    assert_eq!(recovery_errors, 1);
    assert!(pairs.is_empty());
    let turn = client.inner.turn.lock().await;
    assert!(turn.source.is_empty() && turn.translation.is_empty());
    assert!(turn.pending_next_source.is_none());
    drop(turn);
    client.disconnect().await;
}

#[tokio::test]
async fn an_old_response_done_cannot_close_before_trailing_audio_gets_its_input_boundary() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            let _first_audio = socket.next().await;
            for event in [
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"First source."}"#,
                r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"First translation."}"#,
            ] { socket.send(Message::Text(event.into())).await.unwrap(); }
            let _trailing_audio = socket.next().await;
            socket.send(Message::Text(r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#.into())).await.unwrap();
            // Sending audio and completing A do not prove the server has
            // delivered B's VAD/input events. Delay them until after tail.
            for _ in 0..FINISH_TAIL_FRAME_COUNT {
                let _tail = socket.next().await;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            for event in [
                r#"{"type":"input_audio_buffer.speech_started","item_id":"source_b"}"#,
                r#"{"type":"input_audio_buffer.committed","item_id":"source_b"}"#,
                r#"{"type":"response.created","response":{"id":"response_b"}}"#,
                r#"{"type":"response.output_audio_transcript.done","response_id":"response_b","transcript":"Second translation."}"#,
                r#"{"type":"response.done","response":{"id":"response_b","status":"completed"}}"#,
                r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_b","transcript":"Second source."}"#,
            ] { socket.send(Message::Text(event.into())).await.unwrap(); }
            while socket.next().await.is_some() {}
        })
    }).await;
    client.connect().await.unwrap();
    client
        .send_audio(&vec![1; XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT * 2])
        .await
        .unwrap();
    client.finish(Duration::from_millis(250)).await;
    let mut pairs = Vec::new();
    let mut finished = false;
    while let Ok(event) = events.try_recv() {
        match event {
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                source,
                translation,
                ..
            } => pairs.push((source, translation)),
            LiveTranslateServerEvent::SessionFinished => finished = true,
            LiveTranslateServerEvent::Error { code, .. } => panic!("unexpected error: {code}"),
            _ => {}
        }
    }
    assert_eq!(
        pairs,
        [
            ("First source.".into(), "First translation.".into()),
            ("Second source.".into(), "Second translation.".into())
        ]
    );
    assert!(finished);
}

#[tokio::test]
async fn disconnect_cancels_a_finish_wait_without_waiting_for_its_deadline() {
    let (tails_sent, tails_received) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            for _ in 0..FINISH_TAIL_FRAME_COUNT {
                let _ = socket.next().await;
            }
            let _ = tails_sent.send(());
            while socket.next().await.is_some() {}
        })
    })
    .await;
    client.connect().await.unwrap();
    let finishing_client = client.clone();
    let finish = tokio::spawn(async move {
        finishing_client.finish(Duration::from_secs(5)).await;
    });
    tails_received.await.unwrap();
    tokio::time::timeout(Duration::from_millis(500), async {
        client.disconnect().await;
        finish.await.unwrap();
    })
    .await
    .expect("cancellation must release the finish wait immediately");
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(
            event,
            LiveTranslateServerEvent::Error { .. }
                | LiveTranslateServerEvent::SessionFinished
                | LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
        ));
    }
}

#[tokio::test]
async fn configured_source_hint_reaches_the_actual_websocket_setup() {
    let (client, _) = test_client(|mut socket| {
        Box::pin(async move {
            let update: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(
                update["session"]["audio"]["input"]["transcription"]["language_hint"],
                "fr"
            );
            socket.send(setup_ack()).await.unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if message.is_close() {
                    break;
                }
            }
        })
    })
    .await;
    let client = client.with_source_language(crate::core::models::SourceLanguage::French);
    client.connect().await.unwrap();
    client.disconnect().await;
}

#[test]
fn frame_buffer_keeps_only_a_partial_tail() {
    let frame_size = XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let mut pending = vec![0x23; frame_size * 2 + 17];
    let messages = take_complete_audio_messages(&mut pending).unwrap();

    assert_eq!(messages.len(), 2);
    assert_eq!(pending, vec![0x23; 17]);
    for message in messages {
        let value: Value = serde_json::from_str(&message).unwrap();
        let frame = base64::engine::general_purpose::STANDARD
            .decode(value["audio"].as_str().unwrap())
            .unwrap();
        assert_eq!(frame.len(), frame_size);
        assert!(frame.iter().all(|byte| *byte == 0x23));
    }
}

#[tokio::test]
async fn mock_websocket_handles_revised_source_and_never_commits_in_server_vad() {
    let saw_commit = Arc::new(AtomicBool::new(false));
    let server_saw_commit = Arc::clone(&saw_commit);
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            let update: Value = serde_json::from_str(
                socket.next().await.unwrap().unwrap().to_text().unwrap(),
            )
            .unwrap();
            assert_eq!(update["session"]["turn_detection"]["type"], "server_vad");
            assert!(update["session"]["turn_detection"].get("silence_duration_ms").is_none());
            assert_eq!(
                update["session"]["audio"]["input"]["transcription"]["model"],
                "grok-transcribe"
            );
            socket.send(setup_ack()).await.unwrap();

            let audio = socket.next().await.unwrap().unwrap();
            let audio: Value = serde_json::from_str(audio.to_text().unwrap()).unwrap();
            assert_eq!(audio["type"], "input_audio_buffer.append");
            socket
                .send(Message::Text(
                    r#"{"type":"conversation.item.input_audio_transcription.updated","item_id":"item_1","transcript":"I scream"}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"conversation.item.input_audio_transcription.updated","item_id":"item_1","transcript":"Ice cream"}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"item_1","transcript":"Ice cream."}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"response.created","response":{"id":"resp_1"}}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"response.output_audio_transcript.delta","response_id":"resp_1","delta":"アイスクリーム。"}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"response.output_audio_transcript.done","response_id":"resp_1","transcript":"アイスクリーム。"}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"response.done","response":{"id":"resp_1","status":"completed"}}"#.into(),
                ))
                .await
                .unwrap();

            while let Some(Ok(message)) = socket.next().await {
                if message.to_text().ok().is_some_and(|text| {
                    text.contains("input_audio_buffer.commit")
                }) {
                    server_saw_commit.store(true, Ordering::SeqCst);
                }
            }
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client.send_audio(&vec![0; 9_600]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    client.finish(Duration::from_millis(500)).await;

    assert!(!saw_commit.load(Ordering::SeqCst));
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { source, translation, .. }
            if source == "Ice cream." && translation == "アイスクリーム。"
    )));
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
}

#[tokio::test]
async fn finish_appends_vad_silence_before_waiting_for_the_final_turn() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();

            let speech: Value = serde_json::from_str(
                socket.next().await.unwrap().unwrap().to_text().unwrap(),
            )
            .unwrap();
            let speech = base64::engine::general_purpose::STANDARD
                .decode(speech["audio"].as_str().unwrap())
                .unwrap();
            assert!(speech.iter().any(|byte| *byte != 0));

            for _ in 0..FINISH_TAIL_FRAME_COUNT {
                let silence: Value = serde_json::from_str(
                    socket.next().await.unwrap().unwrap().to_text().unwrap(),
                )
                .unwrap();
                let silence = base64::engine::general_purpose::STANDARD
                    .decode(silence["audio"].as_str().unwrap())
                    .unwrap();
                assert_eq!(silence.len(), XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT);
                assert!(silence.iter().all(|byte| *byte == 0));
            }

            socket
                .send(Message::Text(
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"tail_1","transcript":"Last sentence."}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"response.created","response":{"id":"tail_response"}}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"response.output_audio_transcript.done","response_id":"tail_response","transcript":"最後の文。"}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"response.done","response":{"id":"tail_response","status":"completed"}}"#.into(),
                ))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client
        .send_audio(&vec![1; XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    client.finish(Duration::from_millis(500)).await;

    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { source, translation, .. }
            if source == "Last sentence." && translation == "最後の文。"
    )));
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
}

#[tokio::test]
async fn failed_response_emits_fixed_error_and_is_not_a_clean_finish() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"type":"response.created","response":{"id":"failed_response"}}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"response.done","response":{"id":"failed_response","status":"failed"}}"#.into(),
                ))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client
        .send_audio(&vec![1; XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    client.finish(Duration::from_millis(500)).await;

    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::Error { code, message }
            if code == "xai_response_failed" && message == GENERIC_RESPONSE_FAILED_ERROR
    )));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
}

#[tokio::test]
async fn transport_close_while_finishing_is_not_a_clean_finish() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            let _speech = socket.next().await.unwrap().unwrap();
            let _first_tail = socket.next().await.unwrap().unwrap();
            socket.close(None).await.unwrap();
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client
        .send_audio(&vec![1; XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    client.finish(Duration::from_millis(500)).await;

    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::Error { code, message }
            if code == "xai_session_finish_failed" && message == GENERIC_TRANSPORT_ERROR
    )));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
}

#[tokio::test]
async fn finish_is_bounded_when_server_vad_has_not_completed_a_turn() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(setup_ack()).await.unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"input_audio_buffer.speech_started","item_id":"unfinished_source"}"#
                        .into(),
                ))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client.send_audio(&vec![0; 9_600]).await.unwrap();
    let started = tokio::time::Instant::now();
    client.finish(Duration::from_millis(40)).await;
    assert!(started.elapsed() < Duration::from_secs(1));
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::Error { code, message }
            if code == "xai_session_finish_timeout"
                && message == GENERIC_FINISH_TIMEOUT_ERROR
    )));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
}

#[tokio::test]
async fn setup_timeout_is_finite() {
    let (client, _events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        })
    })
    .await;
    assert_eq!(
        client
            .connect_with_timeout(Duration::from_millis(40))
            .await
            .unwrap_err(),
        XAIRealtimeClientError::SessionSetupTimedOut
    );
}
