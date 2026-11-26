use super::*;

fn finish_contract_event(value: &serde_json::Value) -> LiveTranslateServerEvent {
    use LiveTranslateServerEvent::*;
    let text = || value["text"].as_str().unwrap().to_owned();
    let id = value["utteranceId"].as_u64();
    match value["kind"].as_str().unwrap() {
        "sourceDraft" => match id {
            Some(utterance_id) => SourceUtteranceDraft {
                utterance_id,
                text: text(),
                language: None,
            },
            None => SourceDraft {
                text: text(),
                language: None,
            },
        },
        "sourceFinal" => match id {
            Some(utterance_id) => SourceUtteranceFinal {
                utterance_id,
                text: text(),
                language: None,
            },
            None => SourceFinal {
                text: text(),
                language: None,
            },
        },
        "sessionFinished" => SessionFinished,
        _ => panic!("invalid synthetic ASR finish event"),
    }
}

#[tokio::test]
async fn recognition_finish_contracts_match_for_async_and_try_receive() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    let cases = fixtures["asrFinishBarrier"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        for asynchronous in [false, true] {
            let (sender, mut receiver) = provider_event_channel();
            receiver.enable_recognition_finish_drain();
            for event in case["events"].as_array().unwrap() {
                if event["kind"] == "clear" {
                    sender.advance_content_revision();
                } else {
                    sender.send(finish_contract_event(event)).unwrap();
                }
            }
            for expected in case["expected"].as_array().unwrap() {
                let actual = if asynchronous {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(1),
                        receiver.recv_with_revision(),
                    )
                    .await
                    .unwrap()
                    .unwrap()
                } else {
                    receiver.try_recv_with_revision().unwrap()
                };
                assert_eq!(
                    actual.event,
                    finish_contract_event(expected),
                    "{} asynchronous={asynchronous}",
                    case["id"]
                );
            }
            assert!(receiver.try_recv_with_revision().is_err(), "{}", case["id"]);
        }
    }
}

#[tokio::test]
async fn recognition_finish_drain_handles_waiting_receive_and_prioritizes_its_saved_finish() {
    let (sender, mut receiver) = provider_event_channel();
    receiver.enable_recognition_finish_drain();
    let draft = LiveTranslateServerEvent::SourceDraft {
        text: "Synthetic final tail.".into(),
        language: None,
    };
    let mut waiting = Box::pin(receiver.recv_with_revision());
    assert!(futures_util::poll!(&mut waiting).is_pending());
    sender.send(draft.clone()).unwrap();
    sender
        .send(LiveTranslateServerEvent::SessionFinished)
        .unwrap();
    assert_eq!(waiting.await.unwrap().event, draft);

    // New reliable work cannot overtake the one terminal already removed
    // from the queue while its source tail was delivered.
    sender
        .send(LiveTranslateServerEvent::SessionUpdated)
        .unwrap();
    assert_eq!(
        receiver.recv_with_revision().await.unwrap().event,
        LiveTranslateServerEvent::SessionFinished
    );
    assert_eq!(
        receiver.try_recv_with_revision().unwrap().event,
        LiveTranslateServerEvent::SessionUpdated
    );
    assert!(receiver.try_recv_with_revision().is_err());
}

#[test]
fn recognition_finish_drain_keeps_other_draft_lanes_and_normal_receivers_unchanged() {
    for (drain, draft) in [
        (
            false,
            LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic source draft.".into(),
                language: None,
            },
        ),
        (
            true,
            LiveTranslateServerEvent::TranslationDraft("Synthetic translation.".into()),
        ),
        (
            true,
            LiveTranslateServerEvent::UtteranceText {
                utterance_id: "synthetic".into(),
                role: UtteranceRole::Source,
                text: "Synthetic stamped draft.".into(),
                is_final: false,
                language: None,
            },
        ),
    ] {
        let (sender, mut receiver) = provider_event_channel();
        if drain {
            receiver.enable_recognition_finish_drain();
        }
        sender.send(draft).unwrap();
        sender
            .send(LiveTranslateServerEvent::SessionFinished)
            .unwrap();
        assert_eq!(
            receiver.try_recv_with_revision().unwrap().event,
            LiveTranslateServerEvent::SessionFinished
        );
        assert!(receiver.try_recv_with_revision().is_err());
    }
}

#[tokio::test]
async fn recognition_finish_drain_keeps_overflow_ahead_of_its_saved_finish() {
    for asynchronous in [false, true] {
        let (sender, mut receiver) = provider_event_channel_with_capacity(1);
        receiver.enable_recognition_finish_drain();
        sender
            .send(LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic final tail.".into(),
                language: None,
            })
            .unwrap();
        sender
            .send(LiveTranslateServerEvent::SessionFinished)
            .unwrap();
        assert!(matches!(
            receiver.try_recv_with_revision().unwrap().event,
            LiveTranslateServerEvent::SourceDraft { .. }
        ));
        sender
            .send(LiveTranslateServerEvent::SessionUpdated)
            .unwrap();
        assert_eq!(
            sender.send(LiveTranslateServerEvent::SessionUpdated),
            Err(ProviderEventSendError::Backpressure)
        );
        let event = if asynchronous {
            receiver.recv_with_revision().await.unwrap()
        } else {
            receiver.try_recv_with_revision().unwrap()
        };
        assert_eq!(event.event, overflow_event());
        assert_eq!(
            receiver.try_recv_with_revision().unwrap().event,
            LiveTranslateServerEvent::SessionFinished
        );
    }
}

