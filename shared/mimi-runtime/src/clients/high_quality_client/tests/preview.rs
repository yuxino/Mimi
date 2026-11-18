//! Preview pipeline regressions.

use super::*;

#[tokio::test]
async fn fast_drafts_complete_inflight_preview_then_only_the_latest_successor() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (first_started, first_request) = oneshot::channel();
    let (release_first, release_first_rx) = oneshot::channel();
    let (second_started, second_request) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_synthetic_request(&mut socket).await;
        assert!(String::from_utf8(request)
            .unwrap()
            .contains("First preview"));
        first_started.send(()).unwrap();
        release_first_rx.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "latest queued previews must not run beside the in-flight HTTP request"
        );
        let body = r#"{"code":200,"data":"First translation"}"#;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        drop(socket);
        let (mut socket, _) = listener.accept().await.unwrap();
        second_started.send(std::time::Instant::now()).unwrap();
        let request = read_synthetic_request(&mut socket).await;
        let request = String::from_utf8(request).unwrap();
        assert!(request.contains("Newest draft before its timer"));
        assert!(!request.contains("Intermediate replacement"));
        assert!(!request.contains("Latest replacement"));
        tokio::time::sleep(Duration::from_millis(20)).await;
        let body = r#"{"code":200,"data":"Latest translation"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
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
            text: "First preview".into(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview("First preview".into(), Some("en".into()), revision)
        .await;
    tokio::time::timeout(Duration::from_secs(1), first_request)
        .await
        .unwrap()
        .unwrap();
    let allowed_at = client
        .state
        .lock()
        .await
        .preview_request_pacer
        .next_start_at(std::time::Instant::now());
    let preview_id = client.state.lock().await.preview_task.as_ref().unwrap().id;
    for text in ["Intermediate replacement", "Latest replacement"] {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: text.into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.state.lock().await.draft_revision;
        client
            .start_preview(text.into(), Some("en".into()), revision)
            .await;
        let inner = client.state.lock().await;
        assert_eq!(inner.preview_task.as_ref().unwrap().id, preview_id);
        assert_eq!(inner.preview_http_pending, Some(preview_id));
        assert_eq!(inner.pending_preview_revision, Some(revision));
        drop(inner);
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    // This draft has not reached either timer: completion must read the
    // committer's newest bounded text, not the older queued candidate.
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "Newest draft before its timer".into(),
            language: Some("en".into()),
        })
        .await;
    assert!(client.preview_is_current(preview_id));
    release_first.send(()).unwrap();
    let mut pairs = Vec::new();
    tokio::time::timeout(Duration::from_secs(6), async {
        while pairs.len() < 2 {
            if let LiveTranslateServerEvent::SubtitlePreviewPair {
                source,
                translation,
                ..
            } = events.recv().await.unwrap()
            {
                pairs.push((source, translation));
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        pairs,
        [
            ("First preview".into(), "First translation".into()),
            (
                "Newest draft before its timer".into(),
                "Latest translation".into()
            ),
        ]
    );
    assert!(
        second_request.await.unwrap() >= allowed_at,
        "the completed HTTP request must retain its exact pacing boundary"
    );
    assert!(
        client.translation_latency().unwrap().milliseconds < 300,
        "preview spacing is excluded from the actual request measurement"
    );
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn waiting_preview_replacements_send_only_the_latest_candidate() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
        assert!(request.contains("Latest waiting candidate"));
        assert!(!request.contains("First waiting candidate"));
        assert!(!request.contains("Intermediate waiting candidate"));
        write_synthetic_response(
            &mut socket,
            200,
            r#"{"code":200,"data":"Latest waiting result"}"#,
        )
        .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
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
    let mut previous_id = None;
    for text in [
        "First waiting candidate",
        "Intermediate waiting candidate",
        "Latest waiting candidate",
    ] {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: text.into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.state.lock().await.draft_revision;
        client
            .start_preview(text.into(), Some("en".into()), revision)
            .await;
        let inner = client.state.lock().await;
        let id = inner.preview_task.as_ref().unwrap().id;
        assert_ne!(Some(id), previous_id);
        assert!(inner.preview_http_pending.is_none());
        assert!(inner.pending_preview_revision.is_none());
        previous_id = Some(id);
        drop(inner);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let LiveTranslateServerEvent::SubtitlePreviewPair {
                source,
                translation,
                ..
            } = events.recv().await.unwrap()
            {
                assert_eq!(source, "Latest waiting candidate");
                assert_eq!(translation, "Latest waiting result");
                break;
            }
        }
    })
    .await
    .unwrap();
    server.await.unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn returning_to_completed_candidate_cancels_a_different_waiting_preview() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (cancelled, cancelled_rx) = oneshot::channel::<std::time::Instant>();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        reply_to_synthetic_request(
            &mut socket,
            200,
            r#"{"code":200,"data":"Synthetic translation"}"#,
        )
        .await;
        let allowed_at = cancelled_rx.await.unwrap();
        let wait = allowed_at.saturating_duration_since(std::time::Instant::now())
            + Duration::from_millis(100);
        assert!(
            tokio::time::timeout(wait, listener.accept()).await.is_err(),
            "a reverted waiting candidate must never reach HTTP"
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
    let original = "Synthetic original preview";
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: original.into(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview(original.into(), Some("en".into()), revision)
        .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                events.recv().await.unwrap(),
                LiveTranslateServerEvent::TranslationDraft(text)
                    if text == "Synthetic translation"
            ) {
                break;
            }
        }
        while client.state.lock().await.preview_task.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let replacement = "Synthetic original preview with substantially more new words";
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: replacement.into(),
            language: Some("en".into()),
        })
        .await;
    while events.try_recv().is_ok() {}
    let (revision, allowed_at) = {
        let inner = client.state.lock().await;
        (
            inner.draft_revision,
            inner
                .preview_request_pacer
                .next_candidate_start_at(
                    std::time::Instant::now(),
                    PreviewCandidate::new(replacement, Some("en"), 0),
                )
                .unwrap(),
        )
    };
    client
        .start_preview(replacement.into(), Some("en".into()), revision)
        .await;
    tokio::task::yield_now().await;
    let waiting_id = client.state.lock().await.preview_task.as_ref().unwrap().id;
    let late_partial = client.preview_partial_handler(waiting_id);

    let cosmetic = format!("{replacement}!");
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: cosmetic.clone(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview(cosmetic, Some("en".into()), revision)
        .await;
    assert_eq!(
        client.state.lock().await.preview_task.as_ref().unwrap().id,
        waiting_id,
        "same-candidate updates must preserve a waiting owner"
    );

    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: original.into(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview(original.into(), Some("en".into()), revision)
        .await;
    {
        let inner = client.state.lock().await;
        assert!(inner.preview_task.is_none());
        assert!(inner.preview_candidate.is_none());
        assert!(!client.preview_is_current(waiting_id));
    }
    assert_eq!(
        events.try_recv(),
        Ok(LiveTranslateServerEvent::SourceDraft {
            text: original.into(),
            language: Some("en".into()),
        })
    );
    assert_eq!(
        events.try_recv(),
        Ok(LiveTranslateServerEvent::TranslationDraft(String::new()))
    );
    late_partial("Late synthetic replacement".into());
    assert!(events.try_recv().is_err());
    cancelled.send(allowed_at).unwrap();
    server.await.unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn cosmetic_drafts_preserve_an_identical_inflight_http_preview() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (started, started_rx) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = read_synthetic_request(&mut socket).await;
        started.send(()).unwrap();
        release_rx.await.unwrap();
        let body = r#"{"code":200,"data":"Synthetic translation"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
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
    let original = "Synthetic inflight preview";
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: original.into(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview(original.into(), Some("en".into()), revision)
        .await;
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .unwrap()
        .unwrap();
    let preview_id = client.state.lock().await.preview_task.as_ref().unwrap().id;
    let mut controller = crate::core::session::TranslationSessionController::default();
    while let Ok(event) = events.try_recv() {
        controller.handle(event);
    }
    assert!(controller.state.is_translation_preview_pending);
    assert!(!controller.state.is_translation_pending);
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "Synthetic changed candidate while HTTP remains active".into(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview(
            "Synthetic changed candidate while HTTP remains active".into(),
            Some("en".into()),
            revision,
        )
        .await;
    assert_eq!(
        client.state.lock().await.pending_preview_revision,
        Some(revision)
    );
    for text in ["Synthetic inflight preview", "Synthetic, inflight preview!"] {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: text.into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.state.lock().await.draft_revision;
        client
            .start_preview(text.into(), Some("en".into()), revision)
            .await;
        assert_eq!(
            client.state.lock().await.preview_task.as_ref().unwrap().id,
            preview_id
        );
        assert!(client.preview_is_current(preview_id));
        assert!(client.state.lock().await.pending_preview_revision.is_none());
        assert!(events.try_recv().is_err());
    }
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = events.recv().await.unwrap();
            let finished = matches!(
                event,
                LiveTranslateServerEvent::PreviewTranslationFinished { request_id }
                    if request_id == preview_id
            );
            controller.handle(event);
            if finished {
                break;
            }
        }
    })
    .await
    .unwrap();
    while let Ok(event) = events.try_recv() {
        controller.handle(event);
    }
    assert_eq!(
        controller.state.subtitles.translation.text,
        "Synthetic translation"
    );
    assert_eq!(
        controller.state.subtitles.preview_pair,
        Some(crate::core::models::PreviewSubtitlePair {
            utterance_id: None,
            source: original.into(),
            translation: "Synthetic translation".into(),
        })
    );
    assert!(!controller.state.is_translation_preview_pending);
    assert!(!controller.state.is_translation_pending);
    assert!(controller.state.subtitles.history.is_empty());
    server.await.unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn completed_preview_survives_duplicate_and_cosmetic_drafts_without_another_request() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (checked, checked_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        reply_to_synthetic_request(
            &mut socket,
            200,
            r#"{"code":200,"data":"Synthetic translation"}"#,
        )
        .await;
        checked_rx.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "duplicate and cosmetic drafts must not request translation"
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
            text: "Synthetic preview".into(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview("Synthetic preview".into(), Some("en".into()), revision)
        .await;

    let mut controller = crate::core::session::TranslationSessionController::default();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = events.recv().await.unwrap();
            let completed = matches!(
                &event,
                LiveTranslateServerEvent::TranslationDraft(text)
                    if text == "Synthetic translation"
            );
            controller.handle(event);
            if completed {
                break;
            }
        }
        while client.state.lock().await.preview_task.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    while let Ok(event) = events.try_recv() {
        controller.handle(event);
    }
    assert!(!controller.state.is_translation_preview_pending);
    assert_eq!(controller.state.subtitles.source.text, "Synthetic preview");
    assert_eq!(
        controller.state.subtitles.translation.text,
        "Synthetic translation"
    );

    for text in [
        "Synthetic preview",
        "Synthetic, preview.",
        "Synthetic   preview!",
    ] {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: text.into(),
                language: Some("en".into()),
            })
            .await;
        for kind in [DraftTimerKind::Stable, DraftTimerKind::Maximum] {
            let (timer_id, revision) = {
                let inner = client.state.lock().await;
                let slot = match kind {
                    DraftTimerKind::Stable => &inner.draft_stability_task,
                    DraftTimerKind::Maximum => &inner.draft_maximum_wait_task,
                };
                (slot.as_ref().unwrap().id, inner.draft_revision)
            };
            client.handle_draft_timer(kind, timer_id, revision).await;
            assert!(client.state.lock().await.preview_task.is_none());
            assert!(
                events.try_recv().is_err(),
                "a skipped preview must not clear its displayed translation"
            );
        }
    }

    // Actual new words reach raw recognition immediately. The last
    // complete pair remains independently readable while MT waits.
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "Synthetic preview grows".into(),
            language: Some("en".into()),
        })
        .await;
    controller.handle(events.try_recv().unwrap());
    controller.handle(events.try_recv().unwrap());
    assert_eq!(
        controller.state.subtitles.source.text,
        "Synthetic preview grows"
    );
    assert!(controller.state.subtitles.translation.text.is_empty());
    assert_eq!(
        controller.state.subtitles.preview_pair,
        Some(crate::core::models::PreviewSubtitlePair {
            utterance_id: None,
            source: "Synthetic preview".into(),
            translation: "Synthetic translation".into(),
        })
    );
    assert!(events.try_recv().is_err());
    checked.send(()).unwrap();
    server.await.unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn semantic_number_correction_retranslates_the_same_utterances_completed_preview() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for (source, translation) in [
            ("Use 1.5 liters", "Synthetic 1.5 translation"),
            ("Use 15 liters", "Synthetic 15 translation"),
        ] {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(8), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let request = read_synthetic_request(&mut socket).await;
            let header_end = request
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap()
                + 4;
            let body: serde_json::Value = serde_json::from_slice(&request[header_end..]).unwrap();
            assert_eq!(body["text"], source);
            write_synthetic_response(
                &mut socket,
                200,
                &serde_json::json!({"code":200,"data":translation}).to_string(),
            )
            .await;
        }
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
    let mut controller = crate::core::session::TranslationSessionController::default();
    for (index, (source, translation)) in [
        ("Use 1.5 liters", "Synthetic 1.5 translation"),
        ("Use 15 liters", "Synthetic 15 translation"),
    ]
    .into_iter()
    .enumerate()
    {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: 17,
                text: source.into(),
                language: Some("en".into()),
            })
            .await;
        let revision = client.state.lock().await.draft_revision;
        client
            .start_preview(source.into(), Some("en".into()), revision)
            .await;
        let scheduled = client.state.lock().await.preview_task.is_some();
        if !scheduled {
            client.disconnect().await;
            server.abort();
        }
        assert!(
            scheduled,
            "A decimal correction must schedule another preview for the same source identity"
        );
        tokio::time::timeout(Duration::from_secs(8), async {
                loop {
                    let event = events.recv().await.unwrap();
                    let completed = matches!(&event, LiveTranslateServerEvent::SubtitlePreviewPair {
                        source_utterance_id: Some(17), source: actual_source, translation: actual_translation, ..
                    } if actual_source == source && actual_translation == translation);
                    controller.handle(event);
                    if completed { break; }
                }
                while client.state.lock().await.preview_task.is_some() { tokio::task::yield_now().await; }
            }).await.unwrap();
        let pair = controller.state.subtitles.preview_pair.as_ref().unwrap();
        assert_eq!(pair.source, source);
        assert_eq!(pair.translation, translation);
        assert!(
            controller.state.subtitles.history.is_empty(),
            "A corrected preview remains replaceable"
        );
        assert!(!controller.state.subtitles.display_pair_final);
        if index == 0 {
            assert_eq!(controller.state.subtitles.source.text, "Use 1.5 liters");
        }
    }
    client.disconnect().await;
    server.await.unwrap();
}

