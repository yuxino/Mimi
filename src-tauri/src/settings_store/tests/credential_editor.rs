//! Credential editor behavior regressions.

use super::*;

#[test]
fn credential_reveal_is_selected_profile_only_and_does_not_touch_migration() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let default = ServiceProfile::alibaba_default();
    let other = store
        .create_profile(ProviderKind::OpenAIRealtime, "Other")
        .unwrap();
    fake.put(
        PROFILE_KEYCHAIN_SERVICE,
        &credential_account(&default),
        "synthetic-default",
    );
    fake.put(
        PROFILE_KEYCHAIN_SERVICE,
        &credential_account(&other),
        "synthetic-other",
    );
    fake.put(
        LEGACY_KEYCHAIN_SERVICE_V3,
        LEGACY_KEYCHAIN_ACCOUNT,
        "synthetic-legacy",
    );
    assert_eq!(
        store
            .reveal_credential(&other.id, CredentialRevealField::ApiKey, None)
            .unwrap()
            .as_deref(),
        Some("synthetic-other")
    );
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&default)),
        0
    );
    assert_eq!(
        store
            .reveal_credential(&default.id, CredentialRevealField::ApiKey, None)
            .unwrap()
            .as_deref(),
        Some("synthetic-default")
    );
    assert_eq!(
        fake.load_count(LEGACY_KEYCHAIN_SERVICE_V3, LEGACY_KEYCHAIN_ACCOUNT),
        0
    );
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, LEGACY_MIGRATION_TOMBSTONE_ACCOUNT),
        0
    );
    assert_eq!(store.active_profile().unwrap().id, default.id);
    let snapshot =
        serde_json::to_string(&(store.profile_catalog().unwrap(), store.preferences())).unwrap();
    assert!(!snapshot.contains("synthetic-"));
}

#[test]
fn credential_editor_state_distinguishes_anonymous_text_from_saved_speech_key() {
    for route in [TextTranslation::OpenAICompatible, TextTranslation::ChatMock] {
        let fake = FakeSecretStore::default();
        let store = settings(&fake);
        let profile = ServiceProfile::alibaba_default();
        store
            .save_api_key(&profile.id, "synthetic-speech-secret")
            .unwrap();
        let request = ProviderCredentials::AlibabaTranslation {
            api_key: String::new(),
            text_translation: route,
            endpoint: "http://127.0.0.1:8000/v1".into(),
            token: String::new(),
            model: "local-model".into(),
            clear_token: false,
        };
        store.save_credentials(&profile.id, &request).unwrap();
        let state = store
            .credential_editor_state(&profile.id, Some(route))
            .unwrap();
        assert!(state.saved_fields.is_empty());
        assert_eq!(
            state.endpoint.as_deref(),
            Some("http://127.0.0.1:8000/v1/chat/completions")
        );
        assert_eq!(state.model.as_deref(), Some("local-model"));
        let speech = store.credential_editor_state(&profile.id, None).unwrap();
        assert_eq!(
            speech.saved_fields,
            vec![
                CredentialRevealField::ApiKey,
                CredentialRevealField::AsrApiKey
            ]
        );
        assert!(speech.endpoint.is_none());
        assert!(speech.model.is_none());
        assert!(!serde_json::to_string(&speech)
            .unwrap()
            .contains("synthetic-speech-secret"));

        let mut authenticated = request.clone();
        let ProviderCredentials::AlibabaTranslation { token, .. } = &mut authenticated else {
            unreachable!()
        };
        *token = "synthetic-text-secret".into();
        store.save_credentials(&profile.id, &authenticated).unwrap();
        let state = store
            .credential_editor_state(&profile.id, Some(route))
            .unwrap();
        assert_eq!(state.saved_fields, vec![CredentialRevealField::Token]);
        let json = serde_json::to_string(&state).unwrap();
        assert!(!json.contains("synthetic-speech-secret"));
        assert!(!json.contains("synthetic-text-secret"));

        let mut anonymous = request;
        let ProviderCredentials::AlibabaTranslation { clear_token, .. } = &mut anonymous else {
            unreachable!()
        };
        *clear_token = true;
        store.save_credentials(&profile.id, &anonymous).unwrap();
        assert!(store
            .credential_editor_state(&profile.id, Some(route))
            .unwrap()
            .saved_fields
            .is_empty());
        let snapshot = serde_json::to_string(
            &crate::commands::SettingsSnapshotPayload::try_from_store(&store).unwrap(),
        )
        .unwrap();
        for private in [
            "synthetic-speech-secret",
            "synthetic-text-secret",
            "127.0.0.1:8000",
            "local-model",
        ] {
            assert!(!snapshot.contains(private));
        }
    }
}

