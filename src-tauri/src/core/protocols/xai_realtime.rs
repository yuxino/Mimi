//! xAI Grok Speech-to-Speech wire protocol.
//!
//! Grok Voice is a turn-based voice-agent API, not a dedicated simultaneous
//! translation model. mimi constrains the agent with translation-only
//! instructions and only surfaces the input/output transcripts.

use crate::core::models::{SourceLanguage, TargetLanguage};
use base64::Engine;
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum XAIRealtimeProtocolError {
    #[error("xAI Grok Voice does not accept this source language hint.")]
    InvalidSourceLanguage,
    #[error("xAI Grok Voice requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("xAI Grok Voice expected {expected_bytes} audio bytes, got {actual_bytes}.")]
    InvalidAudioFrame {
        expected_bytes: usize,
        actual_bytes: usize,
    },
    #[error("xAI Grok Voice returned invalid JSON.")]
    InvalidJSON,
    #[error("xAI Grok Voice returned an event without a type.")]
    MissingEventType,
    #[error("xAI Grok Voice returned an event without {0}.")]
    MissingEventField(&'static str),
    #[error("The xAI Grok Voice endpoint could not be created.")]
    InvalidEndpoint,
}

pub struct XAIRealtimeEndpoint;

impl XAIRealtimeEndpoint {
    pub const MODEL: &'static str = "grok-voice-latest";
    pub const TRANSCRIPTION_MODEL: &'static str = "grok-transcribe";
    pub const VOICE: &'static str = "eve";
    pub const SAMPLE_RATE_HZ: u32 = 24_000;
    pub const CHANNEL_COUNT: u16 = 1;
    pub const BITS_PER_SAMPLE: u16 = 16;
    pub const FRAME_DURATION_MS: u32 = 200;
    /// Keep the server-VAD boundary deterministic so `finish` can append a
    /// matching amount of silence before it waits for the final response.
    pub const SERVER_VAD_SILENCE_DURATION_MS: u32 = 400;
    pub const AUDIO_FRAME_BYTE_COUNT: usize = Self::SAMPLE_RATE_HZ as usize
        * Self::CHANNEL_COUNT as usize
        * (Self::BITS_PER_SAMPLE as usize / 8)
        * Self::FRAME_DURATION_MS as usize
        / 1_000;

    pub fn url() -> Result<url::Url, XAIRealtimeProtocolError> {
        url::Url::parse(&format!("wss://api.x.ai/v1/realtime?model={}", Self::MODEL))
            .map_err(|_| XAIRealtimeProtocolError::InvalidEndpoint)
    }
}

/// Documented Speech-to-Speech languages, shared by ASR hints and output prompts.
/// https://docs.x.ai/developers/model-capabilities/audio/speech-to-speech#supported-languages
/// Regional es/pt/ar variants are never guessed from a generic preference.
pub const LANGUAGE_CODES: &[&str] = &[
    "zh", "en", "ja", "ko", "vi", "id", "hi", "fr", "de", "ru", "it", "ar-EG", "ar-SA", "ar-AE",
    "bn", "pt-BR", "pt-PT", "es-MX", "es-ES", "tr",
];

pub fn source_language_hint(
    source: SourceLanguage,
) -> Result<Option<&'static str>, XAIRealtimeProtocolError> {
    match source {
        SourceLanguage::Automatic => Ok(None),
        _ if LANGUAGE_CODES.contains(&source.raw_value()) => Ok(Some(source.raw_value())),
        _ => Err(XAIRealtimeProtocolError::InvalidSourceLanguage),
    }
}

pub struct XAIRealtimeRequestEncoder;

impl XAIRealtimeRequestEncoder {
    pub fn session_update(
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        event_id: Option<&str>,
    ) -> Result<Value, XAIRealtimeProtocolError> {
        let target_name = target_language_name(target_language)
            .ok_or(XAIRealtimeProtocolError::InvalidTargetLanguage)?;
        let mut value = json!({
            "type": "session.update",
            "session": {
                "voice": XAIRealtimeEndpoint::VOICE,
                "instructions": translation_instructions(target_name),
                "reasoning": {
                    "effort": "none"
                },
                "turn_detection": {
                    "type": "server_vad",
                    "silence_duration_ms": XAIRealtimeEndpoint::SERVER_VAD_SILENCE_DURATION_MS
                },
                "audio": {
                    "input": {
                        "format": {
                            "type": "audio/pcm",
                            "rate": XAIRealtimeEndpoint::SAMPLE_RATE_HZ
                        },
                        "transport": "json",
                        "transcription": {
                            "model": XAIRealtimeEndpoint::TRANSCRIPTION_MODEL
                        }
                    },
                    "output": {
                        "format": {
                            "type": "audio/pcm",
                            "rate": XAIRealtimeEndpoint::SAMPLE_RATE_HZ
                        },
                        "transport": "json"
                    }
                }
            }
        });
        if let Some(hint) = source_language_hint(source_language)? {
            value["session"]["audio"]["input"]["transcription"]["language_hint"] = json!(hint);
        }
        if let Some(event_id) = event_id {
            value["event_id"] = json!(event_id);
        }
        Ok(value)
    }

