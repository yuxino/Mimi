use super::*;
use crate::models::{SubtitleLine, SubtitlePair};

type SubtitleReducer = super::SubtitleReducer<BoundedTestArchive>;

#[test]
fn translation_start_requires_a_meaningful_current_unconfirmed_source() {
    let mut reducer = SubtitleReducer::default();
    assert!(reducer.can_start_identified_translation("current", "Synthetic source"));
    assert!(reducer.register_source_order("current", 2));
    reducer.apply(identified_text(
        "current",
        UtteranceRole::Source,
        "Synthetic source",
    ));
    assert!(reducer.register_source_order("unseen-old", 1));
    assert!(!reducer.can_start_identified_translation("unseen-old", "Synthetic old source"));
    reducer.discard_source_id("discarded".into());
    let oversized = "x".repeat(crate::models::MAX_SUBTITLE_TEXT_BYTES + 1);
    for (id, text) in [
        ("", "Synthetic source"),
        (" \u{3000}", "Synthetic source"),
        ("current", ""),
        ("current", " \u{3000}"),
        ("discarded", "Synthetic source"),
        (oversized.as_str(), "Synthetic source"),
        ("current", oversized.as_str()),
    ] {
        assert!(!reducer.can_start_identified_translation(id, text));
    }
    reducer.apply(SubtitleEvent::IdentifiedFinalPair {
        utterance_id: "current".into(),
        source: "Synthetic source".into(),
        translation: "Synthetic translation".into(),
    });
    assert!(!reducer.can_start_identified_translation("current", "Synthetic revised source"));
    assert!(reducer.can_start_identified_translation("newer", "Synthetic next source"));
}

#[test]
fn translation_start_cannot_admit_an_unknown_identity_after_ordinal_exhaustion() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(identified_text(
        "known",
        UtteranceRole::Source,
        "Synthetic source",
    ));
    reducer.next_source_ordinal = u64::MAX - 1;
    assert!(reducer.validate_state(100).is_ok());
    assert!(reducer.can_start_identified_translation("known", "Synthetic revised source"));
    assert!(!reducer.can_start_identified_translation("unknown", "Synthetic next source"));
    let before = reducer.snapshot.clone();
    reducer.apply(identified_text(
        "unknown",
        UtteranceRole::Source,
        "Synthetic next source",
    ));
    assert_eq!(reducer.snapshot, before);
}

#[test]
fn source_order_limits_and_exhausted_auto_order_keep_state_restorable() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceBoundary {
        utterance_id: "clock-limit".into(),
        source_order: i64::MAX as u64,
    });
    reducer.apply(SubtitleEvent::SourceBoundary {
        utterance_id: "invalid-clock".into(),
        source_order: i64::MAX as u64 + 1,
    });
    assert_eq!(reducer.identified_sources.len(), 1);
    reducer.apply(SubtitleEvent::UtteranceText {
        utterance_id: "automatic".into(),
        role: UtteranceRole::Source,
        text: "Synthetic current".into(),
        is_final: false,
    });
    assert_eq!(reducer.next_source_ordinal, i64::MAX as u64 + 1);
    assert!(reducer.validate_state(100).is_ok());
    reducer.next_source_ordinal = u64::MAX - 1;
    let before = serde_json::to_value(&reducer).unwrap();
    reducer.apply(SubtitleEvent::UtteranceText {
        utterance_id: "exhausted".into(),
        role: UtteranceRole::Source,
        text: "Synthetic rejected".into(),
        is_final: false,
    });
    reducer.apply(SubtitleEvent::IdentifiedFinalPair {
        utterance_id: "exhausted".into(),
        source: "Synthetic rejected".into(),
        translation: "Synthetic rejected translation".into(),
    });
    assert_eq!(serde_json::to_value(&reducer).unwrap(), before);
    assert!(reducer.validate_state(100).is_ok());
}

fn identified_text(id: &str, role: UtteranceRole, text: &str) -> SubtitleEvent {
    SubtitleEvent::UtteranceText {
        utterance_id: id.into(),
        role,
        text: text.into(),
        is_final: false,
    }
}

fn identified_pair(id: &str, text: &str) -> SubtitleEvent {
    SubtitleEvent::IdentifiedFinalPair {
        utterance_id: id.into(),
        source: text.into(),
        translation: format!("Translated {text}"),
    }
}

#[test]
fn known_order_is_independent_of_unknown_arrival_ordinals_and_cannot_be_retargeted() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(identified_text(
        "unknown",
        UtteranceRole::Source,
        "Synthetic unknown",
    ));
    assert!(reducer.register_source_order("known-z", 1));
    reducer.apply(identified_text(
        "known-z",
        UtteranceRole::Source,
        "Synthetic known",
    ));
    assert_eq!(
        reducer.snapshot.source.utterance_id.as_deref(),
        Some("known-z")
    );
    assert!(!reducer.register_source_order("known-z", 2));
    assert!(!reducer.register_source_order("other", 1));
    assert!(reducer.register_source_order("known-a", 2));
    reducer.apply(identified_text(
        "known-a",
        UtteranceRole::Source,
        "Synthetic newer",
    ));
    // Missing metadata cannot erase the rank learned for this identity.
    reducer.apply(identified_text(
        "known-z",
        UtteranceRole::Source,
        "Synthetic old replay",
    ));
    assert_eq!(reducer.snapshot.source.text, "Synthetic newer");
    assert!(reducer.validate_state(100).is_ok());
}

#[test]
fn old_known_pairs_keep_history_and_the_retained_pair_without_rewinding_live_lanes() {
    let mut reducer = SubtitleReducer::default();
    assert!(reducer.register_source_order("current", 2));
    reducer.apply(identified_text(
        "current",
        UtteranceRole::Source,
        "Synthetic current",
    ));
    reducer.apply(identified_text(
        "current",
        UtteranceRole::Translation,
        "Current preview",
    ));
    assert!(reducer.register_source_order("previous", 1));
    for role in [UtteranceRole::Source, UtteranceRole::Translation] {
        reducer.apply(identified_text("previous", role, "Synthetic stale preview"));
    }
    assert_eq!(reducer.snapshot.source.text, "Synthetic current");
    assert_eq!(reducer.snapshot.translation.text, "Current preview");
    reducer.apply(identified_pair("previous", "Synthetic previous complete"));
    assert_eq!(reducer.snapshot.source.text, "Synthetic current");
    assert_eq!(
        reducer
            .snapshot
            .display_pair
            .as_ref()
            .unwrap()
            .utterance_id
            .as_deref(),
        Some("previous")
    );
    reducer.apply(identified_pair("current", "Synthetic current complete"));
    reducer.apply(identified_pair("previous", "Synthetic duplicate"));
    assert_eq!(reducer.snapshot.source.text, "Synthetic current complete");
    assert_eq!(
        reducer
            .snapshot
            .display_pair
            .as_ref()
            .unwrap()
            .utterance_id
            .as_deref(),
        Some("current")
    );
    assert_eq!(reducer.snapshot.history.len(), 2);
    assert!(reducer.validate_state(100).is_ok());
}

