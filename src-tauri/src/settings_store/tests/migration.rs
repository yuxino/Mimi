//! Migration behavior regressions.

use super::super::profiles::LEGACY_ENV_ALIBABA_PROFILE_ID;
use super::*;

#[test]
fn development_ignores_env_and_keeps_saved_credentials_editable_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let env = directory.path().join(".env");
    std::fs::write(&env, "ALIBABA_API_KEY=synthetic-unused\ninvalid-line").unwrap();
    let load = || {
        SettingsStore::load(
            directory.path().into(),
            false,
            DEVELOPMENT_APPLICATION_IDENTIFIER,
        )
    };
    let store = load();
    assert_eq!(store.credential_storage(), "localFile");
    assert!(store.profile_catalog().unwrap().1.is_empty());
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Editable Alibaba")
        .unwrap();
    assert_eq!(store.credential_state(&profile), CredentialState::Missing);
    store
        .save_api_key(&profile.id, "synthetic-saved-key")
        .unwrap();
    store
        .update_profile(&profile.id, "Renamed Alibaba")
        .unwrap();
    drop(store);
    let reopened = load();
    let profile = reopened.active_profile().unwrap();
    assert_eq!(profile.name, "Renamed Alibaba");
    assert_eq!(
        reopened.credential_state(&profile),
        CredentialState::Present
    );
    assert_eq!(
        reopened.configuration().unwrap().credentials.alibaba_key(),
        Some("synthetic-saved-key")
    );
    reopened
        .save_api_key(&profile.id, "synthetic-replacement")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(env).unwrap(),
        "ALIBABA_API_KEY=synthetic-unused\ninvalid-line"
    );
}

#[test]
fn former_env_profiles_are_ordinary_editable_profiles_without_native_import() {
    let directory = tempfile::tempdir().unwrap();
    let profile = ServiceProfile::new(
        LEGACY_ENV_ALIBABA_PROFILE_ID,
        "Alibaba Cloud",
        ProviderKind::AlibabaCloud,
    )
    .unwrap();
    let catalog = ProfileCatalog {
        active_profile_id: profile.id.clone(),
        profiles: vec![profile.clone()],
        ..ProfileCatalog::default()
    };
    std::fs::write(
        directory.path().join(PROFILE_CATALOG_FILE),
        serde_json::to_vec(&catalog).unwrap(),
    )
    .unwrap();
    let store = SettingsStore::load(
        directory.path().into(),
        false,
        DEVELOPMENT_APPLICATION_IDENTIFIER,
    );
    assert_eq!(store.profile_catalog().unwrap().1, vec![profile.clone()]);
    assert_eq!(store.credential_storage(), "localFile");
    store
        .save_api_key(&profile.id, "synthetic-former-preset-key")
        .unwrap();
    store.update_profile(&profile.id, "My Alibaba").unwrap();
    assert_eq!(
        store.configuration().unwrap().credentials.alibaba_key(),
        Some("synthetic-former-preset-key")
    );
    assert_eq!(store.active_profile().unwrap().name, "My Alibaba");
    assert_eq!(store.secret.pending_imports().unwrap(), 0);
}

#[test]
fn pre_catalog_preferences_preserve_the_legacy_alibaba_profile() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("preferences.json"),
        serde_json::to_vec(&Preferences::default()).unwrap(),
    )
    .unwrap();
    let store = fresh_store(directory.path(), &FakeSecretStore::default());
    assert_eq!(
        store.active_profile().unwrap().id,
        DEFAULT_ALIBABA_PROFILE_ID
    );
    assert_eq!(store.profile_catalog().unwrap().1.len(), 1);
}

