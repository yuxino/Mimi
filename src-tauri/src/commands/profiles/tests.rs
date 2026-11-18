//! Profiles command behavior and capability regressions.

use super::super::settings::LanguageCapabilitiesPayload;
use super::super::tests::PartiallyUnavailableSecretStore;
use super::*;
use crate::core::models::TargetLanguage;

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
    let capabilities = LanguageCapabilitiesPayload::from_profile(&profile, TargetLanguage::German);
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
fn active_session_rejects_every_profile_mutation() {
    assert!(ensure_profile_mutation_allowed(true).is_err());
    assert!(ensure_profile_mutation_allowed(false).is_ok());
}
