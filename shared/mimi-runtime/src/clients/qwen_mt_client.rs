//! Qwen-MT HTTP client for streaming and non-streaming chat completions.

use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::qwen_mt::{
    QwenMTClientError, QwenMTEndpoint, QwenMTMemoryPair, QwenMTModel, QwenMTProtocolError,
    QwenMTRejectionCategory, QwenMTRequestEncoder, QwenMTResponseDecoder, QwenMTStreamDecoder,
    QwenMTTerm,
};
use crate::pipeline_log;
use futures_util::StreamExt;
use std::time::Duration;

// A sentence translation should stay far below these bounds. Enforce byte
// limits even when Content-Length is absent or the server never sends '\n'.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_TRANSLATION_BYTES: usize = 64 * 1024;
const SSE_TAIL_DRAIN_TIMEOUT: Duration = Duration::from_millis(20);
const MAX_SSE_TAIL_DRAIN_BYTES: usize = 8 * 1024;
const MAX_SSE_TAIL_DRAIN_CHUNKS: usize = 32;

#[derive(Clone)]
pub struct QwenMTClient {
    endpoint: QwenMTEndpoint,
    api_key: String,
    source_language: SourceLanguage,
    target_language: TargetLanguage,
    model: QwenMTModel,
    domain_hint: Option<String>,
    terms: Vec<QwenMTTerm>,
    client: reqwest::Client,
    streaming_timeout: Duration,
}