#[test]
fn known_correction_does_not_reopen_an_older_unknown_source_or_display() {
    let mut reducer = SubtitleReducer::default();
    assert!(reducer.register_source_order("known-a", 3));
    reducer.register_source_identity("unknown-old");
    assert!(reducer.register_source_order("known-z", 2));
    reducer.apply(identified_text(
        "known-z",
        UtteranceRole::Source,
        "Synthetic second",
    ));
    reducer.apply(identified_pair("known-z", "Synthetic second complete"));
    reducer.apply(identified_text(
        "known-a",
        UtteranceRole::Source,
        "Synthetic newest",
    ));
    reducer.apply(identified_pair("known-a", "Synthetic newest complete"));
    reducer.apply(identified_text(
        "unknown-old",
        UtteranceRole::Source,
        "Synthetic old unknown",
    ));
    reducer.apply(identified_pair(
        "unknown-old",
        "Synthetic old unknown complete",
    ));
    assert_eq!(reducer.snapshot.source.text, "Synthetic newest complete");
    assert_eq!(
        reducer
            .snapshot
            .display_pair
            .as_ref()
            .unwrap()
            .utterance_id
            .as_deref(),
        Some("known-a")
    );
    assert_eq!(reducer.snapshot.history.len(), 3);
    reducer.apply(identified_text(
        "unknown-new",
        UtteranceRole::Source,
        "Synthetic new unknown",
    ));
    assert_eq!(reducer.snapshot.source.text, "Synthetic new unknown");
    reducer.apply(identified_text(
        "known-z",
        UtteranceRole::Source,
        "Synthetic late known",
    ));
    assert_eq!(reducer.snapshot.source.text, "Synthetic new unknown");
    assert!(reducer.validate_state(100).is_ok());
}

#[test]
fn legacy_order_state_restores_and_rank_scope_resets_without_losing_confirmed_history() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(identified_text(
        "old",
        UtteranceRole::Source,
        "Synthetic old",
    ));
    reducer.apply(identified_text(
        "current",
        UtteranceRole::Source,
        "Synthetic current",
    ));
    let mut legacy = serde_json::to_value(&reducer).unwrap();
    for field in ["latest_known_source_order", "display_known_source_order"] {
        legacy.as_object_mut().unwrap().remove(field);
    }
    for identity in legacy["identified_sources"].as_array_mut().unwrap() {
        identity.as_object_mut().unwrap().remove("known_order");
    }
    let mut restored: SubtitleReducer = serde_json::from_value(legacy).unwrap();
    assert!(restored.validate_state(100).is_ok());
    assert!(restored.register_source_order("current", 100));
    assert!(restored.register_source_order("old", 99));
    restored.apply(identified_text(
        "old",
        UtteranceRole::Source,
        "Synthetic late old",
    ));
    assert_eq!(restored.snapshot.source.text, "Synthetic current");
    restored.apply(identified_pair("current", "Synthetic confirmed"));
    restored.reset_transient();
    assert_eq!(restored.snapshot.history.len(), 1);
    assert!(restored.register_source_order("next", 1));
    restored.apply(identified_text(
        "next",
        UtteranceRole::Source,
        "Synthetic new scope",
    ));
    assert_eq!(restored.snapshot.source.text, "Synthetic new scope");
    let state = serde_json::to_value(&restored).unwrap();
    let mut restored: SubtitleReducer = serde_json::from_value(state).unwrap();
    restored.apply(SubtitleEvent::Clear);
    assert!(restored.snapshot.history.is_empty());
    assert!(!restored.register_source_order("next", 1));
    assert!(restored.register_source_order("after-clear", 1));
    restored.apply(identified_text(
        "after-clear",
        UtteranceRole::Source,
        "Synthetic after clear",
    ));
    assert_eq!(restored.snapshot.source.text, "Synthetic after clear");
    assert!(restored.validate_state(100).is_ok());
}

#[test]
fn late_order_for_translation_only_owner_covers_both_domains_and_its_successor() {
    let mut reducer = SubtitleReducer::default();
    assert!(reducer.register_source_order("future", 3));
    reducer.register_source_identity("old-unknown");
    reducer.apply(identified_text(
        "current",
        UtteranceRole::Translation,
        "Synthetic current translation",
    ));
    assert!(reducer.register_source_order("current", 2));
    assert!(reducer.register_source_order("previous", 1));
    for id in ["previous", "old-unknown"] {
        reducer.apply(identified_text(
            id,
            UtteranceRole::Translation,
            "Synthetic old translation",
        ));
        assert_eq!(
            reducer.snapshot.translation.text,
            "Synthetic current translation"
        );
    }
    reducer.apply(identified_text(
        "future",
        UtteranceRole::Translation,
        "Synthetic newer translation",
    ));
    assert_eq!(
        reducer.snapshot.translation.text,
        "Synthetic newer translation"
    );
    assert!(reducer.validate_state(100).is_ok());
}

#[test]
fn retained_source_does_not_hide_the_visible_translation_comparison_domain() {
    for old_known in [false, true] {
        let mut reducer = SubtitleReducer::default();
        if old_known {
            assert!(reducer.register_source_order("retained", 1));
        }
        reducer.apply(identified_pair("retained", "Synthetic retained complete"));
        assert!(reducer.register_source_order("future", 3));
        assert!(reducer.register_source_order("current", 2));
        reducer.apply(identified_text(
            "current",
            UtteranceRole::Translation,
            "Synthetic current translation",
        ));
        assert_eq!(
            reducer
                .snapshot
                .realtime_preview
                .as_ref()
                .unwrap()
                .translation
                .text,
            "Synthetic current translation"
        );
        assert!(reducer.validate_state(100).is_ok());
        reducer.register_source_identity("unknown-new");
        reducer.apply(identified_text(
            "unknown-new",
            UtteranceRole::Translation,
            "Synthetic new unknown translation",
        ));
        reducer.apply(identified_text(
            "future",
            UtteranceRole::Translation,
            "Synthetic early unknown replacement",
        ));
        assert_eq!(
            reducer.snapshot.translation.text,
            "Synthetic new unknown translation"
        );
        assert!(reducer.validate_state(100).is_ok());
    }
}

