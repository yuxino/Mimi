//! Routing pipeline regressions.

use super::*;

#[tokio::test]
async fn facade_routes_recognition_and_translation_to_distinct_proxies() {
    use crate::clients::translation::translation_client::TranslationClient;
    use crate::core::services::network_proxy::{ProxyConfig, ProxyMode};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let speech_proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let text_proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination_address = destination.local_addr().unwrap();
    let speech_route = ProxyConfig {
        mode: ProxyMode::Custom,
        url: Some(format!("http://{}", speech_proxy.local_addr().unwrap())),
    };
    let text_route = ProxyConfig {
        mode: ProxyMode::Custom,
        url: Some(format!("http://{}", text_proxy.local_addr().unwrap())),
    };
    let configuration =
        custom_configuration(ProviderKind::CustomDashScopeASR, TargetLanguage::Japanese)
            .with_text_credentials(TextTranslationCredentials::OpenAICompatible {
                endpoint: format!("http://{destination_address}/v1"),
                model: "synthetic-model".into(),
                api_key: String::new(),
            })
            .with_stage_network_proxies(speech_route, text_route);
    let text_server = tokio::spawn(async move {
        let (mut socket, _) = text_proxy.accept().await.unwrap();
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
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        assert!(String::from_utf8_lossy(&bytes).starts_with(&format!(
            "POST http://{destination_address}/v1/chat/completions HTTP/1.1"
        )));
        let body = r#"{"choices":[{"message":{"content":"Synthetic translation"}}]}"#;
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
    let speech_server = tokio::spawn(async move {
        let (mut socket, _) = speech_proxy.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut buffer = [0; 1024];
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
            if request.windows(4).any(|part| part == b"\r\n\r\n") {
                break;
            }
        }
        assert!(String::from_utf8_lossy(&request).starts_with("CONNECT example.com:443 HTTP/1.1"));
        socket
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    let (sender, mut events) = provider_event_channel();
    let TranslationClient::HighQuality(client) =
        TranslationClient::new(&configuration, sender).unwrap()
    else {
        panic!("expected independent pipeline")
    };
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
            utterance_id: 1,
            text: "Synthetic source".into(),
            language: None,
        })
        .await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if matches!(
                events.recv().await,
                Some(LiveTranslateServerEvent::SubtitleConfirmedPair { .. })
            ) {
                break;
            }
        }
        text_server.await.unwrap();
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), client.connect())
            .await
            .unwrap()
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(3), speech_server)
        .await
        .unwrap()
        .unwrap();
    client.disconnect().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(20), destination.accept())
            .await
            .is_err()
    );
}

