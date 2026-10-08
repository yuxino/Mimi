//! xAI Grok Voice WebSocket adapter for turn-based translation.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::models::TargetLanguage;
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::xai_realtime::{
    XAIRealtimeEndpoint, XAIRealtimeRequestEncoder, XAIRealtimeServerEvent,
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

const GENERIC_PROVIDER_ERROR: &str = "xAI Grok Voice rejected the session.";
const GENERIC_PROTOCOL_ERROR: &str = "xAI Grok Voice returned an invalid response.";
const GENERIC_TRANSPORT_ERROR: &str = "The xAI Grok Voice connection failed.";
const GENERIC_FINISH_TIMEOUT_ERROR: &str = "xAI Grok Voice did not finish the final turn in time.";
const GENERIC_RESPONSE_FAILED_ERROR: &str = "xAI Grok Voice did not complete the current turn.";
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const MAXIMUM_TRANSCRIPT_BYTES: usize = 128 * 1_024;
const SERVER_VAD_TAIL_FRAME_COUNT: usize = (XAIRealtimeEndpoint::SERVER_VAD_SILENCE_DURATION_MS
    as usize)
    .div_ceil(XAIRealtimeEndpoint::FRAME_DURATION_MS as usize);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum XAIRealtimeClientError {
    #[error("credential_authentication_failed")]
    AuthenticationFailed,
    #[error("Add an xAI API key in Settings.")]
    MissingAPIKey,
    #[error("xAI Grok Voice requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("The xAI Grok Voice session is not connected.")]
    NotConnected,
    #[error("The xAI Grok Voice connection stopped responding.")]
    HealthCheckTimedOut,
    #[error("The xAI Grok Voice connection failed.")]
    TransportFailure,
    #[error("xAI Grok Voice rejected the session configuration.")]
    SessionSetupRejected,
    #[error("xAI Grok Voice did not confirm the session configuration in time.")]
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
struct GrokTurnState {
    item_id: Option<String>,
    source: String,
    source_language: Option<String>,
    source_complete: bool,
    response_id: Option<String>,
    translation: String,
    translation_complete: bool,
    discard_current_turn: bool,
    discarded_item_id: Option<String>,
    pending_next_source: Option<PendingGrokSource>,
}

struct PendingGrokSource {
    item_id: String,
    transcript: String,
    language: Option<String>,
    completed: bool,
}

impl GrokTurnState {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn clear_content(&mut self) {
        let discard = self.discard_current_turn
            || self.item_id.is_some()
            || self.response_id.is_some()
            || !self.source.is_empty()
            || !self.translation.is_empty();
        let item_id = self
            .item_id
            .clone()
            .or_else(|| self.discarded_item_id.clone());
        self.reset();
        self.discard_current_turn = discard;
        self.discarded_item_id = item_id;
    }

    fn update_source(
        &mut self,
        transcript: String,
        item_id: Option<String>,
        language: Option<String>,
        completed: bool,
    ) -> Vec<LiveTranslateServerEvent> {
        if self.discard_current_turn {
            return self.update_pending_next_source(transcript, item_id, language, completed);
        }
        if identifiers_conflict(self.item_id.as_deref(), item_id.as_deref()) {
            // Grok turns are sequential. A new item means an incomplete old
            // pair can no longer be aligned safely, so discard it.
            self.reset();
        }
        if item_id.is_some() {
            self.item_id = item_id;
        }
        self.source = transcript;
        if language.is_some() {
            self.source_language = language;
        }
        self.source_complete |= completed;

        if self.exceeded_safety_limit() {
            return self.safety_limit_error();
        }

        let mut events = if self.source.is_empty() {
            Vec::new()
        } else {
            vec![LiveTranslateServerEvent::SourceDraft {
                text: self.source.clone(),
                language: self.source_language.clone(),
            }]
        };
        if let Some(pair) = self.take_pair_if_complete() {
            events.push(pair);
        }
        events
    }

    fn response_started(&mut self, response_id: Option<String>) {
        if self.discard_current_turn {
            return;
        }
        if identifiers_conflict(self.response_id.as_deref(), response_id.as_deref()) {
            self.translation.clear();
            self.translation_complete = false;
        }
        if response_id.is_some() {
            self.response_id = response_id;
        }
    }

    fn append_translation(
        &mut self,
        delta: String,
        response_id: Option<String>,
    ) -> Vec<LiveTranslateServerEvent> {
        if self.discard_current_turn {
            return Vec::new();
        }
        self.response_started(response_id);
        if delta.is_empty() {
            return Vec::new();
        }
        self.translation.push_str(&delta);
        if self.exceeded_safety_limit() {
            return self.safety_limit_error();
        }
        vec![LiveTranslateServerEvent::TranslationDraft(
            self.translation.clone(),
        )]
    }

    fn complete_translation(
        &mut self,
        final_transcript: Option<String>,
        response_id: Option<String>,
    ) -> Vec<LiveTranslateServerEvent> {
        if self.discard_current_turn {
            return Vec::new();
        }
        self.response_started(response_id);
        let mut events = Vec::new();
        if let Some(final_transcript) = final_transcript.filter(|text| !text.is_empty()) {
            if self.translation != final_transcript {
                self.translation = final_transcript;
                if self.exceeded_safety_limit() {
                    return self.safety_limit_error();
                }
                events.push(LiveTranslateServerEvent::TranslationDraft(
                    self.translation.clone(),
                ));
            }
        }
        self.translation_complete = true;
        if let Some(pair) = self.take_pair_if_complete() {
            events.push(pair);
        }
        events
    }

    fn response_done(&mut self, response_id: Option<String>) -> Vec<LiveTranslateServerEvent> {
        if self.discard_current_turn {
            // A new source item can arrive before the discarded response has
            // finished. Promote that source only after the provider's
            // definitive response boundary so late translation events from the
            // oversized turn can never be paired with it.
            let pending_next_source = self.pending_next_source.take();
            self.reset();
            if let Some(pending) = pending_next_source {
                self.item_id = Some(pending.item_id);
                self.source = pending.transcript;
                self.source_language = pending.language;
                self.source_complete = pending.completed;
            }
            return Vec::new();
        }
        self.response_started(response_id);
        self.translation_complete = true;
        self.take_pair_if_complete().into_iter().collect()
    }

    fn take_pair_if_complete(&mut self) -> Option<LiveTranslateServerEvent> {
        if !self.source_complete
            || !self.translation_complete
            || !is_meaningful(&self.source)
            || !is_meaningful(&self.translation)
        {
            return None;
        }
        let event = LiveTranslateServerEvent::SubtitleFinalPair {
            source: self.source.trim().to_string(),
            language: self.source_language.clone(),
            translation: self.translation.trim().to_string(),
        };
        self.reset();
        Some(event)
    }

    fn exceeded_safety_limit(&self) -> bool {
        self.source.len().saturating_add(self.translation.len()) > MAXIMUM_TRANSCRIPT_BYTES
    }

    fn awaits_response_for_promoted_source(&self) -> bool {
        self.item_id.is_some()
            && self.source_complete
            && is_meaningful(&self.source)
            && self.response_id.is_none()
            && self.translation.is_empty()
    }

    fn safety_limit_error(&mut self) -> Vec<LiveTranslateServerEvent> {
        let discarded_item_id = self.item_id.clone();
        self.reset();
        self.discard_current_turn = true;
        self.discarded_item_id = discarded_item_id;
        vec![LiveTranslateServerEvent::Error {
            code: "xai_transcript_safety_limit".into(),
            message: "xAI Grok Voice transcript buffering exceeded its safety limit.".into(),
        }]
    }

    fn update_pending_next_source(
        &mut self,
        transcript: String,
        item_id: Option<String>,
        language: Option<String>,
        completed: bool,
    ) -> Vec<LiveTranslateServerEvent> {
        let Some(item_id) = item_id else {
            return Vec::new();
        };
        let Some(discarded_item_id) = self.discarded_item_id.as_deref() else {
            return Vec::new();
        };
        if item_id == discarded_item_id {
            return Vec::new();
        }
        if transcript.len() > MAXIMUM_TRANSCRIPT_BYTES {
            self.pending_next_source = None;
            return vec![LiveTranslateServerEvent::Error {
                code: "xai_transcript_safety_limit".into(),
                message: "xAI Grok Voice transcript buffering exceeded its safety limit.".into(),
            }];
        }

        match self.pending_next_source.as_mut() {
            Some(pending) if pending.item_id == item_id => {
                pending.transcript = transcript;
                if language.is_some() {
                    pending.language = language;
                }
                pending.completed |= completed;
            }
            _ => {
                self.pending_next_source = Some(PendingGrokSource {
                    item_id,
                    transcript,
                    language,
                    completed,
                });
            }
        }

        self.pending_next_source
            .as_ref()
            .filter(|pending| !pending.transcript.is_empty())
            .map(|pending| {
                vec![LiveTranslateServerEvent::SourceDraft {
                    text: pending.transcript.clone(),
                    language: pending.language.clone(),
                }]
            })
            .unwrap_or_default()
    }
}

fn identifiers_conflict(current: Option<&str>, incoming: Option<&str>) -> bool {
    matches!((current, incoming), (Some(current), Some(incoming)) if current != incoming)
}

fn is_meaningful(text: &str) -> bool {
    text.chars()
        .any(|character| !character.is_whitespace() && character.is_alphanumeric())
}

struct Inner {
    // Serializes the content barrier with local assembly, never socket/audio state.
    content_lock: Mutex<()>,
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    audio_send_lock: Mutex<()>,
    pending_audio: Mutex<Vec<u8>>,
    turn: Mutex<GrokTurnState>,
    ready: AtomicBool,
    is_closing: AtomicBool,
    has_unfinished_turn: AtomicBool,
    last_response_failed: AtomicBool,
    finish_transport_failed: AtomicBool,
    response_done_notify: Notify,
    pong_notify: Notify,
    generation: AtomicU64,
}

#[derive(Clone)]
pub struct XAIRealtimeClient {
    network: super::provider_network::ProviderNetwork,
    inner: Arc<Inner>,
    endpoint: url::Url,
    api_key: String,
    source_hint: crate::core::models::SourceLanguage,
    target_language: TargetLanguage,
    events: ProviderEventSender,
}

impl XAIRealtimeClient {
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.clone()
    }

    pub fn with_source_language(mut self, source: crate::core::models::SourceLanguage) -> Self {
        self.source_hint = source;
        self
    }

    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    pub async fn clear_content(&self) -> u64 {
        let _content = self.inner.content_lock.lock().await;
        self.inner.turn.lock().await.clear_content();
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
    ) -> Result<Self, XAIRealtimeClientError> {
        let endpoint =
            XAIRealtimeEndpoint::url().map_err(|_| XAIRealtimeClientError::TransportFailure)?;
        Self::with_endpoint(api_key, target_language, events, endpoint)
    }

    fn with_endpoint(
        api_key: &str,
        target_language: TargetLanguage,
        events: ProviderEventSender,
        endpoint: url::Url,
    ) -> Result<Self, XAIRealtimeClientError> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(XAIRealtimeClientError::MissingAPIKey);
        }
        if !target_language.translates_audio() {
            return Err(XAIRealtimeClientError::InvalidTargetLanguage);
        }
        Ok(Self {
            network: super::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                content_lock: Mutex::new(()),
                sink: Mutex::new(None),
                receive_task: Mutex::new(None),
                audio_send_lock: Mutex::new(()),
                pending_audio: Mutex::new(Vec::new()),
                turn: Mutex::new(GrokTurnState::default()),
                ready: AtomicBool::new(false),
                is_closing: AtomicBool::new(false),
                has_unfinished_turn: AtomicBool::new(false),
                last_response_failed: AtomicBool::new(false),
                finish_transport_failed: AtomicBool::new(false),
                response_done_notify: Notify::new(),
                pong_notify: Notify::new(),
                generation: AtomicU64::new(0),
            }),
            endpoint,
            api_key: api_key.to_string(),
            source_hint: crate::core::models::SourceLanguage::Automatic,
            target_language,
            events,
        })
    }

    pub async fn connect(&self) -> Result<(), XAIRealtimeClientError> {
        self.connect_with_timeout(Duration::from_secs(5)).await
    }

    async fn connect_with_timeout(
        &self,
        readiness_timeout: Duration,
    ) -> Result<(), XAIRealtimeClientError> {
        self.disconnect().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let setup_event_id = format!("mimi-xai-session-update-{generation}");

        let mut request = self
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|_| XAIRealtimeClientError::TransportFailure)?;
        let authorization = format!("Bearer {}", self.api_key);
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&authorization)
                .map_err(|_| XAIRealtimeClientError::MissingAPIKey)?,
        );

        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            super::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| XAIRealtimeClientError::TransportFailure)?
        .map_err(|error| {
            if super::connection_diagnostics::authentication_rejected(&error) {
                XAIRealtimeClientError::AuthenticationFailed
            } else {
                XAIRealtimeClientError::TransportFailure
            }
        })?;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .has_unfinished_turn
            .store(false, Ordering::SeqCst);
        self.inner
            .last_response_failed
            .store(false, Ordering::SeqCst);
        self.inner
            .finish_transport_failed
            .store(false, Ordering::SeqCst);
        self.inner.pending_audio.lock().await.clear();
        self.inner.turn.lock().await.reset();

        let (setup_tx, mut setup_rx) = watch::channel(SetupState::Awaiting);
        let task = tokio::spawn(receive_loop(ReceiveContext {
            inner: Arc::clone(&self.inner),
            stream,
            events: self.events.clone(),
            setup_event_id: setup_event_id.clone(),
            setup: setup_tx,
            generation,
        }));
        *self.inner.receive_task.lock().await = Some(task);

        let update = XAIRealtimeRequestEncoder::session_update(
            self.source_hint,
            self.target_language,
            Some(&setup_event_id),
        )
        .map_err(|_| XAIRealtimeClientError::InvalidTargetLanguage)?;
        let complete_setup = async {
            self.send_text(update.to_string()).await?;
            loop {
                match setup_rx.borrow().clone() {
                    SetupState::Ready => return Ok(()),
                    SetupState::Rejected => {
                        return Err(XAIRealtimeClientError::SessionSetupRejected)
                    }
                    SetupState::Awaiting => {}
                }
                if setup_rx.changed().await.is_err() {
                    return Err(XAIRealtimeClientError::SessionSetupRejected);
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
                Err(XAIRealtimeClientError::SessionSetupTimedOut)
            }
        }
    }

    /// Accepts arbitrary PCM chunks and sends only exact 200 ms frames.
    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), XAIRealtimeClientError> {
        if pcm_data.is_empty() {
            return Ok(());
        }
        tokio::time::timeout(SEND_TIMEOUT, self.send_audio_operation(pcm_data))
            .await
            .map_err(|_| XAIRealtimeClientError::TransportFailure)?
    }

    async fn send_audio_operation(&self, pcm_data: &[u8]) -> Result<(), XAIRealtimeClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(XAIRealtimeClientError::NotConnected);
        }
        let _guard = self.inner.audio_send_lock.lock().await;
        let messages = {
            let mut pending = self.inner.pending_audio.lock().await;
            pending.extend_from_slice(pcm_data);
            take_complete_audio_messages(&mut pending)?
        };
        if !messages.is_empty() {
            self.inner.has_unfinished_turn.store(true, Ordering::SeqCst);
        }
        for message in messages {
            self.send_text(message).await?;
        }
        Ok(())
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), XAIRealtimeClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(XAIRealtimeClientError::NotConnected);
        }
        let operation = async {
            let pong = self.inner.pong_notify.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(XAIRealtimeClientError::NotConnected);
                };
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| XAIRealtimeClientError::TransportFailure)?;
            }
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| XAIRealtimeClientError::HealthCheckTimedOut)?
    }

    /// In server-VAD mode xAI explicitly disallows
    /// `input_audio_buffer.commit`. Drain the local frame buffer, append enough
    /// silence to trigger the configured VAD boundary, then wait a bounded
    /// amount of time for the current turn's `response.done` before closing.
    pub async fn finish(&self, timeout: Duration) {
        if !self.inner.ready.load(Ordering::SeqCst)
            || self.inner.is_closing.swap(true, Ordering::SeqCst)
        {
            return;
        }
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let finished_cleanly =
            match tokio::time::timeout(timeout, self.finish_operation(generation)).await {
                Ok(Ok(finished_cleanly)) => finished_cleanly,
                Ok(Err(_)) if self.is_current_generation(generation) => {
                    self.emit(
                        LiveTranslateServerEvent::Error {
                            code: "xai_session_finish_failed".into(),
                            message: GENERIC_TRANSPORT_ERROR.into(),
                        },
                        generation,
                    );
                    false
                }
                Err(_) if self.is_current_generation(generation) => {
                    self.emit(
                        LiveTranslateServerEvent::Error {
                            code: "xai_session_finish_timeout".into(),
                            message: GENERIC_FINISH_TIMEOUT_ERROR.into(),
                        },
                        generation,
                    );
                    false
                }
                _ => false,
            };
        if self.is_current_generation(generation) {
            self.inner.turn.lock().await.reset();
            if finished_cleanly {
                self.emit(LiveTranslateServerEvent::SessionFinished, generation);
            }
        }
        self.disconnect_if_current(generation).await;
    }

    async fn finish_operation(&self, generation: u64) -> Result<bool, XAIRealtimeClientError> {
        let _guard = self.inner.audio_send_lock.lock().await;
        if !self.is_current_generation(generation) {
            return Err(XAIRealtimeClientError::NotConnected);
        }
        let partial = {
            let mut pending = self.inner.pending_audio.lock().await;
            take_padded_audio_message(&mut pending)?
        };
        if let Some(message) = partial {
            self.inner.has_unfinished_turn.store(true, Ordering::SeqCst);
            self.send_text(message).await?;
        }
        let silence = XAIRealtimeRequestEncoder::audio_append(&vec![
            0;
            XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT
        ])
        .map_err(|_| XAIRealtimeClientError::TransportFailure)?
        .to_string();
        for _ in 0..SERVER_VAD_TAIL_FRAME_COUNT {
            self.send_text(silence.clone()).await?;
        }

        while self.is_current_generation(generation)
            && self.inner.has_unfinished_turn.load(Ordering::SeqCst)
        {
            let done = self.inner.response_done_notify.notified();
            tokio::pin!(done);
            done.as_mut().enable();
            if !self.inner.has_unfinished_turn.load(Ordering::SeqCst) {
                break;
            }
            done.await;
        }
        if self.inner.finish_transport_failed.load(Ordering::SeqCst) {
            return Err(XAIRealtimeClientError::TransportFailure);
        }
        Ok(!self.inner.last_response_failed.load(Ordering::SeqCst))
    }

    pub async fn disconnect(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .has_unfinished_turn
            .store(false, Ordering::SeqCst);
        self.inner
            .last_response_failed
            .store(false, Ordering::SeqCst);
        self.inner
            .finish_transport_failed
            .store(false, Ordering::SeqCst);
        if let Some(task) = self.inner.receive_task.lock().await.take() {
            task.abort();
        }
        if let Some(mut sink) = self.inner.sink.lock().await.take() {
            let _ = tokio::time::timeout(Duration::from_millis(250), sink.close()).await;
        }
        self.inner.pending_audio.lock().await.clear();
        self.inner.turn.lock().await.reset();
        self.inner.response_done_notify.notify_waiters();
    }

    async fn send_text(&self, text: String) -> Result<(), XAIRealtimeClientError> {
        let mut sink = self.inner.sink.lock().await;
        let Some(sink) = sink.as_mut() else {
            return Err(XAIRealtimeClientError::NotConnected);
        };
        let evidence =
            crate::development_audio::begin_json(&text, XAIRealtimeEndpoint::SAMPLE_RATE_HZ);
        tokio::time::timeout(
            SEND_TIMEOUT,
            evidence.observe(sink.send(Message::Text(text.into()))),
        )
        .await
        .map_err(|_| XAIRealtimeClientError::TransportFailure)?
        .map_err(|_| XAIRealtimeClientError::TransportFailure)
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
) -> Result<Vec<String>, XAIRealtimeClientError> {
    let frame_size = XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let complete_bytes = pending.len() / frame_size * frame_size;
    let result = pending[..complete_bytes]
        .chunks_exact(frame_size)
        .map(|frame| {
            XAIRealtimeRequestEncoder::audio_append(frame)
                .map(|value| value.to_string())
                .map_err(|_| XAIRealtimeClientError::TransportFailure)
        })
        .collect::<Result<Vec<_>, _>>();
    // Preserve the existing failure semantics: a complete batch is consumed
    // even if encoding one of its frames fails.
    pending.drain(..complete_bytes);
    result
}

