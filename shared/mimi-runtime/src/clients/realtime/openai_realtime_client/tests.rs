#[tokio::test]
async fn clear_discards_local_tail_and_queued_content_without_touching_audio_or_lifecycle() {
    let (sender, mut receiver) = provider_event_channel();
    let endpoint = url::Url::parse("ws://127.0.0.1:1/realtime").unwrap();
    let client = OpenAIRealtimeClient::with_endpoint(
        "test-key-not-real",
        TargetLanguage::Japanese,
        sender.clone(),
        endpoint,
    )
    .unwrap();
    client.inner.ready.store(true, Ordering::SeqCst);
    client.inner.generation.store(7, Ordering::SeqCst);
    *client.inner.pending_audio.lock().await = vec![1, 2, 3];
    client
        .inner
        .committer
        .lock()
        .await
        .append_source_delta("old source", None);
    client
        .inner
        .committer
        .lock()
        .await
        .append_translation_delta("old translation", None);

    sender
        .send(LiveTranslateServerEvent::SubtitleFinalPair {
            source: "old queued source".into(),
            language: None,
            translation: "old queued translation".into(),
        })
        .unwrap();
    sender
        .send(LiveTranslateServerEvent::SessionUpdated)
        .unwrap();
    assert_eq!(client.clear_content().await, 1);
    assert_eq!(client.content_revision(), 1);
    assert_eq!(client.inner.generation.load(Ordering::SeqCst), 7);
    assert!(client.inner.ready.load(Ordering::SeqCst));
    assert_eq!(*client.inner.pending_audio.lock().await, vec![1, 2, 3]);
    assert!(client.inner.committer.lock().await.finish().is_empty());

    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionUpdated)
    );
    assert!(receiver.try_recv().is_err());
    let mut committer = client.inner.committer.lock().await;
    committer.append_source_delta("new source", None);
    committer.append_translation_delta("new translation", None);
    assert!(committer.finish().iter().any(|event| matches!(event,
        LiveTranslateServerEvent::SubtitleFinalPair {source,translation,..}
            if source == "new source" && translation == "new translation")));
}

use super::*;
use crate::clients::transport::provider_events::{provider_event_channel, ProviderEventReceiver};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;

async fn test_client(
    server: impl FnOnce(
            WebSocketStream<TcpStream>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + 'static,
) -> (OpenAIRealtimeClient, ProviderEventReceiver) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        server(socket).await;
    });
    let (events, receiver) = provider_event_channel();
    let endpoint = url::Url::parse(&format!("ws://{address}/realtime")).unwrap();
    let client = OpenAIRealtimeClient::with_endpoint(
        "sk-test-not-real",
        TargetLanguage::Japanese,
        events,
        endpoint,
    )
    .unwrap();
    (client, receiver)
}

fn acknowledge_session(
    mut socket: WebSocketStream<TcpStream>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let update = socket.next().await.unwrap().unwrap();
        let update: Value = serde_json::from_str(update.to_text().unwrap()).unwrap();
        assert_eq!(update["type"], "session.update");
        socket
            .send(Message::Text(
                r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                    .into(),
            ))
            .await
            .unwrap();
        while socket.next().await.is_some() {}
    })
}

use serde_json::Value;

#[tokio::test]
async fn binary_json_preserves_unicode_and_rejects_invalid_utf8_transcripts() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../../../translation-contracts.json")).unwrap();
    let case = &fixtures["binaryJsonTransports"]["openaiRealtime"];
    let setup = case["setup"].to_string();
    let transcript = case["transcript"].to_string();
    let hex = case["invalidTranscriptHex"].as_str().unwrap();
    let invalid: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).unwrap())
        .collect();
    let (corrupt, corrupted) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Binary(setup.into_bytes().into()))
                .await
                .unwrap();
            socket
                .send(Message::Binary(transcript.into_bytes().into()))
                .await
                .unwrap();
            corrupted.await.unwrap();
            socket.send(Message::Binary(invalid.into())).await.unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_secs(2))
        .await
        .unwrap();
    let source = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(source, LiveTranslateServerEvent::SourceDraft { text, .. }
        if text == case["expectedSource"].as_str().unwrap())
    );

    corrupt.send(()).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        error,
        LiveTranslateServerEvent::Error {
            code: "openai_protocol_error".into(),
            message: GENERIC_PROTOCOL_ERROR.into(),
        }
    );
    let task = client.inner.receive_task.lock().await.take().unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(events.try_recv().is_err());
    client.disconnect().await;
}

