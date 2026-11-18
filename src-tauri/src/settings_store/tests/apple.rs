//! Apple behavior regressions.

use super::*;

#[test]
fn apple_text_route_is_metadata_only_preserves_auto_and_ignores_broken_text_storage() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    store
        .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-asr")
        .unwrap();
    let profile = store.active_profile().unwrap();
    let destination = SettingsStore::destination_account(&profile);
    fake.put(
        PROFILE_KEYCHAIN_SERVICE,
        &destination,
        "corrupt legacy destination",
    );
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &destination);
    store.secret_cache.lock().unwrap().clear();
    fake.state.lock().unwrap().loads.clear();
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::Apple, "", "", ""),
        )
        .unwrap();
    let profile = store.active_profile().unwrap();
    assert_eq!(profile.text_translation(), TextTranslation::Apple);
    assert_eq!(
        store.preferences().source_language,
        SourceLanguage::Automatic
    );
    store.prepare_for_listening().unwrap();
    assert_eq!(
        store.preferences().source_language,
        SourceLanguage::Automatic
    );
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination), 0);
    assert_eq!(store.credential_state(&profile), CredentialState::Present);
    assert!(store
        .credential_editor_state(&profile.id, Some(TextTranslation::Apple))
        .unwrap()
        .saved_fields
        .is_empty());
    assert!(!CredentialRevealField::Token.allowed_for(&profile, Some(TextTranslation::Apple)));
    store
        .save_preferences(|prefs| {
            prefs.source_language = SourceLanguage::Japanese;
            prefs.target_language = TargetLanguage::English;
            prefs.network_proxy = ProxyConfig {
                mode: crate::core::services::network_proxy::ProxyMode::Direct,
                url: None,
            };
        })
        .unwrap();
    let config = store.configuration().unwrap();
    assert_eq!(config.credentials.alibaba_key(), Some("synthetic-asr"));
    assert_eq!(
        config.text_credentials,
        Some(TextTranslationCredentials::Apple)
    );
    let probe = store.configuration_for_text_probe(&profile).unwrap();
    assert_eq!(probe.source_language, SourceLanguage::Japanese);
    assert_eq!(probe.target_language, TargetLanguage::English);
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination), 0);
}

#[test]
fn apple_idle_profile_selection_preserves_auto_until_an_explicit_source_is_chosen() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Apple text")
        .unwrap();
    store
        .save_preferences(|prefs| {
            prefs.source_language = SourceLanguage::Automatic;
            prefs.target_language = TargetLanguage::English;
        })
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::Apple, "synthetic-asr", "", ""),
        )
        .unwrap();
    let profile = store.profile(&profile.id).unwrap();
    assert_eq!(
        store
            .preferences_for_profile_selection(&profile, None)
            .unwrap()
            .source_language,
        SourceLanguage::Automatic
    );
    store.select_profile(&profile.id).unwrap();
    assert_eq!(
        store.preferences().source_language,
        SourceLanguage::Automatic
    );
    assert_eq!(
        store.configuration().unwrap_err(),
        "apple_translation_language_unsupported"
    );
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.source_language = SourceLanguage::Japanese
        })
        .unwrap();
    assert_eq!(
        store.configuration().unwrap().source_language,
        SourceLanguage::Japanese
    );
}

#[test]
fn apple_text_route_keeps_custom_recognition_credentials_and_original_bypasses_it() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    for provider in [ProviderKind::AppleSpeech, ProviderKind::CustomOpenAIASR] {
        let profile = store.create_profile(provider, "Local MT").unwrap();
        if provider.is_custom_speech() {
            store
                .save_credentials(
                    &profile.id,
                    &custom_speech_request(
                        "wss://speech.example/realtime",
                        "synthetic",
                        "synthetic-speech",
                    ),
                )
                .unwrap();
        }
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::Apple, "", "", ""),
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        store
            .save_preferences(|prefs| {
                prefs.source_language = SourceLanguage::English;
                prefs.target_language = TargetLanguage::Japanese;
            })
            .unwrap();
        let config = store.configuration().unwrap();
        assert_eq!(config.provider, provider);
        assert_eq!(
            config.text_credentials,
            Some(TextTranslationCredentials::Apple)
        );
        store
            .save_preferences(|prefs| {
                prefs.target_language = TargetLanguage::Original;
                if provider.is_custom_speech() {
                    prefs.source_language = SourceLanguage::Automatic;
                }
            })
            .unwrap();
        assert_eq!(
            store.configuration().unwrap().text_network_proxy.mode,
            crate::core::services::network_proxy::ProxyMode::Direct
        );
        assert_eq!(store.configuration().unwrap().text_credentials, None);
        assert_eq!(
            fake.load_count(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            ),
            0
        );
    }
}