#[tokio::test]
async fn opening_fragments_wait_for_stability_but_maximum_and_corrections_still_preview() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for (kind, correcting) in [
        (DraftTimerKind::Stable, false),
        (DraftTimerKind::Maximum, false),
        (DraftTimerKind::Stable, true),
        (DraftTimerKind::Maximum, true),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            let body = r#"{"code":200,"data":"Synthetic preview"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let (sender, mut events) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_deeplx(
            "synthetic-asr",
            &format!("http://{address}"),
            "",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender,
        )
        .unwrap();
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        if correcting {
            client
                .state
                .lock()
                .await
                .preview_request_pacer
                .record_candidate_start(
                    std::time::Instant::now() - Duration::from_secs(10),
                    PreviewCandidate::new("Previous synthetic preview", Some("en"), 0),
                );
        }
        for text in ["hi", "hello"] {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: text.into(),
                    language: Some("en".into()),
                })
                .await;
        }
        if !correcting && matches!(kind, DraftTimerKind::Stable) {
            let (timer_id, revision) = {
                let inner = client.state.lock().await;
                let slot = match kind {
                    DraftTimerKind::Stable => &inner.draft_stability_task,
                    DraftTimerKind::Maximum => &inner.draft_maximum_wait_task,
                };
                (slot.as_ref().unwrap().id, inner.draft_revision)
            };
            client.handle_draft_timer(kind, timer_id, revision).await;
            assert!(client.state.lock().await.preview_task.is_none());
            assert_eq!(client.translation_latency(), None);
            while let Ok(event) = events.try_recv() {
                assert!(
                    matches!(event, LiveTranslateServerEvent::SourceDraft { .. })
                        || matches!(event, LiveTranslateServerEvent::TranslationDraft(ref text) if text.is_empty())
                );
            }
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: "hello synthetic sentence".into(),
                    language: Some("en".into()),
                })
                .await;
        }
        let expected_source = if !correcting && matches!(kind, DraftTimerKind::Stable) {
            "hello synthetic sentence"
        } else {
            "hello"
        };
        let (timer_id, revision) = {
            let inner = client.state.lock().await;
            let slot = match kind {
                DraftTimerKind::Stable => &inner.draft_stability_task,
                DraftTimerKind::Maximum => &inner.draft_maximum_wait_task,
            };
            (slot.as_ref().unwrap().id, inner.draft_revision)
        };
        client.handle_draft_timer(kind, timer_id, revision).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match events.recv().await.unwrap() {
                    LiveTranslateServerEvent::SourceDraft { text, .. } => {
                        assert_eq!(text, expected_source);
                    }
                    LiveTranslateServerEvent::TranslationDraft(text) => {
                        if text.is_empty() {
                            continue;
                        }
                        assert_eq!(text, "Synthetic preview");
                        break;
                    }
                    LiveTranslateServerEvent::PreviewTranslationStarted { .. }
                    | LiveTranslateServerEvent::PreviewTranslationFinished { .. } => {}
                    LiveTranslateServerEvent::SubtitlePreviewPair {
                        source,
                        translation,
                        ..
                    } => {
                        assert_eq!(source, expected_source);
                        assert_eq!(translation, "Synthetic preview");
                    }
                    event => panic!("preview must not publish a durable event: {event:?}"),
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            client.translation_latency().unwrap().kind,
            TranslationLatencyKind::Request
        );
        while let Ok(event) = events.try_recv() {
            assert!(matches!(
                event,
                LiveTranslateServerEvent::PreviewTranslationFinished { .. }
            ));
        }
        client.disconnect().await;
        server.await.unwrap();
    }
}

