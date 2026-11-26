use super::*;
use crate::clients::transport::provider_events::{provider_event_channel, ProviderEventReceiver};
use crate::core::services::network_proxy::{ProxyConfig, ProxyMode};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, accept_hdr_async};

type ServerSocket = WebSocketStream<TcpStream>;
const MODEL: &str = "gpt-4o-mini-transcribe";

#[tokio::test]
#[allow(clippy::result_large_err)]
async fn builtin_openai_and_azure_use_transcription_intent_and_separate_authentication() {
    for azure in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = if azure {
            CustomSpeechClient::azure(
                "https://synthetic.openai.azure.com",
                "speech-deployment",
                "synthetic-key",
                SourceLanguage::Chinese,
            )
            .unwrap()
        } else {
            CustomSpeechClient::openai("synthetic-key", SourceLanguage::Chinese).unwrap()
        };
        let expected_path = if azure {
            "/openai/v1/realtime"
        } else {
            "/v1/realtime"
        };
        assert_eq!(client.endpoint.path(), expected_path);
        client.endpoint.set_scheme("ws").unwrap();
        client.endpoint.set_host(Some("127.0.0.1")).unwrap();
        client
            .endpoint
            .set_port(Some(listener.local_addr().unwrap().port()))
            .unwrap();
        client
            .set_network(
                ProviderNetwork::resolve(&ProxyConfig {
                    mode: ProxyMode::Direct,
                    url: None,
                })
                .unwrap(),
            )
            .unwrap();
        let (events, _receiver) = provider_event_channel();
        client.set_event_sender(events).await;
        let server = tokio::spawn(async move {
            let mut socket = accept_hdr_async(
                listener.accept().await.unwrap().0,
                move |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                      response| {
                    assert_eq!(request.uri().path(), expected_path);
                    assert_eq!(request.uri().query(), Some("intent=transcription"));
                    assert_eq!(
                        request
                            .headers()
                            .get(if azure { "api-key" } else { "Authorization" })
                            .unwrap(),
                        if azure {
                            "synthetic-key"
                        } else {
                            "Bearer synthetic-key"
                        }
                    );
                    assert!(request
                        .headers()
                        .get(if azure { "Authorization" } else { "api-key" })
                        .is_none());
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let update = wire(&mut socket).await;
            let model = if azure {
                "speech-deployment"
            } else {
                "gpt-live-transcribe"
            };
            assert_eq!(update["session"]["type"], "transcription");
            assert_eq!(
                update["session"]["audio"]["input"]["transcription"]["model"],
                model
            );
            assert_eq!(
                update["session"]["audio"]["input"]["transcription"]["languages"],
                json!(["zh-cn"])
            );
            assert!(update["session"]["audio"]["input"]["turn_detection"].is_null());
            send(
                &mut socket,
                json!({"type":"session.updated", "session":update["session"]}),
            )
            .await;
            while socket.next().await.is_some() {}
        });
        client.connect("test").await.unwrap();
        client.disconnect().await;
        server.await.unwrap();
    }
}

#[tokio::test]
async fn cancelled_builtin_probe_setup_closes_socket_without_dropping_the_client() {
    for azure in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = if azure {
            CustomSpeechClient::azure(
                "https://synthetic.openai.azure.com",
                "speech-deployment",
                "synthetic-key",
                SourceLanguage::English,
            )
            .unwrap()
        } else {
            CustomSpeechClient::openai("synthetic-key", SourceLanguage::English).unwrap()
        };
        client.endpoint.set_scheme("ws").unwrap();
        client.endpoint.set_host(Some("127.0.0.1")).unwrap();
        client
            .endpoint
            .set_port(Some(listener.local_addr().unwrap().port()))
            .unwrap();
        client
            .set_network(
                ProviderNetwork::resolve(&ProxyConfig {
                    mode: ProxyMode::Direct,
                    url: None,
                })
                .unwrap(),
            )
            .unwrap();
        let (events, _receiver) = provider_event_channel();
        client.set_event_sender(events).await;
        let (setup_seen, wait_setup) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut socket = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            assert_eq!(wire(&mut socket).await["session"]["type"], "transcription");
            setup_seen.send(()).unwrap();
            let closed = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .expect("cancelled probe retained its WebSocket");
            assert!(matches!(
                closed,
                None | Some(Err(_)) | Some(Ok(Message::Close(_)))
            ));
        });
        let mut pending = Box::pin(client.connect_for_probe("cancelled-probe"));
        tokio::select! {
            result = &mut pending => panic!("setup unexpectedly completed: {result:?}"),
            _ = wait_setup => {},
        }
        drop(pending);
        server.await.unwrap();
        assert!(!client.inner.ready.load(Ordering::SeqCst));
        assert!(client.inner.receive_task.lock().unwrap().is_none());
        assert!(client.inner.sink.lock().await.is_none());
        client.disconnect().await;
    }
}

