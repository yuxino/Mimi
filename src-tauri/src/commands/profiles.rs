//! Profile catalog and credential editor commands. Secrets never enter broadcasts.

use super::app::app_is_ui_test;
use super::apple_resources::ensure_apple_provider_available;
use super::settings::{emit_settings_snapshot, SettingsSnapshotPayload};
use super::AppState;
use crate::core::models::SourceLanguage;
use crate::core::services::credentials::{CredentialRevealField, ProviderCredentials};
use crate::core::services::network_proxy::ProxyConfig;
use crate::core::services::provider::{
    CustomSpeechLanguagesPatch, ProviderKind, ServiceProfile, TextTranslation, TextTranslationName,
};
use crate::settings_store::{CredentialState, SettingsStore};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProfilePayload {
    pub id: String,
    pub name: String,
    pub provider: ProviderKind,
    pub qwen_mt_model: crate::core::protocols::translation::qwen_mt::QwenMTModel,
    pub alibaba_realtime_mode: crate::core::services::provider::AlibabaRealtimeMode,
    pub alibaba_workspace_id: String,
    pub xai_speech_mode: crate::core::services::provider::XAISpeechMode,
    pub speech_translation_mode: crate::core::services::provider::SpeechTranslationMode,
    pub xai_text_model: String,
    pub language_preset: Option<crate::core::services::provider::ProfileLanguagePreset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_recognition_name: Option<String>,
    pub credential_state: CredentialState,
    pub credential_storage: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_credential_state: Option<CredentialState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_credential_state: Option<CredentialState>,
    pub text_translation: crate::core::services::provider::TextTranslation,
    pub text_translation_names: std::collections::BTreeMap<TextTranslation, String>,
    pub speech_network_proxy: Option<ProxyConfig>,
    pub text_network_proxy: Option<ProxyConfig>,
    pub custom_speech_source_languages: Option<Vec<SourceLanguage>>,
}

impl ServiceProfilePayload {
    pub(super) fn from_profile(store: &SettingsStore, profile: ServiceProfile) -> Self {
        let states = store.custom_credential_states_for_snapshot(&profile);
        let credential_state = states.map_or_else(
            || store.credential_state_for_snapshot(&profile),
            |(speech, text)| speech.combined(text),
        );
        let text_translation = profile.text_translation();
        let credential_storage = store.credential_storage();
        Self {
            speech_network_proxy: profile.speech_network_proxy,
            text_network_proxy: profile.text_network_proxy,
            custom_speech_source_languages: profile.custom_speech_source_languages,
            id: profile.id,
            name: profile.name,
            provider: profile.provider,
            qwen_mt_model: profile.qwen_mt_model,
            alibaba_realtime_mode: profile.alibaba_realtime_mode,
            alibaba_workspace_id: profile.alibaba_workspace_id,
            xai_speech_mode: profile.xai_speech_mode,
            speech_translation_mode: profile.speech_translation_mode,
            xai_text_model: profile.xai_text_model,
            language_preset: profile.language_preset,
            speech_recognition_name: profile.speech_recognition_name,
            credential_state,
            credential_storage,
            speech_credential_state: states.map(|(speech, _)| speech),
            text_credential_state: states.map(|(_, text)| text),
            text_translation,
            text_translation_names: profile.text_translation_names,
        }
    }

    #[cfg(test)]
    fn unavailable(profile: ServiceProfile) -> Self {
        let text_translation = profile.text_translation();
        let standalone = profile.is_standalone_asr();
        Self {
            speech_network_proxy: profile.speech_network_proxy,
            text_network_proxy: profile.text_network_proxy,
            custom_speech_source_languages: profile.custom_speech_source_languages,
            id: profile.id,
            name: profile.name,
            provider: profile.provider,
            qwen_mt_model: profile.qwen_mt_model,
            alibaba_realtime_mode: profile.alibaba_realtime_mode,
            alibaba_workspace_id: profile.alibaba_workspace_id,
            xai_speech_mode: profile.xai_speech_mode,
            speech_translation_mode: profile.speech_translation_mode,
            xai_text_model: profile.xai_text_model,
            language_preset: profile.language_preset,
            speech_recognition_name: profile.speech_recognition_name,
            credential_state: CredentialState::Unavailable,
            credential_storage: "keychain",
            speech_credential_state: standalone.then_some(CredentialState::Unavailable),
            text_credential_state: standalone.then_some(CredentialState::Unavailable),
            text_translation,
            text_translation_names: profile.text_translation_names,
        }
    }
}

