//! Typed independent text destinations and their transport-specific calls.
//!
//! Recognition construction stays at the public entry. This adapter selects
//! text credentials and never substitutes a speech credential or another route.

use super::{
    LiveTranslationConfiguration, ProviderNetwork, ProviderNetworkError, QwenMTClient,
    QwenMTClientError, QwenMTModel, SourceLanguage, TextTranslationCredentials,
};

#[derive(Clone)]
pub(super) enum TextTranslationClient {
    Apple(crate::clients::translation::apple_translation_client::AppleTranslationClient),
    Disabled,
    Qwen(QwenMTClient, QwenMTModel),
    DeepLX(crate::clients::translation::deeplx_client::DeepLXClient),
    DeepL(crate::clients::translation::deepl_client::DeepLClient),
    OpenAICompatible(crate::clients::translation::openai_compatible_client::OpenAICompatibleClient),
}
impl TextTranslationClient {
    pub(super) fn independent(
        configuration: &LiveTranslationConfiguration,
    ) -> Result<Self, QwenMTClientError> {
        let source = configuration.source_language;
        let target = configuration.target_language;
        if !target.translates_audio() {
            return Ok(Self::Disabled);
        }
        match configuration
            .text_credentials
            .as_ref()
            .ok_or(QwenMTClientError::MissingTextTranslation)?
        {
            TextTranslationCredentials::Apple => Ok(Self::Apple(
                crate::clients::translation::apple_translation_client::AppleTranslationClient::new(
                    source, target,
                )?,
            )),
            TextTranslationCredentials::DeepL { api_key } => Ok(Self::DeepL(
                crate::clients::translation::deepl_client::DeepLClient::new(
                    api_key, source, target,
                )
                .map_err(QwenMTClientError::DeepL)?,
            )),
            TextTranslationCredentials::DeepLX { endpoint, token } => Ok(Self::DeepLX(
                crate::clients::translation::deeplx_client::DeepLXClient::new(
                    endpoint, token, source, target,
                )
                .map_err(QwenMTClientError::DeepLX)?,
            )),
            TextTranslationCredentials::XAI { api_key, model } => Ok(Self::OpenAICompatible(
                crate::clients::translation::openai_compatible_client::OpenAICompatibleClient::new(
                    crate::core::services::provider::XAI_TEXT_ENDPOINT,
                    api_key,
                    model,
                    source,
                    target,
                )
                .map_err(QwenMTClientError::OpenAICompatible)?,
            )),
            TextTranslationCredentials::OpenAICompatible {
                endpoint,
                model,
                api_key,
            }
            | TextTranslationCredentials::ChatMock {
                endpoint,
                model,
                api_key,
            } => Ok(Self::OpenAICompatible(
                crate::clients::translation::openai_compatible_client::OpenAICompatibleClient::new(
                    endpoint, api_key, model, source, target,
                )
                .map_err(QwenMTClientError::OpenAICompatible)?,
            )),
        }
    }

    pub(super) fn network_endpoint(&self) -> Option<url::Url> {
        match self {
            Self::Apple(_) | Self::Disabled => None,
            Self::Qwen(client, _) => Some(client.network_endpoint()),
            Self::DeepLX(client) => Some(client.network_endpoint()),
            Self::DeepL(client) => Some(client.network_endpoint()),
            Self::OpenAICompatible(client) => Some(client.network_endpoint()),
        }
    }
    pub(super) fn set_network(
        &mut self,
        network: ProviderNetwork,
    ) -> Result<(), ProviderNetworkError> {
        match self {
            Self::Disabled | Self::Apple(_) => Ok(()),
            Self::Qwen(client, _) => client.set_network(network),
            Self::DeepL(client) => client.set_network(network),
            Self::DeepLX(client) => client.set_network(network),
            Self::OpenAICompatible(client) => client.set_network(network),
        }
    }
    pub(super) fn supports_reported_source(&self, language: Option<&str>) -> bool {
        match self {
            Self::Disabled => true,
            Self::Apple(_) => language
                .is_none_or(|reported| SourceLanguage::from_detected(Some(reported)).is_some()),
            Self::Qwen(_, model) => model.supports_reported_source(language),
            Self::DeepLX(_) | Self::DeepL(_) | Self::OpenAICompatible(_) => true,
        }
    }
    pub(super) async fn translate(
        &self,
        text: &str,
        source: Option<SourceLanguage>,
    ) -> Result<String, QwenMTClientError> {
        match self {
            Self::Disabled => Err(QwenMTClientError::MissingTextTranslation),
            Self::Apple(client) => client.translate(text, source).await,
            Self::Qwen(client, _) => client.translate(text, source, &[]).await,
            Self::DeepL(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::DeepL),
            Self::DeepLX(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::DeepLX),
            Self::OpenAICompatible(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::OpenAICompatible),
        }
    }
    pub(super) async fn translate_streaming(
        &self,
        text: &str,
        source: Option<SourceLanguage>,
        on_partial: impl Fn(String) + Send + Sync,
    ) -> Result<String, QwenMTClientError> {
        match self {
            Self::Disabled => Err(QwenMTClientError::MissingTextTranslation),
            Self::Apple(client) => client.translate(text, source).await,
            Self::Qwen(client, _) => {
                client
                    .translate_streaming(text, source, &[], on_partial)
                    .await
            }
            Self::DeepLX(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::DeepLX),
            Self::DeepL(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::DeepL),
            Self::OpenAICompatible(client) => client
                .translate(text, source)
                .await
                .map_err(QwenMTClientError::OpenAICompatible),
        }
    }
}
