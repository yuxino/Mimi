//! Official DeepL text translation API. No proxy URL or credentials in diagnostics.

use super::deepl_languages::{wire_code, DEEPL_SOURCE_CODES, DEEPL_TARGET_CODES};
use crate::core::models::{SourceLanguage, TargetLanguage};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeepLError {
    #[error("Check the DeepL API key in Settings.")]
    InvalidKey,
    #[error("DeepL timed out. Check your network, then restart subtitles.")]
    Timeout,
    #[error("Could not connect to DeepL. Check your network, then restart subtitles.")]
    Connection,
    #[error("DeepL returned an invalid or empty translation.")]
    Response,
    #[error("DeepL returned too much data.")]
    TooLarge,
    #[error("DeepL rejected the request (code {0}). Check the API key and account quota.")]
    Rejected(u16),
}

impl DeepLError {
    pub fn retryable(&self) -> bool {
        use mimi_core::translation_policy::{classify_http, RetryClass};
        let class = match self {
            Self::Timeout | Self::Connection => RetryClass::Temporary,
            Self::Rejected(code) if *code <= 599 => classify_http(*code),
            _ => RetryClass::Permanent,
        };
        class != RetryClass::Permanent
    }

    pub fn authentication_failure(&self) -> bool {
        matches!(self, Self::InvalidKey | Self::Rejected(401 | 403))
    }

    pub fn diagnostic_label(&self) -> String {
        match self {
            Self::InvalidKey => "deepl.invalid_key".into(),
            Self::Timeout => "deepl.timeout".into(),
            Self::Connection => "deepl.connection".into(),
            Self::Response => "deepl.response".into(),
            Self::TooLarge => "deepl.too_large".into(),
            Self::Rejected(code) => format!("deepl.rejected(code={code})"),
        }
    }
}

/// Free API keys end in :fx. Both endpoints are fixed official HTTPS origins.
pub fn endpoint(api_key: &str) -> Result<&'static str, DeepLError> {
    let key = api_key.trim();
    if key.is_empty() || key.len() > 4096 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(DeepLError::InvalidKey);
    }
    Ok(if key.ends_with(":fx") {
        "https://api-free.deepl.com/v2/translate"
    } else {
        "https://api.deepl.com/v2/translate"
    })
}

pub fn source_code(source: SourceLanguage) -> Result<Option<String>, DeepLError> {
    if source == SourceLanguage::Automatic {
        return Ok(None);
    }
    if !DEEPL_SOURCE_CODES.contains(&source.raw_value()) {
        return Err(DeepLError::Response);
    }
    Ok(Some(wire_code(source.raw_value())))
}

pub fn target_code(target: TargetLanguage) -> Result<String, DeepLError> {
    if !DEEPL_TARGET_CODES.contains(&target.raw_value()) {
        return Err(DeepLError::Response);
    }
    Ok(wire_code(target.raw_value()))
}

/// A detected language only refines automatic mode when this adapter can encode
/// it. Otherwise preserve service-side detection; explicit choices still validate.
pub fn request_with_detected_source(
    text: &str,
    source: SourceLanguage,
    detected_source: Option<SourceLanguage>,
    target: TargetLanguage,
) -> Result<Value, DeepLError> {
    let source = if source == SourceLanguage::Automatic {
        detected_source
            .filter(|language| source_code(*language).is_ok())
            .unwrap_or(source)
    } else {
        source
    };
    request(text, source, target)
}

pub fn request(
    text: &str,
    source: SourceLanguage,
    target: TargetLanguage,
) -> Result<Value, DeepLError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(DeepLError::Response);
    }
    let target = target_code(target)?;
    let mut body = json!({ "text": [text], "target_lang": target });
    let source = source_code(source)?;
    if let Some(source) = source {
        body["source_lang"] = json!(source);
    }
    Ok(body)
}

