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
const ROTATION_SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const ROTATION_DRAIN_TIMEOUT: Duration = Duration::from_millis(1_500);
// Native callback sizes vary; stage by PCM duration, not number of callbacks.
const ROTATION_PENDING_PCM_BYTES: usize = 2 * 16_000 * 2;
// Gemini can legally deliver transcript chunks after `turnComplete` and does
// not provide a separate transcript-terminal event. Treat every normal turn
// boundary (and the final close boundary) as provisional until transcripts
// have stayed quiet long enough to absorb ordinary network jitter.
const TAIL_QUIET_PERIOD: Duration =
    Duration::from_millis(mimi_core::openai_transcript_committer::GEMINI_TURN_TAIL_QUIET_MS);
#[cfg(test)]
const MAXIMUM_TRANSCRIPT_BYTES: usize = 128 * 1_024;

/// Probe-only categorical evidence. Never retain vendor messages or close reasons.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum ProbeTransportStop {
    GoAway,
    WebSocketClose { code: Option<u16> },
    ReceiveError,
    EndOfStream,
}

#[cfg(test)]
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeConnectionEvent {
    pub elapsed_ms: u128,
    pub kind: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GeminiLiveClientError {
    #[error("credential_authentication_failed")]
    AuthenticationFailed,
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
type Stream = futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

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
    rotating: AtomicBool,
    rotation_notify: Notify,
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
    #[cfg(test)]
    probe_transport_stop: std::sync::Mutex<Option<ProbeTransportStop>>,
    #[cfg(test)]
    probe_connection_events: std::sync::Mutex<Vec<ProbeConnectionEvent>>,
    #[cfg(test)]
    probe_connection_epoch: std::sync::Mutex<std::time::Instant>,
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
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.clone()
    }

    #[cfg(test)]
    pub(crate) fn probe_transport_stop(&self) -> Option<ProbeTransportStop> {
        self.inner.probe_transport_stop.lock().unwrap().clone()
    }
    #[cfg(test)]
    pub(crate) fn probe_connection_events(&self) -> Vec<ProbeConnectionEvent> {
        self.inner.probe_connection_events.lock().unwrap().clone()
    }
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
                rotating: AtomicBool::new(false),
                rotation_notify: Notify::new(),
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
                #[cfg(test)]
                probe_transport_stop: std::sync::Mutex::new(None),
                #[cfg(test)]
                probe_connection_events: std::sync::Mutex::new(Vec::new()),
                #[cfg(test)]
                probe_connection_epoch: std::sync::Mutex::new(std::time::Instant::now()),
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
        #[cfg(test)]
        {
            *self.inner.probe_transport_stop.lock().unwrap() = None;
            self.inner.probe_connection_events.lock().unwrap().clear();
            *self.inner.probe_connection_epoch.lock().unwrap() = std::time::Instant::now();
        }
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
        .map_err(|error| {
            if super::connection_diagnostics::authentication_rejected(&error) {
                GeminiLiveClientError::AuthenticationFailed
            } else {
                GeminiLiveClientError::TransportFailure
            }
        })?;
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
            client: self.clone(),
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
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(GeminiLiveClientError::NotConnected);
        }
        if self.inner.rotating.load(Ordering::SeqCst) {
            let mut pending = self.inner.pending_audio.lock().await;
            if pending.len().saturating_add(pcm_data.len()) > ROTATION_PENDING_PCM_BYTES {
                return Err(GeminiLiveClientError::TransportFailure);
            }
            pending.extend_from_slice(pcm_data);
            return Ok(());
        }
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
    /// provider's real turn boundary and its late transcript tail, or the shared
    /// paired-text quiet checkpoint when continuous translation omits the boundary.
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
        let _send_guard = loop {
            let completed = self.inner.rotation_notify.notified();
            tokio::pin!(completed);
            completed.as_mut().enable();
            let guard = self.inner.audio_send_lock.lock().await;
            if !self.inner.rotating.load(Ordering::SeqCst) {
                break guard;
            }
            drop(guard);
            completed.await;
            if !self.is_current_generation(generation) {
                return Err(GeminiLiveClientError::NotConnected);
            }
        };
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
        // Audio is closed to new sends. Let a prepared replacement acquire the
        // barrier, observe Stop, and return to receiving the old socket's tail.
        drop(_send_guard);
        let closed_at = tokio::time::Instant::now();
        while self.is_current_generation(generation) {
            let completed = self.inner.final_turn_notify.notified();
            tokio::pin!(completed);
            completed.as_mut().enable();
            if self.inner.received_final_turn.load(Ordering::SeqCst) {
                break;
            }
            tokio::select! {
                _ = completed => {},
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    let _content = self.inner.content_lock.lock().await;
                    if !self.is_current_generation(generation) {
                        return Err(GeminiLiveClientError::NotConnected);
                    }
                    let mut committer = self.inner.committer.lock().await;
                    // Use the same checkpoint as live captions, never force an
                    // unconfirmed or one-sided tail into history at the deadline.
                    if !committer.turn_complete_received {
                        for event in committer.settle() {
                            self.emit(event, generation);
                        }
                        if closed_at.elapsed() >= Duration::from_millis(
                            mimi_core::openai_transcript_committer::GEMINI_TRANSCRIPT_QUIET_MS,
                        ) && !committer.has_pending() {
                            self.emit(LiveTranslateServerEvent::SessionFinished, generation);
                            return Ok(());
                        }
                    }
                }
            }
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
        self.inner.rotating.store(false, Ordering::SeqCst);
        self.inner.rotation_notify.notify_waiters();
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
    client: GeminiLiveClient,
    inner: Arc<Inner>,
    stream: Stream,
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
                #[cfg(test)]
                {
                    let reason = match &message {
                        Ok(Message::Close(frame)) => ProbeTransportStop::WebSocketClose {
                            code: frame.as_ref().map(|frame| u16::from(frame.code)),
                        },
                        _ => ProbeTransportStop::ReceiveError,
                    };
                    *context.inner.probe_transport_stop.lock().unwrap() = Some(reason);
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
            if event == GeminiLiveServerEvent::GoAway
                && *context.setup.borrow() == SetupState::Ready
                && !context.inner.is_closing.load(Ordering::SeqCst)
            {
                record_rotation(&context.inner, "geminiGoAwayReceived");
                match rotate_connection(&mut context).await {
                    Ok(()) => continue,
                    Err(_) => {
                        // A failed planned rotation still has the existing
                        // bounded SessionManager recovery as its fallback.
                        handle_server_event(&context, GeminiLiveServerEvent::GoAway).await;
                        return;
                    }
                }
            }
            if handle_server_event(&context, event).await {
                return;
            }
        }
    }
    if context.inner.generation.load(Ordering::SeqCst) == context.generation
        && !(context.inner.is_closing.load(Ordering::SeqCst)
            && context.inner.received_final_turn.load(Ordering::SeqCst))
    {
        #[cfg(test)]
        {
            *context.inner.probe_transport_stop.lock().unwrap() =
                Some(ProbeTransportStop::EndOfStream);
        }
        fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR).await;
    }
}