#[test]
fn apple_target_switch_restores_translation_after_original_without_credentials() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::AppleSpeech, "Apple speech and translation")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::Apple, "", "", ""),
        )
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    store
        .save_preferences(|prefs| {
            prefs.source_language = SourceLanguage::Japanese;
            prefs.target_language = TargetLanguage::SimplifiedChinese;
        })
        .unwrap();
    let profile = store.active_profile().unwrap();
    let translated = store.configuration().unwrap();
    let selection = ProviderPreferences {
        source_language: SourceLanguage::Japanese,
        target_language: TargetLanguage::Original,
        translation_mode: TranslationMode::Turbo,
    };
    let original = store
        .configuration_for_target_switch(&profile, translated, selection)
        .unwrap();
    assert_eq!(original.text_credentials, None);
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.target_language = TargetLanguage::Original
        })
        .unwrap();
    let before = store.preferences();
    fake.make_unavailable(
        PROFILE_KEYCHAIN_SERVICE,
        &SettingsStore::destination_account(&profile),
    );
    fake.state.lock().unwrap().loads.clear();

    let restored = store
        .configuration_for_target_switch(
            &profile,
            original,
            ProviderPreferences {
                target_language: TargetLanguage::SimplifiedChinese,
                ..selection
            },
        )
        .unwrap();

    assert_eq!(restored.source_language, SourceLanguage::Japanese);
    assert_eq!(restored.target_language, TargetLanguage::SimplifiedChinese);
    assert_eq!(restored.credentials, ProviderCredentials::AppleSpeech);
    assert_eq!(
        restored.text_credentials,
        Some(TextTranslationCredentials::Apple)
    );
    assert!(fake.state.lock().unwrap().loads.is_empty());
    assert_eq!(store.preferences(), before);
    assert_eq!(store.active_profile().unwrap(), profile);
}

#[test]
fn apple_text_route_explicit_speech_update_preserves_historical_translator_and_rolls_back() {
    for previous in [
        ProviderCredentials::DeepLX {
            asr_api_key: "old-asr".into(),
            endpoint: "https://example.com/translate".into(),
            token: "old-text".into(),
        },
        ProviderCredentials::DeepL {
            asr_api_key: "old-asr".into(),
            api_key: "old-text:fx".into(),
        },
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store
            .create_profile(ProviderKind::DeepLX, "Historical")
            .unwrap();
        let account = credential_account(&profile);
        let previous_value = previous
            .encode_for_keychain(match previous {
                ProviderCredentials::DeepL { .. } => ProviderKind::AlibabaCloud,
                _ => ProviderKind::DeepLX,
            })
            .unwrap();
        fake.put(PROFILE_KEYCHAIN_SERVICE, &account, &previous_value);
        store.secret_cache.lock().unwrap().clear();
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::Apple, "new-asr", "", ""),
            )
            .unwrap();
        let updated_value = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
        let updated = ProviderCredentials::decode_for_profile(&profile, &updated_value).unwrap();
        assert_eq!(updated.alibaba_key(), Some("new-asr"));
        assert!(updated_value.contains("old-text"));
        let selected = store.profile(&profile.id).unwrap();
        assert_eq!(selected.effective_provider(), ProviderKind::AlibabaCloud);
        assert_eq!(selected.text_translation(), TextTranslation::Apple);
        let old_route = match previous {
            ProviderCredentials::DeepLX { .. } => TextTranslation::DeepLX,
            ProviderCredentials::DeepL { .. } => TextTranslation::DeepL,
            _ => unreachable!(),
        };
        store
            .save_credentials(&profile.id, &translation_request(old_route, "", "", ""))
            .unwrap();
        assert!(store
            .text_credentials_for_profile(&store.profile(&profile.id).unwrap())
            .unwrap()
            .is_some());
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::Apple, "", "", ""),
            )
            .unwrap();
        let updated_value = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
        std::fs::remove_file(&store.catalog_path).unwrap();
        std::fs::create_dir(&store.catalog_path).unwrap();
        assert!(store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::Apple, "failed-asr", "", "")
            )
            .is_err());
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
            Some(updated_value.as_str())
        );
    }
}

