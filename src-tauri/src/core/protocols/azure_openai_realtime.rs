//! Azure OpenAI Realtime Translation wire protocol.
//!
//! This adapter follows Microsoft's dedicated `gpt-realtime-translate`
//! WebSocket contract. Translation and transcription deployment names are
//! supplied by the user because Azure deployments are resource-local aliases,
//! not model IDs.

use crate::core::models::TargetLanguage;
use base64::Engine;
use serde_json::{json, Value};
use thiserror::Error;

// Azure deploys the same dedicated gpt-realtime-translate model, with its
// thirteen output languages; ordinary transcription catalogs do not apply.
pub use super::openai_realtime::TRANSLATION_LANGUAGE_CODES;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AzureOpenAIRealtimeProtocolError {
    #[error("Azure OpenAI Realtime Translation requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("Enter a valid Azure OpenAI resource endpoint.")]
    InvalidResourceEndpoint,
    #[error("Enter Azure OpenAI translation and transcription deployment names.")]
    MissingDeployment,
    #[error(
        "Azure OpenAI Realtime Translation expected {expected_bytes} audio bytes, got {actual_bytes}."
    )]
    InvalidAudioFrame {
        expected_bytes: usize,
        actual_bytes: usize,
    },
    #[error("Azure OpenAI Realtime Translation returned invalid JSON.")]
    InvalidJSON,
    #[error("Azure OpenAI Realtime Translation returned an event without a type.")]
    MissingEventType,
    #[error("Azure OpenAI Realtime Translation returned an event without {0}.")]
    MissingEventField(&'static str),
}

#[derive(Debug, Clone)]
pub struct AzureOpenAIRealtimeEndpoint {
    url: url::Url,
}

impl AzureOpenAIRealtimeEndpoint {
    pub const SAMPLE_RATE_HZ: u32 = 24_000;
    pub const CHANNEL_COUNT: u16 = 1;
    pub const BITS_PER_SAMPLE: u16 = 16;
    pub const FRAME_DURATION_MS: u32 = 200;
    pub const AUDIO_FRAME_BYTE_COUNT: usize = Self::SAMPLE_RATE_HZ as usize
        * Self::CHANNEL_COUNT as usize
        * (Self::BITS_PER_SAMPLE as usize / 8)
        * Self::FRAME_DURATION_MS as usize
        / 1_000;

    pub fn new(
        resource_endpoint: &str,
        deployment: &str,
    ) -> Result<Self, AzureOpenAIRealtimeProtocolError> {
        let deployment = deployment.trim();
        if deployment.is_empty() {
            return Err(AzureOpenAIRealtimeProtocolError::MissingDeployment);
        }

        let mut url = url::Url::parse(resource_endpoint.trim())
            .map_err(|_| AzureOpenAIRealtimeProtocolError::InvalidResourceEndpoint)?;
        let official_host = url
            .host_str()
            .map(str::to_ascii_lowercase)
            .is_some_and(|host| {
                [".openai.azure.com", ".openai.azure.cn", ".openai.azure.us"]
                    .iter()
                    .any(|suffix| host.ends_with(suffix) && host.len() > suffix.len())
            });
        if !matches!(url.scheme(), "https" | "wss")
            || !official_host
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err(AzureOpenAIRealtimeProtocolError::InvalidResourceEndpoint);
        }
        url.set_scheme("wss")
            .map_err(|_| AzureOpenAIRealtimeProtocolError::InvalidResourceEndpoint)?;
        url.set_path("/openai/v1/realtime/translations");
        url.query_pairs_mut().append_pair("model", deployment);
        Ok(Self { url })
    }

    pub fn url(&self) -> &url::Url {
        &self.url
    }
}

pub struct AzureOpenAIRealtimeRequestEncoder;

