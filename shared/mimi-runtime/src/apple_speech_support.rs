//! Runtime Apple capabilities mapped to Mimi's existing language choices.
//! Queries are read-only; only `prepare_source` can request language assets.

use crate::apple_speech::{
    self, AppleSpeechCapabilities, AppleSpeechError, AppleSpeechResourceStatus,
};
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::services::provider::ServiceProfile;
use serde::Serialize;
use std::future::Future;
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
    // Query completion is separate from the published resource revision. A
    // preparation may restore one locale after a failed full inventory scan.
    refresh_attempt: u64,
    last_refresh: Option<Result<(), String>>,
}

impl SupportCache {
    fn is_loaded(&self) -> bool {
        self.last_refresh.as_ref().is_some_and(Result::is_ok)
    }

    fn completed_since(&self, attempt: u64) -> Option<Result<AppleSpeechSupport, String>> {
        if self.is_loaded() || self.refresh_attempt != attempt {
            self.last_refresh
                .as_ref()
                .map(|result| result.clone().map(|()| self.snapshot().0))
        } else {
            None
        }
    }

    fn record_refresh(&mut self, support: &Result<AppleSpeechSupport, String>) {
        self.replace(support.clone().unwrap_or_default());
        self.refresh_attempt = self.refresh_attempt.wrapping_add(1);
        self.last_refresh = Some(support.as_ref().map(|_| ()).map_err(Clone::clone));
    }

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
        .is_loaded()
}

pub async fn refresh() -> Result<AppleSpeechSupport, String> {
    load(false).await
}

/// Retry failed inventory reads on a later snapshot request, while sharing a
/// read that completed after this caller started waiting for the refresh lock.
pub async fn ensure_loaded() -> Result<AppleSpeechSupport, String> {
    load(true).await
}

async fn load(reuse_completed: bool) -> Result<AppleSpeechSupport, String> {
    load_support(
        SUPPORT.get_or_init(Default::default),
        &REFRESHING,
        reuse_completed,
        async {
            apple_speech::capabilities()
                .await
                .map(map_capabilities)
                .map_err(|_| "apple_speech_status_failed".to_string())
        },
    )
    .await
}

async fn load_support(
    cache: &Mutex<SupportCache>,
    refreshing: &tokio::sync::Mutex<()>,
    reuse_completed: bool,
    query: impl Future<Output = Result<AppleSpeechSupport, String>>,
) -> Result<AppleSpeechSupport, String> {
    // Remember before waiting so callers of the same failed scan share its
    // error, while a new snapshot request after failure can try once again.
    let attempt = cache.lock().unwrap().refresh_attempt;
    let _refreshing = refreshing.lock().await;
    if reuse_completed {
        if let Some(result) = cache.lock().unwrap().completed_since(attempt) {
            return result;
        }
    }
    let support = query.await;
    // A failed refresh must not keep a stale available/installed claim alive.
    cache.lock().unwrap().record_refresh(&support);
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
mod tests;
