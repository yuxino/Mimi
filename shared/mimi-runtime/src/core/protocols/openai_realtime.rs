//! OpenAI Realtime Translation wire protocol.

use crate::core::models::TargetLanguage;
use base64::Engine;
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OpenAIRealtimeProtocolError {
    #[error("OpenAI Realtime Translation requires a translated output language.")]
    InvalidTargetLanguage,
    #[error(
        "OpenAI Realtime Translation expected {expected_bytes} audio bytes, got {actual_bytes}."
    )]
    InvalidAudioFrame {
        expected_bytes: usize,
        actual_bytes: usize,
    },
    #[error("OpenAI Realtime Translation returned an invalid response.")]
    InvalidJSON,
    #[error("OpenAI Realtime Translation returned an event without a type.")]
    MissingEventType,
    #[error("OpenAI Realtime Translation returned an event without {0}.")]
    MissingEventField(&'static str),
    #[error("The OpenAI Realtime Translation endpoint could not be created.")]
    InvalidEndpoint,
}

/// Dedicated realtime translation outputs; do not borrow ordinary transcription languages.
pub const TRANSLATION_LANGUAGE_CODES: &[&str] = &[
    "zh", "en", "ja", "ko", "ru", "es", "fr", "pt", "de", "it", "vi", "id", "hi",
];

pub struct OpenAIRealtimeEndpoint;

impl OpenAIRealtimeEndpoint {
    pub const MODEL: &'static str = "gpt-realtime-translate";
    pub const SOURCE_TRANSCRIPTION_MODEL: &'static str = "gpt-realtime-whisper";
    pub const SAMPLE_RATE_HZ: u32 = 24_000;
    pub const CHANNEL_COUNT: u16 = 1;
    pub const BITS_PER_SAMPLE: u16 = 16;
    pub const FRAME_DURATION_MS: u32 = 200;
    pub const AUDIO_FRAME_BYTE_COUNT: usize = Self::SAMPLE_RATE_HZ as usize
        * Self::CHANNEL_COUNT as usize
        * (Self::BITS_PER_SAMPLE as usize / 8)
        * Self::FRAME_DURATION_MS as usize
        / 1_000;

    pub fn url() -> Result<url::Url, OpenAIRealtimeProtocolError> {
        url::Url::parse(&format!(
            "wss://api.openai.com/v1/realtime/translations?model={}",
            Self::MODEL
        ))
        .map_err(|_| OpenAIRealtimeProtocolError::InvalidEndpoint)
    }
}

pub struct OpenAIRealtimeRequestEncoder;

impl OpenAIRealtimeRequestEncoder {
    pub fn session_update(
        target_language: TargetLanguage,
        event_id: Option<&str>,
    ) -> Result<Value, OpenAIRealtimeProtocolError> {
        if !TRANSLATION_LANGUAGE_CODES.contains(&target_language.raw_value()) {
            return Err(OpenAIRealtimeProtocolError::InvalidTargetLanguage);
        }
        let mut value = json!({
            "type": "session.update",
            "session": {
                "audio": {
                    "input": {
                        "transcription": {
                            "model": OpenAIRealtimeEndpoint::SOURCE_TRANSCRIPTION_MODEL
                        }
                    },
                    "output": {
                        "language": target_language.raw_value()
                    }
                }
            }
        });
        if let Some(event_id) = event_id {
            value["event_id"] = json!(event_id);
        }
        Ok(value)
    }

    pub fn audio_append(pcm_data: &[u8]) -> Result<Value, OpenAIRealtimeProtocolError> {
        if pcm_data.len() != OpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT {
            return Err(OpenAIRealtimeProtocolError::InvalidAudioFrame {
                expected_bytes: OpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT,
                actual_bytes: pcm_data.len(),
            });
        }
        Ok(json!({
            "type": "session.input_audio_buffer.append",
            "audio": base64::engine::general_purpose::STANDARD.encode(pcm_data)
        }))
    }