#[test]
fn withdrawal_discards_only_its_queued_source_draft_before_delivery() {
    let draft = |id| LiveTranslateServerEvent::SourceUtteranceDraft {
        utterance_id: id,
        text: "Synthetic source draft.".into(),
        language: None,
    };
    let withdrawal = LiveTranslateServerEvent::SubtitlePreviewWithdrawn {
        source_utterance_id: 2,
    };
    let (sender, mut receiver) = provider_event_channel();
    sender.send(draft(2)).unwrap();
    sender.send(withdrawal.clone()).unwrap();
    assert_eq!(receiver.try_recv().unwrap(), withdrawal);
    assert!(receiver.try_recv().is_err());

    let (sender, mut receiver) = provider_event_channel();
    sender.send(draft(3)).unwrap();
    let translation =
        LiveTranslateServerEvent::TranslationDraft("Synthetic older in-flight translation.".into());
    sender.send(translation.clone()).unwrap();
    sender.send(withdrawal.clone()).unwrap();
    assert_eq!(receiver.try_recv().unwrap(), withdrawal);
    assert_eq!(receiver.try_recv().unwrap(), draft(3));
    assert_eq!(receiver.try_recv().unwrap(), translation);
    sender.advance_content_revision();
    sender.send(draft(2)).unwrap();
    assert_eq!(receiver.try_recv().unwrap(), draft(2));
}

#[test]
fn sequential_withdrawals_cannot_resurrect_the_first_queued_draft() {
    let (sender, mut receiver) = provider_event_channel();
    sender
        .send(LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: 1,
            text: "Synthetic withdrawn first draft.".into(),
            language: None,
        })
        .unwrap();
    for id in [1, 2] {
        sender
            .send(LiveTranslateServerEvent::SubtitlePreviewWithdrawn {
                source_utterance_id: id,
            })
            .unwrap();
    }
    for id in [1, 2] {
        assert_eq!(
            receiver.try_recv().unwrap(),
            LiveTranslateServerEvent::SubtitlePreviewWithdrawn {
                source_utterance_id: id
            }
        );
    }
    assert!(receiver.try_recv().is_err());
}

fn identified_preview(id: &str, text: &str) -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair {
        utterance_id: id.into(),
        source: text.into(),
        translation: format!("Synthetic translation {text}"),
        language: Some("en".into()),
    }
}

fn identified_final(id: &str, text: &str) -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
        utterance_id: id.into(),
        source: text.into(),
        translation: format!("Synthetic translation {text}"),
        language: Some("en".into()),
    }
}

#[test]
fn registration_preserves_a_queued_complete_preview_without_a_draft_barrier() {
    let (sender, mut receiver) = provider_event_channel();
    let preview = identified_preview("A", "Synthetic source");
    sender.send(preview.clone()).unwrap();
    let registration = LiveTranslateServerEvent::UtteranceRegistered {
        utterance_id: "A".into(),
    };
    sender.send(registration.clone()).unwrap();
    assert_eq!(receiver.try_recv().unwrap(), registration);
    assert_eq!(receiver.try_recv().unwrap(), preview);
    assert!(receiver.try_recv().is_err());
}

#[test]
fn registered_newer_complete_preview_survives_a_queued_late_final() {
    let (sender, mut receiver) = provider_event_channel();
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    for id in ["A", "B"] {
        sender
            .send(LiveTranslateServerEvent::UtteranceRegistered {
                utterance_id: id.into(),
            })
            .unwrap();
    }
    sender.send(identified_preview("A", "Synthetic A")).unwrap();
    let newer = identified_preview("B", "Synthetic B");
    sender.send(newer.clone()).unwrap();
    sender
        .send(identified_final("A", "Synthetic A final"))
        .unwrap();
    while let Ok(event) = receiver.try_recv() {
        controller.handle(event);
    }
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert_eq!(
        controller.state.subtitles.history[0].source,
        "Synthetic A final"
    );
    assert_eq!(controller.state.subtitles.source.text, "Synthetic B");
    assert_eq!(
        controller.state.subtitles.translation.text,
        "Synthetic translation Synthetic B"
    );
    assert!(!controller.state.subtitles.source.is_final);
}

#[test]
fn complete_preview_flood_leaves_the_reliable_final_lane_available_and_frozen() {
    let (sender, mut receiver) = provider_event_channel_with_capacity(1);
    for index in 0..1_000 {
        sender
            .send(identified_preview("A", &format!("Synthetic draft {index}")))
            .unwrap();
    }
    let final_event = identified_final("A", "Synthetic final");
    sender.send(final_event.clone()).unwrap();
    assert_eq!(receiver.try_recv().unwrap(), final_event);
    assert!(receiver.try_recv().is_err());
    sender
        .send(identified_preview("A", "Synthetic late draft"))
        .unwrap();
    assert!(receiver.try_recv().is_err());
}

#[test]
fn distinct_registered_sentences_preserve_identical_final_text_without_drafts() {
    let (sender, mut receiver) = provider_event_channel();
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    for id in ["A", "B"] {
        sender
            .send(LiveTranslateServerEvent::UtteranceRegistered {
                utterance_id: id.into(),
            })
            .unwrap();
        sender
            .send(identified_final(id, "Synthetic repeated sentence"))
            .unwrap();
    }
    while let Ok(event) = receiver.try_recv() {
        controller.handle(event);
    }
    assert_eq!(controller.state.subtitles.history.len(), 2);
    assert_eq!(
        controller.state.subtitles.source.utterance_id.as_deref(),
        Some("B")
    );
    let expected = controller.state.clone();
    sender
        .send(identified_final("A", "Synthetic repeated sentence"))
        .unwrap();
    controller.handle(receiver.try_recv().unwrap());
    assert_eq!(controller.state, expected);
}

#[test]
fn complete_preview_and_registration_follow_content_revision_admission() {
    let (sender, mut receiver) = provider_event_channel();
    sender
        .send(LiveTranslateServerEvent::UtteranceRegistered {
            utterance_id: "old".into(),
        })
        .unwrap();
    sender
        .send(identified_preview("old", "Synthetic old"))
        .unwrap();
    sender.advance_content_revision();
    let registration = LiveTranslateServerEvent::UtteranceRegistered {
        utterance_id: "new".into(),
    };
    let preview = identified_preview("new", "Synthetic new");
    sender.send(registration.clone()).unwrap();
    sender.send(preview.clone()).unwrap();
    assert_eq!(receiver.try_recv().unwrap(), registration);
    assert_eq!(receiver.try_recv().unwrap(), preview);
    assert!(receiver.try_recv().is_err());
}

