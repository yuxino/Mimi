//! Realtime behavior regressions.

use super::*;

#[test]
fn realtime_modes_default_to_simultaneous_and_restore_independent_configuration() {
    use crate::core::services::provider::SpeechTranslationMode;
    for provider in [
        ProviderKind::OpenAIRealtime,
        ProviderKind::GoogleGeminiLive,
        ProviderKind::AzureOpenAIRealtime,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = fresh_store(directory.path(), &fake);
        let profile = store
            .create_profile_for_setup(provider, "Realtime fixture")
            .unwrap();
        assert_eq!(
            profile.speech_translation_mode,
            SpeechTranslationMode::Simultaneous
        );
        store
            .save_credentials(&profile.id, &realtime_speech_credentials(provider))
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        assert!(!store.configuration().unwrap().uses_realtime_asr());
        let profile =
            save_realtime_speech_mode(&store, &profile.id, SpeechTranslationMode::AsrMt).unwrap();
        assert_eq!(
            profile.text_translation(),
            TextTranslation::OpenAICompatible
        );
        assert!(
            store.configuration().is_err(),
            "recognition credentials cannot supply an independent text key"
        );
        let speech = store.configuration_for_speech_probe(&profile).unwrap();
        assert!(speech.uses_realtime_asr());
        assert_eq!(speech.target_language, TargetLanguage::Original);
        assert!(speech.text_credentials.is_none());
        store
            .save_preferences(|prefs| prefs.target_language = TargetLanguage::Original)
            .unwrap();
        assert!(store.configuration().unwrap().text_credentials.is_none());
        store
            .save_credentials(
                &profile.id,
                &xai_text_request("synthetic-independent-text", "chosen-text-model"),
            )
            .unwrap();
        store
            .save_preferences(|prefs| prefs.target_language = TargetLanguage::English)
            .unwrap();
        let speech_before = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
        let text_before = fake.value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile),
        );
        let profile =
            save_realtime_speech_mode(&store, &profile.id, SpeechTranslationMode::Simultaneous)
                .unwrap();
        assert_eq!(profile.text_translation(), TextTranslation::FollowService);
        assert_eq!(profile.text_translation, Some(TextTranslation::XAI));
        assert!(store.configuration().unwrap().text_credentials.is_none());
        assert!(store.configuration_for_text_probe(&profile).is_err());
        let profile =
            save_realtime_speech_mode(&store, &profile.id, SpeechTranslationMode::AsrMt).unwrap();
        assert!(matches!(store.configuration().unwrap().text_credentials,
                Some(TextTranslationCredentials::XAI { api_key, model }) if api_key == "synthetic-independent-text" && model == "chosen-text-model"));
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
            speech_before
        );
        assert_eq!(
            fake.value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile)
            ),
            text_before
        );
        drop(store);
        let reopened = fresh_store(directory.path(), &fake);
        assert_eq!(
            reopened
                .profile(&profile.id)
                .unwrap()
                .speech_translation_mode,
            SpeechTranslationMode::AsrMt
        );
        assert!(reopened.configuration().unwrap().uses_realtime_asr());
    }
}

#[test]
fn azure_asr_configuration_remains_editable_when_simultaneous_needs_a_deployment() {
    use crate::core::services::provider::SpeechTranslationMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore {
        local_file: true,
        ..Default::default()
    };
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::AzureOpenAIRealtime, "Azure fixture")
        .unwrap();
    let profile =
        save_realtime_speech_mode(&store, &profile.id, SpeechTranslationMode::AsrMt).unwrap();
    let credentials = ProviderCredentials::AzureOpenAI {
        endpoint: "https://synthetic.openai.azure.com".into(),
        deployment: String::new(),
        transcription_deployment: "transcription-deployment".into(),
        api_key: "synthetic-recognition-key".into(),
    };
    store.save_credentials(&profile.id, &credentials).unwrap();
    assert!(store.configuration_for_speech_probe(&profile).is_ok());
    let profile =
        save_realtime_speech_mode(&store, &profile.id, SpeechTranslationMode::Simultaneous)
            .unwrap();
    assert!(store.configuration_for_profile_probe(&profile).is_err());
    assert_eq!(
        store.credential_state_for_snapshot(&profile),
        CredentialState::Missing,
        "a missing translation deployment is incomplete configuration, not a file error"
    );
    let editor = store.credential_editor_state(&profile.id, None).unwrap();
    assert_eq!(editor.deployment.as_deref(), Some(""));
    assert_eq!(
        editor.transcription_deployment.as_deref(),
        Some("transcription-deployment")
    );
    assert!(editor.saved_fields.contains(&CredentialRevealField::ApiKey));
    assert_eq!(
        store
            .reveal_credential(&profile.id, CredentialRevealField::ApiKey, None)
            .unwrap()
            .as_deref(),
        Some("synthetic-recognition-key")
    );
    let repaired = ProviderCredentials::AzureOpenAI {
        endpoint: String::new(),
        deployment: "translation-deployment".into(),
        transcription_deployment: String::new(),
        api_key: String::new(),
    };
    assert!(store
        .configuration_for_speech_draft_probe(&profile, &repaired, false)
        .is_ok());
    assert_eq!(
        store
            .credential_editor_state(&profile.id, None)
            .unwrap()
            .deployment
            .as_deref(),
        Some(""),
        "a draft check must not persist the deployment"
    );
}

