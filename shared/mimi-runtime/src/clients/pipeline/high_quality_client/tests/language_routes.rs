//! Language routes pipeline regressions.

use super::*;

#[tokio::test]
async fn apple_final_uses_reported_source_before_deciding_passthrough() {
    use crate::clients::translation::apple_translation_client::AppleTranslationClient;

    for (target, reported, expected_request_source) in [
        (
            TargetLanguage::English,
            Some("ja"),
            Some(SourceLanguage::Japanese),
        ),
        (
            TargetLanguage::Japanese,
            Some("fr"),
            Some(SourceLanguage::French),
        ),
        (TargetLanguage::English, Some("en"), None),
        (TargetLanguage::English, None, None),
        (
            TargetLanguage::Japanese,
            None,
            Some(SourceLanguage::English),
        ),
    ] {
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = requests.clone();
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_apple(
            "synthetic-asr",
            SourceLanguage::English,
            target,
            sender,
        )
        .unwrap();
        client.mt = Arc::new(TextTranslationClient::Apple(
            AppleTranslationClient::new(SourceLanguage::English, target)
                .unwrap()
                .with_synthetic_translation(move |_, source, target| {
                    observed.lock().unwrap().push((source, target));
                    Ok("Synthetic translated final.".into())
                }),
        ));
        *client.translation_latency.lock().unwrap() = Some(TranslationLatency {
            milliseconds: 123,
            kind: TranslationLatencyKind::Request,
        });
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: "Synthetic source final.".into(),
                language: reported.map(str::to_owned),
            })
            .await;
        let translation = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match events.recv().await.unwrap() {
                    LiveTranslateServerEvent::SubtitleConfirmedPair { translation, .. } => {
                        break translation;
                    }
                    LiveTranslateServerEvent::Error { code, .. } => panic!("{code}"),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            *requests.lock().unwrap(),
            expected_request_source
                .map(|source| vec![(source, target)])
                .unwrap_or_default()
        );
        assert_eq!(
            translation,
            if expected_request_source.is_some() {
                "Synthetic translated final."
            } else {
                "Synthetic source final."
            }
        );
        assert_eq!(
            client.translation_latency().map(|sample| sample.kind),
            expected_request_source.map(|_| TranslationLatencyKind::Request),
            "target={target:?}, reported={reported:?}"
        );
        client.disconnect().await;
    }
}

