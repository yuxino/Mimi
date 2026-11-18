//! Configuration behavior regressions.

use super::*;

#[test]
fn target_switch_still_rejects_a_missing_independent_text_key_before_saving() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let mut profile = store
        .create_profile(ProviderKind::AppleSpeech, "Unconfigured text service")
        .unwrap();
    profile.text_translation = Some(TextTranslation::DeepL);
    let original = LiveTranslationConfiguration::with_credentials(
        ProviderKind::AppleSpeech,
        ProviderCredentials::AppleSpeech,
        SourceLanguage::Japanese,
        TargetLanguage::Original,
        TranslationMode::Turbo,
    )
    .validated()
    .unwrap();
    let before = store.preferences();

    assert_eq!(
        store
            .configuration_for_target_switch(
                &profile,
                original.clone(),
                ProviderPreferences {
                    source_language: SourceLanguage::Japanese,
                    target_language: TargetLanguage::SimplifiedChinese,
                    translation_mode: TranslationMode::Turbo,
                },
            )
            .unwrap_err(),
        crate::core::services::credentials::ProviderCredentialsError::MissingTextTranslation
            .to_string()
    );
    assert_eq!(store.preferences(), before);
    assert_eq!(original.target_language, TargetLanguage::Original);
    assert_eq!(original.text_credentials, None);
}

#[test]
fn profile_language_presets_restore_independent_pairs_without_tracking_temporary_changes() {
    for provider in [ProviderKind::AppleSpeech, ProviderKind::AlibabaCloud] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let japanese = store.create_profile(provider, "Same name").unwrap();
        let english = store.create_profile(provider, "Same name").unwrap();
        let target = if provider == ProviderKind::AppleSpeech {
            TargetLanguage::Original
        } else {
            TargetLanguage::SimplifiedChinese
        };
        let before = store.preferences();
        save_language_preset(&store, &japanese.id, SourceLanguage::Japanese, target);
        save_language_preset(&store, &english.id, SourceLanguage::English, target);
        assert_eq!(store.preferences(), before); // Saving is not activation.
        store.select_profile(&japanese.id).unwrap();
        assert_eq!(
            store.preferences().source_language,
            SourceLanguage::Japanese
        );
        store.select_profile(&english.id).unwrap();
        assert_eq!(store.preferences().source_language, SourceLanguage::English);
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::French
            })
            .unwrap();
        store.select_profile(&english.id).unwrap(); // Reselect restores the preset.
        assert_eq!(store.preferences().source_language, SourceLanguage::English);
        store.update_profile(&japanese.id, "Renamed").unwrap();
        let reopened = SettingsStore::at_path(directory.path().into(), Box::new(fake));
        reopened.select_profile(&japanese.id).unwrap();
        assert_eq!(
            reopened.preferences().source_language,
            SourceLanguage::Japanese
        );
        assert_eq!(reopened.preferences().target_language, target);
        reopened
            .update_profile_options(
                &japanese.id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(crate::core::services::provider::ProfileLanguagePresetPatch { preset: None }),
            )
            .unwrap();
        reopened
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::English
            })
            .unwrap();
        reopened.select_profile(&japanese.id).unwrap();
        assert_eq!(
            reopened.preferences().source_language,
            SourceLanguage::English
        );
    }
}

#[test]
fn profile_language_preset_failures_leave_metadata_and_selection_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = SettingsStore::at_path(
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
    let before = store.preferences();
    let catalog = store.profile_catalog().unwrap();
    let invalid = crate::core::services::provider::ProfileLanguagePreset {
        source_language: SourceLanguage::Automatic,
        target_language: TargetLanguage::Original,
    };
    assert_eq!(
        store
            .update_profile_options(
                &apple.id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(
                    crate::core::services::provider::ProfileLanguagePresetPatch {
                        preset: Some(invalid)
                    }
                )
            )
            .unwrap_err(),
        "profile_language_preset_unsupported"
    );
    let blocked = directory.path().join("blocked-write");
    std::fs::create_dir(&blocked).unwrap();
    store.catalog_path = blocked;
    assert!(store.select_profile(&apple.id).is_err());
    assert_eq!(store.preferences(), before);
    assert_eq!(store.profile_catalog().unwrap(), catalog);
    assert!(store
        .update_profile_options(
            &apple.id,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(crate::core::services::provider::ProfileLanguagePresetPatch { preset: None })
        )
        .is_err());
    assert_eq!(store.profile_catalog().unwrap(), catalog);
}