#[tokio::test]
async fn stable_timer_previews_the_whole_tail_and_replaces_only_a_complete_pair() {
    use tokio::io::AsyncWriteExt;
    let draft = "First synthetic sentence. More synthetic words";
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (started, started_rx) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_synthetic_request(&mut socket).await;
        assert!(String::from_utf8(request).unwrap().contains(draft));
        started.send(()).unwrap();
        release_rx.await.unwrap();
        let body = r#"{"code":200,"data":"Complete synthetic translation"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
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
    let previous = crate::core::models::PreviewSubtitlePair {
        utterance_id: None,
        source: "Previous synthetic source".into(),
        translation: "Previous synthetic translation".into(),
    };
    let mut controller = crate::core::session::TranslationSessionController::default();
    controller.handle(LiveTranslateServerEvent::SubtitlePreviewPair {
        source_utterance_id: None,
        source: previous.source.clone(),
        language: Some("en".into()),
        translation: previous.translation.clone(),
    });
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: draft.into(),
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
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .unwrap()
        .unwrap();
    while let Ok(event) = events.try_recv() {
        controller.handle(event);
    }
    assert!(controller.state.is_translation_preview_pending);
    assert!(!controller.state.is_translation_pending);
    assert_eq!(controller.state.subtitles.preview_pair, Some(previous));
    assert_eq!(controller.state.subtitles.source.text, draft);
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = events.recv().await.unwrap();
            let finished = matches!(
                event,
                LiveTranslateServerEvent::PreviewTranslationFinished { .. }
            );
            controller.handle(event);
            if finished {
                break;
            }
        }
    })
    .await
    .unwrap();
    while let Ok(event) = events.try_recv() {
        controller.handle(event);
    }
    assert_eq!(
        controller.state.subtitles.preview_pair,
        Some(crate::core::models::PreviewSubtitlePair {
            utterance_id: None,
            source: draft.into(),
            translation: "Complete synthetic translation".into(),
        })
    );
    assert!(!controller.state.is_translation_preview_pending);
    assert!(!controller.state.is_translation_pending);
    assert!(controller.state.subtitles.history.is_empty());
    server.await.unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn stable_timer_resets_while_maximum_timer_keeps_its_first_token() {
    let (client, _events) = test_client(TargetLanguage::SimplifiedChinese, 100);
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "まだ話しています".into(),
            language: Some("ja".into()),
        })
        .await;
    let (first_stable, first_maximum) = {
        let inner = client.state.lock().await;
        (
            inner.draft_stability_task.as_ref().unwrap().id,
            inner.draft_maximum_wait_task.as_ref().unwrap().id,
        )
    };

    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "まだ話し続けています".into(),
            language: Some("ja".into()),
        })
        .await;
    let (second_stable, second_maximum) = {
        let inner = client.state.lock().await;
        (
            inner.draft_stability_task.as_ref().unwrap().id,
            inner.draft_maximum_wait_task.as_ref().unwrap().id,
        )
    };

    assert_ne!(first_stable, second_stable);
    assert_eq!(first_maximum, second_maximum);
    client.reset_draft_state().await;
}