#[test]
fn finish_drain_uses_the_same_revision_admission_and_keeps_reliable_tail_events() {
    let (sender, mut receiver) = provider_event_channel();
    sender
        .send(LiveTranslateServerEvent::TranslationFinal("stale".into()))
        .unwrap();
    let revision = sender.advance_content_revision();
    sender
        .send(LiveTranslateServerEvent::TranslationFinal("current".into()))
        .unwrap();
    sender
        .send(LiveTranslateServerEvent::SessionFinished)
        .unwrap();
    let event = receiver.try_recv_with_revision().unwrap();
    assert_eq!(event.content_revision, revision);
    assert_eq!(
        event.event,
        LiveTranslateServerEvent::TranslationFinal("current".into())
    );
    assert_eq!(
        receiver.try_recv_with_revision().unwrap().event,
        LiveTranslateServerEvent::SessionFinished
    );
    assert!(receiver.try_recv_with_revision().is_err());
}

#[tokio::test]
async fn admission_preserves_queue_identity_across_reliable_draft_and_content_boundaries() {
    let (sender, mut receiver) = provider_event_channel();
    sender
        .send(LiveTranslateServerEvent::TranslationStarted)
        .unwrap();
    sender
        .send(LiveTranslateServerEvent::TranslationDraft(
            "Synthetic draft".into(),
        ))
        .unwrap();
    let reliable = receiver.recv_with_revision().await.unwrap();
    let draft = receiver.recv_with_revision().await.unwrap();
    assert_eq!(reliable.transport_sequence, Some(1));
    assert_eq!(draft.transport_sequence, Some(2));
    assert_eq!(draft.content_revision, 0);
    sender.advance_content_revision();
    sender
        .send(LiveTranslateServerEvent::TranslationFinal(
            "Synthetic final".into(),
        ))
        .unwrap();
    let next = receiver.recv_with_revision().await.unwrap();
    assert_eq!(next.transport_sequence, Some(3));
    assert_eq!(next.content_revision, 1);
}

#[tokio::test]
async fn generated_overflow_has_no_fabricated_queue_identity() {
    let (sender, mut receiver) = provider_event_channel_with_capacity(1);
    sender
        .send(LiveTranslateServerEvent::TranslationStarted)
        .unwrap();
    assert_eq!(
        sender.send(LiveTranslateServerEvent::SessionFinished),
        Err(ProviderEventSendError::Backpressure)
    );
    let overflow = receiver.recv_with_revision().await.unwrap();
    assert_eq!(overflow.transport_sequence, None);
    assert!(matches!(
        overflow.event,
        LiveTranslateServerEvent::Error { .. }
    ));
    assert_eq!(
        receiver
            .recv_with_revision()
            .await
            .unwrap()
            .transport_sequence,
        Some(1)
    );
}

#[tokio::test]
async fn a_previous_identified_final_keeps_the_newer_sentence_draft_but_not_its_own() {
    for draft_id in [7, 8] {
        let (sender, mut receiver) = provider_event_channel();
        let draft = LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: draft_id,
            text: "Synthetic repeated lyric".into(),
            language: None,
        };
        sender.send(draft.clone()).unwrap();
        let final_event = LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 7,
            text: "Synthetic repeated lyric".into(),
            language: None,
        };
        sender.send(final_event.clone()).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), final_event);
        if draft_id == 8 {
            assert_eq!(receiver.try_recv().unwrap(), draft);
        }
        assert!(receiver.try_recv().is_err());
        sender
            .send(LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: 9,
                text: "Synthetic cleared draft".into(),
                language: None,
            })
            .unwrap();
        sender.advance_content_revision();
        sender
            .send(LiveTranslateServerEvent::SessionFinished)
            .unwrap();
        assert_eq!(
            receiver.try_recv().unwrap(),
            LiveTranslateServerEvent::SessionFinished
        );
        assert!(receiver.try_recv().is_err());
    }
}

#[tokio::test]
async fn an_older_hq_confirmation_and_activity_keep_a_queued_newer_source() {
    let (sender, mut receiver) = provider_event_channel();
    let draft = LiveTranslateServerEvent::SourceUtteranceDraft {
        utterance_id: 8,
        text: "Synthetic newer source".into(),
        language: None,
    };
    sender.send(draft.clone()).unwrap();
    let confirmed = LiveTranslateServerEvent::SubtitleConfirmedPair {
        utterance_id: 1,
        source_utterance_id: Some(7),
        source: "Synthetic previous source".into(),
        translation: "Synthetic previous translation".into(),
        language: None,
    };
    for event in [
        confirmed.clone(),
        LiveTranslateServerEvent::TranslationStarted,
        LiveTranslateServerEvent::SubtitlePreviewCleared,
    ] {
        sender.send(event.clone()).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), event);
    }
    assert_eq!(receiver.try_recv().unwrap(), draft);
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn clear_revision_drops_queued_content_but_keeps_lifecycle_and_popped_stamp() {
    let (sender, mut receiver) = provider_event_channel();
    sender
        .send(LiveTranslateServerEvent::SubtitleFinalPair {
            source: "old".into(),
            language: None,
            translation: "old translation".into(),
        })
        .unwrap();
    let already_popped = receiver.recv_with_revision().await.unwrap();
    sender
        .send(LiveTranslateServerEvent::SessionUpdated)
        .unwrap();
    sender
        .send(LiveTranslateServerEvent::SubtitleFinalPair {
            source: "queued old".into(),
            language: None,
            translation: "old".into(),
        })
        .unwrap();
    sender
        .send(LiveTranslateServerEvent::SourceDraft {
            text: "old draft".into(),
            language: None,
        })
        .unwrap();
    let revision = sender.advance_content_revision();
    assert!(already_popped.is_content());
    assert_ne!(already_popped.content_revision, revision);
    sender
        .send_content(
            already_popped.content_revision,
            LiveTranslateServerEvent::TranslationFinal("late old callback".into()),
        )
        .unwrap();
    sender
        .send_content(
            revision,
            LiveTranslateServerEvent::SourceDraft {
                text: "new".into(),
                language: None,
            },
        )
        .unwrap();
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SessionUpdated)
    );
    let new = receiver.recv_with_revision().await.unwrap();
    assert_eq!(new.content_revision, revision);
    assert!(
        matches!(new.event, LiveTranslateServerEvent::SourceDraft { text, .. } if text == "new")
    );
    assert!(receiver.try_recv().is_err());
    sender
        .send(LiveTranslateServerEvent::Error {
            code: "authentication_error".into(),
            message: "fixed".into(),
        })
        .unwrap();
    sender.advance_content_revision();
    assert!(matches!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::Error { .. })
    ));
}