#[test]
fn profile_stage_proxies_persist_and_legacy_routes_remain_isolated() {
    use crate::core::services::network_proxy::ProxyMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let mut store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let profile = store.active_profile().unwrap();
    store.save_api_key(&profile.id, "synthetic-asr").unwrap();
    let inherited = ProxyConfig {
        mode: ProxyMode::Custom,
        url: Some("http://127.0.0.1:7890/".into()),
    };
    store
        .save_preferences_for_active_profile(|prefs| prefs.network_proxy = inherited.clone())
        .unwrap();
    let previous = store.configuration().unwrap();
    let text = ProxyConfig {
        mode: ProxyMode::Direct,
        url: None,
    };
    store
        .update_profile_options(
            &profile.id,
            None,
            None,
            Some(text.clone()),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let updated = store.active_profile().unwrap();
    assert_eq!(updated.speech_network_proxy, None);
    assert_eq!(store.configuration().unwrap().network_proxy, inherited);
    assert_eq!(store.configuration().unwrap().text_network_proxy, text);
    assert_eq!(
        store
            .configuration_for_text_probe(&updated)
            .unwrap()
            .network_proxy,
        text
    );
    assert_eq!(
        store
            .configuration_for_speech_probe(&updated)
            .unwrap()
            .network_proxy,
        inherited
    );
    assert_eq!(previous.text_network_proxy, inherited);
    let other = store
        .create_profile(ProviderKind::AlibabaCloud, "Other")
        .unwrap();
    store.save_api_key(&other.id, "synthetic-other").unwrap();
    store.select_profile(&other.id).unwrap();
    assert_eq!(store.configuration().unwrap().text_network_proxy, inherited);
    let speech = ProxyConfig {
        mode: ProxyMode::Custom,
        url: Some("socks5h://127.0.0.1:1080".into()),
    };
    store
        .update_profile_options(
            &profile.id,
            Some("Renamed"),
            Some(speech.clone()),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    assert_eq!(reloaded.configuration().unwrap().network_proxy, speech);
    assert_eq!(reloaded.configuration().unwrap().text_network_proxy, text);
    assert_eq!(
        reloaded
            .configuration_for_profile_probe(&reloaded.active_profile().unwrap())
            .unwrap()
            .text_network_proxy,
        text
    );
    assert_eq!(reloaded.preferences().network_proxy, inherited);
    let before = store.active_profile().unwrap();
    let bytes = std::fs::read(&store.catalog_path).unwrap();
    assert!(store
        .update_profile_options(
            &profile.id,
            Some("Rejected"),
            None,
            Some(ProxyConfig {
                mode: ProxyMode::Custom,
                url: Some("http://user:private-value@localhost:7890".into())
            }),
            None,
            None,
            None,
            None,
        )
        .is_err());
    assert_eq!(store.active_profile().unwrap(), before);
    assert_eq!(std::fs::read(&store.catalog_path).unwrap(), bytes);
    store.catalog_path = directory.path().join("blocked-catalog");
    std::fs::create_dir(&store.catalog_path).unwrap();
    assert!(store
        .update_profile_options(
            &profile.id,
            Some("Unpersisted"),
            Some(ProxyConfig::default()),
            None,
            None,
            None,
            None,
            None,
        )
        .is_err());
    assert_eq!(store.active_profile().unwrap(), before);
    assert_eq!(reloaded.active_profile().unwrap(), before);
}

#[test]
fn audio_input_legacy_default_and_explicit_selection_round_trip() {
    let legacy: super::Preferences = serde_json::from_str("{}").unwrap();
    assert_eq!(legacy.audio_input, AudioInput::System);
    assert!(!legacy.record_session_audio);
    for input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
        let mut preferences = legacy.clone();
        preferences.apply_audio_preferences(Some(input), None);
        let restored: super::Preferences =
            serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
        assert_eq!(restored.audio_input, input);
        assert!(!restored.record_session_audio);
    }
}

#[test]
fn legacy_preferences_keep_both_export_options_off() {
    let preferences: super::Preferences = serde_json::from_str("{}").unwrap();
    assert!(!preferences.retain_session_history);
    assert!(!preferences.record_session_audio);
    let enabled = super::Preferences {
        retain_session_history: true,
        record_session_audio: true,
        ..Default::default()
    };
    let restored: super::Preferences =
        serde_json::from_str(&serde_json::to_string(&enabled).unwrap()).unwrap();
    assert!(restored.retain_session_history && restored.record_session_audio);
}

#[test]
fn legacy_combined_deeplx_text_probe_retries_its_item_after_unlocking() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::DeepLX, "Legacy combined")
        .unwrap();
    let account = credential_account(&profile);
    let value = ProviderCredentials::DeepLX {
        asr_api_key: "synthetic-asr".into(),
        endpoint: "https://translation.example/translate".into(),
        token: "synthetic-text".into(),
    }
    .encode_for_keychain(ProviderKind::DeepLX)
    .unwrap();
    fake.put(PROFILE_KEYCHAIN_SERVICE, &account, &value);
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
    assert_eq!(
        store.credential_state(&profile),
        CredentialState::Unavailable
    );
    assert!(store.configuration_for_text_probe(&profile).is_err());
    fake.state
        .lock()
        .unwrap()
        .unavailable
        .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &account));
    fake.state.lock().unwrap().loads.clear();
    let configuration = store.configuration_for_text_probe(&profile).unwrap();
    assert!(matches!(configuration.credentials,
            crate::core::services::configuration::TextTranslationProbeCredentials::Independent(
                TextTranslationCredentials::DeepLX { token, .. }
            ) if token == "synthetic-text"));
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
}