#[test]
fn azure_incomplete_snapshot_keeps_corrupt_or_unreadable_credentials_unavailable() {
    let fake = FakeSecretStore {
        local_file: true,
        ..Default::default()
    };
    let store = settings(&fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::AzureOpenAIRealtime, "Azure state")
        .unwrap();
    let account = credential_account(&profile);
    assert_eq!(
        store.credential_state_for_snapshot(&profile),
        CredentialState::Missing
    );
    let valid = ProviderCredentials::AzureOpenAI {
        endpoint: "https://synthetic.openai.azure.com".into(),
        deployment: "translation-deployment".into(),
        transcription_deployment: "transcription-deployment".into(),
        api_key: "synthetic-recognition-key".into(),
    };
    store.save_credentials(&profile.id, &valid).unwrap();
    assert_eq!(
        store.credential_state_for_snapshot(&profile),
        CredentialState::Present
    );
    let mut incomplete = serde_json::to_value(&valid).unwrap();
    incomplete["deployment"] = serde_json::json!("");
    let mut invalid_endpoint = incomplete.clone();
    invalid_endpoint["endpoint"] = serde_json::json!("invalid endpoint");
    let mut missing_key = incomplete.clone();
    missing_key["apiKey"] = serde_json::json!("");
    let mut missing_transcription = incomplete;
    missing_transcription["transcriptionDeployment"] = serde_json::json!("");
    for value in [
        "not-json".to_string(),
        invalid_endpoint.to_string(),
        missing_key.to_string(),
        missing_transcription.to_string(),
    ] {
        fake.put(PROFILE_KEYCHAIN_SERVICE, &account, &value);
        store.secret_cache.lock().unwrap().clear();
        assert_eq!(
            store.credential_state_for_snapshot(&profile),
            CredentialState::Unavailable
        );
    }
    fake.put(
        PROFILE_KEYCHAIN_SERVICE,
        &account,
        &valid.encode_for_profile(&profile).unwrap(),
    );
    fake.state.lock().unwrap().failures.insert(
        cache_key(PROFILE_KEYCHAIN_SERVICE, &account),
        SecretStoreError::AccessDenied,
    );
    store.secret_cache.lock().unwrap().clear();
    assert_eq!(
        store.credential_state_for_snapshot(&profile),
        CredentialState::Unavailable
    );
}

#[test]
fn azure_saved_key_repairs_simultaneous_deployment_without_crossing_resource_boundaries() {
    use crate::core::services::provider::SpeechTranslationMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::AzureOpenAIRealtime, "Azure repair")
        .unwrap();
    let profile =
        save_realtime_speech_mode(&store, &profile.id, SpeechTranslationMode::AsrMt).unwrap();
    store
        .save_credentials(
            &profile.id,
            &ProviderCredentials::AzureOpenAI {
                endpoint: "https://synthetic.openai.azure.com".into(),
                deployment: String::new(),
                transcription_deployment: "saved-transcription".into(),
                api_key: "synthetic-saved-key".into(),
            },
        )
        .unwrap();
    let profile =
        save_realtime_speech_mode(&store, &profile.id, SpeechTranslationMode::Simultaneous)
            .unwrap();
    let repair = ProviderCredentials::AzureOpenAI {
        endpoint: "https://SYNTHETIC.openai.azure.com/".into(),
        deployment: "new-translation".into(),
        transcription_deployment: "saved-transcription".into(),
        api_key: String::new(),
    };
    store.save_credentials(&profile.id, &repair).unwrap();
    assert_eq!(
        store
            .configuration_for_profile_probe(&profile)
            .unwrap()
            .credentials,
        ProviderCredentials::AzureOpenAI {
            endpoint: "https://synthetic.openai.azure.com".into(),
            deployment: "new-translation".into(),
            transcription_deployment: "saved-transcription".into(),
            api_key: "synthetic-saved-key".into(),
        }
    );
    let saved = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
    let replacement = ProviderCredentials::AzureOpenAI {
        endpoint: "https://other.openai.azure.com".into(),
        deployment: "new-translation".into(),
        transcription_deployment: "saved-transcription".into(),
        api_key: String::new(),
    };
    assert!(store.save_credentials(&profile.id, &replacement).is_err());
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
        saved
    );
    let other = store
        .create_profile_for_setup(ProviderKind::AzureOpenAIRealtime, "Other Azure")
        .unwrap();
    assert!(store.save_credentials(&other.id, &repair).is_err());
    assert!(fake
        .value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&other))
        .is_none());
}

