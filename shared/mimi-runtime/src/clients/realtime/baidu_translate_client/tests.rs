#[test]
fn audio_clock_absorbs_small_send_delays_without_accumulating_catch_up_credit() {
    let start = tokio::time::Instant::now();
    let mut next = start;
    // Twenty minutes of 3-ms wake/write overhead must not become 90
    // seconds of queued audio behind a real-time capture producer.
    for frame in 1..=30_000 {
        next = next_audio_frame_deadline(next, next + Duration::from_millis(3));
        assert_eq!(
            next.duration_since(start),
            Duration::from_millis(frame * 40)
        );
    }
    let delayed = next + Duration::from_secs(5);
    next = next_audio_frame_deadline(next, delayed);
    assert_eq!(next, delayed);
    next = next_audio_frame_deadline(next, delayed + Duration::from_millis(3));
    assert_eq!(next, delayed + Duration::from_millis(40));
}

#[tokio::test]
async fn handshake_authentication_rejections_preserve_transport_failures() {
    for (status, expected_auth_label) in
        crate::clients::transport::connection_diagnostics::handshake_failure_cases()
    {
        let (endpoint, server) =
            crate::clients::transport::connection_diagnostics::rejected_websocket_endpoint(status)
                .await;
        let (events, _receiver) = provider_event_channel();
        let mut client = BaiduTranslateClient::with_endpoint(
            SourceLanguage::Japanese,
            TargetLanguage::English,
            events,
            endpoint,
        );
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
            assert_eq!(error, BaiduTranslateClientError::AuthenticationFailed);
            assert_eq!(error.to_string(), expected_label);
        } else {
            assert_eq!(error, BaiduTranslateClientError::TransportFailure);
        }
        assert!(!error.to_string().contains("private-handshake-body"));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
    }
}

#[test]
fn clear_observes_an_already_decoded_old_end_without_dropping_the_next_sentence() {
    let mut turn = SentenceContentState {
        in_progress: false,
        discard_until_end: true,
    };
    assert!(!turn.accepts_content(0, 1, false));
    assert!(turn.discard_until_end);
    assert!(!turn.accepts_content(0, 1, true));
    assert!(!turn.discard_until_end);
    assert!(turn.accepts_content(1, 1, false));
    assert!(turn.in_progress);
    assert!(turn.accepts_content(1, 1, true));
    assert!(!turn.in_progress);
}

#[tokio::test]
async fn clear_ignores_current_sentence_tail_but_keeps_socket_and_next_sentence() {
    let (resume, resumed) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket.send(Message::Text(r#"{"code":0,"msg":"Success","data":{"status":"STA"}}"#.into())).await.unwrap();
            socket.send(Message::Text(r#"{"code":0,"data":{"status":"TRN","result":{"type":"MID","asr":"old source","asr_trans":"old translation","sentence":"","sentence_trans":""}}}"#.into())).await.unwrap();
            resumed.await.unwrap();
            for frame in [r#"{"code":0,"data":{"status":"TRN","result":{"type":"FIN","asr":"","asr_trans":"","sentence":"old source final","sentence_trans":"old translation final"}}}"#, r#"{"code":0,"data":{"status":"TRN","result":{"type":"MID","asr":"new source","asr_trans":"new translation","sentence":"","sentence_trans":""}}}"#, r#"{"code":0,"data":{"status":"TRN","result":{"type":"FIN","asr":"","asr_trans":"","sentence":"new source final","sentence_trans":"new translation final"}}}"#] {
                socket.send(Message::Text(frame.into())).await.unwrap();
            }
            while let Some(Ok(message)) = socket.next().await {
                if matches!(&message, Message::Text(text) if text.contains(r#""type":"FINISH""#)) {
                    socket.send(Message::Text(r#"{"code":0,"msg":"Success","data":{"status":"END"}}"#.into())).await.unwrap();
                    break;
                }
            }
        })
    }).await;
    client
        .connect_with_timeout(Duration::from_secs(2))
        .await
        .unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(event, LiveTranslateServerEvent::SourceDraft { .. }) {
            break;
        }
    }
    let generation = client.inner.generation.load(Ordering::SeqCst);
    *client.inner.pending_audio.lock().await = vec![1, 2];
    assert_eq!(client.clear_content().await, 1);
    assert_eq!(client.content_revision(), 1);
    assert_eq!(client.inner.generation.load(Ordering::SeqCst), generation);
    assert!(client.inner.ready.load(Ordering::SeqCst));
    assert!(client.inner.sink.lock().await.is_some());
    assert_eq!(*client.inner.pending_audio.lock().await, vec![1, 2]);
    resume.send(()).unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                translation,
                ..
            } => {
                assert_eq!(source, "new source final");
                assert_eq!(translation, "new translation final");
                break;
            }
            LiveTranslateServerEvent::SourceDraft { text, .. }
            | LiveTranslateServerEvent::TranslationDraft(text) => {
                assert!(!text.starts_with("old"));
            }
            LiveTranslateServerEvent::Error { code, .. } => {
                panic!("unexpected fixed error: {code}")
            }
            _ => {}
        }
    }
    client.finish(Duration::from_secs(2)).await;
    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::SessionFinished)
    );
    client.disconnect().await;
}

