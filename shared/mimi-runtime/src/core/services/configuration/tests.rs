#[test]
fn builtin_asr_modes_use_independent_keys_and_real_recognition_routes() {
    use super::*;
    for (provider, endpoint, rate) in [
        (ProviderKind::OpenAIRealtime, "wss://api.openai.com/v1/realtime", 24_000),
        (ProviderKind::GoogleGeminiLive, "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent", 16_000),
        (ProviderKind::AzureOpenAIRealtime, "wss://synthetic.openai.azure.com/openai/v1/realtime", 24_000),
    ] {
        let speech = if provider == ProviderKind::AzureOpenAIRealtime {
            ProviderCredentials::AzureOpenAI { endpoint: "https://synthetic.openai.azure.com".into(), deployment: String::new(), transcription_deployment: "transcribe-deployment".into(), api_key: "synthetic-speech".into() }
        } else { ProviderCredentials::api_key("synthetic-speech") };
        let mut config = LiveTranslationConfiguration::with_credentials(provider, speech, SourceLanguage::English, TargetLanguage::Japanese, TranslationMode::Turbo)
            .with_speech_translation_mode(SpeechTranslationMode::AsrMt);
        assert!(matches!(config.validated(), Err(LiveTranslationConfigurationError::Credentials(ProviderCredentialsError::MissingTextTranslation))));
        config.text_credentials = Some(TextTranslationCredentials::OpenAICompatible { endpoint: "https://example.com/v1/chat/completions".into(), api_key: String::new(), model: "text-model".into() });
        let checked = config.validated().unwrap();
        assert_eq!(checked.capabilities().input_sample_rate_hz, rate);
        let (actual, text) = checked.network_endpoints().unwrap();
        assert_eq!(actual, endpoint);
        assert_eq!(text.as_deref(), Some("https://example.com/v1/chat/completions"));
        assert_eq!(checked.model_names()[1], "text-model");
        config.target_language = TargetLanguage::Original;
        config.text_credentials = None;
        assert!(config.validated().unwrap().network_endpoints().unwrap().1.is_none());
    }
}

#[test]
fn xai_streaming_uses_its_profile_key_model_and_independent_text_route() {
    use super::*;
    let legacy = LiveTranslationConfiguration::for_provider(
        ProviderKind::XAIRealtime,
        "synthetic-xai-key",
        SourceLanguage::Automatic,
        TargetLanguage::SimplifiedChinese,
        TranslationMode::Turbo,
    );
    assert!(!legacy.uses_xai_asr());
    assert_eq!(legacy.model_names(), ["grok-voice-latest"]);
    let streaming = legacy
        .with_xai_speech(XAISpeechMode::AsrMt, "legacy-unused-model")
        .with_text_credentials(TextTranslationCredentials::XAI {
            api_key: "synthetic-xai-text-key".into(),
            model: "custom-xai-text-model".into(),
        });
    let streaming = streaming.validated().unwrap();
    assert_eq!(streaming.capabilities().input_sample_rate_hz, 16_000);
    assert_eq!(streaming.model_names()[1], "custom-xai-text-model");
    let (speech, text) = streaming.network_endpoints().unwrap();
    assert_eq!(speech, "wss://api.x.ai/v1/stt");
    assert_eq!(
        text.as_deref(),
        Some(crate::core::services::provider::XAI_TEXT_ENDPOINT)
    );
    let probe = streaming.xai_text_probe_configuration().unwrap();
    let TextTranslationProbeCredentials::Independent(TextTranslationCredentials::XAI {
        model,
        api_key,
    }) = probe.credentials
    else {
        panic!("xAI must use its text endpoint");
    };
    assert_eq!(model, "custom-xai-text-model");
    assert_eq!(api_key, "synthetic-xai-text-key");
    let mut original = streaming;
    original.target_language = TargetLanguage::Original;
    assert!(original.network_endpoints().unwrap().1.is_none());
    assert_eq!(original.model_names().len(), 1);
    assert_eq!(
        original
            .xai_text_probe_configuration()
            .unwrap()
            .target_language,
        TargetLanguage::SimplifiedChinese
    );
    original.provider = ProviderKind::AlibabaCloud;
    assert!(original.validated().is_err());
}