#[test]
fn realtime_mode_patch_rejects_other_providers_without_mutating_catalog() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::TencentCloud, "Tencent fixture")
        .unwrap();
    let before = std::fs::read(directory.path().join(PROFILE_CATALOG_FILE)).unwrap();
    assert!(save_realtime_speech_mode(
        &store,
        &profile.id,
        crate::core::services::provider::SpeechTranslationMode::AsrMt
    )
    .is_err());
    assert_eq!(
        std::fs::read(directory.path().join(PROFILE_CATALOG_FILE)).unwrap(),
        before
    );
}

#[test]
fn xai_mode_switch_preserves_independent_text_credentials_and_legacy_metadata() {
    use crate::core::services::provider::XAISpeechMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::XAIRealtime, "xAI")
        .unwrap();
    assert_eq!(profile.xai_speech_mode, XAISpeechMode::AsrMt);
    assert_eq!(
        profile.xai_text_model,
        crate::core::services::provider::DEFAULT_XAI_TEXT_MODEL
    );
    store
        .save_api_key(&profile.id, "synthetic-xai-key")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &xai_text_request("synthetic-text-key", "chosen-text-model"),
        )
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    let key_before = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
    store
        .save_preferences(|prefs| prefs.target_language = TargetLanguage::Original)
        .unwrap();
    let voice = save_xai_options(
        &store,
        &profile.id,
        Some(XAISpeechMode::Voice),
        Some(" custom-xai-model "),
    )
    .unwrap();
    assert_eq!(voice.xai_text_model, "custom-xai-model");
    assert_ne!(
        store.preferences().target_language,
        TargetLanguage::Original
    );
    assert!(!store.configuration().unwrap().uses_xai_asr());
    assert_eq!(voice.text_translation, Some(TextTranslation::XAI));
    let streaming =
        save_xai_options(&store, &profile.id, Some(XAISpeechMode::AsrMt), None).unwrap();
    store
        .save_preferences(|prefs| prefs.target_language = TargetLanguage::Original)
        .unwrap();
    let configuration = store.configuration().unwrap();
    assert!(configuration.uses_xai_asr());
    assert_eq!(configuration.target_language, TargetLanguage::Original);
    assert_eq!(configuration.xai_text_model, "custom-xai-model");
    assert_eq!(
        store
            .configuration_for_speech_probe(&streaming)
            .unwrap()
            .target_language,
        TargetLanguage::Original
    );
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
        key_before
    );
    drop(store);
    let reopened = fresh_store(directory.path(), &fake);
    let profile = reopened.profile(&profile.id).unwrap();
    assert_eq!(profile.xai_speech_mode, XAISpeechMode::AsrMt);
    assert_eq!(profile.xai_text_model, "custom-xai-model");
    assert!(reopened.configuration().unwrap().uses_xai_asr());
    assert!(
        matches!(reopened.configuration_for_text_probe(&profile).unwrap().credentials,
            crate::core::services::configuration::TextTranslationProbeCredentials::Independent(TextTranslationCredentials::XAI { model, api_key })
                if model == "chosen-text-model" && api_key == "synthetic-text-key")
    );
}

