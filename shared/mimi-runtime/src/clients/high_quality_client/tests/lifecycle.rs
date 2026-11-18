//! Lifecycle pipeline regressions.

use super::*;

#[tokio::test]
async fn clear_after_a_real_sse_partial_does_not_restore_late_stream_content() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = url::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _request = read_synthetic_request(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
        let partial =
            "data: {\"choices\":[{\"delta\":{\"content\":\"Synthetic old partial\"}}]}\n\n";
        socket
            .write_all(format!("{:x}\r\n{partial}\r\n", partial.len()).as_bytes())
            .await
            .unwrap();
        release_rx.await.unwrap();
        let late = "data: {\"choices\":[{\"delta\":{\"content\":\" late old suffix\"}}]}\n\ndata: [DONE]\n\n";
        let _ = socket
            .write_all(format!("{:x}\r\n{late}\r\n0\r\n\r\n", late.len()).as_bytes())
            .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let (sender, mut receiver) = provider_event_channel();
    let mut client = HighQualityTranslationClient::new(
        "synthetic-key",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        QwenMTModel::Lite,
        Duration::from_secs(60),
        Duration::from_secs(60),
        12,
        sender,
    )
    .unwrap();
    let TextTranslationClient::Qwen(mt, _) = Arc::make_mut(&mut client.mt) else {
        unreachable!()
    };
    mt.use_synthetic_endpoint(endpoint);
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 1,
            text: "Synthetic old source".into(),
            language: Some("en".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(receiver.recv().await, Some(LiveTranslateServerEvent::TranslationDraft(text)) if text == "Synthetic old partial") { break; }
            }
        }).await.unwrap();
    client.clear_content().await;
    assert!(receiver.try_recv().is_err());
    release_tx.send(()).unwrap();
    server.await.unwrap();
    assert!(receiver.try_recv().is_err());
    assert!(client.state.lock().await.final_worker.is_none());
    client.disconnect().await;
}

#[tokio::test]
async fn clear_keeps_quota_budget_and_rejects_old_preview_and_final_callbacks() {
    let (client, mut receiver) = test_client(TargetLanguage::Japanese, 20);
    let old_revision = client.content_revision();
    let preview_id = client.advance_preview_epoch();
    let late_partial = client.preview_partial_handler(preview_id);
    let cooldown_until = tokio::time::Instant::now() + Duration::from_secs(8);
    let now = std::time::Instant::now();
    let (shared_at, preview_at) = {
        let mut inner = client.state.lock().await;
        inner.committer.update_draft("Synthetic cleared draft");
        inner.last_source_utterance_id = Some(7);
        inner.mt_failure_streak = 2;
        inner.mt_cooldown = Some(MTCooldown {
            until: cooldown_until,
            reason: TranslationRecoveryReason::RateLimited,
        });
        inner.preview_request_pacer.record_start(now);
        inner.preview_request_pacer.suppress_after_rate_limit(now);
        inner
            .preview_request_pacer
            .set_shared_cooldown(Some(now + Duration::from_secs(8)));
        inner.preview_task = Some(TaskSlot {
            id: preview_id,
            handle: tokio::spawn(std::future::pending()),
        });
        inner.preview_http_pending = Some(preview_id);
        inner.pending_preview_revision = Some(inner.draft_revision);
        (
            inner.preview_request_pacer.next_shared_start_at(now),
            inner.preview_request_pacer.next_start_at(now),
        )
    };
    let revision = client.clear_content().await;
    assert_ne!(revision, old_revision);
    late_partial("Synthetic old partial".into());
    client.emit_content(
        old_revision,
        LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: None,
            utterance_id: 1,
            source: "old".into(),
            language: None,
            translation: "old".into(),
        },
    );
    assert!(receiver.try_recv().is_err());
    let inner = client.state.lock().await;
    assert_eq!(inner.mt_failure_streak, 2);
    assert_eq!(inner.mt_cooldown.unwrap().until, cooldown_until);
    assert_eq!(
        inner.preview_request_pacer.next_shared_start_at(now),
        shared_at
    );
    assert_eq!(inner.preview_request_pacer.next_start_at(now), preview_at);
    assert_eq!(inner.last_source_utterance_id, Some(7));
    assert!(!inner.committer.has_pending_text());
    assert!(inner.final_queue.is_empty());
    assert!(inner.preview_task.is_none());
    assert!(inner.pending_preview_revision.is_none());
    assert!(client.mt_work_allowed.load(Ordering::SeqCst));
    drop(inner);
    assert!(client
        .prepare_server_final("Synthetic replay", Some(7))
        .await
        .is_none());
    assert!(client
        .prepare_server_final("Synthetic new sentence", Some(8))
        .await
        .is_some());
    client.disconnect().await;
}

