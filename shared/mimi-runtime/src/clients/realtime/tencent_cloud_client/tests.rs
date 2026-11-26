#[tokio::test]
async fn setup_rejections_preserve_status_without_provider_message_content() {
    let contract: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    for case in contract["tencentSpeechTranslation"]["errors"]
        .as_array()
        .unwrap()
    {
        let code = case["code"].as_i64().unwrap();
        let expected_label = case["label"].as_str().unwrap();
        let (client, _events) = test_client(move |mut socket| {
            Box::pin(async move {
                let rejected = serde_json::json!({
                    "code": code,
                    "message": "private-provider-response-secret",
                });
                socket
                    .send(Message::Text(rejected.to_string().into()))
                    .await
                    .unwrap();
            })
        })
        .await;
        let error = client
            .connect_with_timeout(Duration::from_secs(2))
            .await
            .unwrap_err();
        assert_eq!(error, TencentCloudClientError::ProviderRejected { code });
        assert_eq!(error.to_string(), expected_label);
        assert!(!format!("{error:?}").contains("private-provider-response-secret"));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn active_session_rejection_keeps_code_and_a_fixed_safe_label() {
    let (reject, rejected) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            socket
                .send(Message::Text(
                    r#"{"code":0,"message":"success","voice_id":"v"}"#.into(),
                ))
                .await
                .unwrap();
            rejected.await.unwrap();
            socket
                .send(Message::Text(
                    r#"{"code":6005,"message":"private-provider-response-secret"}"#.into(),
                ))
                .await
                .unwrap();
        })
    })
    .await;
    client
        .connect_with_timeout(Duration::from_secs(2))
        .await
        .unwrap();
    reject.send(()).unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        if let LiveTranslateServerEvent::Error { code, message } = event {
            assert_eq!(code, "tencent_provider_6005");
            assert_eq!(message, "tencent_quota_exhausted");
            break;
        }
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        while !client.inner.receive_ended.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        client.send_audio(&vec![0; 6400]).await,
        Err(TencentCloudClientError::NotConnected)
    );
    assert_eq!(
        client.ping(Duration::from_millis(50)).await,
        Err(TencentCloudClientError::NotConnected)
    );
    client.disconnect().await;
}

#[tokio::test]
async fn rejection_during_finish_closes_without_a_second_timeout_error() {
    let (closed, close_observation) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            socket
                .send(Message::Text(r#"{"code":0}"#.into()))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if matches!(&message, Message::Text(text) if text.contains(r#""type":"end""#)) {
                    socket
                        .send(Message::Text(
                            r#"{"code":6004,"message":"private provider detail"}"#.into(),
                        ))
                        .await
                        .unwrap();
                }
                if matches!(message, Message::Close(_)) {
                    closed.send(()).unwrap();
                    return;
                }
            }
            panic!("client did not close its rejected session");
        })
    })
    .await;
    client.connect().await.unwrap();
    tokio::time::timeout(
        Duration::from_millis(500),
        client.finish(Duration::from_secs(2)),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), close_observation)
        .await
        .unwrap()
        .unwrap();
    let mut errors = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let LiveTranslateServerEvent::Error { code, message } = event {
            errors.push((code, message));
        }
    }
    assert_eq!(
        errors,
        vec![(
            "tencent_provider_6004".into(),
            "tencent_quota_exhausted".into()
        )]
    );
    assert!(!client.inner.ready.load(Ordering::SeqCst));
    assert!(client.inner.sink.lock().await.is_none());
}

#[test]
fn audio_frame_deadlines_do_not_accumulate_timer_overshoot() {
    let frame = Duration::from_millis(TencentCloudEndpoint::FRAME_DURATION_MS.into());
    let start = tokio::time::Instant::now();
    let mut scheduled = start;
    // Windows timers wake 1-14 ms late; 120 frames is 24 s of audio, about
    // when the relative cadence used to overflow the 20-frame queue.
    for index in 0..120u32 {
        let overshoot = Duration::from_millis(u64::from(1 + index % 14));
        let woke_at = scheduled + overshoot;
        scheduled = next_audio_frame_deadline(scheduled, woke_at);
        assert_eq!(scheduled, start + frame * (index + 1));
    }
}

#[test]
fn a_late_audio_frame_is_sent_at_once_without_catch_up_credit() {
    let frame = Duration::from_millis(TencentCloudEndpoint::FRAME_DURATION_MS.into());
    let scheduled = tokio::time::Instant::now();
    let after_stall = scheduled + Duration::from_secs(2);
    let next = next_audio_frame_deadline(scheduled, after_stall);
    assert_eq!(next, after_stall);
    assert_eq!(next_audio_frame_deadline(next, next), next + frame);
}

