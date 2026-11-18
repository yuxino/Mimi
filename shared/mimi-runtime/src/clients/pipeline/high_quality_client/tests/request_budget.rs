//! Request budget pipeline regressions.

use super::*;

#[tokio::test]
async fn lite_rejects_known_unsupported_source_before_http_or_request_accounting() {
    let (sender, _events) = provider_event_channel();
    let client = HighQualityTranslationClient::new(
        "synthetic-key",
        SourceLanguage::Automatic,
        TargetLanguage::Japanese,
        crate::core::protocols::translation::qwen_mt::REALTIME_MT_MODEL,
        Duration::from_secs(60),
        Duration::from_secs(60),
        12,
        sender,
    )
    .unwrap();
    assert!(!client.mt.supports_reported_source(Some("no")));
    assert!(client.mt.supports_reported_source(None));
    assert!(client.mt.supports_reported_source(Some("unknown-report")));
    let error = client
        .translate_with_retry(
            "Synthetic unsupported source",
            Some("no"),
            tokio::time::Instant::now() + Duration::from_secs(1),
            Arc::new(|_| {}),
            TranslationWorkOwner::Preview(999),
            TranslationEvidenceIdentity::default(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error, QwenMTClientError::UnsupportedSource);
    let now = std::time::Instant::now();
    assert_eq!(
        client
            .state
            .lock()
            .await
            .preview_request_pacer
            .next_shared_start_at(now),
        now
    );
}

#[tokio::test]
async fn rejected_inflight_preview_discards_queued_work_without_automatic_retry() {
    for status in [401, 429] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (started, started_rx) = oneshot::channel();
        let (release, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = read_synthetic_request(&mut socket).await;
            started.send(()).unwrap();
            release_rx.await.unwrap();
            write_synthetic_response(&mut socket, status, "").await;
            assert!(
                tokio::time::timeout(Duration::from_millis(300), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Rejected synthetic preview".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.state.lock().await.draft_revision;
        client
            .start_preview(
                "Rejected synthetic preview".into(),
                Some("en".into()),
                revision,
            )
            .await;
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Queued synthetic successor".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.state.lock().await.draft_revision;
        client
            .start_preview(
                "Queued synthetic successor".into(),
                Some("en".into()),
                revision,
            )
            .await;
        assert_eq!(
            client.state.lock().await.pending_preview_revision,
            Some(revision)
        );
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match events.recv().await.unwrap() {
                    LiveTranslateServerEvent::Error { code, .. } if status == 401 => {
                        assert_eq!(code, "translation_authentication_failed");
                        break;
                    }
                    LiveTranslateServerEvent::TranslationDeferred(recovery)
                        if status == 429 && !recovery.retry_scheduled =>
                    {
                        assert_eq!(recovery.reason, TranslationRecoveryReason::RateLimited);
                        break;
                    }
                    LiveTranslateServerEvent::SubtitlePreviewPair { .. } => {
                        panic!("rejected request has no completed pair")
                    }
                    _ => {}
                }
            }
            while client.state.lock().await.preview_task.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        {
            let inner = client.state.lock().await;
            assert!(inner.pending_preview_revision.is_none());
            assert_eq!(inner.pipeline_failed, status == 401);
            if status == 429 {
                assert!(!inner
                    .preview_request_pacer
                    .suppression_remaining(std::time::Instant::now())
                    .is_zero());
            }
        }
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "New speech after rejected preview".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.state.lock().await.draft_revision;
        client
            .start_preview(
                "New speech after rejected preview".into(),
                Some("en".into()),
                revision,
            )
            .await;
        assert!(client.state.lock().await.preview_task.is_none());
        server.await.unwrap();
        client.disconnect().await;
    }
}

#[tokio::test]
async fn final_preempts_preview_but_keeps_the_shared_http_budget_and_order() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for expected in ["First final", "Second final"] {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let request = read_synthetic_request(&mut socket).await;
            let request = String::from_utf8(request).unwrap();
            assert!(request.contains(expected));
            assert!(!request.contains("Unstarted preview"));
            use tokio::io::AsyncWriteExt;
            let body = r#"{"code":200,"data":"Confirmed translation"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(800), listener.accept())
                .await
                .is_err()
        );
    });
    let (sender, mut events) = provider_event_channel();
    let mut client = HighQualityTranslationClient::new_deeplx(
        "synthetic-asr",
        &endpoint,
        "",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender,
    )
    .unwrap();
    client.stable_draft_delay = Duration::from_secs(60);
    client.maximum_wait_delay = Duration::from_secs(60);
    client
        .state
        .lock()
        .await
        .preview_request_pacer
        .record_start(std::time::Instant::now());
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "Unstarted preview".into(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview("Unstarted preview".into(), Some("en".into()), revision)
        .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let started = tokio::time::Instant::now();
    for text in ["First final", "Second final"] {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: text.into(),
                language: Some("en".into()),
            })
            .await;
    }
    let mut sources = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        while sources.len() < 2 {
            if let LiveTranslateServerEvent::SubtitleConfirmedPair { source, .. } =
                events.recv().await.unwrap()
            {
                sources.push(source);
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(sources, ["First final", "Second final"]);
    assert!(started.elapsed() >= Duration::from_millis(1_100));
    assert!(started.elapsed() < Duration::from_secs(3));
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn exhausted_preview_reports_no_scheduled_retry_and_new_speech_keeps_cooldown() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for _ in 0..MAX_TRANSLATION_ATTEMPTS {
            let (mut socket, _) = listener.accept().await.unwrap();
            reply_to_synthetic_request(&mut socket, 503, "").await;
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(300), listener.accept())
                .await
                .is_err()
        );
    });
    let (sender, mut events) = provider_event_channel();
    let mut client = HighQualityTranslationClient::new_deeplx(
        "synthetic-asr",
        &endpoint,
        "",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender,
    )
    .unwrap();
    client.stable_draft_delay = Duration::from_secs(60);
    client.maximum_wait_delay = Duration::from_secs(60);
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "Bounded failed preview".into(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview("Bounded failed preview".into(), Some("en".into()), revision)
        .await;
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            match events.recv().await.unwrap() {
                LiveTranslateServerEvent::TranslationDeferred(recovery)
                    if !recovery.retry_scheduled =>
                {
                    assert_eq!(
                        recovery.reason,
                        TranslationRecoveryReason::TemporarilyUnavailable
                    );
                    assert!(recovery.retry_after_ms > 0);
                    break;
                }
                LiveTranslateServerEvent::Error { .. } => {
                    panic!("preview failure must preserve capture")
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    {
        let inner = client.state.lock().await;
        assert!(inner.preview_task.is_none());
        assert!(!inner.pipeline_failed);
        assert!(inner.mt_cooldown.is_some());
        assert_eq!(inner.mt_failure_streak, MAX_TRANSLATION_ATTEMPTS);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        events.try_recv().is_err(),
        "silence must not start infinite retry timers"
    );
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "New speech during cooldown".into(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview(
            "New speech during cooldown".into(),
            Some("en".into()),
            revision,
        )
        .await;
    tokio::time::timeout(Duration::from_millis(100), async {
        loop {
            if let LiveTranslateServerEvent::TranslationDeferred(recovery) =
                events.recv().await.unwrap()
            {
                assert!(recovery.retry_scheduled);
                assert!(recovery.retry_after_ms > 0);
                break;
            }
        }
    })
    .await
    .unwrap();
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn a_preview_rate_limit_survives_cancellation_and_defers_ordered_finals() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        reply_to_synthetic_request(&mut socket, 429, "").await;
        drop(socket);
        let rejected_at = tokio::time::Instant::now();
        for translation in ["First confirmed", "Second confirmed"] {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert!(rejected_at.elapsed() >= Duration::from_secs(4));
            tokio::time::sleep(Duration::from_millis(20)).await;
            let body = serde_json::json!({"code":200,"data":translation}).to_string();
            reply_to_synthetic_request(&mut socket, 200, &body).await;
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    });
    let (sender, mut events) = provider_event_channel();
    let mut client = HighQualityTranslationClient::new_deeplx(
        "synthetic-asr",
        &endpoint,
        "",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender,
    )
    .unwrap();
    client.stable_draft_delay = Duration::from_secs(60);
    client.maximum_wait_delay = Duration::from_secs(60);
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "Replaceable preview.".into(),
            language: Some("en".into()),
        })
        .await;
    let (timer_id, revision) = {
        let inner = client.state.lock().await;
        (
            inner.draft_stability_task.as_ref().unwrap().id,
            inner.draft_revision,
        )
    };
    client
        .handle_draft_timer(DraftTimerKind::Stable, timer_id, revision)
        .await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let LiveTranslateServerEvent::TranslationDeferred(recovery) =
                events.recv().await.unwrap()
            {
                assert_eq!(recovery.reason, TranslationRecoveryReason::RateLimited);
                assert!(recovery.retry_after_ms >= 29_000);
                assert!(!recovery.retry_scheduled);
                break;
            }
        }
    })
    .await
    .unwrap();
    // Additional ASR changes must not start replacement MT while the
    // thirty-second preview pause reserves capacity for server finals.
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "New cumulative speech during rate limit".into(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview(
            "New cumulative speech during rate limit".into(),
            Some("en".into()),
            revision,
        )
        .await;
    assert!(client.state.lock().await.preview_task.is_none());
    for text in ["First authoritative line.", "Second authoritative line."] {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: text.into(),
                language: Some("en".into()),
            })
            .await;
    }
    {
        let inner = client.state.lock().await;
        assert!(inner.mt_cooldown.is_some());
        assert_eq!(inner.mt_failure_streak, 1);
        assert!(inner.preview_task.is_none());
    }
    let mut pairs = Vec::new();
    tokio::time::timeout(Duration::from_secs(8), async {
        while pairs.len() < 2 {
            match events.recv().await.unwrap() {
                LiveTranslateServerEvent::SubtitleConfirmedPair {
                    source,
                    translation,
                    ..
                } => pairs.push((source, translation)),
                LiveTranslateServerEvent::Error { code, .. } => {
                    panic!("temporary cooldown must preserve finals: {code}")
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        pairs,
        vec![
            ("First authoritative line.".into(), "First confirmed".into()),
            (
                "Second authoritative line.".into(),
                "Second confirmed".into()
            )
        ]
    );
    client
        .wait_for_final_translations(Duration::from_secs(1))
        .await;
    let latency = client.translation_latency().unwrap();
    assert!(latency.milliseconds >= 20);
    assert!(
        latency.milliseconds < 1_000,
        "initial shared cooldown is excluded from HTTP request timing"
    );
    assert!(
        !client
            .state
            .lock()
            .await
            .preview_request_pacer
            .suppression_remaining(std::time::Instant::now())
            .is_zero(),
        "successful finals must not refund the preview pause"
    );
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn transient_finals_exhaust_bounded_retries_with_a_recoverable_error() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for _ in 0..MAX_TRANSLATION_ATTEMPTS {
            let (mut socket, _) = listener.accept().await.unwrap();
            reply_to_synthetic_request(&mut socket, 503, "").await;
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
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
            text: "Synthetic transient final.".into(),
            language: Some("en".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            match events.recv().await.unwrap() {
                LiveTranslateServerEvent::Error { code, message } => {
                    assert_eq!(code, "translation_temporarily_unavailable");
                    assert_eq!(message, "translation_temporarily_unavailable");
                    break;
                }
                LiveTranslateServerEvent::SubtitleConfirmedPair { .. } => {
                    panic!("a rejected request cannot commit a final")
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(client.translation_latency(), None);
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn authentication_failure_is_permanent_and_never_enters_cooldown() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        reply_to_synthetic_request(&mut socket, 401, "").await;
        drop(socket);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
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
            text: "Synthetic authentication check.".into(),
            language: Some("en".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match events.recv().await.unwrap() {
                LiveTranslateServerEvent::Error { code, message } => {
                    assert_eq!(code, "translation_authentication_failed");
                    assert_eq!(message, "credential_authentication_failed");
                    break;
                }
                LiveTranslateServerEvent::TranslationDeferred(_) => {
                    panic!("authentication cannot recover by waiting")
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert!(client.state.lock().await.mt_cooldown.is_none());
    client.disconnect().await;
    server.await.unwrap();
}
