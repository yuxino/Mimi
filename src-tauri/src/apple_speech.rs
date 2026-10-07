//! Platform adapter for Apple's on-device SpeechAnalyzer. No Tauri, credentials,
//! network clients, or capture APIs belong here. Each handle is one audio source.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleSpeechCapabilities {
    pub available: bool,
    pub locales: Vec<AppleSpeechLocale>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleSpeechLocale {
    pub identifier: String,
    /// Preserve the native module state: unsupported is not a missing download.
    #[serde(default)]
    pub status: AppleSpeechResourceStatus,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppleSpeechResourceStatus {
    Unsupported,
    Supported,
    Downloading,
    Installed,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone)]
pub struct AppleSpeechEvent {
    pub text: String,
    pub start_ms: f64,
    pub end_ms: f64,
    pub is_final: bool,
}

// Deliberately omit transcript content from diagnostic formatting.
impl std::fmt::Debug for AppleSpeechEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppleSpeechEvent")
            .field("text_bytes", &self.text.len())
            .field("start_ms", &self.start_ms)
            .field("end_ms", &self.end_ms)
            .field("is_final", &self.is_final)
            .finish()
    }
}

#[derive(Clone, Debug, thiserror::Error)]
// Other platforms only construct Unavailable, while sharing this adapter API.
#[cfg_attr(
    not(all(target_os = "macos", target_arch = "aarch64")),
    allow(dead_code)
)]
pub enum AppleSpeechError {
    #[error("Apple Speech is unavailable on this device")]
    Unavailable,
    #[error("Download the selected Apple speech language before starting")]
    AssetsNotInstalled,
    #[error("Apple Speech language preparation is still in progress")]
    AssetsDownloading,
    #[error("Apple Speech does not support the selected language")]
    InvalidLocale,
    #[error("Apple Speech cannot accept the required audio format")]
    IncompatibleFormat,
    #[error("Apple Speech received invalid PCM audio")]
    InvalidPcm,
    #[error("Apple Speech cannot keep up with the audio or result queue")]
    QueueOverflow,
    #[error("Apple Speech session is closed")]
    Closed,
    #[error("Apple Speech returned an invalid result")]
    InvalidResult,
    #[error("Apple Speech operation timed out")]
    Timeout,
    #[error("Apple Speech language reservations are full")]
    ReservationLimit,
    #[error("Apple Speech system resources are unavailable")]
    ResourcesUnavailable,
    #[error("Apple Speech system service could not be reached")]
    ServiceUnavailable,
    #[error("Apple Speech language download was cancelled")]
    DownloadCancelled,
    #[error("Apple Speech language download could not reach the server")]
    DownloadNetwork,
    #[error("Apple Speech language download needs more disk space")]
    DownloadStorage,
    #[error("Apple Speech could not confirm the installed language state")]
    StatusUnavailable,
    #[error("Apple Speech failed ({domain}, {code})")]
    Native { domain: String, code: i64 },
}

/// Resolve an explicit source language against the dynamically supported list.
/// This is a preference order, not a hard-coded support list or auto detection.
pub fn preferred_locale<'a>(
    language_code: &str,
    locales: &'a [AppleSpeechLocale],
) -> Option<&'a str> {
    let requested = canonical_speech_locale(language_code);
    if requested.is_empty() || requested == "auto" {
        return None;
    }
    if let Some(exact) = locales
        .iter()
        .find(|locale| canonical_speech_locale(&locale.identifier) == requested)
    {
        return Some(&exact.identifier);
    }
    let preferred = match requested.as_str() {
        "en" => "en-US",
        "zh" => "zh-CN",
        "yue" => "yue-CN",
        "ja" => "ja-JP",
        "ko" => "ko-KR",
        "de" => "de-DE",
        "fr" => "fr-FR",
        "es" => "es-ES",
        "it" => "it-IT",
        "pt" => "pt-BR",
        _ => "",
    };
    if let Some(exact) = locales.iter().find(|locale| {
        locale
            .identifier
            .replace('_', "-")
            .eq_ignore_ascii_case(preferred)
    }) {
        return Some(&exact.identifier);
    }
    // Never turn a requested region/script into a different one silently.
    if requested.contains('-') {
        return None;
    }
    locales
        .iter()
        .filter(|locale| {
            let normalized = canonical_speech_locale(&locale.identifier);
            // The selected `zh` code is emitted to the translation pipeline as
            // Simplified Chinese. A Hant locale would make its text skip needed
            // script conversion through the same-language passthrough path.
            if requested == "zh"
                && (normalized.split('-').any(|part| part == "hant")
                    || ["-tw", "-hk", "-mo"]
                        .iter()
                        .any(|suffix| normalized.ends_with(suffix)))
            {
                return false;
            }
            normalized
                .split('-')
                .next()
                .is_some_and(|language| language.eq_ignore_ascii_case(&requested))
        })
        .min_by(|a, b| a.identifier.cmp(&b.identifier))
        .map(|locale| locale.identifier.as_str())
}