#[tokio::test]
async fn clear_cancels_inflight_http_and_queued_finals_then_new_final_recovers() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut old, _) = listener.accept().await.unwrap();
        let request = String::from_utf8(read_synthetic_request(&mut old).await).unwrap();
        assert!(request.contains("Synthetic old active"));
        started_tx.send(()).unwrap();
        release_rx.await.unwrap();
        let body = r#"{"code":200,"data":"Late old translation"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = old.write_all(response.as_bytes()).await;
        let (mut next, _) = tokio::time::timeout(Duration::from_secs(4), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let request = String::from_utf8(read_synthetic_request(&mut next).await).unwrap();
        assert!(request.contains("Synthetic new sentence"));
        assert!(!request.contains("Synthetic old queued"));
        write_synthetic_response(&mut next, 200, r#"{"code":200,"data":"New translation"}"#).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let (sender, mut receiver) = provider_event_channel();
    let mut client = HighQualityTranslationClient::new_deeplx(
        "synthetic-key",
        &endpoint,
        "",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender,
    )
    .unwrap();
    client
        .set_network(
            ProviderNetwork::resolve(&crate::core::network_proxy::ProxyConfig {
                mode: crate::core::network_proxy::ProxyMode::Direct,
                url: None,
            })
            .unwrap(),
        )
        .unwrap();
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 1,
            text: "Synthetic old active".into(),
            language: Some("en".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .unwrap()
        .unwrap();
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 2,
            text: "Synthetic old queued".into(),
            language: Some("en".into()),
        })
        .await;
    assert_eq!(client.state.lock().await.final_queue.len(), 1);
    let revision = client.clear_content().await;
    assert!(receiver.try_recv().is_err());
    release_tx.send(()).unwrap();
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 2,
            text: "Synthetic old queued".into(),
            language: Some("en".into()),
        })
        .await;
    assert!(client.state.lock().await.final_queue.is_empty());
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 3,
            text: "Synthetic new sentence".into(),
            language: Some("en".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let event = receiver.recv_with_revision().await.unwrap();
            assert_eq!(event.content_revision, revision);
            if let LiveTranslateServerEvent::SubtitleConfirmedPair {
                source,
                translation,
                ..
            } = event.event
            {
                assert_eq!(source, "Synthetic new sentence");
                assert_eq!(translation, "New translation");
                break;
            }
        }
    })
    .await
    .unwrap();
    client
        .wait_for_final_translations(Duration::from_secs(1))
        .await;
    assert!(receiver.try_recv().is_err());
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn pause_reconnect_budget_keeps_preview_suppressed_and_final_retry_cancellable() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (old_sender, _old_events) = provider_event_channel();
    let old = HighQualityTranslationClient::new_deeplx(
        "synthetic-asr",
        &endpoint,
        "",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        old_sender,
    )
    .unwrap();
    {
        let mut inner = old.state.lock().await;
        let now = std::time::Instant::now();
        inner.preview_request_pacer.record_start(now);
        inner.preview_request_pacer.suppress_after_rate_limit(now);
        inner
            .preview_request_pacer
            .set_shared_cooldown(Some(now + Duration::from_secs(4)));
    }
    let budget = old.suspend_request_budget().await;
    assert!(!old.mt_work_allowed.load(Ordering::SeqCst));
    old.disconnect().await;
    let (sender, mut events) = provider_event_channel();
    let mut resumed = HighQualityTranslationClient::new_deeplx(
        "synthetic-asr",
        &endpoint,
        "",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender,
    )
    .unwrap();
    resumed.stable_draft_delay = Duration::from_secs(60);
    resumed.maximum_wait_delay = Duration::from_secs(60);
    resumed.restore_request_budget(budget).await;
    resumed
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "Synthetic new speech after resume".into(),
            language: Some("en".into()),
        })
        .await;
    let revision = resumed.state.lock().await.draft_revision;
    resumed
        .start_preview(
            "Synthetic new speech after resume".into(),
            Some("en".into()),
            revision,
        )
        .await;
    assert!(resumed.state.lock().await.preview_task.is_none());
    resumed
        .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
            text: "Synthetic durable final after resume".into(),
            language: Some("en".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let LiveTranslateServerEvent::TranslationDeferred(recovery) =
                events.recv().await.unwrap()
            {
                assert_eq!(recovery.reason, TranslationRecoveryReason::RateLimited);
                assert!(recovery.retry_scheduled);
                assert!(recovery.retry_after_ms > 3_000);
                break;
            }
        }
    })
    .await
    .unwrap();
    resumed.disconnect().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
    assert!(resumed.state.lock().await.final_worker.is_none());
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(
            event,
            LiveTranslateServerEvent::TranslationStarted
                | LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
        ));
    }
}

