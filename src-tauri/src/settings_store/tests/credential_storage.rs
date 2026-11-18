//! Credential storage behavior regressions.

use super::*;

#[cfg(target_os = "macos")]
#[test]
fn selecting_custom_pipeline_does_not_authorize_either_credential_slot() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::CustomDashScopeASR, "Separate pipeline")
        .unwrap();
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
    store
        .save_credentials(
            &profile.id,
            &openai_compatible_request(
                "",
                "https://text.example/v1",
                "synthetic-text",
                "synthetic-model",
            ),
        )
        .unwrap();
    store.secret_cache.lock().unwrap().clear();
    fake.state.lock().unwrap().loads.clear();
    let snapshot = crate::commands::SettingsSnapshotPayload::try_from_store(&store).unwrap();
    let inactive = snapshot
        .profiles
        .iter()
        .find(|p| p.id == profile.id)
        .unwrap();
    assert_eq!(
        inactive.speech_credential_state,
        Some(CredentialState::Present)
    );
    assert_eq!(
        inactive.text_credential_state,
        Some(CredentialState::Present)
    );
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
        0
    );
    assert_eq!(
        fake.load_count(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        ),
        0
    );
    store.select_profile(&profile.id).unwrap();
    crate::commands::SettingsSnapshotPayload::try_from_store(&store).unwrap();
    assert!(fake.state.lock().unwrap().loads.is_empty());
    let configured = store.profile(&profile.id).unwrap();
    for _ in 0..2 {
        assert!(store.configuration_for_profile_probe(&configured).is_ok());
    }
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
        1
    );
    assert_eq!(
        fake.load_count(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        ),
        1
    );
}

#[cfg(target_os = "macos")]
#[test]
fn snapshot_presence_respects_cached_access_failure() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let other = openai_profile(&store, "Access failure");
    let account = credential_account(&other);
    fake.put(PROFILE_KEYCHAIN_SERVICE, &account, "synthetic-key");
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
    assert_eq!(store.credential_state(&other), CredentialState::Unavailable);
    fake.state
        .lock()
        .unwrap()
        .unavailable
        .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &account));
    assert_eq!(
        store.credential_state_for_snapshot(&other),
        CredentialState::Unavailable
    );
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
}

#[test]
fn custom_speech_and_each_text_route_survive_restart_in_separate_os_items() {
    for provider in [
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store.create_profile(provider, "Custom speech").unwrap();
        store
            .save_credentials(
                &profile.id,
                &custom_speech_request(
                    "wss://speech.example/recognition",
                    "speech-model",
                    "synthetic-speech-key",
                ),
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        let account = credential_account(&profile);
        let source_before = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
        fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        for request in [
            translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
            translation_request(
                TextTranslation::DeepLX,
                "",
                "https://text.example/translate",
                "synthetic-deeplx",
            ),
            openai_compatible_request(
                "",
                "https://text.example/v1",
                "synthetic-chat-key",
                "chat-model",
            ),
        ] {
            store.save_credentials(&profile.id, &request).unwrap();
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
                Some(source_before.as_str())
            );
        }
        let destination_before = fake
            .value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile),
            )
            .unwrap();
        for private in [
            "speech.example",
            "speech-model",
            "synthetic-speech-key",
            "text.example",
            "synthetic-deepl",
            "synthetic-chat-key",
            "chat-model",
        ] {
            assert!(!destination_before.contains("synthetic-speech-key"));
            for path in [&store.prefs_path, &store.catalog_path] {
                assert!(!std::fs::read_to_string(path).unwrap().contains(private));
            }
        }
        drop(store);
        fake.state.lock().unwrap().loads.clear();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        for route in [
            TextTranslation::DeepL,
            TextTranslation::DeepLX,
            TextTranslation::OpenAICompatible,
            TextTranslation::FollowService,
        ] {
            store
                .save_credentials(&profile.id, &translation_request(route, "", "", ""))
                .unwrap();
            if route != TextTranslation::FollowService {
                store
                    .save_preferences_for_active_profile(|prefs| {
                        prefs.target_language = TargetLanguage::English
                    })
                    .unwrap();
            }
            let config = store.configuration().unwrap();
            assert_eq!(
                config.credentials.direct_api_key(),
                Some("synthetic-speech-key")
            );
            assert_eq!(config.provider, provider);
            assert_eq!(
                config.capabilities().input_sample_rate_hz,
                provider.capabilities().input_sample_rate_hz
            );
            match route {
                TextTranslation::DeepL => assert!(
                    matches!(config.text_credentials, Some(TextTranslationCredentials::DeepL { api_key }) if api_key == "synthetic-deepl:fx")
                ),
                TextTranslation::DeepLX => assert!(
                    matches!(config.text_credentials, Some(TextTranslationCredentials::DeepLX { token, .. }) if token == "synthetic-deeplx")
                ),
                TextTranslation::OpenAICompatible => assert!(
                    matches!(config.text_credentials, Some(TextTranslationCredentials::OpenAICompatible { api_key, model, .. }) if api_key == "synthetic-chat-key" && model == "chat-model")
                ),
                TextTranslation::XAI | TextTranslation::ChatMock | TextTranslation::Apple => {
                    unreachable!("covered by independent local and ChatMock persistence tests")
                }
                TextTranslation::FollowService => {
                    assert_eq!(config.target_language, TargetLanguage::Original);
                    assert_eq!(config.text_credentials, None);
                }
            }
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
                Some(source_before.as_str())
            );
            assert_eq!(
                fake.value(
                    PROFILE_KEYCHAIN_SERVICE,
                    &SettingsStore::destination_account(&profile)
                )
                .as_deref(),
                Some(destination_before.as_str())
            );
        }
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
    }
}

