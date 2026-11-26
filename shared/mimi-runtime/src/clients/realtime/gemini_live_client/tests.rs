#[tokio::test]
async fn handshake_rejections_preserve_authentication_service_and_transport_categories() {
    for (status, expected_auth_label) in
        crate::clients::transport::connection_diagnostics::handshake_failure_cases()
            .into_iter()
            .chain([400, 404, 429, 500, 503].map(|status| (Some(status), None)))
    {
        let (endpoint, server) =
            crate::clients::transport::connection_diagnostics::rejected_websocket_endpoint(status)
                .await;
        let (events, _receiver) = provider_event_channel();
        let mut client = GeminiLiveClient::with_endpoint(
            "test-key-not-real",
            TargetLanguage::Japanese,
            events,
            endpoint,
            false,
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
            assert_eq!(error, GeminiLiveClientError::AuthenticationFailed);
            assert_eq!(error.to_string(), expected_label);
        } else {
            let expected = match status {
                Some(503) => GeminiLiveClientError::ServiceUnavailable,
                Some(_) => GeminiLiveClientError::SessionSetupRejected,
                None => GeminiLiveClientError::TransportFailure,
            };
            assert_eq!(error, expected, "HTTP status={status:?}");
        }
        assert!(!error.to_string().contains("private-handshake-body"));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn clear_keeps_the_observed_quiet_boundary_without_committing_its_old_tail() {
    let (sender, mut receiver) = provider_event_channel();
    let client = GeminiLiveClient::with_endpoint(
        "test-key-not-real",
        TargetLanguage::Japanese,
        sender.clone(),
        url::Url::parse("ws://127.0.0.1:1/live").unwrap(),
        false,
    )
    .unwrap();
    {
        let mut committer = client.inner.committer.lock().await;
        committer.append_source("old source", None);
        committer.append_translation("old translation");
        committer.turn_complete_received = true;
    }
    let old_epoch = client.inner.turn_boundary_epoch.load(Ordering::SeqCst);
    sender
        .send(LiveTranslateServerEvent::SessionUpdated)
        .unwrap();
    assert_eq!(client.clear_content().await, 1);
    assert_eq!(client.content_revision(), 1);
    assert!(!transcript_boundary_is_current(
        &client.inner,
        0,
        TranscriptBoundary::Normal(old_epoch)
    ));
    {
        let mut committer = client.inner.committer.lock().await;
        assert!(!committer.has_pending());
        assert!(committer.append_source("late old source", None).is_empty());
        assert!(committer
            .append_translation("late old translation")
            .is_empty());
    }
    let epoch = client.inner.turn_boundary_epoch.load(Ordering::SeqCst);
    tokio::time::timeout(
        Duration::from_secs(3),
        publish_turn_after_transcript_quiet(
            &client.inner,
            &sender,
            0,
            TranscriptBoundary::Normal(epoch),
            false,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let mut committer = client.inner.committer.lock().await;
    assert!(!committer.discard_current_turn);
    committer.append_source("new source", None);
    committer.append_translation("new translation");
    assert!(
        matches!(committer.finish_turn().as_slice(), [LiveTranslateServerEvent::SubtitleFinalPair {source,translation,..}]
        if source == "new source" && translation == "new translation")
    );
    drop(committer);
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionUpdated)
    );
    assert!(receiver.try_recv().is_err());
}

use super::*;
use crate::clients::transport::provider_events::{provider_event_channel, ProviderEventReceiver};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;

fn binary_json_fixture() -> Value {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../../../translation-contracts.json")).unwrap();
    fixtures["binaryJsonTransports"]["geminiLive"].clone()
}

fn invalid_binary_transcript(fixture: &Value) -> Vec<u8> {
    fixture["invalidTranscriptHex"]
        .as_str()
        .unwrap()
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn binary_json_preserves_unicode_and_rejects_invalid_utf8_without_replacement() {
    let fixture = binary_json_fixture();
    let valid = fixture["transcript"].to_string();
    assert_eq!(
        decode_binary_event(valid.as_bytes()).unwrap(),
        GeminiLiveServerEvent::decode(&valid).unwrap()
    );
    let invalid = invalid_binary_transcript(&fixture);
    // The previous lossy conversion silently accepted this corrupt string.
    assert!(GeminiLiveServerEvent::decode(&String::from_utf8_lossy(&invalid)).is_ok());
    assert_eq!(
        decode_binary_event(&invalid),
        Err(GeminiLiveProtocolError::InvalidJSON)
    );
    assert_eq!(
        decode_binary_event(b"not-json"),
        Err(GeminiLiveProtocolError::InvalidJSON)
    );
}

#[tokio::test]
async fn binary_json_setup_and_unicode_transcript_reach_final_pair() {
    let fixture = binary_json_fixture();
    let setup = fixture["setup"].to_string().into_bytes();
    let transcript = fixture["transcript"].to_string().into_bytes();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            assert_setup(socket.next().await.unwrap().unwrap());
            socket.send(Message::Binary(setup.into())).await.unwrap();
            socket
                .send(Message::Binary(transcript.into()))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        })
    })
    .await;
    client.connect().await.unwrap();
    let (source, translation) = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match events.recv().await.unwrap() {
                LiveTranslateServerEvent::SubtitleFinalPair {
                    source,
                    translation,
                    ..
                } => break (source, translation),
                LiveTranslateServerEvent::Error { .. } => panic!("valid binary JSON failed"),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(source, fixture["expectedSource"].as_str().unwrap());
    assert_eq!(
        translation,
        fixture["expectedTranslation"].as_str().unwrap()
    );
    client.disconnect().await;
}

#[tokio::test]
async fn invalid_binary_utf8_cannot_complete_initial_or_replacement_setup() {
    for replacement in [false, true] {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                assert_setup(socket.next().await.unwrap().unwrap());
                socket
                    .send(Message::Binary(
                        b"{\"setupComplete\":{\"unused\":\"\xff\"}}".to_vec().into(),
                    ))
                    .await
                    .unwrap();
                while socket.next().await.is_some() {}
            })
        })
        .await;
        let error = if replacement {
            match tokio::time::timeout(Duration::from_secs(2), prepare_replacement(&client))
                .await
                .unwrap()
            {
                Ok(_) => panic!("invalid UTF-8 must not install a replacement"),
                Err(error) => error,
            }
        } else {
            client
                .connect_with_timeout(Duration::from_secs(2))
                .await
                .unwrap_err()
        };
        assert_eq!(error, GeminiLiveClientError::SessionSetupRejected);
        client.disconnect().await;
    }
}

