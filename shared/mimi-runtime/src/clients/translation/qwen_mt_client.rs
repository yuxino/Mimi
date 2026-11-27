//! Qwen-MT HTTP client for streaming and non-streaming chat completions.

use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::translation::qwen_mt::{
    QwenMTClientError, QwenMTEndpoint, QwenMTMemoryPair, QwenMTModel, QwenMTProtocolError,
    QwenMTRejectionCategory, QwenMTRequestEncoder, QwenMTResponseDecoder, QwenMTStreamDecoder,
    QwenMTStreamText, QwenMTTerm,
};
use crate::pipeline_log;
use futures_util::StreamExt;
use std::time::Duration;

// A sentence translation should stay far below these bounds. Enforce byte
// limits even when Content-Length is absent or the server never sends '\n'.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
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
    pub(in crate::clients) fn use_synthetic_endpoint(&mut self, endpoint: url::Url) {
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
        network: crate::clients::transport::provider_network::ProviderNetwork,
    ) -> Result<(), crate::clients::transport::provider_network::ProviderNetworkError> {
        self.client = network
            .http_client_builder(&self.endpoint.url)?
            .build()
            .map_err(|_| crate::core::services::network_proxy::ProxyConfigError::BuilderFailed)?;
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
        let text =
            std::str::from_utf8(&bytes).map_err(|_| QwenMTClientError::InvalidHTTPResponse)?;
        QwenMTResponseDecoder::decode(text).map_err(|error| match error {
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

        let mut decoder = BoundedSseResponse {
            translation: QwenMTStreamText::new(self.model),
            ..Default::default()
        };
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
    translation: QwenMTStreamText,
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
            self.done = handle_sse_line(line.trim_end(), &mut self.translation, on_partial)?;
        }
        if !self.done {
            return Err(incomplete_translation_error());
        }
        Ok(self.translation.text().trim().to_string())
    }
}

/// Lite/Flash send incremental content; Plus sends the whole generated sequence.
/// Empty completion chunks preserve the latest translation in either format.
/// Returns `true` when `[DONE]` or a successful `stop` confirms completion.
fn handle_sse_line(
    line: &str,
    translation: &mut QwenMTStreamText,
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
    let (content, complete) = QwenMTStreamDecoder::decode_chunk_with_completion(payload).map_err(
        |error| match error {
            QwenMTProtocolError::IncompleteTranslation => incomplete_translation_error(),
            _ => QwenMTClientError::InvalidHTTPResponse,
        },
    )?;
    if let Some(content) = content {
        if translation.apply(&content)? {
            on_partial(translation.text().to_owned());
        }
    }
    Ok(complete)
}

fn incomplete_translation_error() -> QwenMTClientError {
    QwenMTClientError::RequestFailed {
        status_code: 200,
        message: QwenMTProtocolError::IncompleteTranslation.to_string(),
    }
}

fn rejected_error(status_code: u16, data: &[u8]) -> QwenMTClientError {
    pipeline_log!(
        "mt http rejected status={} category={}",
        status_code,
        QwenMTRejectionCategory::from_response(data).diagnostic_label()
    );
    // Keep upstream error bodies out of diagnostics and UI-facing messages.
    QwenMTClientError::RequestFailed {
        status_code,
        message: String::new(),
    }
}

#[cfg(test)]
mod tests;
