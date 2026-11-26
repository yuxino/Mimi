use super::*;
use crate::clients::transport::provider_events::provider_event_channel;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

#[tokio::test]
async fn binary_json_preserves_unicode_and_rejects_invalid_utf8_transcripts() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../translation-contracts.json")).unwrap();
    let case = &fixtures["binaryJsonTransports"]["audio3"];
    let setup = case["setup"].to_string();
    let transcript = case["transcript"].to_string();
    let hex = case["invalidTranscriptHex"].as_str().unwrap();
    let invalid: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).unwrap())
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = Audio3ASRClient::new_custom(
        &format!("ws://{}/asr", listener.local_addr().unwrap()),
        "synthetic-model",
        "synthetic-key",
        SourceLanguage::English,
    )
    .unwrap();
    client
        .set_network(
            crate::clients::transport::provider_network::ProviderNetwork::resolve(
                &crate::core::services::network_proxy::ProxyConfig {
                    mode: crate::core::services::network_proxy::ProxyMode::Direct,
                    url: None,
                },
            )
            .unwrap(),
        )
        .unwrap();
    let (events, mut receiver) = provider_event_channel();
    client.set_event_sender(events).await;
    let (corrupt, corrupted) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
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
    });
    client
        .connect_for_probe("binary-json-fixture")
        .await
        .unwrap();
    let source = tokio::time::timeout(Duration::from_secs(2), async {
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionCreated)
        );
        receiver.recv().await.unwrap()
    })
    .await
    .unwrap();
    assert!(
        matches!(source, LiveTranslateServerEvent::SourceUtteranceFinal { text, .. }
        if text == case["expectedSource"].as_str().unwrap())
    );

    corrupt.send(()).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        error,
        LiveTranslateServerEvent::Error {
            code: "audio3_protocol_error".into(),
            message: GENERIC_PROTOCOL_ERROR.into(),
        }
    );
    let task = client.inner.receive_task.lock().await.take().unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(receiver.try_recv().is_err());
    client.disconnect().await;
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

async fn connected_finish_fixture() -> (
    Audio3ASRClient,
    crate::clients::transport::provider_events::ProviderEventReceiver,
    WebSocketStream<tokio::net::TcpStream>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = Audio3ASRClient::new_custom(
        &format!("ws://{}/asr", listener.local_addr().unwrap()),
        "synthetic-model",
        "synthetic-key",
        SourceLanguage::English,
    )
    .unwrap();
    client
        .set_network(
            crate::clients::transport::provider_network::ProviderNetwork::resolve(
                &crate::core::services::network_proxy::ProxyConfig {
                    mode: crate::core::services::network_proxy::ProxyMode::Direct,
                    url: None,
                },
            )
            .unwrap(),
        )
        .unwrap();
    let (sender, mut receiver) = provider_event_channel();
    client.set_event_sender(sender).await;
    let accept = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let run = socket.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(run.contains("run-task"));
        socket
            .send(Message::Text(
                r#"{"header":{"event":"task-started","task_id":"finish-fixture"}}"#.into(),
            ))
            .await
            .unwrap();
        socket
    };
    let (connected, socket) = tokio::join!(client.connect_for_probe("finish-fixture"), accept);
    connected.unwrap();
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionCreated)
    );
    (client, receiver, socket)
}

#[tokio::test]
async fn unsolicited_task_finish_requests_recovery_without_promoting_a_draft() {
    let (client, mut receiver, mut socket) = connected_finish_fixture().await;
    socket.send(Message::Text(serde_json::json!({
        "header":{"event":"result-generated","task_id":"finish-fixture"},
        "payload":{"output":{"sentence":{"text":"Synthetic unfinished caption", "sentence_id":7,"sentence_end":false}}}
    }).to_string().into())).await.unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap(),
        Some(LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: 7,
            ..
        })
    ));
    socket
        .send(Message::Text(
            r#"{"header":{"event":"task-finished","task_id":"finish-fixture"}}"#.into(),
        ))
        .await
        .unwrap();
    socket.send(Message::Text(serde_json::json!({
        "header":{"event":"result-generated","task_id":"finish-fixture"},
        "payload":{"output":{"sentence":{"text":"Synthetic late final", "sentence_id":7,"sentence_end":true}}}
    }).to_string().into())).await.unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event,
        LiveTranslateServerEvent::Error {
            code: "transport_error".into(),
            message: GENERIC_TRANSPORT_ERROR.into(),
        }
    );
    let task = client.inner.receive_task.lock().await.take().unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(receiver.try_recv().is_err());
    assert!(!client.inner.task_finished.load(Ordering::SeqCst));
    client.disconnect().await;
}