    pub fn close() -> Value {
        json!({ "type": "session.close" })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenAIRealtimeServerEvent {
    SessionCreated,
    SessionUpdated {
        source_transcription_model: String,
        target_language: String,
    },
    SourceTranscriptDelta {
        text: String,
        elapsed_ms: Option<u64>,
    },
    TranslationTranscriptDelta {
        text: String,
        elapsed_ms: Option<u64>,
    },
    OutputAudioDelta,
    SessionClosed,
    ProviderError {
        code: String,
        is_recoverable: bool,
        related_event_id: Option<String>,
    },
    Ignored {
        kind: String,
    },
}

impl OpenAIRealtimeServerEvent {
    pub fn decode(text: &str) -> Result<Self, OpenAIRealtimeProtocolError> {
        let value: Value =
            serde_json::from_str(text).map_err(|_| OpenAIRealtimeProtocolError::InvalidJSON)?;
        Self::decode_value(&value)
    }

    pub fn decode_value(value: &Value) -> Result<Self, OpenAIRealtimeProtocolError> {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| !kind.is_empty())
            .ok_or(OpenAIRealtimeProtocolError::MissingEventType)?;

        match kind {
            "session.created" => Ok(Self::SessionCreated),
            "session.updated" => {
                let source_transcription_model = value
                    .pointer("/session/audio/input/transcription/model")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or(OpenAIRealtimeProtocolError::MissingEventField(
                        "session.audio configuration",
                    ))?;
                let target_language = value
                    .pointer("/session/audio/output/language")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or(OpenAIRealtimeProtocolError::MissingEventField(
                        "session.audio configuration",
                    ))?;
                Ok(Self::SessionUpdated {
                    source_transcription_model: source_transcription_model.to_string(),
                    target_language: target_language.to_string(),
                })
            }
            "session.input_transcript.delta" => Ok(Self::SourceTranscriptDelta {
                text: required_string(value, "delta")?,
                elapsed_ms: elapsed_ms(value),
            }),
            "session.output_transcript.delta" => Ok(Self::TranslationTranscriptDelta {
                text: required_string(value, "delta")?,
                elapsed_ms: elapsed_ms(value),
            }),
            "session.output_audio.delta" => Ok(Self::OutputAudioDelta),
            "session.closed" => Ok(Self::SessionClosed),
            "error" => {
                let error = value.get("error");
                let raw_type = error
                    .and_then(|error| error.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("provider_error");
                let raw_code = error
                    .and_then(|error| error.get("code"))
                    .and_then(Value::as_str)
                    .unwrap_or(raw_type);
                let sanitized_type = sanitize_label(raw_type, 64, "provider_error");
                let code = sanitize_label(raw_code, 64, "provider_error");
                let related_event_id = error
                    .and_then(|error| error.get("event_id"))
                    .and_then(Value::as_str)
                    .map(|value| sanitize_label(value, 128, ""))
                    .filter(|value| !value.is_empty());
                Ok(Self::ProviderError {
                    is_recoverable: is_recoverable_provider_error(&sanitized_type, &code),
                    code,
                    related_event_id,
                })
            }
            other => Ok(Self::Ignored {
                kind: sanitize_label(other, 64, "provider_event"),
            }),
        }
    }
}

fn required_string(
    value: &Value,
    field: &'static str,
) -> Result<String, OpenAIRealtimeProtocolError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(OpenAIRealtimeProtocolError::MissingEventField(field))
}

fn elapsed_ms(value: &Value) -> Option<u64> {
    value.get("elapsed_ms").and_then(Value::as_u64)
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
            .filter(|target| !TRANSLATION_LANGUAGE_CODES.contains(&target.raw_value()))
        {
            assert_eq!(
                OpenAIRealtimeRequestEncoder::session_update(target, None).unwrap_err(),
                OpenAIRealtimeProtocolError::InvalidTargetLanguage
            );
        }
    }

    #[test]
    fn endpoint_and_audio_contract_are_fixed() {
        assert_eq!(
            OpenAIRealtimeEndpoint::url().unwrap().as_str(),
            "wss://api.openai.com/v1/realtime/translations?model=gpt-realtime-translate"
        );
        assert_eq!(OpenAIRealtimeEndpoint::SAMPLE_RATE_HZ, 24_000);
        assert_eq!(OpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT, 9_600);
    }

    #[test]
    fn session_update_configures_transcription_and_target() {
        let value = OpenAIRealtimeRequestEncoder::session_update(
            TargetLanguage::Japanese,
            Some("mimi-session-update-7"),
        )
        .unwrap();
        assert_eq!(value["type"], "session.update");
        assert_eq!(value["event_id"], "mimi-session-update-7");
        assert_eq!(
            value["session"]["audio"]["input"]["transcription"]["model"],
            "gpt-realtime-whisper"
        );
        assert_eq!(value["session"]["audio"]["output"]["language"], "ja");
        assert!(matches!(
            OpenAIRealtimeRequestEncoder::session_update(TargetLanguage::Original, None),
            Err(OpenAIRealtimeProtocolError::InvalidTargetLanguage)
        ));
    }

    #[test]
    fn audio_append_requires_one_exact_frame() {
        let frame = vec![7; 9_600];
        let value = OpenAIRealtimeRequestEncoder::audio_append(&frame).unwrap();
        assert_eq!(value["type"], "session.input_audio_buffer.append");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(value["audio"].as_str().unwrap())
                .unwrap(),
            frame
        );
        assert!(matches!(
            OpenAIRealtimeRequestEncoder::audio_append(&frame[..9_599]),
            Err(OpenAIRealtimeProtocolError::InvalidAudioFrame { .. })
        ));
    }

    #[test]
    fn transcript_timing_and_audio_events_decode_without_audio_data() {
        assert_eq!(
            OpenAIRealtimeServerEvent::decode(
                r#"{"type":"session.input_transcript.delta","delta":" hello ","elapsed_ms":1200}"#
            )
            .unwrap(),
            OpenAIRealtimeServerEvent::SourceTranscriptDelta {
                text: " hello ".into(),
                elapsed_ms: Some(1_200)
            }
        );
        assert_eq!(
            OpenAIRealtimeServerEvent::decode(
                r#"{"type":"session.output_audio.delta","delta":"private-audio"}"#
            )
            .unwrap(),
            OpenAIRealtimeServerEvent::OutputAudioDelta
        );
    }

    #[test]
    fn errors_expose_only_sanitized_labels() {
        let event = OpenAIRealtimeServerEvent::decode(
            r#"{"type":"error","error":{"code":"bad code/<secret>","message":"recognized private words","event_id":"setup/<private>"}}"#,
        )
        .unwrap();
        assert_eq!(
            event,
            OpenAIRealtimeServerEvent::ProviderError {
                code: "badcodesecret".into(),
                is_recoverable: true,
                related_event_id: Some("setupprivate".into())
            }
        );
        let description = format!("{event:?}");
        assert!(!description.contains("recognized private words"));
    }

    #[test]
    fn authentication_errors_are_terminal() {
        assert_eq!(
            OpenAIRealtimeServerEvent::decode(
                r#"{"type":"error","error":{"type":"authentication_error","code":"invalid_api_key","message":"private"}}"#
            )
            .unwrap(),
            OpenAIRealtimeServerEvent::ProviderError {
                code: "invalid_api_key".into(),
                is_recoverable: false,
                related_event_id: None
            }
        );
    }
}