#[test]
fn xai_checks_use_independent_keys_and_proxies_without_persisting_drafts() {
    use crate::core::services::configuration::TextTranslationProbeCredentials;
    use crate::core::services::network_proxy::ProxyMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::XAIRealtime, "xAI")
        .unwrap();
    let profile = store
        .update_profile_options_with_xai(
            &profile.id,
            None,
            Some(ProxyConfig {
                mode: ProxyMode::Direct,
                url: None,
            }),
            Some(ProxyConfig {
                mode: ProxyMode::Custom,
                url: Some("http://127.0.0.1:8080".into()),
            }),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let catalog_before = std::fs::read(directory.path().join(PROFILE_CATALOG_FILE)).unwrap();
    let speech = store
        .configuration_for_speech_draft_probe(
            &profile,
            &ProviderCredentials::api_key("synthetic-unsaved-speech"),
            true,
        )
        .unwrap();
    assert!(speech.uses_xai_asr());
    assert_eq!(speech.target_language, TargetLanguage::Original);
    assert_eq!(speech.network_proxy.mode, ProxyMode::Direct);
    let text = store
        .configuration_for_text_draft_probe(
            &profile,
            &xai_text_request("synthetic-unsaved-text", "chosen-grok"),
        )
        .unwrap();
    assert_eq!(text.network_proxy.mode, ProxyMode::Custom);
    assert!(matches!(text.credentials,
            TextTranslationProbeCredentials::Independent(TextTranslationCredentials::XAI { api_key, model })
                if api_key == "synthetic-unsaved-text" && model == "chosen-grok"));
    assert_eq!(
        std::fs::read(directory.path().join(PROFILE_CATALOG_FILE)).unwrap(),
        catalog_before
    );
    assert!(store.credentials_for_profile(&profile).unwrap().is_none());
    assert!(store.destination_value(&profile).unwrap().is_none());
    store
        .save_api_key(&profile.id, "synthetic-speech-key")
        .unwrap();
    assert_eq!(
        store
            .credential_editor_state(&profile.id, None)
            .unwrap()
            .saved_fields,
        vec![CredentialRevealField::ApiKey]
    );
    assert!(
        store.configuration_for_text_probe(&profile).is_err(),
        "a speech key is never borrowed for text"
    );
    store
        .save_credentials(
            &profile.id,
            &xai_text_request("synthetic-text-key", "saved-grok"),
        )
        .unwrap();
    assert!(
        matches!(store.configuration_for_text_probe(&store.profile(&profile.id).unwrap()).unwrap().credentials,
            TextTranslationProbeCredentials::Independent(TextTranslationCredentials::XAI { api_key, model })
                if api_key == "synthetic-text-key" && model == "saved-grok")
    );
    assert_eq!(
        store
            .credential_editor_state(&profile.id, Some(TextTranslation::XAI))
            .unwrap()
            .model
            .as_deref(),
        Some("saved-grok")
    );
    assert_eq!(
        store
            .reveal_credential(
                &profile.id,
                CredentialRevealField::Token,
                Some(TextTranslation::XAI)
            )
            .unwrap()
            .as_deref(),
        Some("synthetic-text-key")
    );
}

#[test]
fn legacy_xai_stt_migrates_once_and_deleted_text_credentials_never_return() {
    for legacy_voice in [false, true] {
        use crate::core::services::provider::XAISpeechMode;
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = fresh_store(directory.path(), &fake);
        let profile = store
            .create_profile_for_setup(ProviderKind::XAIRealtime, "Old xAI")
            .unwrap();
        store
            .save_api_key(&profile.id, "synthetic-old-key")
            .unwrap();
        {
            let mut catalog = store.catalog.lock().unwrap();
            let old = catalog
                .profiles
                .iter_mut()
                .find(|p| p.id == profile.id)
                .unwrap();
            old.xai_independent_translation_migrated = false;
            old.text_translation = None;
            old.xai_text_model = "legacy-grok-model".into();
            if legacy_voice {
                old.xai_speech_mode = XAISpeechMode::Voice;
            }
        }
        store.persist_catalog().unwrap();
        drop(store);
        let store = fresh_store(directory.path(), &fake);
        if legacy_voice {
            assert!(
                !store
                    .profile(&profile.id)
                    .unwrap()
                    .xai_independent_translation_migrated
            );
            assert!(store.destination_value(&profile).unwrap().is_none());
            save_xai_options(&store, &profile.id, Some(XAISpeechMode::AsrMt), None).unwrap();
        }
        let migrated = store.profile(&profile.id).unwrap();
        assert!(migrated.xai_independent_translation_migrated);
        assert_eq!(migrated.text_translation, Some(TextTranslation::XAI));
        assert!(
            matches!(store.text_credentials_for_profile(&migrated).unwrap(),
                Some(TextTranslationCredentials::XAI { api_key, model })
                    if api_key == "synthetic-old-key" && model == "legacy-grok-model")
        );
        store.delete_api_key(&profile.id).unwrap();
        store
            .save_api_key(&profile.id, "synthetic-replacement-speech")
            .unwrap();
        assert!(store.configuration_for_text_probe(&migrated).is_err());
        store
            .save_credentials(
                &profile.id,
                &ProviderCredentials::AlibabaTranslation {
                    api_key: String::new(),
                    text_translation: TextTranslation::FollowService,
                    endpoint: String::new(),
                    token: String::new(),
                    model: String::new(),
                    clear_token: false,
                },
            )
            .unwrap();
        drop(store);
        let reloaded = fresh_store(directory.path(), &fake);
        let selected = reloaded.profile(&profile.id).unwrap();
        assert_eq!(selected.text_translation(), TextTranslation::FollowService);
        assert_eq!(
            reloaded.configuration().unwrap().target_language,
            TargetLanguage::Original
        );
        assert!(reloaded.destination_value(&selected).unwrap().is_none());
    }
}

