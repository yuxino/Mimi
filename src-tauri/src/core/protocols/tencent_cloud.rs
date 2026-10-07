//! Tencent Cloud realtime speech-translation wire protocol.
//!
//! The provider authenticates the WebSocket URL with a lexicographically
//! sorted query signed by HMAC-SHA1. Secret material is deliberately kept out
//! of every error and public debug representation in this module.

use crate::core::models::{SourceLanguage, TargetLanguage};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use thiserror::Error;

const HOST_AND_PATH_PREFIX: &str = "asr.cloud.tencent.com/asr/speech_translate/";
const MAXIMUM_SERVER_MESSAGE_BYTES: usize = 256 * 1_024;
const MAXIMUM_TRANSCRIPT_BYTES: usize = 128 * 1_024;
const MAXIMUM_SIGNATURE_LIFETIME_SECONDS: u64 = 90 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TencentCloudProtocolError {
    #[error("Enter a Tencent Cloud AppID.")]
    MissingAppID,
    #[error("Enter a Tencent Cloud SecretID.")]
    MissingSecretID,
    #[error("Enter a Tencent Cloud SecretKey.")]
    MissingSecretKey,
    #[error("Tencent Cloud realtime translation requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("Tencent Cloud realtime translation does not support this source language.")]
    InvalidSourceLanguage,
    #[error("The Tencent Cloud realtime translation session identity is invalid.")]
    InvalidSessionIdentity,
    #[error("The Tencent Cloud realtime translation signature lifetime is invalid.")]
    InvalidSignatureLifetime,
    #[error("The Tencent Cloud realtime translation endpoint could not be created.")]
    InvalidEndpoint,
    #[error(
        "Tencent Cloud realtime translation expected {expected_bytes} audio bytes, got {actual_bytes}."
    )]
    InvalidAudioFrame {
        expected_bytes: usize,
        actual_bytes: usize,
    },
    #[error("Tencent Cloud realtime translation returned an oversized response.")]
    ResponseTooLarge,
    #[error("Tencent Cloud realtime translation returned invalid JSON.")]
    InvalidJSON,
    #[error("Tencent Cloud realtime translation returned an event without {0}.")]
    MissingEventField(&'static str),
}

/// A signed, single-use Tencent Cloud endpoint.
///
/// Do not add `Debug`: the URL contains a SecretID and a short-lived
/// signature. Callers should construct a fresh endpoint for every connection.
pub struct TencentCloudEndpoint {
    url: url::Url,
}