#[tokio::test]
async fn requested_finish_preserves_final_tail_but_does_not_authorize_reused_task_end() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = Audio3ASRClient::new("synthetic-key", SourceLanguage::English).unwrap();
    client.endpoint.url =
        url::Url::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let (sender, mut receiver) = provider_event_channel();
    client.set_event_sender(sender).await;
    let (trigger, triggered) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut triggered = Some(triggered);
        for connection in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            assert!(socket
                .next()
                .await
                .unwrap()
                .unwrap()
                .to_text()
                .unwrap()
                .contains("run-task"));
            socket
                .send(Message::Text(
                    r#"{"header":{"event":"task-started"}}"#.into(),
                ))
                .await
                .unwrap();
            if connection == 0 {
                assert!(socket
                    .next()
                    .await
                    .unwrap()
                    .unwrap()
                    .to_text()
                    .unwrap()
                    .contains("finish-task"));
                socket.send(Message::Text(serde_json::json!({
                    "header":{"event":"result-generated"},
                    "payload":{"output":{"sentence":{"text":"Synthetic final tail", "sentence_id":3,"sentence_end":true}}}
                }).to_string().into())).await.unwrap();
            } else {
                triggered.take().unwrap().await.unwrap();
            }
            socket
                .send(Message::Text(
                    r#"{"header":{"event":"task-finished"}}"#.into(),
                ))
                .await
                .unwrap();
            assert!(matches!(
                socket.next().await,
                None | Some(Ok(Message::Close(_)))
            ));
        }
    });
    client
        .connect_for_probe("synthetic-reused-task")
        .await
        .unwrap();
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionCreated)
    );
    client.finish(Duration::from_secs(1)).await;
    assert!(matches!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 3,
            ..
        })
    ));
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionFinished)
    );
    client
        .connect_for_probe("synthetic-reused-task")
        .await
        .unwrap();
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionCreated)
    );
    trigger.send(()).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap(),
        Some(LiveTranslateServerEvent::Error {
            code: "transport_error".into(),
            message: GENERIC_TRANSPORT_ERROR.into(),
        })
    );
    client.disconnect().await;
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn unsolicited_task_finish_during_setup_is_an_immediate_transport_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = Audio3ASRClient::new("synthetic-key", SourceLanguage::English).unwrap();
    client.endpoint.url =
        url::Url::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
    let (sender, mut receiver) = provider_event_channel();
    client.set_event_sender(sender).await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let _run = socket.next().await.unwrap().unwrap();
        socket
            .send(Message::Text(
                r#"{"header":{"event":"task-finished"}}"#.into(),
            ))
            .await
            .unwrap();
        assert!(matches!(
            socket.next().await,
            None | Some(Ok(Message::Close(_)))
        ));
    });
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        client.connect_for_probe("synthetic-setup"),
    )
    .await
    .expect("an unexpected finish must not wait for the setup deadline")
    .unwrap_err();
    assert!(error.is_unreachable());
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::Error {
            code: "transport_error".into(),
            message: GENERIC_TRANSPORT_ERROR.into(),
        })
    );
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn finish_is_inert_without_a_connection_and_after_a_completed_finish() {
    let client = Audio3ASRClient::new("synthetic-key", SourceLanguage::English).unwrap();
    let (sender, mut receiver) = provider_event_channel();
    client.set_event_sender(sender).await;
    client.finish(Duration::from_millis(40)).await;
    assert!(receiver.try_recv().is_err());

    let (client, mut receiver, mut socket) = connected_finish_fixture().await;
    let server = tokio::spawn(async move {
        assert!(socket
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap()
            .contains("finish-task"));
        assert!(matches!(
            socket.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    });
    client.finish(Duration::from_millis(40)).await;
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionFinished)
    );
    client.finish(Duration::from_millis(40)).await;
    client.disconnect().await;
    client.finish(Duration::from_millis(40)).await;
    assert!(receiver.try_recv().is_err());
    server.await.unwrap();
}