#[tokio::test]
async fn connect_waits_for_valid_setup_acknowledgement() {
    let (client, _events) = test_client(acknowledge_session).await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn an_immediate_pong_cannot_be_lost_before_the_waiter_is_polled() {
    let (client, _events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                        .into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if let Message::Ping(payload) = message {
                    socket.send(Message::Pong(payload)).await.unwrap();
                    break;
                }
            }
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();

    client.ping(Duration::from_millis(200)).await.unwrap();
    client.disconnect().await;
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
            .connect_with_timeout(Duration::from_millis(50))
            .await
            .unwrap_err(),
        OpenAIRealtimeClientError::SessionSetupTimedOut
    );
}

#[test]
fn upgrade_distinguishes_account_quota_from_request_limiting() {
    use tokio_tungstenite::tungstenite::{http::Response, Error};
    for (body, expected) in [
        (
            Some(r#"{"error":{"type":"rate_limit_error","code":"project_spend_limit_exceeded"}}"#),
            OpenAIRealtimeFailure::Quota,
        ),
        (
            Some(
                r#"{"error":{"type":"rate_limit_error","code":"slow_down","message":"project_spend_limit_exceeded"}}"#,
            ),
            OpenAIRealtimeFailure::RateLimited,
        ),
        (None, OpenAIRealtimeFailure::RateLimited),
    ] {
        let response = Response::builder()
            .status(429)
            .body(body.map(|body| body.as_bytes().to_vec()))
            .unwrap();
        assert_eq!(
            upgrade_failure(Error::Http(Box::new(response))),
            OpenAIRealtimeClientError::ProviderRejected(expected)
        );
    }
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
            OpenAIRealtimeClientError::AuthenticationFailed,
        ),
        (
            "authorization_error",
            "permission_denied",
            false,
            OpenAIRealtimeClientError::ProviderRejected(OpenAIRealtimeFailure::Permission),
        ),
        (
            "authentication_error",
            "key_expired",
            false,
            OpenAIRealtimeClientError::AuthenticationFailed,
        ),
        (
            "authorization_error",
            "invalid_event",
            true,
            OpenAIRealtimeClientError::ProviderRejected(OpenAIRealtimeFailure::Permission),
        ),
        (
            "invalid_request_error",
            "insufficient_quota",
            false,
            OpenAIRealtimeClientError::ProviderRejected(OpenAIRealtimeFailure::Quota),
        ),
        (
            "invalid_request_error",
            "invalid_event",
            false,
            OpenAIRealtimeClientError::ProviderRejected(OpenAIRealtimeFailure::InvalidRequest),
        ),
        (
            "server_error",
            "unknown_service_code",
            false,
            OpenAIRealtimeClientError::ProviderRejected(OpenAIRealtimeFailure::ServiceError),
        ),
        (
            "server_error",
            "unknown_service_code",
            true,
            OpenAIRealtimeClientError::ProviderRejected(OpenAIRealtimeFailure::ServiceError),
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
async fn active_terminal_errors_keep_their_safe_recovery_reason() {
    for (kind, code, expected, retryable) in [
        (
            "invalid_request_error",
            "invalid_api_key",
            "credential_authentication_failed",
            false,
        ),
        (
            "authentication_error",
            "key_expired",
            "credential_authentication_failed",
            false,
        ),
        (
            "authorization_error",
            "invalid_event",
            "openai_realtime_permission_denied",
            false,
        ),
        (
            "invalid_request_error",
            "session_expired",
            "openai_realtime_session_expired",
            true,
        ),
        (
            "rate_limit_error",
            "project_spend_limit_exceeded",
            "openai_realtime_quota_exhausted",
            false,
        ),
    ] {
        let (release, released) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(Message::Text(
                    r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#.into()
                )).await.unwrap();
                released.await.unwrap();
                socket.send(Message::Text(serde_json::json!({
                    "type": "error", "error": {
                        "type": kind, "code": code,
                        "message": "private provider detail authentication_error"
                    }
                }).to_string().into())).await.unwrap();
                socket.send(Message::Text(
                    r#"{"type":"session.input_transcript.delta","delta":"must not be processed"}"#.into()
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
        assert!(client.inner.committer.lock().await.finish().is_empty());
        assert!(events.try_recv().is_err());
        client.disconnect().await;
        assert_eq!(
            event,
            LiveTranslateServerEvent::Error {
                code: expected.into(),
                message: expected.into(),
            },
            "{kind}/{code}"
        );
        assert_eq!(
            crate::core::lifecycle::recovery::provider_error_is_retryable(expected),
            retryable
        );
    }
}

#[tokio::test]
async fn pre_ready_recoverable_error_for_other_request_keeps_waiting() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await.unwrap().unwrap();
            socket.send(Message::Text(r#"{"type":"error","error":{"type":"invalid_request_error","code":"invalid_event","event_id":"unrelated-fixture-request"}}"#.into())).await.unwrap();
            socket.send(Message::Text(r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#.into())).await.unwrap();
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
        Err(OpenAIRealtimeClientError::TransportFailure),
    );
    assert!(events.try_recv().is_err());
    client.disconnect().await;
}

#[test]
fn frame_buffer_keeps_only_a_partial_tail() {
    let mut pending = vec![0x11; 9_600 * 2 + 17];
    let messages = take_complete_audio_messages(&mut pending).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(pending.len(), 17);
    for message in messages {
        let value: Value = serde_json::from_str(&message).unwrap();
        let frame = base64::engine::general_purpose::STANDARD
            .decode(value["audio"].as_str().unwrap())
            .unwrap();
        assert_eq!(frame.len(), 9_600);
        assert!(frame.iter().all(|byte| *byte == 0x11));
    }
}

#[test]
fn final_partial_frame_is_zero_padded() {
    let mut pending = vec![1, 2, 3];
    let message = take_padded_audio_message(&mut pending).unwrap().unwrap();
    let value: Value = serde_json::from_str(&message).unwrap();
    let frame = base64::engine::general_purpose::STANDARD
        .decode(value["audio"].as_str().unwrap())
        .unwrap();
    assert_eq!(frame.len(), 9_600);
    assert_eq!(&frame[..3], &[1, 2, 3]);
    assert!(frame[3..].iter().all(|byte| *byte == 0));
    assert!(pending.is_empty());
}

#[tokio::test]
async fn recoverable_error_is_diagnosed_without_stopping_transcript_flow() {
    use crate::core::development::development_debug::{self, DebugEvent, TraceSink};

    struct DiagnosticGuard;
    impl Drop for DiagnosticGuard {
        fn drop(&mut self) {
            development_debug::initialize(false);
        }
    }
    development_debug::initialize(true);
    development_debug::set_enabled(true).unwrap();
    let _diagnostic_guard = DiagnosticGuard;
    let labels = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_labels = Arc::clone(&labels);
    development_debug::set_sink(Some(TraceSink::new(move |entry| {
        if let DebugEvent::Pipeline { label } = entry.event {
            if label.starts_with("openai_realtime ") {
                captured_labels.lock().unwrap().push(label);
            }
        }
    })))
    .unwrap();

    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(Message::Text(
                r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#.into()
            )).await.unwrap();
            socket.send(Message::Text(
                r#"{"type":"error","error":{"type":"invalid_request_error","code":"private_error_code","message":"private provider body"}}"#.into()
            )).await.unwrap();
            socket.send(Message::Text(
                r#"{"type":"session.input_transcript.delta","delta":"Hello."}"#.into()
            )).await.unwrap();
            socket.send(Message::Text(
                r#"{"type":"session.output_transcript.delta","delta":"こんにちは。"}"#.into()
            )).await.unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if message.to_text().ok().is_some_and(|text| text.contains("session.close")) {
                    socket.send(Message::Text(r#"{"type":"session.closed"}"#.into())).await.unwrap();
                    break;
                }
            }
        })
    }).await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client.finish(Duration::from_millis(500)).await;
    assert_eq!(
        *labels.lock().unwrap(),
        ["openai_realtime recoverable_provider_error"]
    );

    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. }
            if source == "Hello." && translation == "こんにちは。"
    )));
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
}

