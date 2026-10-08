//! Azure OpenAI Realtime Translation WebSocket client.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::models::TargetLanguage;
use crate::core::openai_transcript_committer::OpenAITranscriptPairCommitter;
use crate::core::protocols::azure_openai_realtime::{
    AzureOpenAIRealtimeEndpoint, AzureOpenAIRealtimeRequestEncoder, AzureOpenAIRealtimeServerEvent,
    AzureTranslationStream,
};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
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

const GENERIC_PROVIDER_ERROR: &str = "Azure OpenAI Realtime Translation rejected the session.";
const GENERIC_PROTOCOL_ERROR: &str =
    "Azure OpenAI Realtime Translation returned an invalid response.";
const GENERIC_TRANSPORT_ERROR: &str = "The Azure OpenAI Realtime Translation connection failed.";
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AzureOpenAIRealtimeClientError {
    #[error("credential_authentication_failed")]
    AuthenticationFailed,
    #[error("Add an Azure OpenAI API key in Settings.")]
    MissingAPIKey,
    #[error("Enter a valid Azure OpenAI resource endpoint in Settings.")]
    InvalidResourceEndpoint,
    #[error("Enter Azure OpenAI translation and transcription deployment names in Settings.")]
    MissingDeployment,
    #[error("Azure OpenAI Realtime Translation requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("The Azure OpenAI Realtime Translation session is not connected.")]
    NotConnected,
    #[error("The Azure OpenAI Realtime Translation connection stopped responding.")]
    HealthCheckTimedOut,
    #[error("The Azure OpenAI Realtime Translation connection failed.")]
    TransportFailure,
    #[error("Azure OpenAI Realtime Translation rejected the session configuration.")]
    SessionSetupRejected,
    #[error(
        "Azure OpenAI Realtime Translation did not confirm the session configuration in time."
    )]
    SessionSetupTimedOut,
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected,
}

#[derive(Default)]
struct TranslationStreamState {
    selected: Option<AzureTranslationStream>,
    current_text: String,
}

impl TranslationStreamState {
    fn reset(&mut self) {
        self.selected = None;
        self.current_text.clear();
    }

    fn append_delta(&mut self, stream: AzureTranslationStream, delta: &str) -> Option<String> {
        if self.selected.is_none() {
            self.selected = Some(stream);
        }
        if self.selected != Some(stream) || delta.is_empty() {
            return None;
        }
        self.current_text.push_str(delta);
        Some(delta.to_string())
    }

    fn finish(
        &mut self,
        stream: AzureTranslationStream,
        final_text: Option<String>,
    ) -> Option<String> {
        if self.selected.is_none() {
            self.selected = Some(stream);
        }
        if self.selected != Some(stream) {
            return None;
        }
        let suffix = final_text.and_then(|final_text| {
            if self.current_text.is_empty() {
                Some(final_text)
            } else {
                final_text
                    .strip_prefix(&self.current_text)
                    .filter(|suffix| !suffix.is_empty())
                    .map(str::to_string)
            }
        });
        self.current_text.clear();
        suffix
    }
}

struct Inner {
    // Serializes the content barrier with local assembly, never socket/audio state.
    content_lock: Mutex<()>,
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    audio_send_lock: Mutex<()>,
    pending_audio: Mutex<Vec<u8>>,
    committer: Mutex<OpenAITranscriptPairCommitter>,
    translation_stream: Mutex<TranslationStreamState>,
    ready: AtomicBool,
    is_closing: AtomicBool,
    received_session_closed: AtomicBool,
    close_notify: Notify,
    pong_notify: Notify,
    generation: AtomicU64,
}

#[derive(Clone)]
pub struct AzureOpenAIRealtimeClient {
    network: super::provider_network::ProviderNetwork,
    inner: Arc<Inner>,
    endpoint: url::Url,
    api_key: String,
    transcription_deployment: String,
    target_language: TargetLanguage,
    events: ProviderEventSender,
}

