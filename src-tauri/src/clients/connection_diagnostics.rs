//! Explicit, bounded service checks. No capture, user text or response logging.

use crate::clients::audio3_client::{Audio3ASRClient, Audio3ASRClientError};
use crate::clients::deepl_client::DeepLClient;
use crate::clients::deeplx_client::DeepLXClient;
use crate::clients::openai_compatible_client::OpenAICompatibleClient;
use crate::clients::provider_events::provider_event_channel;
use crate::clients::provider_network::ProviderNetwork;
use crate::clients::qwen_mt_client::QwenMTClient;
use crate::clients::recognition_client::{RecognitionClient, RecognitionClientError};
use crate::clients::translation_client::{ConnectError, TranslationClient};
use crate::core::configuration::{
    LiveTranslationConfiguration, TextTranslationProbeConfiguration,
    TextTranslationProbeCredentials,
};
use crate::core::credentials::{ProviderCredentials, TextTranslationCredentials};
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::deepl::DeepLError;
use crate::core::protocols::deeplx::DeepLXError;
use crate::core::protocols::openai_compatible::OpenAICompatibleError;
use crate::core::protocols::qwen_mt::{QwenMTClientError, REALTIME_MT_MODEL};
use crate::core::provider::ProviderKind;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionCheckStage {
    Speech,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceAvailability {
    Available,
    Unavailable,
    NotTested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionCheckReason {
    CredentialsMissing,
    CredentialsUnavailable,
    CredentialsServiceUnavailable,
    CredentialsAccessDenied,
    LocalDevCredentialsUnavailable,
    InvalidConfiguration,
    AuthenticationRejected,
    ServiceRejected,
    Timeout,
    Unreachable,
    TextTranslationNotConfigured,
}

#[derive(Debug, Serialize)]
pub struct ConnectionDiagnostic {
    pub credential: &'static str,
    pub service: ServiceAvailability,
    pub reason: Option<ConnectionCheckReason>,
    /// Actual setup/request duration; absent when no network request was made.
    #[serde(rename = "elapsedMs", skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

impl ConnectionDiagnostic {
    pub fn not_tested(credential: &'static str) -> Self {
        Self {
            credential,
            service: ServiceAvailability::NotTested,
            reason: None,
            elapsed_ms: None,
        }
    }

    pub fn unavailable(credential: &'static str, reason: ConnectionCheckReason) -> Self {
        Self {
            credential,
            service: ServiceAvailability::Unavailable,
            reason: Some(reason),
            elapsed_ms: None,
        }
    }

    pub fn credential_failure(credential: &'static str) -> Option<Self> {
        let reason = match credential {
            "present" => return None,
            "missing" => ConnectionCheckReason::CredentialsMissing,
            "unavailable" => ConnectionCheckReason::CredentialsUnavailable,
            "serviceUnavailable" => ConnectionCheckReason::CredentialsServiceUnavailable,
            "accessDenied" => ConnectionCheckReason::CredentialsAccessDenied,
            "localDevUnavailable" => ConnectionCheckReason::LocalDevCredentialsUnavailable,
            _ => ConnectionCheckReason::InvalidConfiguration,
        };
        Some(Self::unavailable(credential, reason))
    }
}

/// Only known, content-free labels influence the public preparation result.
pub fn preparation_failure(error: &str) -> ConnectionDiagnostic {
    let credential = match error {
        "custom_speech_credentials_missing" | "text_translation_credentials_missing" => "missing",
        "local_dev_credentials_unavailable" => "localDevUnavailable",
        "credential_store_unavailable" => "unavailable",
        "credential_service_unavailable" => "serviceUnavailable",
        "credential_store_access_denied" => "accessDenied",
        _ if error.starts_with("Add the connection credentials for ") => "missing",
        _ => "invalid",
    };
    if error == "text_translation_not_configured" {
        return ConnectionDiagnostic::unavailable(
            "missing",
            ConnectionCheckReason::TextTranslationNotConfigured,
        );
    }
    ConnectionDiagnostic::credential_failure(credential).unwrap_or_else(|| {
        ConnectionDiagnostic::unavailable("invalid", ConnectionCheckReason::InvalidConfiguration)
    })
}

/// A successful check means the actual configured service accepted its setup.
/// Alibaba's independent MT endpoint additionally must return a nonempty
/// translation of a fixed public test phrase. No app/session state is changed.
pub async fn check_service(configuration: &LiveTranslationConfiguration) -> ConnectionDiagnostic {
    check_speech_service(configuration, true).await
}

pub async fn check_speech_service(
    configuration: &LiveTranslationConfiguration,
    include_translation: bool,
) -> ConnectionDiagnostic {
    let started = Instant::now();
    initialize_probe_tls();
    let result = match configuration.provider {
        ProviderKind::AlibabaCloud | ProviderKind::DeepLX => {
            probe_alibaba(configuration, include_translation).await
        }
        ProviderKind::CustomDashScopeASR | ProviderKind::CustomOpenAIASR => {
            probe_custom_speech(configuration, include_translation).await
        }
        _ => {
            let (events, _receiver) = provider_event_channel();
            match TranslationClient::new(configuration, events) {
                Ok(client) => probe_realtime(&client, PROBE_TIMEOUT).await,
                Err(_) => Err(ConnectionCheckReason::InvalidConfiguration),
            }
        }
    };
    timed_diagnostic(result, started)
}

fn timed_diagnostic(
    result: Result<(), ConnectionCheckReason>,
    started: Instant,
) -> ConnectionDiagnostic {
    let mut diagnostic = match result {
        Ok(()) => ConnectionDiagnostic {
            credential: "present",
            service: ServiceAvailability::Available,
            reason: None,
            elapsed_ms: None,
        },
        Err(reason) => ConnectionDiagnostic::unavailable("present", reason),
    };
    diagnostic.elapsed_ms = Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
    diagnostic
}

pub async fn check_text_service(
    configuration: &TextTranslationProbeConfiguration,
) -> ConnectionDiagnostic {
    initialize_probe_tls();
    let network = match ProviderNetwork::resolve(&configuration.network_proxy) {
        Ok(network) => network,
        Err(_) => {
            return ConnectionDiagnostic::unavailable(
                "present",
                ConnectionCheckReason::InvalidConfiguration,
            );
        }
    };
    let started = Instant::now();
    let target = configuration.target_language;
    let (source, phrase) = fixed_test_phrase(target);
    let result = tokio::time::timeout(PROBE_TIMEOUT, async {
        let translation = match &configuration.credentials {
            TextTranslationProbeCredentials::Qwen { api_key } => {
                probe_qwen(api_key, &network, source, target, phrase).await?
            }
            TextTranslationProbeCredentials::Independent(credentials) => {
                probe_independent_text_translation(credentials, &network, source, target, phrase)
                    .await?
            }
        };
        if translation.trim().is_empty() {
            Err(ConnectionCheckReason::ServiceRejected)
        } else {
            Ok(())
        }
    })
    .await
    .map_err(|_| ConnectionCheckReason::Timeout)
    .and_then(|result| result);
    timed_diagnostic(result, started)
}

fn initialize_probe_tls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

async fn probe_realtime(
    client: &TranslationClient,
    timeout: Duration,
) -> Result<(), ConnectionCheckReason> {
    let result = tokio::time::timeout(timeout, client.connect())
        .await
        .map_err(|_| ConnectionCheckReason::Timeout)
        .and_then(|result| result.map_err(|error| connection_reason(&error)));
    // This also cancels a receive task left by an outer setup timeout.
    client.disconnect().await;
    result
}

async fn probe_alibaba(
    configuration: &LiveTranslationConfiguration,
    include_translation: bool,
) -> Result<(), ConnectionCheckReason> {
    let network = ProviderNetwork::resolve(&configuration.network_proxy)
        .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    let key = match &configuration.credentials {
        ProviderCredentials::ApiKey { api_key } => api_key,
        ProviderCredentials::DeepLX { asr_api_key, .. }
        | ProviderCredentials::DeepL { asr_api_key, .. }
        | ProviderCredentials::OpenAICompatible { asr_api_key, .. }
        | ProviderCredentials::ChatMock { asr_api_key, .. } => asr_api_key,
        _ => return Err(ConnectionCheckReason::InvalidConfiguration),
    };
    let mut asr = Audio3ASRClient::new(key, configuration.source_language)
        .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    asr.set_network(network.clone())
        .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    let (events, _receiver) = provider_event_channel();
    asr.set_event_sender(events).await;
    let task_id = uuid::Uuid::new_v4().simple().to_string();
    let result = tokio::time::timeout(PROBE_TIMEOUT, async {
        asr.connect_for_probe(&task_id)
            .await
            .map_err(|error| audio3_reason(&error))?;
        if include_translation {
            let text_network = ProviderNetwork::resolve(&configuration.text_network_proxy)
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            probe_text_translation(configuration, &text_network).await
        } else {
            Ok(())
        }
    })
    .await
    .map_err(|_| ConnectionCheckReason::Timeout)
    .and_then(|result| result);
    asr.disconnect().await;
    result
}

async fn probe_text_translation(
    configuration: &LiveTranslationConfiguration,
    network: &ProviderNetwork,
) -> Result<(), ConnectionCheckReason> {
    if configuration.provider.is_custom_speech() && configuration.text_credentials.is_none() {
        return if configuration.target_language == TargetLanguage::Original {
            Ok(())
        } else {
            Err(ConnectionCheckReason::InvalidConfiguration)
        };
    }
    // Check both saved credentials even when the current display uses source
    // text only. The fixed test does not change the listening configuration.
    let target = match configuration.target_language {
        TargetLanguage::Original => TargetLanguage::SimplifiedChinese,
        target => target,
    };
    let (source, phrase) = fixed_test_phrase(target);
    let translation = match &configuration.credentials {
        ProviderCredentials::ApiKey { api_key } => {
            probe_qwen(api_key, network, source, target, phrase).await?
        }
        ProviderCredentials::DeepLX {
            endpoint, token, ..
        } => {
            let mut client = DeepLXClient::new(endpoint, token, source, target)
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .set_network(network.clone())
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .translate(phrase, Some(source))
                .await
                .map_err(|error| deeplx_reason(&error))?
        }
        ProviderCredentials::DeepL { api_key, .. } => {
            let mut client = DeepLClient::new(api_key, source, target)
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .set_network(network.clone())
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .translate(phrase, Some(source))
                .await
                .map_err(|error| deepl_reason(&error))?
        }
        ProviderCredentials::OpenAICompatible {
            endpoint,
            api_key,
            model,
            ..
        }
        | ProviderCredentials::ChatMock {
            endpoint,
            api_key,
            model,
            ..
        } => {
            let mut client = OpenAICompatibleClient::new(endpoint, api_key, model, source, target)
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .set_network(network.clone())
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .translate(phrase, Some(source))
                .await
                .map_err(|error| openai_compatible_reason(&error))?
        }
        ProviderCredentials::CustomSpeech { .. } => {
            probe_independent_text_translation(
                configuration
                    .text_credentials
                    .as_ref()
                    .ok_or(ConnectionCheckReason::InvalidConfiguration)?,
                network,
                source,
                target,
                phrase,
            )
            .await?
        }
        _ => return Err(ConnectionCheckReason::InvalidConfiguration),
    };
    if translation.trim().is_empty() {
        Err(ConnectionCheckReason::ServiceRejected)
    } else {
        Ok(())
    }
}

async fn probe_custom_speech(
    configuration: &LiveTranslationConfiguration,
    include_translation: bool,
) -> Result<(), ConnectionCheckReason> {
    let ProviderCredentials::CustomSpeech {
        endpoint,
        model,
        api_key,
    } = &configuration.credentials
    else {
        return Err(ConnectionCheckReason::InvalidConfiguration);
    };
    let network = ProviderNetwork::resolve(&configuration.network_proxy)
        .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    let mut asr = RecognitionClient::custom(
        configuration.provider,
        endpoint,
        model,
        api_key,
        configuration.source_language,
    )
    .map_err(|error| recognition_reason(&error))?;
    asr.set_network(network.clone())
        .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    let (events, _receiver) = provider_event_channel();
    asr.set_event_sender(events).await;
    let task_id = uuid::Uuid::new_v4().simple().to_string();
    let result = tokio::time::timeout(PROBE_TIMEOUT, async {
        asr.connect_for_probe(&task_id)
            .await
            .map_err(|error| recognition_reason(&error))?;
        if include_translation {
            let text_network = ProviderNetwork::resolve(&configuration.text_network_proxy)
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            probe_text_translation(configuration, &text_network).await
        } else {
            Ok(())
        }
    })
    .await
    .map_err(|_| ConnectionCheckReason::Timeout)
    .and_then(|result| result);
    asr.disconnect().await;
    result
}

fn fixed_test_phrase(target: TargetLanguage) -> (SourceLanguage, &'static str) {
    if target == TargetLanguage::English {
        (SourceLanguage::Chinese, "你好。")
    } else {
        (SourceLanguage::English, "Hello.")
    }
}

async fn probe_qwen(
    api_key: &str,
    network: &ProviderNetwork,
    source: SourceLanguage,
    target: TargetLanguage,
    phrase: &str,
) -> Result<String, ConnectionCheckReason> {
    let mut client = QwenMTClient::new(
        api_key,
        source,
        target,
        REALTIME_MT_MODEL,
        Some(crate::core::protocols::qwen_mt::QwenMTDomainHint::spoken_dialogue(source, target)),
        crate::core::protocols::qwen_mt::QwenMTDomainHint::filler_terms(source, target),
        Duration::from_secs(8),
    )
    .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    client
        .set_network(network.clone())
        .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    client
        .translate_streaming(phrase, Some(source), &[], |_| {})
        .await
        .map_err(|error| qwen_reason(&error))
}

async fn probe_independent_text_translation(
    credentials: &TextTranslationCredentials,
    network: &ProviderNetwork,
    source: SourceLanguage,
    target: TargetLanguage,
    phrase: &str,
) -> Result<String, ConnectionCheckReason> {
    match credentials {
        TextTranslationCredentials::DeepL { api_key } => {
            let mut client = DeepLClient::new(api_key, source, target)
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .set_network(network.clone())
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .translate(phrase, Some(source))
                .await
                .map_err(|error| deepl_reason(&error))
        }
        TextTranslationCredentials::DeepLX { endpoint, token } => {
            let mut client = DeepLXClient::new(endpoint, token, source, target)
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .set_network(network.clone())
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .translate(phrase, Some(source))
                .await
                .map_err(|error| deeplx_reason(&error))
        }
        TextTranslationCredentials::OpenAICompatible {
            endpoint,
            model,
            api_key,
        }
        | TextTranslationCredentials::ChatMock {
            endpoint,
            model,
            api_key,
        } => {
            let mut client = OpenAICompatibleClient::new(endpoint, api_key, model, source, target)
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .set_network(network.clone())
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .translate(phrase, Some(source))
                .await
                .map_err(|error| openai_compatible_reason(&error))
        }
    }
}

fn recognition_reason(error: &RecognitionClientError) -> ConnectionCheckReason {
    if error.is_missing_credentials() {
        ConnectionCheckReason::CredentialsMissing
    } else if error.is_invalid_configuration() {
        ConnectionCheckReason::InvalidConfiguration
    } else if error.is_authentication_failure() {
        ConnectionCheckReason::AuthenticationRejected
    } else if error.is_timeout() {
        ConnectionCheckReason::Timeout
    } else if error.is_unreachable() {
        ConnectionCheckReason::Unreachable
    } else {
        ConnectionCheckReason::ServiceRejected
    }
}

fn audio3_reason(error: &Audio3ASRClientError) -> ConnectionCheckReason {
    match error {
        Audio3ASRClientError::MissingAPIKey => ConnectionCheckReason::CredentialsMissing,
        Audio3ASRClientError::InvalidCustomEndpoint | Audio3ASRClientError::InvalidCustomModel => {
            ConnectionCheckReason::InvalidConfiguration
        }
        Audio3ASRClientError::Task(token) if token.starts_with("audio3_error.") => {
            match token.split('.').nth(2) {
                Some("authentication") => ConnectionCheckReason::AuthenticationRejected,
                Some("timeout") => ConnectionCheckReason::Timeout,
                _ => ConnectionCheckReason::ServiceRejected,
            }
        }
        Audio3ASRClientError::ConnectionTimedOut
        | Audio3ASRClientError::TaskSetupTimedOut
        | Audio3ASRClientError::HealthCheckTimedOut => ConnectionCheckReason::Timeout,
        Audio3ASRClientError::TransportFailure | Audio3ASRClientError::NotConnected => {
            ConnectionCheckReason::Unreachable
        }
        Audio3ASRClientError::Task(_) => ConnectionCheckReason::ServiceRejected,
    }
}

fn deeplx_reason(error: &DeepLXError) -> ConnectionCheckReason {
    match error {
        DeepLXError::Rejected(401 | 403) => ConnectionCheckReason::AuthenticationRejected,
        DeepLXError::Timeout => ConnectionCheckReason::Timeout,
        DeepLXError::Connection => ConnectionCheckReason::Unreachable,
        DeepLXError::Endpoint => ConnectionCheckReason::InvalidConfiguration,
        _ => ConnectionCheckReason::ServiceRejected,
    }
}

fn deepl_reason(error: &DeepLError) -> ConnectionCheckReason {
    match error {
        DeepLError::Rejected(401 | 403) => ConnectionCheckReason::AuthenticationRejected,
        DeepLError::Timeout => ConnectionCheckReason::Timeout,
        DeepLError::Connection => ConnectionCheckReason::Unreachable,
        DeepLError::InvalidKey => ConnectionCheckReason::InvalidConfiguration,
        _ => ConnectionCheckReason::ServiceRejected,
    }
}

fn qwen_reason(error: &QwenMTClientError) -> ConnectionCheckReason {
    match error {
        QwenMTClientError::OpenAICompatible(error) => openai_compatible_reason(error),
        QwenMTClientError::DeepLX(error) => deeplx_reason(error),
        QwenMTClientError::MissingAPIKey => ConnectionCheckReason::CredentialsMissing,
        QwenMTClientError::RequestFailed {
            status_code: 401 | 403,
            ..
        } => ConnectionCheckReason::AuthenticationRejected,
        QwenMTClientError::RequestTimedOut => ConnectionCheckReason::Timeout,
        _ => ConnectionCheckReason::ServiceRejected,
    }
}

fn openai_compatible_reason(error: &OpenAICompatibleError) -> ConnectionCheckReason {
    match error {
        OpenAICompatibleError::APIKey => ConnectionCheckReason::CredentialsMissing,
        OpenAICompatibleError::Rejected(401 | 403) => ConnectionCheckReason::AuthenticationRejected,
        OpenAICompatibleError::Timeout => ConnectionCheckReason::Timeout,
        OpenAICompatibleError::Connection => ConnectionCheckReason::Unreachable,
        OpenAICompatibleError::Endpoint
        | OpenAICompatibleError::Model
        | OpenAICompatibleError::Language => ConnectionCheckReason::InvalidConfiguration,
        _ => ConnectionCheckReason::ServiceRejected,
    }
}

fn connection_reason(error: &ConnectError) -> ConnectionCheckReason {
    use crate::clients::azure_openai_realtime_client::AzureOpenAIRealtimeClientError as Azure;
    use crate::clients::baidu_translate_client::BaiduTranslateClientError as Baidu;
    use crate::clients::gemini_live_client::GeminiLiveClientError as Gemini;
    use crate::clients::live_translate_client::LiveTranslateClientError as Live;
    use crate::clients::openai_realtime_client::OpenAIRealtimeClientError as OpenAI;
    use crate::clients::tencent_cloud_client::TencentCloudClientError as Tencent;
    use crate::clients::volcano_engine_client::VolcanoEngineClientError as Volcano;
    use crate::clients::xai_realtime_client::XAIRealtimeClientError as Xai;
    match error {
        ConnectError::MT(error) => qwen_reason(error),
        ConnectError::OpenAI(OpenAI::AuthenticationFailed)
        | ConnectError::Live(Live::AuthenticationFailed) => {
            ConnectionCheckReason::AuthenticationRejected
        }
        ConnectError::Live(
            Live::ConnectionTimedOut | Live::SessionSetupTimedOut | Live::HealthCheckTimedOut,
        )
        | ConnectError::OpenAI(OpenAI::SessionSetupTimedOut | OpenAI::HealthCheckTimedOut)
        | ConnectError::Gemini(Gemini::SessionSetupTimedOut | Gemini::HealthCheckTimedOut)
        | ConnectError::AzureOpenAI(Azure::SessionSetupTimedOut | Azure::HealthCheckTimedOut)
        | ConnectError::TencentCloud(
            Tencent::SessionSetupTimedOut | Tencent::HealthCheckTimedOut,
        )
        | ConnectError::BaiduTranslate(Baidu::SessionSetupTimedOut | Baidu::HealthCheckTimedOut)
        | ConnectError::VolcanoEngine(
            Volcano::ConnectionTimedOut
            | Volcano::SessionSetupTimedOut
            | Volcano::HealthCheckTimedOut,
        )
        | ConnectError::Xai(Xai::SessionSetupTimedOut | Xai::HealthCheckTimedOut) => {
            ConnectionCheckReason::Timeout
        }
        ConnectError::Live(Live::TransportFailure | Live::NotConnected)
        | ConnectError::OpenAI(OpenAI::TransportFailure | OpenAI::NotConnected)
        | ConnectError::Gemini(Gemini::TransportFailure | Gemini::NotConnected)
        | ConnectError::AzureOpenAI(Azure::TransportFailure | Azure::NotConnected)
        | ConnectError::TencentCloud(Tencent::TransportFailure | Tencent::NotConnected)
        | ConnectError::BaiduTranslate(Baidu::TransportFailure | Baidu::NotConnected)
        | ConnectError::VolcanoEngine(Volcano::TransportFailure | Volcano::NotConnected)
        | ConnectError::Xai(Xai::TransportFailure | Xai::NotConnected) => {
            ConnectionCheckReason::Unreachable
        }
        _ => ConnectionCheckReason::ServiceRejected,
    }
}

pub fn authentication_rejected(error: &tokio_tungstenite::tungstenite::Error) -> bool {
    matches!(error, tokio_tungstenite::tungstenite::Error::Http(response)
        if matches!(response.status().as_u16(), 401 | 403))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::openai_realtime_client::OpenAIRealtimeClient;
    use crate::core::models::TranslationMode;
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::Message;

    fn custom_probe_configuration(
        provider: ProviderKind,
        endpoint: String,
    ) -> LiveTranslationConfiguration {
        LiveTranslationConfiguration::with_credentials(
            provider,
            ProviderCredentials::CustomSpeech {
                endpoint,
                model: "synthetic-recognition-model".into(),
                api_key: "synthetic-recognition-key".into(),
            },
            SourceLanguage::Automatic,
            TargetLanguage::Original,
            TranslationMode::Turbo,
        )
        .with_network_proxy(crate::core::network_proxy::ProxyConfig {
            mode: crate::core::network_proxy::ProxyMode::Direct,
            url: None,
        })
    }

    async fn serve_custom_probe_setup(listener: tokio::net::TcpListener, provider: ProviderKind) {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let update = socket.next().await.unwrap().unwrap();
        let update: serde_json::Value = serde_json::from_str(update.to_text().unwrap()).unwrap();
        let ack = if provider == ProviderKind::CustomDashScopeASR {
            assert_eq!(update["payload"]["model"], "synthetic-recognition-model");
            serde_json::json!({"header":{"event":"task-started","task_id":update["header"]["task_id"]}})
        } else {
            assert_eq!(update["type"], "session.update");
            assert_eq!(
                update["session"]["audio"]["input"]["transcription"]["model"],
                "synthetic-recognition-model"
            );
            serde_json::json!({"type":"session.updated","session":update["session"]})
        };
        socket
            .send(Message::Text(ack.to_string().into()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match socket.next().await {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(_))) => {}
                    Some(Err(_)) => break,
                    other => {
                        panic!("connection probe sent audio or an unexpected message: {other:?}")
                    }
                }
            }
        })
        .await
        .expect("probe must close the recognizer");
    }

    #[tokio::test]
    async fn custom_recognition_original_probe_accepts_actual_setup_and_closes_without_pcm_or_mt() {
        for provider in [
            ProviderKind::CustomDashScopeASR,
            ProviderKind::CustomOpenAIASR,
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let configuration = custom_probe_configuration(
                provider,
                format!("ws://{}/recognition", listener.local_addr().unwrap()),
            );
            let server = tokio::spawn(serve_custom_probe_setup(listener, provider));
            let diagnostic = check_service(&configuration).await;
            assert_eq!(diagnostic.service, ServiceAvailability::Available);
            assert_eq!(diagnostic.reason, None);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn speech_stage_does_not_contact_saved_translation_destination_and_reports_elapsed_time()
    {
        let provider = ProviderKind::CustomOpenAIASR;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mt_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let configuration = custom_probe_configuration(
            provider,
            format!("ws://{}/realtime", listener.local_addr().unwrap()),
        )
        .with_text_credentials(TextTranslationCredentials::OpenAICompatible {
            endpoint: format!("http://{}/v1", mt_listener.local_addr().unwrap()),
            model: "synthetic-text-model".into(),
            api_key: "synthetic-text-key".into(),
        });
        let server = tokio::spawn(serve_custom_probe_setup(listener, provider));
        let diagnostic = check_speech_service(&configuration, false).await;
        assert_eq!(diagnostic.service, ServiceAvailability::Available);
        assert!(diagnostic.elapsed_ms.is_some());
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), mt_listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn text_stage_checks_a_fixed_phrase_without_speech_and_reports_response_time() {
        for (status, body, expected) in [
            (
                200,
                r#"{"choices":[{"message":{"content":"こんにちは。"}}]}"#,
                ServiceAvailability::Available,
            ),
            (
                401,
                "private provider response",
                ServiceAvailability::Unavailable,
            ),
            (
                200,
                r#"{"choices":[{"message":{"content":" "}}]}"#,
                ServiceAvailability::Unavailable,
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 2048];
                    let read = socket.read(&mut buffer).await.unwrap();
                    assert!(read > 0);
                    bytes.extend_from_slice(&buffer[..read]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let length: usize = String::from_utf8_lossy(&bytes[..end])
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse().unwrap())
                            })
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer synthetic-text-key"));
                let body_start = request.find("\r\n\r\n").unwrap() + 4;
                let sent: serde_json::Value = serde_json::from_str(&request[body_start..]).unwrap();
                assert_eq!(sent["model"], "synthetic-text-model");
                assert_eq!(sent["messages"][1]["content"], "Hello.");
                tokio::time::sleep(Duration::from_millis(25)).await;
                socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            let configuration = TextTranslationProbeConfiguration {
                credentials: TextTranslationProbeCredentials::Independent(
                    TextTranslationCredentials::OpenAICompatible {
                        endpoint: format!("http://{address}/v1"),
                        model: "synthetic-text-model".into(),
                        api_key: "synthetic-text-key".into(),
                    },
                ),
                target_language: TargetLanguage::Japanese,
                network_proxy: crate::core::network_proxy::ProxyConfig {
                    mode: crate::core::network_proxy::ProxyMode::Direct,
                    url: None,
                },
            };
            let diagnostic = check_text_service(&configuration).await;
            assert_eq!(diagnostic.service, expected);
            assert!(diagnostic.elapsed_ms.unwrap() >= 25);
            let public = serde_json::to_string(&diagnostic).unwrap();
            assert!(!public.contains("private"));
            assert!(!public.contains("synthetic-text-key"));
            assert!(public.contains("elapsedMs"));
            server.await.unwrap();
        }
    }

    #[test]
    fn unprepared_checks_have_no_fake_latency_or_raw_error_content() {
        for (label, reason) in [
            (
                "custom_speech_credentials_missing",
                ConnectionCheckReason::CredentialsMissing,
            ),
            (
                "text_translation_credentials_missing",
                ConnectionCheckReason::CredentialsMissing,
            ),
            (
                "text_translation_not_configured",
                ConnectionCheckReason::TextTranslationNotConfigured,
            ),
            (
                "credential_store_access_denied",
                ConnectionCheckReason::CredentialsAccessDenied,
            ),
            (
                "arbitrary private native error",
                ConnectionCheckReason::InvalidConfiguration,
            ),
        ] {
            let diagnostic = preparation_failure(label);
            assert_eq!(diagnostic.reason, Some(reason));
            assert_eq!(diagnostic.elapsed_ms, None);
            let public = serde_json::to_string(&diagnostic).unwrap();
            assert!(!public.contains("private"));
            assert!(!public.contains("elapsedMs"));
        }
    }

    #[tokio::test]
    async fn custom_recognition_probe_checks_independent_mt_even_in_original_mode_and_closes_on_rejection(
    ) {
        let provider = ProviderKind::CustomDashScopeASR;
        let asr_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mt_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let configuration = custom_probe_configuration(
            provider,
            format!("ws://{}/recognition", asr_listener.local_addr().unwrap()),
        )
        .with_text_credentials(TextTranslationCredentials::OpenAICompatible {
            endpoint: format!("http://{}/v1", mt_listener.local_addr().unwrap()),
            model: "synthetic-text-model".into(),
            api_key: "synthetic-text-key".into(),
        });
        let asr_server = tokio::spawn(serve_custom_probe_setup(asr_listener, provider));
        let mt_server = tokio::spawn(async move {
            let (mut socket, _) = mt_listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 2048];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let length: usize = String::from_utf8_lossy(&bytes[..end])
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|n| n.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer synthetic-text-key"));
            assert!(!request.contains("synthetic-recognition-key"));
            let body_start = request.find("\r\n\r\n").unwrap() + 4;
            let body: serde_json::Value = serde_json::from_str(&request[body_start..]).unwrap();
            assert_eq!(body["model"], "synthetic-text-model");
            assert_eq!(body["messages"][1]["content"], "Hello.");
            let private_response = "arbitrary private provider response";
            socket.write_all(format!("HTTP/1.1 401 Rejected\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{private_response}", private_response.len()).as_bytes()).await.unwrap();
        });
        let diagnostic = check_service(&configuration).await;
        assert_eq!(diagnostic.service, ServiceAvailability::Unavailable);
        assert_eq!(
            diagnostic.reason,
            Some(ConnectionCheckReason::AuthenticationRejected)
        );
        assert!(!serde_json::to_string(&diagnostic)
            .unwrap()
            .contains("private"));
        mt_server.await.unwrap();
        asr_server.await.unwrap();
    }

    #[tokio::test]
    async fn custom_chat_probe_checks_the_configured_model_and_rejects_invalid_responses() {
        assert_chat_probe_contract(false).await;
        assert_chat_probe_contract(true).await;
    }

    async fn assert_chat_probe_contract(chatmock: bool) {
        for (status, body, expected) in [
            (
                200,
                r#"{"choices":[{"message":{"content":"こんにちは。"}}]}"#,
                Ok(()),
            ),
            (
                200,
                r#"{"choices":[{"message":{"content":" "}}]}"#,
                Err(ConnectionCheckReason::ServiceRejected),
            ),
            (
                200,
                r#"{"code":200,"data":"wrong protocol"}"#,
                Err(ConnectionCheckReason::ServiceRejected),
            ),
            (
                401,
                "private response",
                Err(ConnectionCheckReason::AuthenticationRejected),
            ),
            (
                403,
                "private response",
                Err(ConnectionCheckReason::AuthenticationRejected),
            ),
            (
                302,
                "private response",
                Err(ConnectionCheckReason::ServiceRejected),
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 2048];
                    let read = socket.read(&mut buffer).await.unwrap();
                    assert!(read > 0);
                    bytes.extend_from_slice(&buffer[..read]);
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let length: usize = String::from_utf8_lossy(&bytes[..end])
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|n| n.trim().parse().unwrap())
                            })
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request.starts_with("POST /proxy/v1/chat/completions HTTP/1.1"));
                assert!(request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer fixture-chat-key"));
                let body_start = request.find("\r\n\r\n").unwrap() + 4;
                let sent: serde_json::Value = serde_json::from_str(&request[body_start..]).unwrap();
                assert_eq!(sent["model"], "fixture-model");
                assert_eq!(sent["stream"], false);
                assert_eq!(sent["messages"][1]["content"], "Hello.");
                socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            let configuration = LiveTranslationConfiguration::with_credentials(
                ProviderKind::AlibabaCloud,
                if chatmock {
                    ProviderCredentials::ChatMock {
                        asr_api_key: "fixture-asr".into(),
                        endpoint: format!("http://{address}/proxy/v1"),
                        api_key: "fixture-chat-key".into(),
                        model: "fixture-model".into(),
                    }
                } else {
                    ProviderCredentials::OpenAICompatible {
                        asr_api_key: "fixture-asr".into(),
                        endpoint: format!("http://{address}/proxy/v1"),
                        api_key: "fixture-chat-key".into(),
                        model: "fixture-model".into(),
                    }
                },
                SourceLanguage::Automatic,
                TargetLanguage::Japanese,
                TranslationMode::Turbo,
            );
            let network = ProviderNetwork::resolve(&crate::core::network_proxy::ProxyConfig {
                mode: crate::core::network_proxy::ProxyMode::Direct,
                url: None,
            })
            .unwrap();
            assert_eq!(
                probe_text_translation(&configuration, &network).await,
                expected
            );
            server.await.unwrap();
        }
    }

    #[test]
    fn diagnostics_never_mark_credential_failure_available_or_include_raw_errors() {
        for (credential, reason) in [
            ("missing", ConnectionCheckReason::CredentialsMissing),
            ("unavailable", ConnectionCheckReason::CredentialsUnavailable),
            (
                "serviceUnavailable",
                ConnectionCheckReason::CredentialsServiceUnavailable,
            ),
            (
                "accessDenied",
                ConnectionCheckReason::CredentialsAccessDenied,
            ),
            (
                "localDevUnavailable",
                ConnectionCheckReason::LocalDevCredentialsUnavailable,
            ),
            ("invalid", ConnectionCheckReason::InvalidConfiguration),
        ] {
            let result = ConnectionDiagnostic::credential_failure(credential).unwrap();
            assert_eq!(result.service, ServiceAvailability::Unavailable);
            assert_eq!(result.reason, Some(reason));
        }
        assert!(ConnectionDiagnostic::credential_failure("present").is_none());
        assert_eq!(
            serde_json::to_value(ConnectionDiagnostic::unavailable(
                "present",
                qwen_reason(&QwenMTClientError::RequestFailed {
                    status_code: 401,
                    message: "arbitrary private server content".into(),
                }),
            ))
            .unwrap(),
            serde_json::json!({"credential":"present", "service":"unavailable", "reason":"authenticationRejected"}),
        );
        for status in [401, 403] {
            assert_eq!(
                deepl_reason(&DeepLError::Rejected(status)),
                ConnectionCheckReason::AuthenticationRejected
            );
        }
        assert_eq!(
            deepl_reason(&DeepLError::Rejected(456)),
            ConnectionCheckReason::ServiceRejected
        );
        assert_eq!(
            deepl_reason(&DeepLError::Connection),
            ConnectionCheckReason::Unreachable
        );
    }

    #[tokio::test]
    async fn realtime_probe_requires_ready_and_disconnects_on_ready_rejection_and_timeout() {
        for (acknowledgement, expected) in [
            (
                Some(
                    r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#,
                ),
                Ok(()),
            ),
            (
                Some(
                    r#"{"type":"error","error":{"code":"invalid_request_error","message":"fixture-only server text"}}"#,
                ),
                Err(ConnectionCheckReason::ServiceRejected),
            ),
            (None, Err(ConnectionCheckReason::Timeout)),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let update = socket.next().await.unwrap().unwrap();
                let update: serde_json::Value =
                    serde_json::from_str(update.to_text().unwrap()).unwrap();
                assert_eq!(update["type"], "session.update");
                // A recoverable error from another request must neither
                // reject this setup nor be mistaken for its ready ack.
                socket
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "error",
                            "error": {
                                "code": "invalid_request_error",
                                "event_id": "unrelated-fixture-request",
                            },
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
                if let Some(acknowledgement) = acknowledgement {
                    let mut acknowledgement: serde_json::Value =
                        serde_json::from_str(acknowledgement).unwrap();
                    if acknowledgement["type"] == "error" {
                        // The service rejected this exact session.update.
                        // A generic recoverable error without that relation
                        // is correctly ignored by the production ready gate.
                        assert!(update["event_id"].as_str().is_some());
                        acknowledgement["error"]["event_id"] = update["event_id"].clone();
                    }
                    socket
                        .send(Message::Text(acknowledgement.to_string().into()))
                        .await
                        .unwrap();
                }
                let mut closed = false;
                while let Some(message) = socket.next().await {
                    match message.unwrap() {
                        Message::Close(_) => {
                            closed = true;
                            break;
                        }
                        Message::Binary(_) => panic!("a service check must not send audio"),
                        _ => {}
                    }
                }
                assert!(closed, "every probe outcome must close its WebSocket");
                let mut stream = socket.into_inner();
                let mut byte = [0; 1];
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte))
                        .await
                        .unwrap()
                        .unwrap(),
                    0,
                    "the temporary connection must reach peer EOF",
                );
            });
            let (events, _receiver) = provider_event_channel();
            let client = TranslationClient::OpenAIRealtime(
                OpenAIRealtimeClient::with_endpoint(
                    "fixture-only",
                    TargetLanguage::Japanese,
                    events,
                    url::Url::parse(&format!("ws://{address}")).unwrap(),
                )
                .unwrap(),
            );
            let timeout = if acknowledgement.is_some() {
                Duration::from_secs(2)
            } else {
                Duration::from_millis(300)
            };
            assert_eq!(probe_realtime(&client, timeout).await, expected);
            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn http_authentication_rejection_cannot_be_a_successful_probe() {
        for status in [401, 403] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 2048];
                let _ = socket.read(&mut request).await.unwrap();
                socket.write_all(format!("HTTP/1.1 {status} Rejected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            });
            let (events, _receiver) = provider_event_channel();
            let client = TranslationClient::OpenAIRealtime(
                OpenAIRealtimeClient::with_endpoint(
                    "fixture-only",
                    TargetLanguage::Japanese,
                    events,
                    url::Url::parse(&format!("ws://{address}")).unwrap(),
                )
                .unwrap(),
            );
            assert_eq!(
                probe_realtime(&client, Duration::from_secs(2)).await,
                Err(ConnectionCheckReason::AuthenticationRejected)
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn independent_text_service_requires_valid_nonempty_translation_and_authorization() {
        for (status, body, expected) in [
            (200, r#"{"code":200,"data":"こんにちは。"}"#, Ok(())),
            (
                200,
                r#"{"code":200,"data":"  "}"#,
                Err(ConnectionCheckReason::ServiceRejected),
            ),
            (
                200,
                r#"{"code":401,"data":"fixture-only private message"}"#,
                Err(ConnectionCheckReason::AuthenticationRejected),
            ),
            (
                200,
                r#"{"unrelated":"healthy"}"#,
                Err(ConnectionCheckReason::ServiceRejected),
            ),
            (
                401,
                "fixture-only private rejection",
                Err(ConnectionCheckReason::AuthenticationRejected),
            ),
            (
                403,
                "fixture-only private rejection",
                Err(ConnectionCheckReason::AuthenticationRejected),
            ),
            (
                302,
                r#"{"code":200,"data":"こんにちは。"}"#,
                Err(ConnectionCheckReason::ServiceRejected),
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 2048];
                    let read = socket.read(&mut buffer).await.unwrap();
                    assert!(read > 0);
                    bytes.extend_from_slice(&buffer[..read]);
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let length: usize = String::from_utf8_lossy(&bytes[..end])
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|n| n.trim().parse().unwrap())
                            })
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request.starts_with("POST /translate HTTP/1.1"));
                assert!(request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer fixture-token"));
                let body_start = request.find("\r\n\r\n").unwrap() + 4;
                let sent: serde_json::Value = serde_json::from_str(&request[body_start..]).unwrap();
                assert_eq!(sent["text"], "Hello.");
                socket.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            let configuration = LiveTranslationConfiguration::with_credentials(
                ProviderKind::DeepLX,
                ProviderCredentials::DeepLX {
                    asr_api_key: "fixture-asr".into(),
                    endpoint: format!("http://{address}"),
                    token: "fixture-token".into(),
                },
                SourceLanguage::Automatic,
                TargetLanguage::Japanese,
                TranslationMode::Turbo,
            );
            let network = ProviderNetwork::resolve(&crate::core::network_proxy::ProxyConfig {
                mode: crate::core::network_proxy::ProxyMode::Direct,
                url: None,
            })
            .unwrap();
            assert_eq!(
                probe_text_translation(&configuration, &network).await,
                expected
            );
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn text_probe_uses_the_custom_proxy_instead_of_the_direct_endpoint() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let unused_destination = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = unused_destination.local_addr().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 2048];
                let length = socket.read(&mut buffer).await.unwrap();
                assert!(length > 0);
                request.extend_from_slice(&buffer[..length]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let length: usize = String::from_utf8_lossy(&request[..end])
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with(&format!("POST http://{destination}/translate HTTP/1.1")));
            let body = r#"{"code":200,"data":"Synthetic proxy response"}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let configuration = LiveTranslationConfiguration::with_credentials(
            ProviderKind::DeepLX,
            ProviderCredentials::DeepLX {
                asr_api_key: "synthetic-asr".into(),
                endpoint: format!("http://{destination}"),
                token: String::new(),
            },
            SourceLanguage::English,
            TargetLanguage::Japanese,
            TranslationMode::Turbo,
        )
        .with_network_proxy(crate::core::network_proxy::ProxyConfig {
            mode: crate::core::network_proxy::ProxyMode::Custom,
            url: Some(format!("http://{proxy}")),
        });
        let network = ProviderNetwork::resolve(&configuration.network_proxy).unwrap();
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(2),
                probe_text_translation(&configuration, &network)
            )
            .await
            .unwrap(),
            Ok(())
        );
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), unused_destination.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn production_http_client_initializes_tls_in_a_fresh_process() {
        const FLAG: &str = "MIMI_TEST_FRESH_HTTP_CLIENT";
        if std::env::var(FLAG).as_deref() == Ok("1") {
            // Other HTTP tests may have initialized the process-global provider.
            // A fresh process reproduces the first-click path from native QA.
            assert!(rustls::crypto::CryptoProvider::get_default().is_none());
            initialize_probe_tls();
            assert!(QwenMTClient::new(
                "fixture-only",
                SourceLanguage::Automatic,
                TargetLanguage::Japanese,
                REALTIME_MT_MODEL,
                None,
                Vec::new(),
                Duration::from_secs(8),
            )
            .is_ok());
            assert!(rustls::crypto::CryptoProvider::get_default().is_some());
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "clients::connection_diagnostics::tests::production_http_client_initializes_tls_in_a_fresh_process",
                "--nocapture",
            ])
            .env(FLAG, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fresh HTTP constructor failed: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[test]
    fn authenticated_handshake_rejections_are_not_network_failures() {
        for (status, rejected) in [(401, true), (403, true), (429, false), (500, false)] {
            let response = tokio_tungstenite::tungstenite::http::Response::builder()
                .status(status)
                .body(None)
                .unwrap();
            assert_eq!(
                authentication_rejected(&tokio_tungstenite::tungstenite::Error::Http(Box::new(
                    response
                ))),
                rejected
            );
        }
        assert!(!authentication_rejected(
            &tokio_tungstenite::tungstenite::Error::ConnectionClosed
        ));
    }
}