#[tokio::test]
async fn invalid_binary_transcript_emits_error_without_corrupting_subtitles() {
    let fixture = binary_json_fixture();
    let setup = fixture["setup"].to_string().into_bytes();
    let invalid = invalid_binary_transcript(&fixture);
    let (send_invalid, ready) = tokio::sync::oneshot::channel();
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            assert_setup(socket.next().await.unwrap().unwrap());
            socket.send(Message::Binary(setup.into())).await.unwrap();
            ready.await.unwrap();
            socket.send(Message::Binary(invalid.into())).await.unwrap();
            while socket.next().await.is_some() {}
        })
    })
    .await;
    client.connect().await.unwrap();
    send_invalid.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await.unwrap() {
                LiveTranslateServerEvent::Error { code, .. } => {
                    assert_eq!(code, "gemini_live_provider_error");
                    break;
                }
                LiveTranslateServerEvent::SourceDraft { .. }
                | LiveTranslateServerEvent::TranslationDraft(_)
                | LiveTranslateServerEvent::SubtitleFinalPair { .. } => {
                    panic!("invalid UTF-8 must not reach subtitle state")
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    client.disconnect().await;
}

#[test]
fn shared_gemini_transcript_contracts() {
    fn observe(event: LiveTranslateServerEvent, pairs: &mut Vec<Value>, drafts: &mut Vec<Value>) {
        match event {
            LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                language,
                translation,
            } => pairs.push(json!({"source":source,"language":language,"translation":translation})),
            LiveTranslateServerEvent::SourceDraft { text, .. } => {
                drafts.push(json!({"kind":"source","text":text}))
            }
            LiveTranslateServerEvent::TranslationDraft(text) => {
                drafts.push(json!({"kind":"translation","text":text}))
            }
            _ => {}
        }
    }
    let fixtures: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    for case in fixtures["liveTranscriptSequences"].as_array().unwrap() {
        let mut committer = GeminiTranscriptPairCommitter::default();
        let mut pairs = Vec::new();
        let mut drafts = Vec::new();
        let mut failed = false;
        for (index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            let Ok(decoded) = GeminiLiveServerEvent::decode(frame.as_str().unwrap()) else {
                failed = true;
                break;
            };
            for event in decoded {
                let events = match event {
                    GeminiLiveServerEvent::SourceTranscript {
                        text,
                        language_code,
                    } => committer.append_source(&text, language_code),
                    GeminiLiveServerEvent::TranslationTranscript { text, .. } => {
                        committer.append_translation(&text)
                    }
                    GeminiLiveServerEvent::TurnComplete => committer.finish_turn(),
                    _ => Vec::new(),
                };
                for event in events {
                    observe(event, &mut pairs, &mut drafts);
                }
            }
            if case["settleAfterFrames"].as_array().is_some_and(|indices| {
                indices
                    .iter()
                    .any(|value| value.as_u64() == Some(index as u64))
            }) {
                let events = committer.stream.settle(u64::MAX);
                for event in committer.adapt(events) {
                    observe(event, &mut pairs, &mut drafts);
                }
            }
        }
        if case["expected"].is_null() {
            assert!(failed, "{}", case["id"]);
        } else {
            assert!(!failed);
            assert_eq!(json!(pairs), case["expected"], "{}", case["id"]);
            if case["expectedDrafts"].is_array() {
                assert_eq!(json!(drafts), case["expectedDrafts"], "{}", case["id"]);
            }
        }
    }
}

#[tokio::test]
async fn continuous_sentence_pairs_survive_language_only_updates_and_close_without_turn_complete() {
    let (client, mut events) = test_client(|mut socket| Box::pin(async move {
        assert_setup(socket.next().await.unwrap().unwrap());
        socket.send(Message::Text(r#"{"setupComplete":{}}"#.into())).await.unwrap();
        socket.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"Hello.","languageCode":"en"},"outputTranscription":{"text":"こんにちは。"}}}"#.into())).await.unwrap();
        socket.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"languageCode":"en"},"outputTranscription":{"languageCode":"ja"}}}"#.into())).await.unwrap();
        while socket.next().await.is_some() {}
    })).await;
    client.connect().await.unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. }) {
            break;
        }
    }
    client.finish(Duration::from_millis(50)).await;
    let mut finished = false;
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, LiveTranslateServerEvent::Error { .. }));
        finished |= matches!(event, LiveTranslateServerEvent::SessionFinished);
    }
    assert!(finished);
}

#[tokio::test]
async fn stop_checkpoints_a_late_paired_tail_without_turn_complete() {
    let (client, mut events) = test_client(|mut socket| Box::pin(async move {
        assert_setup(socket.next().await.unwrap().unwrap());
        socket.send(Message::Text(r#"{"setupComplete":{}}"#.into())).await.unwrap();
        socket.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"Synthetic source"},"outputTranscription":{"text":"合成"}}}"#.into())).await.unwrap();
        while let Some(Ok(message)) = socket.next().await {
            if message.to_text().ok().is_some_and(|text| text.contains("audioStreamEnd")) {
                tokio::time::sleep(Duration::from_millis(150)).await;
                socket.send(Message::Text(r#"{"serverContent":{"outputTranscription":{"text":"译文"}}}"#.into())).await.unwrap();
            }
        }
    })).await;
    client.connect().await.unwrap();
    loop {
        if matches!(
            events.recv().await.unwrap(),
            LiveTranslateServerEvent::TranslationDraft(_)
        ) {
            break;
        }
    }
    let started = tokio::time::Instant::now();
    client
        .finish(Duration::from_millis(
            mimi_core::translation_policy::GEMINI_FINISH_TIMEOUT_MS,
        ))
        .await;
    assert!(started.elapsed() >= Duration::from_millis(2_100));
    assert!(started.elapsed() < Duration::from_millis(4_000));
    let mut pairs = Vec::new();
    let mut finished = 0;
    while let Ok(event) = events.try_recv() {
        match event {
            LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                translation,
                ..
            } => {
                pairs.push((source, translation));
            }
            LiveTranslateServerEvent::SessionFinished => finished += 1,
            LiveTranslateServerEvent::Error { .. } => {
                panic!("paired quiet tail must finish cleanly")
            }
            _ => {}
        }
    }
    assert_eq!(pairs, vec![("Synthetic source".into(), "合成译文".into())]);
    assert_eq!(finished, 1);
}

