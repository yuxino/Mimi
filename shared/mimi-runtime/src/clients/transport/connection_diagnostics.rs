//! Explicit, bounded service checks. No capture, user text or response logging.

use crate::clients::recognition::audio3_client::{Audio3ASRClient, Audio3ASRClientError};
use crate::clients::recognition::custom_speech_client::CustomSpeechClientError;
use crate::clients::recognition::gemini_transcribe_client::GeminiTranscribeClientError;
use crate::clients::recognition::recognition_client::{RecognitionClient, RecognitionClientError};
use crate::clients::recognition::xai_stt_client::XaiSttClientError;
use crate::clients::translation::deepl_client::DeepLClient;
use crate::clients::translation::deeplx_client::DeepLXClient;
use crate::clients::translation::openai_compatible_client::OpenAICompatibleClient;
use crate::clients::translation::qwen_mt_client::QwenMTClient;
use crate::clients::translation::translation_client::{ConnectError, TranslationClient};
use crate::clients::transport::provider_events::provider_event_channel;
use crate::clients::transport::provider_network::ProviderNetwork;
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::translation::deepl::DeepLError;
use crate::core::protocols::translation::deeplx::DeepLXError;
use crate::core::protocols::translation::openai_compatible::OpenAICompatibleError;
#[cfg(test)]
use crate::core::protocols::translation::qwen_mt::REALTIME_MT_MODEL;
use crate::core::protocols::translation::qwen_mt::{QwenMTClientError, QwenMTModel};
use crate::core::services::configuration::{
    LiveTranslationConfiguration, TextTranslationProbeConfiguration,
    TextTranslationProbeCredentials,
};
use crate::core::services::credentials::{ProviderCredentials, TextTranslationCredentials};
use crate::core::services::provider::ProviderKind;
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
    InvalidConfiguration,
    UnsupportedLanguage,
    LocalRecognitionOverloaded,
    LocalRecognitionTimeout,
    AuthenticationRejected,
    ServiceNotActivated,
    QuotaExhausted,
    ConcurrencyLimited,
    ServiceRejected,
    ProtocolInvalid,
    Timeout,
    Unreachable,
    TextTranslationNotConfigured,
    AppleSpeechAssetsMissing,
    AppleSpeechPreparing,
    AppleSpeechStatusFailed,
    AppleSpeechLanguageUnsupported,
    AppleSpeechUnavailable,
    AppleSpeechRecognitionFailed,
    AppleTranslationAssetsMissing,
    AppleTranslationLanguageUnsupported,
    AppleTranslationUnavailable,
    AppleTranslationFailed,
}

#[derive(Debug, Serialize)]
pub struct ConnectionDiagnostic {
    pub credential: &'static str,
    pub service: ServiceAvailability,
    pub reason: Option<ConnectionCheckReason>,
    /// Actual setup/request duration, including local model setup; absent when skipped.
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
            _ => ConnectionCheckReason::InvalidConfiguration,
        };
        Some(Self::unavailable(credential, reason))
    }
}