#[test]
fn profile_selection_configuration_matches_saved_selection_without_mutating_it() {
    // Keep a target supported by the selected provider; normalize only
    // targets outside its dedicated realtime translation catalog.
    for (target, expected_target) in [
        (TargetLanguage::French, TargetLanguage::French),
        (
            TargetLanguage::TraditionalChinese,
            TargetLanguage::SimplifiedChinese,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake));
        let next = store
            .create_profile(ProviderKind::OpenAIRealtime, "Synthetic next")
            .unwrap();
        store.save_api_key(&next.id, "synthetic-next").unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = SourceLanguage::Japanese;
                prefs.target_language = target;
                prefs.audio_input = AudioInput::Both;
                prefs.retain_session_history = true;
                prefs.record_session_audio = true;
            })
            .unwrap();
        let previous = store.preferences();
        let active = store.active_profile().unwrap();
        let catalog = std::fs::read(&store.catalog_path).unwrap();
        let persisted = std::fs::read(&store.prefs_path).unwrap();

        let proposed = store.configuration_for_profile_selection(&next).unwrap();
        assert_eq!(proposed.source_language, SourceLanguage::Automatic);
        assert_eq!(proposed.target_language, expected_target);
        assert_eq!(store.preferences(), previous);
        assert_eq!(store.active_profile().unwrap(), active);
        assert_eq!(std::fs::read(&store.catalog_path).unwrap(), catalog);
        assert_eq!(std::fs::read(&store.prefs_path).unwrap(), persisted);

        store.select_profile(&next.id).unwrap();
        assert_eq!(store.configuration().unwrap(), proposed);
        let selected = store.preferences();
        assert_eq!(selected.audio_input, AudioInput::Both);
        assert!(selected.retain_session_history);
        assert!(selected.record_session_audio);
    }
}

#[test]
fn unsaved_text_probe_never_needs_asr_or_mutates_profile_preferences_or_secrets() {
    use crate::core::services::configuration::TextTranslationProbeCredentials;
    for provider in [ProviderKind::AlibabaCloud, ProviderKind::CustomOpenAIASR] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store.create_profile(provider, "Draft only").unwrap();
        let catalog = std::fs::read(&store.catalog_path).unwrap();
        let preferences = store.preferences();
        let active = store.active_profile().unwrap();
        let speech_account = credential_account(&profile);
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &speech_account);
        let draft = openai_compatible_request("", "https://draft.example/v1", "", "draft-model");
        let probe = store
            .configuration_for_text_draft_probe(&profile, &draft)
            .unwrap();
        assert!(
            matches!(probe.credentials, TextTranslationProbeCredentials::Independent(
                TextTranslationCredentials::OpenAICompatible { endpoint, model, api_key }
            ) if endpoint == "https://draft.example/v1/chat/completions" && model == "draft-model" && api_key.is_empty())
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
            0
        );
        assert_eq!(store.active_profile().unwrap(), active);
        assert_eq!(store.preferences(), preferences);
        assert_eq!(store.profile(&profile.id).unwrap(), profile);
        assert_eq!(std::fs::read(&store.catalog_path).unwrap(), catalog);
        assert!(fake.state.lock().unwrap().values.is_empty());
    }
}

#[test]
fn text_draft_probe_merges_saved_destination_without_persisting_or_reusing_other_routes() {
    use crate::core::services::configuration::TextTranslationProbeCredentials;
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::CustomOpenAIASR, "Draft check")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &openai_compatible_request(
                "",
                "https://saved.example/v1",
                "synthetic-saved",
                "saved-model",
            ),
        )
        .unwrap();
    let profile = store.profile(&profile.id).unwrap();
    let values = fake.state.lock().unwrap().values.clone();
    let preferences = store.preferences();
    for (endpoint, token, clear_token, route, expected_key) in [
        (
            "",
            "",
            false,
            TextTranslation::OpenAICompatible,
            "synthetic-saved",
        ),
        (
            "https://saved.example/v1",
            "",
            false,
            TextTranslation::OpenAICompatible,
            "synthetic-saved",
        ),
        (
            "https://changed.example/v1",
            "",
            false,
            TextTranslation::OpenAICompatible,
            "",
        ),
        ("", "", true, TextTranslation::OpenAICompatible, ""),
        (
            "",
            "synthetic-draft",
            false,
            TextTranslation::OpenAICompatible,
            "synthetic-draft",
        ),
        (
            "https://saved.example/v1",
            "",
            false,
            TextTranslation::ChatMock,
            "",
        ),
    ] {
        let draft = ProviderCredentials::AlibabaTranslation {
            api_key: String::new(),
            text_translation: route,
            endpoint: endpoint.into(),
            token: token.into(),
            model: "draft-model".into(),
            clear_token,
        };
        let probe = store
            .configuration_for_text_draft_probe(&profile, &draft)
            .unwrap();
        let TextTranslationProbeCredentials::Independent(credentials) = probe.credentials else {
            panic!("independent route expected")
        };
        assert_eq!(credentials.translation(), route);
        match credentials {
            TextTranslationCredentials::OpenAICompatible { api_key, model, .. }
            | TextTranslationCredentials::ChatMock { api_key, model, .. } => {
                assert_eq!(api_key, expected_key);
                assert_eq!(model, "draft-model");
            }
            _ => panic!("chat route expected"),
        }
        assert_eq!(store.profile(&profile.id).unwrap(), profile);
        assert_eq!(store.preferences(), preferences);
        assert_eq!(fake.state.lock().unwrap().values, values);
    }
}