#[tokio::test]
async fn finish_preserves_provider_errors_without_a_synthetic_success() {
    let (client, mut receiver, mut socket) = connected_finish_fixture().await;
    let server = tokio::spawn(async move {
        assert!(socket
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap()
            .contains("finish-task"));
        socket.send(Message::Text(r#"{"header":{"event":"task-failed","task_id":"finish-fixture","error_code":"SERVER_ERROR","error_message":"synthetic failure"}}"#.into())).await.unwrap();
        assert!(matches!(
            socket.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    });
    client.finish(Duration::from_secs(1)).await;
    assert!(matches!(
        receiver.try_recv(),
        Ok(LiveTranslateServerEvent::Error { .. })
    ));
    assert!(receiver.try_recv().is_err());
    client.finish(Duration::from_millis(40)).await;
    assert!(receiver.try_recv().is_err());
    server.await.unwrap();
}

#[tokio::test]
async fn finish_send_failure_cannot_become_a_synthetic_success() {
    let (client, mut receiver, mut socket) = connected_finish_fixture().await;
    // Keep the peer unread so the receive task cannot independently turn
    // this into a transport error. The writer itself rejects further data.
    client
        .inner
        .sink
        .lock()
        .await
        .as_mut()
        .unwrap()
        .close()
        .await
        .unwrap();
    client.finish(Duration::from_secs(1)).await;
    assert!(receiver.try_recv().is_err());
    assert!(matches!(
        socket.next().await,
        Some(Ok(Message::Close(_))) | None
    ));
}

#[tokio::test]
async fn finish_command_wait_uses_the_ack_budget_and_stops_its_producer() {
    let (client, mut receiver, mut socket) = connected_finish_fixture().await;
    let writer = client.inner.sink.lock().await;
    let finisher = {
        let client = client.clone();
        tokio::spawn(async move { client.finish(Duration::from_millis(40)).await })
    };
    // An occupied writer cannot make finish spend an additional send
    // timeout. Producer drain is observable before cleanup acquires it.
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap(),
        Some(LiveTranslateServerEvent::SessionFinished)
    );
    assert!(client.inner.receive_task.lock().await.is_none());
    drop(writer);
    finisher.await.unwrap();
    assert!(matches!(
        socket.next().await,
        Some(Ok(Message::Close(_))) | None
    ));
}

#[tokio::test]
#[allow(clippy::result_large_err)] // Tungstenite fixes the handshake callback error type.
async fn custom_dashscope_uses_selected_model_and_real_task_boundary() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(request.uri().path(), "/api-ws/v1/inference");
                assert!(request.uri().query().is_none());
                assert_eq!(request.headers()["authorization"], "Bearer synthetic-key");
                Ok(response)
            },
        )
        .await
        .unwrap();
        let run_task: serde_json::Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(run_task["header"]["action"], "run-task");
        assert_eq!(run_task["header"]["task_id"], "synthetic-custom-task");
        assert_eq!(run_task["payload"]["task"], "asr");
        assert_eq!(run_task["payload"]["model"], "synthetic-custom-asr-model");
        assert_eq!(run_task["payload"]["parameters"]["sample_rate"], 16_000);
        socket.send(Message::Text(serde_json::json!({"header":{"event":"task-started","task_id":"synthetic-custom-task"}}).to_string().into())).await.unwrap();
        assert!(
            matches!(socket.next().await.unwrap().unwrap(), Message::Binary(bytes) if bytes.as_ref() == [1,2,3,4])
        );
        socket.send(Message::Text(serde_json::json!({"header":{"event":"result-generated","task_id":"synthetic-custom-task"},"payload":{"output":{"sentence":{"sentence_id":1,"sentence_end":true,"text":"Synthetic custom recognition"}}}}).to_string().into())).await.unwrap();
        let finish: serde_json::Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(finish["header"]["action"], "finish-task");
        assert_eq!(finish["header"]["task_id"], "synthetic-custom-task");
        socket.send(Message::Text(serde_json::json!({"header":{"event":"task-finished","task_id":"synthetic-custom-task"}}).to_string().into())).await.unwrap();
        assert!(matches!(
            socket.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    });
    let mut client = Audio3ASRClient::new_custom(
        &format!("ws://{address}/api-ws/v1/inference"),
        "synthetic-custom-asr-model",
        "synthetic-key",
        SourceLanguage::English,
    )
    .unwrap();
    client
        .set_network(
            crate::clients::transport::provider_network::ProviderNetwork::resolve(
                &crate::core::services::network_proxy::ProxyConfig {
                    mode: crate::core::services::network_proxy::ProxyMode::Direct,
                    url: None,
                },
            )
            .unwrap(),
        )
        .unwrap();
    let (events, mut receiver) = provider_event_channel();
    client.set_event_sender(events).await;
    client
        .connect_for_probe("synthetic-custom-task")
        .await
        .unwrap();
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionCreated)
    );
    client.send_audio(&[1, 2, 3, 4]).await.unwrap();
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 1,
            text: "Synthetic custom recognition".into(),
            language: Some("en".into())
        })
    );
    assert!(client
        .send_audio(&vec![
            0;
            crate::core::protocols::recognition::custom_speech::MAX_PCM_CHUNK_BYTES
                + 2
        ])
        .await
        .is_err());
    client.finish(Duration::from_secs(1)).await;
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionFinished)
    );
    server.await.unwrap();
}

