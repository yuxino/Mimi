//! Recognition pipeline regressions.

use super::*;

#[tokio::test]
async fn custom_recognition_completion_contracts_reach_the_public_pipeline_and_controller() {
    use crate::core::lifecycle::session::TranslationSessionController;
    use crate::core::services::network_proxy::{ProxyConfig, ProxyMode};
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    let contract: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    for case in contract["customRecognitionCompletions"].as_array().unwrap() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let configuration = LiveTranslationConfiguration::with_credentials(
            ProviderKind::CustomOpenAIASR,
            ProviderCredentials::CustomSpeech {
                endpoint: format!("ws://{}/v1/realtime", listener.local_addr().unwrap()),
                model: "synthetic-model".into(),
                api_key: "synthetic-only".into(),
            },
            SourceLanguage::English,
            TargetLanguage::Original,
            TranslationMode::Turbo,
        )
        .validated()
        .unwrap();
        let (events, mut receiver) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_custom(&configuration, events).unwrap();
        let network = ProviderNetwork::resolve(&ProxyConfig {
            mode: ProxyMode::Direct,
            url: None,
        })
        .unwrap();
        client.set_stage_networks(network.clone(), network).unwrap();
        let (draft_observed_tx, draft_observed_rx) = oneshot::channel();
        let (completion_sent_tx, completion_sent_rx) = oneshot::channel();
        let draft = case["draft"].as_str().unwrap().to_owned();
        let server_draft = draft.clone();
        let completion = case["completion"].clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let mut setup: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(setup["type"], "session.update");
            setup["type"] = json!("session.updated");
            socket
                .send(Message::Text(setup.to_string().into()))
                .await
                .unwrap();
            loop {
                let message = socket.next().await.unwrap().unwrap();
                if let Message::Text(text) = message {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame["type"] == "input_audio_buffer.commit" {
                        break;
                    }
                }
            }
            for frame in [
                json!({"type":"input_audio_buffer.committed", "item_id":"one"}),
                json!({"type":"conversation.item.input_audio_transcription.delta", "item_id":"one", "delta":server_draft}),
            ] {
                socket
                    .send(Message::Text(frame.to_string().into()))
                    .await
                    .unwrap();
            }
            draft_observed_rx.await.unwrap();
            if !completion.is_null() {
                socket.send(Message::Text(json!({"type":"conversation.item.input_audio_transcription.completed", "item_id":"one", "transcript":completion}).to_string().into())).await.unwrap();
            }
            completion_sent_tx.send(()).unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
        });
        let mut controller = TranslationSessionController::default();
        controller.begin_session();
        controller.did_connect();
        client.connect().await.unwrap();
        let mut pcm: Vec<u8> = 3000_i16
            .to_le_bytes()
            .into_iter()
            .cycle()
            .take(960)
            .collect();
        pcm.extend(vec![0; 30 * 960]);
        client.send_audio(&pcm).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let event = receiver.recv().await.unwrap();
                    let observed = matches!(&event, LiveTranslateServerEvent::SourceUtteranceDraft { text, .. } if text == &draft);
                    controller.handle(event);
                    if observed { break; }
                }
            }).await.unwrap();
        draft_observed_tx.send(()).unwrap();
        completion_sent_rx.await.unwrap();
        controller.begin_stopping();
        client.finish().await;
        while let Ok(event) = receiver.try_recv() {
            controller.handle(event);
        }
        server.await.unwrap();
        let sources = controller
            .state
            .subtitles
            .history
            .iter()
            .map(|pair| pair.source.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            serde_json::to_value(sources).unwrap(),
            case["expected"]["confirmedSources"],
            "{}",
            case["id"]
        );
    }
}