#[test]
fn custom_text_updates_preserve_speech_and_snapshots_do_not_authorize_saved_keys() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::CustomDashScopeASR, "Custom speech")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &custom_speech_request(
                "wss://speech.example/recognition",
                "speech-model",
                "synthetic-speech",
            ),
        )
        .unwrap();
    let account = credential_account(&profile);
    let before = fake.value(PROFILE_KEYCHAIN_SERVICE, &account);
    store.secret_cache.lock().unwrap().clear();
    fake.state.lock().unwrap().loads.clear();
    store
        .save_credentials(
            &profile.id,
            &openai_compatible_request(
                "",
                "https://text.example/v1",
                "synthetic-text",
                "text-model",
            ),
        )
        .unwrap();
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 0);
    assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), before);
    let request = translation_request(
        TextTranslation::DeepL,
        "synthetic-forbidden-speech-update",
        "",
        "synthetic-text",
    );
    assert_eq!(
        store.save_credentials(&profile.id, &request),
        Err("custom_speech_text_update_contains_speech_key".into())
    );
    assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), before);
    store.secret_cache.lock().unwrap().clear();
    fake.state.lock().unwrap().loads.clear();
    store.select_profile(&profile.id).unwrap();
    let snapshot = crate::commands::SettingsSnapshotPayload::try_from_store(&store).unwrap();
    let payload = snapshot
        .profiles
        .iter()
        .find(|entry| entry.id == profile.id)
        .unwrap();
    assert_eq!(
        payload.speech_credential_state,
        Some(CredentialState::Present)
    );
    assert_eq!(
        payload.text_credential_state,
        Some(CredentialState::Present)
    );
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account),
        usize::from(!cfg!(target_os = "macos"))
    );
    assert_eq!(
        fake.load_count(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        ),
        usize::from(!cfg!(target_os = "macos"))
    );
    let configured = store.profile(&profile.id).unwrap();
    for _ in 0..2 {
        assert!(store.configuration_for_profile_probe(&configured).is_ok());
    }
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 1);
    assert_eq!(
        fake.load_count(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        ),
        1
    );
    let json = serde_json::to_string(&snapshot).unwrap();
    for private in [
        "speech.example",
        "text.example",
        "speech-model",
        "text-model",
        "synthetic-speech",
        "synthetic-text",
    ] {
        assert!(!json.contains(private));
    }
    let alibaba = snapshot
        .profiles
        .iter()
        .find(|entry| entry.provider == ProviderKind::AlibabaCloud)
        .unwrap();
    assert_eq!(alibaba.speech_credential_state, None);
    assert_eq!(alibaba.text_credential_state, None);
}

