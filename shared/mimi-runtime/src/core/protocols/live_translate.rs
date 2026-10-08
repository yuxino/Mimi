//! Live-translate and realtime-ASR WebSocket protocols for DashScope's shared
//! endpoint. Authentication uses a Bearer API key; the URL has no Workspace
//! ID component.

use crate::core::models::{SourceLanguage, TargetLanguage, UtteranceRole};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LiveTranslateProtocolError {
    #[error("The live translation endpoint could not be created.")]
    InvalidEndpoint,
    #[error("The live translation service returned invalid JSON.")]
    InvalidJSON,
    #[error("The live translation event is missing its type.")]
    MissingEventType,
    #[error("The live translation service does not support this language selection.")]
    UnsupportedLanguage,
}

/// DashScope unified realtime WebSocket endpoint. The old MaaS host put the
/// workspace id in the URL (`{workspace}.cn-beijing.maas.aliyuncs.com`);
/// the unified host authenticates with `Authorization: Bearer <key>` alone.
pub const DASHSCOPE_REALTIME_WS: &str = "wss://dashscope.aliyuncs.com/api-ws/v1/realtime";

fn realtime_url(model: &str) -> Result<url::Url, LiveTranslateProtocolError> {
    let raw = format!("{DASHSCOPE_REALTIME_WS}?model={model}");
    url::Url::parse(&raw).map_err(|_| LiveTranslateProtocolError::InvalidEndpoint)
}

/// `qwen3.5-livetranslate-flash-realtime` endpoint: realtime transcription with
/// simultaneous translation.
#[derive(Clone)]
pub struct LiveTranslateEndpoint {
    pub url: url::Url,
}

impl LiveTranslateEndpoint {
    pub const MODEL: &'static str = "qwen3.5-livetranslate-flash-realtime";

    pub fn new() -> Result<Self, LiveTranslateProtocolError> {
        Ok(Self {
            url: realtime_url(Self::MODEL)?,
        })
    }
}

fn next_event_id() -> String {
    format!("event_{}", Uuid::new_v4().simple())
}

/// Encoder for the live-translate (`qwen3.5-livetranslate-flash-realtime`)
/// session.
pub enum LiveTranslateRequestEncoder {}

impl LiveTranslateRequestEncoder {
    pub fn session_update(
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        hotwords: &BTreeMap<String, String>,
        event_id: Option<&str>,
    ) -> Result<Value, LiveTranslateProtocolError> {
        if !matches!(
            source_language,
            SourceLanguage::Automatic
                | SourceLanguage::Chinese
                | SourceLanguage::English
                | SourceLanguage::Japanese
                | SourceLanguage::Korean
        ) || !matches!(
            target_language,
            TargetLanguage::Original
                | TargetLanguage::SimplifiedChinese
                | TargetLanguage::English
                | TargetLanguage::Japanese
        ) {
            return Err(LiveTranslateProtocolError::UnsupportedLanguage);
        }
        let mut translation = json!({ "language": target_language.raw_value() });
        if !hotwords.is_empty() {
            translation["corpus"] = json!({ "phrases": hotwords });
        }
        // Automatic source detection omits the transcription language so the
        // server detects it per utterance (mirrors the original app's
        // RealtimeASRProtocol: `sourceLanguage == .automatic ? nil : rawValue`);
        // the per-event language field then drives the detected-language UI.
        let mut transcription = json!({ "model": "qwen3-asr-flash-realtime" });
        if source_language != SourceLanguage::Automatic {
            transcription["language"] = json!(source_language.raw_value());
        }
        Ok(json!({
            "event_id": event_id.unwrap_or(&next_event_id()),
            "type": "session.update",
            "session": {
                "modalities": ["text"],
                "sample_rate": 16_000,
                "input_audio_format": "pcm",
                "input_audio_transcription": transcription,
                "translation": translation
            }
        }))
    }

    pub fn audio_append(
        pcm_data: &[u8],
        event_id: Option<&str>,
    ) -> Result<Value, LiveTranslateProtocolError> {
        let audio = base64::engine::general_purpose::STANDARD.encode(pcm_data);
        Ok(json!({
            "event_id": event_id.unwrap_or(&next_event_id()),
            "type": "input_audio_buffer.append",
            "audio": audio
        }))
    }