impl QwenMTClient {
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.url.clone()
    }

    #[cfg(test)]
    pub(super) fn use_synthetic_endpoint(&mut self, endpoint: url::Url) {
        assert_eq!(endpoint.scheme(), "http");
        assert!(matches!(
            endpoint.host_str(),
            Some("127.0.0.1" | "[::1]" | "localhost")
        ));
        self.endpoint.url = endpoint;
        self.client = http_client_builder().no_proxy().build().unwrap();
    }

    /// Rebuild only this fixed endpoint's connection pool before requests start.
    pub fn set_network(
        &mut self,
        network: super::provider_network::ProviderNetwork,
    ) -> Result<(), super::provider_network::ProviderNetworkError> {
        self.client = network
            .http_client_builder(&self.endpoint.url)?
            .build()
            .map_err(|_| crate::core::network_proxy::ProxyConfigError::BuilderFailed)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        api_key: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        model: QwenMTModel,
        domain_hint: Option<String>,
        terms: Vec<QwenMTTerm>,
        streaming_timeout: Duration,
    ) -> Result<Self, QwenMTClientError> {
        let trimmed_key = api_key.trim();
        if trimmed_key.is_empty() {
            return Err(QwenMTClientError::MissingAPIKey);
        }
        Ok(Self {
            endpoint: QwenMTEndpoint::new().map_err(|_| QwenMTClientError::InvalidHTTPResponse)?,
            api_key: trimmed_key.to_string(),
            source_language,
            target_language,
            model,
            domain_hint,
            terms,
            client: http_client_builder()
                .build()
                .map_err(|_| QwenMTClientError::InvalidHTTPResponse)?,
            streaming_timeout,
        })
    }

    pub async fn translate(
        &self,
        text: &str,
        source_language_override: Option<SourceLanguage>,
        translation_memory: &[QwenMTMemoryPair],
    ) -> Result<String, QwenMTClientError> {
        let body = self.make_body(text, source_language_override, false, translation_memory)?;
        let timeout = if self.model == QwenMTModel::Plus {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(10)
        };
        let response = self
            .client
            .post(self.endpoint.url.clone())
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .timeout(timeout)
            .body(body)
            .send()
            .await
            .map_err(|_| QwenMTClientError::RequestTimedOut)?;

        let status = response.status();
        let bytes = read_bounded_response(response).await;
        if !status.is_success() {
            return Err(rejected_error(status.as_u16(), &bytes.unwrap_or_default()));
        }
        let bytes = bytes?;
        QwenMTResponseDecoder::decode(&String::from_utf8_lossy(&bytes)).map_err(|error| match error
        {
            QwenMTProtocolError::InvalidJSON => QwenMTClientError::InvalidHTTPResponse,
            QwenMTProtocolError::MissingTranslation => QwenMTClientError::RequestFailed {
                status_code: status.as_u16(),
                message: "Qwen-MT returned no translated text.".into(),
            },
            other => QwenMTClientError::RequestFailed {
                status_code: status.as_u16(),
                message: other.to_string(),
            },
        })
    }

    /// Streams the translation, invoking `on_partial` with the accumulated
    /// text after every chunk. The whole request must finish within
    /// `streaming_timeout`.
    pub async fn translate_streaming(
        &self,
        text: &str,
        source_language_override: Option<SourceLanguage>,
        translation_memory: &[QwenMTMemoryPair],
        on_partial: impl Fn(String) + Send + Sync,
    ) -> Result<String, QwenMTClientError> {
        let body = self.make_body(text, source_language_override, true, translation_memory)?;
        let timeout = self.streaming_timeout;
        let request = self
            .client
            .post(self.endpoint.url.clone())
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .body(body);

        let (translated, response) =
            tokio::time::timeout(timeout, async { self.stream(request, &on_partial).await })
                .await
                .map_err(|_| QwenMTClientError::RequestTimedOut)??;
        if let Some(response) = response {
            // The completed result has already passed the request deadline.
            // A tiny transport cleanup must not retroactively turn [DONE]
            // near that deadline into a failed translation.
            drain_completed_sse_tail(&mut response.bytes_stream()).await;
        }
        Ok(translated)
    }

    async fn stream(
        &self,
        request: reqwest::RequestBuilder,
        on_partial: &(impl Fn(String) + Send + Sync),
    ) -> Result<(String, Option<reqwest::Response>), QwenMTClientError> {
        let mut response = request
            .send()
            .await
            .map_err(|_| QwenMTClientError::RequestTimedOut)?;
        let status = response.status();
        if !status.is_success() {
            let bytes = read_bounded_response(response).await.unwrap_or_default();
            return Err(rejected_error(status.as_u16(), &bytes));
        }

        let mut decoder = BoundedSseResponse::default();
        let mut completed = false;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| QwenMTClientError::InvalidHTTPResponse)?
        {
            if decoder.push(&chunk, on_partial)? {
                // [DONE] ends the translation, not necessarily the HTTP/1
                // body. Dropping before a slightly later chunk terminator
                // makes Hyper close a healthy connection instead of pooling
                // it. Consume only a tiny, bounded tail; an incomplete/error
                // tail must never turn an already complete translation into
                // a timeout or block until the streaming request deadline.
                completed = true;
                break;
            }
        }

        let trimmed = decoder.finish(on_partial)?;
        if trimmed.is_empty() {
            return Err(QwenMTClientError::RequestFailed {
                status_code: status.as_u16(),
                message: "Qwen-MT returned no translated text.".into(),
            });
        }
        Ok((trimmed, completed.then_some(response)))
    }

    fn make_body(
        &self,
        text: &str,
        source_language_override: Option<SourceLanguage>,
        stream: bool,
        translation_memory: &[QwenMTMemoryPair],
    ) -> Result<String, QwenMTClientError> {
        let trimmed_text = text.trim().to_string();
        if trimmed_text.is_empty() {
            return Err(QwenMTClientError::RequestFailed {
                status_code: 0,
                message: "Qwen-MT returned no translated text.".into(),
            });
        }
        let request = QwenMTRequestEncoder::request(
            &trimmed_text,
            source_language_override.unwrap_or(self.source_language),
            self.target_language,
            self.model,
            stream,
            self.domain_hint.as_deref(),
            &self.terms,
            translation_memory,
        )
        .map_err(|_| QwenMTClientError::InvalidHTTPResponse)?;
        crate::development_content::request(
            crate::development_content::RequestProtocol::QwenMt,
            &request,
        );
        Ok(request.to_string())
    }
}

