use super::*;
use crate::MAX_SUBTITLE_TEXT_BYTES;

fn identity(item: &str, previous: Option<&str>) -> LivePairIdentity {
    LivePairIdentity {
        item_id: Some(item.into()),
        previous_item_id: previous.map(String::from),
    }
}

#[test]
fn source_ordinals_survive_window_eviction_and_serialization() {
    let mut aligner = LivePairAligner::default();
    let source = LivePairEvent::SourceDraft {
        text: "Synthetic source".into(),
        language: None,
    };
    for index in 0..=MAX_TRACKED_ITEMS {
        aligner.observe_at(&source, &identity(&format!("source-{index}"), None), 0);
    }
    assert_eq!(aligner.source_ordinal("source-0"), None);
    assert_eq!(aligner.source_ordinal("source-1"), Some(1));
    assert_eq!(
        aligner.source_ordinal(&format!("source-{MAX_TRACKED_ITEMS}")),
        Some(MAX_TRACKED_ITEMS as u64)
    );
    let restored: LivePairAligner =
        serde_json::from_value(serde_json::to_value(&aligner).unwrap()).unwrap();
    assert_eq!(restored.source_ordinal("source-0"), None);
    assert_eq!(restored.source_ordinal("source-1"), Some(1));
    assert_eq!(
        restored.source_ordinal(&format!("source-{MAX_TRACKED_ITEMS}")),
        Some(MAX_TRACKED_ITEMS as u64)
    );
    assert!(restored.validate_state().is_ok());
}

#[test]
fn source_ordinal_defaults_for_old_states_and_resets_with_content() {
    let mut aligner = LivePairAligner::default();
    let source = LivePairEvent::SourceDraft {
        text: "Synthetic source".into(),
        language: None,
    };
    for id in ["source-z", "source-a"] {
        aligner.observe_at(&source, &identity(id, None), 0);
    }
    let mut legacy = serde_json::to_value(&aligner).unwrap();
    legacy.as_object_mut().unwrap().remove("source_order_start");
    let mut restored: LivePairAligner = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.source_ordinal("source-z"), Some(0));
    assert_eq!(restored.source_ordinal("source-a"), Some(1));
    restored.clear_content();
    assert_eq!(restored.source_ordinal("source-z"), None);
    assert_eq!(restored.source_ordinal("source-a"), None);
    restored.observe_at(&source, &identity("source-new", None), 0);
    assert_eq!(restored.source_ordinal("source-new"), Some(0));
    assert!(restored.validate_state().is_ok());
}

#[test]
fn an_orphaned_translation_draft_cannot_replace_the_current_identified_translation() {
    let mut aligner = LivePairAligner::default();
    let mut reducer = crate::SubtitleReducer::<crate::NoopArchive>::new(6);
    for (event, identity) in [
        (
            LivePairEvent::ItemCreated,
            identity("response-a", Some("source-a")),
        ),
        (
            LivePairEvent::SourceFinal {
                text: "Synthetic first sentence.".into(),
                language: Some("en".into()),
            },
            identity("source-a", None),
        ),
        (
            LivePairEvent::TranslationFinal {
                text: "Synthetic first translation.".into(),
            },
            identity("response-a", None),
        ),
        (
            LivePairEvent::ItemCreated,
            identity("response-b", Some("source-b")),
        ),
        (
            LivePairEvent::SourceDraft {
                text: "Synthetic current sentence".into(),
                language: Some("en".into()),
            },
            identity("source-b", None),
        ),
        (
            LivePairEvent::TranslationDraft {
                text: "Synthetic current translation".into(),
            },
            identity("response-b", None),
        ),
    ] {
        for output in aligner.observe_at(&event, &identity, 0) {
            if let Some(event) = output.subtitle_event() {
                reducer.apply(event);
            }
        }
    }
    let before = reducer.snapshot.clone();
    for orphan in ["response-a", "unknown-response"] {
        for output in aligner.observe_at(
            &LivePairEvent::TranslationDraft {
                text: "Synthetic orphaned translation".into(),
            },
            &identity(orphan, None),
            1,
        ) {
            if let Some(event) = output.subtitle_event() {
                reducer.apply(event);
            }
        }
        assert_eq!(reducer.snapshot, before, "orphan response {orphan}");
    }
    // The compatibility path remains available only when no identity exists.
    let legacy = LivePairEvent::TranslationDraft {
        text: "Synthetic legacy draft".into(),
    };
    assert_eq!(
        aligner.observe_at(&legacy, &LivePairIdentity::default(), 2),
        vec![legacy]
    );
}

