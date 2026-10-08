//! Volcano Engine Doubao Simultaneous Interpretation 2.0 WebSocket client.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::volcano_engine::{
    VolcanoEngineEndpoint, VolcanoEngineProtocolError, VolcanoEngineRequestEncoder,
    VolcanoEngineServerEvent,
};
use futures_util::{SinkExt, StreamExt};
use std::collections::VecDeque;
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
const MAXIMUM_PENDING_FINALS_PER_SIDE: usize = 16;
const MAXIMUM_PENDING_FINAL_TEXT_BYTES: usize = 128 * 1_024;
const MAXIMUM_RECENT_COMMITTED_TIMINGS: usize = 32;
const GENERIC_PROVIDER_ERROR: &str = "Volcano Engine rejected the translation session.";
const GENERIC_PROTOCOL_ERROR: &str = "Volcano Engine returned an invalid response.";
const GENERIC_TRANSPORT_ERROR: &str = "The Volcano Engine connection failed.";
const TRANSCRIPT_SAFETY_LIMIT_ERROR: &str =
    "Volcano Engine transcript buffering exceeded its safety limit.";
const UNEXPECTED_SESSION_FINISHED_ERROR: &str =
    "Volcano Engine ended the translation session unexpectedly.";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SubtitleTiming {
    start_time_ms: Option<i32>,
    end_time_ms: Option<i32>,
}

impl SubtitleTiming {
    fn new(start_time_ms: Option<i32>, end_time_ms: Option<i32>) -> Self {
        Self {
            start_time_ms,
            end_time_ms,
        }
    }

    fn is_usable(self) -> bool {
        self.start_time_ms.is_some() || self.end_time_ms.is_some()
    }