#[tokio::test]
async fn websocket_drafts_update_the_shared_snapshot_before_unpunctuated_quiet_finals() {
    use crate::core::models::SubtitleEvent;
    use mimi_core::subtitle_reducer::{NoopArchive, SubtitleReducer};

    let (frames, mut pending) = tokio::sync::mpsc::channel::<Value>(8);
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            assert_setup(socket.next().await.unwrap().unwrap());
            socket
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            while let Some(frame) = pending.recv().await {
                socket
                    .send(Message::Text(frame.to_string().into()))
                    .await
                    .unwrap();
            }
            while socket.next().await.is_some() {}
        })
    })
    .await;
    client.connect().await.unwrap();
    let mut reducer = SubtitleReducer::<NoopArchive>::new(6);
    let mut draft_count = 0;
    let mut final_count = 0;
    for (block, chunks) in [["合", "成", "合成第一段"], ["第", "二", "第二段译文"]]
        .into_iter()
        .enumerate()
    {
        // The second block's translation deliberately arrives before ASR.
        if block == 0 {
            frames
                .send(json!({"serverContent":{"inputTranscription":{"text":"Synthetic first source","languageCode":"en"}}}))
                .await
                .unwrap();
        }
        let mut expected = String::new();
        for chunk in chunks {
            if chunk.starts_with(&expected) {
                expected = chunk.into();
            } else {
                expected.push_str(chunk);
            }
            frames
                .send(json!({"serverContent":{"outputTranscription":{"text":chunk}}}))
                .await
                .unwrap();
            loop {
                let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
                    .await
                    .expect("a draft must not wait for the two-second final boundary")
                    .unwrap();
                match event {
                    LiveTranslateServerEvent::SourceDraft { text, .. } => {
                        reducer.apply(SubtitleEvent::SourceDraft(text));
                    }
                    LiveTranslateServerEvent::TranslationDraft(text) => {
                        assert_eq!(text, expected);
                        reducer.apply(SubtitleEvent::TranslationDraft(text));
                        draft_count += 1;
                        break;
                    }
                    LiveTranslateServerEvent::SubtitleFinalPair { .. }
                    | LiveTranslateServerEvent::Error { .. } => {
                        panic!("no final or error is allowed before the live draft")
                    }
                    _ => {}
                }
            }
            assert!(!reducer.snapshot.translation.is_final);
            assert_eq!(reducer.snapshot.translation.text, expected);
            assert_eq!(reducer.snapshot.history.len(), block);
            if block == 1 {
                assert!(reducer.snapshot.display_pair_final);
                assert_ne!(
                    reducer.snapshot.display_pair.as_ref().unwrap().translation,
                    expected
                );
                assert!(reducer.snapshot.source.is_final);
            }
        }
        if block == 1 {
            frames
                .send(json!({"serverContent":{"inputTranscription":{"text":"Synthetic second source","languageCode":"en"}}}))
                .await
                .unwrap();
        }
        let quiet_started = std::time::Instant::now();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
                .await
                .expect("unpunctuated text must close without turnComplete")
                .unwrap();
            match event {
                LiveTranslateServerEvent::SourceDraft { text, .. } => {
                    reducer.apply(SubtitleEvent::SourceDraft(text));
                }
                LiveTranslateServerEvent::SubtitleFinalPair {
                    source,
                    translation,
                    ..
                } => {
                    assert!(quiet_started.elapsed() >= Duration::from_millis(1_900));
                    assert_eq!(translation, expected);
                    reducer.apply(SubtitleEvent::FinalPair {
                        source,
                        translation,
                    });
                    final_count += 1;
                    break;
                }
                LiveTranslateServerEvent::Error { .. } => panic!("unexpected provider error"),
                _ => {}
            }
        }
        assert_eq!(reducer.snapshot.history.len(), block + 1);
        assert!(reducer.snapshot.translation.is_final);
    }
    assert_eq!((draft_count, final_count), (6, 2));
    client.disconnect().await;
}

async fn test_client(
    server: impl FnOnce(
            WebSocketStream<TcpStream>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + 'static,
) -> (GeminiLiveClient, ProviderEventReceiver) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        server(socket).await;
    });
    let (events, receiver) = provider_event_channel();
    let endpoint = url::Url::parse(&format!("ws://{address}/live")).unwrap();
    let client = GeminiLiveClient::with_endpoint(
        "gemini-test-key-not-real",
        TargetLanguage::Japanese,
        events,
        endpoint,
        false,
    )
    .unwrap();
    (client, receiver)
}

fn assert_setup(message: Message) {
    let setup: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
    assert_eq!(
        setup["setup"]["model"],
        "models/gemini-3.5-live-translate-preview"
    );
    assert_eq!(
        setup["setup"]["generationConfig"]["translationConfig"]["targetLanguageCode"],
        "ja"
    );
}

#[tokio::test]
async fn connect_waits_for_setup_complete() {
    let (client, _events) = test_client(|mut socket| {
        Box::pin(async move {
            assert_setup(socket.next().await.unwrap().unwrap());
            // This is deliberately longer than the old 150 ms heuristic:
            // a legal delayed transcript must still be part of the tail.
            tokio::time::sleep(Duration::from_millis(300)).await;
            socket
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
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
    client.disconnect().await;
}

#[tokio::test]
async fn setup_rejection_is_content_free() {
    let secret = "gemini-test-secret-never-log";
    let (events, _receiver) = provider_event_channel();
    let result = GeminiLiveClient::with_endpoint(
        secret,
        TargetLanguage::Original,
        events,
        url::Url::parse("ws://127.0.0.1:9/live").unwrap(),
        false,
    );
    let error = match result {
        Ok(_) => panic!("an untranslated target must be rejected"),
        Err(error) => error,
    };
    assert_eq!(error, GeminiLiveClientError::InvalidTargetLanguage);
    assert!(!format!("{error:?}").contains(secret));

    let (client, _events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(
                    r#"{"error":{"code":403,"status":"PERMISSION_DENIED","message":"private transcript and key"}}"#.into(),
                ))
                .await
                .unwrap();
        })
    })
    .await;
    assert_eq!(
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap_err(),
        GeminiLiveClientError::AuthenticationFailed
    );
}