#[test]
fn custom_original_recognition_does_not_read_a_missing_text_destination() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::CustomOpenAIASR, "Custom speech")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &custom_speech_request(
                "wss://speech.example/realtime",
                "speech-model",
                "synthetic-speech",
            ),
        )
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
        )
        .unwrap();
    let before_probe = store.preferences();
    let configured_profile = store.profile(&profile.id).unwrap();
    let probe = store
        .configuration_for_profile_probe(&configured_profile)
        .unwrap();
    assert_eq!(probe.target_language, TargetLanguage::English);
    assert!(
        matches!(probe.text_credentials, Some(TextTranslationCredentials::DeepL { api_key }) if api_key == "synthetic-deepl:fx")
    );
    assert_eq!(store.preferences(), before_probe);
    assert_eq!(store.active_profile().unwrap().id, profile.id);
    let destination = SettingsStore::destination_account(&profile);
    fake.state
        .lock()
        .unwrap()
        .values
        .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &destination));
    store.secret_cache.lock().unwrap().clear();
    fake.state.lock().unwrap().loads.clear();
    assert_eq!(store.configuration().unwrap().text_credentials, None);
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &destination), 0);
    let profile = store.profile(&profile.id).unwrap();
    assert_eq!(
        store.custom_credential_states(&profile),
        Some((CredentialState::Present, CredentialState::Missing))
    );
    store
        .save_preferences_for_active_profile(|prefs| {
            prefs.target_language = TargetLanguage::English
        })
        .unwrap();
    assert_eq!(
        store.configuration().unwrap_err(),
        crate::core::credentials::ProviderCredentialsError::MissingTextTranslation.to_string()
    );
}

#[test]
fn custom_text_save_failures_roll_back_route_and_destination_without_touching_speech() {
    for fail_destination in [false, true] {
        let fake = FakeSecretStore::default();
        let mut store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::CustomDashScopeASR, "Custom speech")
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &custom_speech_request(
                    "wss://speech.example/recognition",
                    "speech-model",
                    "synthetic-speech",
                ),
            )
            .unwrap();
        store.select_profile(&profile.id).unwrap();
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic-old:fx"),
            )
            .unwrap();
        store
            .save_preferences_for_active_profile(|prefs| {
                prefs.target_language = TargetLanguage::English
            })
            .unwrap();
        let account = credential_account(&profile);
        let destination = SettingsStore::destination_account(&profile);
        let before_speech = fake.value(PROFILE_KEYCHAIN_SERVICE, &account);
        let before_text = fake.value(PROFILE_KEYCHAIN_SERVICE, &destination);
        let before_preferences = store.preferences();
        let blocked = tempfile::tempdir().unwrap();
        if fail_destination {
            fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &destination);
        } else {
            store.catalog_path = blocked.path().into();
        }
        fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        assert!(store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "",
                    "https://new-text.example/v1",
                    "synthetic-new",
                    "new-text-model"
                )
            )
            .is_err());
        assert_eq!(
            store.profile(&profile.id).unwrap().text_translation(),
            TextTranslation::DeepL
        );
        assert_eq!(store.preferences(), before_preferences);
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &account),
            before_speech
        );
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &destination),
            before_text
        );
    }
}

#[test]
fn custom_credential_deletion_rolls_back_text_when_speech_delete_fails_then_removes_both() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::CustomOpenAIASR, "Custom speech")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &custom_speech_request(
                "wss://speech.example/realtime",
                "speech-model",
                "synthetic-speech",
            ),
        )
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx"),
        )
        .unwrap();
    let account = credential_account(&profile);
    let destination = SettingsStore::destination_account(&profile);
    let before_text = fake.value(PROFILE_KEYCHAIN_SERVICE, &destination);
    fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
    assert!(store.delete_api_key(&profile.id).is_err());
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &destination),
        before_text
    );
    assert!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account).is_some());
    fake.state.lock().unwrap().unavailable_deletes.clear();
    store.delete_api_key(&profile.id).unwrap();
    assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), None);
    assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &destination), None);
}

