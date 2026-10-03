//! Cancellable, bounded Chat Completions requests to a configured text service.

use crate::core::{
    models::{SourceLanguage, TargetLanguage},
    protocols::openai_compatible::{self, OpenAICompatibleError, MAX_RESPONSE_BYTES},
};
use futures_util::StreamExt;
use std::time::Duration;

#[derive(Clone)]
pub struct OpenAICompatibleClient {
    endpoint: url::Url,
    api_key: String,
    model: String,
    source: SourceLanguage,
    target: TargetLanguage,
    client: reqwest::Client,
    timeout: Duration,
}

impl OpenAICompatibleClient {
    pub fn new(
        endpoint: &str,
        api_key: &str,
        model: &str,
        source: SourceLanguage,
        target: TargetLanguage,
    ) -> Result<Self, OpenAICompatibleError> {
        if api_key.chars().any(char::is_control) {
            return Err(OpenAICompatibleError::APIKey);
        }
        let api_key = api_key.trim();
        if api_key.len() > 4096
            || reqwest::header::HeaderValue::from_str(&format!("Bearer {api_key}")).is_err()
        {
            return Err(OpenAICompatibleError::APIKey);
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            endpoint: openai_compatible::endpoint(endpoint)?,
            api_key: api_key.into(),
            model: openai_compatible::validate_model(model)?,
            source,
            target,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| OpenAICompatibleError::Connection)?,
            timeout: Duration::from_secs(8),
        })
    }

    /// Freeze the same validated route used by recognition before work starts.
    pub fn set_network(
        &mut self,
        network: super::provider_network::ProviderNetwork,
    ) -> Result<(), super::provider_network::ProviderNetworkError> {
        self.client = network
            .http_client_builder(&self.endpoint)?
            .build()
            .map_err(|_| crate::core::network_proxy::ProxyConfigError::BuilderFailed)?;
        Ok(())
    }

    pub async fn translate(
        &self,
        text: &str,
        source: Option<SourceLanguage>,
    ) -> Result<String, OpenAICompatibleError> {
        let body = openai_compatible::request(
            text,
            source.unwrap_or(self.source),
            self.target,
            &self.model,
        )?;
        tokio::time::timeout(self.timeout, async {
            crate::development_content::request(
                crate::development_content::RequestProtocol::OpenaiCompatible,
                &body,
            );
            let mut request = self.client.post(self.endpoint.clone()).json(&body);
            if !self.api_key.is_empty() {
                request = request.bearer_auth(&self.api_key);
            }
            let response = request.send().await.map_err(|error| {
                if error.is_timeout() {
                    OpenAICompatibleError::Timeout
                } else {
                    OpenAICompatibleError::Connection
                }
            })?;
            if !response.status().is_success() {
                return Err(OpenAICompatibleError::Rejected(response.status().as_u16()));
            }
            if response
                .content_length()
                .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
            {
                return Err(OpenAICompatibleError::TooLarge);
            }
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| OpenAICompatibleError::Connection)?;
                if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
                    return Err(OpenAICompatibleError::TooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            openai_compatible::decode(&bytes)
        })
        .await
        .map_err(|_| OpenAICompatibleError::Timeout)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::provider_network::ProviderNetwork;
    use crate::core::network_proxy::{ProxyConfig, ProxyMode};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
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
                    return String::from_utf8(bytes).unwrap();
                }
            }
        }
    }

    fn direct_client(endpoint: &str) -> OpenAICompatibleClient {
        let mut client = OpenAICompatibleClient::new(
            endpoint,
            "synthetic-translation-key",
            "synthetic-model",
            SourceLanguage::Automatic,
            TargetLanguage::Japanese,
        )
        .unwrap();
        client
            .set_network(
                ProviderNetwork::resolve(&ProxyConfig {
                    mode: ProxyMode::Direct,
                    url: None,
                })
                .unwrap(),
            )
            .unwrap();
        client
    }

    async fn fixture(
        status: u16,
        body: &str,
        chunked: bool,
        extra_headers: &str,
    ) -> (OpenAICompatibleClient, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let response = if chunked {
            format!("HTTP/1.1 {status} Fixture\r\n{extra_headers}Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len())
        } else {
            format!("HTTP/1.1 {status} Fixture\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        };
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let _ = socket.write_all(response.as_bytes()).await;
            request
        });
        (direct_client(&format!("http://{address}/proxy/v1")), task)
    }

    #[tokio::test]
    async fn sends_model_messages_and_only_the_translation_key() {
        let (client, server) = fixture(
            200,
            r#"{"choices":[{"message":{"content":"Synthetic translation"}}]}"#,
            false,
            "",
        )
        .await;
        assert_eq!(
            client
                .translate("Synthetic source", Some(SourceLanguage::English))
                .await
                .unwrap(),
            "Synthetic translation"
        );
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /proxy/v1/chat/completions HTTP/1.1"));
        assert!(request.contains("authorization: Bearer synthetic-translation-key"));
        assert!(!request.contains("asr"));
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["model"], "synthetic-model");
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][1]["content"], "Synthetic source");
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("English into Japanese"));
    }

    #[tokio::test]
    async fn respects_the_configured_proxy_for_custom_destinations() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
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
            request
        });
        let mut client = direct_client("http://127.0.0.1:9/v1");
        client
            .set_network(
                ProviderNetwork::resolve(&ProxyConfig {
                    mode: ProxyMode::Custom,
                    url: Some(proxy),
                })
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            client.translate("Synthetic source", None).await.unwrap(),
            "Synthetic translation"
        );
        assert!(server
            .await
            .unwrap()
            .starts_with("POST http://127.0.0.1:9/v1/chat/completions HTTP/1.1"));
    }

    #[tokio::test]
    async fn rejects_http_and_malformed_responses_without_disclosing_body() {
        for (status, body, expected) in [
            (
                401,
                "private provider data",
                OpenAICompatibleError::Rejected(401),
            ),
            (
                403,
                "private provider data",
                OpenAICompatibleError::Rejected(403),
            ),
            (
                429,
                "private provider data",
                OpenAICompatibleError::Rejected(429),
            ),
            (200, "invalid", OpenAICompatibleError::Response),
            (
                200,
                r#"{"choices":[{"message":{"content":""}}]}"#,
                OpenAICompatibleError::Response,
            ),
            (
                200,
                r#"{"error":{"message":"private provider data"}}"#,
                OpenAICompatibleError::Response,
            ),
        ] {
            let (client, server) = fixture(status, body, false, "").await;
            let error = client
                .translate("Synthetic source", None)
                .await
                .unwrap_err();
            assert_eq!(error, expected);
            assert!(!error.to_string().contains("private"));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn never_follows_redirects_to_another_destination() {
        let redirect = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let location = format!(
            "Location: http://{}/leak\r\n",
            redirect.local_addr().unwrap()
        );
        let (client, server) = fixture(302, "", false, &location).await;
        assert_eq!(
            client.translate("Synthetic source", None).await,
            Err(OpenAICompatibleError::Rejected(302))
        );
        server.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), redirect.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn bodies_and_translated_text_are_bounded_without_content_length() {
        let text = "x".repeat(openai_compatible::MAX_TRANSLATION_BYTES + 1);
        let large_translation =
            serde_json::json!({"choices":[{"message":{"content":text}}]}).to_string();
        for body in [" ".repeat(MAX_RESPONSE_BYTES + 1), large_translation] {
            for chunked in [false, true] {
                let (client, server) = fixture(200, &body, chunked, "").await;
                assert_eq!(
                    client.translate("Synthetic source", None).await,
                    Err(OpenAICompatibleError::TooLarge)
                );
                server.await.unwrap();
            }
        }
    }

    #[tokio::test]
    async fn stalled_body_has_a_total_deadline_and_dropping_work_closes_transport() {
        for cancel in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut client = direct_client(&format!("http://{}", listener.local_addr().unwrap()));
            client.timeout = Duration::from_millis(100);
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                read_request(&mut socket).await;
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                    .await
                    .unwrap();
                ready_tx.send(()).unwrap();
                let mut buffer = [0; 1];
                // Cancellation may close TCP with FIN or RST; both prove the
                // stalled transport ended. Other errors and timeouts still fail.
                match tokio::time::timeout(Duration::from_secs(2), socket.read(&mut buffer))
                    .await
                    .unwrap()
                {
                    Ok(count) => count,
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => 0,
                    Err(error) => panic!("unexpected transport close error: {error}"),
                }
            });
            let task =
                tokio::spawn(async move { client.translate("Synthetic source", None).await });
            ready_rx.await.unwrap();
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                assert_eq!(task.await.unwrap(), Err(OpenAICompatibleError::Timeout));
            }
            assert_eq!(server.await.unwrap(), 0);
        }
    }
    #[tokio::test]
    async fn chatmock_can_translate_without_a_bearer_key_and_never_exposes_reasoning() {
        let (mut client, server) = fixture(
            200,
            r#"{"choices":[{"finish_reason":"stop","message":{"content":"<think>Synthetic reasoning.</think> Synthetic translation"}}]}"#,
            false,
            "",
        ).await;
        client.api_key.clear();
        let anonymous = OpenAICompatibleClient::new(
            client.endpoint.as_str(),
            "",
            "synthetic-model",
            SourceLanguage::Automatic,
            TargetLanguage::Japanese,
        )
        .unwrap();
        assert!(anonymous.api_key.is_empty());
        assert_eq!(
            client.translate("Synthetic source", None).await.unwrap(),
            "Synthetic translation"
        );
        let request = server.await.unwrap();
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["stream"], false);
        assert!(!body.as_object().unwrap().contains_key("reasoning_compat"));
    }

    #[tokio::test]
    async fn chatmock_malformed_or_incomplete_reasoning_fails_without_provider_content() {
        for body in [
            r#"{"choices":[{"finish_reason":"stop","message":{"content":"<think>private unfinished reasoning"}}]}"#,
            r#"{"choices":[{"finish_reason":null,"message":{"content":"private unfinished text"}}]}"#,
            r#"{"choices":[{"finish_reason":"tool_calls","message":{"content":"private tool description"}}]}"#,
        ] {
            let (client, server) = fixture(200, body, false, "").await;
            let error = client
                .translate("Synthetic source", None)
                .await
                .unwrap_err();
            assert_eq!(error, OpenAICompatibleError::Response);
            assert!(!error.to_string().contains("private"));
            server.await.unwrap();
        }
    }

    #[test]
    fn optional_bearer_keys_remain_bounded_and_reject_control_characters() {
        for key in ["", "  ", "synthetic-key"] {
            assert!(OpenAICompatibleClient::new(
                "http://127.0.0.1:8000/v1",
                key,
                "synthetic-model",
                SourceLanguage::English,
                TargetLanguage::Japanese
            )
            .is_ok());
        }
        for key in ["\n", "synthetic\r\nkey", &"x".repeat(4097)] {
            assert!(matches!(
                OpenAICompatibleClient::new(
                    "http://127.0.0.1:8000/v1",
                    key,
                    "synthetic-model",
                    SourceLanguage::English,
                    TargetLanguage::Japanese
                ),
                Err(OpenAICompatibleError::APIKey)
            ));
        }
    }
}
