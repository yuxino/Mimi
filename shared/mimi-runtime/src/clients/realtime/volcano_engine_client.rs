//! Volcano Engine Doubao Simultaneous Interpretation 2.0 WebSocket client.

mod committer;
mod receive;

use crate::clients::transport::provider_events::ProviderEventSender;
#[cfg(test)]
use crate::core::models::UtteranceRole;
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
#[cfg(test)]
use crate::core::protocols::realtime::volcano_engine::VolcanoEngineServerEvent;
use crate::core::protocols::realtime::volcano_engine::{
    VolcanoEngineEndpoint, VolcanoEngineProtocolError, VolcanoEngineRequestEncoder,
};
use committer::VolcanoSubtitlePairCommitter;
#[cfg(test)]
#[allow(unused_imports)]
use committer::{
    PendingFinal, PendingFinalTextLimitExceeded, SubtitleCommitResult, SubtitleTiming,
    MAXIMUM_PENDING_FINALS_PER_SIDE, MAXIMUM_PENDING_FINAL_TEXT_BYTES,
    MAXIMUM_RECENT_COMMITTED_TIMINGS, MAXIMUM_SOURCE_BLOCKS,
};
use futures_util::{SinkExt, StreamExt};
#[cfg(test)]
use receive::{
    handle_server_event, GENERIC_PROTOCOL_ERROR, TRANSCRIPT_SAFETY_LIMIT_ERROR,
    UNEXPECTED_SESSION_FINISHED_ERROR,
};
use receive::{receive_loop, ReceiveContext};
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
use uuid::Uuid;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SETUP_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(250);
const MAXIMUM_AUDIO_INPUT_BYTES_PER_CALL: usize = 1_024 * 1_024;
const GENERIC_TRANSPORT_ERROR: &str = "The Volcano Engine connection failed.";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VolcanoEngineClientError {
    #[error("credential_authentication_failed")]
    AuthenticationFailed,
    #[error("Add a Volcano Engine API key in Settings.")]
    MissingAPIKey,
    #[error("Volcano Engine requires an explicit supported source language.")]
    UnsupportedSourceLanguage,
    #[error("Volcano Engine does not support this translation direction.")]
    UnsupportedTargetLanguage,
    #[error("The Volcano Engine translation session is not connected.")]
    NotConnected,
    #[error("The Volcano Engine connection stopped responding.")]
    HealthCheckTimedOut,
    #[error("The Volcano Engine connection could not be established in time.")]
    ConnectionTimedOut,
    #[error("Volcano Engine did not confirm the session configuration in time.")]
    SessionSetupTimedOut,
    #[error("Volcano Engine rejected the session configuration.")]
    SessionSetupRejected,
    #[error("The Volcano Engine transport failed.")]
    TransportFailure,
    #[error("The Volcano Engine audio input exceeded its bounded call limit.")]
    AudioInputTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected,
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type Stream = futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

struct Inner {
    // Serializes the content barrier with local assembly, never socket/audio state.
    content_lock: Mutex<()>,
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    audio_send_lock: Mutex<()>,
    pending_audio: Mutex<Vec<u8>>,
    committer: Mutex<VolcanoSubtitlePairCommitter>,
    session_id: Mutex<Option<String>>,
    ready: AtomicBool,
    is_closing: AtomicBool,
    received_session_finished: AtomicBool,
    receive_ended: AtomicBool,
    session_finished_notify: Notify,
    pong_notify: Notify,
    generation: AtomicU64,
}

/// The API key is kept out of `Debug` output and attached only to the
/// short-lived WebSocket upgrade request. Provider text and provider error
/// messages are never logged or surfaced by this client.
#[derive(Clone)]
pub struct VolcanoEngineClient {
    network: crate::clients::transport::provider_network::ProviderNetwork,
    inner: Arc<Inner>,
    endpoint: url::Url,
    api_key: String,
    source_language: SourceLanguage,
    target_language: TargetLanguage,
    events: ProviderEventSender,
}

impl VolcanoEngineClient {
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.clone()
    }

    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    pub async fn clear_content(&self) -> u64 {
        let _content = self.inner.content_lock.lock().await;
        self.inner.committer.lock().await.clear_content();
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
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        events: ProviderEventSender,
    ) -> Result<Self, VolcanoEngineClientError> {
        let endpoint =
            VolcanoEngineEndpoint::url().map_err(|_| VolcanoEngineClientError::TransportFailure)?;
        Self::with_endpoint(api_key, source_language, target_language, events, endpoint)
    }

    fn with_endpoint(
        api_key: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        events: ProviderEventSender,
        endpoint: url::Url,
    ) -> Result<Self, VolcanoEngineClientError> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(VolcanoEngineClientError::MissingAPIKey);
        }
        VolcanoEngineRequestEncoder::validate_languages(source_language, target_language)
            .map_err(map_language_error)?;
        Ok(Self {
            network: crate::clients::transport::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                content_lock: Mutex::new(()),
                sink: Mutex::new(None),
                receive_task: Mutex::new(None),
                audio_send_lock: Mutex::new(()),
                pending_audio: Mutex::new(Vec::new()),
                committer: Mutex::new(VolcanoSubtitlePairCommitter::new(source_language)),
                session_id: Mutex::new(None),
                ready: AtomicBool::new(false),
                is_closing: AtomicBool::new(false),
                received_session_finished: AtomicBool::new(false),
                receive_ended: AtomicBool::new(false),
                session_finished_notify: Notify::new(),
                pong_notify: Notify::new(),
                generation: AtomicU64::new(0),
            }),
            endpoint,
            api_key: api_key.to_string(),
            source_language,
            target_language,
            events,
        })
    }

    pub async fn connect(&self) -> Result<(), VolcanoEngineClientError> {
        self.connect_with_timeout(SETUP_TIMEOUT).await
    }

    async fn connect_with_timeout(
        &self,
        readiness_timeout: Duration,
    ) -> Result<(), VolcanoEngineClientError> {
        self.disconnect().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);

        let mut request = self
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|_| VolcanoEngineClientError::TransportFailure)?;
        request.headers_mut().insert(
            "X-Api-Key",
            HeaderValue::from_str(&self.api_key)
                .map_err(|_| VolcanoEngineClientError::MissingAPIKey)?,
        );
        request.headers_mut().insert(
            "X-Api-Resource-Id",
            HeaderValue::from_static(VolcanoEngineEndpoint::RESOURCE_ID),
        );

        let (socket, _) = tokio::time::timeout(
            CONNECT_TIMEOUT,
            crate::clients::transport::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| VolcanoEngineClientError::ConnectionTimedOut)?
        .map_err(|error| {
            if crate::clients::transport::connection_diagnostics::authentication_rejected(&error) {
                VolcanoEngineClientError::AuthenticationFailed
            } else {
                VolcanoEngineClientError::TransportFailure
            }
        })?;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .received_session_finished
            .store(false, Ordering::SeqCst);
        self.inner.receive_ended.store(false, Ordering::SeqCst);
        self.inner.pending_audio.lock().await.clear();
        self.inner.committer.lock().await.reset();
        let session_id = Uuid::new_v4().to_string();
        *self.inner.session_id.lock().await = Some(session_id.clone());

        let (setup_tx, setup_rx) = watch::channel(SetupState::Awaiting);
        let task = tokio::spawn(receive_loop(ReceiveContext {
            inner: Arc::clone(&self.inner),
            stream,
            events: self.events.clone(),
            setup: setup_tx,
            generation,
        }));
        *self.inner.receive_task.lock().await = Some(task);

        let start = VolcanoEngineRequestEncoder::start_session(
            &session_id,
            self.source_language,
            self.target_language,
        )
        .map_err(map_language_error)?;
        let setup = async {
            self.send_binary(start).await?;
            wait_for_setup(setup_rx).await
        };
        let result = match tokio::time::timeout(readiness_timeout, setup).await {
            Ok(result) => result,
            Err(_) => Err(VolcanoEngineClientError::SessionSetupTimedOut),
        };
        match result {
            Ok(()) => {
                self.inner.ready.store(true, Ordering::SeqCst);
                Ok(())
            }
            Err(error) => {
                self.disconnect().await;
                Err(error)
            }
        }
    }

    /// Accepts arbitrary PCM chunks and emits exact 80 ms provider frames.
    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), VolcanoEngineClientError> {
        if pcm_data.is_empty() {
            return Ok(());
        }
        if pcm_data.len() > MAXIMUM_AUDIO_INPUT_BYTES_PER_CALL {
            return Err(VolcanoEngineClientError::AudioInputTooLarge);
        }
        tokio::time::timeout(SEND_TIMEOUT, self.send_audio_operation(pcm_data))
            .await
            .map_err(|_| VolcanoEngineClientError::TransportFailure)?
    }

    async fn send_audio_operation(&self, pcm_data: &[u8]) -> Result<(), VolcanoEngineClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(VolcanoEngineClientError::NotConnected);
        }
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let _send_guard = self.inner.audio_send_lock.lock().await;
        if !self.is_current_generation(generation)
            || !self.inner.ready.load(Ordering::SeqCst)
            || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(VolcanoEngineClientError::NotConnected);
        }
        let session_id = self.current_session_id().await?;
        let frames = {
            let mut pending = self.inner.pending_audio.lock().await;
            pending.extend_from_slice(pcm_data);
            take_complete_audio_messages(&session_id, &mut pending)?
        };
        for frame in frames {
            if !self.is_current_generation(generation)
                || !self.inner.ready.load(Ordering::SeqCst)
                || self.inner.is_closing.load(Ordering::SeqCst)
            {
                return Err(VolcanoEngineClientError::NotConnected);
            }
            self.send_binary(frame).await?;
        }
        Ok(())
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), VolcanoEngineClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(VolcanoEngineClientError::NotConnected);
        }
        let operation = async {
            let pong = self.inner.pong_notify.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(VolcanoEngineClientError::NotConnected);
                };
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| VolcanoEngineClientError::TransportFailure)?;
            }
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| VolcanoEngineClientError::HealthCheckTimedOut)?
    }

    /// Flushes a final zero-padded 80 ms frame, sends FinishSession, waits for
    /// SessionFinished, and then tears down the transport.
    pub async fn finish(&self, timeout: Duration) {
        if !self.inner.ready.load(Ordering::SeqCst) {
            return;
        }
        if self.inner.is_closing.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner
            .received_session_finished
            .store(false, Ordering::SeqCst);
        let generation = self.inner.generation.load(Ordering::SeqCst);
        match tokio::time::timeout(timeout, self.finish_operation(generation)).await {
            Ok(Ok(())) => {}
            Ok(Err(_))
                if self.is_current_generation(generation)
                    && !self.inner.receive_ended.load(Ordering::SeqCst) =>
            {
                self.emit(
                    LiveTranslateServerEvent::Error {
                        code: "volcano_session_close_failed".into(),
                        message: GENERIC_TRANSPORT_ERROR.into(),
                    },
                    generation,
                );
            }
            Err(_)
                if self.is_current_generation(generation)
                    && !self.inner.receive_ended.load(Ordering::SeqCst) =>
            {
                self.emit(
                    LiveTranslateServerEvent::Error {
                        code: "volcano_close_timeout".into(),
                        message: "Volcano Engine did not finish closing in time.".into(),
                    },
                    generation,
                );
            }
            _ => {}
        }
        self.disconnect_if_current(generation).await;
    }

    async fn finish_operation(&self, generation: u64) -> Result<(), VolcanoEngineClientError> {
        let send_guard = self.inner.audio_send_lock.lock().await;
        if !self.is_current_generation(generation) {
            return Err(VolcanoEngineClientError::NotConnected);
        }
        let session_id = self.current_session_id().await?;
        let partial = {
            let mut pending = self.inner.pending_audio.lock().await;
            take_padded_audio_message(&session_id, &mut pending)?
        };
        if let Some(frame) = partial {
            self.send_binary(frame).await?;
        }
        let finish = VolcanoEngineRequestEncoder::finish_session(&session_id)
            .map_err(|_| VolcanoEngineClientError::TransportFailure)?;
        self.send_binary(finish).await?;
        drop(send_guard);

        loop {
            if !self.is_current_generation(generation) {
                return Err(VolcanoEngineClientError::NotConnected);
            }
            let finished = self.inner.session_finished_notify.notified();
            tokio::pin!(finished);
            finished.as_mut().enable();
            if self.inner.received_session_finished.load(Ordering::SeqCst) {
                return Ok(());
            }
            if self.inner.receive_ended.load(Ordering::SeqCst) {
                return Err(VolcanoEngineClientError::NotConnected);
            }
            finished.await;
        }
    }

    pub async fn disconnect(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.receive_ended.store(true, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .received_session_finished
            .store(false, Ordering::SeqCst);
        if let Some(task) = self.inner.receive_task.lock().await.take() {
            task.abort();
        }
        let sink = self.inner.sink.lock().await.take();
        if let Some(mut sink) = sink {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.close()).await;
        }
        self.inner.pending_audio.lock().await.clear();
        self.inner.committer.lock().await.reset();
        *self.inner.session_id.lock().await = None;
        self.inner.session_finished_notify.notify_waiters();
        self.inner.pong_notify.notify_waiters();
    }

    async fn send_binary(&self, frame: Vec<u8>) -> Result<(), VolcanoEngineClientError> {
        let mut sink = self.inner.sink.lock().await;
        let Some(sink) = sink.as_mut() else {
            return Err(VolcanoEngineClientError::NotConnected);
        };
        let evidence =
            crate::development_audio::begin_volcano(&frame, VolcanoEngineEndpoint::SAMPLE_RATE_HZ);
        tokio::time::timeout(
            SEND_TIMEOUT,
            evidence.observe(sink.send(Message::Binary(frame.into()))),
        )
        .await
        .map_err(|_| VolcanoEngineClientError::TransportFailure)?
        .map_err(|_| VolcanoEngineClientError::TransportFailure)
    }

    async fn current_session_id(&self) -> Result<String, VolcanoEngineClientError> {
        self.inner
            .session_id
            .lock()
            .await
            .clone()
            .ok_or(VolcanoEngineClientError::NotConnected)
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

fn map_language_error(error: VolcanoEngineProtocolError) -> VolcanoEngineClientError {
    match error {
        VolcanoEngineProtocolError::UnsupportedSourceLanguage => {
            VolcanoEngineClientError::UnsupportedSourceLanguage
        }
        VolcanoEngineProtocolError::UnsupportedTargetLanguage => {
            VolcanoEngineClientError::UnsupportedTargetLanguage
        }
        _ => VolcanoEngineClientError::TransportFailure,
    }
}

fn take_complete_audio_messages(
    session_id: &str,
    pending: &mut Vec<u8>,
) -> Result<Vec<Vec<u8>>, VolcanoEngineClientError> {
    let frame_size = VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let complete_bytes = pending.len() / frame_size * frame_size;
    let result = pending[..complete_bytes]
        .chunks_exact(frame_size)
        .map(|frame| {
            VolcanoEngineRequestEncoder::audio(session_id, frame)
                .map_err(|_| VolcanoEngineClientError::TransportFailure)
        })
        .collect::<Result<Vec<_>, _>>();
    // Preserve the existing failure semantics: a complete batch is consumed
    // even if encoding one of its frames fails.
    pending.drain(..complete_bytes);
    result
}

fn take_padded_audio_message(
    session_id: &str,
    pending: &mut Vec<u8>,
) -> Result<Option<Vec<u8>>, VolcanoEngineClientError> {
    if pending.is_empty() {
        return Ok(None);
    }
    let mut frame = std::mem::take(pending);
    frame.resize(VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT, 0);
    VolcanoEngineRequestEncoder::audio(session_id, &frame)
        .map(Some)
        .map_err(|_| VolcanoEngineClientError::TransportFailure)
}

async fn wait_for_setup(
    mut setup: watch::Receiver<SetupState>,
) -> Result<(), VolcanoEngineClientError> {
    loop {
        match *setup.borrow() {
            SetupState::Ready => return Ok(()),
            SetupState::Rejected => return Err(VolcanoEngineClientError::SessionSetupRejected),
            SetupState::Awaiting => {}
        }
        if setup.changed().await.is_err() {
            return Err(VolcanoEngineClientError::SessionSetupRejected);
        }
    }
}

#[cfg(test)]
mod tests;