use super::*;
use crate::clients::transport::provider_events::{provider_event_channel, ProviderEventReceiver};
use serde_json::Value;
use tokio::net::TcpListener;

async fn test_client(
    server: impl FnOnce(
            WebSocketStream<TcpStream>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + 'static,
) -> (BaiduTranslateClient, ProviderEventReceiver) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        server(socket).await;
    });
    let (events, receiver) = provider_event_channel();
    let endpoint = url::Url::parse(&format!("ws://{address}/realtime_speech_trans")).unwrap();
    let client = BaiduTranslateClient::with_endpoint(
        SourceLanguage::Japanese,
        TargetLanguage::English,
        events,
        endpoint,
    );
    (client, receiver)
}

#[test]
fn audio_buffer_is_bounded_to_complete_frames_and_one_tail() {
    let mut pending = vec![9; BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT * 2 + 17];
    let frames = take_complete_frames(&mut pending);
    assert_eq!(frames.len(), 2);
    assert!(frames
        .iter()
        .all(|frame| frame.len() == BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT));
    assert_eq!(pending.len(), 17);
    let padded = take_padded_frame(&mut pending).unwrap();
    assert_eq!(padded.len(), BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT);
    assert_eq!(&padded[..17], &[9; 17]);
    assert!(padded[17..].iter().all(|byte| *byte == 0));
}

#[tokio::test]
async fn invalid_audio_only_recovers_with_recent_delivered_silence() {
    for (quiet_ms, resume_sound, expired, expected) in [
        (0, false, false, "baidu_provider_20316"),
        (19_960, false, false, "baidu_provider_20316"),
        (20_000, false, false, "transport_error"),
        (20_000, true, false, "transport_error"),
        (20_000, true, true, "baidu_provider_20316"),
    ] {
        let (reject, rejected) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                socket.next().await.unwrap().unwrap();
                socket
                    .send(Message::Text(
                        r#"{"code":0,"data":{"status":"STA"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                rejected.await.unwrap();
                socket
                    .send(Message::Text(
                        r#"{"code":20316,"msg":"private detail"}"#.into(),
                    ))
                    .await
                    .unwrap();
                while socket.next().await.is_some() {}
            })
        })
        .await;
        client.connect().await.unwrap();
        let evidence_time = tokio::time::Instant::now()
            - if expired {
                Duration::from_secs(6)
            } else {
                Duration::ZERO
            };
        for _ in 0..quiet_ms / 40 {
            client
                .inner
                .delivered_silence
                .lock()
                .await
                .observe(true, evidence_time);
        }
        if resume_sound {
            client
                .send_audio(&vec![7; BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT])
                .await
                .unwrap();
        }
        reject.send(()).unwrap();
        loop {
            if let LiveTranslateServerEvent::Error { code, message } = next_event(&mut events).await
            {
                assert_eq!(code, expected);
                assert!(!message.contains("private"));
                break;
            }
        }
        client.disconnect().await;
        assert!(!client
            .inner
            .delivered_silence
            .lock()
            .await
            .qualifies(tokio::time::Instant::now()));
    }
}

async fn next_event(events: &mut ProviderEventReceiver) -> LiveTranslateServerEvent {
    tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap()
}

fn transcript_frame(kind: &str, source: &str, translation: &str, next: &str) -> String {
    serde_json::json!({"code":0,"data":{"status":"TRN","result":{
        "type":kind,
        "asr": if kind == "MID" { source } else { next },
        "asr_trans": if kind == "MID" { translation } else { "" },
        "sentence": if kind == "FIN" { source } else { "" },
        "sentence_trans": if kind == "FIN" { translation } else { "" }
    }}})
    .to_string()
}

