#[tokio::test]
async fn handshake_authentication_rejections_preserve_transport_failures() {
    for (status, expected_auth_label) in
        crate::clients::transport::connection_diagnostics::handshake_failure_cases()
    {
        let (endpoint, server) =
            crate::clients::transport::connection_diagnostics::rejected_websocket_endpoint(status)
                .await;
        let (events, _receiver) = provider_event_channel();
        let mut client = AzureOpenAIRealtimeClient::with_endpoint(
            "test-key-not-real",
            "test-transcription",
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
            assert_eq!(error, AzureOpenAIRealtimeClientError::AuthenticationFailed);
            assert_eq!(error.to_string(), expected_label);
        } else {
            assert_eq!(error, AzureOpenAIRealtimeClientError::TransportFailure);
        }
        assert!(!error.to_string().contains("private-handshake-body"));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn clear_discards_local_tail_and_queued_content_without_touching_audio_or_lifecycle() {
    let (sender, mut receiver) = provider_event_channel();
    let endpoint = url::Url::parse("ws://127.0.0.1:1/realtime").unwrap();
    let client = AzureOpenAIRealtimeClient::with_endpoint(
        "test-key-not-real",
        "test-transcription",
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
    client
        .inner
        .translation_stream
        .lock()
        .await
        .append_delta(AzureTranslationStream::DedicatedSession, "old translation");
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
    assert!(client
        .inner
        .translation_stream
        .lock()
        .await
        .current_text
        .is_empty());
    assert!(client
        .inner
        .translation_stream
        .lock()
        .await
        .selected
        .is_none());
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
use serde_json::Value;
use tokio::net::TcpListener;

#[test]
fn continuous_translation_deltas_do_not_retain_session_text() {
    let mut state = TranslationStreamState::default();
    for _ in 0..10_000 {
        assert_eq!(
            state.append_delta(AzureTranslationStream::DedicatedSession, "译文 fragment。"),
            Some("译文 fragment。".into())
        );
        assert!(state.current_text.is_empty());
    }
    assert_eq!(
        state.finish(
            AzureTranslationStream::DedicatedSession,
            Some("译文 fragment。".into())
        ),
        None,
        "a compatibility done event must not replay append-only deltas"
    );
}

#[test]
fn response_text_overflow_keeps_deltas_and_resumes_exact_final_tail_matching() {
    let mut state = TranslationStreamState::default();
    let delta = "译文".repeat(128);
    let mut streamed = String::new();
    for _ in 0..100 {
        streamed.push_str(
            &state
                .append_delta(AzureTranslationStream::ResponseText, &delta)
                .unwrap(),
        );
        assert!(state.current_text.len() <= 4_096);
    }
    assert_eq!(streamed, delta.repeat(100));
    assert_eq!(
        state.finish(
            AzureTranslationStream::ResponseText,
            Some(format!("{streamed}final-only tail"))
        ),
        None,
        "after the prefix exceeds its bound, do not guess or replay final text"
    );
    assert!(state.current_text.is_empty());
    assert_eq!(
        state.append_delta(AzureTranslationStream::ResponseText, "普通"),
        Some("普通".into())
    );
    assert_eq!(
        state.finish(
            AzureTranslationStream::ResponseText,
            Some("普通补尾。".into())
        ),
        Some("补尾。".into())
    );
}

#[test]
fn selected_translation_stream_avoids_replays_and_preserves_normal_final_tails() {
    for selected in [
        AzureTranslationStream::DedicatedSession,
        AzureTranslationStream::ResponseText,
    ] {
        let other = match selected {
            AzureTranslationStream::DedicatedSession => AzureTranslationStream::ResponseText,
            AzureTranslationStream::ResponseText => AzureTranslationStream::DedicatedSession,
        };
        let mut state = TranslationStreamState::default();
        assert_eq!(
            state.append_delta(selected, "Bonjour"),
            Some("Bonjour".into())
        );
        assert_eq!(state.append_delta(other, "Bonjour"), None);
        assert_eq!(state.finish(other, Some("Bonjour.".into())), None);
        let expected_tail =
            (selected == AzureTranslationStream::ResponseText).then(|| ".".to_string());
        assert_eq!(
            state.finish(selected, Some("Bonjour.".into())),
            expected_tail
        );
        assert_eq!(state.selected, Some(selected));
        assert!(state.current_text.is_empty());
    }
    let mut state = TranslationStreamState::default();
    assert_eq!(
        state.finish(
            AzureTranslationStream::ResponseText,
            Some("Done only.".into())
        ),
        Some("Done only.".into())
    );
}

#[test]
fn response_text_exact_limit_preserves_tail_and_clear_resets_overflow() {
    let mut state = TranslationStreamState::default();
    let prefix = "a".repeat(4_096);
    assert_eq!(
        state.append_delta(AzureTranslationStream::ResponseText, &prefix),
        Some(prefix.clone())
    );
    assert_eq!(
        state.finish(
            AzureTranslationStream::ResponseText,
            Some(format!("{prefix} tail"))
        ),
        Some(" tail".into())
    );
    state.append_delta(AzureTranslationStream::ResponseText, &"译".repeat(2_000));
    assert!(state.current_text.is_empty());
    state.reset();
    assert_eq!(
        state.finish(
            AzureTranslationStream::ResponseText,
            Some("After clear.".into())
        ),
        Some("After clear.".into())
    );
}

async fn test_client(
    server: impl FnOnce(
            WebSocketStream<TcpStream>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + 'static,
) -> (AzureOpenAIRealtimeClient, ProviderEventReceiver) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        server(socket).await;
    });
    let (events, receiver) = provider_event_channel();
    let endpoint = url::Url::parse(&format!("ws://{address}/realtime")).unwrap();
    let client = AzureOpenAIRealtimeClient::with_endpoint(
        "azure-test-key-not-real",
        "gpt-4o-transcribe-deployment",
        TargetLanguage::Japanese,
        events,
        endpoint,
    )
    .unwrap();
    (client, receiver)
}

#[tokio::test]
async fn binary_json_preserves_unicode_and_rejects_invalid_utf8_transcripts() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../../../translation-contracts.json")).unwrap();
    let case = &fixtures["binaryJsonTransports"]["azureOpenaiRealtime"];
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
            code: "azure_openai_protocol_error".into(),
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
async fn mock_websocket_runs_translation_and_graceful_close() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let update: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(update["session"]["audio"]["output"]["language"], "ja");
            assert_eq!(
                update["session"]["audio"]["input"]["transcription"]["model"],
                "gpt-4o-transcribe-deployment"
            );
            socket
                .send(Message::Text(
                    r#"{"type":"session.updated","session":{"audio":{"output":{"language":"ja"}}}}"#
                        .into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"session.input_transcript.delta","delta":"Hello."}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"type":"response.text.delta","text":"こんにちは。"}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(r#"{"type":"response.text.done"}"#.into()))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if message
                    .to_text()
                    .ok()
                    .is_some_and(|text| text.contains("session.close"))
                {
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
    client.finish(Duration::from_millis(500)).await;

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
}

#[test]
fn frame_buffer_is_bounded_and_preserves_tail() {
    let mut pending = vec![0x41; 9_600 * 2 + 17];
    let messages = take_complete_audio_messages(&mut pending).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(pending.len(), 17);
    for message in messages {
        let value: Value = serde_json::from_str(&message).unwrap();
        let frame = base64::engine::general_purpose::STANDARD
            .decode(value["audio"].as_str().unwrap())
            .unwrap();
        assert_eq!(frame.len(), 9_600);
    }
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
        AzureOpenAIRealtimeClientError::SessionSetupTimedOut
    );
}