#[tokio::test]
async fn clear_keeps_one_socket_and_audio_sending_while_old_sentence_is_discarded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        assert!(socket
            .next()
            .await
            .unwrap()
            .unwrap()
            .to_text()
            .unwrap()
            .contains("run-task"));
        socket
            .send(Message::Text(
                r#"{"header":{"event":"task-started"}}"#.into(),
            ))
            .await
            .unwrap();
        let frame = |id, final_sentence| {
            Message::Text(serde_json::json!({
            "header":{"event":"result-generated"}, "payload":{"output":{"sentence":{
                "text":"Synthetic test sentence", "sentence_id":id, "sentence_end":final_sentence
            }}}
        }).to_string().into())
        };
        socket.send(frame(7, false)).await.unwrap();
        release_rx.await.unwrap();
        let audio = socket.next().await.unwrap().unwrap();
        assert!(matches!(audio, Message::Binary(bytes) if bytes.as_ref() == [1, 2, 3, 4]));
        socket.send(frame(7, false)).await.unwrap();
        socket.send(frame(7, true)).await.unwrap();
        socket.send(frame(8, true)).await.unwrap();
        assert!(matches!(
            socket.next().await,
            None | Some(Ok(Message::Close(_)))
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    });
    let mut client = Audio3ASRClient::new("synthetic-key", SourceLanguage::English).unwrap();
    client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
    client
        .set_network(
            crate::clients::transport::provider_network::ProviderNetwork::resolve(
                &crate::core::services::network_proxy::ProxyConfig {
                    mode: crate::core::services::network_proxy::ProxyMode::Direct,
                    url: None,
                },
            )
            .unwrap(),
        )
        .unwrap();
    let (sender, mut receiver) = provider_event_channel();
    client.set_event_sender(sender).await;
    client
        .connect_for_probe("synthetic-clear-task")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = receiver
                .recv()
                .await
                .expect("synthetic stream closed before its identified draft");
            if matches!(
                event,
                LiveTranslateServerEvent::SourceUtteranceDraft {
                    utterance_id: 7,
                    ..
                }
            ) {
                break;
            }
        }
    })
    .await
    .expect("synthetic identified draft was not delivered");
    assert_eq!(client.clear_content().await, 1);
    client.send_audio(&[1, 2, 3, 4]).await.unwrap();
    release_tx.send(()).unwrap();
    let next = tokio::time::timeout(Duration::from_secs(2), receiver.recv_with_revision())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.content_revision, 1);
    assert!(matches!(
        next.event,
        LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 8,
            ..
        }
    ));
    assert!(receiver.try_recv().is_err());
    client.disconnect().await;
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn service_probe_waits_for_task_ready_without_sending_silence_pcm() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let run = socket.next().await.unwrap().unwrap();
        assert!(run.to_text().unwrap().contains("run-task"));
        socket
            .send(Message::Text(
                r#"{"header":{"event":"task-started"}}"#.into(),
            ))
            .await
            .unwrap();
        // Several normal heartbeat intervals pass; a probe sends no PCM.
        assert!(
            tokio::time::timeout(Duration::from_millis(350), socket.next())
                .await
                .is_err()
        );
        socket.next().await
    });
    let mut client = Audio3ASRClient::new("fixture-only", SourceLanguage::English).unwrap();
    client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
    let (events, _receiver) = provider_event_channel();
    client.set_event_sender(events).await;
    client
        .connect_for_probe("synthetic-probe-task")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    client.disconnect().await;
    let close = server.await.unwrap();
    assert!(matches!(close, None | Some(Ok(Message::Close(_)))));
}

