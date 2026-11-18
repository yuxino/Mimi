//! Independent translator destinations and credential isolation regressions.

use super::*;

#[test]
fn chatmock_and_generic_destinations_stay_independent_across_save_reveal_switch_and_restart() {
    use crate::core::services::configuration::TextTranslationProbeCredentials;
    for provider in [
        ProviderKind::AlibabaCloud,
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store
            .create_profile(provider, "Independent destinations")
            .unwrap();
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
        let speech_before = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
        let destination_account = SettingsStore::destination_account(&profile);
        // Identical endpoint makes route isolation independent of the address-change guard.
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "",
                    "http://127.0.0.1:8000/v1",
                    "synthetic-generic",
                    "generic-model",
                ),
            )
            .unwrap();
        let old_destination = fake
            .value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
            .unwrap();
        assert!(!old_destination.contains("chat_mock"));
        // Old generic settings cannot provide ChatMock's address, model, or key.
        assert!(store
            .save_credentials(&profile.id, &chatmock_request("", "", ""))
            .is_err());
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
                .as_deref(),
            Some(old_destination.as_str())
        );
        assert_eq!(
            store.profile(&profile.id).unwrap().text_translation(),
            TextTranslation::OpenAICompatible
        );
        store
            .save_credentials(
                &profile.id,
                &chatmock_request("http://127.0.0.1:8000/v1", "", "chat-model"),
            )
            .unwrap();
        let current = store.profile(&profile.id).unwrap();
        let probe = store.configuration_for_text_probe(&current).unwrap();
        assert!(
            matches!(probe.credentials, TextTranslationProbeCredentials::Independent(TextTranslationCredentials::ChatMock { api_key, model, .. }) if api_key.is_empty() && model == "chat-model")
        );
        assert_eq!(
            store.reveal_credential(
                &profile.id,
                CredentialRevealField::Token,
                Some(TextTranslation::ChatMock)
            ),
            Ok(None)
        );
        assert!(store
            .reveal_credential(
                &profile.id,
                CredentialRevealField::Token,
                Some(TextTranslation::OpenAICompatible)
            )
            .is_err());
        store
            .save_credentials(&profile.id, &chatmock_request("", "synthetic-chatmock", ""))
            .unwrap();
        let saved = fake
            .value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
            .unwrap();
        assert!(saved.contains("open_ai_compatible") && saved.contains("chat_mock"));
        for path in [&store.prefs_path, &store.catalog_path] {
            let public = std::fs::read_to_string(path).unwrap_or_default();
            for private in [
                "synthetic-generic",
                "synthetic-chatmock",
                "synthetic-speech",
                "127.0.0.1",
                "chat-model",
                "generic-model",
            ] {
                assert!(!public.contains(private));
            }
        }
        drop(store);
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        for route in [
            TextTranslation::OpenAICompatible,
            TextTranslation::ChatMock,
            TextTranslation::OpenAICompatible,
            TextTranslation::ChatMock,
        ] {
            store
                .save_credentials(&profile.id, &translation_request(route, "", "", ""))
                .unwrap();
            let current = store.profile(&profile.id).unwrap();
            let expected = if route == TextTranslation::ChatMock {
                "synthetic-chatmock"
            } else {
                "synthetic-generic"
            };
            assert_eq!(
                store
                    .reveal_credential(&profile.id, CredentialRevealField::Token, Some(route))
                    .unwrap()
                    .as_deref(),
                Some(expected)
            );
            let text = store
                .text_credentials_for_profile(&current)
                .unwrap()
                .unwrap();
            assert_eq!(text.translation(), route);
            let model = match text {
                TextTranslationCredentials::ChatMock { model, .. }
                | TextTranslationCredentials::OpenAICompatible { model, .. } => model,
                _ => unreachable!(),
            };
            assert_eq!(
                model,
                if route == TextTranslation::ChatMock {
                    "chat-model"
                } else {
                    "generic-model"
                }
            );
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
                speech_before
            );
        }
        let clear = ProviderCredentials::AlibabaTranslation {
            api_key: String::new(),
            text_translation: TextTranslation::ChatMock,
            endpoint: String::new(),
            token: String::new(),
            model: String::new(),
            clear_token: true,
        };
        store.save_credentials(&profile.id, &clear).unwrap();
        assert_eq!(
            store.reveal_credential(
                &profile.id,
                CredentialRevealField::Token,
                Some(TextTranslation::ChatMock)
            ),
            Ok(None)
        );
        store
            .save_credentials(&profile.id, &openai_compatible_request("", "", "", ""))
            .unwrap();
        assert_eq!(
            store.reveal_credential(
                &profile.id,
                CredentialRevealField::Token,
                Some(TextTranslation::OpenAICompatible)
            ),
            Ok(Some("synthetic-generic".into()))
        );
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
            speech_before
        );
    }
}