#[tokio::test]
async fn gemini_recovery_setup_authentication_preserves_its_type() {
    for (status, code, expected) in [
        (
            "PERMISSION_DENIED",
            403,
            GeminiLiveClientError::AuthenticationFailed,
        ),
        (
            "UNAUTHENTICATED",
            401,
            GeminiLiveClientError::AuthenticationFailed,
        ),
        (
            "UNAVAILABLE",
            503,
            GeminiLiveClientError::ServiceUnavailable,
        ),
        (
            "RESOURCE_EXHAUSTED",
            429,
            GeminiLiveClientError::SessionSetupRejected,
        ),
        (
            "INVALID_ARGUMENT",
            400,
            GeminiLiveClientError::SessionSetupRejected,
        ),
        (
            "unknown-private-status",
            503,
            GeminiLiveClientError::SessionSetupRejected,
        ),
    ] {
        let (client, _events) = test_client(move |mut socket| {
            Box::pin(async move {
                assert_setup(socket.next().await.unwrap().unwrap());
                socket
                    .send(Message::Text(
                        json!({"error":{"status":status,"code":code,"message":"private payload"}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            })
        })
        .await;
        assert_eq!(
            client.connect().await.unwrap_err(),
            expected,
            "status={status}"
        );
    }
}

async fn next_recovery_error(events: &mut ProviderEventReceiver) -> String {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await.unwrap() {
                LiveTranslateServerEvent::Error { code, message } => {
                    assert!(!message.contains("private payload"));
                    return code;
                }
                LiveTranslateServerEvent::SessionFinished => {
                    panic!("a rejected active session must not finish normally")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("provider failure must be surfaced")
}

#[tokio::test]
async fn gemini_recovery_active_error_uses_fixed_categories() {
    for (status, code, expected) in [
        ("UNAVAILABLE", 503, "gemini_live_service_unavailable"),
        ("PERMISSION_DENIED", 403, "credential_authentication_failed"),
        ("RESOURCE_EXHAUSTED", 429, "gemini_live_provider_error"),
        ("INVALID_ARGUMENT", 400, "gemini_live_provider_error"),
        ("unknown-private-status", 503, "gemini_live_provider_error"),
    ] {
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                assert_setup(socket.next().await.unwrap().unwrap());
                socket
                    .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        json!({"error":{"status":status,"code":code,"message":"private payload"}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                while socket.next().await.is_some() {}
            })
        })
        .await;
        client.connect().await.unwrap();
        assert_eq!(
            next_recovery_error(&mut events).await,
            expected,
            "status={status}"
        );
        client.disconnect().await;
    }
}

#[tokio::test]
async fn gemini_recovery_go_away_during_stop_preserves_the_closing_tail() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            assert_setup(socket.next().await.unwrap().unwrap());
            socket.send(Message::Text(r#"{"setupComplete":{}}"#.into())).await.unwrap();
            let end: Value = serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(end["realtimeInput"]["audioStreamEnd"], true);
            socket.send(Message::Text(r#"{"goAway":{"timeLeft":"10s"}}"#.into())).await.unwrap();
            socket.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"Synthetic closing source"},"outputTranscription":{"text":"Synthetic closing translation"},"turnComplete":true}}"#.into())).await.unwrap();
            while socket.next().await.is_some() {}
        })
    }).await;
    client.connect().await.unwrap();
    client.finish(Duration::from_secs(2)).await;
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. } if source == "Synthetic closing source" && translation == "Synthetic closing translation")), "closing tail must survive the GoAway notice: {received:?}");
    assert_eq!(
        received
            .iter()
            .filter(|event| matches!(event, LiveTranslateServerEvent::SessionFinished))
            .count(),
        1
    );
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
}

#[tokio::test]
async fn gemini_recovery_stop_owns_old_tail_when_unused_replacement_is_rejected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (allow_goaway, goaway_allowed) = tokio::sync::oneshot::channel();
    let (replacement_waiting, replacement_ready) = tokio::sync::oneshot::channel();
    let (stop_received, stop_owned) = tokio::sync::oneshot::channel();
    let (reject_replacement, rejection_released) = tokio::sync::oneshot::channel();
    let (release_tail, tail_released) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut old = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_setup(old.next().await.unwrap().unwrap());
        old.send(Message::Text(r#"{"setupComplete":{}}"#.into()))
            .await
            .unwrap();
        old.send(Message::Text(
            r#"{"serverContent":{"inputTranscription":{"text":"Synthetic owned source"}}}"#.into(),
        ))
        .await
        .unwrap();
        goaway_allowed.await.unwrap();
        old.send(Message::Text(r#"{"goAway":{"timeLeft":"10s"}}"#.into()))
            .await
            .unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let mut replacement = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_setup(replacement.next().await.unwrap().unwrap());
        replacement_waiting.send(()).unwrap();
        let end: Value =
            serde_json::from_str(old.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(end["realtimeInput"]["audioStreamEnd"], true);
        stop_received.send(()).unwrap();
        rejection_released.await.unwrap();
        replacement.send(Message::Text(r#"{"error":{"code":403,"status":"PERMISSION_DENIED","message":"private payload"}}"#.into())).await.unwrap();
        tail_released.await.unwrap();
        // This arrives only after the rejected replacement has settled and
        // Stop already owns the old socket's audioStreamEnd boundary.
        let tail_sent = old.send(Message::Text(r#"{"serverContent":{"outputTranscription":{"text":"Synthetic owned translation"},"turnComplete":true}}"#.into())).await.is_ok();
        while old.next().await.is_some() {}
        tail_sent
    });
    let (sender, mut events) = provider_event_channel();
    let client = GeminiLiveClient::with_endpoint(
        "test-key",
        TargetLanguage::Japanese,
        sender,
        url::Url::parse(&format!("ws://{address}/live")).unwrap(),
        false,
    )
    .unwrap();
    client.connect().await.unwrap();
    mark_transport_stable_for_rotation(&client).await;
    allow_goaway.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), replacement_ready)
        .await
        .unwrap()
        .unwrap();
    let closing_client = client.clone();
    let closing = tokio::spawn(async move {
        closing_client.finish(Duration::from_secs(3)).await;
    });
    tokio::time::timeout(Duration::from_secs(2), stop_owned)
        .await
        .unwrap()
        .unwrap();
    reject_replacement.send(()).unwrap();
    wait_for_rotation(&client, "geminiReplacementSetupFailed").await;
    release_tail.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(4), closing)
        .await
        .unwrap()
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap());
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. } if source == "Synthetic owned source" && translation == "Synthetic owned translation")), "Stop must retain its old socket's tail: {received:?}");
    assert_eq!(
        received
            .iter()
            .filter(|event| matches!(event, LiveTranslateServerEvent::SessionFinished))
            .count(),
        1
    );
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
}

#[tokio::test]
async fn gemini_recovery_transport_stability_rejects_old_health_and_resets_on_handoff() {
    let (client, _events, _) = rotating_test_client(Duration::ZERO).await;
    client.connect().await.unwrap();
    let old = client.inner.transport_health.lock().await.unwrap();
    mark_transport_stable_for_rotation(&client).await;
    client
        .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    wait_for_rotation(&client, "geminiConnectionRotated").await;
    let replacement = client.inner.transport_health.lock().await.unwrap();
    assert_ne!(replacement.connected_at, old.connected_at);
    assert!(!replacement.stable);
    let after_interval =
        replacement.connected_at + crate::core::lifecycle::health::HEALTH_CHECK_INTERVAL;
    assert!(!client.record_transport_health(old, after_interval).await);
    assert!(!client.inner.transport_health.lock().await.unwrap().stable);
    let mut retired_generation = replacement;
    retired_generation.generation = replacement.generation.wrapping_sub(1);
    assert!(
        !client
            .record_transport_health(retired_generation, after_interval)
            .await
    );
    client.inner.is_closing.store(true, Ordering::SeqCst);
    assert!(
        !client
            .record_transport_health(replacement, after_interval)
            .await
    );
    client.inner.is_closing.store(false, Ordering::SeqCst);
    assert!(
        client
            .record_transport_health(replacement, after_interval)
            .await
    );
    client.disconnect().await;
    assert!(
        !client
            .record_transport_health(replacement, after_interval)
            .await
    );
}

