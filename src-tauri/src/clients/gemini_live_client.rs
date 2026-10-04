//! Google Gemini Live Translation WebSocket client.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::models::TargetLanguage;
use crate::core::protocols::gemini_live::{
    GeminiLiveEndpoint, GeminiLiveRequestEncoder, GeminiLiveServerEvent,
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
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const GENERIC_PROVIDER_ERROR: &str = "Gemini Live Translation rejected the session.";
const GENERIC_PROTOCOL_ERROR: &str = "Gemini Live Translation returned an invalid response.";
const GENERIC_TRANSPORT_ERROR: &str = "The Gemini Live Translation connection failed.";
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
// Gemini can legally deliver transcript chunks after `turnComplete` and does
// not provide a separate transcript-terminal event. Treat every normal turn
// boundary (and the final close boundary) as provisional until transcripts
// have stayed quiet long enough to absorb ordinary network jitter.
const TAIL_QUIET_PERIOD: Duration =
    Duration::from_millis(mimi_core::openai_transcript_committer::GEMINI_TURN_TAIL_QUIET_MS);
#[cfg(test)]
const MAXIMUM_TRANSCRIPT_BYTES: usize = 128 * 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GeminiLiveClientError {
    #[error("Add a Google Gemini API key in Settings.")]
    MissingAPIKey,
    #[error("Gemini Live Translation requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("The Gemini Live Translation session is not connected.")]
    NotConnected,
    #[error("The Gemini Live Translation connection stopped responding.")]
    HealthCheckTimedOut,
    #[error("The Gemini Live Translation connection failed.")]
    TransportFailure,
    #[error("Gemini Live Translation rejected the session configuration.")]
    SessionSetupRejected,
    #[error("Gemini Live Translation did not confirm the session configuration in time.")]
    SessionSetupTimedOut,
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected,
}

struct GeminiTranscriptPairCommitter {
    stream: crate::core::openai_transcript_committer::OpenAITranscriptPairCommitter,
    source_language: Option<String>,
    clock: std::time::Instant,
    discard_current_turn: bool,
    turn_complete_received: bool,
}

impl Default for GeminiTranscriptPairCommitter {
    fn default() -> Self {
        Self {
            stream:
                crate::core::openai_transcript_committer::OpenAITranscriptPairCommitter::new_gemini(
                ),
            source_language: None,
            clock: std::time::Instant::now(),
            discard_current_turn: false,
            turn_complete_received: false,
        }
    }
}

impl GeminiTranscriptPairCommitter {
    fn elapsed_ms(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
    fn settle(&mut self) -> Vec<LiveTranslateServerEvent> {
        let events = self.stream.settle(self.elapsed_ms());
        self.adapt(events)
    }
    fn append_source(
        &mut self,
        text: &str,
        language_code: Option<String>,
    ) -> Vec<LiveTranslateServerEvent> {
        if language_code.is_some() {
            self.source_language = language_code;
        }
        let events = self
            .stream
            .append_source_delta(text, Some(self.elapsed_ms()));
        self.adapt(events)
    }

    fn append_translation(&mut self, text: &str) -> Vec<LiveTranslateServerEvent> {
        let events = self
            .stream
            .append_translation_delta(text, Some(self.elapsed_ms()));
        self.adapt(events)
    }

    fn adapt(&mut self, events: Vec<LiveTranslateServerEvent>) -> Vec<LiveTranslateServerEvent> {
        let mut adapted = Vec::new();
        for mut event in events {
            if matches!(event, LiveTranslateServerEvent::Error { .. }) {
                self.discard_current_turn = true;
                adapted.push(LiveTranslateServerEvent::Error {
                    code: "gemini_transcript_safety_limit".into(),
                    message:
                        "Gemini Live Translation transcript buffering exceeded its safety limit."
                            .into(),
                });
                continue;
            }
            if self.discard_current_turn {
                if matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. }) {
                    self.discard_current_turn = false;
                }
                continue;
            }
            match &mut event {
                LiveTranslateServerEvent::SourceDraft { language, .. }
                | LiveTranslateServerEvent::SubtitleFinalPair { language, .. } => {
                    *language = self.source_language.clone()
                }
                _ => {}
            }
            adapted.push(event);
        }
        adapted
    }

    fn finish_turn(&mut self) -> Vec<LiveTranslateServerEvent> {
        let events = self.stream.finish();
        let events = self.adapt(events);
        self.reset();
        events
    }

    fn has_pending(&self) -> bool {
        self.stream.has_pending()
    }

    fn reset(&mut self) {
        self.stream.reset();
        self.source_language = None;
        self.discard_current_turn = false;
        self.turn_complete_received = false;
    }
}

struct Inner {
    // Serializes the content barrier with local assembly, never socket/audio state.
    content_lock: Mutex<()>,
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    audio_send_lock: Mutex<()>,
    pending_audio: Mutex<Vec<u8>>,
    committer: Mutex<GeminiTranscriptPairCommitter>,
    ready: AtomicBool,
    is_closing: AtomicBool,
    received_final_turn: AtomicBool,
    final_turn_notify: Notify,
    transcript_revision: AtomicU64,
    transcript_notify: Notify,
    turn_boundary_epoch: AtomicU64,
    pong_notify: Notify,
    generation: AtomicU64,
}