#[test]
fn legacy_custom_profiles_keep_both_destinations_without_rewriting_the_asr_item() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::DeepLX, "Existing")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &ProviderCredentials::DeepLX {
                asr_api_key: "synthetic-asr".into(),
                endpoint: "https://example.com/translate".into(),
                token: "synthetic-custom".into(),
            },
        )
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    let account = credential_account(&profile);
    let before = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
        )
        .unwrap();
    assert!(
        matches!(store.configuration().unwrap().credentials, ProviderCredentials::DeepL { api_key, .. } if api_key == "synthetic-deepl:fx")
    );
    for route in [
        TextTranslation::DeepLX,
        TextTranslation::FollowService,
        TextTranslation::DeepL,
        TextTranslation::DeepLX,
    ] {
        store
            .save_credentials(&profile.id, &translation_request(route, "", "", ""))
            .unwrap();
        match route {
            TextTranslation::DeepL => assert!(
                matches!(store.configuration().unwrap().credentials, ProviderCredentials::DeepL { api_key, .. } if api_key == "synthetic-deepl:fx")
            ),
            TextTranslation::DeepLX => assert!(
                matches!(store.configuration().unwrap().credentials, ProviderCredentials::DeepLX { endpoint, token, .. } if endpoint == "https://example.com/translate" && token == "synthetic-custom")
            ),
            TextTranslation::FollowService => assert_eq!(
                store.configuration().unwrap().credentials.direct_api_key(),
                Some("synthetic-asr")
            ),
            TextTranslation::XAI
            | TextTranslation::OpenAICompatible
            | TextTranslation::ChatMock
            | TextTranslation::Apple => unreachable!(),
        }
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
            Some(before.as_str())
        );
    }
}

#[test]
fn legacy_deeplx_profile_keeps_its_id_account_and_secret_on_default_loading() {
    let fake = FakeSecretStore::default();
    let store = SettingsStore::in_memory(Box::new(fake.clone()), false);
    let profile = store
        .create_profile(ProviderKind::DeepLX, "Existing custom name")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &ProviderCredentials::DeepLX {
                asr_api_key: "synthetic-asr".into(),
                endpoint: "https://example.com".into(),
                token: "synthetic-token".into(),
            },
        )
        .unwrap();
    let account = credential_account(&profile);
    let original = fake.value(PROFILE_KEYCHAIN_SERVICE, &account);
    store.select_profile(&profile.id).unwrap();
    assert_eq!(
        store.configuration().unwrap().provider,
        ProviderKind::DeepLX
    );
    assert_eq!(store.profile(&profile.id).unwrap().text_translation, None);
    assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), original);
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::FollowService, "", "", ""),
        )
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    assert_eq!(
        store.configuration().unwrap().provider,
        ProviderKind::AlibabaCloud
    );
    assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), original);
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepLX, "", "", ""),
        )
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    assert_eq!(
        store.configuration().unwrap().provider,
        ProviderKind::DeepLX
    );
    assert_eq!(
        store.profile(&profile.id).unwrap().provider,
        ProviderKind::DeepLX
    );
    assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), original);
}

