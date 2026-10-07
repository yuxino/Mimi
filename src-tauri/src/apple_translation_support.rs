//! Runtime text-language inventory and explicit Apple language-pack preparation.
//! Reading support/status and translating never requests a download.

use crate::apple_translation::{self, AppleTranslationCapabilities, AppleTranslationStatus};
use crate::core::models::{SourceLanguage, TargetLanguage};
use serde::Serialize;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleTranslationSupport {
    pub available: bool,
    pub source_languages: Vec<SourceLanguage>,
    pub target_languages: Vec<TargetLanguage>,
}

static SUPPORT: OnceLock<Mutex<Option<AppleTranslationSupport>>> = OnceLock::new();
static PREPARING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn cached() -> AppleTranslationSupport {
    SUPPORT
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_default()
}

pub fn is_loaded() -> bool {
    SUPPORT
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .is_some()
}

pub async fn refresh() -> Result<AppleTranslationSupport, String> {
    let result = apple_translation::capabilities()
        .await
        .map(map_capabilities)
        .map_err(|error| error_label(&error).to_string());
    *SUPPORT.get_or_init(Default::default).lock().unwrap() =
        Some(result.clone().unwrap_or_default());
    result
}

/// Wire codes differ from Apple's BCP 47 identifiers for Chinese and Tagalog.
pub fn source_code(source: SourceLanguage) -> Result<&'static str, &'static str> {
    if source.raw_value() == "zh_tw" {
        return Ok("zh-Hant");
    }
    match source {
        SourceLanguage::Automatic => Err("apple_translation_language_unsupported"),
        SourceLanguage::Chinese => Ok("zh-Hans"),
        SourceLanguage::Filipino => Ok("fil"),
        source => Ok(source.raw_value()),
    }
}

pub fn target_code(target: TargetLanguage) -> Result<&'static str, &'static str> {
    match target {
        TargetLanguage::Original => Err("apple_translation_language_unsupported"),
        TargetLanguage::SimplifiedChinese => Ok("zh-Hans"),
        TargetLanguage::TraditionalChinese => Ok("zh-Hant"),
        TargetLanguage::Tagalog => Ok("fil"),
        target => Ok(target.raw_value()),
    }
}

fn canonical_language(code: &str) -> String {
    let normalized = code.replace('_', "-").to_ascii_lowercase();
    let base = normalized.split('-').next().unwrap_or_default();
    match base {
        "zh" if normalized.contains("hant")
            || normalized.ends_with("-tw")
            || normalized.ends_with("-hk") =>
        {
            "zh-hant".into()
        }
        "zh" => "zh-hans".into(),
        "tl" | "fil" => "fil".into(),
        "nb" | "no" => "no".into(),
        _ => base.into(),
    }
}

fn map_capabilities(native: AppleTranslationCapabilities) -> AppleTranslationSupport {
    if !native.available {
        return AppleTranslationSupport::default();
    }
    let available = |code: &str| {
        native.languages.iter().any(|language| {
            if code.contains('-') && !code.starts_with("zh-") {
                language.replace('_', "-").eq_ignore_ascii_case(code)
            } else {
                canonical_language(language) == canonical_language(code)
            }
        })
    };
    let source_languages = SourceLanguage::ALL
        .into_iter()
        .filter(|source| source_code(*source).is_ok_and(available))
        .collect::<Vec<_>>();
    let target_languages = TargetLanguage::ALL
        .into_iter()
        .filter(|target| target_code(*target).is_ok_and(available))
        .collect::<Vec<_>>();
    AppleTranslationSupport {
        available: !source_languages.is_empty() && !target_languages.is_empty(),
        source_languages,
        target_languages,
    }
}

pub async fn status(
    source: SourceLanguage,
    target: TargetLanguage,
) -> Result<AppleTranslationStatus, String> {
    let source = source_code(source).map_err(str::to_owned)?;
    let target = target_code(target).map_err(str::to_owned)?;
    apple_translation::status(source, target)
        .await
        .map_err(|error| error_label(&error).to_string())
}

/// Validate an actual pair before a saved or live selection changes. A same-
/// language route retains the existing subtitle passthrough without claiming
/// that Apple owns an installed same-language translation model.
pub async fn validate_pair(
    source: SourceLanguage,
    target: TargetLanguage,
    installed_required: bool,
) -> Result<(), String> {
    if target == TargetLanguage::Original {
        return Ok(());
    }
    source_code(source).map_err(str::to_owned)?;
    if target.matches_reported_asr(Some(source.raw_value())) {
        let support = refresh().await?;
        return if support.available
            && support.source_languages.contains(&source)
            && support.target_languages.contains(&target)
        {
            Ok(())
        } else {
            Err("apple_translation_unavailable".into())
        };
    }
    match status(source, target).await? {
        AppleTranslationStatus::Installed => Ok(()),
        AppleTranslationStatus::Supported if !installed_required => Ok(()),
        AppleTranslationStatus::Supported => Err("apple_translation_assets_missing".into()),
        AppleTranslationStatus::Unsupported => Err("apple_translation_language_unsupported".into()),
        AppleTranslationStatus::Unavailable => Err("apple_translation_unavailable".into()),
    }
}