#[tokio::test]
async fn fin_preserves_the_next_draft_and_clear_discards_its_tail() {
    for clear_previous in [false, true] {
        let (send, mut commands) = tokio::sync::mpsc::unbounded_channel::<Vec<String>>();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"code":0,"data":{"status":"STA"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                while let Some(frames) = commands.recv().await {
                    if frames.is_empty() {
                        break;
                    }
                    for frame in frames {
                        socket.send(Message::Text(frame.into())).await.unwrap();
                    }
                }
                let finish = socket.next().await.unwrap().unwrap();
                assert!(finish.to_text().unwrap().contains("FINISH"));
                socket
                    .send(Message::Text(
                        r#"{"code":0,"data":{"status":"END"}}"#.into(),
                    ))
                    .await
                    .unwrap();
            })
        })
        .await;
        client.connect().await.unwrap();
        assert_eq!(
            next_event(&mut events).await,
            LiveTranslateServerEvent::SessionCreated
        );
        assert_eq!(
            next_event(&mut events).await,
            LiveTranslateServerEvent::SessionUpdated
        );
        send.send(vec![transcript_frame(
            "MID",
            "first draft",
            "first translation",
            "",
        )])
        .unwrap();
        assert!(matches!(
            next_event(&mut events).await,
            LiveTranslateServerEvent::SourceDraft { .. }
        ));
        assert!(matches!(
            next_event(&mut events).await,
            LiveTranslateServerEvent::TranslationDraft(_)
        ));
        send.send(vec![transcript_frame("MID", "", "", "")])
            .unwrap();
        assert!(
            matches!(next_event(&mut events).await, LiveTranslateServerEvent::SourceDraft { text, .. } if text.is_empty())
        );
        assert_eq!(
            next_event(&mut events).await,
            LiveTranslateServerEvent::TranslationDraft(String::new())
        );
        assert!(client.inner.content_lock.lock().await.in_progress);
        if clear_previous {
            client.clear_content().await;
        }
        send.send(vec![transcript_frame(
            "FIN",
            "first final",
            "first final translation",
            "next draft",
        )])
        .unwrap();
        if !clear_previous {
            assert!(
                matches!(next_event(&mut events).await, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. }
                if source == "first final" && translation == "first final translation")
            );
        }
        assert!(
            matches!(next_event(&mut events).await, LiveTranslateServerEvent::SourceDraft { text, .. } if text == "next draft")
        );
        assert_eq!(
            next_event(&mut events).await,
            LiveTranslateServerEvent::TranslationDraft(String::new())
        );
        assert!(client.inner.content_lock.lock().await.in_progress);
        client.clear_content().await;
        send.send(vec![
            transcript_frame("MID", "next cleared tail", "next cleared translation", ""),
            transcript_frame(
                "FIN",
                "next cleared final",
                "next cleared final translation",
                "",
            ),
            transcript_frame("FIN", "healthy final", "healthy translation", ""),
        ])
        .unwrap();
        assert!(
            matches!(next_event(&mut events).await, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. }
            if source == "healthy final" && translation == "healthy translation")
        );
        send.send(Vec::new()).unwrap();
        client.finish(Duration::from_secs(2)).await;
        assert_eq!(
            next_event(&mut events).await,
            LiveTranslateServerEvent::SessionFinished
        );
        assert!(events.try_recv().is_err());
    }
}