#[test]
fn failed_legacy_xai_mode_migration_preserves_the_saved_mode_and_preferences() {
    use crate::core::services::provider::XAISpeechMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::XAIRealtime, "Legacy voice")
        .unwrap();
    store
        .save_api_key(&profile.id, "synthetic-legacy-key")
        .unwrap();
    save_xai_options(&store, &profile.id, Some(XAISpeechMode::Voice), None).unwrap();
    mark_legacy_xai_profile(&store, &profile.id);
    store
        .save_preferences(|prefs| {
            prefs.source_language = SourceLanguage::Japanese;
            prefs.target_language = TargetLanguage::Korean;
        })
        .unwrap();
    let catalog_before = std::fs::read(&store.catalog_path).unwrap();
    let prefs_before = std::fs::read(&store.prefs_path).unwrap();
    let preferences = store.preferences();
    fake.state.lock().unwrap().saves_before_failure = Some(0);
    assert!(save_xai_options(&store, &profile.id, Some(XAISpeechMode::AsrMt), None).is_err());
    assert_eq!(std::fs::read(&store.catalog_path).unwrap(), catalog_before);
    assert_eq!(std::fs::read(&store.prefs_path).unwrap(), prefs_before);
    assert_eq!(store.preferences(), preferences);
    assert_eq!(
        store.catalog.lock().unwrap().profiles[0].xai_speech_mode,
        XAISpeechMode::Voice
    );
    fake.state.lock().unwrap().saves_before_failure = None;
    let migrated = save_xai_options(&store, &profile.id, Some(XAISpeechMode::AsrMt), None).unwrap();
    assert!(migrated.xai_independent_translation_migrated);
    assert!(
        matches!(store.text_credentials_for_profile(&migrated).unwrap(),
            Some(TextTranslationCredentials::XAI { api_key, .. }) if api_key == "synthetic-legacy-key")
    );
}

#[test]
fn legacy_xai_mode_retry_reuses_verified_destination_after_catalog_failure() {
    use crate::core::services::provider::XAISpeechMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::XAIRealtime, "Legacy voice")
        .unwrap();
    store
        .save_api_key(&profile.id, "synthetic-original-key")
        .unwrap();
    save_xai_options(&store, &profile.id, Some(XAISpeechMode::Voice), None).unwrap();
    mark_legacy_xai_profile(&store, &profile.id);
    let backup = directory.path().join("catalog-backup");
    std::fs::rename(&store.catalog_path, &backup).unwrap();
    std::fs::create_dir(&store.catalog_path).unwrap();
    assert!(save_xai_options(&store, &profile.id, Some(XAISpeechMode::AsrMt), None).is_err());
    assert_eq!(
        store.catalog.lock().unwrap().profiles[0].xai_speech_mode,
        XAISpeechMode::Voice
    );
    let verified = fake
        .value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile),
        )
        .unwrap();
    std::fs::remove_dir(&store.catalog_path).unwrap();
    std::fs::rename(&backup, &store.catalog_path).unwrap();
    store
        .save_api_key(&profile.id, "synthetic-replacement-speech-key")
        .unwrap();
    let migrated = save_xai_options(&store, &profile.id, Some(XAISpeechMode::AsrMt), None).unwrap();
    assert!(migrated.xai_independent_translation_migrated);
    assert_eq!(
        fake.value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        )
        .as_deref(),
        Some(verified.as_str())
    );
    assert!(
        matches!(store.text_credentials_for_profile(&migrated).unwrap(),
            Some(TextTranslationCredentials::XAI { api_key, .. }) if api_key == "synthetic-original-key")
    );
}

#[test]
fn legacy_xai_model_without_a_key_remains_editable_and_never_borrows_a_later_key() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::XAIRealtime, "No key yet")
        .unwrap();
    mark_legacy_xai_profile(&store, &profile.id);
    let profile = store.profile(&profile.id).unwrap();
    let editor = store
        .credential_editor_state(&profile.id, Some(TextTranslation::XAI))
        .unwrap();
    assert_eq!(editor.model.as_deref(), Some("legacy-custom-grok"));
    assert!(editor.saved_fields.is_empty());
    assert!(store
        .text_credentials_for_profile(&profile)
        .unwrap()
        .is_none());
    store
        .save_api_key(&profile.id, "synthetic-later-speech-key")
        .unwrap();
    assert!(store.configuration_for_text_probe(&profile).is_err());
    store
        .save_credentials(
            &profile.id,
            &xai_text_request("synthetic-explicit-text-key", ""),
        )
        .unwrap();
    assert!(
        matches!(store.text_credentials_for_profile(&profile).unwrap(),
            Some(TextTranslationCredentials::XAI { api_key, model })
                if api_key == "synthetic-explicit-text-key" && model == "legacy-custom-grok")
    );
}