pub(super) fn ensure_profile_mutation_allowed(is_active: bool) -> Result<(), String> {
    if is_active {
        Err("Service profiles cannot be changed while a session is active.".to_string())
    } else {
        Ok(())
    }
}

#[tauri::command]
pub async fn profile_create(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: ProviderKind,
    name: String,
) -> Result<SettingsSnapshotPayload, String> {
    let _lifecycle = state.session.settings_mutation_guard(true).await?;
    ensure_profile_mutation_allowed(state.session.has_active_session())?;
    ensure_apple_provider_available(provider)
        .await
        .inspect_err(|_| {
            let _ = emit_settings_snapshot(&app, &state.settings);
        })?;
    state.settings.create_profile_for_setup(provider, &name)?;
    emit_settings_snapshot(&app, &state.settings)
}

#[tauri::command]
pub async fn profile_duplicate(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    name: String,
) -> Result<SettingsSnapshotPayload, String> {
    let _lifecycle = state.session.settings_mutation_guard(true).await?;
    ensure_profile_mutation_allowed(state.session.has_active_session())?;
    state.settings.duplicate_profile(&profile_id, &name)?;
    emit_settings_snapshot(&app, &state.settings)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Preserve the existing optional IPC patch fields.
pub async fn profile_update(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    name: Option<String>,
    speech_network_proxy: Option<ProxyConfig>,
    text_network_proxy: Option<ProxyConfig>,
    text_translation_name: Option<TextTranslationName>,
    speech_recognition_name: Option<String>,
    custom_speech_languages_patch: Option<CustomSpeechLanguagesPatch>,
    language_preset_patch: Option<crate::core::services::provider::ProfileLanguagePresetPatch>,
    qwen_mt_model: Option<crate::core::protocols::translation::qwen_mt::QwenMTModel>,
    alibaba_realtime_mode: Option<crate::core::services::provider::AlibabaRealtimeMode>,
    alibaba_workspace_id: Option<String>,
    xai_speech_mode: Option<crate::core::services::provider::XAISpeechMode>,
    xai_text_model: Option<String>,
    speech_translation_mode: Option<crate::core::services::provider::SpeechTranslationMode>,
) -> Result<SettingsSnapshotPayload, String> {
    let _lifecycle = state.session.settings_mutation_guard(true).await?;
    ensure_profile_mutation_allowed(state.session.has_active_session())?;
    let previous_profile = state.settings.active_profile()?;
    let changes_active_speech_route = previous_profile.id == profile_id
        && (speech_network_proxy
            .as_ref()
            .is_some_and(|proxy| previous_profile.speech_network_proxy.as_ref() != Some(proxy))
            || alibaba_realtime_mode
                .is_some_and(|mode| previous_profile.alibaba_realtime_mode != mode)
            || alibaba_workspace_id.as_ref().is_some_and(|workspace| {
                previous_profile.alibaba_workspace_id != workspace.trim()
            })
            || xai_speech_mode.is_some_and(|mode| previous_profile.xai_speech_mode != mode)
            || speech_translation_mode
                .is_some_and(|mode| previous_profile.speech_translation_mode != mode));
    let configuration_failure = state.session.configuration_failure_snapshot();
    state.settings.update_profile_options_with_speech_mode(
        &profile_id,
        name.as_deref(),
        speech_network_proxy,
        text_network_proxy,
        text_translation_name,
        speech_recognition_name.as_deref(),
        custom_speech_languages_patch,
        language_preset_patch,
        qwen_mt_model,
        alibaba_realtime_mode,
        alibaba_workspace_id.as_deref(),
        xai_speech_mode,
        xai_text_model.as_deref(),
        speech_translation_mode,
    )?;
    state
        .session
        .configuration_saved(configuration_failure, changes_active_speech_route);
    emit_settings_snapshot(&app, &state.settings)
}

#[tauri::command]
pub async fn profile_select(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    source_language: Option<SourceLanguage>,
) -> Result<SettingsSnapshotPayload, String> {
    match source_language {
        Some(source) => {
            state
                .session
                .switch_profile_with_source(&profile_id, Some(source))
                .await?
        }
        None => state.session.switch_profile(&profile_id).await?,
    }
    emit_settings_snapshot(&app, &state.settings)
}

#[tauri::command]
pub async fn profile_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<SettingsSnapshotPayload, String> {
    let _lifecycle = state.session.settings_mutation_guard(true).await?;
    ensure_profile_mutation_allowed(state.session.has_active_session())?;
    let was_active = state.settings.active_profile()?.id == profile_id;
    let configuration_failure = state.session.configuration_failure_snapshot();
    state.settings.delete_profile(&profile_id)?;
    state
        .session
        .configuration_saved(configuration_failure, was_active);
    emit_settings_snapshot(&app, &state.settings)
}

#[tauri::command]
pub async fn profile_save_credentials(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    credentials: ProviderCredentials,
) -> Result<SettingsSnapshotPayload, String> {
    let _lifecycle = state.session.settings_mutation_guard(true).await?;
    ensure_profile_mutation_allowed(state.session.has_active_session())?;
    let mut profile = state.settings.active_profile()?;
    let is_active_profile = profile.id == profile_id;
    let configuration_failure = state.session.configuration_failure_snapshot();
    if profile.provider == ProviderKind::AppleSpeech && is_active_profile {
        if let ProviderCredentials::AlibabaTranslation {
            text_translation, ..
        } = &credentials
        {
            let prefs = state.settings.preferences();
            profile.text_translation = Some(*text_translation);
            if !profile
                .capabilities(prefs.target_language)
                .source_languages
                .contains(&prefs.source_language)
            {
                return Err("apple_speech_translation_language_unsupported".into());
            }
        }
    }
    if matches!(
        &credentials,
        ProviderCredentials::AlibabaTranslation {
            text_translation: TextTranslation::Apple,
            ..
        }
    ) && (app_is_ui_test() || !crate::apple_translation_support::refresh().await?.available)
    {
        return Err("apple_translation_unavailable".into());
    }
    state.settings.save_credentials(&profile_id, &credentials)?;
    // An explicit successful service save is a repair action; do not read
    // credentials a second time merely to compare private values.
    state
        .session
        .configuration_saved(configuration_failure, is_active_profile);
    emit_settings_snapshot(&app, &state.settings)
}

#[tauri::command]
pub async fn profile_delete_api_key(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<SettingsSnapshotPayload, String> {
    let _lifecycle = state.session.settings_mutation_guard(true).await?;
    ensure_profile_mutation_allowed(state.session.has_active_session())?;
    state.settings.delete_api_key(&profile_id)?;
    emit_settings_snapshot(&app, &state.settings)
}

fn ensure_credential_reveal_window(label: &str) -> Result<(), String> {
    if label == "settings" {
        Ok(())
    } else {
        Err("credential_reveal_not_allowed".into())
    }
}

/// Returns private configuration only to the requesting settings editor.
/// Saved API-key flags do not include credential bytes or emit an event.
#[tauri::command]
pub async fn profile_credential_editor_state(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    profile_id: String,
    text_translation: Option<crate::core::services::provider::TextTranslation>,
) -> Result<crate::settings_store::CredentialEditorState, String> {
    ensure_credential_reveal_window(window.label())?;
    let settings = Arc::clone(&state.settings);
    tauri::async_runtime::spawn_blocking(move || {
        settings.credential_editor_state(&profile_id, text_translation)
    })
    .await
    .map_err(|_| "credential_store_unavailable".to_string())?
}

/// Returns one explicitly requested secret only to this invoke's requester.
/// Credential-file access runs off the IPC/main thread; no event is emitted.
#[tauri::command]
pub async fn profile_reveal_credential(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    profile_id: String,
    field: CredentialRevealField,
    text_translation: Option<crate::core::services::provider::TextTranslation>,
) -> Result<Option<String>, String> {
    ensure_credential_reveal_window(window.label())?;
    let settings = Arc::clone(&state.settings);
    tauri::async_runtime::spawn_blocking(move || {
        settings.reveal_credential(&profile_id, field, text_translation)
    })
    .await
    .map_err(|_| "credential_store_unavailable".to_string())?
}

#[cfg(test)]
mod tests;