#[test]
fn evicting_the_current_known_identity_keeps_its_comparison_domain() {
    for display in [false, true] {
        let mut reducer = SubtitleReducer::default();
        assert!(reducer.register_source_order("current", 3));
        assert!(reducer.register_source_order("future", 4));
        for index in 3..=64 {
            reducer.register_source_identity(&format!("unknown-{index}"));
        }
        assert!(reducer.register_source_order("unknown-64", 2));
        reducer.apply(if display {
            identified_pair("unknown-64", "Synthetic earlier")
        } else {
            identified_text("unknown-64", UtteranceRole::Source, "Synthetic earlier")
        });
        reducer.apply(if display {
            identified_pair("current", "Synthetic current")
        } else {
            identified_text("current", UtteranceRole::Source, "Synthetic current")
        });
        reducer.register_source_identity("extra");
        assert!(!reducer
            .identified_sources
            .iter()
            .any(|item| item.id == "current"));
        reducer.apply(if display {
            identified_pair("future", "Synthetic newer")
        } else {
            identified_text("future", UtteranceRole::Source, "Synthetic newer")
        });
        assert_eq!(reducer.snapshot.source.text, "Synthetic newer");
        if display {
            assert_eq!(
                reducer.snapshot.display_pair.as_ref().unwrap().source,
                "Synthetic newer"
            );
        }
        assert!(reducer.validate_state(100).is_ok());
    }
}

#[test]
fn restored_known_frontiers_cannot_lag_their_visible_owners() {
    let mut reducer = SubtitleReducer::default();
    assert!(reducer.register_source_order("current", 100));
    reducer.apply(identified_pair("current", "Synthetic current"));
    for field in ["latest_known_source_order", "display_known_source_order"] {
        for forged in [serde_json::Value::Null, serde_json::json!(1)] {
            let mut state = serde_json::to_value(&reducer).unwrap();
            state[field] = forged;
            let restored: SubtitleReducer = serde_json::from_value(state).unwrap();
            assert!(restored.validate_state(100).is_err(), "{field}");
        }
    }
    let mut stable = SubtitleReducer::default();
    assert!(stable.register_source_order("maximum-stable", i64::MAX as u64));
    assert_eq!(stable.next_source_ordinal, 1);
    assert!(stable.validate_state(100).is_ok());
}

#[test]
fn known_order_state_stays_bounded_and_rejects_forged_duplicate_ranks() {
    let mut reducer = SubtitleReducer::default();
    for order in 1..=200 {
        let id = format!("synthetic-{order}");
        assert!(reducer.register_source_order(&id, order));
        reducer.apply(identified_text(
            &id,
            UtteranceRole::Source,
            "Synthetic bounded",
        ));
    }
    assert_eq!(reducer.identified_sources.len(), MAX_IDENTIFIED_SOURCES);
    assert_eq!(reducer.discarded_source_ids.len(), MAX_IDENTIFIED_SOURCES);
    assert!(reducer.validate_state(100).is_ok());
    let mut forged = serde_json::to_value(&reducer).unwrap();
    forged["identified_sources"][1]["known_order"] =
        forged["identified_sources"][0]["known_order"].clone();
    let forged: SubtitleReducer = serde_json::from_value(forged).unwrap();
    assert!(forged.validate_state(100).is_err());
}

#[derive(Default)]
struct BoundedTestArchive {
    enabled: bool,
    pairs: Vec<SubtitlePair>,
    bytes: usize,
}
impl BoundedTestArchive {
    fn begin(&mut self, enabled: bool, _started_at_ms: u64) {
        *self = Self {
            enabled,
            ..Self::default()
        };
    }
    fn count(&self) -> usize {
        self.pairs.len()
    }
}
impl ArchiveSink for BoundedTestArchive {
    fn append(&mut self, pair: &SubtitlePair) {
        let bytes = pair.source.len().saturating_add(pair.translation.len());
        if self.enabled
            && self.pairs.len() < 128
            && bytes <= (1024usize * 1024).saturating_sub(self.bytes)
        {
            self.bytes += bytes;
            self.pairs.push(pair.clone());
        }
    }
    fn clear(&mut self) {
        self.begin(self.enabled, 0);
    }
}

fn confirmed(utterance_id: u64, text: &str) -> SubtitleEvent {
    SubtitleEvent::ConfirmedPair {
        source_utterance_id: None,
        utterance_id,
        source: text.into(),
        translation: text.into(),
    }
}

#[test]
fn an_older_confirmation_appends_history_without_replacing_a_newer_complete_pair() {
    let mut reducer = SubtitleReducer::default();
    reducer.archive.begin(true, 0);
    reducer.apply(SubtitleEvent::SourceUtteranceDraft {
        utterance_id: 8,
        text: "Synthetic next source".into(),
    });
    reducer.apply(SubtitleEvent::PreviewPair {
        source_utterance_id: Some(8),
        source: "Synthetic next source".into(),
        translation: "Synthetic next translation".into(),
    });
    let source = reducer.snapshot.source.clone();
    let preview = reducer.snapshot.preview_pair.clone();
    assert_eq!(preview.as_ref().unwrap().utterance_id, source.utterance_id);
    reducer.apply(SubtitleEvent::ConfirmedPair {
        utterance_id: 1,
        source_utterance_id: Some(7),
        source: "Synthetic previous source".into(),
        translation: "Synthetic previous translation".into(),
    });
    assert_eq!(reducer.snapshot.source, source);
    assert_eq!(reducer.snapshot.preview_pair, preview);
    assert_eq!(reducer.snapshot.history.len(), 1);
    assert_eq!(
        reducer.snapshot.history[0].source,
        "Synthetic previous source"
    );
    assert_eq!(reducer.archive.count(), 1);
    for obsolete in [
        SubtitleEvent::SourceUtteranceDraft {
            utterance_id: 7,
            text: "Synthetic stale draft".into(),
        },
        SubtitleEvent::PreviewPair {
            source_utterance_id: Some(7),
            source: "Synthetic stale source".into(),
            translation: "Synthetic stale translation".into(),
        },
    ] {
        reducer.apply(obsolete);
        assert_eq!(reducer.snapshot.source, source);
        assert_eq!(reducer.snapshot.preview_pair, preview);
    }
    reducer.apply(SubtitleEvent::ConfirmedPair {
        utterance_id: 2,
        source_utterance_id: Some(8),
        source: "Synthetic next source".into(),
        translation: "Synthetic next translation".into(),
    });
    assert!(reducer.snapshot.preview_pair.is_none());
    assert!(reducer.snapshot.source.is_final);
    assert_eq!(reducer.snapshot.source.text, "Synthetic next source");
    assert_eq!(
        reducer.snapshot.source.utterance_id,
        reducer.snapshot.display_pair.as_ref().unwrap().utterance_id
    );
    assert_eq!(
        reducer.snapshot.translation.utterance_id,
        reducer.snapshot.source.utterance_id
    );
    assert_eq!(reducer.snapshot.history.len(), 2);
    reducer.apply(SubtitleEvent::Clear);
    reducer.apply(SubtitleEvent::SourceUtteranceDraft {
        utterance_id: 8,
        text: "Synthetic cleared replay".into(),
    });
    assert!(reducer.snapshot.source.text.is_empty());
    reducer.apply(SubtitleEvent::SourceUtteranceDraft {
        utterance_id: 9,
        text: "Synthetic repeated lyric".into(),
    });
    assert_eq!(reducer.snapshot.source.text, "Synthetic repeated lyric");
    let before_reset = reducer.snapshot.source.utterance_id.clone();
    reducer.reset_transient();
    reducer.apply(SubtitleEvent::SourceUtteranceDraft {
        utterance_id: 1,
        text: "Synthetic new task".into(),
    });
    assert_eq!(reducer.snapshot.source.text, "Synthetic new task");
    assert_ne!(reducer.snapshot.source.utterance_id, before_reset);
}

