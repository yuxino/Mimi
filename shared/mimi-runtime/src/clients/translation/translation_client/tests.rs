use super::*;
use crate::clients::transport::provider_events::provider_event_channel;
use crate::core::models::{SourceLanguage, TargetLanguage};

#[test]
fn independent_recognition_setup_rejections_do_not_become_transport_retries() {
    use crate::core::lifecycle::recovery::provider_error_is_retryable;
    for label in [
        "gemini_transcribe_protocol_invalid",
        "gemini_transcribe_session_rejected",
        "xai_stt_protocol_invalid",
        "xai_stt_session_rejected",
        "custom_speech_protocol_invalid",
        "custom_speech_session_rejected",
    ] {
        let error = ConnectError::MT(QwenMTClientError::RequestFailed {
            status_code: 0,
            message: label.into(),
        });
        assert_eq!(error.safe_code(), "provider_error", "{label}");
        assert!(!provider_error_is_retryable(error.safe_code()), "{label}");
        let untrusted = ConnectError::MT(QwenMTClientError::RequestFailed {
            status_code: 0,
            message: format!("{label}: synthetic-private-detail"),
        });
        assert_eq!(untrusted.safe_code(), "transport_error");
    }
}

#[test]
fn independent_recognition_authentication_and_transport_categories_stay_stable() {
    use crate::core::lifecycle::recovery::provider_error_is_retryable;
    for provider in ["gemini_transcribe", "xai_stt", "custom_speech"] {
        for (status_code, suffix, code, retryable) in [
            (
                401,
                "authentication_failed",
                "credential_authentication_failed",
                false,
            ),
            (503, "unreachable", "transport_error", true),
            (408, "setup_timeout", "transport_error", true),
        ] {
            let message = format!("{provider}_{suffix}");
            let error = ConnectError::MT(QwenMTClientError::RequestFailed {
                status_code,
                message: message.clone(),
            });
            assert_eq!(error.safe_code(), code, "{message}");
            assert_eq!(
                provider_error_is_retryable(error.safe_code()),
                retryable,
                "{message}"
            );
        }
    }
}

#[test]
fn built_in_setup_rejections_do_not_become_transport_retries() {
    use crate::core::lifecycle::recovery::provider_error_is_retryable;
    for (error, label, code) in [
        (
            ConnectError::OpenAI(OpenAIRealtimeClientError::SessionSetupRejected),
            "openai_realtime_provider_error",
            "provider_error",
        ),
        (
            ConnectError::Xai(XAIRealtimeClientError::SessionSetupRejected),
            "xai_realtime_provider_error",
            "provider_error",
        ),
        (
            ConnectError::Gemini(GeminiLiveClientError::SessionSetupRejected),
            "gemini_live_provider_error",
            "provider_error",
        ),
        (
            ConnectError::OpenAI(OpenAIRealtimeClientError::AuthenticationFailed),
            "credential_authentication_failed",
            "credential_authentication_failed",
        ),
        (
            ConnectError::Xai(XAIRealtimeClientError::AuthenticationFailed),
            "credential_authentication_failed",
            "credential_authentication_failed",
        ),
        (
            ConnectError::Gemini(GeminiLiveClientError::AuthenticationFailed),
            "credential_authentication_failed",
            "credential_authentication_failed",
        ),
        (
            ConnectError::MT(QwenMTClientError::RequestFailed {
                status_code: 0,
                message: "audio3_error.setup.task_failed.OTHER".into(),
            }),
            "audio3_provider_error",
            "provider_error",
        ),
    ] {
        assert_eq!(error.recognition_failure_label(), Some(label));
        assert_eq!(error.safe_code(), code);
        assert!(!provider_error_is_retryable(code));
    }
    let temporary = ConnectError::Gemini(GeminiLiveClientError::ServiceUnavailable);
    assert_eq!(temporary.safe_code(), "gemini_live_service_unavailable");
    assert!(provider_error_is_retryable(temporary.safe_code()));
    let untrusted = ConnectError::MT(QwenMTClientError::RequestFailed {
        status_code: 0,
        message: "audio3_error.setup.task_failed.OTHER: synthetic-private-detail".into(),
    });
    assert_eq!(untrusted.recognition_failure_label(), None);
}