#[tokio::test]
async fn sentence_failure_releases_a_cleared_turn_and_idle_timeout_uses_recovery() {
    for (code, clear_previous) in [
        (20_311, true),
        (20_312, true),
        (20_315, true),
        (20_314, true),
        (20_316, true),
        (20_312, false),
    ] {
        let (resume, resumed) = tokio::sync::oneshot::channel();
        let (healthy, healthy_allowed) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"code":0,"data":{"status":"STA"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        transcript_frame("MID", "obsolete", "obsolete translation", "").into(),
                    ))
                    .await
                    .unwrap();
                resumed.await.unwrap();
                socket
                    .send(Message::Text(
                        serde_json::json!({"code":code,"msg":"private provider content"})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                if matches!(code, 20_314 | 20_316) {
                    return;
                }
                healthy_allowed.await.unwrap();
                socket
                    .send(Message::Text(
                        transcript_frame("FIN", "healthy final", "healthy translation", "").into(),
                    ))
                    .await
                    .unwrap();
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"code":0,"data":{"status":"END"}}"#.into(),
                    ))
                    .await
                    .unwrap();
            })
        })
        .await;
        client.connect().await.unwrap();
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        controller.handle(LiveTranslateServerEvent::SubtitleFinalPair {
            source: "retained final".into(),
            language: Some("ja".into()),
            translation: "retained translation".into(),
        });
        for _ in 0..4 {
            controller.handle(next_event(&mut events).await);
        }
        if clear_previous {
            client.clear_content().await;
            controller.clear_subtitles();
        }
        if code == 20_316 {
            for _ in 0..500 {
                client
                    .inner
                    .delivered_silence
                    .lock()
                    .await
                    .observe(true, tokio::time::Instant::now());
            }
        }
        resume.send(()).unwrap();
        if matches!(code, 20_314 | 20_316) {
            let LiveTranslateServerEvent::Error { code, message } = next_event(&mut events).await
            else {
                panic!("expected timeout recovery");
            };
            assert!(crate::core::lifecycle::recovery::provider_error_is_retryable(&code));
            assert!(!message.contains("private"));
            client.disconnect().await;
        } else {
            for expected in [
                LiveTranslateServerEvent::SubtitlePreviewCleared,
                LiveTranslateServerEvent::SourceDraft {
                    text: String::new(),
                    language: Some("ja".into()),
                },
                LiveTranslateServerEvent::TranslationDraft(String::new()),
            ] {
                let event = next_event(&mut events).await;
                assert_eq!(event, expected);
                controller.handle(event);
            }
            assert!(controller.state.subtitles.realtime_preview.is_none());
            if !clear_previous {
                assert!(controller.state.subtitles.display_pair_final);
                assert_eq!(
                    controller
                        .state
                        .subtitles
                        .display_pair
                        .as_ref()
                        .unwrap()
                        .translation,
                    "retained translation"
                );
            }
            healthy.send(()).unwrap();
            assert!(
                matches!(next_event(&mut events).await, LiveTranslateServerEvent::SubtitleFinalPair { source, .. } if source == "healthy final")
            );
            client.finish(Duration::from_secs(2)).await;
            assert_eq!(
                next_event(&mut events).await,
                LiveTranslateServerEvent::SessionFinished
            );
        }
    }
}

#[tokio::test]
async fn batched_audio_is_paced_and_finish_flushes_one_padded_tail() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"code":0,"data":{"status":"STA"}}"#.into(),
                ))
                .await
                .unwrap();
            for index in 0..4 {
                let frame = socket.next().await.unwrap().unwrap().into_data();
                assert_eq!(frame.len(), BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT);
                if index == 3 {
                    assert_eq!(&frame[..2], &[9, 9]);
                    assert!(frame[2..].iter().all(|byte| *byte == 0));
                } else {
                    assert!(frame.iter().all(|byte| *byte == 9));
                }
            }
            assert!(socket
                .next()
                .await
                .unwrap()
                .unwrap()
                .to_text()
                .unwrap()
                .contains("FINISH"));
            socket
                .send(Message::Text(
                    r#"{"code":0,"data":{"status":"END"}}"#.into(),
                ))
                .await
                .unwrap();
        })
    })
    .await;
    client.connect().await.unwrap();
    let stale_generation = client.inner.generation.load(Ordering::SeqCst) - 1;
    assert_eq!(
        client
            .send_text(
                BaiduTranslateRequestEncoder::finish().to_string(),
                stale_generation
            )
            .await,
        Err(BaiduTranslateClientError::NotConnected)
    );
    assert_eq!(
        client
            .send_paced_binary(
                vec![0; BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT],
                &mut None,
                stale_generation
            )
            .await,
        Err(BaiduTranslateClientError::NotConnected)
    );
    let started = tokio::time::Instant::now();
    client
        .send_audio(&vec![
            9;
            BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT * 3 + 2
        ])
        .await
        .unwrap();
    client.finish(Duration::from_secs(2)).await;
    assert!(started.elapsed() >= Duration::from_millis(120));
    assert_eq!(
        next_event(&mut events).await,
        LiveTranslateServerEvent::SessionCreated
    );
    assert_eq!(
        next_event(&mut events).await,
        LiveTranslateServerEvent::SessionUpdated
    );
    assert_eq!(
        next_event(&mut events).await,
        LiveTranslateServerEvent::SessionFinished
    );
    assert!(client.inner.pending_audio.lock().await.is_empty());
}

