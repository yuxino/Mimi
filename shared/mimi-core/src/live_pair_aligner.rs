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
            .utterance_order
            .iter()
            .chain(self.response_order.iter())
            .chain(self.current_source_id.iter())
            .cloned()
            .collect();
        for item in old_ids {
            self.discard_item(item);
        }
        self.current_source_id = None;
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
                    return vec![event.clone()];
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
                let allow_draft_source =
                    matches!(&self.current_source_id, Some(current) if current != &source_id);
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

    /// Moves the recognition stream to `source_id`. Keep a previous source
    /// without a translation so a late response can still find its own text.
    fn start_utterance(&mut self, source_id: &str) -> Vec<LivePairEvent> {
        if self.current_source_id.as_deref() == Some(source_id) {
            return Vec::new();
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
