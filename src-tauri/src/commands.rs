//! Tauri command handlers exposed to the frontend. The IPC contract is
//! documented in docs/plans/2026-08-22-multi-provider-professional-settings-design.md.

use crate::core::audio_input::AudioInput;
use crate::core::credentials::{CredentialRevealField, ProviderCredentials};
use crate::core::models::{
    SourceLanguage, SubtitleColor, SubtitleDisplayMode, TargetLanguage, TranslationMode,
};
use crate::core::network_proxy::ProxyConfig;
use crate::core::provider::{
    CustomSpeechLanguagesPatch, ProviderKind, ServiceProfile, TextTranslation, TextTranslationName,
};
use crate::session_manager::{SessionManager, SessionStateEvent};
use crate::settings_store::{CredentialState, PulseStyle, SettingsStore, SubtitleAlignment};
use crate::windows::{
    OverlayControlMode, OverlayControlWindowManager, OverlayWindowManager, TrayPanelManager,
};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter, Listener, Manager, State};
use tauri_plugin_opener::OpenerExt;

const SETTINGS_NAVIGATION_EVENT: &str = "settings-navigate";
const SETTINGS_NAVIGATION_READY_EVENT: &str = "settings-navigation-ready";
const RELEASES_LATEST_URL: &str = "https://github.com/yuxino/mimi/releases/latest";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SettingsNavigationTarget {
    Service,
    Export,
    AppleSpeechResources,
    ActiveProfile,
}

pub struct AppState {
    pub settings: Arc<SettingsStore>,
    pub session: Arc<SessionManager>,
    /// Single source of truth for the overlay window geometry.
    pub overlay: Arc<std::sync::Mutex<crate::windows::OverlayState>>,
}

