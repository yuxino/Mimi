//! DashScope realtime protocols. Legacy ASR uses the unified endpoint;
//! optional 3.8 audio translation models use a Beijing workspace-specific host.

use crate::core::models::{SourceLanguage, TargetLanguage, UtteranceRole};
use crate::core::services::provider::AlibabaRealtimeMode;
use base64::Engine;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
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
    #[error("Enter the Beijing Model Studio workspace ID for this realtime model.")]
    MissingWorkspace,
}

/// DashScope unified realtime WebSocket endpoint. The old MaaS host put the
/// workspace id in the URL (`{workspace}.cn-beijing.maas.aliyuncs.com`);
/// the unified host authenticates with `Authorization: Bearer <key>` alone.
pub const DASHSCOPE_REALTIME_WS: &str = "wss://dashscope.aliyuncs.com/api-ws/v1/realtime";

/// Qwen3.8 text output catalog, including text-only destinations.
/// https://help.aliyun.com/zh/model-studio/qwen3-5-livetranslate-flash-realtime#supported-languages
pub const TRANSLATION_LANGUAGE_CODES: &[&str] = &[
    "zh", "en", "ar", "de", "fr", "es", "pt", "id", "it", "ko", "ru", "th", "vi", "ja", "tr", "hi",
    "ms", "nl", "ur", "nb", "sv", "da", "he", "fi", "pl", "is", "cs", "fil", "fa", "yue", "el",
    "af", "ast", "be", "bg", "bn", "bs", "ca", "ceb", "et", "gl", "gu", "hr", "hu", "jv", "kk",
    "kn", "ky", "lv", "mk", "ml", "mr", "pa", "ro", "sk", "sl", "sw", "tg", "az", "uk",
];

pub fn translation_language_code(language: TargetLanguage) -> Option<&'static str> {
    let code = match language.raw_value() {
        "original" => return Some("en"),
        "no" => "nb",
        "tl" => "fil",
        code => code,
    };
    TRANSLATION_LANGUAGE_CODES.contains(&code).then_some(code)
}

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

    /// New 3.8 models require a regional, workspace-specific host. Never
    /// interpolate a URL or a hostname supplied by a profile.
    pub fn for_mode(
        mode: AlibabaRealtimeMode,
        workspace_id: &str,
    ) -> Result<Self, LiveTranslateProtocolError> {
        let workspace = workspace_id.trim();
        if workspace.is_empty() {
            return Err(LiveTranslateProtocolError::MissingWorkspace);
        }
        if workspace.len() > 63
            || !workspace
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || !workspace.as_bytes()[0].is_ascii_alphanumeric()
            || !workspace.as_bytes()[workspace.len() - 1].is_ascii_alphanumeric()
        {
            return Err(LiveTranslateProtocolError::InvalidEndpoint);
        }
        let model = match mode {
            AlibabaRealtimeMode::LiveTranslate => "qwen3.8-livetranslate-flash-realtime",
            AlibabaRealtimeMode::Disabled => {
                return Err(LiveTranslateProtocolError::InvalidEndpoint)
            }
        };
        let url = url::Url::parse(&format!(
            "wss://{workspace}.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime?model={model}"
        ))
        .map_err(|_| LiveTranslateProtocolError::InvalidEndpoint)?;
        Ok(Self { url })
    }
}

fn next_event_id() -> String {
    format!("event_{}", Uuid::new_v4().simple())
}

/// Encoder for the live-translate (`qwen3.5-livetranslate-flash-realtime`)
/// session.
pub enum LiveTranslateRequestEncoder {}