#[tokio::test]
async fn a_recoverable_error_for_session_setup_still_rejects_the_connection() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let update = socket.next().await.unwrap().unwrap();
            let update: Value = serde_json::from_str(update.to_text().unwrap()).unwrap();
            let error = serde_json::json!({
                "type": "error",
                "error": {
                    "type": "invalid_request_error",
                    "code": "invalid_event",
                    "message": "private setup details",
                    "event_id": update["event_id"],
                },
            });
            socket
                .send(Message::Text(error.to_string().into()))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        })
    })
    .await;

    assert_eq!(
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap_err(),
        OpenAIRealtimeClientError::ProviderRejected(OpenAIRealtimeFailure::InvalidRequest)
    );
    assert!(!client.inner.ready.load(Ordering::SeqCst));
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn a_fatal_provider_error_still_stops_transcript_flow() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(Message::Text(
                r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#.into()
            )).await.unwrap();
            socket.send(Message::Text(
                r#"{"type":"error","error":{"type":"authentication_error","code":"invalid_api_key","message":"private authentication details"}}"#.into()
            )).await.unwrap();
            socket.send(Message::Text(
                r#"{"type":"session.input_transcript.delta","delta":"must not be processed"}"#.into()
            )).await.unwrap();
            while socket.next().await.is_some() {}
        })
    }).await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event,
        LiveTranslateServerEvent::Error {
            code: "credential_authentication_failed".into(),
            message: "credential_authentication_failed".into(),
        }
    );
    let receive_task = client.inner.receive_task.lock().await.take().unwrap();
    tokio::time::timeout(Duration::from_millis(500), receive_task)
        .await
        .unwrap()
        .unwrap();
    assert!(client.inner.committer.lock().await.finish().is_empty());
    assert!(events.try_recv().is_err());
    client.disconnect().await;
}