#[test]
fn follow_latency_floors_the_difference_without_rounding_each_boundary() {
    let mut aligner = LivePairAligner::default();
    aligner.observe_at(
        &LivePairEvent::ItemCreated,
        &identity("response", Some("source")),
        0,
    );
    aligner.observe_at(
        &LivePairEvent::SourceFinal {
            text: "Source sentence".into(),
            language: Some("en".into()),
        },
        &identity("source", None),
        1_900_000,
    );
    assert_eq!(
        aligner.observe_at(
            &LivePairEvent::TranslationFinal {
                text: "完整译文".into(),
            },
            &identity("response", None),
            3_100_000,
        ),
        vec![LivePairEvent::FinalPair {
            utterance_id: "source".into(),
            source: "Source sentence".into(),
            translation: "完整译文".into(),
            language: Some("en".into()),
            follow_latency_ms: Some(1),
        }]
    );
}

#[test]
fn late_previous_source_final_cannot_confirm_or_lose_the_current_draft() {
    let mut aligner = LivePairAligner::default();
    let mut reducer = crate::SubtitleReducer::<crate::NoopArchive>::new(6);
    let steps = [
        (
            LivePairEvent::ItemCreated,
            identity("response-a", Some("source-a")),
        ),
        (
            LivePairEvent::SourceDraft {
                text: "Synthetic A draft".into(),
                language: Some("en".into()),
            },
            identity("source-a", None),
        ),
        (
            LivePairEvent::SourceDraft {
                text: "Synthetic B incomplete".into(),
                language: Some("en".into()),
            },
            identity("source-b", None),
        ),
        (
            LivePairEvent::ItemCreated,
            identity("response-b", Some("source-b")),
        ),
        (
            LivePairEvent::TranslationFinal {
                text: "Synthetic B translation.".into(),
            },
            identity("response-b", None),
        ),
        (
            LivePairEvent::SourceFinal {
                text: "Synthetic A complete.".into(),
                language: Some("en".into()),
            },
            identity("source-a", None),
        ),
        (
            LivePairEvent::SourceFinal {
                text: "Synthetic B complete.".into(),
                language: Some("en".into()),
            },
            identity("source-b", None),
        ),
        (
            LivePairEvent::TranslationFinal {
                text: "Synthetic B translation.".into(),
            },
            identity("response-b", None),
        ),
        (
            LivePairEvent::TranslationFinal {
                text: "Synthetic A translation.".into(),
            },
            identity("response-a", None),
        ),
    ];
    let mut observed = Vec::new();
    for (index, (event, identity)) in steps.into_iter().enumerate() {
        let output = aligner.observe_at(&event, &identity, index as u64 * 1_000_000);
        for event in &output {
            if let Some(event) = event.subtitle_event() {
                reducer.apply(event);
            }
        }
        observed.push(output);
    }
    let history: Vec<_> = reducer
        .snapshot
        .history
        .iter()
        .map(|pair| (pair.source.as_str(), pair.translation.as_str()))
        .collect();
    assert_eq!(
        history,
        vec![
            ("Synthetic B complete.", "Synthetic B translation."),
            ("Synthetic A complete.", "Synthetic A translation."),
        ],
        "late A output={:?}; B final output={:?}; B translation replay={:?}",
        observed[5],
        observed[6],
        observed[7]
    );
    assert!(!observed[5].iter().any(|event| matches!(event, LivePairEvent::FinalPair { utterance_id, .. } if utterance_id == "source-b")));
}

