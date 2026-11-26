#[test]
fn azure_asr_storage_and_drafts_do_not_require_an_unused_translation_deployment() {
    use super::*;
    let mut profile =
        ServiceProfile::new("azure", "Azure", ProviderKind::AzureOpenAIRealtime).unwrap();
    profile
        .set_speech_translation_mode(SpeechTranslationMode::AsrMt)
        .unwrap();
    let credentials = ProviderCredentials::AzureOpenAI {
        endpoint: "https://synthetic.openai.azure.com".into(),
        deployment: String::new(),
        transcription_deployment: "speech-alias".into(),
        api_key: "synthetic-key".into(),
    };
    let encoded = credentials.encode_for_profile(&profile).unwrap();
    assert_eq!(
        ProviderCredentials::decode_for_profile(&profile, &encoded).unwrap(),
        credentials
    );
    let blank = ProviderCredentials::AzureOpenAI {
        endpoint: String::new(),
        deployment: String::new(),
        transcription_deployment: String::new(),
        api_key: String::new(),
    };
    assert_eq!(
        blank
            .resolve_draft_for_profile(&profile, Some(&credentials))
            .unwrap(),
        credentials
    );
    profile
        .set_speech_translation_mode(SpeechTranslationMode::Simultaneous)
        .unwrap();
    assert!(credentials.encode_for_profile(&profile).is_err());
    assert!(ProviderCredentials::decode_for_profile(&profile, &encoded).is_err());
}

#[test]
fn apple_credentials_cannot_be_saved_decoded_or_revealed_as_a_secret() {
    let profile = ServiceProfile::new("apple", "Apple", ProviderKind::AppleSpeech).unwrap();
    assert_eq!(
        ProviderCredentials::AppleSpeech.validated_for(profile.provider),
        Ok(ProviderCredentials::AppleSpeech)
    );
    assert!(ProviderCredentials::AppleSpeech
        .encode_for_keychain(profile.provider)
        .is_err());
    assert!(ProviderCredentials::decode_from_keychain(
        profile.provider,
        "{\"kind\":\"appleSpeech\"}"
    )
    .is_err());
    assert!(!CredentialRevealField::AsrApiKey.allowed_for(&profile, None));
    assert!(!CredentialRevealField::ApiKey.allowed_for(&profile, None));
    assert_eq!(ProviderCredentials::AppleSpeech.direct_api_key(), None);
}

use super::*;

#[test]
fn text_draft_merging_binds_optional_auth_to_route_and_endpoint() {
    for route in [
        TextTranslation::OpenAICompatible,
        TextTranslation::ChatMock,
        TextTranslation::DeepLX,
    ] {
        let saved = TextTranslationCredentials::resolve_update(
            route,
            None,
            "https://text.example/v1",
            Some("synthetic-saved"),
            "saved-model",
        )
        .unwrap();
        let same = TextTranslationCredentials::resolve_update(
            route,
            Some(&saved),
            "https://text.example/v1",
            None,
            "draft-model",
        )
        .unwrap();
        let changed = TextTranslationCredentials::resolve_update(
            route,
            Some(&saved),
            "https://other.example/v1",
            None,
            "draft-model",
        )
        .unwrap();
        let key = |value: &TextTranslationCredentials| match value {
            TextTranslationCredentials::OpenAICompatible { api_key, .. }
            | TextTranslationCredentials::ChatMock { api_key, .. } => api_key.clone(),
            TextTranslationCredentials::DeepLX { token, .. } => token.clone(),
            _ => unreachable!(),
        };
        assert_eq!(key(&same), "synthetic-saved");
        assert!(key(&changed).is_empty());
        let cleared =
            TextTranslationCredentials::resolve_update(route, Some(&saved), "", Some(""), "")
                .unwrap();
        assert!(key(&cleared).is_empty());
        if route.uses_chat_completions() {
            let other_route = if route == TextTranslation::ChatMock {
                TextTranslation::OpenAICompatible
            } else {
                TextTranslation::ChatMock
            };
            let other = TextTranslationCredentials::resolve_update(
                other_route,
                Some(&saved),
                "https://text.example/v1",
                None,
                "other-model",
            )
            .unwrap();
            assert!(key(&other).is_empty());
        }
    }
}

