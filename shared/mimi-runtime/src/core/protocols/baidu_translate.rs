//! Baidu Cloud realtime speech-translation wire protocol.

use crate::core::models::{SourceLanguage, TargetLanguage};
use serde_json::{json, Value};
use thiserror::Error;

const MAXIMUM_SERVER_MESSAGE_BYTES: usize = 256 * 1_024;
const MAXIMUM_TRANSCRIPT_BYTES: usize = 128 * 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BaiduTranslateProtocolError {
    #[error("Enter a Baidu Cloud AppID.")]
    MissingAppID,
    #[error("Enter a Baidu Cloud AppKey.")]
    MissingAppKey,
    #[error("Baidu realtime translation requires an explicit supported source language.")]
    InvalidSourceLanguage,
    #[error("Baidu realtime translation requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("The Baidu realtime translation endpoint could not be created.")]
    InvalidEndpoint,
    #[error(
        "Baidu realtime translation expected {expected_bytes} audio bytes, got {actual_bytes}."
    )]
    InvalidAudioFrame {
        expected_bytes: usize,
        actual_bytes: usize,
    },
    #[error("Baidu realtime translation returned an oversized response.")]
    ResponseTooLarge,
    #[error("Baidu realtime translation returned invalid JSON.")]
    InvalidJSON,
    #[error("Baidu realtime translation returned an event without {0}.")]
    MissingEventField(&'static str),
}

pub struct BaiduTranslateEndpoint;

impl BaiduTranslateEndpoint {
    pub const SAMPLE_RATE_HZ: u32 = 16_000;
    pub const CHANNEL_COUNT: u16 = 1;
    pub const BITS_PER_SAMPLE: u16 = 16;
    pub const FRAME_DURATION_MS: u32 = 40;
    pub const AUDIO_FRAME_BYTE_COUNT: usize = Self::SAMPLE_RATE_HZ as usize
        * Self::CHANNEL_COUNT as usize
        * (Self::BITS_PER_SAMPLE as usize / 8)
        * Self::FRAME_DURATION_MS as usize
        / 1_000;

    pub fn url() -> Result<url::Url, BaiduTranslateProtocolError> {
        url::Url::parse("wss://aip.baidubce.com/ws/realtime_speech_trans")
            .map_err(|_| BaiduTranslateProtocolError::InvalidEndpoint)
    }
}

pub struct BaiduTranslateRequestEncoder;

impl BaiduTranslateRequestEncoder {
    pub fn start(
        app_id: &str,
        app_key: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
    ) -> Result<Value, BaiduTranslateProtocolError> {
        let app_id = app_id.trim();
        let app_key = app_key.trim();
        if app_id.is_empty() {
            return Err(BaiduTranslateProtocolError::MissingAppID);
        }
        if app_key.is_empty() {
            return Err(BaiduTranslateProtocolError::MissingAppKey);
        }
        if app_id.len() > 512 || app_key.len() > 512 {
            return Err(BaiduTranslateProtocolError::MissingAppKey);
        }
        Ok(json!({
            "type": "START",
            "from": source_language_code(source_language)?,
            "to": target_language_code(target_language)?,
            "app_id": app_id,
            "app_key": app_key,
            "sampling_rate": BaiduTranslateEndpoint::SAMPLE_RATE_HZ
        }))
    }

    pub fn validate_audio_frame(pcm_data: &[u8]) -> Result<(), BaiduTranslateProtocolError> {
        if pcm_data.len() != BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT {
            return Err(BaiduTranslateProtocolError::InvalidAudioFrame {
                expected_bytes: BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT,
                actual_bytes: pcm_data.len(),
            });
        }
        Ok(())
    }

    pub fn finish() -> Value {
        json!({ "type": "FINISH" })
    }
}

// Exact realtime speech-translation catalog, not Baidu's separate text API.
pub const LANGUAGE_CODES: &[(&str, &str)] = &[
    ("zh", "zh"),
    ("en", "en"),
    ("yue", "yue"),
    ("ja", "jp"),
    ("ko", "kor"),
    ("fr", "fra"),
    ("es", "spa"),
    ("th", "th"),
    ("ar", "ara"),
    ("ru", "ru"),
    ("pt", "pt"),
    ("de", "de"),
    ("it", "it"),
    ("el", "el"),
    ("nl", "nl"),
    ("pl", "pl"),
    ("bg", "bul"),
    ("da", "dan"),
    ("fi", "fin"),
    ("cs", "cs"),
    ("ro", "rom"),
    ("sv", "swe"),
    ("hu", "hu"),
    ("vi", "vie"),
    ("id", "id"),
    ("ca", "cat"),
    ("he", "heb"),
    ("hi", "hi"),
    ("ms", "may"),
    ("no", "nor"),
    ("is", "ice"),
    ("tl", "fil"),
    ("km", "hkm"),
    ("hr", "hrv"),
    ("lv", "lav"),
    ("bn", "ben"),
    ("ne", "nep"),
    ("af", "afr"),
    ("sk", "sk"),
    ("si", "sin"),
    ("sr", "srp"),
    ("sw", "swa"),
    ("tr", "tr"),
    ("uk", "ukr"),
    ("hy", "arm"),
];