#[test]
fn next_translation_before_any_source_keeps_its_link_and_waits_for_final() {
    let mut aligner = LivePairAligner::default();
    aligner.observe_at(
        &LivePairEvent::ItemCreated,
        &identity("response-a", Some("source-a")),
        0,
    );
    aligner.observe_at(
        &LivePairEvent::SourceFinal {
            text: "Synthetic A complete.".into(),
            language: Some("en".into()),
        },
        &identity("source-a", None),
        1,
    );
    assert!(aligner
        .observe_at(
            &LivePairEvent::TranslationFinal {
                text: "Synthetic A translation.".into()
            },
            &identity("response-a", None),
            2
        )
        .iter()
        .any(|event| matches!(event, LivePairEvent::FinalPair { .. })));
    aligner.observe_at(
        &LivePairEvent::ItemCreated,
        &identity("response-b", Some("source-b")),
        3,
    );
    assert!(aligner
        .observe_at(
            &LivePairEvent::TranslationFinal {
                text: "Synthetic B translation.".into()
            },
            &identity("response-b", None),
            4
        )
        .is_empty());
    let output = aligner.observe_at(
        &LivePairEvent::SourceFinal {
            text: "Synthetic B complete.".into(),
            language: Some("en".into()),
        },
        &identity("source-b", None),
        5,
    );
    assert!(output.iter().any(|event| matches!(event, LivePairEvent::FinalPair { utterance_id, source, translation, .. } if utterance_id == "source-b" && source == "Synthetic B complete." && translation == "Synthetic B translation.")), "B source-final output={output:?}; retained response links={}", aligner.response_count());
}

fn legacy_roundtrip(aligner: LivePairAligner) -> LivePairAligner {
    let mut state = serde_json::to_value(aligner).unwrap();
    state.as_object_mut().unwrap().remove("source_order");
    let restored: LivePairAligner = serde_json::from_value(state).unwrap();
    assert!(restored.validate_state().is_ok());
    restored
}

#[test]
fn legacy_state_keeps_a_future_translation_placeholder_after_retired_current_source() {
    let mut aligner = LivePairAligner::default();
    aligner.observe_at(
        &LivePairEvent::ItemCreated,
        &identity("response-a", Some("source-a")),
        0,
    );
    aligner.observe_at(
        &LivePairEvent::SourceFinal {
            text: "Synthetic A.".into(),
            language: None,
        },
        &identity("source-a", None),
        1,
    );
    aligner.observe_at(
        &LivePairEvent::TranslationFinal {
            text: "Synthetic A translation.".into(),
        },
        &identity("response-a", None),
        2,
    );
    aligner.observe_at(
        &LivePairEvent::ItemCreated,
        &identity("response-b", Some("source-b")),
        3,
    );
    aligner.observe_at(
        &LivePairEvent::TranslationFinal {
            text: "Synthetic B translation.".into(),
        },
        &identity("response-b", None),
        4,
    );
    let mut aligner = legacy_roundtrip(aligner);
    let output = aligner.observe_at(
        &LivePairEvent::SourceFinal {
            text: "Synthetic B.".into(),
            language: None,
        },
        &identity("source-b", None),
        5,
    );
    assert!(output.iter().any(|event| matches!(event, LivePairEvent::FinalPair { utterance_id, source, .. } if utterance_id == "source-b" && source == "Synthetic B.")));
    assert!(aligner.validate_state().is_ok());
}

#[test]
fn legacy_translation_placeholder_before_current_is_not_a_seen_source() {
    let mut aligner = LivePairAligner::default();
    aligner.observe_at(
        &LivePairEvent::ItemCreated,
        &identity("response-c", Some("source-c")),
        0,
    );
    aligner.observe_at(
        &LivePairEvent::TranslationFinal {
            text: "Synthetic C translation.".into(),
        },
        &identity("response-c", None),
        1,
    );
    aligner.observe_at(
        &LivePairEvent::SourceDraft {
            text: "Synthetic A draft".into(),
            language: None,
        },
        &identity("source-a", None),
        2,
    );
    let mut aligner = legacy_roundtrip(aligner);
    assert!(aligner
        .observe_at(
            &LivePairEvent::TranslationFinal {
                text: "Synthetic C translation.".into()
            },
            &identity("response-c", None),
            3
        )
        .is_empty());
    aligner.observe_at(
        &LivePairEvent::SourceDraft {
            text: "Synthetic C draft".into(),
            language: None,
        },
        &identity("source-c", None),
        4,
    );
    let output = aligner.observe_at(
        &LivePairEvent::SourceFinal {
            text: "Synthetic C complete.".into(),
            language: None,
        },
        &identity("source-c", None),
        5,
    );
    assert!(output.iter().any(|event| matches!(event, LivePairEvent::FinalPair { utterance_id, .. } if utterance_id == "source-c")), "C final output={output:?}; current={:?}", aligner.current_source_id);
}

