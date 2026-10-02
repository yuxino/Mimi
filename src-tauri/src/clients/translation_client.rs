//! Provider- and mode-dispatching translation client facade.

use crate::clients::azure_openai_realtime_client::{
    AzureOpenAIRealtimeClient, AzureOpenAIRealtimeClientError,
};
use crate::clients::baidu_translate_client::{BaiduTranslateClient, BaiduTranslateClientError};
use crate::clients::gemini_live_client::{GeminiLiveClient, GeminiLiveClientError};
use crate::clients::high_quality_client::HighQualityTranslationClient;
use crate::clients::live_translate_client::{LiveTranslateClient, LiveTranslateClientError};
use crate::clients::openai_realtime_client::{OpenAIRealtimeClient, OpenAIRealtimeClientError};
use crate::clients::provider_events::ProviderEventSender;
use crate::clients::provider_network::{ProviderNetwork, ProviderNetworkError};
use crate::clients::tencent_cloud_client::{TencentCloudClient, TencentCloudClientError};
use crate::clients::volcano_engine_client::{VolcanoEngineClient, VolcanoEngineClientError};
use crate::clients::xai_realtime_client::{XAIRealtimeClient, XAIRealtimeClientError};
use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::credentials::{ProviderCredentials, ProviderCredentialsError};
use crate::core::diagnostics::TranslationLatency;
use crate::core::models::TranslationMode;
use crate::core::preview_pacing::MTRequestBudget;
use crate::core::protocols::qwen_mt::{QwenMTClientError, QwenMTModel, REALTIME_MT_MODEL};
use crate::core::provider::ProviderKind;
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Clone)]
pub enum TranslationClient {
    LowLatency(LiveTranslateClient),
    HighQuality(HighQualityTranslationClient),
    OpenAIRealtime(OpenAIRealtimeClient),
    GeminiLive(GeminiLiveClient),
    AzureOpenAIRealtime(AzureOpenAIRealtimeClient),
    TencentCloud(TencentCloudClient),
    BaiduTranslate(BaiduTranslateClient),
    VolcanoEngine(VolcanoEngineClient),
    XaiRealtime(XAIRealtimeClient),
}

impl TranslationClient {
    /// Only Audio3 synthesizes idle PCM; other provider transports are unchanged.
    pub fn set_audio_pending_gate(&self, gate: crate::core::pending_pcm::PendingPcmGate) {
        if let Self::HighQuality(client) = self {
            client.set_audio_pending_gate(gate);
        }
    }

    pub fn new(
        configuration: &LiveTranslationConfiguration,
        events: ProviderEventSender,
    ) -> Result<Self, TranslationClientError> {
        let network = ProviderNetwork::resolve(&configuration.network_proxy)?;
        let mut client = Self::new_without_network(configuration, events)?;
        client.set_network(network)?;
        Ok(client)
    }

    fn set_network(&mut self, network: ProviderNetwork) -> Result<(), ProviderNetworkError> {
        match self {
            Self::LowLatency(client) => client.set_network(network),
            Self::HighQuality(client) => client.set_network(network),
            Self::OpenAIRealtime(client) => client.set_network(network),
            Self::GeminiLive(client) => client.set_network(network),
            Self::AzureOpenAIRealtime(client) => client.set_network(network),
            Self::TencentCloud(client) => client.set_network(network),
            Self::BaiduTranslate(client) => client.set_network(network),
            Self::VolcanoEngine(client) => client.set_network(network),
            Self::XaiRealtime(client) => client.set_network(network),
        }
    }

