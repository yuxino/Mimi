//! Runtime Apple capabilities mapped to Mimi's existing language choices.
//! Queries are read-only; only `prepare_source` can request language assets.

use crate::apple_speech::{
    self, AppleSpeechCapabilities, AppleSpeechError, AppleSpeechResourceStatus,
};
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::provider::ServiceProfile;
use serde::Serialize;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleSpeechLanguage {
    pub source_language: SourceLanguage,
    pub locale: String,
    pub status: AppleSpeechResourceStatus,
    pub installed: bool,
    pub downloading: bool,
}

impl AppleSpeechLanguage {
    pub fn set_status(&mut self, status: AppleSpeechResourceStatus) {
        self.status = status;
        self.installed = status == AppleSpeechResourceStatus::Installed;
        self.downloading = status == AppleSpeechResourceStatus::Downloading;
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleSpeechSupport {
    pub available: bool,
    pub languages: Vec<AppleSpeechLanguage>,
}

#[derive(Default)]
struct SupportCache {
    support: Option<AppleSpeechSupport>,
    revision: u64,
}

impl SupportCache {
    fn replace(&mut self, support: AppleSpeechSupport) {
        if self.support.as_ref() != Some(&support) {
            self.support = Some(support);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    fn snapshot(&self) -> (AppleSpeechSupport, u64) {
        (self.support.clone().unwrap_or_default(), self.revision)
    }

    fn prepared_locale(&mut self, locale: &str, before: AppleSpeechSupport) -> AppleSpeechSupport {
        let mut support = self.support.clone().unwrap_or_default();
        // Keep concurrent refreshes of other locales. If a failed refresh
        // cleared the cache, restore only the locale just confirmed by Swift.
        for mut prepared in before
            .languages
            .into_iter()
            .filter(|item| item.locale == locale)
        {
            prepared.set_status(AppleSpeechResourceStatus::Installed);
            if let Some(language) = support
                .languages
                .iter_mut()
                .find(|item| item.source_language == prepared.source_language)
            {
                *language = prepared;
            } else {
                support.languages.push(prepared);
            }
        }
        support.available = !support.languages.is_empty();
        self.replace(support.clone());
        support
    }

    fn unconfirmed_locale(
        &mut self,
        locale: &str,
        before: &AppleSpeechSupport,
    ) -> AppleSpeechSupport {
        let mut support = self.support.clone().unwrap_or_default();
        for previous in before.languages.iter().filter(|item| item.locale == locale) {
            if let Some(language) = support
                .languages
                .iter_mut()
                .find(|item| item.source_language == previous.source_language)
            {
                if !matches!(
                    language.status,
                    AppleSpeechResourceStatus::Installed | AppleSpeechResourceStatus::Downloading
                ) {
                    language.set_status(AppleSpeechResourceStatus::Unknown);
                }
            } else {
                let mut language = previous.clone();
                language.set_status(AppleSpeechResourceStatus::Unknown);
                support.languages.push(language);
            }
        }
        support.available = !support.languages.is_empty();
        self.replace(support.clone());
        support
    }
}

static SUPPORT: OnceLock<Mutex<SupportCache>> = OnceLock::new();
static REFRESHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PREPARING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Read the complete resource state and its revision under the same lock.
/// A route-filtered ready list cannot represent every resource state change.
pub fn cached_with_revision() -> (AppleSpeechSupport, u64) {
    SUPPORT
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .snapshot()
}

pub fn is_loaded() -> bool {
    SUPPORT
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .support
        .is_some()
}

pub async fn refresh() -> Result<AppleSpeechSupport, String> {
    let _refreshing = REFRESHING.lock().await;
    let native = apple_speech::capabilities()
        .await
        .map_err(|_| "apple_speech_status_failed".to_string());
    let support = native.map(map_capabilities);
    // A failed refresh must not keep a stale available/installed claim alive.
    SUPPORT
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .replace(support.clone().unwrap_or_default());
    support
}

pub async fn locale_for_source(source: SourceLanguage) -> Result<String, String> {
    let support = refresh().await?;
    let language = validate_source(&support, source, true)?;
    Ok(language.locale.clone())
}

pub async fn prepare_source(source: SourceLanguage) -> Result<AppleSpeechSupport, String> {
    let _preparing = PREPARING
        .try_lock()
        .map_err(|_| "apple_speech_preparing".to_string())?;
    let before = refresh().await?;
    let language = require_language(&before, source)?;
    match language.status {
        AppleSpeechResourceStatus::Installed | AppleSpeechResourceStatus::Downloading => {
            return Ok(before)
        }
        AppleSpeechResourceStatus::Supported => {}
        AppleSpeechResourceStatus::Unsupported | AppleSpeechResourceStatus::Unknown => {
            return Err("apple_speech_status_failed".into())
        }
    }
    let locale = language.locale.clone();
    match apple_speech::prepare(&locale).await {
        Ok(()) => {
            // Swift has just confirmed this exact module is installed. Do not
            // make acknowledgement depend on another scan of every unrelated
            // locale, which can time out after a successful installation.
            // Let an older in-flight scan finish before applying this newer
            // confirmation. Never hold this lock during the actual download.
            let _refreshing = REFRESHING.lock().await;
            Ok(SUPPORT
                .get_or_init(Default::default)
                .lock()
                .unwrap()
                .prepared_locale(&locale, before))
        }
        Err(error) => {
            // Downloads can finish while the service disconnects or a native
            // deadline expires. Publish the real post-operation inventory even
            // on failure, rather than keeping an old "not downloaded" snapshot.
            let after = refresh().await;
            if after.as_ref().is_ok_and(|support| {
                require_language(support, source).is_ok_and(|language| {
                    matches!(
                        language.status,
                        AppleSpeechResourceStatus::Installed
                            | AppleSpeechResourceStatus::Downloading
                    )
                })
            }) {
                return after;
            }
            if matches!(
                error,
                AppleSpeechError::StatusUnavailable | AppleSpeechError::ServiceUnavailable
            ) {
                // A failed module/service check is not proof of missing assets,
                // even if the immediate inventory query falls back to supported.
                let _refreshing = REFRESHING.lock().await;
                let latest = SUPPORT
                    .get_or_init(Default::default)
                    .lock()
                    .unwrap()
                    .unconfirmed_locale(&locale, &before);
                if require_language(&latest, source).is_ok_and(|language| {
                    matches!(
                        language.status,
                        AppleSpeechResourceStatus::Installed
                            | AppleSpeechResourceStatus::Downloading
                    )
                }) {
                    return Ok(latest);
                }
            }
            tracing::warn!(locale = %locale, error = %error, "Apple Speech language preparation failed");
            Err(prepare_error_label(&error).into())
        }
    }
}

fn prepare_error_label(error: &AppleSpeechError) -> &'static str {
    match error {
        AppleSpeechError::Unavailable => "apple_speech_unavailable",
        AppleSpeechError::InvalidLocale => "apple_speech_language_unsupported",
        AppleSpeechError::AssetsNotInstalled => "apple_speech_assets_missing",
        AppleSpeechError::AssetsDownloading => "apple_speech_preparing",
        AppleSpeechError::ReservationLimit => "apple_speech_reservation_limit",
        AppleSpeechError::ResourcesUnavailable => "apple_speech_resources_unavailable",
        AppleSpeechError::ServiceUnavailable => "apple_speech_service_unavailable",
        AppleSpeechError::DownloadCancelled => "apple_speech_download_cancelled",
        AppleSpeechError::DownloadNetwork => "apple_speech_download_network",
        AppleSpeechError::DownloadStorage => "apple_speech_download_storage",
        AppleSpeechError::StatusUnavailable => "apple_speech_status_failed",
        AppleSpeechError::Timeout => "apple_speech_download_timeout",
        _ => "apple_speech_prepare_failed",
    }
}

/// A saved idle selection may precede an explicit download, but a live or
/// paused session must have assets before its resumable language can change.
pub fn validate_source(
    support: &AppleSpeechSupport,
    source: SourceLanguage,
    installed_required: bool,
) -> Result<&AppleSpeechLanguage, String> {
    let language = require_language(support, source)?;
    match language.status {
        AppleSpeechResourceStatus::Unsupported | AppleSpeechResourceStatus::Unknown => {
            return Err("apple_speech_status_failed".into())
        }
        AppleSpeechResourceStatus::Downloading if installed_required => {
            return Err("apple_speech_preparing".into())
        }
        AppleSpeechResourceStatus::Supported if installed_required => {
            return Err("apple_speech_assets_missing".into())
        }
        _ => {}
    }
    Ok(language)
}

/// Runtime support and the chosen text encoder must both accept the source.
/// Refuse a route change before normalization can select a different OS locale.
pub fn validate_profile_source<'a>(
    support: &'a AppleSpeechSupport,
    profile: &ServiceProfile,
    source: SourceLanguage,
    target: TargetLanguage,
    installed_required: bool,
) -> Result<&'a AppleSpeechLanguage, String> {
    let language = validate_source(support, source, installed_required)?;
    if !profile
        .capabilities(target)
        .source_languages
        .contains(&source)
    {
        return Err("apple_speech_translation_language_unsupported".into());
    }
    Ok(language)
}

/// Refresh has already replaced the cached capability snapshot. Even when
/// selection is rejected, publish that snapshot so other windows discard
/// choices whose resources disappeared or whose status could not be read.
pub fn validate_refreshed_profile_source(
    support: Result<AppleSpeechSupport, String>,
    profile: &ServiceProfile,
    source: SourceLanguage,
    target: TargetLanguage,
    installed_required: bool,
    publish_rejected: impl FnOnce(),
) -> Result<(), String> {
    support
        .and_then(|support| {
            validate_profile_source(&support, profile, source, target, installed_required)
                .map(|_| ())
        })
        .inspect_err(|_| publish_rejected())
}

fn require_language(
    support: &AppleSpeechSupport,
    source: SourceLanguage,
) -> Result<&AppleSpeechLanguage, String> {
    if !support.available {
        return Err("apple_speech_unavailable".into());
    }
    support
        .languages
        .iter()
        .find(|language| language.source_language == source)
        .ok_or_else(|| "apple_speech_language_unsupported".to_string())
}

fn map_capabilities(native: AppleSpeechCapabilities) -> AppleSpeechSupport {
    if !native.available {
        return AppleSpeechSupport::default();
    }
    let mut languages = Vec::new();
    for source in SourceLanguage::ALL {
        if source == SourceLanguage::Automatic {
            continue;
        }
        // Choosing a locale must be stable when assets are installed/removed.
        // Installed status is deliberately not a sort key.
        if let Some(identifier) =
            apple_speech::preferred_locale(source.raw_value(), &native.locales)
        {
            let locale = native
                .locales
                .iter()
                .find(|locale| locale.identifier == identifier)
                .unwrap();
            languages.push(AppleSpeechLanguage {
                source_language: source,
                locale: locale.identifier.clone(),
                status: locale.status,
                installed: locale.status == AppleSpeechResourceStatus::Installed,
                downloading: locale.status == AppleSpeechResourceStatus::Downloading,
            });
        }
    }
    AppleSpeechSupport {
        available: !languages.is_empty(),
        languages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple_speech::AppleSpeechLocale;

    fn locale(identifier: &str, installed: bool) -> AppleSpeechLocale {
        AppleSpeechLocale {
            identifier: identifier.into(),
            status: if installed {
                AppleSpeechResourceStatus::Installed
            } else {
                AppleSpeechResourceStatus::Supported
            },
        }
    }

    #[test]
    fn only_supported_status_is_a_missing_resource() {
        for status in [
            AppleSpeechResourceStatus::Unsupported,
            AppleSpeechResourceStatus::Unknown,
        ] {
            let support = map_capabilities(AppleSpeechCapabilities {
                available: true,
                locales: vec![AppleSpeechLocale {
                    identifier: "ja-JP".into(),
                    status,
                }],
            });
            let language = require_language(&support, SourceLanguage::Japanese).unwrap();
            assert_eq!(language.status, status);
            assert!(!language.installed);
            assert!(!language.downloading);
            for installed_required in [false, true] {
                assert_eq!(
                    validate_source(&support, SourceLanguage::Japanese, installed_required)
                        .unwrap_err(),
                    "apple_speech_status_failed"
                );
            }
        }
        let support = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![locale("ja-JP", false)],
        });
        assert!(validate_source(&support, SourceLanguage::Japanese, false).is_ok());
        assert_eq!(
            validate_source(&support, SourceLanguage::Japanese, true).unwrap_err(),
            "apple_speech_assets_missing"
        );
    }