/// The API key is deliberately kept in a non-`Debug` type. It is attached to
/// a short-lived connection URL and every transport failure is mapped to a
/// content-free error before leaving this client.
#[derive(Clone)]
pub struct GeminiLiveClient {
    network: super::provider_network::ProviderNetwork,
    inner: Arc<Inner>,
    endpoint: url::Url,
    authenticate_with_query: bool,
    api_key: String,
    target_language: TargetLanguage,
    events: ProviderEventSender,
}

impl GeminiLiveClient {
    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    pub async fn clear_content(&self) -> u64 {
        let _content = self.inner.content_lock.lock().await;
        cancel_normal_turn_boundary(&self.inner);
        let mut committer = self.inner.committer.lock().await;
        let pending_boundary = committer.turn_complete_received;
        let active = committer.discard_current_turn || committer.has_pending();
        committer.reset();
        committer.discard_current_turn = active;
        committer.turn_complete_received = pending_boundary;
        let revision = self.events.advance_content_revision();
        // A boundary already observed before Clear still owns its late tails.
        // Re-arm only that quiet boundary, never create a new server turn.
        if pending_boundary && !self.inner.is_closing.load(Ordering::SeqCst) {
            schedule_normal_turn_commit_for(
                &self.inner,
                &self.events,
                self.inner.generation.load(Ordering::SeqCst),
            );
        }
        revision
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
    ) -> Result<Self, GeminiLiveClientError> {
        let endpoint =
            GeminiLiveEndpoint::url().map_err(|_| GeminiLiveClientError::TransportFailure)?;
        Self::with_endpoint(api_key, target_language, events, endpoint, true)
    }