#[test]
fn existing_profile_credential_does_not_read_migration_items() {
    let fake = FakeSecretStore::default();
    let default = ServiceProfile::alibaba_default();
    fake.put(
        PROFILE_KEYCHAIN_SERVICE,
        &credential_account(&default),
        "sk-profile",
    );
    fake.put(
        PROFILE_KEYCHAIN_SERVICE,
        LEGACY_MIGRATION_TOMBSTONE_ACCOUNT,
        LEGACY_MIGRATION_TOMBSTONE_VALUE,
    );
    fake.put(
        LEGACY_KEYCHAIN_SERVICE_V3,
        LEGACY_KEYCHAIN_ACCOUNT,
        "sk-legacy",
    );
    let store = settings(&fake);

    assert_eq!(store.credential_state(&default), CredentialState::Present);
    assert_eq!(store.load_api_key().unwrap().as_deref(), Some("sk-profile"));
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&default)),
        1
    );
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT),
        0
    );
    assert_eq!(
        fake.load_count(LEGACY_KEYCHAIN_SERVICE_V3, LEGACY_KEYCHAIN_ACCOUNT),
        0
    );
}

#[test]
fn explicit_diagnostic_recovers_legacy_deeplx_destination_after_unlock() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::DeepLX, "Existing")
        .unwrap();
    let account = credential_account(&profile);
    let destination_account = SettingsStore::destination_account(&profile);
    let legacy = ProviderCredentials::DeepLX {
        asr_api_key: "synthetic-asr".into(),
        endpoint: "https://example.com/legacy".into(),
        token: "synthetic-legacy-token".into(),
    }
    .encode_for_keychain(ProviderKind::DeepLX)
    .unwrap();
    fake.put(PROFILE_KEYCHAIN_SERVICE, &account, &legacy);
    fake.put(
        PROFILE_KEYCHAIN_SERVICE,
        &destination_account,
        &serde_json::to_string(&TextTranslationDestination {
            endpoint: "https://example.com/translate".into(),
            token: "synthetic-custom-token".into(),
            deep_l_api_key: None,
            open_ai_compatible: None,
            chat_mock: None,
            xai: None,
        })
        .unwrap(),
    );
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &destination_account);
    assert_eq!(
        store.credential_state(&profile),
        CredentialState::Unavailable
    );

    fake.state.lock().unwrap().unavailable.clear();
    // Normal reads keep the failure cached until the user explicitly retries.
    assert_eq!(
        store.credential_state(&profile),
        CredentialState::Unavailable
    );
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination_account),
        1
    );
    assert_eq!(store.credential_diagnostic(&profile), "present");
    assert!(matches!(
        store.credentials_for_profile(&profile).unwrap(),
        Some(ProviderCredentials::DeepLX { asr_api_key, endpoint, token })
            if asr_api_key == "synthetic-asr"
                && endpoint == "https://example.com/translate"
                && token == "synthetic-custom-token"
    ));
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination_account),
        2
    );
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
}

