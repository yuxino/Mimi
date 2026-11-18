//! Profiles behavior regressions.

use super::*;

#[test]
fn duplicate_profile_copies_private_slots_and_metadata_without_selecting() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let source = store
        .create_profile(ProviderKind::AlibabaCloud, "Original")
        .unwrap();
    store
        .save_api_key(&source.id, "synthetic-original-key")
        .unwrap();
    store
        .write_destination_value(&source, Some("synthetic-retained-destination"), false)
        .unwrap();
    let mut preset = source.clone();
    preset.language_preset = Some(crate::core::provider::ProfileLanguagePreset {
        source_language: SourceLanguage::Japanese,
        target_language: TargetLanguage::SimplifiedChinese,
    });
    preset.qwen_mt_model = crate::core::protocols::qwen_mt::QwenMTModel::Flash;
    store
        .mutate_catalog_with_normalization(None, |catalog| {
            catalog.profiles[0] = preset.clone();
            Ok(())
        })
        .unwrap();
    let before = store.preferences();
    let copy = store
        .duplicate_profile(&source.id, "Original (copy)")
        .unwrap();
    assert_ne!(copy.id, source.id);
    assert_eq!(copy.language_preset, preset.language_preset);
    assert_eq!(copy.qwen_mt_model, preset.qwen_mt_model);
    assert_eq!(store.active_profile().unwrap().id, source.id);
    assert_eq!(store.preferences(), before);
    assert_eq!(
        store.load_api_key_for_profile(&copy).unwrap().as_deref(),
        Some("synthetic-original-key")
    );
    assert_eq!(
        store.destination_value(&copy).unwrap().as_deref(),
        Some("synthetic-retained-destination")
    );
    store.save_api_key(&copy.id, "synthetic-copy-key").unwrap();
    assert_eq!(
        store.load_api_key_for_profile(&source).unwrap().as_deref(),
        Some("synthetic-original-key")
    );
    let public = std::fs::read_to_string(directory.path().join("service-profiles.json")).unwrap();
    assert!(!public.contains("synthetic-original-key"));
    assert!(!public.contains("synthetic-retained-destination"));
    drop(store);
    let reopened = fresh_store(directory.path(), &fake);
    assert_eq!(reopened.profile_catalog().unwrap().1.len(), 2);
    assert_eq!(reopened.active_profile().unwrap().id, source.id);
}

#[test]
fn duplicate_profile_failure_keeps_original_and_removes_partial_new_slots() {
    for catalog_failure in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = fresh_store(directory.path(), &fake);
        let source = store
            .create_profile(ProviderKind::AlibabaCloud, "Original")
            .unwrap();
        store
            .save_api_key(&source.id, "synthetic-original-key")
            .unwrap();
        store
            .write_destination_value(&source, Some("synthetic-destination"), false)
            .unwrap();
        let catalog = store.profile_catalog().unwrap();
        let values = fake.state.lock().unwrap().values.clone();
        let cached_slots = store.secret_cache.lock().unwrap().clone();
        if catalog_failure {
            std::fs::remove_file(&store.catalog_path).unwrap();
            std::fs::create_dir(&store.catalog_path).unwrap();
        }
        for _ in 0..16 {
            if !catalog_failure {
                // The first private slot saves; the second one fails.
                fake.state.lock().unwrap().saves_before_failure = Some(1);
            }
            assert!(store.duplicate_profile(&source.id, "Copy").is_err());
            assert_eq!(store.profile_catalog().unwrap(), catalog);
            assert_eq!(fake.state.lock().unwrap().values, values);
            assert_eq!(*store.secret_cache.lock().unwrap(), cached_slots);
        }
        if catalog_failure {
            std::fs::remove_dir(&store.catalog_path).unwrap();
        } else {
            fake.state.lock().unwrap().saves_before_failure = None;
        }
        let copy = store
            .duplicate_profile(&source.id, "Recovered copy")
            .unwrap();
        assert_ne!(copy.id, source.id);
        assert_eq!(
            store.profile_catalog().unwrap().1.len(),
            catalog.1.len() + 1
        );
        assert_eq!(
            store.load_api_key_for_profile(&source).unwrap().as_deref(),
            Some("synthetic-original-key")
        );
    }
}

