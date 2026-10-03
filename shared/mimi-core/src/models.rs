//! Subtitle wire models. Existing desktop IPC field names remain stable.

use serde::{Deserialize, Serialize};

/// One independent capture/recognition lane. A lane can never mix inputs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioSource {
    #[default]
    System,
    Microphone,
}

/// Bound complete subtitle fields without truncating words or UTF-8 characters.
pub const MAX_SUBTITLE_TEXT_BYTES: usize = 64 * 1024;

pub fn subtitle_text_within_limit(text: &str) -> bool {
    text.len() <= MAX_SUBTITLE_TEXT_BYTES
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranslationRecoveryReason {
    RateLimited,
    TemporarilyUnavailable,
}

/// Local, nonterminal MT recovery. Contains no service response or content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRecovery {
    pub reason: TranslationRecoveryReason,
    pub retry_after_ms: u64,
    /// False when a replaceable preview exhausted its bounded attempts. No
    /// retry runs until new speech schedules work; capture continues.
    pub retry_scheduled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleLine {
    pub text: String,
    #[serde(rename = "isFinal")]
    pub is_final: bool,
    /// The provider utterance this line belongs to, when the provider identifies
    /// its utterances. Both preview lines of one utterance carry the *source*
    /// item id, so presentation can refuse to stack a translation under the next
    /// sentence's original.
    #[serde(rename = "utteranceId")]
    pub utterance_id: Option<String>,
}

impl SubtitleLine {
    pub fn new(text: impl Into<String>, is_final: bool) -> Self {
        Self {
            text: text.into(),
            is_final,
            utterance_id: None,
        }
    }

    /// The same line stamped with the provider utterance it belongs to.
    pub fn for_utterance(text: impl Into<String>, is_final: bool, utterance_id: String) -> Self {
        Self {
            text: text.into(),
            is_final,
            utterance_id: Some(utterance_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitlePair {
    #[serde(default, rename = "audioSource")]
    pub audio_source: AudioSource,
    pub source: String,
    pub translation: String,
    /// Epoch milliseconds; equality intentionally ignores display time.
    #[serde(rename = "createdAt")]
    pub created_at_ms: u64,
}

impl SubtitlePair {
    pub fn new(source: String, translation: String, created_at_ms: u64) -> Self {
        Self {
            audio_source: AudioSource::System,
            source,
            translation,
            created_at_ms,
        }
    }
}

impl PartialEq for SubtitlePair {
    fn eq(&self, other: &Self) -> bool {
        self.audio_source == other.audio_source
            && self.source == other.source
            && self.translation == other.translation
    }
}

impl Eq for SubtitlePair {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewSubtitlePair {
    #[serde(
        default,
        rename = "utteranceId",
        skip_serializing_if = "Option::is_none"
    )]
    pub utterance_id: Option<String>,
    pub source: String,
    pub translation: String,
}

/// One independent capture lane. Translation state and utterance IDs never cross lanes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSubtitleSnapshot {
    pub audio_source: AudioSource,
    pub source: SubtitleLine,
    pub translation: SubtitleLine,
    pub history: Vec<SubtitlePair>,
    pub preview_pair: Option<PreviewSubtitlePair>,
    #[serde(
        default,
        rename = "displayPair",
        skip_serializing_if = "Option::is_none"
    )]
    pub display_pair: Option<PreviewSubtitlePair>,
    pub detected_language: Option<String>,
    pub is_translation_pending: bool,
    pub is_translation_preview_pending: bool,
    pub is_translation_timed_out: bool,
    pub translation_recovery: Option<TranslationRecovery>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleSnapshot {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<SourceSubtitleSnapshot>,
    pub source: SubtitleLine,
    pub translation: SubtitleLine,
    pub history: Vec<SubtitlePair>,
    #[serde(default, rename = "previewPair")]
    pub preview_pair: Option<PreviewSubtitlePair>,
    #[serde(
        default,
        rename = "displayPair",
        skip_serializing_if = "Option::is_none"
    )]
    pub display_pair: Option<PreviewSubtitlePair>,
}

impl SubtitleSnapshot {
    pub fn empty() -> Self {
        Self {
            tracks: Vec::new(),
            source: SubtitleLine::new("", false),
            translation: SubtitleLine::new("", false),
            history: Vec::new(),
            preview_pair: None,
            display_pair: None,
        }
    }
}