#[tokio::test]
async fn idle_audio_sends_silence_and_finish_stops_heartbeat() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (silence_ready_tx, silence_ready_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let run = socket.next().await.unwrap().unwrap().into_text().unwrap();
        let run: serde_json::Value = serde_json::from_str(&run).unwrap();
        assert_eq!(run["payload"]["parameters"]["heartbeat"], true);
        socket
            .send(Message::Text(
                r#"{"header":{"event":"task-started"}}"#.into(),
            ))
            .await
            .unwrap();
        for _ in 0..2 {
            let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(
                matches!(frame, Message::Binary(ref bytes) if bytes.len() == SILENCE_PCM.len() && bytes.iter().all(|byte| *byte == 0))
            );
        }
        silence_ready_tx.send(()).unwrap();
        loop {
            let frame = socket.next().await.unwrap().unwrap();
            if let Message::Text(text) = frame {
                assert!(text.contains("finish-task"));
                socket
                    .send(Message::Text(
                        r#"{"header":{"event":"task-finished"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                break;
            }
        }
        let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap();
        assert!(matches!(next, None | Some(Ok(Message::Close(_)))));
    });
    let mut client = Audio3ASRClient::new("synthetic-test-key", SourceLanguage::English).unwrap();
    client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
    let (sender, mut events) = provider_event_channel();
    client.set_event_sender(sender).await;
    client.connect("synthetic-task").await.unwrap();
    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::SessionCreated)
    );
    tokio::time::timeout(Duration::from_secs(2), silence_ready_rx)
        .await
        .unwrap()
        .unwrap();
    client.finish(Duration::from_secs(1)).await;
    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::SessionFinished)
    );
    server.await.unwrap();
}

#[tokio::test]
async fn custom_unsupported_language_remains_terminal_and_content_free() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let run = socket.next().await.unwrap().unwrap().into_text().unwrap();
        let run: serde_json::Value = serde_json::from_str(&run).unwrap();
        assert_eq!(
            run["payload"]["parameters"]["language_hints"],
            serde_json::json!(["ja"])
        );
        socket.send(Message::Text(serde_json::json!({"header": {
            "event":"task-failed", "task_id":run["header"]["task_id"],
            "error_code":"UNSUPPORTED_LANGUAGE", "error_message":"synthetic-private-response"
        }}).to_string().into())).await.unwrap();
        let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap();
        assert!(matches!(next, None | Some(Ok(Message::Close(_)))));
    });
    let client = Audio3ASRClient::new_custom(
        &format!("ws://{address}/asr"),
        "fixture",
        "fixture-key",
        SourceLanguage::Japanese,
    )
    .unwrap();
    let (sender, mut events) = provider_event_channel();
    client.set_event_sender(sender).await;
    let token = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
    assert_eq!(
        client
            .connect("fixture-task")
            .await
            .unwrap_err()
            .to_string(),
        token
    );
    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::Error {
            code: token.into(),
            message: token.into()
        })
    );
    server.await.unwrap();
}

#[tokio::test]
async fn setup_authentication_failure_remains_terminal_and_safe() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let _run = socket.next().await.unwrap().unwrap();
        socket.send(Message::Text(r#"{"header":{"event":"task-failed","error_code":"INVALID_API_KEY","error_message":"synthetic-private-response"}}"#.into())).await.unwrap();
        let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap();
        assert!(matches!(next, None | Some(Ok(Message::Close(_)))));
    });
    let mut client = Audio3ASRClient::new("synthetic-test-key", SourceLanguage::English).unwrap();
    client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
    let (sender, mut events) = provider_event_channel();
    client.set_event_sender(sender).await;
    let error = client.connect("synthetic-task").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "audio3_error.setup.authentication.INVALID_API_KEY"
    );
    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::Error {
            code: "audio3_error.setup.authentication.INVALID_API_KEY".into(),
            message: "audio3_error.setup.authentication.INVALID_API_KEY".into()
        })
    );
    server.await.unwrap();
}