#[test]
fn duplicate_profile_bounds_and_unavailable_source_fail_before_writing() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let source = store.active_profile().unwrap();
    let before = store.profile_catalog().unwrap();
    assert!(store.duplicate_profile("missing", "Copy").is_err());
    assert!(store.duplicate_profile(&source.id, "").is_err());
    assert!(store
        .duplicate_profile(&source.id, &"長".repeat(65))
        .is_err());
    assert_eq!(store.profile_catalog().unwrap(), before);
    let copy = store
        .duplicate_profile(&source.id, "Incomplete copy")
        .unwrap();
    assert!(store.load_api_key_for_profile(&copy).unwrap().is_none());
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &credential_account(&source));
    store.secret_cache.lock().unwrap().clear();
    assert!(store
        .duplicate_profile(&source.id, "Unavailable copy")
        .is_err());
    fake.state.lock().unwrap().unavailable.clear();
    assert!(store
        .duplicate_profile(&source.id, "Recovered copy")
        .is_ok());
    for index in store.profile_catalog().unwrap().1.len()..MAXIMUM_PROFILE_COUNT {
        store
            .create_profile(ProviderKind::OpenAIRealtime, &format!("Profile {index}"))
            .unwrap();
    }
    assert!(store.duplicate_profile(&copy.id, "Over limit").is_err());
    assert_eq!(
        store.profile_catalog().unwrap().1.len(),
        MAXIMUM_PROFILE_COUNT
    );
}

#[test]
fn first_confirmed_profile_becomes_active_and_survives_restart() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = fresh_store(directory.path(), &fake);
    let first = store
        .create_profile(ProviderKind::GoogleGeminiLive, "First service")
        .unwrap();
    assert_eq!(store.active_profile().unwrap(), first);
    let second = store
        .create_profile(ProviderKind::AlibabaCloud, "Second service")
        .unwrap();
    drop(store);
    let restarted = fresh_store(directory.path(), &fake);
    assert_eq!(restarted.active_profile().unwrap(), first);
    assert_eq!(restarted.profile_catalog().unwrap().1, vec![first, second]);
}

#[test]
fn profile_selection_preserves_original_without_reading_custom_text_credentials() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let next = store
        .create_profile(ProviderKind::CustomOpenAIASR, "Recognition only")
        .unwrap();
    store
        .save_credentials(
            &next.id,
            &custom_speech_request(
                "wss://speech.example/realtime",
                "synthetic-model",
                "synthetic-key",
            ),
        )
        .unwrap();
    store
        .save_credentials(
            &next.id,
            &translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
        )
        .unwrap();
    let next = store.profile(&next.id).unwrap();
    let destination = SettingsStore::destination_account(&next);
    store
        .secret_cache
        .lock()
        .unwrap()
        .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &destination));
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &destination);
    let previous_reads = fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination);
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.target_language = TargetLanguage::Original
        })
        .unwrap();

    let proposed = store.configuration_for_profile_selection(&next).unwrap();
    assert_eq!(proposed.target_language, TargetLanguage::Original);
    assert_eq!(proposed.text_credentials, None);
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination),
        previous_reads
    );
    assert_eq!(
        store.active_profile().unwrap().id,
        DEFAULT_ALIBABA_PROFILE_ID
    );
}

#[test]
fn speech_draft_reuses_saved_key_and_checks_new_profile_without_persistence() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::OpenAIRealtime, "Integrated draft")
        .unwrap();
    let new = store
        .configuration_for_speech_draft_probe(
            &profile,
            &ProviderCredentials::api_key("synthetic-new"),
            false,
        )
        .unwrap();
    assert_eq!(new.credentials.direct_api_key(), Some("synthetic-new"));
    assert!(fake.state.lock().unwrap().values.is_empty());
    store
        .save_credentials(
            &profile.id,
            &ProviderCredentials::api_key("synthetic-saved"),
        )
        .unwrap();
    let before = fake.state.lock().unwrap().values.clone();
    let merged = store
        .configuration_for_speech_draft_probe(&profile, &ProviderCredentials::api_key(""), false)
        .unwrap();
    assert_eq!(merged.credentials.direct_api_key(), Some("synthetic-saved"));
    assert_eq!(fake.state.lock().unwrap().values, before);
}

