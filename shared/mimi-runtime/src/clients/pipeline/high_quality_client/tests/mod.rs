//! Synthetic pipeline fixtures and behavior regressions grouped by owner.

use super::*;
use crate::core::models::TranslationMode;
use crate::core::services::provider::ProviderKind;

mod final_translation;
mod language_routes;
mod lifecycle;
mod preview;
mod recognition;
mod request_budget;
mod routing;

fn custom_configuration(
    provider: ProviderKind,
    target: TargetLanguage,
) -> LiveTranslationConfiguration {
    LiveTranslationConfiguration::with_credentials(
        provider,
        ProviderCredentials::CustomSpeech {
            endpoint: "wss://example.com/recognition".into(),
            model: "synthetic-recognition-model".into(),
            api_key: "synthetic-recognition-key".into(),
        },
        SourceLanguage::Automatic,
        target,
        TranslationMode::Turbo,
    )
}

fn independent_route_configuration(
    provider: ProviderKind,
    target: TargetLanguage,
) -> LiveTranslationConfiguration {
    if provider.is_custom_speech() {
        custom_configuration(provider, target)
    } else {
        let configuration = LiveTranslationConfiguration::for_provider(
            provider,
            "synthetic-speech-key",
            SourceLanguage::English,
            target,
            TranslationMode::Turbo,
        );
        if provider == ProviderKind::XAIRealtime {
            configuration.with_xai_speech(
                crate::core::services::provider::XAISpeechMode::AsrMt,
                "legacy-unused-model",
            )
        } else {
            configuration
        }
    }
}

fn test_client(
    target_language: TargetLanguage,
    threshold: usize,
) -> (HighQualityTranslationClient, ProviderEventReceiver) {
    let (events, receiver) = provider_event_channel();
    let client = HighQualityTranslationClient::new(
        "test-key",
        SourceLanguage::Japanese,
        target_language,
        QwenMTModel::Plus,
        Duration::from_secs(60),
        Duration::from_secs(60),
        threshold,
        events,
    )
    .unwrap();
    (client, receiver)
}

async fn read_synthetic_request(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut request = Vec::new();
    loop {
        let mut chunk = [0; 1024];
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0, "synthetic HTTP request ended before its body");
        request.extend_from_slice(&chunk[..count]);
        assert!(request.len() <= 16 * 1024);
        if let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&request[..header_end]).unwrap();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                return request;
            }
        }
    }
}

async fn reply_to_synthetic_request(socket: &mut tokio::net::TcpStream, status: u16, body: &str) {
    let _ = read_synthetic_request(socket).await;
    write_synthetic_response(socket, status, body).await;
}

async fn write_synthetic_response(socket: &mut tokio::net::TcpStream, status: u16, body: &str) {
    use tokio::io::AsyncWriteExt;
    let response = format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await.unwrap();
}
