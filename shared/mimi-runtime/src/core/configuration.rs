//! Immutable, provider-resolved live-translation configuration.

use crate::core::credentials::{
    ProviderCredentials, ProviderCredentialsError, TextTranslationCredentials,
};
use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode};
use crate::core::network_proxy::{ProxyConfig, ProxyConfigError};
use crate::core::protocols::qwen_mt::QwenMTModel;
use crate::core::provider::{ProviderCapabilities, ProviderKind, TextTranslation};
use std::fmt;
use thiserror::Error;

/// A text-only readiness request never needs a speech key or opens an audio session.
#[derive(Clone)]
pub enum TextTranslationProbeCredentials {
    Qwen { api_key: String },
    Independent(TextTranslationCredentials),
}

#[derive(Clone)]
pub struct TextTranslationProbeConfiguration {
    pub credentials: TextTranslationProbeCredentials,
    pub qwen_mt_model: QwenMTModel,
    pub source_language: SourceLanguage,
    pub target_language: TargetLanguage,
    pub network_proxy: ProxyConfig,
}

impl fmt::Debug for TextTranslationProbeConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextTranslationProbeConfiguration")
            .field("credentials", &"[REDACTED]")
            .field("target_language", &self.target_language)
            .field("network_proxy", &self.network_proxy)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LiveTranslationConfigurationError {
    #[error("{0}")]
    Credentials(#[from] ProviderCredentialsError),
    #[error("The selected service does not support this source language.")]
    UnsupportedSourceLanguage,
    #[error("The selected service does not support this target language.")]
    UnsupportedTargetLanguage,
    #[error("The selected service does not support this translation mode.")]
    UnsupportedTranslationMode,
    #[error("{0}")]
    NetworkProxy(#[from] ProxyConfigError),
}

#[derive(Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveTranslationConfiguration {
    pub provider: ProviderKind,
    pub qwen_mt_model: QwenMTModel,
    pub credentials: ProviderCredentials,
    pub text_credentials: Option<TextTranslationCredentials>,
    pub source_language: SourceLanguage,
    pub target_language: TargetLanguage,
    pub translation_mode: TranslationMode,
    /// Recognition (or the shared integrated realtime connection).
    pub network_proxy: ProxyConfig,
    pub text_network_proxy: ProxyConfig,
}

impl fmt::Debug for LiveTranslationConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveTranslationConfiguration")
            .field("provider", &self.provider)
            .field("qwen_mt_model", &self.qwen_mt_model)
            .field("credentials", &"[REDACTED]")
            .field("text_credentials", &"[REDACTED]")
            .field("source_language", &self.source_language)
            .field("target_language", &self.target_language)
            .field("text_network_proxy", &self.text_network_proxy)
            .field("translation_mode", &self.translation_mode)
            .field("network_proxy", &self.network_proxy)
            .finish()
    }
}

impl LiveTranslationConfiguration {
    /// Resolve the actual factory's endpoints before platform proxy selection.
    /// Constructors do not connect. Credentials in signed/query URLs are removed
    /// from this adapter-facing route snapshot; provider requests keep originals.
    pub fn network_endpoints(
        &self,
    ) -> Result<(String, Option<String>), crate::clients::translation_client::TranslationClientError>
    {
        let (events, _) = crate::clients::provider_events::provider_event_channel();
        let client = crate::clients::translation_client::TranslationClient::new_without_network(
            self, events,
        )?;
        let (speech, text) = client.network_endpoints()?;
        let route = |mut endpoint: url::Url| {
            endpoint.set_query(None);
            endpoint.set_fragment(None);
            endpoint.into()
        };
        let text = self
            .target_language
            .translates_audio()
            .then_some(text)
            .flatten();
        Ok((speech.map(&route).unwrap_or_default(), text.map(route)))
    }

    pub fn for_provider(
        provider: ProviderKind,
        api_key: impl Into<String>,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        translation_mode: TranslationMode,
    ) -> Self {
        Self {
            provider,
            qwen_mt_model: Default::default(),
            credentials: ProviderCredentials::api_key(api_key),
            text_credentials: None,
            source_language,
            target_language,
            translation_mode,
            network_proxy: ProxyConfig::default(),
            text_network_proxy: ProxyConfig::default(),
        }
    }