#[test]
fn baidu_setup_bridge_stops_explicit_rejections_but_preserves_transport_and_auth() {
    for error in [
        BaiduTranslateClientError::SessionSetupRejected,
        BaiduTranslateClientError::SessionSetupRejectedWithCode(20_316),
        BaiduTranslateClientError::SessionSetupRejectedWithCode(99_999),
        BaiduTranslateClientError::SessionSetupRejectedWithCode(i64::MIN),
    ] {
        let error = ConnectError::BaiduTranslate(error);
        assert_eq!(
            error.recognition_failure_label(),
            Some("baidu_realtime_provider_error")
        );
        assert_eq!(error.safe_code(), "provider_error");
        assert!(!crate::core::lifecycle::recovery::provider_error_is_retryable(error.safe_code()));
    }
    for error in [
        BaiduTranslateClientError::SessionSetupRejectedWithCode(20_314),
        BaiduTranslateClientError::TransportFailure,
    ] {
        let error = ConnectError::BaiduTranslate(error);
        assert_eq!(error.safe_code(), "transport_error");
        assert!(crate::core::lifecycle::recovery::provider_error_is_retryable(error.safe_code()));
    }
    let auth = ConnectError::BaiduTranslate(BaiduTranslateClientError::AuthenticationFailed);
    assert_eq!(
        auth.recognition_failure_label(),
        Some("credential_authentication_failed")
    );
    assert_eq!(auth.safe_code(), "credential_authentication_failed");
}

#[test]
fn apple_recognition_failure_labels_preserve_only_exact_safe_codes() {
    for label in [
        "apple_speech_runtime_service_unavailable",
        "apple_speech_runtime_timeout",
        "apple_speech_stream_closed",
        "apple_speech_result_backlog",
        "apple_speech_recognition_failed",
        "apple_speech_invalid_result",
        "apple_speech_audio_failed",
        "apple_speech_not_connected",
        "apple_speech_unavailable",
        "apple_speech_assets_missing",
        "apple_speech_preparing",
        "apple_speech_language_unsupported",
        "apple_speech_reservation_limit",
        "apple_speech_resources_unavailable",
        "apple_speech_status_failed",
    ] {
        let error = ConnectError::MT(QwenMTClientError::RequestFailed {
            status_code: 0,
            message: label.into(),
        });
        assert_eq!(error.recognition_failure_label(), Some(label));
    }
    for message in [
        "apple_speech_private/context",
        "apple_speech_runtime_timeout: private/context",
        "vendor content",
        "transport_error",
    ] {
        let error = ConnectError::MT(QwenMTClientError::RequestFailed {
            status_code: 0,
            message: message.into(),
        });
        assert_eq!(error.recognition_failure_label(), None);
    }
}

#[test]
fn apple_text_override_selects_bounded_pipeline_without_text_network_credentials() {
    for provider in [
        ProviderKind::AlibabaCloud,
        ProviderKind::CustomOpenAIASR,
        ProviderKind::AppleSpeech,
    ] {
        let credentials = match provider {
            ProviderKind::AppleSpeech => ProviderCredentials::AppleSpeech,
            ProviderKind::CustomOpenAIASR => ProviderCredentials::CustomSpeech {
                endpoint: "wss://speech.example/realtime".into(),
                model: "synthetic".into(),
                api_key: "synthetic-speech".into(),
            },
            _ => ProviderCredentials::api_key("synthetic-speech"),
        };
        for target in [TargetLanguage::Japanese, TargetLanguage::Original] {
            let configuration = LiveTranslationConfiguration::with_credentials(
                provider,
                credentials.clone(),
                SourceLanguage::English,
                target,
                TranslationMode::Turbo,
            )
            .with_text_credentials(TextTranslationCredentials::Apple);
            let (events, _receiver) = provider_event_channel();
            assert!(matches!(
                TranslationClient::new(&configuration, events).unwrap(),
                TranslationClient::HighQuality(_)
            ));
        }
    }
}

