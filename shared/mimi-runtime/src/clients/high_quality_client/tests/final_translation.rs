//! Final translation pipeline regressions.

use super::*;

#[tokio::test]
async fn final_and_disconnect_cancel_inflight_http_and_its_latest_queued_preview() {
    use tokio::io::AsyncReadExt;
    for final_arrives in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (started, started_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
            assert!(request.contains("In-flight synthetic source"));
            started.send(()).unwrap();
            let mut tail = Vec::new();
            tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut tail))
                .await
                .unwrap()
                .unwrap();
            assert!(
                tail.is_empty(),
                "priority cancellation must close the old HTTP response"
            );
            if final_arrives {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let request = String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
                assert!(request.contains("Authoritative synthetic final"));
                assert!(!request.contains("Queued synthetic candidate"));
                write_synthetic_response(
                    &mut socket,
                    200,
                    r#"{"code":200,"data":"Confirmed synthetic result"}"#,
                )
                .await;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
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
                text: "In-flight synthetic source".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.state.lock().await.draft_revision;
        client
            .start_preview(
                "In-flight synthetic source".into(),
                Some("en".into()),
                revision,
            )
            .await;
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
        let preview_id = client.state.lock().await.preview_task.as_ref().unwrap().id;
        let late_partial = client.preview_partial_handler(preview_id);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Queued synthetic candidate".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.state.lock().await.draft_revision;
        client
            .start_preview(
                "Queued synthetic candidate".into(),
                Some("en".into()),
                revision,
            )
            .await;
        assert_eq!(
            client.state.lock().await.pending_preview_revision,
            Some(revision)
        );
        while events.try_recv().is_ok() {}
        if final_arrives {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                    text: "Authoritative synthetic final".into(),
                    language: Some("en".into()),
                })
                .await;
        } else {
            client.disconnect().await;
        }
        assert!(client.state.lock().await.pending_preview_revision.is_none());
        assert!(!client.preview_is_current(preview_id));
        late_partial("Late synthetic old response".into());
        server.await.unwrap();
        if final_arrives {
            client
                .wait_for_final_translations(Duration::from_secs(2))
                .await;
        }
        let mut confirmed = 0;
        while let Ok(event) = events.try_recv() {
            match event {
                LiveTranslateServerEvent::SubtitlePreviewPair { .. } => {
                    panic!("cancelled preview must not publish a late pair")
                }
                LiveTranslateServerEvent::TranslationDraft(text) => assert!(text.is_empty()),
                LiveTranslateServerEvent::SubtitleConfirmedPair {
                    source,
                    translation,
                    ..
                } => {
                    assert!(final_arrives);
                    assert_eq!(source, "Authoritative synthetic final");
                    assert_eq!(translation, "Confirmed synthetic result");
                    confirmed += 1;
                }
                _ => {}
            }
        }
        assert_eq!(confirmed, usize::from(final_arrives));
        assert!(client.state.lock().await.preview_task.is_none());
        client.disconnect().await;
    }
}