#[tokio::test]
async fn gemini_recovery_unstable_go_away_delegates_without_opening_replacement() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_setup(socket.next().await.unwrap().unwrap());
        socket
            .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
            .await
            .unwrap();
        socket
            .send(Message::Text(r#"{"goAway":{"timeLeft":"0s"}}"#.into()))
            .await
            .unwrap();
        let opened_replacement =
            tokio::time::timeout(Duration::from_millis(300), listener.accept())
                .await
                .is_ok();
        while socket.next().await.is_some() {}
        opened_replacement
    });
    let (sender, mut events) = provider_event_channel();
    let client = GeminiLiveClient::with_endpoint(
        "test-key",
        TargetLanguage::Japanese,
        sender,
        url::Url::parse(&format!("ws://{address}/live")).unwrap(),
        false,
    )
    .unwrap();
    client.connect().await.unwrap();
    assert_eq!(next_recovery_error(&mut events).await, "transport_error");
    client.disconnect().await;
    assert!(
        !tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap(),
        "unstable GoAway must leave retries to the shared bounded recovery driver"
    );
}

#[tokio::test]
async fn gemini_recovery_go_away_replacement_preserves_setup_rejection() {
    for (status, code, expected) in [
        ("PERMISSION_DENIED", 403, "credential_authentication_failed"),
        ("UNAVAILABLE", 503, "gemini_live_service_unavailable"),
        ("RESOURCE_EXHAUSTED", 429, "gemini_live_provider_error"),
        ("unknown-private-status", 503, "gemini_live_provider_error"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (allow_goaway, goaway_allowed) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut old = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert_setup(old.next().await.unwrap().unwrap());
            old.send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            goaway_allowed.await.unwrap();
            old.send(Message::Text(r#"{"goAway":{"timeLeft":"10s"}}"#.into()))
                .await
                .unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let mut replacement = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert_setup(replacement.next().await.unwrap().unwrap());
            replacement
                .send(Message::Text(
                    json!({"error":{"status":status,"code":code,"message":"private payload"}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            while old.next().await.is_some() {}
        });
        let (sender, mut events) = provider_event_channel();
        let client = GeminiLiveClient::with_endpoint(
            "test-key",
            TargetLanguage::Japanese,
            sender,
            url::Url::parse(&format!("ws://{address}/live")).unwrap(),
            false,
        )
        .unwrap();
        client.connect().await.unwrap();
        mark_transport_stable_for_rotation(&client).await;
        allow_goaway.send(()).unwrap();
        assert_eq!(
            next_recovery_error(&mut events).await,
            expected,
            "status={status}"
        );
        client.disconnect().await;
        server.await.unwrap();
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
            .connect_with_timeout(Duration::from_millis(50))
            .await
            .unwrap_err(),
        GeminiLiveClientError::SessionSetupTimedOut
    );
}

#[tokio::test]
async fn established_go_away_requests_redacted_transport_recovery() {
    let private_time_left = "private-provider-time-left";
    let private_message = "private-provider-message";
    let (client, mut events) = test_client(move |mut socket| {
        Box::pin(async move {
            assert_setup(socket.next().await.unwrap().unwrap());
            socket
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    format!(
                        r#"{{"goAway":{{"timeLeft":"{private_time_left}","message":"{private_message}"}}}}"#
                    )
                    .into(),
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

    let mut received = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
            .await
            .expect("GoAway must produce a recovery event")
            .expect("the provider event channel must remain open");
        let is_error = matches!(event, LiveTranslateServerEvent::Error { .. });
        received.push(event);
        if is_error {
            break;
        }
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::Error { code, message }
            if code == "transport_error" && message == GENERIC_TRANSPORT_ERROR
    )));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    let debug = format!("{received:?}");
    assert!(!debug.contains(private_time_left));
    assert!(!debug.contains(private_message));
    assert_eq!(
        client.probe_transport_stop(),
        Some(ProbeTransportStop::GoAway)
    );
    client.disconnect().await;
}

async fn rotating_test_client(
    setup_delay: Duration,
) -> (GeminiLiveClient, ProviderEventReceiver, Arc<Mutex<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let samples = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&samples);
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut old = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_setup(old.next().await.unwrap().unwrap());
        old.send(Message::Text(r#"{"setupComplete":{}}"#.into()))
            .await
            .unwrap();
        let old_samples = Arc::clone(&recorded);
        let old_task = tokio::spawn(async move {
            let mut first = true;
            while let Some(Ok(message)) = old.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                let value: Value = serde_json::from_str(&text).unwrap();
                if let Some(audio) = value["realtimeInput"]["audio"]["data"].as_str() {
                    let pcm = base64::engine::general_purpose::STANDARD
                        .decode(audio)
                        .unwrap();
                    old_samples
                        .lock()
                        .await
                        .extend(pcm.into_iter().filter(|byte| *byte != 0));
                    if first {
                        first = false;
                        old.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"Old source"},"outputTranscription":{"text":"旧译文"}}}"#.into())).await.unwrap();
                        // Duplicate notifications must not start a second replacement.
                        for _ in 0..2 {
                            old.send(Message::Text(r#"{"goAway":{"timeLeft":"private"}}"#.into()))
                                .await
                                .unwrap();
                        }
                    }
                } else if value["realtimeInput"]["audioStreamEnd"] == true {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    if old.send(Message::Text(r#"{"serverContent":{"outputTranscription":{"text":"完整。"},"turnComplete":true}}"#.into())).await.is_err() { return; }
                }
            }
        });
        let (stream, _) = listener.accept().await.unwrap();
        let mut new = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_setup(new.next().await.unwrap().unwrap());
        tokio::time::sleep(setup_delay).await;
        if new
            .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
            .await
            .is_err()
        {
            old_task.abort();
            return;
        }
        let mut first = true;
        while let Some(Ok(message)) = new.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            if let Some(audio) = value["realtimeInput"]["audio"]["data"].as_str() {
                let pcm = base64::engine::general_purpose::STANDARD
                    .decode(audio)
                    .unwrap();
                recorded
                    .lock()
                    .await
                    .extend(pcm.into_iter().filter(|byte| *byte != 0));
                if first {
                    first = false;
                    new.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"New source"},"outputTranscription":{"text":"新会话译文。"}}}"#.into())).await.unwrap();
                }
            } else if value["realtimeInput"]["audioStreamEnd"] == true {
                let _ = new
                    .send(Message::Text(
                        r#"{"serverContent":{"turnComplete":true}}"#.into(),
                    ))
                    .await;
            }
        }
        old_task.abort();
    });
    let (events, receiver) = provider_event_channel();
    let client = GeminiLiveClient::with_endpoint(
        "gemini-test-key-not-real",
        TargetLanguage::Japanese,
        events,
        url::Url::parse(&format!("ws://{address}/live")).unwrap(),
        false,
    )
    .unwrap();
    (client, receiver, samples)
}

#[tokio::test]
async fn planned_rotation_keeps_native_audio_queue_and_late_old_caption() {
    let (client, mut events, samples) = rotating_test_client(Duration::from_millis(300)).await;
    client.connect().await.unwrap();
    mark_transport_stable_for_rotation(&client).await;
    let reader = tokio::spawn(async move {
        let mut received = Vec::new();
        while let Some(event) = events.recv().await {
            let finished = matches!(event, LiveTranslateServerEvent::SessionFinished);
            received.push(event);
            if finished {
                break;
            }
        }
        received
    });
    let failed = Arc::new(AtomicBool::new(false));
    let failed_callback = Arc::clone(&failed);
    let send_client = client.clone();
    let pipeline = crate::audio::send_pipeline::AudioSendPipeline::spawn(
        move |data| {
            let client = send_client.clone();
            async move { client.send_audio(&data).await }
        },
        move |_| {
            failed_callback.store(true, Ordering::SeqCst);
        },
    );
    let ingress = pipeline.ingress().unwrap();
    // Native callbacks/resampler output can be much smaller than a wire
    // frame. Twenty queued buffers therefore do not imply two seconds.
    for index in 0..150 {
        ingress
            .try_send(vec![
                index / 5 + 1;
                GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT / 5
            ])
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(pipeline.finish(Duration::from_secs(2)).await);
    assert!(!failed.load(Ordering::SeqCst));
    client.finish(Duration::from_secs(2)).await;
    let received = tokio::time::timeout(Duration::from_secs(1), reader)
        .await
        .unwrap()
        .unwrap();
    // A session boundary pads one partial frame, so compare every original
    // nonzero PCM byte, ignoring only that known zero padding.
    let expected = (1..=30)
        .flat_map(|index| std::iter::repeat_n(index, GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT))
        .collect::<Vec<u8>>();
    assert_eq!(*samples.lock().await, expected);
    assert_eq!(
        received
            .iter()
            .filter(|event| matches!(event, LiveTranslateServerEvent::SessionCreated))
            .count(),
        1
    );
    assert_eq!(
        client
            .probe_connection_events()
            .iter()
            .filter(|event| event.kind == "geminiConnectionRotated")
            .count(),
        1
    );
    assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. } if source == "Old source" && translation == "旧译文完整。")));
    assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::TranslationDraft(text) if text == "新会话译文。")));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
    assert_eq!(client.probe_transport_stop(), None);
}

