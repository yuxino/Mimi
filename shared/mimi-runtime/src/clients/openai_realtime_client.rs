//! OpenAI Realtime Translation WebSocket client.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::models::TargetLanguage;
use crate::core::openai_transcript_committer::OpenAITranscriptPairCommitter;
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::openai_realtime::{
    OpenAIRealtimeEndpoint, OpenAIRealtimeRequestEncoder, OpenAIRealtimeServerEvent,
};
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

const GENERIC_PROVIDER_ERROR: &str = "OpenAI Realtime Translation rejected the session.";
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
    #[error("OpenAI Realtime Translation did not confirm the session configuration in time.")]
    SessionSetupTimedOut,
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected,
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
    network: super::provider_network::ProviderNetwork,
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
        network: super::provider_network::ProviderNetwork,
    ) -> Result<(), super::provider_network::ProviderNetworkError> {
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

    pub(super) fn with_endpoint(
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
            network: super::provider_network::ProviderNetwork::default(),
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
            super::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| OpenAIRealtimeClientError::TransportFailure)?
        .map_err(|error| {
            if super::connection_diagnostics::authentication_rejected(&error) {
                OpenAIRealtimeClientError::AuthenticationFailed
            } else {
                OpenAIRealtimeClientError::TransportFailure
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
                    SetupState::Rejected => {
                        return Err(OpenAIRealtimeClientError::SessionSetupRejected)
                    }
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
            Ok(Message::Text(text)) => OpenAIRealtimeServerEvent::decode(&text),
            Ok(Message::Binary(data)) => {
                OpenAIRealtimeServerEvent::decode(&String::from_utf8_lossy(&data))
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
                fail_receive_loop(&context, "openai_protocol_error", GENERIC_PROTOCOL_ERROR).await;
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
                    let _ = context.setup.send(SetupState::Rejected);
                } else {
                    emit_if_current(
                        context,
                        LiveTranslateServerEvent::Error {
                            code: "openai_session_configuration_mismatch".into(),
                            message: GENERIC_PROVIDER_ERROR.into(),
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
        OpenAIRealtimeServerEvent::ProviderError {
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
            if is_recoverable {
                return false;
            }
            emit_if_current(
                context,
                LiveTranslateServerEvent::Error {
                    code: format!("openai_provider_error.{code}"),
                    message: GENERIC_PROVIDER_ERROR.into(),
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
mod tests {
    #[tokio::test]
    async fn clear_discards_local_tail_and_queued_content_without_touching_audio_or_lifecycle() {
        let (sender, mut receiver) = provider_event_channel();
        let endpoint = url::Url::parse("ws://127.0.0.1:1/realtime").unwrap();
        let client = OpenAIRealtimeClient::with_endpoint(
            "test-key-not-real",
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
    use tokio::net::TcpListener;

    async fn test_client(
        server: impl FnOnce(
                WebSocketStream<TcpStream>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + 'static,
    ) -> (OpenAIRealtimeClient, ProviderEventReceiver) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server(socket).await;
        });
        let (events, receiver) = provider_event_channel();
        let endpoint = url::Url::parse(&format!("ws://{address}/realtime")).unwrap();
        let client = OpenAIRealtimeClient::with_endpoint(
            "sk-test-not-real",
            TargetLanguage::Japanese,
            events,
            endpoint,
        )
        .unwrap();
        (client, receiver)
    }

    fn acknowledge_session(
        mut socket: WebSocketStream<TcpStream>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let update = socket.next().await.unwrap().unwrap();
            let update: Value = serde_json::from_str(update.to_text().unwrap()).unwrap();
            assert_eq!(update["type"], "session.update");
            socket
                .send(Message::Text(
                    r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                        .into(),
                ))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        })
    }

    use serde_json::Value;

    #[tokio::test]
    async fn connect_waits_for_valid_setup_acknowledgement() {
        let (client, _events) = test_client(acknowledge_session).await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn an_immediate_pong_cannot_be_lost_before_the_waiter_is_polled() {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                            .into(),
                    ))
                    .await
                    .unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if let Message::Ping(payload) = message {
                        socket.send(Message::Pong(payload)).await.unwrap();
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

        client.ping(Duration::from_millis(200)).await.unwrap();
        client.disconnect().await;
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
                .connect_with_timeout(Duration::from_millis(50))
                .await
                .unwrap_err(),
            OpenAIRealtimeClientError::SessionSetupTimedOut
        );
    }

    #[test]
    fn frame_buffer_keeps_only_a_partial_tail() {
        let mut pending = vec![0x11; 9_600 * 2 + 17];
        let messages = take_complete_audio_messages(&mut pending).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(pending.len(), 17);
        for message in messages {
            let value: Value = serde_json::from_str(&message).unwrap();
            let frame = base64::engine::general_purpose::STANDARD
                .decode(value["audio"].as_str().unwrap())
                .unwrap();
            assert_eq!(frame.len(), 9_600);
            assert!(frame.iter().all(|byte| *byte == 0x11));
        }
    }

    #[test]
    fn final_partial_frame_is_zero_padded() {
        let mut pending = vec![1, 2, 3];
        let message = take_padded_audio_message(&mut pending).unwrap().unwrap();
        let value: Value = serde_json::from_str(&message).unwrap();
        let frame = base64::engine::general_purpose::STANDARD
            .decode(value["audio"].as_str().unwrap())
            .unwrap();
        assert_eq!(frame.len(), 9_600);
        assert_eq!(&frame[..3], &[1, 2, 3]);
        assert!(frame[3..].iter().all(|byte| *byte == 0));
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn recoverable_error_does_not_stop_transcript_flow() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(Message::Text(
                    r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#.into()
                )).await.unwrap();
                socket.send(Message::Text(
                    r#"{"type":"error","error":{"type":"invalid_request_error","code":"invalid_event","message":"private"}}"#.into()
                )).await.unwrap();
                socket.send(Message::Text(
                    r#"{"type":"session.input_transcript.delta","delta":"Hello."}"#.into()
                )).await.unwrap();
                socket.send(Message::Text(
                    r#"{"type":"session.output_transcript.delta","delta":"こんにちは。"}"#.into()
                )).await.unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if message.to_text().ok().is_some_and(|text| text.contains("session.close")) {
                        socket.send(Message::Text(r#"{"type":"session.closed"}"#.into())).await.unwrap();
                        break;
                    }
                }
            })
        }).await;
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
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
    }

    #[tokio::test]
    async fn close_timeout_is_finite_and_drops_an_unconfirmed_tail() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                            .into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"session.input_transcript.delta","delta":"unfinished source"}"#
                            .into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"session.output_transcript.delta","delta":"未完の訳"}"#
                            .into(),
                    ))
                    .await
                    .unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if message
                        .to_text()
                        .ok()
                        .is_some_and(|text| text.contains("session.close"))
                    {
                        tokio::time::sleep(Duration::from_secs(2)).await;
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
        let started = tokio::time::Instant::now();
        client.finish(Duration::from_millis(50)).await;
        assert!(started.elapsed() < Duration::from_secs(1));

        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::Error { code, .. } if code == "openai_close_timeout"
        )));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. })));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    }

    #[tokio::test]
    async fn graceful_session_closed_flushes_one_atomic_tail_pair() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                            .into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"session.input_transcript.delta","delta":"final source tail"}"#
                            .into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"session.output_transcript.delta","delta":"最後の訳"}"#
                            .into(),
                    ))
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
        assert_eq!(
            received
                .iter()
                .filter(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. }))
                .count(),
            1
        );
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. }
                if source == "final source tail" && translation == "最後の訳"
        )));
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    }

    #[tokio::test]
    async fn unsolicited_session_closed_requests_transport_recovery() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                            .into(),
                    ))
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
                socket
                    .send(Message::Text(r#"{"type":"session.closed"}"#.into()))
                    .await
                    .unwrap();
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("transport recovery event should arrive before the timeout")
            .expect("transport recovery event channel should stay open");
        assert!(matches!(
            event,
            LiveTranslateServerEvent::Error { code, message }
                if code == "transport_error" && message == GENERIC_TRANSPORT_ERROR
        ));
        client.disconnect().await;
    }

    #[tokio::test]
    async fn stale_socket_events_are_suppressed_after_disconnect() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"type":"session.updated","session":{"audio":{"input":{"transcription":{"model":"gpt-realtime-whisper"}},"output":{"language":"ja"}}}}"#
                            .into(),
                    ))
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;
                let _ = socket
                    .send(Message::Text(
                        r#"{"type":"session.input_transcript.delta","delta":"stale private text"}"#
                            .into(),
                    ))
                    .await;
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.disconnect().await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(events.try_recv().is_err());
    }
}