impl Default for SubtitleSnapshot {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "SubtitleEventWire", into = "SubtitleEventWire")]
pub enum SubtitleEvent {
    SourceDraft(String),
    SourceUtteranceDraft {
        utterance_id: u64,
        text: String,
    },
    SourceFinal(String),
    TranslationDraft(String),
    TranslationFinal(String),
    /// Atomically replaces a completed preview without confirming history.
    PreviewPair {
        source_utterance_id: Option<u64>,
        source: String,
        translation: String,
    },
    ClearPreview,
    /// Text from a provider that identifies its utterances. `role` selects the
    /// preview line and `utterance_id` is always the *source* utterance id, so
    /// both lines of one utterance carry the same identity.
    UtteranceText {
        utterance_id: String,
        role: UtteranceRole,
        text: String,
        is_final: bool,
    },
    /// Commits a source/translation pair as one reducer operation. Providers
    /// whose two append-only streams are aligned client-side use this event so
    /// finals from different connection generations can never be cross-paired.
    FinalPair {
        source: String,
        translation: String,
    },
    /// A complete pair with the provider's exact source item identity.
    IdentifiedFinalPair {
        utterance_id: String,
        source: String,
        translation: String,
    },
    /// A reliable final boundary identified within the current generation.
    ConfirmedPair {
        utterance_id: u64,
        source_utterance_id: Option<u64>,
        source: String,
        translation: String,
    },
    Clear,
}

impl SubtitleEvent {
    pub fn text_within_limit(&self) -> bool {
        match self {
            Self::SourceDraft(text)
            | Self::SourceUtteranceDraft { text, .. }
            | Self::SourceFinal(text)
            | Self::TranslationDraft(text)
            | Self::TranslationFinal(text) => subtitle_text_within_limit(text),
            Self::UtteranceText {
                utterance_id, text, ..
            } => subtitle_text_within_limit(utterance_id) && subtitle_text_within_limit(text),
            Self::IdentifiedFinalPair {
                utterance_id,
                source,
                translation,
            } => {
                subtitle_text_within_limit(utterance_id)
                    && subtitle_text_within_limit(source)
                    && subtitle_text_within_limit(translation)
            }
            Self::PreviewPair {
                source,
                translation,
                ..
            }
            | Self::FinalPair {
                source,
                translation,
            }
            | Self::ConfirmedPair {
                source,
                translation,
                ..
            } => subtitle_text_within_limit(source) && subtitle_text_within_limit(translation),
            Self::Clear | Self::ClearPreview => true,
        }
    }
}

/// Which preview line a stamped provider text belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UtteranceRole {
    Source,
    Translation,
}
// This wire adapter preserves the established tuple-style Rust API while
// giving every platform one flat, tagged event schema.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum SubtitleEventWire {
    SourceDraft {
        text: String,
    },
    SourceUtteranceDraft {
        utterance_id: u64,
        text: String,
    },
    SourceFinal {
        text: String,
    },
    TranslationDraft {
        text: String,
    },
    TranslationFinal {
        text: String,
    },
    PreviewPair {
        source_utterance_id: Option<u64>,
        source: String,
        translation: String,
    },
    ClearPreview,
    UtteranceText {
        utterance_id: String,
        role: UtteranceRole,
        text: String,
        is_final: bool,
    },
    FinalPair {
        source: String,
        translation: String,
    },
    IdentifiedFinalPair {
        utterance_id: String,
        source: String,
        translation: String,
    },
    ConfirmedPair {
        utterance_id: u64,
        source_utterance_id: Option<u64>,
        source: String,
        translation: String,
    },
    Clear,
}

impl From<SubtitleEvent> for SubtitleEventWire {
    fn from(event: SubtitleEvent) -> Self {
        match event {
            SubtitleEvent::SourceDraft(text) => Self::SourceDraft { text },
            SubtitleEvent::SourceUtteranceDraft { utterance_id, text } => {
                Self::SourceUtteranceDraft { utterance_id, text }
            }
            SubtitleEvent::SourceFinal(text) => Self::SourceFinal { text },
            SubtitleEvent::TranslationDraft(text) => Self::TranslationDraft { text },
            SubtitleEvent::TranslationFinal(text) => Self::TranslationFinal { text },
            SubtitleEvent::PreviewPair {
                source_utterance_id,
                source,
                translation,
            } => Self::PreviewPair {
                source_utterance_id,
                source,
                translation,
            },
            SubtitleEvent::ClearPreview => Self::ClearPreview,
            SubtitleEvent::UtteranceText {
                utterance_id,
                role,
                text,
                is_final,
            } => Self::UtteranceText {
                utterance_id,
                role,
                text,
                is_final,
            },
            SubtitleEvent::FinalPair {
                source,
                translation,
            } => Self::FinalPair {
                source,
                translation,
            },
            SubtitleEvent::IdentifiedFinalPair {
                utterance_id,
                source,
                translation,
            } => Self::IdentifiedFinalPair {
                utterance_id,
                source,
                translation,
            },
            SubtitleEvent::ConfirmedPair {
                utterance_id,
                source_utterance_id,
                source,
                translation,
            } => Self::ConfirmedPair {
                utterance_id,
                source_utterance_id,
                source,
                translation,
            },
            SubtitleEvent::Clear => Self::Clear,
        }
    }
}

