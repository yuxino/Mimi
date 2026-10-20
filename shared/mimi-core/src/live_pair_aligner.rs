//! Shared DashScope identity pairing. Recognition/translation are paired only
//! through their provider item link; known orphan responses never guess FIFO.

use crate::models::{subtitle_text_within_limit, SubtitleEvent, UtteranceRole};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

pub const MAX_TRACKED_ITEMS: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LivePairIdentity {
    pub item_id: Option<String>,
    pub previous_item_id: Option<String>,
}

/// Decoded provider input and aligned output. Lifecycle passthrough keeps its
/// platform-owned payload; only subtitle fields participate in the algorithm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LivePairEvent {
    SourceDraft {
        text: String,
        language: Option<String>,
    },
    SourceFinal {
        text: String,
        language: Option<String>,
    },
    TranslationDraft {
        text: String,
    },
    TranslationFinal {
        text: String,
    },
    ItemCreated,
    SessionFinished,
    Passthrough {
        is_content: bool,
    },
    UtteranceText {
        utterance_id: String,
        role: UtteranceRole,
        text: String,
        is_final: bool,
        language: Option<String>,
    },
    FinalPair {
        utterance_id: String,
        source: String,
        translation: String,
        language: Option<String>,
        follow_latency_ms: Option<u64>,
    },
}