#[test]
fn complete_draft_probes_can_bypass_unavailable_saved_credentials() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::CustomOpenAIASR, "Unavailable saved fields")
        .unwrap();
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
    fake.make_unavailable(
        PROFILE_KEYCHAIN_SERVICE,
        &SettingsStore::destination_account(&profile),
    );
    let speech = custom_speech_request("wss://draft.example/realtime", "speech-model", "synthetic");
    assert!(store
        .configuration_for_speech_draft_probe(&profile, &speech, true)
        .is_ok());
    let text = openai_compatible_request(
        "",
        "https://draft.example/v1",
        "synthetic-text",
        "text-model",
    );
    assert!(store
        .configuration_for_text_draft_probe(&profile, &text)
        .is_ok());
    assert!(fake.state.lock().unwrap().loads.is_empty());
    assert!(fake.state.lock().unwrap().values.is_empty());
    let invalid = openai_compatible_request(
        "",
        "https://user:secret@invalid.example/v1",
        "synthetic",
        "model",
    );
    assert!(store
        .configuration_for_text_draft_probe(&profile, &invalid)
        .is_err());
    assert!(fake.state.lock().unwrap().loads.is_empty());
    assert!(store
        .configuration_for_text_draft_probe(&profile, &speech)
        .is_err());
    assert!(store
        .configuration_for_speech_draft_probe(&profile, &text, true)
        .is_err());
    assert_eq!(store.profile(&profile.id).unwrap(), profile);
}

#[test]
fn custom_language_declaration_persists_normalizes_and_preserves_unrelated_patches() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let profile = store
        .create_profile(ProviderKind::CustomDashScopeASR, "Synthetic")
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.target_language = TargetLanguage::Original;
            prefs.source_language = SourceLanguage::Japanese;
        })
        .unwrap();
    let saved = store
        .update_profile_options(
            &profile.id,
            None,
            None,
            None,
            None,
            None,
            Some(CustomSpeechLanguagesPatch {
                languages: Some(vec![
                    SourceLanguage::German,
                    SourceLanguage::English,
                    SourceLanguage::German,
                ]),
            }),
            None,
        )
        .unwrap();
    assert_eq!(
        saved.custom_speech_source_languages,
        Some(vec![SourceLanguage::English, SourceLanguage::German])
    );
    assert_eq!(
        store.preferences().source_language,
        SourceLanguage::Automatic
    );
    store.update_profile(&profile.id, "Renamed").unwrap();
    let restored = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    assert_eq!(
        restored
            .active_profile()
            .unwrap()
            .custom_speech_source_languages,
        saved.custom_speech_source_languages
    );
    assert_eq!(
        restored.preferences().source_language,
        SourceLanguage::Automatic
    );
    assert!(store
        .save_preferences_for_active_profile(
            |prefs| prefs.source_language = SourceLanguage::Japanese
        )
        .is_err());
    store
        .save_preferences_for_active_profile(|prefs| prefs.source_language = SourceLanguage::German)
        .unwrap();
    store
        .update_profile_options(
            &profile.id,
            None,
            None,
            None,
            None,
            None,
            Some(CustomSpeechLanguagesPatch {
                languages: Some(vec![]),
            }),
            None,
        )
        .unwrap();
    assert_eq!(
        store.preferences().source_language,
        SourceLanguage::Automatic
    );
    store
        .update_profile_options(
            &profile.id,
            None,
            None,
            None,
            None,
            None,
            Some(CustomSpeechLanguagesPatch { languages: None }),
            None,
        )
        .unwrap();
    store
        .save_preferences_for_active_profile(|prefs| prefs.source_language = SourceLanguage::French)
        .unwrap();
    assert_eq!(
        store
            .active_profile()
            .unwrap()
            .custom_speech_source_languages,
        None
    );
    assert!(fake.state.lock().unwrap().loads.is_empty());
    assert!(fake.state.lock().unwrap().values.is_empty());
}

#[test]
fn inactive_language_declaration_does_not_normalize_the_active_profile() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.target_language = TargetLanguage::Original;
            prefs.source_language = SourceLanguage::Japanese;
        })
        .unwrap();
    let before = store.preferences();
    let custom = store
        .create_profile(ProviderKind::CustomOpenAIASR, "Other")
        .unwrap();
    store
        .update_profile_options(
            &custom.id,
            None,
            None,
            None,
            None,
            None,
            Some(CustomSpeechLanguagesPatch {
                languages: Some(vec![SourceLanguage::English]),
            }),
            None,
        )
        .unwrap();
    assert_eq!(store.preferences(), before);
    let builtin = store.active_profile().unwrap();
    assert!(store
        .update_profile_options(
            &builtin.id,
            None,
            None,
            None,
            None,
            None,
            Some(CustomSpeechLanguagesPatch { languages: None }),
            None
        )
        .is_err());
    assert_eq!(store.preferences(), before);
    assert_eq!(store.active_profile().unwrap(), builtin);
    store.select_profile(&custom.id).unwrap();
    assert_eq!(
        store.preferences().source_language,
        SourceLanguage::Automatic
    );
}