#[test]
fn oversized_caption_is_not_retained_and_only_a_fixed_error_reaches_the_receiver() {
    let (sender, mut receiver) = provider_event_channel();
    let oversized = "x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1);
    for event in [
        LiveTranslateServerEvent::SourceDraft {
            text: oversized.clone(),
            language: None,
        },
        LiveTranslateServerEvent::TranslationDraft(oversized.clone()),
        LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: None,
            utterance_id: 1,
            source: oversized,
            language: None,
            translation: "valid".into(),
        },
    ] {
        assert_eq!(
            sender.send(event),
            Err(ProviderEventSendError::TextTooLarge)
        );
        assert_eq!(
            receiver.try_recv(),
            Ok(LiveTranslateServerEvent::text_limit_error())
        );
    }
    assert!(sender.inner.source_draft.borrow().is_none());
    assert!(sender.inner.translation_draft.borrow().is_none());
    assert!(receiver.try_recv().is_err());
}

#[test]
fn invalidated_draft_cannot_overtake_a_final_during_publication() {
    let (sender, mut receiver) = provider_event_channel();
    let epoch = AtomicU64::new(1);
    let final_pair = LiveTranslateServerEvent::SubtitleFinalPair {
        source: "confirmed".into(),
        language: Some("en".into()),
        translation: "已确认".into(),
    };
    let (checked_tx, checked_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let timeout = std::time::Duration::from_secs(5);

    std::thread::scope(|scope| {
        let sender = &sender;
        let epoch = &epoch;
        let final_pair = &final_pair;
        scope.spawn(move || {
            checked_rx.recv_timeout(timeout).unwrap();
            epoch.store(2, Ordering::SeqCst);
            // Force a final into the old check-then-send gap if publication
            // is unlocked. With an atomic check/send it must wait instead.
            let can_publish = sender.inner.dispatch.try_lock().is_ok();
            if can_publish {
                sender.send(final_pair.clone()).unwrap();
                resume_tx.send(()).unwrap();
            } else {
                resume_tx.send(()).unwrap();
                sender.send(final_pair.clone()).unwrap();
            }
        });

        sender
            .send_if(
                LiveTranslateServerEvent::TranslationDraft("obsolete preview".into()),
                || {
                    let was_current = epoch.load(Ordering::SeqCst) == 1;
                    checked_tx.send(()).unwrap();
                    resume_rx.recv_timeout(timeout).unwrap();
                    was_current
                },
            )
            .unwrap();
    });

    assert_eq!(receiver.try_recv(), Ok(final_pair));
    assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));
}

#[test]
fn stale_drafts_are_discarded_without_suppressing_current_drafts() {
    for draft in [
        LiveTranslateServerEvent::SourceDraft {
            text: "current source".into(),
            language: Some("en".into()),
        },
        LiveTranslateServerEvent::TranslationDraft("当前译文".into()),
    ] {
        let (sender, mut receiver) = provider_event_channel();
        let epoch = AtomicU64::new(1);
        sender
            .send_if(draft.clone(), || epoch.load(Ordering::SeqCst) == 1)
            .unwrap();
        assert_eq!(receiver.try_recv(), Ok(draft.clone()));

        epoch.store(2, Ordering::SeqCst);
        sender
            .send(LiveTranslateServerEvent::TranslationStarted)
            .unwrap();
        sender
            .send_if(draft.clone(), || epoch.load(Ordering::SeqCst) == 1)
            .unwrap();
        assert_eq!(
            receiver.try_recv(),
            Ok(LiveTranslateServerEvent::TranslationStarted)
        );
        assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));

        sender
            .send_if(draft.clone(), || epoch.load(Ordering::SeqCst) == 2)
            .unwrap();
        assert_eq!(receiver.try_recv(), Ok(draft));
    }
}

#[tokio::test]
async fn drafts_are_latest_only() {
    let (sender, mut receiver) = provider_event_channel();
    sender
        .send(LiveTranslateServerEvent::SourceDraft {
            text: "first".into(),
            language: Some("en".into()),
        })
        .unwrap();
    sender
        .send(LiveTranslateServerEvent::SourceDraft {
            text: "latest".into(),
            language: Some("en".into()),
        })
        .unwrap();

    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::SourceDraft {
            text: "latest".into(),
            language: Some("en".into()),
        })
    );
}

#[tokio::test]
async fn interleaved_draft_lanes_are_delivered_in_publish_order() {
    let (sender, mut receiver) = provider_event_channel();
    let translation = LiveTranslateServerEvent::TranslationDraft("older preview".into());
    let source = LiveTranslateServerEvent::SourceDraft {
        text: "new source".into(),
        language: Some("en".into()),
    };
    sender.send(translation.clone()).unwrap();
    sender.send(source.clone()).unwrap();

    assert_eq!(receiver.recv().await, Some(translation));
    assert_eq!(receiver.recv().await, Some(source));
}