#[test]
fn confirming_a_preview_clears_it_while_retaining_the_next_raw_sentence() {
    for next_source in ["", "Next sentence begins"] {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceUtteranceDraft {
            utterance_id: 7,
            text: "Earlier partial sentence".into(),
        });
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: Some(7),
            source: "Earlier partial sentence".into(),
            translation: "较早的完整预览".into(),
        });
        reducer.apply(SubtitleEvent::SourceUtteranceDraft {
            utterance_id: 8,
            text: next_source.into(),
        });
        let newer_raw = reducer.snapshot.source.clone();

        reducer.apply(SubtitleEvent::ConfirmedPair {
            utterance_id: 1,
            source_utterance_id: Some(7),
            source: "Earlier corrected complete sentence.".into(),
            translation: "较早句子的最终译文。".into(),
        });

        assert_eq!(reducer.snapshot.source, newer_raw);
        assert!(!reducer.snapshot.source.is_final);
        assert!(
            reducer.snapshot.preview_pair.is_none(),
            "a newer raw sentence cannot keep the confirmed sentence's old preview"
        );
        assert_eq!(reducer.snapshot.history.len(), 1);
        assert_eq!(
            reducer.snapshot.history[0].source,
            "Earlier corrected complete sentence."
        );
        assert_eq!(
            reducer.snapshot.history[0].translation,
            "较早句子的最终译文。"
        );
        assert_eq!(reducer.preview_source_utterance_id, None);
        assert_eq!(reducer.latest_source_utterance_id, Some(8));
    }
}

#[test]
fn completed_pair_ownership_is_stable_on_revisions_and_changes_after_clear_or_reset() {
    let mut reducer = SubtitleReducer::default();
    let draft = || SubtitleEvent::SourceUtteranceDraft {
        utterance_id: 8,
        text: "Synthetic source".into(),
    };
    let preview = || SubtitleEvent::PreviewPair {
        source_utterance_id: Some(8),
        source: "Synthetic source".into(),
        translation: "Synthetic translation".into(),
    };
    reducer.apply(draft());
    reducer.apply(preview());
    let first = reducer
        .snapshot
        .preview_pair
        .as_ref()
        .unwrap()
        .utterance_id
        .clone();
    assert_eq!(first, reducer.snapshot.source.utterance_id);
    reducer.apply(SubtitleEvent::PreviewPair {
        source_utterance_id: Some(8),
        source: "Synthetic revised source".into(),
        translation: "Synthetic revised translation".into(),
    });
    assert_eq!(
        reducer.snapshot.preview_pair.as_ref().unwrap().utterance_id,
        first
    );
    reducer.apply(SubtitleEvent::Clear);
    reducer.apply(draft());
    reducer.apply(preview());
    let second = reducer
        .snapshot
        .preview_pair
        .as_ref()
        .unwrap()
        .utterance_id
        .clone();
    assert_ne!(second, first);
    assert_eq!(second, reducer.snapshot.source.utterance_id);
    reducer.reset_transient();
    reducer.apply(draft());
    reducer.apply(preview());
    assert_ne!(
        reducer.snapshot.preview_pair.as_ref().unwrap().utterance_id,
        second
    );
    let json = serde_json::to_value(&reducer.snapshot.preview_pair).unwrap();
    assert!(json["utteranceId"].is_string());
    reducer.apply(SubtitleEvent::PreviewPair {
        source_utterance_id: None,
        source: "Synthetic legacy source".into(),
        translation: "Synthetic legacy translation".into(),
    });
    let json = serde_json::to_value(&reducer.snapshot.preview_pair).unwrap();
    assert!(json.get("utteranceId").is_none());
}

#[test]
fn every_oversized_caption_preserves_complete_pair_and_confirmation_identity() {
    let mut reducer = SubtitleReducer::default();
    reducer.archive.begin(true, 0);
    reducer.apply(confirmed(1, "Synthetic final"));
    reducer.apply(SubtitleEvent::PreviewPair {
        source_utterance_id: None,
        source: "Synthetic complete preview".into(),
        translation: "Synthetic complete translation".into(),
    });
    let expected = reducer.snapshot.clone();
    let oversized = "x".repeat(crate::models::MAX_SUBTITLE_TEXT_BYTES + 1);
    let events = [
        SubtitleEvent::SourceDraft(oversized.clone()),
        SubtitleEvent::SourceFinal(oversized.clone()),
        SubtitleEvent::TranslationDraft(oversized.clone()),
        SubtitleEvent::TranslationFinal(oversized.clone()),
        SubtitleEvent::UtteranceText {
            utterance_id: "synthetic-id".into(),
            role: UtteranceRole::Source,
            text: oversized.clone(),
            is_final: false,
        },
        SubtitleEvent::UtteranceText {
            utterance_id: "synthetic-id".into(),
            role: UtteranceRole::Translation,
            text: oversized.clone(),
            is_final: true,
        },
        SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source: oversized.clone(),
            translation: "valid".into(),
        },
        SubtitleEvent::FinalPair {
            source: "valid".into(),
            translation: oversized.clone(),
        },
        SubtitleEvent::ConfirmedPair {
            source_utterance_id: None,
            utterance_id: 2,
            source: oversized.clone(),
            translation: "valid".into(),
        },
        SubtitleEvent::ConfirmedPair {
            source_utterance_id: None,
            utterance_id: 2,
            source: "valid".into(),
            translation: oversized,
        },
    ];
    for event in events {
        reducer.apply(event);
        assert_eq!(reducer.snapshot, expected);
        assert_eq!(reducer.archive.count(), 1);
    }
    let exact = "🙂".repeat(crate::models::MAX_SUBTITLE_TEXT_BYTES / 4);
    reducer.apply(confirmed(2, &exact));
    assert_eq!(reducer.snapshot.source.text, exact);
    assert_eq!(reducer.snapshot.history.len(), 2);
    assert_eq!(reducer.archive.count(), 2);
}