/// Establish the replacement while capture/audio and old transcripts continue.
/// No resumption handle or audio replay: neither has reliable alignment metadata
/// for this continuous translation model. Only one socket receives input audio.
async fn prepare_replacement(client: &GeminiLiveClient) -> Result<(Sink, Stream), ()> {
    let request = client
        .authenticated_endpoint()
        .into_client_request()
        .map_err(|_| ())?;
    let (socket, _) = super::provider_network::websocket(request, &client.network)
        .await
        .map_err(|_| ())?;
    let (mut sink, mut stream) = socket.split();
    let setup = GeminiLiveRequestEncoder::setup(client.target_language).map_err(|_| ())?;
    sink.send(Message::Text(setup.to_string().into()))
        .await
        .map_err(|_| ())?;
    while let Some(message) = stream.next().await {
        let events = match message.map_err(|_| ())? {
            Message::Text(text) => GeminiLiveServerEvent::decode(&text).map_err(|_| ())?,
            Message::Binary(data) => {
                GeminiLiveServerEvent::decode(&String::from_utf8_lossy(&data)).map_err(|_| ())?
            }
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            Message::Close(_) => return Err(()),
        };
        if events == [GeminiLiveServerEvent::SetupComplete] {
            return Ok((sink, stream));
        }
        // Do not discard unexpected transcript or terminal events during setup.
        return Err(());
    }
    Err(())
}