#[tokio::test]
async fn mock_websocket_covers_start_audio_ping_transcript_and_finish() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let start = socket.next().await.unwrap().unwrap();
            let start: Value = serde_json::from_str(start.to_text().unwrap()).unwrap();
            assert_eq!(start["type"], "START");
            assert_eq!(start["from"], "jp");
            assert_eq!(start["to"], "en");
            assert_eq!(start["sampling_rate"], 16_000);
            socket
                .send(Message::Text(
                    r#"{"code":0,"msg":"Success","data":{"status":"STA"}}"#.into(),
                ))
                .await
                .unwrap();
            let mut saw_audio = false;
            while let Some(Ok(message)) = socket.next().await {
                match message {
                    Message::Binary(frame) => {
                        assert_eq!(frame.len(), BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT);
                        assert!(frame.iter().all(|byte| *byte == 9));
                        saw_audio = true;
                    }
                    Message::Ping(payload) => {
                        socket.send(Message::Pong(payload)).await.unwrap();
                    }
                    Message::Text(text) if text.contains(r#""type":"FINISH""#) => {
                        assert!(saw_audio);
                        socket.send(Message::Text(
                            r#"{"code":0,"data":{"status":"TRN","result":{"type":"MID","asr":"こんにちは","asr_trans":"Hello","sentence":"","sentence_trans":""}}}"#.into()
                        )).await.unwrap();
                        socket.send(Message::Text(
                            r#"{"code":0,"data":{"status":"TRN","result":{"type":"FIN","asr":"","asr_trans":"","sentence":"こんにちは。","sentence_trans":"Hello."}}}"#.into()
                        )).await.unwrap();
                        socket.send(Message::Text(
                            r#"{"code":0,"msg":"Success","data":{"status":"END"}}"#.into()
                        )).await.unwrap();
                        break;
                    }
                    _ => {}
                }
            }
        })
    })
    .await;

    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    client
        .send_audio(&vec![9; BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    client.ping(Duration::from_millis(500)).await.unwrap();
    client.finish(Duration::from_millis(500)).await;

    let mut saw_pair = false;
    let mut saw_finished = false;
    for _ in 0..8 {
        let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(100), events.recv()).await
        else {
            break;
        };
        match event {
            LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                language,
                translation,
            } => {
                assert_eq!(source, "こんにちは。");
                assert_eq!(language.as_deref(), Some("ja"));
                assert_eq!(translation, "Hello.");
                saw_pair = true;
            }
            LiveTranslateServerEvent::SessionFinished => saw_finished = true,
            LiveTranslateServerEvent::Error { code, .. } => {
                panic!("unexpected error event: {code}")
            }
            _ => {}
        }
    }
    assert!(saw_pair);
    assert!(saw_finished);
}

#[tokio::test]
async fn unsolicited_end_is_a_sanitized_error_not_a_clean_finish() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"code":0,"msg":"Success","data":{"status":"STA"}}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"code":0,"msg":"private provider detail","data":{"status":"END"}}"#.into(),
                ))
                .await
                .unwrap();
        })
    })
    .await;

    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();

    let mut saw_error = false;
    for _ in 0..4 {
        let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(100), events.recv()).await
        else {
            break;
        };
        match event {
            LiveTranslateServerEvent::Error { code, message } => {
                assert_eq!(code, UNEXPECTED_SESSION_END_CODE);
                assert_eq!(message, UNEXPECTED_SESSION_END_ERROR);
                assert!(!message.contains("private provider detail"));
                saw_error = true;
            }
            LiveTranslateServerEvent::SessionFinished => {
                panic!("unsolicited END must not look like a clean finish")
            }
            _ => {}
        }
    }
    assert!(saw_error);
}

#[tokio::test]
async fn recoverable_sentence_error_keeps_the_stream_alive() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"code":0,"data":{"status":"STA"}}"#.into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"code":20312,"msg":"private provider detail"}"#.into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if message.to_text().is_ok_and(|text| text.contains("FINISH")) {
                    socket
                        .send(Message::Text(
                            r#"{"code":0,"data":{"status":"END"}}"#.into(),
                        ))
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
    client.finish(Duration::from_millis(500)).await;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await
    {
        assert!(!matches!(event, LiveTranslateServerEvent::Error { .. }));
        if matches!(event, LiveTranslateServerEvent::SessionFinished) {
            return;
        }
    }
    panic!("missing SessionFinished");
}