impl From<SubtitleEventWire> for SubtitleEvent {
    fn from(event: SubtitleEventWire) -> Self {
        match event {
            SubtitleEventWire::SourceDraft { text } => Self::SourceDraft(text),
            SubtitleEventWire::SourceUtteranceDraft { utterance_id, text } => {
                Self::SourceUtteranceDraft { utterance_id, text }
            }
            SubtitleEventWire::SourceFinal { text } => Self::SourceFinal(text),
            SubtitleEventWire::TranslationDraft { text } => Self::TranslationDraft(text),
            SubtitleEventWire::TranslationFinal { text } => Self::TranslationFinal(text),
            SubtitleEventWire::PreviewPair {
                source_utterance_id,
                source,
                translation,
            } => Self::PreviewPair {
                source_utterance_id,
                source,
                translation,
            },
            SubtitleEventWire::ClearPreview => Self::ClearPreview,
            SubtitleEventWire::UtteranceText {
                utterance_id,
                role,
                text,
                is_final,
            } => Self::UtteranceText {
                utterance_id,
                role,
                text,
                is_final,
            },
            SubtitleEventWire::FinalPair {
                source,
                translation,
            } => Self::FinalPair {
                source,
                translation,
            },
            SubtitleEventWire::IdentifiedFinalPair {
                utterance_id,
                source,
                translation,
            } => Self::IdentifiedFinalPair {
                utterance_id,
                source,
                translation,
            },
            SubtitleEventWire::ConfirmedPair {
                utterance_id,
                source_utterance_id,
                source,
                translation,
            } => Self::ConfirmedPair {
                utterance_id,
                source_utterance_id,
                source,
                translation,
            },
            SubtitleEventWire::Clear => Self::Clear,
        }
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;

    #[test]
    fn shared_events_have_one_flat_schema_and_round_trip_every_variant() {
        let events = [
            SubtitleEvent::SourceDraft("英文、日本語、한국어、中文。".into()),
            SubtitleEvent::SourceUtteranceDraft {
                utterance_id: 0,
                text: "Draft".into(),
            },
            SubtitleEvent::SourceFinal("Final".into()),
            SubtitleEvent::TranslationDraft("草稿".into()),
            SubtitleEvent::TranslationFinal("最终".into()),
            SubtitleEvent::PreviewPair {
                source_utterance_id: Some(3),
                source: "Preview".into(),
                translation: "预览".into(),
            },
            SubtitleEvent::ClearPreview,
            SubtitleEvent::UtteranceText {
                utterance_id: "provider-1".into(),
                role: UtteranceRole::Source,
                text: "Source".into(),
                is_final: false,
            },
            SubtitleEvent::UtteranceText {
                utterance_id: "provider-1".into(),
                role: UtteranceRole::Translation,
                text: "译文".into(),
                is_final: true,
            },
            SubtitleEvent::FinalPair {
                source: "Pair".into(),
                translation: "完整对".into(),
            },
            SubtitleEvent::IdentifiedFinalPair {
                utterance_id: "provider-2".into(),
                source: "Identified pair".into(),
                translation: "有身份的完整对".into(),
            },
            SubtitleEvent::ConfirmedPair {
                utterance_id: 1,
                source_utterance_id: Some(0),
                source: "Confirmed".into(),
                translation: "确认".into(),
            },
            SubtitleEvent::Clear,
        ];
        for event in events {
            let json = serde_json::to_value(&event).unwrap();
            assert!(json["type"].as_str().is_some());
            assert!(json.get("data").is_none());
            assert_eq!(
                serde_json::from_value::<SubtitleEvent>(json).unwrap(),
                event
            );
        }
        assert_eq!(
            serde_json::to_value(SubtitleEvent::SourceDraft("Complete".into())).unwrap(),
            serde_json::json!({"type":"source_draft","text":"Complete"})
        );
        assert_eq!(
            serde_json::to_value(UtteranceRole::Translation).unwrap(),
            "translation"
        );
    }

    #[test]
    fn malformed_shared_identities_roles_and_fields_are_rejected() {
        for value in [
            serde_json::json!({"type":"source_utterance_draft","utterance_id":true,"text":"Invalid"}),
            serde_json::json!({"type":"confirmed_pair","utterance_id":-1,"source":"a","translation":"b"}),
            serde_json::json!({"type":"source_draft","text":"Invalid","other":1}),
            serde_json::json!({"type":"utterance_text","utterance_id":"id","role":"unknown","text":"Invalid","is_final":false}),
            serde_json::json!({"type":"unknown"}),
        ] {
            assert!(serde_json::from_value::<SubtitleEvent>(value).is_err());
        }
    }

    #[test]
    fn old_snapshots_without_display_pair_keep_the_existing_wire_contract() {
        let original = serde_json::json!({"source":{"text":"","isFinal":false,"utteranceId":null},"translation":{"text":"","isFinal":false,"utteranceId":null},"history":[],"previewPair":null});
        let snapshot: SubtitleSnapshot = serde_json::from_value(original.clone()).unwrap();
        assert!(snapshot.display_pair.is_none());
        assert_eq!(serde_json::to_value(snapshot).unwrap(), original);
    }
}