#[test]
fn failed_custom_language_write_rolls_back_preferences_and_profile() {
    for fail_catalog in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let mut store = SettingsStore::at_path(directory.path().into(), Box::new(fake));
        let custom = store
            .create_profile(ProviderKind::CustomDashScopeASR, "Synthetic")
            .unwrap();
        store.select_profile(&custom.id).unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::Original;
                prefs.source_language = SourceLanguage::Japanese;
            })
            .unwrap();
        let before = store.preferences();
        let blocked = directory.path().join("blocked-write");
        std::fs::create_dir(&blocked).unwrap();
        if fail_catalog {
            store.catalog_path = blocked;
        } else {
            store.prefs_path = blocked;
        }
        assert!(store
            .update_profile_options(
                &custom.id,
                None,
                None,
                None,
                None,
                None,
                Some(CustomSpeechLanguagesPatch {
                    languages: Some(vec![SourceLanguage::English])
                }),
                None
            )
            .is_err());
        assert_eq!(store.preferences(), before);
        assert_eq!(store.active_profile().unwrap(), custom);
        let restarted = SettingsStore::at_path(
            directory.path().into(),
            Box::new(FakeSecretStore::default()),
        );
        assert_eq!(restarted.preferences(), before);
        assert_eq!(restarted.active_profile().unwrap(), custom);
    }
}

#[test]
fn speech_probes_and_listening_honor_declared_languages_without_widening_them() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let custom = store
        .create_profile(ProviderKind::CustomDashScopeASR, "Synthetic")
        .unwrap();
    let speech = custom_speech_request("wss://speech.example/asr", "synthetic-model", "synthetic");
    store.save_credentials(&custom.id, &speech).unwrap();
    let custom = store
        .update_profile_options(
            &custom.id,
            None,
            None,
            None,
            None,
            None,
            Some(CustomSpeechLanguagesPatch {
                languages: Some(vec![SourceLanguage::French, SourceLanguage::German]),
            }),
            None,
        )
        .unwrap();
    // Current preferences belong to another profile; a probe may normalize
    // its temporary source, but cannot silently broaden this declaration.
    store
        .save_preferences(|prefs| {
            prefs.source_language = SourceLanguage::Japanese;
            prefs.target_language = TargetLanguage::Original;
        })
        .unwrap();
    assert_eq!(
        store
            .configuration_for_speech_probe(&custom)
            .unwrap()
            .source_language,
        SourceLanguage::Automatic
    );
    assert_eq!(
        store
            .configuration_for_speech_draft_probe(&custom, &speech, true)
            .unwrap()
            .source_language,
        SourceLanguage::Automatic
    );
    assert_eq!(
        store.preferences().source_language,
        SourceLanguage::Japanese
    );
    assert_eq!(
        store.configuration_for_profile(&custom).unwrap_err(),
        crate::core::services::configuration::LiveTranslationConfigurationError::UnsupportedSourceLanguage
            .to_string()
    );
    store.select_profile(&custom.id).unwrap();
    store
        .save_preferences_for_active_profile(|prefs| prefs.source_language = SourceLanguage::German)
        .unwrap();
    store
        .save_credentials(
            &custom.id,
            &openai_compatible_request("", "https://translation.example/v1", "", "synthetic-model"),
        )
        .unwrap();
    store
        .save_preferences_for_active_profile(|prefs| prefs.target_language = TargetLanguage::French)
        .unwrap();
    let configuration = store.configuration().unwrap();
    assert_eq!(configuration.source_language, SourceLanguage::German);
    assert_eq!(configuration.target_language, TargetLanguage::French);
    assert_eq!(
        store
            .active_profile()
            .unwrap()
            .custom_speech_source_languages,
        custom.custom_speech_source_languages
    );
}

#[test]
fn explicit_settings_preserve_chinese_translation_and_reject_wrong_provider_languages() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.source_language = SourceLanguage::Chinese;
            prefs.target_language = TargetLanguage::English;
        })
        .unwrap();
    assert_eq!(store.preferences().target_language, TargetLanguage::English);
    let openai = store
        .create_profile(ProviderKind::OpenAIRealtime, "Synthetic realtime")
        .unwrap();
    store.select_profile(&openai.id).unwrap();
    let before = store.preferences();
    assert!(store
        .save_preferences_for_active_profile(
            |prefs| prefs.target_language = TargetLanguage::TraditionalChinese
        )
        .is_err());
    assert_eq!(store.preferences(), before);
    assert!(store
        .save_preferences_for_active_profile(|prefs| prefs.source_language = SourceLanguage::French)
        .is_err());
    assert_eq!(store.preferences(), before);
}