    pub fn finish(event_id: Option<&str>) -> Result<Value, LiveTranslateProtocolError> {
        Ok(json!({
            "event_id": event_id.unwrap_or(&next_event_id()),
            "type": "session.finish"
        }))
    }
}

/// Server events shared by every pipeline in the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveTranslateServerEvent {
    SessionCreated,
    SessionUpdated,
    SourceDraft {
        text: String,
        language: Option<String>,
    },
    /// A real Audio3 sentence identity, retained on its replaceable draft lane.
    /// Empty text can mark the recognizer's explicit start of a new sentence.
    SourceUtteranceDraft {
        utterance_id: u64,
        text: String,
        language: Option<String>,
    },
    SourceFinal {
        text: String,
        language: Option<String>,
    },
    /// Audio3's real positive sentence ID, scoped to its recognizer task.
    SourceUtteranceFinal {
        utterance_id: u64,
        text: String,
        language: Option<String>,
    },
    TranslationStarted,
    /// Local, replaceable HTTP preview lifecycle. Never a final boundary.
    PreviewTranslationStarted {
        request_id: u64,
    },
    PreviewTranslationFinished {
        request_id: u64,
    },
    SubtitlePreviewPair {
        source_utterance_id: Option<u64>,
        source: String,
        language: Option<String>,
        translation: String,
    },
    /// Invalidates only an obsolete preview at a real source sentence boundary.
    SubtitlePreviewCleared,
    /// Locally generated MT backoff; the system-audio/ASR session stays alive.
    TranslationDeferred(crate::core::diagnostics::TranslationRecovery),
    TranslationDraft(String),
    TranslationFinal(String),
    /// Text stamped with the provider utterance it belongs to. `utterance_id` is
    /// always the *source* item: a translation's response item is resolved
    /// through `previous_item_id` before it reaches this event, so both preview
    /// lines of one utterance share the same identity.
    UtteranceText {
        utterance_id: String,
        role: UtteranceRole,
        text: String,
        is_final: bool,
        language: Option<String>,
    },
    SubtitleFinalPair {
        source: String,
        language: Option<String>,
        translation: String,
    },
    /// DashScope's complete pair retains its actual source item identity.
    SubtitleIdentifiedFinalPair {
        utterance_id: String,
        source: String,
        language: Option<String>,
        translation: String,
    },
    /// Locally accepted HQ utterance identity, scoped to one connection.
    /// Reliable final delivery preserves repeated text even without drafts.
    SubtitleConfirmedPair {
        utterance_id: u64,
        source_utterance_id: Option<u64>,
        source: String,
        language: Option<String>,
        translation: String,
    },
    SessionFinished,
    Error {
        code: String,
        message: String,
    },
    Ignored {
        kind: String,
    },
}

/// Wire identity carried by DashScope realtime events.
///
/// `item_id` names the conversation item an event belongs to. `previous_item_id`
/// appears on `conversation.item.created` and links a response item to the input
/// item it answers. The live-translate protocol streams recognition and
/// translation independently and the two finals of one utterance arrive tens of
/// milliseconds apart in either order, so this identity — not arrival order — is
/// what pairs an original line with its translation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveTranslateEventIdentity {
    pub item_id: Option<String>,
    pub previous_item_id: Option<String>,
}

impl LiveTranslateServerEvent {
    pub fn text_within_limit(&self) -> bool {
        use crate::core::models::subtitle_text_within_limit;
        match self {
            Self::SourceDraft { text, .. }
            | Self::SourceUtteranceDraft { text, .. }
            | Self::SourceFinal { text, .. }
            | Self::SourceUtteranceFinal { text, .. }
            | Self::UtteranceText { text, .. }
            | Self::TranslationDraft(text)
            | Self::TranslationFinal(text) => subtitle_text_within_limit(text),
            Self::SubtitlePreviewPair {
                source,
                translation,
                ..
            }
            | Self::SubtitleFinalPair {
                source,
                translation,
                ..
            }
            | Self::SubtitleConfirmedPair {
                source,
                translation,
                ..
            } => subtitle_text_within_limit(source) && subtitle_text_within_limit(translation),
            Self::SubtitleIdentifiedFinalPair {
                utterance_id,
                source,
                translation,
                ..
            } => {
                subtitle_text_within_limit(utterance_id)
                    && subtitle_text_within_limit(source)
                    && subtitle_text_within_limit(translation)
            }
            _ => true,
        }
    }