    pub fn with_credentials(
        provider: ProviderKind,
        credentials: ProviderCredentials,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        translation_mode: TranslationMode,
    ) -> Self {
        Self {
            provider,
            qwen_mt_model: Default::default(),
            credentials,
            text_credentials: None,
            source_language,
            target_language,
            translation_mode,
            network_proxy: ProxyConfig::default(),
            text_network_proxy: ProxyConfig::default(),
        }
    }

    pub fn with_network_proxy(mut self, network_proxy: ProxyConfig) -> Self {
        self.text_network_proxy = network_proxy.clone();
        self.network_proxy = network_proxy;
        self
    }

    pub fn with_stage_network_proxies(mut self, speech: ProxyConfig, text: ProxyConfig) -> Self {
        self.network_proxy = speech;
        self.text_network_proxy = text;
        self
    }

    pub fn with_qwen_mt_model(mut self, model: QwenMTModel) -> Self {
        self.qwen_mt_model = model;
        self
    }

    pub fn with_text_credentials(mut self, credentials: TextTranslationCredentials) -> Self {
        self.text_credentials = Some(credentials);
        self
    }

    /// Legacy mode values remain readable, but every new session uses Turbo.
    /// Provider-specific transports and independent text destinations remain
    /// resolved by the provider facade.
    pub fn effective_translation_mode(&self) -> TranslationMode {
        TranslationMode::Turbo
    }

    pub fn capabilities(&self) -> ProviderCapabilities {
        let route = self.text_credentials.as_ref().map_or_else(
            || match self.credentials {
                ProviderCredentials::DeepL { .. } => TextTranslation::DeepL,
                ProviderCredentials::DeepLX { .. } => TextTranslation::DeepLX,
                ProviderCredentials::OpenAICompatible { .. } => TextTranslation::OpenAICompatible,
                ProviderCredentials::ChatMock { .. } => TextTranslation::ChatMock,
                _ => TextTranslation::FollowService,
            },
            TextTranslationCredentials::translation,
        );
        self.provider
            .capabilities_for_route(route, self.target_language)
            .for_source(self.source_language)
    }

    /// Returns a trimmed, validated copy of the configuration.
    pub fn validated(&self) -> Result<Self, LiveTranslationConfigurationError> {
        let network_proxy = if self.provider == ProviderKind::AppleSpeech {
            ProxyConfig {
                mode: crate::core::network_proxy::ProxyMode::Direct,
                url: None,
            }
        } else {
            self.network_proxy.validate()?
        };
        let text_network_proxy = if matches!(
            self.text_credentials,
            Some(TextTranslationCredentials::Apple)
        ) || (self.provider == ProviderKind::AppleSpeech
            && !self.target_language.translates_audio())
        {
            ProxyConfig {
                mode: crate::core::network_proxy::ProxyMode::Direct,
                url: None,
            }
        } else {
            self.text_network_proxy.validate()?
        };
        let credentials = self.credentials.validated_for(self.provider)?;

        let text_credentials =
            if self.provider.is_standalone_asr() && self.target_language.translates_audio() {
                Some(
                    self.text_credentials
                        .as_ref()
                        .ok_or(ProviderCredentialsError::MissingTextTranslation)?
                        .validated()?,
                )
            } else if self.provider.is_standalone_asr() {
                None
            } else {
                self.text_credentials.clone()
            };

        let capabilities = self.capabilities();
        if !capabilities
            .source_languages
            .contains(&self.source_language)
        {
            return Err(LiveTranslationConfigurationError::UnsupportedSourceLanguage);
        }
        if !capabilities
            .target_languages
            .contains(&self.target_language)
        {
            return Err(LiveTranslationConfigurationError::UnsupportedTargetLanguage);
        }
        let translation_mode = self.effective_translation_mode();
        if !capabilities.translation_modes.contains(&translation_mode) {
            return Err(LiveTranslationConfigurationError::UnsupportedTranslationMode);
        }

        Ok(Self {
            provider: self.provider,
            qwen_mt_model: self.qwen_mt_model,
            credentials,
            text_credentials,
            source_language: self.source_language,
            target_language: self.target_language,
            translation_mode,
            network_proxy,
            text_network_proxy,
        })
    }
}

#[cfg(test)]
mod tests {
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
        use crate::core::network_proxy::ProxyMode;
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
        use crate::core::network_proxy::ProxyMode;
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
        use crate::core::network_proxy::ProxyMode;
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
        use crate::core::protocols::{audio3, qwen_mt};
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
        use crate::core::network_proxy::ProxyMode;
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
}