    fn new_without_network(
        configuration: &LiveTranslationConfiguration,
        events: ProviderEventSender,
    ) -> Result<Self, TranslationClientError> {
        let credentials = configuration
            .credentials
            .validated_for(configuration.provider)?;
        match configuration.provider {
            ProviderKind::OpenAIRealtime => {
                return OpenAIRealtimeClient::new(
                    direct_api_key(&credentials)?,
                    configuration.target_language,
                    events,
                )
                .map(Self::OpenAIRealtime)
                .map_err(TranslationClientError::OpenAI);
            }
            ProviderKind::GoogleGeminiLive => {
                return GeminiLiveClient::new(
                    direct_api_key(&credentials)?,
                    configuration.target_language,
                    events,
                )
                .map(Self::GeminiLive)
                .map_err(TranslationClientError::Gemini);
            }
            ProviderKind::AzureOpenAIRealtime => {
                let (endpoint, deployment, transcription_deployment, api_key) = credentials
                    .azure_openai()
                    .ok_or(ProviderCredentialsError::ProviderMismatch)?;
                return AzureOpenAIRealtimeClient::new(
                    endpoint,
                    deployment,
                    transcription_deployment,
                    api_key,
                    configuration.target_language,
                    events,
                )
                .map(Self::AzureOpenAIRealtime)
                .map_err(TranslationClientError::AzureOpenAI);
            }
            ProviderKind::XAIRealtime => {
                return XAIRealtimeClient::new(
                    direct_api_key(&credentials)?,
                    configuration.target_language,
                    events,
                )
                .map(Self::XaiRealtime)
                .map_err(TranslationClientError::Xai);
            }
            ProviderKind::VolcanoEngine => {
                return VolcanoEngineClient::new(
                    direct_api_key(&credentials)?,
                    configuration.source_language,
                    configuration.target_language,
                    events,
                )
                .map(Self::VolcanoEngine)
                .map_err(TranslationClientError::VolcanoEngine);
            }
            ProviderKind::TencentCloud => {
                let (app_id, secret_id, secret_key) = credentials
                    .tencent_cloud()
                    .ok_or(ProviderCredentialsError::ProviderMismatch)?;
                return TencentCloudClient::new(
                    app_id,
                    secret_id,
                    secret_key,
                    configuration.source_language,
                    configuration.target_language,
                    events,
                )
                .map(Self::TencentCloud)
                .map_err(TranslationClientError::TencentCloud);
            }
            ProviderKind::BaiduTranslate => {
                let (app_id, app_key) = credentials
                    .baidu_translate()
                    .ok_or(ProviderCredentialsError::ProviderMismatch)?;
                return BaiduTranslateClient::new(
                    app_id,
                    app_key,
                    configuration.source_language,
                    configuration.target_language,
                    events,
                )
                .map(Self::BaiduTranslate)
                .map_err(TranslationClientError::BaiduTranslate);
            }
            ProviderKind::DeepLX => {
                let ProviderCredentials::DeepLX {
                    asr_api_key,
                    endpoint,
                    token,
                } = &credentials
                else {
                    return Err(ProviderCredentialsError::ProviderMismatch.into());
                };
                return HighQualityTranslationClient::new_deeplx(
                    asr_api_key,
                    endpoint,
                    token,
                    configuration.source_language,
                    configuration.target_language,
                    events,
                )
                .map(Self::HighQuality)
                .map_err(TranslationClientError::MT);
            }
            ProviderKind::AlibabaCloud => {
                if let ProviderCredentials::OpenAICompatible {
                    asr_api_key,
                    endpoint,
                    api_key,
                    model,
                } = &credentials
                {
                    return HighQualityTranslationClient::new_openai_compatible(
                        asr_api_key,
                        endpoint,
                        api_key,
                        model,
                        configuration.source_language,
                        configuration.target_language,
                        events,
                    )
                    .map(Self::HighQuality)
                    .map_err(TranslationClientError::MT);
                }
                if let ProviderCredentials::DeepL {
                    asr_api_key,
                    api_key,
                } = &credentials
                {
                    return HighQualityTranslationClient::new_deepl(
                        asr_api_key,
                        api_key,
                        configuration.source_language,
                        configuration.target_language,
                        events,
                    )
                    .map(Self::HighQuality)
                    .map_err(TranslationClientError::MT);
                }
            }
        }
        // Automatic source recognition omits the transcription language on
        // the wire so the recognition service detects the language per
        // utterance (both protocol encoders handle `Automatic` this way,
        // mirroring the original app's RealtimeASRProtocol).
        match configuration.effective_translation_mode() {
            TranslationMode::LowLatency => {
                let client = LiveTranslateClient::new(
                    direct_api_key(&credentials)?,
                    configuration.source_language,
                    configuration.target_language,
                    BTreeMap::new(),
                    events,
                )
                .map_err(TranslationClientError::Live)?;
                Ok(Self::LowLatency(client))
            }
            TranslationMode::HighQuality => {
                let client = HighQualityTranslationClient::new(
                    direct_api_key(&credentials)?,
                    configuration.source_language,
                    configuration.target_language,
                    QwenMTModel::Plus,
                    Duration::from_millis(1_200),
                    Duration::from_millis(4_500),
                    20,
                    events,
                )
                .map_err(TranslationClientError::MT)?;
                Ok(Self::HighQuality(client))
            }
            TranslationMode::Turbo => {
                let client = HighQualityTranslationClient::new(
                    direct_api_key(&credentials)?,
                    configuration.source_language,
                    configuration.target_language,
                    REALTIME_MT_MODEL,
                    Duration::from_millis(250),
                    Duration::from_millis(1_000),
                    12,
                    events,
                )
                .map_err(TranslationClientError::MT)?;
                Ok(Self::HighQuality(client))
            }
        }
    }