#[test]
fn custom_speech_failed_readback_restores_the_existing_item_in_place() {
    #[derive(Clone)]
    struct FailOnceReadback {
        inner: FakeSecretStore,
        rejected_value: Arc<Mutex<Option<String>>>,
    }
    impl SecretStore for FailOnceReadback {
        fn load(&self, service: &str, account: &str) -> Result<Option<String>, SecretStoreError> {
            let value = self.inner.load(service, account)?;
            let mut rejected = self.rejected_value.lock().unwrap();
            if rejected.is_some() && rejected.as_deref() == value.as_deref() {
                *rejected = None;
                return Err(SecretStoreError::Unavailable);
            }
            Ok(value)
        }
        fn save(&self, service: &str, account: &str, value: &str) -> Result<(), SecretStoreError> {
            self.inner.save(service, account, value)
        }
        fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreError> {
            self.inner.delete(service, account)
        }
    }
    let fake = FakeSecretStore::default();
    let rejected_value = Arc::new(Mutex::new(None));
    let store = SettingsStore::in_memory(
        Box::new(FailOnceReadback {
            inner: fake.clone(),
            rejected_value: Arc::clone(&rejected_value),
        }),
        false,
    );
    let profile = store
        .create_profile(ProviderKind::CustomDashScopeASR, "Custom speech")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &custom_speech_request(
                "wss://speech.example/recognition",
                "old-model",
                "synthetic-old-key",
            ),
        )
        .unwrap();
    let account = credential_account(&profile);
    let previous = fake.value(PROFILE_KEYCHAIN_SERVICE, &account);
    fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
    let replacement = custom_speech_request(
        "wss://speech.example/recognition",
        "new-model",
        "synthetic-new-key",
    );
    *rejected_value.lock().unwrap() =
        Some(replacement.encode_for_keychain(profile.provider).unwrap());
    assert_eq!(
        store.save_credentials(&profile.id, &replacement),
        Err(CREDENTIAL_STORE_UNAVAILABLE.into())
    );
    assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), previous);
    assert_eq!(
        store
            .credentials_for_profile(&profile)
            .unwrap()
            .unwrap()
            .direct_api_key(),
        Some("synthetic-old-key")
    );

    // A repair must restore even an undecodable previous value when the
    // replacement's durable read-back fails, without deleting the slot.
    let malformed = "malformed synthetic speech value";
    fake.put(PROFILE_KEYCHAIN_SERVICE, &account, malformed);
    store.secret_cache.lock().unwrap().clear();
    *rejected_value.lock().unwrap() =
        Some(replacement.encode_for_keychain(profile.provider).unwrap());
    assert_eq!(
        store.save_credentials(&profile.id, &replacement),
        Err(CREDENTIAL_STORE_UNAVAILABLE.into())
    );
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
        Some(malformed)
    );
    assert_eq!(
        store.credentials_for_profile(&profile),
        Err(crate::core::credentials::ProviderCredentialsError::InvalidStoredValue.to_string())
    );
    store.save_credentials(&profile.id, &replacement).unwrap();
    assert_eq!(
        store.credentials_for_profile(&profile).unwrap(),
        Some(replacement)
    );
}

#[test]
fn credential_failures_preserve_categories_and_recover_without_plaintext_storage() {
    for (failure, diagnostic, public_error) in [
        (
            SecretStoreError::Unavailable,
            "unavailable",
            CREDENTIAL_STORE_UNAVAILABLE,
        ),
        (
            SecretStoreError::ServiceUnavailable,
            "serviceUnavailable",
            CREDENTIAL_SERVICE_UNAVAILABLE,
        ),
        (
            SecretStoreError::AccessDenied,
            "accessDenied",
            CREDENTIAL_STORE_ACCESS_DENIED,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::load_with_secret(
            directory.path().to_path_buf(),
            false,
            Box::new(fake.clone()),
            PROFILE_KEYCHAIN_SERVICE,
        );
        let profile = store
            .create_profile(ProviderKind::AlibabaCloud, "Recovery")
            .unwrap();
        let slot = cache_key(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
        fake.state
            .lock()
            .unwrap()
            .failures
            .insert(slot.clone(), failure);
        assert_eq!(
            store.credential_state(&profile),
            CredentialState::Unavailable
        );
        assert_eq!(store.credential_diagnostic(&profile), diagnostic);
        assert_eq!(store.configuration().unwrap_err().to_string(), public_error);
        assert_eq!(
            store
                .save_api_key(&profile.id, "synthetic-recovery-value")
                .unwrap_err(),
            public_error
        );
        assert_eq!(store.delete_api_key(&profile.id).unwrap_err(), public_error);
        assert_eq!(store.active_profile().unwrap().id, profile.id);
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
            None
        );

        fake.state.lock().unwrap().failures.remove(&slot);
        assert_eq!(store.credential_diagnostic(&profile), "missing");
        assert_eq!(store.credential_state(&profile), CredentialState::Missing);
        store
            .save_api_key(&profile.id, "synthetic-recovery-value")
            .unwrap();
        assert_eq!(store.credential_diagnostic(&profile), "present");
        for path in [store.prefs_path.clone(), store.catalog_path.clone()] {
            if let Ok(json) = std::fs::read_to_string(path) {
                assert!(!json.contains("synthetic-recovery-value"));
            }
        }
        store.delete_api_key(&profile.id).unwrap();
        assert_eq!(store.credential_diagnostic(&profile), "missing");
    }
}

#[test]
fn unavailable_credential_store_is_not_reported_as_missing() {
    let fake = FakeSecretStore::default();
    let default = ServiceProfile::alibaba_default();
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &credential_account(&default));
    let store = settings(&fake);

    assert_eq!(
        store.credential_state(&default),
        CredentialState::Unavailable
    );
    assert_eq!(
        store.load_api_key().unwrap_err(),
        CREDENTIAL_STORE_UNAVAILABLE
    );
    // All windows share the first failed read, avoiding repeated OS
    // authorization prompts for the same slot during one app launch.
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&default)),
        1
    );
}