#[test]
fn legacy_empty_current_source_is_observed_before_the_next_source() {
    let mut aligner = LivePairAligner::default();
    aligner.observe_at(
        &LivePairEvent::SourceDraft {
            text: String::new(),
            language: None,
        },
        &identity("source-a", None),
        0,
    );
    let mut aligner = legacy_roundtrip(aligner);
    aligner.observe_at(
        &LivePairEvent::SourceDraft {
            text: "Synthetic B draft".into(),
            language: None,
        },
        &identity("source-b", None),
        1,
    );
    aligner.observe_at(
        &LivePairEvent::ItemCreated,
        &identity("response-b", Some("source-b")),
        2,
    );
    aligner.observe_at(
        &LivePairEvent::TranslationFinal {
            text: "Synthetic B translation.".into(),
        },
        &identity("response-b", None),
        3,
    );
    let output = aligner.observe_at(
        &LivePairEvent::SourceFinal {
            text: "Synthetic A.".into(),
            language: None,
        },
        &identity("source-a", None),
        4,
    );
    assert!(!output.iter().any(|event| matches!(event, LivePairEvent::FinalPair { utterance_id, .. } if utterance_id == "source-b")));
    assert_eq!(aligner.current_source_id.as_deref(), Some("source-b"));
    assert!(aligner.validate_state().is_ok());
}

#[test]
fn observed_source_ids_stay_bounded_unique_and_survive_retirement_until_clear() {
    let mut aligner = LivePairAligner::default();
    for index in 0..MAX_TRACKED_ITEMS + 8 {
        let source = format!("source-{index}");
        let response = format!("response-{index}");
        aligner.observe_at(
            &LivePairEvent::ItemCreated,
            &identity(&response, Some(&source)),
            0,
        );
        aligner.observe_at(
            &LivePairEvent::SourceFinal {
                text: "Synthetic source.".into(),
                language: None,
            },
            &identity(&source, None),
            1,
        );
        aligner.observe_at(
            &LivePairEvent::TranslationFinal {
                text: "Synthetic translation.".into(),
            },
            &identity(&response, None),
            2,
        );
        assert!(aligner.validate_state().is_ok());
    }
    assert_eq!(aligner.tracked_utterance_count(), 0);
    assert_eq!(aligner.source_order.len(), MAX_TRACKED_ITEMS);
    let previous = format!("source-{}", MAX_TRACKED_ITEMS + 6);
    let current = format!("source-{}", MAX_TRACKED_ITEMS + 7);
    for _ in 0..4 {
        aligner.observe_at(
            &LivePairEvent::SourceFinal {
                text: "Synthetic old source.".into(),
                language: None,
            },
            &identity(&previous, None),
            3,
        );
        assert_eq!(aligner.current_source_id.as_ref(), Some(&current));
        assert_eq!(aligner.source_order.len(), MAX_TRACKED_ITEMS);
        assert!(aligner.validate_state().is_ok());
    }
    let mut forged = serde_json::to_value(&aligner).unwrap();
    forged["source_order"] = serde_json::json!([current, current]);
    assert_eq!(
        serde_json::from_value::<LivePairAligner>(forged)
            .unwrap()
            .validate_state(),
        Err("live_pair_state_invalid")
    );
    aligner.clear_content();
    assert!(aligner.source_order.is_empty());
    assert!(aligner.current_source_id.is_none());
    assert!(aligner
        .observe_at(
            &LivePairEvent::SourceFinal {
                text: "Synthetic stale source.".into(),
                language: None
            },
            &identity(&previous, None),
            4
        )
        .is_empty());
    assert!(aligner.validate_state().is_ok());
}