#[test]
fn profile_metadata_patches_preserve_independent_names_and_proxies() {
    use crate::core::network_proxy::ProxyMode;
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let profile = store.active_profile().unwrap();
    let configuration_name = "B 站 · @home / (测试) 😀";
    let translator_name = "翻译 & 字幕 + [本地] 🐱";
    store
        .update_profile(&profile.id, &format!("  {configuration_name}  "))
        .unwrap();
    let text_proxy = ProxyConfig {
        mode: ProxyMode::Direct,
        url: None,
    };
    store
        .update_profile_options(
            &profile.id,
            None,
            None,
            Some(text_proxy.clone()),
            Some(TextTranslationName {
                route: TextTranslation::OpenAICompatible,
                name: format!("  {translator_name}  "),
            }),
            None,
            None,
            None,
        )
        .unwrap();
    // An independently saved alias or proxy must not replay a name copied
    // from an older snapshot. Renaming in turn preserves those fields.
    assert_eq!(store.profile(&profile.id).unwrap().name, configuration_name);
    let renamed = format!("{configuration_name} 2");
    store.update_profile(&profile.id, &renamed).unwrap();
    store
        .update_profile_options(
            &profile.id,
            None,
            Some(ProxyConfig::default()),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let saved = reloaded.profile(&profile.id).unwrap();
    assert_eq!(saved.name, renamed);
    assert_eq!(saved.text_network_proxy, Some(text_proxy));
    assert_eq!(saved.speech_network_proxy, Some(ProxyConfig::default()));
    assert_eq!(
        saved.text_translation_names[&TextTranslation::OpenAICompatible],
        translator_name
    );
    let secrets = fake.state.lock().unwrap();
    assert!(secrets.loads.is_empty());
    assert!(secrets.values.is_empty());
}

#[test]
fn profile_name_patches_enforce_unicode_bounds_without_losing_saved_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let store = SettingsStore::at_path(
        directory.path().into(),
        Box::new(FakeSecretStore::default()),
    );
    let profile = store.active_profile().unwrap();
    let name = "😀".repeat(64);
    let saved = store
        .update_profile_options(
            &profile.id,
            Some(&name),
            None,
            None,
            Some(TextTranslationName {
                route: TextTranslation::OpenAICompatible,
                name: name.clone(),
            }),
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(saved.name, name);
    for invalid in ["  ".to_string(), "😀".repeat(65)] {
        assert!(store.update_profile(&profile.id, &invalid).is_err());
        assert_eq!(store.profile(&profile.id).unwrap(), saved);
    }
    assert!(store
        .update_profile_options(
            &profile.id,
            None,
            None,
            None,
            Some(TextTranslationName {
                route: TextTranslation::OpenAICompatible,
                name: "😀".repeat(65),
            }),
            None,
            None,
            None,
        )
        .is_err());
    assert_eq!(store.profile(&profile.id).unwrap(), saved);
}

#[cfg(target_os = "macos")]
#[test]
fn settings_snapshots_and_profile_switches_never_authorize_keys() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let first = ServiceProfile::alibaba_default();
    let second = openai_profile(&store, "Second provider");
    let third = store
        .create_profile(ProviderKind::GoogleGeminiLive, "Third provider")
        .unwrap();
    for profile in [&first, &second, &third] {
        fake.put(
            PROFILE_KEYCHAIN_SERVICE,
            &credential_account(profile),
            "synthetic-profile-key",
        );
    }
    for profile in [&first, &second, &third, &first, &third, &second] {
        store.select_profile(&profile.id).unwrap();
        for _ in 0..2 {
            let snapshot =
                crate::commands::SettingsSnapshotPayload::try_from_store(&store).unwrap();
            assert_eq!(snapshot.active_profile_id, profile.id);
            assert!(snapshot
                .profiles
                .iter()
                .all(|p| p.credential_state == CredentialState::Present));
        }
        assert!(fake.state.lock().unwrap().loads.is_empty());
    }
    // Actual listening configuration still validates each selected key once.
    for profile in [&first, &second, &third] {
        store.select_profile(&profile.id).unwrap();
        for _ in 0..2 {
            assert!(store.configuration().is_ok());
        }
        assert_eq!(
            fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(profile)),
            1
        );
    }
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT),
        0
    );
}