#[test]
fn active_text_route_change_normalizes_languages_but_inactive_profile_and_probe_do_not() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let active = store.active_profile().unwrap();
    store.save_api_key(&active.id, "synthetic-asr").unwrap();
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.source_language = SourceLanguage::French;
            prefs.target_language = TargetLanguage::Khmer;
        })
        .unwrap();
    let expanded = store.preferences();
    let other = store
        .create_profile(ProviderKind::AlibabaCloud, "Synthetic DeepL")
        .unwrap();
    store
        .save_credentials(
            &other.id,
            &translation_request(TextTranslation::DeepL, "synthetic-asr", "", "synthetic:fx"),
        )
        .unwrap();
    assert_eq!(store.preferences(), expanded);
    let updated = store.profile(&other.id).unwrap();
    let probe = store.configuration_for_profile_probe(&updated).unwrap();
    assert_eq!(probe.source_language, SourceLanguage::French);
    assert_eq!(probe.target_language, TargetLanguage::Original);
    assert_eq!(store.preferences(), expanded);
    store
        .save_credentials(
            &active.id,
            &translation_request(TextTranslation::DeepL, "", "", "synthetic:fx"),
        )
        .unwrap();
    assert_eq!(store.preferences().source_language, SourceLanguage::French);
    assert_eq!(
        store.preferences().target_language,
        TargetLanguage::Original
    );
    assert!(store.configuration().is_ok());
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.target_language = TargetLanguage::English
        })
        .unwrap();
    assert!(store
        .save_preferences_for_active_profile(|prefs| prefs.target_language = TargetLanguage::Khmer)
        .is_err());
    store
        .save_credentials(
            &active.id,
            &translation_request(
                TextTranslation::DeepLX,
                "",
                "https://example.com/translate",
                "",
            ),
        )
        .unwrap();
    assert!(store.configuration().is_ok());
    assert!(store.preferences().target_language.translates_audio());
    store
        .save_credentials(
            &active.id,
            &translation_request(TextTranslation::FollowService, "", "", ""),
        )
        .unwrap();
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.source_language = SourceLanguage::French;
            prefs.target_language = TargetLanguage::Khmer;
        })
        .unwrap();
    assert!(store.configuration().is_ok());
}

#[test]
fn failed_route_catalog_commit_restores_expanded_preferences_in_memory_and_on_disk() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let mut store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let profile = store.active_profile().unwrap();
    store.save_api_key(&profile.id, "synthetic-asr").unwrap();
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.source_language = SourceLanguage::French;
            prefs.target_language = TargetLanguage::Persian;
        })
        .unwrap();
    let before = store.preferences();
    store.catalog_path = directory.path().join("blocked-catalog");
    std::fs::create_dir(&store.catalog_path).unwrap();
    assert!(store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepL, "", "", "synthetic:fx")
        )
        .is_err());
    assert_eq!(store.preferences(), before);
    assert_eq!(
        store.active_profile().unwrap().text_translation(),
        TextTranslation::FollowService
    );
    let persisted: Preferences =
        serde_json::from_slice(&std::fs::read(&store.prefs_path).unwrap()).unwrap();
    assert_eq!(persisted, before);
    assert!(fake
        .value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        )
        .is_none());
    assert!(store.configuration().is_ok());
}

#[test]
fn expanded_languages_persist_and_reload_with_route_validation() {
    let fake = FakeSecretStore::default();
    let directory = tempfile::tempdir().unwrap();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.source_language = SourceLanguage::Filipino;
            prefs.target_language = TargetLanguage::Persian;
        })
        .unwrap();
    let restarted = SettingsStore::at_path(directory.path().into(), Box::new(fake));
    assert_eq!(
        restarted.preferences().source_language,
        SourceLanguage::Filipino
    );
    assert_eq!(
        restarted.preferences().target_language,
        TargetLanguage::Persian
    );
}

#[test]
fn stage_probes_read_only_their_saved_credentials_without_switching_or_persisting() {
    use crate::core::services::configuration::TextTranslationProbeCredentials;
    for provider in [
        ProviderKind::AlibabaCloud,
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store.create_profile(provider, "Stage probe").unwrap();
        if provider.is_custom_speech() {
            store
                .save_credentials(
                    &profile.id,
                    &custom_speech_request(
                        "wss://speech.example/realtime",
                        "synthetic-model",
                        "synthetic-speech",
                    ),
                )
                .unwrap();
        } else {
            store
                .save_credentials(
                    &profile.id,
                    &ProviderCredentials::api_key("synthetic-speech"),
                )
                .unwrap();
        }
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "",
                    "https://text.example/v1",
                    "synthetic-text",
                    "synthetic-text-model",
                ),
            )
            .unwrap();
        let profile = store.profile(&profile.id).unwrap();
        let speech_account = credential_account(&profile);
        let text_account = SettingsStore::destination_account(&profile);
        let before = store.preferences();
        let before_active_id = store.active_profile().unwrap().id;
        store.secret_cache.lock().unwrap().clear();
        fake.state.lock().unwrap().loads.clear();
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &text_account);
        let speech = store.configuration_for_speech_probe(&profile).unwrap();
        assert_eq!(speech.target_language, TargetLanguage::Original);
        assert_eq!(speech.text_credentials, None);
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
            1
        );
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &text_account), 0);
        {
            let mut state = fake.state.lock().unwrap();
            state
                .unavailable
                .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &text_account));
            state
                .unavailable
                .insert(cache_key(PROFILE_KEYCHAIN_SERVICE, &speech_account));
            state.loads.clear();
        }
        store.secret_cache.lock().unwrap().clear();
        let text = store.configuration_for_text_probe(&profile).unwrap();
        assert!(
            matches!(text.credentials, TextTranslationProbeCredentials::Independent(TextTranslationCredentials::OpenAICompatible { api_key, .. }) if api_key == "synthetic-text")
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
            0
        );
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &text_account), 1);
        assert_eq!(store.preferences(), before);
        assert_eq!(store.active_profile().unwrap().id, before_active_id);
    }
}

