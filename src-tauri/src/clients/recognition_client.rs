//! Recognition-only clients shared by the bounded text-translation pipeline.

use super::apple_speech_client::AppleSpeechClient;
use super::audio3_client::{Audio3ASRClient, Audio3ASRClientError};
use super::custom_speech_client::{CustomSpeechClient, CustomSpeechClientError};
use super::local_speech_client::LocalSpeechClient;
use super::provider_events::ProviderEventSender;
use super::provider_network::{ProviderNetwork, ProviderNetworkError};
use crate::core::models::SourceLanguage;
use crate::core::pending_pcm::PendingPcmGate;
use crate::core::provider::ProviderKind;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecognitionClientError {
    #[error("{0}")]
    Apple(String),
    #[error("{0}")]
    Local(String),
    #[error("{0}")]
    Alibaba(#[from] Audio3ASRClientError),
    #[error("{0}")]
    Custom(#[from] CustomSpeechClientError),
}

impl RecognitionClientError {
    pub fn is_authentication_failure(&self) -> bool {
        matches!(
            self,
            Self::Custom(CustomSpeechClientError::AuthenticationFailed)
        ) || matches!(self, Self::Alibaba(Audio3ASRClientError::Task(label)) if label.contains(".authentication."))
    }

    pub fn is_timeout(&self) -> bool {
        if matches!(self, Self::Local(label) if matches!(label.as_str(), "local_model_setup_timeout" | "local_model_health_timeout"))
        {
            return true;
        }
        if matches!(self, Self::Apple(label) if label == "apple_speech_setup_timeout") {
            return true;
        }
        matches!(
            self,
            Self::Custom(
                CustomSpeechClientError::SetupTimeout | CustomSpeechClientError::HealthTimeout
            ) | Self::Alibaba(
                Audio3ASRClientError::ConnectionTimedOut
                    | Audio3ASRClientError::TaskSetupTimedOut
                    | Audio3ASRClientError::HealthCheckTimedOut
            )
        ) || matches!(self, Self::Alibaba(Audio3ASRClientError::Task(label)) if label.contains(".timeout."))
    }

    pub fn is_invalid_configuration(&self) -> bool {
        if matches!(self, Self::Local(label) if matches!(label.as_str(), "local_models_unavailable" | "local_model_missing" | "local_model_busy"))
        {
            return true;
        }
        if matches!(self, Self::Apple(label) if matches!(label.as_str(), "apple_speech_unavailable" | "apple_speech_language_unsupported" | "apple_speech_assets_missing" | "apple_speech_status_failed"))
        {
            return true;
        }
        matches!(
            self,
            Self::Custom(
                CustomSpeechClientError::InvalidEndpoint | CustomSpeechClientError::InvalidModel
            ) | Self::Alibaba(
                Audio3ASRClientError::InvalidCustomEndpoint
                    | Audio3ASRClientError::InvalidCustomModel
            )
        )
    }

    pub fn is_missing_credentials(&self) -> bool {
        matches!(
            self,
            Self::Custom(CustomSpeechClientError::MissingAPIKey)
                | Self::Alibaba(Audio3ASRClientError::MissingAPIKey)
        )
    }

    pub fn is_unreachable(&self) -> bool {
        matches!(
            self,
            Self::Custom(
                CustomSpeechClientError::Unreachable | CustomSpeechClientError::NotConnected
            )
        ) || matches!(self, Self::Alibaba(error) if error.is_unreachable())
    }
}

#[derive(Clone)]
pub enum RecognitionClient {
    Apple(AppleSpeechClient),
    Local(LocalSpeechClient),
    Audio3(Audio3ASRClient),
    OpenAI(CustomSpeechClient),
}

impl RecognitionClient {
    pub fn standalone(
        configuration: &crate::core::configuration::LiveTranslationConfiguration,
    ) -> Result<Self, RecognitionClientError> {
        match (&configuration.provider, &configuration.credentials) {
            (
                ProviderKind::LocalSpeech,
                crate::core::credentials::ProviderCredentials::LocalSpeech { model },
            ) => Ok(Self::Local(LocalSpeechClient::new(
                *model,
                configuration.source_language,
            ))),
            (
                ProviderKind::AppleSpeech,
                crate::core::credentials::ProviderCredentials::AppleSpeech,
            ) => Ok(Self::Apple(AppleSpeechClient::new(
                configuration.source_language,
            ))),
            (
                provider,
                crate::core::credentials::ProviderCredentials::CustomSpeech {
                    endpoint,
                    model,
                    api_key,
                },
            ) => Self::custom(
                *provider,
                endpoint,
                model,
                api_key,
                configuration.source_language,
            ),
            _ => Err(CustomSpeechClientError::InvalidEndpoint.into()),
        }
    }

    pub fn alibaba(api_key: &str, source: SourceLanguage) -> Result<Self, RecognitionClientError> {
        Ok(Self::Audio3(Audio3ASRClient::new(api_key, source)?))
    }

    pub fn custom(
        provider: ProviderKind,
        endpoint: &str,
        model: &str,
        api_key: &str,
        source: SourceLanguage,
    ) -> Result<Self, RecognitionClientError> {
        match provider {
            ProviderKind::CustomDashScopeASR => Ok(Self::Audio3(Audio3ASRClient::new_custom(
                endpoint, model, api_key, source,
            )?)),
            ProviderKind::CustomOpenAIASR => Ok(Self::OpenAI(CustomSpeechClient::new(
                endpoint, model, api_key, source,
            )?)),
            _ => Err(CustomSpeechClientError::InvalidEndpoint.into()),
        }
    }

    pub fn set_network(&mut self, network: ProviderNetwork) -> Result<(), ProviderNetworkError> {
        match self {
            Self::Apple(_) | Self::Local(_) => Ok(()),
            Self::Audio3(client) => client.set_network(network),
            Self::OpenAI(client) => client.set_network(network),
        }
    }

    pub fn set_audio_pending_gate(&self, gate: PendingPcmGate) {
        match self {
            Self::Apple(_) | Self::Local(_) => {}
            Self::Audio3(client) => client.set_audio_pending_gate(gate),
            Self::OpenAI(client) => client.set_audio_pending_gate(gate),
        }
    }

    pub async fn set_event_sender(&self, events: ProviderEventSender) {
        match self {
            Self::Apple(client) => client.set_event_sender(events),
            Self::Local(client) => client.set_event_sender(events),
            Self::Audio3(client) => client.set_event_sender(events).await,
            Self::OpenAI(client) => client.set_event_sender(events).await,
        }
    }

    pub async fn connect(&self, task_id: &str) -> Result<(), RecognitionClientError> {
        match self {
            Self::Apple(client) => client.connect().await,
            Self::Local(client) => client.connect().await,
            Self::Audio3(client) => client.connect(task_id).await.map_err(Into::into),
            Self::OpenAI(client) => client.connect(task_id).await.map_err(Into::into),
        }
    }

    pub async fn connect_for_probe(&self, task_id: &str) -> Result<(), RecognitionClientError> {
        match self {
            Self::Apple(client) => client.connect().await,
            Self::Local(client) => client.connect().await,
            Self::Audio3(client) => client.connect_for_probe(task_id).await.map_err(Into::into),
            Self::OpenAI(client) => client.connect_for_probe(task_id).await.map_err(Into::into),
        }
    }

    pub async fn send_audio(&self, pcm: &[u8]) -> Result<(), RecognitionClientError> {
        match self {
            Self::Apple(client) => client.send_audio(pcm).await,
            Self::Local(client) => client.send_audio(pcm),
            Self::Audio3(client) => client.send_audio(pcm).await.map_err(Into::into),
            Self::OpenAI(client) => client.send_audio(pcm).await.map_err(Into::into),
        }
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), RecognitionClientError> {
        match self {
            Self::Apple(client) => client.ping(),
            Self::Local(client) => client.ping(timeout).await,
            Self::Audio3(client) => client.ping(timeout).await.map_err(Into::into),
            Self::OpenAI(client) => client.ping(timeout).await.map_err(Into::into),
        }
    }

    pub async fn finish(&self, timeout: Duration) {
        match self {
            Self::Apple(client) => client.finish(timeout).await,
            Self::Local(client) => client.finish(timeout).await,
            Self::Audio3(client) => client.finish(timeout).await,
            Self::OpenAI(client) => client.finish(timeout).await,
        }
    }

    pub async fn disconnect(&self) {
        match self {
            Self::Apple(client) => client.disconnect().await,
            Self::Local(client) => client.disconnect().await,
            Self::Audio3(client) => client.disconnect().await,
            Self::OpenAI(client) => client.disconnect().await,
        }
    }

    pub async fn clear_content(&self) -> u64 {
        match self {
            Self::Apple(client) => client.clear_content(),
            Self::Local(client) => client.clear_content().await,
            Self::Audio3(client) => client.clear_content().await,
            Self::OpenAI(client) => client.clear_content().await,
        }
    }

    pub fn content_revision(&self) -> u64 {
        match self {
            Self::Apple(client) => client.content_revision(),
            Self::Local(client) => client.content_revision(),
            Self::Audio3(client) => client.content_revision(),
            Self::OpenAI(client) => client.content_revision(),
        }
    }
}