#[tokio::test]
async fn empty_completion_cancels_only_its_preview_and_preserves_confirmed_display() {
    use crate::core::lifecycle::session::TranslationSessionController;
    let (client, mut receiver) = test_client(TargetLanguage::Japanese, 20);
    let mut controller = TranslationSessionController::default();
    controller.handle(LiveTranslateServerEvent::SubtitleConfirmedPair {
        utterance_id: 1,
        source_utterance_id: Some(1),
        source: "Synthetic confirmed source.".into(),
        language: Some("en".into()),
        translation: "Synthetic confirmed translation.".into(),
    });
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: 2,
            text: "Synthetic withdrawn preview.".into(),
            language: Some("en".into()),
        })
        .await;
    while let Ok(event) = receiver.try_recv() {
        controller.handle(event);
    }
    let preview_id = client.advance_preview_epoch();
    client.state.lock().await.preview_task = Some(TaskSlot {
        id: preview_id,
        handle: tokio::spawn(std::future::pending()),
    });
    let late_partial = client.preview_partial_handler(preview_id);
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 2,
            text: " \t ".into(),
            language: Some("en".into()),
        })
        .await;
    assert!(!client.preview_is_current(preview_id));
    assert!(!client.state.lock().await.committer.has_pending_text());
    late_partial("Synthetic stale partial.".into());
    client.emit_preview(
        preview_id,
        LiveTranslateServerEvent::SubtitlePreviewPair {
            source_utterance_id: Some(2),
            source: "Synthetic stale source.".into(),
            language: None,
            translation: "Synthetic stale completion.".into(),
        },
    );
    while let Ok(event) = receiver.try_recv() {
        controller.handle(event);
    }
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert!(controller.state.subtitles.display_pair_final);
    assert_eq!(
        controller
            .state
            .subtitles
            .display_pair
            .as_ref()
            .unwrap()
            .source,
        "Synthetic confirmed source."
    );
    assert!(controller.state.subtitles.preview_pair.is_none());
    assert!(controller.state.subtitles.source.text.is_empty());
    client.flush_pending_draft().await;
    assert!(client.state.lock().await.final_queue.is_empty());
    client.disconnect().await;
}

#[tokio::test]
async fn older_empty_completion_preserves_newer_preview_and_ordered_final_work() {
    let (client, mut receiver) = test_client(TargetLanguage::Japanese, 20);
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: 2,
            text: "Synthetic newer preview.".into(),
            language: Some("en".into()),
        })
        .await;
    let preview_id = client.advance_preview_epoch();
    let request = TranslationRequest {
        text: "Synthetic durable source.".into(),
        language: Some("en".into()),
        boundary: FinalBoundary::ServerFinal,
        utterance_revision: 10,
        source_utterance_id: Some(1),
        content_revision: 0,
        enqueued_at: tokio::time::Instant::now(),
    };
    {
        let mut inner = client.state.lock().await;
        inner.preview_task = Some(TaskSlot {
            id: preview_id,
            handle: tokio::spawn(std::future::pending()),
        });
        inner.final_worker = Some(TaskSlot {
            id: 99,
            handle: tokio::spawn(std::future::pending()),
        });
        inner.active_final = Some(request.key());
        inner.final_queue.push_back(request);
    }
    while receiver.try_recv().is_ok() {}
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 1,
            text: String::new(),
            language: Some("en".into()),
        })
        .await;
    let inner = client.state.lock().await;
    assert_eq!(inner.last_source_utterance_id, Some(1));
    assert_eq!(inner.current_source_utterance_id, Some(2));
    assert_eq!(
        inner.committer.preview_latest_draft(false).as_deref(),
        Some("Synthetic newer preview.")
    );
    assert_eq!(inner.preview_task.as_ref().unwrap().id, preview_id);
    assert_eq!(inner.final_worker.as_ref().unwrap().id, 99);
    assert!(inner.active_final.is_some());
    assert_eq!(inner.final_queue.len(), 1);
    drop(inner);
    assert!(client.preview_is_current(preview_id));
    assert_eq!(
        receiver.try_recv().unwrap(),
        LiveTranslateServerEvent::SubtitlePreviewWithdrawn {
            source_utterance_id: 1
        }
    );
    assert!(receiver.try_recv().is_err());
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: 1,
            text: "Synthetic retired replay.".into(),
            language: Some("en".into()),
        })
        .await;
    assert!(receiver.try_recv().is_err());
    client.disconnect().await;
}