#[test]
fn explicit_diagnostic_recovers_failed_storage_without_restart() {
    let fake = FakeSecretStore::default();
    let profile = ServiceProfile::alibaba_default();
    let account = credential_account(&profile);
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
    let store = settings(&fake);
    assert_eq!(store.credential_diagnostic(&profile), "unavailable");
    fake.state.lock().unwrap().unavailable.clear();
    assert_eq!(store.credential_diagnostic(&profile), "missing");
    store
        .save_api_key(&profile.id, "synthetic-test-value")
        .unwrap();
    assert_eq!(store.credential_diagnostic(&profile), "present");
}

#[test]
fn explicit_alibaba_save_retries_failed_reads_without_a_diagnostic() {
    for route in [TextTranslation::FollowService, TextTranslation::Apple] {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store.active_profile().unwrap();
        let account = credential_account(&profile);
        fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        let request = translation_request(route, "synthetic-key", "", "");
        assert_eq!(
            store.save_credentials(&profile.id, &request),
            Err(CREDENTIAL_STORE_UNAVAILABLE.into())
        );
        fake.state.lock().unwrap().unavailable.clear();
        store.save_credentials(&profile.id, &request).unwrap();
        assert_eq!(store.credential_state(&profile), CredentialState::Present);
        if route == TextTranslation::Apple {
            assert_eq!(
                fake.load_count(
                    PROFILE_KEYCHAIN_SERVICE,
                    &SettingsStore::destination_account(&profile),
                ),
                0
            );
        }
    }
}

#[test]
fn explicit_custom_save_retries_only_its_failed_stage() {
    for speech_save in [false, true] {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store
            .create_profile(ProviderKind::CustomOpenAIASR, "Synthetic custom")
            .unwrap();
        let speech = credential_account(&profile);
        let text = SettingsStore::destination_account(&profile);
        for account in [&speech, &text] {
            fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, account);
            assert!(store
                .load_secret(PROFILE_KEYCHAIN_SERVICE, account)
                .is_err());
        }
        let request = if speech_save {
            ProviderCredentials::CustomSpeech {
                endpoint: "wss://speech.example/v1".into(),
                model: "synthetic-model".into(),
                api_key: "synthetic-speech".into(),
            }
        } else {
            translation_request(TextTranslation::DeepL, "", "", "synthetic-text:fx")
        };
        assert!(store.save_credentials(&profile.id, &request).is_err());
        fake.state.lock().unwrap().unavailable.clear();
        fake.state.lock().unwrap().loads.clear();
        store.save_credentials(&profile.id, &request).unwrap();
        let (saved, untouched) = if speech_save {
            (&speech, &text)
        } else {
            (&text, &speech)
        };
        assert!(store
            .load_secret(PROFILE_KEYCHAIN_SERVICE, saved)
            .unwrap()
            .is_some());
        assert!(store
            .load_secret(PROFILE_KEYCHAIN_SERVICE, untouched)
            .is_err());
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, untouched), 0);
    }
}
