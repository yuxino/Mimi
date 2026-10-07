//! Google Gemini Live Translation wire protocol.

use crate::core::models::TargetLanguage;
use base64::Engine;
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GeminiLiveProtocolError {
    #[error("Gemini Live Translation requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("Gemini Live Translation expected {expected_bytes} audio bytes, got {actual_bytes}.")]
    InvalidAudioFrame {
        expected_bytes: usize,
        actual_bytes: usize,
    },
    #[error("Gemini Live Translation returned an invalid response.")]
    InvalidJSON,
    #[error("Gemini Live Translation returned an event without a recognized kind.")]
    MissingEventKind,
    #[error("Gemini Live Translation returned an event without {0}.")]
    MissingEventField(&'static str),
    #[error("The Gemini Live Translation endpoint could not be created.")]
    InvalidEndpoint,
}

pub struct GeminiLiveEndpoint;

impl GeminiLiveEndpoint {
    pub const MODEL: &'static str = "gemini-3.5-live-translate-preview";
    pub const SAMPLE_RATE_HZ: u32 = 16_000;
    pub const CHANNEL_COUNT: u16 = 1;
    pub const BITS_PER_SAMPLE: u16 = 16;
    pub const FRAME_DURATION_MS: u32 = 100;
    pub const AUDIO_FRAME_BYTE_COUNT: usize = Self::SAMPLE_RATE_HZ as usize
        * Self::CHANNEL_COUNT as usize
        * (Self::BITS_PER_SAMPLE as usize / 8)
        * Self::FRAME_DURATION_MS as usize
        / 1_000;
    pub const AUDIO_MIME_TYPE: &'static str = "audio/pcm;rate=16000";

    /// Returns the credential-free endpoint. The API key is attached only to
    /// the transient connection request so it is never retained in this type.
    pub fn url() -> Result<url::Url, GeminiLiveProtocolError> {
        url::Url::parse(
            "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent",
        )
        .map_err(|_| GeminiLiveProtocolError::InvalidEndpoint)
    }
}

pub struct GeminiLiveRequestEncoder;

impl GeminiLiveRequestEncoder {
    pub fn setup(target_language: TargetLanguage) -> Result<Value, GeminiLiveProtocolError> {
        let target_language_code = target_language_code(target_language)?;
        Ok(json!({
            "setup": {
                "model": format!("models/{}", GeminiLiveEndpoint::MODEL),
                "inputAudioTranscription": {},
                "outputAudioTranscription": {},
                "generationConfig": {
                    "responseModalities": ["AUDIO"],
                    "translationConfig": {
                        "targetLanguageCode": target_language_code,
                        "echoTargetLanguage": true
                    }
                }
            }
        }))
    }

    pub fn audio(pcm_data: &[u8]) -> Result<Value, GeminiLiveProtocolError> {
        if pcm_data.len() != GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT {
            return Err(GeminiLiveProtocolError::InvalidAudioFrame {
                expected_bytes: GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT,
                actual_bytes: pcm_data.len(),
            });
        }
        Ok(json!({
            "realtimeInput": {
                "audio": {
                    "data": base64::engine::general_purpose::STANDARD.encode(pcm_data),
                    "mimeType": GeminiLiveEndpoint::AUDIO_MIME_TYPE
                }
            }
        }))
    }

    pub fn audio_stream_end() -> Value {
        json!({
            "realtimeInput": {
                "audioStreamEnd": true
            }
        })
    }
}

/// Exact gemini-3.5-live-translate-preview catalog, expressed in Mimi codes.
/// https://ai.google.dev/gemini-api/docs/live-api/live-translate#supported-languages
/// Norwegian uses the documented `no` alias; `nb` is not a separate language.
pub const LANGUAGE_CODES: &[&str] = &[
    "zh", "en", "ja", "zh_tw", "ko", "ru", "es", "fr", "de", "it", "th", "vi", "id", "ms", "ar",
    "hi", "he", "ur", "bn", "pl", "nl", "tr", "km", "cs", "sv", "hu", "da", "fi", "tl", "fa", "af",
    "ak", "sq", "am", "hy", "az", "eu", "be", "bg", "my", "ca", "hr", "et", "gl", "ka", "el", "gu",
    "ha", "is", "jv", "kn", "kk", "rw", "lo", "lv", "lt", "mk", "ml", "mr", "mn", "ne", "no",
    "pt-BR", "pt-PT", "pa", "ro", "sr", "sd", "si", "sk", "sl", "su", "sw", "ta", "te", "uk", "uz",
    "zu",
];

pub fn target_language_code(
    target_language: TargetLanguage,
) -> Result<&'static str, GeminiLiveProtocolError> {
    if !LANGUAGE_CODES.contains(&target_language.raw_value()) {
        return Err(GeminiLiveProtocolError::InvalidTargetLanguage);
    }
    match target_language {
        TargetLanguage::SimplifiedChinese => Ok("zh-Hans"),
        TargetLanguage::TraditionalChinese => Ok("zh-Hant"),
        TargetLanguage::Tagalog => Ok("fil"),
        _ => Ok(target_language.raw_value()),
    }
}