#[tokio::test]
async fn reliable_events_take_priority_and_suppress_older_drafts() {
    let (sender, mut receiver) = provider_event_channel();
    sender
        .send(LiveTranslateServerEvent::SourceDraft {
            text: "stale".into(),
            language: Some("en".into()),
        })
        .unwrap();
    let final_pair = LiveTranslateServerEvent::SubtitleFinalPair {
        source: "confirmed".into(),
        language: Some("en".into()),
        translation: "已确认".into(),
    };
    sender.send(final_pair.clone()).unwrap();

    assert_eq!(receiver.recv().await, Some(final_pair));
    let no_stale =
        tokio::time::timeout(std::time::Duration::from_millis(20), receiver.recv()).await;
    assert!(no_stale.is_err());
}

#[tokio::test]
async fn preview_lifecycle_and_pair_preserve_independent_latest_drafts() {
    for use_async_receive in [false, true] {
        let (sender, mut receiver) = provider_event_channel();
        let started = LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 1 };
        let translation =
            LiveTranslateServerEvent::TranslationDraft("synthetic translation".into());
        let pair = LiveTranslateServerEvent::SubtitlePreviewPair {
            source_utterance_id: None,
            source: "synthetic source".into(),
            language: Some("en".into()),
            translation: "synthetic translation".into(),
        };
        let finished = LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 1 };
        let latest_source = LiveTranslateServerEvent::SourceDraft {
            text: "newer synthetic source".into(),
            language: Some("en".into()),
        };
        sender.send(started.clone()).unwrap();
        sender.send(translation.clone()).unwrap();
        sender.send(pair.clone()).unwrap();
        sender.send(finished.clone()).unwrap();
        sender.send(latest_source.clone()).unwrap();

        // Reliable activity must be observable without swallowing raw
        // recognition or the successful latest-value translation lane.
        for expected in [started, pair, finished, translation, latest_source] {
            let actual = if use_async_receive {
                receiver.recv().await.unwrap()
            } else {
                receiver.try_recv().unwrap()
            };
            assert_eq!(actual, expected);
        }
        assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));
    }
}

#[test]
fn final_barrier_remains_ordered_after_preview_signals_and_discards_older_drafts() {
    let (sender, mut receiver) = provider_event_channel();
    let pair = LiveTranslateServerEvent::SubtitlePreviewPair {
        source_utterance_id: None,
        source: "synthetic source".into(),
        language: Some("en".into()),
        translation: "synthetic preview".into(),
    };
    let finished = LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 1 };
    let final_pair = LiveTranslateServerEvent::SubtitleFinalPair {
        source: "corrected synthetic final".into(),
        language: Some("en".into()),
        translation: "synthetic final".into(),
    };
    sender.send(pair.clone()).unwrap();
    sender
        .send(LiveTranslateServerEvent::TranslationDraft(
            "obsolete draft".into(),
        ))
        .unwrap();
    sender.send(finished.clone()).unwrap();
    sender.send(final_pair.clone()).unwrap();

    assert_eq!(receiver.try_recv(), Ok(pair));
    assert_eq!(receiver.try_recv(), Ok(finished));
    assert_eq!(receiver.try_recv(), Ok(final_pair));
    assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));
}

#[tokio::test]
async fn reliable_overflow_emits_one_recoverable_control_event() {
    let (sender, mut receiver) = provider_event_channel_with_capacity(1);
    sender
        .send(LiveTranslateServerEvent::TranslationStarted)
        .unwrap();
    assert_eq!(
        sender.send(LiveTranslateServerEvent::SessionFinished),
        Err(ProviderEventSendError::Backpressure)
    );

    assert_eq!(receiver.recv().await, Some(overflow_event()));
    assert_eq!(
        sender.send(LiveTranslateServerEvent::SessionFinished),
        Err(ProviderEventSendError::Backpressure)
    );
    assert_eq!(
        receiver.recv().await,
        Some(LiveTranslateServerEvent::TranslationStarted)
    );
}

#[test]
fn overflow_diagnostic_is_content_free() {
    let LiveTranslateServerEvent::Error { code, message } = overflow_event() else {
        unreachable!();
    };
    assert_eq!(code, OVERFLOW_CODE);
    assert!(!message.contains("subtitle"));
    assert!(!message.contains("translation text"));
}

fn stamped_draft(utterance_id: &str, role: UtteranceRole, text: &str) -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::UtteranceText {
        utterance_id: utterance_id.into(),
        role,
        text: text.into(),
        is_final: false,
        language: None,
    }
}

#[test]
fn ordered_application_preserves_current_deadlines_and_duplicate_final_ownership() {
    use crate::core::audio::audio_input::{AudioInput, AudioSource};
    use crate::core::lifecycle::session::TranslationSessionController;
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.did_connect();
    let now = std::time::Instant::now();
    let mut deadlines = Vec::new();
    for source in [AudioSource::System, AudioSource::Microphone] {
        let (sender, mut receiver) = provider_event_channel();
        sender
            .send_with_source_order(
                stamped_draft("current", UtteranceRole::Source, "Synthetic current"),
                Some(1),
            )
            .unwrap();
        receiver
            .try_recv_with_revision()
            .unwrap()
            .apply_to(&mut controller, source);
        let started = ProviderEvent::unsequenced(LiveTranslateServerEvent::TranslationStarted, 0)
            .apply_to_at(&mut controller, source, now);
        deadlines.push((source, started.translation_pending_deadline.unwrap()));
        sender
            .send_with_source_order(
                identified_final("old", "Synthetic previous complete"),
                Some(0),
            )
            .unwrap();
        let old = receiver
            .try_recv_with_revision()
            .unwrap()
            .apply_to(&mut controller, source);
        assert!(!old.resolves_translation_timeout);
        assert_eq!(
            controller.translation_pending_deadline_from(source),
            started.translation_pending_deadline
        );
        assert_eq!(
            controller
                .state
                .subtitles
                .tracks
                .iter()
                .find(|track| track.audio_source == source)
                .unwrap()
                .source
                .utterance_id
                .as_deref(),
            Some("current")
        );
    }
    for (source, deadline) in deadlines {
        assert_eq!(
            controller.translation_pending_deadline_from(source),
            Some(deadline)
        );
        let (sender, mut receiver) = provider_event_channel();
        sender
            .send_with_source_order(
                identified_final("current", "Synthetic current complete"),
                Some(1),
            )
            .unwrap();
        let current = receiver
            .try_recv_with_revision()
            .unwrap()
            .apply_to(&mut controller, source);
        assert!(current.resolves_translation_timeout);
        assert!(controller
            .translation_pending_deadline_from(source)
            .is_none());
        let next = ProviderEvent::unsequenced(LiveTranslateServerEvent::TranslationStarted, 0)
            .apply_to_at(
                &mut controller,
                source,
                now + std::time::Duration::from_secs(1),
            );
        sender
            .send_with_source_order(identified_final("current", "Synthetic duplicate"), Some(1))
            .unwrap();
        let duplicate = receiver
            .try_recv_with_revision()
            .unwrap()
            .apply_to(&mut controller, source);
        assert!(!duplicate.resolves_translation_timeout);
        assert_eq!(
            controller.translation_pending_deadline_from(source),
            next.translation_pending_deadline
        );
    }
    assert_eq!(controller.state.subtitles.history.len(), 4);
}