#[tokio::test]
async fn disconnect_cancels_a_healthy_preview_wait_without_http_or_stale_events() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (sender, mut events) = provider_event_channel();
    let client = HighQualityTranslationClient::new_deeplx(
        "synthetic-asr",
        &endpoint,
        "",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender,
    )
    .unwrap();
    client
        .state
        .lock()
        .await
        .preview_request_pacer
        .record_start(std::time::Instant::now());
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "Cancelled before HTTP".into(),
            language: Some("en".into()),
        })
        .await;
    while events.try_recv().is_ok() {}
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview("Cancelled before HTTP".into(), Some("en".into()), revision)
        .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    client.disconnect().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(800), listener.accept())
            .await
            .is_err()
    );
    assert!(events.try_recv().is_err());
    let inner = client.state.lock().await;
    assert!(inner.preview_task.is_none());
    let now = std::time::Instant::now();
    assert_eq!(inner.preview_request_pacer.next_start_at(now), now);
}

#[tokio::test]
async fn stopping_during_mt_cooldown_cancels_retry_and_clears_owned_state() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        reply_to_synthetic_request(&mut socket, 429, "").await;
        drop(socket);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), listener.accept())
                .await
                .is_err()
        );
    });
    let (sender, mut events) = provider_event_channel();
    let client = HighQualityTranslationClient::new_deeplx(
        "synthetic-asr",
        &endpoint,
        "",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender,
    )
    .unwrap();
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
            text: "Synthetic final before stop.".into(),
            language: Some("en".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !matches!(
            events.recv().await.unwrap(),
            LiveTranslateServerEvent::TranslationDeferred(_)
        ) {}
    })
    .await
    .unwrap();
    client.finish().await;
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
            text: "A teardown final must not restart text requests.".into(),
            language: Some("en".into()),
        })
        .await;
    assert!(!client.mt_work_allowed.load(Ordering::SeqCst));
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(
            event,
            LiveTranslateServerEvent::Error { .. }
                | LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
                | LiveTranslateServerEvent::TranslationStarted
        ));
    }
    let inner = client.state.lock().await;
    assert!(inner.final_worker.is_none());
    assert!(inner.final_queue.is_empty());
    assert!(inner.mt_cooldown.is_none());
    assert_eq!(inner.mt_failure_streak, 0);
    drop(inner);
    assert_eq!(client.translation_latency(), None);
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn reset_preview_cannot_overwrite_or_clear_a_new_preview() {
    let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
    let old_id = client.advance_preview_epoch();
    let old_partial = client.preview_partial_handler(old_id);
    client.reset_draft_state().await;
    let current_id = client.advance_preview_epoch();
    let current_partial = client.preview_partial_handler(current_id);
    let current_source = LiveTranslateServerEvent::SourceDraft {
        text: "new source".into(),
        language: Some("en".into()),
    };
    client.emit_preview(current_id, current_source.clone());
    current_partial("新的预览".into());

    client.emit_preview(
        old_id,
        LiveTranslateServerEvent::SourceDraft {
            text: "obsolete source".into(),
            language: Some("en".into()),
        },
    );
    old_partial("obsolete partial".into());
    old_partial(String::new());
    client.emit_preview(
        old_id,
        LiveTranslateServerEvent::TranslationDraft(String::new()),
    );

    assert_eq!(events.try_recv(), Ok(current_source));
    assert_eq!(
        events.try_recv(),
        Ok(LiveTranslateServerEvent::TranslationDraft(
            "新的预览".into()
        ))
    );
    assert_eq!(
        events.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    );
}

