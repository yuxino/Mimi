//! OpenAI Realtime Translation WebSocket client.

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::core::models::TargetLanguage;
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::realtime::openai_realtime::{
    OpenAIRealtimeEndpoint, OpenAIRealtimeFailure, OpenAIRealtimeProtocolError,
    OpenAIRealtimeRequestEncoder, OpenAIRealtimeServerEvent,
};
use crate::core::subtitles::openai_transcript_committer::OpenAITranscriptPairCommitter;
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const GENERIC_PROTOCOL_ERROR: &str = "OpenAI Realtime Translation returned an invalid response.";
const GENERIC_TRANSPORT_ERROR: &str = "The OpenAI Realtime Translation connection failed.";
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OpenAIRealtimeClientError {
    #[error("credential_authentication_failed")]
    AuthenticationFailed,
    #[error("Add an OpenAI API key in Settings.")]
    MissingAPIKey,
    #[error("OpenAI Realtime Translation requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("The OpenAI Realtime Translation session is not connected.")]
    NotConnected,
    #[error("The OpenAI Realtime Translation connection stopped responding.")]
    HealthCheckTimedOut,
    #[error("The OpenAI Realtime Translation connection failed.")]
    TransportFailure,
    #[error("OpenAI Realtime Translation rejected the session configuration.")]
    SessionSetupRejected,
    #[error("OpenAI Realtime Translation rejected the request.")]
    ProviderRejected(OpenAIRealtimeFailure),
    #[error("OpenAI Realtime Translation is temporarily unavailable.")]
    UpgradeUnavailable,
    #[error("OpenAI Realtime Translation did not confirm the session configuration in time.")]
    SessionSetupTimedOut,
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected(OpenAIRealtimeClientError),
}

impl OpenAIRealtimeClientError {
    fn setup_rejection(failure: OpenAIRealtimeFailure) -> Self {
        match failure {
            OpenAIRealtimeFailure::Authentication => Self::AuthenticationFailed,
            OpenAIRealtimeFailure::Rejected => Self::SessionSetupRejected,
            other => Self::ProviderRejected(other),
        }
    }

    pub fn recognition_failure_label(&self) -> Option<&'static str> {
        match self {
            Self::AuthenticationFailed => Some(OpenAIRealtimeFailure::Authentication.label()),
            Self::SessionSetupRejected => Some(OpenAIRealtimeFailure::Rejected.label()),
            // An expiry before readiness is a rejected setup. A replacement
            // cannot renew its retry budget by rejecting every setup as expired.
            Self::ProviderRejected(OpenAIRealtimeFailure::SessionExpired) => {
                Some("openai_realtime_setup_expired")
            }
            Self::ProviderRejected(failure) => Some(failure.label()),
            Self::UpgradeUnavailable => Some("openai_realtime_service_unavailable"),
            _ => None,
        }
    }
}

fn upgrade_failure(error: tokio_tungstenite::tungstenite::Error) -> OpenAIRealtimeClientError {
    use OpenAIRealtimeFailure as Failure;
    let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
        return OpenAIRealtimeClientError::TransportFailure;
    };
    let failure = match response.status().as_u16() {
        400 => Failure::InvalidRequest,
        401 => Failure::Authentication,
        403 => Failure::Permission,
        404 => Failure::ModelUnavailable,
        429 => {
            // HTTP 429 also covers quota and billing. Only an exact code can
            // establish which limit was reached when a response body is present.
            let body_failure = response
                .body()
                .as_deref()
                .filter(|body| body.len() <= 16 * 1024)
                .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok())
                .map(|body| Failure::from_error(&body["error"]));
            if body_failure == Some(Failure::Quota) {
                Failure::Quota
            } else {
                Failure::RateLimited
            }
        }
        500 | 502 | 503 | 504 => return OpenAIRealtimeClientError::UpgradeUnavailable,
        _ => return OpenAIRealtimeClientError::TransportFailure,
    };
    OpenAIRealtimeClientError::setup_rejection(failure)
}