#[tokio::test]
async fn openai_compatible_final_and_disconnect_cancel_inflight_and_queued_previews() {
    use tokio::io::AsyncReadExt;
    for final_arrives in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (started, started_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
            assert!(request.contains("In-flight synthetic source"));
            started.send(()).unwrap();
            let mut tail = Vec::new();
            tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut tail))
                .await
                .unwrap()
                .unwrap();
            assert!(
                tail.is_empty(),
                "priority cancellation must close the old HTTP response"
            );
            if final_arrives {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let request = String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
                assert!(request.contains("Authoritative synthetic final"));
                assert!(!request.contains("Queued synthetic candidate"));
                write_synthetic_response(
                        &mut socket,
                        200,
                        r#"{"choices":[{"finish_reason":"stop","message":{"content":"Confirmed synthetic result"}}]}"#,
                    )
                    .await;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .is_err()
            );
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_openai_compatible(
            "synthetic-asr",
            &endpoint,
            "synthetic-translation-key",
            "synthetic-model",
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
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "In-flight synthetic source".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.state.lock().await.draft_revision;
        client
            .start_preview(
                "In-flight synthetic source".into(),
                Some("en".into()),
                revision,
            )
            .await;
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
        let preview_id = client.state.lock().await.preview_task.as_ref().unwrap().id;
        let late_partial = client.preview_partial_handler(preview_id);
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: "Queued synthetic candidate".into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.state.lock().await.draft_revision;
        client
            .start_preview(
                "Queued synthetic candidate".into(),
                Some("en".into()),
                revision,
            )
            .await;
        assert_eq!(
            client.state.lock().await.pending_preview_revision,
            Some(revision)
        );
        while events.try_recv().is_ok() {}
        if final_arrives {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                    text: "Authoritative synthetic final".into(),
                    language: Some("en".into()),
                })
                .await;
        } else {
            client.disconnect().await;
        }
        assert!(client.state.lock().await.pending_preview_revision.is_none());
        assert!(!client.preview_is_current(preview_id));
        late_partial("Late synthetic old response".into());
        server.await.unwrap();
        if final_arrives {
            client
                .wait_for_final_translations(Duration::from_secs(2))
                .await;
        }
        let mut confirmed = 0;
        while let Ok(event) = events.try_recv() {
            match event {
                LiveTranslateServerEvent::SubtitlePreviewPair { .. } => {
                    panic!("cancelled preview must not publish a late pair")
                }
                LiveTranslateServerEvent::TranslationDraft(text) => assert!(text.is_empty()),
                LiveTranslateServerEvent::SubtitleConfirmedPair {
                    source,
                    translation,
                    ..
                } => {
                    assert!(final_arrives);
                    assert_eq!(source, "Authoritative synthetic final");
                    assert_eq!(translation, "Confirmed synthetic result");
                    confirmed += 1;
                }
                _ => {}
            }
        }
        assert_eq!(confirmed, usize::from(final_arrives));
        assert!(client.state.lock().await.preview_task.is_none());
        client.disconnect().await;
    }
}

#[tokio::test]
async fn original_mode_and_cancelled_final_workers_do_not_report_request_latency() {
    for target in [TargetLanguage::Original, TargetLanguage::SimplifiedChinese] {
        let (client, _events) = test_client(target, 20);
        *client.translation_latency.lock().unwrap() = Some(TranslationLatency {
            milliseconds: 150,
            kind: TranslationLatencyKind::Request,
        });
        if target == TargetLanguage::Original {
            assert_eq!(client.translation_latency(), None);
        }
        client.cancel_final_translations().await;
        assert_eq!(client.translation_latency(), None);
    }
}

#[tokio::test]
async fn a_revised_final_clears_the_displayed_preview_before_its_translation_arrives() {
    use crate::core::session::TranslationSessionController;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (release_translation, wait_for_release) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        wait_for_release.await.unwrap();
        let body = r#"{"code":200,"data":"Confirmed translation"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let (sender, mut events) = provider_event_channel();
    let client = HighQualityTranslationClient::new_deeplx(
        "synthetic-asr",
        &format!("http://{address}"),
        "",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender,
    )
    .unwrap();
    let mut controller = TranslationSessionController::default();
    controller.handle(LiveTranslateServerEvent::SourceDraft {
        text: "Provisional recognition".into(),
        language: Some("en".into()),
    });
    controller.handle(LiveTranslateServerEvent::TranslationDraft(
        "Provisional translation".into(),
    ));
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
            text: "Corrected recognition.".into(),
            language: Some("en".into()),
        })
        .await;

    // The HTTP response is gated, so a final or a partial cannot hide a
    // stale draft. The new source must arrive with an empty translation.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = events.recv().await.unwrap();
            assert!(!matches!(
                event,
                LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
            ));
            controller.handle(event);
            if controller.state.subtitles.source.text == "Corrected recognition."
                && controller.state.subtitles.translation.text.is_empty()
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(controller.state.is_translation_pending);
    assert!(controller.state.subtitles.history.is_empty());

    release_translation.send(()).unwrap();
    let final_event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        final_event,
        LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
    ));
    controller.handle(final_event);
    assert_eq!(controller.state.subtitles.history.len(), 1);
    let pair = &controller.state.subtitles.history[0];
    assert_eq!(pair.source, "Corrected recognition.");
    assert_eq!(pair.translation, "Confirmed translation");
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn original_mode_commits_server_final_as_one_atomic_pair() {
    let (client, mut events) = test_client(TargetLanguage::Original, 20);
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
            text: "今日は晴れです。".into(),
            language: Some("ja".into()),
        })
        .await;

    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: None,
            utterance_id: 1,
            source: "今日は晴れです。".into(),
            language: Some("ja".into()),
            translation: "今日は晴れです。".into(),
        })
    );
}