/// One request carries one source; never accept a different number of results.
pub fn decode(bytes: &[u8]) -> Result<String, DeepLError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| DeepLError::Response)?;
    let translations = value
        .get("translations")
        .and_then(Value::as_array)
        .filter(|translations| translations.len() == 1)
        .ok_or(DeepLError::Response)?;
    let text = translations[0]
        .get("text")
        .and_then(Value::as_str)
        .ok_or(DeepLError::Response)?
        .trim();
    if text.is_empty() {
        return Err(DeepLError::Response);
    }
    if text.len() > 64 * 1024 {
        return Err(DeepLError::TooLarge);
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_documented_languages_reach_the_independent_text_route() {
        for source in SourceLanguage::ALL {
            assert_eq!(
                request("Synthetic.", source, TargetLanguage::English).is_ok(),
                source == SourceLanguage::Automatic
                    || DEEPL_SOURCE_CODES.contains(&source.raw_value())
            );
        }
        for target in TargetLanguage::ALL {
            assert_eq!(
                request("Synthetic.", SourceLanguage::Automatic, target).is_ok(),
                DEEPL_TARGET_CODES.contains(&target.raw_value())
            );
        }
    }

    #[test]
    fn keys_choose_only_official_free_or_pro_origins() {
        assert_eq!(
            endpoint(" synthetic:fx ").unwrap(),
            "https://api-free.deepl.com/v2/translate"
        );
        assert_eq!(
            endpoint("synthetic").unwrap(),
            "https://api.deepl.com/v2/translate"
        );
        for key in ["", " ", "a\r\nb", "a b", "秘密"] {
            assert_eq!(endpoint(key), Err(DeepLError::InvalidKey));
        }
        assert_eq!(endpoint(&"a".repeat(4097)), Err(DeepLError::InvalidKey));
    }

    #[test]
    fn sends_one_text_and_omits_automatic_source_language() {
        for (source, expected) in [
            (SourceLanguage::Automatic, None),
            (SourceLanguage::Chinese, Some("ZH")),
            (SourceLanguage::English, Some("EN")),
            (SourceLanguage::Japanese, Some("JA")),
            (SourceLanguage::Korean, Some("KO")),
        ] {
            for (target, code) in [
                (TargetLanguage::SimplifiedChinese, "ZH"),
                (TargetLanguage::English, "EN"),
                (TargetLanguage::Japanese, "JA"),
            ] {
                let body = request(" synthetic ", source, target).unwrap();
                assert_eq!(body["text"], json!(["synthetic"]));
                assert_eq!(body["target_lang"], code);
                assert_eq!(body.get("source_lang").and_then(Value::as_str), expected);
                assert_eq!(
                    body.as_object().unwrap().len(),
                    2 + usize::from(expected.is_some())
                );
            }
        }
        assert_eq!(
            request(" ", SourceLanguage::English, TargetLanguage::Japanese),
            Err(DeepLError::Response)
        );
        assert_eq!(
            request(
                "synthetic",
                SourceLanguage::English,
                TargetLanguage::Original
            ),
            Err(DeepLError::Response)
        );
    }

    #[test]
    fn accepts_exactly_one_nonempty_translation_without_error_content() {
        assert_eq!(
            decode(
                r#"{"translations":[{"detected_source_language":"EN","text":" 合成字幕 "}]}"#
                    .as_bytes()
            )
            .unwrap(),
            "合成字幕"
        );
        for body in [
            r#"{"translations":[]}"#,
            r#"{"translations":[{"text":"one"},{"text":"two"}]}"#,
            r#"{"translations":[{"text":" "}]}"#,
            r#"{"translations":[{"text":123}]}"#,
            r#"{"translations":[{}]}"#,
            r#"{"message":"private text token"}"#,
            "invalid",
        ] {
            let error = decode(body.as_bytes()).unwrap_err();
            assert_eq!(error, DeepLError::Response);
            assert!(!error.to_string().contains("private"));
        }
        let body = json!({"translations":[{"text":"a".repeat(64 * 1024 + 1)}]});
        assert_eq!(
            decode(&serde_json::to_vec(&body).unwrap()),
            Err(DeepLError::TooLarge)
        );
    }

    #[test]
    fn classifies_authentication_quota_and_transient_statuses() {
        for code in [401, 403] {
            assert!(DeepLError::Rejected(code).authentication_failure());
            assert!(!DeepLError::Rejected(code).retryable());
        }
        assert!(!DeepLError::Rejected(456).authentication_failure());
        assert!(!DeepLError::Rejected(456).retryable());
        for code in [408, 429, 500, 503, 599] {
            assert!(DeepLError::Rejected(code).retryable());
        }
        assert_eq!(
            DeepLError::Rejected(456).diagnostic_label(),
            "deepl.rejected(code=456)"
        );
    }
}