    #[test]
    fn failed_readiness_invalidates_missing_claim_and_publishes_a_revision() {
        let before = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![locale("en-US", true), locale("ja-JP", false)],
        });
        let mut cache = SupportCache::default();
        cache.replace(before.clone());
        let revision = cache.snapshot().1;
        let after = cache.unconfirmed_locale("ja-JP", &before);
        assert_eq!(cache.snapshot().1, revision + 1);
        assert!(
            require_language(&after, SourceLanguage::English)
                .unwrap()
                .installed
        );
        let uncertain = require_language(&after, SourceLanguage::Japanese).unwrap();
        assert_eq!(uncertain.status, AppleSpeechResourceStatus::Unknown);
        assert!(!uncertain.installed);
        assert!(!uncertain.downloading);
        assert_eq!(
            validate_source(&after, SourceLanguage::Japanese, true).unwrap_err(),
            "apple_speech_status_failed"
        );
        cache.unconfirmed_locale("ja-JP", &before);
        assert_eq!(cache.snapshot().1, revision + 1);
    }

    #[test]
    fn failed_readiness_preserves_newer_download_or_install_confirmation() {
        let before = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![locale("ja-JP", false)],
        });
        for status in [
            AppleSpeechResourceStatus::Installed,
            AppleSpeechResourceStatus::Downloading,
        ] {
            let mut cache = SupportCache::default();
            cache.replace(map_capabilities(AppleSpeechCapabilities {
                available: true,
                locales: vec![AppleSpeechLocale {
                    identifier: "ja-JP".into(),
                    status,
                }],
            }));
            let revision = cache.snapshot().1;
            let after = cache.unconfirmed_locale("ja-JP", &before);
            assert_eq!(
                require_language(&after, SourceLanguage::Japanese)
                    .unwrap()
                    .status,
                status
            );
            assert_eq!(cache.snapshot().1, revision);
        }
    }

    #[test]
    fn prepared_locale_keeps_newer_status_of_other_languages() {
        let before = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![locale("en-US", true), locale("ja-JP", false)],
        });
        let mut cache = SupportCache::default();
        cache.replace(map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![
                locale("en-US", false),
                locale("ja-JP", false),
                locale("fr-FR", true),
            ],
        }));
        let after = cache.prepared_locale("ja-JP", before);
        assert!(
            require_language(&after, SourceLanguage::Japanese)
                .unwrap()
                .installed
        );
        assert!(
            !require_language(&after, SourceLanguage::English)
                .unwrap()
                .installed
        );
        assert!(
            require_language(&after, SourceLanguage::French)
                .unwrap()
                .installed
        );
    }

    #[test]
    fn prepared_locale_does_not_restore_unrelated_stale_readiness() {
        let before = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![locale("en-US", true), locale("ja-JP", false)],
        });
        let mut cache = SupportCache::default();
        cache.replace(AppleSpeechSupport::default());
        let after = cache.prepared_locale("ja-JP", before);
        assert!(after.available);
        assert_eq!(after.languages.len(), 1);
        assert!(
            require_language(&after, SourceLanguage::Japanese)
                .unwrap()
                .installed
        );
        assert!(require_language(&after, SourceLanguage::English).is_err());
    }

    #[test]
    fn background_download_is_distinct_from_missing_or_installed_assets() {
        let mut pending = locale("ja-JP", false);
        pending.status = AppleSpeechResourceStatus::Downloading;
        let support = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![pending],
        });
        assert!(
            require_language(&support, SourceLanguage::Japanese)
                .unwrap()
                .downloading
        );
        assert_eq!(
            validate_source(&support, SourceLanguage::Japanese, true).unwrap_err(),
            "apple_speech_preparing"
        );
        let mut cache = SupportCache::default();
        cache.replace(support.clone());
        let after = cache.prepared_locale("ja-JP", support);
        let ready = validate_source(&after, SourceLanguage::Japanese, true).unwrap();
        assert!(ready.installed);
        assert!(!ready.downloading);
    }

    #[test]
    fn preparation_failures_keep_actionable_safe_categories() {
        for (error, expected) in [
            (
                AppleSpeechError::AssetsDownloading,
                "apple_speech_preparing",
            ),
            (
                AppleSpeechError::ServiceUnavailable,
                "apple_speech_service_unavailable",
            ),
            (
                AppleSpeechError::ReservationLimit,
                "apple_speech_reservation_limit",
            ),
            (
                AppleSpeechError::ResourcesUnavailable,
                "apple_speech_resources_unavailable",
            ),
            (
                AppleSpeechError::DownloadCancelled,
                "apple_speech_download_cancelled",
            ),
            (
                AppleSpeechError::DownloadNetwork,
                "apple_speech_download_network",
            ),
            (
                AppleSpeechError::DownloadStorage,
                "apple_speech_download_storage",
            ),
            (
                AppleSpeechError::StatusUnavailable,
                "apple_speech_status_failed",
            ),
            (AppleSpeechError::Timeout, "apple_speech_download_timeout"),
            (
                AppleSpeechError::Native {
                    domain: "private/path/content".into(),
                    code: 9,
                },
                "apple_speech_prepare_failed",
            ),
        ] {
            assert_eq!(prepare_error_label(&error), expected);
        }
    }

    #[test]
    fn equivalent_refreshes_keep_the_complete_support_revision_stable() {
        let mut cache = SupportCache::default();
        assert!(cache.support.is_none());
        assert_eq!(cache.snapshot(), (AppleSpeechSupport::default(), 0));
        cache.replace(map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![locale("en-US", true), locale("ja-JP", false)],
        }));
        let first = cache.snapshot();
        assert_eq!(first.1, 1);
        // Native ordering has no semantic effect after locale mapping.
        cache.replace(map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![locale("ja-JP", false), locale("en-US", true)],
        }));
        assert_eq!(cache.snapshot(), first);
        // Repeated failed/unavailable refreshes must not create event loops.
        cache.replace(AppleSpeechSupport::default());
        assert_eq!(cache.snapshot(), (AppleSpeechSupport::default(), 2));
        cache.replace(AppleSpeechSupport::default());
        assert_eq!(cache.snapshot(), (AppleSpeechSupport::default(), 2));
    }

    #[test]
    fn support_revision_changes_even_when_the_route_ready_list_does_not() {
        use crate::core::provider::{ProviderKind, TextTranslation};
        let mut profile =
            ServiceProfile::new("synthetic", "Synthetic", ProviderKind::AppleSpeech).unwrap();
        profile.text_translation = Some(TextTranslation::DeepL);
        let ready_sources = |support: &AppleSpeechSupport| {
            profile
                .capabilities(TargetLanguage::English)
                .source_languages
                .into_iter()
                .filter(|source| validate_source(support, *source, true).is_ok())
                .collect::<Vec<_>>()
        };
        let mut cache = SupportCache::default();
        let mut support = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![locale("en-US", true), locale("km-KH", true)],
        });
        cache.replace(support.clone());
        let before = cache.snapshot();
        assert_eq!(ready_sources(&before.0), vec![SourceLanguage::English]);
        support
            .languages
            .iter_mut()
            .find(|language| language.source_language == SourceLanguage::Khmer)
            .unwrap()
            .set_status(AppleSpeechResourceStatus::Supported);
        cache.replace(support.clone());
        let after = cache.snapshot();
        assert_eq!(ready_sources(&after.0), ready_sources(&before.0));
        assert_eq!(after.1, before.1 + 1);

        support
            .languages
            .iter_mut()
            .for_each(|language| language.set_status(AppleSpeechResourceStatus::Supported));
        cache.replace(support);
        let available_without_ready_assets = cache.snapshot();
        assert!(available_without_ready_assets.0.available);
        assert!(ready_sources(&available_without_ready_assets.0).is_empty());
        cache.replace(AppleSpeechSupport::default());
        let unavailable = cache.snapshot();
        assert!(!unavailable.0.available);
        assert!(ready_sources(&unavailable.0).is_empty());
        assert_eq!(unavailable.1, available_without_ready_assets.1 + 1);
    }

    #[test]
    fn live_language_selection_distinguishes_missing_assets_from_unsupported_locales() {
        let support = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![locale("en-US", true), locale("ja-JP", false)],
        });
        assert!(validate_source(&support, SourceLanguage::English, true).is_ok());
        assert!(validate_source(&support, SourceLanguage::Japanese, false).is_ok());
        assert_eq!(
            validate_source(&support, SourceLanguage::Japanese, true).unwrap_err(),
            "apple_speech_assets_missing"
        );
        assert_eq!(
            validate_source(&support, SourceLanguage::French, true).unwrap_err(),
            "apple_speech_language_unsupported"
        );
        assert_eq!(
            validate_source(
                &AppleSpeechSupport::default(),
                SourceLanguage::English,
                true
            )
            .unwrap_err(),
            "apple_speech_unavailable"
        );
    }

    #[test]
    fn rejects_incompatible_text_routes_without_inventing_an_os_language() {
        use crate::core::provider::{ProviderKind, TextTranslation};
        let support = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![locale("en-US", true), locale("km-KH", true)],
        });
        for route in [TextTranslation::DeepL, TextTranslation::DeepLX] {
            let mut profile =
                ServiceProfile::new("synthetic", "Synthetic", ProviderKind::AppleSpeech).unwrap();
            profile.text_translation = Some(route);
            assert!(validate_profile_source(
                &support,
                &profile,
                SourceLanguage::Khmer,
                TargetLanguage::Original,
                true,
            )
            .is_ok());
            assert_eq!(
                validate_profile_source(
                    &support,
                    &profile,
                    SourceLanguage::Khmer,
                    TargetLanguage::English,
                    true,
                )
                .unwrap_err(),
                "apple_speech_translation_language_unsupported"
            );
            assert!(validate_profile_source(
                &support,
                &profile,
                SourceLanguage::English,
                TargetLanguage::English,
                true,
            )
            .is_ok());
            assert_eq!(
                validate_profile_source(
                    &support,
                    &profile,
                    SourceLanguage::Chinese,
                    TargetLanguage::English,
                    true,
                )
                .unwrap_err(),
                "apple_speech_language_unsupported"
            );
        }
    }

    #[test]
    fn rejected_refreshed_selections_publish_capabilities_and_preserve_the_actual_error() {
        use crate::core::provider::{ProviderKind, TextTranslation};
        let mut profile =
            ServiceProfile::new("synthetic", "Synthetic", ProviderKind::AppleSpeech).unwrap();
        profile.text_translation = Some(TextTranslation::DeepL);
        let support = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![
                locale("en-US", true),
                locale("ja-JP", false),
                locale("km-KH", true),
            ],
        });
        for (refreshed, source, expected) in [
            (
                Err("apple_speech_status_failed".to_string()),
                SourceLanguage::English,
                "apple_speech_status_failed",
            ),
            (
                Ok(AppleSpeechSupport::default()),
                SourceLanguage::English,
                "apple_speech_unavailable",
            ),
            (
                Ok(support.clone()),
                SourceLanguage::Japanese,
                "apple_speech_assets_missing",
            ),
            (
                Ok(support.clone()),
                SourceLanguage::Chinese,
                "apple_speech_language_unsupported",
            ),
            (
                Ok(support.clone()),
                SourceLanguage::Khmer,
                "apple_speech_translation_language_unsupported",
            ),
        ] {
            let mut publications = 0;
            let result = validate_refreshed_profile_source(
                refreshed,
                &profile,
                source,
                TargetLanguage::English,
                true,
                || publications += 1,
            );
            assert_eq!(result.unwrap_err(), expected);
            assert_eq!(publications, 1);
        }
        // Accepted changes keep the normal publish-after-save boundary. Idle
        // preferences may still name a supported language before preparation.
        for (source, installed_required) in [
            (SourceLanguage::English, true),
            (SourceLanguage::Japanese, false),
        ] {
            assert!(validate_refreshed_profile_source(
                Ok(support.clone()),
                &profile,
                source,
                TargetLanguage::English,
                installed_required,
                || panic!("successful validation published before saving"),
            )
            .is_ok());
        }
    }

    #[test]
    fn hides_unavailable_devices_and_unrepresentable_languages() {
        for native in [
            AppleSpeechCapabilities {
                available: false,
                locales: vec![locale("en-US", true)],
            },
            AppleSpeechCapabilities {
                available: true,
                locales: vec![locale("zz-XX", true)],
            },
        ] {
            assert_eq!(map_capabilities(native), AppleSpeechSupport::default());
        }
    }

    #[test]
    fn language_choices_are_dynamic_explicit_and_region_stable() {
        let support = map_capabilities(AppleSpeechCapabilities {
            available: true,
            locales: vec![
                locale("en_AU", true),
                locale("en_US", false),
                locale("ja_JP", true),
                locale("fr-FR", true),
                locale("yue-HK", true),
            ],
        });
        assert!(support.available);
        assert_eq!(support.languages.len(), 5);
        let english = require_language(&support, SourceLanguage::English).unwrap();
        assert_eq!(english.locale, "en_US");
        assert!(!english.installed);
        assert!(require_language(&support, SourceLanguage::Automatic).is_err());
        assert!(require_language(&support, SourceLanguage::Chinese).is_err());
        assert_eq!(
            require_language(&support, SourceLanguage::Japanese)
                .unwrap()
                .locale,
            "ja_JP"
        );
    }
}