#[test]
fn apple_text_route_updates_alibaba_key_without_accessing_separate_text_destinations() {
    for (route, request) in [
        (
            TextTranslation::DeepLX,
            translation_request(
                TextTranslation::DeepLX,
                "",
                "https://example.com/translate",
                "synthetic-text",
            ),
        ),
        (
            TextTranslation::DeepL,
            translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
        ),
        (
            TextTranslation::OpenAICompatible,
            openai_compatible_request(
                "",
                "https://example.com/v1",
                "synthetic-text",
                "synthetic-model",
            ),
        ),
        (
            TextTranslation::ChatMock,
            chatmock_request(
                "http://localhost:8000/v1",
                "synthetic-text",
                "synthetic-model",
            ),
        ),
    ] {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store.active_profile().unwrap();
        store.save_api_key(&profile.id, "old-asr").unwrap();
        store.save_credentials(&profile.id, &request).unwrap();
        let destination = SettingsStore::destination_account(&profile);
        let previous = fake.value(PROFILE_KEYCHAIN_SERVICE, &destination).unwrap();
        let reads = fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination);
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::Apple, "new-asr", "", ""),
            )
            .unwrap();
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile))
                .as_deref(),
            Some("new-asr")
        );
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &destination)
                .as_deref(),
            Some(previous.as_str())
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination),
            reads
        );
        store
            .save_credentials(&profile.id, &translation_request(route, "", "", ""))
            .unwrap();
        assert!(store
            .text_credentials_for_profile(&store.profile(&profile.id).unwrap())
            .unwrap()
            .is_some());
    }
}

#[test]
fn apple_profile_lifecycle_never_reads_or_writes_a_speech_credential() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::AppleSpeech, "Local Apple")
        .unwrap();
    let speech_account = credential_account(&profile);
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &speech_account);
    store.select_profile(&profile.id).unwrap();
    assert_eq!(
        store.credential_state_for_snapshot(&profile),
        CredentialState::Present
    );
    assert_eq!(store.credential_diagnostic(&profile), "present");
    let config = store.configuration().unwrap();
    assert_eq!(config.credentials, ProviderCredentials::AppleSpeech);
    assert_eq!(config.target_language, TargetLanguage::Original);
    assert_ne!(config.source_language, SourceLanguage::Automatic);
    assert!(store.configuration_for_speech_probe(&profile).is_ok());
    assert!(store
        .configuration_for_speech_draft_probe(&profile, &ProviderCredentials::AppleSpeech, true)
        .is_ok());
    assert!(store
        .credential_editor_state(&profile.id, None)
        .unwrap()
        .saved_fields
        .is_empty());
    assert!(store
        .save_credentials(&profile.id, &ProviderCredentials::AppleSpeech)
        .is_err());
    assert!(store
        .save_api_key(&profile.id, "synthetic-unrelated")
        .is_err());
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
        0
    );
    store.delete_profile(&profile.id).unwrap();
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
        0
    );
    assert!(fake
        .value(PROFILE_KEYCHAIN_SERVICE, &speech_account)
        .is_none());
}

#[test]
fn explicit_apple_source_selection_overrides_a_preset_without_rewriting_it() {
    let directory = tempfile::tempdir().unwrap();
    let store = SettingsStore::at_path(
        directory.path().into(),
        Box::new(FakeSecretStore::default()),
    );
    let apple = store
        .create_profile(ProviderKind::AppleSpeech, "Apple")
        .unwrap();
    save_language_preset(
        &store,
        &apple.id,
        SourceLanguage::Japanese,
        TargetLanguage::Original,
    );
    store
        .save_preferences(|prefs| prefs.target_language = TargetLanguage::Original)
        .unwrap();
    store
        .select_profile_with_source(&apple.id, Some(SourceLanguage::English))
        .unwrap();
    assert_eq!(store.preferences().source_language, SourceLanguage::English);
    store.select_profile(&apple.id).unwrap();
    assert_eq!(
        store.preferences().source_language,
        SourceLanguage::Japanese
    );
}