/// A single Gemini server envelope can contain transcription updates and a
/// turn boundary together, so decoding returns every meaningful event in wire
/// order. Audio payloads are represented only by a marker and never retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeminiLiveServerEvent {
    SetupComplete,
    SourceTranscript {
        text: String,
        language_code: Option<String>,
    },
    TranslationTranscript {
        text: String,
        language_code: Option<String>,
    },
    OutputAudio,
    GenerationComplete,
    Interrupted,
    TurnComplete,
    GoAway,
    ProviderError {
        code: String,
        is_recoverable: bool,
    },
    Ignored {
        kind: String,
    },
}

impl GeminiLiveServerEvent {
    pub fn decode(text: &str) -> Result<Vec<Self>, GeminiLiveProtocolError> {
        let value: Value =
            serde_json::from_str(text).map_err(|_| GeminiLiveProtocolError::InvalidJSON)?;
        Self::decode_value(&value)
    }

    pub fn decode_value(value: &Value) -> Result<Vec<Self>, GeminiLiveProtocolError> {
        if value.get("setupComplete").is_some() {
            return Ok(vec![Self::SetupComplete]);
        }
        if let Some(error) = value.get("error") {
            let raw_status = error.get("status").and_then(Value::as_str).unwrap_or("");
            let code = if raw_status.is_empty() {
                error
                    .get("code")
                    .and_then(Value::as_i64)
                    .map(|code| format!("http_{code}"))
                    .unwrap_or_else(|| "provider_error".to_string())
            } else {
                sanitize_label(raw_status, 64, "provider_error")
            };
            return Ok(vec![Self::ProviderError {
                is_recoverable: is_recoverable_provider_error(&code),
                code,
            }]);
        }
        if value.get("goAway").is_some() {
            return Ok(vec![Self::GoAway]);
        }
        if let Some(content) = value.get("serverContent") {
            return decode_server_content(content);
        }

        let kind = value
            .as_object()
            .and_then(|object| object.keys().next())
            .map(|kind| sanitize_label(kind, 64, "provider_event"))
            .ok_or(GeminiLiveProtocolError::MissingEventKind)?;
        Ok(vec![Self::Ignored { kind }])
    }
}

fn decode_server_content(
    content: &Value,
) -> Result<Vec<GeminiLiveServerEvent>, GeminiLiveProtocolError> {
    let mut events = Vec::new();

    if let Some(transcription) = content.get("inputTranscription") {
        let text =
            required_transcript_text(transcription, "serverContent.inputTranscription.text")?;
        if !text.is_empty() {
            events.push(GeminiLiveServerEvent::SourceTranscript {
                text,
                language_code: language_code(transcription),
            });
        }
    }
    if let Some(transcription) = content.get("outputTranscription") {
        let text =
            required_transcript_text(transcription, "serverContent.outputTranscription.text")?;
        if !text.is_empty() {
            events.push(GeminiLiveServerEvent::TranslationTranscript {
                text,
                language_code: language_code(transcription),
            });
        }
    }
    let has_audio = content
        .pointer("/modelTurn/parts")
        .and_then(Value::as_array)
        .is_some_and(|parts| parts.iter().any(|part| part.get("inlineData").is_some()));
    if has_audio {
        events.push(GeminiLiveServerEvent::OutputAudio);
    }
    if content
        .get("generationComplete")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        events.push(GeminiLiveServerEvent::GenerationComplete);
    }
    if content
        .get("interrupted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        events.push(GeminiLiveServerEvent::Interrupted);
    }
    // The boundary is deliberately last: transcript fields can share this
    // envelope and must be appended before the client commits the pair.
    if content
        .get("turnComplete")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        events.push(GeminiLiveServerEvent::TurnComplete);
    }

    if events.is_empty() {
        events.push(GeminiLiveServerEvent::Ignored {
            kind: "serverContent".into(),
        });
    }
    Ok(events)
}