    /// The official source and translation finals describe the same input
    /// interval. Require the full interval when both sides provide it; fall
    /// back to one shared boundary only when the other boundary is absent.
    fn matches(self, other: Self) -> bool {
        match (
            self.start_time_ms,
            self.end_time_ms,
            other.start_time_ms,
            other.end_time_ms,
        ) {
            (Some(left_start), Some(left_end), Some(right_start), Some(right_end)) => {
                left_start == right_start && left_end == right_end
            }
            (_, Some(left), _, Some(right)) => left == right,
            (Some(left), _, Some(right), _) => left == right,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingFinal {
    timing: SubtitleTiming,
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingFinalTextLimitExceeded;

type SubtitleCommitResult = Result<Option<LiveTranslateServerEvent>, PendingFinalTextLimitExceeded>;

struct VolcanoSubtitlePairCommitter {
    source_draft: String,
    translation_draft: String,
    source_in_progress: bool,
    translation_in_progress: bool,
    discard_source: bool,
    discard_translation: bool,
    source_language: String,
    sources: VecDeque<PendingFinal>,
    translations: VecDeque<PendingFinal>,
    pending_text_bytes: usize,
    recently_committed: VecDeque<SubtitleTiming>,
}

impl VolcanoSubtitlePairCommitter {
    fn new(source_language: SourceLanguage) -> Self {
        Self {
            source_draft: String::new(),
            translation_draft: String::new(),
            source_in_progress: false,
            translation_in_progress: false,
            discard_source: false,
            discard_translation: false,
            source_language: source_language.raw_value().into(),
            sources: VecDeque::new(),
            translations: VecDeque::new(),
            pending_text_bytes: 0,
            recently_committed: VecDeque::new(),
        }
    }

    fn push_source(
        &mut self,
        text: String,
        start_time_ms: Option<i32>,
        end_time_ms: Option<i32>,
    ) -> SubtitleCommitResult {
        let pending = PendingFinal {
            timing: SubtitleTiming::new(start_time_ms, end_time_ms),
            text: text.trim().to_string(),
        };
        if !self.accepts(&pending) {
            return Ok(None);
        }
        if let Some(index) = matching_index(&self.translations, pending.timing) {
            let Some(translation) = self.translations.remove(index) else {
                return Ok(None);
            };
            self.pending_text_bytes = self
                .pending_text_bytes
                .saturating_sub(translation.text.len());
            self.mark_committed(pending.timing);
            return Ok(Some(LiveTranslateServerEvent::SubtitleFinalPair {
                source: pending.text,
                language: Some(self.source_language.clone()),
                translation: translation.text,
            }));
        }
        if upsert_bounded(&mut self.sources, pending, &mut self.pending_text_bytes) {
            self.reset();
            return Err(PendingFinalTextLimitExceeded);
        }
        Ok(None)
    }

    fn push_translation(
        &mut self,
        text: String,
        start_time_ms: Option<i32>,
        end_time_ms: Option<i32>,
    ) -> SubtitleCommitResult {
        let pending = PendingFinal {
            timing: SubtitleTiming::new(start_time_ms, end_time_ms),
            text: text.trim().to_string(),
        };
        if !self.accepts(&pending) {
            return Ok(None);
        }
        if let Some(index) = matching_index(&self.sources, pending.timing) {
            let Some(source) = self.sources.remove(index) else {
                return Ok(None);
            };
            self.pending_text_bytes = self.pending_text_bytes.saturating_sub(source.text.len());
            self.mark_committed(pending.timing);
            return Ok(Some(LiveTranslateServerEvent::SubtitleFinalPair {
                source: source.text,
                language: Some(self.source_language.clone()),
                translation: pending.text,
            }));
        }
        if upsert_bounded(
            &mut self.translations,
            pending,
            &mut self.pending_text_bytes,
        ) {
            self.reset();
            return Err(PendingFinalTextLimitExceeded);
        }
        Ok(None)
    }

    fn accepts(&self, pending: &PendingFinal) -> bool {
        !pending.text.is_empty()
            && pending.timing.is_usable()
            && !self
                .recently_committed
                .iter()
                .any(|timing| timing.matches(pending.timing))
    }

    fn mark_committed(&mut self, timing: SubtitleTiming) {
        self.recently_committed.push_back(timing);
        while self.recently_committed.len() > MAXIMUM_RECENT_COMMITTED_TIMINGS {
            self.recently_committed.pop_front();
        }
    }

    fn clear_content(&mut self) {
        let active = self.source_in_progress
            || self.translation_in_progress
            || !self.sources.is_empty()
            || !self.translations.is_empty();
        // Known old final intervals remain tombstoned, without retaining text.
        let timings: Vec<_> = self
            .sources
            .iter()
            .chain(self.translations.iter())
            .map(|pending| pending.timing)
            .collect();
        for timing in timings {
            self.mark_committed(timing);
        }
        self.sources.clear();
        self.translations.clear();
        self.source_draft.clear();
        self.translation_draft.clear();
        self.pending_text_bytes = 0;
        self.discard_source |= active;
        self.discard_translation |= active;
    }

    fn reset(&mut self) {
        self.source_draft.clear();
        self.translation_draft.clear();
        self.source_in_progress = false;
        self.translation_in_progress = false;
        self.discard_source = false;
        self.discard_translation = false;
        self.sources.clear();
        self.translations.clear();
        self.pending_text_bytes = 0;
        self.recently_committed.clear();
    }
}

fn matching_index(queue: &VecDeque<PendingFinal>, timing: SubtitleTiming) -> Option<usize> {
    queue
        .iter()
        .position(|pending| pending.timing.matches(timing))
}

/// Returns true when the combined retained source/translation text exceeds
/// the safety limit. The caller resets both queues before returning control.
fn upsert_bounded(
    queue: &mut VecDeque<PendingFinal>,
    pending: PendingFinal,
    pending_text_bytes: &mut usize,
) -> bool {
    if let Some(index) = matching_index(queue, pending.timing) {
        if let Some(replaced) = queue.remove(index) {
            *pending_text_bytes = pending_text_bytes.saturating_sub(replaced.text.len());
        }
    }
    *pending_text_bytes = pending_text_bytes.saturating_add(pending.text.len());
    queue.push_back(pending);
    while queue.len() > MAXIMUM_PENDING_FINALS_PER_SIDE {
        if let Some(evicted) = queue.pop_front() {
            *pending_text_bytes = pending_text_bytes.saturating_sub(evicted.text.len());
        }
    }
    *pending_text_bytes > MAXIMUM_PENDING_FINAL_TEXT_BYTES
}

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
    session_finished_notify: Notify,
    pong_notify: Notify,
    generation: AtomicU64,
}

/// The API key is kept out of `Debug` output and attached only to the
/// short-lived WebSocket upgrade request. Provider text and provider error
/// messages are never logged or surfaced by this client.
#[derive(Clone)]
pub struct VolcanoEngineClient {
    network: super::provider_network::ProviderNetwork,
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
        network: super::provider_network::ProviderNetwork,
    ) -> Result<(), super::provider_network::ProviderNetworkError> {
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
            network: super::provider_network::ProviderNetwork::default(),
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
            super::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| VolcanoEngineClientError::ConnectionTimedOut)?
        .map_err(|error| {
            if super::connection_diagnostics::authentication_rejected(&error) {
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
            source_language: self.source_language,
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
        let _send_guard = self.inner.audio_send_lock.lock().await;
        let session_id = self.current_session_id().await?;
        let frames = {
            let mut pending = self.inner.pending_audio.lock().await;
            pending.extend_from_slice(pcm_data);
            take_complete_audio_messages(&session_id, &mut pending)?
        };
        for frame in frames {
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
            Ok(Err(_)) if self.is_current_generation(generation) => {
                self.emit(
                    LiveTranslateServerEvent::Error {
                        code: "volcano_session_close_failed".into(),
                        message: GENERIC_TRANSPORT_ERROR.into(),
                    },
                    generation,
                );
            }
            Err(_) if self.is_current_generation(generation) => {
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
            finished.await;
        }
    }

    pub async fn disconnect(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.ready.store(false, Ordering::SeqCst);
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

struct ReceiveContext {
    inner: Arc<Inner>,
    stream: Stream,
    events: ProviderEventSender,
    setup: watch::Sender<SetupState>,
    source_language: SourceLanguage,
    generation: u64,
}

async fn receive_loop(mut context: ReceiveContext) {
    while let Some(message) = context.stream.next().await {
        if context.inner.generation.load(Ordering::SeqCst) != context.generation {
            return;
        }
        let event = match message {
            Ok(Message::Binary(frame)) => VolcanoEngineServerEvent::decode(&frame),
            Ok(Message::Text(frame)) => VolcanoEngineServerEvent::decode(frame.as_bytes()),
            Ok(Message::Pong(_)) => {
                context.inner.pong_notify.notify_waiters();
                continue;
            }
            Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => {
                if context.inner.is_closing.load(Ordering::SeqCst)
                    && context
                        .inner
                        .received_session_finished
                        .load(Ordering::SeqCst)
                {
                    return;
                }
                fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR);
                return;
            }
        };
        let event = match event {
            Ok(event) => event,
            Err(_) => {
                fail_receive_loop(&context, "volcano_protocol_error", GENERIC_PROTOCOL_ERROR);
                return;
            }
        };
        if handle_server_event(&context, event).await {
            return;
        }
    }

    if context.inner.generation.load(Ordering::SeqCst) == context.generation
        && !(context.inner.is_closing.load(Ordering::SeqCst)
            && context
                .inner
                .received_session_finished
                .load(Ordering::SeqCst))
    {
        fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR);
    }
}

fn fail_receive_loop(context: &ReceiveContext, code: &str, message: &str) {
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
async fn handle_server_event(context: &ReceiveContext, event: VolcanoEngineServerEvent) -> bool {
    let revision = context.events.content_revision();
    let _content = context.inner.content_lock.lock().await;
    if context.inner.generation.load(Ordering::SeqCst) != context.generation {
        return false;
    }
    if revision != context.events.content_revision()
        && matches!(
            &event,
            VolcanoEngineServerEvent::SourceSubtitleStarted
                | VolcanoEngineServerEvent::SourceSubtitleDraft(_)
                | VolcanoEngineServerEvent::SourceSubtitleFinal { .. }
                | VolcanoEngineServerEvent::TranslationSubtitleStarted
                | VolcanoEngineServerEvent::TranslationSubtitleDraft(_)
                | VolcanoEngineServerEvent::TranslationSubtitleFinal { .. }
        )
    {
        return false;
    }

    let setup_is_awaiting = *context.setup.borrow() == SetupState::Awaiting;
    if setup_is_awaiting
        && !matches!(
            &event,
            VolcanoEngineServerEvent::SessionStarted
                | VolcanoEngineServerEvent::SessionFailed { .. }
        )
    {
        let _ = context.setup.send(SetupState::Rejected);
        return true;
    }

    match event {
        VolcanoEngineServerEvent::SessionStarted => {
            if setup_is_awaiting {
                if !emit_if_current(context, LiveTranslateServerEvent::SessionCreated)
                    || !emit_if_current(context, LiveTranslateServerEvent::SessionUpdated)
                {
                    let _ = context.setup.send(SetupState::Rejected);
                    return true;
                }
                let _ = context.setup.send(SetupState::Ready);
            }
        }
        VolcanoEngineServerEvent::SourceSubtitleStarted => {
            let mut committer = context.inner.committer.lock().await;
            committer.source_draft.clear();
            committer.source_in_progress = true;
            committer.discard_source = false;
        }
        VolcanoEngineServerEvent::SourceSubtitleDraft(text) => {
            let mut committer = context.inner.committer.lock().await;
            committer.source_in_progress = true;
            if committer.discard_source {
                return false;
            }
            if committer.source_draft.len().saturating_add(text.len())
                > crate::core::models::MAX_SUBTITLE_TEXT_BYTES
            {
                committer.reset();
                emit_commit_result(context, Err(PendingFinalTextLimitExceeded));
                return true;
            }
            committer.source_draft.push_str(&text);
            let text = committer.source_draft.clone();
            if !emit_if_current(
                context,
                LiveTranslateServerEvent::SourceDraft {
                    text,
                    language: Some(context.source_language.raw_value().into()),
                },
            ) {
                return true;
            }
        }
        VolcanoEngineServerEvent::SourceSubtitleFinal {
            text,
            start_time_ms,
            end_time_ms,
        } => {
            let mut committer = context.inner.committer.lock().await;
            committer.source_in_progress = false;
            committer.source_draft.clear();
            if committer.discard_source {
                return false;
            }
            let commit = committer.push_source(text, start_time_ms, end_time_ms);
            if !emit_commit_result(context, commit) {
                return true;
            }
        }
        VolcanoEngineServerEvent::TranslationSubtitleStarted => {
            let mut committer = context.inner.committer.lock().await;
            committer.translation_draft.clear();
            committer.translation_in_progress = true;
            committer.discard_translation = false;
            if !emit_if_current(context, LiveTranslateServerEvent::TranslationStarted) {
                return true;
            }
        }
        VolcanoEngineServerEvent::TranslationSubtitleDraft(text) => {
            let mut committer = context.inner.committer.lock().await;
            committer.translation_in_progress = true;
            if committer.discard_translation {
                return false;
            }
            if committer.translation_draft.len().saturating_add(text.len())
                > crate::core::models::MAX_SUBTITLE_TEXT_BYTES
            {
                committer.reset();
                emit_commit_result(context, Err(PendingFinalTextLimitExceeded));
                return true;
            }
            committer.translation_draft.push_str(&text);
            let text = committer.translation_draft.clone();
            if !emit_if_current(context, LiveTranslateServerEvent::TranslationDraft(text)) {
                return true;
            }
        }
        VolcanoEngineServerEvent::TranslationSubtitleFinal {
            text,
            start_time_ms,
            end_time_ms,
        } => {
            let mut committer = context.inner.committer.lock().await;
            committer.translation_in_progress = false;
            committer.translation_draft.clear();
            if committer.discard_translation {
                return false;
            }
            let commit = committer.push_translation(text, start_time_ms, end_time_ms);
            if !emit_commit_result(context, commit) {
                return true;
            }
        }
        VolcanoEngineServerEvent::SessionFinished => {
            // An unmatched source or translation final is an incomplete tail,
            // not evidence that two different sentence intervals belong
            // together. Drop it before publishing the terminal event.
            context.inner.committer.lock().await.reset();
            if context.inner.is_closing.load(Ordering::SeqCst) {
                context
                    .inner
                    .received_session_finished
                    .store(true, Ordering::SeqCst);
                context.inner.session_finished_notify.notify_waiters();
                emit_if_current(context, LiveTranslateServerEvent::SessionFinished);
            } else {
                context.inner.ready.store(false, Ordering::SeqCst);
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: "volcano_unexpected_session_finished".into(),
                        message: UNEXPECTED_SESSION_FINISHED_ERROR.into(),
                    },
                );
            }
            return true;
        }
        VolcanoEngineServerEvent::SessionFailed { status_code } => {
            context.inner.committer.lock().await.reset();
            if setup_is_awaiting {
                let _ = context.setup.send(SetupState::Rejected);
            } else {
                let code = status_code
                    .map(|status| format!("volcano_provider_error.{status}"))
                    .unwrap_or_else(|| "volcano_provider_error".into());
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code,
                        message: GENERIC_PROVIDER_ERROR.into(),
                    },
                );
            }
            return true;
        }
        VolcanoEngineServerEvent::Ignored { event } => {
            if !emit_if_current(
                context,
                LiveTranslateServerEvent::Ignored {
                    kind: format!("volcanoEvent{event}"),
                },
            ) {
                return true;
            }
        }
    }
    false
}

fn emit_commit_result(context: &ReceiveContext, result: SubtitleCommitResult) -> bool {
    match result {
        Ok(Some(pair)) => emit_if_current(context, pair),
        Ok(None) => true,
        Err(PendingFinalTextLimitExceeded) => emit_if_current(
            context,
            LiveTranslateServerEvent::Error {
                code: "volcano_transcript_safety_limit".into(),
                message: TRANSCRIPT_SAFETY_LIMIT_ERROR.into(),
            },
        ),
    }
}

fn emit_if_current(context: &ReceiveContext, event: LiveTranslateServerEvent) -> bool {
    context.inner.generation.load(Ordering::SeqCst) == context.generation
        && context.events.send(event).is_ok()
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
            let mut client = VolcanoEngineClient::with_endpoint(
                "test-key-not-real",
                SourceLanguage::English,
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
                assert_eq!(error, VolcanoEngineClientError::AuthenticationFailed);
                assert_eq!(error.to_string(), expected_label);
            } else {
                assert_eq!(error, VolcanoEngineClientError::TransportFailure);
            }
            assert!(!error.to_string().contains("private-handshake-body"));
            assert!(!client.inner.ready.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn clear_removes_pending_text_but_tombstones_its_real_interval() {
        let (sender, mut receiver) = provider_event_channel();
        let client = VolcanoEngineClient::with_endpoint(
            "test-key-not-real",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender.clone(),
            url::Url::parse("ws://127.0.0.1:1/translate").unwrap(),
        )
        .unwrap();
        {
            let mut committer = client.inner.committer.lock().await;
            committer.source_in_progress = true;
            committer.source_draft = "old source preview".into();
            committer.translation_draft = "old translation preview".into();
            committer
                .push_source("old source".into(), Some(0), Some(10))
                .unwrap();
        }
        sender
            .send(LiveTranslateServerEvent::SessionFinished)
            .unwrap();
        assert_eq!(client.clear_content().await, 1);
        assert_eq!(client.content_revision(), 1);
        let mut committer = client.inner.committer.lock().await;
        assert!(committer.sources.is_empty() && committer.translations.is_empty());
        assert!(committer.source_draft.is_empty() && committer.translation_draft.is_empty());
        assert_eq!(committer.pending_text_bytes, 0);
        assert!(committer.discard_source && committer.discard_translation);
        assert!(committer
            .push_translation("late old".into(), Some(0), Some(10))
            .unwrap()
            .is_none());
        assert!(committer
            .push_source("old replay".into(), Some(0), Some(10))
            .unwrap()
            .is_none());
        committer.discard_source = false;
        committer.discard_translation = false;
        committer
            .push_source("new source".into(), Some(11), Some(20))
            .unwrap();
        let pair = committer
            .push_translation("new translation".into(), Some(11), Some(20))
            .unwrap();
        assert!(
            matches!(pair, Some(LiveTranslateServerEvent::SubtitleFinalPair {source,translation,..})
            if source == "new source" && translation == "new translation")
        );
        drop(committer);
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        );
    }

    use super::*;
    use crate::clients::provider_events::{provider_event_channel, ProviderEventReceiver};
    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpListener;

    const EVENT_START_SESSION: u32 = 100;
    const EVENT_FINISH_SESSION: u32 = 102;
    const EVENT_SESSION_STARTED: u32 = 150;
    const EVENT_SESSION_FINISHED: u32 = 152;
    const EVENT_SESSION_FAILED: u32 = 153;
    const EVENT_TASK_REQUEST: u32 = 200;
    const EVENT_SOURCE_SUBTITLE_START: u32 = 650;
    const EVENT_SOURCE_SUBTITLE_RESPONSE: u32 = 651;
    const EVENT_SOURCE_SUBTITLE_END: u32 = 652;
    const EVENT_TRANSLATION_SUBTITLE_START: u32 = 653;
    const EVENT_TRANSLATION_SUBTITLE_RESPONSE: u32 = 654;
    const EVENT_TRANSLATION_SUBTITLE_END: u32 = 655;

    async fn test_client(
        server: impl FnOnce(
                WebSocketStream<TcpStream>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + 'static,
    ) -> (VolcanoEngineClient, ProviderEventReceiver) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server(socket).await;
        });
        let (events, receiver) = provider_event_channel();
        let endpoint = url::Url::parse(&format!("ws://{address}/translate")).unwrap();
        let client = VolcanoEngineClient::with_endpoint(
            "volcano-test-key-not-real",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            events,
            endpoint,
        )
        .unwrap();
        (client, receiver)
    }