    pub fn text_limit_error() -> Self {
        Self::Error {
            code: "subtitle_text_too_large".into(),
            message: "The subtitle service returned too much text.".into(),
        }
    }

    /// Decodes one server frame together with the identity it carries.
    pub fn decode_with_identity(
        text: &str,
    ) -> Result<(Self, LiveTranslateEventIdentity), LiveTranslateProtocolError> {
        let json: Value =
            serde_json::from_str(text).map_err(|_| LiveTranslateProtocolError::InvalidJSON)?;
        Self::decode_value_with_identity(&json)
    }

    pub fn decode_value_with_identity(
        json: &Value,
    ) -> Result<(Self, LiveTranslateEventIdentity), LiveTranslateProtocolError> {
        let identity = LiveTranslateEventIdentity {
            item_id: item_id_of(json),
            previous_item_id: json
                .get("previous_item_id")
                .and_then(Value::as_str)
                .map(String::from),
        };
        let event = Self::decode_normalized(json)?;
        let field_valid =
            |text: Option<&str>| text.is_none_or(crate::core::models::subtitle_text_within_limit);
        let language_valid = match &event {
            Self::SourceDraft { language, .. } | Self::SourceFinal { language, .. } => {
                field_valid(language.as_deref())
            }
            _ => true,
        };
        if !event.text_within_limit()
            || !field_valid(identity.item_id.as_deref())
            || !field_valid(identity.previous_item_id.as_deref())
            || !language_valid
        {
            return Err(LiveTranslateProtocolError::InvalidJSON);
        }
        Ok((event, identity))
    }