    pub async fn connect(&self) -> Result<(), ConnectError> {
        match self {
            Self::LowLatency(client) => client.connect().await.map_err(ConnectError::Live),
            Self::HighQuality(client) => client.connect().await.map_err(ConnectError::MT),
            Self::OpenAIRealtime(client) => client.connect().await.map_err(ConnectError::OpenAI),
            Self::GeminiLive(client) => client.connect().await.map_err(ConnectError::Gemini),
            Self::AzureOpenAIRealtime(client) => {
                client.connect().await.map_err(ConnectError::AzureOpenAI)
            }
            Self::TencentCloud(client) => {
                client.connect().await.map_err(ConnectError::TencentCloud)
            }
            Self::BaiduTranslate(client) => {
                client.connect().await.map_err(ConnectError::BaiduTranslate)
            }
            Self::VolcanoEngine(client) => {
                client.connect().await.map_err(ConnectError::VolcanoEngine)
            }
            Self::XaiRealtime(client) => client.connect().await.map_err(ConnectError::Xai),
        }
    }

    /// Content clearing preserves the provider connection and audio route.
    pub async fn clear_content(&self) -> u64 {
        match self {
            Self::LowLatency(client) => client.clear_content().await,
            Self::HighQuality(client) => client.clear_content().await,
            Self::OpenAIRealtime(client) => client.clear_content().await,
            Self::GeminiLive(client) => client.clear_content().await,
            Self::AzureOpenAIRealtime(client) => client.clear_content().await,
            Self::TencentCloud(client) => client.clear_content().await,
            Self::BaiduTranslate(client) => client.clear_content().await,
            Self::VolcanoEngine(client) => client.clear_content().await,
            Self::XaiRealtime(client) => client.clear_content().await,
        }
    }