#[tokio::test]
async fn empty_completion_does_not_withdraw_another_sources_displayed_preview() {
    use crate::core::lifecycle::session::TranslationSessionController;
    let (client, mut receiver) = test_client(TargetLanguage::Japanese, 20);
    let mut controller = TranslationSessionController::default();
    controller.handle(LiveTranslateServerEvent::SourceUtteranceDraft {
        utterance_id: 1,
        text: "Synthetic active final source.".into(),
        language: Some("en".into()),
    });
    controller.handle(LiveTranslateServerEvent::SubtitlePreviewPair {
        source_utterance_id: Some(1),
        source: "Synthetic active final source.".into(),
        language: Some("en".into()),
        translation: "Synthetic active final preview.".into(),
    });
    // The earlier final owns the display while ASR accumulates a new
    // sentence behind its durable worker. That new draft is not a pair.
    client.state.lock().await.final_worker = Some(TaskSlot {
        id: 99,
        handle: tokio::spawn(std::future::pending()),
    });
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: 2,
            text: "Synthetic pending source.".into(),
            language: Some("en".into()),
        })
        .await;
    while let Ok(event) = receiver.try_recv() {
        controller.handle(event);
    }
    let displayed = controller.state.subtitles.display_pair.clone();
    assert_eq!(
        displayed.as_ref().unwrap().source,
        "Synthetic active final source."
    );
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 2,
            text: String::new(),
            language: Some("en".into()),
        })
        .await;
    while let Ok(event) = receiver.try_recv() {
        controller.handle(event);
    }
    assert_eq!(controller.state.subtitles.display_pair, displayed);
    assert_eq!(
        client.state.lock().await.final_worker.as_ref().unwrap().id,
        99
    );
    client.disconnect().await;
}

#[tokio::test]
async fn older_empty_completion_withdraws_its_cached_preview_without_touching_the_newer_task() {
    use crate::core::lifecycle::session::TranslationSessionController;
    let (client, mut receiver) = test_client(TargetLanguage::Japanese, 20);
    let mut controller = TranslationSessionController::default();
    controller.handle(LiveTranslateServerEvent::SourceUtteranceDraft {
        utterance_id: 1,
        text: "Synthetic old source.".into(),
        language: None,
    });
    controller.handle(LiveTranslateServerEvent::SubtitlePreviewPair {
        source_utterance_id: Some(1),
        source: "Synthetic old source.".into(),
        language: None,
        translation: "Synthetic old translation.".into(),
    });
    client.state.lock().await.final_worker = Some(TaskSlot {
        id: 99,
        handle: tokio::spawn(std::future::pending()),
    });
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: 2,
            text: "Synthetic newer pending draft.".into(),
            language: None,
        })
        .await;
    while let Ok(event) = receiver.try_recv() {
        controller.handle(event);
    }
    let preview_id = client.advance_preview_epoch();
    client.state.lock().await.preview_task = Some(TaskSlot {
        id: preview_id,
        handle: tokio::spawn(std::future::pending()),
    });
    assert!(controller.state.subtitles.preview_pair.is_some());
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 1,
            text: String::new(),
            language: None,
        })
        .await;
    while let Ok(event) = receiver.try_recv() {
        controller.handle(event);
    }
    assert!(controller.state.subtitles.preview_pair.is_none());
    assert!(controller.state.subtitles.display_pair.is_none());
    let inner = client.state.lock().await;
    assert_eq!(inner.current_source_utterance_id, Some(2));
    assert_eq!(
        inner.committer.preview_latest_draft(false).as_deref(),
        Some("Synthetic newer pending draft.")
    );
    assert_eq!(inner.preview_task.as_ref().unwrap().id, preview_id);
    assert_eq!(inner.final_worker.as_ref().unwrap().id, 99);
    drop(inner);
    assert!(client.preview_is_current(preview_id));
    client.disconnect().await;
}