struct Inner {
    // Serializes the content barrier with local assembly, never socket/audio state.
    content_lock: Mutex<()>,
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    audio_send_lock: Mutex<()>,
    pending_audio: Mutex<Vec<u8>>,
    committer: Mutex<OpenAITranscriptPairCommitter>,
    ready: AtomicBool,
    is_closing: AtomicBool,
    received_session_closed: AtomicBool,
    close_notify: Notify,
    pong_notify: Notify,
    generation: AtomicU64,
}

#[derive(Clone)]
pub struct OpenAIRealtimeClient {
    network: crate::clients::transport::provider_network::ProviderNetwork,
    inner: Arc<Inner>,
    endpoint: url::Url,
    api_key: String,
    target_language: TargetLanguage,
    events: ProviderEventSender,
}

impl OpenAIRealtimeClient {
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.clone()
    }

    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    #[cfg(feature = "test-support")]
    pub async fn disable_source_isolation_for_comparison(&self) {
        self.inner
            .committer
            .lock()
            .await
            .disable_source_isolation_for_comparison();
    }

    /// Clears locally accepted subtitles without reconnecting or dropping audio.
    /// This protocol has no source-turn boundary: future deltas remain valid.
    pub async fn clear_content(&self) -> u64 {
        let _content = self.inner.content_lock.lock().await;
        self.inner.committer.lock().await.reset();
        self.events.advance_content_revision()
    }

    /// Applied before connect so ASR and translation share one immutable route.
    pub fn set_network(
        &mut self,
        network: crate::clients::transport::provider_network::ProviderNetwork,
    ) -> Result<(), crate::clients::transport::provider_network::ProviderNetworkError> {
        self.network = network;
        Ok(())
    }

    pub fn new(
        api_key: &str,
        target_language: TargetLanguage,
        events: ProviderEventSender,
    ) -> Result<Self, OpenAIRealtimeClientError> {
        let endpoint = OpenAIRealtimeEndpoint::url()
            .map_err(|_| OpenAIRealtimeClientError::TransportFailure)?;
        Self::with_endpoint(api_key, target_language, events, endpoint)
    }

    pub(in crate::clients) fn with_endpoint(
        api_key: &str,
        target_language: TargetLanguage,
        events: ProviderEventSender,
        endpoint: url::Url,
    ) -> Result<Self, OpenAIRealtimeClientError> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(OpenAIRealtimeClientError::MissingAPIKey);
        }
        if !target_language.translates_audio() {
            return Err(OpenAIRealtimeClientError::InvalidTargetLanguage);
        }
        Ok(Self {
            network: crate::clients::transport::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                content_lock: Mutex::new(()),
                sink: Mutex::new(None),
                receive_task: Mutex::new(None),
                audio_send_lock: Mutex::new(()),
                pending_audio: Mutex::new(Vec::new()),
                committer: Mutex::new(OpenAITranscriptPairCommitter::default()),
                ready: AtomicBool::new(false),
                is_closing: AtomicBool::new(false),
                received_session_closed: AtomicBool::new(false),
                close_notify: Notify::new(),
                pong_notify: Notify::new(),
                generation: AtomicU64::new(0),
            }),
            endpoint,
            api_key: api_key.to_string(),
            target_language,
            events,
        })
    }

    pub async fn connect(&self) -> Result<(), OpenAIRealtimeClientError> {
        self.connect_with_timeout(Duration::from_secs(5)).await
    }

    async fn connect_with_timeout(
        &self,
        readiness_timeout: Duration,
    ) -> Result<(), OpenAIRealtimeClientError> {
        self.disconnect().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let setup_event_id = format!("mimi-session-update-{generation}");

        let mut request = self
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|_| OpenAIRealtimeClientError::TransportFailure)?;
        let authorization = format!("Bearer {}", self.api_key);
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&authorization)
                .map_err(|_| OpenAIRealtimeClientError::MissingAPIKey)?,
        );

        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            crate::clients::transport::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| OpenAIRealtimeClientError::TransportFailure)?
        .map_err(upgrade_failure)?;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .received_session_closed
            .store(false, Ordering::SeqCst);
        self.inner.pending_audio.lock().await.clear();
        self.inner.committer.lock().await.reset();

        let (setup_tx, mut setup_rx) = watch::channel(SetupState::Awaiting);
        let task = tokio::spawn(receive_loop(ReceiveContext {
            inner: Arc::clone(&self.inner),
            stream,
            events: self.events.clone(),
            target_language: self.target_language,
            setup: setup_tx,
            setup_event_id: setup_event_id.clone(),
            generation,
        }));
        *self.inner.receive_task.lock().await = Some(task);

        let update = OpenAIRealtimeRequestEncoder::session_update(
            self.target_language,
            Some(&setup_event_id),
        )
        .map_err(|_| OpenAIRealtimeClientError::InvalidTargetLanguage)?;
        // One deadline covers both the update send and the acknowledgement;
        // a socket whose write side stalls cannot make connect unbounded.
        let complete_setup = async {
            self.send_text(update.to_string())
                .await
                .map_err(|_| OpenAIRealtimeClientError::TransportFailure)?;
            loop {
                match setup_rx.borrow().clone() {
                    SetupState::Ready => return Ok(()),
                    SetupState::Rejected(error) => return Err(error),
                    SetupState::Awaiting => {}
                }
                if setup_rx.changed().await.is_err() {
                    return Err(OpenAIRealtimeClientError::SessionSetupRejected);
                }
            }
        };
        match tokio::time::timeout(readiness_timeout, complete_setup).await {
            Ok(Ok(())) => {
                self.inner.ready.store(true, Ordering::SeqCst);
                Ok(())
            }
            Ok(Err(error)) => {
                self.disconnect().await;
                Err(error)
            }
            Err(_) => {
                self.disconnect().await;
                Err(OpenAIRealtimeClientError::SessionSetupTimedOut)
            }
        }
    }

    /// Accepts arbitrary PCM chunks and sends only exact 200 ms frames.
    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), OpenAIRealtimeClientError> {
        if pcm_data.is_empty() {
            return Ok(());
        }
        tokio::time::timeout(SEND_TIMEOUT, self.send_audio_operation(pcm_data))
            .await
            .map_err(|_| OpenAIRealtimeClientError::TransportFailure)?
    }

    async fn send_audio_operation(&self, pcm_data: &[u8]) -> Result<(), OpenAIRealtimeClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(OpenAIRealtimeClientError::NotConnected);
        }

        let _send_guard = self.inner.audio_send_lock.lock().await;
        let messages = {
            let mut pending = self.inner.pending_audio.lock().await;
            pending.extend_from_slice(pcm_data);
            take_complete_audio_messages(&mut pending)?
        };
        for message in messages {
            self.send_text(message).await?;
        }
        Ok(())
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), OpenAIRealtimeClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(OpenAIRealtimeClientError::NotConnected);
        }
        let operation = async {
            let pong = self.inner.pong_notify.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(OpenAIRealtimeClientError::NotConnected);
                };
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| OpenAIRealtimeClientError::TransportFailure)?;
            }
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| OpenAIRealtimeClientError::HealthCheckTimedOut)?
    }

    /// Drains queued audio, pads the final frame, and waits for a real
    /// `session.closed`. The whole operation is bounded by `timeout`.
    pub async fn finish(&self, timeout: Duration) {
        if !self.inner.ready.load(Ordering::SeqCst) {
            return;
        }
        if self.inner.is_closing.swap(true, Ordering::SeqCst) {
            return;
        }
        let generation = self.inner.generation.load(Ordering::SeqCst);
        match tokio::time::timeout(timeout, self.finish_operation(generation)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) if self.is_current_generation(generation) => {
                self.inner.committer.lock().await.reset();
                self.inner.pending_audio.lock().await.clear();
                self.emit(
                    LiveTranslateServerEvent::Error {
                        code: "openai_session_close_failed".into(),
                        message: GENERIC_TRANSPORT_ERROR.into(),
                    },
                    generation,
                );
            }
            Err(_) if self.is_current_generation(generation) => {
                self.inner.committer.lock().await.reset();
                self.inner.pending_audio.lock().await.clear();
                self.emit(
                    LiveTranslateServerEvent::Error {
                        code: "openai_close_timeout".into(),
                        message: "OpenAI Realtime Translation did not finish closing in time."
                            .into(),
                    },
                    generation,
                );
            }
            _ => {}
        }
        self.disconnect_if_current(generation).await;
    }

    async fn finish_operation(&self, generation: u64) -> Result<(), OpenAIRealtimeClientError> {
        let _send_guard = self.inner.audio_send_lock.lock().await;
        if !self.is_current_generation(generation) {
            return Err(OpenAIRealtimeClientError::NotConnected);
        }
        let partial = {
            let mut pending = self.inner.pending_audio.lock().await;
            take_padded_audio_message(&mut pending)?
        };
        if let Some(message) = partial {
            self.send_text(message).await?;
        }
        self.send_text(OpenAIRealtimeRequestEncoder::close().to_string())
            .await?;
        while self.is_current_generation(generation) {
            let closed = self.inner.close_notify.notified();
            tokio::pin!(closed);
            closed.as_mut().enable();
            if self.inner.received_session_closed.load(Ordering::SeqCst) {
                break;
            }
            closed.await;
        }
        Ok(())
    }

    pub async fn disconnect(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .received_session_closed
            .store(false, Ordering::SeqCst);
        if let Some(task) = self.inner.receive_task.lock().await.take() {
            task.abort();
        }
        let sink = self.inner.sink.lock().await.take();
        if let Some(mut sink) = sink {
            let _ = tokio::time::timeout(Duration::from_millis(250), sink.close()).await;
        }
        self.inner.pending_audio.lock().await.clear();
        self.inner.committer.lock().await.reset();
        self.inner.close_notify.notify_waiters();
    }

    async fn send_text(&self, text: String) -> Result<(), OpenAIRealtimeClientError> {
        let mut sink = self.inner.sink.lock().await;
        let Some(sink) = sink.as_mut() else {
            return Err(OpenAIRealtimeClientError::NotConnected);
        };
        let evidence =
            crate::development_audio::begin_json(&text, OpenAIRealtimeEndpoint::SAMPLE_RATE_HZ);
        tokio::time::timeout(
            SEND_TIMEOUT,
            evidence.observe(sink.send(Message::Text(text.into()))),
        )
        .await
        .map_err(|_| OpenAIRealtimeClientError::TransportFailure)?
        .map_err(|_| OpenAIRealtimeClientError::TransportFailure)
    }

    fn emit(&self, event: LiveTranslateServerEvent, generation: u64) {
        if self.is_current_generation(generation) {
            let _ = self.events.send(event);
        }
    }

    fn is_current_generation(&self, generation: u64) -> bool {
        self.inner.generation.load(Ordering::SeqCst) == generation
    }

    async fn disconnect_if_current(&self, generation: u64) {
        if self.is_current_generation(generation) {
            self.disconnect().await;
        }
    }
}