#[test]
fn xai_asr_requires_independent_text_and_original_drops_it() {
    use super::*;
    let configuration = LiveTranslationConfiguration::for_provider(
        ProviderKind::XAIRealtime,
        "synthetic-speech-key",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        TranslationMode::Turbo,
    )
    .with_xai_speech(XAISpeechMode::AsrMt, "legacy-model-must-not-supply-a-route");
    assert!(configuration.is_standalone_asr());
    assert!(matches!(
        configuration.validated(),
        Err(LiveTranslationConfigurationError::Credentials(
            ProviderCredentialsError::MissingTextTranslation
        ))
    ));
    assert!(matches!(
        configuration.xai_text_probe_configuration(),
        Err(LiveTranslationConfigurationError::Credentials(
            ProviderCredentialsError::MissingTextTranslation
        ))
    ));
    let mut original = configuration.clone();
    original.target_language = TargetLanguage::Original;
    original.text_credentials = Some(TextTranslationCredentials::XAI {
        api_key: String::new(),
        model: String::new(),
    });
    assert!(original.validated().unwrap().text_credentials.is_none());
    let custom = configuration
        .with_text_credentials(TextTranslationCredentials::OpenAICompatible {
            endpoint: "http://127.0.0.1:1/v1".into(),
            api_key: "synthetic-independent-key".into(),
            model: "chosen-text-model".into(),
        })
        .validated()
        .unwrap();
    assert_eq!(
        custom.model_names(),
        [
            crate::core::protocols::recognition::xai_stt::MODEL,
            "chosen-text-model"
        ]
    );
    assert_eq!(
        custom.network_endpoints().unwrap().1.as_deref(),
        Some("http://127.0.0.1:1/v1/chat/completions")
    );
    let TextTranslationProbeCredentials::Independent(
        TextTranslationCredentials::OpenAICompatible { api_key, .. },
    ) = custom.xai_text_probe_configuration().unwrap().credentials
    else {
        panic!("expected selected independent probe");
    };
    assert_eq!(api_key, "synthetic-independent-key");
}

#[test]
fn experimental_realtime_configuration_requires_workspace_and_has_one_model() {
    use super::*;
    let config = LiveTranslationConfiguration::for_provider(
        ProviderKind::AlibabaCloud,
        "synthetic-key",
        SourceLanguage::Automatic,
        TargetLanguage::English,
        TranslationMode::Turbo,
    );
    assert_eq!(config.alibaba_realtime_mode, AlibabaRealtimeMode::Disabled);
    let mode = AlibabaRealtimeMode::LiveTranslate;
    assert_eq!(
        config
            .clone()
            .with_alibaba_realtime(mode, "")
            .validated()
            .unwrap_err(),
        LiveTranslationConfigurationError::InvalidAlibabaWorkspaceId
    );
    let active = config.clone().with_alibaba_realtime(mode, "test-workspace");
    assert_eq!(
        active.model_names(),
        vec![mode.model_name().unwrap().to_string()]
    );
    assert!(active.validated().is_ok());
    let mut unsupported = active;
    unsupported.source_language = SourceLanguage::English;
    assert_eq!(
        unsupported.validated().unwrap_err(),
        LiveTranslationConfigurationError::UnsupportedSourceLanguage
    );
}