#[tokio::test]
async fn recognition_drafts_show_immediately_on_the_bounded_latest_value_lane() {
    let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 100);
    for text in ["hi", "hello", " ... ", "  "] {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                text: text.into(),
                language: Some("en".into()),
            })
            .await;
    }
    assert_eq!(
        events.try_recv(),
        Ok(LiveTranslateServerEvent::SourceDraft {
            text: "hello".into(),
            language: Some("en".into()),
        })
    );
    assert_eq!(
        events.try_recv(),
        Ok(LiveTranslateServerEvent::TranslationDraft(String::new()))
    );
    assert!(events.try_recv().is_err());
    client.reset_draft_state().await;
}

#[tokio::test]
async fn same_text_finish_tail_keeps_its_own_source_identity_in_active_and_queued_lanes() {
    for (earlier_id, tail_id) in [(Some(7), Some(8)), (None, None)] {
        for active in [false, true] {
            let (client, _events) = test_client(TargetLanguage::SimplifiedChinese, 20);
            let earlier = TranslationRequest {
                text: "Synthetic repeated sentence".into(),
                language: Some("en".into()),
                boundary: FinalBoundary::ServerFinal,
                utterance_revision: 1,
                source_utterance_id: earlier_id,
                content_revision: 0,
                enqueued_at: tokio::time::Instant::now(),
            };
            {
                let mut inner = client.state.lock().await;
                inner.final_worker = Some(TaskSlot {
                    id: 999,
                    handle: tokio::spawn(std::future::pending()),
                });
                if active {
                    inner.active_final = Some(earlier.key());
                } else {
                    inner.final_queue.push_back(earlier);
                }
                inner.committer.update_draft("Synthetic repeated sentence");
                inner.current_source_utterance_id = tail_id;
                inner.latest_draft_language = Some("en".into());
                inner.next_confirmation_id = 1;
            }
            client.flush_pending_draft().await;
            let inner = client.state.lock().await;
            let identities: Vec<_> = inner
                .final_queue
                .iter()
                .map(|request| {
                    (
                        request.source_utterance_id,
                        request.utterance_revision,
                        request.boundary,
                    )
                })
                .collect();
            let expected = if active {
                vec![(tail_id, 2, FinalBoundary::SessionFinish)]
            } else {
                vec![
                    (earlier_id, 1, FinalBoundary::ServerFinal),
                    (tail_id, 2, FinalBoundary::SessionFinish),
                ]
            };
            assert_eq!(identities, expected);
            drop(inner);
            client.disconnect().await;
        }
    }
}

#[test]
fn final_request_alias_requires_matching_source_identity_and_content_revision() {
    let request = |boundary, confirmation, source, revision| TranslationRequest {
        text: "Synthetic repeated sentence".into(),
        language: Some("en".into()),
        boundary,
        utterance_revision: confirmation,
        source_utterance_id: source,
        content_revision: revision,
        enqueued_at: tokio::time::Instant::now(),
    };
    let server = request(FinalBoundary::ServerFinal, 1, Some(7), 0);
    assert!(server
        .key()
        .matches(&request(FinalBoundary::SessionFinish, 2, Some(7), 0)));
    assert!(!server
        .key()
        .matches(&request(FinalBoundary::SessionFinish, 2, Some(8), 0)));
    assert!(!server
        .key()
        .matches(&request(FinalBoundary::SessionFinish, 2, None, 0)));
    assert!(!server
        .key()
        .matches(&request(FinalBoundary::SessionFinish, 2, Some(7), 1)));
    assert!(!server
        .key()
        .matches(&request(FinalBoundary::ServerFinal, 2, Some(8), 0)));
    let unknown = request(FinalBoundary::ServerFinal, 1, None, 0);
    assert!(!unknown
        .key()
        .matches(&request(FinalBoundary::SessionFinish, 2, None, 0)));
    assert!(unknown
        .key()
        .matches(&request(FinalBoundary::ServerFinal, 1, None, 0)));
}