#[test]
fn azure_drafts_compare_canonical_resource_owners_before_retaining_keys() {
    let draft = ProviderCredentials::AzureOpenAI {
        endpoint: String::new(),
        deployment: String::new(),
        transcription_deployment: String::new(),
        api_key: String::new(),
    };
    for endpoint in [
        "https://saved.openai.azure.com/",
        " https://SAVED.openai.azure.com/// ",
    ] {
        let saved = ProviderCredentials::AzureOpenAI {
            endpoint: endpoint.into(),
            deployment: "translate".into(),
            transcription_deployment: "transcribe".into(),
            api_key: "synthetic-saved-key".into(),
        };
        let resolved = draft
            .resolve_draft(ProviderKind::AzureOpenAIRealtime, Some(&saved))
            .unwrap();
        assert_eq!(
            resolved.azure_openai(),
            Some((
                "https://saved.openai.azure.com",
                "translate",
                "transcribe",
                "synthetic-saved-key"
            ))
        );
        let changed = ProviderCredentials::AzureOpenAI {
            endpoint: "https://different.openai.azure.com".into(),
            deployment: String::new(),
            transcription_deployment: String::new(),
            api_key: String::new(),
        };
        assert!(changed
            .resolve_draft(ProviderKind::AzureOpenAIRealtime, Some(&saved))
            .is_err());
    }
    let damaged = ProviderCredentials::AzureOpenAI {
        endpoint: "invalid-legacy-endpoint".into(),
        deployment: String::new(),
        transcription_deployment: String::new(),
        api_key: "synthetic-old-key".into(),
    };
    let replacement = ProviderCredentials::AzureOpenAI {
        endpoint: "https://replacement.openai.azure.com".into(),
        deployment: "translate".into(),
        transcription_deployment: "transcribe".into(),
        api_key: "synthetic-explicit-key".into(),
    };
    assert_eq!(
        replacement.resolve_draft(ProviderKind::AzureOpenAIRealtime, Some(&damaged)),
        Ok(replacement)
    );
}

#[test]
fn general_drafts_reuse_keys_only_for_unchanged_service_identity() {
    let azure = ProviderCredentials::AzureOpenAI {
        endpoint: "https://old.openai.azure.com".into(),
        deployment: "translate".into(),
        transcription_deployment: "transcribe".into(),
        api_key: "synthetic-azure".into(),
    };
    let tencent = ProviderCredentials::TencentCloud {
        app_id: "123".into(),
        secret_id: "synthetic-id".into(),
        secret_key: "synthetic-key".into(),
    };
    let baidu = ProviderCredentials::BaiduTranslate {
        app_id: "123".into(),
        app_key: "synthetic-baidu".into(),
    };
    for (provider, saved, blank, changed) in [
        (
            ProviderKind::AzureOpenAIRealtime,
            azure,
            ProviderCredentials::AzureOpenAI {
                endpoint: String::new(),
                deployment: "new-deployment".into(),
                transcription_deployment: String::new(),
                api_key: String::new(),
            },
            ProviderCredentials::AzureOpenAI {
                endpoint: "https://new.openai.azure.com".into(),
                deployment: String::new(),
                transcription_deployment: String::new(),
                api_key: String::new(),
            },
        ),
        (
            ProviderKind::TencentCloud,
            tencent,
            ProviderCredentials::TencentCloud {
                app_id: "123".into(),
                secret_id: String::new(),
                secret_key: String::new(),
            },
            ProviderCredentials::TencentCloud {
                app_id: "456".into(),
                secret_id: String::new(),
                secret_key: String::new(),
            },
        ),
        (
            ProviderKind::BaiduTranslate,
            baidu,
            ProviderCredentials::BaiduTranslate {
                app_id: "123".into(),
                app_key: String::new(),
            },
            ProviderCredentials::BaiduTranslate {
                app_id: "456".into(),
                app_key: String::new(),
            },
        ),
    ] {
        assert!(blank.resolve_draft(provider, Some(&saved)).is_ok());
        assert!(changed.resolve_draft(provider, Some(&saved)).is_err());
    }
}