#[tokio::test]
async fn audio3_finish_drain_contract() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let fixtures: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    for case in fixtures["audio3FinishDrain"].as_array().unwrap() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let wire_case = case.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let run = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let run: serde_json::Value = serde_json::from_str(&run).unwrap();
            assert_eq!(run["header"]["action"], "run-task");
            let task_id = run["header"]["task_id"].as_str().unwrap();
            assert!(!task_id.is_empty());
            socket
                .send(Message::Text(
                    serde_json::json!({"header":{"event":"task-started","task_id":task_id}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            socket.send(Message::Text(serde_json::json!({"header":{"event":"result-generated","task_id":task_id},"payload":{"output":{"sentence":{"sentence_id":1,"sentence_end":false,"text":wire_case["initialDraft"]}}}}).to_string().into())).await.unwrap();
            loop {
                let frame = socket.next().await.unwrap().unwrap();
                if let Message::Text(text) = frame {
                    let command: serde_json::Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(command["header"]["action"], "finish-task");
                    assert_eq!(command["header"]["task_id"], task_id);
                    break;
                }
            }
            // This synthetic server deliberately sends a tail after Stop
            // and omits its terminal notification in the missing-ack cases.
            tokio::time::sleep(Duration::from_millis(40)).await;
            for sentence in wire_case["finishSentences"].as_array().unwrap() {
                socket.send(Message::Text(serde_json::json!({"header":{"event":"result-generated","task_id":task_id},"payload":{"output":{"sentence":{"sentence_id":sentence["id"],"sentence_end":sentence["final"],"text":sentence["text"]}}}}).to_string().into())).await.unwrap();
            }
            if wire_case["acknowledge"].as_bool().unwrap() {
                socket
                    .send(Message::Text(
                        serde_json::json!({"header":{"event":"task-finished","task_id":task_id}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(3), socket.next())
                    .await
                    .unwrap(),
                None | Some(Ok(Message::Close(_)))
            ));
        });
        let mut configuration =
            custom_configuration(ProviderKind::CustomDashScopeASR, TargetLanguage::Original);
        configuration.credentials = ProviderCredentials::CustomSpeech {
            endpoint: format!("ws://{address}/asr"),
            model: "synthetic-recognition".into(),
            api_key: "synthetic-key".into(),
        };
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_custom(&configuration, sender).unwrap();
        client
            .set_network(
                ProviderNetwork::resolve(&crate::core::network_proxy::ProxyConfig {
                    mode: crate::core::network_proxy::ProxyMode::Direct,
                    url: None,
                })
                .unwrap(),
            )
            .unwrap();
        client.connect().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(
                    events.recv().await,
                    Some(LiveTranslateServerEvent::SourceUtteranceDraft {
                        utterance_id: 1,
                        ..
                    })
                ) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        let started = tokio::time::Instant::now();
        client.finish().await;
        let elapsed = started.elapsed();
        let received: Vec<_> = std::iter::from_fn(|| events.try_recv().ok()).collect();
        let finished = received
            .iter()
            .filter(|event| **event == LiveTranslateServerEvent::SessionFinished)
            .count();
        let confirmed: Vec<_> = received
            .into_iter()
            .filter_map(|event| match event {
                LiveTranslateServerEvent::SubtitleConfirmedPair { source, .. } => Some(source),
                _ => None,
            })
            .collect();
        server.await.unwrap();
        assert_eq!(
            serde_json::json!(confirmed),
            case["confirmedSources"],
            "{}",
            case["id"]
        );
        assert_eq!(
            finished, 1,
            "{}: the stopped producer must acknowledge its drain exactly once",
            case["id"]
        );
        eprintln!("audio3 finish fixture={} elapsed={elapsed:?}", case["id"]);
    }
}

#[tokio::test]
async fn finish_fallback_preserves_latest_queued_asr_draft_before_finish_ack() {
    let (client, mut events) = test_client(TargetLanguage::Original, 20);
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "Synthetic partial".into(),
            language: Some("ja".into()),
        })
        .await;
    while events.try_recv().is_ok() {}

    // The recognizer can publish its last cumulative draft and terminal
    // acknowledgement before the bridge receives another scheduling turn.
    let (asr_events, asr_receiver) = provider_event_channel();
    asr_events
        .send(LiveTranslateServerEvent::SourceDraft {
            text: "Synthetic partial with the final tail.".into(),
            language: Some("ja".into()),
        })
        .unwrap();
    asr_events
        .send(LiveTranslateServerEvent::SessionFinished)
        .unwrap();
    client.install_asr_bridge(asr_receiver).await;
    assert!(
        client
            .wait_for_asr_bridge_finish(Duration::from_secs(1))
            .await
    );

    // Exercise the same fallback that finish invokes after this ack. No
    // provider or audio capture is started, and Original never invokes MT.
    client.flush_pending_draft().await;
    client.disconnect_asr_bridge().await;
    let confirmed: Vec<_> = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| match event {
            LiveTranslateServerEvent::SubtitleConfirmedPair { source, .. } => Some(source),
            _ => None,
        })
        .collect();
    assert_eq!(confirmed, ["Synthetic partial with the final tail."]);
}