#[test]
fn confirmed_identity_preserves_repeated_text_and_rejects_older_or_duplicate_ids() {
    let mut reducer = SubtitleReducer::new(2);
    reducer.archive.begin(true, 0);
    for id in [1, 2, 3] {
        reducer.apply(confirmed(id, "Synthetic repeated line"));
    }
    assert_eq!(reducer.snapshot.history.len(), 2);
    assert_eq!(reducer.archive.count(), 3);
    let snapshot = reducer.snapshot.clone();
    for id in [3, 1, 2] {
        reducer.apply(confirmed(id, "Stale synthetic revision"));
    }
    assert_eq!(reducer.snapshot, snapshot);
    assert_eq!(reducer.archive.count(), 3);
}

#[test]
fn clear_keeps_identity_watermark_and_reconnect_resets_it_with_history_preserved() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(confirmed(9, "Synthetic line"));
    reducer.apply(SubtitleEvent::Clear);
    reducer.apply(confirmed(9, "Synthetic replay"));
    assert!(reducer.snapshot.history.is_empty());
    reducer.apply(confirmed(10, "New synthetic line"));
    assert_eq!(reducer.snapshot.history.len(), 1);
    reducer.reset_transient();
    reducer.apply(confirmed(0, "New generation synthetic line"));
    assert_eq!(reducer.snapshot.history.len(), 2);
}

#[test]
fn empty_confirmation_does_not_claim_identity_and_wrapped_serials_remain_ordered() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(confirmed(u64::MAX, " \n"));
    reducer.apply(confirmed(u64::MAX, "Synthetic line"));
    reducer.apply(confirmed(1, "Synthetic line"));
    reducer.apply(confirmed(u64::MAX, "Obsolete line"));
    assert_eq!(reducer.snapshot.history.len(), 2);
    assert_eq!(reducer.snapshot.source.text, "Synthetic line");
}

#[test]
fn subtitle_reducer_starts_empty() {
    let reducer = SubtitleReducer::default();
    assert_eq!(reducer.snapshot, SubtitleSnapshot::empty());
}

#[test]
fn drafts_remain_visibly_unconfirmed() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceDraft("Hello wor".into()));
    reducer.apply(SubtitleEvent::TranslationDraft("你好，世".into()));

    assert_eq!(
        reducer.snapshot.source,
        SubtitleLine::new("Hello wor", false)
    );
    assert_eq!(
        reducer.snapshot.translation,
        SubtitleLine::new("你好，世", false)
    );
}

#[test]
fn a_complete_preview_survives_new_raw_drafts_without_pairing_them() {
    let mut reducer = SubtitleReducer::default();
    reducer.archive.begin(true, 0);
    reducer.apply(SubtitleEvent::SourceDraft("Old recognized source".into()));
    reducer.apply(SubtitleEvent::TranslationDraft(
        "Old raw translation".into(),
    ));
    let stable = PreviewSubtitlePair {
        utterance_id: None,
        source: "Old recognized source".into(),
        translation: "Complete translated preview".into(),
    };
    reducer.apply(SubtitleEvent::PreviewPair {
        source_utterance_id: None,
        source: stable.source.clone(),
        translation: stable.translation.clone(),
    });
    reducer.apply(SubtitleEvent::SourceDraft(
        "New recognition still growing".into(),
    ));
    reducer.apply(SubtitleEvent::TranslationDraft(String::new()));
    reducer.apply(SubtitleEvent::SourceFinal(
        "New recognition revised.".into(),
    ));

    assert_eq!(reducer.snapshot.preview_pair, Some(stable));
    assert_eq!(reducer.snapshot.source.text, "New recognition revised.");
    assert!(reducer.snapshot.translation.text.is_empty());
    assert!(reducer.snapshot.history.is_empty());
    assert_eq!(reducer.archive.count(), 0);
}

#[test]
fn preview_pairs_replace_one_slot_without_rewinding_raw_asr_or_entering_history() {
    let mut reducer = SubtitleReducer::new(2);
    reducer.archive.begin(true, 0);
    reducer.apply(SubtitleEvent::SourceDraft("Latest ASR draft".into()));
    reducer.apply(SubtitleEvent::TranslationDraft("Latest raw draft".into()));
    for index in 0..2_000 {
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source: format!("Earlier candidate {index}"),
            translation: format!("Completed preview {index}"),
        });
    }
    assert_eq!(
        reducer.snapshot.preview_pair,
        Some(PreviewSubtitlePair {
            utterance_id: None,
            source: "Earlier candidate 1999".into(),
            translation: "Completed preview 1999".into(),
        })
    );
    assert_eq!(reducer.snapshot.source.text, "Latest ASR draft");
    assert_eq!(reducer.snapshot.translation.text, "Latest raw draft");
    assert!(reducer.snapshot.history.is_empty());
    assert_eq!(reducer.archive.count(), 0);
}

#[test]
fn empty_or_oversized_preview_pairs_keep_the_last_complete_pair() {
    let mut reducer = SubtitleReducer::default();
    let stable = PreviewSubtitlePair {
        utterance_id: None,
        source: "Stable source".into(),
        translation: "Stable translation".into(),
    };
    reducer.apply(SubtitleEvent::PreviewPair {
        source_utterance_id: None,
        source: stable.source.clone(),
        translation: stable.translation.clone(),
    });
    for (source, translation) in [
        (" \n\t".into(), "Translation".into()),
        ("Source".into(), " \n\t".into()),
        ("x".repeat(MAX_PREVIEW_TEXT_BYTES + 1), "Translation".into()),
        ("Source".into(), "字".repeat(MAX_PREVIEW_TEXT_BYTES / 3 + 1)),
    ] {
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source,
            translation,
        });
        assert_eq!(reducer.snapshot.preview_pair, Some(stable.clone()));
    }
    assert!(reducer.snapshot.history.is_empty());
}

#[test]
fn complete_preview_text_at_the_byte_limit_is_not_truncated() {
    let mut reducer = SubtitleReducer::default();
    let source = format!("{}a", "字".repeat(MAX_PREVIEW_TEXT_BYTES / 3));
    let translation = "🙂".repeat(MAX_PREVIEW_TEXT_BYTES / 4);
    assert_eq!(source.len(), MAX_PREVIEW_TEXT_BYTES);
    assert_eq!(translation.len(), MAX_PREVIEW_TEXT_BYTES);
    reducer.apply(SubtitleEvent::PreviewPair {
        source_utterance_id: None,
        source: source.clone(),
        translation: translation.clone(),
    });
    assert_eq!(
        reducer.snapshot.preview_pair,
        Some(PreviewSubtitlePair {
            utterance_id: None,
            source,
            translation
        })
    );
}