#[test]
fn invalid_proxy_is_rejected_before_provider_client_construction() {
    let configuration = LiveTranslationConfiguration::for_provider(
        ProviderKind::AlibabaCloud,
        "synthetic-key",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        TranslationMode::Turbo,
    )
    .with_network_proxy(crate::core::services::network_proxy::ProxyConfig {
        mode: crate::core::services::network_proxy::ProxyMode::Custom,
        url: Some("http://synthetic-user:synthetic-password@127.0.0.1:8888".into()),
    });
    let (events, _receiver) = provider_event_channel();
    let error = match TranslationClient::new(&configuration, events) {
        Err(error) => error,
        Ok(_) => panic!("authenticated proxy URLs must be rejected"),
    };
    assert_eq!(error.diagnostic_label(), "configuration.network_proxy");
    assert!(!error.to_string().contains("synthetic-password"));
}

#[test]
fn deeplx_factory_reuses_audio3_pipeline_with_separate_credentials() {
    let configuration = LiveTranslationConfiguration::with_credentials(
        ProviderKind::DeepLX,
        ProviderCredentials::DeepLX {
            asr_api_key: "synthetic-asr".into(),
            endpoint: "https://example.com/translate".into(),
            token: "synthetic-token".into(),
        },
        SourceLanguage::Automatic,
        TargetLanguage::English,
        TranslationMode::Turbo,
    );
    let (events, _receiver) = provider_event_channel();
    assert!(matches!(
        TranslationClient::new(&configuration, events).unwrap(),
        TranslationClient::HighQuality(_)
    ));
}

#[test]
fn custom_recognition_factory_requires_an_independent_translation_route() {
    use crate::core::services::credentials::TextTranslationCredentials;
    for provider in [
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let mut configuration = LiveTranslationConfiguration::with_credentials(
            provider,
            ProviderCredentials::CustomSpeech {
                endpoint: "wss://example.com/recognition".into(),
                model: "synthetic-recognition-model".into(),
                api_key: "synthetic-recognition-key".into(),
            },
            SourceLanguage::Automatic,
            TargetLanguage::Original,
            TranslationMode::Turbo,
        )
        .validated()
        .unwrap();
        let (events, _receiver) = provider_event_channel();
        assert!(matches!(
            TranslationClient::new(&configuration, events).unwrap(),
            TranslationClient::HighQuality(_)
        ));
        configuration.target_language = TargetLanguage::Japanese;
        let (events, _receiver) = provider_event_channel();
        assert!(matches!(
            TranslationClient::new(&configuration, events),
            Err(TranslationClientError::MT(
                QwenMTClientError::MissingTextTranslation
            ))
        ));
        configuration.text_credentials = Some(TextTranslationCredentials::DeepLX {
            endpoint: "https://example.com/translate".into(),
            token: "synthetic-text-token".into(),
        });
        configuration = configuration.validated().unwrap();
        let (events, _receiver) = provider_event_channel();
        assert!(matches!(
            TranslationClient::new(&configuration, events).unwrap(),
            TranslationClient::HighQuality(_)
        ));
    }
}

#[test]
fn provider_factory_selects_openai_realtime() {
    let configuration = LiveTranslationConfiguration::for_provider(
        ProviderKind::OpenAIRealtime,
        "sk-test-not-real",
        SourceLanguage::Automatic,
        TargetLanguage::English,
        TranslationMode::Turbo,
    );
    let (events, _receiver) = provider_event_channel();
    assert!(matches!(
        TranslationClient::new(&configuration, events).unwrap(),
        TranslationClient::OpenAIRealtime(_)
    ));
}

#[test]
fn deepl_text_override_uses_the_bounded_pipeline_and_preserves_original_mode() {
    for target in [TargetLanguage::SimplifiedChinese, TargetLanguage::Original] {
        let configuration = LiveTranslationConfiguration::with_credentials(
            ProviderKind::AlibabaCloud,
            ProviderCredentials::DeepL {
                asr_api_key: "synthetic-asr".into(),
                api_key: "synthetic-deepl:fx".into(),
            },
            SourceLanguage::Automatic,
            target,
            TranslationMode::Turbo,
        )
        .validated()
        .unwrap();
        let (events, _receiver) = provider_event_channel();
        assert!(matches!(
            TranslationClient::new(&configuration, events).unwrap(),
            TranslationClient::HighQuality(_)
        ));
    }
}