#[tokio::test]
async fn repeated_accepted_finals_keep_identity_through_coalesced_drafts_and_duplicate_replays() {
    // Exercise both direct original confirmation and the serial final
    // worker's same-language branch, without issuing a provider request.
    for target in [TargetLanguage::Original, TargetLanguage::Japanese] {
        let (client, mut events) = test_client(target, 20);
        for _ in 0..2 {
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
                    text: "Synthetic repeated lyric".into(),
                    language: Some("ja".into()),
                })
                .await;
            client
                .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                    text: "Synthetic repeated lyric".into(),
                    language: Some("ja".into()),
                })
                .await;
        }
        client
            .wait_for_final_translations(Duration::from_secs(1))
            .await;
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        controller.archive_mut().begin(true, 0);
        let mut ids = Vec::new();
        let mut replay = None;
        while let Ok(event) = events.try_recv() {
            let LiveTranslateServerEvent::SubtitleConfirmedPair { utterance_id, .. } = &event
            else {
                panic!("the reliable final barrier must have coalesced every raw draft: {event:?}");
            };
            ids.push(*utterance_id);
            replay = Some(event.clone());
            controller.handle(event);
        }
        assert_eq!(ids, [1, 2]);
        assert_eq!(controller.state.subtitles.history.len(), 2);
        assert_eq!(controller.archive().count(), 2);
        assert!(
            controller.state.subtitles.history[0].created_at_ms
                < controller.state.subtitles.history[1].created_at_ms
        );
        client.emit(replay.unwrap());
        controller.handle(events.try_recv().unwrap());
        assert_eq!(controller.state.subtitles.history.len(), 2);
        assert_eq!(controller.archive().count(), 2);
        assert!(events.try_recv().is_err());
        client.disconnect().await;
    }
}

#[tokio::test]
async fn audio3_sentence_ids_keep_repeated_finals_when_both_draft_lanes_are_coalesced() {
    use crate::core::protocols::recognition::audio3::Audio3ASRServerEventDecoder;

    for target in [TargetLanguage::Original, TargetLanguage::Japanese] {
        let (client, mut confirmed_events) = test_client(target, 20);
        let (source_events, mut source_receiver) = provider_event_channel();
        // Queue everything before either consumer runs. The reliable source
        // finals suppress every draft, including the only new-sentence hint.
        for sentence_id in [1, 2, 2] {
            for is_final in [false, true] {
                let message = serde_json::json!({
                    "header": {"event": "result-generated"},
                    "payload": {"output": {"sentence": {
                        "text": "Synthetic repeated lyric",
                        "sentence_id": sentence_id,
                        "sentence_end": is_final
                    }}}
                });
                let event = Audio3ASRServerEventDecoder::decode(&message.to_string())
                    .unwrap()
                    .subtitle_event(SourceLanguage::Japanese);
                source_events.send(event).unwrap();
            }
        }
        let mut received_source_ids = Vec::new();
        while let Ok(event) = source_receiver.try_recv() {
            let LiveTranslateServerEvent::SourceUtteranceFinal { utterance_id, .. } = &event else {
                panic!("a reliable source boundary must supersede the draft: {event:?}");
            };
            received_source_ids.push(*utterance_id);
            client.handle_asr_event(event).await;
        }
        assert_eq!(received_source_ids, [1, 2, 2]);
        client
            .wait_for_final_translations(Duration::from_secs(1))
            .await;
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        controller.archive_mut().begin(true, 0);
        let mut received_confirmation_ids = Vec::new();
        let mut confirmed_source_ids = Vec::new();
        while let Ok(event) = confirmed_events.try_recv() {
            let LiveTranslateServerEvent::SubtitleConfirmedPair {
                utterance_id,
                source_utterance_id,
                ..
            } = &event
            else {
                panic!("expected only completed confirmations: {event:?}");
            };
            received_confirmation_ids.push(*utterance_id);
            confirmed_source_ids.push(*source_utterance_id);
            controller.handle(event);
        }
        assert_eq!(received_confirmation_ids, [1, 2]);
        assert_eq!(confirmed_source_ids, [Some(1), Some(2)]);
        assert_eq!(controller.state.subtitles.history.len(), 2);
        assert_eq!(controller.archive().count(), 2);
        client.disconnect().await;
    }
}