#[test]
fn valid_final_boundaries_replace_the_preview_even_when_source_was_revised() {
    for final_event in [
        SubtitleEvent::FinalPair {
            source: "Corrected source.".into(),
            translation: "Confirmed translation.".into(),
        },
        SubtitleEvent::TranslationFinal("Confirmed translation.".into()),
        SubtitleEvent::UtteranceText {
            utterance_id: "confirmed-source".into(),
            role: UtteranceRole::Translation,
            text: "Confirmed translation.".into(),
            is_final: true,
        },
    ] {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceFinal("Corrected source.".into()));
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source: "Provisional source".into(),
            translation: "Provisional translation".into(),
        });
        reducer.apply(final_event);
        assert!(reducer.snapshot.preview_pair.is_none());
        assert_eq!(reducer.snapshot.history.len(), 1);
        assert_eq!(reducer.snapshot.history[0].source, "Corrected source.");
        assert_eq!(
            reducer.snapshot.history[0].translation,
            "Confirmed translation."
        );
    }
}

#[test]
fn an_empty_final_cannot_erase_a_complete_preview_without_replacing_it() {
    let mut reducer = SubtitleReducer::default();
    let stable = PreviewSubtitlePair {
        utterance_id: None,
        source: "Stable source".into(),
        translation: "Stable translation".into(),
    };
    reducer.apply(SubtitleEvent::PreviewPair {
        source_utterance_id: None,
        source: stable.source.clone(),
        translation: stable.translation.clone(),
    });
    reducer.apply(SubtitleEvent::FinalPair {
        source: "Final source".into(),
        translation: " \n".into(),
    });
    assert_eq!(reducer.snapshot.preview_pair, Some(stable));
    assert!(reducer.snapshot.history.is_empty());
}

#[test]
fn transient_reset_and_manual_clear_remove_the_preview_with_distinct_history_lifetimes() {
    let mut reducer = SubtitleReducer::default();
    reducer.archive.begin(true, 0);
    reducer.apply(SubtitleEvent::FinalPair {
        source: "Confirmed source".into(),
        translation: "Confirmed translation".into(),
    });
    reducer.apply(SubtitleEvent::PreviewPair {
        source_utterance_id: None,
        source: "Current source".into(),
        translation: "Current preview".into(),
    });
    reducer.reset_transient();
    assert!(reducer.snapshot.preview_pair.is_none());
    assert_eq!(reducer.snapshot.history.len(), 1);
    assert_eq!(reducer.archive.count(), 1);
    assert!(reducer.snapshot.source.is_final);
    reducer.apply(SubtitleEvent::PreviewPair {
        source_utterance_id: None,
        source: "Current source".into(),
        translation: "Current preview".into(),
    });
    reducer.apply(SubtitleEvent::Clear);
    assert_eq!(reducer.snapshot, SubtitleSnapshot::empty());
    assert_eq!(reducer.archive.count(), 0);
}

#[test]
fn a_new_recognition_cycle_preserves_repeated_lyrics_but_duplicate_finals_stay_idempotent() {
    for limit in [0, 20] {
        let mut reducer = SubtitleReducer::new(limit);
        reducer.archive.begin(true, 0);
        for _ in 0..2 {
            reducer.apply(SubtitleEvent::SourceDraft("Repeated lyric".into()));
            // Revising one recognition cycle does not create several finals.
            reducer.apply(SubtitleEvent::SourceDraft("Repeated lyric".into()));
            for _ in 0..2 {
                reducer.apply(SubtitleEvent::FinalPair {
                    source: "Repeated lyric".into(),
                    translation: "Repeated translation".into(),
                });
            }
        }
        assert_eq!(reducer.snapshot.history.len(), limit.min(2));
        assert_eq!(reducer.archive.count(), 2);
        if limit > 0 {
            assert!(
                reducer.snapshot.history[0].created_at_ms
                    < reducer.snapshot.history[1].created_at_ms
            );
        }
    }
}

#[test]
fn idless_duplicate_suppression_is_independent_of_retention_and_audio_source() {
    for audio_source in [
        crate::models::AudioSource::System,
        crate::models::AudioSource::Microphone,
    ] {
        let mut reducer = SubtitleReducer::new(0);
        reducer.audio_source = audio_source;
        reducer.archive.begin(true, 0);
        let pair = || SubtitleEvent::FinalPair {
            source: "Synthetic same source".into(),
            translation: "Synthetic same translation".into(),
        };
        // The source-only final and atomic final projections share one
        // identity-less confirmation boundary.
        reducer.apply(SubtitleEvent::SourceFinal("Synthetic same source".into()));
        reducer.apply(SubtitleEvent::TranslationFinal(
            "Synthetic same translation".into(),
        ));
        reducer.apply(pair());
        assert_eq!(reducer.archive.count(), 1);
        assert_eq!(reducer.archive.pairs[0].audio_source, audio_source);
        reducer.set_history_limit(3);
        reducer.reset_transient();
        reducer.apply(pair());
        assert!(reducer.snapshot.history.is_empty());
        assert_eq!(reducer.archive.count(), 1);

        reducer.begin_new_session();
        reducer.apply(pair());
        assert_eq!(reducer.archive.count(), 2);
        assert_eq!(reducer.snapshot.history.len(), 1);
        reducer.apply(SubtitleEvent::Clear);
        reducer.apply(pair());
        assert_eq!(reducer.archive.count(), 1);
        assert_eq!(reducer.snapshot.history.len(), 1);
    }
}

#[test]
fn current_confirmation_time_survives_disabled_history_and_ignores_duplicates() {
    let mut reducer = SubtitleReducer::new(0);
    reducer.archive.begin(true, 0);
    let event = SubtitleEvent::FinalPair {
        source: "Synthetic source".into(),
        translation: "Synthetic translation".into(),
    };
    reducer.apply(event.clone());
    let time = reducer.snapshot.display_pair_created_at_ms.unwrap();
    assert!(time > 0);
    assert!(reducer.snapshot.history.is_empty());
    reducer.apply(event);
    assert_eq!(reducer.snapshot.display_pair_created_at_ms, Some(time));
    assert_eq!(reducer.archive.count(), 1);
    reducer.apply(SubtitleEvent::Clear);
    assert_eq!(reducer.snapshot.display_pair_created_at_ms, None);
}