#[tokio::test]
async fn explicit_cloud_text_source_is_not_replaced_by_recognizer_report() {
    use tokio::io::AsyncWriteExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_synthetic_request(&mut socket).await;
        let body_start = request
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap()
            + 4;
        let body: serde_json::Value = serde_json::from_slice(&request[body_start..]).unwrap();
        assert_eq!(body["source_lang"], "EN");
        let response = r#"{"code":200,"data":"Synthetic translation"}"#;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
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
            text: "Synthetic explicit-source final.".into(),
            language: Some("fr".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await.unwrap() {
                LiveTranslateServerEvent::SubtitleConfirmedPair { translation, .. } => {
                    assert_eq!(translation, "Synthetic translation");
                    break;
                }
                LiveTranslateServerEvent::Error { code, .. } => panic!("{code}"),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    server.await.unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn apple_unknown_report_cannot_fall_back_to_same_language_passthrough() {
    let (sender, mut events) = provider_event_channel();
    let client = HighQualityTranslationClient::new_apple(
        "synthetic-asr",
        SourceLanguage::English,
        TargetLanguage::English,
        sender,
    )
    .unwrap();
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
            text: "Synthetic unknown-language final.".into(),
            language: Some("unknown-report".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match events.recv().await.unwrap() {
                LiveTranslateServerEvent::Error { code, .. } => {
                    assert_eq!(code, "translation_source_unsupported");
                    break;
                }
                LiveTranslateServerEvent::SubtitleConfirmedPair { .. } => {
                    panic!("unknown source must not be committed as translated")
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn same_language_draft_cancels_a_cross_language_preview_and_rejects_its_late_events() {
    let (client, mut events) = test_client(TargetLanguage::Japanese, 20);
    let old_id = client.advance_preview_epoch();
    let old_partial = client.preview_partial_handler(old_id);
    client.state.lock().await.preview_task = Some(TaskSlot {
        id: old_id,
        handle: tokio::spawn(std::future::pending()),
    });
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "同じ言語のプレビュー。".into(),
            language: Some("ja".into()),
        })
        .await;
    old_partial("obsolete translation".into());
    client.emit_preview(
        old_id,
        LiveTranslateServerEvent::SourceDraft {
            text: "obsolete source".into(),
            language: Some("en".into()),
        },
    );
    assert_eq!(
        events.try_recv(),
        Ok(LiveTranslateServerEvent::SourceDraft {
            text: "同じ言語のプレビュー。".into(),
            language: Some("ja".into()),
        })
    );
    assert_eq!(
        events.try_recv(),
        Ok(LiveTranslateServerEvent::TranslationDraft(
            "同じ言語のプレビュー。".into()
        ))
    );
    assert!(events.try_recv().is_err());
    assert!(client.state.lock().await.preview_task.is_none());
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "An unknown-language preview.".into(),
            language: None,
        })
        .await;
    assert_eq!(
        events.try_recv(),
        Ok(LiveTranslateServerEvent::SourceDraft {
            text: "An unknown-language preview.".into(),
            language: None,
        })
    );
    assert_eq!(
        events.try_recv(),
        Ok(LiveTranslateServerEvent::TranslationDraft(String::new()))
    );
    let inner = client.state.lock().await;
    assert!(inner.draft_stability_task.is_some());
    assert!(inner.draft_maximum_wait_task.is_some());
    drop(inner);
    client.reset_draft_state().await;
    old_partial("still obsolete".into());
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn mixed_language_finals_keep_order_and_unknown_language_still_requests_translation() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (request_seen, wait_for_request) = oneshot::channel();
    let (release, wait_for_release) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut request_seen = Some(request_seen);
        let mut wait_for_release = Some(wait_for_release);
        for translation in ["First translated", "Unknown translated"] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            if let Some(request_seen) = request_seen.take() {
                request_seen.send(()).unwrap();
                wait_for_release.take().unwrap().await.unwrap();
            }
            let body = serde_json::json!({"code":200,"data":translation}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    });
    let (sender, mut events) = provider_event_channel();
    let client = HighQualityTranslationClient::new_deeplx(
        "synthetic-asr",
        &endpoint,
        "",
        SourceLanguage::Automatic,
        TargetLanguage::Japanese,
        sender,
    )
    .unwrap();
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
            text: "First English line.".into(),
            language: Some("en".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(1), wait_for_request)
        .await
        .unwrap()
        .unwrap();
    for (text, language) in [
        ("日本語はそのまま。", Some("ja")),
        ("Unknown language line.", None),
    ] {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: text.into(),
                language: language.map(str::to_string),
            })
            .await;
    }
    let mut started = 0;
    while let Ok(event) = events.try_recv() {
        match event {
            LiveTranslateServerEvent::TranslationStarted => started += 1,
            LiveTranslateServerEvent::SubtitleConfirmedPair { .. } => {
                panic!("later finals must wait for the active final")
            }
            _ => {}
        }
    }
    release.send(()).unwrap();
    let mut pairs = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), async {
        while pairs.len() < 3 {
            match events.recv().await.unwrap() {
                LiveTranslateServerEvent::TranslationStarted => started += 1,
                LiveTranslateServerEvent::SubtitleConfirmedPair {
                    source,
                    translation,
                    ..
                } => pairs.push((source, translation)),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        pairs,
        vec![
            ("First English line.".into(), "First translated".into()),
            ("日本語はそのまま。".into(), "日本語はそのまま。".into()),
            ("Unknown language line.".into(), "Unknown translated".into()),
        ]
    );
    assert_eq!(started, 2);
    client
        .wait_for_final_translations(Duration::from_secs(1))
        .await;
    assert_eq!(
        client.translation_latency().unwrap().kind,
        TranslationLatencyKind::Request
    );
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn a_deferred_same_language_preview_and_finish_use_no_text_request() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (sender, mut events) = provider_event_channel();
    let client = HighQualityTranslationClient::new_deeplx(
        "synthetic-asr",
        &format!("http://{}", listener.local_addr().unwrap()),
        "",
        SourceLanguage::Automatic,
        TargetLanguage::English,
        sender,
    )
    .unwrap();
    client.state.lock().await.final_worker = Some(TaskSlot {
        id: 999,
        handle: tokio::spawn(std::future::pending()),
    });
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "An unfinished synthetic line".into(),
            language: Some("en".into()),
        })
        .await;
    assert!(events.try_recv().is_err());
    abort_task(&mut client.state.lock().await.final_worker);
    client.resume_pending_preview_if_final_lane_idle().await;
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
    for expected in [
        LiveTranslateServerEvent::SourceDraft {
            text: "An unfinished synthetic line".into(),
            language: Some("en".into()),
        },
        LiveTranslateServerEvent::TranslationDraft("An unfinished synthetic line".into()),
    ] {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap(),
            Some(expected)
        );
    }
    client.flush_pending_draft().await;
    client.flush_pending_draft().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap(),
        Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: None,
            utterance_id: 1,
            source: "An unfinished synthetic line".into(),
            language: Some("en".into()),
            translation: "An unfinished synthetic line".into(),
        })
    );
    client
        .wait_for_final_translations(Duration::from_secs(1))
        .await;
    assert!(events.try_recv().is_err());
    assert_eq!(client.translation_latency(), None);
    client.disconnect().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn same_language_drafts_and_finals_skip_http_and_clear_request_latency() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    for (target, language) in [
        (TargetLanguage::English, "en"),
        (TargetLanguage::Japanese, "ja"),
        (TargetLanguage::SimplifiedChinese, "zh"),
        (TargetLanguage::SimplifiedChinese, "zh-CN"),
        (TargetLanguage::SimplifiedChinese, "zh-Hans"),
    ] {
        let (sender, mut events) = provider_event_channel();
        let client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &endpoint,
            "",
            SourceLanguage::Automatic,
            target,
            sender,
        )
        .unwrap();
        *client.translation_latency.lock().unwrap() = Some(TranslationLatency {
            milliseconds: 123,
            kind: TranslationLatencyKind::Request,
        });
        for text in ["Synthetic", "Synthetic unchanged line."] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: text.into(),
                    language: Some(language.into()),
                })
                .await;
        }
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic unchanged line.".into(),
                language: Some(language.into()),
            })
        );
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::TranslationDraft(
                "Synthetic unchanged line.".into()
            ))
        );
        assert!(events.try_recv().is_err());
        assert_eq!(client.translation_latency(), None);
        {
            let inner = client.state.lock().await;
            assert!(inner.preview_task.is_none());
            assert!(inner.draft_stability_task.is_none());
            assert!(inner.draft_maximum_wait_task.is_none());
        }
        let final_event = LiveTranslateServerEvent::SourceFinal {
            text: "Synthetic unchanged line.".into(),
            language: Some(language.into()),
        };
        client.handle_asr_event(final_event.clone()).await;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap(),
            Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: None,
                utterance_id: 1,
                source: "Synthetic unchanged line.".into(),
                language: Some(language.into()),
                translation: "Synthetic unchanged line.".into(),
            })
        );
        client.handle_asr_event(final_event).await;
        client
            .wait_for_final_translations(Duration::from_secs(1))
            .await;
        assert!(events.try_recv().is_err());
        assert_eq!(client.translation_latency(), None);
        client.disconnect().await;
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}