    fn decode_normalized(json: &Value) -> Result<Self, LiveTranslateProtocolError> {
        let kind = json
            .get("type")
            .and_then(Value::as_str)
            .ok_or(LiveTranslateProtocolError::MissingEventType)?;

        match kind {
            "session.created" => Ok(Self::SessionCreated),
            "session.updated" => Ok(Self::SessionUpdated),
            "session.finished" => Ok(Self::SessionFinished),

            "conversation.item.input_audio_transcription.text" => Ok(Self::SourceDraft {
                text: combined_text(json),
                language: json
                    .get("language")
                    .and_then(Value::as_str)
                    .map(String::from),
            }),

            "conversation.item.input_audio_transcription.completed" => Ok(Self::SourceFinal {
                text: json
                    .get("transcript")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                language: json
                    .get("language")
                    .and_then(Value::as_str)
                    .map(String::from),
            }),

            "response.text.text" | "response.audio_transcript.text" => {
                Ok(Self::TranslationDraft(combined_text(json)))
            }

            "response.text.done" => Ok(Self::TranslationFinal(
                json.get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            )),

            "response.audio_transcript.done" => Ok(Self::TranslationFinal(
                json.get("transcript")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            )),

            "error" => {
                let error = json.get("error");
                Ok(Self::Error {
                    code: error
                        .and_then(|e| e.get("code"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown_error")
                        .to_string(),
                    message: error
                        .and_then(|e| e.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("Alibaba Cloud returned an unknown error.")
                        .to_string(),
                })
            }

            other => Ok(Self::Ignored {
                kind: other.to_string(),
            }),
        }
    }
}

/// Confirmed `text` plus tentative `stash`, trimmed — the server's combined
/// preview representation.
fn combined_text(json: &Value) -> String {
    let confirmed = json.get("text").and_then(Value::as_str).unwrap_or("");
    let tentative = json.get("stash").and_then(Value::as_str).unwrap_or("");
    format!("{confirmed}{tentative}").trim().to_string()
}

/// The conversation item an event belongs to: a top-level `item_id`, or the
/// nested `item.id` of a created item.
fn item_id_of(json: &Value) -> Option<String> {
    json.get("item_id")
        .and_then(Value::as_str)
        .or_else(|| {
            json.get("item")
                .and_then(|item| item.get("id"))
                .and_then(Value::as_str)
        })
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_pairing_identity_and_language_are_bounded_before_retention() {
        let oversized = "x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1);
        for json in [
            json!({"type":"conversation.item.created","item_id":oversized,"previous_item_id":"valid"}),
            json!({"type":"conversation.item.created","item_id":"valid","previous_item_id":oversized}),
            json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"valid","transcript":"Valid source","language":oversized}),
        ] {
            assert_eq!(
                LiveTranslateServerEvent::decode_value_with_identity(&json),
                Err(LiveTranslateProtocolError::InvalidJSON)
            );
        }
    }

    #[test]
    fn expanded_app_languages_do_not_expand_the_legacy_realtime_wire_contract() {
        for source in SourceLanguage::ALL.into_iter().filter(|source| {
            !matches!(
                source,
                SourceLanguage::Automatic
                    | SourceLanguage::Chinese
                    | SourceLanguage::English
                    | SourceLanguage::Japanese
                    | SourceLanguage::Korean
            )
        }) {
            assert_eq!(
                LiveTranslateRequestEncoder::session_update(
                    source,
                    TargetLanguage::English,
                    &BTreeMap::new(),
                    None
                )
                .unwrap_err(),
                LiveTranslateProtocolError::UnsupportedLanguage
            );
        }
        for target in TargetLanguage::ALL.into_iter().filter(|target| {
            !matches!(
                target,
                TargetLanguage::Original
                    | TargetLanguage::SimplifiedChinese
                    | TargetLanguage::English
                    | TargetLanguage::Japanese
            )
        }) {
            assert_eq!(
                LiveTranslateRequestEncoder::session_update(
                    SourceLanguage::Automatic,
                    target,
                    &BTreeMap::new(),
                    None
                )
                .unwrap_err(),
                LiveTranslateProtocolError::UnsupportedLanguage
            );
        }
    }

    fn decode(text: &str) -> LiveTranslateServerEvent {
        LiveTranslateServerEvent::decode_with_identity(text)
            .expect("the test frame is a documented event")
            .0
    }

    #[test]
    fn endpoint_builds_the_unified_realtime_url() {
        let endpoint = LiveTranslateEndpoint::new().unwrap();
        assert_eq!(
            endpoint.url.as_str(),
            "wss://dashscope.aliyuncs.com/api-ws/v1/realtime?model=qwen3.5-livetranslate-flash-realtime"
        );
    }

    #[test]
    fn session_update_requests_text_only_chinese_translation_and_source_transcript() {
        let data = LiveTranslateRequestEncoder::session_update(
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
            &BTreeMap::new(),
            Some("event-session"),
        )
        .unwrap();
        let session = &data["session"];
        let transcription = &session["input_audio_transcription"];
        let translation = &session["translation"];

        assert_eq!(data["type"], "session.update");
        assert_eq!(session["modalities"], json!(["text"]));
        assert_eq!(session["sample_rate"], 16_000);
        assert_eq!(session["input_audio_format"], "pcm");
        assert_eq!(transcription["model"], "qwen3-asr-flash-realtime");
        assert_eq!(transcription["language"], "ja");
        assert_eq!(translation["language"], "zh");
    }

    #[test]
    fn session_update_includes_hotword_corpus_when_present() {
        let hotwords = BTreeMap::from([("mimi".to_string(), "耳".to_string())]);
        let data = LiveTranslateRequestEncoder::session_update(
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
            &hotwords,
            Some("event-hotwords"),
        )
        .unwrap();
        assert_eq!(
            data["session"]["translation"]["corpus"]["phrases"]["mimi"],
            "耳"
        );
    }

    #[test]
    fn audio_append_base64_encodes_pcm_bytes() {
        let data =
            LiveTranslateRequestEncoder::audio_append(&[0x00, 0x7F, 0xFF], Some("event-audio"))
                .unwrap();
        assert_eq!(data["type"], "input_audio_buffer.append");
        assert_eq!(data["audio"], "AH//");
    }

    #[test]
    fn session_update_selects_an_explicit_target_language() {
        let data = LiveTranslateRequestEncoder::session_update(
            SourceLanguage::English,
            TargetLanguage::Japanese,
            &BTreeMap::new(),
            Some("event-target"),
        )
        .unwrap();
        assert_eq!(data["session"]["translation"]["language"], "ja");
    }

    #[test]
    fn finish_event_uses_the_documented_type() {
        let json = LiveTranslateRequestEncoder::finish(Some("event-finish")).unwrap();
        assert_eq!(json["type"], "session.finish");
    }

    #[test]
    fn source_preview_combines_confirmed_and_tentative_text() {
        let event = decode(
            r#"{"type":"conversation.item.input_audio_transcription.text","text":"Hello","stash":" world","language":"en"}"#,
        );
        assert_eq!(
            event,
            LiveTranslateServerEvent::SourceDraft {
                text: "Hello world".into(),
                language: Some("en".into())
            }
        );
    }

    #[test]
    fn asr_preview_combines_confirmed_text_and_stash() {
        let event = decode(
            r#"{"type":"conversation.item.input_audio_transcription.text","text":"今日は","stash":"晴れです","language":"ja"}"#,
        );
        assert_eq!(
            event,
            LiveTranslateServerEvent::SourceDraft {
                text: "今日は晴れです".into(),
                language: Some("ja".into())
            }
        );
    }

    #[test]
    fn source_completion_decodes_the_final_transcript() {
        let event = decode(
            r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"Hello world.","language":"en"}"#,
        );
        assert_eq!(
            event,
            LiveTranslateServerEvent::SourceFinal {
                text: "Hello world.".into(),
                language: Some("en".into())
            }
        );
    }