#[tokio::test]
async fn replayed_source_identity_cannot_cancel_new_draft_and_resets_per_task() {
    let (client, _events) = test_client(TargetLanguage::SimplifiedChinese, 20);
    assert_eq!(
        client
            .prepare_server_final("Synthetic final", Some(7))
            .await,
        Some(1)
    );
    let epoch = client.advance_preview_epoch();
    {
        let mut inner = client.state.lock().await;
        inner.committer.update_draft("Synthetic next sentence");
        inner.latest_draft_language = Some("en".into());
        inner.preview_task = Some(TaskSlot {
            id: epoch,
            handle: tokio::spawn(std::future::pending()),
        });
    }
    for id in [7, 6] {
        assert!(client
            .prepare_server_final("Synthetic final", Some(id))
            .await
            .is_none());
        let inner = client.state.lock().await;
        assert!(inner.committer.has_pending_text());
        assert_eq!(inner.draft_revision, 1);
        assert_eq!(inner.latest_draft_language.as_deref(), Some("en"));
        assert_eq!(inner.preview_task.as_ref().map(|task| task.id), Some(epoch));
        assert_eq!(client.preview_epoch.load(Ordering::SeqCst), epoch);
    }
    client.reset_draft_state().await;
    assert_eq!(
        client
            .prepare_server_final("Synthetic final", Some(1))
            .await,
        Some(1)
    );
    client.disconnect().await;
}

#[tokio::test]
async fn identified_drafts_cannot_resurrect_a_confirmed_sentence_or_replace_a_newer_one() {
    use crate::core::protocols::recognition::audio3::Audio3ASRServerEventDecoder;

    let draft = |id, text| {
        let frame = serde_json::json!({
            "header": {"event": "result-generated"},
            "payload": {"output": {"sentence": {
                "sentence_id": id, "text": text, "sentence_end": false
            }}}
        });
        Audio3ASRServerEventDecoder::decode(&frame.to_string())
            .unwrap()
            .subtitle_event(SourceLanguage::English)
    };
    let (client, _events) = test_client(TargetLanguage::Original, 20);
    client
        .prepare_server_final("Synthetic repeated lyric", Some(7))
        .await;
    client
        .handle_asr_event(draft(7, "Synthetic repeated lyric"))
        .await;
    assert!(!client.state.lock().await.committer.has_pending_text());

    client
        .handle_asr_event(draft(8, "Synthetic repeated lyric"))
        .await;
    assert_eq!(
        client
            .state
            .lock()
            .await
            .committer
            .preview_latest_draft(false)
            .as_deref(),
        Some("Synthetic repeated lyric")
    );
    client
        .handle_asr_event(draft(7, "Synthetic stale revision"))
        .await;
    assert_eq!(
        client
            .state
            .lock()
            .await
            .committer
            .preview_latest_draft(false)
            .as_deref(),
        Some("Synthetic repeated lyric")
    );
    client.disconnect().await;
}