#[test]
fn legacy_preferences_default_to_centered_card_presentation() {
    let preferences: Preferences = serde_json::from_str("{}").unwrap();

    assert!(preferences.subtitle_font_family.is_empty());
    assert_eq!(preferences.subtitle_background_opacity, 80);
    assert_eq!(preferences.subtitle_color, SubtitleColor::White);
    assert_eq!(preferences.subtitle_alignment, SubtitleAlignment::Center);
    assert_eq!(
        preferences.subtitle_display_mode,
        SubtitleDisplayMode::Translation
    );
    assert!(!preferences.subtitle_blends_with_background);
    assert!(preferences.show_intermediate_subtitles);
    assert!(!preferences.show_subtitle_dividers);
    assert!(!preferences.show_subtitle_timestamps);
    assert_eq!(preferences.pulse_style, PulseStyle::Ribbon);
}

#[test]
fn pulse_style_migration_preserves_existing_motion_choices() {
    for json in [
        r#"{"font_size":19,"pulse_animation":false,"subtitle_animation":true}"#,
        r#"{"font_size":19,"pulse_animation":false,"subtitle_animation":true,"pulse_style":"classic"}"#,
        r#"{"font_size":19,"pulse_animation":false,"subtitle_animation":true,"pulse_style":"futureStyle"}"#,
    ] {
        let prefs: Preferences = serde_json::from_str(json).unwrap();
        assert_eq!(prefs.pulse_style, PulseStyle::Ribbon);
        assert_eq!(serde_json::to_value(prefs.pulse_style).unwrap(), "ribbon");
        assert_eq!(prefs.font_size, 19.0);
        assert_eq!(prefs.pulse_animation, Some(false));
        assert_eq!(prefs.subtitle_animation, Some(true));
    }
}

#[test]
fn legacy_translation_modes_migrate_to_turbo_without_loading_credentials() {
    for mode in ["lowLatency", "highQuality", "turbo"] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        std::fs::write(
            directory.path().join("preferences.json"),
            serde_json::to_vec(&serde_json::json!({
                "translation_mode": mode,
                "pulse_style": "classic",
                "pulse_animation": false,
                "show_in_dock": false,
                "font_size": 19
            }))
            .unwrap(),
        )
        .unwrap();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let prefs = store.preferences();
        assert_eq!(prefs.translation_mode, TranslationMode::Turbo);
        assert_eq!(prefs.pulse_style, PulseStyle::Ribbon);
        assert_eq!(prefs.pulse_animation, Some(false));
        assert_eq!(prefs.font_size, 19.0);
        assert!(!prefs.show_in_dock);
        store
            .save_preferences(|prefs| prefs.translation_mode = TranslationMode::HighQuality)
            .unwrap();
        let persisted: Preferences = serde_json::from_slice(
            &std::fs::read(directory.path().join("preferences.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(persisted.translation_mode, TranslationMode::Turbo);
        assert_eq!(persisted.pulse_style, PulseStyle::Ribbon);
        assert!(!persisted.show_in_dock);
        assert!(fake.state.lock().unwrap().loads.is_empty());
    }
}

#[test]
fn development_scope_does_not_read_or_migrate_release_credentials() {
    let fake = FakeSecretStore::default();
    fake.put(
        PROFILE_KEYCHAIN_SERVICE,
        &credential_account(&ServiceProfile::alibaba_default()),
        "sk-release-profile",
    );
    fake.put(
        LEGACY_KEYCHAIN_SERVICE_V3,
        LEGACY_KEYCHAIN_ACCOUNT,
        "sk-release-legacy",
    );
    let store = SettingsStore::in_memory_with_scope(
        Box::new(fake.clone()),
        false,
        DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
    );

    assert_eq!(store.load_api_key().unwrap(), None);
    store
        .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "sk-development")
        .unwrap();
    assert_eq!(
        fake.value(
            DEVELOPMENT_PROFILE_KEYCHAIN_SERVICE,
            &credential_account(&ServiceProfile::alibaba_default())
        )
        .as_deref(),
        Some("sk-development")
    );
    assert_eq!(
        fake.value(
            PROFILE_KEYCHAIN_SERVICE,
            &credential_account(&ServiceProfile::alibaba_default())
        )
        .as_deref(),
        Some("sk-release-profile")
    );
}