#[test]
fn legacy_xai_upgrade_retries_transient_credential_errors_in_the_same_store() {
    for fail_write in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = fresh_store(directory.path(), &fake);
        let profile = store
            .create_profile_for_setup(ProviderKind::XAIRealtime, "Old xAI")
            .unwrap();
        store
            .save_api_key(&profile.id, "synthetic-migration-key")
            .unwrap();
        mark_legacy_xai_profile(&store, &profile.id);
        let account = SettingsStore::destination_account(&profile);
        store.secret_cache.lock().unwrap().clear();
        if fail_write {
            fake.state.lock().unwrap().saves_before_failure = Some(0);
        } else {
            fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        }
        assert!(store.profile_catalog().is_err());
        assert!(!store.catalog.lock().unwrap().profiles[0].xai_independent_translation_migrated);
        {
            let mut state = fake.state.lock().unwrap();
            state.saves_before_failure = None;
            state.unavailable.clear();
        }
        let profile = store.profile(&profile.id).unwrap();
        assert!(profile.xai_independent_translation_migrated);
        assert!(
            matches!(store.text_credentials_for_profile(&profile).unwrap(),
                Some(TextTranslationCredentials::XAI { api_key, model })
                    if api_key == "synthetic-migration-key" && model == "legacy-custom-grok")
        );
    }
}

#[test]
fn legacy_xai_upgrade_reuses_verified_destination_after_catalog_write_failure() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::XAIRealtime, "Old xAI")
        .unwrap();
    store
        .save_api_key(&profile.id, "synthetic-original-key")
        .unwrap();
    mark_legacy_xai_profile(&store, &profile.id);
    let file = directory.path().join(PROFILE_CATALOG_FILE);
    let backup = directory.path().join("catalog-backup");
    std::fs::rename(&file, &backup).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert!(store.profile_catalog().is_err());
    assert!(!store.catalog.lock().unwrap().profiles[0].xai_independent_translation_migrated);
    let verified = fake
        .value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile),
        )
        .unwrap();
    std::fs::remove_dir(&file).unwrap();
    std::fs::rename(&backup, &file).unwrap();
    assert!(
        store
            .profile(&profile.id)
            .unwrap()
            .xai_independent_translation_migrated
    );
    assert_eq!(
        fake.value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        )
        .as_deref(),
        Some(verified.as_str())
    );
}

#[test]
fn xai_saved_route_metadata_can_be_restored_without_revealing_an_inactive_key() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::XAIRealtime, "xAI")
        .unwrap();
    store
        .save_api_key(&profile.id, "synthetic-recognition-key")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &xai_text_request("synthetic-grok-key", "saved-custom-grok"),
        )
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &ProviderCredentials::AlibabaTranslation {
                api_key: String::new(),
                text_translation: TextTranslation::DeepL,
                endpoint: String::new(),
                token: "synthetic-deepl:fx".into(),
                model: String::new(),
                clear_token: false,
            },
        )
        .unwrap();
    let editor = store
        .credential_editor_state(&profile.id, Some(TextTranslation::XAI))
        .unwrap();
    assert_eq!(editor.model.as_deref(), Some("saved-custom-grok"));
    assert_eq!(editor.saved_fields, vec![CredentialRevealField::Token]);
    assert_eq!(
        store.profile(&profile.id).unwrap().text_translation(),
        TextTranslation::DeepL
    );
    assert!(store
        .reveal_credential(
            &profile.id,
            CredentialRevealField::Token,
            Some(TextTranslation::XAI)
        )
        .is_err());
    store
        .save_credentials(&profile.id, &xai_text_request("", ""))
        .unwrap();
    assert!(
        matches!(store.text_credentials_for_profile(&store.profile(&profile.id).unwrap()).unwrap(),
            Some(TextTranslationCredentials::XAI { api_key, model })
                if api_key == "synthetic-grok-key" && model == "saved-custom-grok")
    );
}

#[test]
fn xai_voice_probes_never_change_provider_or_use_an_independent_text_route() {
    use crate::core::services::provider::XAISpeechMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::XAIRealtime, "Voice")
        .unwrap();
    store
        .save_api_key(&profile.id, "synthetic-voice-key")
        .unwrap();
    let voice = save_xai_options(&store, &profile.id, Some(XAISpeechMode::Voice), None).unwrap();
    let draft = store
        .configuration_for_speech_draft_probe(
            &voice,
            &ProviderCredentials::api_key("synthetic-voice-draft"),
            false,
        )
        .unwrap();
    assert_eq!(draft.provider, ProviderKind::XAIRealtime);
    assert!(!draft.uses_xai_asr());
    assert_eq!(
        store
            .configuration_for_speech_probe(&voice)
            .unwrap()
            .provider,
        ProviderKind::XAIRealtime
    );
    assert!(store.configuration_for_text_probe(&voice).is_err());
    assert!(store
        .configuration_for_text_draft_probe(
            &voice,
            &xai_text_request("synthetic-text-key", "grok-4.3")
        )
        .is_err());
    assert!(store
        .save_credentials(
            &profile.id,
            &xai_text_request("synthetic-text-key", "grok-4.3")
        )
        .is_err());
}

