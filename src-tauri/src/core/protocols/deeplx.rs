//! DeepLX-compatible text-only JSON API; not the official DeepL API.
use super::deepl_languages::{wire_code, DEEPLX_SOURCE_CODES, DEEPLX_TARGET_CODES};
use crate::core::models::{SourceLanguage, TargetLanguage};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeepLXError {
    #[error("Check the DeepLX endpoint in Settings. Use HTTPS, or HTTP on localhost, without URL credentials, query or fragment.")]
    Endpoint,
    #[error("DeepLX timed out. Check the endpoint, network and server, then restart subtitles.")]
    Timeout,
    #[error("Could not connect to DeepLX. Check the endpoint, network and server, then restart subtitles.")]
    Connection,
    #[error("DeepLX returned an invalid or empty translation. Check that the endpoint supports the DeepLX /translate JSON API.")]
    Response,
    #[error("DeepLX returned too much data. Check the server's /translate response.")]
    TooLarge,
    #[error("DeepLX rejected the request (code {0}). Check the endpoint and optional Bearer token with your server administrator.")]
    Rejected(u16),
}
impl DeepLXError {
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
        matches!(self, Self::Rejected(401 | 403))
    }
    pub fn diagnostic_label(&self) -> String {
        match self {
            Self::Rejected(code) => format!("deeplx.rejected(code={code})"),
            Self::Endpoint => "deeplx.endpoint".into(),
            Self::Timeout => "deeplx.timeout".into(),
            Self::Connection => "deeplx.connection".into(),
            Self::Response => "deeplx.response".into(),
            Self::TooLarge => "deeplx.too_large".into(),
        }
    }
}

/// Preserve a reverse-proxy prefix or an explicit API path. Append /translate
/// to base paths only; never join against the origin and discard a prefix.
pub fn endpoint(value: &str) -> Result<url::Url, DeepLXError> {
    if value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(DeepLXError::Endpoint);
    }
    let mut url = url::Url::parse(value.trim()).map_err(|_| DeepLXError::Endpoint)?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && local)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(DeepLXError::Endpoint);
    }
    let path = url.path().trim_end_matches('/').to_string();
    if !path.ends_with("/translate") {
        url.set_path(&format!("{path}/translate"));
    } else {
        url.set_path(&path);
    }
    Ok(url)
}

pub fn source_code(source: SourceLanguage) -> Result<String, DeepLXError> {
    if source == SourceLanguage::Automatic {
        return Ok("auto".into());
    }
    if !DEEPLX_SOURCE_CODES.contains(&source.raw_value()) {
        return Err(DeepLXError::Response);
    }
    Ok(wire_code(source.raw_value()))
}

pub fn target_code(target: TargetLanguage) -> Result<String, DeepLXError> {
    if !DEEPLX_TARGET_CODES.contains(&target.raw_value()) {
        return Err(DeepLXError::Response);
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
) -> Result<Value, DeepLXError> {
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
) -> Result<Value, DeepLXError> {
    let source = source_code(source)?;
    let target = target_code(target)?;
    if text.trim().is_empty() {
        return Err(DeepLXError::Response);
    }
    Ok(json!({"text":text.trim(), "source_lang":source, "target_lang":target}))
}

pub fn decode(bytes: &[u8]) -> Result<String, DeepLXError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| DeepLXError::Response)?;
    let code = value
        .get("code")
        .and_then(Value::as_u64)
        .ok_or(DeepLXError::Response)?;
    if code != 200 {
        return Err(u16::try_from(code)
            .map(DeepLXError::Rejected)
            .unwrap_or(DeepLXError::Response));
    }
    let text = value
        .get("data")
        .and_then(Value::as_str)
        .ok_or(DeepLXError::Response)?
        .trim();
    if text.is_empty() {
        return Err(DeepLXError::Response);
    }
    if text.len() > 64 * 1024 {
        return Err(DeepLXError::TooLarge);
    }
    Ok(text.to_string())
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
                    || DEEPLX_SOURCE_CODES.contains(&source.raw_value())
            );
        }
        for target in TargetLanguage::ALL {
            assert_eq!(
                request("Synthetic.", SourceLanguage::Automatic, target).is_ok(),
                DEEPLX_TARGET_CODES.contains(&target.raw_value())
            );
        }
    }

    #[test]
    fn endpoints_preserve_prefix_without_double_translate() {
        for (input, expected) in [
            ("https://example.com", "https://example.com/translate"),
            (
                "https://example.com/api/",
                "https://example.com/api/translate",
            ),
            (
                "https://example.com/api/translate/",
                "https://example.com/api/translate",
            ),
            ("http://localhost:1188", "http://localhost:1188/translate"),
            ("http://[::1]:1188", "http://[::1]:1188/translate"),
        ] {
            assert_eq!(endpoint(input).unwrap().as_str(), expected);
        }
        for input in [
            "http://example.com",
            "ftp://example.com",
            "https://user:secret@example.com",
            "https://example.com?token=secret",
            "https://example.com/#secret",
            "invalid",
        ] {
            assert_eq!(endpoint(input), Err(DeepLXError::Endpoint));
        }
    }
    #[test]
    fn maps_languages_and_drops_untrusted_error_content() {
        for (source, code) in [
            (SourceLanguage::Automatic, "auto"),
            (SourceLanguage::Chinese, "ZH"),
            (SourceLanguage::English, "EN"),
            (SourceLanguage::Japanese, "JA"),
            (SourceLanguage::Korean, "KO"),
        ] {
            for (target, expected) in [
                (TargetLanguage::SimplifiedChinese, "ZH"),
                (TargetLanguage::English, "EN"),
                (TargetLanguage::Japanese, "JA"),
            ] {
                let body = request(" synthetic ", source, target).unwrap();
                assert_eq!(
                    body,
                    json!({"text":"synthetic","source_lang":code,"target_lang":expected})
                );
            }
        }
        assert_eq!(
            decode(br#"{"code":403,"message":"private text token"}"#),
            Err(DeepLXError::Rejected(403))
        );
        for body in [
            r#"{"code":200,"data":" "}"#,
            r#"{"code":200,"data":[]}"#,
            r#"{"data":"hello"}"#,
            "invalid",
        ] {
            assert_eq!(decode(body.as_bytes()), Err(DeepLXError::Response));
        }
        assert_eq!(
            decode(r#"{"code":200,"data":" 合成字幕 "}"#.as_bytes()).unwrap(),
            "合成字幕"
        );
    }
}