impl LiveTranslateRequestEncoder {
    /// Minimal audio-only 3.8 integration. Source recognition is automatic;
    /// original mode retains recognition while discarding model responses.
    pub fn session_update_for_mode(
        mode: AlibabaRealtimeMode,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        hotwords: &BTreeMap<String, String>,
        event_id: Option<&str>,
    ) -> Result<Value, LiveTranslateProtocolError> {
        if source_language != SourceLanguage::Automatic
            || translation_language_code(target_language).is_none()
        {
            return Err(LiveTranslateProtocolError::UnsupportedLanguage);
        }
        let session = match mode {
            AlibabaRealtimeMode::LiveTranslate => {
                let target = translation_language_code(target_language)
                    .ok_or(LiveTranslateProtocolError::UnsupportedLanguage)?;
                let mut translation = json!({"language": target});
                if !hotwords.is_empty() {
                    translation["corpus"] = json!({"phrases": hotwords});
                }
                // LiveTranslate 3.8 fixes input to mono 16 kHz PCM;
                // its audio.input schema documents only turn detection.
                json!({"output_modalities": ["text"], "translation": translation})
            }
            AlibabaRealtimeMode::Disabled => {
                return Self::session_update(source_language, target_language, hotwords, event_id)
            }
        };
        Ok(
            json!({"event_id": event_id.unwrap_or(&next_event_id()), "type": "session.update", "session": session}),
        )
    }

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
        let mut message = Map::new();
        message.insert(
            "event_id".into(),
            Value::String(event_id.unwrap_or(&next_event_id()).to_owned()),
        );
        message.insert("type".into(), "input_audio_buffer.append".into());
        message.insert("audio".into(), Value::String(audio));
        Ok(Value::Object(message))
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
    /// Reliable, content-free source order; no recognized text is finalized.
    SubtitleSourceBoundary {
        utterance_id: String,
        source_order: u64,
    },
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
    SubtitlePreviewWithdrawn {
        source_utterance_id: u64,
    },
    /// Locally generated MT backoff; the system-audio/ASR session stays alive.
    TranslationDeferred(crate::core::diagnostics::latency::TranslationRecovery),
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
    /// Reliable metadata only, published once before a sentence's drafts.
    UtteranceRegistered {
        utterance_id: String,
    },
    /// One complete replaceable provider result, preserving source ownership.
    SubtitleIdentifiedPreviewPair {
        utterance_id: String,
        source: String,
        translation: String,
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
            Self::SubtitleSourceBoundary { utterance_id, .. } => {
                subtitle_text_within_limit(utterance_id)
            }
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
            }
            | Self::SubtitleIdentifiedPreviewPair {
                utterance_id,
                source,
                translation,
                ..
            } => {
                subtitle_text_within_limit(utterance_id)
                    && subtitle_text_within_limit(source)
                    && subtitle_text_within_limit(translation)
            }
            Self::UtteranceRegistered { utterance_id } => subtitle_text_within_limit(utterance_id),
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

/// Stateful 3.8 wire adapter. Delta fragments are bounded before retention;
/// text.done is a preview until the enclosing response finishes successfully.
pub struct AlibabaRealtimeEventDecoder {
    mode: AlibabaRealtimeMode,
    source: BTreeMap<String, String>,
    translation: BTreeMap<String, String>,
    response_for_item: BTreeMap<String, String>,
    order: VecDeque<String>,
    discarded: VecDeque<String>,
    // Response IDs also survive a content clear as discarded identities, even
    // when no output item had arrived before that boundary.
    active_responses: BTreeSet<String>,
}

type IdentifiedEvent = (LiveTranslateServerEvent, LiveTranslateEventIdentity);
const MAX_REALTIME_ITEMS: usize = mimi_core::live_pair_aligner::MAX_TRACKED_ITEMS;

impl AlibabaRealtimeEventDecoder {
    pub fn new(mode: AlibabaRealtimeMode) -> Self {
        Self {
            mode,
            source: BTreeMap::new(),
            translation: BTreeMap::new(),
            response_for_item: BTreeMap::new(),
            order: VecDeque::new(),
            discarded: VecDeque::new(),
            active_responses: BTreeSet::new(),
        }
    }

    pub fn clear_content(&mut self) {
        let ids: Vec<_> = self
            .order
            .iter()
            .chain(self.active_responses.iter())
            .cloned()
            .collect();
        for id in ids {
            self.discard(id);
        }
        self.source.clear();
        self.translation.clear();
        self.response_for_item.clear();
        self.order.clear();
        self.active_responses.clear();
    }

    fn discard(&mut self, id: String) {
        if !self.discarded.contains(&id) {
            self.discarded.push_back(id);
        }
        while self.discarded.len() > MAX_REALTIME_ITEMS {
            self.discarded.pop_front();
        }
    }