#[tokio::test]
async fn real_pcm_is_preserved_and_terminal_error_does_not_become_transport_recovery() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let _run = socket.next().await.unwrap().unwrap();
        socket
            .send(Message::Text(
                r#"{"header":{"event":"task-started"}}"#.into(),
            ))
            .await
            .unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let frame = socket.next().await.unwrap().unwrap();
                if matches!(&frame, Message::Binary(bytes) if bytes.iter().all(|byte| *byte == 0)) {
                    continue;
                }
                break frame;
            }
        })
        .await
        .unwrap();
        assert!(matches!(frame, Message::Binary(ref bytes) if bytes.as_ref() == [1, 2, 3, 4]));
        socket.send(Message::Text(r#"{"header":{"event":"task-failed","error_code":"CLIENT_ERROR","error_message":"request timeout after 23 seconds."}}"#.into())).await.unwrap();
        let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap();
        assert!(matches!(next, None | Some(Ok(Message::Close(_)))));
    });
    let mut client = Audio3ASRClient::new("synthetic-test-key", SourceLanguage::English).unwrap();
    client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
    let (sender, mut events) = provider_event_channel();
    client.set_event_sender(sender).await;
    client.connect("synthetic-task").await.unwrap();
    let _ready = events.recv().await;
    client.send_audio(&[1, 2, 3, 4]).await.unwrap();
    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::Error {
            code: "audio3_error.recognition.timeout.CLIENT_ERROR".into(),
            message: "audio3_error.recognition.timeout.CLIENT_ERROR".into()
        })
    );
    client.send_audio(&[1, 2]).await.unwrap();
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn idle_send_clock_can_insert_silence_before_real_pcm_already_in_the_pipeline() {
    controlled_pending_pcm_gap(false, false).await;
}

#[tokio::test]
async fn a_bound_pending_pcm_gate_prevents_extra_silence_before_delayed_real_pcm() {
    controlled_pending_pcm_gap(true, false).await;
}

#[tokio::test]
async fn idle_silence_is_still_sent_after_the_bound_real_pcm_pipeline_drains() {
    controlled_pending_pcm_gap(true, true).await;
}

