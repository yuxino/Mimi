//! Explicit native speech and translation resource operations.

use super::app::app_is_ui_test;
use super::profiles::ensure_profile_mutation_allowed;
use super::settings::emit_settings_snapshot;
use super::AppState;
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::provider::{ProviderKind, TextTranslation};
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn get_apple_speech_support(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::apple_speech_support::AppleSpeechSupport, String> {
    if app_is_ui_test() {
        return Ok(Default::default());
    }
    let result = crate::apple_speech_support::refresh().await;
    emit_settings_snapshot(&app, &state.settings)?;
    result
}

#[tauri::command]
pub async fn prepare_apple_speech_language(
    app: AppHandle,
    state: State<'_, AppState>,
    source_language: SourceLanguage,
) -> Result<crate::apple_speech_support::AppleSpeechSupport, String> {
    if app_is_ui_test() {
        return Err("apple_speech_ui_test_unavailable".into());
    }
    {
        let _lifecycle = state.session.settings_mutation_guard(true).await?;
        ensure_profile_mutation_allowed(state.session.has_active_session())?;
    }
    // The explicit settings action is the only path allowed to request assets.
    let result = crate::apple_speech_support::prepare_source(source_language).await;
    emit_settings_snapshot(&app, &state.settings)?;
    result
}

#[tauri::command]
pub async fn get_apple_translation_support(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::apple_translation_support::AppleTranslationSupport, String> {
    if app_is_ui_test() {
        return Ok(Default::default());
    }
    let result = crate::apple_translation_support::refresh().await;
    emit_settings_snapshot(&app, &state.settings)?;
    result
}

#[tauri::command]
pub async fn get_apple_translation_status(
    source_language: SourceLanguage,
    target_language: TargetLanguage,
) -> Result<crate::apple_translation::AppleTranslationStatus, String> {
    if app_is_ui_test() {
        return Ok(crate::apple_translation::AppleTranslationStatus::Unavailable);
    }
    crate::apple_translation_support::status(source_language, target_language).await
}

#[tauri::command]
pub async fn prepare_apple_translation_languages(
    app: AppHandle,
    state: State<'_, AppState>,
    source_language: SourceLanguage,
    target_language: TargetLanguage,
    ui_language: String,
) -> Result<crate::apple_translation::AppleTranslationStatus, String> {
    if app_is_ui_test() {
        return Err("apple_translation_unavailable".into());
    }
    let _lifecycle = state.session.settings_mutation_guard(true).await?;
    ensure_profile_mutation_allowed(state.session.has_active_session())?;
    let prefs = state.settings.preferences();
    let repairs_current_pair = state
        .settings
        .active_profile()
        .is_ok_and(|profile| profile.text_translation() == TextTranslation::Apple)
        && prefs.source_language == source_language
        && prefs.target_language == target_language;
    let configuration_failure = repairs_current_pair
        .then(|| state.session.configuration_failure_snapshot())
        .flatten();
    let result =
        crate::apple_translation_support::prepare(source_language, target_language, &ui_language)
            .await;
    let _ = crate::apple_translation_support::refresh().await;
    if result.is_ok() {
        state
            .session
            .configuration_saved(configuration_failure, true);
    }
    emit_settings_snapshot(&app, &state.settings)?;
    result.map(|()| crate::apple_translation::AppleTranslationStatus::Installed)
}

pub(super) async fn ensure_apple_provider_available(provider: ProviderKind) -> Result<(), String> {
    if provider == ProviderKind::AppleSpeech
        && (app_is_ui_test() || !crate::apple_speech_support::refresh().await?.available)
    {
        return Err("apple_speech_unavailable".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