    fn track(&mut self, id: &str) {
        if !self.order.iter().any(|known| known == id) {
            self.order.push_back(id.to_string());
        }
        while self.order.len() > MAX_REALTIME_ITEMS {
            if let Some(old) = self.order.pop_front() {
                self.source.remove(&old);
                self.translation.remove(&old);
                self.response_for_item.remove(&old);
                self.discard(old);
            }
        }
    }

    pub fn decode(
        &mut self,
        wire: &str,
    ) -> Result<Vec<IdentifiedEvent>, LiveTranslateProtocolError> {
        if self.mode == AlibabaRealtimeMode::Disabled {
            return LiveTranslateServerEvent::decode_with_identity(wire).map(|event| vec![event]);
        }
        let json: Value =
            serde_json::from_str(wire).map_err(|_| LiveTranslateProtocolError::InvalidJSON)?;
        let kind = json
            .get("type")
            .and_then(Value::as_str)
            .ok_or(LiveTranslateProtocolError::MissingEventType)?;
        // Keep upstream error bodies out of app diagnostics and user messages.
        if matches!(
            kind,
            "error" | "conversation.item.input_audio_transcription.failed"
        ) {
            return Ok(vec![(
                safe_realtime_error(json.get("error"), false),
                LiveTranslateEventIdentity::default(),
            )]);
        }
        let identity = LiveTranslateEventIdentity {
            item_id: item_id_of(&json),
            previous_item_id: json
                .get("previous_item_id")
                .and_then(Value::as_str)
                .map(String::from),
        };
        let response_id = json
            .get("response_id")
            .or_else(|| json.pointer("/response/id"))
            .and_then(Value::as_str);
        for text in [
            identity.item_id.as_deref(),
            identity.previous_item_id.as_deref(),
            response_id,
        ]
        .into_iter()
        .flatten()
        {
            if !crate::core::models::subtitle_text_within_limit(text) {
                return Err(LiveTranslateProtocolError::InvalidJSON);
            }
        }
        let lifecycle = matches!(
            kind,
            "session.created" | "session.updated" | "session.finished"
        );
        if !lifecycle
            && (identity
                .item_id
                .as_ref()
                .is_some_and(|id| self.discarded.contains(id))
                || response_id.is_some_and(|id| self.discarded.iter().any(|old| old == id)))
        {
            return Ok(Vec::new());
        }
        if let Some(id) = identity.item_id.as_deref() {
            self.track(id);
        }
        match kind {
            "response.created" => {
                if let Some(id) = response_id {
                    self.active_responses.insert(id.to_string());
                    while self.active_responses.len() > MAX_REALTIME_ITEMS {
                        self.active_responses.pop_first();
                    }
                }
            }
            "conversation.item.input_audio_transcription.delta" => {
                let id = identity
                    .item_id
                    .as_deref()
                    .ok_or(LiveTranslateProtocolError::InvalidJSON)?;
                let delta = json
                    .get("delta")
                    .and_then(Value::as_str)
                    .ok_or(LiveTranslateProtocolError::InvalidJSON)?;
                let mut text = self.source.get(id).cloned().unwrap_or_default();
                text.push_str(delta);
                check_realtime_text(&text)?;
                self.source.insert(id.to_string(), text.clone());
                return Ok(vec![(
                    LiveTranslateServerEvent::SourceDraft {
                        text,
                        language: language_of(&json)?,
                    },
                    identity,
                )]);
            }
            "conversation.item.input_audio_transcription.completed" => {
                if let Some(id) = identity.item_id.as_deref() {
                    self.source.remove(id);
                }
            }
            "response.text.delta" | "response.text.done" => {
                let id = identity
                    .item_id
                    .as_deref()
                    .ok_or(LiveTranslateProtocolError::InvalidJSON)?;
                let text = if kind == "response.text.done" {
                    json.get("text")
                        .and_then(Value::as_str)
                        .ok_or(LiveTranslateProtocolError::InvalidJSON)?
                        .to_string()
                } else {
                    let delta = json
                        .get("delta")
                        .and_then(Value::as_str)
                        .ok_or(LiveTranslateProtocolError::InvalidJSON)?;
                    let mut text = self.translation.get(id).cloned().unwrap_or_default();
                    text.push_str(delta);
                    text
                };
                check_realtime_text(&text)?;
                self.translation.insert(id.to_string(), text.clone());
                if let Some(response) = response_id {
                    self.response_for_item
                        .insert(id.to_string(), response.to_string());
                }
                return Ok(vec![(
                    LiveTranslateServerEvent::TranslationDraft(text),
                    identity,
                )]);
            }
            "response.done" => {
                return self.finish_response(&json, response_id);
            }
            _ => {}
        }
        LiveTranslateServerEvent::decode_value_with_identity(&json).map(|event| vec![event])
    }

