//! Immutable, provider-resolved live-translation configuration.

use crate::core::models::{SourceLanguage, TargetLanguage, TranslationMode};
use crate::core::protocols::translation::qwen_mt::QwenMTModel;
use crate::core::services::credentials::{
    ProviderCredentials, ProviderCredentialsError, TextTranslationCredentials,
};
use crate::core::services::network_proxy::{ProxyConfig, ProxyConfigError};
use crate::core::services::provider::{
    alibaba_workspace_id_is_valid, AlibabaRealtimeMode, ProviderCapabilities, ProviderKind,
    SpeechTranslationMode, TextTranslation, XAISpeechMode,
};
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
    #[error("This service does not support the selected speech translation mode.")]
    UnsupportedSpeechTranslationMode,
    #[error("Invalid xAI speech mode or text model.")]
    InvalidXAISpeech,
    #[error("{0}")]
    Credentials(#[from] ProviderCredentialsError),
    #[error("The selected service does not support this source language.")]
    UnsupportedSourceLanguage,
    #[error("The selected service does not support this target language.")]
    UnsupportedTargetLanguage,
    #[error("The selected service does not support this translation mode.")]
    UnsupportedTranslationMode,
    #[error("The Alibaba realtime workspace ID is missing or invalid.")]
    InvalidAlibabaWorkspaceId,
    #[error("Experimental realtime routes require an Alibaba Cloud profile.")]
    UnsupportedAlibabaRealtime,
    #[error("{0}")]
    NetworkProxy(#[from] ProxyConfigError),
}

#[derive(Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveTranslationConfiguration {
    pub provider: ProviderKind,
    pub qwen_mt_model: QwenMTModel,
    #[serde(default)]
    pub alibaba_realtime_mode: AlibabaRealtimeMode,
    #[serde(default)]
    pub alibaba_workspace_id: String,
    #[serde(default)]
    pub xai_speech_mode: XAISpeechMode,
    #[serde(default)]
    pub speech_translation_mode: SpeechTranslationMode,
    #[serde(default = "crate::core::services::provider::default_xai_text_model")]
    pub xai_text_model: String,
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
            .field("alibaba_realtime_mode", &self.alibaba_realtime_mode)
            .field("xai_speech_mode", &self.xai_speech_mode)
            .field("speech_translation_mode", &self.speech_translation_mode)
            .field("xai_text_model", &self.xai_text_model)
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
    /// Display the selected models without retaining credentials in native UI state.
    /// Model identifiers come from the same protocol constants as the factory.
    pub fn model_names(&self) -> Vec<String> {
        use crate::core::protocols::{
            realtime::gemini_live::GeminiLiveEndpoint,
            realtime::openai_realtime::OpenAIRealtimeEndpoint,
            realtime::xai_realtime::XAIRealtimeEndpoint, recognition::audio3::Audio3ASREndpoint,
        };
        if self.provider == ProviderKind::AlibabaCloud {
            if let Some(model) = self.alibaba_realtime_mode.model_name() {
                return vec![model.to_owned()];
            }
        }
        let speech = match self.provider {
            ProviderKind::AlibabaCloud | ProviderKind::DeepLX => Audio3ASREndpoint::MODEL,
            ProviderKind::OpenAIRealtime if self.uses_realtime_asr() => "gpt-live-transcribe",
            ProviderKind::OpenAIRealtime => OpenAIRealtimeEndpoint::MODEL,
            ProviderKind::GoogleGeminiLive if self.uses_realtime_asr() => {
                crate::core::protocols::recognition::gemini_transcribe::MODEL
            }
            ProviderKind::GoogleGeminiLive => GeminiLiveEndpoint::MODEL,
            ProviderKind::XAIRealtime if self.uses_xai_asr() => {
                crate::core::protocols::recognition::xai_stt::MODEL
            }
            ProviderKind::XAIRealtime => XAIRealtimeEndpoint::MODEL,
            ProviderKind::CustomDashScopeASR | ProviderKind::CustomOpenAIASR => {
                match &self.credentials {
                    ProviderCredentials::CustomSpeech { model, .. } => model,
                    _ => "",
                }
            }
            ProviderKind::AzureOpenAIRealtime => match &self.credentials {
                ProviderCredentials::AzureOpenAI {
                    transcription_deployment,
                    ..
                } if self.uses_realtime_asr() => transcription_deployment,
                ProviderCredentials::AzureOpenAI { deployment, .. } => deployment,
                _ => "",
            },
            _ => "",
        };
        let mut names = Vec::new();
        if !speech.is_empty() {
            names.push(speech.to_owned());
        }
        if self.target_language.translates_audio()
            && (self.uses_independent_asr()
                || matches!(
                    self.provider,
                    ProviderKind::AlibabaCloud
                        | ProviderKind::DeepLX
                        | ProviderKind::AppleSpeech
                        | ProviderKind::CustomDashScopeASR
                        | ProviderKind::CustomOpenAIASR
                ))
        {
            let text = match &self.text_credentials {
                Some(
                    TextTranslationCredentials::OpenAICompatible { model, .. }
                    | TextTranslationCredentials::ChatMock { model, .. }
                    | TextTranslationCredentials::XAI { model, .. },
                ) => model.as_str(),
                Some(TextTranslationCredentials::DeepL { .. }) => "DeepL",
                Some(TextTranslationCredentials::DeepLX { .. }) => "DeepLX",
                Some(TextTranslationCredentials::Apple) => "Apple Translation",
                None if self.is_standalone_asr() => "",
                None => match &self.credentials {
                    ProviderCredentials::OpenAICompatible { model, .. }
                    | ProviderCredentials::ChatMock { model, .. } => model,
                    ProviderCredentials::DeepL { .. } => "DeepL",
                    ProviderCredentials::DeepLX { .. } => "DeepLX",
                    _ => self.qwen_mt_model.raw_name(),
                },
            };
            if !text.is_empty() {
                names.push(text.to_owned());
            }
        }
        names
    }
    /// Resolve the actual factory's endpoints before platform proxy selection.
    /// Constructors do not connect. Credentials in signed/query URLs are removed
    /// from this adapter-facing route snapshot; provider requests keep originals.
    pub fn network_endpoints(
        &self,
    ) -> Result<
        (String, Option<String>),
        crate::clients::translation::translation_client::TranslationClientError,
    > {
        let (events, _) = crate::clients::transport::provider_events::provider_event_channel();
        let client = crate::clients::translation::translation_client::TranslationClient::new_without_network(
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
        Ok((speech.map(route).unwrap_or_default(), text.map(route)))
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
            alibaba_realtime_mode: Default::default(),
            alibaba_workspace_id: String::new(),
            xai_speech_mode: Default::default(),
            speech_translation_mode: Default::default(),
            xai_text_model: crate::core::services::provider::default_xai_text_model(),
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
            alibaba_realtime_mode: Default::default(),
            alibaba_workspace_id: String::new(),
            xai_speech_mode: Default::default(),
            speech_translation_mode: Default::default(),
            xai_text_model: crate::core::services::provider::default_xai_text_model(),
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

    pub fn with_alibaba_realtime(
        mut self,
        mode: AlibabaRealtimeMode,
        workspace_id: impl Into<String>,
    ) -> Self {
        self.alibaba_realtime_mode = mode;
        self.alibaba_workspace_id = workspace_id.into();
        self
    }

    pub fn with_text_credentials(mut self, credentials: TextTranslationCredentials) -> Self {
        self.text_credentials = Some(credentials);
        self
    }

    pub fn with_xai_speech(mut self, mode: XAISpeechMode, model: impl Into<String>) -> Self {
        self.xai_speech_mode = mode;
        self.xai_text_model = model.into();
        self
    }

    pub fn with_speech_translation_mode(mut self, mode: SpeechTranslationMode) -> Self {
        self.speech_translation_mode = mode;
        self
    }

    pub fn uses_realtime_asr(&self) -> bool {
        self.provider.supports_speech_translation_mode()
            && self.speech_translation_mode == SpeechTranslationMode::AsrMt
    }

    pub fn uses_independent_asr(&self) -> bool {
        self.uses_xai_asr() || self.uses_realtime_asr()
    }

    pub fn uses_xai_asr(&self) -> bool {
        self.provider == ProviderKind::XAIRealtime && self.xai_speech_mode == XAISpeechMode::AsrMt
    }

    pub fn is_standalone_asr(&self) -> bool {
        self.provider.is_standalone_asr() || self.uses_independent_asr()
    }

    /// A legacy session-envelope probe reads only its independently selected
    /// text credentials. It never borrows the speech key or legacy text model.
    pub fn xai_text_probe_configuration(
        &self,
    ) -> Result<TextTranslationProbeConfiguration, LiveTranslationConfigurationError> {
        if !self.uses_xai_asr() {
            return Err(LiveTranslationConfigurationError::InvalidXAISpeech);
        }
        let credentials = self
            .text_credentials
            .as_ref()
            .ok_or(ProviderCredentialsError::MissingTextTranslation)?
            .validated()?;
        Ok(TextTranslationProbeConfiguration {
            credentials: TextTranslationProbeCredentials::Independent(credentials),
            qwen_mt_model: self.qwen_mt_model,
            source_language: self.source_language,
            target_language: if self.target_language.translates_audio() {
                self.target_language
            } else {
                TargetLanguage::SimplifiedChinese
            },
            network_proxy: self.text_network_proxy.clone(),
        })
    }

    /// Legacy mode values remain readable, but every new session uses Turbo.
    /// Provider-specific transports and independent text destinations remain
    /// resolved by the provider facade.
    pub fn effective_translation_mode(&self) -> TranslationMode {
        TranslationMode::Turbo
    }

    pub fn capabilities(&self) -> ProviderCapabilities {
        if self.provider == ProviderKind::AlibabaCloud
            && self.alibaba_realtime_mode != AlibabaRealtimeMode::Disabled
        {
            return self.alibaba_realtime_mode.capabilities();
        }
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
        if self.uses_realtime_asr() {
            return crate::core::services::provider::realtime_asr_capabilities(
                self.provider,
                route,
                self.target_language,
            )
            .for_source(self.source_language);
        }
        if self.uses_xai_asr() {
            return crate::core::services::provider::xai_asr_capabilities(
                route,
                self.target_language,
            )
            .for_source(self.source_language);
        }
        self.provider
            .capabilities_for_route_with_model(route, self.target_language, self.qwen_mt_model)
            .for_source(self.source_language)
    }

    /// Returns a trimmed, validated copy of the configuration.
    pub fn validated(&self) -> Result<Self, LiveTranslationConfigurationError> {
        if self.speech_translation_mode != SpeechTranslationMode::Simultaneous
            && !self.provider.supports_speech_translation_mode()
        {
            return Err(LiveTranslationConfigurationError::UnsupportedSpeechTranslationMode);
        }
        if self.provider != ProviderKind::XAIRealtime
            && (self.xai_speech_mode != XAISpeechMode::Voice
                || self.xai_text_model != crate::core::services::provider::DEFAULT_XAI_TEXT_MODEL)
        {
            return Err(LiveTranslationConfigurationError::InvalidXAISpeech);
        }
        let xai_text_model =
            crate::core::protocols::translation::openai_compatible::validate_model(
                &self.xai_text_model,
            )
            .map_err(|_| LiveTranslationConfigurationError::InvalidXAISpeech)?;
        if self.alibaba_realtime_mode != AlibabaRealtimeMode::Disabled {
            if self.provider != ProviderKind::AlibabaCloud {
                return Err(LiveTranslationConfigurationError::UnsupportedAlibabaRealtime);
            }
            let workspace = self.alibaba_workspace_id.trim();
            if !alibaba_workspace_id_is_valid(workspace) {
                return Err(LiveTranslationConfigurationError::InvalidAlibabaWorkspaceId);
            }
        }
        let network_proxy = if self.provider == ProviderKind::AppleSpeech {
            ProxyConfig {
                mode: crate::core::services::network_proxy::ProxyMode::Direct,
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
                mode: crate::core::services::network_proxy::ProxyMode::Direct,
                url: None,
            }
        } else {
            self.text_network_proxy.validate()?
        };
        let credentials = self
            .credentials
            .validated_for_speech_mode(self.provider, self.speech_translation_mode)?;

        let text_credentials =
            if self.is_standalone_asr() && self.target_language.translates_audio() {
                Some(
                    self.text_credentials
                        .as_ref()
                        .ok_or(ProviderCredentialsError::MissingTextTranslation)?
                        .validated()?,
                )
            } else if self.is_standalone_asr() {
                None
            } else {
                self.text_credentials
                    .as_ref()
                    .map(TextTranslationCredentials::validated)
                    .transpose()?
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
            alibaba_realtime_mode: self.alibaba_realtime_mode,
            alibaba_workspace_id: self.alibaba_workspace_id.trim().to_string(),
            xai_speech_mode: self.xai_speech_mode,
            speech_translation_mode: self.speech_translation_mode,
            xai_text_model,
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
mod tests;