async fn controlled_pending_pcm_gap(bind_gate: bool, idle_after_drain: bool) {
    use crate::audio::send_pipeline::AudioSendPipeline;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let first_pcm = [1_u8, 0].repeat(800); // 50 ms of nonzero 16 kHz PCM16.
    let delayed_pcm = [64_u8, 0].repeat(1600); // 100 ms, already captured and audible.
    let expected_first = first_pcm.clone();
    let expected_delayed = delayed_pcm.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let run = socket.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(run.contains("run-task"));
        socket
            .send(Message::Text(
                r#"{"header":{"event":"task-started"}}"#.into(),
            ))
            .await
            .unwrap();
        let mut frames = Vec::new();
        while let Some(message) = socket.next().await {
            match message.unwrap() {
                Message::Binary(bytes) => frames.push(bytes.to_vec()),
                Message::Close(_) => break,
                _ => {}
            }
        }
        let expect_zero = !bind_gate || idle_after_drain;
        assert_eq!(frames.len(), if expect_zero { 3 } else { 2 });
        assert_eq!(frames[0], expected_first);
        if bind_gate {
            assert_eq!(frames[1], expected_delayed);
            if idle_after_drain {
                assert_eq!(frames[2], SILENCE_PCM);
            }
        } else {
            assert_eq!(frames[1], SILENCE_PCM);
            assert_eq!(frames[2], expected_delayed);
        }
        let (zero_frames, zero_bytes, real_bytes) =
            frames
                .iter()
                .fold((0, 0, 0), |(count, zeros, real), bytes| {
                    if bytes.iter().all(|byte| *byte == 0) {
                        (count + 1, zeros + bytes.len(), real)
                    } else {
                        (count, zeros, real + bytes.len())
                    }
                });
        assert_eq!(zero_frames, usize::from(expect_zero));
        assert_eq!(zero_bytes, if expect_zero { 3200 } else { 0 });
        assert_eq!(real_bytes, 4800); // All 150 ms of captured PCM survives.
        assert_eq!(
            zero_bytes + real_bytes,
            if expect_zero { 8000 } else { 4800 }
        );
    });

    let mut client = Audio3ASRClient::new("synthetic-test-key", SourceLanguage::English).unwrap();
    client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
    let (sender, mut events) = provider_event_channel();
    client.set_event_sender(sender).await;
    // Disable only the autonomous timer so the exact production idle-send
    // operation below runs at a deterministic simulated scheduling gap.
    client.connect_for_probe("synthetic-task").await.unwrap();
    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::SessionCreated)
    );
    client.send_audio(&first_pcm).await.unwrap();

    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let entered = Arc::new(std::sync::Mutex::new(Some(entered_tx)));
    let release = Arc::new(std::sync::Mutex::new(Some(release_rx)));
    let delayed_client = client.clone();
    let pipeline = AudioSendPipeline::spawn(
        move |data| {
            let client = delayed_client.clone();
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            async move {
                let entered = entered.lock().unwrap().take();
                if let Some(entered) = entered {
                    let _ = entered.send(data.len());
                }
                // A producer/network-worker scheduling pause before it
                // enters the socket. No captured audio is actually absent.
                let release = release.lock().unwrap().take();
                if let Some(release) = release {
                    let _ = release.await;
                }
                client.send_audio(&data).await
            }
        },
        |_| panic!("synthetic local audio transport failed"),
    );
    let pending_gate = pipeline.pending_pcm_gate();
    if bind_gate {
        client.set_audio_pending_gate(pending_gate.clone());
    }
    pipeline.ingress().unwrap().try_send(delayed_pcm).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), entered_rx)
            .await
            .unwrap()
            .unwrap(),
        3200
    );
    assert_eq!(pipeline.input_activity(), (true, true));
    assert_eq!(pending_gate.pending_count(), 1);

    // Backdating the successful-send clock models a >100 ms gap without
    // a real-time sleep/testutil. Real PCM is still pending in the pipeline.
    {
        let now = tokio::time::Instant::now();
        let mut clock = client.inner.audio_clock.lock().await;
        clock.started_at = Some(now - Duration::from_millis(151));
        clock.last_sent_at = Some(now - SILENCE_INTERVAL - Duration::from_millis(1));
    }
    if bind_gate {
        // A real PCM writer can hold this lock while the receive loop's
        // heartbeat fires. Pending work must let that loop keep reading.
        let held_sink = client.inner.sink.lock().await;
        {
            let idle_send = send_silence_if_idle(&client.inner);
            tokio::pin!(idle_send);
            assert!(
                matches!(
                    futures_util::poll!(idle_send.as_mut()),
                    std::task::Poll::Ready(Ok(()))
                ),
                "pending real PCM must skip the heartbeat without waiting for its writer"
            );
        }
        drop(held_sink);
    } else {
        send_silence_if_idle(&client.inner).await.unwrap();
    }
    release_tx.send(()).unwrap();
    assert!(pipeline.finish(Duration::from_secs(2)).await);
    assert_eq!(pending_gate.pending_count(), 0);
    if idle_after_drain {
        {
            let now = tokio::time::Instant::now();
            let mut clock = client.inner.audio_clock.lock().await;
            clock.started_at = Some(now - Duration::from_millis(350));
            clock.last_sent_at = Some(now - SILENCE_INTERVAL - Duration::from_millis(1));
        }
        send_silence_if_idle(&client.inner).await.unwrap();
    }
    client.disconnect().await;
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn failed_pcm_send_does_not_consume_silence_budget() {
    let (client, _receiver, mut socket) = connected_finish_fixture().await;
    client
        .inner
        .audio_clock
        .lock()
        .await
        .start(tokio::time::Instant::now() - Duration::from_secs(1));
    let before = {
        let clock = client.inner.audio_clock.lock().await;
        (clock.audio_end_ns, clock.last_sent_at)
    };
    client
        .inner
        .sink
        .lock()
        .await
        .as_mut()
        .unwrap()
        .close()
        .await
        .unwrap();
    assert!(client.send_audio(&[1, 0].repeat(1600)).await.is_err());
    assert!(send_silence_if_idle(&client.inner).await.is_err());
    let clock = client.inner.audio_clock.lock().await;
    assert_eq!((clock.audio_end_ns, clock.last_sent_at), before);
    drop(clock);
    client.disconnect().await;
    assert!(matches!(
        socket.next().await,
        Some(Ok(Message::Close(_))) | None
    ));
}