#[test]
fn custom_speech_retains_only_its_unchanged_endpoint_key_and_never_borrows_another_profile() {
    for provider in [
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store.create_profile(provider, "Custom speech").unwrap();
        let other = store.create_profile(provider, "Other speech").unwrap();
        let first = custom_speech_request(
            "wss://first.example/recognition",
            "first-model",
            "synthetic-first",
        );
        store.save_credentials(&profile.id, &first).unwrap();
        let account = credential_account(&profile);
        let before = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
        fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        assert!(store
            .save_credentials(&other.id, &custom_speech_request("", "", ""))
            .is_err());
        assert!(store
            .save_credentials(
                &profile.id,
                &custom_speech_request("wss://second.example/recognition", "second-model", "")
            )
            .is_err());
        assert!(store
            .save_credentials(
                &profile.id,
                &custom_speech_request(
                    "wss://second.example/recognition",
                    "",
                    "synthetic-new-service"
                )
            )
            .is_err());
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
            Some(before.as_str())
        );
        store
            .save_credentials(
                &profile.id,
                &custom_speech_request(" wss://first.example/recognition ", "updated-model", ""),
            )
            .unwrap();
        let credentials = store.credentials_for_profile(&profile).unwrap().unwrap();
        assert!(
            matches!(&credentials, ProviderCredentials::CustomSpeech { endpoint, model, api_key }
                if endpoint == "wss://first.example/recognition" && model == "updated-model" && api_key == "synthetic-first")
        );
        store
            .save_credentials(
                &profile.id,
                &custom_speech_request(
                    "wss://second.example/recognition",
                    "second-model",
                    "synthetic-second",
                ),
            )
            .unwrap();
        assert!(
            matches!(store.credentials_for_profile(&profile).unwrap().unwrap(), ProviderCredentials::CustomSpeech { endpoint, model, api_key }
                if endpoint == "wss://second.example/recognition" && model == "second-model" && api_key == "synthetic-second")
        );
    }
}

#[test]
fn advanced_translation_survives_restart_without_secret_metadata() {
    let fake = FakeSecretStore::default();
    let directory = std::env::temp_dir().join(format!(
        "mimi-text-translation-test-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let store = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Ali")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &translation_request(
                TextTranslation::DeepLX,
                "synthetic-asr",
                "http://localhost:1188",
                "",
            ),
        )
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    drop(store);
    let reloaded = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
    assert_eq!(
        reloaded.configuration().unwrap().provider,
        ProviderKind::DeepLX
    );
    assert_eq!(reloaded.active_profile().unwrap().id, profile.id);
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile))
            .as_deref(),
        Some("synthetic-asr")
    );
    let metadata = std::fs::read_to_string(directory.join(PROFILE_CATALOG_FILE)).unwrap();
    for secret in ["synthetic-asr", "localhost", "endpoint", "token"] {
        assert!(!metadata.contains(secret));
    }
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn failed_advanced_catalog_write_restores_key_destination_and_selection() {
    let fake = FakeSecretStore::default();
    let store = SettingsStore::in_memory(Box::new(fake.clone()), false);
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Ali")
        .unwrap();
    store.save_api_key(&profile.id, "synthetic-old").unwrap();
    store
        .save_credentials(
            &profile.id,
            &translation_request(
                TextTranslation::DeepLX,
                "",
                "https://example.com/old",
                "old-token",
            ),
        )
        .unwrap();
    let previous_key = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
    let previous_destination = fake.value(
        PROFILE_KEYCHAIN_SERVICE,
        &SettingsStore::destination_account(&profile),
    );
    let directory = std::env::temp_dir().join(format!(
        "mimi-text-write-test-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let mut store = store;
    // A directory cannot be replaced by the atomic catalog file write.
    store.catalog_path = directory.clone();
    assert!(store
        .save_credentials(
            &profile.id,
            &translation_request(
                TextTranslation::DeepLX,
                "synthetic-new",
                "https://example.com/new",
                "new-token"
            )
        )
        .is_err());
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
        previous_key
    );
    assert_eq!(
        fake.value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        ),
        previous_destination
    );
    assert_eq!(
        store.profile(&profile.id).unwrap().text_translation(),
        TextTranslation::DeepLX
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn failed_destination_delete_preserves_profile_and_key() {
    let fake = FakeSecretStore::default();
    let store = SettingsStore::in_memory(Box::new(fake.clone()), false);
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Ali")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &translation_request(
                TextTranslation::DeepLX,
                "synthetic-asr",
                "https://example.com",
                "",
            ),
        )
        .unwrap();
    fake.make_delete_unavailable(
        PROFILE_KEYCHAIN_SERVICE,
        &SettingsStore::destination_account(&profile),
    );
    assert!(store.delete_profile(&profile.id).is_err());
    assert!(store.profile(&profile.id).is_ok());
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile))
            .as_deref(),
        Some("synthetic-asr")
    );
    assert!(fake
        .value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        )
        .is_some());
}