#[test]
fn model_names_follow_selected_route_and_never_include_secrets_or_legacy_modes() {
    use super::*;
    let mut config = LiveTranslationConfiguration::for_provider(
        ProviderKind::AlibabaCloud,
        "synthetic-private-key",
        SourceLanguage::Automatic,
        TargetLanguage::SimplifiedChinese,
        TranslationMode::Turbo,
    );
    for model in [QwenMTModel::Lite, QwenMTModel::Flash, QwenMTModel::Plus] {
        config.qwen_mt_model = model;
        for mode in [
            TranslationMode::Turbo,
            TranslationMode::LowLatency,
            TranslationMode::HighQuality,
        ] {
            config.translation_mode = mode;
            assert_eq!(
                config.model_names(),
                ["qwen-audio-3.0-asr-flash-streaming", model.raw_name()]
            );
        }
    }
    config.text_credentials = Some(TextTranslationCredentials::OpenAICompatible {
        endpoint: "https://synthetic.invalid/v1".into(),
        model: "selected-text-model".into(),
        api_key: "synthetic-private-text-key".into(),
    });
    assert_eq!(config.model_names().last().unwrap(), "selected-text-model");
    config.target_language = TargetLanguage::Original;
    assert_eq!(config.model_names(), ["qwen-audio-3.0-asr-flash-streaming"]);
    for (provider, model) in [
        (
            ProviderKind::GoogleGeminiLive,
            "gemini-3.5-live-translate-preview",
        ),
        (ProviderKind::OpenAIRealtime, "gpt-realtime-translate"),
        (ProviderKind::XAIRealtime, "grok-voice-latest"),
    ] {
        config.provider = provider;
        assert_eq!(config.model_names(), [model]);
    }
}
#[test]
fn adapter_configuration_uses_the_actual_pipeline_and_original_has_no_text_stage() {
    let raw = serde_json::json!({
        "provider": "alibabaCloud", "credentials": {"kind": "apiKey", "apiKey": "synthetic"},
        "textCredentials": null, "sourceLanguage": "auto", "targetLanguage": "zh",
        "qwenMtModel": "lite", "translationMode": "turbo",
        "networkProxy": {"mode": "direct", "url": null},
        "textNetworkProxy": {"mode": "direct", "url": null}
    });
    let config: LiveTranslationConfiguration = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(config.alibaba_realtime_mode, AlibabaRealtimeMode::Disabled);
    assert!(config.alibaba_workspace_id.is_empty());
    let config = config.validated().unwrap();
    let (speech, text) = config.network_endpoints().unwrap();
    assert!(speech.starts_with("wss://"));
    assert!(text.unwrap().starts_with("https://"));
    assert!(!speech.contains("synthetic"));
    let mut original = config;
    original.target_language = TargetLanguage::Original;
    assert!(original.network_endpoints().unwrap().1.is_none());
    let mut unknown = raw;
    unknown["copiedAndroidSetting"] = serde_json::json!(true);
    assert!(serde_json::from_value::<LiveTranslationConfiguration>(unknown).is_err());
}

#[test]
fn apple_text_credentials_are_local_and_keep_speech_network_independent() {
    use crate::core::services::network_proxy::ProxyMode;
    let speech = ProxyConfig {
        mode: ProxyMode::Custom,
        url: Some("http://localhost:7890".into()),
    };
    let config = LiveTranslationConfiguration::with_credentials(
        ProviderKind::AlibabaCloud,
        ProviderCredentials::api_key("synthetic-asr"),
        SourceLanguage::English,
        TargetLanguage::Japanese,
        TranslationMode::Turbo,
    )
    .with_text_credentials(TextTranslationCredentials::Apple)
    .with_stage_network_proxies(
        speech.clone(),
        ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some("invalid".into()),
        },
    );
    let validated = config.validated().unwrap();
    assert_eq!(validated.network_proxy, speech.validate().unwrap());
    assert_eq!(validated.text_network_proxy.mode, ProxyMode::Direct);
    assert_eq!(
        validated.text_credentials,
        Some(TextTranslationCredentials::Apple)
    );
    let mut invalid = config.clone();
    invalid.source_language = SourceLanguage::Automatic;
    assert_eq!(
        invalid.validated().unwrap_err(),
        LiveTranslationConfigurationError::UnsupportedSourceLanguage
    );
}

#[test]
fn apple_configuration_has_no_speech_secret_no_auto_and_only_text_network() {
    use crate::core::services::network_proxy::ProxyMode;
    let mut configuration = LiveTranslationConfiguration::with_credentials(
        ProviderKind::AppleSpeech,
        ProviderCredentials::AppleSpeech,
        SourceLanguage::English,
        TargetLanguage::Original,
        TranslationMode::Turbo,
    );
    configuration.network_proxy = ProxyConfig {
        mode: ProxyMode::Custom,
        url: Some("invalid".into()),
    };
    configuration.text_network_proxy = configuration.network_proxy.clone();
    let original = configuration.validated().unwrap();
    assert_eq!(original.credentials.direct_api_key(), None);
    assert_eq!(original.network_proxy.mode, ProxyMode::Direct);
    assert_eq!(original.text_network_proxy.mode, ProxyMode::Direct);
    assert_eq!(original.capabilities().input_sample_rate_hz, 16_000);
    configuration.source_language = SourceLanguage::Automatic;
    assert_eq!(
        configuration.validated(),
        Err(LiveTranslationConfigurationError::UnsupportedSourceLanguage)
    );
    configuration.source_language = SourceLanguage::English;
    configuration.target_language = TargetLanguage::SimplifiedChinese;
    configuration.text_network_proxy = ProxyConfig::default();
    assert_eq!(
        configuration.validated(),
        Err(LiveTranslationConfigurationError::Credentials(
            ProviderCredentialsError::MissingTextTranslation
        ))
    );
    configuration.text_credentials = Some(TextTranslationCredentials::OpenAICompatible {
        endpoint: "http://localhost:8080/v1".into(),
        model: "synthetic-model".into(),
        api_key: String::new(),
    });
    assert!(configuration.validated().is_ok());
    configuration.credentials = ProviderCredentials::api_key("synthetic-unrelated-key");
    assert_eq!(
        configuration.validated(),
        Err(LiveTranslationConfigurationError::Credentials(
            ProviderCredentialsError::ProviderMismatch
        ))
    );
}