#[tokio::test]
async fn a_real_new_begin_cancels_only_the_old_preview_and_keeps_quota_and_final_order() {
    use crate::core::protocols::recognition::audio3::Audio3ASRServerEventDecoder;

    let (client, mut events) = test_client(TargetLanguage::Japanese, 20);
    client.prepare_identified_draft(7).await;
    while events.try_recv().is_ok() {}
    let preview_id = client.advance_preview_epoch();
    let late_partial = client.preview_partial_handler(preview_id);
    let now = std::time::Instant::now();
    let cooldown_until = tokio::time::Instant::now() + Duration::from_secs(8);
    let (shared_at, preview_at) = {
        let mut inner = client.state.lock().await;
        inner.committer.update_draft("Synthetic old draft");
        inner.preview_task = Some(TaskSlot {
            id: preview_id,
            handle: tokio::spawn(std::future::pending()),
        });
        inner.preview_http_pending = Some(preview_id);
        inner.pending_preview_revision = Some(inner.draft_revision);
        inner.mt_failure_streak = 2;
        inner.mt_cooldown = Some(MTCooldown {
            until: cooldown_until,
            reason: TranslationRecoveryReason::RateLimited,
        });
        inner.preview_request_pacer.record_start(now);
        inner.preview_request_pacer.suppress_after_rate_limit(now);
        for id in [5, 6] {
            inner.final_queue.push_back(TranslationRequest {
                text: "Synthetic durable source".into(),
                language: Some("en".into()),
                boundary: FinalBoundary::ServerFinal,
                utterance_revision: id,
                source_utterance_id: Some(id),
                content_revision: client.content_revision(),
                enqueued_at: tokio::time::Instant::now(),
            });
        }
        (
            inner.preview_request_pacer.next_shared_start_at(now),
            inner.preview_request_pacer.next_start_at(now),
        )
    };
    let frame = serde_json::json!({
        "header": {"event": "result-generated"},
        "payload": {"output": {"sentence": {
            "sentence_id": 8, "text": "", "sentence_begin": true, "sentence_end": false
        }}}
    });
    client
        .handle_asr_event(
            Audio3ASRServerEventDecoder::decode(&frame.to_string())
                .unwrap()
                .subtitle_event(SourceLanguage::English),
        )
        .await;
    late_partial("Synthetic obsolete callback".into());
    assert_eq!(
        events.try_recv().unwrap(),
        LiveTranslateServerEvent::PreviewTranslationFinished {
            request_id: preview_id
        }
    );
    assert_eq!(
        events.try_recv().unwrap(),
        LiveTranslateServerEvent::SubtitlePreviewCleared
    );
    assert!(
        matches!(events.try_recv().unwrap(), LiveTranslateServerEvent::SourceUtteranceDraft { utterance_id: 8, text, .. } if text.is_empty())
    );
    assert!(events.try_recv().is_err());
    let inner = client.state.lock().await;
    assert_eq!(inner.current_source_utterance_id, Some(8));
    assert!(inner.preview_task.is_none());
    assert!(inner.pending_preview_revision.is_none());
    assert!(!inner.committer.has_pending_text());
    assert_eq!(inner.mt_failure_streak, 2);
    assert_eq!(inner.mt_cooldown.unwrap().until, cooldown_until);
    assert_eq!(
        inner.preview_request_pacer.next_shared_start_at(now),
        shared_at
    );
    assert_eq!(inner.preview_request_pacer.next_start_at(now), preview_at);
    assert_eq!(
        inner
            .final_queue
            .iter()
            .map(|request| request.source_utterance_id)
            .collect::<Vec<_>>(),
        [Some(5), Some(6)]
    );
    drop(inner);
    client.disconnect().await;
}

#[tokio::test]
async fn same_identified_sentence_revisions_keep_the_active_preview_owner() {
    let (client, mut events) = test_client(TargetLanguage::SimplifiedChinese, 20);
    client.prepare_identified_draft(8).await;
    while events.try_recv().is_ok() {}
    let preview_id = client.advance_preview_epoch();
    {
        let mut inner = client.state.lock().await;
        inner.preview_task = Some(TaskSlot {
            id: preview_id,
            handle: tokio::spawn(std::future::pending()),
        });
        inner.preview_http_pending = Some(preview_id);
    }
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: 8,
            text: "Synthetic revised sentence".into(),
            language: Some("en".into()),
        })
        .await;
    assert_eq!(client.preview_epoch.load(Ordering::SeqCst), preview_id);
    assert_eq!(
        client.state.lock().await.preview_task.as_ref().unwrap().id,
        preview_id
    );
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(
            event,
            LiveTranslateServerEvent::SubtitlePreviewCleared
        ));
    }
    client.disconnect().await;
}