#[test]
fn independent_factory_freely_combines_recognizers_and_typed_text_destinations() {
    use crate::clients::translation::translation_client::TranslationClient;
    for provider in [
        ProviderKind::XAIRealtime,
        ProviderKind::AlibabaCloud,
        ProviderKind::CustomOpenAIASR,
    ] {
        for (credentials, expected) in [
            (
                TextTranslationCredentials::DeepL {
                    api_key: "synthetic-text:fx".into(),
                },
                "https://api-free.deepl.com/v2/translate",
            ),
            (
                TextTranslationCredentials::OpenAICompatible {
                    endpoint: "http://127.0.0.1:1/v1".into(),
                    model: "independent-model".into(),
                    api_key: "independent-key".into(),
                },
                "http://127.0.0.1:1/v1/chat/completions",
            ),
            (
                TextTranslationCredentials::XAI {
                    api_key: "independent-grok-key".into(),
                    model: "selected-grok-model".into(),
                },
                crate::core::services::provider::XAI_TEXT_ENDPOINT,
            ),
        ] {
            let configuration = independent_route_configuration(provider, TargetLanguage::Japanese)
                .with_text_credentials(credentials);
            let (events, _) = provider_event_channel();
            let TranslationClient::HighQuality(client) =
                TranslationClient::new_without_network(&configuration, events).unwrap()
            else {
                panic!("expected independent pipeline");
            };
            assert_eq!(client.network_endpoints().1.unwrap().as_str(), expected);
            if expected.contains("deepl") {
                assert!(matches!(
                    client.mt.as_ref(),
                    TextTranslationClient::DeepL(_)
                ));
            } else {
                assert!(matches!(
                    client.mt.as_ref(),
                    TextTranslationClient::OpenAICompatible(_)
                ));
            }
        }
    }
    let configuration =
        independent_route_configuration(ProviderKind::XAIRealtime, TargetLanguage::Japanese);
    let (events, _) = provider_event_channel();
    assert!(matches!(
        HighQualityTranslationClient::new_xai(&configuration, events),
        Err(QwenMTClientError::MissingTextTranslation)
    ));
    let configuration =
        independent_route_configuration(ProviderKind::XAIRealtime, TargetLanguage::Original)
            .with_text_credentials(TextTranslationCredentials::XAI {
                api_key: String::new(),
                model: String::new(),
            })
            .validated()
            .unwrap();
    assert!(configuration.text_credentials.is_none());
    let (events, _) = provider_event_channel();
    let client = HighQualityTranslationClient::new_xai(&configuration, events).unwrap();
    assert!(matches!(
        client.mt.as_ref(),
        TextTranslationClient::Disabled
    ));
    assert!(client.network_endpoints().1.is_none());
}

#[tokio::test]
async fn fixed_typed_destinations_use_the_text_proxy_without_contacting_the_speech_route() {
    use crate::clients::translation::translation_client::TranslationClient;
    use crate::core::services::network_proxy::{ProxyConfig, ProxyMode};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    for (provider, credentials, authority) in [
        (
            ProviderKind::XAIRealtime,
            TextTranslationCredentials::DeepL {
                api_key: "synthetic-text:fx".into(),
            },
            "api-free.deepl.com:443",
        ),
        (
            ProviderKind::CustomOpenAIASR,
            TextTranslationCredentials::XAI {
                api_key: "synthetic-grok-key".into(),
                model: "selected-grok-model".into(),
            },
            "api.x.ai:443",
        ),
        (
            ProviderKind::AlibabaCloud,
            TextTranslationCredentials::XAI {
                api_key: "synthetic-grok-key".into(),
                model: "selected-grok-model".into(),
            },
            "api.x.ai:443",
        ),
    ] {
        let speech = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let text = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let configuration = independent_route_configuration(provider, TargetLanguage::Japanese)
            .with_text_credentials(credentials)
            .with_stage_network_proxies(
                ProxyConfig {
                    mode: ProxyMode::Custom,
                    url: Some(format!("http://{}", speech.local_addr().unwrap())),
                },
                ProxyConfig {
                    mode: ProxyMode::Custom,
                    url: Some(format!("http://{}", text.local_addr().unwrap())),
                },
            );
        let (events, _) = provider_event_channel();
        let TranslationClient::HighQuality(client) =
            TranslationClient::new(&configuration, events).unwrap()
        else {
            panic!("expected independent pipeline");
        };
        let server = tokio::spawn(async move {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), text.accept())
                .await
                .unwrap()
                .unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(socket.read_u8().await.unwrap());
                assert!(header.len() < 8192);
            }
            let header = String::from_utf8(header).unwrap();
            assert!(header.starts_with(&format!("CONNECT {authority} HTTP/1.1")));
            assert!(!header.contains("synthetic-speech-key"));
            socket
                .write_all(b"HTTP/1.1 502 Fixture\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 1,
                text: "Synthetic route selection".into(),
                language: Some("en".into()),
            })
            .await;
        server.await.unwrap();
        client.disconnect().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), speech.accept())
                .await
                .is_err()
        );
    }
}