#[tokio::test]
async fn queued_pcm_frames_are_paced_at_the_documented_realtime_rate() {
    let (observed, observation) = tokio::sync::oneshot::channel();
    let (client, _events) = test_client(move |mut socket| {
        Box::pin(async move {
            socket
                .send(Message::Text(r#"{"code":0}"#.into()))
                .await
                .unwrap();
            let mut frames = Vec::new();
            while let Some(Ok(message)) = socket.next().await {
                if let Message::Binary(frame) = message {
                    assert_eq!(frame.len(), TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT);
                    frames.push(tokio::time::Instant::now());
                    if frames.len() == 3 {
                        observed.send(frames).unwrap();
                        return;
                    }
                }
            }
            panic!("missing paced audio frames");
        })
    })
    .await;
    client.connect().await.unwrap();
    client
        .send_audio(&vec![0; TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT * 3])
        .await
        .unwrap();
    let frames = tokio::time::timeout(Duration::from_secs(2), observation)
        .await
        .unwrap()
        .unwrap();
    // Allow small local scheduling/arrival jitter without allowing a burst.
    assert!(frames
        .windows(2)
        .all(|pair| pair[1].duration_since(pair[0]) >= Duration::from_millis(180)));
    client.disconnect().await;
}

/// Bound real socket work even while the Tokio pacing clock is frozen.
async fn while_draining_fixture<F: std::future::Future>(drain: F) -> F::Output {
    let (finished, wait) = std::sync::mpsc::channel::<()>();
    let watchdog = tokio::task::spawn_blocking(move || {
        let _ = wait.recv_timeout(Duration::from_secs(2));
    });
    // An active blocking task also prevents Tokio from auto-advancing a
    // frozen clock while the sockets are waiting for the OS.
    tokio::select! {
        result = drain => {
            drop(finished);
            result
        }
        _ = watchdog => panic!("timed out draining the blocked WebSocket fixture"),
    }
}

#[tokio::test]
async fn a_blocked_socket_write_does_not_leave_an_extra_catch_up_frame() {
    let test_started = tokio::time::Instant::now();
    let (resume_reads, reads_resumed) = tokio::sync::oneshot::channel();
    let (stalled_frame_index, stalled_frame_to_read) = tokio::sync::oneshot::channel();
    let (drained, write_drained) = tokio::sync::oneshot::channel();
    let (observed, observation) = tokio::sync::oneshot::channel();
    const PRIMER_BYTES: usize = 16 * 1024 * 1024;
    let (client, _events) = test_client(move |mut socket| {
        Box::pin(async move {
            tokio::time::timeout(Duration::from_secs(5), async {
                socket
                    .send(Message::Text(r#"{"code":0}"#.into()))
                    .await
                    .unwrap();
                reads_resumed.await.unwrap();
                let stalled_frame = stalled_frame_to_read.await.unwrap();
                let mut drained = Some(drained);
                let mut frames = 0;
                let mut primer_decoded_at = None;
                while let Some(Ok(message)) = socket.next().await {
                    if let Message::Binary(frame) = message {
                        if frame.len() == PRIMER_BYTES {
                            primer_decoded_at = Some(test_started.elapsed());
                            continue;
                        }
                        assert_eq!(frame.len(), TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT);
                        frames += 1;
                        if frames == stalled_frame {
                            drained
                                .take()
                                .unwrap()
                                .send(primer_decoded_at.unwrap())
                                .unwrap();
                        }
                        if frames == stalled_frame + 2 {
                            observed.send(()).unwrap();
                            return;
                        }
                    }
                }
                panic!("missing paced frames after blocked write");
            })
            .await
            .unwrap();
        })
    })
    .await;
    client.connect().await.unwrap();
    // Fill the actual local WebSocket's write buffer while its peer is not
    // reading. Some kernels still accept an initial PCM frame, so identify
    // the send that actually blocks instead of assuming it is the first.
    tokio::time::timeout(Duration::from_secs(2), async {
        client
            .inner
            .sink
            .lock()
            .await
            .as_mut()
            .unwrap()
            .feed(Message::Binary(vec![0; PRIMER_BYTES].into()))
            .await
            .unwrap();
    })
    .await
    .unwrap();
    let audio = vec![0; TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT];
    let mut completed = Vec::new();
    let mut stalled_frame = None;
    let mut resume_reads = Some(resume_reads);
    let mut stalled_frame_index = Some(stalled_frame_index);
    let mut write_drained = Some(write_drained);
    for frame_index in 1..=6 {
        assert!(
            stalled_frame.is_some() || frame_index <= 4,
            "fixture did not block a socket write"
        );
        let previous_deadline = *client.inner.next_audio_frame_at.lock().await;
        let started_at = tokio::time::Instant::now();
        let send = client.send_audio(&audio);
        tokio::pin!(send);
        if stalled_frame.is_none() {
            tokio::select! {
                // Poll the write before the timer so an already-complete
                // send cannot be mistaken for socket backpressure.
                biased;
                result = &mut send => result.unwrap(),
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    let scheduled_at = previous_deadline.unwrap_or(started_at);
                    assert!(
                        scheduled_at.elapsed() >= Duration::from_millis(200),
                        "fixture only waited for normal audio pacing"
                    );
                    stalled_frame = Some(frame_index);
                    stalled_frame_index.take().unwrap().send(frame_index).unwrap();
                    eprintln!(
                        "stalled_frame={frame_index} reader_resumed={:?}",
                        test_started.elapsed()
                    );
                    // Send completion only means the local socket accepted
                    // the bytes. Freeze the clock while the peer consumes
                    // the synthetic backlog so the acknowledgment cannot
                    // spend part of the next real pacing interval.
                    tokio::time::pause();
                    resume_reads.take().unwrap().send(()).unwrap();
                    let (stalled_write_completed, primer_decoded_at) = while_draining_fixture(async {
                        (&mut send).await.unwrap();
                        let completed = std::time::Instant::now();
                        let decoded = write_drained.take().unwrap().await.unwrap();
                        (completed, decoded)
                    }).await;
                    assert!(
                        stalled_write_completed.elapsed() < Duration::from_millis(180),
                        "the local fixture took too long to acknowledge the drain"
                    );
                    tokio::time::resume();
                    eprintln!(
                        "primer_decoded={primer_decoded_at:?} stalled_frame_drained={:?}",
                        test_started.elapsed()
                    );
                },
            }
        } else {
            tokio::time::timeout(Duration::from_secs(2), &mut send)
                .await
                .unwrap()
                .unwrap();
        }
        let completed_at = tokio::time::Instant::now();
        let next_deadline = *client.inner.next_audio_frame_at.lock().await;
        if stalled_frame == Some(frame_index) {
            assert!(
                next_deadline.is_some_and(|deadline| deadline <= completed_at),
                "the stalled write must make one frame immediately due"
            );
        }
        eprintln!(
            "frame={frame_index} started={:?} completed={:?} previous_deadline={:?} next_deadline={:?}",
            started_at.duration_since(test_started),
            completed_at.duration_since(test_started),
            previous_deadline.map(|at| at.duration_since(test_started)),
            next_deadline.map(|at| at.duration_since(test_started)),
        );
        completed.push(completed_at);
        if stalled_frame.is_some_and(|index| frame_index == index + 2) {
            break;
        }
    }
    tokio::time::timeout(Duration::from_secs(2), observation)
        .await
        .unwrap()
        .unwrap();
    client.disconnect().await;
    // The stalled completion can be followed by one immediately due frame;
    // the next frame must return to the 200-ms clock instead of bursting too.
    let final_gap = completed[completed.len() - 1].duration_since(completed[completed.len() - 2]);
    assert!(
        final_gap >= Duration::from_millis(180),
        "frames after stalled frame {stalled_frame:?} were only {final_gap:?} apart"
    );
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
        let mut client = TencentCloudClient::with_endpoint(
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
            assert_eq!(error, TencentCloudClientError::AuthenticationFailed);
            assert_eq!(error.to_string(), expected_label);
        } else {
            assert_eq!(error, TencentCloudClientError::TransportFailure);
        }
        assert!(!error.to_string().contains("private-handshake-body"));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
    }
}

fn sentence(id: &str, start: u64, end: u64) -> TencentCloudSentence {
    TencentCloudSentence {
        id: id.into(),
        start_time_ms: start,
        end_time_ms: end,
    }
}

#[test]
fn clear_observes_an_already_decoded_old_end_without_dropping_the_next_sentence() {
    let mut turn = SentenceContentState::default();
    assert!(turn.accepts_content(0, 0, sentence("A", 0, 800), false));
    turn.clear();
    assert!(!turn.accepts_content(0, 1, sentence("A", 0, 900), false));
    assert!(!turn.accepts_content(0, 1, sentence("A", 0, 1000), true));
    assert!(turn.accepts_content(1, 1, sentence("B", 1000, 1800), false));
    assert!(turn.accepts_content(1, 1, sentence("B", 1000, 2000), true));
}

#[test]
fn sentence_metadata_rejects_old_drafts_duplicates_and_time_regressions() {
    let mut turn = SentenceContentState::default();
    assert!(turn.accepts_content(0, 0, sentence("A", 0, 800), false));
    assert!(!turn.accepts_content(0, 0, sentence("A", 0, 700), false));
    assert!(!turn.accepts_content(0, 0, sentence("A", 1, 900), true));
    assert!(turn.accepts_content(0, 0, sentence("B", 1000, 1800), false));
    assert!(!turn.accepts_content(0, 0, sentence("A", 0, 900), false));
    // A's first complete pair remains durable, without taking over B.
    assert!(turn.accepts_content(0, 0, sentence("A", 0, 1000), true));
    assert!(!turn.accepts_content(0, 0, sentence("A", 0, 1000), true));
    assert!(turn.accepts_content(0, 0, sentence("B", 1000, 2000), true));
    assert!(!turn.accepts_content(0, 0, sentence("B", 1000, 2100), false));
    assert!(!turn.accepts_content(0, 0, sentence("unseen-old", 500, 700), true));
}

#[test]
fn clear_does_not_wait_for_a_missing_final_and_metadata_stays_bounded() {
    let mut turn = SentenceContentState::default();
    assert!(turn.accepts_content(0, 0, sentence("A", 0, 800), false));
    turn.clear();
    assert!(turn.accepts_content(1, 1, sentence("B", 1000, 1800), false));
    assert!(!turn.accepts_content(1, 1, sentence("A", 0, 1000), true));
    for index in 2..1000 {
        assert!(turn.accepts_content(
            1,
            1,
            sentence(&index.to_string(), index * 1000, index * 1000 + 800),
            true
        ));
    }
    assert_eq!(turn.sentences.len(), MAX_TRACKED_SENTENCES);
    assert!(!turn.accepts_content(1, 1, sentence("A", 0, 1000), true));
}

#[tokio::test]
async fn clear_ignores_current_sentence_tail_but_keeps_socket_and_next_sentence() {
    let (resume, resumed) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            socket.send(Message::Text(r#"{"code":0,"message":"success","voice_id":"v"}"#.into())).await.unwrap();
            socket.send(Message::Text(r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"old source","target_text":"old translation","sentence_end":false,"start_time":0,"end_time":800},"sentence_id":"old-sentence"}"#.into())).await.unwrap();
            resumed.await.unwrap();
            for frame in [r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"old source final","target_text":"old translation final","sentence_end":true,"start_time":0,"end_time":800},"sentence_id":"old-sentence"}"#, r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"new source","target_text":"new translation","sentence_end":false,"start_time":1000,"end_time":1800},"sentence_id":"new-sentence"}"#, r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"new source final","target_text":"new translation final","sentence_end":true,"start_time":1000,"end_time":1800},"sentence_id":"new-sentence"}"#] {
                socket.send(Message::Text(frame.into())).await.unwrap();
            }
            while let Some(Ok(message)) = socket.next().await {
                if matches!(&message, Message::Text(text) if text.contains(r#""type":"end""#)) {
                    socket.send(Message::Text(r#"{"code":0,"final":1}"#.into())).await.unwrap();
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
        if matches!(
            event,
            LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair { .. }
        ) {
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
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                source,
                translation,
                ..
            } => {
                assert_eq!(source, "new source final");
                assert_eq!(translation, "new translation final");
                break;
            }
            LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair {
                source,
                translation,
                ..
            } => {
                assert!(!source.starts_with("old"));
                assert!(!translation.starts_with("old"));
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
use tokio::net::TcpListener;

async fn test_client(
    server: impl FnOnce(
            WebSocketStream<TcpStream>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + 'static,
) -> (TencentCloudClient, ProviderEventReceiver) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        server(socket).await;
    });
    let (events, receiver) = provider_event_channel();
    let endpoint = url::Url::parse(&format!("ws://{address}/speech_translate")).unwrap();
    let client = TencentCloudClient::with_endpoint(
        SourceLanguage::Chinese,
        TargetLanguage::Japanese,
        events,
        endpoint,
    );
    (client, receiver)
}

#[test]
fn audio_buffer_is_bounded_to_complete_frames_and_one_tail() {
    let mut pending = vec![7; TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT * 2 + 19];
    let frames = take_complete_frames(&mut pending);
    assert_eq!(frames.len(), 2);
    assert!(frames
        .iter()
        .all(|frame| frame.len() == TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT));
    assert_eq!(pending.len(), 19);
    let padded = take_padded_frame(&mut pending).unwrap();
    assert_eq!(padded.len(), TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT);
    assert_eq!(&padded[..19], &[7; 19]);
    assert!(padded[19..].iter().all(|byte| *byte == 0));
}

#[tokio::test]
async fn zero_final_handshake_is_ready_before_audio() {
    let (observed_audio, audio_observation) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            socket
                .send(Message::Text(
                    r#"{"code":0,"message":"success","voice_id":"fixture-voice","final":0}"#.into(),
                ))
                .await
                .unwrap();
            let mut saw_audio = false;
            while let Some(Ok(message)) = socket.next().await {
                saw_audio |= matches!(message, Message::Binary(_));
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
            observed_audio.send(saw_audio).unwrap();
        })
    })
    .await;

    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    assert!(client.inner.ready.load(Ordering::SeqCst));
    for expected in [
        LiveTranslateServerEvent::SessionCreated,
        LiveTranslateServerEvent::SessionUpdated,
    ] {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap(),
            Some(expected)
        );
    }
    client.disconnect().await;
    assert!(
        !tokio::time::timeout(Duration::from_secs(1), audio_observation)
            .await
            .unwrap()
            .unwrap()
    );
}

#[tokio::test]
async fn mock_websocket_covers_setup_audio_ping_transcript_and_finish() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            socket
                .send(Message::Text(
                    r#"{"code":0,"message":"success","voice_id":"v"}"#.into(),
                ))
                .await
                .unwrap();
            let mut saw_audio = false;
            while let Some(Ok(message)) = socket.next().await {
                match message {
                    Message::Binary(frame) => {
                        assert_eq!(frame.len(), TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT);
                        assert!(frame.iter().all(|byte| *byte == 7));
                        saw_audio = true;
                    }
                    Message::Ping(payload) => {
                        socket.send(Message::Pong(payload)).await.unwrap();
                    }
                    Message::Text(text) if text.contains(r#""type":"end""#) => {
                        assert!(saw_audio);
                        socket.send(Message::Text(
                            r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"你好","target_text":"こんにちは","sentence_end":false,"start_time":0,"end_time":800},"sentence_id":"old-sentence"}"#.into()
                        )).await.unwrap();
                        socket.send(Message::Text(
                            r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"你好。","target_text":"こんにちは。","sentence_end":true,"start_time":0,"end_time":800},"sentence_id":"old-sentence"}"#.into()
                        )).await.unwrap();
                        socket
                            .send(Message::Text(r#"{"code":0,"final":1}"#.into()))
                            .await
                            .unwrap();
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
        .send_audio(&vec![7; TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT])
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
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                source,
                language,
                translation,
                ..
            } => {
                assert_eq!(source, "你好。");
                assert_eq!(language.as_deref(), Some("zh"));
                assert_eq!(translation, "こんにちは。");
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
async fn wire_sentence_sequence_preserves_current_pairs_and_distinct_confirmations() {
    let (publish, publication) = tokio::sync::oneshot::channel();
    let (client, mut events) =
        test_client(move |mut socket| {
            Box::pin(async move {
                let frame =
                    |id: &str, start: u64, end: u64, text: &str, final_sentence: bool| {
                        Message::Text(serde_json::json!({
                    "code": 0, "sentence_id": id,
                    "result": {
                        "source": "en", "target": "zh",
                        "source_text": text, "target_text": format!("Translated {text}"),
                        "start_time": start, "end_time": end, "sentence_end": final_sentence
                    }
                }).to_string().into())
                    };
                socket
                    .send(Message::Text(r#"{"code":0}"#.into()))
                    .await
                    .unwrap();
                publication.await.unwrap();
                for message in [
                    frame("A", 0, 500, "Synthetic A draft", false),
                    frame("B", 1000, 1500, "Synthetic B draft", false),
                    frame("A", 0, 800, "Synthetic A final", true),
                    frame("A", 0, 800, "Synthetic A duplicate", true),
                    frame("A", 0, 900, "Synthetic A stale", false),
                ] {
                    socket.send(message).await.unwrap();
                }
                while let Some(Ok(message)) = socket.next().await {
                    if matches!(&message, Message::Text(text) if text.contains(r#""type":"end""#)) {
                        for message in [
                            frame("B", 1000, 1800, "Synthetic repeated final", true),
                            frame("B", 1000, 1900, "Synthetic B stale", false),
                            frame("C", 2000, 2800, "Synthetic repeated final", true),
                            frame("C", 2000, 2800, "Synthetic repeated final", true),
                            Message::Text(r#"{"code":0,"final":1}"#.into()),
                        ] {
                            socket.send(message).await.unwrap();
                        }
                        return;
                    }
                }
                panic!("missing end message");
            })
        })
        .await;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    client.connect().await.unwrap();
    publish.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        let mut saw_current_preview = false;
        loop {
            let event = events.recv().await.unwrap();
            saw_current_preview |= matches!(&event, LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair { utterance_id, .. } if utterance_id == "B");
            controller.handle(event);
            if saw_current_preview && !controller.state.subtitles.history.is_empty() { break; }
        }
    }).await.unwrap();
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert_eq!(
        controller.state.subtitles.history[0].source,
        "Synthetic A final"
    );
    assert_eq!(controller.state.subtitles.source.text, "Synthetic B draft");
    assert_eq!(
        controller.state.subtitles.translation.text,
        "Translated Synthetic B draft"
    );
    client.finish(Duration::from_secs(1)).await;
    while let Ok(event) = events.try_recv() {
        controller.handle(event);
    }
    assert_eq!(controller.state.subtitles.history.len(), 3);
    assert_eq!(
        controller.state.subtitles.history[1].source,
        "Synthetic repeated final"
    );
    assert_eq!(
        controller.state.subtitles.history[2].source,
        "Synthetic repeated final"
    );
    assert_eq!(
        controller.state.subtitles.source.utterance_id.as_deref(),
        Some("C")
    );
    assert!(controller.state.subtitles.source.is_final);
    assert!(controller.state.subtitles.realtime_preview.is_none());
}

#[tokio::test]
async fn unsolicited_final_is_a_sanitized_error_not_a_clean_finish() {
    let (end, ended) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            socket
                .send(Message::Text(
                    r#"{"code":0,"message":"success","voice_id":"v"}"#.into(),
                ))
                .await
                .unwrap();
            ended.await.unwrap();
            socket
                .send(Message::Text(r#"{"code":0,"final":1}"#.into()))
                .await
                .unwrap();
        })
    })
    .await;

    client
        .connect_with_timeout(Duration::from_millis(500))
        .await
        .unwrap();
    end.send(()).unwrap();

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
                saw_error = true;
            }
            LiveTranslateServerEvent::SessionFinished => {
                panic!("unsolicited final must not look like a clean finish")
            }
            _ => {}
        }
    }
    assert!(saw_error);
}

#[tokio::test]
async fn a_terminal_immediately_after_handshake_cannot_revive_readiness() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            for frame in [r#"{"code":0}"#, r#"{"code":6005}"#] {
                socket.send(Message::Text(frame.into())).await.unwrap();
            }
            while let Some(Ok(message)) = socket.next().await {
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
        })
    })
    .await;
    // Depending on task scheduling, readiness can precede the error or
    // connect can already report rejection. Neither may revive the socket.
    let result = client.connect().await;
    assert!(result.is_ok() || result == Err(TencentCloudClientError::SessionSetupRejected));
    tokio::time::timeout(Duration::from_secs(1), async {
        while !client.inner.receive_ended.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!client.inner.ready.load(Ordering::SeqCst));
    assert_eq!(
        client.send_audio(&vec![0; 6400]).await,
        Err(TencentCloudClientError::NotConnected)
    );
    let mut errors = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let LiveTranslateServerEvent::Error { code, .. } = event {
            errors.push(code);
        }
    }
    assert_eq!(errors, vec!["tencent_provider_6005"]);
    client.disconnect().await;
}

#[tokio::test]
async fn setup_timeout_is_finite() {
    let (client, _events) = test_client(|socket| {
        Box::pin(async move {
            let _keep_open = socket;
            tokio::time::sleep(Duration::from_secs(1)).await;
            drop(_keep_open);
        })
    })
    .await;
    assert_eq!(
        client
            .connect_with_timeout(Duration::from_millis(30))
            .await
            .unwrap_err(),
        TencentCloudClientError::SessionSetupTimedOut
    );
}