pub(crate) fn sync_overlay_minimum(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        OverlayWindowManager::sync_minimum_height(app, &state.overlay, &state.settings);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProfilePayload {
    pub id: String,
    pub name: String,
    pub provider: ProviderKind,
    pub language_preset: Option<crate::core::provider::ProfileLanguagePreset>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_recognition_name: Option<String>,
    pub credential_state: CredentialState,
    pub credential_storage: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech_credential_state: Option<CredentialState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_credential_state: Option<CredentialState>,
    pub text_translation: crate::core::provider::TextTranslation,
    pub text_translation_names: std::collections::BTreeMap<TextTranslation, String>,
    pub speech_network_proxy: Option<ProxyConfig>,
    pub text_network_proxy: Option<ProxyConfig>,
    pub custom_speech_source_languages: Option<Vec<SourceLanguage>>,
}

impl ServiceProfilePayload {
    fn from_profile(store: &SettingsStore, profile: ServiceProfile) -> Self {
        let states = store.custom_credential_states_for_snapshot(&profile);
        let credential_state = states.map_or_else(
            || store.credential_state_for_snapshot(&profile),
            |(speech, text)| speech.combined(text),
        );
        let text_translation = profile.text_translation();
        let credential_storage = store.profile_credential_storage(&profile.id);
        Self {
            speech_network_proxy: profile.speech_network_proxy,
            text_network_proxy: profile.text_network_proxy,
            custom_speech_source_languages: profile.custom_speech_source_languages,
            id: profile.id,
            name: profile.name,
            provider: profile.provider,
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

    fn unavailable(profile: ServiceProfile) -> Self {
        let text_translation = profile.text_translation();
        Self {
            speech_network_proxy: profile.speech_network_proxy,
            text_network_proxy: profile.text_network_proxy,
            custom_speech_source_languages: profile.custom_speech_source_languages,
            id: profile.id,
            name: profile.name,
            provider: profile.provider,
            language_preset: profile.language_preset,
            speech_recognition_name: profile.speech_recognition_name,
            credential_state: CredentialState::Unavailable,
            credential_storage: "keychain",
            speech_credential_state: profile
                .provider
                .is_standalone_asr()
                .then_some(CredentialState::Unavailable),
            text_credential_state: profile
                .provider
                .is_standalone_asr()
                .then_some(CredentialState::Unavailable),
            text_translation,
            text_translation_names: profile.text_translation_names,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageCapabilitiesPayload {
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
    fn from_profile(profile: &ServiceProfile, target: TargetLanguage) -> Self {
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
    pub system_audio_target: crate::core::system_audio_target::SystemAudioTarget,
    pub show_in_dock: bool,
    pub network_proxy: ProxyConfig,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple_speech::AppleSpeechResourceStatus;

    #[test]
    fn profile_snapshots_keep_speech_names_when_credentials_are_unavailable() {
        let store = SettingsStore::in_memory(Box::new(PartiallyUnavailableSecretStore), false);
        let mut profile =
            ServiceProfile::new("custom", "Configuration", ProviderKind::CustomOpenAIASR).unwrap();
        profile
            .set_speech_recognition_name("Whisper · 本地")
            .unwrap();
        for payload in [
            ServiceProfilePayload::from_profile(&store, profile.clone()),
            ServiceProfilePayload::unavailable(profile),
        ] {
            let json = serde_json::to_value(payload).unwrap();
            assert_eq!(json["speechRecognitionName"], "Whisper · 本地");
            assert_eq!(json["name"], "Configuration");
            for field in ["endpoint", "apiKey", "token", "model"] {
                assert!(json.get(field).is_none());
            }
        }
    }

    #[test]
    fn profile_snapshots_keep_translation_names_when_credentials_are_unavailable() {
        let store = SettingsStore::in_memory(Box::new(PartiallyUnavailableSecretStore), false);
        let mut profile = ServiceProfile::alibaba_default();
        profile
            .set_text_translation_name(TextTranslation::OpenAICompatible, "Work translator")
            .unwrap();
        for payload in [
            ServiceProfilePayload::from_profile(&store, profile.clone()),
            ServiceProfilePayload::unavailable(profile),
        ] {
            let json = serde_json::to_value(payload).unwrap();
            assert_eq!(
                json["textTranslationNames"]["openAICompatible"],
                "Work translator"
            );
            for field in ["endpoint", "apiKey", "token", "model"] {
                assert!(json.get(field).is_none());
            }
        }
        let patch: TextTranslationName = serde_json::from_value(serde_json::json!({
            "route": "openAICompatible", "name": "Work translator"
        }))
        .unwrap();
        assert_eq!(patch.route, TextTranslation::OpenAICompatible);
    }

    #[test]
    fn custom_language_metadata_and_filtered_snapshot_survive_unavailable_credentials() {
        let store = SettingsStore::in_memory(Box::new(PartiallyUnavailableSecretStore), false);
        let mut profile =
            ServiceProfile::new("custom", "Synthetic", ProviderKind::CustomOpenAIASR).unwrap();
        profile
            .set_custom_speech_source_languages(Some(vec![SourceLanguage::French]))
            .unwrap();
        profile.text_translation = Some(TextTranslation::OpenAICompatible);
        let capabilities =
            LanguageCapabilitiesPayload::from_profile(&profile, TargetLanguage::German);
        assert_eq!(
            capabilities.source_languages,
            vec![SourceLanguage::Automatic, SourceLanguage::French]
        );
        assert_eq!(capabilities.target_languages, TargetLanguage::ALL);
        for payload in [
            ServiceProfilePayload::from_profile(&store, profile.clone()),
            ServiceProfilePayload::unavailable(profile),
        ] {
            let json = serde_json::to_value(payload).unwrap();
            assert_eq!(
                json["customSpeechSourceLanguages"],
                serde_json::json!(["fr"])
            );
            for secret_field in ["endpoint", "apiKey", "token", "model"] {
                assert!(json.get(secret_field).is_none());
            }
        }
        let unknown = ServiceProfilePayload::unavailable(
            ServiceProfile::new("old", "Legacy", ProviderKind::CustomDashScopeASR).unwrap(),
        );
        assert!(serde_json::to_value(unknown).unwrap()["customSpeechSourceLanguages"].is_null());
    }

    fn apple_language_support() -> crate::apple_speech_support::AppleSpeechSupport {
        use crate::apple_speech_support::{AppleSpeechLanguage, AppleSpeechSupport};
        AppleSpeechSupport {
            available: true,
            languages: vec![
                AppleSpeechLanguage {
                    source_language: SourceLanguage::English,
                    locale: "en-US".into(),
                    status: AppleSpeechResourceStatus::Installed,
                    installed: true,
                    downloading: false,
                },
                AppleSpeechLanguage {
                    source_language: SourceLanguage::Japanese,
                    locale: "ja-JP".into(),
                    status: AppleSpeechResourceStatus::Supported,
                    installed: false,
                    downloading: false,
                },
                AppleSpeechLanguage {
                    source_language: SourceLanguage::French,
                    locale: "fr-FR".into(),
                    status: AppleSpeechResourceStatus::Supported,
                    installed: false,
                    downloading: false,
                },
            ],
        }
    }

    #[test]
    fn apple_language_snapshot_only_offers_ready_sources_on_available_devices() {
        let profile = ServiceProfile::new("apple", "Apple", ProviderKind::AppleSpeech).unwrap();
        let mut support = apple_language_support();
        let payload = LanguageCapabilitiesPayload::from_profile_with_apple_support(
            &profile,
            TargetLanguage::Original,
            Some((&support, 1)),
        );
        let json = serde_json::to_value(payload).unwrap();
        assert_eq!(json["profileId"], "apple");
        assert_eq!(json["appleSpeechSupportRevision"], 1);
        assert_eq!(json["sourceLanguages"], serde_json::json!(["en"]));
        assert_eq!(support, apple_language_support());

        support.available = false;
        for support in [Some((&support, 1)), None] {
            let payload = LanguageCapabilitiesPayload::from_profile_with_apple_support(
                &profile,
                TargetLanguage::Original,
                support,
            );
            assert!(payload.source_languages.is_empty());
        }
    }

    #[test]
    fn apple_language_snapshot_keeps_ready_sources_within_the_text_route() {
        let mut profile = ServiceProfile::new("apple", "Apple", ProviderKind::AppleSpeech).unwrap();
        let mut support = apple_language_support();
        support.languages[2].set_status(AppleSpeechResourceStatus::Installed);
        for route in [TextTranslation::DeepL, TextTranslation::DeepLX] {
            profile.text_translation = Some(route);
            let translated = LanguageCapabilitiesPayload::from_profile_with_apple_support(
                &profile,
                TargetLanguage::English,
                Some((&support, 1)),
            );
            assert_eq!(
                translated
                    .source_languages
                    .iter()
                    .map(|source| source.raw_value())
                    .collect::<std::collections::BTreeSet<_>>(),
                ["en", "fr"].into_iter().collect()
            );
            let original = LanguageCapabilitiesPayload::from_profile_with_apple_support(
                &profile,
                TargetLanguage::Original,
                Some((&support, 1)),
            );
            assert_eq!(
                original.source_languages,
                vec![SourceLanguage::English, SourceLanguage::French]
            );
        }
        profile.text_translation = Some(TextTranslation::OpenAICompatible);
        let translated = LanguageCapabilitiesPayload::from_profile_with_apple_support(
            &profile,
            TargetLanguage::English,
            Some((&support, 1)),
        );
        assert_eq!(
            translated.source_languages,
            vec![SourceLanguage::English, SourceLanguage::French]
        );
    }

    #[test]
    fn apple_language_snapshot_adds_a_prepared_source_without_hiding_downloadable_resources() {
        let profile = ServiceProfile::new("apple", "Apple", ProviderKind::AppleSpeech).unwrap();
        let mut support = apple_language_support();
        let before = LanguageCapabilitiesPayload::from_profile_with_apple_support(
            &profile,
            TargetLanguage::Original,
            Some((&support, 1)),
        );
        assert_eq!(before.source_languages, vec![SourceLanguage::English]);
        support.languages[1].set_status(AppleSpeechResourceStatus::Installed);
        let after = LanguageCapabilitiesPayload::from_profile_with_apple_support(
            &profile,
            TargetLanguage::Original,
            Some((&support, 1)),
        );
        assert_eq!(
            after.source_languages,
            vec![SourceLanguage::English, SourceLanguage::Japanese]
        );
        // The resource preparation response still includes the unprepared
        // French locale; no fallback language is inserted into the picker.
        let resources = serde_json::to_value(&support).unwrap();
        assert_eq!(resources["languages"].as_array().unwrap().len(), 3);
        assert_eq!(resources["languages"][2]["sourceLanguage"], "fr");
        assert_eq!(resources["languages"][2]["installed"], false);
        support
            .languages
            .iter_mut()
            .for_each(|language| language.set_status(AppleSpeechResourceStatus::Supported));
        let empty = LanguageCapabilitiesPayload::from_profile_with_apple_support(
            &profile,
            TargetLanguage::Original,
            Some((&support, 1)),
        );
        assert!(empty.source_languages.is_empty());
    }

    #[test]
    fn language_capability_snapshot_is_stamped_and_tracks_the_atomic_profile_route() {
        let store = SettingsStore::in_memory(Box::new(PartiallyUnavailableSecretStore), false);
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::French
            })
            .unwrap();
        let snapshot = SettingsSnapshotPayload::try_from_store(&store).unwrap();
        let value = serde_json::to_value(snapshot).unwrap();
        let caps = &value["languageCapabilities"];
        assert_eq!(caps["profileId"], value["activeProfileId"]);
        assert_eq!(caps["targetLanguage"], value["targetLanguage"]);
        assert_eq!(caps["provider"], "alibabaCloud");
        assert!(caps.get("appleSpeechSupportRevision").is_none());
        assert_eq!(caps["textTranslation"], "followService");
        assert_eq!(caps["sourceLanguages"].as_array().unwrap().len(), 25);
        assert_eq!(caps["targetLanguages"].as_array().unwrap().len(), 32);
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::Original;
                prefs.source_language = SourceLanguage::Greek;
            })
            .unwrap();
        let snapshot = SettingsSnapshotPayload::try_from_store(&store).unwrap();
        assert_eq!(
            snapshot
                .language_capabilities
                .unwrap()
                .source_languages
                .len(),
            31
        );
        let mut profile = ServiceProfile::alibaba_default();
        profile.text_translation = Some(TextTranslation::DeepL);
        let deep_l = LanguageCapabilitiesPayload::from_profile(&profile, TargetLanguage::Original);
        assert_eq!(deep_l.text_translation, TextTranslation::DeepL);
        assert_eq!(deep_l.source_languages.len(), 31);
        assert_eq!(
            deep_l.target_languages.len(),
            crate::core::protocols::deepl_languages::DEEPL_TARGET_CODES.len() + 1
        );
        profile.text_translation = Some(TextTranslation::OpenAICompatible);
        let custom = LanguageCapabilitiesPayload::from_profile(&profile, TargetLanguage::Original);
        assert_eq!(
            custom
                .source_languages
                .iter()
                .map(|source| source.raw_value())
                .collect::<Vec<_>>(),
            std::iter::once("auto")
                .chain(
                    crate::core::protocols::audio3::LANGUAGE_CODES
                        .iter()
                        .copied()
                )
                .collect::<Vec<_>>()
        );
        assert_eq!(custom.target_languages, TargetLanguage::ALL);
        let custom = serde_json::to_value(custom).unwrap();
        assert_eq!(custom["provider"], "alibabaCloud");
        assert_eq!(custom["textTranslation"], "openAICompatible");
        for secret_field in ["endpoint", "apiKey", "token", "model"] {
            assert!(custom.get(secret_field).is_none());
        }
    }

    #[test]
    fn background_blend_forces_expanded_overlay() {
        assert!(!normalize_overlay_collapsed(true, true));
        assert!(!normalize_overlay_collapsed(false, true));
        assert!(normalize_overlay_collapsed(true, false));
    }

    #[test]
    fn immersive_mode_toggle_inverts_the_current_preference() {
        assert!(toggled_immersive_mode(false));
        assert!(!toggled_immersive_mode(true));
    }

    struct PartiallyUnavailableSecretStore;

    impl crate::settings_store::SecretStore for PartiallyUnavailableSecretStore {
        fn load(
            &self,
            _service: &str,
            account: &str,
        ) -> Result<Option<String>, crate::settings_store::SecretStoreError> {
            if account.contains("alibaba-default") {
                Err(crate::settings_store::SecretStoreError::Unavailable)
            } else {
                Ok(None)
            }
        }

        fn save(
            &self,
            _service: &str,
            _account: &str,
            _value: &str,
        ) -> Result<(), crate::settings_store::SecretStoreError> {
            Ok(())
        }

        fn delete(
            &self,
            _service: &str,
            _account: &str,
        ) -> Result<(), crate::settings_store::SecretStoreError> {
            Ok(())
        }
    }

    #[test]
    fn diagnostics_are_registered_and_allowed_only_in_settings() {
        for command in [
            "profile_test_connection",
            "windows_audio_status",
            "audio_census",
            "support_diagnostics",
            "app_open_support_issue",
            "profile_reveal_credential",
            "profile_credential_editor_state",
            "open_tencent_setup_page",
        ] {
            assert!(include_str!("lib.rs").contains(&format!("commands::{command},")));
            let permissions = include_str!("../permissions/app.toml");
            let permitted: Vec<_> = permissions
                .split("[[permission]]")
                .filter(|entry| entry.contains(&format!("\"{command}\"")))
                .collect();
            assert_eq!(permitted.len(), 1);
            assert!(permitted[0].contains("identifier = \"app-settings\""));
            let capability: serde_json::Value =
                serde_json::from_str(include_str!("../capabilities/settings.json")).unwrap();
            assert!(capability["permissions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|permission| permission == "app-settings"));
            assert_eq!(capability["windows"], serde_json::json!(["settings"]));
        }
    }

    #[test]
    fn tencent_setup_pages_reject_arbitrary_urls() {
        for (value, expected) in [
            ("account", "https://console.cloud.tencent.com/developer"),
            ("apiKey", "https://console.cloud.tencent.com/cam/capi"),
            ("asr", "https://console.cloud.tencent.com/asr"),
        ] {
            let page: TencentSetupPage = serde_json::from_value(serde_json::json!(value)).unwrap();
            assert_eq!(page.url(), expected);
        }
        for value in ["https://example.com", "file:///tmp/test", "Account", ""] {
            assert!(serde_json::from_value::<TencentSetupPage>(serde_json::json!(value)).is_err());
        }
    }

    #[test]
    fn credential_reveal_rejects_every_other_window_before_store_access() {
        assert_eq!(ensure_credential_reveal_window("settings"), Ok(()));
        for label in [
            "overlay",
            "overlay-control",
            "tray-panel",
            "main",
            "settings-copy",
            "",
        ] {
            assert_eq!(
                ensure_credential_reveal_window(label),
                Err("credential_reveal_not_allowed".into())
            );
        }
    }

    #[test]
    fn overlay_can_restart_an_errored_session() {
        let permissions = include_str!("../permissions/app.toml");
        let overlay_permission = permissions
            .split("[[permission]]")
            .find(|entry| entry.contains("identifier = \"app-overlay\""))
            .unwrap();
        assert!(overlay_permission.contains("\"session_start\""));
        assert!(overlay_permission.contains("\"session_stop\""));
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/overlay.json")).unwrap();
        assert_eq!(capability["windows"], serde_json::json!(["overlay"]));
        assert!(capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|permission| permission == "app-overlay"));
        assert!(include_str!("lib.rs").contains("commands::session_start,"));
    }

    #[test]
    fn control_panel_can_retry_without_pause_or_stop_access() {
        let permission = include_str!("../permissions/app.toml")
            .split("[[permission]]")
            .find(|entry| entry.contains("identifier = \"app-overlay-control\""))
            .unwrap();
        assert!(!permission.contains("\"session_toggle_paused\""));
        assert!(permission.contains("\"session_start\""));
        assert!(!permission.contains("\"session_stop\""));
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/overlay-control.json")).unwrap();
        assert_eq!(
            capability["windows"],
            serde_json::json!(["overlay-control"])
        );
        assert!(capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|permission| permission == "app-overlay-control"));
        assert!(include_str!("lib.rs").contains("commands::session_toggle_paused,"));
    }

    #[test]
    fn control_panel_retry_ipc_is_authorized_by_its_native_permission() {
        // UI tests replace start with a mock. Check the real callback, store,
        // IPC command and native ACL together so a visible retry can reach Rust.
        let control_window =
            include_str!("../../src/windows/overlay-control/OverlayControlWindow.tsx");
        assert!(control_window
            .contains("onRetrySession={errorRequiresConfiguration ? undefined : start}"));
        assert!(include_str!("../../src/lib/store.ts")
            .split("start: async () => {")
            .nth(1)
            .unwrap()
            .split("return;")
            .next()
            .unwrap()
            .contains("await sessionStart();"));
        let ipc = include_str!("../../src/lib/ipc.ts");
        let command = ipc
            .split("export function sessionStart(): Promise<void> {")
            .nth(1)
            .unwrap()
            .split("return invoke(\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert_eq!(command, "session_start");
        let permission = include_str!("../permissions/app.toml")
            .split("[[permission]]")
            .find(|entry| entry.contains("identifier = \"app-overlay-control\""))
            .unwrap();
        assert!(permission.contains(&format!("\"{command}\"")));
        assert!(include_str!("lib.rs").contains(&format!("commands::{command},")));
    }

    #[test]
    fn native_pointer_cursor_intents_are_registered_and_overlay_scoped() {
        let permissions = include_str!("../permissions/app.toml");
        let permitted: Vec<_> = permissions
            .split("[[permission]]")
            .filter(|entry| entry.contains("\"overlay_set_pointer_cursor\""))
            .collect();
        assert_eq!(permitted.len(), 1);
        assert!(permitted[0].contains("identifier = \"app-overlay\""));
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/overlay.json")).unwrap();
        assert_eq!(capability["windows"], serde_json::json!(["overlay"]));
        assert!(capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|permission| permission == "app-overlay"));
        assert!(include_str!("lib.rs").contains("commands::overlay_set_pointer_cursor,"));
        assert_eq!(ensure_overlay_pointer_cursor_window("overlay"), Ok(()));
        for label in ["settings", "overlay-control", "tray-panel", "main", ""] {
            assert_eq!(
                ensure_overlay_pointer_cursor_window(label),
                Err("overlay_pointer_cursor_not_allowed".into())
            );
        }
    }

    #[test]
    fn capture_status_is_readable_in_settings_and_control_panel_only() {
        let permissions = include_str!("../permissions/app.toml");
        let permitted: Vec<_> = permissions
            .split("[[permission]]")
            .filter(|entry| entry.contains("\"capture_status\""))
            .collect();
        assert_eq!(permitted.len(), 2);
        assert!(permitted
            .iter()
            .all(|entry| entry.contains("identifier = \"app-settings\"")
                || entry.contains("identifier = \"app-overlay-control\"")));
        assert!(include_str!("lib.rs").contains("commands::capture_status,"));
    }

    #[test]
    fn apple_asset_management_is_settings_only() {
        for command in ["get_apple_speech_support", "prepare_apple_speech_language"] {
            let permissions = include_str!("../permissions/app.toml");
            let permitted: Vec<_> = permissions
                .split("[[permission]]")
                .filter(|entry| entry.contains(&format!("\"{command}\"")))
                .collect();
            assert_eq!(permitted.len(), 1);
            assert!(permitted[0].contains("identifier = \"app-settings\""));
            assert!(include_str!("lib.rs").contains(&format!("commands::{command},")));
        }
    }

    #[test]
    fn live_audio_capture_switches_are_scoped_to_settings_and_control_panel() {
        let permissions = include_str!("../permissions/app.toml");
        for command in [
            "session_switch_audio_input",
            "session_switch_system_audio_target",
        ] {
            let permitted: Vec<_> = permissions
                .split("[[permission]]")
                .filter(|entry| entry.contains(&format!("\"{command}\"")))
                .collect();
            assert_eq!(permitted.len(), 2);
            assert!(permitted
                .iter()
                .all(|entry| entry.contains("identifier = \"app-settings\"")
                    || entry.contains("identifier = \"app-overlay-control\"")));
            assert!(include_str!("lib.rs").contains(&format!("commands::{command},")));
        }
    }

    #[test]
    fn target_switch_is_scoped_to_settings_control_and_tray_windows() {
        let permissions = include_str!("../permissions/app.toml");
        let permitted: Vec<_> = permissions
            .split("[[permission]]")
            .filter(|entry| entry.contains("\"session_switch_target_language\""))
            .collect();
        assert_eq!(permitted.len(), 3);
        for identifier in ["app-settings", "app-overlay-control", "app-tray-panel"] {
            assert!(permitted
                .iter()
                .any(|entry| entry.contains(&format!("identifier = \"{identifier}\""))));
        }
        assert!(include_str!("lib.rs").contains("commands::session_switch_target_language,"));
    }

    #[test]
    fn island_width_measurement_is_control_window_scoped() {
        let permissions = include_str!("../permissions/app.toml");
        let permitted: Vec<_> = permissions
            .split("[[permission]]")
            .filter(|entry| entry.contains("\"overlay_control_set_island_width\""))
            .collect();
        assert_eq!(permitted.len(), 1);
        assert!(permitted[0].contains("identifier = \"app-overlay-control\""));
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/overlay-control.json")).unwrap();
        assert_eq!(
            capability["windows"],
            serde_json::json!(["overlay-control"])
        );
        assert!(include_str!("lib.rs").contains("commands::overlay_control_set_island_width,"));
    }

    #[test]
    fn frontend_readiness_markers_are_test_only_and_window_scoped() {
        let directory =
            std::env::temp_dir().join(format!("mimi-ui-ready-{}", uuid::Uuid::new_v4()));
        let destination = Some(directory.as_os_str());
        write_ui_test_frontend_ready(false, "settings", destination).unwrap();
        write_ui_test_frontend_ready(true, "../outside", destination).unwrap();
        assert!(!directory.exists());

        write_ui_test_frontend_ready(true, "settings", destination).unwrap();
        assert_eq!(
            std::fs::read(directory.join("settings")).unwrap(),
            b"ready\n"
        );
        assert!(!directory.join("overlay").exists());
        write_ui_test_frontend_ready(true, "overlay", destination).unwrap();
        assert_eq!(
            std::fs::read(directory.join("overlay")).unwrap(),
            b"ready\n"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dock_preference_is_settings_and_tray_panel_scoped() {
        for show in [false, true] {
            let draft = SettingsDraft {
                show_in_dock: Some(show),
                ..SettingsDraft::default()
            };
            for label in ["settings", "tray-panel"] {
                assert!(ensure_settings_draft_window_allowed(label, &draft).is_ok());
            }
            for label in ["overlay", "overlay-control", "unknown"] {
                assert!(ensure_settings_draft_window_allowed(label, &draft).is_err());
            }
        }
        let presentation = SettingsDraft {
            subtitle_display_mode: Some(SubtitleDisplayMode::Bilingual),
            ..SettingsDraft::default()
        };
        assert!(ensure_settings_draft_window_allowed("overlay-control", &presentation).is_ok());
    }

    #[test]
    fn intermediate_subtitle_toggle_is_allowed_during_an_active_session() {
        for enabled in [false, true] {
            let draft: SettingsDraft =
                serde_json::from_value(serde_json::json!({"showIntermediateSubtitles": enabled}))
                    .unwrap();
            assert_eq!(draft.show_intermediate_subtitles, Some(enabled));
            assert!(ensure_settings_draft_allowed(&draft, true).is_ok());
            assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
            assert!(ensure_settings_draft_window_allowed("tray-panel", &draft).is_ok());
        }
    }

    #[test]
    fn subtitle_timestamp_toggle_is_allowed_during_an_active_session() {
        for enabled in [false, true] {
            let draft: SettingsDraft =
                serde_json::from_value(serde_json::json!({"showSubtitleTimestamps": enabled}))
                    .unwrap();
            assert_eq!(draft.show_subtitle_timestamps, Some(enabled));
            assert!(ensure_settings_draft_allowed(&draft, true).is_ok());
            assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
        }
    }

    #[test]
    fn subtitle_font_choice_is_optional_and_allowed_during_an_active_session() {
        assert!(SettingsDraft::default().subtitle_font_family.is_none());
        for family in ["", "Noto Sans CJK SC", "思源黑体"] {
            let draft: SettingsDraft =
                serde_json::from_value(serde_json::json!({"subtitleFontFamily": family})).unwrap();
            assert_eq!(draft.subtitle_font_family.as_deref(), Some(family));
            assert!(ensure_settings_draft_allowed(&draft, true).is_ok());
            assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
        }
        assert!(serde_json::from_value::<SettingsDraft>(
            serde_json::json!({"subtitleFontFamily": ["Font", "Fallback"]})
        )
        .is_err());
    }

    #[test]
    fn dock_access_does_not_widen_settings_only_preferences() {
        for field in [
            serde_json::json!({"retainSessionHistory": false}),
            serde_json::json!({"recordSessionAudio": false}),
            serde_json::json!({"windowsAudioSource": ""}),
            serde_json::json!({"audioInput": "microphone"}),
            serde_json::json!({"networkProxy": {"mode": "direct"}}),
        ] {
            for show in [None, Some(false), Some(true)] {
                let mut draft: SettingsDraft = serde_json::from_value(field.clone()).unwrap();
                draft.show_in_dock = show;
                assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
                for label in ["tray-panel", "overlay", "overlay-control", "unknown"] {
                    assert!(ensure_settings_draft_window_allowed(label, &draft).is_err());
                }
            }
        }
    }

    #[test]
    fn settings_payload_is_camel_case_and_write_only() {
        let payload = SettingsSnapshotPayload {
            credential_storage: "keychain",
            profiles: vec![ServiceProfilePayload {
                language_preset: None,
                speech_network_proxy: None,
                text_network_proxy: None,
                custom_speech_source_languages: None,
                id: "alibaba-default".into(),
                name: "Alibaba Cloud".into(),
                provider: ProviderKind::AlibabaCloud,
                speech_recognition_name: None,
                credential_state: CredentialState::Present,
                credential_storage: "keychain",
                speech_credential_state: None,
                text_credential_state: None,
                text_translation: crate::core::provider::TextTranslation::FollowService,
                text_translation_names: std::collections::BTreeMap::new(),
            }],
            active_profile_id: "alibaba-default".into(),
            language_capabilities: None,
            source_language: SourceLanguage::Japanese,
            target_language: TargetLanguage::SimplifiedChinese,
            translation_mode: TranslationMode::HighQuality,
            font_size: 18.0,
            subtitle_font_family: "Noto Sans CJK SC".into(),
            subtitle_background_opacity: 80,
            subtitle_color: SubtitleColor::White,
            microphone_subtitle_color: SubtitleColor::Yellow,
            subtitle_alignment: SubtitleAlignment::Center,
            subtitle_display_mode: SubtitleDisplayMode::Translation,
            show_intermediate_subtitles: true,
            show_subtitle_dividers: false,
            show_subtitle_timestamps: false,
            pulse_animation: None,
            pulse_style: PulseStyle::Ribbon,
            subtitle_animation: None,
            subtitle_blends_with_background: false,
            is_overlay_locked: false,
            ui_language: None,
            retain_session_history: false,
            record_session_audio: false,
            audio_input: AudioInput::System,
            microphone_input_available: crate::core::audio_input::MICROPHONE_INPUT_AVAILABLE,
            windows_audio_source: String::new(),
            system_audio_target: Default::default(),
            show_in_dock: false,
            network_proxy: ProxyConfig::default(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["activeProfileId"], "alibaba-default");
        assert_eq!(json["microphoneInputAvailable"], true);
        assert_eq!(json["credentialStorage"], "keychain");
        assert_eq!(json["pulseStyle"], "ribbon");
        assert_eq!(json["showInDock"], false);
        assert_eq!(json["audioInput"], "system");
        assert_eq!(json["networkProxy"]["mode"], "system");
        assert_eq!(json["profiles"][0]["provider"], "alibabaCloud");
        assert_eq!(json["profiles"][0]["credentialState"], "present");
        assert_eq!(json["profiles"][0]["textTranslation"], "followService");
        for secret_field in ["apiKey", "asrApiKey", "endpoint", "token"] {
            assert!(json["profiles"][0].get(secret_field).is_none());
        }
        assert_eq!(json["subtitleAlignment"], "center");
        assert_eq!(json["subtitleFontFamily"], "Noto Sans CJK SC");
        assert_eq!(json["subtitleBackgroundOpacity"], 80);
        assert_eq!(json["subtitleColor"], "white");
        assert_eq!(json["subtitleDisplayMode"], "translation");
        assert_eq!(json["showIntermediateSubtitles"], true);
        assert_eq!(json["showSubtitleDividers"], false);
        assert_eq!(json["showSubtitleTimestamps"], false);
        assert_eq!(json["microphoneSubtitleColor"], "yellow");
        assert_eq!(json["subtitleBlendsWithBackground"], false);
        assert!(json.get("apiKey").is_none());
        assert!(json.get("hasAPIKey").is_none());
        assert!(!json.to_string().contains("secret"));
    }

    #[test]
    fn active_session_rejects_every_profile_mutation() {
        assert!(ensure_profile_mutation_allowed(true).is_err());
        assert!(ensure_profile_mutation_allowed(false).is_ok());
    }

    #[test]
    fn archive_preferences_require_an_inactive_session_for_both_directions() {
        for enabled in [true, false] {
            for draft in [
                SettingsDraft {
                    retain_session_history: Some(enabled),
                    ..Default::default()
                },
                SettingsDraft {
                    record_session_audio: Some(enabled),
                    ..Default::default()
                },
            ] {
                assert!(ensure_settings_draft_allowed(&draft, true).is_err());
                assert!(ensure_settings_draft_allowed(&draft, false).is_ok());
            }
        }
    }

    #[test]
    fn audio_source_changes_require_stop_even_when_resetting_to_system() {
        for source in ["wasapi:headphones", ""] {
            let draft = SettingsDraft {
                windows_audio_source: Some(source.into()),
                ..Default::default()
            };
            assert!(ensure_settings_draft_allowed(&draft, true).is_err());
            assert!(ensure_settings_draft_allowed(&draft, false).is_ok());
        }
    }

    #[test]
    fn application_settings_draft_is_settings_only_and_enumeration_reaches_live_controls() {
        use crate::core::system_audio_target::SystemAudioTarget;
        let draft = SettingsDraft {
            system_audio_target: Some(SystemAudioTarget::Application {
                id: "test.player".into(),
                name: "Player".into(),
            }),
            ..Default::default()
        };
        assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
        for label in ["overlay", "overlay-control", "tray-panel"] {
            assert!(ensure_settings_draft_window_allowed(label, &draft).is_err());
        }
        let permissions = include_str!("../permissions/app.toml");
        let entries: Vec<_> = permissions
            .split("[[permission]]")
            .filter(|entry| entry.contains("\"audio_applications\""))
            .collect();
        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .all(|entry| entry.contains("identifier = \"app-settings\"")
                || entry.contains("identifier = \"app-overlay-control\"")));
    }

    #[test]
    fn audio_input_changes_are_settings_only_and_require_stop() {
        for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
            let draft = SettingsDraft {
                audio_input: Some(input),
                ..Default::default()
            };
            assert!(ensure_settings_draft_allowed(&draft, true).is_err());
            assert!(ensure_settings_draft_allowed(&draft, false).is_ok());
            assert!(ensure_settings_draft_window_allowed("settings", &draft).is_ok());
            for window in ["tray-panel", "overlay", "overlay-control"] {
                assert!(ensure_settings_draft_window_allowed(window, &draft).is_err());
            }
        }
        assert!(
            serde_json::from_value::<SettingsDraft>(serde_json::json!({"audioInput":"none"}))
                .is_err()
        );
    }

    #[test]
    fn capture_commands_accept_all_explicit_input_selections() {
        for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
            assert!(ensure_audio_input_available(input).is_ok());
        }
    }

    #[test]
    fn every_proxy_mode_is_camel_case_and_requires_a_stopped_session() {
        for value in [
            serde_json::json!({"mode":"system"}),
            serde_json::json!({"mode":"direct"}),
            serde_json::json!({"mode":"custom","url":"http://127.0.0.1:8888"}),
        ] {
            let draft: SettingsDraft =
                serde_json::from_value(serde_json::json!({"networkProxy":value})).unwrap();
            assert!(draft.network_proxy.is_some());
            assert_eq!(
                ensure_settings_draft_allowed(&draft, true),
                Err("network_proxy_change_requires_stop".into())
            );
            assert!(ensure_settings_draft_allowed(&draft, false).is_ok());
        }
        assert!(SettingsDraft::default().network_proxy.is_none());
    }

    #[test]
    fn custom_color_draft_accepts_rgb_and_rejects_invalid_values() {
        let draft: SettingsDraft =
            serde_json::from_str(r##"{"subtitleColor":"#a1b2c3"}"##).unwrap();
        assert_eq!(
            draft.subtitle_color,
            Some(SubtitleColor::Custom([0xa1, 0xb2, 0xc3]))
        );
        assert!(ensure_settings_draft_allowed(&draft, true).is_ok());
        assert!(serde_json::from_str::<SettingsDraft>(r##"{"subtitleColor":"#abc"}"##).is_err());
    }

    #[test]
    fn active_session_rejects_pipeline_settings_but_allows_visual_settings() {
        let pipeline = SettingsDraft {
            source_language: Some(SourceLanguage::Japanese),
            ..SettingsDraft::default()
        };
        assert!(ensure_settings_draft_allowed(&pipeline, true).is_err());

        let visual = SettingsDraft {
            font_size: Some(19.0),
            subtitle_font_family: Some("Noto Sans CJK SC".into()),
            subtitle_background_opacity: Some(65),
            subtitle_color: Some(SubtitleColor::Custom([0x12, 0x34, 0x56])),
            subtitle_alignment: Some(SubtitleAlignment::Right),
            subtitle_display_mode: Some(SubtitleDisplayMode::Bilingual),
            show_subtitle_dividers: Some(true),
            pulse_animation: Some(true),
            pulse_style: Some(PulseStyle::Syllable),
            subtitle_animation: Some(true),
            subtitle_blends_with_background: Some(true),
            is_overlay_locked: Some(true),
            ui_language: Some("ja".into()),
            ..SettingsDraft::default()
        };
        assert!(ensure_settings_draft_allowed(&visual, true).is_ok());
    }

    #[test]
    fn one_unavailable_credential_does_not_fail_the_snapshot() {
        let store = SettingsStore::in_memory(Box::new(PartiallyUnavailableSecretStore), false);
        store
            .create_profile(ProviderKind::OpenAIRealtime, "OpenAI")
            .unwrap();

        let payload = SettingsSnapshotPayload::try_from_store(&store).unwrap();
        assert_eq!(payload.profiles.len(), 2);
        assert_eq!(
            payload.profiles[0].credential_state,
            CredentialState::Unavailable
        );
        assert_eq!(
            payload.profiles[1].credential_state,
            CredentialState::Missing
        );
    }

    #[test]
    fn settings_navigation_target_accepts_only_known_sections() {
        assert_eq!(
            serde_json::from_str::<SettingsNavigationTarget>(r#""service""#).unwrap(),
            SettingsNavigationTarget::Service
        );
        assert_eq!(
            serde_json::from_str::<SettingsNavigationTarget>(r#""export""#).unwrap(),
            SettingsNavigationTarget::Export
        );
        assert_eq!(
            serde_json::from_str::<SettingsNavigationTarget>(r#""appleSpeechResources""#).unwrap(),
            SettingsNavigationTarget::AppleSpeechResources
        );
        assert_eq!(
            serde_json::from_str::<SettingsNavigationTarget>(r#""activeProfile""#).unwrap(),
            SettingsNavigationTarget::ActiveProfile
        );
        assert!(serde_json::from_str::<SettingsNavigationTarget>(r#""general""#).is_err());
    }

    #[test]
    fn normal_quit_exits_only_after_successful_finalization_and_sanitizes_failures() {
        let exited = std::cell::Cell::new(false);
        let failure = std::io::Error::other("synthetic private failure detail");
        assert_eq!(
            finish_quit(Err(failure), || exited.set(true)),
            Err("Could not save session history before quitting.".to_string())
        );
        assert!(!exited.get());
        assert_eq!(finish_quit(Ok(()), || exited.set(true)), Ok(()));
        assert!(exited.get());
    }

    #[test]
    fn simultaneous_quit_requests_share_one_save_and_failed_save_can_retry() {
        let gate = QuitRequestGate(AtomicBool::new(false));
        let exited = std::cell::Cell::new(false);
        let failed = (|| {
            let request = gate.begin().expect("first request accepted");
            assert!(gate.begin().is_none());
            finish_quit(Err(std::io::Error::other("synthetic failure")), || {
                exited.set(true)
            })?;
            request.exit_requested();
            Ok::<(), String>(())
        })();
        assert!(failed.is_err());
        assert!(!exited.get());
        let retry = gate.begin().expect("save failure permits a retry");
        finish_quit(Ok(()), || exited.set(true)).unwrap();
        retry.exit_requested();
        assert!(exited.get());
        assert!(
            gate.begin().is_none(),
            "exit already queued; do not save again"
        );
    }

    #[tokio::test]
    async fn cancelled_quit_future_releases_the_request_gate() {
        let gate = Arc::new(QuitRequestGate(AtomicBool::new(false)));
        let task_gate = Arc::clone(&gate);
        let (started, received) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _request = task_gate.begin().unwrap();
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        received.await.unwrap();
        assert!(gate.begin().is_none());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(gate.begin().is_some());
    }
}

impl SettingsSnapshotPayload {
    pub fn from_store(store: &SettingsStore) -> Self {
        match Self::try_from_store(store) {
            Ok(payload) => payload,
            Err(_) => {
                let prefs = store.preferences();
                let (active_profile_id, profiles) = store.profile_catalog_or_default();
                Self {
                    credential_storage: store.credential_storage(),
                    profiles: profiles
                        .into_iter()
                        .map(ServiceProfilePayload::unavailable)
                        .collect(),
                    active_profile_id,
                    language_capabilities: None,
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

                    pulse_animation: prefs.pulse_animation,
                    pulse_style: prefs.pulse_style,

                    subtitle_animation: prefs.subtitle_animation,
                    subtitle_blends_with_background: prefs.subtitle_blends_with_background,
                    is_overlay_locked: prefs.overlay_locked,
                    ui_language: prefs.ui_language,
                    retain_session_history: prefs.retain_session_history,
                    record_session_audio: prefs.record_session_audio,
                    audio_input: prefs.audio_input,
                    microphone_input_available:
                        crate::core::audio_input::MICROPHONE_INPUT_AVAILABLE,
                    windows_audio_source: prefs.windows_audio_source,
                    system_audio_target: prefs.system_audio_target,
                    show_in_dock: prefs.show_in_dock,
                    network_proxy: prefs.network_proxy,
                }
            }
        }
    }

    pub fn try_from_store(store: &SettingsStore) -> Result<Self, String> {
        let (prefs, active_profile_id, profiles) = store.preferences_and_catalog()?;
        let language_capabilities = profiles
            .iter()
            .find(|profile| profile.id == active_profile_id)
            .map(|profile| {
                LanguageCapabilitiesPayload::from_profile(profile, prefs.target_language)
            });
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

            pulse_animation: prefs.pulse_animation,
            pulse_style: prefs.pulse_style,

            subtitle_animation: prefs.subtitle_animation,
            subtitle_blends_with_background: prefs.subtitle_blends_with_background,
            is_overlay_locked: prefs.overlay_locked,
            ui_language: prefs.ui_language,
            retain_session_history: prefs.retain_session_history,
            record_session_audio: prefs.record_session_audio,
            audio_input: prefs.audio_input,
            microphone_input_available: crate::core::audio_input::MICROPHONE_INPUT_AVAILABLE,
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
    pub system_audio_target: Option<crate::core::system_audio_target::SystemAudioTarget>,
    pub show_in_dock: Option<bool>,
    pub network_proxy: Option<ProxyConfig>,
}

/// Reads public settings and per-profile credential presence. API-key values
/// never enter this payload. Async keeps OS credential-store reads off wry's
/// main-thread path.
#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> Result<SettingsSnapshotPayload, String> {
    if !app_is_ui_test()
        && !crate::apple_speech_support::is_loaded()
        && state
            .settings
            .active_profile()
            .is_ok_and(|profile| profile.provider == ProviderKind::AppleSpeech)
    {
        // Query runtime support once for an already selected local recognizer.
        // Failure leaves an empty language list and is surfaced by its editor.
        let _ = crate::apple_speech_support::refresh().await;
    }
    if !app_is_ui_test()
        && !crate::apple_translation_support::is_loaded()
        && state
            .settings
            .active_profile()
            .is_ok_and(|profile| profile.text_translation() == TextTranslation::Apple)
    {
        let _ = crate::apple_translation_support::refresh().await;
    }
    Ok(SettingsSnapshotPayload::from_store(&state.settings))
}

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

async fn ensure_apple_provider_available(provider: ProviderKind) -> Result<(), String> {
    if provider == ProviderKind::AppleSpeech
        && (app_is_ui_test() || !crate::apple_speech_support::refresh().await?.available)
    {
        return Err("apple_speech_unavailable".into());
    }
    Ok(())
}

/// Lets the frontend select a deterministic updater adapter during native UI
/// tests. The production updater never receives this flag or fixture data.
#[tauri::command]
pub fn app_is_ui_test() -> bool {
    std::env::var("MIMI_UI_TEST").as_deref() == Ok("1")
}

/// A native smoke test must observe a mounted frontend, not just a window title.
/// Neither the destination nor marker contents can be supplied by the frontend.
#[tauri::command]
pub fn app_ui_test_frontend_ready(window: tauri::WebviewWindow) -> Result<(), String> {
    write_ui_test_frontend_ready(
        app_is_ui_test(),
        window.label(),
        std::env::var_os("MIMI_UI_TEST_FRONTEND_READY_DIR").as_deref(),
    )
    .map_err(|_| "Could not write the UI-test readiness marker.".to_string())
}

fn write_ui_test_frontend_ready(
    is_ui_test: bool,
    label: &str,
    directory: Option<&std::ffi::OsStr>,
) -> std::io::Result<()> {
    if !is_ui_test || !matches!(label, "settings" | "overlay") {
        return Ok(());
    }
    if let Some(directory) = directory {
        let directory = std::path::Path::new(directory);
        std::fs::create_dir_all(directory)?;
        std::fs::write(directory.join(label), b"ready\n")?;
    }
    Ok(())
}

/// The Windows ZIP carries this marker beside the executable. It identifies
/// an extract-and-run copy before Settings can offer an NSIS update.
#[tauri::command]
pub fn app_is_portable() -> bool {
    #[cfg(target_os = "windows")]
    {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("mimi.portable")))
            .is_some_and(|marker| marker.is_file())
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Tauri can replace an AppImage, but cannot update a package-manager install.
#[tauri::command]
pub fn app_is_linux_package() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none_or(|path| path.is_empty())
}

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
    if draft.show_in_dock.is_some() && !matches!(label, "settings" | "tray-panel") {
        return Err(
            "The Dock preference can only be changed in settings or the tray panel.".into(),
        );
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
            if let Some(source) = draft.windows_audio_source {
                prefs.windows_audio_source = source;
            }
            prefs.apply_audio_preferences(draft.audio_input, draft.record_session_audio);
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
    crate::mac_dock::save_with_policy(
        state.settings.preferences().show_in_dock,
        draft.show_in_dock,
        |show| crate::mac_dock::apply(app, show),
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
    let recording =
        (target_changed || draft.audio_input.is_some() || draft.record_session_audio.is_some())
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
        crate::refresh_native_tray_language(app);
    }

    let payload = SettingsSnapshotPayload::try_from_store(&state.settings)?;
    let _ = app.emit("settings-changed", payload.clone());
    Ok(payload)
}

/// Serializes a native Dock toggle with settings and tray-panel saves.
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

fn ensure_profile_mutation_allowed(is_active: bool) -> Result<(), String> {
    if is_active {
        Err("Service profiles cannot be changed while a session is active.".to_string())
    } else {
        Ok(())
    }
}

fn emit_settings_snapshot(
    app: &AppHandle,
    settings: &SettingsStore,
) -> Result<SettingsSnapshotPayload, String> {
    sync_overlay_minimum(app);
    let payload = SettingsSnapshotPayload::try_from_store(settings)?;
    let _ = app.emit("settings-changed", payload.clone());
    Ok(payload)
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
    state.settings.create_profile(provider, &name)?;
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
    language_preset_patch: Option<crate::core::provider::ProfileLanguagePresetPatch>,
) -> Result<SettingsSnapshotPayload, String> {
    let _lifecycle = state.session.settings_mutation_guard(true).await?;
    ensure_profile_mutation_allowed(state.session.has_active_session())?;
    let previous_profile = state.settings.active_profile()?;
    let changes_active_speech_route = previous_profile.id == profile_id
        && speech_network_proxy
            .as_ref()
            .is_some_and(|proxy| previous_profile.speech_network_proxy.as_ref() != Some(proxy));
    let configuration_failure = state.session.configuration_failure_snapshot();
    state.settings.update_profile_options(
        &profile_id,
        name.as_deref(),
        speech_network_proxy,
        text_network_proxy,
        text_translation_name,
        speech_recognition_name.as_deref(),
        custom_speech_languages_patch,
        language_preset_patch,
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
    text_translation: Option<crate::core::provider::TextTranslation>,
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
/// OS credential access runs off the IPC/main thread; no event is emitted.
#[tauri::command]
pub async fn profile_reveal_credential(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    profile_id: String,
    field: CredentialRevealField,
    text_translation: Option<crate::core::provider::TextTranslation>,
) -> Result<Option<String>, String> {
    ensure_credential_reveal_window(window.label())?;
    let settings = Arc::clone(&state.settings);
    tauri::async_runtime::spawn_blocking(move || {
        settings.reveal_credential(&profile_id, field, text_translation)
    })
    .await
    .map_err(|_| "credential_store_unavailable".to_string())?
}

#[tauri::command]
pub async fn session_start(state: State<'_, AppState>) -> Result<(), String> {
    ensure_audio_input_available(state.settings.preferences().audio_input)?;
    state.session.start().await
}

#[tauri::command]
pub async fn session_stop(state: State<'_, AppState>) -> Result<(), String> {
    state.session.stop().await;
    Ok(())
}

#[tauri::command]
pub async fn session_toggle_paused(state: State<'_, AppState>) -> Result<(), String> {
    state.session.toggle_paused().await
}

#[tauri::command]
pub async fn session_clear_subtitles(state: State<'_, AppState>) -> Result<(), String> {
    // Clear from the tray/overlay must invalidate export snapshots atomically
    // with their acquisition and final write, just like settings-page clear.
    let _lifecycle = state.session.settings_mutation_guard(false).await?;
    state
        .session
        .clear_subtitles()
        .await
        .map_err(|_| "Could not clear subtitles.")?;
    Ok(())
}

#[tauri::command]
pub async fn session_switch_audio_input(
    state: State<'_, AppState>,
    input: AudioInput,
) -> Result<(), String> {
    ensure_audio_input_available(input)?;
    state.session.switch_audio_input(input).await
}

fn ensure_audio_input_available(input: AudioInput) -> Result<(), String> {
    if input.is_available() {
        Ok(())
    } else {
        Err("microphone_input_unavailable".into())
    }
}

#[tauri::command]
pub async fn session_switch_system_audio_target(
    state: State<'_, AppState>,
    target: crate::core::system_audio_target::SystemAudioTarget,
) -> Result<(), String> {
    state.session.switch_system_audio_target(target).await
}

#[tauri::command]
pub async fn session_switch_source_language(
    state: State<'_, AppState>,
    language: SourceLanguage,
) -> Result<(), String> {
    // The session manager broadcasts settings-changed immediately after the
    // preference write, so no window keeps a stale selection while the
    // reconnect (which this awaits) is still in flight.
    state.session.switch_source_language(language).await
}

#[tauri::command]
pub async fn session_switch_target_language(
    state: State<'_, AppState>,
    language: TargetLanguage,
) -> Result<(), String> {
    state.session.switch_target_language(language).await
}

#[tauri::command]
pub async fn session_switch_translation_mode(
    state: State<'_, AppState>,
    mode: TranslationMode,
) -> Result<(), String> {
    state.session.switch_translation_mode(mode).await;
    Ok(())
}

#[tauri::command]
pub fn overlay_set_collapsed(
    app: AppHandle,
    state: State<'_, AppState>,
    collapsed: bool,
) -> Result<(), String> {
    sync_overlay_minimum(&app);
    let preferences = state.settings.preferences();
    let collapsed =
        normalize_overlay_collapsed(collapsed, preferences.subtitle_blends_with_background);
    state.session.set_overlay_collapsed(collapsed);
    OverlayWindowManager::set_collapsed(&app, &state.overlay, &state.settings, collapsed);
    OverlayWindowManager::sync_presentation(
        &app,
        state.session.should_show_overlay(),
        collapsed,
        preferences.overlay_locked || preferences.subtitle_blends_with_background,
        preferences.subtitle_blends_with_background,
    );
    // The frontend's collapse UI state only updates through the
    // session-state event; without this the overlay renders the wrong
    // layout after collapsing/expanding.
    state.session.publish_state();
    Ok(())
}

fn normalize_overlay_collapsed(requested: bool, background_blend: bool) -> bool {
    requested && !background_blend
}

#[tauri::command]
pub fn overlay_set_locked(
    app: AppHandle,
    state: State<'_, AppState>,
    locked: bool,
) -> Result<(), String> {
    state
        .settings
        .save_preferences(|prefs| prefs.overlay_locked = locked)?;
    let preferences = state.settings.preferences();
    OverlayWindowManager::sync_presentation(
        &app,
        state.session.should_show_overlay(),
        state.session.is_overlay_collapsed(),
        locked || preferences.subtitle_blends_with_background,
        preferences.subtitle_blends_with_background,
    );
    let _ = app.emit(
        "settings-changed",
        SettingsSnapshotPayload::from_store(&state.settings),
    );
    Ok(())
}

fn ensure_overlay_pointer_cursor_window(label: &str) -> Result<(), String> {
    if label == "overlay" {
        Ok(())
    } else {
        Err("overlay_pointer_cursor_not_allowed".into())
    }
}

/// Applies a renderer hit-test result for the last native hover sample only.
/// The inactive macOS WebView cannot rely on normal cursor-update tracking.
#[tauri::command]
pub async fn overlay_set_pointer_cursor(
    app: AppHandle,
    window: tauri::WebviewWindow,
    x: f64,
    y: f64,
    pointing: bool,
) -> Result<bool, String> {
    ensure_overlay_pointer_cursor_window(window.label())?;
    #[cfg(target_os = "macos")]
    {
        crate::windows::set_overlay_pointer_cursor(
            &app,
            crate::core::overlay_pointer::OverlayPointerPosition { x, y },
            pointing,
        )
        .await
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Other platforms use their native WebView cursor handling and do
        // not install the inactive-panel pointer relay.
        let _ = (app, x, y, pointing);
        Ok(false)
    }
}

#[tauri::command]
pub fn overlay_show(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if state.session.should_show_overlay() {
        let preferences = state.settings.preferences();
        OverlayWindowManager::sync_presentation(
            &app,
            true,
            state.session.is_overlay_collapsed(),
            preferences.overlay_locked || preferences.subtitle_blends_with_background,
            preferences.subtitle_blends_with_background,
        );
        OverlayWindowManager::follow_active_space(&app, &state.overlay, &state.settings);
    }
    Ok(())
}

/// Marks an explicit user drag before Tauri hands the gesture to AppKit.
#[tauri::command]
pub fn overlay_move_start(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if !OverlayWindowManager::move_start(&app, &state.overlay) {
        return Ok(());
    }
    if window.start_dragging().is_err() {
        OverlayWindowManager::move_cancel(&state.overlay);
        return Err("Could not start overlay drag.".to_string());
    }
    Ok(())
}

/// Toggles the child overlay control between its compact island and expanded
/// panel. The legacy command name remains part of the IPC contract.
#[tauri::command]
pub fn overlay_popover_toggle(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    OverlayControlWindowManager::toggle_panel(&app);
    // Refresh snapshots after a hidden/reloaded WebView so its checkmarks and
    // lifecycle guards never depend solely on an older broadcast.
    state.session.publish_settings();
    state.session.publish_state();
    Ok(())
}

/// Returns the expanded panel to its compact island. The legacy command name
/// is kept for backwards-compatible frontend bundles.
#[tauri::command]
pub fn overlay_popover_hide(app: AppHandle) -> Result<(), String> {
    OverlayControlWindowManager::dismiss_panel(&app);
    Ok(())
}

/// Current native mode for initial control-window hydration after a reload.
#[tauri::command]
pub fn overlay_control_state(app: AppHandle) -> Result<OverlayControlMode, String> {
    Ok(OverlayControlWindowManager::mode(&app))
}

/// Applies a tightly-fitted panel height measured by the control WebView.
#[tauri::command]
pub fn overlay_control_set_panel_height(app: AppHandle, height: f64) -> Result<(), String> {
    OverlayControlWindowManager::set_panel_height(&app, height);
    Ok(())
}

/// Applies the collapsed capsule's content width independently of its panel.
#[tauri::command]
pub fn overlay_control_set_island_width(app: AppHandle, width: f64) -> Result<(), String> {
    OverlayControlWindowManager::set_island_width(&app, width);
    Ok(())
}

/// The current session state snapshot, for windows that boot after the last
/// broadcast (e.g. the overlay control). Async: cloning the full controller
/// state (subtitle history included) must not run on the main thread.
#[tauri::command]
pub async fn session_get_state(state: State<'_, AppState>) -> Result<SessionStateEvent, String> {
    Ok(state.session.current_state_event())
}

/// Begins an overlay resize drag. `region` is one of topLeft/top/topRight/
/// left/right/bottomLeft/bottom/bottomRight; `x`/`y` are the pointer position
/// in screen (logical) coordinates. Ignored unless the overlay is expanded.
#[tauri::command]
pub fn resize_start(
    app: AppHandle,
    state: State<'_, AppState>,
    region: String,
    x: f64,
    y: f64,
) -> Result<(), String> {
    OverlayWindowManager::resize_start(&app, &state.overlay, &region, x, y)
}

/// Continues a resize drag with the current pointer position (screen logical
/// px). The dragged edge/corner stays anchored, sizes clamp to min/max, and
/// the frame keeps at least 48 px on screen.
#[tauri::command]
pub fn resize_move(
    app: AppHandle,
    state: State<'_, AppState>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    OverlayWindowManager::resize_move(&app, &state.overlay, x, y);
    Ok(())
}

/// Ends a resize drag and commits the final frame.
#[tauri::command]
pub fn resize_end(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    OverlayWindowManager::resize_end(&app, &state.overlay, &state.settings);
    Ok(())
}

#[tauri::command]
pub fn tray_panel_hide(app: AppHandle) -> Result<(), String> {
    TrayPanelManager::hide(&app);
    Ok(())
}

fn emit_settings_navigation(app: &AppHandle, target: SettingsNavigationTarget) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.emit(SETTINGS_NAVIGATION_EVENT, target);
    }
}

#[tauri::command]
pub fn app_show_settings(
    app: AppHandle,
    target: Option<SettingsNavigationTarget>,
) -> Result<(), String> {
    // Close the tray panel first: it is always-on-top, so the settings
    // window would otherwise open behind it and the click would look like a
    // no-op. Collapse the overlay control panel for the same reason.
    TrayPanelManager::hide(&app);
    OverlayControlWindowManager::dismiss_panel(&app);

    // The settings window normally exists for the whole app lifetime and is
    // merely hidden on close. If its webview crashed, however, recreating it
    // and emitting immediately would race the frontend listener. In that
    // recovery path, wait for SettingsView to announce that its navigation
    // listener is installed before delivering the one-shot intent.
    let settings_window_exists = app.get_webview_window("settings").is_some();
    if !settings_window_exists {
        if let Some(target) = target {
            let ready_app = app.clone();
            app.once(SETTINGS_NAVIGATION_READY_EVENT, move |_| {
                emit_settings_navigation(&ready_app, target);
            });
        }
    }

    crate::windows::ensure_settings_window(&app);
    if settings_window_exists {
        if let Some(target) = target {
            emit_settings_navigation(&app, target);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn app_quit(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    quit_application(app, Arc::clone(&state.session)).await
}

/// Every normal exit uses the same stop/finalize boundary. A failed save keeps
/// the pending archive and app alive so the user can retry rather than lose it.
pub async fn quit_application(app: AppHandle, session: Arc<SessionManager>) -> Result<(), String> {
    let Some(request) = NORMAL_QUIT_GATE.begin() else {
        return Ok(());
    };
    session.stop().await;
    #[cfg(any(test, feature = "development-debugger"))]
    crate::development_debugger::finish_on_quit(app.clone()).await?;
    finish_quit(session.persist_current_history(), || app.exit(0))?;
    request.exit_requested();
    Ok(())
}

static NORMAL_QUIT_GATE: QuitRequestGate = QuitRequestGate(AtomicBool::new(false));

/// Share one stop/save across simultaneous Dock, menu, tray and IPC requests.
/// Failed or cancelled futures release the gate; successful exit keeps it shut.
struct QuitRequestGate(AtomicBool);

impl QuitRequestGate {
    fn begin(&self) -> Option<QuitRequest<'_>> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| QuitRequest {
                gate: self,
                exiting: false,
            })
    }
}

struct QuitRequest<'a> {
    gate: &'a QuitRequestGate,
    exiting: bool,
}

impl QuitRequest<'_> {
    fn exit_requested(mut self) {
        self.exiting = true;
    }
}

impl Drop for QuitRequest<'_> {
    fn drop(&mut self) {
        if !self.exiting {
            self.gate.0.store(false, Ordering::Release);
        }
    }
}

fn finish_quit(finalization: std::io::Result<()>, exit: impl FnOnce()) -> Result<(), String> {
    finalization.map_err(|_| "Could not save session history before quitting.".to_string())?;
    exit();
    Ok(())
}

/// Explicit authorization/readiness check. No capture or current-session changes.
#[tauri::command]
pub async fn profile_test_connection(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    profile_id: String,
    stage: Option<crate::clients::connection_diagnostics::ConnectionCheckStage>,
    credentials: Option<ProviderCredentials>,
    source_language: Option<SourceLanguage>,
) -> Result<crate::clients::connection_diagnostics::ConnectionDiagnostic, String> {
    use crate::clients::connection_diagnostics::{
        check_service, check_speech_service, check_text_service, preparation_failure,
        ConnectionCheckReason, ConnectionCheckStage, ConnectionDiagnostic,
    };
    let (_, profiles) = state.settings.profile_catalog()?;
    let profile = profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or("profile_not_found")?;
    if source_language.is_some()
        && (profile.provider != ProviderKind::AppleSpeech
            || stage != Some(ConnectionCheckStage::Speech)
            || credentials.is_some())
    {
        return Err("apple_speech_source_override_invalid".into());
    }
    if let Some(credentials) = credentials {
        // A draft is an explicit, ephemeral check, never a saved-profile
        // readiness request. In particular it must not emit a credential snapshot.
        return Ok(match stage {
            Some(ConnectionCheckStage::Text) => match state
                .settings
                .configuration_for_text_draft_probe(profile, &credentials)
            {
                Err(error) => preparation_failure(&error),
                Ok(_) if app_is_ui_test() => ConnectionDiagnostic::not_tested("present"),
                Ok(configuration) => check_text_service(&configuration).await,
            },
            stage => match state.settings.configuration_for_speech_draft_probe(
                profile,
                &credentials,
                stage.is_some(),
            ) {
                Err(error) => preparation_failure(&error),
                Ok(_) if app_is_ui_test() => ConnectionDiagnostic::not_tested("present"),
                Ok(configuration) => check_speech_service(&configuration, stage.is_none()).await,
            },
        });
    }
    if let Some(stage) = stage {
        // Stage checks deliberately avoid an aggregate credential snapshot: a
        // text-only check must not prompt for or require the recognizer's key.
        let diagnostic = match stage {
            ConnectionCheckStage::Speech => {
                let configuration = match source_language {
                    Some(source) => state
                        .settings
                        .configuration_for_speech_probe_with_source(profile, Some(source)),
                    None => state.settings.configuration_for_speech_probe(profile),
                };
                match configuration {
                    Err(error) => preparation_failure(&error),
                    Ok(_) if app_is_ui_test() => ConnectionDiagnostic::not_tested("present"),
                    Ok(configuration) => check_speech_service(&configuration, false).await,
                }
            }
            ConnectionCheckStage::Text => {
                match state.settings.configuration_for_text_probe(profile) {
                    Err(error) => preparation_failure(&error),
                    Ok(_) if app_is_ui_test() => ConnectionDiagnostic::not_tested("present"),
                    Ok(configuration) => check_text_service(&configuration).await,
                }
            }
        };
        if stage == ConnectionCheckStage::Speech && profile.provider == ProviderKind::AppleSpeech {
            // The native check refreshes resource status even on failure. Keep
            // its ready/unknown result in sync with the settings resource row.
            emit_settings_snapshot(&app, &state.settings)?;
        }
        return Ok(diagnostic);
    }
    let storage = state.settings.credential_diagnostic(profile);
    emit_settings_snapshot(&app, &state.settings)?;
    if let Some(failure) = ConnectionDiagnostic::credential_failure(storage) {
        return Ok(failure);
    }
    if app_is_ui_test() {
        return Ok(ConnectionDiagnostic::not_tested(storage));
    }
    let configuration = match state.settings.configuration_for_profile_probe(profile) {
        Ok(configuration) => configuration,
        Err(_) => {
            return Ok(ConnectionDiagnostic::unavailable(
                storage,
                ConnectionCheckReason::InvalidConfiguration,
            ));
        }
    };
    let diagnostic = check_service(&configuration).await;
    if profile.provider == ProviderKind::AppleSpeech {
        emit_settings_snapshot(&app, &state.settings)?;
    }
    Ok(diagnostic)
}

#[tauri::command]
pub async fn windows_audio_status(
    state: State<'_, AppState>,
) -> Result<Option<crate::audio::AudioSourceSnapshot>, String> {
    state.session.windows_audio_status()
}

/// Read-only audio census: render endpoints with levels, plus the application
/// sessions currently attached to them. Never cached; callers poll.
#[tauri::command]
pub async fn audio_census() -> crate::audio::census::AudioCensus {
    if app_is_ui_test() {
        return crate::audio::census::AudioCensus::default();
    }
    crate::audio::census::census()
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

#[tauri::command]
pub async fn capture_status(
    state: State<'_, AppState>,
) -> Result<crate::audio::CaptureStatus, String> {
    Ok(state.session.capture_status())
}

/// Listing does not start capture or read provider credentials. UI-test mode
/// returns synthetic choices and never invokes OS capture APIs.
#[tauri::command]
pub async fn audio_applications(
    state: State<'_, AppState>,
) -> Result<crate::audio::applications::ApplicationSnapshot, String> {
    if app_is_ui_test() {
        return Ok(crate::audio::applications::ApplicationSnapshot {
            supported: true,
            applications: vec![crate::audio::applications::AudioApplication {
                id: "test.player".into(),
                name: "Test Player".into(),
                icon_data_url: None,
            }],
        });
    }
    state
        .session
        .audio_applications()
        .await
        .map_err(|error| error.to_string())
}