#[test]
fn alibaba_original_to_translation_restores_the_selected_grok_destination() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = ServiceProfile::alibaba_default();
    store
        .save_credentials(
            &profile.id,
            &ProviderCredentials::AlibabaTranslation {
                api_key: "synthetic-alibaba-key".into(),
                text_translation: TextTranslation::XAI,
                endpoint: String::new(),
                token: "synthetic-grok-key".into(),
                model: "chosen-grok".into(),
                clear_token: false,
            },
        )
        .unwrap();
    let profile = store.profile(&profile.id).unwrap();
    store
        .save_preferences(|prefs| prefs.target_language = TargetLanguage::Original)
        .unwrap();
    let original = store.configuration().unwrap();
    assert!(original.text_credentials.is_none());
    let selection = profile
        .preferences_after_target_switch(
            ProviderPreferences {
                source_language: original.source_language,
                target_language: TargetLanguage::Original,
                translation_mode: original.translation_mode,
            },
            TargetLanguage::Japanese,
        )
        .unwrap();
    let translated = store
        .configuration_for_target_switch(&profile, original, selection)
        .unwrap();
    assert!(matches!(translated.text_credentials,
            Some(TextTranslationCredentials::XAI { api_key, model }) if api_key=="synthetic-grok-key" && model=="chosen-grok"));
    assert_eq!(
        translated.credentials.direct_api_key(),
        Some("synthetic-alibaba-key")
    );
}

#[test]
fn xai_metadata_rejections_do_not_change_catalog_or_credentials() {
    use crate::core::services::provider::XAISpeechMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile_for_setup(ProviderKind::XAIRealtime, "xAI")
        .unwrap();
    let before = std::fs::read(directory.path().join(PROFILE_CATALOG_FILE)).unwrap();
    assert!(save_xai_options(&store, &profile.id, None, Some("bad\nmodel")).is_err());
    assert_eq!(
        std::fs::read(directory.path().join(PROFILE_CATALOG_FILE)).unwrap(),
        before
    );
    assert!(save_xai_options(
        &store,
        DEFAULT_ALIBABA_PROFILE_ID,
        Some(XAISpeechMode::Voice),
        None
    )
    .is_err());
    assert_eq!(
        std::fs::read(directory.path().join(PROFILE_CATALOG_FILE)).unwrap(),
        before
    );
}

#[test]
fn new_alibaba_setup_defaults_to_livetranslate_and_survives_restart() {
    use crate::core::services::provider::AlibabaRealtimeMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let legacy = store
        .create_profile(ProviderKind::AlibabaCloud, "Legacy")
        .unwrap();
    let profile = store
        .create_profile_for_setup(ProviderKind::AlibabaCloud, "New")
        .unwrap();
    assert_eq!(
        profile.alibaba_realtime_mode,
        AlibabaRealtimeMode::LiveTranslate
    );
    assert!(profile.alibaba_workspace_id.is_empty());
    assert_eq!(
        store.profile(&legacy.id).unwrap().alibaba_realtime_mode,
        AlibabaRealtimeMode::Disabled
    );
    let copy = store.duplicate_profile(&legacy.id, "Copy").unwrap();
    assert_eq!(copy.alibaba_realtime_mode, AlibabaRealtimeMode::Disabled);
    let other = store
        .create_profile_for_setup(ProviderKind::OpenAIRealtime, "Other")
        .unwrap();
    assert_eq!(other.alibaba_realtime_mode, AlibabaRealtimeMode::Disabled);
    drop(store);
    let reopened = fresh_store(directory.path(), &fake);
    assert_eq!(
        reopened.profile(&profile.id).unwrap().alibaba_realtime_mode,
        AlibabaRealtimeMode::LiveTranslate
    );
    assert_eq!(
        reopened.profile(&legacy.id).unwrap().alibaba_realtime_mode,
        AlibabaRealtimeMode::Disabled
    );
}

