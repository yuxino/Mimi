//! Google Gemini Live Translation WebSocket client.

mod rotation;
mod turn_boundary;

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::core::models::TargetLanguage;
use crate::core::protocols::realtime::gemini_live::{
    GeminiLiveEndpoint, GeminiLiveProtocolError, GeminiLiveRequestEncoder, GeminiLiveServerEvent,
};
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use futures_util::{SinkExt, StreamExt};
#[cfg(test)]
use rotation::prepare_replacement;
use rotation::{record_rotation, rotate_connection, RotationFailure};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use turn_boundary::{
    cancel_normal_turn_boundary, publish_turn_after_transcript_quiet, schedule_normal_turn_commit,
    schedule_normal_turn_commit_for, TranscriptBoundary,
};
#[cfg(test)]
#[allow(unused_imports)]
use turn_boundary::{transcript_boundary_is_current, TAIL_QUIET_PERIOD};

const GENERIC_PROVIDER_ERROR: &str = "Gemini Live Translation rejected the session.";
const GENERIC_TRANSPORT_ERROR: &str = "The Gemini Live Translation connection failed.";
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
// Native callback sizes vary; stage by PCM duration, not number of callbacks.
const ROTATION_PENDING_PCM_BYTES: usize = 2 * 16_000 * 2;
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
    #[error("gemini_live_service_unavailable")]
    ServiceUnavailable,
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
    Rejected(GeminiLiveClientError),
}

impl GeminiLiveClientError {
    fn from_provider_code(code: &str) -> Self {
        match code {
            "unauthenticated" | "permission_denied" => Self::AuthenticationFailed,
            "unavailable" => Self::ServiceUnavailable,
            _ => Self::SessionSetupRejected,
        }
    }

    fn server_event(self) -> LiveTranslateServerEvent {
        let (code, message) = match self {
            Self::AuthenticationFailed => {
                ("credential_authentication_failed", GENERIC_PROVIDER_ERROR)
            }
            Self::ServiceUnavailable => ("gemini_live_service_unavailable", GENERIC_PROVIDER_ERROR),
            Self::TransportFailure
            | Self::NotConnected
            | Self::HealthCheckTimedOut
            | Self::SessionSetupTimedOut => ("transport_error", GENERIC_TRANSPORT_ERROR),
            _ => ("gemini_live_provider_error", GENERIC_PROVIDER_ERROR),
        };
        LiveTranslateServerEvent::Error {
            code: code.into(),
            message: message.into(),
        }
    }
}

fn connection_error(error: tokio_tungstenite::tungstenite::Error) -> GeminiLiveClientError {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => match response.status().as_u16() {
            401 | 403 => GeminiLiveClientError::AuthenticationFailed,
            503 => GeminiLiveClientError::ServiceUnavailable,
            _ => GeminiLiveClientError::SessionSetupRejected,
        },
        _ => GeminiLiveClientError::TransportFailure,
    }
}

struct GeminiTranscriptPairCommitter {
    stream: crate::core::subtitles::openai_transcript_committer::OpenAITranscriptPairCommitter,
    source_language: Option<String>,
    clock: std::time::Instant,
    discard_current_turn: bool,
    turn_complete_received: bool,
}

impl Default for GeminiTranscriptPairCommitter {
    fn default() -> Self {
        Self {
            stream:
                crate::core::subtitles::openai_transcript_committer::OpenAITranscriptPairCommitter::new_gemini(
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

#[derive(Clone, Copy)]
struct TransportHealth {
    generation: u64,
    connected_at: Instant,
    stable: bool,
}

impl TransportHealth {
    fn connected(generation: u64) -> Self {
        Self {
            generation,
            connected_at: Instant::now(),
            stable: false,
        }
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
    transport_health: Mutex<Option<TransportHealth>>,
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
    network: crate::clients::transport::provider_network::ProviderNetwork,
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
        network: crate::clients::transport::provider_network::ProviderNetwork,
    ) -> Result<(), crate::clients::transport::provider_network::ProviderNetworkError> {
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
            network: crate::clients::transport::provider_network::ProviderNetwork::default(),
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
                transport_health: Mutex::new(None),
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
            crate::clients::transport::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| GeminiLiveClientError::TransportFailure)?
        .map_err(connection_error)?;
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
                    SetupState::Rejected(error) => return Err(error),
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
            let observed = {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(GeminiLiveClientError::NotConnected);
                };
                let observed = self
                    .inner
                    .transport_health
                    .lock()
                    .await
                    .as_ref()
                    .copied()
                    .ok_or(GeminiLiveClientError::NotConnected)?;
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| GeminiLiveClientError::TransportFailure)?;
                observed
            };
            pong.await;
            self.record_transport_health(observed, Instant::now()).await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| GeminiLiveClientError::HealthCheckTimedOut)?
    }