#[test]
fn custom_speech_draft_does_not_reuse_key_or_model_for_changed_endpoint() {
    let saved = ProviderCredentials::CustomSpeech {
        endpoint: "wss://speech.example/realtime".into(),
        model: "saved-model".into(),
        api_key: "synthetic-key".into(),
    };
    let draft = ProviderCredentials::CustomSpeech {
        endpoint: String::new(),
        model: "new-model".into(),
        api_key: String::new(),
    };
    let resolved = draft
        .resolve_draft(ProviderKind::CustomOpenAIASR, Some(&saved))
        .unwrap();
    assert!(
        matches!(resolved, ProviderCredentials::CustomSpeech { api_key, model, .. } if api_key == "synthetic-key" && model == "new-model")
    );
    let changed = ProviderCredentials::CustomSpeech {
        endpoint: "wss://other.example/realtime".into(),
        model: "new-model".into(),
        api_key: String::new(),
    };
    assert!(changed
        .resolve_draft(ProviderKind::CustomOpenAIASR, Some(&saved))
        .is_err());
}

#[test]
fn custom_speech_credentials_are_independent_scoped_and_debug_redacted() {
    for provider in [
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let credentials = ProviderCredentials::CustomSpeech {
            endpoint: " wss://speech.example/recognition ".into(),
            model: " synthetic-speech-model ".into(),
            api_key: " synthetic-speech-key ".into(),
        };
        let encoded = credentials.encode_for_keychain(provider).unwrap();
        let validated = ProviderCredentials::decode_from_keychain(provider, &encoded).unwrap();
        assert_eq!(validated.alibaba_key(), None);
        assert_eq!(validated.direct_api_key(), Some("synthetic-speech-key"));
        assert!(
            matches!(&validated, ProviderCredentials::CustomSpeech { endpoint, model, .. }
            if endpoint == "wss://speech.example/recognition" && model == "synthetic-speech-model")
        );
        assert_eq!(
            validated.validated_for(ProviderKind::AlibabaCloud),
            Err(ProviderCredentialsError::ProviderMismatch)
        );
        assert_eq!(
            ProviderCredentials::api_key("synthetic").validated_for(provider),
            Err(ProviderCredentialsError::ProviderMismatch)
        );
        let profile = ServiceProfile::new("custom", "Speech", provider).unwrap();
        assert_eq!(
            validated.revealed_field(&profile, CredentialRevealField::ApiKey, None),
            Ok(Some("synthetic-speech-key"))
        );
        assert_eq!(
            validated.revealed_field(&profile, CredentialRevealField::Token, None),
            Err(ProviderCredentialsError::InvalidRevealField)
        );
        for value in [
            "speech.example",
            "synthetic-speech-model",
            "synthetic-speech-key",
        ] {
            assert!(!format!("{validated:?}").contains(value));
        }
    }
    assert!(serde_json::from_str::<ProviderCredentials>(r#"{"kind":"customSpeech","endpoint":"wss://speech.example","model":"model","apiKey":"synthetic","token":"wrong-destination"}"#).is_err());
}

#[test]
fn independent_text_credentials_validate_without_any_recognition_key() {
    let destinations = [
        TextTranslationCredentials::DeepL {
            api_key: " synthetic-deepl:fx ".into(),
        },
        TextTranslationCredentials::DeepLX {
            endpoint: "https://destination.example/translate".into(),
            token: " synthetic-deeplx ".into(),
        },
        TextTranslationCredentials::OpenAICompatible {
            endpoint: "https://destination.example/v1".into(),
            model: " synthetic-text-model ".into(),
            api_key: " synthetic-text-key ".into(),
        },
    ];
    for destination in destinations {
        let validated = destination.validated().unwrap();
        assert_eq!(validated.translation(), destination.translation());
        for value in [
            "destination.example",
            "synthetic-deepl",
            "synthetic-deeplx",
            "synthetic-text-model",
            "synthetic-text-key",
        ] {
            assert!(!format!("{validated:?}").contains(value));
        }
    }
    assert_eq!(
        TextTranslationCredentials::DeepL { api_key: "".into() }.validated(),
        Err(ProviderCredentialsError::MissingTextTranslation)
    );
    let anonymous = TextTranslationCredentials::OpenAICompatible {
        endpoint: "http://127.0.0.1:8000/v1".into(),
        model: "model".into(),
        api_key: "".into(),
    }
    .validated()
    .unwrap();
    assert!(
        matches!(anonymous, TextTranslationCredentials::OpenAICompatible { api_key, .. } if api_key.is_empty())
    );
}

#[test]
fn credential_reveal_is_single_field_and_provider_scoped() {
    let tencent = ServiceProfile::new("tencent", "Tencent", ProviderKind::TencentCloud).unwrap();
    let credentials = ProviderCredentials::TencentCloud {
        app_id: "123".into(),
        secret_id: "synthetic-id".into(),
        secret_key: "synthetic-key".into(),
    };
    assert_eq!(
        credentials.revealed_field(&tencent, CredentialRevealField::SecretId, None),
        Ok(Some("synthetic-id"))
    );
    assert_eq!(
        credentials.revealed_field(&tencent, CredentialRevealField::SecretKey, None),
        Ok(Some("synthetic-key"))
    );
    let alibaba = ServiceProfile::alibaba_default();
    assert_eq!(
        credentials.revealed_field(&alibaba, CredentialRevealField::SecretKey, None),
        Err(ProviderCredentialsError::InvalidRevealField)
    );
    assert_eq!(
        credentials.revealed_field(&tencent, CredentialRevealField::ApiKey, None),
        Err(ProviderCredentialsError::InvalidRevealField)
    );
    assert!(!format!("{credentials:?}").contains("synthetic"));
    for field in ["endpoint", "appId", "api_key", "unknown"] {
        assert!(serde_json::from_str::<CredentialRevealField>(&format!("\"{field}\"")).is_err());
    }
}

#[test]
fn credential_reveal_requires_the_saved_independent_route() {
    let mut profile = ServiceProfile::alibaba_default();
    profile.text_translation = Some(TextTranslation::DeepL);
    let official = ProviderCredentials::DeepL {
        asr_api_key: "synthetic-asr".into(),
        api_key: "synthetic-official:fx".into(),
    };
    assert_eq!(
        official.revealed_field(
            &profile,
            CredentialRevealField::Token,
            Some(TextTranslation::DeepL)
        ),
        Ok(Some("synthetic-official:fx"))
    );
    assert_eq!(
        official.revealed_field(&profile, CredentialRevealField::ApiKey, None),
        Ok(Some("synthetic-asr"))
    );
    for route in [
        None,
        Some(TextTranslation::DeepLX),
        Some(TextTranslation::FollowService),
    ] {
        assert_eq!(
            official.revealed_field(&profile, CredentialRevealField::Token, route),
            Err(ProviderCredentialsError::InvalidRevealField)
        );
    }
    profile.text_translation = Some(TextTranslation::DeepLX);
    let custom = ProviderCredentials::DeepLX {
        asr_api_key: "synthetic-asr".into(),
        endpoint: "https://example.com/translate".into(),
        token: String::new(),
    };
    assert_eq!(
        custom.revealed_field(
            &profile,
            CredentialRevealField::Token,
            Some(TextTranslation::DeepLX)
        ),
        Ok(None)
    );
    let mut legacy = ServiceProfile::new("legacy", "Legacy", ProviderKind::DeepLX).unwrap();
    legacy.text_translation = Some(TextTranslation::FollowService);
    assert_eq!(
        ProviderCredentials::api_key("synthetic-asr").revealed_field(
            &legacy,
            CredentialRevealField::AsrApiKey,
            None
        ),
        Ok(Some("synthetic-asr"))
    );
    let request = ProviderCredentials::AlibabaTranslation {
        api_key: "synthetic-asr".into(),
        text_translation: TextTranslation::DeepLX,
        endpoint: String::new(),
        token: "synthetic-request".into(),
        model: String::new(),
        clear_token: false,
    };
    assert_eq!(
        request.revealed_field(
            &profile,
            CredentialRevealField::Token,
            Some(TextTranslation::DeepLX)
        ),
        Err(ProviderCredentialsError::InvalidRevealField)
    );
}

#[test]
fn credential_reveal_rejects_corrupt_field_without_echoing_it() {
    let profile = ServiceProfile::alibaba_default();
    for value in ["synthetic-private\nbody".into(), "x".repeat(1_025)] {
        let error = ProviderCredentials::api_key(value)
            .revealed_field(&profile, CredentialRevealField::ApiKey, None)
            .unwrap_err();
        assert_eq!(error, ProviderCredentialsError::InvalidStoredValue);
        assert!(!error.to_string().contains("synthetic-private"));
    }
}

#[test]
fn alibaba_translation_request_is_write_only_and_debug_redacted() {
    let request: ProviderCredentials = serde_json::from_str(r#"{"kind":"alibabaTranslation","apiKey":"","textTranslation":"deepLX","endpoint":"https://example.com","token":"synthetic-token"}"#).unwrap();
    assert!(
        matches!(&request, ProviderCredentials::AlibabaTranslation { api_key, text_translation: TextTranslation::DeepLX, clear_token: false, .. } if api_key.is_empty())
    );
    let clear_request: ProviderCredentials = serde_json::from_str(r#"{"kind":"alibabaTranslation","apiKey":"","textTranslation":"openAICompatible","endpoint":"","token":"","clearToken":true}"#).unwrap();
    let encoded = serde_json::to_value(&clear_request).unwrap();
    assert_eq!(encoded["clearToken"], true);
    assert_eq!(
        serde_json::from_value::<ProviderCredentials>(encoded).unwrap(),
        clear_request
    );
    assert!(request
        .encode_for_keychain(ProviderKind::AlibabaCloud)
        .is_err());
    assert!(request.encode_for_keychain(ProviderKind::DeepLX).is_err());
    for value in ["example.com", "synthetic-token"] {
        assert!(!format!("{request:?}").contains(value));
    }
    let mut legacy = ServiceProfile::new("existing", "Existing", ProviderKind::DeepLX).unwrap();
    legacy.text_translation = Some(TextTranslation::FollowService);
    assert!(
        matches!(ProviderCredentials::decode_for_profile(&legacy, "synthetic-asr").unwrap(), ProviderCredentials::ApiKey { api_key } if api_key == "synthetic-asr")
    );
}

#[test]
fn deeplx_credentials_round_trip_securely_and_validate_optional_token() {
    let value = ProviderCredentials::DeepLX {
        asr_api_key: "synthetic-asr".into(),
        endpoint: "https://example.com/api".into(),
        token: "".into(),
    };
    let ipc: ProviderCredentials = serde_json::from_str(r#"{"kind":"deepLX","asrApiKey":"synthetic-asr","endpoint":"https://example.com/api","token":""}"#).unwrap();
    assert_eq!(ipc, value);
    let encoded = value.encode_for_keychain(ProviderKind::DeepLX).unwrap();
    let decoded =
        ProviderCredentials::decode_from_keychain(ProviderKind::DeepLX, &encoded).unwrap();
    assert!(
        matches!(&decoded, ProviderCredentials::DeepLX { endpoint, token, .. } if endpoint == "https://example.com/api/translate" && token.is_empty())
    );
    assert!(!format!("{decoded:?}").contains("synthetic-asr"));
    assert!(!format!("{decoded:?}").contains("example.com"));
    assert!(value.validated_for(ProviderKind::AlibabaCloud).is_err());
    let bad = ProviderCredentials::DeepLX {
        asr_api_key: "".into(),
        endpoint: "https://example.com".into(),
        token: "token".into(),
    };
    assert!(bad.validated_for(ProviderKind::DeepLX).is_err());
    let bad = ProviderCredentials::DeepLX {
        asr_api_key: "asr".into(),
        endpoint: "https://example.com".into(),
        token: "token\nInjected".into(),
    };
    assert!(bad.validated_for(ProviderKind::DeepLX).is_err());
}

#[test]
fn historical_single_api_keys_remain_raw_and_readable() {
    let credentials =
        ProviderCredentials::decode_from_keychain(ProviderKind::OpenAIRealtime, "  sk-legacy  ")
            .unwrap();
    assert_eq!(
        credentials
            .encode_for_keychain(ProviderKind::OpenAIRealtime)
            .unwrap(),
        "sk-legacy"
    );
}

#[test]
fn openai_compatible_credentials_are_scoped_trimmed_and_redacted() {
    let credentials: ProviderCredentials = serde_json::from_str(
        r#"{"kind":"openAICompatible","asrApiKey":" synthetic-asr ","endpoint":"https://example.com/v1","apiKey":" synthetic-third-party ","model":" synthetic-model "}"#,
    )
    .unwrap();
    let validated = credentials
        .validated_for(ProviderKind::AlibabaCloud)
        .unwrap();
    assert_eq!(validated.alibaba_key(), Some("synthetic-asr"));
    let ProviderCredentials::OpenAICompatible {
        endpoint,
        api_key,
        model,
        ..
    } = &validated
    else {
        panic!("expected OpenAI-compatible credentials")
    };
    assert!(endpoint.starts_with("https://example.com/"));
    assert_eq!(api_key, "synthetic-third-party");
    assert_eq!(model, "synthetic-model");
    assert_eq!(validated.direct_api_key(), None);
    assert_eq!(
        validated.validated_for(ProviderKind::OpenAIRealtime),
        Err(ProviderCredentialsError::ProviderMismatch)
    );
    for private in [
        "synthetic-asr",
        "synthetic-third-party",
        "example.com",
        "synthetic-model",
    ] {
        assert!(!format!("{validated:?}").contains(private));
    }
    let mut profile = ServiceProfile::alibaba_default();
    profile.text_translation = Some(TextTranslation::OpenAICompatible);
    assert_eq!(
        validated.revealed_field(
            &profile,
            CredentialRevealField::Token,
            Some(TextTranslation::OpenAICompatible)
        ),
        Ok(Some("synthetic-third-party"))
    );
    assert_eq!(
        validated.revealed_field(
            &profile,
            CredentialRevealField::Token,
            Some(TextTranslation::DeepL)
        ),
        Err(ProviderCredentialsError::InvalidRevealField)
    );
}

#[test]
fn openai_compatible_credentials_fail_closed_on_missing_or_invalid_fields() {
    for (asr_api_key, endpoint, api_key, model, expected) in [
        (
            "",
            "https://example.com/v1",
            "synthetic",
            "model",
            ProviderCredentialsError::Missing(ProviderKind::AlibabaCloud),
        ),
        (
            "asr",
            "http://example.com/v1",
            "synthetic",
            "model",
            ProviderCredentialsError::InvalidOpenAICompatibleEndpoint,
        ),
        (
            "asr",
            "https://example.com/v1?secret=synthetic",
            "synthetic",
            "model",
            ProviderCredentialsError::InvalidOpenAICompatibleEndpoint,
        ),
        (
            "asr",
            "https://example.com/v1",
            "synthetic\ninjected",
            "model",
            ProviderCredentialsError::InvalidField,
        ),
        (
            "asr",
            "https://example.com/v1",
            "synthetic",
            "",
            ProviderCredentialsError::InvalidOpenAICompatibleModel,
        ),
        (
            "asr",
            "https://example.com/v1",
            "synthetic",
            "synthetic\ninjected",
            ProviderCredentialsError::InvalidOpenAICompatibleModel,
        ),
    ] {
        let error = ProviderCredentials::OpenAICompatible {
            asr_api_key: asr_api_key.into(),
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            model: model.into(),
        }
        .validated_for(ProviderKind::AlibabaCloud)
        .unwrap_err();
        assert_eq!(error, expected);
        assert!(!error.to_string().contains("synthetic"));
    }
    assert_eq!(
        ProviderCredentials::OpenAICompatible {
            asr_api_key: "asr".into(),
            endpoint: "https://example.com/v1".into(),
            api_key: "key".into(),
            model: "x".repeat(257),
        }
        .validated_for(ProviderKind::AlibabaCloud)
        .unwrap_err(),
        ProviderCredentialsError::InvalidOpenAICompatibleModel
    );
    let request: ProviderCredentials = serde_json::from_str(
        r#"{"kind":"alibabaTranslation","apiKey":"","textTranslation":"deepLX","endpoint":"","token":""}"#,
    )
    .unwrap();
    assert!(
        matches!(request, ProviderCredentials::AlibabaTranslation { model, .. } if model.is_empty())
    );
}

#[test]
fn structured_credentials_round_trip_without_debug_disclosure() {
    let secret = "tencent-secret-value";
    let credentials = ProviderCredentials::TencentCloud {
        app_id: "123456".into(),
        secret_id: "AKIDexample".into(),
        secret_key: secret.into(),
    };
    let encoded = credentials
        .encode_for_keychain(ProviderKind::TencentCloud)
        .unwrap();
    assert_eq!(
        ProviderCredentials::decode_from_keychain(ProviderKind::TencentCloud, &encoded).unwrap(),
        credentials
    );
    assert!(!format!("{credentials:?}").contains(secret));
}

#[test]
fn frontend_camel_case_ipc_shapes_deserialize_for_every_credential_kind() {
    let cases = [
        (
            r#"{"kind":"apiKey","apiKey":"sk-test"}"#,
            ProviderCredentials::api_key("sk-test"),
        ),
        (
            r#"{"kind":"azureOpenAI","endpoint":"https://mimi.openai.azure.com","deployment":"translate","transcriptionDeployment":"transcribe","apiKey":"azure-secret"}"#,
            ProviderCredentials::AzureOpenAI {
                endpoint: "https://mimi.openai.azure.com".into(),
                deployment: "translate".into(),
                transcription_deployment: "transcribe".into(),
                api_key: "azure-secret".into(),
            },
        ),
        (
            r#"{"kind":"tencentCloud","appId":"123456","secretId":"AKIDexample","secretKey":"tencent-secret"}"#,
            ProviderCredentials::TencentCloud {
                app_id: "123456".into(),
                secret_id: "AKIDexample".into(),
                secret_key: "tencent-secret".into(),
            },
        ),
        (
            r#"{"kind":"baiduTranslate","appId":"app_123","appKey":"baidu-secret"}"#,
            ProviderCredentials::BaiduTranslate {
                app_id: "app_123".into(),
                app_key: "baidu-secret".into(),
            },
        ),
    ];

    for (json, expected) in cases {
        assert_eq!(
            serde_json::from_str::<ProviderCredentials>(json).unwrap(),
            expected
        );
    }
}

#[test]
fn credential_ipc_shape_rejects_snake_case_and_unknown_fields() {
    for json in [
        r#"{"kind":"apiKey","api_key":"sk-test"}"#,
        r#"{"kind":"azureOpenAI","endpoint":"https://mimi.openai.azure.com","deployment":"translate","transcription_deployment":"transcribe","apiKey":"secret"}"#,
        r#"{"kind":"tencentCloud","appId":"123456","secretId":"AKIDexample","secretKey":"secret","extra":"not-allowed"}"#,
        r#"{"kind":"baiduTranslate","appId":"app_123","app_key":"secret"}"#,
    ] {
        assert!(serde_json::from_str::<ProviderCredentials>(json).is_err());
    }
}

#[test]
fn providers_reject_the_wrong_credential_shape() {
    assert_eq!(
        ProviderCredentials::api_key("sk-test")
            .validated_for(ProviderKind::TencentCloud)
            .unwrap_err(),
        ProviderCredentialsError::ProviderMismatch
    );
}

#[test]
fn azure_accepts_only_official_resource_endpoints() {
    let valid = ProviderCredentials::AzureOpenAI {
        endpoint: " https://mimi-test.openai.azure.com/ ".into(),
        deployment: " translate-prod ".into(),
        transcription_deployment: " transcribe-prod ".into(),
        api_key: " azure-secret ".into(),
    }
    .validated_for(ProviderKind::AzureOpenAIRealtime)
    .unwrap();
    assert_eq!(
        valid.azure_openai(),
        Some((
            "https://mimi-test.openai.azure.com",
            "translate-prod",
            "transcribe-prod",
            "azure-secret"
        ))
    );

    for endpoint in [
        "http://mimi.openai.azure.com",
        "https://example.com",
        "https://mimi.openai.azure.com/path",
        "https://mimi.openai.azure.com?secret=value",
        "https://user@mimi.openai.azure.com",
    ] {
        assert_eq!(
            ProviderCredentials::AzureOpenAI {
                endpoint: endpoint.into(),
                deployment: "translate".into(),
                transcription_deployment: "transcribe".into(),
                api_key: "secret".into(),
            }
            .validated_for(ProviderKind::AzureOpenAIRealtime)
            .unwrap_err(),
            ProviderCredentialsError::InvalidAzureEndpoint
        );
    }
}

#[test]
fn every_required_value_is_trimmed_and_empty_values_fail_closed() {
    let baidu = ProviderCredentials::BaiduTranslate {
        app_id: " app_123 ".into(),
        app_key: " secret ".into(),
    }
    .validated_for(ProviderKind::BaiduTranslate)
    .unwrap();
    assert_eq!(baidu.baidu_translate(), Some(("app_123", "secret")));

    assert!(matches!(
        ProviderCredentials::TencentCloud {
            app_id: "123".into(),
            secret_id: "".into(),
            secret_key: "secret".into(),
        }
        .validated_for(ProviderKind::TencentCloud),
        Err(ProviderCredentialsError::Missing(
            ProviderKind::TencentCloud
        ))
    ));
}
#[test]
fn anonymous_openai_compatible_keys_are_isolated_from_required_recognition_credentials() {
    let credentials = ProviderCredentials::OpenAICompatible {
        asr_api_key: "synthetic-asr".into(),
        endpoint: "http://127.0.0.1:8000/v1".into(),
        api_key: String::new(),
        model: "synthetic-model".into(),
    }
    .validated_for(ProviderKind::AlibabaCloud)
    .unwrap();
    assert_eq!(credentials.alibaba_key(), Some("synthetic-asr"));
    assert!(
        matches!(credentials, ProviderCredentials::OpenAICompatible { api_key, .. } if api_key.is_empty())
    );
    for key in [
        "\n".into(),
        "bad\r\nkey".into(),
        "x".repeat(MAXIMUM_CREDENTIAL_FIELD_LENGTH + 1),
    ] {
        assert_eq!(
            TextTranslationCredentials::OpenAICompatible {
                endpoint: "http://127.0.0.1:8000/v1".into(),
                model: "synthetic-model".into(),
                api_key: key,
            }
            .validated(),
            Err(ProviderCredentialsError::InvalidField)
        );
    }
    assert_eq!(
        ProviderCredentials::DeepL {
            asr_api_key: "asr".into(),
            api_key: String::new()
        }
        .validated_for(ProviderKind::AlibabaCloud),
        Err(ProviderCredentialsError::Missing(
            ProviderKind::AlibabaCloud
        ))
    );
}
#[test]
fn chatmock_credentials_keep_route_identity_and_scoped_optional_authentication() {
    let text = TextTranslationCredentials::ChatMock {
        endpoint: "http://127.0.0.1:8000/v1".into(),
        model: "synthetic-model".into(),
        api_key: "".into(),
    }
    .validated()
    .unwrap();
    assert_eq!(text.translation(), TextTranslation::ChatMock);
    let request: ProviderCredentials = serde_json::from_value(serde_json::json!({
        "kind":"alibabaTranslation", "apiKey":"", "textTranslation":"chatMock", "endpoint":"", "token":"", "model":"", "clearToken":true
    })).unwrap();
    assert!(matches!(
        request,
        ProviderCredentials::AlibabaTranslation {
            text_translation: TextTranslation::ChatMock,
            clear_token: true,
            ..
        }
    ));
    let credentials = ProviderCredentials::ChatMock {
        asr_api_key: "synthetic-asr".into(),
        endpoint: "http://127.0.0.1:8000/v1".into(),
        api_key: "synthetic-chatmock".into(),
        model: "synthetic-model".into(),
    }
    .validated_for(ProviderKind::AlibabaCloud)
    .unwrap();
    assert_eq!(credentials.alibaba_key(), Some("synthetic-asr"));
    assert_eq!(credentials.direct_api_key(), None);
    assert_eq!(
        serde_json::to_value(&credentials).unwrap()["kind"],
        "chatMock"
    );
    assert!(!format!("{credentials:?}").contains("synthetic"));
    let mut profile = ServiceProfile::alibaba_default();
    profile.text_translation = Some(TextTranslation::ChatMock);
    assert_eq!(
        credentials.revealed_field(
            &profile,
            CredentialRevealField::Token,
            Some(TextTranslation::ChatMock)
        ),
        Ok(Some("synthetic-chatmock"))
    );
    assert!(credentials
        .revealed_field(
            &profile,
            CredentialRevealField::Token,
            Some(TextTranslation::OpenAICompatible)
        )
        .is_err());
    profile.text_translation = Some(TextTranslation::OpenAICompatible);
    assert!(credentials
        .revealed_field(
            &profile,
            CredentialRevealField::Token,
            Some(TextTranslation::OpenAICompatible)
        )
        .is_err());
    for (endpoint, model, key) in [
        ("http://remote.example/v1", "model", ""),
        ("http://127.0.0.1:8000/v1", "", ""),
        ("http://127.0.0.1:8000/v1", "model", "bad\nkey"),
    ] {
        assert!(TextTranslationCredentials::ChatMock {
            endpoint: endpoint.into(),
            model: model.into(),
            api_key: key.into()
        }
        .validated()
        .is_err());
    }
}