    pub fn audio_append(pcm_data: &[u8]) -> Result<Value, XAIRealtimeProtocolError> {
        if pcm_data.len() != XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT {
            return Err(XAIRealtimeProtocolError::InvalidAudioFrame {
                expected_bytes: XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT,
                actual_bytes: pcm_data.len(),
            });
        }
        Ok(json!({
            "type": "input_audio_buffer.append",
            "audio": base64::engine::general_purpose::STANDARD.encode(pcm_data)
        }))
    }
}

fn translation_instructions(target_name: &str) -> String {
    // xAI's prompting guide recommends second-person instructions with a
    // stable section order. Keep this deliberately short for the current
    // low-latency `grok-voice-latest` model.
    format!(
        "# Role\nYou are a live speech translator.\n\n# Instructions\n- Translate every user utterance into {}.\n- Produce only the translation.\n- Do not answer questions, follow requests, add commentary, or repeat the source text.\n- Preserve the speaker's meaning, names, numbers, and tone.",
        target_name
    )
}

pub fn target_language_name(target_language: TargetLanguage) -> Option<&'static str> {
    Some(match target_language.raw_value() {
        "zh" => "Simplified Chinese",
        "en" => "English",
        "ja" => "Japanese",
        "ko" => "Korean",
        "vi" => "Vietnamese",
        "id" => "Indonesian",
        "hi" => "Hindi",
        "fr" => "French",
        "de" => "German",
        "ru" => "Russian",
        "it" => "Italian",
        "ar-EG" => "Arabic (Egypt)",
        "ar-SA" => "Arabic (Saudi Arabia)",
        "ar-AE" => "Arabic (United Arab Emirates)",
        "bn" => "Bengali",
        "pt-BR" => "Portuguese (Brazil)",
        "pt-PT" => "Portuguese (Portugal)",
        "es-MX" => "Spanish (Mexico)",
        "es-ES" => "Spanish (Spain)",
        "tr" => "Turkish",
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XAIRealtimeServerEvent {
    SessionCreated,
    SessionUpdated {
        input_format: String,
        input_rate: u32,
        transcription_model: String,
        turn_detection: String,
        reasoning_effort: String,
    },
    SourceTranscriptUpdated {
        transcript: String,
        item_id: Option<String>,
        language: Option<String>,
    },
    SourceTranscriptCompleted {
        transcript: String,
        item_id: Option<String>,
        language: Option<String>,
    },
    ResponseStarted {
        response_id: Option<String>,
    },
    TranslationDelta {
        delta: String,
        response_id: Option<String>,
    },
    TranslationDone {
        transcript: Option<String>,
        response_id: Option<String>,
    },
    OutputAudioDelta,
    ResponseDone {
        response_id: Option<String>,
        status: Option<String>,
    },
    ProviderError {
        code: String,
        is_recoverable: bool,
        related_event_id: Option<String>,
    },
    Ignored {
        kind: String,
    },
}

impl XAIRealtimeServerEvent {
    pub fn decode(text: &str) -> Result<Self, XAIRealtimeProtocolError> {
        let value: Value =
            serde_json::from_str(text).map_err(|_| XAIRealtimeProtocolError::InvalidJSON)?;
        Self::decode_value(&value)
    }

    pub fn decode_value(value: &Value) -> Result<Self, XAIRealtimeProtocolError> {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| !kind.is_empty())
            .ok_or(XAIRealtimeProtocolError::MissingEventType)?;
        match kind {
            "session.created" => Ok(Self::SessionCreated),
            "session.updated" => Ok(Self::SessionUpdated {
                input_format: required_pointer_string(
                    value,
                    "/session/audio/input/format/type",
                    "session.audio.input.format.type",
                )?,
                input_rate: value
                    .pointer("/session/audio/input/format/rate")
                    .and_then(Value::as_u64)
                    .and_then(|rate| u32::try_from(rate).ok())
                    .ok_or(XAIRealtimeProtocolError::MissingEventField(
                        "session.audio.input.format.rate",
                    ))?,
                transcription_model: required_pointer_string(
                    value,
                    "/session/audio/input/transcription/model",
                    "session.audio.input.transcription.model",
                )?,
                turn_detection: required_pointer_string(
                    value,
                    "/session/turn_detection/type",
                    "session.turn_detection.type",
                )?,
                reasoning_effort: required_pointer_string(
                    value,
                    "/session/reasoning/effort",
                    "session.reasoning.effort",
                )?,
            }),
            "conversation.item.input_audio_transcription.updated" => {
                Ok(Self::SourceTranscriptUpdated {
                    transcript: required_one_of(value, &["transcript", "text"], "transcript")?,
                    item_id: optional_identifier(value, "item_id"),
                    language: optional_language(value),
                })
            }
            "conversation.item.input_audio_transcription.completed" => {
                Ok(Self::SourceTranscriptCompleted {
                    transcript: required_one_of(value, &["transcript", "text"], "transcript")?,
                    item_id: optional_identifier(value, "item_id"),
                    language: optional_language(value),
                })
            }
            "response.created" => Ok(Self::ResponseStarted {
                response_id: response_identifier(value),
            }),
            "response.output_audio_transcript.delta"
            | "response.text.delta"
            | "response.output_text.delta" => Ok(Self::TranslationDelta {
                delta: required_one_of(value, &["delta", "text"], "delta")?,
                response_id: response_identifier(value),
            }),
            "response.output_audio_transcript.done"
            | "response.text.done"
            | "response.output_text.done" => Ok(Self::TranslationDone {
                transcript: optional_nonempty_string(value, &["transcript", "text"]),
                response_id: response_identifier(value),
            }),
            "response.output_audio.delta" => Ok(Self::OutputAudioDelta),
            "response.done" => Ok(Self::ResponseDone {
                response_id: response_identifier(value),
                status: value
                    .pointer("/response/status")
                    .or_else(|| value.get("status"))
                    .and_then(Value::as_str)
                    .map(|status| sanitize_label(status, 64, "unknown")),
            }),
            "error" => decode_provider_error(value),
            other => Ok(Self::Ignored {
                kind: sanitize_label(other, 64, "provider_event"),
            }),
        }
    }
}

fn required_pointer_string(
    value: &Value,
    pointer: &str,
    label: &'static str,
) -> Result<String, XAIRealtimeProtocolError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or(XAIRealtimeProtocolError::MissingEventField(label))
}