#[test]
fn a_new_session_identical_final_refreshes_its_confirmation_time() {
    let mut reducer = SubtitleReducer::default();
    let pair = || SubtitleEvent::FinalPair {
        source: "Synthetic repeated source".into(),
        translation: "Synthetic repeated translation".into(),
    };
    reducer.apply(pair());
    let previous_time = reducer.snapshot.display_pair_created_at_ms.unwrap();
    reducer.begin_new_session();
    reducer.apply(pair());
    assert_eq!(reducer.snapshot.history.len(), 2);
    let current_time = reducer.snapshot.history.last().unwrap().created_at_ms;
    assert!(current_time > previous_time);
    assert_eq!(
        reducer.snapshot.display_pair_created_at_ms,
        Some(current_time)
    );
}

#[test]
fn an_older_identified_final_cannot_replace_the_current_time() {
    let mut reducer = SubtitleReducer::default();
    for id in ["A", "B"] {
        reducer.apply(SubtitleEvent::UtteranceText {
            utterance_id: id.into(),
            role: UtteranceRole::Source,
            text: "Same synthetic source".into(),
            is_final: false,
        });
    }
    let event = |id: &str| SubtitleEvent::IdentifiedFinalPair {
        utterance_id: id.into(),
        source: "Same synthetic source".into(),
        translation: "Same synthetic translation".into(),
    };
    reducer.apply(event("B"));
    let time = reducer.snapshot.display_pair_created_at_ms;
    assert_eq!(
        time,
        reducer
            .snapshot
            .history
            .last()
            .map(|pair| pair.created_at_ms)
    );
    reducer.apply(event("A"));
    assert_eq!(reducer.snapshot.display_pair_created_at_ms, time);
}

#[test]
fn blank_drafts_or_reset_cannot_rearm_a_duplicate_confirmation() {
    let mut reducer = SubtitleReducer::default();
    for _ in 0..2 {
        reducer.apply(SubtitleEvent::FinalPair {
            source: "Confirmed source".into(),
            translation: "Confirmed translation".into(),
        });
        reducer.apply(SubtitleEvent::SourceDraft(" \n".into()));
        reducer.apply(SubtitleEvent::TranslationDraft(
            "A translation update".into(),
        ));
    }
    assert_eq!(reducer.snapshot.history.len(), 1);
    reducer.apply(SubtitleEvent::SourceDraft("Confirmed source".into()));
    reducer.reset_transient();
    reducer.apply(SubtitleEvent::FinalPair {
        source: "Confirmed source".into(),
        translation: "Confirmed translation".into(),
    });
    assert_eq!(reducer.snapshot.history.len(), 1);
}

#[test]
fn new_session_admits_one_identical_idless_final_after_empty_events_and_reconnect() {
    let mut reducer = SubtitleReducer::default();
    let pair = || SubtitleEvent::FinalPair {
        source: "Repeated confirmation".into(),
        translation: "Repeated translation".into(),
    };
    reducer.apply(pair());
    reducer.begin_new_session();
    reducer.archive.begin(true, 1);
    assert_eq!(reducer.snapshot.history.len(), 1);
    for (source, translation) in [("", "Repeated translation"), ("Repeated confirmation", "")] {
        reducer.apply(SubtitleEvent::FinalPair {
            source: source.into(),
            translation: translation.into(),
        });
    }
    reducer.apply(SubtitleEvent::SourceDraft(" \n".into()));
    // Start may perform several connection attempts before a valid final.
    reducer.reset_transient();
    reducer.reset_transient();
    reducer.apply(pair());
    assert_eq!(reducer.snapshot.history.len(), 2);
    assert_eq!(reducer.archive.count(), 1);
    reducer.apply(pair());
    reducer.reset_transient();
    reducer.apply(pair());
    assert_eq!(reducer.snapshot.history.len(), 2);
    assert_eq!(reducer.archive.count(), 1);
}

#[test]
fn identified_confirmations_consume_the_new_session_idless_admission() {
    for event in [
        SubtitleEvent::ConfirmedPair {
            utterance_id: 1,
            source_utterance_id: Some(1),
            source: "Same confirmation".into(),
            translation: "Same translation".into(),
        },
        SubtitleEvent::IdentifiedFinalPair {
            utterance_id: "provider-item-1".into(),
            source: "Same confirmation".into(),
            translation: "Same translation".into(),
        },
    ] {
        let mut reducer = SubtitleReducer::default();
        reducer.begin_new_session();
        reducer.apply(event);
        reducer.reset_transient();
        reducer.apply(SubtitleEvent::FinalPair {
            source: "Same confirmation".into(),
            translation: "Same translation".into(),
        });
        assert_eq!(reducer.snapshot.history.len(), 1);
    }
}

#[test]
fn older_serialized_states_keep_idless_duplicate_suppression() {
    let mut reducer = SubtitleReducer::default();
    let pair = || SubtitleEvent::FinalPair {
        source: "Existing confirmation".into(),
        translation: "Existing translation".into(),
    };
    reducer.apply(pair());
    let mut state = serde_json::to_value(&reducer).unwrap();
    state
        .as_object_mut()
        .unwrap()
        .remove("has_session_confirmation");
    let mut restored: SubtitleReducer = serde_json::from_value(state).unwrap();
    restored.reset_transient();
    restored.apply(pair());
    assert_eq!(restored.snapshot.history.len(), 1);
}

#[test]
fn final_translation_creates_a_history_pair() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceFinal("Hello world.".into()));
    reducer.apply(SubtitleEvent::TranslationFinal("你好，世界。".into()));

    assert_eq!(
        reducer.snapshot.source,
        SubtitleLine::new("Hello world.", true)
    );
    assert_eq!(
        reducer.snapshot.translation,
        SubtitleLine::new("你好，世界。", true)
    );
    assert_eq!(
        reducer.snapshot.history,
        vec![SubtitlePair::new(
            "Hello world.".into(),
            "你好，世界。".into(),
            0
        )]
    );
}

#[test]
fn an_atomic_final_pair_alone_enters_history() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceFinal("legacy pending".into()));
    reducer.apply(SubtitleEvent::FinalPair {
        source: "OpenAI source".into(),
        translation: "OpenAI translation".into(),
    });

    assert_eq!(reducer.snapshot.history.len(), 1);
    assert_eq!(reducer.snapshot.history[0].source, "OpenAI source");
    assert_eq!(
        reducer.snapshot.history[0].translation,
        "OpenAI translation"
    );
}

#[test]
fn a_new_draft_keeps_confirmed_history_available() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceFinal("Hello.".into()));
    reducer.apply(SubtitleEvent::TranslationFinal("你好。".into()));
    reducer.apply(SubtitleEvent::SourceDraft("How are".into()));
    reducer.apply(SubtitleEvent::TranslationDraft("你最近".into()));

    assert_eq!(
        reducer.snapshot.history,
        vec![SubtitlePair::new("Hello.".into(), "你好。".into(), 0)]
    );
    assert_eq!(
        reducer.snapshot.translation,
        SubtitleLine::new("你最近", false)
    );
}