    pub fn content_revision(&self) -> u64 {
        match self {
            Self::LowLatency(client) => client.content_revision(),
            Self::HighQuality(client) => client.content_revision(),
            Self::OpenAIRealtime(client) => client.content_revision(),
            Self::GeminiLive(client) => client.content_revision(),
            Self::AzureOpenAIRealtime(client) => client.content_revision(),
            Self::TencentCloud(client) => client.content_revision(),
            Self::BaiduTranslate(client) => client.content_revision(),
            Self::VolcanoEngine(client) => client.content_revision(),
            Self::XaiRealtime(client) => client.content_revision(),
        }
    }

    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), ConnectError> {
        match self {
            Self::LowLatency(client) => client
                .send_audio(pcm_data)
                .await
                .map_err(ConnectError::Live),
            Self::HighQuality(client) => {
                client.send_audio(pcm_data).await.map_err(ConnectError::MT)
            }
            Self::OpenAIRealtime(client) => client
                .send_audio(pcm_data)
                .await
                .map_err(ConnectError::OpenAI),
            Self::GeminiLive(client) => client
                .send_audio(pcm_data)
                .await
                .map_err(ConnectError::Gemini),
            Self::AzureOpenAIRealtime(client) => client
                .send_audio(pcm_data)
                .await
                .map_err(ConnectError::AzureOpenAI),
            Self::TencentCloud(client) => client
                .send_audio(pcm_data)
                .await
                .map_err(ConnectError::TencentCloud),
            Self::BaiduTranslate(client) => client
                .send_audio(pcm_data)
                .await
                .map_err(ConnectError::BaiduTranslate),
            Self::VolcanoEngine(client) => client
                .send_audio(pcm_data)
                .await
                .map_err(ConnectError::VolcanoEngine),
            Self::XaiRealtime(client) => {
                client.send_audio(pcm_data).await.map_err(ConnectError::Xai)
            }
        }
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), ConnectError> {
        match self {
            Self::LowLatency(client) => client.ping(timeout).await.map_err(ConnectError::Live),
            Self::HighQuality(client) => client.ping(timeout).await.map_err(ConnectError::MT),
            Self::OpenAIRealtime(client) => {
                client.ping(timeout).await.map_err(ConnectError::OpenAI)
            }
            Self::GeminiLive(client) => client.ping(timeout).await.map_err(ConnectError::Gemini),
            Self::AzureOpenAIRealtime(client) => client
                .ping(timeout)
                .await
                .map_err(ConnectError::AzureOpenAI),
            Self::TencentCloud(client) => client
                .ping(timeout)
                .await
                .map_err(ConnectError::TencentCloud),
            Self::BaiduTranslate(client) => client
                .ping(timeout)
                .await
                .map_err(ConnectError::BaiduTranslate),
            Self::VolcanoEngine(client) => client
                .ping(timeout)
                .await
                .map_err(ConnectError::VolcanoEngine),
            Self::XaiRealtime(client) => client.ping(timeout).await.map_err(ConnectError::Xai),
        }
    }

    /// Providers with an unambiguous measured boundary expose it here; an
    /// unavailable measurement must stay absent rather than guess a duration.
    pub fn translation_latency(&self) -> Option<TranslationLatency> {
        match self {
            Self::LowLatency(client) => client.translation_latency(),
            Self::HighQuality(client) => client.translation_latency(),
            _ => None,
        }
    }

    pub async fn finish(&self) {
        match self {
            Self::LowLatency(client) => client.finish(Duration::from_secs(1)).await,
            Self::HighQuality(client) => client.finish().await,
            Self::OpenAIRealtime(client) => client.finish(Duration::from_secs(2)).await,
            Self::GeminiLive(client) => client.finish(Duration::from_secs(2)).await,
            Self::AzureOpenAIRealtime(client) => client.finish(Duration::from_secs(2)).await,
            Self::TencentCloud(client) => client.finish(Duration::from_secs(2)).await,
            Self::BaiduTranslate(client) => client.finish(Duration::from_secs(2)).await,
            Self::VolcanoEngine(client) => client.finish(Duration::from_secs(2)).await,
            Self::XaiRealtime(client) => client.finish(Duration::from_secs(2)).await,
        }
    }

    pub(crate) async fn suspend_mt_request_budget(&self) -> Option<MTRequestBudget> {
        match self {
            Self::HighQuality(client) => Some(client.suspend_request_budget().await),
            _ => None,
        }
    }

    pub(crate) async fn restore_mt_request_budget(&self, budget: Option<MTRequestBudget>) {
        if let (Self::HighQuality(client), Some(budget)) = (self, budget) {
            client.restore_request_budget(budget).await;
        }
    }

    pub async fn disconnect(&self) {
        match self {
            Self::LowLatency(client) => client.disconnect().await,
            Self::HighQuality(client) => client.disconnect().await,
            Self::OpenAIRealtime(client) => client.disconnect().await,
            Self::GeminiLive(client) => client.disconnect().await,
            Self::AzureOpenAIRealtime(client) => client.disconnect().await,
            Self::TencentCloud(client) => client.disconnect().await,
            Self::BaiduTranslate(client) => client.disconnect().await,
            Self::VolcanoEngine(client) => client.disconnect().await,
            Self::XaiRealtime(client) => client.disconnect().await,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("{0}")]
    Live(#[from] LiveTranslateClientError),
    #[error("{0}")]
    MT(#[from] QwenMTClientError),
    #[error("{0}")]
    OpenAI(#[from] OpenAIRealtimeClientError),
    #[error("{0}")]
    Gemini(#[from] GeminiLiveClientError),
    #[error("{0}")]
    AzureOpenAI(#[from] AzureOpenAIRealtimeClientError),
    #[error("{0}")]
    TencentCloud(#[from] TencentCloudClientError),
    #[error("{0}")]
    BaiduTranslate(#[from] BaiduTranslateClientError),
    #[error("{0}")]
    VolcanoEngine(#[from] VolcanoEngineClientError),
    #[error("{0}")]
    Xai(#[from] XAIRealtimeClientError),
}

impl ConnectError {
    /// Content-free provider label for diagnostics. The wrapped error remains
    /// available for user-facing status, but is never written verbatim to
    /// pipeline logs.
    pub fn diagnostic_label(&self) -> &'static str {
        match self {
            Self::Live(_) => "provider.live_translate",
            Self::MT(_) => "provider.alibaba_high_quality",
            Self::OpenAI(_) => "provider.openai_realtime",
            Self::Gemini(_) => "provider.google_gemini_live",
            Self::AzureOpenAI(_) => "provider.azure_openai_realtime",
            Self::TencentCloud(_) => "provider.tencent_cloud",
            Self::BaiduTranslate(_) => "provider.baidu_translate",
            Self::VolcanoEngine(_) => "provider.volcano_engine",
            Self::Xai(_) => "provider.xai_realtime",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TranslationClientError {
    #[error("{0}")]
    Network(#[from] ProviderNetworkError),
    #[error("{0}")]
    Live(#[from] LiveTranslateClientError),
    #[error("{0}")]
    MT(#[from] QwenMTClientError),
    #[error("{0}")]
    OpenAI(#[from] OpenAIRealtimeClientError),
    #[error("{0}")]
    Gemini(#[from] GeminiLiveClientError),
    #[error("{0}")]
    AzureOpenAI(#[from] AzureOpenAIRealtimeClientError),
    #[error("{0}")]
    TencentCloud(#[from] TencentCloudClientError),
    #[error("{0}")]
    BaiduTranslate(#[from] BaiduTranslateClientError),
    #[error("{0}")]
    VolcanoEngine(#[from] VolcanoEngineClientError),
    #[error("{0}")]
    Xai(#[from] XAIRealtimeClientError),
    #[error("{0}")]
    Credentials(#[from] ProviderCredentialsError),
}

impl TranslationClientError {
    pub fn diagnostic_label(&self) -> &'static str {
        match self {
            Self::Network(_) => "configuration.network_proxy",
            Self::Live(_) => "configuration.live_translate",
            Self::MT(_) => "configuration.alibaba_high_quality",
            Self::OpenAI(_) => "configuration.openai_realtime",
            Self::Gemini(_) => "configuration.google_gemini_live",
            Self::AzureOpenAI(_) => "configuration.azure_openai_realtime",
            Self::TencentCloud(_) => "configuration.tencent_cloud",
            Self::BaiduTranslate(_) => "configuration.baidu_translate",
            Self::VolcanoEngine(_) => "configuration.volcano_engine",
            Self::Xai(_) => "configuration.xai_realtime",
            Self::Credentials(_) => "configuration.provider_credentials",
        }
    }
}

fn direct_api_key(credentials: &ProviderCredentials) -> Result<&str, ProviderCredentialsError> {
    credentials
        .direct_api_key()
        .ok_or(ProviderCredentialsError::ProviderMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::provider_events::provider_event_channel;
    use crate::core::models::{SourceLanguage, TargetLanguage};

    #[test]
    fn invalid_proxy_is_rejected_before_provider_client_construction() {
        let configuration = LiveTranslationConfiguration::for_provider(
            ProviderKind::AlibabaCloud,
            "synthetic-key",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            TranslationMode::Turbo,
        )
        .with_network_proxy(crate::core::network_proxy::ProxyConfig {
            mode: crate::core::network_proxy::ProxyMode::Custom,
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
        for target in [TargetLanguage::Japanese, TargetLanguage::Original] {
            let configuration = LiveTranslationConfiguration::with_credentials(
                ProviderKind::AlibabaCloud,
                ProviderCredentials::OpenAICompatible {
                    asr_api_key: "synthetic-asr".into(),
                    endpoint: "https://example.com/proxy/v1".into(),
                    api_key: "synthetic-translation-key".into(),
                    model: "synthetic-model".into(),
                },
                SourceLanguage::Automatic,
                target,
                TranslationMode::Turbo,
            )
            .validated()
            .unwrap();
            assert_eq!(configuration.capabilities().source_languages.len(), 5);
            assert_eq!(configuration.capabilities().target_languages.len(), 4);
            let (events, _receiver) = provider_event_channel();
            assert!(matches!(
                TranslationClient::new(&configuration, events).unwrap(),
                TranslationClient::HighQuality(_)
            ));
            let mut unsupported = configuration;
            unsupported.source_language = SourceLanguage::French;
            assert!(matches!(unsupported.validated(), Err(crate::core::configuration::LiveTranslationConfigurationError::UnsupportedSourceLanguage)));
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
}