async fn mark_transport_stable_for_rotation(client: &GeminiLiveClient) {
    let observed = client.inner.transport_health.lock().await.unwrap();
    let now = observed.connected_at + crate::core::lifecycle::health::HEALTH_CHECK_INTERVAL;
    assert!(
        !client
            .record_transport_health(observed, now - Duration::from_nanos(1))
            .await
    );
    assert!(client.record_transport_health(observed, now).await);
}

async fn wait_for_rotation(client: &GeminiLiveClient, kind: &'static str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !client
            .probe_connection_events()
            .iter()
            .any(|event| event.kind == kind)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

async fn wait_for_handoff(client: &GeminiLiveClient) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !client.inner.rotating.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stop_during_handoff_drains_staged_pcm_into_replacement() {
    let (client, mut events, samples) = rotating_test_client(Duration::ZERO).await;
    client.connect().await.unwrap();
    mark_transport_stable_for_rotation(&client).await;
    client
        .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    wait_for_handoff(&client).await;
    client
        .send_audio(&vec![2; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    client.finish(Duration::from_secs(3)).await;
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    let expected = [1, 2]
        .into_iter()
        .flat_map(|index| std::iter::repeat_n(index, GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT))
        .collect::<Vec<u8>>();
    assert_eq!(*samples.lock().await, expected);
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, .. } if source == "New source")));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
    assert!(!client.inner.rotating.load(Ordering::SeqCst));
}

#[tokio::test]
async fn stop_waits_for_old_tail_after_replacement_setup() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (replacement_setup, setup_received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut old = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_setup(old.next().await.unwrap().unwrap());
        old.send(Message::Text(r#"{"setupComplete":{}}"#.into()))
            .await
            .unwrap();
        assert!(matches!(old.next().await, Some(Ok(Message::Text(_)))));
        old.send(Message::Text(r#"{"goAway":{}}"#.into()))
            .await
            .unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut replacement = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_setup(replacement.next().await.unwrap().unwrap());
        replacement_setup.send(()).unwrap();
        let Message::Text(end) = old.next().await.unwrap().unwrap() else {
            panic!("Stop must end the old audio stream");
        };
        assert_eq!(
            serde_json::from_str::<Value>(&end).unwrap()["realtimeInput"]["audioStreamEnd"],
            true
        );
        // Stop is now waiting for the old tail. Complete replacement setup
        // first, and release that tail only after Stop rejects the replacement.
        replacement
            .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
            .await
            .unwrap();
        let replacement_closed = matches!(replacement.next().await, Some(Ok(Message::Close(_))));
        let tail_sent = old.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"Old closing source"},"outputTranscription":{"text":"Old closing translation"},"turnComplete":true}}"#.into())).await.is_ok();
        while let Some(Ok(message)) = old.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
        (replacement_closed, tail_sent)
    });
    let (sender, mut events) = provider_event_channel();
    let client = GeminiLiveClient::with_endpoint(
        "gemini-test-key-not-real",
        TargetLanguage::Japanese,
        sender,
        url::Url::parse(&format!("ws://{address}/live")).unwrap(),
        false,
    )
    .unwrap();
    client.connect().await.unwrap();
    mark_transport_stable_for_rotation(&client).await;
    client
        .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), setup_received)
        .await
        .unwrap()
        .unwrap();
    client.finish(Duration::from_secs(2)).await;
    let (replacement_closed, tail_sent) = tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(
        replacement_closed,
        "replacement setup must not block behind Stop's final-turn wait"
    );
    assert!(tail_sent);
    assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. } if source == "Old closing source" && translation == "Old closing translation")));
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
    assert!(!client
        .probe_connection_events()
        .iter()
        .any(|event| event.kind == "geminiConnectionRotated"));
}