#[test]
fn a_plus_final_replaces_its_preview_and_alone_enters_history() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceDraft("今日は晴れです".into()));
    reducer.apply(SubtitleEvent::TranslationDraft("今天晴天".into()));
    assert!(reducer.snapshot.history.is_empty());

    reducer.apply(SubtitleEvent::SourceFinal("今日は晴れです。".into()));
    reducer.apply(SubtitleEvent::TranslationFinal("今天天气很好。".into()));

    assert_eq!(
        reducer.snapshot.history,
        vec![SubtitlePair::new(
            "今日は晴れです。".into(),
            "今天天气很好。".into(),
            0
        )]
    );
    assert_eq!(
        reducer.snapshot.translation,
        SubtitleLine::new("今天天气很好。", true)
    );
}

#[test]
fn a_late_final_translation_uses_the_recognition_line_on_screen() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceFinal("First sentence.".into()));
    reducer.apply(SubtitleEvent::SourceDraft("Second sentence".into()));
    reducer.apply(SubtitleEvent::TranslationFinal("第一句。".into()));

    assert_eq!(
        reducer.snapshot.history,
        vec![SubtitlePair::new(
            "Second sentence".into(),
            "第一句。".into(),
            0
        )]
    );
    assert_eq!(
        reducer.snapshot.source,
        SubtitleLine::new("Second sentence", false)
    );
}

#[test]
fn duplicate_finals_do_not_duplicate_history() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceFinal("Hello.".into()));
    reducer.apply(SubtitleEvent::TranslationFinal("你好。".into()));
    reducer.apply(SubtitleEvent::TranslationFinal("你好。".into()));
    assert_eq!(reducer.snapshot.history.len(), 1);
}

#[test]
fn identical_source_and_translation_remain_in_history() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceFinal("嗯啊".into()));
    reducer.apply(SubtitleEvent::TranslationFinal("嗯啊".into()));

    assert_eq!(
        reducer.snapshot.history,
        vec![SubtitlePair::new("嗯啊".into(), "嗯啊".into(), 0)]
    );
}

#[test]
fn history_is_bounded() {
    let mut reducer = SubtitleReducer::new(2);
    for index in 1..=3 {
        reducer.apply(SubtitleEvent::SourceFinal(format!("source {index}")));
        reducer.apply(SubtitleEvent::TranslationFinal(format!(
            "translation {index}"
        )));
    }

    assert_eq!(
        reducer.snapshot.history,
        vec![
            SubtitlePair::new("source 2".into(), "translation 2".into(), 0),
            SubtitlePair::new("source 3".into(), "translation 3".into(), 0),
        ]
    );
}

#[test]
fn recognition_finals_without_translations_never_enter_history() {
    let mut reducer = SubtitleReducer::new(2);
    reducer.apply(SubtitleEvent::SourceFinal("source 1".into()));
    reducer.apply(SubtitleEvent::SourceFinal("source 2".into()));
    reducer.apply(SubtitleEvent::SourceFinal("source 3".into()));

    assert!(reducer.snapshot.history.is_empty());
    assert_eq!(reducer.snapshot.source, SubtitleLine::new("source 3", true));

    reducer.apply(SubtitleEvent::TranslationFinal("late translation".into()));
    assert_eq!(
        reducer.snapshot.history,
        vec![SubtitlePair::new(
            "source 3".into(),
            "late translation".into(),
            0
        )]
    );
}

#[test]
fn clear_resets_all_subtitle_state() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceFinal("Hello.".into()));
    reducer.apply(SubtitleEvent::TranslationFinal("你好。".into()));
    reducer.apply(SubtitleEvent::Clear);

    assert_eq!(reducer.snapshot, SubtitleSnapshot::empty());
}

#[test]
fn reconnect_clears_unconfirmed_lines_before_a_late_translation_final() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceDraft("generation A".into()));

    reducer.reset_transient();
    assert_eq!(reducer.snapshot.source, SubtitleLine::new("", false));
    reducer.apply(SubtitleEvent::TranslationFinal("译文 A".into()));
    assert!(reducer.snapshot.history.is_empty());

    reducer.apply(SubtitleEvent::SourceFinal("generation B".into()));
    reducer.apply(SubtitleEvent::TranslationFinal("译文 B".into()));
    assert_eq!(
        reducer.snapshot.history,
        vec![SubtitlePair::new("generation B".into(), "译文 B".into(), 0)]
    );
}

#[test]
fn blank_draft_does_not_overwrite_confirmed_final() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::SourceFinal("Hello.".into()));
    reducer.apply(SubtitleEvent::TranslationFinal("你好。".into()));
    reducer.apply(SubtitleEvent::TranslationDraft("   ".into()));

    assert_eq!(
        reducer.snapshot.translation,
        SubtitleLine::new("你好。", true)
    );
}

#[test]
fn stamped_text_keeps_one_utterance_identity_on_both_lines() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::UtteranceText {
        utterance_id: "item_source".into(),
        role: UtteranceRole::Source,
        text: "Hello.".into(),
        is_final: true,
    });
    reducer.apply(SubtitleEvent::UtteranceText {
        utterance_id: "item_source".into(),
        role: UtteranceRole::Translation,
        text: "你好。".into(),
        is_final: false,
    });

    assert_eq!(
        reducer.snapshot.source.utterance_id.as_deref(),
        Some("item_source")
    );
    assert_eq!(
        reducer.snapshot.translation.utterance_id.as_deref(),
        Some("item_source")
    );
    assert!(reducer.snapshot.history.is_empty());

    reducer.apply(SubtitleEvent::UtteranceText {
        utterance_id: "item_source".into(),
        role: UtteranceRole::Translation,
        text: "你好。".into(),
        is_final: true,
    });
    assert_eq!(
        reducer.snapshot.history,
        vec![SubtitlePair::new("Hello.".into(), "你好。".into(), 0)]
    );
}

#[test]
fn stamped_blank_draft_does_not_overwrite_confirmed_final() {
    let mut reducer = SubtitleReducer::default();
    reducer.apply(SubtitleEvent::UtteranceText {
        utterance_id: "item_source".into(),
        role: UtteranceRole::Translation,
        text: "你好。".into(),
        is_final: true,
    });
    reducer.apply(SubtitleEvent::UtteranceText {
        utterance_id: "item_source".into(),
        role: UtteranceRole::Translation,
        text: "   ".into(),
        is_final: false,
    });

    assert_eq!(
        reducer.snapshot.translation,
        SubtitleLine::for_utterance("你好。", true, "item_source".into())
    );
}