#[test]
fn apple_profile_selection_saves_the_chosen_source_with_the_profile() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let apple = store
        .create_profile(ProviderKind::AppleSpeech, "Apple")
        .unwrap();
    store
        .save_preferences(|prefs| {
            prefs.source_language = SourceLanguage::Automatic;
            prefs.target_language = TargetLanguage::Original;
            prefs.audio_input = AudioInput::Both;
            prefs.retain_session_history = true;
            prefs.record_session_audio = true;
        })
        .unwrap();
    let before = store.preferences();
    let catalog = std::fs::read(&store.catalog_path).unwrap();
    let persisted = std::fs::read(&store.prefs_path).unwrap();
    let proposed = store
        .configuration_for_profile_selection_with_source(&apple, Some(SourceLanguage::English))
        .unwrap();
    assert_eq!(proposed.source_language, SourceLanguage::English);
    assert_eq!(proposed.target_language, TargetLanguage::Original);
    assert_eq!(store.preferences(), before);
    assert_eq!(std::fs::read(&store.catalog_path).unwrap(), catalog);
    assert_eq!(std::fs::read(&store.prefs_path).unwrap(), persisted);

    store
        .select_profile_with_source(&apple.id, Some(SourceLanguage::English))
        .unwrap();
    assert_eq!(store.configuration().unwrap(), proposed);
    assert_eq!(store.active_profile().unwrap().id, apple.id);
    let selected = store.preferences();
    assert_eq!(selected.audio_input, AudioInput::Both);
    assert!(selected.retain_session_history);
    assert!(selected.record_session_audio);
    // Reusing the active profile must still apply the requested language.
    store
        .select_profile_with_source(&apple.id, Some(SourceLanguage::Japanese))
        .unwrap();
    assert_eq!(
        store.preferences().source_language,
        SourceLanguage::Japanese
    );
    let reopened = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    assert_eq!(reopened.active_profile().unwrap().id, apple.id);
    assert_eq!(
        reopened.preferences().source_language,
        SourceLanguage::Japanese
    );
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&apple)),
        0
    );
}

#[test]
fn apple_explicit_profile_source_rejects_incompatible_routes_without_normalizing_it() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let apple = store
        .create_profile(ProviderKind::AppleSpeech, "Apple")
        .unwrap();
    store
        .save_credentials(
            &apple.id,
            &translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
        )
        .unwrap();
    let apple = store.profile(&apple.id).unwrap();
    store
        .save_preferences(|prefs| prefs.target_language = TargetLanguage::English)
        .unwrap();
    let before = store.preferences();
    let active = store.active_profile().unwrap();
    for (profile, source, expected) in [
        (
            &apple,
            SourceLanguage::Automatic,
            "apple_speech_language_unsupported",
        ),
        (
            &apple,
            SourceLanguage::Khmer,
            "apple_speech_translation_language_unsupported",
        ),
        (
            &active,
            SourceLanguage::English,
            "apple_speech_source_override_invalid",
        ),
    ] {
        assert_eq!(
            store
                .configuration_for_profile_selection_with_source(profile, Some(source))
                .unwrap_err(),
            expected
        );
        assert_eq!(
            store
                .select_profile_with_source(&profile.id, Some(source))
                .unwrap_err(),
            expected
        );
        assert_eq!(store.preferences(), before);
        assert_eq!(store.active_profile().unwrap(), active);
    }
    // A recognition-only target does not impose the text provider's limits.
    store
        .save_preferences(|prefs| prefs.target_language = TargetLanguage::Original)
        .unwrap();
    store
        .select_profile_with_source(&apple.id, Some(SourceLanguage::Khmer))
        .unwrap();
    assert_eq!(store.preferences().source_language, SourceLanguage::Khmer);
}

#[test]
fn apple_profile_source_selection_rolls_back_if_catalog_persistence_fails() {
    let directory = tempfile::tempdir().unwrap();
    let store = SettingsStore::at_path(
        directory.path().into(),
        Box::new(FakeSecretStore::default()),
    );
    let apple = store
        .create_profile(ProviderKind::AppleSpeech, "Apple")
        .unwrap();
    store
        .save_preferences(|prefs| {
            prefs.source_language = SourceLanguage::Automatic;
            prefs.target_language = TargetLanguage::Original;
        })
        .unwrap();
    let before = store.preferences();
    let persisted = std::fs::read(&store.prefs_path).unwrap();
    std::fs::remove_file(&store.catalog_path).unwrap();
    std::fs::create_dir(&store.catalog_path).unwrap();
    assert_eq!(
        store
            .select_profile_with_source(&apple.id, Some(SourceLanguage::Japanese))
            .unwrap_err(),
        PROFILE_CATALOG_UNAVAILABLE
    );
    assert_eq!(store.preferences(), before);
    assert_eq!(std::fs::read(&store.prefs_path).unwrap(), persisted);
    assert_eq!(
        store.active_profile().unwrap().id,
        DEFAULT_ALIBABA_PROFILE_ID
    );
}

