//! Bounded official DeepL HTTPS requests. Dropping the future cancels the request.

use crate::core::{
    models::{SourceLanguage, TargetLanguage},
    protocols::deepl::{self, DeepLError},
};
use futures_util::StreamExt;
use reqwest::header::{HeaderValue, AUTHORIZATION};
use std::time::Duration;

#[derive(Clone)]
pub struct DeepLClient {
    endpoint: url::Url,
    authorization: HeaderValue,
    client: reqwest::Client,
    source: SourceLanguage,
    target: TargetLanguage,
    timeout: Duration,
}

impl DeepLClient {
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.clone()
    }

    /// Rebuild only this fixed endpoint's connection pool before requests start.
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

    pub fn new(
        api_key: &str,
        source: SourceLanguage,
        target: TargetLanguage,
    ) -> Result<Self, DeepLError> {
        let endpoint =
            url::Url::parse(deepl::endpoint(api_key)?).map_err(|_| DeepLError::Connection)?;
        let mut authorization =
            HeaderValue::from_str(&format!("DeepL-Auth-Key {}", api_key.trim()))
                .map_err(|_| DeepLError::InvalidKey)?;
        authorization.set_sensitive(true);
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            endpoint,
            authorization,
            source,
            target,
            timeout: Duration::from_secs(8),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| DeepLError::Connection)?,
        })
    }

    pub async fn translate(
        &self,
        text: &str,
        source: Option<SourceLanguage>,
    ) -> Result<String, DeepLError> {
        let body = deepl::request_with_detected_source(text, self.source, source, self.target)?;
        tokio::time::timeout(self.timeout, async {
            crate::development_content::request(
                crate::development_content::RequestProtocol::DeepL,
                &body,
            );
            let response = self
                .client
                .post(self.endpoint.clone())
                .header(AUTHORIZATION, self.authorization.clone())
                .json(&body)
                .send()
                .await
                .map_err(|error| {
                    if error.is_timeout() {
                        DeepLError::Timeout
                    } else {
                        DeepLError::Connection
                    }
                })?;
            // Server error bodies may echo credentials or subtitle text.
            if !response.status().is_success() {
                return Err(DeepLError::Rejected(response.status().as_u16()));
            }
            const LIMIT: usize = 1024 * 1024;
            if response
                .content_length()
                .is_some_and(|length| length > LIMIT as u64)
            {
                return Err(DeepLError::TooLarge);
            }
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| DeepLError::Connection)?;
                if chunk.len() > LIMIT - bytes.len() {
                    return Err(DeepLError::TooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            deepl::decode(&bytes)
        })
        .await
        .map_err(|_| DeepLError::Timeout)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 2048];
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let length: usize = String::from_utf8_lossy(&bytes[..end])
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|length| length.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    return String::from_utf8(bytes).unwrap();
                }
            }
        }
    }

    fn fixture_client(address: std::net::SocketAddr) -> DeepLClient {
        let mut client = DeepLClient::new(
            "synthetic:fx",
            SourceLanguage::Automatic,
            TargetLanguage::Japanese,
        )
        .unwrap();
        // Only the test fixture replaces the otherwise fixed official URL.
        client.endpoint = url::Url::parse(&format!("http://{address}/v2/translate")).unwrap();
        client.client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        client
    }

    async fn fixture(
        status: u16,
        body: &str,
        chunked: bool,
    ) -> (DeepLClient, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = fixture_client(listener.local_addr().unwrap());
        let response = if chunked {
            format!("HTTP/1.1 {status} Fixture\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len())
        } else {
            format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/redirected\r\nConnection: close\r\n\r\n{body}", body.len())
        };
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let _ = socket.write_all(response.as_bytes()).await;
            request
        });
        (client, task)
    }

    #[tokio::test]
    async fn uses_deepl_auth_and_single_text_with_optional_detected_source() {
        for source in [
            None,
            Some(SourceLanguage::English),
            Some(SourceLanguage::French),
            Some(SourceLanguage::German),
        ] {
            let (client, server) = fixture(
                200,
                r#"{"translations":[{"detected_source_language":"EN","text":"合成テキスト"}]}"#,
                false,
            )
            .await;
            assert_eq!(
                client.translate("synthetic", source).await.unwrap(),
                "合成テキスト"
            );
            let request = server.await.unwrap();
            assert!(request.starts_with("POST /v2/translate HTTP/1.1"));
            assert!(request.contains("authorization: DeepL-Auth-Key synthetic:fx"));
            assert!(!request.contains("Bearer"));
            let mut expected = serde_json::json!({"text":["synthetic"],"target_lang":"JA"});
            if let Some(source) = source {
                expected["source_lang"] =
                    serde_json::json!(source.raw_value().to_ascii_uppercase());
            }
            let body: serde_json::Value =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body, expected);
        }
    }

    #[tokio::test]
    async fn rejects_http_errors_and_redirects_without_disclosing_server_content() {
        for (status, body, expected) in [
            (401, "private key text", DeepLError::Rejected(401)),
            (403, "private key text", DeepLError::Rejected(403)),
            (456, "private key text", DeepLError::Rejected(456)),
            (302, "", DeepLError::Rejected(302)),
            (200, "invalid", DeepLError::Response),
            (
                200,
                r#"{"translations":[{"text":""}]}"#,
                DeepLError::Response,
            ),
            (
                200,
                r#"{"translations":[{"text":"one"},{"text":"two"}]}"#,
                DeepLError::Response,
            ),
        ] {
            let (client, server) = fixture(status, body, false).await;
            let error = client.translate("synthetic", None).await.unwrap_err();
            assert_eq!(error, expected);
            assert!(!error.to_string().contains("private"));
            assert!(!error.diagnostic_label().contains("private"));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn bounds_declared_and_chunked_response_bodies() {
        let body = " ".repeat(1024 * 1024 + 1);
        for chunked in [false, true] {
            let (client, server) = fixture(200, &body, chunked).await;
            assert_eq!(
                client.translate("synthetic", None).await,
                Err(DeepLError::TooLarge)
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn stalled_body_times_out_and_cancellation_closes_the_request() {
        for cancel in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut client = fixture_client(listener.local_addr().unwrap());
            client.timeout = Duration::from_millis(150);
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                read_request(&mut socket).await;
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                    .await
                    .unwrap();
                ready_tx.send(()).unwrap();
                let mut byte = [0];
                let closed = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
                    .await
                    .expect("cancelled request should close its connection");
                assert!(matches!(closed, Ok(0) | Err(_)));
            });
            let task = tokio::spawn(async move { client.translate("synthetic", None).await });
            ready_rx.await.unwrap();
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                assert_eq!(task.await.unwrap(), Err(DeepLError::Timeout));
            }
            server.await.unwrap();
        }
    }
}