fn canonical_speech_locale(code: &str) -> String {
    let normalized = code.replace('_', "-").to_ascii_lowercase();
    let (base, region) = normalized.split_once('-').unwrap_or((&normalized, ""));
    let base = match base {
        "nb" => "no",
        "fil" => "tl",
        base => base,
    };
    if region.is_empty() {
        base.into()
    } else {
        format!("{base}-{region}")
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod native;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub use native::{capabilities, prepare, start, AppleSpeechEvents, AppleSpeechSession};

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
mod unsupported {
    use super::{AppleSpeechCapabilities, AppleSpeechError, AppleSpeechEvent};

    pub async fn capabilities() -> Result<AppleSpeechCapabilities, AppleSpeechError> {
        Ok(AppleSpeechCapabilities::default())
    }
    pub async fn prepare(_: &str) -> Result<(), AppleSpeechError> {
        Err(AppleSpeechError::Unavailable)
    }
    pub async fn start(
        _: &str,
    ) -> Result<(AppleSpeechSession, AppleSpeechEvents), AppleSpeechError> {
        Err(AppleSpeechError::Unavailable)
    }
    pub struct AppleSpeechSession;
    impl AppleSpeechSession {
        pub fn send_pcm(&self, _: &[u8]) -> Result<(), AppleSpeechError> {
            Err(AppleSpeechError::Unavailable)
        }
        pub async fn finish(&mut self) -> Result<(), AppleSpeechError> {
            Err(AppleSpeechError::Unavailable)
        }
        pub fn cancel(&mut self) {}
    }
    pub struct AppleSpeechEvents;
    impl AppleSpeechEvents {
        pub async fn recv(&mut self) -> Result<Option<AppleSpeechEvent>, AppleSpeechError> {
            Err(AppleSpeechError::Unavailable)
        }
    }
}
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub use unsupported::{capabilities, prepare, start, AppleSpeechEvents, AppleSpeechSession};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_resource_status_preserves_unknown_instead_of_trusting_legacy_flags() {
        for (status, expected) in [
            ("unsupported", AppleSpeechResourceStatus::Unsupported),
            ("supported", AppleSpeechResourceStatus::Supported),
            ("downloading", AppleSpeechResourceStatus::Downloading),
            ("installed", AppleSpeechResourceStatus::Installed),
            ("unknown", AppleSpeechResourceStatus::Unknown),
            ("future_status", AppleSpeechResourceStatus::Unknown),
        ] {
            let locale: AppleSpeechLocale = serde_json::from_value(serde_json::json!({
                "identifier": "ja-JP", "status": status, "installed": true
            }))
            .unwrap();
            assert_eq!(locale.status, expected);
        }
        let missing: AppleSpeechLocale = serde_json::from_value(serde_json::json!({
            "identifier": "ja-JP", "installed": true, "downloading": false
        }))
        .unwrap();
        assert_eq!(missing.status, AppleSpeechResourceStatus::Unknown);
    }

    fn locales(names: &[&str]) -> Vec<AppleSpeechLocale> {
        names
            .iter()
            .map(|name| AppleSpeechLocale {
                identifier: (*name).to_owned(),
                status: AppleSpeechResourceStatus::Supported,
            })
            .collect()
    }

    #[test]
    fn selects_only_dynamic_languages_and_preserves_explicit_regions() {
        let available = locales(&["en-GB", "en-US", "zh-TW", "ja-JP"]);
        assert_eq!(preferred_locale("en", &available), Some("en-US"));
        assert_eq!(preferred_locale("EN_gb", &available), Some("en-GB"));
        assert_eq!(preferred_locale("zh", &available), None);
        assert_eq!(preferred_locale("zh-CN", &available), None);
        assert_eq!(preferred_locale("auto", &available), None);
        assert_eq!(preferred_locale("ko", &available), None);
    }

    #[test]
    fn diagnostics_never_include_recognized_text() {
        let event = AppleSpeechEvent {
            text: "private recognized content".into(),
            start_ms: 0.0,
            end_ms: 10.0,
            is_final: false,
        };
        assert!(!format!("{event:?}").contains("private recognized content"));
    }

    #[test]
    fn normalizes_native_locale_separators_before_region_preference() {
        let available = locales(&["en_AU", "en_US", "en_GB", "zh_CN"]);
        assert_eq!(preferred_locale("en", &available), Some("en_US"));
        assert_eq!(preferred_locale("EN_gb", &available), Some("en_GB"));
        assert_eq!(preferred_locale("zh-CN", &available), Some("zh_CN"));
        assert_eq!(preferred_locale("en-CA", &available), None);
    }

    #[test]
    fn language_aliases_preserve_explicit_regions_and_runtime_inventory() {
        let available = locales(&["nb-NO", "fil_PH"]);
        assert_eq!(preferred_locale("no", &available), Some("nb-NO"));
        assert_eq!(preferred_locale("no-NO", &available), Some("nb-NO"));
        assert_eq!(preferred_locale("tl", &available), Some("fil_PH"));
        assert_eq!(preferred_locale("tl-PH", &available), Some("fil_PH"));
        assert_eq!(preferred_locale("no-SE", &available), None);
        assert_eq!(preferred_locale("tl-US", &available), None);
        let aliases = locales(&["no_NO", "tl-PH"]);
        assert_eq!(preferred_locale("nb-NO", &aliases), Some("no_NO"));
        assert_eq!(preferred_locale("fil-PH", &aliases), Some("tl-PH"));
        assert_eq!(preferred_locale("no", &locales(&["en-US"])), None);
    }

    #[test]
    fn simplified_chinese_never_uses_a_traditional_recognition_locale() {
        for name in ["zh-TW", "zh_HK", "zh-MO", "zh-Hant", "zh-Hant-CN"] {
            assert_eq!(preferred_locale("zh", &locales(&[name])), None, "{name}");
        }
        assert_eq!(
            preferred_locale("zh", &locales(&["zh-TW", "zh-SG"])),
            Some("zh-SG")
        );
        assert_eq!(
            preferred_locale("zh", &locales(&["zh-TW", "zh-CN"])),
            Some("zh-CN")
        );
        assert_eq!(
            preferred_locale("zh_tw", &locales(&["zh-TW"])),
            Some("zh-TW")
        );
    }
}