    fn write_varint(output: &mut Vec<u8>, mut value: u64) {
        while value >= 0x80 {
            output.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        output.push(value as u8);
    }

    fn write_varint_field(output: &mut Vec<u8>, field: u32, value: u64) {
        write_varint(output, u64::from(field) << 3);
        write_varint(output, value);
    }

    fn write_bytes_field(output: &mut Vec<u8>, field: u32, value: &[u8]) {
        write_varint(output, (u64::from(field) << 3) | 2);
        write_varint(output, value.len() as u64);
        output.extend_from_slice(value);
    }

    fn server_event(event: u64, text: Option<&str>) -> Vec<u8> {
        let mut message = Vec::new();
        write_varint_field(&mut message, 2, event);
        if let Some(text) = text {
            write_bytes_field(&mut message, 4, text.as_bytes());
        }
        message
    }

    fn timed_server_event(event: u64, text: &str, start_time_ms: u64, end_time_ms: u64) -> Vec<u8> {
        let mut message = server_event(event, Some(text));
        write_varint_field(&mut message, 5, start_time_ms);
        write_varint_field(&mut message, 6, end_time_ms);
        message
    }

    fn failed_server_event(status: u64, private_message: &str) -> Vec<u8> {
        let mut meta = Vec::new();
        write_varint_field(&mut meta, 3, status);
        write_bytes_field(&mut meta, 4, private_message.as_bytes());
        let mut message = Vec::new();
        write_bytes_field(&mut message, 1, &meta);
        write_varint_field(&mut message, 2, EVENT_SESSION_FAILED as u64);
        message
    }

    fn read_varint(input: &mut &[u8]) -> Option<u64> {
        let mut value = 0_u64;
        for shift in (0..70).step_by(7) {
            let byte = *input.first()?;
            *input = &input[1..];
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn find_varint(message: &[u8], wanted_field: u32) -> Option<u64> {
        let mut input = message;
        while !input.is_empty() {
            let key = read_varint(&mut input)?;
            let field = u32::try_from(key >> 3).ok()?;
            let wire = (key & 7) as u8;
            match wire {
                0 => {
                    let value = read_varint(&mut input)?;
                    if field == wanted_field {
                        return Some(value);
                    }
                }
                2 => {
                    let length = usize::try_from(read_varint(&mut input)?).ok()?;
                    if length > input.len() {
                        return None;
                    }
                    input = &input[length..];
                }
                _ => return None,
            }
        }
        None
    }

    fn find_bytes(message: &[u8], wanted_field: u32) -> Option<&[u8]> {
        let mut input = message;
        while !input.is_empty() {
            let key = read_varint(&mut input)?;
            let field = u32::try_from(key >> 3).ok()?;
            let wire = (key & 7) as u8;
            match wire {
                0 => {
                    read_varint(&mut input)?;
                }
                2 => {
                    let length = usize::try_from(read_varint(&mut input)?).ok()?;
                    if length > input.len() {
                        return None;
                    }
                    let (value, rest) = input.split_at(length);
                    input = rest;
                    if field == wanted_field {
                        return Some(value);
                    }
                }
                _ => return None,
            }
        }
        None
    }

    fn binary(message: Message) -> Vec<u8> {
        match message {
            Message::Binary(frame) => frame.to_vec(),
            other => panic!("expected binary protobuf frame, got {other:?}"),
        }
    }

    fn assert_pair(event: SubtitleCommitResult, expected_source: &str, expected_translation: &str) {
        assert!(matches!(
            event,
            Ok(Some(LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                language,
                translation,
            })) if source == expected_source
                && language.as_deref() == Some("en")
                && translation == expected_translation
        ));
    }

    #[test]
    fn final_pair_committer_accepts_either_final_arrival_order() {
        let mut source_first = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(source_first
            .push_source("one".into(), Some(10), Some(90))
            .unwrap()
            .is_none());
        assert_pair(
            source_first.push_translation("一".into(), Some(10), Some(90)),
            "one",
            "一",
        );

        let mut translation_first = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(translation_first
            .push_translation("二".into(), Some(100), Some(190))
            .unwrap()
            .is_none());
        assert_pair(
            translation_first.push_source("two".into(), Some(100), Some(190)),
            "two",
            "二",
        );
    }

    #[test]
    fn timing_identity_prevents_cross_sentence_pairing_during_reordering() {
        let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(committer
            .push_source("sentence A".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert!(committer
            .push_source("sentence B".into(), Some(100), Some(200))
            .unwrap()
            .is_none());

        // B completes first, followed by A. Each translation must select the
        // source with the same official input interval rather than the oldest
        // or most recently received source.
        assert_pair(
            committer.push_translation("译文 B".into(), Some(100), Some(200)),
            "sentence B",
            "译文 B",
        );
        assert_pair(
            committer.push_translation("译文 A".into(), Some(0), Some(100)),
            "sentence A",
            "译文 A",
        );
    }

    #[test]
    fn mismatched_or_incomplete_tails_are_never_forged_into_a_pair() {
        let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(committer
            .push_source("old source".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert!(committer
            .push_translation("new translation".into(), Some(100), Some(200))
            .unwrap()
            .is_none());
        assert!(committer
            .push_source("missing timing".into(), None, None)
            .unwrap()
            .is_none());

        committer.reset();
        assert!(committer.sources.is_empty());
        assert!(committer.translations.is_empty());
        assert!(committer
            .push_translation("late unmatched tail".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_committed_timing_cannot_emit_a_duplicate_pair() {
        let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(committer
            .push_source("once".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert_pair(
            committer.push_translation("一次".into(), Some(0), Some(100)),
            "once",
            "一次",
        );
        assert!(committer
            .push_source("duplicate source".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert!(committer
            .push_translation("重复译文".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert!(committer.sources.is_empty());
        assert!(committer.translations.is_empty());
    }

    #[test]
    fn pending_final_text_budget_resets_and_allows_the_next_pair() {
        let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        let oversized_half = MAXIMUM_PENDING_FINAL_TEXT_BYTES / 2 + 1;
        assert!(committer
            .push_source("s".repeat(oversized_half), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert_eq!(committer.pending_text_bytes, oversized_half);

        assert_eq!(
            committer.push_translation("t".repeat(oversized_half), Some(100), Some(200),),
            Err(PendingFinalTextLimitExceeded)
        );
        assert!(committer.sources.is_empty());
        assert!(committer.translations.is_empty());
        assert_eq!(committer.pending_text_bytes, 0);

        assert!(committer
            .push_source("next sentence".into(), Some(200), Some(300))
            .unwrap()
            .is_none());
        assert_pair(
            committer.push_translation("次の文".into(), Some(200), Some(300)),
            "next sentence",
            "次の文",
        );
        assert_eq!(committer.pending_text_bytes, 0);
    }

    #[test]
    fn constructor_validates_languages_without_exposing_the_key() {
        let secret = "volcano-private-key-never-log";
        let (events, _receiver) = provider_event_channel();
        let result = VolcanoEngineClient::with_endpoint(
            secret,
            SourceLanguage::Automatic,
            TargetLanguage::English,
            events,
            url::Url::parse("ws://127.0.0.1:9/translate").unwrap(),
        );
        let error = match result {
            Ok(_) => panic!("automatic source language must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error, VolcanoEngineClientError::UnsupportedSourceLanguage);
        assert!(!format!("{error:?}").contains(secret));
    }

    #[allow(clippy::result_large_err)]
    #[tokio::test]
    async fn websocket_upgrade_uses_the_current_api_key_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(
                        request.headers().get("X-Api-Key").unwrap(),
                        "volcano-current-api-key-not-real"
                    );
                    assert_eq!(
                        request.headers().get("X-Api-Resource-Id").unwrap(),
                        VolcanoEngineEndpoint::RESOURCE_ID
                    );
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let _ = socket.next().await;
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_STARTED as u64, None).into(),
                ))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        });

        let (events, _receiver) = provider_event_channel();
        let client = VolcanoEngineClient::with_endpoint(
            "volcano-current-api-key-not-real",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            events,
            url::Url::parse(&format!("ws://{address}/translate")).unwrap(),
        )
        .unwrap();
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn connect_waits_for_the_official_session_started_event() {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                let start = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&start, 2), Some(EVENT_START_SESSION as u64));
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
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
        client.disconnect().await;
    }

    #[tokio::test]
    async fn shared_incremental_subtitle_sequences_keep_full_previews_and_sentence_boundaries() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../../../translation-contracts.json")).unwrap();
        for case in contract["volcanoSubtitleSequences"].as_array().unwrap() {
            let frames = case["frames"]
                .as_array()
                .unwrap()
                .iter()
                .map(|hex| {
                    let hex = hex.as_str().unwrap();
                    (0..hex.len())
                        .step_by(2)
                        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = url::Url::parse(&format!(
                "ws://{}/translate",
                listener.local_addr().unwrap()
            ))
            .unwrap();
            let (connected, accepted) =
                tokio::join!(tokio_tungstenite::connect_async(endpoint.clone()), async {
                    let (stream, _) = listener.accept().await.unwrap();
                    tokio_tungstenite::accept_async(stream).await
                });
            let (socket, _) = connected.unwrap();
            let _server = accepted.unwrap();
            let (_, stream) = socket.split();
            let (sender, mut events) = provider_event_channel();
            let client = VolcanoEngineClient::with_endpoint(
                "test-key-not-real",
                SourceLanguage::Japanese,
                TargetLanguage::SimplifiedChinese,
                sender.clone(),
                endpoint,
            )
            .unwrap();
            let (setup, _) = watch::channel(SetupState::Ready);
            let context = ReceiveContext {
                inner: Arc::clone(&client.inner),
                stream,
                events: sender,
                setup,
                source_language: SourceLanguage::Japanese,
                generation: 0,
            };
            let mut source_drafts = Vec::new();
            let mut translation_drafts = Vec::new();
            let mut source_finals = Vec::new();
            let mut translation_finals = Vec::new();
            let expected = &case["expected"];
            // Drain after each real handler call. Draft transport intentionally
            // coalesces snapshots, so a burst socket test cannot assert every preview.
            for frame in frames {
                assert!(
                    !handle_server_event(
                        &context,
                        VolcanoEngineServerEvent::decode(&frame).unwrap()
                    )
                    .await
                );
                while let Ok(event) = events.try_recv() {
                    match event {
                        LiveTranslateServerEvent::SourceDraft { text, .. } => {
                            source_drafts.push(text)
                        }
                        LiveTranslateServerEvent::TranslationDraft(text) => {
                            translation_drafts.push(text)
                        }
                        LiveTranslateServerEvent::SubtitleFinalPair {
                            source,
                            translation,
                            ..
                        } => {
                            source_finals.push(source);
                            translation_finals.push(translation);
                        }
                        LiveTranslateServerEvent::Error { code, .. } => {
                            panic!("unexpected failure {code}")
                        }
                        _ => {}
                    }
                }
            }
            assert_eq!(
                serde_json::json!({
                    "sourceDrafts": source_drafts, "translationDrafts": translation_drafts,
                    "sourceFinals": source_finals, "translationFinals": translation_finals,
                }),
                *expected,
                "{}",
                case["id"]
            );
        }
    }

    #[tokio::test]
    async fn incremental_draft_overflow_is_bounded_on_both_sides() {
        for event in [
            EVENT_SOURCE_SUBTITLE_RESPONSE,
            EVENT_TRANSLATION_SUBTITLE_RESPONSE,
        ] {
            for character in ["a", "日"] {
                let (client, mut events) = test_client(move |mut socket| {
                    Box::pin(async move {
                        socket.next().await.unwrap().unwrap();
                        socket
                            .send(Message::Binary(
                                server_event(EVENT_SESSION_STARTED as u64, None).into(),
                            ))
                            .await
                            .unwrap();
                        socket.next().await.unwrap().unwrap();
                        for text in [
                            character.repeat(
                                crate::core::models::MAX_SUBTITLE_TEXT_BYTES / character.len(),
                            ),
                            character.into(),
                        ] {
                            socket
                                .send(Message::Binary(
                                    server_event(event as u64, Some(&text)).into(),
                                ))
                                .await
                                .unwrap();
                        }
                        while socket.next().await.is_some() {}
                    })
                })
                .await;
                client.connect().await.unwrap();
                client
                    .send_audio(&vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT])
                    .await
                    .unwrap();
                loop {
                    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                        .await
                        .unwrap()
                        .unwrap();
                    if let LiveTranslateServerEvent::Error { code, .. } = event {
                        assert_eq!(code, "volcano_transcript_safety_limit");
                        break;
                    }
                }
                client.disconnect().await;
            }
        }
    }

    #[tokio::test]
    async fn empty_subtitle_events_do_not_disconnect_or_block_the_next_final_pair() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                socket.next().await.unwrap().unwrap();
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
                // Send only after connect has completed, as with normal audio.
                socket.next().await.unwrap().unwrap();
                for event in [
                    EVENT_SOURCE_SUBTITLE_RESPONSE,
                    EVENT_SOURCE_SUBTITLE_END,
                    EVENT_TRANSLATION_SUBTITLE_RESPONSE,
                    EVENT_TRANSLATION_SUBTITLE_END,
                ] {
                    for text in [None, Some("")] {
                        socket
                            .send(Message::Binary(server_event(event as u64, text).into()))
                            .await
                            .unwrap();
                    }
                }
                for response in [
                    timed_server_event(
                        EVENT_SOURCE_SUBTITLE_END as u64,
                        "Synthetic source.",
                        100,
                        200,
                    ),
                    timed_server_event(
                        EVENT_TRANSLATION_SUBTITLE_END as u64,
                        "合成テスト。",
                        100,
                        200,
                    ),
                ] {
                    socket.send(Message::Binary(response.into())).await.unwrap();
                }
                let finish = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_FINISHED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
            })
        })
        .await;
        client.connect().await.unwrap();
        client
            .send_audio(&vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap()
                .unwrap();
            match event {
                LiveTranslateServerEvent::Error { code, .. } => panic!("unexpected failure {code}"),
                LiveTranslateServerEvent::SubtitleFinalPair {
                    source,
                    translation,
                    ..
                } => {
                    assert_eq!(source, "Synthetic source.");
                    assert_eq!(translation, "合成テスト。");
                    break;
                }
                _ => {}
            }
        }
        client.finish(Duration::from_secs(1)).await;
        let remaining = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
        assert!(remaining
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!remaining.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::Error { .. }
                | LiveTranslateServerEvent::SubtitleFinalPair { .. }
        )));
        client.disconnect().await;
    }

    #[tokio::test]
    async fn full_mock_lifecycle_frames_audio_and_emits_subtitles() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let start = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&start, 2), Some(EVENT_START_SESSION as u64));
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();

                let audio = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&audio, 2), Some(EVENT_TASK_REQUEST as u64));
                let source_audio = find_bytes(&audio, 4).unwrap();
                let pcm = find_bytes(source_audio, 14).unwrap();
                assert_eq!(pcm.len(), VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT);
                assert_eq!(&pcm[..3], &[1, 2, 3]);
                assert!(pcm[3..].iter().all(|byte| *byte == 0));

                for response in [
                    server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
                    server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Hello")),
                ] {
                    socket.send(Message::Binary(response.into())).await.unwrap();
                }

                let finish = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));
                for response in [
                    server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, None),
                    server_event(
                        EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64,
                        Some("こんにちは"),
                    ),
                    // Finals can arrive in either order. Sentence timing, not
                    // arrival order, is the pairing identity.
                    timed_server_event(
                        EVENT_TRANSLATION_SUBTITLE_END as u64,
                        "こんにちは。",
                        1_000,
                        1_800,
                    ),
                    timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Hello.", 1_000, 1_800),
                    server_event(EVENT_SESSION_FINISHED as u64, None),
                ] {
                    socket.send(Message::Binary(response.into())).await.unwrap();
                }
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        let mut pcm = vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT];
        pcm[..3].copy_from_slice(&[1, 2, 3]);
        client.send_audio(&pcm).await.unwrap();

        let mut received = Vec::new();
        loop {
            let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
                .await
                .unwrap()
                .unwrap();
            let saw_source_draft = matches!(
                &event,
                LiveTranslateServerEvent::SourceDraft { text, language }
                    if text == "Hello" && language.as_deref() == Some("en")
            );
            received.push(event);
            if saw_source_draft {
                break;
            }
        }

        client.finish(Duration::from_secs(1)).await;
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SourceDraft { text, language }
                if text == "Hello" && language.as_deref() == Some("en")
        )));
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleFinalPair { source, language, translation }
                if source == "Hello."
                    && language.as_deref() == Some("en")
                    && translation == "こんにちは。"
        )));
        let pair_index = received
            .iter()
            .position(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair { .. }))
            .unwrap();
        let finished_index = received
            .iter()
            .position(|event| matches!(event, LiveTranslateServerEvent::SessionFinished))
            .unwrap();
        assert!(pair_index < finished_index);
        assert!(!received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SourceFinal { .. }
                | LiveTranslateServerEvent::TranslationFinal(_)
        )));
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
    }

    #[tokio::test]
    async fn graceful_finish_drops_an_incomplete_confirmed_tail() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
                let finish = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));
                socket
                    .send(Message::Binary(
                        timed_server_event(
                            EVENT_SOURCE_SUBTITLE_END as u64,
                            "unmatched final source",
                            2_000,
                            2_800,
                        )
                        .into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_FINISHED as u64, None).into(),
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
        client.finish(Duration::from_secs(1)).await;

        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleFinalPair { .. }
                | LiveTranslateServerEvent::SourceFinal { .. }
                | LiveTranslateServerEvent::TranslationFinal(_)
        )));
    }

    #[tokio::test]
    async fn unsolicited_session_finished_is_a_safe_local_error() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();

                let audio = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&audio, 2), Some(EVENT_TASK_REQUEST as u64));
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_FINISHED as u64, None).into(),
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
        client
            .send_audio(&vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();

        let mut received = Vec::new();
        loop {
            let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
                .await
                .unwrap()
                .unwrap();
            let is_error = matches!(
                &event,
                LiveTranslateServerEvent::Error { code, message }
                    if code == "volcano_unexpected_session_finished"
                        && message == UNEXPECTED_SESSION_FINISHED_ERROR
            );
            received.push(event);
            if is_error {
                break;
            }
        }
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
        client.disconnect().await;
    }

    #[tokio::test]
    async fn oversized_unmatched_final_buffer_emits_safe_error_and_recovers() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
                let finish = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));

                let oversized_half = MAXIMUM_PENDING_FINAL_TEXT_BYTES / 2 + 1;
                for response in [
                    timed_server_event(
                        EVENT_SOURCE_SUBTITLE_END as u64,
                        &"private source".repeat(oversized_half / "private source".len() + 1),
                        0,
                        100,
                    ),
                    timed_server_event(
                        EVENT_TRANSLATION_SUBTITLE_END as u64,
                        &"private translation"
                            .repeat(oversized_half / "private translation".len() + 1),
                        100,
                        200,
                    ),
                    timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "next sentence", 200, 300),
                    timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "次の文", 200, 300),
                    server_event(EVENT_SESSION_FINISHED as u64, None),
                ] {
                    socket.send(Message::Binary(response.into())).await.unwrap();
                }
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.finish(Duration::from_secs(1)).await;

        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::Error { code, message }
                if code == "volcano_transcript_safety_limit"
                    && message == TRANSCRIPT_SAFETY_LIMIT_ERROR
        )));
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleFinalPair { source, language, translation }
                if source == "next sentence"
                    && language.as_deref() == Some("en")
                    && translation == "次の文"
        )));
        assert!(!received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SourceFinal { text, .. }
                if text.contains("private")
        ) || matches!(
            event,
            LiveTranslateServerEvent::TranslationFinal(translation)
                if translation.contains("private")
        )));
    }

    #[tokio::test]
    async fn provider_setup_failure_is_content_free() {
        let private_detail = "private transcript and volcano-secret-key";
        let (client, _events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        failed_server_event(45_000_001, private_detail).into(),
                    ))
                    .await
                    .unwrap();
            })
        })
        .await;
        let error = client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap_err();
        assert_eq!(error, VolcanoEngineClientError::SessionSetupRejected);
        assert!(!format!("{error:?}").contains(private_detail));
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
            VolcanoEngineClientError::SessionSetupTimedOut
        );
    }

    #[tokio::test]
    async fn an_immediate_pong_cannot_be_lost() {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
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

    #[test]
    fn audio_buffer_keeps_only_a_partial_tail() {
        let frame_size = VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT;
        let mut pending = vec![0x22; frame_size * 2 + 17];
        let messages = take_complete_audio_messages("session-buffer", &mut pending).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(pending, vec![0x22; 17]);
        for message in messages {
            assert_eq!(find_varint(&message, 2), Some(EVENT_TASK_REQUEST as u64));
            assert_eq!(
                find_bytes(find_bytes(&message, 4).unwrap(), 14)
                    .unwrap()
                    .len(),
                frame_size
            );
        }
    }

    #[test]
    fn failed_audio_encoding_still_consumes_the_complete_batch() {
        let frame_size = VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT;
        let mut pending = vec![0x22; frame_size * 2 + 17];

        assert_eq!(
            take_complete_audio_messages("", &mut pending),
            Err(VolcanoEngineClientError::TransportFailure)
        );
        assert_eq!(pending, vec![0x22; 17]);
    }

    #[test]
    fn final_partial_audio_frame_is_zero_padded() {
        let mut pending = vec![1, 2, 3];
        let message = take_padded_audio_message("session-tail", &mut pending)
            .unwrap()
            .unwrap();
        let pcm = find_bytes(find_bytes(&message, 4).unwrap(), 14).unwrap();
        assert_eq!(pcm.len(), VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT);
        assert_eq!(&pcm[..3], &[1, 2, 3]);
        assert!(pcm[3..].iter().all(|byte| *byte == 0));
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn malformed_response_after_setup_is_a_content_free_protocol_error() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Binary(vec![0x10, 0x80].into()))
                    .await
                    .unwrap();
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::Error { code, message }
                if code == "volcano_protocol_error" && message == GENERIC_PROTOCOL_ERROR
        )));
        client.disconnect().await;
    }
}