#[test]
fn apple_speech_probe_checks_the_visible_language_without_mutation_or_text_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let apple = store
        .create_profile(ProviderKind::AppleSpeech, "Apple")
        .unwrap();
    store
        .save_credentials(
            &apple.id,
            &translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
        )
        .unwrap();
    let apple = store.profile(&apple.id).unwrap();
    store
        .save_preferences(|prefs| {
            prefs.source_language = SourceLanguage::Automatic;
            prefs.target_language = TargetLanguage::English;
        })
        .unwrap();
    let before = store.preferences();
    let active = store.active_profile().unwrap();
    let catalog = std::fs::read(&store.catalog_path).unwrap();
    let persisted = std::fs::read(&store.prefs_path).unwrap();
    store.secret_cache.lock().unwrap().clear();
    fake.state.lock().unwrap().loads.clear();
    fake.make_unavailable(
        PROFILE_KEYCHAIN_SERVICE,
        &SettingsStore::destination_account(&apple),
    );
    for source in [
        SourceLanguage::English,
        SourceLanguage::Japanese,
        SourceLanguage::French,
    ] {
        let probe = store
            .configuration_for_speech_probe_with_source(&apple, Some(source))
            .unwrap();
        assert_eq!(probe.source_language, source);
        assert_eq!(probe.target_language, TargetLanguage::Original);
        assert_eq!(probe.credentials, ProviderCredentials::AppleSpeech);
        assert_eq!(probe.text_credentials, None);
    }
    assert_eq!(
        store
            .configuration_for_speech_probe_with_source(&apple, Some(SourceLanguage::Automatic))
            .unwrap_err(),
        "apple_speech_language_unsupported"
    );
    assert_eq!(
        store
            .configuration_for_speech_probe_with_source(&active, Some(SourceLanguage::English))
            .unwrap_err(),
        "apple_speech_source_override_invalid"
    );
    assert_eq!(store.preferences(), before);
    assert_eq!(store.active_profile().unwrap(), active);
    assert_eq!(std::fs::read(&store.catalog_path).unwrap(), catalog);
    assert_eq!(std::fs::read(&store.prefs_path).unwrap(), persisted);
    assert!(fake.state.lock().unwrap().loads.is_empty());
}

#[test]
fn apple_independent_translation_owns_its_credentials_and_probes_without_speech() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::AppleSpeech, "Apple and local MT")
        .unwrap();
    let speech_account = credential_account(&profile);
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &speech_account);
    store
        .save_credentials(
            &profile.id,
            &openai_compatible_request("", "http://localhost:8080/v1", "", "synthetic-local-model"),
        )
        .unwrap();
    let profile = store.profile(&profile.id).unwrap();
    store.select_profile(&profile.id).unwrap();
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.source_language = SourceLanguage::English;
            prefs.target_language = TargetLanguage::SimplifiedChinese;
        })
        .unwrap();
    let config = store.configuration().unwrap();
    assert_eq!(config.credentials, ProviderCredentials::AppleSpeech);
    assert!(
        matches!(config.text_credentials, Some(TextTranslationCredentials::OpenAICompatible { api_key, .. }) if api_key.is_empty())
    );
    assert!(store.configuration_for_text_probe(&profile).is_ok());
    assert!(store.configuration_for_speech_probe(&profile).is_ok());
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
        0
    );
    assert!(store
        .save_credentials(
            &profile.id,
            &openai_compatible_request(
                "synthetic-forbidden-speech",
                "http://localhost:8080/v1",
                "",
                "synthetic-model"
            )
        )
        .is_err());
    store.delete_profile(&profile.id).unwrap();
    assert!(fake
        .value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        )
        .is_none());
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
        0
    );
}
