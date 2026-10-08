//! Audio 3.0 high-quality ASR protocol (`qwen-audio-3.0-asr-flash-streaming`)
//! over DashScope's shared `/api-ws/v1/inference` endpoint. Authentication is
//! a Bearer API key; no Workspace ID participates in the request.

use crate::core::models::SourceLanguage;
use crate::core::protocols::live_translate::{
    LiveTranslateProtocolError, LiveTranslateServerEvent,
};
use serde_json::{json, Value};

/// DashScope unified WebSocket endpoint for streaming inference (same
/// protocol as the old MaaS `{workspace}.cn-beijing.maas.aliyuncs.com` host,
/// but no workspace-id in the URL — auth is `Authorization: Bearer <key>`).
pub const DASHSCOPE_INFERENCE_WS: &str = "wss://dashscope.aliyuncs.com/api-ws/v1/inference";

pub const LANGUAGE_CODES: &[&str] = &[
    "zh", "en", "ja", "ko", "vi", "th", "id", "ms", "tl", "hi", "ar", "fr", "de", "es", "pt", "ru",
    "it", "nl", "sv", "da", "fi", "no", "el", "pl", "cs", "hu", "ro", "bg", "hr", "sk",
];

#[derive(Clone)]
pub struct Audio3ASREndpoint {
    pub url: url::Url,
}

impl Audio3ASREndpoint {
    pub const MODEL: &'static str = "qwen-audio-3.0-asr-flash-streaming";

    pub fn new() -> Result<Self, LiveTranslateProtocolError> {
        Ok(Self {
            url: url::Url::parse(DASHSCOPE_INFERENCE_WS)
                .map_err(|_| LiveTranslateProtocolError::InvalidEndpoint)?,
        })
    }
}

pub enum Audio3ASRRequestEncoder {}

impl Audio3ASRRequestEncoder {
    pub fn run_task(
        task_id: &str,
        source_language: SourceLanguage,
        context: Option<&str>,
    ) -> Result<Value, LiveTranslateProtocolError> {
        Self::run_task_for_model(task_id, source_language, context, Audio3ASREndpoint::MODEL)
    }

    /// Custom addresses opt into this exact Audio3 contract, including its
    /// parameters and authoritative sentence events.
    pub fn run_task_for_model(
        task_id: &str,
        source_language: SourceLanguage,
        context: Option<&str>,
        model: &str,
    ) -> Result<Value, LiveTranslateProtocolError> {
        if model == Audio3ASREndpoint::MODEL
            && source_language != SourceLanguage::Automatic
            && !LANGUAGE_CODES.contains(&source_language.raw_value())
        {
            return Err(LiveTranslateProtocolError::UnsupportedLanguage);
        }
        let trimmed_context = context.map(str::trim).filter(|t| !t.is_empty());

        let mut parameters = json!({
            "format": "pcm",
            "sample_rate": 16_000,
            "semantic_punctuation_enabled": true,
            "heartbeat": true
        });
        if source_language != SourceLanguage::Automatic {
            parameters["language_hints"] = json!([source_language.raw_value()]);
        }

        let mut input = json!({});
        if let Some(text) = trimmed_context {
            input["context"] = json!([{
                "role": "user",
                "content": [{ "type": "input_text", "text": text }]
            }]);
        }

        Ok(json!({
            "header": {
                "action": "run-task",
                "task_id": task_id,
                "streaming": "duplex"
            },
            "payload": {
                "task_group": "audio",
                "task": "asr",
                "function": "recognition",
                "model": model,
                "parameters": parameters,
                "input": input
            }
        }))
    }