impl TencentCloudEndpoint {
    pub const TRANSLATION_MODEL: &'static str = "hunyuan-translation-lite";
    pub const SAMPLE_RATE_HZ: u32 = 16_000;
    pub const CHANNEL_COUNT: u16 = 1;
    pub const BITS_PER_SAMPLE: u16 = 16;
    pub const FRAME_DURATION_MS: u32 = 200;
    pub const AUDIO_FRAME_BYTE_COUNT: usize = Self::SAMPLE_RATE_HZ as usize
        * Self::CHANNEL_COUNT as usize
        * (Self::BITS_PER_SAMPLE as usize / 8)
        * Self::FRAME_DURATION_MS as usize
        / 1_000;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        app_id: &str,
        secret_id: &str,
        secret_key: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        timestamp: u64,
        expired: u64,
        nonce: u64,
        voice_id: &str,
    ) -> Result<Self, TencentCloudProtocolError> {
        let app_id = app_id.trim();
        let secret_id = secret_id.trim();
        let secret_key = secret_key.trim();
        let voice_id = voice_id.trim();

        if app_id.is_empty() {
            return Err(TencentCloudProtocolError::MissingAppID);
        }
        if !app_id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(TencentCloudProtocolError::InvalidSessionIdentity);
        }
        if secret_id.is_empty() {
            return Err(TencentCloudProtocolError::MissingSecretID);
        }
        if !is_unreserved(secret_id) {
            return Err(TencentCloudProtocolError::InvalidSessionIdentity);
        }
        if secret_key.is_empty() {
            return Err(TencentCloudProtocolError::MissingSecretKey);
        }
        if voice_id.is_empty() || voice_id.len() > 128 || !is_unreserved(voice_id) {
            return Err(TencentCloudProtocolError::InvalidSessionIdentity);
        }
        if nonce == 0 || nonce > 9_999_999_999 {
            return Err(TencentCloudProtocolError::InvalidSessionIdentity);
        }
        if expired <= timestamp
            || expired.saturating_sub(timestamp) >= MAXIMUM_SIGNATURE_LIFETIME_SECONDS
        {
            return Err(TencentCloudProtocolError::InvalidSignatureLifetime);
        }

        let source = source_language_code(source_language)?;
        let target = target_language_code(target_language)?;
        if !supported_target_codes(source).contains(&target) {
            return Err(TencentCloudProtocolError::InvalidTargetLanguage);
        }
        let parameters = canonical_parameters(
            secret_id, source, target, timestamp, expired, nonce, voice_id,
        );
        let canonical_query = join_query(&parameters);
        let canonical_request = format!("{HOST_AND_PATH_PREFIX}{app_id}?{canonical_query}");
        let signature = base64::engine::general_purpose::STANDARD.encode(hmac_sha1(
            secret_key.as_bytes(),
            canonical_request.as_bytes(),
        ));
        let encoded_signature = percent_encode(&signature);
        let raw_url = format!("wss://{canonical_request}&signature={encoded_signature}");
        let url =
            url::Url::parse(&raw_url).map_err(|_| TencentCloudProtocolError::InvalidEndpoint)?;
        Ok(Self { url })
    }

    pub fn url(&self) -> &url::Url {
        &self.url
    }
}

fn canonical_parameters(
    secret_id: &str,
    source: &str,
    target: &str,
    timestamp: u64,
    expired: u64,
    nonce: u64,
    voice_id: &str,
) -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        ("expired", expired.to_string()),
        ("nonce", nonce.to_string()),
        ("secretid", secret_id.to_string()),
        ("source", source.to_string()),
        ("target", target.to_string()),
        ("timestamp", timestamp.to_string()),
        (
            "trans_model",
            TencentCloudEndpoint::TRANSLATION_MODEL.to_string(),
        ),
        ("voice_format", "1".to_string()),
        ("voice_id", voice_id.to_string()),
    ])
}

fn join_query(parameters: &BTreeMap<&str, String>) -> String {
    parameters
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn is_unreserved(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

/// Exact source-dependent directions documented for hunyuan-translation-lite.
pub fn supported_target_codes(source: &str) -> &'static [&'static str] {
    match source {
        "zh" => &["zh", "en", "ja", "ko", "yue", "id", "th"],
        "en" => &["zh", "en", "ja", "ko", "yue", "id", "th"],
        "zh_en" => &["zh_en", "zh", "en", "ja", "ko", "yue", "id", "th"],
        "ja" => &["zh", "en", "ja", "ko", "yue"],
        "ko" => &["zh", "en", "ja", "ko", "yue"],
        "yue" => &["zh", "en", "ja", "ko", "yue"],
        "id" => &["zh", "en", "id"],
        "th" => &["zh", "en", "th"],
        "ru" => &["zh", "en", "ru"],
        _ => &[],
    }
}

fn source_language_code(
    source_language: SourceLanguage,
) -> Result<&'static str, TencentCloudProtocolError> {
    // Compatibility for saved configurations predating the explicit mixed-language choice.
    let code = if source_language == SourceLanguage::Automatic {
        "zh_en"
    } else {
        source_language.raw_value()
    };
    if supported_target_codes(code).is_empty() {
        Err(TencentCloudProtocolError::InvalidSourceLanguage)
    } else {
        Ok(code)
    }
}

fn target_language_code(
    target_language: TargetLanguage,
) -> Result<&'static str, TencentCloudProtocolError> {
    let code = target_language.raw_value();
    if supported_target_codes(code).is_empty() {
        Err(TencentCloudProtocolError::InvalidTargetLanguage)
    } else {
        Ok(code)
    }
}