    fn with_endpoint(
        api_key: &str,
        target_language: TargetLanguage,
        events: ProviderEventSender,
        endpoint: url::Url,
        authenticate_with_query: bool,
    ) -> Result<Self, GeminiLiveClientError> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(GeminiLiveClientError::MissingAPIKey);
        }
        if !target_language.translates_audio() {
            return Err(GeminiLiveClientError::InvalidTargetLanguage);
        }
        Ok(Self {
            network: super::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                content_lock: Mutex::new(()),
                sink: Mutex::new(None),
                receive_task: Mutex::new(None),
                audio_send_lock: Mutex::new(()),
                pending_audio: Mutex::new(Vec::new()),
                committer: Mutex::new(GeminiTranscriptPairCommitter::default()),
                ready: AtomicBool::new(false),
                is_closing: AtomicBool::new(false),
                received_final_turn: AtomicBool::new(false),
                final_turn_notify: Notify::new(),
                transcript_revision: AtomicU64::new(0),
                transcript_notify: Notify::new(),
                turn_boundary_epoch: AtomicU64::new(0),
                pong_notify: Notify::new(),
                generation: AtomicU64::new(0),
            }),
            endpoint,
            authenticate_with_query,
            api_key: api_key.to_string(),
            target_language,
            events,
        })
    }

    pub async fn connect(&self) -> Result<(), GeminiLiveClientError> {
        self.connect_with_timeout(Duration::from_secs(5)).await
    }

    async fn connect_with_timeout(
        &self,
        readiness_timeout: Duration,
    ) -> Result<(), GeminiLiveClientError> {
        self.disconnect().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);

        let request = self
            .authenticated_endpoint()
            .into_client_request()
            .map_err(|_| GeminiLiveClientError::TransportFailure)?;
        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            super::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| GeminiLiveClientError::TransportFailure)?
        .map_err(|_| GeminiLiveClientError::TransportFailure)?;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .received_final_turn
            .store(false, Ordering::SeqCst);
        self.inner.pending_audio.lock().await.clear();
        self.inner.committer.lock().await.reset();

        let (setup_tx, mut setup_rx) = watch::channel(SetupState::Awaiting);
        let task = tokio::spawn(receive_loop(ReceiveContext {
            inner: Arc::clone(&self.inner),
            stream,
            events: self.events.clone(),
            setup: setup_tx,
            generation,
        }));
        *self.inner.receive_task.lock().await = Some(task);

        let setup = GeminiLiveRequestEncoder::setup(self.target_language)
            .map_err(|_| GeminiLiveClientError::InvalidTargetLanguage)?;
        let complete_setup = async {
            self.send_text(setup.to_string())
                .await
                .map_err(|_| GeminiLiveClientError::TransportFailure)?;
            loop {
                match setup_rx.borrow().clone() {
                    SetupState::Ready => return Ok(()),
                    SetupState::Rejected => {
                        return Err(GeminiLiveClientError::SessionSetupRejected)
                    }
                    SetupState::Awaiting => {}
                }
                if setup_rx.changed().await.is_err() {
                    return Err(GeminiLiveClientError::SessionSetupRejected);
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
                Err(GeminiLiveClientError::SessionSetupTimedOut)
            }
        }
    }

    fn authenticated_endpoint(&self) -> url::Url {
        let mut endpoint = self.endpoint.clone();
        if self.authenticate_with_query {
            endpoint
                .query_pairs_mut()
                .append_pair("key", self.api_key.as_str());
        }
        endpoint
    }

    /// Accepts arbitrary PCM chunks and sends only exact 100 ms frames.
    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), GeminiLiveClientError> {
        if pcm_data.is_empty() {
            return Ok(());
        }
        tokio::time::timeout(SEND_TIMEOUT, self.send_audio_operation(pcm_data))
            .await
            .map_err(|_| GeminiLiveClientError::TransportFailure)?
    }

    async fn send_audio_operation(&self, pcm_data: &[u8]) -> Result<(), GeminiLiveClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(GeminiLiveClientError::NotConnected);
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

    pub async fn ping(&self, timeout: Duration) -> Result<(), GeminiLiveClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(GeminiLiveClientError::NotConnected);
        }
        let operation = async {
            let pong = self.inner.pong_notify.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(GeminiLiveClientError::NotConnected);
                };
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| GeminiLiveClientError::TransportFailure)?;
            }
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| GeminiLiveClientError::HealthCheckTimedOut)?
    }

    /// Flushes the final PCM frame, sends `audioStreamEnd`, then waits for the
    /// provider's real turn boundary and a short quiet period for transcript
    /// fields that may legally arrive after that boundary.
    pub async fn finish(&self, timeout: Duration) {
        if !self.inner.ready.load(Ordering::SeqCst) {
            return;
        }
        if self.inner.is_closing.swap(true, Ordering::SeqCst) {
            return;
        }
        cancel_normal_turn_boundary(&self.inner);
        self.inner
            .received_final_turn
            .store(false, Ordering::SeqCst);
        let generation = self.inner.generation.load(Ordering::SeqCst);
        match tokio::time::timeout(timeout, self.finish_operation(generation)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) if self.is_current_generation(generation) => {
                self.inner.committer.lock().await.reset();
                self.inner.pending_audio.lock().await.clear();
                self.emit(
                    LiveTranslateServerEvent::Error {
                        code: "gemini_session_close_failed".into(),
                        message: GENERIC_TRANSPORT_ERROR.into(),
                    },
                    generation,
                );
            }
            Err(_) if self.is_current_generation(generation) => {
                let _content = self.inner.content_lock.lock().await;
                let mut committer = self.inner.committer.lock().await;
                let pending = committer.has_pending();
                committer.reset();
                self.inner.pending_audio.lock().await.clear();
                // The continuous translation model need not emit turnComplete.
                // Already committed stable blocks are durable; an unmatched
                // tail is still rejected rather than promoted at the deadline.
                self.emit(
                    if pending {
                        LiveTranslateServerEvent::Error {
                            code: "gemini_close_timeout".into(),
                            message: "Gemini Live Translation did not finish closing in time."
                                .into(),
                        }
                    } else {
                        LiveTranslateServerEvent::SessionFinished
                    },
                    generation,
                );
            }
            _ => {}
        }
        self.disconnect_if_current(generation).await;
    }

    async fn finish_operation(&self, generation: u64) -> Result<(), GeminiLiveClientError> {
        let _send_guard = self.inner.audio_send_lock.lock().await;
        if !self.is_current_generation(generation) {
            return Err(GeminiLiveClientError::NotConnected);
        }
        let partial = {
            let mut pending = self.inner.pending_audio.lock().await;
            take_padded_audio_message(&mut pending)?
        };
        if let Some(message) = partial {
            self.send_text(message).await?;
        }
        self.send_text(GeminiLiveRequestEncoder::audio_stream_end().to_string())
            .await?;
        while self.is_current_generation(generation) {
            let completed = self.inner.final_turn_notify.notified();
            tokio::pin!(completed);
            completed.as_mut().enable();
            if self.inner.received_final_turn.load(Ordering::SeqCst) {
                break;
            }
            completed.await;
        }
        publish_turn_after_transcript_quiet(
            &self.inner,
            &self.events,
            generation,
            TranscriptBoundary::Closing,
            true,
        )
        .await
        .ok_or(GeminiLiveClientError::NotConnected)?;
        Ok(())
    }

    pub async fn disconnect(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        cancel_normal_turn_boundary(&self.inner);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .received_final_turn
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
        self.inner.final_turn_notify.notify_waiters();
        self.inner.transcript_notify.notify_waiters();
    }

    async fn send_text(&self, text: String) -> Result<(), GeminiLiveClientError> {
        let mut sink = self.inner.sink.lock().await;
        let Some(sink) = sink.as_mut() else {
            return Err(GeminiLiveClientError::NotConnected);
        };
        let evidence =
            crate::development_audio::begin_json(&text, GeminiLiveEndpoint::SAMPLE_RATE_HZ);
        tokio::time::timeout(
            SEND_TIMEOUT,
            evidence.observe(sink.send(Message::Text(text.into()))),
        )
        .await
        .map_err(|_| GeminiLiveClientError::TransportFailure)?
        .map_err(|_| GeminiLiveClientError::TransportFailure)
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
) -> Result<Vec<String>, GeminiLiveClientError> {
    let frame_size = GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let complete_bytes = pending.len() / frame_size * frame_size;
    let result = pending[..complete_bytes]
        .chunks_exact(frame_size)
        .map(|frame| {
            GeminiLiveRequestEncoder::audio(frame)
                .map(|value| value.to_string())
                .map_err(|_| GeminiLiveClientError::TransportFailure)
        })
        .collect::<Result<Vec<_>, _>>();
    // Preserve the existing failure semantics: a complete batch is consumed
    // even if encoding one of its frames fails.
    pending.drain(..complete_bytes);
    result
}