#[tokio::test]
async fn baidu_setup_preserves_explicit_rejection_and_transport_categories() {
    use crate::clients::translation::translation_client::ConnectError;
    for (frame, expected, safe_code) in [
        (
            None,
            BaiduTranslateClientError::TransportFailure,
            "transport_error",
        ),
        (
            Some(r#"{"code":0,"data":{"status":"END"}}"#),
            BaiduTranslateClientError::TransportFailure,
            "transport_error",
        ),
        (
            Some(r#"{"code":20314,"msg":"private provider detail"}"#),
            BaiduTranslateClientError::SessionSetupRejectedWithCode(20_314),
            "transport_error",
        ),
        (
            Some(r#"{"code":20316,"msg":"private provider detail"}"#),
            BaiduTranslateClientError::SessionSetupRejectedWithCode(20_316),
            "provider_error",
        ),
        (
            Some(r#"{"code":99999,"msg":"private provider detail"}"#),
            BaiduTranslateClientError::SessionSetupRejectedWithCode(99_999),
            "provider_error",
        ),
        (
            Some("invalid-json"),
            BaiduTranslateClientError::SessionSetupRejected,
            "provider_error",
        ),
    ] {
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await.unwrap().unwrap();
                if let Some(frame) = frame {
                    socket.send(Message::Text(frame.into())).await.unwrap();
                    while socket.next().await.is_some() {}
                } else {
                    socket.close(None).await.unwrap();
                }
            })
        })
        .await;
        let error = client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap_err();
        assert_eq!(error, expected, "frame={frame:?}");
        assert!(!error.to_string().contains("private provider detail"));
        assert_eq!(ConnectError::BaiduTranslate(error).safe_code(), safe_code);
        assert!(!client.inner.ready.load(Ordering::SeqCst));
        assert!(events.try_recv().is_err());
        client.disconnect().await;
    }
}

#[tokio::test]
async fn setup_timeout_is_finite() {
    let (client, _events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        })
    })
    .await;
    assert_eq!(
        client
            .connect_with_timeout(Duration::from_millis(30))
            .await
            .unwrap_err(),
        BaiduTranslateClientError::SessionSetupTimedOut
    );
}

#[tokio::test]
async fn independent_review_fin_retains_its_next_draft() {
    let (publish, mut commands) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"code":0,"data":{"status":"STA"}}"#.into(),
                ))
                .await
                .unwrap();
            while let Some(frame) = commands.recv().await {
                if frame.is_empty() {
                    break;
                }
                socket.send(Message::Text(frame.into())).await.unwrap();
            }
        })
    })
    .await;
    client.connect().await.unwrap();
    publish.send(r#"{"code":0,"data":{"status":"TRN","result":{"type":"FIN","sentence":"Synthetic A final","sentence_trans":"Translated A final","asr":"Synthetic B draft","asr_trans":"Translated B draft"}}}"#.into()).unwrap();
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    tokio::time::timeout(Duration::from_secs(2), async {
        while controller.state.subtitles.history.is_empty() {
            controller.handle(events.recv().await.unwrap());
        }
    })
    .await
    .unwrap();
    while let Ok(event) = events.try_recv() {
        controller.handle(event);
    }
    assert_eq!(
        controller.state.subtitles.history[0].source,
        "Synthetic A final"
    );
    assert_eq!(
        controller.state.subtitles.history[0].translation,
        "Translated A final"
    );
    assert_eq!(
        controller.state.subtitles.source.text, "Synthetic B draft",
        "FIN must carry the next sentence independently"
    );
    assert_eq!(
        controller.state.subtitles.translation.text,
        "Translated B draft"
    );
    publish.send(String::new()).unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn finish_returns_promptly_when_server_closes_without_session_finished() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"code":0,"msg":"Success","data":{"status":"STA"}}"#.into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if matches!(&message, Message::Text(text) if text.contains(r#""type":"FINISH""#)) {
                    let _ = socket.close(None).await;
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
    let started = tokio::time::Instant::now();
    client.finish(Duration::from_secs(5)).await;
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "finish should wake immediately when the receive loop terminates"
    );
    let mut error_codes = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let LiveTranslateServerEvent::Error { code, .. } = event {
            error_codes.push(code);
        }
    }
    assert_eq!(error_codes, vec!["transport_error"]);
}