async fn xai_loopback_read_http(socket: &mut tokio::net::TcpStream) -> serde_json::Value {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 4096];
        let read = socket.read(&mut buffer).await.unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&buffer[..read]);
        assert!(bytes.len() <= 64 * 1024);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            assert!(headers.starts_with("POST /v1/chat/completions HTTP/1.1"));
            assert!(headers
                .to_ascii_lowercase()
                .contains("authorization: bearer synthetic-text-key"));
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse().unwrap())
                })
                .unwrap();
            if bytes.len() >= end + 4 + length {
                return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
            }
        }
    }
}

async fn xai_loopback_confirmed_pair(
    receiver: &mut ProviderEventReceiver,
    expected_id: u64,
    expected_source: &str,
    expected_translation: &str,
    expected_revision: u64,
) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let envelope = receiver.recv_with_revision().await.unwrap();
            match envelope.event {
                LiveTranslateServerEvent::SubtitleConfirmedPair {
                    source_utterance_id,
                    source,
                    translation,
                    ..
                } => {
                    assert_eq!(source_utterance_id, Some(expected_id));
                    assert_eq!(source, expected_source);
                    assert_eq!(translation, expected_translation);
                    assert_eq!(envelope.content_revision, expected_revision);
                    return;
                }
                LiveTranslateServerEvent::Error { code, .. } => {
                    panic!("Synthetic pipeline failed: {code}")
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn custom_recognition_original_mode_commits_without_constructing_or_accounting_for_mt() {
    for provider in [
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let configuration = custom_configuration(provider, TargetLanguage::Original);
        let (sender, mut events) = provider_event_channel();
        let client = HighQualityTranslationClient::new_custom(&configuration, sender).unwrap();
        assert!(matches!(
            client.mt.as_ref(),
            TextTranslationClient::Disabled
        ));
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 1,
                text: "Synthetic original sentence".into(),
                language: Some("en".into()),
            })
            .await;
        assert!(
            matches!(events.recv().await, Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: Some(1), source, translation, ..
            }) if source == "Synthetic original sentence" && translation == source)
        );
        assert!(client.state.lock().await.final_worker.is_none());
        assert!(client.state.lock().await.preview_task.is_none());
        assert_eq!(client.translation_latency(), None);
        let now = std::time::Instant::now();
        assert_eq!(
            client
                .state
                .lock()
                .await
                .preview_request_pacer
                .next_shared_start_at(now),
            now
        );
        assert_eq!(
            client
                .suspend_request_budget()
                .await
                .shared_cooldown_until(),
            None
        );
        client.disconnect().await;
    }
}

#[test]
fn custom_recognition_translation_cannot_reuse_the_recognition_key_as_a_qwen_key() {
    let configuration =
        custom_configuration(ProviderKind::CustomDashScopeASR, TargetLanguage::Japanese);
    let (sender, _events) = provider_event_channel();
    assert!(matches!(
        HighQualityTranslationClient::new_custom(&configuration, sender),
        Err(QwenMTClientError::MissingTextTranslation)
    ));
}

#[tokio::test]
async fn custom_recognition_uses_its_independent_chat_key_and_preserves_final_order() {
    assert_independent_chat_final_order(false).await;
    assert_independent_chat_final_order(true).await;
}