fn take_padded_audio_message(
    pending: &mut Vec<u8>,
) -> Result<Option<String>, XAIRealtimeClientError> {
    if pending.is_empty() {
        return Ok(None);
    }
    let mut frame = std::mem::take(pending);
    frame.resize(XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT, 0);
    XAIRealtimeRequestEncoder::audio_append(&frame)
        .map(|value| Some(value.to_string()))
        .map_err(|_| XAIRealtimeClientError::TransportFailure)
}

struct ReceiveContext {
    inner: Arc<Inner>,
    stream: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    events: ProviderEventSender,
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
            Ok(Message::Text(text)) => XAIRealtimeServerEvent::decode(&text),
            // JSON transport makes binary frames output audio only. mimi does
            // not play provider output audio, so discard them without parsing
            // or retaining their contents.
            Ok(Message::Binary(_)) => continue,
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
                fail_receive_loop(&context, "xai_protocol_error", GENERIC_PROTOCOL_ERROR).await;
                return;
            }
        };
        if handle_server_event(&context, event).await {
            return;
        }
    }
    if context.inner.generation.load(Ordering::SeqCst) == context.generation
        && !context.inner.is_closing.load(Ordering::SeqCst)
    {
        fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR).await;
    }
}

async fn fail_receive_loop(context: &ReceiveContext, code: &str, message: &str) {
    if context.inner.is_closing.load(Ordering::SeqCst) {
        context
            .inner
            .finish_transport_failed
            .store(true, Ordering::SeqCst);
    }
    context
        .inner
        .has_unfinished_turn
        .store(false, Ordering::SeqCst);
    context.inner.response_done_notify.notify_waiters();
    if *context.setup.borrow() == SetupState::Awaiting {
        let _ = context.setup.send(SetupState::Rejected);
    } else if !context.inner.is_closing.load(Ordering::SeqCst) {
        emit_if_current(
            context,
            LiveTranslateServerEvent::Error {
                code: code.into(),
                message: message.into(),
            },
        );
    }
}