fn take_complete_audio_messages(
    pending: &mut Vec<u8>,
) -> Result<Vec<String>, OpenAIRealtimeClientError> {
    let frame_size = OpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let complete_bytes = pending.len() / frame_size * frame_size;
    let result = pending[..complete_bytes]
        .chunks_exact(frame_size)
        .map(|frame| {
            OpenAIRealtimeRequestEncoder::audio_append(frame)
                .map(|value| value.to_string())
                .map_err(|_| OpenAIRealtimeClientError::TransportFailure)
        })
        .collect::<Result<Vec<_>, _>>();
    // Preserve the existing failure semantics: a complete batch is consumed
    // even if encoding one of its frames fails.
    pending.drain(..complete_bytes);
    result
}

fn take_padded_audio_message(
    pending: &mut Vec<u8>,
) -> Result<Option<String>, OpenAIRealtimeClientError> {
    if pending.is_empty() {
        return Ok(None);
    }
    let frame_size = OpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let mut frame = std::mem::take(pending);
    frame.resize(frame_size, 0);
    OpenAIRealtimeRequestEncoder::audio_append(&frame)
        .map(|value| Some(value.to_string()))
        .map_err(|_| OpenAIRealtimeClientError::TransportFailure)
}

struct ReceiveContext {
    inner: Arc<Inner>,
    stream: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    events: ProviderEventSender,
    target_language: TargetLanguage,
    setup: watch::Sender<SetupState>,
    setup_event_id: String,
    generation: u64,
}