#[tokio::test]
async fn starting_an_older_final_does_not_replace_a_newer_source_still_queued_in_the_outer_lane() {
    let (client, mut events) = test_client(TargetLanguage::Japanese, 20);
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: 8,
            text: "Synthetic next source".into(),
            language: Some("ja".into()),
        })
        .await;
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 7,
            text: "Synthetic previous source".into(),
            language: Some("ja".into()),
        })
        .await;
    client
        .wait_for_final_translations(Duration::from_secs(1))
        .await;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    while let Ok(event) = events.try_recv() {
        controller.handle(event);
    }
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert_eq!(
        controller.state.subtitles.history[0].source,
        "Synthetic previous source"
    );
    assert_eq!(
        controller.state.subtitles.source.text,
        "Synthetic next source"
    );
    assert!(!controller.state.subtitles.source.is_final);
    assert_eq!(
        client.state.lock().await.current_source_utterance_id,
        Some(8)
    );
    assert_eq!(
        client
            .state
            .lock()
            .await
            .committer
            .preview_latest_draft(false)
            .as_deref(),
        Some("Synthetic next source")
    );
    client.disconnect().await;
}

#[tokio::test]
async fn oversized_source_cannot_claim_identity_replace_a_candidate_or_bypass_into_finals() {
    let oversized = "x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1);
    for target in [TargetLanguage::Original, TargetLanguage::Japanese] {
        let (client, mut events) = test_client(target, 20);
        {
            let mut inner = client.state.lock().await;
            inner.committer.update_draft("Synthetic prior candidate");
            inner.draft_revision = 5;
            inner.last_source_utterance_id = Some(7);
        }
        for event in [
            LiveTranslateServerEvent::SourceDraft {
                text: oversized.clone(),
                language: Some("ja".into()),
            },
            LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 8,
                text: oversized.clone(),
                language: Some("ja".into()),
            },
        ] {
            client.handle_asr_event(event).await;
            assert_eq!(
                events.try_recv(),
                Ok(LiveTranslateServerEvent::text_limit_error())
            );
        }
        client
            .start_preview(oversized.clone(), Some("ja".into()), 5)
            .await;
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::text_limit_error())
        );
        client
            .enqueue_final(
                oversized.clone(),
                Some("ja".into()),
                FinalBoundary::ServerFinal,
                5,
                None,
            )
            .await;
        assert_eq!(
            events.try_recv(),
            Ok(LiveTranslateServerEvent::text_limit_error())
        );
        assert!(client
            .prepare_server_final(&oversized, Some(8))
            .await
            .is_none());
        let inner = client.state.lock().await;
        assert_eq!(inner.draft_revision, 5);
        assert_eq!(inner.last_source_utterance_id, Some(7));
        assert_eq!(
            inner.committer.preview_latest_draft(false).as_deref(),
            Some("Synthetic prior candidate")
        );
        assert!(inner.preview_task.is_none());
        assert!(inner.final_worker.is_none());
        assert!(inner.final_queue.is_empty());
        assert!(events.try_recv().is_err());
        drop(inner);
        client.disconnect().await;
    }
}

#[test]
fn recognition_bridge_preserves_nonretryable_authentication_and_configuration_categories() {
    use crate::clients::recognition::custom_speech_client::CustomSpeechClientError;
    use crate::clients::translation::translation_client::ConnectError;
    for (error, code) in [
        (
            CustomSpeechClientError::AuthenticationFailed,
            "credential_authentication_failed",
        ),
        (
            CustomSpeechClientError::MissingAPIKey,
            "credential_authentication_failed",
        ),
        (
            CustomSpeechClientError::InvalidEndpoint,
            "invalid_configuration",
        ),
        (
            CustomSpeechClientError::InvalidModel,
            "invalid_configuration",
        ),
        (CustomSpeechClientError::Unreachable, "transport_error"),
        (CustomSpeechClientError::SetupTimeout, "transport_error"),
    ] {
        let error = super::recognition_error(super::RecognitionClientError::Custom(error));
        assert_eq!(ConnectError::MT(error).safe_code(), code);
    }
}
