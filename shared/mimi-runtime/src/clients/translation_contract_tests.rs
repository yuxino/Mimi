//! Optional authentication is asserted on real local HTTP requests, using the shared fixtures.
use super::{
    deeplx_client::DeepLXClient, openai_compatible_client::OpenAICompatibleClient,
    provider_network::ProviderNetwork,
};
use crate::core::{
    models::{SourceLanguage, TargetLanguage},
    network_proxy::{ProxyConfig, ProxyMode},
};
use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn exchange(listener: tokio::net::TcpListener, response_body: String) -> String {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 2048];
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0, "incomplete synthetic HTTP request");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 64 * 1024, "oversized synthetic HTTP request");
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let length: usize = String::from_utf8_lossy(&bytes[..end])
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|length| length.trim().parse().unwrap())
                })
                .unwrap();
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    socket.write_all(response.as_bytes()).await.unwrap();
    String::from_utf8(bytes).unwrap()
}

#[tokio::test]
async fn shared_optional_authorization_uses_actual_client_requests() {
    let fixtures: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../translation-contracts.json"
    )))
    .unwrap();
    assert_eq!(fixtures["schemaVersion"], 1);
    for case in fixtures["optionalAuthorization"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let provider = case["provider"].as_str().unwrap();
        let response_case = fixtures["responses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|response| response["provider"] == provider && response["expected"].is_string())
            .unwrap();
        let response_body = response_case["body"].as_str().unwrap().to_owned();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(3), exchange(listener, response_body))
                .await
                .expect("synthetic HTTP fixture timed out")
        });
        let network = ProviderNetwork::resolve(&ProxyConfig {
            mode: ProxyMode::Direct,
            url: None,
        })
        .unwrap();
        let key = case["apiKey"].as_str().unwrap();
        let translated = match provider {
            "openaiCompatible" => {
                let mut client = OpenAICompatibleClient::new(
                    &endpoint,
                    key,
                    "synthetic-model",
                    SourceLanguage::English,
                    TargetLanguage::SimplifiedChinese,
                )
                .unwrap();
                client.set_network(network).unwrap();
                client.translate("Hello.", None).await.map_err(|_| ())
            }
            "deepLX" => {
                let mut client = DeepLXClient::new(
                    &endpoint,
                    key,
                    SourceLanguage::English,
                    TargetLanguage::SimplifiedChinese,
                )
                .unwrap();
                client.set_network(network).unwrap();
                client.translate("Hello.", None).await.map_err(|_| ())
            }
            _ => panic!("unknown optional-authorization provider in {id}"),
        };
        let request = server.await.unwrap();
        assert_eq!(
            translated.unwrap(),
            response_case["expected"].as_str().unwrap(),
            "{id}"
        );
        let headers = request.split_once("\r\n\r\n").unwrap().0;
        let authorization = headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("authorization")
                .then_some(value.trim())
        });
        assert_eq!(authorization, case["expected"].as_str(), "{id}");
    }
}
