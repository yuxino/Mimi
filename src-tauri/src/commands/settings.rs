//! Non-secret settings snapshots, bootstrap and guarded preference updates.

use super::app::app_is_ui_test;
use super::profiles::ServiceProfilePayload;
use super::session::ensure_audio_input_available;
use super::{sync_overlay_minimum, AppState};
use crate::core::audio::audio_input::AudioInput;
use crate::core::models::{
    SourceLanguage, SubtitleColor, SubtitleDisplayMode, TargetLanguage, TranslationMode,
};
use crate::core::services::network_proxy::ProxyConfig;
use crate::core::services::provider::{ProviderKind, ServiceProfile, TextTranslation};
use crate::settings_store::{PulseStyle, SettingsStore, SubtitleAlignment};
use crate::windows::OverlayWindowManager;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageCapabilitiesPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qwen_mt_model: Option<crate::core::protocols::translation::qwen_mt::QwenMTModel>,
    pub xai_speech_mode: Option<crate::core::services::provider::XAISpeechMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_translation_mode: Option<crate::core::services::provider::SpeechTranslationMode>,
    pub profile_id: String,
    pub provider: ProviderKind,
    pub text_translation: TextTranslation,
    pub target_language: TargetLanguage,
    pub source_languages: Vec<SourceLanguage>,
    pub target_languages: Vec<TargetLanguage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apple_speech_support_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_speech_source_languages: Option<Option<Vec<SourceLanguage>>>,
}

impl LanguageCapabilitiesPayload {
    fn for_active_profile(
        profiles: &[ServiceProfile],
        active_profile_id: &str,
        target_language: TargetLanguage,
    ) -> Option<Self> {
        profiles
            .iter()
            .find(|profile| profile.id == active_profile_id)
            .map(|profile| Self::from_profile(profile, target_language))
    }

    pub(super) fn from_profile(profile: &ServiceProfile, target: TargetLanguage) -> Self {
        let support = (profile.provider == ProviderKind::AppleSpeech)
            .then(crate::apple_speech_support::cached_with_revision);
        Self::from_profile_with_apple_support(
            profile,
            target,
            support
                .as_ref()
                .map(|(support, revision)| (support, *revision)),
        )
    }

