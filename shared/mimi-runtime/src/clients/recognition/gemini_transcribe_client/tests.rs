use super::*;
use crate::clients::transport::provider_events::{provider_event_channel, ProviderEventReceiver};
use crate::core::services::network_proxy::{ProxyConfig, ProxyMode};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_tungstenite::{accept_async, accept_hdr_async};
type Server = WebSocketStream<TcpStream>;

async fn fixture() -> (GeminiTranscribeClient, ProviderEventReceiver, TcpListener) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client =
        GeminiTranscribeClient::new("synthetic-speech-key", SourceLanguage::English).unwrap();
    client.set_loopback_for_test(listener.local_addr().unwrap());
    client
        .set_network(
            ProviderNetwork::resolve(&ProxyConfig {
                mode: ProxyMode::Direct,
                url: None,
            })
            .unwrap(),
        )
        .unwrap();
    let (events, receiver) = provider_event_channel();
    client.set_event_sender(events).await;
    (client, receiver, listener)
}
async fn send(socket: &mut Server, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}
async fn message(socket: &mut Server) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}
async fn setup(socket: &mut Server) {
    assert_eq!(
        message(socket).await["setup"]["model"],
        "models/gemini-3.5-transcribe-live"
    );
    send(socket, json!({"setupComplete":{}})).await;
}
async fn next_content(
    receiver: &mut ProviderEventReceiver,
) -> crate::clients::transport::provider_events::ProviderEvent {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = receiver.recv_with_revision().await.unwrap();
            if event.is_content() {
                return event;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn binary_json_setup_and_transcripts_work_with_text_or_binary_frames() {
    let contract: Value =
        serde_json::from_str(include_str!("../../../../../translation-contracts.json")).unwrap();
    let contract = &contract["geminiTranscribeTransport"];
    for (binary_setup, binary_content) in [(true, true), (true, false), (false, true)] {
        let (client, mut receiver, listener) = fixture().await;
        let wire = contract.clone();
        let server = tokio::spawn(async move {
            let mut socket = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            let frame = |value: &Value, binary| {
                let text = value.to_string();
                if binary {
                    Message::Binary(text.into_bytes().into())
                } else {
                    Message::Text(text.into())
                }
            };
            let _ = message(&mut socket).await;
            socket
                .send(frame(&wire["setup"], binary_setup))
                .await
                .unwrap();
            socket
                .send(frame(&wire["voiceActivityStart"], binary_content))
                .await
                .unwrap();
            let audio = message(&mut socket).await;
            assert_eq!(
                audio["realtimeInput"]["audio"]["mimeType"],
                "audio/pcm;rate=16000"
            );
            socket
                .send(frame(&wire["draft"], binary_content))
                .await
                .unwrap();
            assert_eq!(message(&mut socket).await, gemini_transcribe::finish());
            socket
                .send(frame(&wire["voiceActivityEnd"], binary_content))
                .await
                .unwrap();
            socket
                .send(frame(&wire["final"], binary_content))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        });
        client.connect("binary-json").await.unwrap();
        client.send_audio(&[0; 3200]).await.unwrap();
        let draft = next_content(&mut receiver).await;
        assert!(
            matches!(draft.event, LiveTranslateServerEvent::SourceUtteranceDraft { ref text, .. }
            if text == contract["expectedDraft"].as_str().unwrap())
        );
        client.finish(Duration::from_secs(2)).await;
        let tail = next_content(&mut receiver).await;
        assert!(
            matches!(tail.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. }
            if text == contract["expectedFinal"].as_str().unwrap())
        );
        assert!(matches!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        ));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn invalid_binary_payloads_fail_before_and_after_setup() {
    let payloads = [
        vec![0xff, 0xfe],
        b"{invalid json".to_vec(),
        b"{}".to_vec(),
        br#"{"serverContent":{"inputTranscription":{}}}"#.to_vec(),
    ];
    for ready in [false, true] {
        for payload in &payloads {
            let (client, mut receiver, listener) = fixture().await;
            let payload = payload.clone();
            let server = tokio::spawn(async move {
                let mut socket = accept_async(listener.accept().await.unwrap().0)
                    .await
                    .unwrap();
                if ready {
                    setup(&mut socket).await;
                    let _ = message(&mut socket).await;
                } else {
                    let _ = message(&mut socket).await;
                }
                socket.send(Message::Binary(payload.into())).await.unwrap();
                while socket.next().await.is_some() {}
            });
            if ready {
                client.connect("invalid-binary").await.unwrap();
                client.send_audio(&[0; 3200]).await.unwrap();
                tokio::time::timeout(Duration::from_secs(2), async {
                    loop {
                        match receiver.recv().await.unwrap() {
                            LiveTranslateServerEvent::SessionCreated => {}
                            LiveTranslateServerEvent::Error { code, .. } => {
                                assert_eq!(
                                    code,
                                    GeminiTranscribeClientError::InvalidProtocol.to_string()
                                );
                                break;
                            }
                            event => {
                                panic!("Unexpected event after invalid binary payload: {event:?}")
                            }
                        }
                    }
                })
                .await
                .unwrap();
            } else {
                assert!(matches!(
                    client.connect("invalid-binary").await,
                    Err(GeminiTranscribeClientError::InvalidProtocol)
                ));
            }
            client.disconnect().await;
            server.await.unwrap();
        }
    }
}

#[tokio::test]
async fn binary_server_errors_preserve_authentication_and_rejection_classification() {
    for (code, expected) in [
        (401, GeminiTranscribeClientError::AuthenticationFailed),
        (403, GeminiTranscribeClientError::AuthenticationFailed),
        (429, GeminiTranscribeClientError::SessionRejected),
    ] {
        let (client, _receiver, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let mut socket = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            let _ = message(&mut socket).await;
            socket
                .send(Message::Binary(
                    json!({"error":{"code":code}})
                        .to_string()
                        .into_bytes()
                        .into(),
                ))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        });
        assert_eq!(
            client
                .connect("binary-error")
                .await
                .unwrap_err()
                .to_string(),
            expected.to_string()
        );
        client.disconnect().await;
        server.await.unwrap();
    }
}

#[tokio::test]
#[allow(clippy::result_large_err)]
async fn speech_only_handshake_streams_and_stop_preserves_authoritative_tail() {
    let (client, mut receiver, listener) = fixture().await;
    let server = tokio::spawn(async move {
        let mut socket = accept_hdr_async(
            listener.accept().await.unwrap().0,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(
                    request.headers().get("x-goog-api-key").unwrap(),
                    "synthetic-speech-key"
                );
                assert!(request.headers().get("Authorization").is_none());
                assert!(request.uri().query().is_none());
                Ok(response)
            },
        )
        .await
        .unwrap();
        setup(&mut socket).await;
        let audio = message(&mut socket).await;
        assert_eq!(
            audio["realtimeInput"]["audio"]["mimeType"],
            "audio/pcm;rate=16000"
        );
        send(
            &mut socket,
            json!({"serverContent":{"inputTranscription":{"text":"First segment."}}}),
        )
        .await;
        assert_eq!(message(&mut socket).await, gemini_transcribe::finish());
        send(&mut socket, json!({"serverContent":{"inputTranscription":{"text":"Final tail."},"turnComplete":true}})).await;
        while socket.next().await.is_some() {}
    });
    client.connect("test").await.unwrap();
    client.send_audio(&[0; 3200]).await.unwrap();
    let first = next_content(&mut receiver).await;
    assert!(
        matches!(first.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "First segment.")
    );
    client.finish(Duration::from_secs(2)).await;
    let tail = next_content(&mut receiver).await;
    assert!(
        matches!(tail.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "Final tail.")
    );
    assert!(matches!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionFinished)
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn stop_drains_authoritative_transcription_after_turn_complete() {
    let (client, mut receiver, listener) = fixture().await;
    let server = tokio::spawn(async move {
        let mut socket = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        setup(&mut socket).await;
        assert_eq!(message(&mut socket).await, gemini_transcribe::finish());
        send(&mut socket, json!({"serverContent":{"turnComplete":true}})).await;
        tokio::time::sleep(TAIL_QUIET_PERIOD / 2).await;
        send(
            &mut socket,
            json!({"serverContent":{"inputTranscription":{"text":"Late authoritative tail."}}}),
        )
        .await;
        while socket.next().await.is_some() {}
    });
    client.connect("test").await.unwrap();
    client.finish(Duration::from_secs(2)).await;
    let tail = next_content(&mut receiver).await;
    assert!(
        matches!(tail.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "Late authoritative tail.")
    );
    assert!(matches!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionFinished)
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn prior_turn_complete_cannot_finish_before_the_audio_end_signal() {
    let (client, mut receiver, listener) = fixture().await;
    let (send_prior, wait_prior) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut socket = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        setup(&mut socket).await;
        wait_prior.await.unwrap();
        send(&mut socket, json!({"serverContent":{"interimInputTranscription":{"text":"Prior preview"},"turnComplete":true}})).await;
        assert_eq!(message(&mut socket).await, gemini_transcribe::finish());
        send(&mut socket, json!({"serverContent":{"inputTranscription":{"text":"Flushed final."},"turnComplete":true}})).await;
        while socket.next().await.is_some() {}
    });
    client.connect("test").await.unwrap();
    let audio_guard = client.inner.audio_operation.lock().await;
    let closing_client = client.clone();
    let closing = tokio::spawn(async move {
        closing_client.finish(Duration::from_secs(2)).await;
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !client.inner.closing.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    send_prior.send(()).unwrap();
    let _ = next_content(&mut receiver).await;
    assert!(!client.inner.done.load(Ordering::SeqCst));
    drop(audio_guard);
    closing.await.unwrap();
    let tail = next_content(&mut receiver).await;
    assert!(
        matches!(tail.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "Flushed final.")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn old_marker_while_end_write_is_blocked_cannot_consume_the_drain_window() {
    let (client, mut receiver, listener) = fixture().await;
    let (send_prior, wait_prior) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut socket = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        setup(&mut socket).await;
        wait_prior.await.unwrap();
        send(&mut socket, json!({"serverContent":{"interimInputTranscription":{"text":"Earlier preview"},"turnComplete":true}})).await;
        assert_eq!(message(&mut socket).await, gemini_transcribe::finish());
        tokio::time::sleep(TAIL_QUIET_PERIOD / 2).await;
        send(
            &mut socket,
            json!({"serverContent":{"inputTranscription":{"text":"Post-write final."}}}),
        )
        .await;
        while socket.next().await.is_some() {}
    });
    client.connect("test").await.unwrap();
    let sink_guard = client.inner.sink.lock().await;
    let closing_client = client.clone();
    let closing = tokio::spawn(async move {
        closing_client.finish(Duration::from_secs(2)).await;
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !client.inner.closing.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    send_prior.send(()).unwrap();
    let _ = next_content(&mut receiver).await;
    tokio::time::sleep(TAIL_QUIET_PERIOD + Duration::from_millis(20)).await;
    assert!(!client.inner.end_sent.load(Ordering::SeqCst));
    assert!(!client.inner.done.load(Ordering::SeqCst));
    drop(sink_guard);
    closing.await.unwrap();
    let tail = next_content(&mut receiver).await;
    assert!(
        matches!(tail.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "Post-write final.")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn clear_opens_a_new_socket_and_cannot_commit_old_audio() {
    let (client, mut receiver, listener) = fixture().await;
    let (first_ready, wait_first) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut first = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        setup(&mut first).await;
        send(
            &mut first,
            json!({"serverContent":{"interimInputTranscription":{"text":"Old draft"}}}),
        )
        .await;
        first_ready.send(()).unwrap();
        while let Some(Ok(message)) = first.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
        let _ = first
            .send(Message::Text(
                json!({"serverContent":{"inputTranscription":{"text":"Stale final"}}})
                    .to_string()
                    .into(),
            ))
            .await;
        let mut second = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        setup(&mut second).await;
        send(
            &mut second,
            json!({"serverContent":{"inputTranscription":{"text":"New final"}}}),
        )
        .await;
        while second.next().await.is_some() {}
    });
    client.connect("test").await.unwrap();
    wait_first.await.unwrap();
    let _ = next_content(&mut receiver).await;
    assert_eq!(client.clear_content().await, 1);
    let next = next_content(&mut receiver).await;
    assert_eq!(next.content_revision, 1);
    assert!(
        matches!(next.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "New final")
    );
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn cancelled_probe_closes_its_socket_while_client_is_retained() {
    let (client, _receiver, listener) = fixture().await;
    let (setup_seen, wait_setup) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut socket = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        let _ = message(&mut socket).await;
        setup_seen.send(()).unwrap();
        let closed = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("cancelled Gemini probe retained its WebSocket");
        assert!(matches!(
            closed,
            None | Some(Err(_)) | Some(Ok(Message::Close(_)))
        ));
    });
    let mut pending = Box::pin(client.connect_for_probe("cancelled"));
    tokio::select! { result = &mut pending => panic!("unexpected setup: {result:?}"), _ = wait_setup => {} }
    drop(pending);
    server.await.unwrap();
    assert!(client.inner.sink.lock().await.is_none());
    client.disconnect().await;
}

#[tokio::test]
async fn cancelled_setup_retires_receiver_before_the_next_connection() {
    let (client, mut receiver, listener) = fixture().await;
    let (old_setup, wait_old_setup) = oneshot::channel();
    let (send_late, wait_send_late) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut first = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        let _ = message(&mut first).await;
        old_setup.send(()).unwrap();
        wait_send_late.await.unwrap();
        let _ = first
            .send(Message::Text(
                json!({"setupComplete":{}}).to_string().into(),
            ))
            .await;
        let _ = first
            .send(Message::Text(
                json!({"serverContent":{"inputTranscription":{"text":"Late old final"}}})
                    .to_string()
                    .into(),
            ))
            .await;
        let mut second = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        setup(&mut second).await;
        send(
            &mut second,
            json!({"serverContent":{"inputTranscription":{"text":"Fresh final"}}}),
        )
        .await;
        while second.next().await.is_some() {}
    });
    let mut connect = Box::pin(client.connect("cancelled"));
    tokio::select! { _ = &mut connect => panic!("setup must wait for acknowledgement"), _ = wait_old_setup => {} }
    drop(connect);
    send_late.send(()).unwrap();
    client.connect("fresh").await.unwrap();
    let event = next_content(&mut receiver).await;
    assert!(
        matches!(event.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "Fresh final")
    );
    client.disconnect().await;
    server.await.unwrap();
}
