#[test]
fn recognition_protocol_failures_are_distinct_from_service_rejections() {
    for (error, expected) in [
        (
            RecognitionClientError::Gemini(GeminiTranscribeClientError::InvalidProtocol),
            ConnectionCheckReason::ProtocolInvalid,
        ),
        (
            RecognitionClientError::Gemini(GeminiTranscribeClientError::SessionRejected),
            ConnectionCheckReason::ServiceRejected,
        ),
        (
            RecognitionClientError::Gemini(GeminiTranscribeClientError::AuthenticationFailed),
            ConnectionCheckReason::AuthenticationRejected,
        ),
        (
            RecognitionClientError::Gemini(GeminiTranscribeClientError::Unreachable),
            ConnectionCheckReason::Unreachable,
        ),
        (
            RecognitionClientError::Xai(XaiSttClientError::InvalidProtocol),
            ConnectionCheckReason::ProtocolInvalid,
        ),
        (
            RecognitionClientError::Xai(XaiSttClientError::SessionRejected),
            ConnectionCheckReason::ServiceRejected,
        ),
        (
            RecognitionClientError::Xai(XaiSttClientError::AuthenticationFailed),
            ConnectionCheckReason::AuthenticationRejected,
        ),
        (
            RecognitionClientError::Xai(XaiSttClientError::Unreachable),
            ConnectionCheckReason::Unreachable,
        ),
        (
            RecognitionClientError::Custom(CustomSpeechClientError::InvalidProtocol),
            ConnectionCheckReason::ProtocolInvalid,
        ),
        (
            RecognitionClientError::Custom(CustomSpeechClientError::SessionRejected),
            ConnectionCheckReason::ServiceRejected,
        ),
        (
            RecognitionClientError::Custom(CustomSpeechClientError::AuthenticationFailed),
            ConnectionCheckReason::AuthenticationRejected,
        ),
        (
            RecognitionClientError::Custom(CustomSpeechClientError::Unreachable),
            ConnectionCheckReason::Unreachable,
        ),
    ] {
        assert_eq!(recognition_reason(&error), expected);
    }
    assert_eq!(
        serde_json::to_value(ConnectionDiagnostic::unavailable(
            "present",
            ConnectionCheckReason::ProtocolInvalid
        ))
        .unwrap()["reason"],
        "protocolInvalid"
    );
}

#[test]
fn local_text_checks_use_a_synthetic_phrase_in_the_selected_source() {
    assert_eq!(
        apple_test_phrase(SourceLanguage::Japanese),
        Some("こんにちは。")
    );
    assert_eq!(apple_test_phrase(SourceLanguage::Chinese), Some("你好。"));
    assert_eq!(apple_test_phrase(SourceLanguage::English), Some("Hello."));
    for (code, phrase) in [
        ("tr", "Merhaba."),
        ("uk", "Вітаю."),
        ("pt-BR", "Olá."),
        ("ar-EG", "مرحبا."),
    ] {
        let source = serde_json::from_value(serde_json::json!(code)).unwrap();
        assert_eq!(apple_test_phrase(source), Some(phrase));
    }
    // Global language expansion must not send an English probe mislabeled as another language.
    assert_eq!(apple_test_phrase(SourceLanguage::Automatic), None);
    let unmapped = serde_json::from_value(serde_json::json!("ast")).unwrap();
    assert_eq!(apple_test_phrase(unmapped), None);
}

#[test]
fn tencent_json_rejections_use_shared_actionable_categories() {
    let contract: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    for case in contract["tencentSpeechTranslation"]["errors"]
        .as_array()
        .unwrap()
    {
        let expected = match case["expected"].as_str().unwrap() {
            "configuration" => ConnectionCheckReason::InvalidConfiguration,
            "authentication" => ConnectionCheckReason::AuthenticationRejected,
            "activation" => ConnectionCheckReason::ServiceNotActivated,
            "quota" => ConnectionCheckReason::QuotaExhausted,
            "capacity" => ConnectionCheckReason::ConcurrencyLimited,
            "rejected" => ConnectionCheckReason::ServiceRejected,
            unknown => panic!("unsupported Tencent fixture category: {unknown}"),
        };
        let error = ConnectError::TencentCloud(
            crate::clients::realtime::tencent_cloud_client::TencentCloudClientError::ProviderRejected {
                code: case["code"].as_i64().unwrap(),
            },
        );
        assert_eq!(connection_reason(&error), expected);
    }
}