    #[test]
    fn identity_keeps_the_recognition_item_of_source_events() {
        let (event, identity) = LiveTranslateServerEvent::decode_with_identity(
            r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"Hello world.","item_id":"item_source"}"#,
        )
        .unwrap();
        assert_eq!(
            event,
            LiveTranslateServerEvent::SourceFinal {
                text: "Hello world.".into(),
                language: None
            }
        );
        assert_eq!(
            identity,
            LiveTranslateEventIdentity {
                item_id: Some("item_source".into()),
                previous_item_id: None,
            }
        );
    }

    #[test]
    fn identity_keeps_the_response_item_of_translation_events() {
        let (event, identity) = LiveTranslateServerEvent::decode_with_identity(
            r#"{"type":"response.text.done","text":"你好，世界。","item_id":"item_response","response_id":"resp_1"}"#,
        )
        .unwrap();
        assert_eq!(
            event,
            LiveTranslateServerEvent::TranslationFinal("你好，世界。".into())
        );
        assert_eq!(identity.item_id.as_deref(), Some("item_response"));
        assert_eq!(identity.previous_item_id, None);
    }

    #[test]
    fn identity_links_a_created_response_item_to_its_input_item() {
        let (event, identity) = LiveTranslateServerEvent::decode_with_identity(
            r#"{"type":"conversation.item.created","item":{"id":"item_response","role":"assistant"},"previous_item_id":"item_source"}"#,
        )
        .unwrap();
        assert_eq!(
            event,
            LiveTranslateServerEvent::Ignored {
                kind: "conversation.item.created".into()
            }
        );
        assert_eq!(identity.item_id.as_deref(), Some("item_response"));
        assert_eq!(identity.previous_item_id.as_deref(), Some("item_source"));
    }

    #[test]
    fn translation_preview_and_completion_decode() {
        let preview = decode(r#"{"type":"response.text.text","text":"你好","stash":"，世界"}"#);
        let final_event = decode(r#"{"type":"response.text.done","text":"你好，世界。"}"#);

        assert_eq!(
            preview,
            LiveTranslateServerEvent::TranslationDraft("你好，世界".into())
        );
        assert_eq!(
            final_event,
            LiveTranslateServerEvent::TranslationFinal("你好，世界。".into())
        );
    }

    #[test]
    fn session_and_error_events_decode() {
        let updated = decode(r#"{"type":"session.updated"}"#);
        let finished = decode(r#"{"type":"session.finished"}"#);
        let failure =
            decode(r#"{"type":"error","error":{"code":"invalid_value","message":"Bad language"}}"#);

        assert_eq!(updated, LiveTranslateServerEvent::SessionUpdated);
        assert_eq!(finished, LiveTranslateServerEvent::SessionFinished);
        assert_eq!(
            failure,
            LiveTranslateServerEvent::Error {
                code: "invalid_value".into(),
                message: "Bad language".into()
            }
        );
    }

    #[test]
    fn unknown_events_are_ignored_without_failing_the_receive_loop() {
        let event = decode(r#"{"type":"response.created"}"#);
        assert_eq!(
            event,
            LiveTranslateServerEvent::Ignored {
                kind: "response.created".into()
            }
        );
    }

    #[test]
    fn malformed_json_and_missing_type_fail_cleanly() {
        assert!(matches!(
            LiveTranslateServerEvent::decode_with_identity("not json"),
            Err(LiveTranslateProtocolError::InvalidJSON)
        ));
        assert!(matches!(
            LiveTranslateServerEvent::decode_with_identity(r#"{"event_id":"x"}"#),
            Err(LiveTranslateProtocolError::MissingEventType)
        ));
    }
}