async fn fixture() -> (CustomSpeechClient, ProviderEventReceiver, TcpListener) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = CustomSpeechClient::new(
        &format!("ws://{}/v1/realtime", listener.local_addr().unwrap()),
        MODEL,
        "synthetic-key",
        SourceLanguage::English,
    )
    .unwrap();
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

async fn wire(socket: &mut ServerSocket) -> Value {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                other => panic!("Unexpected synthetic wire message: {other:?}"),
            }
        }
    })
    .await
    .unwrap()
}

async fn send(socket: &mut ServerSocket, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

async fn setup(socket: &mut ServerSocket) {
    let mut update = wire(socket).await;
    assert_eq!(update["type"], "session.update");
    assert_eq!(update["session"]["type"], "transcription");
    assert_eq!(
        update["session"]["audio"]["input"]["format"],
        json!({"type":"audio/pcm","rate":24000})
    );
    assert_eq!(
        update["session"]["audio"]["input"]["transcription"]["model"],
        MODEL
    );
    assert_eq!(
        update["session"]["audio"]["input"]["transcription"]["language"],
        "en"
    );
    assert!(update["session"]["audio"]["input"]["turn_detection"].is_null());
    update["type"] = json!("session.updated");
    send(socket, update).await;
}

async fn committed(socket: &mut ServerSocket, item: &str) {
    assert_eq!(wire(socket).await["type"], "input_audio_buffer.append");
    loop {
        let event = wire(socket).await;
        if event["type"] == "input_audio_buffer.commit" {
            break;
        }
        assert_eq!(event["type"], "input_audio_buffer.append");
    }
    send(
        socket,
        json!({"type":"input_audio_buffer.committed","item_id":item}),
    )
    .await;
}

async fn completed(socket: &mut ServerSocket, item: &str, text: &str) {
    send(socket, json!({"type":"conversation.item.input_audio_transcription.completed","item_id":item,"content_index":0,"transcript":text})).await;
}

async fn next_final(receiver: &mut ProviderEventReceiver) -> (u64, String) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match receiver.recv().await.unwrap() {
                LiveTranslateServerEvent::SourceUtteranceFinal {
                    utterance_id, text, ..
                } => return (utterance_id, text),
                LiveTranslateServerEvent::Error { code, .. } => {
                    panic!("Synthetic ASR failed: {code}")
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}

fn voice_then_silence() -> Vec<u8> {
    let mut pcm: Vec<u8> = 3000_i16
        .to_le_bytes()
        .into_iter()
        .cycle()
        .take(960)
        .collect();
    pcm.extend(vec![0; 30 * 960]);
    pcm
}

#[tokio::test]
#[allow(clippy::result_large_err)] // Tungstenite fixes the handshake callback error type.
async fn probe_waits_for_real_setup_ack_and_sends_no_pcm() {
    let (client, mut receiver, listener) = fixture().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(
                    request.uri().path_and_query().unwrap().as_str(),
                    "/v1/realtime?intent=transcription"
                );
                assert_eq!(request.headers()["authorization"], "Bearer synthetic-key");
                Ok(response)
            },
        )
        .await
        .unwrap();
        setup(&mut socket).await;
        assert!(matches!(
            socket.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    });
    client.connect_for_probe("synthetic-probe").await.unwrap();
    assert!(matches!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionCreated)
    ));
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn authentication_error_frames_preserve_fixed_diagnostic_classification() {
    let (client, mut receiver, listener) = fixture().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let _ = wire(&mut socket).await;
        send(&mut socket, json!({"type":"error","error":{"code":"invalid_api_key","message":"Synthetic sensitive provider body"}})).await;
        let _ = socket.next().await;
    });
    assert_eq!(
        client.connect("synthetic-auth").await,
        Err(CustomSpeechClientError::AuthenticationFailed)
    );
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::Error {
            code: "custom_speech_authentication_failed".into(),
            message: "custom_speech_authentication_failed".into()
        })
    );
    server.await.unwrap();
}