#[test]
fn openai_compatible_override_uses_audio3_and_preserves_original_mode() {
    assert_compatible_factory(false);
    assert_compatible_factory(true);
}

fn assert_compatible_factory(chatmock: bool) {
    for target in [
        TargetLanguage::Japanese,
        TargetLanguage::Original,
        TargetLanguage::German,
        TargetLanguage::TraditionalChinese,
    ] {
        let configuration = LiveTranslationConfiguration::with_credentials(
            ProviderKind::AlibabaCloud,
            if chatmock {
                ProviderCredentials::ChatMock {
                    asr_api_key: "synthetic-asr".into(),
                    endpoint: "https://example.com/proxy/v1".into(),
                    api_key: "synthetic-translation-key".into(),
                    model: "synthetic-model".into(),
                }
            } else {
                ProviderCredentials::OpenAICompatible {
                    asr_api_key: "synthetic-asr".into(),
                    endpoint: "https://example.com/proxy/v1".into(),
                    api_key: "synthetic-translation-key".into(),
                    model: "synthetic-model".into(),
                }
            },
            SourceLanguage::Automatic,
            target,
            TranslationMode::Turbo,
        )
        .validated()
        .unwrap();
        assert_eq!(
            configuration
                .capabilities()
                .source_languages
                .iter()
                .map(|source| source.raw_value())
                .collect::<Vec<_>>(),
            std::iter::once("auto")
                .chain(
                    crate::core::protocols::recognition::audio3::LANGUAGE_CODES
                        .iter()
                        .copied()
                )
                .collect::<Vec<_>>()
        );
        assert_eq!(
            configuration.capabilities().target_languages,
            TargetLanguage::ALL
        );
        let (events, _receiver) = provider_event_channel();
        assert!(matches!(
            TranslationClient::new(&configuration, events).unwrap(),
            TranslationClient::HighQuality(_)
        ));
        let mut french = configuration;
        french.source_language = SourceLanguage::French;
        let french = french.validated().unwrap();
        let (events, _receiver) = provider_event_channel();
        assert!(matches!(
            TranslationClient::new(&french, events).unwrap(),
            TranslationClient::HighQuality(_)
        ));
    }
}

#[test]
fn provider_factory_upgrades_every_alibaba_mode_to_the_turbo_pipeline() {
    for mode in [
        TranslationMode::LowLatency,
        TranslationMode::HighQuality,
        TranslationMode::Turbo,
    ] {
        let configuration = LiveTranslationConfiguration::for_provider(
            ProviderKind::AlibabaCloud,
            "sk-test-not-real",
            SourceLanguage::Automatic,
            TargetLanguage::SimplifiedChinese,
            mode,
        );
        let (events, _receiver) = provider_event_channel();
        assert!(matches!(
            TranslationClient::new(&configuration, events).unwrap(),
            TranslationClient::HighQuality(_)
        ));
    }
}

#[test]
fn explicitly_selected_alibaba_realtime_routes_use_the_shared_client() {
    let mode = AlibabaRealtimeMode::LiveTranslate;
    let configuration = LiveTranslationConfiguration::for_provider(
        ProviderKind::AlibabaCloud,
        "synthetic-key",
        SourceLanguage::Automatic,
        TargetLanguage::SimplifiedChinese,
        TranslationMode::Turbo,
    )
    .with_alibaba_realtime(mode, "synthetic-workspace")
    .validated()
    .unwrap();
    let (events, _receiver) = provider_event_channel();
    let TranslationClient::LowLatency(client) =
        TranslationClient::new_without_network(&configuration, events).unwrap()
    else {
        panic!("explicit realtime mode must select the portable realtime client");
    };
    let endpoint = client.network_endpoint();
    assert_eq!(
        endpoint.host_str(),
        Some("synthetic-workspace.cn-beijing.maas.aliyuncs.com")
    );
    assert_eq!(
        endpoint
            .query_pairs()
            .find(|(key, _)| key == "model")
            .unwrap()
            .1,
        mode.model_name().unwrap()
    );
}