#[test]
fn provider_handshake_authentication_is_an_actionable_diagnostic() {
    for error in [
        ConnectError::Gemini(crate::clients::realtime::gemini_live_client::GeminiLiveClientError::AuthenticationFailed),
        ConnectError::AzureOpenAI(crate::clients::realtime::azure_openai_realtime_client::AzureOpenAIRealtimeClientError::AuthenticationFailed),
        ConnectError::Xai(crate::clients::realtime::xai_realtime_client::XAIRealtimeClientError::AuthenticationFailed),
        ConnectError::TencentCloud(crate::clients::realtime::tencent_cloud_client::TencentCloudClientError::AuthenticationFailed),
        ConnectError::BaiduTranslate(crate::clients::realtime::baidu_translate_client::BaiduTranslateClientError::AuthenticationFailed),
        ConnectError::VolcanoEngine(crate::clients::realtime::volcano_engine_client::VolcanoEngineClientError::AuthenticationFailed),
    ] {
        assert_eq!(connection_reason(&error), ConnectionCheckReason::AuthenticationRejected);
        assert_eq!(error.to_string(), "credential_authentication_failed");
    }
}

#[tokio::test]
async fn typed_grok_text_probe_uses_its_fixed_endpoint_and_selected_text_proxy() {
    use crate::core::services::network_proxy::{ProxyConfig, ProxyMode};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let configuration = TextTranslationProbeConfiguration {
        credentials: TextTranslationProbeCredentials::Independent(
            TextTranslationCredentials::XAI {
                api_key: "synthetic-independent-text-key".into(),
                model: "chosen-grok-model".into(),
            },
        ),
        qwen_mt_model: Default::default(),
        source_language: SourceLanguage::English,
        target_language: TargetLanguage::Japanese,
        network_proxy: ProxyConfig {
            mode: ProxyMode::Custom,
            url: Some(format!("http://{}", listener.local_addr().unwrap())),
        },
    };
    assert_eq!(
        text_probe_network_endpoint(&configuration)
            .unwrap()
            .as_deref(),
        Some(crate::core::services::provider::XAI_TEXT_ENDPOINT)
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(socket.read_u8().await.unwrap());
            assert!(header.len() < 8192);
        }
        assert!(String::from_utf8(header)
            .unwrap()
            .starts_with("CONNECT api.x.ai:443 HTTP/1.1"));
        socket
            .write_all(b"HTTP/1.1 502 Fixture\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let diagnostic = check_text_service(&configuration).await;
    assert_eq!(diagnostic.service, ServiceAvailability::Unavailable);
    assert_eq!(diagnostic.reason, Some(ConnectionCheckReason::Unreachable));
    server.await.unwrap();
}

#[test]
fn apple_probe_reports_resource_recovery_without_network_or_credential_advice() {
    for (label, expected) in [
        (
            "apple_speech_preparing",
            ConnectionCheckReason::AppleSpeechPreparing,
        ),
        (
            "apple_speech_status_failed",
            ConnectionCheckReason::AppleSpeechStatusFailed,
        ),
        (
            "apple_speech_service_unavailable",
            ConnectionCheckReason::AppleSpeechStatusFailed,
        ),
        (
            "apple_speech_assets_missing",
            ConnectionCheckReason::AppleSpeechAssetsMissing,
        ),
        (
            "apple_speech_language_unsupported",
            ConnectionCheckReason::AppleSpeechLanguageUnsupported,
        ),
        (
            "apple_speech_unavailable",
            ConnectionCheckReason::AppleSpeechUnavailable,
        ),
        (
            "apple_speech_setup_timeout",
            ConnectionCheckReason::AppleSpeechRecognitionFailed,
        ),
        (
            "unknown-private-error",
            ConnectionCheckReason::AppleSpeechRecognitionFailed,
        ),
    ] {
        assert_eq!(
            recognition_reason(&RecognitionClientError::Apple(label.into())),
            expected
        );
    }
}

use super::*;
use crate::clients::realtime::openai_realtime_client::OpenAIRealtimeClient;
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
    .with_network_proxy(crate::core::services::network_proxy::ProxyConfig {
        mode: crate::core::services::network_proxy::ProxyMode::Direct,
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
async fn speech_stage_does_not_contact_saved_translation_destination_and_reports_elapsed_time() {
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
    for (status, body, expected, injected) in [
        (
            200,
            r#"{"choices":[{"message":{"content":"こんにちは。"}}]}"#,
            ServiceAvailability::Available,
            false,
        ),
        (
            401,
            "private provider response",
            ServiceAvailability::Unavailable,
            false,
        ),
        (
            200,
            r#"{"choices":[{"message":{"content":" "}}]}"#,
            ServiceAvailability::Unavailable,
            false,
        ),
        (
            200,
            r#"{"choices":[{"message":{"content":"こんにちは。"}}]}"#,
            ServiceAvailability::Available,
            true,
        ),
        (
            401,
            "private provider response",
            ServiceAvailability::Unavailable,
            true,
        ),
        (
            200,
            r#"{"choices":[{"message":{"content":" "}}]}"#,
            ServiceAvailability::Unavailable,
            true,
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
            qwen_mt_model: Default::default(),
            source_language: SourceLanguage::English,
            credentials: TextTranslationProbeCredentials::Independent(
                TextTranslationCredentials::OpenAICompatible {
                    endpoint: format!("http://{address}/v1"),
                    model: "synthetic-text-model".into(),
                    api_key: "synthetic-text-key".into(),
                },
            ),
            target_language: TargetLanguage::Japanese,
            network_proxy: crate::core::services::network_proxy::ProxyConfig {
                mode: crate::core::services::network_proxy::ProxyMode::Direct,
                url: None,
            },
        };
        assert_eq!(
            text_probe_network_endpoint(&configuration).unwrap(),
            Some(format!("http://{address}/v1/chat/completions"))
        );
        let diagnostic = if injected {
            let network = ProviderNetwork::resolve(&configuration.network_proxy).unwrap();
            check_text_service_with_network(&configuration, network).await
        } else {
            check_text_service(&configuration).await
        };
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
            "apple_translation_language_unsupported",
            ConnectionCheckReason::AppleTranslationLanguageUnsupported,
        ),
        (
            "apple_translation_assets_missing",
            ConnectionCheckReason::AppleTranslationAssetsMissing,
        ),
        (
            "apple_translation_unavailable",
            ConnectionCheckReason::AppleTranslationUnavailable,
        ),
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
            "apple_speech_language_unsupported",
            ConnectionCheckReason::AppleSpeechLanguageUnsupported,
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
        let network =
            ProviderNetwork::resolve(&crate::core::services::network_proxy::ProxyConfig {
                mode: crate::core::services::network_proxy::ProxyMode::Direct,
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
        ConnectionCheckReason::QuotaExhausted
    );
    assert_eq!(
        deepl_reason(&DeepLError::Connection),
        ConnectionCheckReason::Unreachable
    );
}

#[test]
fn deepl_rejection_diagnostics_follow_shared_contracts() {
    let contract: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    for case in contract["textProbeRejections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["provider"] == "deepL")
    {
        let status = u16::try_from(case["httpStatus"].as_u64().unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(deepl_reason(&DeepLError::Rejected(status))).unwrap(),
            case["expected"],
            "{}",
            case["id"]
        );
    }
}

#[test]
fn unsupported_audio3_language_has_an_actionable_probe_reason() {
    assert_eq!(
        audio3_reason(&Audio3ASRClientError::Task(
            "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE".into()
        )),
        ConnectionCheckReason::UnsupportedLanguage
    );
    assert_eq!(
        serde_json::to_value(ConnectionCheckReason::UnsupportedLanguage).unwrap(),
        "unsupportedLanguage"
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
            Err(ConnectionCheckReason::InvalidConfiguration),
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
                    // A recoverable error explicitly tied to a different
                    // request is ignored by the production ready gate.
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
    let contract: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    let mut cases = vec![
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
    ];
    for case in contract["textProbeRejections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["provider"] == "deepLX")
    {
        let expected = match case["expected"].as_str().unwrap() {
            "concurrencyLimited" => ConnectionCheckReason::ConcurrencyLimited,
            "quotaExhausted" => ConnectionCheckReason::QuotaExhausted,
            "serviceRejected" => ConnectionCheckReason::ServiceRejected,
            other => panic!("unsupported probe fixture reason: {other}"),
        };
        cases.push((
            u16::try_from(case["httpStatus"].as_u64().unwrap()).unwrap(),
            case["body"].as_str().unwrap(),
            Err(expected),
        ));
    }
    for (status, body, expected) in cases {
        let body = body.to_owned();
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
        let network =
            ProviderNetwork::resolve(&crate::core::services::network_proxy::ProxyConfig {
                mode: crate::core::services::network_proxy::ProxyMode::Direct,
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
    .with_network_proxy(crate::core::services::network_proxy::ProxyConfig {
        mode: crate::core::services::network_proxy::ProxyMode::Custom,
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