async fn assert_independent_chat_final_order(chatmock: bool) {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut configuration =
        custom_configuration(ProviderKind::CustomDashScopeASR, TargetLanguage::Japanese);
    configuration.text_credentials = Some(if chatmock {
        TextTranslationCredentials::ChatMock {
            endpoint: format!("http://{}/v1", listener.local_addr().unwrap()),
            model: "synthetic-translation-model".into(),
            api_key: "synthetic-translation-key".into(),
        }
    } else {
        TextTranslationCredentials::OpenAICompatible {
            endpoint: format!("http://{}/v1", listener.local_addr().unwrap()),
            model: "synthetic-translation-model".into(),
            api_key: "synthetic-translation-key".into(),
        }
    });
    let server = tokio::spawn(async move {
        for index in 1..=2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = String::from_utf8(read_synthetic_request(&mut socket).await).unwrap();
            assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer synthetic-translation-key"));
            assert!(!request.contains("synthetic-recognition-key"));
            let body_start = request.find("\r\n\r\n").unwrap() + 4;
            let json: serde_json::Value = serde_json::from_str(&request[body_start..]).unwrap();
            assert_eq!(json["model"], "synthetic-translation-model");
            assert_eq!(json["stream"], false);
            assert_eq!(
                json["messages"][1]["content"],
                format!("Synthetic sentence {index}")
            );
            let body = format!(
                r#"{{"choices":[{{"message":{{"content":"Synthetic translation {index}"}}}}]}}"#
            );
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
        }
    });
    let (sender, mut events) = provider_event_channel();
    let mut client = HighQualityTranslationClient::new_custom(&configuration, sender).unwrap();
    client
        .set_network(
            ProviderNetwork::resolve(&crate::core::services::network_proxy::ProxyConfig {
                mode: crate::core::services::network_proxy::ProxyMode::Direct,
                url: None,
            })
            .unwrap(),
        )
        .unwrap();
    for utterance_id in 1..=2 {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id,
                text: format!("Synthetic sentence {utterance_id}"),
                language: Some("en".into()),
            })
            .await;
    }
    let pairs = tokio::time::timeout(Duration::from_secs(5), async {
        let mut pairs = Vec::new();
        while pairs.len() < 2 {
            if let Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id,
                source,
                translation,
                ..
            }) = events.recv().await
            {
                pairs.push((source_utterance_id, source, translation));
            }
        }
        pairs
    })
    .await
    .unwrap();
    assert_eq!(
        pairs,
        vec![
            (
                Some(1),
                "Synthetic sentence 1".into(),
                "Synthetic translation 1".into()
            ),
            (
                Some(2),
                "Synthetic sentence 2".into(),
                "Synthetic translation 2".into()
            ),
        ]
    );
    server.await.unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn repeated_document_preview_and_finals_do_not_send_session_history_as_tm_list() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/translate", listener.local_addr().unwrap());
    let document = "Synthetic document with an earlier sentence and a later tail.";
    let prefix = "Synthetic document with an earlier sentence";
    let server = tokio::spawn(async move {
        for (input, output) in [
            (document, "Synthetic first complete document translation"),
            (prefix, "Synthetic shorter prefix translation"),
            (document, "Synthetic repeated complete document translation"),
        ] {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let request = read_synthetic_request(&mut socket).await;
            let body_start = request
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap()
                + 4;
            let json: serde_json::Value = serde_json::from_slice(&request[body_start..]).unwrap();
            assert_eq!(json["messages"].as_array().unwrap().len(), 1);
            assert_eq!(json["messages"][0]["content"], input);
            assert_eq!(json["model"], "qwen-mt-lite");
            assert_eq!(json["stream"], true);
            assert!(json["translation_options"].get("tm_list").is_none(),
                    "confirmed history must not be presented as a style example for a new prefix or final");
            assert!(json["translation_options"].get("domains").is_none());
            let event = serde_json::json!({"choices":[{"delta":{"content":output}}]});
            let response = format!("data: {event}\n\ndata: [DONE]\n\n");
            write_synthetic_response(&mut socket, 200, &response).await;
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let (sender, mut events) = provider_event_channel();
    let mut client = HighQualityTranslationClient::new(
        "synthetic-key",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        QwenMTModel::Lite,
        Duration::from_secs(60),
        Duration::from_secs(60),
        12,
        sender,
    )
    .unwrap();
    let TextTranslationClient::Qwen(mt, _) = Arc::make_mut(&mut client.mt) else {
        panic!("fixture uses Qwen")
    };
    mt.use_synthetic_endpoint(endpoint.parse().unwrap());
    let mut controller = crate::core::lifecycle::session::TranslationSessionController::default();
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
            text: document.into(),
            language: Some("en".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            let confirmed = matches!(
                event,
                LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
            );
            controller.handle(event);
            if confirmed {
                break;
            }
        }
    })
    .await
    .unwrap();
    client
        .wait_for_final_translations(Duration::from_secs(1))
        .await;
    assert_eq!(
        controller.state.subtitles.translation.text,
        "Synthetic first complete document translation"
    );
    controller.clear_subtitles();
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceDraft {
            text: prefix.into(),
            language: Some("en".into()),
        })
        .await;
    let revision = client.state.lock().await.draft_revision;
    client
        .start_preview(prefix.into(), Some("en".into()), revision)
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            let completed = matches!(event, LiveTranslateServerEvent::SubtitlePreviewPair { .. });
            controller.handle(event);
            if completed {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(controller.state.subtitles.history.is_empty());
    assert_eq!(
        controller.state.subtitles.preview_pair,
        Some(crate::core::models::PreviewSubtitlePair {
            utterance_id: None,
            source: prefix.into(),
            translation: "Synthetic shorter prefix translation".into(),
        })
    );
    client
        .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
            text: document.into(),
            language: Some("en".into()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            let confirmed = matches!(
                event,
                LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
            );
            controller.handle(event);
            if confirmed {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(controller.state.subtitles.preview_pair.is_none());
    assert_eq!(controller.state.subtitles.source.text, document);
    assert_eq!(
        controller.state.subtitles.translation.text,
        "Synthetic repeated complete document translation"
    );
    client
        .wait_for_final_translations(Duration::from_secs(1))
        .await;
    server.await.unwrap();
    client.disconnect().await;
}

#[tokio::test]
async fn deeplx_finals_preserve_pairs_and_order_through_real_http() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for translation in ["first translated", "second translated"] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            tokio::time::sleep(Duration::from_millis(40)).await;
            let body = serde_json::json!({"code":200,"data":translation}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let (sender, mut receiver) = provider_event_channel();
    let client = HighQualityTranslationClient::new_deeplx(
        "synthetic-asr",
        &format!("http://{address}"),
        "",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender,
    )
    .unwrap();
    for text in ["first synthetic sentence.", "second synthetic sentence."] {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: text.into(),
                language: Some("en".into()),
            })
            .await;
    }
    let mut pairs = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        while pairs.len() < 2 {
            if let Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                source,
                translation,
                ..
            }) = receiver.recv().await
            {
                pairs.push((source, translation));
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        pairs,
        vec![
            (
                "first synthetic sentence.".into(),
                "first translated".into()
            ),
            (
                "second synthetic sentence.".into(),
                "second translated".into()
            )
        ]
    );
    let latency = client.translation_latency().unwrap();
    assert_eq!(latency.kind, TranslationLatencyKind::Request);
    assert!(latency.milliseconds >= 40);
    client.disconnect().await;
    assert_eq!(client.translation_latency(), None);
    server.await.unwrap();
}