#[test]
fn openai_compatible_destination_survives_restart_and_route_switches_without_crossing_secrets() {
    for provider in [ProviderKind::AlibabaCloud, ProviderKind::DeepLX] {
        let directory = tempfile::tempdir().unwrap();
        let fake = FakeSecretStore::default();
        let store = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        let profile = store.create_profile(provider, "Speech").unwrap();
        if provider == ProviderKind::DeepLX {
            store
                .save_credentials(
                    &profile.id,
                    &ProviderCredentials::DeepLX {
                        asr_api_key: "synthetic-asr".into(),
                        endpoint: "https://example.com/translate".into(),
                        token: "synthetic-deeplx".into(),
                    },
                )
                .unwrap();
        } else {
            store.save_api_key(&profile.id, "synthetic-asr").unwrap();
            store
                .save_credentials(
                    &profile.id,
                    &translation_request(
                        TextTranslation::DeepLX,
                        "",
                        "https://example.com/translate",
                        "synthetic-deeplx",
                    ),
                )
                .unwrap();
        }
        store.select_profile(&profile.id).unwrap();
        let account = credential_account(&profile);
        let original_asr_item = fake.value(PROFILE_KEYCHAIN_SERVICE, &account).unwrap();
        fake.make_delete_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
        fake.make_delete_unavailable(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile),
        );
        store
            .save_credentials(
                &profile.id,
                &translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
            )
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "",
                    "https://third-party.example/v1",
                    "synthetic-openai",
                    "synthetic-model",
                ),
            )
            .unwrap();
        let destination_before = fake
            .value(
                PROFILE_KEYCHAIN_SERVICE,
                &SettingsStore::destination_account(&profile),
            )
            .unwrap();
        for private in [
            "synthetic-asr",
            "synthetic-deepl",
            "synthetic-openai",
            "synthetic-model",
            "third-party.example",
        ] {
            for path in [&store.prefs_path, &store.catalog_path] {
                assert!(!std::fs::read_to_string(path).unwrap().contains(private));
            }
        }
        drop(store);
        fake.state.lock().unwrap().loads.clear();
        let reloaded = SettingsStore::at_path(directory.path().into(), Box::new(fake.clone()));
        assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), 0);
        for route in [
            TextTranslation::OpenAICompatible,
            TextTranslation::DeepL,
            TextTranslation::DeepLX,
            TextTranslation::FollowService,
            TextTranslation::OpenAICompatible,
        ] {
            let request = translation_request(route, "", "", "");
            reloaded.save_credentials(&profile.id, &request).unwrap();
            let credentials = reloaded.configuration().unwrap().credentials;
            assert_eq!(credentials.alibaba_key(), Some("synthetic-asr"));
            match route {
                TextTranslation::OpenAICompatible => {
                    let ProviderCredentials::OpenAICompatible {
                        endpoint,
                        api_key,
                        model,
                        ..
                    } = &credentials
                    else {
                        panic!("expected OpenAI-compatible credentials")
                    };
                    assert!(endpoint.starts_with("https://third-party.example/v1"));
                    assert_eq!(api_key, "synthetic-openai");
                    assert_eq!(model, "synthetic-model");
                }
                TextTranslation::DeepL => assert!(
                    matches!(credentials, ProviderCredentials::DeepL { api_key, .. } if api_key == "synthetic-deepl:fx")
                ),
                TextTranslation::DeepLX => assert!(
                    matches!(credentials, ProviderCredentials::DeepLX { token, endpoint, .. } if token == "synthetic-deeplx" && endpoint == "https://example.com/translate")
                ),
                TextTranslation::XAI | TextTranslation::ChatMock | TextTranslation::Apple => {
                    unreachable!("covered by independent local and ChatMock persistence tests")
                }
                TextTranslation::FollowService => {
                    assert_eq!(credentials.direct_api_key(), Some("synthetic-asr"))
                }
            }
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
                Some(original_asr_item.as_str())
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
fn openai_compatible_explicit_key_removal_preserves_endpoint_model_and_other_secrets() {
    for provider in [
        ProviderKind::AlibabaCloud,
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = store.create_profile(provider, "Remove text key").unwrap();
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
                &translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
            )
            .unwrap();
        store
            .save_credentials(
                &profile.id,
                &openai_compatible_request(
                    "",
                    "http://127.0.0.1:8000/v1",
                    "synthetic-reverse-proxy",
                    "local-model",
                ),
            )
            .unwrap();
        let speech_before = fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile));
        let destination_account = SettingsStore::destination_account(&profile);
        let before = fake.value(PROFILE_KEYCHAIN_SERVICE, &destination_account);
        // Contradictory or unrelated removal requests fail before any mutation.
        for (route, token) in [
            (TextTranslation::OpenAICompatible, "synthetic-new-key"),
            (TextTranslation::DeepL, ""),
            (TextTranslation::DeepLX, ""),
            (TextTranslation::FollowService, ""),
        ] {
            let request = ProviderCredentials::AlibabaTranslation {
                api_key: String::new(),
                text_translation: route,
                endpoint: String::new(),
                token: token.into(),
                model: String::new(),
                clear_token: true,
            };
            assert!(store.save_credentials(&profile.id, &request).is_err());
            assert_eq!(
                fake.value(PROFILE_KEYCHAIN_SERVICE, &destination_account),
                before
            );
        }
        let request = ProviderCredentials::AlibabaTranslation {
            api_key: String::new(),
            text_translation: TextTranslation::OpenAICompatible,
            endpoint: String::new(),
            token: String::new(),
            model: String::new(),
            clear_token: true,
        };
        store.save_credentials(&profile.id, &request).unwrap();
        let profile = store.profile(&profile.id).unwrap();
        let text = store
            .text_credentials_for_profile(&profile)
            .unwrap()
            .unwrap();
        assert!(
            matches!(text, TextTranslationCredentials::OpenAICompatible { endpoint, api_key, model }
                if endpoint == "http://127.0.0.1:8000/v1/chat/completions" && api_key.is_empty() && model == "local-model")
        );
        let destination = fake
            .value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
            .unwrap();
        assert!(!destination.contains("synthetic-reverse-proxy"));
        assert!(destination.contains("synthetic-deepl:fx"));
        assert_eq!(
            fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
            speech_before
        );
    }
}