#[test]
fn credential_editor_state_hydrates_requested_text_metadata_without_reading_speech() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = ServiceProfile::alibaba_default();
    store
        .save_credentials(
            &profile.id,
            &openai_compatible_request(
                "synthetic-speech-secret",
                "http://127.0.0.1:8000/v1",
                "",
                "local-model",
            ),
        )
        .unwrap();
    store.secret_cache.lock().unwrap().clear();
    fake.state.lock().unwrap().loads.clear();
    let absent = store
        .credential_editor_state(&profile.id, Some(TextTranslation::DeepLX))
        .unwrap();
    assert!(absent.saved_fields.is_empty());
    assert!(absent.endpoint.is_none());
    let speech_account = credential_account(&profile);
    let text_account = SettingsStore::destination_account(&profile);
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &speech_account);
    assert!(store
        .credential_editor_state(&profile.id, Some(TextTranslation::OpenAICompatible))
        .is_ok());
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &speech_account),
        0
    );
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &text_account), 1);
    assert_eq!(
        store
            .credential_editor_state(&profile.id, None)
            .err()
            .as_deref(),
        Some(CREDENTIAL_STORE_UNAVAILABLE)
    );
}

#[test]
fn credential_editor_state_ui_fixtures_never_access_real_credentials() {
    let fake = FakeSecretStore::default();
    let store = SettingsStore::in_memory(Box::new(fake.clone()), true);
    let profile = ServiceProfile::alibaba_default();
    let speech = store.credential_editor_state(&profile.id, None).unwrap();
    assert!(speech.saved_fields.contains(&CredentialRevealField::ApiKey));
    assert_eq!(
        store.reveal_credential(&profile.id, CredentialRevealField::ApiKey, None),
        Ok(Some("sk-demo-not-a-real-key".into()))
    );
    store
        .save_credentials(
            &profile.id,
            &openai_compatible_request("", "http://127.0.0.1:8000/v1", "", "local-model"),
        )
        .unwrap();
    let text = store
        .credential_editor_state(&profile.id, Some(TextTranslation::OpenAICompatible))
        .unwrap();
    assert!(text.saved_fields.is_empty());
    assert_eq!(
        text.endpoint.as_deref(),
        Some("http://127.0.0.1:8000/v1/chat/completions")
    );
    assert_eq!(text.model.as_deref(), Some("local-model"));
    let state = fake.state.lock().unwrap();
    assert!(state.loads.is_empty());
    assert!(state.values.is_empty());
}

#[test]
fn credential_editor_state_hydrates_provider_fields_without_secret_values() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    for (provider, credentials, expected) in [
        (
            ProviderKind::CustomOpenAIASR,
            custom_speech_request(
                "wss://speech.example/realtime",
                "speech-model",
                "synthetic-speech-secret",
            ),
            serde_json::json!({"savedFields": ["apiKey", "asrApiKey"], "endpoint": "wss://speech.example/realtime", "model": "speech-model"}),
        ),
        (
            ProviderKind::AzureOpenAIRealtime,
            ProviderCredentials::AzureOpenAI {
                endpoint: "https://sample.openai.azure.com".into(),
                deployment: "realtime".into(),
                transcription_deployment: "transcribe".into(),
                api_key: "synthetic-azure-secret".into(),
            },
            serde_json::json!({"savedFields": ["apiKey"], "endpoint": "https://sample.openai.azure.com", "deployment": "realtime", "transcriptionDeployment": "transcribe"}),
        ),
        (
            ProviderKind::TencentCloud,
            ProviderCredentials::TencentCloud {
                app_id: "12345".into(),
                secret_id: "synthetic-tencent-id".into(),
                secret_key: "synthetic-tencent-secret".into(),
            },
            serde_json::json!({"savedFields": ["secretId", "secretKey"], "appId": "12345"}),
        ),
        (
            ProviderKind::BaiduTranslate,
            ProviderCredentials::BaiduTranslate {
                app_id: "67890".into(),
                app_key: "synthetic-baidu-secret".into(),
            },
            serde_json::json!({"savedFields": ["appKey"], "appId": "67890"}),
        ),
    ] {
        let profile = store
            .create_profile(provider, "Saved editor fields")
            .unwrap();
        store.save_credentials(&profile.id, &credentials).unwrap();
        let state = store.credential_editor_state(&profile.id, None).unwrap();
        assert_eq!(serde_json::to_value(state).unwrap(), expected);
    }
}