fn take_padded_audio_message(
    pending: &mut Vec<u8>,
) -> Result<Option<String>, GeminiLiveClientError> {
    if pending.is_empty() {
        return Ok(None);
    }
    let frame_size = GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let mut frame = std::mem::take(pending);
    frame.resize(frame_size, 0);
    GeminiLiveRequestEncoder::audio(&frame)
        .map(|value| Some(value.to_string()))
        .map_err(|_| GeminiLiveClientError::TransportFailure)
}

struct ReceiveContext {
    inner: Arc<Inner>,
    stream: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    events: ProviderEventSender,
    setup: watch::Sender<SetupState>,
    generation: u64,
}

async fn receive_loop(mut context: ReceiveContext) {
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        let message = tokio::select! {
            message = context.stream.next() => { let Some(message) = message else { break }; message },
            _ = tick.tick() => {
                let _content = context.inner.content_lock.lock().await;
                if context.inner.generation.load(Ordering::SeqCst) != context.generation { return; }
                let mut committer = context.inner.committer.lock().await;
                if !committer.turn_complete_received {
                    emit_all_if_current(&context, committer.settle());
                }
                continue;
            }
        };
        if context.inner.generation.load(Ordering::SeqCst) != context.generation {
            return;
        }
        let events = match message {
            Ok(Message::Text(text)) => GeminiLiveServerEvent::decode(&text),
            Ok(Message::Binary(data)) => {
                GeminiLiveServerEvent::decode(&String::from_utf8_lossy(&data))
            }
            Ok(Message::Pong(_)) => {
                context.inner.pong_notify.notify_waiters();
                continue;
            }
            Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => {
                if context.inner.is_closing.load(Ordering::SeqCst)
                    && context.inner.received_final_turn.load(Ordering::SeqCst)
                {
                    return;
                }
                fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR).await;
                return;
            }
        };
        let events = match events {
            Ok(events) => events,
            Err(_) => {
                fail_receive_loop(&context, "gemini_protocol_error", GENERIC_PROTOCOL_ERROR).await;
                return;
            }
        };
        for event in events {
            if handle_server_event(&context, event).await {
                return;
            }
        }
    }
    if context.inner.generation.load(Ordering::SeqCst) == context.generation
        && !(context.inner.is_closing.load(Ordering::SeqCst)
            && context.inner.received_final_turn.load(Ordering::SeqCst))
    {
        fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR).await;
    }
}

async fn fail_receive_loop(context: &ReceiveContext, code: &str, message: &str) {
    cancel_normal_turn_boundary(&context.inner);
    context.inner.committer.lock().await.reset();
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
async fn handle_server_event(context: &ReceiveContext, event: GeminiLiveServerEvent) -> bool {
    let revision = context.events.content_revision();
    let _content = context.inner.content_lock.lock().await;
    if context.inner.generation.load(Ordering::SeqCst) != context.generation {
        return false;
    }
    if revision != context.events.content_revision()
        && matches!(
            &event,
            GeminiLiveServerEvent::SourceTranscript { .. }
                | GeminiLiveServerEvent::TranslationTranscript { .. }
        )
    {
        return false;
    }

    let setup_is_awaiting = *context.setup.borrow() == SetupState::Awaiting;
    if setup_is_awaiting
        && !matches!(
            &event,
            GeminiLiveServerEvent::SetupComplete
                | GeminiLiveServerEvent::ProviderError { .. }
                | GeminiLiveServerEvent::GoAway
        )
    {
        let _ = context.setup.send(SetupState::Rejected);
        return true;
    }

    match event {
        GeminiLiveServerEvent::SetupComplete => {
            if setup_is_awaiting {
                emit_if_current(context, LiveTranslateServerEvent::SessionCreated);
                emit_if_current(context, LiveTranslateServerEvent::SessionUpdated);
                let _ = context.setup.send(SetupState::Ready);
            } else {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Ignored {
                        kind: "duplicateSetupComplete".into(),
                    },
                );
            }
        }
        GeminiLiveServerEvent::SourceTranscript {
            text,
            language_code,
        } => {
            let events = {
                let mut committer = context.inner.committer.lock().await;
                let events = committer.append_source(&text, language_code);
                // Publish the revision before releasing the committer. A
                // quiet-boundary task can therefore never commit between the
                // append and its wake-up notification.
                note_transcript(context);
                events
            };
            emit_all_if_current(context, events);
        }
        GeminiLiveServerEvent::TranslationTranscript { text, .. } => {
            let events = {
                let mut committer = context.inner.committer.lock().await;
                let events = committer.append_translation(&text);
                note_transcript(context);
                events
            };
            emit_all_if_current(context, events);
        }
        GeminiLiveServerEvent::TurnComplete => {
            context.inner.committer.lock().await.turn_complete_received = true;
            if context.inner.is_closing.load(Ordering::SeqCst) {
                context
                    .inner
                    .received_final_turn
                    .store(true, Ordering::SeqCst);
                context.inner.final_turn_notify.notify_waiters();
            } else {
                schedule_normal_turn_commit(context);
            }
        }
        GeminiLiveServerEvent::Interrupted => {
            cancel_normal_turn_boundary(&context.inner);
            context.inner.committer.lock().await.reset();
        }
        GeminiLiveServerEvent::GoAway => {
            cancel_normal_turn_boundary(&context.inner);
            context.inner.committer.lock().await.reset();
            if setup_is_awaiting {
                let _ = context.setup.send(SetupState::Rejected);
            } else {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        // GoAway is an expected server-driven connection
                        // rotation. Reuse the session manager's bounded
                        // transport recovery instead of surfacing a terminal
                        // provider error to the user.
                        code: "transport_error".into(),
                        message: GENERIC_TRANSPORT_ERROR.into(),
                    },
                );
            }
            return true;
        }
        GeminiLiveServerEvent::ProviderError { code, .. } => {
            cancel_normal_turn_boundary(&context.inner);
            context.inner.committer.lock().await.reset();
            if setup_is_awaiting {
                let _ = context.setup.send(SetupState::Rejected);
            } else {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: format!("gemini_provider_error.{code}"),
                        message: GENERIC_PROVIDER_ERROR.into(),
                    },
                );
            }
            return true;
        }
        GeminiLiveServerEvent::OutputAudio | GeminiLiveServerEvent::GenerationComplete => {}
        GeminiLiveServerEvent::Ignored { kind } => {
            emit_if_current(context, LiveTranslateServerEvent::Ignored { kind });
        }
    }
    false
}