fn language_code(code: &str) -> Option<&'static str> {
    LANGUAGE_CODES
        .iter()
        .find_map(|(app, wire)| (*app == code).then_some(*wire))
}

fn source_language_code(
    source_language: SourceLanguage,
) -> Result<&'static str, BaiduTranslateProtocolError> {
    language_code(source_language.raw_value())
        .ok_or(BaiduTranslateProtocolError::InvalidSourceLanguage)
}

fn target_language_code(
    target_language: TargetLanguage,
) -> Result<&'static str, BaiduTranslateProtocolError> {
    language_code(target_language.raw_value())
        .ok_or(BaiduTranslateProtocolError::InvalidTargetLanguage)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaiduTranslateServerEvent {
    SessionReady,
    Transcript {
        source_text: String,
        target_text: String,
        sentence_end: bool,
    },
    SessionFinished,
    ProviderError {
        code: i64,
        is_recoverable: bool,
    },
    Ignored {
        kind: String,
    },
}

impl BaiduTranslateServerEvent {
    pub fn decode(text: &str) -> Result<Self, BaiduTranslateProtocolError> {
        if text.len() > MAXIMUM_SERVER_MESSAGE_BYTES {
            return Err(BaiduTranslateProtocolError::ResponseTooLarge);
        }
        let value: Value =
            serde_json::from_str(text).map_err(|_| BaiduTranslateProtocolError::InvalidJSON)?;
        Self::decode_value(&value)
    }

    pub fn decode_value(value: &Value) -> Result<Self, BaiduTranslateProtocolError> {
        let code = value
            .get("code")
            .and_then(Value::as_i64)
            .ok_or(BaiduTranslateProtocolError::MissingEventField("code"))?;
        if code != 0 {
            return Ok(Self::ProviderError {
                code,
                is_recoverable: matches!(code, 20_311 | 20_312 | 20_313 | 20_315 | 20_316),
            });
        }

        let data = value
            .get("data")
            .ok_or(BaiduTranslateProtocolError::MissingEventField("data"))?;
        let status = data.get("status").and_then(Value::as_str).ok_or(
            BaiduTranslateProtocolError::MissingEventField("data.status"),
        )?;
        match status {
            "STA" => Ok(Self::SessionReady),
            "END" => Ok(Self::SessionFinished),
            "TRN" => decode_translation(data),
            other => Ok(Self::Ignored {
                kind: sanitize_label(other, 32, "provider_event"),
            }),
        }
    }
}

fn decode_translation(
    data: &Value,
) -> Result<BaiduTranslateServerEvent, BaiduTranslateProtocolError> {
    let result = data
        .get("result")
        .ok_or(BaiduTranslateProtocolError::MissingEventField(
            "data.result",
        ))?;
    let result_type = result.get("type").and_then(Value::as_str).ok_or(
        BaiduTranslateProtocolError::MissingEventField("data.result.type"),
    )?;
    match result_type {
        "MID" => Ok(BaiduTranslateServerEvent::Transcript {
            source_text: bounded_string(result, "asr")?,
            target_text: bounded_string(result, "asr_trans")?,
            sentence_end: false,
        }),
        "FIN" => Ok(BaiduTranslateServerEvent::Transcript {
            source_text: bounded_string(result, "sentence")?,
            target_text: bounded_string(result, "sentence_trans")?,
            sentence_end: true,
        }),
        other => Ok(BaiduTranslateServerEvent::Ignored {
            kind: sanitize_label(other, 32, "translation_result"),
        }),
    }
}