#[tokio::test]
async fn server_final_cancels_timers_and_invalidates_an_inflight_preview() {
    let (client, _events) = test_client(TargetLanguage::SimplifiedChinese, 20);
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "今日は晴れです。".into(),
            language: Some("ja".into()),
        })
        .await;
    let preview_id = client.advance_preview_epoch();
    {
        let mut inner = client.state.lock().await;
        inner.preview_task = Some(TaskSlot {
            id: preview_id,
            handle: tokio::spawn(std::future::pending()),
        });
    }

    assert!(client
        .prepare_server_final("今日は晴れです。", None)
        .await
        .is_some());

    let inner = client.state.lock().await;
    assert!(inner.draft_stability_task.is_none());
    assert!(inner.draft_maximum_wait_task.is_none());
    assert!(inner.preview_task.is_none());
    assert!(!inner.committer.has_pending_text());
    assert_ne!(client.preview_epoch.load(Ordering::SeqCst), preview_id);
}

#[tokio::test]
async fn server_final_discards_late_source_partial_and_completed_previews() {
    let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
    let preview_id = client.advance_preview_epoch();
    let partial = client.preview_partial_handler(preview_id);
    assert!(client
        .prepare_server_final("confirmed", None)
        .await
        .is_some());
    let final_pair = LiveTranslateServerEvent::SubtitleConfirmedPair {
        source_utterance_id: None,
        utterance_id: 0,
        source: "confirmed".into(),
        language: Some("en".into()),
        translation: "已确认".into(),
    };
    client.emit(final_pair.clone());

    client.emit_preview(
        preview_id,
        LiveTranslateServerEvent::SourceDraft {
            text: "obsolete source".into(),
            language: Some("en".into()),
        },
    );
    partial("obsolete partial".into());
    client.emit_preview(
        preview_id,
        LiveTranslateServerEvent::TranslationDraft("obsolete completion".into()),
    );
    client.emit_preview(
        preview_id,
        LiveTranslateServerEvent::SubtitlePreviewPair {
            source_utterance_id: None,
            source: "obsolete source".into(),
            language: Some("en".into()),
            translation: "obsolete completion".into(),
        },
    );
    client.emit_preview(
        preview_id,
        LiveTranslateServerEvent::PreviewTranslationStarted {
            request_id: preview_id,
        },
    );
    client.emit_preview(
        preview_id,
        LiveTranslateServerEvent::PreviewTranslationFinished {
            request_id: preview_id,
        },
    );

    assert_eq!(events.try_recv(), Ok(final_pair));
    assert_eq!(
        events.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    );
}

#[tokio::test]
async fn active_final_defers_preview_until_the_final_lane_is_idle() {
    let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
    {
        let mut inner = client.state.lock().await;
        inner.final_worker = Some(TaskSlot {
            id: 999,
            handle: tokio::spawn(std::future::pending()),
        });
        inner.active_final = Some(FinalRequestKey {
            text: "earlier final".into(),
            boundary: FinalBoundary::ServerFinal,
            utterance_revision: 1,
            source_utterance_id: None,
            content_revision: 0,
        });
    }

    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "まだ話し続けています".into(),
            language: Some("ja".into()),
        })
        .await;

    let pending_revision = {
        let inner = client.state.lock().await;
        assert!(inner.committer.has_pending_text());
        assert!(inner.draft_stability_task.is_none());
        assert!(inner.draft_maximum_wait_task.is_none());
        assert!(inner.preview_task.is_none());
        inner.draft_revision
    };
    assert!(events.try_recv().is_err());

    {
        let mut inner = client.state.lock().await;
        abort_task(&mut inner.final_worker);
        inner.active_final = None;
    }
    client.resume_pending_preview_if_final_lane_idle().await;

    let inner = client.state.lock().await;
    assert_eq!(inner.draft_revision, pending_revision);
    assert!(inner.draft_stability_task.is_some());
    assert!(inner.draft_maximum_wait_task.is_some());
    drop(inner);
    client.reset_draft_state().await;
}