#[tokio::test]
async fn disconnect_cancels_stalled_receive_timer_before_waiting_for_its_audio_lock() {
    let (client, _receiver, _listener) = fixture().await;
    let old_generation = client.inner.generation.load(Ordering::SeqCst);
    client.inner.ready.store(true, Ordering::SeqCst);
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let inner = client.inner.clone();
    let task = tokio::spawn(async move {
        let _audio = inner.audio_operation.lock().await;
        entered_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    *client.inner.receive_task.lock().unwrap() = Some(task.abort_handle());
    entered_rx.await.unwrap();
    tokio::time::timeout(Duration::from_millis(100), client.disconnect())
        .await
        .unwrap();
    assert!(!client.is_current(old_generation));
    assert!(!client.inner.ready.load(Ordering::SeqCst));
    assert!(client.inner.receive_task.lock().unwrap().is_none());
}

#[tokio::test]
async fn socket_creation_alone_does_not_pass_setup_and_provider_body_is_not_retained() {
    let (client, _receiver, listener) = fixture().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let _ = wire(&mut socket).await;
        send(&mut socket, json!({"type":"session.created"})).await;
        assert!(matches!(
            socket.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    });
    assert_eq!(
        client.connect_with_timeout(Duration::from_millis(40)).await,
        Err(CustomSpeechClientError::SetupTimeout)
    );
    server.await.unwrap();

    let (client, mut receiver, listener) = fixture().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let _ = wire(&mut socket).await;
        send(&mut socket, json!({"type":"error","error":{"code":"arbitrary-secret-code","message":"synthetic-secret-provider-body"}})).await;
        let _ = socket.next().await;
    });
    assert_eq!(
        client.connect("synthetic-rejected").await,
        Err(CustomSpeechClientError::SessionRejected)
    );
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::Error {
            code: "custom_speech_session_rejected".into(),
            message: "custom_speech_session_rejected".into()
        })
    );
    server.await.unwrap();
}

#[tokio::test]
async fn audio_turns_stream_bounded_pcm_and_out_of_order_finals_keep_capture_order() {
    let (client, mut receiver, listener) = fixture().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        setup(&mut socket).await;
        committed(&mut socket, "one").await;
        committed(&mut socket, "two").await;
        completed(&mut socket, "two", "Synthetic second final").await;
        completed(&mut socket, "one", "Synthetic first final").await;
        let _ = socket.next().await;
    });
    client.connect("synthetic-order").await.unwrap();
    // Idle system audio must not build an unlimited remote input buffer.
    for _ in 0..20 {
        client.send_audio(&vec![0; 9600]).await.unwrap();
    }
    client.send_audio(&voice_then_silence()).await.unwrap();
    client.send_audio(&voice_then_silence()).await.unwrap();
    assert_eq!(
        next_final(&mut receiver).await,
        (1, "Synthetic first final".into())
    );
    assert_eq!(
        next_final(&mut receiver).await,
        (2, "Synthetic second final".into())
    );
    assert!(client
        .inner
        .content
        .lock()
        .unwrap()
        .pending_started
        .is_empty());
    client.finish(Duration::from_secs(1)).await;
    server.await.unwrap();
}