fn note_transcript(context: &ReceiveContext) {
    context
        .inner
        .transcript_revision
        .fetch_add(1, Ordering::SeqCst);
    context.inner.transcript_notify.notify_waiters();
}

#[derive(Clone, Copy)]
enum TranscriptBoundary {
    Normal(u64),
    Closing,
}

fn transcript_boundary_is_current(
    inner: &Inner,
    generation: u64,
    boundary: TranscriptBoundary,
) -> bool {
    if inner.generation.load(Ordering::SeqCst) != generation {
        return false;
    }
    match boundary {
        TranscriptBoundary::Normal(epoch) => {
            !inner.is_closing.load(Ordering::SeqCst)
                && inner.turn_boundary_epoch.load(Ordering::SeqCst) == epoch
        }
        TranscriptBoundary::Closing => inner.is_closing.load(Ordering::SeqCst),
    }
}

async fn publish_turn_after_transcript_quiet(
    inner: &Arc<Inner>,
    events: &ProviderEventSender,
    generation: u64,
    boundary: TranscriptBoundary,
    publish_session_finished: bool,
) -> Option<()> {
    loop {
        let revision = wait_for_transcript_quiet(inner, generation, boundary).await?;
        let _content = inner.content_lock.lock().await;
        let mut committer = inner.committer.lock().await;
        if !transcript_boundary_is_current(inner, generation, boundary) {
            return None;
        }
        if inner.transcript_revision.load(Ordering::SeqCst) != revision {
            drop(committer);
            continue;
        }
        // Commit and publish while holding the same guard used by transcript,
        // interruption, and terminal handlers. A reset can therefore never
        // slip between clearing the buffers and publishing their final pair.
        for event in committer.finish_turn() {
            let _ = events.send(event);
        }
        if publish_session_finished {
            let _ = events.send(LiveTranslateServerEvent::SessionFinished);
        }
        return Some(());
    }
}

async fn wait_for_transcript_quiet(
    inner: &Arc<Inner>,
    generation: u64,
    boundary: TranscriptBoundary,
) -> Option<u64> {
    loop {
        if !transcript_boundary_is_current(inner, generation, boundary) {
            return None;
        }
        let revision = inner.transcript_revision.load(Ordering::SeqCst);
        let changed = inner.transcript_notify.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if !transcript_boundary_is_current(inner, generation, boundary)
            || inner.transcript_revision.load(Ordering::SeqCst) != revision
        {
            continue;
        }
        if tokio::time::timeout(TAIL_QUIET_PERIOD, changed)
            .await
            .is_ok()
        {
            continue;
        }
        return Some(revision);
    }
}

async fn emit_normal_turn_after_transcript_quiet(
    inner: &Arc<Inner>,
    events: &ProviderEventSender,
    generation: u64,
    epoch: u64,
) {
    let _ = publish_turn_after_transcript_quiet(
        inner,
        events,
        generation,
        TranscriptBoundary::Normal(epoch),
        false,
    )
    .await;
}

fn schedule_normal_turn_commit(context: &ReceiveContext) {
    schedule_normal_turn_commit_for(&context.inner, &context.events, context.generation);
}

fn schedule_normal_turn_commit_for(
    inner: &Arc<Inner>,
    events: &ProviderEventSender,
    generation: u64,
) {
    let epoch = inner
        .turn_boundary_epoch
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1);
    inner.transcript_notify.notify_waiters();
    let inner = Arc::clone(inner);
    let events = events.clone();
    drop(tokio::spawn(async move {
        emit_normal_turn_after_transcript_quiet(&inner, &events, generation, epoch).await;
    }));
}