#[test]
fn deeplx_values_use_profile_secure_storage_and_never_snapshot_json() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::DeepLX, "Third-party translation")
        .unwrap();
    let credentials = ProviderCredentials::DeepLX {
        asr_api_key: "synthetic-asr".into(),
        endpoint: "https://example.com/translate".into(),
        token: "synthetic-token".into(),
    };
    store.save_credentials(&profile.id, &credentials).unwrap();
    store.select_profile(&profile.id).unwrap();
    assert_eq!(store.configuration().unwrap().credentials, credentials);
    assert_eq!(store.credential_state(&profile), CredentialState::Present);
    let snapshot =
        serde_json::to_string(&(store.profile_catalog().unwrap(), store.preferences())).unwrap();
    for value in ["synthetic-asr", "synthetic-token", "https://example.com"] {
        assert!(!snapshot.contains(value));
    }
    store.delete_api_key(&profile.id).unwrap();
    assert_eq!(store.credential_state(&profile), CredentialState::Missing);
}

#[test]
fn credentials_are_isolated_by_profile_and_provider() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let first = openai_profile(&store, "Work");
    let second = openai_profile(&store, "Personal");

    store.save_api_key(&first.id, "sk-first").unwrap();
    store.save_api_key(&second.id, "sk-second").unwrap();

    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&first))
            .as_deref(),
        Some("sk-first")
    );
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&second))
            .as_deref(),
        Some("sk-second")
    );
    assert_ne!(credential_account(&first), credential_account(&second));
}

#[test]
fn creating_a_profile_never_copies_the_active_secret() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    store
        .save_api_key(DEFAULT_ALIBABA_PROFILE_ID, "sk-alibaba")
        .unwrap();
    let profile = openai_profile(&store, "OpenAI");

    assert_eq!(store.credential_state(&profile), CredentialState::Missing);
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
        None
    );
}

#[test]
fn diagnostic_does_not_retry_another_profiles_failed_slot() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let other = openai_profile(&store, "Synthetic other profile");
    let account = credential_account(&other);
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
    assert_eq!(store.credential_state(&other), CredentialState::Unavailable);
    store.credential_diagnostic(&ServiceProfile::alibaba_default());
    assert_eq!(store.credential_state(&other), CredentialState::Unavailable);
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
}

#[test]
fn profile_crud_preserves_catalog_invariants() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let created = openai_profile(&store, "  Work  ");
    assert_eq!(created.name, "Work");
    store.update_profile(&created.id, "Production").unwrap();
    store.select_profile(&created.id).unwrap();
    assert_eq!(store.active_profile().unwrap().id, created.id);

    store.delete_profile(DEFAULT_ALIBABA_PROFILE_ID).unwrap();
    assert_eq!(store.profile_catalog().unwrap().1.len(), 1);
    assert_eq!(store.delete_profile(&created.id).unwrap_err(), LAST_PROFILE);
}