#[test]
fn missing_openai_compatible_fields_leave_route_and_secret_items_unchanged() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store.active_profile().unwrap();
    store.save_api_key(&profile.id, "synthetic-asr").unwrap();
    for request in [
        openai_compatible_request("", "", "synthetic-key", "synthetic-model"),
        openai_compatible_request("", "https://example.com/v1", "synthetic-key", ""),
    ] {
        assert!(store.save_credentials(&profile.id, &request).is_err());
        assert_eq!(
            store.profile(&profile.id).unwrap().text_translation(),
            TextTranslation::FollowService
        );
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
            .is_none());
    }
    let before_loads = fake.state.lock().unwrap().loads.len();
    for request in [
        openai_compatible_request(
            "",
            "http://example.com/v1",
            "synthetic-key",
            "synthetic-model",
        ),
        openai_compatible_request(
            "",
            "https://example.com/v1",
            "synthetic-key",
            "invalid\nmodel",
        ),
    ] {
        let error = store.save_credentials(&profile.id, &request).unwrap_err();
        assert!(!error.contains("synthetic"));
    }
    assert_eq!(fake.state.lock().unwrap().loads.len(), before_loads);
}

#[test]
fn openai_compatible_model_updates_retain_keys_but_new_destinations_never_inherit_them() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store.active_profile().unwrap();
    store
        .save_credentials(
            &profile.id,
            &openai_compatible_request(
                "synthetic-asr",
                "https://first.example/v1",
                "synthetic-first",
                "first-model",
            ),
        )
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &openai_compatible_request("", "", "", "updated-model"),
        )
        .unwrap();
    let config = store.configuration().unwrap();
    assert!(
        matches!(&config.credentials, ProviderCredentials::OpenAICompatible {
            endpoint, api_key, model, ..
        } if endpoint.starts_with("https://first.example/") && api_key == "synthetic-first" && model == "updated-model")
    );
    store
        .save_credentials(
            &profile.id,
            &openai_compatible_request("", "http://127.0.0.1:8000/v1", "", "local-model"),
        )
        .unwrap();
    let config = store.configuration().unwrap();
    assert!(
        matches!(&config.credentials, ProviderCredentials::OpenAICompatible {
            asr_api_key, endpoint, api_key, model,
        } if asr_api_key == "synthetic-asr" && endpoint == "http://127.0.0.1:8000/v1/chat/completions" && api_key.is_empty() && model == "local-model")
    );
    assert!(!fake
        .value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        )
        .unwrap()
        .contains("synthetic-first"));
}