pub struct TencentCloudRequestEncoder;

impl TencentCloudRequestEncoder {
    pub fn validate_audio_frame(pcm_data: &[u8]) -> Result<(), TencentCloudProtocolError> {
        if pcm_data.len() != TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT {
            return Err(TencentCloudProtocolError::InvalidAudioFrame {
                expected_bytes: TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT,
                actual_bytes: pcm_data.len(),
            });
        }
        Ok(())
    }

    pub fn finish() -> Value {
        json!({ "type": "end" })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TencentCloudServerEvent {
    SessionReady,
    Transcript {
        source_text: String,
        target_text: String,
        source_language: String,
        sentence_end: bool,
    },
    SessionFinished,
    ProviderError {
        code: i64,
    },
    Ignored {
        kind: String,
    },
}

impl TencentCloudServerEvent {
    pub fn decode(text: &str) -> Result<Self, TencentCloudProtocolError> {
        if text.len() > MAXIMUM_SERVER_MESSAGE_BYTES {
            return Err(TencentCloudProtocolError::ResponseTooLarge);
        }
        let value: Value =
            serde_json::from_str(text).map_err(|_| TencentCloudProtocolError::InvalidJSON)?;
        Self::decode_value(&value)
    }

    pub fn decode_value(value: &Value) -> Result<Self, TencentCloudProtocolError> {
        let code = value
            .get("code")
            .and_then(Value::as_i64)
            .ok_or(TencentCloudProtocolError::MissingEventField("code"))?;
        if code != 0 {
            return Ok(Self::ProviderError { code });
        }

        let final_value = value.get("final").and_then(Value::as_u64);
        if final_value == Some(1) {
            return Ok(Self::SessionFinished);
        }

        if let Some(result) = value.get("result") {
            let source_text = bounded_string(result, "source_text")?;
            let target_text = bounded_string(result, "target_text")?;
            let source_language = bounded_label(result, "source", 16)?;
            let _target_language = bounded_label(result, "target", 16)?;
            let sentence_end = result.get("sentence_end").and_then(Value::as_bool).ok_or(
                TencentCloudProtocolError::MissingEventField("result.sentence_end"),
            )?;
            return Ok(Self::Transcript {
                source_text,
                target_text,
                source_language,
                sentence_end,
            });
        }

        match final_value {
            // A successful handshake includes final: 0 before any audio is sent.
            None | Some(0) => Ok(Self::SessionReady),
            Some(value) => Ok(Self::Ignored {
                kind: format!("final_{value}"),
            }),
        }
    }
}

fn bounded_string(value: &Value, field: &'static str) -> Result<String, TencentCloudProtocolError> {
    let text = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or(TencentCloudProtocolError::MissingEventField(field))?;
    if text.len() > MAXIMUM_TRANSCRIPT_BYTES {
        return Err(TencentCloudProtocolError::ResponseTooLarge);
    }
    Ok(text.to_string())
}

fn bounded_label(
    value: &Value,
    field: &'static str,
    maximum_length: usize,
) -> Result<String, TencentCloudProtocolError> {
    let label = value
        .get(field)
        .and_then(Value::as_str)
        .filter(|label| {
            !label.is_empty()
                && label.len() <= maximum_length
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
        .ok_or(TencentCloudProtocolError::MissingEventField(field))?;
    Ok(label.to_string())
}

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    const BLOCK_BYTES: usize = 64;
    let mut normalized_key = [0_u8; BLOCK_BYTES];
    if key.len() > BLOCK_BYTES {
        normalized_key[..20].copy_from_slice(&sha1(key));
    } else {
        normalized_key[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0x36_u8; BLOCK_BYTES];
    let mut outer_pad = [0x5c_u8; BLOCK_BYTES];
    for index in 0..BLOCK_BYTES {
        inner_pad[index] ^= normalized_key[index];
        outer_pad[index] ^= normalized_key[index];
    }

    let mut inner = Vec::with_capacity(BLOCK_BYTES + message.len());
    inner.extend_from_slice(&inner_pad);
    inner.extend_from_slice(message);
    let inner_digest = sha1(&inner);

    let mut outer = Vec::with_capacity(BLOCK_BYTES + inner_digest.len());
    outer.extend_from_slice(&outer_pad);
    outer.extend_from_slice(&inner_digest);
    sha1(&outer)
}

fn sha1(input: &[u8]) -> [u8; 20] {
    let bit_length = (input.len() as u64).wrapping_mul(8);
    let mut message = Vec::with_capacity((input.len() + 72) & !63);
    message.extend_from_slice(input);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());

    let mut h0 = 0x6745_2301_u32;
    let mut h1 = 0xefcd_ab89_u32;
    let mut h2 = 0x98ba_dcfe_u32;
    let mut h3 = 0x1032_5476_u32;
    let mut h4 = 0xc3d2_e1f0_u32;

    for chunk in message.as_chunks::<64>().0 {
        let mut words = [0_u32; 80];
        for (index, word) in words[..16].iter_mut().enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes(chunk[offset..offset + 4].try_into().unwrap());
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;
        for (index, word) in words.iter().enumerate() {
            let (function, constant) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(function)
                .wrapping_add(e)
                .wrapping_add(constant)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut digest = [0_u8; 20];
    for (index, word) in [h0, h1, h2, h3, h4].iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_catalog_validates_every_pair_before_signing() {
        let contract: Value = serde_json::from_str(include_str!(
            "../../../../shared/translation-contracts.json"
        ))
        .unwrap();
        let pairs = contract["providerLanguageCatalogs"]["tencent"]["pairs"]
            .as_object()
            .unwrap();
        for (source, targets) in pairs {
            let expected: Vec<&str> = targets
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(supported_target_codes(source), expected);
            for target in TargetLanguage::ALL {
                let from = serde_json::from_value(json!(source)).unwrap();
                let endpoint = TencentCloudEndpoint::new(
                    "1250000000",
                    "test-id",
                    "test-key",
                    from,
                    target,
                    100,
                    3700,
                    1,
                    "test-voice",
                );
                assert_eq!(
                    endpoint.is_ok(),
                    expected.contains(&target.raw_value()),
                    "{source} -> {}",
                    target.raw_value()
                );
            }
        }
        assert_eq!(
            source_language_code(SourceLanguage::Automatic).unwrap(),
            "zh_en"
        );
        for source in SourceLanguage::ALL {
            assert_eq!(
                source_language_code(source).is_ok(),
                source == SourceLanguage::Automatic || pairs.contains_key(source.raw_value())
            );
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn sha1_and_hmac_sha1_match_standard_vectors() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&hmac_sha1(&[0x0b; 20], b"Hi There")),
            "b617318655057264e28bc0b6fb378c8ef146be00"
        );
    }

    #[test]
    fn signed_endpoint_uses_the_official_sorted_query_contract() {
        for case in shared_contract()["signing"].as_array().unwrap() {
            let credentials = &case["credentials"];
            let endpoint = TencentCloudEndpoint::new(
                credentials["appId"].as_str().unwrap(),
                credentials["secretId"].as_str().unwrap(),
                credentials["secretKey"].as_str().unwrap(),
                serde_json::from_value(case["source"].clone()).unwrap(),
                serde_json::from_value(case["target"].clone()).unwrap(),
                case["timestamp"].as_u64().unwrap(),
                case["expired"].as_u64().unwrap(),
                case["nonce"].as_u64().unwrap(),
                case["voiceId"].as_str().unwrap(),
            )
            .unwrap();
            assert_eq!(
                endpoint.url().as_str(),
                case["expected"]["url"].as_str().unwrap()
            );
            assert_eq!(
                TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT as u64,
                case["expected"]["frameBytes"].as_u64().unwrap()
            );
        }
    }

    fn shared_contract() -> Value {
        let contract: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../shared/translation-contracts.json"
        )))
        .unwrap();
        contract["tencentSpeechTranslation"].clone()
    }

    #[test]
    fn shared_response_contract_rejects_malformed_fields_and_keeps_final_pairs() {
        for case in shared_contract()["events"].as_array().unwrap() {
            let actual = TencentCloudServerEvent::decode(case["frame"].as_str().unwrap());
            let expected = &case["expected"];
            if expected.is_null() {
                assert!(actual.is_err(), "{}", case["id"]);
                continue;
            }
            match (actual.unwrap(), expected["type"].as_str().unwrap()) {
                (TencentCloudServerEvent::SessionReady, "ready")
                | (TencentCloudServerEvent::SessionFinished, "finished")
                | (TencentCloudServerEvent::Ignored { .. }, "ignored") => {}
                (
                    TencentCloudServerEvent::Transcript {
                        source_text,
                        target_text,
                        source_language,
                        sentence_end,
                    },
                    kind,
                ) => {
                    if kind == "ignored" {
                        assert!(
                            sentence_end
                                && (source_text.trim().is_empty() || target_text.trim().is_empty())
                        );
                    } else {
                        assert_eq!(sentence_end, kind == "finalPair");
                        assert_eq!(source_text.trim(), expected["source"].as_str().unwrap());
                        assert_eq!(
                            target_text.trim(),
                            expected["translation"].as_str().unwrap()
                        );
                        assert_eq!(source_language, expected["language"].as_str().unwrap());
                    }
                }
                (event, kind) => panic!("unexpected Tencent fixture event {event:?} for {kind}"),
            }
        }
    }

    #[test]
    fn endpoint_and_audio_contract_are_fixed() {
        assert_eq!(TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT, 6_400);
        assert!(TencentCloudRequestEncoder::validate_audio_frame(&vec![0; 6_400]).is_ok());
        assert!(matches!(
            TencentCloudRequestEncoder::validate_audio_frame(&[0; 3]),
            Err(TencentCloudProtocolError::InvalidAudioFrame { .. })
        ));
        assert_eq!(
            TencentCloudRequestEncoder::finish(),
            json!({ "type": "end" })
        );
    }

    #[test]
    fn decodes_full_replacement_drafts_and_sentence_finals() {
        let draft = TencentCloudServerEvent::decode(
            r#"{"code":0,"message":"success","voice_id":"v","sentence_id":"s","result":{"source":"zh","target":"ja","source_text":"你好","target_text":"こんにちは","start_time":0,"end_time":800,"sentence_end":false},"final":0}"#,
        )
        .unwrap();
        assert_eq!(
            draft,
            TencentCloudServerEvent::Transcript {
                source_text: "你好".into(),
                target_text: "こんにちは".into(),
                source_language: "zh".into(),
                sentence_end: false,
            }
        );

        let final_sentence = TencentCloudServerEvent::decode(
            r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"你好。","target_text":"こんにちは。","sentence_end":true}}"#,
        )
        .unwrap();
        assert!(matches!(
            final_sentence,
            TencentCloudServerEvent::Transcript {
                sentence_end: true,
                ..
            }
        ));
        assert_eq!(
            TencentCloudServerEvent::decode(r#"{"code":0,"final":1}"#).unwrap(),
            TencentCloudServerEvent::SessionFinished
        );
    }

    #[test]
    fn provider_messages_and_oversized_text_never_escape() {
        assert_eq!(
            TencentCloudServerEvent::decode(r#"{"code":6008,"message":"private provider detail"}"#)
                .unwrap(),
            TencentCloudServerEvent::ProviderError { code: 6008 }
        );
        let oversized = "x".repeat(MAXIMUM_TRANSCRIPT_BYTES + 1);
        let value = json!({
            "code": 0,
            "result": {
                "source": "zh",
                "target": "en",
                "source_text": oversized,
                "target_text": "ok",
                "sentence_end": false
            }
        });
        assert_eq!(
            TencentCloudServerEvent::decode_value(&value).unwrap_err(),
            TencentCloudProtocolError::ResponseTooLarge
        );
    }
}