#[tokio::test]
async fn finish_ack_waits_for_the_bridge_to_consume_the_queued_final() {
    let (client, mut events) = test_client(TargetLanguage::Original, 20);
    let (asr_events, asr_receiver) = provider_event_channel();
    client.install_asr_bridge(asr_receiver).await;
    let mut finish_ack = client.take_asr_bridge_finish_ack().await.unwrap();

    // Hold the draft/final state so the bridge cannot finish consuming the
    // authoritative final. The per-connection acknowledgement uses separate
    // bridge state, so observing it never shares this lock.
    let inner = client.state.lock().await;
    asr_events
        .send(LiveTranslateServerEvent::SourceFinal {
            text: "最後の字幕。".into(),
            language: Some("ja".into()),
        })
        .unwrap();
    asr_events
        .send(LiveTranslateServerEvent::SessionFinished)
        .unwrap();
    tokio::task::yield_now().await;
    assert!(matches!(
        finish_ack.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));

    drop(inner);
    assert!(tokio::time::timeout(Duration::from_secs(1), finish_ack)
        .await
        .unwrap()
        .is_ok());
    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: None,
            utterance_id: 1,
            source: "最後の字幕。".into(),
            language: Some("ja".into()),
            translation: "最後の字幕。".into(),
        })
    );
    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::SessionFinished)
    );
    client.disconnect_asr_bridge().await;
}

#[tokio::test]
async fn finish_ack_is_scoped_to_one_bridge_connection() {
    let (client, _events) = test_client(TargetLanguage::Original, 20);

    let (first_events, first_receiver) = provider_event_channel();
    client.install_asr_bridge(first_receiver).await;
    first_events
        .send(LiveTranslateServerEvent::SessionFinished)
        .unwrap();
    assert!(
        client
            .wait_for_asr_bridge_finish(Duration::from_secs(1))
            .await
    );

    let (_second_events, second_receiver) = provider_event_channel();
    client.install_asr_bridge(second_receiver).await;
    assert!(
        !client
            .wait_for_asr_bridge_finish(Duration::from_millis(25))
            .await
    );
    client.disconnect_asr_bridge().await;
}