#[test]
fn independent_routes_are_validated_immutable_and_redacted() {
    use crate::core::services::network_proxy::ProxyMode;
    let speech = ProxyConfig {
        mode: ProxyMode::Custom,
        url: Some("http://private-speech.example:7890".into()),
    };
    let text = ProxyConfig {
        mode: ProxyMode::Direct,
        url: None,
    };
    let mut configuration = config("synthetic-key", SourceLanguage::English)
        .with_stage_network_proxies(speech.clone(), text.clone());
    let resolved = configuration.validated().unwrap();
    assert_eq!(resolved.network_proxy.mode, speech.mode);
    assert_eq!(resolved.text_network_proxy, text);
    configuration.text_network_proxy = ProxyConfig {
        mode: ProxyMode::Custom,
        url: Some("http://user:private-value@localhost".into()),
    };
    assert!(configuration.validated().is_err());
    assert_eq!(resolved.text_network_proxy.mode, ProxyMode::Direct);
    assert!(!format!("{configuration:?}").contains("private-value"));
    assert!(!format!("{resolved:?}").contains("private-speech"));
}

use super::*;

#[test]
fn custom_speech_configuration_requires_mt_only_for_translated_targets() {
    for (provider, rate) in [
        (ProviderKind::CustomDashScopeASR, 16_000),
        (ProviderKind::CustomOpenAIASR, 24_000),
    ] {
        let mut configuration = LiveTranslationConfiguration::with_credentials(
            provider,
            ProviderCredentials::CustomSpeech {
                endpoint: "wss://speech.example/recognition".into(),
                model: "synthetic-speech-model".into(),
                api_key: "synthetic-speech-key".into(),
            },
            SourceLanguage::Automatic,
            TargetLanguage::Original,
            TranslationMode::Turbo,
        );
        assert_eq!(
            configuration
                .validated()
                .unwrap()
                .capabilities()
                .input_sample_rate_hz,
            rate
        );
        configuration.target_language = TargetLanguage::English;
        assert_eq!(
            configuration.validated(),
            Err(LiveTranslationConfigurationError::Credentials(
                ProviderCredentialsError::MissingTextTranslation
            ))
        );
        configuration.text_credentials = Some(TextTranslationCredentials::DeepL {
            api_key: "synthetic-mt-key".into(),
        });
        let validated = configuration.validated().unwrap();
        assert_eq!(
            validated.credentials.direct_api_key(),
            Some("synthetic-speech-key")
        );
        assert!(
            matches!(validated.text_credentials, Some(TextTranslationCredentials::DeepL { api_key }) if api_key == "synthetic-mt-key")
        );
        configuration.target_language = TargetLanguage::Original;
        configuration.text_credentials =
            Some(TextTranslationCredentials::DeepL { api_key: "".into() });
        assert_eq!(configuration.validated().unwrap().text_credentials, None);
        assert!(!format!("{configuration:?}").contains("synthetic-speech"));
        configuration.source_language = SourceLanguage::Khmer;
        assert!(configuration.validated().is_ok());
        configuration.target_language = TargetLanguage::English;
        configuration.text_credentials = Some(TextTranslationCredentials::DeepL {
            api_key: "synthetic-mt-key".into(),
        });
        assert_eq!(
            configuration.validated(),
            Err(LiveTranslationConfigurationError::UnsupportedSourceLanguage)
        );
    }
}