    fn finish_response(
        &mut self,
        json: &Value,
        response_id: Option<&str>,
    ) -> Result<Vec<IdentifiedEvent>, LiveTranslateProtocolError> {
        let response = json
            .get("response")
            .ok_or(LiveTranslateProtocolError::InvalidJSON)?;
        let completed = response.get("status").and_then(Value::as_str) == Some("completed");
        if let Some(id) = response_id {
            self.active_responses.remove(id);
        }
        let mut outputs = BTreeMap::new();
        if let Some(items) = response.get("output").and_then(Value::as_array) {
            if items.len() > MAX_REALTIME_ITEMS {
                return Err(LiveTranslateProtocolError::InvalidJSON);
            }
            for item in items {
                let Some(id) = item.get("id").and_then(Value::as_str) else {
                    continue;
                };
                check_realtime_text(id)?;
                let mut text = String::new();
                if let Some(parts) = item.get("content").and_then(Value::as_array) {
                    for part in parts {
                        if let Some(fragment) = part.get("text").and_then(Value::as_str) {
                            text.push_str(fragment);
                            check_realtime_text(&text)?;
                        }
                    }
                }
                outputs.insert(id.to_string(), text);
            }
        }
        if let Some(response_id) = response_id {
            for (item, known_response) in &self.response_for_item {
                if known_response == response_id {
                    outputs.entry(item.clone()).or_default();
                }
            }
        }
        let mut events = Vec::new();
        for (id, full_text) in outputs {
            let staged = self.translation.remove(&id).unwrap_or_default();
            self.response_for_item.remove(&id);
            if self.discarded.contains(&id) {
                continue;
            }
            let text = if full_text.is_empty() {
                staged
            } else {
                full_text
            };
            if completed && !text.trim().is_empty() {
                check_realtime_text(&text)?;
                events.push((
                    LiveTranslateServerEvent::TranslationFinal(text.trim().into()),
                    LiveTranslateEventIdentity {
                        item_id: Some(id.clone()),
                        previous_item_id: None,
                    },
                ));
            } else if !completed {
                events.push((
                    LiveTranslateServerEvent::TranslationDraft(String::new()),
                    LiveTranslateEventIdentity {
                        item_id: Some(id.clone()),
                        previous_item_id: None,
                    },
                ));
            }
            self.discard(id);
        }
        if response.get("status").and_then(Value::as_str) == Some("failed") {
            events.push((
                safe_realtime_error(response.pointer("/status_details/error"), true),
                LiveTranslateEventIdentity::default(),
            ));
        }
        Ok(events)
    }
}

/// Preserve only exact recognized error codes; arbitrary error prose and
/// unknown failures must never acquire automatic retry authority.
fn common_response_stream_timeout(error: Option<&Value>) -> bool {
    let Some(error) = error else { return false };
    if error.get("code").and_then(Value::as_str) != Some("COMMON_ERROR")
        || match error.get("type") {
            None => false,
            Some(Value::String(kind)) => kind != "server_error",
            Some(_) => true,
        }
    {
        return false;
    }
    let Some(message) = error.get("message").and_then(Value::as_str) else {
        return false;
    };
    // The observed service reason has two timing fields. Parse its complete
    // grammar; a timeout word inside unrelated provider prose grants no retry.
    let message = message.strip_suffix('.').unwrap_or(message);
    if message.len() > 160
        || !message.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || byte.is_ascii_whitespace()
                || matches!(byte, b':' | b'=' | b',' | b'(' | b')' | b'_')
        })
    {
        return false;
    }
    let fields: Vec<_> = message
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|field| !field.is_empty())
        .collect();
    let [response, stream, first_timeout, second_timeout, first, second, elapsed, third, fourth] =
        fields.as_slice()
    else {
        return false;
    };
    // Accept the provider's timeout_seconds / elapsed_ms fields as well as
    // the equivalent explicit numeric-unit spelling. Every token is consumed.
    let (seconds, unit_seconds, milliseconds, unit_ms) = if first.eq_ignore_ascii_case("seconds") {
        (*second, *first, *fourth, *third)
    } else {
        (*first, *second, *third, *fourth)
    };
    if ![
        (response, "response"),
        (stream, "stream"),
        (first_timeout, "timeout"),
        (second_timeout, "timeout"),
        (&unit_seconds, "seconds"),
        (elapsed, "elapsed"),
        (&unit_ms, "ms"),
    ]
    .iter()
    .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
    {
        return false;
    }
    let canonical_number = |value: &str, maximum: usize| {
        if value.is_empty() || value.len() > maximum || !value.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number = value.parse::<u64>().ok()?;
        (number.to_string() == value).then_some(number)
    };
    match (
        canonical_number(seconds, 6),
        canonical_number(milliseconds, 12),
    ) {
        (Some(seconds), Some(milliseconds)) => seconds == 60 && milliseconds >= seconds * 1000,
        _ => false,
    }
}