#[test]
fn deepl_preset_and_custom_destination_keep_their_secrets_separate() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Speech")
        .unwrap();
    store.save_api_key(&profile.id, "synthetic-asr").unwrap();
    store.select_profile(&profile.id).unwrap();
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepL, "", "", "synthetic-deepl:fx"),
        )
        .unwrap();
    assert!(matches!(store.configuration().unwrap().credentials,
            ProviderCredentials::DeepL { asr_api_key, api_key } if asr_api_key == "synthetic-asr" && api_key == "synthetic-deepl:fx"));
    store
        .save_credentials(
            &profile.id,
            &translation_request(
                TextTranslation::DeepLX,
                "",
                "https://example.com/translate",
                "synthetic-custom",
            ),
        )
        .unwrap();
    assert!(matches!(store.configuration().unwrap().credentials,
            ProviderCredentials::DeepLX { token, .. } if token == "synthetic-custom"));
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::FollowService, "", "", ""),
        )
        .unwrap();
    assert_eq!(
        store.configuration().unwrap().credentials.alibaba_key(),
        Some("synthetic-asr")
    );
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepL, "", "", ""),
        )
        .unwrap();
    assert!(matches!(store.configuration().unwrap().credentials,
            ProviderCredentials::DeepL { api_key, .. } if api_key == "synthetic-deepl:fx"));
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepLX, "", "", ""),
        )
        .unwrap();
    assert!(matches!(store.configuration().unwrap().credentials,
            ProviderCredentials::DeepLX { endpoint, token, .. } if endpoint == "https://example.com/translate" && token == "synthetic-custom"));
    let snapshot = serde_json::to_string(&store.profile_catalog().unwrap()).unwrap();
    for private in [
        "synthetic-asr",
        "synthetic-deepl",
        "synthetic-custom",
        "example.com",
    ] {
        assert!(!snapshot.contains(private));
    }
    let profile = store.profile(&profile.id).unwrap();
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile))
            .as_deref(),
        Some("synthetic-asr")
    );
}