    pub fn finish_task(task_id: &str) -> Result<Value, LiveTranslateProtocolError> {
        Ok(json!({
            "header": {
                "action": "finish-task",
                "task_id": task_id,
                "streaming": "duplex"
            },
            "payload": {
                "input": {}
            }
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Audio3ASRServerEvent {
    TaskStarted,
    Transcription {
        text: String,
        is_final: bool,
        sentence_id: Option<u64>,
    },
    Heartbeat,
    TaskFinished,
    TaskFailed {
        code: String,
        message: String,
    },
    Ignored {
        kind: String,
    },
}

impl Audio3ASRServerEvent {
    /// Maps this recognizer event to the shared subtitle event stream.
    pub fn subtitle_event(&self, source_language: SourceLanguage) -> LiveTranslateServerEvent {
        let reported_language = (source_language != SourceLanguage::Automatic)
            .then(|| source_language.raw_value().to_string());
        match self {
            Self::TaskStarted => LiveTranslateServerEvent::SessionCreated,
            Self::Transcription {
                text,
                is_final,
                sentence_id,
            } => {
                if *is_final {
                    if let Some(utterance_id) = sentence_id.filter(|id| *id > 0) {
                        LiveTranslateServerEvent::SourceUtteranceFinal {
                            utterance_id,
                            text: text.clone(),
                            language: reported_language,
                        }
                    } else {
                        LiveTranslateServerEvent::SourceFinal {
                            text: text.clone(),
                            language: reported_language,
                        }
                    }
                } else if let Some(utterance_id) = sentence_id.filter(|id| *id > 0) {
                    LiveTranslateServerEvent::SourceUtteranceDraft {
                        utterance_id,
                        text: text.clone(),
                        language: reported_language,
                    }
                } else {
                    LiveTranslateServerEvent::SourceDraft {
                        text: text.clone(),
                        language: reported_language,
                    }
                }
            }
            Self::Heartbeat => LiveTranslateServerEvent::Ignored {
                kind: "heartbeat".into(),
            },
            Self::TaskFinished => LiveTranslateServerEvent::SessionFinished,
            Self::TaskFailed { code, message } => LiveTranslateServerEvent::Error {
                code: code.clone(),
                message: message.clone(),
            },
            Self::Ignored { kind } => LiveTranslateServerEvent::Ignored { kind: kind.clone() },
        }
    }
}

pub enum Audio3ASRServerEventDecoder {}

/// Audio3 returns recognition JSON only, never an output audio stream.
pub const MAX_AUDIO3_MESSAGE_BYTES: usize = 1024 * 1024;

impl Audio3ASRServerEventDecoder {
    /// A custom gateway must echo the task identity accepted by its run-task.
    pub fn decode_for_task(
        text: &str,
        task_id: &str,
    ) -> Result<Audio3ASRServerEvent, LiveTranslateProtocolError> {
        if text.len() > MAX_AUDIO3_MESSAGE_BYTES {
            return Err(LiveTranslateProtocolError::InvalidJSON);
        }
        let json: Value =
            serde_json::from_str(text).map_err(|_| LiveTranslateProtocolError::InvalidJSON)?;
        if json.pointer("/header/task_id").and_then(Value::as_str) != Some(task_id) {
            return Err(LiveTranslateProtocolError::InvalidJSON);
        }
        Self::decode(text)
    }

    pub fn decode(text: &str) -> Result<Audio3ASRServerEvent, LiveTranslateProtocolError> {
        if text.len() > MAX_AUDIO3_MESSAGE_BYTES {
            return Err(LiveTranslateProtocolError::InvalidJSON);
        }
        let json: Value =
            serde_json::from_str(text).map_err(|_| LiveTranslateProtocolError::InvalidJSON)?;
        let header = json
            .get("header")
            .ok_or(LiveTranslateProtocolError::InvalidJSON)?;
        let event = header
            .get("event")
            .and_then(Value::as_str)
            .ok_or(LiveTranslateProtocolError::MissingEventType)?;

        match event {
            "task-started" => Ok(Audio3ASRServerEvent::TaskStarted),
            "task-finished" => Ok(Audio3ASRServerEvent::TaskFinished),
            "task-failed" => {
                let (code, category) = safe_task_failure(
                    header
                        .get("error_code")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    header
                        .get("error_message")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                );
                Ok(Audio3ASRServerEvent::TaskFailed {
                    code: code.into(),
                    message: category.into(),
                })
            }
            "result-generated" => {
                let sentence = json
                    .pointer("/payload/output/sentence")
                    .ok_or(LiveTranslateProtocolError::InvalidJSON)?;
                if sentence.get("heartbeat").and_then(Value::as_bool) == Some(true) {
                    return Ok(Audio3ASRServerEvent::Heartbeat);
                }
                let text = sentence.get("text").and_then(Value::as_str).unwrap_or("");
                if !crate::core::models::subtitle_text_within_limit(text) {
                    return Err(LiveTranslateProtocolError::InvalidJSON);
                }
                let text = text.trim().to_string();
                let sentence_id = sentence
                    .get("sentence_id")
                    .and_then(Value::as_u64)
                    .filter(|id| *id > 0);
                let is_final = sentence.get("sentence_end").and_then(Value::as_bool) == Some(true);
                let is_real_begin = !is_final
                    && sentence_id.is_some()
                    && sentence.get("sentence_begin").and_then(Value::as_bool) == Some(true);
                if text.is_empty() && !is_real_begin {
                    return Ok(Audio3ASRServerEvent::Ignored {
                        kind: "empty-result".into(),
                    });
                }
                Ok(Audio3ASRServerEvent::Transcription {
                    text,
                    is_final,
                    sentence_id,
                })
            }
            other => Ok(Audio3ASRServerEvent::Ignored {
                kind: other.to_string(),
            }),
        }
    }
}

/// Exact code allowlist and an anchored timeout grammar. Never preserve free
/// provider text or arbitrary "sanitized" codes (which can still contain keys).
fn safe_task_failure(code: &str, message: &str) -> (&'static str, &'static str) {
    let safe_code = match code {
        "CLIENT_ERROR" => "CLIENT_ERROR",
        "UNSUPPORTED_LANGUAGE" => "UNSUPPORTED_LANGUAGE",
        "LOCAL_ASR_OVERLOADED" => "LOCAL_ASR_OVERLOADED",
        "LOCAL_ASR_TIMEOUT" => "LOCAL_ASR_TIMEOUT",
        "SERVER_ERROR" => "SERVER_ERROR",
        "InvalidApiKey" | "INVALID_API_KEY" | "invalid_api_key" => "INVALID_API_KEY",
        "Unauthorized" | "UNAUTHORIZED" | "unauthorized" | "authentication_error" => "UNAUTHORIZED",
        "RequestTimeout" | "REQUEST_TIMEOUT" => "REQUEST_TIMEOUT",
        "Throttling" | "TOO_MANY_REQUESTS" => "THROTTLED",
        _ => "OTHER",
    };
    let request_timeout = message
        .strip_prefix("request timeout after ")
        .and_then(|tail| tail.strip_suffix(" seconds."))
        .is_some_and(|seconds| {
            !seconds.is_empty()
                && seconds.len() <= 6
                && seconds.bytes().all(|byte| byte.is_ascii_digit())
        });
    let category = match safe_code {
        "INVALID_API_KEY" | "UNAUTHORIZED" => "authentication",
        "REQUEST_TIMEOUT" => "timeout",
        "UNSUPPORTED_LANGUAGE" => "unsupported_language",
        "LOCAL_ASR_OVERLOADED" => "local_overload",
        "LOCAL_ASR_TIMEOUT" => "local_timeout",
        "CLIENT_ERROR" if request_timeout => "timeout",
        "CLIENT_ERROR" => "request",
        "SERVER_ERROR" => "service",
        "THROTTLED" => "rate_limit",
        _ => "task_failed",
    };
    (safe_code, category)
}

/// Only exact content-free tokens may stop automatic reconnection for setup repair.
pub fn failure_requires_configuration(token: &str) -> bool {
    let Some(rest) = token.strip_prefix("audio3_error.") else {
        return false;
    };
    let Some((phase, failure)) = rest.split_once('.') else {
        return false;
    };
    (phase == "setup" && failure == "request.CLIENT_ERROR")
        || matches!(phase, "setup" | "recognition" | "connection")
            && matches!(
                failure,
                "unsupported_language.UNSUPPORTED_LANGUAGE"
                    | "authentication.INVALID_API_KEY"
                    | "authentication.UNAUTHORIZED"
                    | "authentication.HTTP_AUTH"
            )
}

/// Provider-facing recognition context hints. These strings are wire assets.
pub enum Audio3ASRContext {}

impl Audio3ASRContext {
    pub fn audiovisual_dialogue(language: SourceLanguage) -> &'static str {
        match language {
            SourceLanguage::Chinese => {
                "中文影视口语对白，包括语气词、停顿、喘息、呻吟、哭声、笑声和其他发声。"
            }
            SourceLanguage::English => {
                "Natural English audiovisual dialogue, including interjections, hesitations, breaths, gasps, moans, cries, laughter, and other vocalizations."
            }
            SourceLanguage::Japanese => {
                "日本語の映像作品の自然な口語会話。感動詞、間投詞、息遣い、喘ぎ声、うめき声、泣き声、笑い声などの発声を含む。"
            }
            SourceLanguage::Korean => {
                "한국어 영상 작품의 자연스러운 구어 대화. 감탄사, 머뭇거림, 숨소리, 신음, 울음, 웃음 등 발성을 포함함."
            }
            _ => {
                "Natural audiovisual dialogue, including interjections, breaths, gasps, moans, cries, laughter, and other vocalizations."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_task_events_require_the_selected_task_identity_and_model() {
        let custom = Audio3ASRRequestEncoder::run_task_for_model(
            "custom-task",
            SourceLanguage::English,
            None,
            "custom-asr-model",
        )
        .unwrap();
        assert_eq!(custom["payload"]["model"], "custom-asr-model");
        assert_eq!(custom["payload"]["parameters"]["format"], "pcm");
        assert_eq!(custom["payload"]["parameters"]["sample_rate"], 16000);
        assert!(Audio3ASRServerEventDecoder::decode_for_task(
            r#"{"header":{"event":"task-started","task_id":"other-task"}}"#,
            "custom-task"
        )
        .is_err());
        assert!(Audio3ASRServerEventDecoder::decode_for_task(
            r#"{"header":{"event":"task-started"}}"#,
            "custom-task"
        )
        .is_err());
        assert_eq!(
            Audio3ASRServerEventDecoder::decode_for_task(
                r#"{"header":{"event":"task-started","task_id":"custom-task"}}"#,
                "custom-task"
            )
            .unwrap(),
            Audio3ASRServerEvent::TaskStarted
        );
    }

    #[test]
    fn all_thirty_audio3_hints_encode_exact_codes_and_auto_omits_them() {
        let expected = [
            "zh", "en", "ja", "ko", "vi", "th", "id", "ms", "tl", "hi", "ar", "fr", "de", "es",
            "pt", "ru", "it", "nl", "sv", "da", "fi", "no", "el", "pl", "cs", "hu", "ro", "bg",
            "hr", "sk",
        ];
        assert_eq!(LANGUAGE_CODES, expected);
        for source in SourceLanguage::ALL {
            if source != SourceLanguage::Automatic && !expected.contains(&source.raw_value()) {
                assert!(Audio3ASRRequestEncoder::run_task("synthetic-task", source, None).is_err());
                continue;
            }
            let payload = Audio3ASRRequestEncoder::run_task(
                "synthetic-task",
                source,
                Some(Audio3ASRContext::audiovisual_dialogue(source)),
            )
            .unwrap();
            let hints = &payload["payload"]["parameters"]["language_hints"];
            if source == SourceLanguage::Automatic {
                assert!(hints.is_null());
            } else {
                assert_eq!(hints, &serde_json::json!([source.raw_value()]));
            }
            assert_eq!(
                payload["payload"]["model"],
                "qwen-audio-3.0-asr-flash-streaming"
            );
        }
    }

    #[test]
    fn audio3_endpoint_uses_the_unified_inference_websocket() {
        let endpoint = Audio3ASREndpoint::new().unwrap();
        assert_eq!(
            endpoint.url.as_str(),
            "wss://dashscope.aliyuncs.com/api-ws/v1/inference"
        );
    }

    #[test]
    fn run_task_favors_accurate_sentence_boundaries() {
        let data = Audio3ASRRequestEncoder::run_task(
            "task-123",
            SourceLanguage::Japanese,
            Some("日本語の自然な会話"),
        )
        .unwrap();
        let header = &data["header"];
        let payload = &data["payload"];
        let parameters = &payload["parameters"];
        let input = &payload["input"];

        assert_eq!(header["action"], "run-task");
        assert_eq!(header["task_id"], "task-123");
        assert_eq!(payload["model"], "qwen-audio-3.0-asr-flash-streaming");
        assert_eq!(parameters["format"], "pcm");
        assert_eq!(parameters["sample_rate"], 16_000);
        assert_eq!(parameters["semantic_punctuation_enabled"], true);
        assert_eq!(parameters["heartbeat"], true);
        assert_eq!(parameters["language_hints"], json!(["ja"]));
        assert!(
            input.get("context").is_some(),
            "dialogue context should improve recognition"
        );
        assert!(
            parameters.get("special_word_filter").is_none(),
            "sensitive filtering must stay disabled"
        );
    }

    #[test]
    fn automatic_recognition_omits_language_hints() {
        let data = Audio3ASRRequestEncoder::run_task("task-auto", SourceLanguage::Automatic, None)
            .unwrap();
        let parameters = &data["payload"]["parameters"];
        assert!(parameters.get("language_hints").is_none());
    }

    #[test]
    fn finish_task_preserves_the_task_identifier() {
        let data = Audio3ASRRequestEncoder::finish_task("task-finish").unwrap();
        let header = &data["header"];
        let payload = &data["payload"];

        assert_eq!(header["action"], "finish-task");
        assert_eq!(header["task_id"], "task-finish");
        assert!(
            payload["input"].is_object(),
            "finish-task requires an empty input object"
        );
    }

    #[test]
    fn interim_and_final_results_map_to_subtitle_events() {
        let draft = Audio3ASRServerEventDecoder::decode(
            r#"{"header":{"event":"result-generated"},"payload":{"output":{"sentence":{"text":"今日は","heartbeat":false,"sentence_end":false}}}}"#,
        )
        .unwrap();
        let final_event = Audio3ASRServerEventDecoder::decode(
            r#"{"header":{"event":"result-generated"},"payload":{"output":{"sentence":{"text":"今日は晴れです。","heartbeat":false,"sentence_end":true}}}}"#,
        )
        .unwrap();

        assert_eq!(
            draft.subtitle_event(SourceLanguage::Japanese),
            LiveTranslateServerEvent::SourceDraft {
                text: "今日は".into(),
                language: Some("ja".into())
            }
        );
        assert_eq!(
            final_event.subtitle_event(SourceLanguage::Japanese),
            LiveTranslateServerEvent::SourceFinal {
                text: "今日は晴れです。".into(),
                language: Some("ja".into())
            }
        );
    }

    #[test]
    fn positive_sentence_identity_is_preserved_without_inventing_missing_ids() {
        for id in [1, 2, u64::MAX] {
            let message = json!({
                "header": {"event": "result-generated"},
                "payload": {"output": {"sentence": {
                    "text": "Synthetic repeated lyric", "sentence_end": true,
                    "sentence_id": id
                }}}
            });
            let event = Audio3ASRServerEventDecoder::decode(&message.to_string()).unwrap();
            assert_eq!(
                event.subtitle_event(SourceLanguage::Japanese),
                LiveTranslateServerEvent::SourceUtteranceFinal {
                    utterance_id: id,
                    text: "Synthetic repeated lyric".into(),
                    language: Some("ja".into()),
                }
            );
        }
        for id in [json!(null), json!(0), json!(-1), json!(1.0), json!("1")] {
            let message = json!({
                "header": {"event": "result-generated"},
                "payload": {"output": {"sentence": {
                    "text": "Synthetic final", "sentence_end": true,
                    "sentence_id": id
                }}}
            });
            let event = Audio3ASRServerEventDecoder::decode(&message.to_string()).unwrap();
            assert!(matches!(
                event.subtitle_event(SourceLanguage::Automatic),
                LiveTranslateServerEvent::SourceFinal { language: None, .. }
            ));
        }
        let heartbeat = Audio3ASRServerEventDecoder::decode(
            r#"{"header":{"event":"result-generated"},"payload":{"output":{"sentence":{"text":"Synthetic ignored text","heartbeat":true,"sentence_end":true,"sentence_id":0}}}}"#,
        )
        .unwrap();
        assert_eq!(heartbeat, Audio3ASRServerEvent::Heartbeat);
    }

    #[test]
    fn an_empty_real_sentence_begin_preserves_its_boundary() {
        let event = Audio3ASRServerEventDecoder::decode(
            r#"{"header":{"event":"result-generated"},"payload":{"output":{"sentence":{"text":"","sentence_begin":true,"sentence_end":false,"sentence_id":8}}}}"#,
        ).unwrap();
        assert!(matches!(event, Audio3ASRServerEvent::Transcription {
            text, is_final: false, sentence_id: Some(8)
        } if text.is_empty()));
    }

    #[test]
    fn recognition_rejects_oversized_fields_and_json_without_truncating_utf8() {
        let exact = "🙂".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES / 4);
        for is_final in [false, true] {
            let frame = |text: &str| {
                json!({
                    "header": {"event": "result-generated"},
                    "payload": {"output": {"sentence": {
                        "text": text, "sentence_end": is_final, "sentence_id": 1
                    }}}
                })
                .to_string()
            };
            let event = Audio3ASRServerEventDecoder::decode(&frame(&exact)).unwrap();
            assert!(
                matches!(event, Audio3ASRServerEvent::Transcription { text, .. } if text == exact)
            );
            assert!(Audio3ASRServerEventDecoder::decode(&frame(&format!("{exact}a"))).is_err());
        }
        assert!(
            Audio3ASRServerEventDecoder::decode(&" ".repeat(MAX_AUDIO3_MESSAGE_BYTES + 1)).is_err()
        );
    }

    #[test]
    fn lifecycle_and_failures_decode() {
        let started = Audio3ASRServerEventDecoder::decode(
            r#"{"header":{"event":"task-started"},"payload":{}}"#,
        )
        .unwrap();
        let failed = Audio3ASRServerEventDecoder::decode(
            r#"{"header":{"event":"task-failed","error_code":"CLIENT_ERROR","error_message":"Bad request"},"payload":{}}"#,
        )
        .unwrap();

        assert_eq!(started, Audio3ASRServerEvent::TaskStarted);
        assert_eq!(
            failed,
            Audio3ASRServerEvent::TaskFailed {
                code: "CLIENT_ERROR".into(),
                message: "request".into()
            }
        );
    }

    #[test]
    fn failure_classification_is_exact_and_content_free() {
        assert_eq!(
            safe_task_failure("CLIENT_ERROR", "request timeout after 23 seconds."),
            ("CLIENT_ERROR", "timeout")
        );
        assert_eq!(
            safe_task_failure("CLIENT_ERROR", "request timeout after secret seconds."),
            ("CLIENT_ERROR", "request")
        );
        assert_eq!(
            safe_task_failure(
                "CLIENT_ERROR",
                "request timeout after 23 seconds. private text"
            ),
            ("CLIENT_ERROR", "request")
        );
        assert_eq!(
            safe_task_failure("INVALID_API_KEY", "request timeout after 23 seconds."),
            ("INVALID_API_KEY", "authentication")
        );
        assert_eq!(
            safe_task_failure("synthetic-private-code", "secret"),
            ("OTHER", "task_failed")
        );
        let event = Audio3ASRServerEventDecoder::decode(r#"{"header":{"event":"task-failed","error_code":"synthetic-private-code","error_message":"synthetic-private-text"}}"#).unwrap();
        assert_eq!(
            event,
            Audio3ASRServerEvent::TaskFailed {
                code: "OTHER".into(),
                message: "task_failed".into()
            }
        );
    }

    #[test]
    fn unsupported_language_is_whitelisted_without_echoing_service_text() {
        let decoded = Audio3ASRServerEventDecoder::decode_for_task(
            r#"{"header":{"event":"task-failed","task_id":"fixture","error_code":"UNSUPPORTED_LANGUAGE","error_message":"private text and credential"}}"#,
            "fixture",
        ).unwrap();
        assert_eq!(
            decoded,
            Audio3ASRServerEvent::TaskFailed {
                code: "UNSUPPORTED_LANGUAGE".into(),
                message: "unsupported_language".into(),
            }
        );
        assert_eq!(
            safe_task_failure("UNSUPPORTED_LANGUAGE private", "unsupported_language"),
            ("OTHER", "task_failed")
        );
        for phase in ["setup", "recognition", "connection"] {
            assert!(failure_requires_configuration(&format!(
                "audio3_error.{phase}.unsupported_language.UNSUPPORTED_LANGUAGE"
            )));
            assert_eq!(
                failure_requires_configuration(&format!(
                    "audio3_error.{phase}.request.CLIENT_ERROR"
                )),
                phase == "setup"
            );
        }
        for token in [
            "transport_error",
            "audio3_error.setup.timeout.CLIENT_ERROR",
            "audio3_error.recognition.service.SERVER_ERROR",
            "audio3_error.setup.unsupported_language.OTHER",
            "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE.private",
        ] {
            assert!(!failure_requires_configuration(token));
        }
    }

    #[test]
    fn local_asr_failures_are_exact_codes_and_remain_retryable() {
        for (code, category) in [
            ("LOCAL_ASR_OVERLOADED", "local_overload"),
            ("LOCAL_ASR_TIMEOUT", "local_timeout"),
        ] {
            assert_eq!(
                safe_task_failure(code, "private path and text"),
                (code, category)
            );
            assert!(!failure_requires_configuration(&format!(
                "audio3_error.recognition.{category}.{code}"
            )));
            assert_eq!(
                safe_task_failure(&format!("{code} private"), "private"),
                ("OTHER", "task_failed")
            );
        }
        assert!(!failure_requires_configuration(
            "audio3_error.recognition.request.CLIENT_ERROR"
        ));
    }

    #[test]
    fn heartbeat_and_empty_results_are_ignored() {
        let heartbeat = Audio3ASRServerEventDecoder::decode(
            r#"{"header":{"event":"result-generated"},"payload":{"output":{"sentence":{"text":"","heartbeat":true,"sentence_end":false}}}}"#,
        )
        .unwrap();
        assert_eq!(heartbeat, Audio3ASRServerEvent::Heartbeat);

        let empty = Audio3ASRServerEventDecoder::decode(
            r#"{"header":{"event":"result-generated"},"payload":{"output":{"sentence":{"text":"  ","heartbeat":false,"sentence_end":false}}}}"#,
        )
        .unwrap();
        assert_eq!(
            empty,
            Audio3ASRServerEvent::Ignored {
                kind: "empty-result".into()
            }
        );
    }

    #[test]
    fn context_hints_are_verbatim() {
        assert_eq!(
            Audio3ASRContext::audiovisual_dialogue(SourceLanguage::Japanese),
            "日本語の映像作品の自然な口語会話。感動詞、間投詞、息遣い、喘ぎ声、うめき声、泣き声、笑い声などの発声を含む。"
        );
        assert!(
            Audio3ASRContext::audiovisual_dialogue(SourceLanguage::Chinese)
                .contains("中文影视口语对白")
        );
    }
}