impl LivePairEvent {
    fn is_content(&self) -> bool {
        match self {
            Self::ItemCreated | Self::SessionFinished => false,
            Self::Passthrough { is_content } => *is_content,
            _ => true,
        }
    }
    pub fn subtitle_event(&self) -> Option<SubtitleEvent> {
        Some(match self {
            Self::SourceDraft { text, .. } => SubtitleEvent::SourceDraft(text.clone()),
            Self::SourceFinal { text, .. } => SubtitleEvent::SourceFinal(text.clone()),
            Self::TranslationDraft { text } => SubtitleEvent::TranslationDraft(text.clone()),
            Self::TranslationFinal { text } => SubtitleEvent::TranslationFinal(text.clone()),
            Self::UtteranceText {
                utterance_id,
                role,
                text,
                is_final,
                ..
            } => SubtitleEvent::UtteranceText {
                utterance_id: utterance_id.clone(),
                role: *role,
                text: text.clone(),
                is_final: *is_final,
            },
            Self::FinalPair {
                utterance_id,
                source,
                translation,
                ..
            } => SubtitleEvent::IdentifiedFinalPair {
                utterance_id: utterance_id.clone(),
                source: source.clone(),
                translation: translation.clone(),
            },
            _ => return None,
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackedUtterance {
    source_text: String,
    source_language: Option<String>,
    source_final: bool,
    translation_final: Option<String>,
    source_final_at: Option<u64>,
    translation_final_at: Option<u64>,
}

/// One bounded alignment state. Timing inputs are monotonic nanosecond offsets;
/// only the follow-latency difference is projected as milliseconds.
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LivePairAligner {
    discarded_items: VecDeque<String>,
    current_source_id: Option<String>,
    // Source observation order is independent of translation placeholders and
    // survives retirement of complete pairs so old replays cannot move it back.
    #[serde(default)]
    source_order: VecDeque<String>,
    #[serde(default)]
    source_order_start: u64,
    utterances: HashMap<String, TrackedUtterance>,
    utterance_order: VecDeque<String>,
    responses: HashMap<String, String>,
    response_order: VecDeque<String>,
}

impl LivePairAligner {
    pub fn discard_item(&mut self, item: String) {
        if !self.discarded_items.contains(&item) {
            self.discarded_items.push_back(item);
            while self.discarded_items.len() > MAX_TRACKED_ITEMS {
                self.discarded_items.pop_front();
            }
        }
    }

    pub fn clear_content(&mut self) {
        let old_ids: Vec<_> = self
            .source_order
            .iter()
            .chain(self.utterance_order.iter())
            .chain(self.response_order.iter())
            .chain(self.current_source_id.iter())
            .cloned()
            .collect();
        for item in old_ids {
            self.discard_item(item);
        }
        self.current_source_id = None;
        self.source_order.clear();
        self.source_order_start = 0;
        self.utterances.clear();
        self.utterance_order.clear();
        self.responses.clear();
        self.response_order.clear();
    }

    pub fn observe_at(
        &mut self,
        event: &LivePairEvent,
        identity: &LivePairIdentity,
        received_at_ns: u64,
    ) -> Vec<LivePairEvent> {
        self.restore_legacy_source_order();
        if event.is_content()
            && identity
                .item_id
                .as_ref()
                .is_some_and(|item| self.discarded_items.contains(item))
        {
            return Vec::new();
        }
        // A created item links a response item to the input item it answers.
        if let (true, Some(item_id), Some(previous_item_id)) = (
            matches!(event, LivePairEvent::ItemCreated),
            identity.item_id.as_deref(),
            identity.previous_item_id.as_deref(),
        ) {
            if !self.responses.contains_key(item_id) {
                self.response_order.push_back(item_id.to_string());
            }
            self.responses
                .insert(item_id.to_string(), previous_item_id.to_string());
            while self.responses.len() > MAX_TRACKED_ITEMS {
                if let Some(oldest) = self.response_order.pop_front() {
                    self.responses.remove(&oldest);
                }
            }
            return Vec::new();
        }

        match event {
            LivePairEvent::SourceDraft { text, language }
            | LivePairEvent::SourceFinal { text, language } => {
                let Some(item_id) = identity.item_id.as_deref() else {
                    return vec![event.clone()];
                };
                // A new source can legitimately follow an old conversation item.
                // Only an actual translation may use the response -> source link.
                self.responses.remove(item_id);
                self.response_order
                    .retain(|item| self.responses.contains_key(item));
                let is_final = matches!(event, LivePairEvent::SourceFinal { .. });
                let mut events = self.start_utterance(item_id);
                let utterance = self.track(item_id);
                utterance.source_text = text.clone();
                utterance.source_language = language.clone();
                utterance.source_final |= is_final;
                if is_final && utterance.source_final_at.is_none() {
                    utterance.source_final_at = Some(received_at_ns);
                }
                events.push(LivePairEvent::UtteranceText {
                    utterance_id: item_id.to_string(),
                    role: UtteranceRole::Source,
                    text: text.clone(),
                    is_final,
                    language: language.clone(),
                });
                if is_final {
                    events.extend(self.take_pair(item_id, false));
                }
                events
            }
            LivePairEvent::TranslationDraft { text } => {
                let Some(response_id) = identity.item_id.as_deref() else {
                    return vec![event.clone()];
                };
                let Some(source_id) = self.responses.get(response_id).cloned() else {
                    // Retired or unknown responses still have an identity.
                    // Dropping that identity would let a late draft replace
                    // the translation belonging to the current source.
                    return Vec::new();
                };
                if self.discarded_items.contains(&source_id) {
                    return Vec::new();
                }
                vec![LivePairEvent::UtteranceText {
                    utterance_id: source_id,
                    role: UtteranceRole::Translation,
                    text: text.clone(),
                    is_final: false,
                    language: None,
                }]
            }
            LivePairEvent::TranslationFinal { text } => {
                let Some(response_id) = identity.item_id.as_deref() else {
                    // Without identity the legacy best-effort path still applies.
                    return vec![event.clone()];
                };
                let Some(source_id) = self.responses.get(response_id).cloned() else {
                    // A known response with no source link must not fall back
                    // to arrival-order pairing with another utterance.
                    return Vec::new();
                };
                if self.discarded_items.contains(&source_id) {
                    return Vec::new();
                }
                // An utterance the recognition stream has already left can no
                // longer receive its final, so its own text is the best source.
                let allow_draft_source = self.source_order.contains(&source_id)
                    && matches!(&self.current_source_id, Some(current) if current != &source_id);
                let utterance = self.track(&source_id);
                utterance.translation_final = Some(text.clone());
                utterance.translation_final_at = Some(received_at_ns);
                self.take_pair(&source_id, allow_draft_source)
            }
            // A graceful close is the last point where an unmatched translation
            // can be paired with the text its own utterance produced.
            LivePairEvent::SessionFinished => {
                let mut events = self
                    .current_source_id
                    .clone()
                    .map(|source_id| self.flush(&source_id))
                    .unwrap_or_default();
                events.push(event.clone());
                events
            }
            _ => vec![event.clone()],
        }
    }

    /// Stable order of an observed source in the current bounded content window.
    /// Unknown or retired-from-window identities supply no ordering evidence.
    pub fn source_ordinal(&self, source_id: &str) -> Option<u64> {
        self.source_order
            .iter()
            .position(|id| id == source_id)
            .and_then(|index| self.source_order_start.checked_add(index as u64))
    }

    /// Moves the recognition stream to `source_id`. Keep a previous source
    /// without a translation so a late response can still find its own text.
    fn start_utterance(&mut self, source_id: &str) -> Vec<LivePairEvent> {
        if self.source_order.iter().any(|id| id == source_id) {
            return Vec::new();
        }
        self.source_order.push_back(source_id.to_string());
        while self.source_order.len() > MAX_TRACKED_ITEMS {
            self.source_order.pop_front();
            self.source_order_start = self.source_order_start.saturating_add(1);
        }
        let previous = self.current_source_id.replace(source_id.to_string());
        previous
            .filter(|source_id| {
                self.utterances
                    .get(source_id)
                    .is_some_and(|utterance| utterance.translation_final.is_some())
            })
            .map(|source_id| self.flush(&source_id))
            .unwrap_or_default()
    }

    /// Older bridge states lack source_order. Their current source is known,
    /// including an empty draft or a retired completed pair. Other entries need
    /// actual source text or a final marker: track order alone cannot distinguish
    /// an old empty draft from an early future translation placeholder.
    fn restore_legacy_source_order(&mut self) {
        if !self.source_order.is_empty() {
            return;
        }
        for id in &self.utterance_order {
            let utterance = &self.utterances[id];
            if utterance.source_final || !utterance.source_text.is_empty() {
                self.source_order.push_back(id.clone());
            }
        }
        if let Some(current) = &self.current_source_id {
            self.source_order.retain(|id| id != current);
            self.source_order.push_back(current.clone());
        }
        while self.source_order.len() > MAX_TRACKED_ITEMS {
            self.source_order.pop_front();
            self.source_order_start = self.source_order_start.saturating_add(1);
        }
    }

    fn flush(&mut self, source_id: &str) -> Vec<LivePairEvent> {
        let pair = self.take_pair(source_id, true);
        self.retire(source_id);
        pair
    }

    /// Emits the pair for one utterance once both sides are authoritative. An
    /// empty translation or an empty recognized result retires the utterance
    /// without a pair and never consumes another utterance's text.
    fn take_pair(&mut self, source_id: &str, allow_draft_source: bool) -> Vec<LivePairEvent> {
        let Some(utterance) = self.utterances.get(source_id) else {
            return Vec::new();
        };
        let Some(translation) = utterance.translation_final.clone() else {
            return Vec::new();
        };
        if !utterance.source_final && !allow_draft_source {
            return Vec::new();
        }
        let source = utterance.source_text.trim().to_string();
        let language = utterance.source_language.clone();
        let follow_latency_ms = utterance
            .source_final_at
            .zip(utterance.translation_final_at)
            .map(|(source_at, translation_at)| {
                translation_at.saturating_sub(source_at) / 1_000_000
            });
        self.retire(source_id);
        if source.is_empty() || translation.trim().is_empty() {
            return Vec::new();
        }
        vec![LivePairEvent::FinalPair {
            utterance_id: source_id.to_string(),
            source,
            language,
            translation: translation.trim().to_string(),
            follow_latency_ms,
        }]
    }

    fn track(&mut self, source_id: &str) -> &mut TrackedUtterance {
        if !self.utterances.contains_key(source_id) {
            self.utterance_order.push_back(source_id.to_string());
            while self.utterance_order.len() > MAX_TRACKED_ITEMS {
                if let Some(oldest) = self.utterance_order.pop_front() {
                    self.retire(&oldest);
                }
            }
        }
        self.utterances.entry(source_id.to_string()).or_default()
    }

    fn retire(&mut self, source_id: &str) {
        self.utterances.remove(source_id);
        self.utterance_order.retain(|item| item != source_id);
        self.responses.remove(source_id);
        self.responses.retain(|_, source| source != source_id);
        self.response_order
            .retain(|item| self.responses.contains_key(item));
    }
}

impl LivePairAligner {
    pub fn tracked_utterance_count(&self) -> usize {
        self.utterances.len()
    }
    pub fn response_count(&self) -> usize {
        self.responses.len()
    }
    pub fn discarded_item_count(&self) -> usize {
        self.discarded_items.len()
    }

    /// Bounded restored state must preserve index/order consistency: otherwise
    /// adding an item to a forged empty eviction order could fail to progress.
    pub fn validate_state(&self) -> Result<(), &'static str> {
        let ids_valid = |items: &VecDeque<String>| {
            items.len() <= MAX_TRACKED_ITEMS
                && items.iter().all(|id| subtitle_text_within_limit(id))
                && items.iter().collect::<HashSet<_>>().len() == items.len()
        };
        let text_valid = |text: Option<&str>| text.is_none_or(subtitle_text_within_limit);
        if ids_valid(&self.discarded_items)
            && ids_valid(&self.source_order)
            && (self.source_order.is_empty()
                || self.source_order.back() == self.current_source_id.as_ref())
            && ids_valid(&self.utterance_order)
            && ids_valid(&self.response_order)
            && self.utterances.len() == self.utterance_order.len()
            && self.responses.len() == self.response_order.len()
            && self
                .utterance_order
                .iter()
                .all(|id| self.utterances.contains_key(id))
            && self
                .response_order
                .iter()
                .all(|id| self.responses.contains_key(id))
            && text_valid(self.current_source_id.as_deref())
            && self.responses.iter().all(|(id, source)| {
                subtitle_text_within_limit(id) && subtitle_text_within_limit(source)
            })
            && self.utterances.values().all(|u| {
                subtitle_text_within_limit(&u.source_text)
                    && text_valid(u.source_language.as_deref())
                    && text_valid(u.translation_final.as_deref())
            })
        {
            Ok(())
        } else {
            Err("live_pair_state_invalid")
        }
    }
}

#[cfg(test)]
mod tests {
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
}