#[test]
fn alibaba_realtime_metadata_switch_reuses_key_and_preserves_text_destination() {
    use crate::core::services::provider::AlibabaRealtimeMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Synthetic")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &translation_request(
                TextTranslation::DeepL,
                "synthetic-asr",
                "",
                "synthetic-text",
            ),
        )
        .unwrap();
    let speech_before = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
    let text_before = fake.value(
        PROFILE_KEYCHAIN_SERVICE,
        &SettingsStore::destination_account(&profile),
    );
    let selected = store
        .update_profile_options_with_realtime(
            &profile.id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(AlibabaRealtimeMode::LiveTranslate),
            Some("synthetic-workspace"),
        )
        .unwrap();
    let configuration = store.configuration().unwrap();
    assert_eq!(
        configuration.alibaba_realtime_mode,
        AlibabaRealtimeMode::LiveTranslate
    );
    assert_eq!(configuration.alibaba_workspace_id, "synthetic-workspace");
    assert_eq!(configuration.source_language, SourceLanguage::Automatic);
    assert_eq!(
        configuration.credentials.alibaba_key(),
        Some("synthetic-asr")
    );
    assert_eq!(selected.text_translation, Some(TextTranslation::DeepL));
    assert_eq!(
        store
            .configuration_for_speech_draft_probe(
                &selected,
                &ProviderCredentials::api_key(""),
                true,
            )
            .unwrap()
            .alibaba_realtime_mode,
        AlibabaRealtimeMode::LiveTranslate
    );
    assert_eq!(
        store
            .configuration_for_speech_probe(&selected)
            .unwrap()
            .alibaba_realtime_mode,
        AlibabaRealtimeMode::LiveTranslate
    );
    store
        .update_profile_options_with_realtime(
            &profile.id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(AlibabaRealtimeMode::Disabled),
            None,
        )
        .unwrap();
    assert_eq!(
        store.profile(&profile.id).unwrap().text_translation(),
        TextTranslation::DeepL
    );
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
        speech_before
    );
    assert_eq!(
        fake.value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        ),
        text_before
    );
}

#[test]
fn alibaba_realtime_missing_workspace_does_not_commit_partial_profile_changes() {
    use crate::core::services::provider::AlibabaRealtimeMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Synthetic")
        .unwrap();
    let before = std::fs::read(directory.path().join(PROFILE_CATALOG_FILE)).unwrap();
    assert!(store
        .update_profile_options_with_realtime(
            &profile.id,
            Some("Changed"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(AlibabaRealtimeMode::LiveTranslate),
            Some(""),
        )
        .is_err());
    assert_eq!(
        std::fs::read(directory.path().join(PROFILE_CATALOG_FILE)).unwrap(),
        before
    );
    assert_eq!(store.profile(&profile.id).unwrap(), profile);
}

#[test]
fn qwen_model_metadata_survives_reopening_settings_without_reading_credentials() {
    use crate::core::protocols::translation::qwen_mt::QwenMTModel;
    let directory = tempfile::tempdir().unwrap();
    atomic_write(
        &directory.path().join(PROFILE_CATALOG_FILE),
        &serde_json::to_vec(&ProfileCatalog::legacy_alibaba()).unwrap(),
    )
    .unwrap();
    let fake = FakeSecretStore::default();
    let open = || {
        SettingsStore::load_with_secret(
            directory.path().into(),
            false,
            Box::new(fake.clone()),
            PROFILE_KEYCHAIN_SERVICE,
        )
    };
    let store = open();
    store
        .update_profile_options_with_model(
            DEFAULT_ALIBABA_PROFILE_ID,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(QwenMTModel::Flash),
        )
        .unwrap();
    drop(store);
    assert_eq!(
        open().active_profile().unwrap().qwen_mt_model,
        QwenMTModel::Flash
    );
    assert_eq!(
        fake.load_count(
            PROFILE_KEYCHAIN_SERVICE,
            "provider-profile:alibaba-default:alibabaCloud:api-key"
        ),
        0
    );
}

#[test]
fn qwen_model_selection_is_scoped_and_reaches_both_session_and_text_probe() {
    use crate::core::protocols::translation::qwen_mt::QwenMTModel;
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    store
        .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "synthetic-asr")
        .unwrap();
    let second = store
        .create_profile(ProviderKind::AlibabaCloud, "Second")
        .unwrap();
    for model in [QwenMTModel::Flash, QwenMTModel::Plus, QwenMTModel::Lite] {
        let profile = store
            .update_profile_options_with_model(
                DEFAULT_ALIBABA_PROFILE_ID,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(model),
            )
            .unwrap();
        assert_eq!(store.configuration().unwrap().qwen_mt_model, model);
        assert_eq!(
            store
                .configuration_for_text_probe(&profile)
                .unwrap()
                .qwen_mt_model,
            model
        );
        assert_eq!(
            store
                .profile_catalog()
                .unwrap()
                .1
                .iter()
                .find(|p| p.id == second.id)
                .unwrap()
                .qwen_mt_model,
            QwenMTModel::Lite
        );
    }
    let other = store
        .create_profile(ProviderKind::GoogleGeminiLive, "Other")
        .unwrap();
    assert!(store
        .update_profile_options_with_model(
            &other.id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(QwenMTModel::Flash)
        )
        .is_err());
}