#[test]
fn deleting_a_profile_without_a_saved_credential_is_idempotent() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = openai_profile(&store, "No credential");

    store.delete_profile(&profile.id).unwrap();

    assert!(store
        .profile_catalog()
        .unwrap()
        .1
        .iter()
        .all(|candidate| candidate.id != profile.id));
}

#[test]
fn profile_count_is_bounded() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    for index in 1..MAXIMUM_PROFILE_COUNT {
        store
            .create_profile(ProviderKind::OpenAIRealtime, &format!("Profile {index}"))
            .unwrap();
    }
    assert!(store
        .create_profile(ProviderKind::OpenAIRealtime, "One too many")
        .is_err());
    assert_eq!(
        store.profile_catalog().unwrap().1.len(),
        MAXIMUM_PROFILE_COUNT
    );
}

#[test]
fn failed_credential_delete_rolls_profile_metadata_back() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = openai_profile(&store, "Work");
    store.save_api_key(&profile.id, "sk-work").unwrap();
    let account = credential_account(&profile);
    fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);

    assert_eq!(
        store.delete_profile(&profile.id).unwrap_err(),
        CREDENTIAL_STORE_UNAVAILABLE
    );
    assert!(store
        .profile_catalog()
        .unwrap()
        .1
        .iter()
        .any(|candidate| candidate.id == profile.id));
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
        Some("sk-work")
    );
}