#[test]
fn text_probe_can_precede_custom_speech_setup_and_retry_only_its_locked_slot() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::CustomOpenAIASR, "Text first")
        .unwrap();
    assert_eq!(
        store.configuration_for_text_probe(&profile).unwrap_err(),
        "text_translation_not_configured"
    );
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
        )
        .unwrap();
    let profile = store.profile(&profile.id).unwrap();
    let speech_account = credential_account(&profile);
    let text_account = SettingsStore::destination_account(&profile);
    store.secret_cache.lock().unwrap().clear();
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &text_account);
    assert!(store.configuration_for_text_probe(&profile).is_err());
    fake.state
        .lock()
        .unwrap()
        .unavailable
        .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &text_account));
    fake.state.lock().unwrap().loads.clear();
    assert!(store.configuration_for_text_probe(&profile).is_ok());
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &text_account), 1);
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
        0
    );
}

#[test]
fn anonymous_openai_compatible_destination_survives_restart_and_probes_without_speech_keys() {
    use crate::core::services::configuration::TextTranslationProbeCredentials;
    for provider in [
        ProviderKind::AlibabaCloud,
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store.create_profile(provider, "Anonymous text").unwrap();
        let speech = if provider.is_custom_speech() {
            custom_speech_request(
                "wss://speech.example/realtime",
                "speech-model",
                "synthetic-speech",
            )
        } else {
            ProviderCredentials::api_key("synthetic-speech")
        };
        store.save_credentials(&profile.id, &speech).unwrap();
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request("", "http://127.0.0.1:8000/v1", "", "local-model"),
            )
            .unwrap();
        let speech_account = credential_account(&profile);
        let destination_account = SettingsStore::destination_account(&profile);
        let speech_before = fake.value(PROFILE_KEYCHAIN_SERVICE, &speech_account);
        let destination = fake
            .value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
            .unwrap();
        assert!(!destination.contains("synthetic-speech"));
        drop(store);
        fake.state.lock().unwrap().loads.clear();
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = reloaded.profile(&profile.id).unwrap();
        assert_eq!(
            profile.text_translation(),
            TextTranslation::OpenAICompatible
        );
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &speech_account);
        let probe = reloaded.configuration_for_text_probe(&profile).unwrap();
        assert!(
            matches!(probe.credentials, TextTranslationProbeCredentials::Independent(
                TextTranslationCredentials::OpenAICompatible { endpoint, api_key, model }
            ) if endpoint == "http://127.0.0.1:8000/v1/chat/completions" && api_key.is_empty() && model == "local-model")
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
            0
        );
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination_account),
            1
        );
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &speech_account),
            speech_before
        );
    }
}

#[test]
fn failed_openai_compatible_destination_or_catalog_write_preserves_the_working_configuration() {
    for fail_destination in [false, true] {
        let fake = FakeSecretStore::default();
        let mut store = settings(&fake);
        let profile = store.active_profile().unwrap();
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "synthetic-old-asr",
                    "https://old.example/v1",
                    "synthetic-old-key",
                    "old-model",
                ),
            )
            .unwrap();
        let before_key = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
        let before_destination = fake.value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile),
        );
        let before_prefs = store.preferences();
        let blocked_catalog = tempfile::tempdir().unwrap();
        if fail_destination {
            fake.make_unavailable(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile),
            );
        } else {
            store.catalog_path = blocked_catalog.path().into();
        }
        assert!(store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "synthetic-new-asr",
                    "https://new.example/v1",
                    "synthetic-new-key",
                    "new-model"
                )
            )
            .is_err());
        assert_eq!(store.preferences(), before_prefs);
        assert_eq!(
            store.profile(&profile.id).unwrap().text_translation(),
            TextTranslation::OpenAICompatible
        );
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
            before_key
        );
        assert_eq!(
            fake.value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            ),
            before_destination
        );
        fake.state.lock().unwrap().unavailable.clear();
        assert_eq!(
            store.credential_diagnostic(&store.active_profile().unwrap()),
            "present"
        );
        assert!(
            matches!(store.configuration().unwrap().credentials, ProviderCredentials::OpenAICompatible { asr_api_key, api_key, model, .. } if asr_api_key == "synthetic-old-asr" && api_key == "synthetic-old-key" && model == "old-model")
        );
    }
}