#[tokio::test]
async fn stable_callback_clears_only_itself_and_preserves_maximum_wait() {
    let (mut client, _events) = test_client(TargetLanguage::SimplifiedChinese, 100);
    // This timer now starts a short-draft preview. Keep its request local
    // and pending until reset cancels it; no provider connection is needed.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    client.mt = Arc::new(TextTranslationClient::DeepLX(
        crate::clients::deeplx_client::DeepLXClient::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "",
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
        )
        .unwrap(),
    ));
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "短い未完の文".into(),
            language: Some("ja".into()),
        })
        .await;
    let (stable_id, maximum_id, revision) = {
        let inner = client.state.lock().await;
        (
            inner.draft_stability_task.as_ref().unwrap().id,
            inner.draft_maximum_wait_task.as_ref().unwrap().id,
            inner.draft_revision,
        )
    };

    client
        .handle_draft_timer(DraftTimerKind::Stable, stable_id, revision)
        .await;
    let inner = client.state.lock().await;
    assert!(inner.draft_stability_task.is_none());
    assert_eq!(
        inner.draft_maximum_wait_task.as_ref().map(|task| task.id),
        Some(maximum_id)
    );
    drop(inner);
    client.reset_draft_state().await;
}

#[tokio::test]
async fn a_running_preview_keeps_its_source_until_replaced_instead_of_mixing_new_text() {
    let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
    let preview_id = client.advance_preview_epoch();
    client.state.lock().await.preview_task = Some(TaskSlot {
        id: preview_id,
        handle: tokio::spawn(std::future::pending()),
    });
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: "new synthetic source".into(),
            language: Some("en".into()),
        })
        .await;
    assert!(events.try_recv().is_err());
    assert!(client.state.lock().await.committer.has_pending_text());
    client.reset_draft_state().await;
}