fn translating_final(id: &str, text: &str) -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::UtteranceText {
        utterance_id: id.into(),
        role: UtteranceRole::Source,
        text: text.into(),
        is_final: true,
        language: None,
    }
}

#[test]
fn translating_source_final_commits_one_envelope_and_one_timer_per_source() {
    use crate::core::audio::audio_input::{AudioInput, AudioSource};
    use crate::core::lifecycle::session::{
        TranslationSessionController, TRANSLATION_PENDING_TIMEOUT,
    };
    let mut controller = TranslationSessionController::default();
    controller.set_audio_input(AudioInput::Both);
    controller.did_connect();
    let now = std::time::Instant::now();
    for source in [AudioSource::System, AudioSource::Microphone] {
        // One remaining reliable slot is enough for both text and activity.
        let (sender, mut receiver) = provider_event_channel_with_capacity(1);
        let final_ = translating_final("current", "Synthetic current source.");
        sender
            .send_translating_source_final(final_.clone(), Some(0))
            .unwrap();
        let envelope = receiver.try_recv_with_revision().unwrap();
        assert_eq!(envelope.event, final_);
        assert!(envelope.starts_translation);
        let application = envelope.clone().apply_to_at(&mut controller, source, now);
        assert_eq!(
            application.translation_pending_deadline,
            Some(now + TRANSLATION_PENDING_TIMEOUT)
        );
        assert!(!application.resolves_translation_timeout);
        assert_eq!(
            controller.translation_pending_deadline_from(source),
            application.translation_pending_deadline
        );
        assert!(receiver.try_recv_with_revision().is_err());
    }
    assert!(controller
        .expire_translation_pending_from(AudioSource::System, now + TRANSLATION_PENDING_TIMEOUT));
    assert!(controller
        .translation_pending_deadline_from(AudioSource::Microphone)
        .is_some());
}

#[test]
fn translating_activity_is_cleared_or_rejected_with_its_source_final() {
    use crate::core::audio::audio_input::AudioSource;
    let (sender, mut receiver) = provider_event_channel_with_capacity(1);
    sender
        .send_translating_source_final(
            translating_final("cleared", "Synthetic cleared source"),
            Some(0),
        )
        .unwrap();
    sender.advance_content_revision();
    assert!(receiver.try_recv_with_revision().is_err());

    sender
        .send(LiveTranslateServerEvent::SessionCreated)
        .unwrap();
    assert_eq!(
        sender.send_translating_source_final(
            translating_final("rejected", "Synthetic rejected source"),
            Some(1)
        ),
        Err(ProviderEventSendError::Backpressure)
    );
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    let mut controls = 0;
    while let Ok(envelope) = receiver.try_recv_with_revision() {
        controls += usize::from(matches!(
            envelope.event,
            LiveTranslateServerEvent::Error { .. }
        ));
        assert!(!envelope.starts_translation);
        assert!(envelope
            .apply_to(&mut controller, AudioSource::System)
            .translation_pending_deadline
            .is_none());
    }
    assert_eq!(controls, 1);
    assert!(!controller.state.is_translation_pending);
    assert!(controller.state.subtitles.source.text.is_empty());
}

#[test]
fn only_valid_identified_source_finals_can_carry_translation_activity() {
    use crate::core::audio::audio_input::AudioSource;
    let oversized = "x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1);
    for event in [
        stamped_draft("draft", UtteranceRole::Source, "Synthetic draft"),
        LiveTranslateServerEvent::TranslationStarted,
        LiveTranslateServerEvent::Error {
            code: "synthetic_error".into(),
            message: "Synthetic failure".into(),
        },
        translating_final("oversized-text", &oversized),
    ] {
        let (sender, mut receiver) = provider_event_channel();
        let result = sender.send_translating_source_final(event, Some(0));
        assert!(result.is_ok() || result == Err(ProviderEventSendError::TextTooLarge));
        assert!(
            !receiver
                .try_recv_with_revision()
                .unwrap()
                .starts_translation
        );
    }
    for (id, text) in [
        (" ", "Synthetic source"),
        ("empty", " \u{3000}"),
        (oversized.as_str(), "Synthetic source"),
    ] {
        let (sender, mut receiver) = provider_event_channel();
        sender
            .send_translating_source_final(translating_final(id, text), None)
            .unwrap();
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        controller.did_connect();
        let application = receiver
            .try_recv_with_revision()
            .unwrap()
            .apply_to(&mut controller, AudioSource::System);
        assert!(application.translation_pending_deadline.is_none());
        assert!(!controller.state.is_translation_pending);
    }
}