#[test]
fn default_alibaba_validates_every_lite_target_and_the_full_original_catalog() {
    use crate::core::protocols::{recognition::audio3, translation::qwen_mt};
    for source in SourceLanguage::ALL {
        let mut configuration = config("synthetic", source);
        configuration.target_language = TargetLanguage::Original;
        let is_audio3_source = source == SourceLanguage::Automatic
            || audio3::LANGUAGE_CODES.contains(&source.raw_value());
        assert_eq!(configuration.validated().is_ok(), is_audio3_source);
        for target in TargetLanguage::ALL
            .into_iter()
            .filter(|target| target.translates_audio())
        {
            configuration.target_language = target;
            let supported = is_audio3_source
                && (source == SourceLanguage::Automatic
                    || qwen_mt::QWEN_MT_LITE_LANGUAGE_CODES.contains(&source.raw_value()))
                && qwen_mt::QWEN_MT_LITE_LANGUAGE_CODES.contains(&target.raw_value());
            assert_eq!(
                configuration.validated().is_ok(),
                supported,
                "source={source:?} target={target:?}"
            );
        }
    }
}

#[test]
fn independent_text_credentials_keep_their_language_contract() {
    let mut configuration = LiveTranslationConfiguration::with_credentials(
        ProviderKind::AlibabaCloud,
        ProviderCredentials::DeepL {
            asr_api_key: "synthetic".into(),
            api_key: "synthetic:fx".into(),
        },
        SourceLanguage::Automatic,
        TargetLanguage::French,
        TranslationMode::Turbo,
    );
    assert!(configuration.validated().is_ok());
    configuration.target_language = TargetLanguage::Khmer;
    assert_eq!(
        configuration.validated().unwrap_err(),
        LiveTranslationConfigurationError::UnsupportedTargetLanguage
    );
    configuration.target_language = TargetLanguage::English;
    configuration.source_language = SourceLanguage::Khmer;
    assert_eq!(
        configuration.validated().unwrap_err(),
        LiveTranslationConfigurationError::UnsupportedSourceLanguage
    );
    configuration.provider = ProviderKind::DeepLX;
    configuration.credentials = ProviderCredentials::DeepLX {
        asr_api_key: "synthetic".into(),
        endpoint: "https://example.com/translate".into(),
        token: String::new(),
    };
    configuration.source_language = SourceLanguage::Automatic;
    configuration.target_language = TargetLanguage::French;
    assert!(configuration.validated().is_ok());
    configuration.target_language = TargetLanguage::Khmer;
    assert_eq!(
        configuration.validated().unwrap_err(),
        LiveTranslationConfigurationError::UnsupportedTargetLanguage
    );
}

fn config(api_key: &str, source_language: SourceLanguage) -> LiveTranslationConfiguration {
    LiveTranslationConfiguration::for_provider(
        ProviderKind::AlibabaCloud,
        api_key,
        source_language,
        TargetLanguage::SimplifiedChinese,
        TranslationMode::HighQuality,
    )
}

#[test]
fn automatic_language_upgrades_legacy_high_quality_to_turbo() {
    let configuration = config("sk-test", SourceLanguage::Automatic);
    assert_eq!(
        configuration.effective_translation_mode(),
        TranslationMode::Turbo
    );
}

#[test]
fn turbo_mode_stays_turbo_even_with_automatic_source() {
    let configuration = LiveTranslationConfiguration::for_provider(
        ProviderKind::AlibabaCloud,
        "sk-test",
        SourceLanguage::Automatic,
        TargetLanguage::SimplifiedChinese,
        TranslationMode::Turbo,
    );
    assert_eq!(
        configuration.effective_translation_mode(),
        TranslationMode::Turbo
    );
}

#[test]
fn original_subtitles_preserve_the_strongest_recognition_backend() {
    let configuration = LiveTranslationConfiguration::for_provider(
        ProviderKind::AlibabaCloud,
        "secret",
        SourceLanguage::Japanese,
        TargetLanguage::Original,
        TranslationMode::HighQuality,
    );
    assert_eq!(
        configuration.effective_translation_mode(),
        TranslationMode::Turbo
    );
}

#[test]
fn configuration_normalizes_legacy_modes_without_changing_languages() {
    for mode in [
        TranslationMode::LowLatency,
        TranslationMode::HighQuality,
        TranslationMode::Turbo,
    ] {
        let configuration = LiveTranslationConfiguration::for_provider(
            ProviderKind::AlibabaCloud,
            "sk-test",
            SourceLanguage::Japanese,
            TargetLanguage::English,
            mode,
        );
        let validated = configuration.validated().unwrap();
        assert_eq!(validated.translation_mode, TranslationMode::Turbo);
        assert_eq!(validated.target_language, TargetLanguage::English);
        assert_eq!(validated.source_language, SourceLanguage::Japanese);
    }
}