#[tokio::test]
async fn stop_flushes_short_voiced_tail_and_waits_for_final_before_finished() {
    use base64::Engine;
    let (client, mut receiver, listener) = fixture().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        setup(&mut socket).await;
        let append = wire(&mut socket).await;
        assert_eq!(append["type"], "input_audio_buffer.append");
        let pcm = base64::engine::general_purpose::STANDARD
            .decode(append["audio"].as_str().unwrap())
            .unwrap();
        assert_eq!(pcm.len(), 4800);
        assert_eq!(pcm[..2], 3000_i16.to_le_bytes());
        assert!(pcm[2..].iter().all(|byte| *byte == 0));
        assert_eq!(wire(&mut socket).await["type"], "input_audio_buffer.commit");
        send(
            &mut socket,
            json!({"type":"input_audio_buffer.committed","item_id":"tail"}),
        )
        .await;
        completed(&mut socket, "tail", "Synthetic stopped tail").await;
        assert!(matches!(
            socket.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    });
    client.connect("synthetic-stop").await.unwrap();
    client.send_audio(&3000_i16.to_le_bytes()).await.unwrap();
    client.finish(Duration::from_secs(1)).await;
    assert_eq!(
        next_final(&mut receiver).await,
        (1, "Synthetic stopped tail".into())
    );
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionFinished)
    );
    server.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_cannot_overtake_a_retired_completion_before_publication() {
    for text in ["", " \t ", "Synthetic accepted final."] {
        let (client, mut receiver, listener) = fixture().await;
        let (complete_tx, complete_rx) = tokio::sync::oneshot::channel();
        let (publication_tx, publication_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
        *client.inner.before_final_publication.lock().unwrap() = Some(Box::new(move || {
            // Hold the exact interval after turn retirement and before
            // publication while another worker starts session finish.
            publication_tx.send(()).unwrap();
            resume_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        }));
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            setup(&mut socket).await;
            committed(&mut socket, "tail").await;
            complete_rx.await.unwrap();
            completed(&mut socket, "tail", text).await;
            assert!(matches!(
                socket.next().await,
                Some(Ok(Message::Close(_))) | None
            ));
        });
        client.connect("synthetic-publication-race").await.unwrap();
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionCreated)
        );
        client.send_audio(&voice_then_silence()).await.unwrap();
        complete_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), publication_rx)
            .await
            .unwrap()
            .unwrap();
        let (finish_started_tx, finish_started_rx) = tokio::sync::oneshot::channel();
        let finishing = tokio::spawn(async move {
            finish_started_tx.send(()).unwrap();
            client.finish(Duration::from_secs(1)).await;
        });
        finish_started_rx.await.unwrap();
        let premature = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .ok()
            .flatten();
        // Release before asserting so a failing regression cannot strand
        // the receiver or leave a loopback connection alive.
        resume_tx.send(()).unwrap();
        finishing.await.unwrap();
        server.await.unwrap();
        assert_eq!(premature, None, "finish overtook completion {text:?}");
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 1,
                text: text.into(),
                language: Some("en".into()),
            })
        );
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        );
    }
}

#[tokio::test]
async fn clear_waits_for_buffer_barrier_and_cannot_republish_old_callbacks() {
    let (client, mut receiver, listener) = fixture().await;
    let (old_committed_tx, old_committed_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        setup(&mut socket).await;
        committed(&mut socket, "old").await;
        old_committed_tx.send(()).unwrap();
        assert_eq!(wire(&mut socket).await["type"], "input_audio_buffer.clear");
        send(&mut socket, json!({"type":"conversation.item.input_audio_transcription.delta","item_id":"old","delta":"Synthetic stale preview"})).await;
        send(&mut socket, json!({"type":"input_audio_buffer.cleared"})).await;
        completed(&mut socket, "old", "Synthetic old final").await;
        committed(&mut socket, "new").await;
        completed(&mut socket, "new", "Synthetic new final").await;
        let _ = socket.next().await;
    });
    client.connect("synthetic-clear").await.unwrap();
    client.send_audio(&voice_then_silence()).await.unwrap();
    old_committed_rx.await.unwrap();
    assert_eq!(client.clear_content().await, 1);
    client.send_audio(&voice_then_silence()).await.unwrap();
    assert_eq!(
        next_final(&mut receiver).await,
        (2, "Synthetic new final".into())
    );
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn idle_capture_gap_commits_and_stop_deadline_cancels_missing_final() {
    let (client, mut receiver, listener) = fixture().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        setup(&mut socket).await;
        committed(&mut socket, "missing-final").await;
        assert!(matches!(
            socket.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    });
    client.connect("synthetic-idle").await.unwrap();
    client
        .send_audio(
            &3000_i16
                .to_le_bytes()
                .into_iter()
                .cycle()
                .take(960)
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    {
        let mut content = client.inner.content.lock().unwrap();
        content.last_pcm_at = Some(tokio::time::Instant::now() - IDLE_COMMIT_DELAY);
    }
    client.tick().await.unwrap();
    let started = tokio::time::Instant::now();
    client.finish(Duration::from_millis(40)).await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(client.inner.content.lock().unwrap().turns.is_empty());
    while !matches!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionFinished)
    ) {}
    server.await.unwrap();
}