async fn receive_loop(mut context: ReceiveContext) {
    #[cfg(feature = "test-support")]
    let trace_started = std::time::Instant::now();
    while let Some(message) = context.stream.next().await {
        if context.inner.generation.load(Ordering::SeqCst) != context.generation {
            return;
        }
        let event = match message {
            Ok(Message::Text(text)) => OpenAIRealtimeServerEvent::decode(&text),
            Ok(Message::Binary(data)) => std::str::from_utf8(&data)
                .map_err(|_| OpenAIRealtimeProtocolError::InvalidJSON)
                .and_then(OpenAIRealtimeServerEvent::decode),
            Ok(Message::Pong(_)) => {
                context.inner.pong_notify.notify_waiters();
                continue;
            }
            Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => {
                fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR).await;
                return;
            }
        };
        let event = match event {
            Ok(event) => event,
            Err(_) => {
                fail_receive_loop(&context, "openai_protocol_error", GENERIC_PROTOCOL_ERROR).await;
                return;
            }
        };
        #[cfg(feature = "test-support")]
        if std::env::var("MIMI_REALTIME_METADATA_TRACE").as_deref() == Ok("1") {
            let lane = match &event {
                OpenAIRealtimeServerEvent::SourceTranscriptDelta { text, elapsed_ms } => {
                    Some(("source", text, elapsed_ms))
                }
                OpenAIRealtimeServerEvent::TranslationTranscriptDelta { text, elapsed_ms } => {
                    Some(("translation", text, elapsed_ms))
                }
                _ => None,
            };
            if let Some((lane, text, alignment_ms)) = lane {
                let sentence_end = text
                    .trim_end_matches(|c: char| {
                        c.is_whitespace()
                            || matches!(c, '"' | '\'' | '”' | '’' | '」' | '』' | ')' | '）')
                    })
                    .ends_with(['.', '!', '?', '。', '！', '？']);
                eprintln!(
                    "{}",
                    serde_json::json!({"phase":"wire_progress", "lane":lane, "characters":text.chars().count(), "sentence_end":sentence_end,
                    "alignment_ms":alignment_ms, "received_ms":trace_started.elapsed().as_millis()})
                );
            }
        }
        if handle_server_event(&context, event).await {
            return;
        }
    }
    if context.inner.generation.load(Ordering::SeqCst) == context.generation
        && !context.inner.received_session_closed.load(Ordering::SeqCst)
    {
        fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR).await;
    }
}

