//! Fixed help/release destinations and content-free support diagnostics.

use super::AppState;
use serde::Deserialize;
use std::sync::LazyLock;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

const RELEASES_LATEST_URL: &str = "https://github.com/yuxino/mimi/releases/latest";

/// Opens a single hard-coded release destination. The frontend cannot supply
/// or widen the URL, and no generic opener permission is exposed to WebViews.
#[tauri::command]
pub fn app_open_releases(app: AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(RELEASES_LATEST_URL, None::<&str>)
        .map_err(|_| {
            tracing::warn!(label = "system_opener_failed", "release page open failed");
            "Could not open the release page.".to_string()
        })
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlibabaSetupPage {
    Guide,
    ApiKey,
    Workspace,
}

impl AlibabaSetupPage {
    fn url(self) -> &'static str {
        match self {
            Self::Guide => {
                "https://help.aliyun.com/zh/model-studio/qwen3-5-livetranslate-flash-realtime"
            }
            Self::ApiKey => "https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key",
            Self::Workspace => "https://bailian.console.aliyun.com/cn-beijing/settings/workspace",
        }
    }
}

/// Only explicit user actions open one of the fixed official setup pages.
#[tauri::command]
pub fn open_alibaba_setup_page(app: AppHandle, page: AlibabaSetupPage) -> Result<(), String> {
    app.opener()
        .open_url(page.url(), None::<&str>)
        .map_err(|_| {
            tracing::warn!(
                label = "system_opener_failed",
                "Alibaba setup page open failed"
            );
            "Could not open the Alibaba Cloud setup page.".to_string()
        })
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextTranslationSetupPage {
    ApiKey,
    Guide,
}

fn text_translation_setup_url(
    preset_id: &str,
    page: TextTranslationSetupPage,
) -> Option<&'static str> {
    catalog_setup_url("compatiblePresets", preset_id, page)
}

fn catalog_setup_url(
    catalog: &str,
    service_id: &str,
    page: TextTranslationSetupPage,
) -> Option<&'static str> {
    static CATALOG: LazyLock<Option<serde_json::Value>> = LazyLock::new(|| {
        serde_json::from_str(include_str!("../../../shared/provider-copy.json")).ok()
    });
    let field = match page {
        TextTranslationSetupPage::ApiKey => "apiKeyUrl",
        TextTranslationSetupPage::Guide => "guideUrl",
    };
    let preset = CATALOG
        .as_ref()?
        .get(catalog)?
        .as_array()?
        .iter()
        .find(|preset| preset.get("id").and_then(serde_json::Value::as_str) == Some(service_id))?;
    if matches!(page, TextTranslationSetupPage::ApiKey)
        && catalog == "compatiblePresets"
        && preset.get("setupKind").and_then(serde_json::Value::as_str) != Some("cloud")
    {
        return None;
    }
    let value = preset.get(field)?.as_str()?;
    let url = url::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && !value.chars().any(char::is_control))
    .then_some(value)
}

/// Built-in speech and dedicated text services use the same fixed catalog on both platforms.
#[tauri::command]
pub fn open_service_setup_page(
    app: AppHandle,
    service_id: String,
    page: TextTranslationSetupPage,
) -> Result<(), String> {
    let url = catalog_setup_url("serviceSetups", &service_id, page)
        .ok_or_else(|| "service_setup_page_unavailable".to_string())?;
    app.opener().open_url(url, None::<&str>).map_err(|_| {
        tracing::warn!(
            label = "system_opener_failed",
            "service setup page open failed"
        );
        "service_setup_open_failed".to_string()
    })
}

/// Settings supplies only a catalog ID and page kind, never an editable URL.
#[tauri::command]
pub fn open_text_translation_setup_page(
    app: AppHandle,
    preset_id: String,
    page: TextTranslationSetupPage,
) -> Result<(), String> {
    let url = text_translation_setup_url(&preset_id, page)
        .ok_or_else(|| "text_translation_setup_page_unavailable".to_string())?;
    app.opener().open_url(url, None::<&str>).map_err(|_| {
        tracing::warn!(
            label = "system_opener_failed",
            "text translation setup page open failed"
        );
        "text_translation_setup_open_failed".to_string()
    })
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TencentSetupPage {
    Account,
    ApiKey,
    Asr,
}

impl TencentSetupPage {
    fn url(self) -> &'static str {
        match self {
            Self::Account => "https://console.cloud.tencent.com/developer",
            Self::ApiKey => "https://console.cloud.tencent.com/cam/capi",
            Self::Asr => "https://console.cloud.tencent.com/asr",
        }
    }
}

/// Explicit navigation only: one fixed privacy page, no grant mutation.
#[tauri::command]
pub fn app_open_audio_privacy_settings(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        app.opener()
            .open_url(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
                None::<&str>,
            )
            .map_err(|_| "audio_privacy_settings_open_failed".to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("audio_privacy_settings_unsupported".into())
    }
}

/// Settings can open these public setup pages without supplying an arbitrary URL.
#[tauri::command]
pub fn open_tencent_setup_page(app: AppHandle, page: TencentSetupPage) -> Result<(), String> {
    app.opener()
        .open_url(page.url(), None::<&str>)
        .map_err(|_| {
            tracing::warn!(
                label = "system_opener_failed",
                "Tencent setup page open failed"
            );
            "Could not open the Tencent Cloud setup page.".to_string()
        })
}

#[tauri::command]
pub async fn support_diagnostics(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.session.support_diagnostics())
}

#[tauri::command]
pub async fn app_open_support_issue(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::core::support_diagnostics::SupportIssue, String> {
    let issue = state.session.support_issue();
    app.opener()
        .open_url(&issue.url, None::<&str>)
        .map_err(|_| "support_issue_open_failed".to_string())?;
    Ok(issue)
}

#[cfg(test)]
mod tests;