#[test]
fn rejected_stopping_content_cannot_register_rank_ahead_of_an_accepted_final_tail() {
    use crate::core::audio::audio_input::AudioSource;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    controller.begin_stopping();
    let (sender, mut receiver) = provider_event_channel();
    sender
        .send_with_source_order(
            stamped_draft("ignored", UtteranceRole::Source, "Synthetic ignored"),
            Some(0),
        )
        .unwrap();
    receiver
        .try_recv_with_revision()
        .unwrap()
        .apply_to(&mut controller, AudioSource::System);
    sender
        .send_with_source_order(identified_final("tail", "Synthetic accepted tail"), Some(0))
        .unwrap();
    receiver
        .try_recv_with_revision()
        .unwrap()
        .apply_to(&mut controller, AudioSource::System);
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert_eq!(
        controller.state.subtitles.history[0].source,
        "Synthetic accepted tail"
    );
}

#[test]
fn ordered_application_accepts_zero_again_after_reconnect_and_content_clear() {
    use crate::core::audio::audio_input::AudioSource;
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    let (sender, mut receiver) = provider_event_channel();
    sender
        .send_with_source_order(
            identified_final("previous", "Synthetic previous complete"),
            Some(99),
        )
        .unwrap();
    receiver
        .try_recv_with_revision()
        .unwrap()
        .apply_to(&mut controller, AudioSource::System);
    controller.begin_connecting();
    sender
        .send_with_source_order(
            stamped_draft("next", UtteranceRole::Source, "Synthetic reconnected"),
            Some(0),
        )
        .unwrap();
    receiver
        .try_recv_with_revision()
        .unwrap()
        .apply_to(&mut controller, AudioSource::System);
    assert_eq!(
        controller.state.subtitles.source.text,
        "Synthetic reconnected"
    );
    assert_eq!(controller.state.subtitles.history.len(), 1);
    sender
        .send_with_source_order(
            identified_final("stale", "Synthetic pre-clear final"),
            Some(100),
        )
        .unwrap();
    sender.advance_content_revision();
    controller.clear_subtitles();
    sender
        .send_with_source_order(
            stamped_draft("fresh", UtteranceRole::Source, "Synthetic after clear"),
            Some(0),
        )
        .unwrap();
    receiver
        .try_recv_with_revision()
        .unwrap()
        .apply_to(&mut controller, AudioSource::System);
    assert_eq!(
        controller.state.subtitles.source.text,
        "Synthetic after clear"
    );
    assert!(controller.state.subtitles.history.is_empty());
    assert!(receiver.try_recv_with_revision().is_err());
}

#[test]
fn ordered_stamped_lanes_survive_a_late_previous_final() {
    let (sender, mut receiver) = provider_event_channel();
    // Registration order is authoritative even when opaque IDs sort the
    // other way and source drafts have coalesced before reduction.
    for id in ["source_z", "source_a"] {
        sender
            .send(LiveTranslateServerEvent::UtteranceRegistered {
                utterance_id: id.into(),
            })
            .unwrap();
    }
    for (role, text) in [
        (UtteranceRole::Source, "Synthetic following source"),
        (
            UtteranceRole::Translation,
            "Synthetic following translation",
        ),
    ] {
        sender
            .send_with_source_order(stamped_draft("source_a", role, text), Some(1))
            .unwrap();
    }
    sender
        .send_with_source_order(identified_final("source_z", "Synthetic previous"), Some(0))
        .unwrap();
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    while let Ok(event) = receiver.try_recv() {
        controller.handle(event);
    }
    assert_eq!(
        controller.state.subtitles.source.text,
        "Synthetic following source"
    );
    assert_eq!(
        controller.state.subtitles.translation.text,
        "Synthetic following translation"
    );
    assert!(!controller.state.subtitles.source.is_final);
    assert_eq!(controller.state.subtitles.history.len(), 1);
    assert_eq!(
        controller.state.subtitles.history[0].source,
        "Synthetic previous"
    );
}

#[test]
fn ordered_older_stamped_drafts_cannot_replay_after_a_newer_final() {
    for role in [UtteranceRole::Source, UtteranceRole::Translation] {
        let (sender, mut receiver) = provider_event_channel();
        for id in ["source_z", "source_a"] {
            sender
                .send(LiveTranslateServerEvent::UtteranceRegistered {
                    utterance_id: id.into(),
                })
                .unwrap();
        }
        sender
            .send_with_source_order(
                stamped_draft("source_z", role, "Synthetic stale draft"),
                Some(0),
            )
            .unwrap();
        let final_event = identified_final("source_a", "Synthetic newer final");
        sender
            .send_with_source_order(final_event.clone(), Some(1))
            .unwrap();
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        let mut delivered_stale = false;
        while let Ok(event) = receiver.try_recv() {
            delivered_stale |= matches!(event, LiveTranslateServerEvent::UtteranceText { .. });
            controller.handle(event);
        }
        assert!(!delivered_stale);
        assert_eq!(
            controller.state.subtitles.source.utterance_id.as_deref(),
            Some("source_a")
        );
        assert!(controller.state.subtitles.source.is_final);
        assert_eq!(controller.state.subtitles.history.len(), 1);
    }
}