async fn rotate_connection(context: &mut ReceiveContext) -> Result<(), ()> {
    let client = context.client.clone();
    let replacement = tokio::time::timeout(ROTATION_SETUP_TIMEOUT, prepare_replacement(&client));
    tokio::pin!(replacement);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let (mut next_sink, next_stream) = loop {
        tokio::select! {
            result = &mut replacement => break result.map_err(|_| ())??,
            message = context.stream.next() => {
                process_rotation_message(context, message).await?;
            },
            _ = tick.tick() => {
                let _content = context.inner.content_lock.lock().await;
                if context.inner.generation.load(Ordering::SeqCst) != context.generation { return Err(()); }
                let mut committer = context.inner.committer.lock().await;
                if !committer.turn_complete_received {
                    emit_all_if_current(context, committer.settle());
                }
            }
        }
    };

    let inner = Arc::clone(&context.inner);
    let _send = inner.audio_send_lock.lock().await;
    if inner.generation.load(Ordering::SeqCst) != context.generation {
        return Err(());
    }
    if inner.is_closing.load(Ordering::SeqCst) {
        // Stop owns the old socket; leave its real closing boundary intact.
        let _ = tokio::time::timeout(Duration::from_millis(250), next_sink.close()).await;
        return Ok(());
    }
    let partial =
        take_padded_audio_message(&mut *inner.pending_audio.lock().await).map_err(|_| ())?;
    if let Some(message) = partial {
        client.send_text(message).await.map_err(|_| ())?;
    }
    client
        .send_text(GeminiLiveRequestEncoder::audio_stream_end().to_string())
        .await
        .map_err(|_| ())?;
    inner.rotating.store(true, Ordering::SeqCst);
    let handoff = RotationHandoff {
        inner: Arc::clone(&inner),
        generation: context.generation,
    };
    drop(_send);

    // Continue reading late old-session output without mixing it into the new
    // session. A finite quiet drain precedes finalization and the socket switch.
    let deadline = tokio::time::Instant::now() + ROTATION_DRAIN_TIMEOUT;
    let mut quiet_since = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(quiet_since + TAIL_QUIET_PERIOD) => break,
            _ = tokio::time::sleep_until(deadline) => return Err(()),
            message = context.stream.next() => {
                let revision = inner.transcript_revision.load(Ordering::SeqCst);
                process_rotation_message(context, message).await?;
                if inner.transcript_revision.load(Ordering::SeqCst) != revision {
                    quiet_since = tokio::time::Instant::now();
                }
            }
        }
    }

    let _send = inner.audio_send_lock.lock().await;
    let _content = inner.content_lock.lock().await;
    if inner.generation.load(Ordering::SeqCst) != context.generation {
        return Err(());
    }
    // Once audioStreamEnd was sent, Stop must finish the replacement and its
    // staged audio; it must never send those buffers into the ended old stream.
    cancel_normal_turn_boundary(&inner);
    emit_all_if_current(context, inner.committer.lock().await.finish_turn());
    // Stop may have observed the old socket's turnComplete during the drain.
    // The replacement must acknowledge its own audioStreamEnd before closing.
    inner.received_final_turn.store(false, Ordering::SeqCst);
    let old_sink = inner.sink.lock().await.replace(next_sink);
    context.stream = next_stream;
    drop(_content);
    let messages = {
        let mut pending = inner.pending_audio.lock().await;
        take_complete_audio_messages(&mut pending).map_err(|_| ())?
    };
    for message in messages {
        client.send_text(message).await.map_err(|_| ())?;
    }
    record_rotation(&inner, "geminiConnectionRotated");
    drop(handoff);
    // Close the retired transport without extending the audio-send barrier.
    drop(_send);
    if let Some(mut sink) = old_sink {
        let _ = tokio::time::timeout(Duration::from_millis(250), sink.close()).await;
    }
    Ok(())
}

struct RotationHandoff {
    inner: Arc<Inner>,
    generation: u64,
}

impl Drop for RotationHandoff {
    fn drop(&mut self) {
        if self.inner.generation.load(Ordering::SeqCst) == self.generation {
            self.inner.rotating.store(false, Ordering::SeqCst);
            self.inner.rotation_notify.notify_waiters();
        }
    }
}

fn record_rotation(_inner: &Inner, kind: &'static str) {
    crate::pipeline_log!("gemini connection rotation status={}", kind);
    #[cfg(test)]
    {
        let elapsed_ms = _inner
            .probe_connection_epoch
            .lock()
            .unwrap()
            .elapsed()
            .as_millis();
        let mut events = _inner.probe_connection_events.lock().unwrap();
        if events.len() < 16 {
            events.push(ProbeConnectionEvent { elapsed_ms, kind });
        }
    }
}