#[tokio::test]
async fn identical_server_final_is_only_deduplicated_without_a_new_draft() {
    let (client, mut events) = test_client(TargetLanguage::Original, 20);
    let final_event = LiveTranslateServerEvent::SourceFinal {
        text: "はい。".into(),
        language: Some("ja".into()),
    };
    client.handle_asr_event(final_event.clone()).await;
    client.handle_asr_event(final_event.clone()).await;
    assert!(matches!(
        events.recv().await,
        Some(LiveTranslateServerEvent::SubtitleConfirmedPair { .. })
    ));
    assert!(events.try_recv().is_err());

    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "はい".into(),
            language: Some("ja".into()),
        })
        .await;
    let _ = events.recv().await;
    let _ = events.recv().await;
    client.handle_asr_event(final_event).await;
    assert!(matches!(
        events.recv().await,
        Some(LiveTranslateServerEvent::SubtitleConfirmedPair { .. })
    ));
}

#[tokio::test]
async fn an_older_final_keeps_an_already_received_new_sentence_candidate() {
    use crate::core::protocols::audio3::Audio3ASRServerEventDecoder;

    let (client, _events) = test_client(TargetLanguage::Original, 20);
    let frame = serde_json::json!({
        "header": {"event": "result-generated"},
        "payload": {"output": {"sentence": {
            "sentence_id": 8, "text": "Synthetic next sentence", "sentence_end": false
        }}}
    });
    client
        .handle_asr_event(
            Audio3ASRServerEventDecoder::decode(&frame.to_string())
                .unwrap()
                .subtitle_event(SourceLanguage::English),
        )
        .await;
    assert!(client
        .prepare_server_final("Synthetic previous sentence", Some(7))
        .await
        .is_some());
    assert_eq!(
        client
            .state
            .lock()
            .await
            .committer
            .preview_latest_draft(false)
            .as_deref(),
        Some("Synthetic next sentence")
    );
    client.disconnect().await;
}

#[tokio::test]
async fn final_queue_has_a_hard_limit_and_reports_overload_once() {
    let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
    {
        let mut inner = client.state.lock().await;
        inner.final_worker = Some(TaskSlot {
            id: 999,
            handle: tokio::spawn(std::future::pending()),
        });
    }

    for revision in 1..=MAX_FINAL_QUEUE_DEPTH as u64 {
        client
            .enqueue_final(
                format!("server final {revision}"),
                Some("ja".into()),
                FinalBoundary::ServerFinal,
                revision,
                None,
            )
            .await;
    }
    client
        .enqueue_final(
            "overflow".into(),
            Some("ja".into()),
            FinalBoundary::ServerFinal,
            99,
            None,
        )
        .await;
    client
        .enqueue_final(
            "second overflow".into(),
            Some("ja".into()),
            FinalBoundary::ServerFinal,
            100,
            None,
        )
        .await;

    let inner = client.state.lock().await;
    assert_eq!(inner.final_queue.len(), MAX_FINAL_QUEUE_DEPTH);
    assert!(inner.pipeline_failed);
    drop(inner);
    assert_eq!(
        events.recv().await,
        Some(LiveTranslateServerEvent::Error {
            code: OVERLOAD_ERROR_CODE.into(),
            message: OVERLOAD_ERROR_MESSAGE.into(),
        })
    );
    assert!(events.try_recv().is_err());
    client.cancel_final_translations().await;
}