#[test]
fn connection_check_configuration_reads_selected_profile_without_switching() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let active_id = store.active_profile().unwrap().id;
    let profile = openai_profile(&store, "Synthetic check profile");
    store
        .save_api_key(&profile.id, "fixture-check-only")
        .unwrap();
    {
        let mut prefs = store.prefs.lock().unwrap();
        prefs.source_language = SourceLanguage::Chinese;
        prefs.target_language = TargetLanguage::Original;
    }
    let before = store.preferences();
    assert!(store.configuration_for_profile(&profile).is_err());
    let configuration = store.configuration_for_profile_probe(&profile).unwrap();
    assert_eq!(configuration.provider, ProviderKind::OpenAIRealtime);
    assert_eq!(configuration.source_language, SourceLanguage::Automatic);
    assert!(configuration.target_language.translates_audio());
    assert_eq!(store.preferences(), before);
    assert_eq!(
        configuration.credentials.direct_api_key(),
        Some("fixture-check-only")
    );
    assert_eq!(store.active_profile().unwrap().id, active_id);
    assert_eq!(
        fake.load_count(
            PROFILE_KEYCHAIN_SERVICE,
            &credential_account(&ServiceProfile::alibaba_default())
        ),
        0
    );
}

#[test]
fn structured_credentials_remain_keychain_only_and_resolve_configuration() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::AzureOpenAIRealtime, "Azure")
        .unwrap();
    let secret = "azure-private-value";
    let credentials = ProviderCredentials::AzureOpenAI {
        endpoint: "https://mimi.openai.azure.com".into(),
        deployment: "translate".into(),
        transcription_deployment: "transcribe".into(),
        api_key: secret.into(),
    };

    store.save_credentials(&profile.id, &credentials).unwrap();
    assert_eq!(store.credential_state(&profile), CredentialState::Present);
    store.select_profile(&profile.id).unwrap();
    let configuration = store.configuration().unwrap();
    assert_eq!(configuration.credentials, credentials);

    let catalog = serde_json::to_string(&store.profile_catalog().unwrap()).unwrap();
    assert!(!catalog.contains(secret));
    assert!(!catalog.contains("mimi.openai.azure.com"));
}

#[test]
fn subtitle_colors_persist_without_changing_provider_configuration() {
    let directory = std::env::temp_dir().join(format!(
        "mimi-subtitle-color-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
    store
        .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "test-key")
        .unwrap();
    let original_configuration = store.configuration().unwrap();
    for color in [
        SubtitleColor::White,
        SubtitleColor::Teal,
        SubtitleColor::Yellow,
        SubtitleColor::Green,
        SubtitleColor::Pink,
        SubtitleColor::Custom([0x12, 0x34, 0x56]),
    ] {
        store
            .save_preferences_for_active_profile(|prefs| prefs.subtitle_color = color)
            .unwrap();
        assert_eq!(store.configuration().unwrap(), original_configuration);
        let reloaded = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
        assert_eq!(reloaded.preferences().subtitle_color, color);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn display_modes_persist_without_changing_provider_configuration() {
    let directory = std::env::temp_dir().join(format!(
        "mimi-display-mode-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
    store
        .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "test-key")
        .unwrap();
    let original_configuration = store.configuration().unwrap();
    for mode in [
        SubtitleDisplayMode::Bilingual,
        SubtitleDisplayMode::Original,
        SubtitleDisplayMode::Translation,
    ] {
        store
            .save_preferences_for_active_profile(|prefs| prefs.subtitle_display_mode = mode)
            .unwrap();
        assert_eq!(store.configuration().unwrap(), original_configuration);
        let reloaded = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
        assert_eq!(reloaded.preferences().subtitle_display_mode, mode);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn subtitle_dividers_persist_without_changing_provider_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    store
        .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-asr")
        .unwrap();
    let original_configuration = store.configuration().unwrap();
    assert!(!store.preferences().show_subtitle_dividers);
    let legacy: Preferences = serde_json::from_str(
        r#"{"keep_subtitle_text_opaque":true,"font_size":22,"subtitle_background_opacity":35}"#,
    )
    .unwrap();
    assert_eq!(legacy.font_size, 22.0);
    assert_eq!(legacy.subtitle_background_opacity, 35);
    assert_eq!(legacy.microphone_subtitle_color, SubtitleColor::Yellow);
    for enabled in [true, false] {
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.show_subtitle_dividers = enabled;
                prefs.microphone_subtitle_color = SubtitleColor::Custom([0x12, 0x34, 0x56]);
            })
            .unwrap();
        assert_eq!(store.configuration().unwrap(), original_configuration);
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(reloaded.preferences().show_subtitle_dividers, enabled);
        assert_eq!(
            reloaded.preferences().microphone_subtitle_color,
            SubtitleColor::Custom([0x12, 0x34, 0x56])
        );
        assert_eq!(reloaded.preferences().subtitle_color, SubtitleColor::White);
    }
}

#[test]
fn subtitle_timestamps_persist_without_changing_provider_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    store
        .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-asr")
        .unwrap();
    let original_configuration = store.configuration().unwrap();
    assert!(!store.preferences().show_subtitle_timestamps);
    for enabled in [true, false] {
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.show_subtitle_timestamps = enabled;
            })
            .unwrap();
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(reloaded.preferences().show_subtitle_timestamps, enabled);
        assert_eq!(reloaded.configuration().unwrap(), original_configuration);
        assert_eq!(
            crate::commands::SettingsSnapshotPayload::try_from_store(&reloaded)
                .unwrap()
                .show_subtitle_timestamps,
            enabled
        );
    }
}