#[tokio::test]
async fn stop_waits_for_replacement_tail_after_old_turn_complete() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (old_ended, old_end_received) = tokio::sync::oneshot::channel();
    let (release_old_tail, old_tail_released) = tokio::sync::oneshot::channel();
    let (replacement_ended, replacement_end_received) = tokio::sync::oneshot::channel();
    let (release_replacement_tail, replacement_tail_released) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut old = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_setup(old.next().await.unwrap().unwrap());
        old.send(Message::Text(r#"{"setupComplete":{}}"#.into()))
            .await
            .unwrap();
        assert!(matches!(old.next().await, Some(Ok(Message::Text(_)))));
        old.send(Message::Text(r#"{"goAway":{}}"#.into()))
            .await
            .unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let mut replacement = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_setup(replacement.next().await.unwrap().unwrap());
        replacement
            .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
            .await
            .unwrap();
        let Message::Text(end) = old.next().await.unwrap().unwrap() else {
            panic!("Rotation must end the old audio stream");
        };
        assert_eq!(
            serde_json::from_str::<Value>(&end).unwrap()["realtimeInput"]["audioStreamEnd"],
            true
        );
        old_ended.send(()).unwrap();
        old_tail_released.await.unwrap();
        old.send(Message::Text(
            r#"{"serverContent":{"turnComplete":true}}"#.into(),
        ))
        .await
        .unwrap();
        let Message::Text(audio) = replacement.next().await.unwrap().unwrap() else {
            panic!("Replacement must receive staged audio before Stop");
        };
        let audio: Value = serde_json::from_str(&audio).unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(audio["realtimeInput"]["audio"]["data"].as_str().unwrap())
                .unwrap(),
            vec![2; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT]
        );
        let Message::Text(end) = replacement.next().await.unwrap().unwrap() else {
            panic!("Stop must end the replacement audio stream");
        };
        assert_eq!(
            serde_json::from_str::<Value>(&end).unwrap()["realtimeInput"]["audioStreamEnd"],
            true
        );
        replacement_ended.send(()).unwrap();
        replacement_tail_released.await.unwrap();
        let tail_sent = replacement.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"Replacement closing source"},"outputTranscription":{"text":"Replacement closing translation"},"turnComplete":true}}"#.into())).await.is_ok();
        while let Some(Ok(message)) = replacement.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
        tail_sent
    });
    let (sender, mut events) = provider_event_channel();
    let client = GeminiLiveClient::with_endpoint(
        "gemini-test-key-not-real",
        TargetLanguage::Japanese,
        sender,
        url::Url::parse(&format!("ws://{address}/live")).unwrap(),
        false,
    )
    .unwrap();
    client.connect().await.unwrap();
    mark_transport_stable_for_rotation(&client).await;
    client
        .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), old_end_received)
        .await
        .unwrap()
        .unwrap();
    wait_for_handoff(&client).await;
    client
        .send_audio(&vec![2; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    let closing_client = client.clone();
    let mut closing = tokio::spawn(async move {
        closing_client.finish(Duration::from_secs(4)).await;
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !client.inner.is_closing.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Only the old socket acknowledges after Stop starts, so its boundary
    // must not satisfy the replacement's final-turn wait.
    release_old_tail.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), replacement_end_received)
        .await
        .unwrap()
        .unwrap();
    let finished_early =
        tokio::time::timeout(TAIL_QUIET_PERIOD + Duration::from_millis(200), &mut closing).await;
    release_replacement_tail.send(()).unwrap();
    let finished_before_replacement_tail = match finished_early {
        Ok(result) => {
            result.unwrap();
            true
        }
        Err(_) => {
            tokio::time::timeout(Duration::from_secs(2), closing)
                .await
                .unwrap()
                .unwrap();
            false
        }
    };
    let tail_sent = tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(
        !finished_before_replacement_tail,
        "old turnComplete must not finish the replacement before its own tail"
    );
    assert!(tail_sent);
    assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. } if source == "Replacement closing source" && translation == "Replacement closing translation")));
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
}

#[tokio::test]
async fn rotation_staging_rejects_pcm_beyond_two_seconds() {
    let (client, _events, _) = rotating_test_client(Duration::ZERO).await;
    client.connect().await.unwrap();
    mark_transport_stable_for_rotation(&client).await;
    client
        .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    wait_for_handoff(&client).await;
    client
        .send_audio(&vec![2; ROTATION_PENDING_PCM_BYTES])
        .await
        .unwrap();
    assert_eq!(
        client.send_audio(&[3, 3]).await,
        Err(GeminiLiveClientError::TransportFailure)
    );
    assert_eq!(
        client.inner.pending_audio.lock().await.len(),
        ROTATION_PENDING_PCM_BYTES
    );
    client.disconnect().await;
    assert!(client.inner.pending_audio.lock().await.is_empty());
    assert!(!client.inner.rotating.load(Ordering::SeqCst));
}

#[tokio::test]
async fn stop_during_rotation_setup_does_not_install_replacement() {
    let (client, mut events, _) = rotating_test_client(Duration::from_secs(2)).await;
    client.connect().await.unwrap();
    mark_transport_stable_for_rotation(&client).await;
    client
        .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    wait_for_rotation(&client, "geminiGoAwayReceived").await;
    client.finish(Duration::from_secs(2)).await;
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!client
        .probe_connection_events()
        .iter()
        .any(|event| event.kind == "geminiConnectionRotated"));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
    assert!(!client.inner.ready.load(Ordering::SeqCst));
}

#[tokio::test]
async fn clear_during_rotation_does_not_confirm_retired_text() {
    let (client, mut events, _) = rotating_test_client(Duration::from_millis(300)).await;
    client.connect().await.unwrap();
    mark_transport_stable_for_rotation(&client).await;
    client
        .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    wait_for_rotation(&client, "geminiGoAwayReceived").await;
    client.clear_content().await;
    let mut received = Vec::new();
    wait_for_rotation(&client, "geminiConnectionRotated").await;
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    client
        .send_audio(&vec![2; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
        .await
        .unwrap();
    client.finish(Duration::from_secs(2)).await;
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(!received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, .. } if source == "Old source")));
    assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. } if source == "New source" && translation == "新会话译文。")));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
}