#[tokio::test]
async fn opening_wait_is_bounded_without_a_final_and_short_finals_do_not_wait() {
    for final_event in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let started = std::time::Instant::now();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let elapsed = started.elapsed();
            let request = String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
            assert!(request.contains("short"));
            write_synthetic_response(
                &mut socket,
                200,
                r#"{"code":200,"data":"Synthetic short result"}"#,
            )
            .await;
            elapsed
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
        client.stable_draft_delay = Duration::from_millis(250);
        client.maximum_wait_delay = Duration::from_secs(1);
        if final_event {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                    text: "short".into(),
                    language: Some("en".into()),
                })
                .await;
        } else {
            for text in ["s", "sh", "sho", "short"] {
                client
                    .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                        text: text.into(),
                        language: Some("en".into()),
                    })
                    .await;
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        }
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                let event = events.recv().await.unwrap();
                match event {
                    LiveTranslateServerEvent::SubtitleConfirmedPair {
                        source,
                        translation,
                        ..
                    } => {
                        assert!(final_event);
                        assert_eq!(source, "short");
                        assert_eq!(translation, "Synthetic short result");
                        break;
                    }
                    LiveTranslateServerEvent::SubtitlePreviewPair {
                        source,
                        translation,
                        ..
                    } => {
                        assert!(!final_event);
                        assert_eq!(source, "short");
                        assert_eq!(translation, "Synthetic short result");
                        break;
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        let elapsed = server.await.unwrap();
        if final_event {
            assert!(elapsed < Duration::from_secs(1));
        } else {
            assert!(elapsed >= Duration::from_millis(1950) && elapsed < Duration::from_secs(3));
        }
        client.disconnect().await;
    }
}