impl AzureOpenAIRealtimeClient {
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.clone()
    }

    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    /// Clears locally accepted subtitles without reconnecting or dropping audio.
    /// This protocol has no source-turn boundary: future deltas remain valid.
    pub async fn clear_content(&self) -> u64 {
        let _content = self.inner.content_lock.lock().await;
        self.inner.translation_stream.lock().await.reset();
        self.inner.committer.lock().await.reset();
        self.events.advance_content_revision()
    }

    /// Applied before connect so ASR and translation share one immutable route.
    pub fn set_network(
        &mut self,
        network: super::provider_network::ProviderNetwork,
    ) -> Result<(), super::provider_network::ProviderNetworkError> {
        self.network = network;
        Ok(())
    }

    pub fn new(
        resource_endpoint: &str,
        deployment: &str,
        transcription_deployment: &str,
        api_key: &str,
        target_language: TargetLanguage,
        events: ProviderEventSender,
    ) -> Result<Self, AzureOpenAIRealtimeClientError> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(AzureOpenAIRealtimeClientError::MissingAPIKey);
        }
        if deployment.trim().is_empty() {
            return Err(AzureOpenAIRealtimeClientError::MissingDeployment);
        }
        if transcription_deployment.trim().is_empty() {
            return Err(AzureOpenAIRealtimeClientError::MissingDeployment);
        }
        let endpoint = AzureOpenAIRealtimeEndpoint::new(resource_endpoint, deployment)
            .map_err(|_| AzureOpenAIRealtimeClientError::InvalidResourceEndpoint)?;
        Self::with_endpoint(
            api_key,
            transcription_deployment,
            target_language,
            events,
            endpoint.url().clone(),
        )
    }

    fn with_endpoint(
        api_key: &str,
        transcription_deployment: &str,
        target_language: TargetLanguage,
        events: ProviderEventSender,
        endpoint: url::Url,
    ) -> Result<Self, AzureOpenAIRealtimeClientError> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(AzureOpenAIRealtimeClientError::MissingAPIKey);
        }
        let transcription_deployment = transcription_deployment.trim();
        if transcription_deployment.is_empty() {
            return Err(AzureOpenAIRealtimeClientError::MissingDeployment);
        }
        if !target_language.translates_audio() {
            return Err(AzureOpenAIRealtimeClientError::InvalidTargetLanguage);
        }
        Ok(Self {
            network: super::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                content_lock: Mutex::new(()),
                sink: Mutex::new(None),
                receive_task: Mutex::new(None),
                audio_send_lock: Mutex::new(()),
                pending_audio: Mutex::new(Vec::new()),
                committer: Mutex::new(OpenAITranscriptPairCommitter::default()),
                translation_stream: Mutex::new(TranslationStreamState::default()),
                ready: AtomicBool::new(false),
                is_closing: AtomicBool::new(false),
                received_session_closed: AtomicBool::new(false),
                close_notify: Notify::new(),
                pong_notify: Notify::new(),
                generation: AtomicU64::new(0),
            }),
            endpoint,
            api_key: api_key.to_string(),
            transcription_deployment: transcription_deployment.to_string(),
            target_language,
            events,
        })
    }

    pub async fn connect(&self) -> Result<(), AzureOpenAIRealtimeClientError> {
        self.connect_with_timeout(Duration::from_secs(5)).await
    }

    async fn connect_with_timeout(
        &self,
        readiness_timeout: Duration,
    ) -> Result<(), AzureOpenAIRealtimeClientError> {
        self.disconnect().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let setup_event_id = format!("mimi-azure-session-update-{generation}");

        let mut request = self
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|_| AzureOpenAIRealtimeClientError::TransportFailure)?;
        request.headers_mut().insert(
            "api-key",
            HeaderValue::from_str(&self.api_key)
                .map_err(|_| AzureOpenAIRealtimeClientError::MissingAPIKey)?,
        );

        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            super::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| AzureOpenAIRealtimeClientError::TransportFailure)?
        .map_err(|error| {
            if super::connection_diagnostics::authentication_rejected(&error) {
                AzureOpenAIRealtimeClientError::AuthenticationFailed
            } else {
                AzureOpenAIRealtimeClientError::TransportFailure
            }
        })?;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .received_session_closed
            .store(false, Ordering::SeqCst);
        self.inner.pending_audio.lock().await.clear();
        self.inner.committer.lock().await.reset();
        self.inner.translation_stream.lock().await.reset();

        let (setup_tx, mut setup_rx) = watch::channel(SetupState::Awaiting);
        let task = tokio::spawn(receive_loop(ReceiveContext {
            inner: Arc::clone(&self.inner),
            stream,
            events: self.events.clone(),
            target_language: self.target_language,
            setup_event_id: setup_event_id.clone(),
            setup: setup_tx,
            generation,
        }));
        *self.inner.receive_task.lock().await = Some(task);

        let update = AzureOpenAIRealtimeRequestEncoder::session_update(
            self.target_language,
            &self.transcription_deployment,
            Some(&setup_event_id),
        )
        .map_err(|_| AzureOpenAIRealtimeClientError::InvalidTargetLanguage)?;
        let complete_setup = async {
            self.send_text(update.to_string()).await?;
            loop {
                match setup_rx.borrow().clone() {
                    SetupState::Ready => return Ok(()),
                    SetupState::Rejected => {
                        return Err(AzureOpenAIRealtimeClientError::SessionSetupRejected)
                    }
                    SetupState::Awaiting => {}
                }
                if setup_rx.changed().await.is_err() {
                    return Err(AzureOpenAIRealtimeClientError::SessionSetupRejected);
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
                Err(AzureOpenAIRealtimeClientError::SessionSetupTimedOut)
            }
        }
    }

    /// Accepts arbitrary PCM chunks and sends only exact 200 ms frames.
    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), AzureOpenAIRealtimeClientError> {
        if pcm_data.is_empty() {
            return Ok(());
        }
        tokio::time::timeout(SEND_TIMEOUT, self.send_audio_operation(pcm_data))
            .await
            .map_err(|_| AzureOpenAIRealtimeClientError::TransportFailure)?
    }

    async fn send_audio_operation(
        &self,
        pcm_data: &[u8],
    ) -> Result<(), AzureOpenAIRealtimeClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(AzureOpenAIRealtimeClientError::NotConnected);
        }
        let _guard = self.inner.audio_send_lock.lock().await;
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

    pub async fn ping(&self, timeout: Duration) -> Result<(), AzureOpenAIRealtimeClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(AzureOpenAIRealtimeClientError::NotConnected);
        }
        let operation = async {
            let pong = self.inner.pong_notify.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(AzureOpenAIRealtimeClientError::NotConnected);
                };
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| AzureOpenAIRealtimeClientError::TransportFailure)?;
            }
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| AzureOpenAIRealtimeClientError::HealthCheckTimedOut)?
    }

    pub async fn finish(&self, timeout: Duration) {
        if !self.inner.ready.load(Ordering::SeqCst)
            || self.inner.is_closing.swap(true, Ordering::SeqCst)
        {
            return;
        }
        let generation = self.inner.generation.load(Ordering::SeqCst);
        match tokio::time::timeout(timeout, self.finish_operation(generation)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) if self.is_current_generation(generation) => self.emit(
                LiveTranslateServerEvent::Error {
                    code: "azure_openai_session_close_failed".into(),
                    message: GENERIC_TRANSPORT_ERROR.into(),
                },
                generation,
            ),
            Err(_) if self.is_current_generation(generation) => self.emit(
                LiveTranslateServerEvent::Error {
                    code: "azure_openai_close_timeout".into(),
                    message: "Azure OpenAI Realtime Translation did not finish closing in time."
                        .into(),
                },
                generation,
            ),
            _ => {}
        }
        self.disconnect_if_current(generation).await;
    }

    async fn finish_operation(
        &self,
        generation: u64,
    ) -> Result<(), AzureOpenAIRealtimeClientError> {
        let _guard = self.inner.audio_send_lock.lock().await;
        if !self.is_current_generation(generation) {
            return Err(AzureOpenAIRealtimeClientError::NotConnected);
        }
        let partial = {
            let mut pending = self.inner.pending_audio.lock().await;
            take_padded_audio_message(&mut pending)?
        };
        if let Some(message) = partial {
            self.send_text(message).await?;
        }
        self.send_text(AzureOpenAIRealtimeRequestEncoder::close().to_string())
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
        if let Some(mut sink) = self.inner.sink.lock().await.take() {
            let _ = tokio::time::timeout(Duration::from_millis(250), sink.close()).await;
        }
        self.inner.pending_audio.lock().await.clear();
        self.inner.committer.lock().await.reset();
        self.inner.translation_stream.lock().await.reset();
        self.inner.close_notify.notify_waiters();
    }

    async fn send_text(&self, text: String) -> Result<(), AzureOpenAIRealtimeClientError> {
        let mut sink = self.inner.sink.lock().await;
        let Some(sink) = sink.as_mut() else {
            return Err(AzureOpenAIRealtimeClientError::NotConnected);
        };
        let evidence = crate::development_audio::begin_json(
            &text,
            AzureOpenAIRealtimeEndpoint::SAMPLE_RATE_HZ,
        );
        tokio::time::timeout(
            SEND_TIMEOUT,
            evidence.observe(sink.send(Message::Text(text.into()))),
        )
        .await
        .map_err(|_| AzureOpenAIRealtimeClientError::TransportFailure)?
        .map_err(|_| AzureOpenAIRealtimeClientError::TransportFailure)
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
) -> Result<Vec<String>, AzureOpenAIRealtimeClientError> {
    let frame_size = AzureOpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let complete_bytes = pending.len() / frame_size * frame_size;
    let result = pending[..complete_bytes]
        .chunks_exact(frame_size)
        .map(|frame| {
            AzureOpenAIRealtimeRequestEncoder::audio_append(frame)
                .map(|value| value.to_string())
                .map_err(|_| AzureOpenAIRealtimeClientError::TransportFailure)
        })
        .collect::<Result<Vec<_>, _>>();
    // Preserve the existing failure semantics: a complete batch is consumed
    // even if encoding one of its frames fails.
    pending.drain(..complete_bytes);
    result
}