async fn process_rotation_message(
    context: &ReceiveContext,
    message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
) -> Result<(), ()> {
    let message = message.ok_or(())?.map_err(|_| ())?;
    let events = match message {
        Message::Text(text) => GeminiLiveServerEvent::decode(&text).map_err(|_| ())?,
        Message::Binary(data) => {
            GeminiLiveServerEvent::decode(&String::from_utf8_lossy(&data)).map_err(|_| ())?
        }
        Message::Pong(_) => {
            context.inner.pong_notify.notify_waiters();
            return Ok(());
        }
        Message::Ping(_) | Message::Frame(_) => return Ok(()),
        Message::Close(_) => return Err(()),
    };
    for event in events {
        if event != GeminiLiveServerEvent::GoAway && handle_server_event(context, event).await {
            return Err(());
        }
    }
    Ok(())
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
            #[cfg(test)]
            {
                *context.inner.probe_transport_stop.lock().unwrap() =
                    Some(ProbeTransportStop::GoAway);
            }
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
    async fn handshake_authentication_rejections_preserve_transport_failures() {
        for (status, expected_auth_label) in
            crate::clients::connection_diagnostics::handshake_failure_cases()
        {
            let (endpoint, server) =
                crate::clients::connection_diagnostics::rejected_websocket_endpoint(status).await;
            let (events, _receiver) = provider_event_channel();
            let mut client = GeminiLiveClient::with_endpoint(
                "test-key-not-real",
                TargetLanguage::Japanese,
                events,
                endpoint,
                false,
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
                assert_eq!(error, GeminiLiveClientError::AuthenticationFailed);
                assert_eq!(error.to_string(), expected_label);
            } else {
                assert_eq!(error, GeminiLiveClientError::TransportFailure);
            }
            assert!(!error.to_string().contains("private-handshake-body"));
            assert!(!client.inner.ready.load(Ordering::SeqCst));
        }
    }

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
        fn observe(
            event: LiveTranslateServerEvent,
            pairs: &mut Vec<Value>,
            drafts: &mut Vec<Value>,
        ) {
            match event {
                LiveTranslateServerEvent::SubtitleFinalPair {
                    source,
                    language,
                    translation,
                } => pairs
                    .push(json!({"source":source,"language":language,"translation":translation})),
                LiveTranslateServerEvent::SourceDraft { text, .. } => {
                    drafts.push(json!({"kind":"source","text":text}))
                }
                LiveTranslateServerEvent::TranslationDraft(text) => {
                    drafts.push(json!({"kind":"translation","text":text}))
                }
                _ => {}
            }
        }
        let fixtures: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../translation-contracts.json"
        )))
        .unwrap();
        for case in fixtures["liveTranscriptSequences"].as_array().unwrap() {
            let mut committer = GeminiTranscriptPairCommitter::default();
            let mut pairs = Vec::new();
            let mut drafts = Vec::new();
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
                        observe(event, &mut pairs, &mut drafts);
                    }
                }
                if case["settleAfterFrames"].as_array().is_some_and(|indices| {
                    indices
                        .iter()
                        .any(|value| value.as_u64() == Some(index as u64))
                }) {
                    let events = committer.stream.settle(u64::MAX);
                    for event in committer.adapt(events) {
                        observe(event, &mut pairs, &mut drafts);
                    }
                }
            }
            if case["expected"].is_null() {
                assert!(failed, "{}", case["id"]);
            } else {
                assert!(!failed);
                assert_eq!(json!(pairs), case["expected"], "{}", case["id"]);
                if case["expectedDrafts"].is_array() {
                    assert_eq!(json!(drafts), case["expectedDrafts"], "{}", case["id"]);
                }
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

    #[tokio::test]
    async fn stop_checkpoints_a_late_paired_tail_without_turn_complete() {
        let (client, mut events) = test_client(|mut socket| Box::pin(async move {
            assert_setup(socket.next().await.unwrap().unwrap());
            socket.send(Message::Text(r#"{"setupComplete":{}}"#.into())).await.unwrap();
            socket.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"Synthetic source"},"outputTranscription":{"text":"合成"}}}"#.into())).await.unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if message.to_text().ok().is_some_and(|text| text.contains("audioStreamEnd")) {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    socket.send(Message::Text(r#"{"serverContent":{"outputTranscription":{"text":"译文"}}}"#.into())).await.unwrap();
                }
            }
        })).await;
        client.connect().await.unwrap();
        loop {
            if matches!(
                events.recv().await.unwrap(),
                LiveTranslateServerEvent::TranslationDraft(_)
            ) {
                break;
            }
        }
        let started = tokio::time::Instant::now();
        client
            .finish(Duration::from_millis(
                mimi_core::translation_policy::GEMINI_FINISH_TIMEOUT_MS,
            ))
            .await;
        assert!(started.elapsed() >= Duration::from_millis(2_100));
        assert!(started.elapsed() < Duration::from_millis(4_000));
        let mut pairs = Vec::new();
        let mut finished = 0;
        while let Ok(event) = events.try_recv() {
            match event {
                LiveTranslateServerEvent::SubtitleFinalPair {
                    source,
                    translation,
                    ..
                } => {
                    pairs.push((source, translation));
                }
                LiveTranslateServerEvent::SessionFinished => finished += 1,
                LiveTranslateServerEvent::Error { .. } => {
                    panic!("paired quiet tail must finish cleanly")
                }
                _ => {}
            }
        }
        assert_eq!(pairs, vec![("Synthetic source".into(), "合成译文".into())]);
        assert_eq!(finished, 1);
    }

    #[tokio::test]
    async fn websocket_drafts_update_the_shared_snapshot_before_unpunctuated_quiet_finals() {
        use crate::core::models::SubtitleEvent;
        use mimi_core::subtitle_reducer::{NoopArchive, SubtitleReducer};

        let (frames, mut pending) = tokio::sync::mpsc::channel::<Value>(8);
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                assert_setup(socket.next().await.unwrap().unwrap());
                socket
                    .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                    .await
                    .unwrap();
                while let Some(frame) = pending.recv().await {
                    socket
                        .send(Message::Text(frame.to_string().into()))
                        .await
                        .unwrap();
                }
                while socket.next().await.is_some() {}
            })
        })
        .await;
        client.connect().await.unwrap();
        let mut reducer = SubtitleReducer::<NoopArchive>::new(6);
        let mut draft_count = 0;
        let mut final_count = 0;
        for (block, chunks) in [["合", "成", "合成第一段"], ["第", "二", "第二段译文"]]
            .into_iter()
            .enumerate()
        {
            // The second block's translation deliberately arrives before ASR.
            if block == 0 {
                frames
                    .send(json!({"serverContent":{"inputTranscription":{"text":"Synthetic first source","languageCode":"en"}}}))
                    .await
                    .unwrap();
            }
            let mut expected = String::new();
            for chunk in chunks {
                if chunk.starts_with(&expected) {
                    expected = chunk.into();
                } else {
                    expected.push_str(chunk);
                }
                frames
                    .send(json!({"serverContent":{"outputTranscription":{"text":chunk}}}))
                    .await
                    .unwrap();
                loop {
                    let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
                        .await
                        .expect("a draft must not wait for the two-second final boundary")
                        .unwrap();
                    match event {
                        LiveTranslateServerEvent::SourceDraft { text, .. } => {
                            reducer.apply(SubtitleEvent::SourceDraft(text));
                        }
                        LiveTranslateServerEvent::TranslationDraft(text) => {
                            assert_eq!(text, expected);
                            reducer.apply(SubtitleEvent::TranslationDraft(text));
                            draft_count += 1;
                            break;
                        }
                        LiveTranslateServerEvent::SubtitleFinalPair { .. }
                        | LiveTranslateServerEvent::Error { .. } => {
                            panic!("no final or error is allowed before the live draft")
                        }
                        _ => {}
                    }
                }
                assert!(!reducer.snapshot.translation.is_final);
                assert_eq!(reducer.snapshot.translation.text, expected);
                assert_eq!(reducer.snapshot.history.len(), block);
                if block == 1 {
                    assert!(reducer.snapshot.display_pair_final);
                    assert_ne!(
                        reducer.snapshot.display_pair.as_ref().unwrap().translation,
                        expected
                    );
                    assert!(reducer.snapshot.source.is_final);
                }
            }
            if block == 1 {
                frames
                    .send(json!({"serverContent":{"inputTranscription":{"text":"Synthetic second source","languageCode":"en"}}}))
                    .await
                    .unwrap();
            }
            let quiet_started = std::time::Instant::now();
            loop {
                let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
                    .await
                    .expect("unpunctuated text must close without turnComplete")
                    .unwrap();
                match event {
                    LiveTranslateServerEvent::SourceDraft { text, .. } => {
                        reducer.apply(SubtitleEvent::SourceDraft(text));
                    }
                    LiveTranslateServerEvent::SubtitleFinalPair {
                        source,
                        translation,
                        ..
                    } => {
                        assert!(quiet_started.elapsed() >= Duration::from_millis(1_900));
                        assert_eq!(translation, expected);
                        reducer.apply(SubtitleEvent::FinalPair {
                            source,
                            translation,
                        });
                        final_count += 1;
                        break;
                    }
                    LiveTranslateServerEvent::Error { .. } => panic!("unexpected provider error"),
                    _ => {}
                }
            }
            assert_eq!(reducer.snapshot.history.len(), block + 1);
            assert!(reducer.snapshot.translation.is_final);
        }
        assert_eq!((draft_count, final_count), (6, 2));
        client.disconnect().await;
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
        assert_eq!(
            client.probe_transport_stop(),
            Some(ProbeTransportStop::GoAway)
        );
        client.disconnect().await;
    }

    async fn rotating_test_client(
        setup_delay: Duration,
    ) -> (GeminiLiveClient, ProviderEventReceiver, Arc<Mutex<Vec<u8>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&samples);
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut old = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert_setup(old.next().await.unwrap().unwrap());
            old.send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            let old_samples = Arc::clone(&recorded);
            let old_task = tokio::spawn(async move {
                let mut first = true;
                while let Some(Ok(message)) = old.next().await {
                    let Message::Text(text) = message else {
                        continue;
                    };
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if let Some(audio) = value["realtimeInput"]["audio"]["data"].as_str() {
                        let pcm = base64::engine::general_purpose::STANDARD
                            .decode(audio)
                            .unwrap();
                        old_samples
                            .lock()
                            .await
                            .extend(pcm.into_iter().filter(|byte| *byte != 0));
                        if first {
                            first = false;
                            old.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"Old source"},"outputTranscription":{"text":"旧译文"}}}"#.into())).await.unwrap();
                            // Duplicate notifications must not start a second replacement.
                            for _ in 0..2 {
                                old.send(Message::Text(
                                    r#"{"goAway":{"timeLeft":"private"}}"#.into(),
                                ))
                                .await
                                .unwrap();
                            }
                        }
                    } else if value["realtimeInput"]["audioStreamEnd"] == true {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        if old.send(Message::Text(r#"{"serverContent":{"outputTranscription":{"text":"完整。"},"turnComplete":true}}"#.into())).await.is_err() { return; }
                    }
                }
            });
            let (stream, _) = listener.accept().await.unwrap();
            let mut new = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert_setup(new.next().await.unwrap().unwrap());
            tokio::time::sleep(setup_delay).await;
            if new
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .is_err()
            {
                old_task.abort();
                return;
            }
            let mut first = true;
            while let Some(Ok(message)) = new.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                let value: Value = serde_json::from_str(&text).unwrap();
                if let Some(audio) = value["realtimeInput"]["audio"]["data"].as_str() {
                    let pcm = base64::engine::general_purpose::STANDARD
                        .decode(audio)
                        .unwrap();
                    recorded
                        .lock()
                        .await
                        .extend(pcm.into_iter().filter(|byte| *byte != 0));
                    if first {
                        first = false;
                        new.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"New source"},"outputTranscription":{"text":"新会话译文。"}}}"#.into())).await.unwrap();
                    }
                } else if value["realtimeInput"]["audioStreamEnd"] == true {
                    let _ = new
                        .send(Message::Text(
                            r#"{"serverContent":{"turnComplete":true}}"#.into(),
                        ))
                        .await;
                }
            }
            old_task.abort();
        });
        let (events, receiver) = provider_event_channel();
        let client = GeminiLiveClient::with_endpoint(
            "gemini-test-key-not-real",
            TargetLanguage::Japanese,
            events,
            url::Url::parse(&format!("ws://{address}/live")).unwrap(),
            false,
        )
        .unwrap();
        (client, receiver, samples)
    }

    #[tokio::test]
    async fn planned_rotation_keeps_native_audio_queue_and_late_old_caption() {
        let (client, mut events, samples) = rotating_test_client(Duration::from_millis(300)).await;
        client.connect().await.unwrap();
        let reader = tokio::spawn(async move {
            let mut received = Vec::new();
            while let Some(event) = events.recv().await {
                let finished = matches!(event, LiveTranslateServerEvent::SessionFinished);
                received.push(event);
                if finished {
                    break;
                }
            }
            received
        });
        let failed = Arc::new(AtomicBool::new(false));
        let failed_callback = Arc::clone(&failed);
        let send_client = client.clone();
        let pipeline = crate::audio::send_pipeline::AudioSendPipeline::spawn(
            move |data| {
                let client = send_client.clone();
                async move { client.send_audio(&data).await }
            },
            move |_| {
                failed_callback.store(true, Ordering::SeqCst);
            },
        );
        let ingress = pipeline.ingress().unwrap();
        // Native callbacks/resampler output can be much smaller than a wire
        // frame. Twenty queued buffers therefore do not imply two seconds.
        for index in 0..150 {
            ingress
                .try_send(vec![
                    index / 5 + 1;
                    GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT / 5
                ])
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(pipeline.finish(Duration::from_secs(2)).await);
        assert!(!failed.load(Ordering::SeqCst));
        client.finish(Duration::from_secs(2)).await;
        let received = tokio::time::timeout(Duration::from_secs(1), reader)
            .await
            .unwrap()
            .unwrap();
        // A session boundary pads one partial frame, so compare every original
        // nonzero PCM byte, ignoring only that known zero padding.
        let expected = (1..=30)
            .flat_map(|index| {
                std::iter::repeat_n(index, GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT)
            })
            .collect::<Vec<u8>>();
        assert_eq!(*samples.lock().await, expected);
        assert_eq!(
            received
                .iter()
                .filter(|event| matches!(event, LiveTranslateServerEvent::SessionCreated))
                .count(),
            1
        );
        assert_eq!(
            client
                .probe_connection_events()
                .iter()
                .filter(|event| event.kind == "geminiConnectionRotated")
                .count(),
            1
        );
        assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. } if source == "Old source" && translation == "旧译文完整。")));
        assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::TranslationDraft(text) if text == "新会话译文。")));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
        assert_eq!(client.probe_transport_stop(), None);
    }

    async fn wait_for_rotation(client: &GeminiLiveClient, kind: &'static str) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !client
                .probe_connection_events()
                .iter()
                .any(|event| event.kind == kind)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn wait_for_handoff(client: &GeminiLiveClient) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !client.inner.rotating.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn stop_during_handoff_drains_staged_pcm_into_replacement() {
        let (client, mut events, samples) = rotating_test_client(Duration::ZERO).await;
        client.connect().await.unwrap();
        client
            .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        wait_for_handoff(&client).await;
        client
            .send_audio(&vec![2; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        client.finish(Duration::from_secs(3)).await;
        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        let expected = [1, 2]
            .into_iter()
            .flat_map(|index| {
                std::iter::repeat_n(index, GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT)
            })
            .collect::<Vec<u8>>();
        assert_eq!(*samples.lock().await, expected);
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, .. } if source == "New source")));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
        assert!(!client.inner.rotating.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn stop_waits_for_old_tail_after_replacement_setup() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (replacement_setup, setup_received) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut old = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert_setup(old.next().await.unwrap().unwrap());
            old.send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            assert!(matches!(old.next().await, Some(Ok(Message::Text(_)))));
            old.send(Message::Text(r#"{"goAway":{}}"#.into()))
                .await
                .unwrap();

            let (stream, _) = listener.accept().await.unwrap();
            let mut replacement = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert_setup(replacement.next().await.unwrap().unwrap());
            replacement_setup.send(()).unwrap();
            let Message::Text(end) = old.next().await.unwrap().unwrap() else {
                panic!("Stop must end the old audio stream");
            };
            assert_eq!(
                serde_json::from_str::<Value>(&end).unwrap()["realtimeInput"]["audioStreamEnd"],
                true
            );
            // Stop is now waiting for the old tail. Complete replacement setup
            // first, and release that tail only after Stop rejects the replacement.
            replacement
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            let replacement_closed =
                matches!(replacement.next().await, Some(Ok(Message::Close(_))));
            let tail_sent = old.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"Old closing source"},"outputTranscription":{"text":"Old closing translation"},"turnComplete":true}}"#.into())).await.is_ok();
            while let Some(Ok(message)) = old.next().await {
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
            (replacement_closed, tail_sent)
        });
        let (sender, mut events) = provider_event_channel();
        let client = GeminiLiveClient::with_endpoint(
            "gemini-test-key-not-real",
            TargetLanguage::Japanese,
            sender,
            url::Url::parse(&format!("ws://{address}/live")).unwrap(),
            false,
        )
        .unwrap();
        client.connect().await.unwrap();
        client
            .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), setup_received)
            .await
            .unwrap()
            .unwrap();
        client.finish(Duration::from_secs(2)).await;
        let (replacement_closed, tail_sent) = tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap();
        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(
            replacement_closed,
            "replacement setup must not block behind Stop's final-turn wait"
        );
        assert!(tail_sent);
        assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. } if source == "Old closing source" && translation == "Old closing translation")));
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
        assert!(!client
            .probe_connection_events()
            .iter()
            .any(|event| event.kind == "geminiConnectionRotated"));
    }

    #[tokio::test]
    async fn stop_waits_for_replacement_tail_after_old_turn_complete() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (old_ended, old_end_received) = tokio::sync::oneshot::channel();
        let (release_old_tail, old_tail_released) = tokio::sync::oneshot::channel();
        let (replacement_ended, replacement_end_received) = tokio::sync::oneshot::channel();
        let (release_replacement_tail, replacement_tail_released) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut old = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert_setup(old.next().await.unwrap().unwrap());
            old.send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            assert!(matches!(old.next().await, Some(Ok(Message::Text(_)))));
            old.send(Message::Text(r#"{"goAway":{}}"#.into()))
                .await
                .unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let mut replacement = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert_setup(replacement.next().await.unwrap().unwrap());
            replacement
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .unwrap();
            let Message::Text(end) = old.next().await.unwrap().unwrap() else {
                panic!("Rotation must end the old audio stream");
            };
            assert_eq!(
                serde_json::from_str::<Value>(&end).unwrap()["realtimeInput"]["audioStreamEnd"],
                true
            );
            old_ended.send(()).unwrap();
            old_tail_released.await.unwrap();
            old.send(Message::Text(
                r#"{"serverContent":{"turnComplete":true}}"#.into(),
            ))
            .await
            .unwrap();
            let Message::Text(audio) = replacement.next().await.unwrap().unwrap() else {
                panic!("Replacement must receive staged audio before Stop");
            };
            let audio: Value = serde_json::from_str(&audio).unwrap();
            assert_eq!(
                base64::engine::general_purpose::STANDARD
                    .decode(audio["realtimeInput"]["audio"]["data"].as_str().unwrap())
                    .unwrap(),
                vec![2; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT]
            );
            let Message::Text(end) = replacement.next().await.unwrap().unwrap() else {
                panic!("Stop must end the replacement audio stream");
            };
            assert_eq!(
                serde_json::from_str::<Value>(&end).unwrap()["realtimeInput"]["audioStreamEnd"],
                true
            );
            replacement_ended.send(()).unwrap();
            replacement_tail_released.await.unwrap();
            let tail_sent = replacement.send(Message::Text(r#"{"serverContent":{"inputTranscription":{"text":"Replacement closing source"},"outputTranscription":{"text":"Replacement closing translation"},"turnComplete":true}}"#.into())).await.is_ok();
            while let Some(Ok(message)) = replacement.next().await {
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
            tail_sent
        });
        let (sender, mut events) = provider_event_channel();
        let client = GeminiLiveClient::with_endpoint(
            "gemini-test-key-not-real",
            TargetLanguage::Japanese,
            sender,
            url::Url::parse(&format!("ws://{address}/live")).unwrap(),
            false,
        )
        .unwrap();
        client.connect().await.unwrap();
        client
            .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), old_end_received)
            .await
            .unwrap()
            .unwrap();
        wait_for_handoff(&client).await;
        client
            .send_audio(&vec![2; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        let closing_client = client.clone();
        let mut closing = tokio::spawn(async move {
            closing_client.finish(Duration::from_secs(4)).await;
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !client.inner.is_closing.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Only the old socket acknowledges after Stop starts, so its boundary
        // must not satisfy the replacement's final-turn wait.
        release_old_tail.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), replacement_end_received)
            .await
            .unwrap()
            .unwrap();
        let finished_early =
            tokio::time::timeout(TAIL_QUIET_PERIOD + Duration::from_millis(200), &mut closing)
                .await;
        release_replacement_tail.send(()).unwrap();
        let finished_before_replacement_tail = match finished_early {
            Ok(result) => {
                result.unwrap();
                true
            }
            Err(_) => {
                tokio::time::timeout(Duration::from_secs(2), closing)
                    .await
                    .unwrap()
                    .unwrap();
                false
            }
        };
        let tail_sent = tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap();
        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(
            !finished_before_replacement_tail,
            "old turnComplete must not finish the replacement before its own tail"
        );
        assert!(tail_sent);
        assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. } if source == "Replacement closing source" && translation == "Replacement closing translation")));
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
    }

    #[tokio::test]
    async fn rotation_staging_rejects_pcm_beyond_two_seconds() {
        let (client, _events, _) = rotating_test_client(Duration::ZERO).await;
        client.connect().await.unwrap();
        client
            .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        wait_for_handoff(&client).await;
        client
            .send_audio(&vec![2; ROTATION_PENDING_PCM_BYTES])
            .await
            .unwrap();
        assert_eq!(
            client.send_audio(&[3, 3]).await,
            Err(GeminiLiveClientError::TransportFailure)
        );
        assert_eq!(
            client.inner.pending_audio.lock().await.len(),
            ROTATION_PENDING_PCM_BYTES
        );
        client.disconnect().await;
        assert!(client.inner.pending_audio.lock().await.is_empty());
        assert!(!client.inner.rotating.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn stop_during_rotation_setup_does_not_install_replacement() {
        let (client, mut events, _) = rotating_test_client(Duration::from_secs(2)).await;
        client.connect().await.unwrap();
        client
            .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        wait_for_rotation(&client, "geminiGoAwayReceived").await;
        client.finish(Duration::from_secs(2)).await;
        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!client
            .probe_connection_events()
            .iter()
            .any(|event| event.kind == "geminiConnectionRotated"));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn clear_during_rotation_does_not_confirm_retired_text() {
        let (client, mut events, _) = rotating_test_client(Duration::from_millis(300)).await;
        client.connect().await.unwrap();
        client
            .send_audio(&vec![1; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        wait_for_rotation(&client, "geminiGoAwayReceived").await;
        client.clear_content().await;
        let mut received = Vec::new();
        wait_for_rotation(&client, "geminiConnectionRotated").await;
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        client
            .send_audio(&vec![2; GeminiLiveEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        client.finish(Duration::from_secs(2)).await;
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(!received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, .. } if source == "Old source")));
        assert!(received.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. } if source == "New source" && translation == "新会话译文。")));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
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