    async fn record_transport_health(&self, observed: TransportHealth, now: Instant) -> bool {
        let mut transport = self.inner.transport_health.lock().await;
        let Some(current) = transport.as_mut() else {
            return false;
        };
        if self.inner.generation.load(Ordering::SeqCst) != observed.generation
            || self.inner.is_closing.load(Ordering::SeqCst)
            || current.generation != observed.generation
            || current.connected_at != observed.connected_at
        {
            return false;
        }
        if crate::core::lifecycle::recovery::connection_health_is_stable(
            current.generation,
            observed.generation,
            current.connected_at,
            now,
        ) {
            current.stable = true;
        }
        current.stable
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
        *self.inner.transport_health.lock().await = None;
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

fn decode_binary_event(data: &[u8]) -> Result<Vec<GeminiLiveServerEvent>, GeminiLiveProtocolError> {
    let text = std::str::from_utf8(data).map_err(|_| GeminiLiveProtocolError::InvalidJSON)?;
    GeminiLiveServerEvent::decode(text)
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
            Ok(Message::Binary(data)) => decode_binary_event(&data),
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
                fail_receive_loop(&context, GeminiLiveClientError::TransportFailure).await;
                return;
            }
        };
        let events = match events {
            Ok(events) => events,
            Err(_) => {
                fail_receive_loop(&context, GeminiLiveClientError::SessionSetupRejected).await;
                return;
            }
        };
        for event in events {
            if event == GeminiLiveServerEvent::GoAway
                && *context.setup.borrow() == SetupState::Ready
                && !context.inner.is_closing.load(Ordering::SeqCst)
            {
                let stable = context
                    .inner
                    .transport_health
                    .lock()
                    .await
                    .as_ref()
                    .is_some_and(|health| health.generation == context.generation && health.stable);
                if !stable {
                    if handle_server_event(&context, GeminiLiveServerEvent::GoAway).await {
                        return;
                    }
                    continue;
                }
                record_rotation(&context.inner, "geminiGoAwayReceived");
                match rotate_connection(&mut context).await {
                    Ok(()) => continue,
                    Err(RotationFailure::Failed(error)) => {
                        #[cfg(test)]
                        {
                            *context.inner.probe_transport_stop.lock().unwrap() =
                                Some(ProbeTransportStop::GoAway);
                        }
                        // Replacement setup failures retain their classification;
                        // permanent rejections must not become transport retries.
                        fail_receive_loop(&context, error).await;
                        return;
                    }
                    Err(RotationFailure::AlreadyReported) => return,
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
        fail_receive_loop(&context, GeminiLiveClientError::TransportFailure).await;
    }
}

fn report_failure(context: &ReceiveContext, error: GeminiLiveClientError) {
    if *context.setup.borrow() == SetupState::Awaiting {
        let _ = context.setup.send(SetupState::Rejected(error));
    } else {
        emit_if_current(context, error.server_event());
    }
}

async fn fail_receive_loop(context: &ReceiveContext, error: GeminiLiveClientError) {
    let _content = context.inner.content_lock.lock().await;
    if context.inner.generation.load(Ordering::SeqCst) != context.generation {
        return;
    }
    cancel_normal_turn_boundary(&context.inner);
    context.inner.committer.lock().await.reset();
    report_failure(context, error);
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
        let _ = context.setup.send(SetupState::Rejected(
            GeminiLiveClientError::SessionSetupRejected,
        ));
        return true;
    }

    match event {
        GeminiLiveServerEvent::SetupComplete => {
            if setup_is_awaiting {
                *context.inner.transport_health.lock().await =
                    Some(TransportHealth::connected(context.generation));
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
            // GoAway announces a future disconnect. During Stop, keep reading
            // the existing socket's final transcripts and real turn boundary.
            if context.inner.is_closing.load(Ordering::SeqCst) {
                return false;
            }
            #[cfg(test)]
            {
                *context.inner.probe_transport_stop.lock().unwrap() =
                    Some(ProbeTransportStop::GoAway);
            }
            cancel_normal_turn_boundary(&context.inner);
            context.inner.committer.lock().await.reset();
            report_failure(context, GeminiLiveClientError::TransportFailure);
            return true;
        }
        GeminiLiveServerEvent::ProviderError { code, .. } => {
            cancel_normal_turn_boundary(&context.inner);
            context.inner.committer.lock().await.reset();
            report_failure(context, GeminiLiveClientError::from_provider_code(&code));
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