#[tokio::test]
async fn close_timeout_is_finite_and_drops_an_unconfirmed_tail() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                        .into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"session.input_transcript.delta","delta":"unfinished source"}"#
                        .into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"session.output_transcript.delta","delta":"未完の訳"}"#
                        .into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if message
                    .to_text()
                    .ok()
                    .is_some_and(|text| text.contains("session.close"))
                {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    break;
                }
            }
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let started = tokio::time::Instant::now();
    client.finish(Duration::from_millis(50)).await;
    assert!(started.elapsed() < Duration::from_secs(1));

    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::Error { code, .. } if code == "openai_close_timeout"
    )));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. })));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
}

#[tokio::test]
async fn graceful_session_closed_flushes_one_atomic_tail_pair() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                        .into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"session.input_transcript.delta","delta":"final source tail"}"#
                        .into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"session.output_transcript.delta","delta":"最後の訳"}"#
                        .into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if message
                    .to_text()
                    .ok()
                    .is_some_and(|text| text.contains("session.close"))
                {
                    tokio::time::sleep(Duration::from_millis(2_100)).await;
                    socket
                        .send(Message::Text(r#"{"type":"session.closed"}"#.into()))
                        .await
                        .unwrap();
                    break;
                }
            }
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let client =
        crate::clients::translation::translation_client::TranslationClient::OpenAIRealtime(client);
    tokio::time::timeout(client.finish_timeout(), client.finish())
        .await
        .unwrap();

    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert_eq!(
        received
            .iter()
            .filter(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. }))
            .count(),
        1
    );
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. }
            if source == "final source tail" && translation == "最後の訳"
    )));
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
}

#[tokio::test]
async fn unsolicited_session_closed_requests_transport_recovery() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                        .into(),
                ))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            socket
                .send(Message::Text(r#"{"type":"session.closed"}"#.into()))
                .await
                .unwrap();
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("transport recovery event should arrive before the timeout")
        .expect("transport recovery event channel should stay open");
    assert!(matches!(
        event,
        LiveTranslateServerEvent::Error { code, message }
            if code == "transport_error" && message == GENERIC_TRANSPORT_ERROR
    ));
    client.disconnect().await;
}

#[tokio::test]
async fn stale_socket_events_are_suppressed_after_disconnect() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                        .into(),
                ))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = socket
                .send(Message::Text(
                    r#"{"type":"session.input_transcript.delta","delta":"stale private text"}"#
                        .into(),
                ))
                .await;
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client.disconnect().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(events.try_recv().is_err());
}