#[test]
fn network_proxy_persists_globally_and_probe_and_session_use_identical_configuration() {
    use crate::core::services::network_proxy::ProxyMode;
    let legacy: Preferences = serde_json::from_str(r#"{"ui_language":"ja"}"#).unwrap();
    assert_eq!(legacy.network_proxy, ProxyConfig::default());
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    store
        .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-asr")
        .unwrap();
    let profile = store.active_profile().unwrap();
    let original = store.configuration().unwrap();
    for proxy in [
        ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some("socks5h://127.0.0.1:1080".into()),
        },
        ProxyConfig {
            mode: ProxyMode::Direct,
            url: None,
        },
        ProxyConfig::default(),
    ] {
        store
            .save_preferences_for_active_profile(|prefs| prefs.network_proxy = proxy.clone())
            .unwrap();
        assert_eq!(store.configuration().unwrap().network_proxy, proxy);
        assert_eq!(
            store
                .configuration_for_profile_probe(&profile)
                .unwrap()
                .network_proxy,
            proxy
        );
        let other = store
            .create_profile(ProviderKind::OpenAIRealtime, "Synthetic")
            .unwrap();
        store.select_profile(&other.id).unwrap();
        assert_eq!(store.preferences().network_proxy, proxy);
        store.select_profile(&profile.id).unwrap();
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(reloaded.preferences().network_proxy, proxy);
    }
    assert_eq!(original.network_proxy, ProxyConfig::default());
}

#[test]
fn original_only_source_is_normalized_to_auto_when_translation_is_selected() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.target_language = TargetLanguage::Original;
            prefs.source_language = SourceLanguage::Greek;
        })
        .unwrap();
    assert_eq!(store.preferences().source_language, SourceLanguage::Greek);
    store
        .save_preferences_for_active_profile(|prefs| prefs.target_language = TargetLanguage::French)
        .unwrap();
    assert_eq!(
        store.preferences().source_language,
        SourceLanguage::Automatic
    );
    assert_eq!(store.preferences().target_language, TargetLanguage::French);
}

#[test]
fn loading_preserves_explicit_chinese_translation_target() {
    let fake = FakeSecretStore::default();
    let directory = std::env::temp_dir().join(format!(
        "mimi-preferences-chinese-original-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let stale = Preferences {
        source_language: SourceLanguage::Chinese,
        target_language: TargetLanguage::English,
        ..Preferences::default()
    };
    std::fs::write(
        directory.join("preferences.json"),
        serde_json::to_vec_pretty(&stale).unwrap(),
    )
    .unwrap();

    let store = SettingsStore::at_path(directory.clone(), Box::new(fake));
    let normalized = store.preferences();
    assert_eq!(normalized.source_language, SourceLanguage::Chinese);
    assert_eq!(normalized.target_language, TargetLanguage::English);

    let persisted: Preferences =
        serde_json::from_slice(&std::fs::read(directory.join("preferences.json")).unwrap())
            .unwrap();
    assert_eq!(persisted, normalized);

    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn explicit_provider_preserves_chinese_translation_target() {
    let mut preferences = Preferences {
        source_language: SourceLanguage::Chinese,
        target_language: TargetLanguage::English,
        ..Preferences::default()
    };

    normalize_preferences_value(
        &mut preferences,
        &ServiceProfile::new("tencent", "Tencent", ProviderKind::TencentCloud).unwrap(),
    );
    assert_eq!(preferences.source_language, SourceLanguage::Chinese);
    assert_eq!(preferences.target_language, TargetLanguage::English);

    preferences.target_language = TargetLanguage::SimplifiedChinese;
    normalize_preferences_value(
        &mut preferences,
        &ServiceProfile::new("tencent", "Tencent", ProviderKind::TencentCloud).unwrap(),
    );
    assert_eq!(
        preferences.target_language,
        TargetLanguage::SimplifiedChinese
    );
}