async fn fail_receive_loop(context: &ReceiveContext, code: &str, message: &str) {
    if *context.setup.borrow() == SetupState::Awaiting {
        let failure = if code == "transport_error" {
            OpenAIRealtimeClientError::TransportFailure
        } else {
            OpenAIRealtimeClientError::SessionSetupRejected
        };
        let _ = context.setup.send(SetupState::Rejected(failure));
    } else {
        emit_if_current(
            context,
            LiveTranslateServerEvent::Error {
                code: code.into(),
                message: message.into(),
            },
        );
    }
}

/// Returns true when the receive loop must stop.
async fn handle_server_event(context: &ReceiveContext, event: OpenAIRealtimeServerEvent) -> bool {
    let revision = context.events.content_revision();
    let _content = context.inner.content_lock.lock().await;
    if context.inner.generation.load(Ordering::SeqCst) != context.generation {
        return false;
    }
    if revision != context.events.content_revision()
        && matches!(
            &event,
            OpenAIRealtimeServerEvent::SourceTranscriptDelta { .. }
                | OpenAIRealtimeServerEvent::TranslationTranscriptDelta { .. }
        )
    {
        return false;
    }

    match event {
        OpenAIRealtimeServerEvent::SessionCreated => {
            emit_if_current(context, LiveTranslateServerEvent::SessionCreated);
        }
        OpenAIRealtimeServerEvent::SessionUpdated {
            source_transcription_model,
            target_language,
        } => {
            if source_transcription_model != OpenAIRealtimeEndpoint::SOURCE_TRANSCRIPTION_MODEL
                || target_language != context.target_language.raw_value()
            {
                if *context.setup.borrow() == SetupState::Awaiting {
                    let _ = context.setup.send(SetupState::Rejected(
                        OpenAIRealtimeClientError::SessionSetupRejected,
                    ));
                } else {
                    emit_if_current(
                        context,
                        LiveTranslateServerEvent::Error {
                            code: OpenAIRealtimeFailure::InvalidRequest.label().into(),
                            message: OpenAIRealtimeFailure::InvalidRequest.label().into(),
                        },
                    );
                }
                return true;
            }
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Ready);
            } else {
                emit_if_current(context, LiveTranslateServerEvent::SessionUpdated);
            }
        }
        OpenAIRealtimeServerEvent::SourceTranscriptDelta { text, elapsed_ms } => {
            let events = context
                .inner
                .committer
                .lock()
                .await
                .append_source_delta(&text, elapsed_ms);
            emit_all_if_current(context, events);
        }
        OpenAIRealtimeServerEvent::TranslationTranscriptDelta { text, elapsed_ms } => {
            let events = context
                .inner
                .committer
                .lock()
                .await
                .append_translation_delta(&text, elapsed_ms);
            emit_all_if_current(context, events);
        }
        OpenAIRealtimeServerEvent::OutputAudioDelta => {}
        OpenAIRealtimeServerEvent::SessionClosed => {
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Rejected(
                    OpenAIRealtimeClientError::TransportFailure,
                ));
                return true;
            }
            if !context.inner.is_closing.load(Ordering::SeqCst) {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: "transport_error".into(),
                        message: GENERIC_TRANSPORT_ERROR.into(),
                    },
                );
                return true;
            }
            let events = context.inner.committer.lock().await.finish();
            emit_all_if_current(context, events);
            emit_if_current(context, LiveTranslateServerEvent::SessionFinished);
            context
                .inner
                .received_session_closed
                .store(true, Ordering::SeqCst);
            context.inner.close_notify.notify_waiters();
            return true;
        }
        OpenAIRealtimeServerEvent::ProviderError {
            failure,
            related_event_id,
        } => {
            if *context.setup.borrow() == SetupState::Awaiting {
                if !failure.ends_active_session()
                    && related_event_id
                        .as_deref()
                        .is_some_and(|id| id != context.setup_event_id)
                {
                    // An explicit error for another request does not reject
                    // our pending session.update acknowledgement.
                    return false;
                }
                // A pre-ready rejection cannot become a timeout just because
                // the service omitted its related event id. "Recoverable"
                // means an established socket may continue, not permission to
                // reconnect an unaccepted configuration or an unknown error.
                let _ = context.setup.send(SetupState::Rejected(
                    OpenAIRealtimeClientError::setup_rejection(failure),
                ));
                return true;
            }
            if !failure.ends_active_session() {
                // Recoverable provider errors keep the session open. A fixed
                // diagnostic kind cannot expose provider text or error fields.
                crate::pipeline_log!("openai_realtime recoverable_provider_error");
                return false;
            }
            emit_if_current(
                context,
                LiveTranslateServerEvent::Error {
                    code: failure.label().into(),
                    message: failure.label().into(),
                },
            );
            return true;
        }
        OpenAIRealtimeServerEvent::Ignored { kind } => {
            emit_if_current(context, LiveTranslateServerEvent::Ignored { kind });
        }
    }
    false
}

fn emit_all_if_current(context: &ReceiveContext, events: Vec<LiveTranslateServerEvent>) {
    for event in events {
        emit_if_current(context, event);
    }
}

fn emit_if_current(context: &ReceiveContext, event: LiveTranslateServerEvent) {
    if context.inner.generation.load(Ordering::SeqCst) == context.generation {
        let _ = context.events.send(event);
    }
}

#[cfg(test)]
mod tests;