#[test]
fn stamped_final_freezes_its_own_queued_and_late_drafts() {
    for role in [UtteranceRole::Source, UtteranceRole::Translation] {
        let (sender, mut receiver) = provider_event_channel();
        let draft = stamped_draft("source", role, "Synthetic draft");
        sender
            .send_with_source_order(draft.clone(), Some(0))
            .unwrap();
        let final_event = identified_final("source", "Synthetic final");
        sender
            .send_with_source_order(final_event.clone(), Some(0))
            .unwrap();
        assert_eq!(receiver.try_recv(), Ok(final_event));
        assert!(receiver.try_recv().is_err());
        sender.send_with_source_order(draft, Some(0)).unwrap();
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn unknown_stamped_order_and_unstamped_drafts_keep_final_barriers() {
    for role in [UtteranceRole::Source, UtteranceRole::Translation] {
        for (draft_order, final_order) in [(None, Some(0)), (Some(1), None), (None, None)] {
            let (sender, mut receiver) = provider_event_channel();
            sender
                .send_with_source_order(
                    stamped_draft("following", role, "Synthetic unknown draft"),
                    draft_order,
                )
                .unwrap();
            let final_event = identified_final("previous", "Synthetic final");
            sender
                .send_with_source_order(final_event.clone(), final_order)
                .unwrap();
            assert_eq!(receiver.try_recv(), Ok(final_event));
            assert!(receiver.try_recv().is_err());
        }
    }
    let (sender, mut receiver) = provider_event_channel();
    sender
        .send(LiveTranslateServerEvent::SourceDraft {
            text: "Synthetic source".into(),
            language: None,
        })
        .unwrap();
    sender
        .send(LiveTranslateServerEvent::TranslationDraft(
            "Synthetic translation".into(),
        ))
        .unwrap();
    let final_event = identified_final("previous", "Synthetic final");
    sender
        .send_with_source_order(final_event.clone(), Some(0))
        .unwrap();
    assert_eq!(receiver.try_recv(), Ok(final_event));
    assert!(receiver.try_recv().is_err());
}

#[test]
fn later_barriers_still_discard_ordered_stamped_drafts() {
    for barrier in [
        LiveTranslateServerEvent::SessionUpdated,
        LiveTranslateServerEvent::SessionFinished,
        LiveTranslateServerEvent::Error {
            code: "synthetic".into(),
            message: "Synthetic failure".into(),
        },
        LiveTranslateServerEvent::SubtitleFinalPair {
            source: "Synthetic source".into(),
            translation: "Synthetic translation".into(),
            language: None,
        },
    ] {
        for role in [UtteranceRole::Source, UtteranceRole::Translation] {
            let (sender, mut receiver) = provider_event_channel();
            sender
                .send_with_source_order(
                    stamped_draft("following", role, "Synthetic draft"),
                    Some(1),
                )
                .unwrap();
            let final_event = identified_final("previous", "Synthetic final");
            sender
                .send_with_source_order(final_event.clone(), Some(0))
                .unwrap();
            sender.send(barrier.clone()).unwrap();
            assert_eq!(receiver.try_recv(), Ok(final_event));
            assert_eq!(receiver.try_recv(), Ok(barrier.clone()));
            assert!(receiver.try_recv().is_err());
        }
    }
}

#[test]
fn content_clear_releases_stamped_final_freeze_and_order() {
    for role in [UtteranceRole::Source, UtteranceRole::Translation] {
        let (sender, mut receiver) = provider_event_channel();
        let final_event = identified_final("source", "Synthetic previous final");
        sender
            .send_with_source_order(final_event.clone(), Some(99))
            .unwrap();
        assert_eq!(receiver.try_recv(), Ok(final_event));
        sender.advance_content_revision();
        let draft = stamped_draft("source", role, "Synthetic new content");
        sender
            .send_with_source_order(draft.clone(), Some(0))
            .unwrap();
        assert_eq!(receiver.try_recv(), Ok(draft));
        assert!(receiver.try_recv().is_err());
        let next_draft = stamped_draft("following", role, "Synthetic following content");
        sender
            .send_with_source_order(next_draft.clone(), Some(1))
            .unwrap();
        let final_event = identified_final("source", "Synthetic new final");
        sender
            .send_with_source_order(final_event.clone(), Some(0))
            .unwrap();
        assert_eq!(receiver.try_recv(), Ok(final_event));
        assert_eq!(receiver.try_recv(), Ok(next_draft));
    }
}

#[test]
fn source_order_does_not_change_admission_after_the_barrier() {
    for role in [UtteranceRole::Source, UtteranceRole::Translation] {
        for order in [None, Some(0)] {
            let (sender, mut receiver) = provider_event_channel();
            let final_event = identified_final("newer", "Synthetic final");
            sender
                .send_with_source_order(final_event.clone(), Some(1))
                .unwrap();
            assert_eq!(receiver.try_recv(), Ok(final_event));
            // The reducer still owns admission of later publications. The
            // transport exception only repairs already-queued previews.
            let draft = stamped_draft("older-or-unknown", role, "Synthetic later publication");
            sender.send_with_source_order(draft.clone(), order).unwrap();
            assert_eq!(receiver.try_recv(), Ok(draft));
        }
    }
}

#[test]
fn stamped_drafts_use_the_latest_value_lanes() {
    let (sender, mut receiver) = provider_event_channel();
    // Far more drafts than the reliable lane could hold: an accidental
    // reliable-lane send would fill it and fail with Backpressure.
    for index in 0..200 {
        sender
            .send(stamped_draft(
                &format!("source_{index}"),
                UtteranceRole::Source,
                &format!("source draft {index}"),
            ))
            .unwrap();
        sender
            .send(stamped_draft(
                &format!("source_{index}"),
                UtteranceRole::Translation,
                &format!("译文草稿 {index}"),
            ))
            .unwrap();
    }

    assert_eq!(
        receiver.try_recv(),
        Ok(stamped_draft(
            "source_199",
            UtteranceRole::Source,
            "source draft 199"
        ))
    );
    assert_eq!(
        receiver.try_recv(),
        Ok(stamped_draft(
            "source_199",
            UtteranceRole::Translation,
            "译文草稿 199"
        ))
    );
    assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));
}

#[test]
fn stamped_finals_stay_on_the_reliable_lane() {
    let (sender, mut receiver) = provider_event_channel();
    for index in 0..200 {
        sender
            .send(stamped_draft(
                &format!("source_{index}"),
                UtteranceRole::Source,
                &format!("draft {index}"),
            ))
            .unwrap();
    }

    let source_final = LiveTranslateServerEvent::UtteranceText {
        utterance_id: "source_199".into(),
        role: UtteranceRole::Source,
        text: "final".into(),
        is_final: true,
        language: Some("en".into()),
    };
    let pair = LiveTranslateServerEvent::SubtitleFinalPair {
        source: "final".into(),
        language: Some("en".into()),
        translation: "定稿".into(),
    };
    sender.send(source_final.clone()).unwrap();
    sender.send(pair.clone()).unwrap();

    assert_eq!(receiver.try_recv(), Ok(source_final));
    assert_eq!(receiver.try_recv(), Ok(pair));
    // The draft published before them cannot resurface after the final.
    assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));
}