fn required_transcript_text(
    transcription: &Value,
    field: &'static str,
) -> Result<String, GeminiLiveProtocolError> {
    match transcription.get("text") {
        Some(Value::String(text)) => Ok(text.clone()),
        // Protobuf JSON omits an empty string. Live Translate sends legal
        // language-only transcription updates during silence.
        None if transcription.is_object() => Ok(String::new()),
        _ => Err(GeminiLiveProtocolError::MissingEventField(field)),
    }
}

fn language_code(transcription: &Value) -> Option<String> {
    transcription
        .get("languageCode")
        .and_then(Value::as_str)
        .map(|value| sanitize_label(value, 32, ""))
        .filter(|value| !value.is_empty())
}

fn sanitize_label(value: &str, maximum_length: usize, fallback: &str) -> String {
    let sanitized: String = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
        .flat_map(char::to_lowercase)
        .take(maximum_length)
        .collect();
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

fn is_recoverable_provider_error(code: &str) -> bool {
    const TERMINAL: [&str; 10] = [
        "failed_precondition",
        "http_400",
        "http_401",
        "http_403",
        "http_404",
        "invalid_argument",
        "not_found",
        "permission_denied",
        "unauthenticated",
        "unimplemented",
    ];
    !TERMINAL.contains(&code)
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
                target_language_code(target).unwrap_err(),
                GeminiLiveProtocolError::InvalidTargetLanguage
            );
        }
    }

    #[test]
    fn every_documented_live_translation_target_has_an_exact_wire_code() {
        assert_eq!(LANGUAGE_CODES.len(), 78);
        for code in LANGUAGE_CODES {
            let target = serde_json::from_value(json!(code)).unwrap();
            let expected = match *code {
                "zh" => "zh-Hans",
                "zh_tw" => "zh-Hant",
                "tl" => "fil",
                _ => code,
            };
            assert_eq!(target_language_code(target).unwrap(), expected);
            let setup = GeminiLiveRequestEncoder::setup(target).unwrap();
            assert_eq!(
                setup["setup"]["generationConfig"]["translationConfig"]["targetLanguageCode"],
                expected
            );
            assert_eq!(setup["setup"]["inputAudioTranscription"], json!({}));
        }
    }

    #[test]
    fn endpoint_and_audio_contract_match_gemini_live_translation() {
        assert_eq!(
            GeminiLiveEndpoint::url().unwrap().as_str(),
            "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent"
        );
        assert_eq!(GeminiLiveEndpoint::SAMPLE_RATE_HZ, 16_000);
        assert_eq!(GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT, 3_200);
    }

    #[test]
    fn setup_enables_audio_both_transcripts_and_live_translation() {
        let value = GeminiLiveRequestEncoder::setup(TargetLanguage::Japanese).unwrap();
        assert_eq!(
            value["setup"]["model"],
            "models/gemini-3.5-live-translate-preview"
        );
        let generation = &value["setup"]["generationConfig"];
        assert_eq!(generation["responseModalities"], json!(["AUDIO"]));
        assert_eq!(value["setup"]["inputAudioTranscription"], json!({}));
        assert_eq!(value["setup"]["outputAudioTranscription"], json!({}));
        assert!(generation.get("inputAudioTranscription").is_none());
        assert!(generation.get("outputAudioTranscription").is_none());
        assert_eq!(generation["translationConfig"]["targetLanguageCode"], "ja");
        assert_eq!(generation["translationConfig"]["echoTargetLanguage"], true);
        assert_eq!(
            GeminiLiveRequestEncoder::setup(TargetLanguage::SimplifiedChinese).unwrap()["setup"]
                ["generationConfig"]["translationConfig"]["targetLanguageCode"],
            "zh-Hans"
        );
        assert!(matches!(
            GeminiLiveRequestEncoder::setup(TargetLanguage::Original),
            Err(GeminiLiveProtocolError::InvalidTargetLanguage)
        ));
    }

    #[test]
    fn audio_requires_one_exact_hundred_millisecond_pcm_frame() {
        let frame = vec![0x17; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT];
        let value = GeminiLiveRequestEncoder::audio(&frame).unwrap();
        assert_eq!(
            value["realtimeInput"]["audio"]["mimeType"],
            "audio/pcm;rate=16000"
        );
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(value["realtimeInput"]["audio"]["data"].as_str().unwrap())
                .unwrap(),
            frame
        );
        assert!(matches!(
            GeminiLiveRequestEncoder::audio(&frame[..frame.len() - 1]),
            Err(GeminiLiveProtocolError::InvalidAudioFrame { .. })
        ));
        assert_eq!(
            GeminiLiveRequestEncoder::audio_stream_end(),
            json!({"realtimeInput": {"audioStreamEnd": true}})
        );
    }

    #[test]
    fn one_server_fixture_decodes_transcripts_audio_and_boundary_in_order() {
        let events = GeminiLiveServerEvent::decode(
            r#"{
                "serverContent": {
                    "inputTranscription": {"text":"Hello ","languageCode":"en-US"},
                    "outputTranscription": {"text":"こんにちは ","languageCode":"ja-JP"},
                    "modelTurn": {"parts":[{"inlineData":{"mimeType":"audio/pcm;rate=24000","data":"private-audio"}}]},
                    "generationComplete": true,
                    "turnComplete": true
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            events,
            vec![
                GeminiLiveServerEvent::SourceTranscript {
                    text: "Hello ".into(),
                    language_code: Some("en-us".into())
                },
                GeminiLiveServerEvent::TranslationTranscript {
                    text: "こんにちは ".into(),
                    language_code: Some("ja-jp".into())
                },
                GeminiLiveServerEvent::OutputAudio,
                GeminiLiveServerEvent::GenerationComplete,
                GeminiLiveServerEvent::TurnComplete,
            ]
        );
        assert!(!format!("{events:?}").contains("private-audio"));
    }

    #[test]
    fn setup_and_stream_lifecycle_fixtures_decode() {
        assert_eq!(
            GeminiLiveServerEvent::decode(r#"{"setupComplete":{}}"#).unwrap(),
            vec![GeminiLiveServerEvent::SetupComplete]
        );
        assert_eq!(
            GeminiLiveServerEvent::decode(
                r#"{"serverContent":{"interrupted":true,"turnComplete":true}}"#
            )
            .unwrap(),
            vec![
                GeminiLiveServerEvent::Interrupted,
                GeminiLiveServerEvent::TurnComplete
            ]
        );
        assert_eq!(
            GeminiLiveServerEvent::decode(r#"{"goAway":{"timeLeft":"private"}}"#).unwrap(),
            vec![GeminiLiveServerEvent::GoAway]
        );
    }

    #[test]
    fn provider_errors_expose_only_a_sanitized_code() {
        let events = GeminiLiveServerEvent::decode(
            r#"{"error":{"code":403,"status":"PERMISSION_DENIED","message":"recognized private words and api key"}}"#,
        )
        .unwrap();
        assert_eq!(
            events,
            vec![GeminiLiveServerEvent::ProviderError {
                code: "permission_denied".into(),
                is_recoverable: false,
            }]
        );
        let description = format!("{events:?}");
        assert!(!description.contains("recognized private words"));
        assert!(!description.contains("api key"));
    }

    #[test]
    fn invalid_and_incomplete_fixtures_fail_content_free() {
        assert_eq!(
            GeminiLiveServerEvent::decode("private invalid json").unwrap_err(),
            GeminiLiveProtocolError::InvalidJSON
        );
        assert_eq!(
            GeminiLiveServerEvent::decode(
                r#"{"serverContent":{"inputTranscription":{"text":false}}}"#
            )
            .unwrap_err(),
            GeminiLiveProtocolError::MissingEventField("serverContent.inputTranscription.text")
        );
        assert_eq!(
            GeminiLiveServerEvent::decode("{}").unwrap_err(),
            GeminiLiveProtocolError::MissingEventKind
        );
    }
}