async fn drain_completed_sse_tail<S, B>(stream: &mut S)
where
    S: futures_util::Stream<Item = Result<B, reqwest::Error>> + Unpin,
    B: AsRef<[u8]>,
{
    let _ = tokio::time::timeout(SSE_TAIL_DRAIN_TIMEOUT, async {
        let mut received = 0;
        for _ in 0..MAX_SSE_TAIL_DRAIN_CHUNKS {
            let Some(chunk) = stream.next().await else {
                break;
            };
            let Ok(chunk) = chunk else { break };
            let length = chunk.as_ref().len();
            if length > MAX_SSE_TAIL_DRAIN_BYTES - received {
                break;
            }
            received += length;
            if received == MAX_SSE_TAIL_DRAIN_BYTES {
                break;
            }
        }
    })
    .await;
}

fn http_client_builder() -> reqwest::ClientBuilder {
    // reqwest 0.13's no-provider mode requires explicit initialization. The
    // updater does the same when checking for updates, but translation may be
    // used first. A previously installed provider is intentionally preserved.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
}

async fn read_bounded_response(response: reqwest::Response) -> Result<Vec<u8>, QwenMTClientError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(QwenMTClientError::ResponseTooLarge);
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| QwenMTClientError::InvalidHTTPResponse)?;
        if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
            return Err(QwenMTClientError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[derive(Default)]
struct BoundedSseResponse {
    buffer: Vec<u8>,
    scanned: usize,
    received: usize,
    translation: String,
    done: bool,
}

impl BoundedSseResponse {
    fn push(
        &mut self,
        chunk: &[u8],
        on_partial: &(impl Fn(String) + Send + Sync),
    ) -> Result<bool, QwenMTClientError> {
        if self.done {
            return Ok(true);
        }
        if chunk.len() > MAX_RESPONSE_BYTES - self.received {
            return Err(QwenMTClientError::ResponseTooLarge);
        }
        self.received += chunk.len();
        self.buffer.extend_from_slice(chunk);
        let mut consumed = 0;
        // Scan only newly received bytes and compact once per network chunk.
        // UTF-8 is decoded only after a complete line has arrived.
        while let Some(offset) = self.buffer[self.scanned..].iter().position(|&b| b == b'\n') {
            let end = self.scanned + offset + 1;
            let line = std::str::from_utf8(&self.buffer[consumed..end])
                .map_err(|_| QwenMTClientError::InvalidHTTPResponse)?;
            self.done = handle_sse_line(line.trim_end(), &mut self.translation, on_partial)?;
            consumed = end;
            self.scanned = end;
            if self.done {
                break;
            }
        }
        self.buffer.drain(..consumed);
        self.scanned = self.buffer.len();
        Ok(self.done)
    }

    fn finish(
        mut self,
        on_partial: &(impl Fn(String) + Send + Sync),
    ) -> Result<String, QwenMTClientError> {
        // Preserve support for servers that omit the final newline.
        if !self.done && !self.buffer.is_empty() {
            let line = std::str::from_utf8(&self.buffer)
                .map_err(|_| QwenMTClientError::InvalidHTTPResponse)?;
            handle_sse_line(line.trim_end(), &mut self.translation, on_partial)?;
        }
        Ok(self.translation.trim().to_string())
    }
}

/// Handles one SSE `data:` line, appending decoded content to `translation`.
/// Returns `true` when the stream has reached `[DONE]`.
fn handle_sse_line(
    line: &str,
    translation: &mut String,
    on_partial: &(impl Fn(String) + Send + Sync),
) -> Result<bool, QwenMTClientError> {
    let Some(payload) = line.strip_prefix("data:") else {
        return Ok(false);
    };
    let payload = payload.trim();
    if payload.is_empty() {
        return Ok(false);
    }
    if payload == "[DONE]" {
        return Ok(true);
    }
    let content = QwenMTStreamDecoder::decode_chunk(payload)
        .map_err(|_| QwenMTClientError::InvalidHTTPResponse)?;
    if let Some(content) = content {
        if !content.is_empty() {
            if content.len() > MAX_TRANSLATION_BYTES - translation.len() {
                return Err(QwenMTClientError::ResponseTooLarge);
            }
            translation.push_str(&content);
            on_partial(translation.clone());
        }
    }
    Ok(false)
}

fn rejected_error(status_code: u16, data: &[u8]) -> QwenMTClientError {
    pipeline_log!(
        "mt http rejected status={} category={}",
        status_code,
        QwenMTRejectionCategory::from_response(data).diagnostic_label()
    );
    QwenMTClientError::RequestFailed {
        status_code,
        message: error_message(data),
    }
}

fn error_message(data: &[u8]) -> String {
    #[derive(serde::Deserialize)]
    struct ErrorBody {
        error: Option<ErrorInner>,
    }
    #[derive(serde::Deserialize)]
    struct ErrorInner {
        message: Option<String>,
    }
    serde_json::from_slice::<ErrorBody>(data)
        .ok()
        .and_then(|body| body.error)
        .and_then(|error| error.message)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn sse_accepts_every_utf8_chunk_boundary_and_ignores_data_after_done() {
        let data =
            "data: {\"choices\":[{\"delta\":{\"content\":\"今天\"}}]}\r\n\ndata: [DONE]\ninvalid";
        for split in 0..=data.len() {
            let mut decoder = BoundedSseResponse::default();
            decoder.push(&data.as_bytes()[..split], &|_| {}).unwrap();
            decoder.push(&data.as_bytes()[split..], &|_| {}).unwrap();
            assert_eq!(decoder.finish(&|_| {}).unwrap(), "今天");
        }
    }

    #[test]
    fn sse_accepts_a_final_line_without_newline() {
        let mut decoder = BoundedSseResponse::default();
        decoder
            .push(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}",
                &|_| {},
            )
            .unwrap();
        assert_eq!(decoder.finish(&|_| {}).unwrap(), "hello");
    }

    #[test]
    fn sse_rejects_oversized_unterminated_and_repeated_lines_before_growth() {
        let mut decoder = BoundedSseResponse::default();
        decoder
            .push(&vec![b' '; MAX_RESPONSE_BYTES], &|_| {})
            .unwrap();
        assert_eq!(
            decoder.push(b" ", &|_| {}),
            Err(QwenMTClientError::ResponseTooLarge)
        );
        assert_eq!(decoder.buffer.len(), MAX_RESPONSE_BYTES);

        let mut decoder = BoundedSseResponse::default();
        for _ in 0..16 {
            decoder
                .push(&vec![b'\n'; MAX_RESPONSE_BYTES / 16], &|_| {})
                .unwrap();
        }
        assert!(decoder.buffer.is_empty());
        assert_eq!(
            decoder.push(b"\n", &|_| {}),
            Err(QwenMTClientError::ResponseTooLarge)
        );
    }

    #[test]
    fn translation_limit_rejects_before_publishing_an_oversized_preview() {
        let mut translation = "x".repeat(MAX_TRANSLATION_BYTES);
        let error = handle_sse_line(
            r#"data: {"choices":[{"delta":{"content":"x"}}]}"#,
            &mut translation,
            &|_| panic!("oversized partial must not be published"),
        );
        assert_eq!(error, Err(QwenMTClientError::ResponseTooLarge));
        assert_eq!(translation.len(), MAX_TRANSLATION_BYTES);
    }

    async fn fixture_client(response: String) -> (QwenMTClient, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            // Oversized responses may intentionally close before reading the body.
            let _ = socket.write_all(response.as_bytes()).await;
            String::from_utf8(request).unwrap()
        });
        let mut client = QwenMTClient::new(
            "fixture-only",
            SourceLanguage::English,
            TargetLanguage::SimplifiedChinese,
            QwenMTModel::Flash,
            None,
            vec![],
            Duration::from_millis(300),
        )
        .unwrap();
        client.endpoint.url = format!("http://{address}/translate").parse().unwrap();
        client.client = http_client_builder().no_proxy().build().unwrap();
        (client, server)
    }

    #[tokio::test]
    async fn http_translation_preserves_json_request_and_response() {
        let body = r#"{"choices":[{"message":{"content":"你好"}}]}"#;
        let (client, server) = fixture_client(format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        ))
        .await;
        assert_eq!(client.translate("hello", None, &[]).await.unwrap(), "你好");
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /translate HTTP/1.1\r\n"));
        assert!(request.contains("authorization: Bearer fixture-only"));
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["model"], "qwen-mt-flash");
        assert_eq!(body["stream"], false);
    }

    #[tokio::test]
    async fn http_streaming_preserves_partials_and_done() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"好\"}}]}\n\ndata: [DONE]\n";
        let (client, server) = fixture_client(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len(),
        )).await;
        let partials = std::sync::Mutex::new(Vec::new());
        assert_eq!(
            client
                .translate_streaming("hello", None, &[], |text| {
                    partials.lock().unwrap().push(text);
                })
                .await
                .unwrap(),
            "你好"
        );
        assert_eq!(*partials.lock().unwrap(), vec!["你", "你好"]);
        let request = server.await.unwrap();
        assert!(request.contains("accept: text/event-stream"));
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["stream"], true);
    }

    async fn read_fixture_request(socket: &mut tokio::net::TcpStream) -> bool {
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 1024];
            let Ok(read) = socket.read(&mut buffer).await else {
                return false;
            };
            if read == 0 {
                return false;
            }
            bytes.extend_from_slice(&buffer[..read]);
            assert!(
                bytes.len() < 16 * 1024,
                "synthetic request must stay bounded"
            );
            if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let length: usize = String::from_utf8_lossy(&bytes[..end])
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|length| length.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    return true;
                }
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completed_sse_reuses_tcp_when_http_terminator_arrives_after_done() {
        assert_eq!(
            released_sse_tail_connection_count(true).await,
            1,
            "both translations must share one TCP connection"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_completed_sse_without_drain_cannot_reuse_the_late_tail_connection() {
        assert_eq!(
            released_sse_tail_connection_count(false).await,
            2,
            "the fixture must still expose the original immediate-drop defect"
        );
    }

    async fn released_sse_tail_connection_count(drain_tail: bool) -> usize {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release_tail, tail_consumed) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.set_nodelay(true).unwrap();
            assert!(read_fixture_request(&mut socket).await);
            let body =
                "data: {\"choices\":[{\"delta\":{\"content\":\"complete\"}}]}\n\ndata: [DONE]\n\n";
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n", body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
            // Release the separate HTTP terminator when the client consumes
            // this single complete SSE chunk, rather than racing a 5 ms sleep
            // against the product's 20 ms drain. On this current-thread
            // runtime the callback wakes this task, but it runs only when the
            // client yields while awaiting the tail. TCP_NODELAY prevents the
            // tiny terminator from waiting for the first chunk's delayed ACK.
            tail_consumed.await.unwrap();
            let _ = socket.write_all(b"0\r\n\r\n").await;
            let reused =
                tokio::time::timeout(Duration::from_secs(1), read_fixture_request(&mut socket))
                    .await
                    .unwrap();
            let connections = if reused {
                1
            } else {
                socket = tokio::time::timeout(Duration::from_secs(1), listener.accept())
                    .await
                    .unwrap()
                    .unwrap()
                    .0;
                assert!(read_fixture_request(&mut socket).await);
                2
            };
            socket
                .write_all(format!("{response}0\r\n\r\n").as_bytes())
                .await
                .unwrap();
            connections
        });
        let mut client = QwenMTClient::new(
            "fixture-only",
            SourceLanguage::English,
            TargetLanguage::SimplifiedChinese,
            QwenMTModel::Lite,
            None,
            vec![],
            Duration::from_secs(2),
        )
        .unwrap();
        client.endpoint.url = format!("http://{address}/translate").parse().unwrap();
        client.client = http_client_builder().no_proxy().build().unwrap();
        let release_tail = std::sync::Mutex::new(Some(release_tail));
        for _ in 0..2 {
            let on_partial = |text| {
                assert_eq!(text, "complete");
                if let Some(release) = release_tail.lock().unwrap().take() {
                    release.send(()).unwrap();
                }
            };
            let translated = if drain_tail {
                client
                    .translate_streaming("synthetic", None, &[], on_partial)
                    .await
                    .unwrap()
            } else {
                // Negative control executes the actual decoder/HTTP path but
                // drops its owned response at DONE, as the old wrapper did.
                // No production cleanup timeout or implementation is changed.
                let request = client
                    .client
                    .post(client.endpoint.url.clone())
                    .bearer_auth(&client.api_key)
                    .header("Content-Type", "application/json")
                    .header("Accept", "text/event-stream")
                    .body(client.make_body("synthetic", None, true, &[]).unwrap());
                let (translated, response) = tokio::time::timeout(
                    Duration::from_secs(2),
                    client.stream(request, &on_partial),
                )
                .await
                .unwrap()
                .unwrap();
                drop(response);
                translated
            };
            assert_eq!(translated, "complete");
        }
        server.await.unwrap()
    }

    #[tokio::test]
    async fn completed_sse_with_stalled_tail_returns_success_and_closes_without_waiting_for_request_deadline(
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert!(read_fixture_request(&mut socket).await);
            let body =
                "data: {\"choices\":[{\"delta\":{\"content\":\"complete\"}}]}\n\ndata: [DONE]\n\n";
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n", body.len()).as_bytes()).await.unwrap();
            // No HTTP terminator follows. Returning the result must drop the
            // response and close this connection rather than leave a task.
            tokio::time::timeout(Duration::from_secs(1), socket.read(&mut [0; 1]))
                .await
                .unwrap()
                .unwrap()
        });
        let mut client = QwenMTClient::new(
            "fixture-only",
            SourceLanguage::English,
            TargetLanguage::SimplifiedChinese,
            QwenMTModel::Lite,
            None,
            vec![],
            Duration::from_secs(2),
        )
        .unwrap();
        client.endpoint.url = format!("http://{address}/translate").parse().unwrap();
        client.client = http_client_builder().no_proxy().build().unwrap();
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            client.translate_streaming("synthetic", None, &[], |_| {}),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap(), "complete");
        assert_eq!(server.await.unwrap(), 0);
    }

    #[tokio::test]
    async fn completed_sse_tail_drain_bounds_bytes_without_collecting_or_spawning() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let polls = AtomicUsize::new(0);
        let mut stream = futures_util::stream::repeat_with(|| {
            polls.fetch_add(1, Ordering::Relaxed);
            Ok::<_, reqwest::Error>(vec![0; 4096])
        });
        drain_completed_sse_tail(&mut stream).await;
        assert_eq!(polls.load(Ordering::Relaxed), 2);
        let empty_polls = AtomicUsize::new(0);
        let mut empty_stream = futures_util::stream::repeat_with(|| {
            empty_polls.fetch_add(1, Ordering::Relaxed);
            Ok::<_, reqwest::Error>(Vec::<u8>::new())
        });
        drain_completed_sse_tail(&mut empty_stream).await;
        assert_eq!(
            empty_polls.load(Ordering::Relaxed),
            MAX_SSE_TAIL_DRAIN_CHUNKS
        );
    }

    #[tokio::test]
    async fn cancelling_completed_sse_during_tail_drain_closes_the_owned_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert!(read_fixture_request(&mut socket).await);
            let body =
                "data: {\"choices\":[{\"delta\":{\"content\":\"complete\"}}]}\n\ndata: [DONE]\n\n";
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n", body.len()).as_bytes()).await.unwrap();
            tokio::time::timeout(Duration::from_secs(1), socket.read(&mut [0; 1]))
                .await
                .unwrap()
                .unwrap()
        });
        let mut client = QwenMTClient::new(
            "fixture-only",
            SourceLanguage::English,
            TargetLanguage::SimplifiedChinese,
            QwenMTModel::Lite,
            None,
            vec![],
            Duration::from_secs(2),
        )
        .unwrap();
        client.endpoint.url = format!("http://{address}/translate").parse().unwrap();
        client.client = http_client_builder().no_proxy().build().unwrap();
        let (partial, received) = tokio::sync::oneshot::channel();
        let partial = std::sync::Mutex::new(Some(partial));
        let request = tokio::spawn(async move {
            client
                .translate_streaming("synthetic", None, &[], |_| {
                    if let Some(sender) = partial.lock().unwrap().take() {
                        let _ = sender.send(());
                    }
                })
                .await
        });
        received.await.unwrap();
        request.abort();
        let _ = request.await;
        assert_eq!(server.await.unwrap(), 0);
    }

    #[tokio::test]
    async fn streaming_deadline_cancels_a_stalled_http_body() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let read = socket.read(&mut request).await.unwrap();
            assert!(read > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let mut client = QwenMTClient::new(
            "fixture-only",
            SourceLanguage::English,
            TargetLanguage::SimplifiedChinese,
            QwenMTModel::Flash,
            None,
            vec![],
            Duration::from_millis(100),
        )
        .unwrap();
        client.endpoint.url = format!("http://{address}/translate").parse().unwrap();
        client.client = http_client_builder().no_proxy().build().unwrap();
        let result = client.translate_streaming("hello", None, &[], |_| {}).await;
        server.abort();
        assert_eq!(result, Err(QwenMTClientError::RequestTimedOut));
    }

    #[tokio::test]
    #[ignore = "manual public HTTPS certificate/proxy smoke; sends no credentials or subtitle data"]
    async fn public_https_uses_system_certificate_validation() {
        let client = http_client_builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        for endpoint in ["https://dashscope.aliyuncs.com", "https://github.com"] {
            client
                .head(endpoint)
                .send()
                .await
                .expect("public HTTPS handshake must succeed");
        }
    }

    #[tokio::test]
    async fn chunked_http_body_is_bounded_without_content_length() {
        let chunk = " ".repeat(MAX_RESPONSE_BYTES + 1);
        let (client, server) = fixture_client(format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{chunk}\r\n0\r\n\r\n", chunk.len(),
        )).await;
        assert_eq!(
            client.translate("hello", None, &[]).await,
            Err(QwenMTClientError::ResponseTooLarge)
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn oversized_http_error_preserves_authentication_status() {
        let (client, server) = fixture_client(format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_RESPONSE_BYTES + 1,
        ))
        .await;
        let error = client.translate("hello", None, &[]).await.unwrap_err();
        assert!(error.is_authentication_failure());
        assert_eq!(
            error,
            QwenMTClientError::RequestFailed {
                status_code: 401,
                message: String::new()
            }
        );
        server.await.unwrap();
    }

    #[test]
    fn sse_line_appends_content_and_reports_done() {
        let mut translation = String::new();
        let partials = std::sync::Mutex::new(Vec::new());

        let handled = handle_sse_line(
            r#"data:{"choices":[{"delta":{"content":"今天"}}]}"#,
            &mut translation,
            &|text| partials.lock().unwrap().push(text),
        )
        .unwrap();
        assert!(!handled);
        assert_eq!(translation, "今天");
        assert_eq!(*partials.lock().unwrap(), vec!["今天".to_string()]);

        let done = handle_sse_line("data: [DONE]", &mut translation, &|_| {}).unwrap();
        assert!(done);
        assert_eq!(translation, "今天");
    }

    #[test]
    fn non_data_lines_are_ignored() {
        let mut translation = String::new();
        let handled = handle_sse_line("event: message", &mut translation, &|_| {}).unwrap();
        assert!(!handled);
        assert!(translation.is_empty());
    }
}