#[test]
fn failed_catalog_commit_does_not_delete_a_profile_credential() {
    let fake = FakeSecretStore::default();
    let directory = std::env::temp_dir().join(format!(
        "mimi-profile-commit-failure-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let store = SettingsStore::at_path(directory.clone(), Box::new(fake.clone()));
    let profile = store
        .create_profile(ProviderKind::OpenAIRealtime, "Work")
        .unwrap();
    store.save_api_key(&profile.id, "sk-work").unwrap();
    let account = credential_account(&profile);

    let catalog_path = directory.join(PROFILE_CATALOG_FILE);
    std::fs::remove_file(&catalog_path).unwrap();
    std::fs::create_dir(&catalog_path).unwrap();

    assert_eq!(
        store.delete_profile(&profile.id).unwrap_err(),
        PROFILE_CATALOG_UNAVAILABLE
    );
    assert!(store
        .profile_catalog()
        .unwrap()
        .1
        .iter()
        .any(|candidate| candidate.id == profile.id));
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
        Some("sk-work")
    );

    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn catalog_is_atomic_json_and_contains_no_secret() {
    let fake = FakeSecretStore::default();
    let directory = std::env::temp_dir().join(format!(
        "mimi-profile-test-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let store = SettingsStore::at_path(directory.clone(), Box::new(fake));
    let profile = store
        .create_profile(ProviderKind::OpenAIRealtime, "Work")
        .unwrap();
    store
        .save_api_key(&profile.id, "super-secret-value")
        .unwrap();

    let catalog_path = directory.join(PROFILE_CATALOG_FILE);
    let text = std::fs::read_to_string(&catalog_path).unwrap();
    let catalog: ProfileCatalog = serde_json::from_str(&text).unwrap();
    assert_eq!(catalog.schema_version, PROFILE_CATALOG_SCHEMA_VERSION);
    assert!(!text.contains("super-secret-value"));
    assert!(!text.contains("apiKey"));
    assert!(std::fs::read_dir(&directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));

    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn invalid_catalog_is_preserved_and_blocks_mutation() {
    let fake = FakeSecretStore::default();
    let directory = std::env::temp_dir().join(format!(
        "mimi-profile-invalid-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(PROFILE_CATALOG_FILE);
    std::fs::write(&path, r#"{"schemaVersion":99,"profiles":[]}"#).unwrap();
    let store = SettingsStore::at_path(directory.clone(), Box::new(fake));

    assert_eq!(
        store
            .create_profile(ProviderKind::OpenAIRealtime, "Work")
            .unwrap_err(),
        PROFILE_CATALOG_UNAVAILABLE
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        r#"{"schemaVersion":99,"profiles":[]}"#
    );
    assert_eq!(
        crate::commands::SettingsSnapshotPayload::try_from_store(&store).unwrap_err(),
        PROFILE_CATALOG_UNAVAILABLE
    );

    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn speech_name_patches_persist_independently_without_credential_access() {
    for provider in [
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store
            .create_profile(provider, "Original configuration")
            .unwrap();
        store
            .update_profile_options(
                &profile.id,
                None,
                Some(ProxyConfig::default()),
                None,
                Some(TextTranslationName {
                    route: TextTranslation::OpenAICompatible,
                    name: "Independent translator".into(),
                }),
                None,
                None,
                None,
            )
            .unwrap();
        store
            .update_profile(&profile.id, "Renamed configuration")
            .unwrap();
        let named = store
            .update_profile_options(
                &profile.id,
                None,
                None,
                None,
                None,
                Some("  Whisper · 本地  "),
                None,
                None,
            )
            .unwrap();
        assert_eq!(named.name, "Renamed configuration");
        assert_eq!(
            named.speech_recognition_name.as_deref(),
            Some("Whisper · 本地")
        );
        assert_eq!(named.speech_network_proxy, Some(ProxyConfig::default()));
        assert_eq!(
            named.text_translation_names[&TextTranslation::OpenAICompatible],
            "Independent translator"
        );
        let other = store.create_profile(provider, "Other").unwrap();
        assert!(other.speech_recognition_name.is_none());
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(reloaded.profile(&profile.id).unwrap(), named);
        let cleared = reloaded
            .update_profile_options(&profile.id, None, None, None, None, Some("  "), None, None)
            .unwrap();
        assert!(cleared.speech_recognition_name.is_none());
        assert_eq!(cleared.name, named.name);
        assert_eq!(cleared.text_translation_names, named.text_translation_names);
        assert_eq!(cleared.speech_network_proxy, named.speech_network_proxy);
        let reopened = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(reopened.profile(&profile.id).unwrap(), cleared);
        let secrets = fake.state.lock().unwrap();
        assert!(secrets.loads.is_empty());
        assert!(secrets.values.is_empty());
    }
}

#[test]
fn speech_name_updates_validate_and_roll_back_failed_writes() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let mut store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let built_in = store.active_profile().unwrap();
    assert!(store
        .update_profile_options(
            &built_in.id,
            None,
            None,
            None,
            None,
            Some("Unsupported"),
            None,
            None
        )
        .is_err());
    assert_eq!(store.active_profile().unwrap(), built_in);
    let profile = store
        .create_profile(ProviderKind::CustomOpenAIASR, "Custom")
        .unwrap();
    for invalid in [
        "😀".repeat(65),
        "line\nbreak".into(),
        "control\u{0000}".into(),
    ] {
        assert!(store
            .update_profile_options(
                &profile.id,
                Some("Unpersisted configuration"),
                None,
                None,
                None,
                Some(&invalid),
                None,
                None,
            )
            .is_err());
        assert_eq!(store.profile(&profile.id).unwrap(), profile);
    }
    store.catalog_path = directory.path().join("blocked-catalog");
    std::fs::create_dir(&store.catalog_path).unwrap();
    assert!(store
        .update_profile_options(
            &profile.id,
            None,
            None,
            None,
            None,
            Some("Unpersisted recognition"),
            None,
            None,
        )
        .is_err());
    assert_eq!(store.profile(&profile.id).unwrap(), profile);
    let reopened = SettingsStore::at_path(directory.path().into(), Box::new(fake));
    assert_eq!(reopened.profile(&profile.id).unwrap(), profile);
}

#[test]
fn translation_names_persist_independently_without_credential_access() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let profile = store.active_profile().unwrap();
    for (route, name) in [
        (TextTranslation::OpenAICompatible, "  Work translator  "),
        (TextTranslation::DeepLX, "Local translator"),
    ] {
        store
            .update_profile_options(
                &profile.id,
                None,
                None,
                None,
                Some(TextTranslationName {
                    route,
                    name: name.into(),
                }),
                None,
                None,
                None,
            )
            .unwrap();
    }
    store
        .update_profile(&profile.id, "Renamed profile")
        .unwrap();
    let other = store
        .create_profile(ProviderKind::AlibabaCloud, "Other")
        .unwrap();
    assert!(other.text_translation_names.is_empty());
    let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let restored = reloaded.profile(&profile.id).unwrap();
    assert_eq!(restored.name, "Renamed profile");
    assert_eq!(
        restored.text_translation_names[&TextTranslation::OpenAICompatible],
        "Work translator"
    );
    assert_eq!(
        restored.text_translation_names[&TextTranslation::DeepLX],
        "Local translator"
    );
    assert_eq!(restored.text_translation(), TextTranslation::FollowService);
    reloaded
        .update_profile_options(
            &profile.id,
            None,
            None,
            None,
            Some(TextTranslationName {
                route: TextTranslation::OpenAICompatible,
                name: "  ".into(),
            }),
            None,
            None,
            None,
        )
        .unwrap();
    let reopened = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let names = reopened
        .profile(&profile.id)
        .unwrap()
        .text_translation_names;
    assert_eq!(names.len(), 1);
    assert_eq!(names[&TextTranslation::DeepLX], "Local translator");
    let secrets = fake.state.lock().unwrap();
    assert!(secrets.loads.is_empty());
    assert!(secrets.values.is_empty());
}

#[test]
fn translation_name_updates_reject_invalid_values_and_roll_back_failed_writes() {
    let directory = tempfile::tempdir().unwrap();
    let fake = FakeSecretStore::default();
    let mut store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
    let profile = store.active_profile().unwrap();
    for (route, name) in [
        (TextTranslation::OpenAICompatible, "x".repeat(65)),
        (TextTranslation::FollowService, "Unsupported".into()),
    ] {
        assert!(store
            .update_profile_options(
                &profile.id,
                Some("Unpersisted profile name"),
                None,
                None,
                Some(TextTranslationName { route, name }),
                None,
                None,
                None,
            )
            .is_err());
        assert_eq!(store.active_profile().unwrap(), profile);
    }
    store.catalog_path = directory.path().join("blocked-catalog");
    std::fs::create_dir(&store.catalog_path).unwrap();
    assert!(store
        .update_profile_options(
            &profile.id,
            None,
            None,
            None,
            Some(TextTranslationName {
                route: TextTranslation::OpenAICompatible,
                name: "Unpersisted translator".into(),
            }),
            None,
            None,
            None,
        )
        .is_err());
    assert_eq!(store.active_profile().unwrap(), profile);
    let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake));
    assert_eq!(reloaded.active_profile().unwrap(), profile);
}

#[test]
fn translation_names_survive_route_saves_without_changing_saved_secrets() {
    for provider in [ProviderKind::AlibabaCloud, ProviderKind::CustomOpenAIASR] {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store.create_profile(provider, "Named translator").unwrap();
        let speech = if provider.is_custom_speech() {
            custom_speech_request("wss://speech.example/realtime", "speech-model", "synthetic")
        } else {
            ProviderCredentials::api_key("synthetic")
        };
        store.save_credentials(&profile.id, &speech).unwrap();
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "",
                    "https://translation.example/v1",
                    "synthetic-text-key",
                    "text-model",
                ),
            )
            .unwrap();
        let before = fake.state.lock().unwrap().values.clone();
        fake.state.lock().unwrap().loads.clear();
        let named = store
            .update_profile_options(
                &profile.id,
                None,
                None,
                None,
                Some(TextTranslationName {
                    route: TextTranslation::OpenAICompatible,
                    name: "Work translator".into(),
                }),
                provider.is_custom_speech().then_some("My recognition"),
                None,
                None,
            )
            .unwrap();
        assert_eq!(fake.state.lock().unwrap().values, before);
        assert!(fake.state.lock().unwrap().loads.is_empty());
        for route in [
            TextTranslation::FollowService,
            TextTranslation::OpenAICompatible,
        ] {
            store
                .save_credentials(&profile.id, &translation_request(route, "", "", ""))
                .unwrap();
            let switched = store.profile(&profile.id).unwrap();
            assert_eq!(switched.text_translation(), route);
            assert_eq!(
                switched.text_translation_names,
                named.text_translation_names
            );
            assert_eq!(
                switched.speech_recognition_name,
                named.speech_recognition_name
            );
        }
        assert_eq!(fake.state.lock().unwrap().values, before);
    }
}