#[tokio::test]
async fn openai_compatible_finals_preserve_pairs_and_order_through_real_http() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for translation in ["first translated", "second translated"] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            tokio::time::sleep(Duration::from_millis(40)).await;
            let body = serde_json::json!({"choices":[{"finish_reason":"stop","message":{"content":translation}}]}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let (sender, mut receiver) = provider_event_channel();
    let mut client = HighQualityTranslationClient::new_openai_compatible(
        "synthetic-asr",
        &format!("http://{address}"),
        "synthetic-translation-key",
        "synthetic-model",
        SourceLanguage::English,
        TargetLanguage::Japanese,
        sender,
    )
    .unwrap();
    client
        .set_network(
            ProviderNetwork::resolve(&crate::core::services::network_proxy::ProxyConfig {
                mode: crate::core::services::network_proxy::ProxyMode::Direct,
                url: None,
            })
            .unwrap(),
        )
        .unwrap();
    for text in ["first synthetic sentence.", "second synthetic sentence."] {
        client
            .handle_asr_event(LiveTranslateServerEvent::SourceFinal {
                text: text.into(),
                language: Some("en".into()),
            })
            .await;
    }
    let mut pairs = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        while pairs.len() < 2 {
            if let Some(LiveTranslateServerEvent::SubtitleConfirmedPair {
                source,
                translation,
                ..
            }) = receiver.recv().await
            {
                pairs.push((source, translation));
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        pairs,
        vec![
            (
                "first synthetic sentence.".into(),
                "first translated".into()
            ),
            (
                "second synthetic sentence.".into(),
                "second translated".into()
            )
        ]
    );
    let latency = client.translation_latency().unwrap();
    assert_eq!(latency.kind, TranslationLatencyKind::Request);
    assert!(latency.milliseconds >= 40);
    client.disconnect().await;
    assert_eq!(client.translation_latency(), None);
    server.await.unwrap();
}

#[tokio::test]
#[allow(clippy::result_large_err)] // Tungstenite fixes the handshake callback error type.
async fn xai_stt_chunks_reach_real_mt_before_speech_final_and_original_never_requests_http() {
    use crate::core::services::network_proxy::{ProxyConfig, ProxyMode};
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;
    use tokio_tungstenite::{accept_hdr_async, tungstenite::Message};

    for target in [TargetLanguage::Japanese, TargetLanguage::Original] {
        let speech = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let text = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let configuration = LiveTranslationConfiguration::for_provider(
            ProviderKind::XAIRealtime,
            "synthetic-speech-key",
            SourceLanguage::English,
            target,
            TranslationMode::Turbo,
        )
        .with_xai_speech(
            crate::core::services::provider::XAISpeechMode::AsrMt,
            "legacy-unused-model",
        )
        .with_text_credentials(TextTranslationCredentials::OpenAICompatible {
            endpoint: format!("http://{}/v1", text.local_addr().unwrap()),
            api_key: "synthetic-text-key".into(),
            model: "synthetic-text-model".into(),
        });
        let (events, mut receiver) = provider_event_channel();
        let mut client = HighQualityTranslationClient::new_xai(&configuration, events).unwrap();
        let RecognitionClient::Xai(asr) = &mut client.asr_client else {
            panic!("expected xAI ASR");
        };
        asr.set_loopback_for_test(speech.local_addr().unwrap());
        client.stable_draft_delay = Duration::from_secs(60);
        client.maximum_wait_delay = Duration::from_secs(60);
        client
            .set_network(
                ProviderNetwork::resolve(&ProxyConfig {
                    mode: ProxyMode::Direct,
                    url: None,
                })
                .unwrap(),
            )
            .unwrap();
        let (requests, mut received_requests) = mpsc::channel::<String>(8);
        let text_server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = text.accept().await.unwrap();
                let request = xai_loopback_read_http(&mut socket).await;
                assert_eq!(request["model"], "synthetic-text-model");
                assert_eq!(request["stream"], false);
                let source = request["messages"][1]["content"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                requests.send(source.clone()).await.unwrap();
                let body = json!({"choices":[{"finish_reason":"stop","message":{"content":format!("translated {source}")}}]}).to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let (frames, mut pending_frames) = mpsc::channel::<Value>(8);
        let speech_server = tokio::spawn(async move {
            for connection in 0..2 {
                let mut socket = accept_hdr_async(
                    speech.accept().await.unwrap().0,
                    |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                     response| {
                        assert_eq!(
                            request.headers().get("Authorization").unwrap(),
                            "Bearer synthetic-speech-key"
                        );
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                socket
                    .send(Message::Text(
                        json!({"type":"transcript.created"}).to_string().into(),
                    ))
                    .await
                    .unwrap();
                assert_eq!(
                    socket.next().await.unwrap().unwrap(),
                    Message::Binary(vec![1, 0, 2, 0].into())
                );
                loop {
                    tokio::select! {
                        frame = pending_frames.recv() => {
                            socket.send(Message::Text(frame.unwrap().to_string().into())).await.unwrap();
                        }
                        message = socket.next() => {
                            match message.unwrap().unwrap() {
                                Message::Text(text) => {
                                    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["type"], "audio.done");
                                    assert_eq!(connection, 1, "Stop must follow the first connection's clear boundary");
                                    socket.send(Message::Text(json!({"type":"transcript.done","text":"fourth synthetic chunk","duration":3.0}).to_string().into())).await.unwrap();
                                    return;
                                }
                                Message::Close(_) => break,
                                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                                other => panic!("Unexpected synthetic STT wire message: {other:?}"),
                            }
                        }
                    }
                }
            }
        });
        let partial = |text: &str,
                       is_final: bool,
                       speech_final: bool,
                       start: f64,
                       duration: f64| json!({"type":"transcript.partial","text":text,"is_final":is_final,"speech_final":speech_final,"start":start,"duration":duration});
        client.connect().await.unwrap();
        client.send_audio(&[1, 0, 2, 0]).await.unwrap();
        frames
            .send(partial("first synthetic draft", false, false, 0.0, 1.0))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match receiver.recv().await.unwrap() {
                    LiveTranslateServerEvent::SourceUtteranceDraft {
                        utterance_id: 1,
                        text,
                        ..
                    } if text == "first synthetic draft" => break,
                    LiveTranslateServerEvent::SubtitleConfirmedPair { .. } => {
                        panic!("An interim STT result became a durable subtitle")
                    }
                    LiveTranslateServerEvent::Error { code, .. } => {
                        panic!("Synthetic pipeline failed: {code}")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), received_requests.recv())
                .await
                .is_err()
        );

        for (id, source) in [(1, "first synthetic chunk"), (2, "second synthetic chunk")] {
            frames
                .send(partial(source, true, false, (id - 1) as f64 * 3.0, 3.0))
                .await
                .unwrap();
            if target.translates_audio() {
                // This request is received while the server has sent no
                // speech_final and the client has not begun stopping.
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(2), received_requests.recv())
                        .await
                        .unwrap()
                        .unwrap(),
                    source
                );
            }
            let translated = if target.translates_audio() {
                format!("translated {source}")
            } else {
                source.to_owned()
            };
            xai_loopback_confirmed_pair(&mut receiver, id, source, &translated, 0).await;
        }
        frames
            .send(partial(
                "first synthetic chunk second synthetic chunk",
                true,
                true,
                0.0,
                6.0,
            ))
            .await
            .unwrap();
        // A following real chunk is a processing barrier: any duplicate
        // summary request or confirmation would arrive before this pair.
        let source = "third synthetic chunk";
        frames
            .send(partial(source, true, false, 6.0, 3.0))
            .await
            .unwrap();
        if target.translates_audio() {
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), received_requests.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                source
            );
        }
        let translated = if target.translates_audio() {
            format!("translated {source}")
        } else {
            source.to_owned()
        };
        xai_loopback_confirmed_pair(&mut receiver, 3, source, &translated, 0).await;
        assert!(received_requests.try_recv().is_err());

        // The real pipeline retains its replay watermark across clear.
        // The replacement xAI socket must therefore continue at ID 4.
        assert_eq!(client.clear_content().await, 1);
        client.send_audio(&[1, 0, 2, 0]).await.unwrap();
        let source = "fourth synthetic chunk";
        frames
            .send(partial(source, true, false, 0.0, 3.0))
            .await
            .unwrap();
        if target.translates_audio() {
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), received_requests.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                source
            );
        }
        let translated = if target.translates_audio() {
            format!("translated {source}")
        } else {
            source.to_owned()
        };
        xai_loopback_confirmed_pair(&mut receiver, 4, source, &translated, 1).await;
        client.finish().await;
        speech_server.await.unwrap();
        while let Ok(event) = receiver.try_recv() {
            assert!(!matches!(
                event,
                LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
            ));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), received_requests.recv())
                .await
                .is_err()
        );
        if target == TargetLanguage::Original {
            assert_eq!(client.translation_latency(), None);
        }
        text_server.abort();
        let _ = text_server.await;
    }
}