impl AzureOpenAIRealtimeRequestEncoder {
    pub fn session_update(
        target_language: TargetLanguage,
        transcription_deployment: &str,
        event_id: Option<&str>,
    ) -> Result<Value, AzureOpenAIRealtimeProtocolError> {
        if !TRANSLATION_LANGUAGE_CODES.contains(&target_language.raw_value()) {
            return Err(AzureOpenAIRealtimeProtocolError::InvalidTargetLanguage);
        }
        let transcription_deployment = transcription_deployment.trim();
        if transcription_deployment.is_empty() {
            return Err(AzureOpenAIRealtimeProtocolError::MissingDeployment);
        }
        // Azure requires the existing transcription deployment name here;
        // the translation deployment remains selected in the WebSocket URL.
        let mut value = json!({
            "type": "session.update",
            "session": {
                "audio": {
                    "input": {
                        "transcription": {
                            "model": transcription_deployment
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

    pub fn audio_append(pcm_data: &[u8]) -> Result<Value, AzureOpenAIRealtimeProtocolError> {
        if pcm_data.len() != AzureOpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT {
            return Err(AzureOpenAIRealtimeProtocolError::InvalidAudioFrame {
                expected_bytes: AzureOpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureTranslationStream {
    DedicatedSession,
    ResponseText,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AzureOpenAIRealtimeServerEvent {
    SessionCreated,
    SessionUpdated {
        target_language: String,
    },
    SourceTranscriptDelta {
        text: String,
        elapsed_ms: Option<u64>,
    },
    TranslationTranscriptDelta {
        text: String,
        elapsed_ms: Option<u64>,
        stream: AzureTranslationStream,
    },
    TranslationTranscriptDone {
        text: Option<String>,
        stream: AzureTranslationStream,
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

impl AzureOpenAIRealtimeServerEvent {
    pub fn decode(text: &str) -> Result<Self, AzureOpenAIRealtimeProtocolError> {
        let value: Value = serde_json::from_str(text)
            .map_err(|_| AzureOpenAIRealtimeProtocolError::InvalidJSON)?;
        Self::decode_value(&value)
    }

    pub fn decode_value(value: &Value) -> Result<Self, AzureOpenAIRealtimeProtocolError> {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| !kind.is_empty())
            .ok_or(AzureOpenAIRealtimeProtocolError::MissingEventType)?;

        match kind {
            "session.created" => Ok(Self::SessionCreated),
            "session.updated" => Ok(Self::SessionUpdated {
                target_language: value
                    .pointer("/session/audio/output/language")
                    .and_then(Value::as_str)
                    .filter(|language| !language.is_empty())
                    .ok_or(AzureOpenAIRealtimeProtocolError::MissingEventField(
                        "session.audio.output.language",
                    ))?
                    .to_string(),
            }),
            "session.input_transcript.delta" => Ok(Self::SourceTranscriptDelta {
                text: required_string(value, "delta")?,
                elapsed_ms: elapsed_ms(value),
            }),
            "session.output_transcript.delta" => Ok(Self::TranslationTranscriptDelta {
                text: required_string(value, "delta")?,
                elapsed_ms: elapsed_ms(value),
                stream: AzureTranslationStream::DedicatedSession,
            }),
            "session.output_transcript.done" => Ok(Self::TranslationTranscriptDone {
                text: optional_nonempty_string(value, &["transcript", "text"]),
                stream: AzureTranslationStream::DedicatedSession,
            }),
            "response.text.delta" | "response.output_text.delta" => {
                Ok(Self::TranslationTranscriptDelta {
                    // Microsoft's official example uses `text`; the OpenAI GA
                    // compatibility event uses `delta`.
                    text: required_one_of(value, &["text", "delta"], "text")?,
                    elapsed_ms: elapsed_ms(value),
                    stream: AzureTranslationStream::ResponseText,
                })
            }
            "response.text.done" | "response.output_text.done" => {
                Ok(Self::TranslationTranscriptDone {
                    text: optional_nonempty_string(value, &["text", "transcript"]),
                    stream: AzureTranslationStream::ResponseText,
                })
            }
            "session.output_audio.delta" | "response.output_audio.delta" => {
                Ok(Self::OutputAudioDelta)
            }
            "session.closed" => Ok(Self::SessionClosed),
            "error" => decode_provider_error(value),
            other => Ok(Self::Ignored {
                kind: sanitize_label(other, 64, "provider_event"),
            }),
        }
    }
}

fn required_string(
    value: &Value,
    field: &'static str,
) -> Result<String, AzureOpenAIRealtimeProtocolError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(AzureOpenAIRealtimeProtocolError::MissingEventField(field))
}

fn required_one_of(
    value: &Value,
    fields: &[&'static str],
    label: &'static str,
) -> Result<String, AzureOpenAIRealtimeProtocolError> {
    optional_nonempty_string(value, fields)
        .ok_or(AzureOpenAIRealtimeProtocolError::MissingEventField(label))
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

fn elapsed_ms(value: &Value) -> Option<u64> {
    value.get("elapsed_ms").and_then(Value::as_u64)
}

fn decode_provider_error(
    value: &Value,
) -> Result<AzureOpenAIRealtimeServerEvent, AzureOpenAIRealtimeProtocolError> {
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
    Ok(AzureOpenAIRealtimeServerEvent::ProviderError {
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
            .filter(|target| !TRANSLATION_LANGUAGE_CODES.contains(&target.raw_value()))
        {
            assert_eq!(
                AzureOpenAIRealtimeRequestEncoder::session_update(
                    target,
                    "synthetic-deployment",
                    None
                )
                .unwrap_err(),
                AzureOpenAIRealtimeProtocolError::InvalidTargetLanguage
            );
        }
    }

    #[test]
    fn all_dedicated_model_targets_are_configurable() {
        for code in TRANSLATION_LANGUAGE_CODES {
            let target = serde_json::from_value(json!(code)).unwrap();
            let setup = AzureOpenAIRealtimeRequestEncoder::session_update(
                target,
                "synthetic-transcription",
                None,
            )
            .unwrap();
            assert_eq!(setup["session"]["audio"]["output"]["language"], *code);
            assert_eq!(
                setup["session"]["audio"]["input"]["transcription"]["model"],
                "synthetic-transcription"
            );
        }
    }

    #[test]
    fn endpoint_uses_resource_host_and_escaped_deployment() {
        let endpoint = AzureOpenAIRealtimeEndpoint::new(
            "https://mimi.openai.azure.com/",
            "translate production",
        )
        .unwrap();
        assert_eq!(
            endpoint.url().as_str(),
            "wss://mimi.openai.azure.com/openai/v1/realtime/translations?model=translate+production"
        );
        assert_eq!(AzureOpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT, 9_600);
    }

    #[test]
    fn endpoint_rejects_non_base_or_insecure_urls() {
        assert!(AzureOpenAIRealtimeEndpoint::new("http://mimi.example", "deployment").is_err());
        assert!(AzureOpenAIRealtimeEndpoint::new("https://mimi.example", "deployment").is_err());
        assert!(AzureOpenAIRealtimeEndpoint::new(
            "https://mimi.openai.azure.com/openai/v1",
            "deployment"
        )
        .is_err());
        assert!(AzureOpenAIRealtimeEndpoint::new(
            "https://mimi.openai.azure.com/?api-key=secret",
            "deployment"
        )
        .is_err());
    }

    #[test]
    fn session_update_configures_translation_and_source_transcription() {
        let value = AzureOpenAIRealtimeRequestEncoder::session_update(
            TargetLanguage::Japanese,
            "gpt-4o-transcribe-deployment",
            Some("mimi-setup-1"),
        )
        .unwrap();
        assert_eq!(value["type"], "session.update");
        assert_eq!(value["event_id"], "mimi-setup-1");
        assert_eq!(value["session"]["audio"]["output"]["language"], "ja");
        assert_eq!(
            value["session"]["audio"]["input"]["transcription"]["model"],
            "gpt-4o-transcribe-deployment"
        );
    }

    #[test]
    fn decodes_dedicated_and_microsoft_example_translation_events() {
        assert_eq!(
            AzureOpenAIRealtimeServerEvent::decode(
                r#"{"type":"session.output_transcript.delta","delta":"こんにちは。"}"#
            )
            .unwrap(),
            AzureOpenAIRealtimeServerEvent::TranslationTranscriptDelta {
                text: "こんにちは。".into(),
                elapsed_ms: None,
                stream: AzureTranslationStream::DedicatedSession,
            }
        );
        assert_eq!(
            AzureOpenAIRealtimeServerEvent::decode(
                r#"{"type":"response.text.delta","text":"Bonjour"}"#
            )
            .unwrap(),
            AzureOpenAIRealtimeServerEvent::TranslationTranscriptDelta {
                text: "Bonjour".into(),
                elapsed_ms: None,
                stream: AzureTranslationStream::ResponseText,
            }
        );
        assert_eq!(
            AzureOpenAIRealtimeServerEvent::decode(r#"{"type":"response.text.done"}"#).unwrap(),
            AzureOpenAIRealtimeServerEvent::TranslationTranscriptDone {
                text: None,
                stream: AzureTranslationStream::ResponseText,
            }
        );
    }

    #[test]
    fn provider_errors_do_not_expose_messages() {
        assert_eq!(
            AzureOpenAIRealtimeServerEvent::decode(
                r#"{"type":"error","error":{"type":"authentication_error","code":"invalid_api_key","message":"key sk-private"}}"#
            )
            .unwrap(),
            AzureOpenAIRealtimeServerEvent::ProviderError {
                code: "invalid_api_key".into(),
                is_recoverable: false,
                related_event_id: None,
            }
        );
    }
}
