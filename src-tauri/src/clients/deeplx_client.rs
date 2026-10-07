//! Bounded, cancellable DeepLX HTTP requests. No redirects or server-message logging.
use crate::core::{
    models::{SourceLanguage, TargetLanguage},
    protocols::deeplx::{self, DeepLXError},
};
use futures_util::StreamExt;
use std::time::Duration;

#[derive(Clone)]
pub struct DeepLXClient {
    endpoint: url::Url,
    token: String,
    client: reqwest::Client,
    source: SourceLanguage,
    target: TargetLanguage,
    timeout: Duration,
}
impl DeepLXClient {
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
        endpoint: &str,
        token: &str,
        source: SourceLanguage,
        target: TargetLanguage,
    ) -> Result<Self, DeepLXError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            endpoint: deeplx::endpoint(endpoint)?,
            token: token.into(),
            source,
            target,
            timeout: Duration::from_secs(8),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| DeepLXError::Connection)?,
        })
    }
    pub async fn translate(
        &self,
        text: &str,
        source: Option<SourceLanguage>,
    ) -> Result<String, DeepLXError> {
        let body = deeplx::request_with_detected_source(text, self.source, source, self.target)?;
        tokio::time::timeout(self.timeout, async {
            crate::development_content::request(
                crate::development_content::RequestProtocol::DeepLX,
                &body,
            );
            let mut request = self.client.post(self.endpoint.clone()).json(&body);
            if !self.token.is_empty() {
                request = request.bearer_auth(&self.token);
            }
            let response = request.send().await.map_err(|error| {
                if error.is_timeout() {
                    DeepLXError::Timeout
                } else {
                    DeepLXError::Connection
                }
            })?;
            if !response.status().is_success() {
                return Err(DeepLXError::Rejected(response.status().as_u16()));
            }
            const LIMIT: usize = 1024 * 1024;
            if response
                .content_length()
                .is_some_and(|length| length > LIMIT as u64)
            {
                return Err(DeepLXError::TooLarge);
            }
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| DeepLXError::Connection)?;
                if chunk.len() > LIMIT - bytes.len() {
                    return Err(DeepLXError::TooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            deeplx::decode(&bytes)
        })
        .await
        .map_err(|_| DeepLXError::Timeout)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn fixture(
        status: u16,
        body: &str,
        token: &str,
        chunked: bool,
    ) -> (DeepLXClient, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let response = if chunked {
            format!("HTTP/1.1 {status} Fixture\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",body.len())
        } else {
            format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len())
        };
        let task = tokio::spawn(async move {
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
            let _ = socket.write_all(response.as_bytes()).await;
            String::from_utf8(bytes).unwrap()
        });
        let mut client = DeepLXClient::new(
            &format!("http://{address}/api"),
            token,
            SourceLanguage::Automatic,
            TargetLanguage::Japanese,
        )
        .unwrap();
        client.client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        (client, task)
    }
    #[tokio::test]
    async fn sends_only_text_and_optional_token_to_configured_destination() {
        for token in ["", "synthetic-token"] {
            let (client, server) =
                fixture(200, r#"{"code":200,"data":"合成テキスト"}"#, token, false).await;
            assert_eq!(
                client
                    .translate("synthetic", Some(SourceLanguage::English))
                    .await
                    .unwrap(),
                "合成テキスト"
            );
            let request = server.await.unwrap();
            assert!(request.starts_with("POST /api/translate HTTP/1.1"));
            assert_eq!(
                request.contains("authorization: Bearer synthetic-token"),
                !token.is_empty()
            );
            assert!(!request.contains("asr"));
            let body: serde_json::Value =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(
                body,
                serde_json::json!({"text":"synthetic","source_lang":"EN","target_lang":"JA"})
            );
        }
    }
    #[tokio::test]
    async fn automatic_source_keeps_detection_for_unmapped_reports() {
        for source in [SourceLanguage::Khmer, SourceLanguage::Asturian] {
            let (client, server) =
                fixture(200, r#"{"code":200,"data":"synthetic result"}"#, "", false).await;
            client.translate("synthetic", Some(source)).await.unwrap();
            let request = server.await.unwrap();
            let body: serde_json::Value =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(
                body,
                serde_json::json!({"text":"synthetic","source_lang":"auto","target_lang":"JA"})
            );
        }
    }

    #[tokio::test]
    async fn rejects_http_and_application_errors_without_disclosing_body() {
        for (status, body, expected) in [
            (401, "private token", DeepLXError::Rejected(401)),
            (
                200,
                r#"{"code":429,"message":"private text"}"#,
                DeepLXError::Rejected(429),
            ),
            (200, "invalid", DeepLXError::Response),
            (200, r#"{"code":200,"data":""}"#, DeepLXError::Response),
            (302, "", DeepLXError::Rejected(302)),
        ] {
            let (client, server) = fixture(status, body, "", false).await;
            let error = client.translate("synthetic", None).await.unwrap_err();
            assert_eq!(error, expected);
            assert!(!error.to_string().contains("private"));
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn stalled_body_times_out_and_dropping_request_cancels_it() {
        for cancel in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 4096];
                assert!(socket.read(&mut bytes).await.unwrap() > 0);
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                    .await
                    .unwrap();
                ready_tx.send(()).unwrap();
                std::future::pending::<()>().await;
            });
            let mut client = DeepLXClient::new(
                &format!("http://{address}"),
                "",
                SourceLanguage::English,
                TargetLanguage::Japanese,
            )
            .unwrap();
            client.client = reqwest::Client::builder().no_proxy().build().unwrap();
            client.timeout = Duration::from_millis(150);
            let task = tokio::spawn(async move { client.translate("synthetic", None).await });
            ready_rx.await.unwrap();
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                assert_eq!(task.await.unwrap(), Err(DeepLXError::Timeout));
            }
            server.abort();
        }
    }
    #[tokio::test]
    async fn body_size_is_bounded() {
        let body = " ".repeat(1024 * 1024 + 1);
        for chunked in [false, true] {
            let (client, server) = fixture(200, &body, "", chunked).await;
            assert_eq!(
                client.translate("synthetic", None).await,
                Err(DeepLXError::TooLarge)
            );
            server.await.unwrap();
        }
    }
}