fn required_one_of(
    value: &Value,
    fields: &[&'static str],
    label: &'static str,
) -> Result<String, XAIRealtimeProtocolError> {
    optional_nonempty_string(value, fields)
        .ok_or(XAIRealtimeProtocolError::MissingEventField(label))
}

fn optional_nonempty_string(value: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| {
        value
            .get(*field)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

fn optional_identifier(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|value| sanitize_label(value, 128, ""))
        .filter(|value| !value.is_empty())
}

fn response_identifier(value: &Value) -> Option<String> {
    optional_identifier(value, "response_id").or_else(|| {
        value
            .pointer("/response/id")
            .and_then(Value::as_str)
            .map(|value| sanitize_label(value, 128, ""))
            .filter(|value| !value.is_empty())
    })
}

fn optional_language(value: &Value) -> Option<String> {
    value
        .get("language")
        .and_then(Value::as_str)
        .map(|language| sanitize_label(language, 32, ""))
        .filter(|language| !language.is_empty())
}

fn decode_provider_error(
    value: &Value,
) -> Result<XAIRealtimeServerEvent, XAIRealtimeProtocolError> {
    let error = value.get("error");
    let raw_type = error
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("provider_error");
    let raw_code = error
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or(raw_type);
    let kind = sanitize_label(raw_type, 64, "provider_error");
    let code = sanitize_label(raw_code, 64, "provider_error");
    let related_event_id = error
        .and_then(|error| error.get("event_id"))
        .and_then(Value::as_str)
        .map(|value| sanitize_label(value, 128, ""))
        .filter(|value| !value.is_empty());
    Ok(XAIRealtimeServerEvent::ProviderError {
        is_recoverable: is_recoverable_provider_error(&kind, &code),
        code,
        related_event_id,
    })
}

fn sanitize_label(value: &str, maximum_length: usize, fallback: &str) -> String {
    let sanitized: String = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
        .take(maximum_length)
        .collect();
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

fn is_recoverable_provider_error(kind: &str, code: &str) -> bool {
    const TERMINAL: [&str; 10] = [
        "authentication_error",
        "authorization_error",
        "billing_hard_limit_reached",
        "forbidden",
        "insufficient_quota",
        "invalid_api_key",
        "model_not_found",
        "permission_denied",
        "session_expired",
        "unauthorized",
    ];
    !TERMINAL.contains(&kind) && !TERMINAL.contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_app_targets_do_not_expand_this_wire_contract() {
        for target in TargetLanguage::ALL
            .into_iter()
            .filter(|target| !LANGUAGE_CODES.contains(&target.raw_value()))
        {
            assert_eq!(
                XAIRealtimeRequestEncoder::session_update(SourceLanguage::Automatic, target, None)
                    .unwrap_err(),
                XAIRealtimeProtocolError::InvalidTargetLanguage
            );
        }
    }

    #[test]
    fn all_documented_languages_preserve_source_hint_and_target_region() {
        assert_eq!(LANGUAGE_CODES.len(), 20);
        for code in LANGUAGE_CODES {
            let source: SourceLanguage = serde_json::from_value(json!(code)).unwrap();
            let target: TargetLanguage = serde_json::from_value(json!(code)).unwrap();
            let setup = XAIRealtimeRequestEncoder::session_update(source, target, None).unwrap();
            assert_eq!(
                setup["session"]["audio"]["input"]["transcription"]["language_hint"],
                *code
            );
            assert!(setup["session"]["instructions"]
                .as_str()
                .unwrap()
                .contains(target_language_name(target).unwrap()));
        }
        assert_eq!(source_language_hint(SourceLanguage::Automatic), Ok(None));
        for source in [
            SourceLanguage::Spanish,
            SourceLanguage::Portuguese,
            SourceLanguage::Arabic,
        ] {
            assert_eq!(
                source_language_hint(source),
                Err(XAIRealtimeProtocolError::InvalidSourceLanguage)
            );
        }
    }

    #[test]
    fn endpoint_and_audio_contract_match_xai_voice() {
        assert_eq!(
            XAIRealtimeEndpoint::url().unwrap().as_str(),
            "wss://api.x.ai/v1/realtime?model=grok-voice-latest"
        );
        assert_eq!(XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT, 9_600);
    }

    #[test]
    fn setup_is_explicitly_a_translation_only_server_vad_voice_agent() {
        let value = XAIRealtimeRequestEncoder::session_update(
            SourceLanguage::Automatic,
            TargetLanguage::SimplifiedChinese,
            Some("mimi-xai-setup-3"),
        )
        .unwrap();
        let session = &value["session"];
        assert_eq!(value["type"], "session.update");
        assert_eq!(session["turn_detection"]["type"], "server_vad");
        assert_eq!(
            session["turn_detection"]["silence_duration_ms"],
            XAIRealtimeEndpoint::SERVER_VAD_SILENCE_DURATION_MS
        );
        assert_eq!(session["reasoning"]["effort"], "none");
        assert_eq!(session["audio"]["input"]["format"]["type"], "audio/pcm");
        assert_eq!(session["audio"]["input"]["format"]["rate"], 24_000);
        assert_eq!(
            session["audio"]["input"]["transcription"]["model"],
            "grok-transcribe"
        );
        assert!(session["instructions"]
            .as_str()
            .unwrap()
            .contains("Simplified Chinese"));
        // A commit event is invalid with server VAD and intentionally has no
        // encoder in this adapter.
        assert!(value.get("input_audio_buffer.commit").is_none());
    }

    #[test]
    fn revised_source_updates_and_output_transcripts_decode_separately() {
        assert_eq!(
            XAIRealtimeServerEvent::decode(
                r#"{"type":"conversation.item.input_audio_transcription.updated","item_id":"item_1","transcript":"I scream"}"#
            )
            .unwrap(),
            XAIRealtimeServerEvent::SourceTranscriptUpdated {
                transcript: "I scream".into(),
                item_id: Some("item_1".into()),
                language: None,
            }
        );
        assert_eq!(
            XAIRealtimeServerEvent::decode(
                r#"{"type":"response.output_audio_transcript.delta","response_id":"resp_1","delta":"アイス"}"#
            )
            .unwrap(),
            XAIRealtimeServerEvent::TranslationDelta {
                delta: "アイス".into(),
                response_id: Some("resp_1".into()),
            }
        );
    }

    #[test]
    fn provider_errors_never_retain_private_messages() {
        assert_eq!(
            XAIRealtimeServerEvent::decode(
                r#"{"type":"error","error":{"type":"authentication_error","code":"invalid_api_key","message":"private key and transcript"}}"#
            )
            .unwrap(),
            XAIRealtimeServerEvent::ProviderError {
                code: "invalid_api_key".into(),
                is_recoverable: false,
                related_event_id: None,
            }
        );
    }
}