#[tokio::test]
async fn malformed_protocol_and_stale_generations_fail_without_content_or_body() {
    let (client, mut receiver, listener) = fixture().await;
    let (malformed_tx, malformed_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        setup(&mut socket).await;
        malformed_rx.await.unwrap();
        send(&mut socket, json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"unknown","transcript":"Synthetic uncommitted final"})).await;
        let _ = socket.next().await;
    });
    client.connect("synthetic-malformed").await.unwrap();
    let generation = client.inner.generation.load(Ordering::SeqCst);
    malformed_tx.send(()).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(LiveTranslateServerEvent::Error { code, message }) = receiver.recv().await {
                return (code, message);
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        error,
        (
            "custom_speech_protocol_invalid".into(),
            "custom_speech_protocol_invalid".into()
        )
    );
    client.disconnect().await;
    client.emit(
        LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 99,
            text: "Synthetic stale generation".into(),
            language: None,
        },
        None,
        generation,
    );
    client.fail(CustomSpeechClientError::SessionRejected, generation);
    assert!(
        tokio::time::timeout(Duration::from_millis(40), receiver.recv())
            .await
            .is_err()
    );
    server.await.unwrap();
}

#[tokio::test]
async fn every_pending_turn_has_its_own_bounded_completion_deadline() {
    let (client, _receiver, _listener) = fixture().await;
    let now = tokio::time::Instant::now();
    let mut content = client.inner.content.lock().unwrap();
    let first = content.turns.start(0).unwrap();
    content.turns.commit().unwrap();
    content.turns.acknowledge("first".into()).unwrap();
    let second = content.turns.start(0).unwrap();
    content.turns.commit().unwrap();
    content.turns.acknowledge("second".into()).unwrap();
    content
        .pending_started
        .push_back((first, now - MAX_FINAL_WAIT));
    content.pending_started.push_back((second, now));
    content
        .turns
        .complete("first".into(), "Synthetic first".into(), 0)
        .unwrap();
    trim_pending_started(&mut content);
    assert_eq!(content.pending_started.front(), Some(&(second, now)));
}

#[tokio::test]
async fn empty_completion_retires_its_deadline_without_refunding_a_later_turn() {
    for final_text in ["", " \t "] {
        let (client, _receiver, _listener) = fixture().await;
        client.inner.ready.store(true, Ordering::SeqCst);
        let now = tokio::time::Instant::now();
        {
            let mut content = client.inner.content.lock().unwrap();
            let first = content.turns.start(0).unwrap();
            content.turns.commit().unwrap();
            content.turns.acknowledge("first".into()).unwrap();
            let second = content.turns.start(0).unwrap();
            content.turns.commit().unwrap();
            content.turns.acknowledge("second".into()).unwrap();
            content
                .pending_started
                .push_back((first, now - MAX_FINAL_WAIT));
            content.pending_started.push_back((second, now));
            let output = content
                .turns
                .complete("first".into(), final_text.into(), 0)
                .unwrap();
            assert_eq!(output.len(), 1);
            assert_eq!(output[0].sequence, first);
            trim_pending_started(&mut content);
            assert_eq!(content.pending_started.front(), Some(&(second, now)));
        }
        assert!(client.tick().await.is_ok());
        client
            .inner
            .content
            .lock()
            .unwrap()
            .pending_started
            .front_mut()
            .unwrap()
            .1 = now - MAX_FINAL_WAIT;
        assert!(matches!(
            client.tick().await,
            Err(CustomSpeechClientError::InvalidProtocol)
        ));
    }
}