pub async fn require_installed(
    source: SourceLanguage,
    target: TargetLanguage,
) -> Result<(), String> {
    validate_pair(source, target, true).await
}

pub async fn prepare(
    source: SourceLanguage,
    target: TargetLanguage,
    ui_language: &str,
) -> Result<(), String> {
    if !matches!(ui_language, "en" | "zh" | "ja") {
        return Err("apple_translation_invalid_input".into());
    }
    let _preparing = PREPARING
        .try_lock()
        .map_err(|_| "apple_translation_busy".to_string())?;
    let source_code = source_code(source).map_err(str::to_owned)?;
    let target_code = target_code(target).map_err(str::to_owned)?;
    apple_translation::prepare(source_code, target_code, ui_language)
        .await
        .map_err(|error| error_label(&error).to_string())?;
    // Apple can return from preparation while another process downloads. Do not
    // announce readiness until the current pair is actually installed.
    require_installed(source, target).await
}

/// Only stable allowlisted labels may leave the native adapter or enter logs.
pub fn error_label(error: &apple_translation::AppleTranslationError) -> &'static str {
    match error.to_string().as_str() {
        "apple_translation_unavailable" => "apple_translation_unavailable",
        "apple_translation_assets_missing" => "apple_translation_assets_missing",
        "apple_translation_language_unsupported" => "apple_translation_language_unsupported",
        "apple_translation_invalid_input" => "apple_translation_invalid_input",
        "apple_translation_cancelled" => "apple_translation_cancelled",
        "apple_translation_busy" => "apple_translation_busy",
        "apple_translation_timeout" => "apple_translation_timeout",
        "apple_translation_invalid_result" => "apple_translation_invalid_result",
        _ => "apple_translation_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_inventory_preserves_chinese_scripts_and_hides_auto_original() {
        let support = map_capabilities(AppleTranslationCapabilities {
            available: true,
            languages: vec![
                "en-US".into(),
                "ja".into(),
                "zh-Hant".into(),
                "fil".into(),
                "xx".into(),
            ],
        });
        assert!(support.available);
        assert!(support.source_languages.contains(&SourceLanguage::English));
        assert!(support.source_languages.contains(&SourceLanguage::Filipino));
        assert!(!support
            .source_languages
            .contains(&SourceLanguage::Automatic));
        assert!(!support.source_languages.contains(&SourceLanguage::Chinese));
        assert!(support
            .target_languages
            .contains(&TargetLanguage::TraditionalChinese));
        assert!(!support
            .target_languages
            .contains(&TargetLanguage::SimplifiedChinese));
        assert!(!support.target_languages.contains(&TargetLanguage::Original));
        assert_eq!(
            target_code(TargetLanguage::TraditionalChinese),
            Ok("zh-Hant")
        );
        assert_eq!(source_code(SourceLanguage::Chinese), Ok("zh-Hans"));
    }
    #[test]
    fn unavailable_or_unrepresentable_inventory_has_no_choices() {
        for available in [false, true] {
            assert_eq!(
                map_capabilities(AppleTranslationCapabilities {
                    available,
                    languages: vec!["xx".into()]
                }),
                AppleTranslationSupport::default()
            );
        }
    }

    #[test]
    fn runtime_inventory_does_not_invent_regions_from_a_base_language() {
        let support = map_capabilities(AppleTranslationCapabilities {
            available: true,
            languages: vec!["en".into(), "pt".into(), "ar".into()],
        });
        for code in ["en", "pt", "ar"] {
            let source = serde_json::from_value(serde_json::json!(code)).unwrap();
            let target = serde_json::from_value(serde_json::json!(code)).unwrap();
            assert!(support.source_languages.contains(&source));
            assert!(support.target_languages.contains(&target));
        }
        for code in [
            "en-US", "en-GB", "pt-BR", "pt-PT", "ar-EG", "ar-SA", "ar-AE",
        ] {
            let source = serde_json::from_value(serde_json::json!(code)).unwrap();
            let target = serde_json::from_value(serde_json::json!(code)).unwrap();
            assert!(!support.source_languages.contains(&source), "{code}");
            assert!(!support.target_languages.contains(&target), "{code}");
        }
    }

    #[test]
    fn runtime_inventory_accepts_only_the_exact_reported_region_and_preserves_scripts() {
        let support = map_capabilities(AppleTranslationCapabilities {
            available: true,
            languages: vec![
                "en_US".into(),
                "pt-BR".into(),
                "ar-EG".into(),
                "zh-Hant".into(),
            ],
        });
        for code in ["en-US", "pt-BR", "ar-EG", "zh_tw"] {
            let source = serde_json::from_value(serde_json::json!(code)).unwrap();
            let target = serde_json::from_value(serde_json::json!(code)).unwrap();
            assert!(support.source_languages.contains(&source), "{code}");
            assert!(support.target_languages.contains(&target), "{code}");
        }
        for code in ["en-GB", "pt-PT", "ar-SA", "zh"] {
            let source = serde_json::from_value(serde_json::json!(code)).unwrap();
            let target = serde_json::from_value(serde_json::json!(code)).unwrap();
            assert!(!support.source_languages.contains(&source), "{code}");
            assert!(!support.target_languages.contains(&target), "{code}");
        }
    }
}