#[tokio::test]
async fn repeated_task_ready_does_not_reset_successful_pcm_budget() {
    let (client, mut receiver, mut socket) = connected_finish_fixture().await;
    client.send_audio(&[1, 0].repeat(1600)).await.unwrap();
    assert!(matches!(socket.next().await, Some(Ok(Message::Binary(_)))));
    let before = {
        let clock = client.inner.audio_clock.lock().await;
        (clock.started_at, clock.audio_end_ns, clock.last_sent_at)
    };
    socket
        .send(Message::Text(
            r#"{"header":{"event":"task-started","task_id":"finish-fixture"}}"#.into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionCreated)
    );
    let clock = client.inner.audio_clock.lock().await;
    assert_eq!(
        (clock.started_at, clock.audio_end_ns, clock.last_sent_at),
        before
    );
    drop(clock);
    client.disconnect().await;
}

#[tokio::test]
async fn batched_continuous_pcm_does_not_accumulate_idle_silence() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let _run = socket.next().await.unwrap().unwrap();
        socket
            .send(Message::Text(
                r#"{"header":{"event":"task-started"}}"#.into(),
            ))
            .await
            .unwrap();
        let mut real_frames = Vec::new();
        let mut silence_bytes = 0usize;
        while let Some(message) = socket.next().await {
            match message.unwrap() {
                Message::Binary(bytes) if bytes.iter().all(|byte| *byte == 0) => {
                    silence_bytes += bytes.len();
                }
                Message::Binary(bytes) => real_frames.push(bytes.to_vec()),
                Message::Close(_) => break,
                _ => {}
            }
        }
        (real_frames, silence_bytes)
    });
    let mut client = Audio3ASRClient::new("fixture-only", SourceLanguage::English).unwrap();
    client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
    client
        .set_network(
            crate::clients::transport::provider_network::ProviderNetwork::resolve(
                &crate::core::services::network_proxy::ProxyConfig {
                    mode: crate::core::services::network_proxy::ProxyMode::Direct,
                    url: None,
                },
            )
            .unwrap(),
        )
        .unwrap();
    let (sender, mut events) = provider_event_channel();
    client.set_event_sender(sender).await;
    let gate = PendingPcmGate::default();
    client.set_audio_pending_gate(gate.clone());
    client.connect("batched-pcm-fixture").await.unwrap();
    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::SessionCreated)
    );
    let started = tokio::time::Instant::now();
    let mut previous_capture = started;
    let mut longest_gap = Duration::ZERO;
    for index in 1..=10u32 {
        tokio::time::sleep_until(started + Duration::from_millis(200) * index).await;
        let captured = tokio::time::Instant::now();
        longest_gap = longest_gap.max(captured.saturating_duration_since(previous_capture));
        previous_capture = captured;
        let pcm = [index as u8, 0].repeat(3200); // 200 ms of 16 kHz mono PCM16.
        let pending = gate.acquire();
        client.send_audio(&pcm).await.unwrap();
        drop(pending);
    }
    let elapsed = started.elapsed();
    client.disconnect().await;
    let (real_frames, silence_bytes) = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    let real_bytes: usize = real_frames.iter().map(Vec::len).sum();
    eprintln!(
        "batched continuous PCM: realBytes={real_bytes} silenceBytes={silence_bytes} elapsedMs={}",
        elapsed.as_millis()
    );
    assert_eq!(
        real_frames,
        (1..=10u8)
            .map(|value| [value, 0].repeat(3200))
            .collect::<Vec<_>>()
    );
    // A delayed CI executor may deliver a larger initial batch. Allow that
    // measured gap and genuinely elapsed idle time, never one extra gap per
    // callback. The clock tests above cover an exact virtual one-hour run.
    let allowed_gap =
        elapsed.saturating_sub(Duration::from_secs(2)) + longest_gap + SILENCE_INTERVAL;
    assert!(
        silence_bytes as u128 * 1_000_000_000 / 32_000 <= allowed_gap.as_nanos(),
        "idle silence accumulated beyond the measured capture gap: {silence_bytes} bytes"
    );
}