fn safe_realtime_error(error: Option<&Value>, response_failed: bool) -> LiveTranslateServerEvent {
    let wire_code = error
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str);
    let response_stream_timeout = common_response_stream_timeout(error);
    crate::pipeline_log!(
        "live translate service error kind={}",
        match wire_code {
            Some("response_idle_timeout") => "response_idle_timeout",
            Some("COMMON_ERROR") if response_stream_timeout => "response_stream_timeout",
            Some("COMMON_ERROR") => match error
                .and_then(|error| error.get("type"))
                .and_then(|value| value.as_str())
            {
                Some("server_error") => "common_server_error",
                Some("invalid_request_error") => "common_invalid_request",
                _ => "common_error",
            },
            Some("RequestTimeOut") => "request_timeout",
            Some("ResponseTimeout") => "response_timeout",
            Some("InternalError.Timeout") => "internal_timeout",
            Some("ServiceUnavailable" | "InternalError" | "internal_error") =>
                "service_unavailable",
            Some("InvalidApiKey" | "invalid_api_key" | "unauthorized" | "authentication_error") =>
                "authentication",
            Some("InvalidParameter" | "invalid_request_error") => "invalid_request",
            Some("client_error") => "client_error",
            _ => "other",
        }
    );
    let (code, message) = match wire_code {
        // Observed with a healthy but quiet 3.8 connection. Retire that
        // connection through the same bounded policy as a transport loss;
        // do not invent audio, assume a fixed lifetime or retry every error.
        Some("response_idle_timeout") => (
            "alibaba_realtime_idle_timeout",
            "The Alibaba realtime session timed out while idle.",
        ),
        Some("COMMON_ERROR") if response_stream_timeout => (
            "alibaba_realtime_idle_timeout",
            "The Alibaba realtime session timed out while idle.",
        ),
        Some("InvalidApiKey" | "invalid_api_key" | "unauthorized" | "authentication_error") => (
            "credential_authentication_failed",
            "credential_authentication_failed",
        ),
        Some("ServiceUnavailable" | "InternalError" | "internal_error") => (
            "alibaba_realtime_service_unavailable",
            "The Alibaba realtime service is temporarily unavailable.",
        ),
        _ if response_failed => (
            "alibaba_realtime_response_failed",
            "The Alibaba realtime response failed.",
        ),
        _ => (
            "alibaba_realtime_provider_error",
            "The Alibaba realtime service rejected the request.",
        ),
    };
    LiveTranslateServerEvent::Error {
        code: code.into(),
        message: message.into(),
    }
}

fn check_realtime_text(text: &str) -> Result<(), LiveTranslateProtocolError> {
    if crate::core::models::subtitle_text_within_limit(text) {
        Ok(())
    } else {
        Err(LiveTranslateProtocolError::InvalidJSON)
    }
}

fn language_of(json: &Value) -> Result<Option<String>, LiveTranslateProtocolError> {
    let language = json.get("language").and_then(Value::as_str);
    if let Some(language) = language {
        check_realtime_text(language)?;
    }
    Ok(language.map(String::from))
}

#[cfg(test)]
mod tests;