#[test]
fn configuration_requires_an_api_key() {
    let configuration = config("   ", SourceLanguage::English);
    assert!(matches!(
        configuration.validated(),
        Err(LiveTranslationConfigurationError::Credentials(
            ProviderCredentialsError::Missing(ProviderKind::AlibabaCloud)
        ))
    ));
}

#[test]
fn configuration_requires_only_the_api_key() {
    // The unified DashScope endpoints authenticate with the API key only.
    let configuration = config("sk-test", SourceLanguage::English);
    let validated = configuration.validated().unwrap();
    assert_eq!(validated.credentials.direct_api_key(), Some("sk-test"));
}

#[test]
fn configuration_trims_valid_credentials() {
    let configuration = config("  sk-test  ", SourceLanguage::Korean);
    let validated = configuration.validated().unwrap();
    assert_eq!(validated.credentials.direct_api_key(), Some("sk-test"));
    assert_eq!(validated.source_language, SourceLanguage::Korean);
}

#[test]
fn openai_configuration_accepts_only_its_capability_matrix() {
    let valid = LiveTranslationConfiguration::for_provider(
        ProviderKind::OpenAIRealtime,
        "sk-openai",
        SourceLanguage::Automatic,
        TargetLanguage::Japanese,
        TranslationMode::Turbo,
    );
    assert_eq!(
        valid.validated().unwrap().provider,
        ProviderKind::OpenAIRealtime
    );

    let invalid_source = LiveTranslationConfiguration::for_provider(
        ProviderKind::OpenAIRealtime,
        "sk-openai",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        TranslationMode::Turbo,
    );
    assert!(matches!(
        invalid_source.validated(),
        Err(LiveTranslationConfigurationError::UnsupportedSourceLanguage)
    ));
    let invalid_target = LiveTranslationConfiguration::for_provider(
        ProviderKind::OpenAIRealtime,
        "sk-openai",
        SourceLanguage::Automatic,
        TargetLanguage::Original,
        TranslationMode::Turbo,
    );
    assert!(matches!(
        invalid_target.validated(),
        Err(LiveTranslationConfigurationError::UnsupportedTargetLanguage)
    ));
}

#[test]
fn provider_capabilities_define_the_session_sample_rate() {
    let alibaba = config("sk-test", SourceLanguage::Automatic);
    assert_eq!(alibaba.provider.capabilities().input_sample_rate_hz, 16_000);
    let openai = LiveTranslationConfiguration::for_provider(
        ProviderKind::OpenAIRealtime,
        "sk-openai",
        SourceLanguage::Automatic,
        TargetLanguage::English,
        TranslationMode::Turbo,
    );
    assert_eq!(openai.provider.capabilities().input_sample_rate_hz, 24_000);
}

#[test]
fn structured_provider_credentials_are_validated_without_disclosure() {
    let secret = "azure-private-value";
    let configuration = LiveTranslationConfiguration::with_credentials(
        ProviderKind::AzureOpenAIRealtime,
        ProviderCredentials::AzureOpenAI {
            endpoint: "https://mimi.openai.azure.com".into(),
            deployment: "translate".into(),
            transcription_deployment: "transcribe".into(),
            api_key: secret.into(),
        },
        SourceLanguage::Automatic,
        TargetLanguage::English,
        TranslationMode::Turbo,
    )
    .validated()
    .unwrap();
    assert_eq!(configuration.provider, ProviderKind::AzureOpenAIRealtime);
    assert!(!format!("{configuration:?}").contains(secret));
}

#[test]
fn debug_description_redacts_the_api_key() {
    let secret = "sk-private-test-value";
    let configuration = config(secret, SourceLanguage::English);
    let description = format!("{configuration:?}");
    assert!(!description.contains(secret));
    assert!(description.contains("[REDACTED]"));
}

#[test]
fn validated_proxy_is_an_immutable_copy_and_debug_does_not_disclose_its_address() {
    use crate::core::services::network_proxy::ProxyMode;
    let mut original =
        config("synthetic-key", SourceLanguage::English).with_network_proxy(ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some("http://private-proxy.example:8888".into()),
        });
    let validated = original.validated().unwrap();
    original.network_proxy = ProxyConfig {
        mode: ProxyMode::Direct,
        url: None,
    };
    assert_eq!(validated.network_proxy.mode, ProxyMode::Custom);
    assert_eq!(original.network_proxy.mode, ProxyMode::Direct);
    assert!(!format!("{validated:?}").contains("private-proxy.example"));
}