/// Only known, content-free labels influence the public preparation result.
pub fn preparation_failure(error: &str) -> ConnectionDiagnostic {
    let local_reason = match error {
        "apple_translation_assets_missing" => {
            Some(ConnectionCheckReason::AppleTranslationAssetsMissing)
        }
        "apple_translation_language_unsupported" | "apple_translation_invalid_input" => {
            Some(ConnectionCheckReason::AppleTranslationLanguageUnsupported)
        }
        "apple_translation_unavailable" => Some(ConnectionCheckReason::AppleTranslationUnavailable),
        _ => None,
    };
    if let Some(reason) = local_reason {
        return ConnectionDiagnostic::unavailable("present", reason);
    }
    let credential = match error {
        "custom_speech_credentials_missing" | "text_translation_credentials_missing" => "missing",
        "credential_store_unavailable" => "unavailable",
        "credential_service_unavailable" => "serviceUnavailable",
        "credential_store_access_denied" => "accessDenied",
        _ if error.starts_with("Add the connection credentials for ") => "missing",
        _ => "invalid",
    };
    if error == "apple_speech_language_unsupported" {
        return ConnectionDiagnostic::unavailable(
            "present",
            ConnectionCheckReason::AppleSpeechLanguageUnsupported,
        );
    }
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
        _ if configuration.uses_independent_asr() => {
            match (
                ProviderNetwork::resolve(&configuration.network_proxy),
                ProviderNetwork::resolve(&configuration.text_network_proxy),
            ) {
                (Ok(speech), Ok(text)) => {
                    probe_independent_speech(configuration, include_translation, speech, text).await
                }
                _ => Err(ConnectionCheckReason::InvalidConfiguration),
            }
        }
        ProviderKind::AlibabaCloud | ProviderKind::DeepLX
            if configuration.alibaba_realtime_mode
                == crate::core::services::provider::AlibabaRealtimeMode::Disabled =>
        {
            probe_alibaba(configuration, include_translation).await
        }
        ProviderKind::CustomDashScopeASR
        | ProviderKind::CustomOpenAIASR
        | ProviderKind::AppleSpeech => {
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

/// Native Android adapters supply their platform trust and proxy snapshots;
/// the STT handshake and text request remain in this shared implementation.
pub async fn check_independent_speech_with_network(
    configuration: &LiveTranslationConfiguration,
    include_translation: bool,
    speech_network: ProviderNetwork,
    text_network: ProviderNetwork,
) -> ConnectionDiagnostic {
    let started = Instant::now();
    let result = probe_independent_speech(
        configuration,
        include_translation,
        speech_network,
        text_network,
    )
    .await;
    timed_diagnostic(result, started)
}

async fn probe_independent_speech(
    configuration: &LiveTranslationConfiguration,
    include_translation: bool,
    speech_network: ProviderNetwork,
    text_network: ProviderNetwork,
) -> Result<(), ConnectionCheckReason> {
    if !configuration.uses_independent_asr() {
        return Err(ConnectionCheckReason::InvalidConfiguration);
    }
    let mut asr =
        RecognitionClient::standalone(configuration).map_err(|error| recognition_reason(&error))?;
    asr.set_network(speech_network)
        .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    let (events, _receiver) = provider_event_channel();
    asr.set_event_sender(events).await;
    let task_id = uuid::Uuid::new_v4().simple().to_string();
    let result = tokio::time::timeout(PROBE_TIMEOUT, async {
        asr.connect_for_probe(&task_id)
            .await
            .map_err(|error| recognition_reason(&error))?;
        if include_translation {
            probe_text_translation(configuration, &text_network).await?;
        }
        Ok(())
    })
    .await
    .map_err(|_| ConnectionCheckReason::Timeout)
    .and_then(|result| result);
    asr.disconnect().await;
    result
}

#[cfg(test)]
#[tokio::test]
async fn selected_alibaba_realtime_probe_validates_its_workspace_before_network() {
    use crate::core::services::provider::AlibabaRealtimeMode;
    let configuration = LiveTranslationConfiguration::with_credentials(
        ProviderKind::AlibabaCloud,
        ProviderCredentials::api_key("synthetic-key"),
        SourceLanguage::Automatic,
        TargetLanguage::SimplifiedChinese,
        crate::core::models::TranslationMode::Turbo,
    )
    .with_alibaba_realtime(AlibabaRealtimeMode::LiveTranslate, "");
    let result = check_speech_service(&configuration, false).await;
    assert_eq!(
        result.reason,
        Some(ConnectionCheckReason::InvalidConfiguration)
    );
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
    let local = crate::core::services::network_proxy::ProxyConfig {
        mode: crate::core::services::network_proxy::ProxyMode::Direct,
        url: None,
    };
    let apple = matches!(
        configuration.credentials,
        TextTranslationProbeCredentials::Independent(TextTranslationCredentials::Apple)
    );
    if !apple {
        initialize_probe_tls();
    }
    let network = match ProviderNetwork::resolve(if apple {
        &local
    } else {
        &configuration.network_proxy
    }) {
        Ok(network) => network,
        Err(_) => {
            return ConnectionDiagnostic::unavailable(
                "present",
                ConnectionCheckReason::InvalidConfiguration,
            )
        }
    };
    check_text_service_with_network(configuration, network).await
}

/// The actual desktop readiness probe with a platform route/trust snapshot.
/// Native adapters do not implement a second HTTP request or status classifier.
pub async fn check_text_service_with_network(
    configuration: &TextTranslationProbeConfiguration,
    network: ProviderNetwork,
) -> ConnectionDiagnostic {
    if matches!(
        configuration.credentials,
        TextTranslationProbeCredentials::Independent(TextTranslationCredentials::Apple)
    ) {
        let started = Instant::now();
        let result = tokio::time::timeout(
            PROBE_TIMEOUT,
            probe_apple_translation(configuration.source_language, configuration.target_language),
        )
        .await
        .map_err(|_| ConnectionCheckReason::Timeout)
        .and_then(|result| result.map(|_| ()));
        return timed_diagnostic(result, started);
    }
    initialize_probe_tls();
    let started = Instant::now();
    let target = configuration.target_language;
    let (source, phrase) = fixed_test_phrase(target);
    let result = tokio::time::timeout(PROBE_TIMEOUT, async {
        let translation = match &configuration.credentials {
            TextTranslationProbeCredentials::Qwen { api_key } => {
                probe_qwen(
                    api_key,
                    &network,
                    source,
                    target,
                    phrase,
                    configuration.qwen_mt_model,
                )
                .await?
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

/// Resolve only the actual text-probe client's endpoint, without a speech key,
/// session or request. Android uses it for its endpoint-specific proxy snapshot.
pub fn text_probe_network_endpoint(
    configuration: &TextTranslationProbeConfiguration,
) -> Result<Option<String>, ConnectionCheckReason> {
    let target = configuration.target_language;
    let (source, _) = fixed_test_phrase(target);
    let mut endpoint = match &configuration.credentials {
        TextTranslationProbeCredentials::Qwen { api_key } => QwenMTClient::new(
            api_key,
            source,
            target,
            configuration.qwen_mt_model,
            None,
            vec![],
            Duration::from_secs(8),
        )
        .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?
        .network_endpoint(),
        TextTranslationProbeCredentials::Independent(credentials) => match credentials {
            TextTranslationCredentials::Apple => return Ok(None),
            TextTranslationCredentials::XAI { api_key, model } => OpenAICompatibleClient::new(
                crate::core::services::provider::XAI_TEXT_ENDPOINT,
                api_key,
                model,
                source,
                target,
            )
            .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?
            .network_endpoint(),
            TextTranslationCredentials::DeepL { api_key } => {
                DeepLClient::new(api_key, source, target)
                    .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?
                    .network_endpoint()
            }
            TextTranslationCredentials::DeepLX { endpoint, token } => {
                DeepLXClient::new(endpoint, token, source, target)
                    .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?
                    .network_endpoint()
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
            } => OpenAICompatibleClient::new(endpoint, api_key, model, source, target)
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?
                .network_endpoint(),
        },
    };
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    Ok(Some(endpoint.into()))
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
    if let Some(credentials) = &configuration.text_credentials {
        let target = if configuration.target_language.translates_audio() {
            configuration.target_language
        } else {
            TargetLanguage::SimplifiedChinese
        };
        let (source, phrase) = fixed_test_phrase(target);
        return probe_independent_text_translation(credentials, network, source, target, phrase)
            .await
            .map(|_| ());
    }
    if configuration.is_standalone_asr() && configuration.text_credentials.is_none() {
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
            probe_qwen(
                api_key,
                network,
                source,
                target,
                phrase,
                configuration.qwen_mt_model,
            )
            .await?
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
        ProviderCredentials::CustomSpeech { .. } | ProviderCredentials::AppleSpeech => {
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
    let mut asr =
        RecognitionClient::standalone(configuration).map_err(|error| recognition_reason(&error))?;
    if configuration.provider != ProviderKind::AppleSpeech {
        let network = ProviderNetwork::resolve(&configuration.network_proxy)
            .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
        asr.set_network(network)
            .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    }
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
    model: QwenMTModel,
) -> Result<String, ConnectionCheckReason> {
    let mut client = QwenMTClient::new(
        api_key,
        source,
        target,
        model,
        Some(
            crate::core::protocols::translation::qwen_mt::QwenMTDomainHint::spoken_dialogue(
                source, target,
            ),
        ),
        crate::core::protocols::translation::qwen_mt::QwenMTDomainHint::filler_terms(
            source, target,
        ),
        Duration::from_secs(8),
    )
    .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    client
        .set_network(network.clone())
        .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
    let result = if model == QwenMTModel::Plus {
        client.translate(phrase, Some(source), &[]).await
    } else {
        client
            .translate_streaming(phrase, Some(source), &[], |_| {})
            .await
    };
    result.map_err(|error| qwen_reason(&error))
}

async fn probe_independent_text_translation(
    credentials: &TextTranslationCredentials,
    network: &ProviderNetwork,
    source: SourceLanguage,
    target: TargetLanguage,
    phrase: &str,
) -> Result<String, ConnectionCheckReason> {
    match credentials {
        TextTranslationCredentials::Apple => probe_apple_translation(source, target).await,
        TextTranslationCredentials::XAI { api_key, model } => {
            let mut client = OpenAICompatibleClient::new(
                crate::core::services::provider::XAI_TEXT_ENDPOINT,
                api_key,
                model,
                source,
                target,
            )
            .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .set_network(network.clone())
                .map_err(|_| ConnectionCheckReason::InvalidConfiguration)?;
            client
                .translate(phrase, Some(source))
                .await
                .map_err(|error| openai_compatible_reason(&error))
        }
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

async fn probe_apple_translation(
    source: SourceLanguage,
    target: TargetLanguage,
) -> Result<String, ConnectionCheckReason> {
    let client =
        crate::clients::translation::apple_translation_client::AppleTranslationClient::new(
            source, target,
        )
        .map_err(|error| qwen_reason(&error))?;
    client
        .check_ready()
        .await
        .map_err(|error| qwen_reason(&error))?;
    let phrase = apple_test_phrase(source)
        .ok_or(ConnectionCheckReason::AppleTranslationLanguageUnsupported)?;
    if target.matches_reported_asr(Some(source.raw_value())) {
        return Ok(phrase.into());
    }
    client
        .translate(phrase, Some(source))
        .await
        .map_err(|error| qwen_reason(&error))
}

// Public synthetic setup phrases in the selected source. No recognized text or
// user content participates in an explicit translation check.
fn apple_test_phrase(source: SourceLanguage) -> Option<&'static str> {
    Some(
        match source.raw_value().split('-').next().unwrap_or_default() {
            "en" => "Hello.",
            "zh" | "zh_tw" => "你好。",
            "ja" => "こんにちは。",
            "ko" => "안녕하세요.",
            "vi" => "Xin chào.",
            "th" => "สวัสดี",
            "id" => "Halo.",
            "ms" => "Helo.",
            "tl" => "Kumusta.",
            "hi" => "नमस्ते।",
            "ar" => "مرحبا.",
            "fr" => "Bonjour.",
            "de" => "Hallo.",
            "es" => "Hola.",
            "pt" => "Olá.",
            "ru" => "Здравствуйте.",
            "it" => "Ciao.",
            "nl" => "Hallo.",
            "sv" => "Hej.",
            "da" => "Hej.",
            "fi" | "no" => "Hei.",
            "el" => "Γεια σας.",
            "pl" => "Dzień dobry.",
            "cs" => "Dobrý den.",
            "hu" => "Jó napot.",
            "ro" => "Bună ziua.",
            "bg" => "Здравейте.",
            "hr" => "Dobar dan.",
            "sk" => "Dobrý deň.",
            "tr" => "Merhaba.",
            "uk" => "Вітаю.",
            _ => return None,
        },
    )
}

fn recognition_reason(error: &RecognitionClientError) -> ConnectionCheckReason {
    if matches!(
        error,
        RecognitionClientError::Gemini(GeminiTranscribeClientError::InvalidProtocol)
            | RecognitionClientError::Xai(XaiSttClientError::InvalidProtocol)
            | RecognitionClientError::Custom(CustomSpeechClientError::InvalidProtocol)
    ) {
        return ConnectionCheckReason::ProtocolInvalid;
    }
    if let RecognitionClientError::Apple(label) = error {
        return match label.as_str() {
            "apple_speech_assets_missing" => ConnectionCheckReason::AppleSpeechAssetsMissing,
            "apple_speech_preparing" => ConnectionCheckReason::AppleSpeechPreparing,
            "apple_speech_status_failed" | "apple_speech_service_unavailable" => {
                ConnectionCheckReason::AppleSpeechStatusFailed
            }
            "apple_speech_language_unsupported" => {
                ConnectionCheckReason::AppleSpeechLanguageUnsupported
            }
            "apple_speech_unavailable" => ConnectionCheckReason::AppleSpeechUnavailable,
            _ => ConnectionCheckReason::AppleSpeechRecognitionFailed,
        };
    }
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
                Some("local_overload") => ConnectionCheckReason::LocalRecognitionOverloaded,
                Some("local_timeout") => ConnectionCheckReason::LocalRecognitionTimeout,
                Some("unsupported_language")
                    if token.ends_with(".unsupported_language.UNSUPPORTED_LANGUAGE") =>
                {
                    ConnectionCheckReason::UnsupportedLanguage
                }
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
        DeepLXError::Rejected(429) => ConnectionCheckReason::ConcurrencyLimited,
        DeepLXError::Rejected(456) => ConnectionCheckReason::QuotaExhausted,
        DeepLXError::Timeout => ConnectionCheckReason::Timeout,
        DeepLXError::Connection => ConnectionCheckReason::Unreachable,
        DeepLXError::Endpoint => ConnectionCheckReason::InvalidConfiguration,
        _ => ConnectionCheckReason::ServiceRejected,
    }
}

fn deepl_reason(error: &DeepLError) -> ConnectionCheckReason {
    match error {
        DeepLError::Rejected(401 | 403) => ConnectionCheckReason::AuthenticationRejected,
        DeepLError::Rejected(429) => ConnectionCheckReason::ConcurrencyLimited,
        DeepLError::Rejected(456) => ConnectionCheckReason::QuotaExhausted,
        DeepLError::Timeout => ConnectionCheckReason::Timeout,
        DeepLError::Connection => ConnectionCheckReason::Unreachable,
        DeepLError::InvalidKey => ConnectionCheckReason::InvalidConfiguration,
        _ => ConnectionCheckReason::ServiceRejected,
    }
}

fn qwen_reason(error: &QwenMTClientError) -> ConnectionCheckReason {
    match error {
        QwenMTClientError::Apple(label) => match *label {
            "apple_translation_assets_missing" => {
                ConnectionCheckReason::AppleTranslationAssetsMissing
            }
            "apple_translation_language_unsupported" => {
                ConnectionCheckReason::AppleTranslationLanguageUnsupported
            }
            "apple_translation_unavailable" => ConnectionCheckReason::AppleTranslationUnavailable,
            "apple_translation_timeout" => ConnectionCheckReason::Timeout,
            _ => ConnectionCheckReason::AppleTranslationFailed,
        },
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

pub(crate) fn connection_reason(error: &ConnectError) -> ConnectionCheckReason {
    use crate::clients::realtime::azure_openai_realtime_client::AzureOpenAIRealtimeClientError as Azure;
    use crate::clients::realtime::baidu_translate_client::BaiduTranslateClientError as Baidu;
    use crate::clients::realtime::gemini_live_client::GeminiLiveClientError as Gemini;
    use crate::clients::realtime::live_translate_client::LiveTranslateClientError as Live;
    use crate::clients::realtime::openai_realtime_client::OpenAIRealtimeClientError as OpenAI;
    use crate::clients::realtime::tencent_cloud_client::TencentCloudClientError as Tencent;
    use crate::clients::realtime::volcano_engine_client::VolcanoEngineClientError as Volcano;
    use crate::clients::realtime::xai_realtime_client::XAIRealtimeClientError as Xai;
    use crate::core::protocols::realtime::openai_realtime::OpenAIRealtimeFailure as OpenAIFailure;
    match error {
        ConnectError::MT(error) => qwen_reason(error),
        ConnectError::TencentCloud(Tencent::ProviderRejected { code }) => match code {
            6001 => ConnectionCheckReason::InvalidConfiguration,
            6002 => ConnectionCheckReason::AuthenticationRejected,
            6003 => ConnectionCheckReason::ServiceNotActivated,
            6004 | 6005 => ConnectionCheckReason::QuotaExhausted,
            6006 => ConnectionCheckReason::ConcurrencyLimited,
            _ => ConnectionCheckReason::ServiceRejected,
        },
        ConnectError::OpenAI(OpenAI::ProviderRejected(OpenAIFailure::Quota)) => {
            ConnectionCheckReason::QuotaExhausted
        }
        ConnectError::OpenAI(OpenAI::ProviderRejected(OpenAIFailure::RateLimited)) => {
            ConnectionCheckReason::ConcurrencyLimited
        }
        ConnectError::OpenAI(OpenAI::ProviderRejected(
            OpenAIFailure::ModelUnavailable | OpenAIFailure::InvalidRequest,
        )) => ConnectionCheckReason::InvalidConfiguration,
        ConnectError::OpenAI(
            OpenAI::AuthenticationFailed | OpenAI::ProviderRejected(OpenAIFailure::Permission),
        )
        | ConnectError::Live(Live::AuthenticationFailed)
        | ConnectError::Gemini(Gemini::AuthenticationFailed)
        | ConnectError::AzureOpenAI(Azure::AuthenticationFailed)
        | ConnectError::TencentCloud(Tencent::AuthenticationFailed)
        | ConnectError::BaiduTranslate(Baidu::AuthenticationFailed)
        | ConnectError::VolcanoEngine(Volcano::AuthenticationFailed)
        | ConnectError::Xai(Xai::AuthenticationFailed) => {
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

/// Authentication labels shared with Android, without changing either wire protocol.
#[cfg(test)]
pub(crate) fn handshake_failure_cases() -> Vec<(Option<u16>, Option<String>)> {
    let contract: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    assert_eq!(contract["schemaVersion"], 1);
    let cases = contract["websocketHandshakeFailures"].as_array().unwrap();
    assert!(!cases.is_empty());
    cases
        .iter()
        .map(|case| {
            (
                case["httpStatus"]
                    .as_u64()
                    .map(|status| u16::try_from(status).unwrap()),
                case["expected"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

/// Local handshake fixture used by each real provider connect path. None closes
/// the socket without an HTTP response; no endpoint or credential leaves loopback.
#[cfg(test)]
pub(crate) async fn rejected_websocket_endpoint(
    status: Option<u16>,
) -> (url::Url, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let mut buffer = [0; 1024];
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
            assert!(request.len() <= 8192);
        }
        if let Some(status) = status {
            let body = "private-handshake-body";
            let response = format!(
                "HTTP/1.1 {status} Rejected\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (
        url::Url::parse(&format!("ws://{address}/realtime")).unwrap(),
        server,
    )
}

#[cfg(test)]
mod tests;