#[test]
fn missing_deepl_key_does_not_replace_the_working_route_or_write_credentials() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Speech")
        .unwrap();
    store.save_api_key(&profile.id, "synthetic-asr").unwrap();
    assert!(store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepL, "", "", "")
        )
        .is_err());
    assert_eq!(
        store.profile(&profile.id).unwrap().text_translation(),
        TextTranslation::FollowService
    );
    assert!(fake
        .value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        )
        .is_none());
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile))
            .as_deref(),
        Some("synthetic-asr")
    );
}

#[test]
fn alibaba_advanced_translation_reuses_one_raw_key_and_retains_the_destination() {
    let fake = FakeSecretStore::default();
    let store = SettingsStore::in_memory(Box::new(fake.clone()), false);
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Work")
        .unwrap();
    store.save_api_key(&profile.id, "synthetic-asr").unwrap();
    let account = credential_account(&profile);
    let destination_account = SettingsStore::destination_account(&profile);
    store
        .save_credentials(
            &profile.id,
            &translation_request(
                TextTranslation::DeepLX,
                "",
                "https://example.com/api",
                "synthetic-token",
            ),
        )
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &account).as_deref(),
        Some("synthetic-asr")
    );
    let destination = fake
        .value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
        .unwrap();
    assert!(!destination.contains("synthetic-asr"));
    assert!(destination.contains("synthetic-token"));
    assert_eq!(
        store.configuration().unwrap().provider,
        ProviderKind::DeepLX
    );
    let snapshot = serde_json::to_string(&store.profile_catalog().unwrap()).unwrap();
    for secret in ["synthetic-asr", "synthetic-token", "example.com"] {
        assert!(!snapshot.contains(secret));
    }
    store.update_profile(&profile.id, "Renamed").unwrap();
    assert_eq!(
        store.profile(&profile.id).unwrap().text_translation(),
        TextTranslation::DeepLX
    );
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
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &destination_account)
            .as_deref(),
        Some(destination.as_str())
    );
    store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepLX, "", "", "new-synthetic-token"),
        )
        .unwrap();
    store.select_profile(&profile.id).unwrap();
    assert!(
        matches!(store.configuration().unwrap().credentials, ProviderCredentials::DeepLX { asr_api_key, endpoint, token } if asr_api_key == "synthetic-asr" && endpoint == "https://example.com/api/translate" && token == "new-synthetic-token")
    );
    store.delete_api_key(&profile.id).unwrap();
    assert_eq!(fake.value(PROFILE_KEYCHAIN_SERVICE, &account), None);
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &destination_account),
        None
    );
}

#[test]
fn advanced_translation_rejects_unsupported_providers_invalid_destinations_and_missing_keys() {
    let fake = FakeSecretStore::default();
    let store = SettingsStore::in_memory(Box::new(fake.clone()), false);
    let other = store
        .create_profile(ProviderKind::OpenAIRealtime, "Other")
        .unwrap();
    assert!(store
        .save_credentials(
            &other.id,
            &translation_request(
                TextTranslation::DeepLX,
                "synthetic",
                "https://example.com",
                ""
            )
        )
        .is_err());
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Ali")
        .unwrap();
    let before_loads = fake.state.lock().unwrap().loads.len();
    assert!(store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepLX, "synthetic", "bad", "")
        )
        .is_err());
    assert_eq!(fake.state.lock().unwrap().loads.len(), before_loads);
    assert!(store
        .save_credentials(
            &profile.id,
            &translation_request(TextTranslation::DeepLX, "", "https://example.com", "")
        )
        .is_err());
    assert_eq!(
        fake.value(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
        None
    );
    assert_eq!(
        fake.value(
            PROFILE_KEYCHAIN_SERVICE,
            &SettingsStore::destination_account(&profile)
        ),
        None
    );
    assert_eq!(store.profile(&profile.id).unwrap().text_translation, None);
}