#[test]
fn alibaba_model_selection_preserves_model_and_stream_behavior_in_factory() {
    for model in [QwenMTModel::Lite, QwenMTModel::Flash, QwenMTModel::Plus] {
        let configuration = LiveTranslationConfiguration::for_provider(
            ProviderKind::AlibabaCloud,
            "synthetic-key",
            SourceLanguage::English,
            TargetLanguage::SimplifiedChinese,
            TranslationMode::Turbo,
        )
        .with_qwen_mt_model(model)
        .validated()
        .unwrap();
        let (events, _receiver) = provider_event_channel();
        let TranslationClient::HighQuality(client) =
            TranslationClient::new(&configuration, events).unwrap()
        else {
            panic!("expected recognition and text translation pipeline");
        };
        assert_eq!(
            client.qwen_model_for_test(),
            Some((model, model != QwenMTModel::Plus))
        );
    }
}

#[test]
fn provider_factory_selects_google_and_xai_realtime_adapters() {
    for (provider, expected) in [
        (ProviderKind::GoogleGeminiLive, "gemini"),
        (ProviderKind::XAIRealtime, "xai"),
    ] {
        let configuration = LiveTranslationConfiguration::for_provider(
            provider,
            "sk-test-not-real",
            SourceLanguage::Automatic,
            TargetLanguage::English,
            TranslationMode::Turbo,
        );
        let (events, _receiver) = provider_event_channel();
        let client = TranslationClient::new(&configuration, events).unwrap();
        assert!(matches!(
            (expected, client),
            ("gemini", TranslationClient::GeminiLive(_))
                | ("xai", TranslationClient::XaiRealtime(_))
        ));
    }
}

#[test]
fn provider_factory_selects_azure_with_structured_credentials() {
    let configuration = LiveTranslationConfiguration::with_credentials(
        ProviderKind::AzureOpenAIRealtime,
        ProviderCredentials::AzureOpenAI {
            endpoint: "https://mimi.openai.azure.com".into(),
            deployment: "translate".into(),
            transcription_deployment: "transcribe".into(),
            api_key: "azure-test-not-real".into(),
        },
        SourceLanguage::Automatic,
        TargetLanguage::Japanese,
        TranslationMode::Turbo,
    );
    let (events, _receiver) = provider_event_channel();
    assert!(matches!(
        TranslationClient::new(&configuration, events).unwrap(),
        TranslationClient::AzureOpenAIRealtime(_)
    ));
}

#[test]
fn provider_factory_selects_volcano_engine() {
    let configuration = LiveTranslationConfiguration::for_provider(
        ProviderKind::VolcanoEngine,
        "volcano-test-not-real",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        TranslationMode::Turbo,
    );
    let (events, _receiver) = provider_event_channel();
    assert!(matches!(
        TranslationClient::new(&configuration, events).unwrap(),
        TranslationClient::VolcanoEngine(_)
    ));
}

#[test]
fn provider_factory_selects_tencent_with_structured_credentials() {
    let configuration = LiveTranslationConfiguration::with_credentials(
        ProviderKind::TencentCloud,
        ProviderCredentials::TencentCloud {
            app_id: "1250000000".into(),
            secret_id: "AKIDtest".into(),
            secret_key: "tencent-test-not-real".into(),
        },
        SourceLanguage::Japanese,
        TargetLanguage::SimplifiedChinese,
        TranslationMode::Turbo,
    );
    let (events, _receiver) = provider_event_channel();
    assert!(matches!(
        TranslationClient::new(&configuration, events).unwrap(),
        TranslationClient::TencentCloud(_)
    ));
}

#[test]
fn provider_factory_selects_baidu_with_structured_credentials() {
    let configuration = LiveTranslationConfiguration::with_credentials(
        ProviderKind::BaiduTranslate,
        ProviderCredentials::BaiduTranslate {
            app_id: "baidu-app-id".into(),
            app_key: "baidu-test-not-real".into(),
        },
        SourceLanguage::English,
        TargetLanguage::Japanese,
        TranslationMode::Turbo,
    );
    let (events, _receiver) = provider_event_channel();
    assert!(matches!(
        TranslationClient::new(&configuration, events).unwrap(),
        TranslationClient::BaiduTranslate(_)
    ));
}