fn take_padded_audio_message(
    pending: &mut Vec<u8>,
) -> Result<Option<String>, AzureOpenAIRealtimeClientError> {
    if pending.is_empty() {
        return Ok(None);
    }
    let mut frame = std::mem::take(pending);
    frame.resize(AzureOpenAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT, 0);
    AzureOpenAIRealtimeRequestEncoder::audio_append(&frame)
        .map(|value| Some(value.to_string()))
        .map_err(|_| AzureOpenAIRealtimeClientError::TransportFailure)
}

struct ReceiveContext {
    inner: Arc<Inner>,
    stream: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    events: ProviderEventSender,
    target_language: TargetLanguage,
    setup_event_id: String,
    setup: watch::Sender<SetupState>,
    generation: u64,
}

async fn receive_loop(mut context: ReceiveContext) {
    while let Some(message) = context.stream.next().await {
        if context.inner.generation.load(Ordering::SeqCst) != context.generation {
            return;
        }
        let event = match message {
            Ok(Message::Text(text)) => AzureOpenAIRealtimeServerEvent::decode(&text),
            Ok(Message::Binary(data)) => {
                AzureOpenAIRealtimeServerEvent::decode(&String::from_utf8_lossy(&data))
            }
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
                fail_receive_loop(
                    &context,
                    "azure_openai_protocol_error",
                    GENERIC_PROTOCOL_ERROR,
                )
                .await;
                return;
            }
        };
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
        let _ = context.setup.send(SetupState::Rejected);
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

async fn handle_server_event(
    context: &ReceiveContext,
    event: AzureOpenAIRealtimeServerEvent,
) -> bool {
    let revision = context.events.content_revision();
    let _content = context.inner.content_lock.lock().await;
    if context.inner.generation.load(Ordering::SeqCst) != context.generation {
        return false;
    }
    if revision != context.events.content_revision()
        && matches!(
            &event,
            AzureOpenAIRealtimeServerEvent::SourceTranscriptDelta { .. }
                | AzureOpenAIRealtimeServerEvent::TranslationTranscriptDelta { .. }
                | AzureOpenAIRealtimeServerEvent::TranslationTranscriptDone { .. }
        )
    {
        return false;
    }

    match event {
        AzureOpenAIRealtimeServerEvent::SessionCreated => {
            emit_if_current(context, LiveTranslateServerEvent::SessionCreated);
        }
        AzureOpenAIRealtimeServerEvent::SessionUpdated { target_language } => {
            if target_language != context.target_language.raw_value() {
                let _ = context.setup.send(SetupState::Rejected);
                return true;
            }
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Ready);
            } else {
                emit_if_current(context, LiveTranslateServerEvent::SessionUpdated);
            }
        }
        AzureOpenAIRealtimeServerEvent::SourceTranscriptDelta { text, elapsed_ms } => {
            let events = context
                .inner
                .committer
                .lock()
                .await
                .append_source_delta(&text, elapsed_ms);
            emit_all_if_current(context, events);
        }
        AzureOpenAIRealtimeServerEvent::TranslationTranscriptDelta {
            text,
            elapsed_ms,
            stream,
        } => {
            let delta = context
                .inner
                .translation_stream
                .lock()
                .await
                .append_delta(stream, &text);
            if let Some(delta) = delta {
                let events = context
                    .inner
                    .committer
                    .lock()
                    .await
                    .append_translation_delta(&delta, elapsed_ms);
                emit_all_if_current(context, events);
            }
        }
        AzureOpenAIRealtimeServerEvent::TranslationTranscriptDone { text, stream } => {
            let suffix = context
                .inner
                .translation_stream
                .lock()
                .await
                .finish(stream, text);
            if let Some(suffix) = suffix {
                let events = context
                    .inner
                    .committer
                    .lock()
                    .await
                    .append_translation_delta(&suffix, None);
                emit_all_if_current(context, events);
            }
        }
        AzureOpenAIRealtimeServerEvent::OutputAudioDelta => {}
        AzureOpenAIRealtimeServerEvent::SessionClosed => {
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Rejected);
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
        AzureOpenAIRealtimeServerEvent::ProviderError {
            code,
            is_recoverable,
            related_event_id,
        } => {
            if *context.setup.borrow() == SetupState::Awaiting
                && (related_event_id.as_deref() == Some(context.setup_event_id.as_str())
                    || !is_recoverable)
            {
                let _ = context.setup.send(SetupState::Rejected);
                return true;
            }
            if !is_recoverable {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: format!("azure_openai_provider_error.{code}"),
                        message: GENERIC_PROVIDER_ERROR.into(),
                    },
                );
                return true;
            }
        }
        AzureOpenAIRealtimeServerEvent::Ignored { kind } => {
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
mod tests {
    #[tokio::test]
    async fn handshake_authentication_rejections_preserve_transport_failures() {
        for (status, expected_auth_label) in
            crate::clients::connection_diagnostics::handshake_failure_cases()
        {
            let (endpoint, server) =
                crate::clients::connection_diagnostics::rejected_websocket_endpoint(status).await;
            let (events, _receiver) = provider_event_channel();
            let mut client = AzureOpenAIRealtimeClient::with_endpoint(
                "test-key-not-real",
                "test-transcription",
                TargetLanguage::Japanese,
                events,
                endpoint,
            )
            .unwrap();
            client.network = super::super::provider_network::ProviderNetwork::resolve(
                &crate::core::network_proxy::ProxyConfig {
                    mode: crate::core::network_proxy::ProxyMode::Direct,
                    url: None,
                },
            )
            .unwrap();
            let error = client.connect().await.unwrap_err();
            server.await.unwrap();
            if let Some(expected_label) = expected_auth_label {
                assert_eq!(error, AzureOpenAIRealtimeClientError::AuthenticationFailed);
                assert_eq!(error.to_string(), expected_label);
            } else {
                assert_eq!(error, AzureOpenAIRealtimeClientError::TransportFailure);
            }
            assert!(!error.to_string().contains("private-handshake-body"));
            assert!(!client.inner.ready.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn clear_discards_local_tail_and_queued_content_without_touching_audio_or_lifecycle() {
        let (sender, mut receiver) = provider_event_channel();
        let endpoint = url::Url::parse("ws://127.0.0.1:1/realtime").unwrap();
        let client = AzureOpenAIRealtimeClient::with_endpoint(
            "test-key-not-real",
            "test-transcription",
            TargetLanguage::Japanese,
            sender.clone(),
            endpoint,
        )
        .unwrap();
        client.inner.ready.store(true, Ordering::SeqCst);
        client.inner.generation.store(7, Ordering::SeqCst);
        *client.inner.pending_audio.lock().await = vec![1, 2, 3];
        client
            .inner
            .committer
            .lock()
            .await
            .append_source_delta("old source", None);
        client
            .inner
            .committer
            .lock()
            .await
            .append_translation_delta("old translation", None);
        client
            .inner
            .translation_stream
            .lock()
            .await
            .append_delta(AzureTranslationStream::DedicatedSession, "old translation");
        sender
            .send(LiveTranslateServerEvent::SubtitleFinalPair {
                source: "old queued source".into(),
                language: None,
                translation: "old queued translation".into(),
            })
            .unwrap();
        sender
            .send(LiveTranslateServerEvent::SessionUpdated)
            .unwrap();
        assert_eq!(client.clear_content().await, 1);
        assert_eq!(client.content_revision(), 1);
        assert_eq!(client.inner.generation.load(Ordering::SeqCst), 7);
        assert!(client.inner.ready.load(Ordering::SeqCst));
        assert_eq!(*client.inner.pending_audio.lock().await, vec![1, 2, 3]);
        assert!(client.inner.committer.lock().await.finish().is_empty());
        assert!(client
            .inner
            .translation_stream
            .lock()
            .await
            .current_text
            .is_empty());
        assert!(client
            .inner
            .translation_stream
            .lock()
            .await
            .selected
            .is_none());
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionUpdated)
        );
        assert!(receiver.try_recv().is_err());
        let mut committer = client.inner.committer.lock().await;
        committer.append_source_delta("new source", None);
        committer.append_translation_delta("new translation", None);
        assert!(committer.finish().iter().any(|event| matches!(event,
            LiveTranslateServerEvent::SubtitleFinalPair {source,translation,..}
                if source == "new source" && translation == "new translation")));
    }

    use super::*;
    use crate::clients::provider_events::{provider_event_channel, ProviderEventReceiver};
    use base64::Engine;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::Value;
    use tokio::net::TcpListener;

    async fn test_client(
        server: impl FnOnce(
                WebSocketStream<TcpStream>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + 'static,
    ) -> (AzureOpenAIRealtimeClient, ProviderEventReceiver) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server(socket).await;
        });
        let (events, receiver) = provider_event_channel();
        let endpoint = url::Url::parse(&format!("ws://{address}/realtime")).unwrap();
        let client = AzureOpenAIRealtimeClient::with_endpoint(
            "azure-test-key-not-real",
            "gpt-4o-transcribe-deployment",
            TargetLanguage::Japanese,
            events,
            endpoint,
        )
        .unwrap();
        (client, receiver)
    }

    #[tokio::test]
    async fn mock_websocket_runs_translation_and_graceful_close() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let update: Value = serde_json::from_str(
                    socket.next().await.unwrap().unwrap().to_text().unwrap(),
                )
                .unwrap();
                assert_eq!(update["session"]["audio"]["output"]["language"], "ja");
                assert_eq!(
                    update["session"]["audio"]["input"]["transcription"]["model"],
                    "gpt-4o-transcribe-deployment"
                );
                socket
                    .send(Message::Text(
                        r#"{"type":"session.updated","session":{"audio":{"output":{"language":"ja"}}}}"#
                            .into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"session.input_transcript.delta","delta":"Hello."}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"response.text.delta","text":"こんにちは。"}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(r#"{"type":"response.text.done"}"#.into()))
                    .await
                    .unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if message
                        .to_text()
                        .ok()
                        .is_some_and(|text| text.contains("session.close"))
                    {
                        socket
                            .send(Message::Text(r#"{"type":"session.closed"}"#.into()))
                            .await
                            .unwrap();
                        break;
                    }
                }
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        client.finish(Duration::from_millis(500)).await;

        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. }
                if source == "Hello." && translation == "こんにちは。"
        )));
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    }

    #[test]
    fn frame_buffer_is_bounded_and_preserves_tail() {
        let mut pending = vec![0x41; 9_600 * 2 + 17];
        let messages = take_complete_audio_messages(&mut pending).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(pending.len(), 17);
        for message in messages {
            let value: Value = serde_json::from_str(&message).unwrap();
            let frame = base64::engine::general_purpose::STANDARD
                .decode(value["audio"].as_str().unwrap())
                .unwrap();
            assert_eq!(frame.len(), 9_600);
        }
    }

    #[tokio::test]
    async fn setup_timeout_is_finite() {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                tokio::time::sleep(Duration::from_secs(2)).await;
            })
        })
        .await;
        assert_eq!(
            client
                .connect_with_timeout(Duration::from_millis(40))
                .await
                .unwrap_err(),
            AzureOpenAIRealtimeClientError::SessionSetupTimedOut
        );
    }
}