async fn handle_server_event(context: &ReceiveContext, event: XAIRealtimeServerEvent) -> bool {
    let revision = context.events.content_revision();
    let _content = context.inner.content_lock.lock().await;
    if context.inner.generation.load(Ordering::SeqCst) != context.generation {
        return false;
    }
    if revision != context.events.content_revision()
        && matches!(
            &event,
            XAIRealtimeServerEvent::SourceTranscriptUpdated { .. }
                | XAIRealtimeServerEvent::SourceTranscriptCompleted { .. }
                | XAIRealtimeServerEvent::ResponseStarted { .. }
                | XAIRealtimeServerEvent::TranslationDelta { .. }
                | XAIRealtimeServerEvent::TranslationDone { .. }
        )
    {
        return false;
    }

    match event {
        XAIRealtimeServerEvent::SessionCreated => {
            emit_if_current(context, LiveTranslateServerEvent::SessionCreated);
        }
        XAIRealtimeServerEvent::SessionUpdated {
            input_format,
            input_rate,
            transcription_model,
            turn_detection,
            reasoning_effort,
        } => {
            let valid = input_format == "audio/pcm"
                && input_rate == XAIRealtimeEndpoint::SAMPLE_RATE_HZ
                && transcription_model == XAIRealtimeEndpoint::TRANSCRIPTION_MODEL
                && turn_detection == "server_vad"
                && reasoning_effort == "none";
            if !valid {
                let _ = context.setup.send(SetupState::Rejected);
                return true;
            }
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Ready);
            } else {
                emit_if_current(context, LiveTranslateServerEvent::SessionUpdated);
            }
        }
        XAIRealtimeServerEvent::SourceTranscriptUpdated {
            transcript,
            item_id,
            language,
        } => {
            let events = context
                .inner
                .turn
                .lock()
                .await
                .update_source(transcript, item_id, language, false);
            emit_all_if_current(context, events);
        }
        XAIRealtimeServerEvent::SourceTranscriptCompleted {
            transcript,
            item_id,
            language,
        } => {
            let events = context
                .inner
                .turn
                .lock()
                .await
                .update_source(transcript, item_id, language, true);
            emit_all_if_current(context, events);
        }
        XAIRealtimeServerEvent::ResponseStarted { response_id } => {
            context
                .inner
                .last_response_failed
                .store(false, Ordering::SeqCst);
            let mut turn = context.inner.turn.lock().await;
            turn.response_started(response_id);
            if !turn.discard_current_turn {
                emit_if_current(context, LiveTranslateServerEvent::TranslationStarted);
            }
        }
        XAIRealtimeServerEvent::TranslationDelta { delta, response_id } => {
            let events = context
                .inner
                .turn
                .lock()
                .await
                .append_translation(delta, response_id);
            emit_all_if_current(context, events);
        }
        XAIRealtimeServerEvent::TranslationDone {
            transcript,
            response_id,
        } => {
            let events = context
                .inner
                .turn
                .lock()
                .await
                .complete_translation(transcript, response_id);
            emit_all_if_current(context, events);
        }
        XAIRealtimeServerEvent::OutputAudioDelta => {}
        XAIRealtimeServerEvent::ResponseDone {
            response_id,
            status,
        } => {
            let successful = status.as_deref().is_none_or(|status| status == "completed");
            if successful {
                let (events, has_followup_turn) = {
                    let mut turn = context.inner.turn.lock().await;
                    let events = turn.response_done(response_id);
                    let has_followup_turn = turn.awaits_response_for_promoted_source();
                    (events, has_followup_turn)
                };
                emit_all_if_current(context, events);
                context
                    .inner
                    .last_response_failed
                    .store(false, Ordering::SeqCst);
                context
                    .inner
                    .has_unfinished_turn
                    .store(has_followup_turn, Ordering::SeqCst);
            } else {
                context.inner.turn.lock().await.reset();
                context
                    .inner
                    .last_response_failed
                    .store(true, Ordering::SeqCst);
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: "xai_response_failed".into(),
                        message: GENERIC_RESPONSE_FAILED_ERROR.into(),
                    },
                );
                context
                    .inner
                    .has_unfinished_turn
                    .store(false, Ordering::SeqCst);
            }
            context.inner.response_done_notify.notify_waiters();
        }
        XAIRealtimeServerEvent::ProviderError {
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
                        code: format!("xai_provider_error.{code}"),
                        message: GENERIC_PROVIDER_ERROR.into(),
                    },
                );
                return true;
            }
        }
        XAIRealtimeServerEvent::Ignored { kind } => {
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
            let mut client = XAIRealtimeClient::with_endpoint(
                "test-key-not-real",
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
                assert_eq!(error, XAIRealtimeClientError::AuthenticationFailed);
                assert_eq!(error.to_string(), expected_label);
            } else {
                assert_eq!(error, XAIRealtimeClientError::TransportFailure);
            }
            assert!(!error.to_string().contains("private-handshake-body"));
            assert!(!client.inner.ready.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn clear_waits_for_real_response_boundary_then_accepts_the_next_item() {
        let (sender, mut receiver) = provider_event_channel();
        let endpoint = url::Url::parse("ws://127.0.0.1:1/realtime").unwrap();
        let client = XAIRealtimeClient::with_endpoint(
            "test-key-not-real",
            TargetLanguage::Japanese,
            sender.clone(),
            endpoint,
        )
        .unwrap();
        {
            let mut turn = client.inner.turn.lock().await;
            turn.update_source("old source".into(), Some("old-item".into()), None, true);
            turn.response_started(Some("old-response".into()));
            turn.append_translation("old translation".into(), Some("old-response".into()));
        }
        sender
            .send(LiveTranslateServerEvent::Error {
                code: "transport_error".into(),
                message: GENERIC_TRANSPORT_ERROR.into(),
            })
            .unwrap();
        assert_eq!(client.clear_content().await, 1);
        assert_eq!(client.content_revision(), 1);
        let mut turn = client.inner.turn.lock().await;
        assert!(turn.source.is_empty() && turn.translation.is_empty());
        assert!(turn
            .append_translation("late old".into(), Some("old-response".into()))
            .is_empty());
        assert!(turn
            .complete_translation(Some("late old final".into()), Some("old-response".into()))
            .is_empty());
        turn.update_source("new source".into(), Some("new-item".into()), None, true);
        assert!(turn.response_done(Some("old-response".into())).is_empty());
        assert!(!turn.discard_current_turn);
        turn.response_started(Some("new-response".into()));
        let events =
            turn.complete_translation(Some("new translation".into()), Some("new-response".into()));
        assert!(events.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleFinalPair {source,translation,..}
            if source == "new source" && translation == "new translation")));
        drop(turn);
        assert!(matches!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::Error { .. })
        ));
    }

    use super::*;
    use crate::clients::provider_events::{provider_event_channel, ProviderEventReceiver};
    use base64::Engine;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::Value;
    use std::sync::atomic::AtomicBool;
    use tokio::net::TcpListener;

    async fn test_client(
        server: impl FnOnce(
                WebSocketStream<TcpStream>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + 'static,
    ) -> (XAIRealtimeClient, ProviderEventReceiver) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server(socket).await;
        });
        let (events, receiver) = provider_event_channel();
        let endpoint = url::Url::parse(&format!("ws://{address}/realtime")).unwrap();
        let client = XAIRealtimeClient::with_endpoint(
            "xai-test-key-not-real",
            TargetLanguage::Japanese,
            events,
            endpoint,
        )
        .unwrap();
        (client, receiver)
    }

    fn setup_ack() -> Message {
        Message::Text(
            r#"{"type":"session.updated","session":{"reasoning":{"effort":"none"},"turn_detection":{"type":"server_vad"},"audio":{"input":{"format":{"type":"audio/pcm","rate":24000},"transcription":{"model":"grok-transcribe"}}}}}"#
                .into(),
        )
    }

    #[test]
    fn transcript_safety_limit_discards_same_turn_residue_without_cross_pairing() {
        let mut turn = GrokTurnState::default();
        let source_error = turn.update_source(
            "s".repeat(MAXIMUM_TRANSCRIPT_BYTES + 1),
            Some("oversized_source".into()),
            None,
            false,
        );
        assert!(matches!(
            source_error.as_slice(),
            [LiveTranslateServerEvent::Error { code, message }]
                if code == "xai_transcript_safety_limit"
                    && message == "xAI Grok Voice transcript buffering exceeded its safety limit."
        ));
        assert!(turn.source.is_empty());
        assert!(turn.translation.is_empty());
        assert!(turn.item_id.is_none());
        assert!(turn.discard_current_turn);
        assert_eq!(turn.discarded_item_id.as_deref(), Some("oversized_source"));

        assert!(turn
            .update_source(
                "stale source tail".into(),
                Some("oversized_source".into()),
                None,
                true,
            )
            .is_empty());

        let source_events = turn.update_source(
            "Next sentence.".into(),
            Some("next_source".into()),
            Some("en".into()),
            true,
        );
        assert!(matches!(
            source_events.as_slice(),
            [LiveTranslateServerEvent::SourceDraft { text, language }]
                if text == "Next sentence." && language.as_deref() == Some("en")
        ));
        assert!(turn.discard_current_turn);
        assert!(turn.source.is_empty());

        turn.response_started(Some("old_response".into()));
        assert!(turn
            .append_translation("stale translation tail".into(), Some("old_response".into()))
            .is_empty());
        assert!(turn
            .complete_translation(
                Some("stale final translation".into()),
                Some("old_response".into()),
            )
            .is_empty());
        assert!(turn.response_done(Some("old_response".into())).is_empty());
        assert!(!turn.discard_current_turn);
        assert_eq!(turn.item_id.as_deref(), Some("next_source"));
        assert_eq!(turn.source, "Next sentence.");
        assert!(turn.awaits_response_for_promoted_source());

        turn.response_started(Some("next_response".into()));
        let pair = turn.complete_translation(Some("次の文。".into()), Some("next_response".into()));
        assert!(matches!(
            pair.as_slice(),
            [LiveTranslateServerEvent::TranslationDraft(translation),
             LiveTranslateServerEvent::SubtitleFinalPair { source, language, translation: final_translation }]
                if translation == "次の文。"
                    && source == "Next sentence."
                    && language.as_deref() == Some("en")
                    && final_translation == "次の文。"
        ));

        let source_bytes = MAXIMUM_TRANSCRIPT_BYTES / 2;
        let _ = turn.update_source(
            "s".repeat(source_bytes),
            Some("combined_turn".into()),
            None,
            true,
        );
        let combined_error = turn.append_translation(
            "t".repeat(MAXIMUM_TRANSCRIPT_BYTES - source_bytes + 1),
            Some("combined_response".into()),
        );
        assert!(matches!(
            combined_error.as_slice(),
            [LiveTranslateServerEvent::Error { code, .. }]
                if code == "xai_transcript_safety_limit"
        ));
        assert!(turn.source.is_empty());
        assert!(turn.translation.is_empty());
        assert!(turn.response_id.is_none());
        assert!(turn.discard_current_turn);
        assert_eq!(turn.discarded_item_id.as_deref(), Some("combined_turn"));

        let mut unidentified_turn = GrokTurnState::default();
        let _ = unidentified_turn.update_source(
            "s".repeat(MAXIMUM_TRANSCRIPT_BYTES + 1),
            None,
            None,
            false,
        );
        assert!(unidentified_turn
            .update_source(
                "cannot prove this is a new turn".into(),
                Some("candidate_item".into()),
                None,
                true,
            )
            .is_empty());
        assert!(unidentified_turn
            .response_done(Some("discarded_response".into()))
            .is_empty());
        assert!(!unidentified_turn.discard_current_turn);
        assert!(!unidentified_turn
            .update_source(
                "Safe after response.done.".into(),
                Some("confirmed_new_item".into()),
                None,
                true,
            )
            .is_empty());
    }

    #[tokio::test]
    async fn configured_source_hint_reaches_the_actual_websocket_setup() {
        let (client, _) = test_client(|mut socket| {
            Box::pin(async move {
                let update: Value =
                    serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                        .unwrap();
                assert_eq!(
                    update["session"]["audio"]["input"]["transcription"]["language_hint"],
                    "fr"
                );
                socket.send(setup_ack()).await.unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if message.is_close() {
                        break;
                    }
                }
            })
        })
        .await;
        let client = client.with_source_language(crate::core::models::SourceLanguage::French);
        client.connect().await.unwrap();
        client.disconnect().await;
    }

    #[test]
    fn frame_buffer_keeps_only_a_partial_tail() {
        let frame_size = XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT;
        let mut pending = vec![0x23; frame_size * 2 + 17];
        let messages = take_complete_audio_messages(&mut pending).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(pending, vec![0x23; 17]);
        for message in messages {
            let value: Value = serde_json::from_str(&message).unwrap();
            let frame = base64::engine::general_purpose::STANDARD
                .decode(value["audio"].as_str().unwrap())
                .unwrap();
            assert_eq!(frame.len(), frame_size);
            assert!(frame.iter().all(|byte| *byte == 0x23));
        }
    }

    #[tokio::test]
    async fn mock_websocket_handles_revised_source_and_never_commits_in_server_vad() {
        let saw_commit = Arc::new(AtomicBool::new(false));
        let server_saw_commit = Arc::clone(&saw_commit);
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let update: Value = serde_json::from_str(
                    socket.next().await.unwrap().unwrap().to_text().unwrap(),
                )
                .unwrap();
                assert_eq!(update["session"]["turn_detection"]["type"], "server_vad");
                assert_eq!(
                    update["session"]["turn_detection"]["silence_duration_ms"],
                    XAIRealtimeEndpoint::SERVER_VAD_SILENCE_DURATION_MS
                );
                assert_eq!(
                    update["session"]["audio"]["input"]["transcription"]["model"],
                    "grok-transcribe"
                );
                socket.send(setup_ack()).await.unwrap();

                let audio = socket.next().await.unwrap().unwrap();
                let audio: Value = serde_json::from_str(audio.to_text().unwrap()).unwrap();
                assert_eq!(audio["type"], "input_audio_buffer.append");
                socket
                    .send(Message::Text(
                        r#"{"type":"conversation.item.input_audio_transcription.updated","item_id":"item_1","transcript":"I scream"}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"conversation.item.input_audio_transcription.updated","item_id":"item_1","transcript":"Ice cream"}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"item_1","transcript":"Ice cream."}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"response.created","response":{"id":"resp_1"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"response.output_audio_transcript.delta","response_id":"resp_1","delta":"アイスクリーム。"}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"response.output_audio_transcript.done","response_id":"resp_1","transcript":"アイスクリーム。"}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"response.done","response":{"id":"resp_1","status":"completed"}}"#.into(),
                    ))
                    .await
                    .unwrap();

                while let Some(Ok(message)) = socket.next().await {
                    if message.to_text().ok().is_some_and(|text| {
                        text.contains("input_audio_buffer.commit")
                    }) {
                        server_saw_commit.store(true, Ordering::SeqCst);
                    }
                }
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.send_audio(&vec![0; 9_600]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        client.finish(Duration::from_millis(500)).await;

        assert!(!saw_commit.load(Ordering::SeqCst));
        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. }
                if source == "Ice cream." && translation == "アイスクリーム。"
        )));
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    }

    #[tokio::test]
    async fn finish_appends_vad_silence_before_waiting_for_the_final_turn() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();

                let speech: Value = serde_json::from_str(
                    socket.next().await.unwrap().unwrap().to_text().unwrap(),
                )
                .unwrap();
                let speech = base64::engine::general_purpose::STANDARD
                    .decode(speech["audio"].as_str().unwrap())
                    .unwrap();
                assert!(speech.iter().any(|byte| *byte != 0));

                for _ in 0..SERVER_VAD_TAIL_FRAME_COUNT {
                    let silence: Value = serde_json::from_str(
                        socket.next().await.unwrap().unwrap().to_text().unwrap(),
                    )
                    .unwrap();
                    let silence = base64::engine::general_purpose::STANDARD
                        .decode(silence["audio"].as_str().unwrap())
                        .unwrap();
                    assert_eq!(silence.len(), XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT);
                    assert!(silence.iter().all(|byte| *byte == 0));
                }

                socket
                    .send(Message::Text(
                        r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"tail_1","transcript":"Last sentence."}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"response.created","response":{"id":"tail_response"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"response.output_audio_transcript.done","response_id":"tail_response","transcript":"最後の文。"}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"response.done","response":{"id":"tail_response","status":"completed"}}"#.into(),
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
        client
            .send_audio(&vec![1; XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        client.finish(Duration::from_millis(500)).await;

        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. }
                if source == "Last sentence." && translation == "最後の文。"
        )));
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
    }

    #[tokio::test]
    async fn failed_response_emits_fixed_error_and_is_not_a_clean_finish() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"type":"response.created","response":{"id":"failed_response"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"type":"response.done","response":{"id":"failed_response","status":"failed"}}"#.into(),
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
        client
            .send_audio(&vec![1; XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        client.finish(Duration::from_millis(500)).await;

        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::Error { code, message }
                if code == "xai_response_failed" && message == GENERIC_RESPONSE_FAILED_ERROR
        )));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    }

    #[tokio::test]
    async fn transport_close_while_finishing_is_not_a_clean_finish() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                let _speech = socket.next().await.unwrap().unwrap();
                let _first_tail = socket.next().await.unwrap().unwrap();
                socket.close(None).await.unwrap();
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client
            .send_audio(&vec![1; XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        client.finish(Duration::from_millis(500)).await;

        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::Error { code, message }
                if code == "xai_session_finish_failed" && message == GENERIC_TRANSPORT_ERROR
        )));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
    }

    #[tokio::test]
    async fn finish_is_bounded_when_server_vad_has_not_completed_a_turn() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                while socket.next().await.is_some() {}
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.send_audio(&vec![0; 9_600]).await.unwrap();
        let started = tokio::time::Instant::now();
        client.finish(Duration::from_millis(40)).await;
        assert!(started.elapsed() < Duration::from_secs(1));
        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::Error { code, message }
                if code == "xai_session_finish_timeout"
                    && message == GENERIC_FINISH_TIMEOUT_ERROR
        )));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
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
            XAIRealtimeClientError::SessionSetupTimedOut
        );
    }
}