#[test]
fn credential_editor_state_supports_legacy_deeplx_and_keeps_private_paths_out_of_snapshots() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::DeepLX, "Legacy text destination")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &ProviderCredentials::DeepLX {
                asr_api_key: "synthetic-speech-secret".into(),
                endpoint: "https://example.com/private-path/translate".into(),
                token: String::new(),
            },
        )
        .unwrap();
    let state = store
        .credential_editor_state(&profile.id, Some(TextTranslation::DeepLX))
        .unwrap();
    assert!(state.saved_fields.is_empty());
    assert_eq!(
        state.endpoint.as_deref(),
        Some("https://example.com/private-path/translate")
    );
    let snapshot = serde_json::to_string(
        &crate::commands::SettingsSnapshotPayload::try_from_store(&store).unwrap(),
    )
    .unwrap();
    assert!(!snapshot.contains("private-path"));
    assert!(!snapshot.contains("synthetic-speech-secret"));
}

#[test]
fn credential_reveal_has_safe_missing_failure_and_mismatch_results() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::OpenAIRealtime, "Missing")
        .unwrap();
    assert_eq!(
        store
            .reveal_credential(
                &profile.id,
                CredentialRevealField::Token,
                Some(TextTranslation::DeepLX)
            )
            .unwrap_err(),
        "credential_reveal_field_mismatch"
    );
    assert_eq!(
        fake.load_count(PROFILE_KEYCHAIN_SERVICE, &credential_account(&profile)),
        0
    );
    assert_eq!(
        store.reveal_credential(&profile.id, CredentialRevealField::ApiKey, None),
        Ok(None)
    );
    let failing = store
        .create_profile(ProviderKind::OpenAIRealtime, "Unavailable")
        .unwrap();
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &credential_account(&failing));
    assert_eq!(
        store
            .reveal_credential(&failing.id, CredentialRevealField::ApiKey, None)
            .unwrap_err(),
        CREDENTIAL_STORE_UNAVAILABLE
    );
}

#[test]
fn credential_reveal_destination_is_saved_route_scoped_and_independent_of_asr() {
    let fake = FakeSecretStore::default();
    let store = settings(&fake);
    let profile = store
        .create_profile(ProviderKind::AlibabaCloud, "Speech")
        .unwrap();
    store
        .save_credentials(
            &profile.id,
            &translation_request(
                TextTranslation::DeepL,
                "synthetic-asr",
                "",
                "synthetic-official:fx",
            ),
        )
        .unwrap();
    let profile = store.profile(&profile.id).unwrap();
    let account = SettingsStore::destination_account(&profile);
    let count = fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account);
    assert_eq!(
        store
            .reveal_credential(
                &profile.id,
                CredentialRevealField::Token,
                Some(TextTranslation::DeepLX)
            )
            .unwrap_err(),
        "credential_reveal_field_mismatch"
    );
    assert_eq!(fake.load_count(PROFILE_KEYCHAIN_SERVICE, &account), count);
    assert_eq!(
        store
            .reveal_credential(
                &profile.id,
                CredentialRevealField::Token,
                Some(TextTranslation::DeepL)
            )
            .unwrap()
            .as_deref(),
        Some("synthetic-official:fx")
    );
    // A broken destination must not make the independent ASR key unreadable.
    fake.make_unavailable(PROFILE_KEYCHAIN_SERVICE, &account);
    store
        .secret_cache
        .lock()
        .unwrap()
        .remove(&cache_key(PROFILE_KEYCHAIN_SERVICE, &account));
    assert_eq!(
        store
            .reveal_credential(&profile.id, CredentialRevealField::ApiKey, None)
            .unwrap()
            .as_deref(),
        Some("synthetic-asr")
    );
    assert_eq!(
        store
            .reveal_credential(
                &profile.id,
                CredentialRevealField::Token,
                Some(TextTranslation::DeepL)
            )
            .unwrap_err(),
        CREDENTIAL_STORE_UNAVAILABLE
    );
}