fn cancel_normal_turn_boundary(inner: &Inner) {
    inner.turn_boundary_epoch.fetch_add(1, Ordering::SeqCst);
    inner.transcript_notify.notify_waiters();
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
    async fn clear_keeps_the_observed_quiet_boundary_without_committing_its_old_tail() {
        let (sender, mut receiver) = provider_event_channel();
        let client = GeminiLiveClient::with_endpoint(
            "test-key-not-real",
            TargetLanguage::Japanese,
            sender.clone(),
            url::Url::parse("ws://127.0.0.1:1/live").unwrap(),
            false,
        )
        .unwrap();
        {
            let mut committer = client.inner.committer.lock().await;
            committer.append_source("old source", None);
            committer.append_translation("old translation");
            committer.turn_complete_received = true;
        }
        let old_epoch = client.inner.turn_boundary_epoch.load(Ordering::SeqCst);
        sender
            .send(LiveTranslateServerEvent::SessionUpdated)
            .unwrap();
        assert_eq!(client.clear_content().await, 1);
        assert_eq!(client.content_revision(), 1);
        assert!(!transcript_boundary_is_current(
            &client.inner,
            0,
            TranscriptBoundary::Normal(old_epoch)
        ));
        {
            let mut committer = client.inner.committer.lock().await;
            assert!(!committer.has_pending());
            assert!(committer.append_source("late old source", None).is_empty());
            assert!(committer
                .append_translation("late old translation")
                .is_empty());
        }
        let epoch = client.inner.turn_boundary_epoch.load(Ordering::SeqCst);
        tokio::time::timeout(
            Duration::from_secs(3),
            publish_turn_after_transcript_quiet(
                &client.inner,
                &sender,
                0,
                TranscriptBoundary::Normal(epoch),
                false,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let mut committer = client.inner.committer.lock().await;
        assert!(!committer.discard_current_turn);
        committer.append_source("new source", None);
        committer.append_translation("new translation");
        assert!(
            matches!(committer.finish_turn().as_slice(), [LiveTranslateServerEvent::SubtitleFinalPair {source,translation,..}]
            if source == "new source" && translation == "new translation")
        );
        drop(committer);
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionUpdated)
        );
        assert!(receiver.try_recv().is_err());
    }

    use super::*;
    use crate::clients::provider_events::{provider_event_channel, ProviderEventReceiver};
    use base64::Engine;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio::net::TcpListener;

    #[test]
    fn shared_gemini_transcript_contracts() {
        let fixtures: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../shared/translation-contracts.json"
        )))
        .unwrap();
        for case in fixtures["liveTranscriptSequences"].as_array().unwrap() {
            let mut committer = GeminiTranscriptPairCommitter::default();
            let mut pairs = Vec::new();
            let mut failed = false;
            for (index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let Ok(decoded) = GeminiLiveServerEvent::decode(frame.as_str().unwrap()) else {
                    failed = true;
                    break;
                };
                for event in decoded {
                    let events = match event {
                        GeminiLiveServerEvent::SourceTranscript {
                            text,
                            language_code,
                        } => committer.append_source(&text, language_code),
                        GeminiLiveServerEvent::TranslationTranscript { text, .. } => {
                            committer.append_translation(&text)
                        }
                        GeminiLiveServerEvent::TurnComplete => committer.finish_turn(),
                        _ => Vec::new(),
                    };
                    for event in events {
                        if let LiveTranslateServerEvent::SubtitleFinalPair {
                            source,
                            language,
                            translation,
                        } = event
                        {
                            pairs.push(json!({"source":source,"language":language,"translation":translation}));
                        }
                    }
                }
                if case["settleAfterFrames"].as_array().is_some_and(|indices| {
                    indices
                        .iter()
                        .any(|value| value.as_u64() == Some(index as u64))
                }) {
                    let events = committer.stream.settle(u64::MAX);
                    for event in committer.adapt(events) {
                        if let LiveTranslateServerEvent::SubtitleFinalPair {
                            source,
                            language,
                            translation,
                        } = event
                        {
                            pairs.push(json!({"source":source,"language":language,"translation":translation}));
                        }
                    }
                }
            }
            if case["expected"].is_null() {
                assert!(failed, "{}", case["id"]);
            } else {
                assert!(!failed);
                assert_eq!(json!(pairs), case["expected"], "{}", case["id"]);
            }
        }
    }

    #[tokio::test]
    async fn continuous_sentence_pairs_survive_language_only_updates_and_close_without_turn_complete(
    ) {
        let (client, mut events) = test_client(|mut socket| Box::pin(async move {
            assert_setup(socket.next().await.unwrap().unwrap());
            socket.send(Message::Text(r#"{"setupComplete":{}}"#.into())).await.unwrap();
            socket.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"Hello.","languageCode":"en"},"outputTranscription":{"text":"こんにちは。"}}}"#.into())).await.unwrap();
            socket.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"languageCode":"en"},"outputTranscription":{"languageCode":"ja"}}}"#.into())).await.unwrap();
            while socket.next().await.is_some() {}
        })).await;
        client.connect().await.unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
                .await
                .unwrap()
                .unwrap();
            if matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. }) {
                break;
            }
        }
        client.finish(Duration::from_millis(50)).await;
        let mut finished = false;
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, LiveTranslateServerEvent::Error { .. }));
            finished |= matches!(event, LiveTranslateServerEvent::SessionFinished);
        }
        assert!(finished);
    }

    async fn test_client(
        server: impl FnOnce(
                WebSocketStream<TcpStream>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + 'static,
    ) -> (GeminiLiveClient, ProviderEventReceiver) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server(socket).await;
        });
        let (events, receiver) = provider_event_channel();
        let endpoint = url::Url::parse(&format!("ws://{address}/live")).unwrap();
        let client = GeminiLiveClient::with_endpoint(
            "gemini-test-key-not-real",
            TargetLanguage::Japanese,
            events,
            endpoint,
            false,
        )
        .unwrap();
        (client, receiver)
    }

    fn assert_setup(message: Message) {
        let setup: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        assert_eq!(
            setup["setup"]["model"],
            "models/gemini-3.5-live-translate-preview"
        );
        assert_eq!(
            setup["setup"]["generationConfig"]["translationConfig"]["targetLanguageCode"],
            "ja"
        );
    }

    #[tokio::test]
    async fn connect_waits_for_setup_complete() {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                assert_setup(socket.next().await.unwrap().unwrap());
                // This is deliberately longer than the old 150 ms heuristic:
                // a legal delayed transcript must still be part of the tail.
                tokio::time::sleep(Duration::from_millis(300)).await;
                socket
                    .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                    .await
                    .unwrap();
                while socket.next().await.is_some() {}
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn setup_rejection_is_content_free() {
        let secret = "gemini-test-secret-never-log";
        let (events, _receiver) = provider_event_channel();
        let result = GeminiLiveClient::with_endpoint(
            secret,
            TargetLanguage::Original,
            events,
            url::Url::parse("ws://127.0.0.1:9/live").unwrap(),
            false,
        );
        let error = match result {
            Ok(_) => panic!("an untranslated target must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error, GeminiLiveClientError::InvalidTargetLanguage);
        assert!(!format!("{error:?}").contains(secret));

        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"error":{"code":403,"status":"PERMISSION_DENIED","message":"private transcript and key"}}"#.into(),
                    ))
                    .await
                    .unwrap();
            })
        })
        .await;
        assert_eq!(
            client
                .connect_with_timeout(Duration::from_millis(500))
                .await
                .unwrap_err(),
            GeminiLiveClientError::SessionSetupRejected
        );
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
            GeminiLiveClientError::SessionSetupTimedOut
        );
    }

    #[tokio::test]
    async fn established_go_away_requests_redacted_transport_recovery() {
        let private_time_left = "private-provider-time-left";
        let private_message = "private-provider-message";
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                assert_setup(socket.next().await.unwrap().unwrap());
                socket
                    .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        format!(
                            r#"{{"goAway":{{"timeLeft":"{private_time_left}","message":"{private_message}"}}}}"#
                        )
                        .into(),
                    ))
                    .await
                    .unwrap();
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();

        let mut received = Vec::new();
        loop {
            let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
                .await
                .expect("GoAway must produce a recovery event")
                .expect("the provider event channel must remain open");
            let is_error = matches!(event, LiveTranslateServerEvent::Error { .. });
            received.push(event);
            if is_error {
                break;
            }
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::Error { code, message }
                if code == "transport_error" && message == GENERIC_TRANSPORT_ERROR
        )));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        let debug = format!("{received:?}");
        assert!(!debug.contains(private_time_left));
        assert!(!debug.contains(private_message));
        client.disconnect().await;
    }

    #[test]
    fn frame_buffer_keeps_only_a_partial_tail() {
        let frame_size = GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT;
        let mut pending = vec![0x22; frame_size * 2 + 17];
        let messages = take_complete_audio_messages(&mut pending).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(pending.len(), 17);
        for message in messages {
            let value: Value = serde_json::from_str(&message).unwrap();
            let frame = base64::engine::general_purpose::STANDARD
                .decode(value["realtimeInput"]["audio"]["data"].as_str().unwrap())
                .unwrap();
            assert_eq!(frame.len(), frame_size);
            assert!(frame.iter().all(|byte| *byte == 0x22));
        }
    }

    #[test]
    fn final_partial_frame_is_zero_padded() {
        let mut pending = vec![1, 2, 3];
        let message = take_padded_audio_message(&mut pending).unwrap().unwrap();
        let value: Value = serde_json::from_str(&message).unwrap();
        let frame = base64::engine::general_purpose::STANDARD
            .decode(value["realtimeInput"]["audio"]["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(frame.len(), GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT);
        assert_eq!(&frame[..3], &[1, 2, 3]);
        assert!(frame[3..].iter().all(|byte| *byte == 0));
        assert!(pending.is_empty());
    }

    #[test]
    fn transcript_safety_limit_discards_the_rest_of_the_turn() {
        let mut committer = GeminiTranscriptPairCommitter::default();
        let events =
            committer.append_source(&"s".repeat(MAXIMUM_TRANSCRIPT_BYTES + 1), Some("en".into()));
        assert!(matches!(
            events.as_slice(),
            [LiveTranslateServerEvent::Error { code, message }]
                if code == "gemini_transcript_safety_limit"
                    && message
                        == "Gemini Live Translation transcript buffering exceeded its safety limit."
        ));

        assert!(committer.append_source("stale source", None).is_empty());
        assert!(committer.append_translation("stale translation").is_empty());
        assert!(committer.finish_turn().is_empty());

        let _ = committer.append_source("Next sentence.", Some("en".into()));
        let _ = committer.append_translation("次の文。");
        let events = committer.finish_turn();
        assert!(matches!(
            events.last(),
            Some(LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                language,
                translation,
            }) if source == "Next sentence."
                && language.as_deref() == Some("en")
                && translation == "次の文。"
        ));
    }

    #[tokio::test]
    async fn full_mock_lifecycle_waits_for_turn_complete_and_trailing_transcripts() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                assert_setup(socket.next().await.unwrap().unwrap());
                socket
                    .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                    .await
                    .unwrap();
                let audio = socket.next().await.unwrap().unwrap();
                let audio: Value = serde_json::from_str(audio.to_text().unwrap()).unwrap();
                let frame = base64::engine::general_purpose::STANDARD
                    .decode(
                        audio["realtimeInput"]["audio"]["data"]
                            .as_str()
                            .unwrap(),
                    )
                    .unwrap();
                assert_eq!(frame.len(), GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT);
                assert_eq!(&frame[..3], &[1, 2, 3]);

                let end = socket.next().await.unwrap().unwrap();
                let end: Value = serde_json::from_str(end.to_text().unwrap()).unwrap();
                assert_eq!(end, json!({"realtimeInput": {"audioStreamEnd": true}}));
                socket
                    .send(Message::Text(
                        r#"{"serverContent":{"inputTranscription":{"text":"Hello ","languageCode":"en-US"},"outputTranscription":{"text":"こんにちは"},"turnComplete":true}}"#.into(),
                    ))
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
                socket
                    .send(Message::Text(
                        r#"{"serverContent":{"inputTranscription":{"text":"world."},"outputTranscription":{"text":"世界。"}}}"#.into(),
                    ))
                    .await
                    .unwrap();
                while socket.next().await.is_some() {}
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.send_audio(&[1, 2, 3]).await.unwrap();
        client.finish(Duration::from_secs(1)).await;

        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleFinalPair { source, language, translation }
                if source == "Hello world."
                    && language.as_deref() == Some("en-us")
                    && translation == "こんにちは世界。"
        )));
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
    }

    #[tokio::test]
    async fn normal_turn_boundary_waits_for_late_tail_without_mixing_the_next_turn() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                assert_setup(socket.next().await.unwrap().unwrap());
                socket
                    .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"serverContent":{"inputTranscription":{"text":"First "},"outputTranscription":{"text":"第一"},"turnComplete":true}}"#.into(),
                    ))
                    .await
                    .unwrap();
                // The tail is deliberately later than the old 150 ms grace.
                tokio::time::sleep(Duration::from_millis(250)).await;
                socket
                    .send(Message::Text(
                        r#"{"serverContent":{"inputTranscription":{"text":"sentence."},"outputTranscription":{"text":"句。"}}}"#.into(),
                    ))
                    .await
                    .unwrap();
                tokio::time::sleep(TAIL_QUIET_PERIOD + Duration::from_millis(100)).await;
                socket
                    .send(Message::Text(
                        r#"{"serverContent":{"inputTranscription":{"text":"Next sentence."},"outputTranscription":{"text":"次の文。"},"turnComplete":true}}"#.into(),
                    ))
                    .await
                    .unwrap();
                while socket.next().await.is_some() {}
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();

        let mut final_pairs = Vec::new();
        while final_pairs.len() < 2 {
            let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
                .await
                .expect("both quiet-boundary commits must finish")
                .expect("the provider event channel must remain open");
            if let LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                language,
                translation,
            } = event
            {
                final_pairs.push((source, language, translation));
            }
        }
        assert_eq!(
            final_pairs,
            vec![
                ("First sentence.".into(), None, "第一句。".into()),
                ("Next sentence.".into(), None, "次の文。".into()),
            ]
        );
        client.disconnect().await;
    }

    #[tokio::test]
    async fn interrupted_close_discards_partial_pair_but_finishes_cleanly() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                assert_setup(socket.next().await.unwrap().unwrap());
                socket
                    .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"serverContent":{"inputTranscription":{"text":"partial private source"},"outputTranscription":{"text":"partial private translation"}}}"#.into(),
                    ))
                    .await
                    .unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if message
                        .to_text()
                        .ok()
                        .is_some_and(|text| text.contains("audioStreamEnd"))
                    {
                        socket
                            .send(Message::Text(
                                r#"{"serverContent":{"interrupted":true,"turnComplete":true}}"#
                                    .into(),
                            ))
                            .await
                            .unwrap();
                        while socket.next().await.is_some() {}
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
        client.finish(Duration::from_secs(2)).await;

        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. })));
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
                    .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"serverContent":{"inputTranscription":{"text":"private source"},"outputTranscription":{"text":"private translation"}}}"#.into(),
                    ))
                    .await
                    .unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if message
                        .to_text()
                        .ok()
                        .is_some_and(|text| text.contains("audioStreamEnd"))
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
            LiveTranslateServerEvent::Error { code, .. } if code == "gemini_close_timeout"
        )));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. })));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    }

    #[tokio::test]
    async fn an_immediate_pong_cannot_be_lost() {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
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
    async fn stale_socket_events_are_suppressed_after_disconnect() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;
                let _ = socket
                    .send(Message::Text(
                        r#"{"serverContent":{"inputTranscription":{"text":"stale private text"}}}"#
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
        while events.try_recv().is_ok() {}
        client.disconnect().await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(events.try_recv().is_err());
    }
}
