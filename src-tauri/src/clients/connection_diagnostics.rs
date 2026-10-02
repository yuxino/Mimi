//! Explicit, bounded service checks. No capture, user text or response logging.

use crate::clients::audio3_client::{Audio3ASRClient, Audio3ASRClientError};
use crate::clients::deepl_client::DeepLClient;
use crate::clients::deeplx_client::DeepLXClient;
use crate::clients::openai_compatible_client::OpenAICompatibleClient;
use crate::clients::provider_events::provider_event_channel;
use crate::clients::provider_network::ProviderNetwork;
use crate::clients::qwen_mt_client::QwenMTClient;
use crate::clients::translation_client::{ConnectError, TranslationClient};
use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::credentials::ProviderCredentials;
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::deepl::DeepLError;
use crate::core::protocols::deeplx::DeepLXError;
use crate::core::protocols::openai_compatible::OpenAICompatibleError;
use crate::core::protocols::qwen_mt::{QwenMTClientError, REALTIME_MT_MODEL};
use crate::core::provider::ProviderKind;
use serde::Serialize;
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

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
}

#[derive(Debug, Serialize)]
pub struct ConnectionDiagnostic {
    pub credential: &'static str,
    pub service: ServiceAvailability,
    pub reason: Option<ConnectionCheckReason>,
}

impl ConnectionDiagnostic {
    pub fn not_tested(credential: &'static str) -> Self {
        Self {
            credential,
            service: ServiceAvailability::NotTested,
            reason: None,
        }
    }

    pub fn unavailable(credential: &'static str, reason: ConnectionCheckReason) -> Self {
        Self {
            credential,
            service: ServiceAvailability::Unavailable,
            reason: Some(reason),
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

/// A successful check means the actual configured service accepted its setup.
/// Alibaba's independent MT endpoint additionally must return a nonempty
/// translation of a fixed public test phrase. No app/session state is changed.
pub async fn check_service(configuration: &LiveTranslationConfiguration) -> ConnectionDiagnostic {
    initialize_probe_tls();
    let result = match configuration.provider {
        ProviderKind::AlibabaCloud | ProviderKind::DeepLX => probe_alibaba(configuration).await,
        _ => {
            let (events, _receiver) = provider_event_channel();
            match TranslationClient::new(configuration, events) {
                Ok(client) => probe_realtime(&client, PROBE_TIMEOUT).await,
                Err(_) => Err(ConnectionCheckReason::InvalidConfiguration),
            }
        }
    };
    match result {
        Ok(()) => ConnectionDiagnostic {
            credential: "present",
            service: ServiceAvailability::Available,
            reason: None,
        },
        Err(reason) => ConnectionDiagnostic::unavailable("present", reason),
    }
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
) -> Result<(), ConnectionCheckReason> {
    let network = ProviderNetwork::resolve(&configuration.network_proxy)
        .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    let key = match &configuration.credentials {
        ProviderCredentials::ApiKey { api_key } => api_key,
        ProviderCredentials::DeepLX { asr_api_key, .. }
        | ProviderCredentials::DeepL { asr_api_key, .. }
        | ProviderCredentials::OpenAICompatible { asr_api_key, .. } => asr_api_key,
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
        probe_text_translation(configuration, &network).await
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
    // Check both saved credentials even when the current display uses source
    // text only. The fixed test does not change the listening configuration.
    let target = match configuration.target_language {
        TargetLanguage::Original => TargetLanguage::SimplifiedChinese,
        target => target,
    };
    let (source, phrase) = if target == TargetLanguage::English {
        (SourceLanguage::Chinese, "你好。")
    } else {
        (SourceLanguage::English, "Hello.")
    };
    let translation = match &configuration.credentials {
        ProviderCredentials::ApiKey { api_key } => {
            let mut client = QwenMTClient::new(
                api_key,
                configuration.source_language,
                target,
                REALTIME_MT_MODEL,
                Some(
                    crate::core::protocols::qwen_mt::QwenMTDomainHint::spoken_dialogue(
                        configuration.source_language,
                        target,
                    ),
                ),
                crate::core::protocols::qwen_mt::QwenMTDomainHint::filler_terms(
                    configuration.source_language,
                    target,
                ),
                Duration::from_secs(8),
            )
            .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .set_network(network.clone())
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .translate_streaming(phrase, Some(source), &[], |_| {})
                .await
                .map_err(|error| qwen_reason(&error))?
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
        _ => return Err(ConnectionCheckReason::InvalidConfiguration),
    };
    if translation.trim().is_empty() {
        Err(ConnectionCheckReason::ServiceRejected)
    } else {
        Ok(())
    }
}

fn audio3_reason(error: &Audio3ASRClientError) -> ConnectionCheckReason {
    match error {
        Audio3ASRClientError::MissingAPIKey => ConnectionCheckReason::CredentialsMissing,
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

    #[tokio::test]
    async fn custom_chat_probe_checks_the_configured_model_and_rejects_invalid_responses() {
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
                ProviderCredentials::OpenAICompatible {
                    asr_api_key: "fixture-asr".into(),
                    endpoint: format!("http://{address}/proxy/v1"),
                    api_key: "fixture-chat-key".into(),
                    model: "fixture-model".into(),
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