fn bounded_string(
    value: &Value,
    field: &'static str,
) -> Result<String, BaiduTranslateProtocolError> {
    let text = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or(BaiduTranslateProtocolError::MissingEventField(field))?;
    if text.len() > MAXIMUM_TRANSCRIPT_BYTES {
        return Err(BaiduTranslateProtocolError::ResponseTooLarge);
    }
    Ok(text.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_catalog_maps_every_supported_language_and_rejects_other_app_languages() {
        let contract: Value =
            serde_json::from_str(include_str!("../../../../translation-contracts.json")).unwrap();
        let codes = contract["providerLanguageCatalogs"]["baidu"]["wireByCode"]
            .as_object()
            .unwrap();
        assert_eq!(codes.len(), LANGUAGE_CODES.len());
        for (code, wire) in codes {
            let source: SourceLanguage = serde_json::from_value(json!(code)).unwrap();
            let target: TargetLanguage = serde_json::from_value(json!(code)).unwrap();
            assert_eq!(
                source_language_code(source).unwrap(),
                wire.as_str().unwrap()
            );
            assert_eq!(
                target_language_code(target).unwrap(),
                wire.as_str().unwrap()
            );
        }
        for source in SourceLanguage::ALL {
            assert_eq!(
                source_language_code(source).is_ok(),
                codes.contains_key(source.raw_value())
            );
        }
        for target in TargetLanguage::ALL {
            assert_eq!(
                target_language_code(target).is_ok(),
                codes.contains_key(target.raw_value())
            );
        }
    }

    #[test]
    fn endpoint_start_and_audio_contract_match_the_official_protocol() {
        assert_eq!(
            BaiduTranslateEndpoint::url().unwrap().as_str(),
            "wss://aip.baidubce.com/ws/realtime_speech_trans"
        );
        let start = BaiduTranslateRequestEncoder::start(
            "app-id",
            "app-key",
            SourceLanguage::Japanese,
            TargetLanguage::SimplifiedChinese,
        )
        .unwrap();
        assert_eq!(start["type"], "START");
        assert_eq!(start["from"], "jp");
        assert_eq!(start["to"], "zh");
        assert_eq!(start["sampling_rate"], 16_000);
        assert_eq!(BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT, 1_280);
        assert!(BaiduTranslateRequestEncoder::validate_audio_frame(&vec![0; 1_280]).is_ok());
        assert_eq!(
            BaiduTranslateRequestEncoder::finish(),
            json!({ "type": "FINISH" })
        );
    }

    #[test]
    fn automatic_source_is_rejected_because_baidu_does_not_document_it() {
        assert_eq!(
            BaiduTranslateRequestEncoder::start(
                "app-id",
                "app-key",
                SourceLanguage::Automatic,
                TargetLanguage::English,
            )
            .unwrap_err(),
            BaiduTranslateProtocolError::InvalidSourceLanguage
        );
    }

    #[test]
    fn decodes_start_draft_final_and_end_events() {
        assert_eq!(
            BaiduTranslateServerEvent::decode(
                r#"{"code":0,"msg":"Success","data":{"status":"STA"}}"#
            )
            .unwrap(),
            BaiduTranslateServerEvent::SessionReady
        );
        assert_eq!(
            BaiduTranslateServerEvent::decode(
                r#"{"code":0,"data":{"status":"TRN","result":{"type":"MID","asr":"今天","asr_trans":"Today","sentence":"","sentence_trans":""}}}"#
            )
            .unwrap(),
            BaiduTranslateServerEvent::Transcript {
                source_text: "今天".into(),
                target_text: "Today".into(),
                sentence_end: false,
            }
        );
        assert_eq!(
            BaiduTranslateServerEvent::decode(
                r#"{"code":0,"data":{"status":"TRN","result":{"type":"FIN","asr":"","asr_trans":"","sentence":"今天天气不错。","sentence_trans":"The weather is nice today."}}}"#
            )
            .unwrap(),
            BaiduTranslateServerEvent::Transcript {
                source_text: "今天天气不错。".into(),
                target_text: "The weather is nice today.".into(),
                sentence_end: true,
            }
        );
        assert_eq!(
            BaiduTranslateServerEvent::decode(r#"{"code":0,"data":{"status":"END"}}"#).unwrap(),
            BaiduTranslateServerEvent::SessionFinished
        );
    }

    #[test]
    fn provider_error_messages_are_discarded_and_recoverability_is_explicit() {
        assert_eq!(
            BaiduTranslateServerEvent::decode(
                r#"{"code":20312,"msg":"private translated content"}"#
            )
            .unwrap(),
            BaiduTranslateServerEvent::ProviderError {
                code: 20_312,
                is_recoverable: true,
            }
        );
        assert_eq!(
            BaiduTranslateServerEvent::decode(
                r#"{"code":31003,"msg":"private credential detail"}"#
            )
            .unwrap(),
            BaiduTranslateServerEvent::ProviderError {
                code: 31_003,
                is_recoverable: false,
            }
        );
    }
}