    fn from_profile_with_apple_support(
        profile: &ServiceProfile,
        target: TargetLanguage,
        support: Option<(&crate::apple_speech_support::AppleSpeechSupport, u64)>,
    ) -> Self {
        let mut capabilities = profile.capabilities(target);
        if profile.provider == ProviderKind::AppleSpeech {
            // Recognition pickers offer only sources that can start now. The
            // resource editor keeps the complete supported/downloadable list.
            // Profile capabilities already enforce the text route intersection.
            capabilities.source_languages.retain(|source| {
                support.is_some_and(|(support, _)| {
                    crate::apple_speech_support::validate_source(support, *source, true).is_ok()
                })
            });
        }
        if profile.text_translation() == TextTranslation::Apple {
            let text_support = crate::apple_translation_support::cached();
            capabilities.target_languages.retain(|target| {
                *target == TargetLanguage::Original
                    || text_support.target_languages.contains(target)
            });
            if target.translates_audio() {
                capabilities
                    .source_languages
                    .retain(|source| text_support.source_languages.contains(source));
            }
        }
        Self {
            qwen_mt_model: (profile.provider == ProviderKind::AlibabaCloud)
                .then_some(profile.qwen_mt_model),
            xai_speech_mode: (profile.provider == ProviderKind::XAIRealtime)
                .then_some(profile.xai_speech_mode),
            speech_translation_mode: profile
                .provider
                .supports_speech_translation_mode()
                .then_some(profile.speech_translation_mode),
            profile_id: profile.id.clone(),
            provider: profile.provider,
            text_translation: profile.text_translation(),
            target_language: target,
            source_languages: capabilities.source_languages,
            target_languages: capabilities.target_languages,
            apple_speech_support_revision: (profile.provider == ProviderKind::AppleSpeech)
                .then(|| support.map_or(0, |(_, revision)| revision)),
            custom_speech_source_languages: profile
                .provider
                .is_custom_speech()
                .then(|| profile.custom_speech_source_languages.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshotPayload {
    pub profiles: Vec<ServiceProfilePayload>,
    pub credential_storage: &'static str,
    pub active_profile_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_capabilities: Option<LanguageCapabilitiesPayload>,
    pub source_language: SourceLanguage,
    pub target_language: TargetLanguage,
    pub translation_mode: TranslationMode,
    pub font_size: f64,
    pub subtitle_font_family: String,
    pub subtitle_background_opacity: u8,
    pub subtitle_color: SubtitleColor,
    pub microphone_subtitle_color: SubtitleColor,
    pub subtitle_alignment: SubtitleAlignment,
    pub subtitle_display_mode: SubtitleDisplayMode,
    pub show_intermediate_subtitles: bool,
    pub show_subtitle_dividers: bool,
    pub show_subtitle_timestamps: bool,
    pub custom_subtitle_count_enabled: bool,
    /// 1..=10; applies only while the custom count is enabled.
    pub visible_subtitle_count: u8,
    /// `None` follows the operating system's reduce-motion setting.
    pub pulse_animation: Option<bool>,
    pub pulse_style: PulseStyle,
    pub subtitle_animation: Option<bool>,
    pub subtitle_blends_with_background: bool,
    #[serde(rename = "isOverlayLocked")]
    pub is_overlay_locked: bool,
    #[serde(rename = "uiLanguage")]
    pub ui_language: Option<String>,
    pub retain_session_history: bool,
    pub record_session_audio: bool,
    pub audio_input: AudioInput,
    pub microphone_input_available: bool,
    pub windows_audio_source: String,
    pub system_audio_target: crate::core::audio::system_audio_target::SystemAudioTarget,
    pub show_in_dock: bool,
    pub network_proxy: ProxyConfig,
}

impl SettingsSnapshotPayload {
    pub fn try_from_store(store: &SettingsStore) -> Result<Self, String> {
        let (prefs, active_profile_id, profiles) = store.preferences_and_catalog()?;
        let language_capabilities = LanguageCapabilitiesPayload::for_active_profile(
            &profiles,
            &active_profile_id,
            prefs.target_language,
        );
        Ok(Self {
            credential_storage: store.credential_storage(),
            profiles: profiles
                .into_iter()
                .map(|profile| ServiceProfilePayload::from_profile(store, profile))
                .collect(),
            active_profile_id,
            language_capabilities,
            source_language: prefs.source_language,
            target_language: prefs.target_language,
            translation_mode: prefs.translation_mode,
            font_size: prefs.font_size,
            subtitle_font_family: prefs.subtitle_font_family,
            subtitle_background_opacity: prefs.subtitle_background_opacity,
            subtitle_color: prefs.subtitle_color,
            microphone_subtitle_color: prefs.microphone_subtitle_color,
            subtitle_alignment: prefs.subtitle_alignment,
            subtitle_display_mode: prefs.subtitle_display_mode,
            show_intermediate_subtitles: prefs.show_intermediate_subtitles,
            show_subtitle_dividers: prefs.show_subtitle_dividers,
            show_subtitle_timestamps: prefs.show_subtitle_timestamps,
            custom_subtitle_count_enabled: prefs.custom_subtitle_count_enabled,
            visible_subtitle_count: prefs.visible_subtitle_count,

            pulse_animation: prefs.pulse_animation,
            pulse_style: prefs.pulse_style,

            subtitle_animation: prefs.subtitle_animation,
            subtitle_blends_with_background: prefs.subtitle_blends_with_background,
            is_overlay_locked: prefs.overlay_locked,
            ui_language: prefs.ui_language,
            retain_session_history: prefs.retain_session_history,
            record_session_audio: prefs.record_session_audio,
            audio_input: prefs.audio_input,
            microphone_input_available: crate::core::audio::audio_input::MICROPHONE_INPUT_AVAILABLE,
            windows_audio_source: prefs.windows_audio_source,
            system_audio_target: prefs.system_audio_target,
            show_in_dock: prefs.show_in_dock,
            network_proxy: prefs.network_proxy,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsDraft {
    pub source_language: Option<SourceLanguage>,
    pub target_language: Option<TargetLanguage>,
    pub translation_mode: Option<TranslationMode>,
    pub font_size: Option<f64>,
    pub subtitle_font_family: Option<String>,
    pub subtitle_background_opacity: Option<u8>,
    pub subtitle_color: Option<SubtitleColor>,
    pub microphone_subtitle_color: Option<SubtitleColor>,
    pub subtitle_alignment: Option<SubtitleAlignment>,
    pub subtitle_display_mode: Option<SubtitleDisplayMode>,
    pub show_intermediate_subtitles: Option<bool>,
    pub show_subtitle_dividers: Option<bool>,
    pub show_subtitle_timestamps: Option<bool>,
    pub custom_subtitle_count_enabled: Option<bool>,
    pub visible_subtitle_count: Option<u8>,
    pub pulse_animation: Option<bool>,
    pub pulse_style: Option<PulseStyle>,
    pub subtitle_animation: Option<bool>,
    pub subtitle_blends_with_background: Option<bool>,
    pub is_overlay_locked: Option<bool>,
    pub ui_language: Option<String>,
    pub retain_session_history: Option<bool>,
    pub record_session_audio: Option<bool>,
    pub audio_input: Option<AudioInput>,
    pub windows_audio_source: Option<String>,
    pub system_audio_target: Option<crate::core::audio::system_audio_target::SystemAudioTarget>,
    pub show_in_dock: Option<bool>,
    pub network_proxy: Option<ProxyConfig>,
}

/// Reads public settings and per-profile credential presence. API-key values
/// never enter this payload. Async keeps credential-file reads off the
/// WebView's main thread.
#[tauri::command]
pub async fn settings_get(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SettingsSnapshotPayload, String> {
    get_settings_snapshot(&state.settings, app_is_ui_test(), |payload| {
        let _ = app.emit("settings-changed", payload);
    })
    .await
}

async fn get_settings_snapshot(
    settings: &SettingsStore,
    is_ui_test: bool,
    publish: impl FnOnce(&SettingsSnapshotPayload),
) -> Result<SettingsSnapshotPayload, String> {
    // Only language capabilities are compared after the resource load.
    // Project credential states once, as part of the final snapshot.
    let previous = {
        let (prefs, active_profile_id, profiles) = settings.preferences_and_catalog()?;
        LanguageCapabilitiesPayload::for_active_profile(
            &profiles,
            &active_profile_id,
            prefs.target_language,
        )
    };
    load_and_publish_settings_snapshot(
        previous,
        load_settings_snapshot(settings, is_ui_test),
        publish,
    )
    .await
}

async fn load_and_publish_settings_snapshot(
    previous: Option<LanguageCapabilitiesPayload>,
    load: impl std::future::Future<Output = Result<SettingsSnapshotPayload, String>>,
    publish: impl FnOnce(&SettingsSnapshotPayload),
) -> Result<SettingsSnapshotPayload, String> {
    let payload = load.await?;
    // Another window may have initialized while Apple's inventory was not
    // readable. Publish recovered capabilities to it without making equivalent
    // bootstrap reads trigger the resource editor's refresh cycle again.
    if payload.language_capabilities != previous {
        publish(&payload);
    }
    Ok(payload)
}

async fn load_settings_snapshot(
    settings: &SettingsStore,
    is_ui_test: bool,
) -> Result<SettingsSnapshotPayload, String> {
    if !is_ui_test
        && !crate::apple_speech_support::is_loaded()
        && settings
            .active_profile()
            .is_ok_and(|profile| profile.provider == ProviderKind::AppleSpeech)
    {
        // Query runtime support once for an already selected local recognizer.
        // Failure leaves an empty language list and is surfaced by its editor.
        let _ = crate::apple_speech_support::ensure_loaded().await;
    }
    if !is_ui_test
        && !crate::apple_translation_support::is_loaded()
        && settings
            .active_profile()
            .is_ok_and(|profile| profile.text_translation() == TextTranslation::Apple)
    {
        let _ = crate::apple_translation_support::ensure_loaded().await;
    }
    SettingsSnapshotPayload::try_from_store(settings)
}

fn ensure_settings_draft_window_allowed(label: &str, draft: &SettingsDraft) -> Result<(), String> {
    if (draft.retain_session_history.is_some()
        || draft.record_session_audio.is_some()
        || draft.audio_input.is_some()
        || draft.windows_audio_source.is_some()
        || draft.system_audio_target.is_some()
        || draft.network_proxy.is_some())
        && label != "settings"
    {
        return Err("These preferences can only be changed in settings.".into());
    }
    if draft.show_in_dock.is_some() && label != "settings" {
        return Err("The Dock preference can only be changed in settings.".into());
    }
    Ok(())
}

/// Saves non-secret preferences only. Credentials use dedicated write-only
/// commands so a general settings draft can never echo or overwrite a key.
#[tauri::command]
pub async fn settings_save(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    draft: SettingsDraft,
) -> Result<SettingsSnapshotPayload, String> {
    ensure_settings_draft_window_allowed(window.label(), &draft)?;
    apply_settings_draft(&app, &state, draft).await
}

async fn apply_settings_draft(
    app: &AppHandle,
    state: &AppState,
    draft: SettingsDraft,
) -> Result<SettingsSnapshotPayload, String> {
    let changes_listening_settings = draft.source_language.is_some()
        || draft.target_language.is_some()
        || draft.translation_mode.is_some()
        || draft.retain_session_history.is_some()
        || draft.record_session_audio.is_some()
        || draft.audio_input.is_some()
        || draft.windows_audio_source.is_some()
        || draft.system_audio_target.is_some()
        || draft.network_proxy.is_some();
    let _lifecycle = state
        .session
        .settings_mutation_guard(changes_listening_settings)
        .await
        .map_err(|error| {
            if draft.network_proxy.is_some() {
                "network_proxy_change_requires_stop".to_string()
            } else {
                error
            }
        })?;
    if draft.source_language.is_some() || draft.target_language.is_some() {
        let profile = state.settings.active_profile()?;
        if profile.text_translation() == TextTranslation::Apple {
            let prefs = state.settings.preferences();
            if app_is_ui_test()
                && draft
                    .target_language
                    .unwrap_or(prefs.target_language)
                    .translates_audio()
            {
                return Err("apple_translation_unavailable".into());
            }
            crate::apple_translation_support::validate_pair(
                draft.source_language.unwrap_or(prefs.source_language),
                draft.target_language.unwrap_or(prefs.target_language),
                false,
            )
            .await?;
        }
        if profile.provider == ProviderKind::AppleSpeech {
            if app_is_ui_test() {
                return Err("apple_speech_ui_test_unavailable".into());
            }
            let prefs = state.settings.preferences();
            crate::apple_speech_support::validate_refreshed_profile_source(
                crate::apple_speech_support::refresh().await,
                &profile,
                draft.source_language.unwrap_or(prefs.source_language),
                draft.target_language.unwrap_or(prefs.target_language),
                false,
                || {
                    let _ = emit_settings_snapshot(app, &state.settings);
                },
            )?;
        }
    }
    apply_settings_draft_guarded(app, state, draft)
}

/// Applies a settings draft while the caller owns the settings mutation
/// guard. Keeping the read and write inside one guard makes native toggles
/// atomic with ordinary settings saves.
fn apply_settings_draft_guarded(
    app: &AppHandle,
    state: &AppState,
    draft: SettingsDraft,
) -> Result<SettingsSnapshotPayload, String> {
    if let Some(input) = draft.audio_input {
        ensure_audio_input_available(input)?;
    }
    #[cfg(not(target_os = "macos"))]
    if draft.show_in_dock.is_some() {
        return Err("dock-preference-unsupported".into());
    }
    let configuration_failure = state.session.configuration_failure_snapshot();
    let previous_preferences = state.settings.preferences();
    let changes_ui_language = draft.ui_language.is_some();
    let enables_background_blend = draft.subtitle_blends_with_background == Some(true);
    ensure_settings_draft_allowed(&draft, state.session.has_active_session())?;
    let network_proxy = draft
        .network_proxy
        .as_ref()
        .map(ProxyConfig::validate)
        .transpose()
        .map_err(|error| error.to_string())?;
    let needs_save = draft.source_language.is_some()
        || draft.target_language.is_some()
        || draft.translation_mode.is_some()
        || draft.font_size.is_some()
        || draft.subtitle_font_family.is_some()
        || draft.subtitle_background_opacity.is_some()
        || draft.subtitle_color.is_some()
        || draft.microphone_subtitle_color.is_some()
        || draft.subtitle_alignment.is_some()
        || draft.subtitle_display_mode.is_some()
        || draft.show_intermediate_subtitles.is_some()
        || draft.show_subtitle_dividers.is_some()
        || draft.show_subtitle_timestamps.is_some()
        || draft.custom_subtitle_count_enabled.is_some()
        || draft.visible_subtitle_count.is_some()
        || draft.pulse_animation.is_some()
        || draft.pulse_style.is_some()
        || draft.subtitle_animation.is_some()
        || draft.subtitle_blends_with_background.is_some()
        || draft.is_overlay_locked.is_some()
        || draft.ui_language.is_some()
        || draft.retain_session_history.is_some()
        || draft.record_session_audio.is_some()
        || draft.audio_input.is_some()
        || draft.windows_audio_source.is_some()
        || draft.system_audio_target.is_some()
        || draft.show_in_dock.is_some()
        || draft.network_proxy.is_some();
    if !needs_save {
        return SettingsSnapshotPayload::try_from_store(&state.settings);
    }

    if draft
        .system_audio_target
        .as_ref()
        .is_some_and(|target| !target.validate())
    {
        return Err("application_audio_invalid_target".into());
    }
    if draft
        .system_audio_target
        .as_ref()
        .is_some_and(|target| target.application_id().is_some())
        && !crate::audio::applications::supported()
    {
        return Err(crate::audio::SystemAudioCaptureError::ApplicationUnsupported.to_string());
    }
    let target_changed = draft
        .system_audio_target
        .as_ref()
        .is_some_and(|target| *target != state.settings.preferences().system_audio_target);
    let windows_source_changed = draft
        .windows_audio_source
        .as_ref()
        .is_some_and(|source| *source != previous_preferences.windows_audio_source);
    let save_preferences = || {
        state.settings.save_preferences_for_active_profile(|prefs| {
            if let Some(proxy) = &network_proxy {
                prefs.network_proxy = proxy.clone();
            }
            if let Some(source_language) = draft.source_language {
                prefs.source_language = source_language;
            }
            if let Some(target_language) = draft.target_language {
                prefs.target_language = target_language;
            }
            if let Some(translation_mode) = draft.translation_mode {
                prefs.translation_mode = translation_mode;
            }
            if let Some(enabled) = draft.retain_session_history {
                prefs.retain_session_history = enabled;
            }
            prefs.apply_audio_preferences(draft.audio_input, draft.record_session_audio);
            if let Some(source) = draft.windows_audio_source {
                prefs.apply_windows_audio_source(source);
            }
            if let Some(opacity) = draft.subtitle_background_opacity {
                prefs.subtitle_background_opacity = opacity;
            }
            if let Some(target) = draft.system_audio_target {
                prefs.apply_system_audio_target(target);
            }
            if let Some(font_size) = draft.font_size {
                prefs.font_size = font_size;
            }
            if let Some(family) = &draft.subtitle_font_family {
                prefs.subtitle_font_family = family.clone();
            }
            if let Some(mode) = draft.subtitle_display_mode {
                prefs.subtitle_display_mode = mode;
            }
            if let Some(enabled) = draft.show_intermediate_subtitles {
                prefs.show_intermediate_subtitles = enabled;
            }
            if let Some(enabled) = draft.show_subtitle_dividers {
                prefs.show_subtitle_dividers = enabled;
            }
            if let Some(enabled) = draft.show_subtitle_timestamps {
                prefs.show_subtitle_timestamps = enabled;
            }
            if let Some(enabled) = draft.custom_subtitle_count_enabled {
                prefs.custom_subtitle_count_enabled = enabled;
            }
            if let Some(count) = draft.visible_subtitle_count {
                prefs.visible_subtitle_count = count;
            }
            if let Some(style) = draft.pulse_style {
                prefs.pulse_style = style;
            }
            if let Some(pulse) = draft.pulse_animation {
                prefs.pulse_animation = Some(pulse);
            }
            if let Some(motion) = draft.subtitle_animation {
                prefs.subtitle_animation = Some(motion);
            }
            if let Some(color) = draft.microphone_subtitle_color {
                prefs.microphone_subtitle_color = color;
            }
            if let Some(color) = draft.subtitle_color {
                prefs.subtitle_color = color;
            }
            if let Some(alignment) = draft.subtitle_alignment {
                prefs.subtitle_alignment = alignment;
            }
            if let Some(blends) = draft.subtitle_blends_with_background {
                prefs.subtitle_blends_with_background = blends;
            }
            if let Some(locked) = draft.is_overlay_locked {
                prefs.overlay_locked = locked;
            }
            if let Some(show) = draft.show_in_dock {
                prefs.show_in_dock = show;
            }
            if let Some(language) = &draft.ui_language {
                prefs.ui_language = Some(language.clone());
            }
        })
    };
    #[cfg(target_os = "macos")]
    crate::platform::macos::dock::save_with_policy(
        state.settings.preferences().show_in_dock,
        draft.show_in_dock,
        |show| crate::platform::macos::dock::apply(app, show),
        save_preferences,
    )?;
    #[cfg(not(target_os = "macos"))]
    save_preferences()?;
    let saved_preferences = state.settings.preferences();
    state.session.configuration_saved(
        configuration_failure,
        previous_preferences.source_language != saved_preferences.source_language
            || previous_preferences.network_proxy != saved_preferences.network_proxy,
    );
    sync_overlay_minimum(app);
    // Source changes can clear recording even when the draft omits it. Use
    // the saved value so a combined draft cannot retain old-source audio.
    let recording = (target_changed
        || windows_source_changed
        || draft.audio_input.is_some()
        || draft.record_session_audio.is_some())
    .then(|| state.settings.preferences().record_session_audio);
    state
        .session
        .apply_archive_opt_out(draft.retain_session_history, recording);
    // Background blending has no meaningful collapsed presentation. Enforce
    // this natively so the invariant also holds while the WebView is hidden
    // or reloading; do not rely on a React effect to repair geometry later.
    if enables_background_blend && state.session.is_overlay_collapsed() {
        state.session.set_overlay_collapsed(false);
        OverlayWindowManager::set_collapsed(app, &state.overlay, &state.settings, false);
        state.session.publish_state();
    }
    if draft.is_overlay_locked.is_some() || draft.subtitle_blends_with_background.is_some() {
        let preferences = state.settings.preferences();
        OverlayWindowManager::sync_presentation(
            app,
            state.session.should_show_overlay(),
            state.session.is_overlay_collapsed(),
            preferences.overlay_locked || preferences.subtitle_blends_with_background,
            preferences.subtitle_blends_with_background,
        );
    }
    if changes_ui_language
        || draft.subtitle_display_mode.is_some()
        || draft.pulse_animation.is_some()
        || draft.subtitle_animation.is_some()
        || draft.show_in_dock.is_some()
    {
        crate::platform::menu::refresh_native_tray_language(app);
    }

    let payload = SettingsSnapshotPayload::try_from_store(&state.settings)?;
    let _ = app.emit("settings-changed", payload.clone());
    Ok(payload)
}

/// Serializes a native Dock toggle with settings saves.
#[cfg(target_os = "macos")]
pub(crate) async fn toggle_dock_visibility(app: AppHandle) -> Result<(), String> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "Application state is unavailable.".to_string())?;
    let _lifecycle = state.session.settings_mutation_guard(false).await?;
    let show = !state.settings.preferences().show_in_dock;
    apply_settings_draft_guarded(
        &app,
        &state,
        SettingsDraft {
            show_in_dock: Some(show),
            ..SettingsDraft::default()
        },
    )?;
    Ok(())
}

/// Toggles the persisted Immersive Mode presentation from native surfaces
/// such as the global shortcut. This deliberately reuses the settings mutation
/// path so collapsed geometry, click-through state, and settings broadcasts
/// stay identical to UI-triggered changes.
pub(crate) async fn toggle_immersive_mode(app: &AppHandle) -> Result<(), String> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "Application state is unavailable.".to_string())?;
    let _lifecycle = state.session.settings_mutation_guard(false).await?;
    let enabled =
        toggled_immersive_mode(state.settings.preferences().subtitle_blends_with_background);
    apply_settings_draft_guarded(
        app,
        &state,
        SettingsDraft {
            subtitle_blends_with_background: Some(enabled),
            ..SettingsDraft::default()
        },
    )?;
    Ok(())
}

/// Uses the same persisted, broadcast mutation as settings and tray controls.
pub(crate) async fn set_subtitle_display_mode(
    app: &AppHandle,
    requested: Option<SubtitleDisplayMode>,
) -> Result<(), String> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "Application state is unavailable.".to_string())?;
    let _lifecycle = state.session.settings_mutation_guard(false).await?;
    let mode =
        requested.unwrap_or_else(|| state.settings.preferences().subtitle_display_mode.next());
    apply_settings_draft_guarded(
        app,
        &state,
        SettingsDraft {
            subtitle_display_mode: Some(mode),
            ..SettingsDraft::default()
        },
    )?;
    Ok(())
}

fn toggled_immersive_mode(current: bool) -> bool {
    !current
}

fn ensure_settings_draft_allowed(draft: &SettingsDraft, is_active: bool) -> Result<(), String> {
    if is_active && draft.network_proxy.is_some() {
        return Err("network_proxy_change_requires_stop".into());
    }
    if is_active
        && (draft.source_language.is_some()
            || draft.target_language.is_some()
            || draft.translation_mode.is_some()
            || draft.retain_session_history.is_some()
            || draft.record_session_audio.is_some()
            || draft.audio_input.is_some()
            || draft.windows_audio_source.is_some()
            || draft.system_audio_target.is_some())
    {
        Err(
            "Listening settings cannot be changed through settings while a session is active."
                .to_string(),
        )
    } else {
        Ok(())
    }
}

pub(super) fn emit_settings_snapshot(
    app: &AppHandle,
    settings: &SettingsStore,
) -> Result<SettingsSnapshotPayload, String> {
    sync_overlay_minimum(app);
    let payload = SettingsSnapshotPayload::try_from_store(settings)?;
    let _ = app.emit("settings-changed", payload.clone());
    Ok(payload)
}

#[cfg(test)]
mod tests;