#[test]
fn frame_buffer_keeps_only_a_partial_tail() {
    let frame_size = GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let mut pending = vec![0x22; frame_size * 2 + 17];
    let messages = take_complete_audio_messages(&mut pending).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(pending.len(), 17);
    for message in messages {
        let value: Value = serde_json::from_str(&message).unwrap();
        let frame = base64::engine::general_purpose::STANDARD
            .decode(value["realtimeInput"]["audio"]["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(frame.len(), frame_size);
        assert!(frame.iter().all(|byte| *byte == 0x22));
    }
}

#[test]
fn final_partial_frame_is_zero_padded() {
    let mut pending = vec![1, 2, 3];
    let message = take_padded_audio_message(&mut pending).unwrap().unwrap();
    let value: Value = serde_json::from_str(&message).unwrap();
    let frame = base64::engine::general_purpose::STANDARD
        .decode(value["realtimeInput"]["audio"]["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(frame.len(), GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT);
    assert_eq!(&frame[..3], &[1, 2, 3]);
    assert!(frame[3..].iter().all(|byte| *byte == 0));
    assert!(pending.is_empty());
}

#[test]
fn transcript_safety_limit_discards_the_rest_of_the_turn() {
    let mut committer = GeminiTranscriptPairCommitter::default();
    let events =
        committer.append_source(&"s".repeat(MAXIMUM_TRANSCRIPT_BYTES + 1), Some("en".into()));
    assert!(matches!(
        events.as_slice(),
        [LiveTranslateServerEvent::Error { code, message }]
            if code == "gemini_transcript_safety_limit"
                && message
                    == "Gemini Live Translation transcript buffering exceeded its safety limit."
    ));

    assert!(committer.append_source("stale source", None).is_empty());
    assert!(committer.append_translation("stale translation").is_empty());
    assert!(committer.finish_turn().is_empty());

    let _ = committer.append_source("Next sentence.", Some("en".into()));
    let _ = committer.append_translation("次の文。");
    let events = committer.finish_turn();
    assert!(matches!(
        events.last(),
        Some(LiveTranslateServerEvent::SubtitleFinalPair {
            source,
            language,
            translation,
        }) if source == "Next sentence."
            && language.as_deref() == Some("en")
            && translation == "次の文。"
    ));
}

#[tokio::test]
async fn full_mock_lifecycle_waits_for_turn_complete_and_trailing_transcripts() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            assert_setup(socket.next().await.unwrap().unwrap());
            socket
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            let audio = socket.next().await.unwrap().unwrap();
            let audio: Value = serde_json::from_str(audio.to_text().unwrap()).unwrap();
            let frame = base64::engine::general_purpose::STANDARD
                .decode(
                    audio["realtimeInput"]["audio"]["data"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(frame.len(), GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT);
            assert_eq!(&frame[..3], &[1, 2, 3]);

            let end = socket.next().await.unwrap().unwrap();
            let end: Value = serde_json::from_str(end.to_text().unwrap()).unwrap();
            assert_eq!(end, json!({"realtimeInput": {"audioStreamEnd": true}}));
            socket
                .send(Message::Text(
                    r#"{"serverContent":{"inputTranscription":{"text":"Hello ","languageCode":"en-US"},"outputTranscription":{"text":"こんにちは"},"turnComplete":true}}"#.into(),
                ))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            socket
                .send(Message::Text(
                    r#"{"serverContent":{"inputTranscription":{"text":"world."},"outputTranscription":{"text":"世界。"}}}"#.into(),
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
    client.send_audio(&[1, 2, 3]).await.unwrap();
    client.finish(Duration::from_secs(1)).await;

    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received.iter().any(|event| matches!(
        event,
        LiveTranslateServerEvent::SubtitleFinalPair { source, language, translation }
            if source == "Hello world."
                && language.as_deref() == Some("en-us")
                && translation == "こんにちは世界。"
    )));
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
}

#[tokio::test]
async fn normal_turn_boundary_waits_for_late_tail_without_mixing_the_next_turn() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            assert_setup(socket.next().await.unwrap().unwrap());
            socket
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"serverContent":{"inputTranscription":{"text":"First "},"outputTranscription":{"text":"第一"},"turnComplete":true}}"#.into(),
                ))
                .await
                .unwrap();
            // The tail is deliberately later than the old 150 ms grace.
            tokio::time::sleep(Duration::from_millis(250)).await;
            socket
                .send(Message::Text(
                    r#"{"serverContent":{"inputTranscription":{"text":"sentence."},"outputTranscription":{"text":"句。"}}}"#.into(),
                ))
                .await
                .unwrap();
            tokio::time::sleep(TAIL_QUIET_PERIOD + Duration::from_millis(100)).await;
            socket
                .send(Message::Text(
                    r#"{"serverContent":{"inputTranscription":{"text":"Next sentence."},"outputTranscription":{"text":"次の文。"},"turnComplete":true}}"#.into(),
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

    let mut final_pairs = Vec::new();
    while final_pairs.len() < 2 {
        let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .expect("both quiet-boundary commits must finish")
            .expect("the provider event channel must remain open");
        if let LiveTranslateServerEvent::SubtitleFinalPair {
            source,
            language,
            translation,
        } = event
        {
            final_pairs.push((source, language, translation));
        }
    }
    assert_eq!(
        final_pairs,
        vec![
            ("First sentence.".into(), None, "第一句。".into()),
            ("Next sentence.".into(), None, "次の文。".into()),
        ]
    );
    client.disconnect().await;
}

#[tokio::test]
async fn interrupted_close_discards_partial_pair_but_finishes_cleanly() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            assert_setup(socket.next().await.unwrap().unwrap());
            socket
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"serverContent":{"inputTranscription":{"text":"partial private source"},"outputTranscription":{"text":"partial private translation"}}}"#.into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if message
                    .to_text()
                    .ok()
                    .is_some_and(|text| text.contains("audioStreamEnd"))
                {
                    socket
                        .send(Message::Text(
                            r#"{"serverContent":{"interrupted":true,"turnComplete":true}}"#
                                .into(),
                        ))
                        .await
                        .unwrap();
                    while socket.next().await.is_some() {}
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
    client.finish(Duration::from_secs(2)).await;

    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    assert!(received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. })));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
}

#[tokio::test]
async fn close_timeout_is_finite_and_drops_an_unconfirmed_tail() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    r#"{"serverContent":{"inputTranscription":{"text":"private source"},"outputTranscription":{"text":"private translation"}}}"#.into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if message
                    .to_text()
                    .ok()
                    .is_some_and(|text| text.contains("audioStreamEnd"))
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
        LiveTranslateServerEvent::Error { code, .. } if code == "gemini_close_timeout"
    )));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. })));
    assert!(!received
        .iter()
        .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
}

#[tokio::test]
async fn an_immediate_pong_cannot_be_lost() {
    let (client, _events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if let Message::Ping(payload) = message {
                    socket.send(Message::Pong(payload)).await.unwrap();
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
    assert!(!client.inner.transport_health.lock().await.unwrap().stable);
    // Advance only this transport's start time; the second real Pong must
    // still be received before it can authorize planned rotation.
    client
        .inner
        .transport_health
        .lock()
        .await
        .as_mut()
        .unwrap()
        .connected_at -= crate::core::lifecycle::health::HEALTH_CHECK_INTERVAL;
    assert!(!client.inner.transport_health.lock().await.unwrap().stable);
    client.ping(Duration::from_millis(200)).await.unwrap();
    assert!(client.inner.transport_health.lock().await.unwrap().stable);
    client.disconnect().await;
}

#[tokio::test]
async fn stale_socket_events_are_suppressed_after_disconnect() {
    let (client, mut events) = test_client(|mut socket| {
        Box::pin(async move {
            let _ = socket.next().await;
            socket
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = socket
                .send(Message::Text(
                    r#"{"serverContent":{"inputTranscription":{"text":"stale private text"}}}"#
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
    while events.try_recv().is_ok() {}
    client.disconnect().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(events.try_recv().is_err());
}