#[test]
fn known_orphans_and_cleared_tombstones_remain_bounded_through_restoration() {
    let mut aligner = LivePairAligner::default();
    for index in 0..MAX_TRACKED_ITEMS * 3 {
        let response = format!("response-{index}");
        let source = format!("source-{index}");
        aligner.observe_at(
            &LivePairEvent::ItemCreated,
            &identity(&response, Some(&source)),
            0,
        );
        assert!(aligner
            .observe_at(
                &LivePairEvent::TranslationFinal {
                    text: "译文".into()
                },
                &identity(&response, None),
                1,
            )
            .is_empty());
    }
    assert_eq!(aligner.response_count(), MAX_TRACKED_ITEMS);
    assert_eq!(aligner.tracked_utterance_count(), MAX_TRACKED_ITEMS);
    assert!(aligner.validate_state().is_ok());
    aligner.clear_content();
    assert_eq!(aligner.response_count(), 0);
    assert_eq!(aligner.tracked_utterance_count(), 0);
    assert_eq!(aligner.discarded_item_count(), MAX_TRACKED_ITEMS);
    assert!(aligner.validate_state().is_ok());
    let restored: LivePairAligner =
        serde_json::from_str(&serde_json::to_string(&aligner).unwrap()).unwrap();
    assert_eq!(restored.discarded_item_count(), MAX_TRACKED_ITEMS);
    assert!(restored.validate_state().is_ok());
}

#[test]
fn restored_state_rejects_forged_or_duplicate_eviction_indexes() {
    let mut aligner = LivePairAligner::default();
    aligner.observe_at(
        &LivePairEvent::SourceDraft {
            text: "Valid preview".into(),
            language: None,
        },
        &identity("source", None),
        0,
    );
    let valid = serde_json::to_value(aligner).unwrap();
    for bad_order in [
        serde_json::json!([]),
        serde_json::json!(["source", "source"]),
    ] {
        let mut forged = valid.clone();
        forged["utterance_order"] = bad_order;
        let restored: LivePairAligner = serde_json::from_value(forged).unwrap();
        assert_eq!(restored.validate_state(), Err("live_pair_state_invalid"));
    }
}

#[test]
fn restored_state_rejects_oversized_retained_fields_and_tombstone_counts() {
    let oversized = "a".repeat(MAX_SUBTITLE_TEXT_BYTES + 1);
    let mut aligner = LivePairAligner::default();
    aligner.discard_item(oversized.clone());
    assert_eq!(aligner.validate_state(), Err("live_pair_state_invalid"));
    let mut aligner = LivePairAligner::default();
    aligner.observe_at(
        &LivePairEvent::SourceDraft {
            text: oversized,
            language: None,
        },
        &identity("source", None),
        0,
    );
    assert_eq!(aligner.validate_state(), Err("live_pair_state_invalid"));
    let mut state = serde_json::to_value(LivePairAligner::default()).unwrap();
    state["discarded_items"] = serde_json::json!((0..=MAX_TRACKED_ITEMS)
        .map(|i| format!("discarded-{i}"))
        .collect::<Vec<_>>());
    let restored: LivePairAligner = serde_json::from_value(state).unwrap();
    assert_eq!(restored.validate_state(), Err("live_pair_state_invalid"));
}

#[test]
fn event_projection_preserves_exact_subtitle_role_and_full_text() {
    let event = LivePairEvent::UtteranceText {
        utterance_id: "source-ja".into(),
        role: UtteranceRole::Translation,
        text: "超级大回转，完整句子。".into(),
        is_final: false,
        language: None,
    };
    assert_eq!(
        event.subtitle_event(),
        Some(SubtitleEvent::UtteranceText {
            utterance_id: "source-ja".into(),
            role: UtteranceRole::Translation,
            text: "超级大回转，完整句子。".into(),
            is_final: false,
        })
    );
    assert_eq!(LivePairEvent::SessionFinished.subtitle_event(), None);
}
