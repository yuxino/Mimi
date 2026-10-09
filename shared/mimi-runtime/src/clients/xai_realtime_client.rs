//! xAI Grok Voice WebSocket adapter for turn-based translation.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::models::{TargetLanguage, UtteranceRole};
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
// The official live-interpreter cookbook sends 1.5 seconds of trailing audio.
// This is a local finish strategy, not a configured/known server-VAD threshold.
const FINISH_TAIL_SILENCE_MS: u32 = 1_500;
const FINISH_TAIL_FRAME_COUNT: usize =
    (FINISH_TAIL_SILENCE_MS as usize).div_ceil(XAIRealtimeEndpoint::FRAME_DURATION_MS as usize);

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
    source_committed: bool,
    speech_active: bool,
    response_id: Option<String>,
    translation: String,
    translation_complete: bool,
    response_complete: bool,
    discard_current_turn: bool,
    discarded_item_id: Option<String>,
    pending_next_source: Option<PendingGrokSource>,
    retired_source_ids: std::collections::VecDeque<String>,
    retired_response_ids: std::collections::VecDeque<String>,
    pairing_ambiguous: bool,
    discarded_pending_response: bool,
}

struct PendingGrokSource {
    item_id: String,
    transcript: String,
    language: Option<String>,
    completed: bool,
    committed: bool,
    speech_active: bool,
}

impl GrokTurnState {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn reset_content(&mut self) {
        let retired_source_ids = std::mem::take(&mut self.retired_source_ids);
        let retired_response_ids = std::mem::take(&mut self.retired_response_ids);
        let discarded_pending_response = self.discarded_pending_response;
        self.reset();
        self.retired_source_ids = retired_source_ids;
        self.retired_response_ids = retired_response_ids;
        self.discarded_pending_response = discarded_pending_response;
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
        let response_id = self.response_id.clone();
        self.retire_pending_source();
        self.reset_content();
        self.response_id = response_id;
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
        if item_id
            .as_ref()
            .is_some_and(|id| self.retired_source_ids.contains(id))
        {
            return Vec::new();
        }
        if self.reject_source_while_discarded_response_is_pending(item_id.as_deref()) {
            return Vec::new();
        }
        if self.discard_current_turn {
            return self.update_pending_next_source(transcript, item_id, language, completed);
        }
        if identifiers_conflict(self.item_id.as_deref(), item_id.as_deref()) {
            // VAD may start the next input before the preceding response ends.
            // Keep its source separate until that response's terminal event.
            if self.response_id.is_some() || self.source_committed || self.source_complete {
                return self.update_pending_next_source(transcript, item_id, language, completed);
            }
            self.advance_turn();
        }
        if item_id.is_some() {
            self.item_id = item_id;
        }
        self.source = transcript;
        if language.is_some() {
            self.source_language = language;
        }
        self.source_complete |= completed;
        self.source_committed |= completed;
        if completed {
            self.speech_active = false;
        }

        if self.exceeded_safety_limit() {
            return self.safety_limit_error();
        }

        let mut events = self
            .item_id
            .as_ref()
            .filter(|_| !self.source.is_empty())
            .map(|item_id| {
                vec![LiveTranslateServerEvent::UtteranceText {
                    utterance_id: item_id.clone(),
                    role: UtteranceRole::Source,
                    text: self.source.clone(),
                    is_final: false,
                    language: self.source_language.clone(),
                }]
            })
            .unwrap_or_default();
        if let Some(pair) = self.take_pair_if_complete() {
            events.push(pair);
        }
        events
    }

    fn response_started(&mut self, response_id: Option<String>) -> bool {
        let Some(response_id) = response_id else {
            return false;
        };
        if self.retired_response_ids.contains(&response_id) {
            return false;
        }
        if identifiers_conflict(self.response_id.as_deref(), Some(&response_id)) {
            self.advance_turn();
        }
        self.response_id = Some(response_id);
        if self.item_id.is_none() {
            // The response carries no input item identity. Without a known
            // input boundary it cannot safely borrow a later source transcript.
            self.discard_current_turn = true;
        }
        !self.discard_current_turn
    }

    fn accepts_response(&mut self, response_id: Option<String>) -> bool {
        let Some(id) = response_id else { return false };
        if self.retired_response_ids.contains(&id)
            || identifiers_conflict(self.response_id.as_deref(), Some(&id))
        {
            return false;
        }
        if self.response_id.is_none() {
            self.response_id = Some(id);
        }
        if self.item_id.is_none() {
            self.discard_current_turn = true;
        }
        !self.discard_current_turn
    }

    fn append_translation(
        &mut self,
        delta: String,
        response_id: Option<String>,
    ) -> Vec<LiveTranslateServerEvent> {
        if !self.accepts_response(response_id) {
            return Vec::new();
        }
        if delta.is_empty() {
            return Vec::new();
        }
        self.translation.push_str(&delta);
        if self.exceeded_safety_limit() {
            return self.safety_limit_error();
        }
        self.translation_preview().into_iter().collect()
    }

    fn complete_translation(
        &mut self,
        final_transcript: Option<String>,
        response_id: Option<String>,
    ) -> Vec<LiveTranslateServerEvent> {
        if !self.accepts_response(response_id) {
            return Vec::new();
        }
        let mut events = Vec::new();
        if let Some(final_transcript) = final_transcript.filter(|text| !text.is_empty()) {
            if self.translation != final_transcript {
                self.translation = final_transcript;
                if self.exceeded_safety_limit() {
                    return self.safety_limit_error();
                }
                events.extend(self.translation_preview());
            }
        }
        self.translation_complete = true;
        if let Some(pair) = self.take_pair_if_complete() {
            events.push(pair);
        }
        events
    }

    fn translation_preview(&self) -> Option<LiveTranslateServerEvent> {
        self.item_id
            .as_ref()
            .map(|item_id| LiveTranslateServerEvent::UtteranceText {
                utterance_id: item_id.clone(),
                role: UtteranceRole::Translation,
                text: self.translation.clone(),
                // Transcript completion is still a preview until response.done
                // confirms success. Marking this final would bypass that gate.
                is_final: false,
                language: None,
            })
    }

    #[cfg(test)]
    fn response_done(&mut self, response_id: Option<String>) -> Vec<LiveTranslateServerEvent> {
        self.finish_response(response_id, true).unwrap_or_default()
    }

    fn finish_response(
        &mut self,
        response_id: Option<String>,
        successful: bool,
    ) -> Option<Vec<LiveTranslateServerEvent>> {
        let response_id = response_id?;
        if self.retired_response_ids.contains(&response_id)
            || identifiers_conflict(self.response_id.as_deref(), Some(&response_id))
        {
            return None;
        }
        self.response_id = Some(response_id);
        if self.item_id.is_none() {
            self.discard_current_turn = true;
        }
        if self.discard_current_turn || !successful {
            // A new source item can arrive before the discarded response has
            // finished. Promote that source only after the provider's
            // definitive response boundary so late translation events from the
            // oversized turn can never be paired with it.
            // The active/discarded A boundary comes first. The next discarded
            // response, which has no retained input, resolves discarded B.
            if self.item_id.is_none() && self.discarded_item_id.is_none() {
                self.discarded_pending_response = false;
            }
            self.advance_turn();
            return Some(Vec::new());
        }
        self.translation_complete = true;
        self.response_complete = true;
        Some(self.take_pair_if_complete().into_iter().collect())
    }

    fn take_pair_if_complete(&mut self) -> Option<LiveTranslateServerEvent> {
        if !self.source_complete
            || !self.translation_complete
            || !self.response_complete
            || self.item_id.is_none()
        {
            return None;
        }
        let event = (is_meaningful(&self.source) && is_meaningful(&self.translation)).then(|| {
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id: self.item_id.clone().unwrap(),
                source: self.source.trim().to_string(),
                language: self.source_language.clone(),
                translation: self.translation.trim().to_string(),
            }
        });
        self.advance_turn();
        event
    }

    fn exceeded_safety_limit(&self) -> bool {
        self.source.len().saturating_add(self.translation.len()) > MAXIMUM_TRANSCRIPT_BYTES
    }

    fn has_unfinished_turn(&self) -> bool {
        self.speech_active
            || self.discarded_pending_response
            || self.source_committed
            || self.response_id.is_some()
            || self.pending_next_source.is_some()
    }

    fn advance_turn(&mut self) {
        let pending = self.pending_next_source.take();
        if let Some(id) = self
            .item_id
            .take()
            .or_else(|| self.discarded_item_id.take())
        {
            remember_retired_id(&mut self.retired_source_ids, id);
        }
        if let Some(id) = self.response_id.take() {
            remember_retired_id(&mut self.retired_response_ids, id);
        }
        self.reset_content();
        if let Some(pending) = pending {
            self.item_id = Some(pending.item_id);
            self.source = pending.transcript;
            self.source_language = pending.language;
            self.source_complete = pending.completed;
            self.source_committed = pending.committed;
            self.speech_active = pending.speech_active;
        }
    }

    fn input_boundary(&mut self, item_id: Option<String>, committed: bool) {
        let Some(item_id) = item_id else { return };
        if self.retired_source_ids.contains(&item_id) {
            return;
        }
        if self.reject_source_while_discarded_response_is_pending(Some(&item_id)) {
            return;
        }
        if self.item_id.as_deref() == Some(&item_id) {
            self.source_committed |= committed;
            self.speech_active = !committed;
            return;
        }
        if self.discard_current_turn
            || self.response_id.is_some()
            || self.source_committed
            || self.source_complete
        {
            self.update_pending_next_source(String::new(), Some(item_id.clone()), None, false);
            if let Some(pending) = self
                .pending_next_source
                .as_mut()
                .filter(|pending| pending.item_id == item_id)
            {
                pending.committed |= committed;
                pending.speech_active = !committed;
            }
        } else {
            self.advance_turn();
            self.item_id = Some(item_id);
            self.source_committed = committed;
            self.speech_active = !committed;
        }
    }

    fn safety_limit_error(&mut self) -> Vec<LiveTranslateServerEvent> {
        let discarded_item_id = self.item_id.clone();
        let response_id = self.response_id.clone();
        self.retire_pending_source();
        self.reset_content();
        self.response_id = response_id;
        self.discard_current_turn = true;
        self.discarded_item_id = discarded_item_id;
        vec![LiveTranslateServerEvent::Error {
            code: "xai_transcript_safety_limit".into(),
            message: "xAI Grok Voice transcript buffering exceeded its safety limit.".into(),
        }]
    }

    fn retire_pending_source(&mut self) {
        if let Some(pending) = self.pending_next_source.take() {
            self.discarded_pending_response = true;
            remember_retired_id(&mut self.retired_source_ids, pending.item_id);
        }
    }

    fn reject_source_while_discarded_response_is_pending(&mut self, item_id: Option<&str>) -> bool {
        if !self.discarded_pending_response {
            return false;
        }
        let Some(item_id) = item_id else {
            return true;
        };
        if self.item_id.as_deref() == Some(item_id) {
            return false;
        }
        if self.discarded_item_id.as_deref() == Some(item_id)
            || self.retired_source_ids.iter().any(|id| id == item_id)
        {
            return true;
        }
        // C precedes a response for discarded B. Since responses carry no input
        // identity, borrowing C would be a guess. Recover once with no text.
        self.reset_content();
        self.discard_current_turn = true;
        self.pairing_ambiguous = true;
        true
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
        if (self.discard_current_turn && self.discarded_item_id.is_none())
            || self.discarded_item_id.as_deref() == Some(&item_id)
            || self.retired_source_ids.contains(&item_id)
        {
            return Vec::new();
        }
        if self
            .pending_next_source
            .as_ref()
            .is_some_and(|pending| pending.item_id != item_id)
        {
            // Check identity before size: an oversized second pending item
            // cannot be collapsed into a single discarded-response tombstone.
            self.reset_content();
            self.discard_current_turn = true;
            self.pairing_ambiguous = true;
            return Vec::new();
        }
        if transcript.len() > MAXIMUM_TRANSCRIPT_BYTES {
            self.retire_pending_source();
            self.discarded_pending_response = true;
            remember_retired_id(&mut self.retired_source_ids, item_id);
            return vec![LiveTranslateServerEvent::Error {
                code: "xai_transcript_safety_limit".into(),
                message: "xAI Grok Voice transcript buffering exceeded its safety limit.".into(),
            }];
        }

        match self.pending_next_source.as_mut() {
            Some(pending) => {
                if !transcript.is_empty() || completed {
                    pending.transcript = transcript;
                }
                if language.is_some() {
                    pending.language = language;
                }
                pending.completed |= completed;
                pending.committed |= completed;
                if completed {
                    pending.speech_active = false;
                }
            }
            _ => {
                self.pending_next_source = Some(PendingGrokSource {
                    item_id,
                    transcript,
                    language,
                    completed,
                    committed: completed,
                    speech_active: !completed,
                });
            }
        }

        self.pending_next_source
            .as_ref()
            .filter(|pending| !pending.transcript.is_empty())
            .map(|pending| {
                vec![LiveTranslateServerEvent::UtteranceText {
                    utterance_id: pending.item_id.clone(),
                    role: UtteranceRole::Source,
                    text: pending.transcript.clone(),
                    is_final: false,
                    language: pending.language.clone(),
                }]
            })
            .unwrap_or_default()
    }
}

fn remember_retired_id(ids: &mut std::collections::VecDeque<String>, id: String) {
    if ids.len() == 4 {
        ids.pop_front();
    }
    ids.push_back(id);
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
    /// `input_audio_buffer.commit`. Drain the local frame buffer, append the
    /// cookbook's bounded silence tail, then wait a bounded
    /// amount of time for trailing VAD/input/response events before closing.
    /// A preceding `response.done` cannot acknowledge subsequently sent audio.
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
                    // A PCM frame does not imply a VAD speech turn. Keep the
                    // socket alive for the existing deadline so a preceding
                    // response.done cannot hide late events for trailing audio.
                    let unfinished = self.inner.turn.lock().await.has_unfinished_turn();
                    if unfinished {
                        self.emit(
                            LiveTranslateServerEvent::Error {
                                code: "xai_session_finish_timeout".into(),
                                message: GENERIC_FINISH_TIMEOUT_ERROR.into(),
                            },
                            generation,
                        );
                    }
                    !unfinished
                        && !self.inner.last_response_failed.load(Ordering::SeqCst)
                        && !self.inner.finish_transport_failed.load(Ordering::SeqCst)
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
            self.send_text(message).await?;
        }
        let silence = XAIRealtimeRequestEncoder::audio_append(&vec![
            0;
            XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT
        ])
        .map_err(|_| XAIRealtimeClientError::TransportFailure)?
        .to_string();
        for _ in 0..FINISH_TAIL_FRAME_COUNT {
            self.send_text(silence.clone()).await?;
        }

        while self.is_current_generation(generation) {
            let done = self.inner.response_done_notify.notified();
            tokio::pin!(done);
            done.as_mut().enable();
            if !self.is_current_generation(generation) {
                break;
            }
            if self.inner.finish_transport_failed.load(Ordering::SeqCst) {
                return Err(XAIRealtimeClientError::TransportFailure);
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
                | XAIRealtimeServerEvent::SpeechStarted { .. }
                | XAIRealtimeServerEvent::SourceCommitted { .. }
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
        XAIRealtimeServerEvent::SpeechStarted { item_id } => {
            context
                .inner
                .turn
                .lock()
                .await
                .input_boundary(item_id, false);
        }
        XAIRealtimeServerEvent::SourceCommitted { item_id } => {
            context
                .inner
                .turn
                .lock()
                .await
                .input_boundary(item_id, true);
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
            let mut turn = context.inner.turn.lock().await;
            if turn.response_started(response_id) {
                context
                    .inner
                    .last_response_failed
                    .store(false, Ordering::SeqCst);
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
            let successful = status.as_deref() == Some("completed");
            let cancelled = status.as_deref() == Some("cancelled");
            let events = context
                .inner
                .turn
                .lock()
                .await
                .finish_response(response_id, successful);
            let Some(events) = events else {
                return false;
            };
            emit_all_if_current(context, events);
            if successful || cancelled {
                context
                    .inner
                    .last_response_failed
                    .store(false, Ordering::SeqCst);
            } else {
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
    if context.inner.turn.lock().await.pairing_ambiguous {
        fail_receive_loop(context, "transport_error", GENERIC_PROTOCOL_ERROR).await;
        return true;
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
        let previews =
            turn.complete_translation(Some("new translation".into()), Some("new-response".into()));
        assert!(!previews.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
        )));
        let events = turn.response_done(Some("new-response".into()));
        assert!(events.iter().any(|event| matches!(event, LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {source,translation,..}
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

    #[tokio::test]
    async fn interrupted_response_cannot_pair_its_translation_with_the_next_source() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                for event in [
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"First source."}"#,
                    r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                    r#"{"type":"response.output_audio_transcript.delta","response_id":"response_a","delta":"First translation."}"#,
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_b","transcript":"Second source."}"#,
                    r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"First translation."}"#,
                    r#"{"type":"response.done","response":{"id":"response_a","status":"cancelled"}}"#,
                    r#"{"type":"response.created","response":{"id":"response_b"}}"#,
                    r#"{"type":"response.output_audio_transcript.done","response_id":"response_b","transcript":"Second translation."}"#,
                    r#"{"type":"response.done","response":{"id":"response_b","status":"completed"}}"#,
                ] {
                    socket.send(Message::Text(event.into())).await.unwrap();
                }
                while socket.next().await.is_some() {}
            })
        }).await;
        client.connect().await.unwrap();
        let mut pairs = Vec::new();
        let mut errors = Vec::new();
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), events.recv()).await
        {
            match event {
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                    source,
                    translation,
                    ..
                } => {
                    pairs.push((source, translation));
                }
                LiveTranslateServerEvent::Error { code, .. } => errors.push(code),
                _ => {}
            }
        }
        client.disconnect().await;
        assert_eq!(
            pairs,
            [("Second source.".into(), "Second translation.".into())]
        );
        assert!(
            errors.is_empty(),
            "normal VAD cancellation must not fail the session"
        );
    }

    #[tokio::test]
    async fn failed_response_does_not_confirm_its_completed_transcript() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                for event in [
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"First source."}"#,
                    r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                    r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"Unconfirmed translation."}"#,
                    r#"{"type":"response.done","response":{"id":"response_a","status":"failed"}}"#,
                ] {
                    socket.send(Message::Text(event.into())).await.unwrap();
                }
                while socket.next().await.is_some() {}
            })
        }).await;
        client.connect().await.unwrap();
        let mut pairs = Vec::new();
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), events.recv()).await
        {
            if let LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. } = event {
                pairs.push(event);
            }
        }
        client.disconnect().await;
        assert!(pairs.is_empty());
    }

    #[tokio::test]
    async fn silence_only_audio_finishes_without_a_response_timeout_error() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                while socket.next().await.is_some() {}
            })
        })
        .await;
        client.connect().await.unwrap();
        client
            .send_audio(&vec![0; XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        client.finish(Duration::from_millis(40)).await;
        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
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
            [LiveTranslateServerEvent::UtteranceText { text, language, utterance_id, role: UtteranceRole::Source, is_final: false }]
                if text == "Next sentence." && language.as_deref() == Some("en")
                    && utterance_id == "next_source"
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
        assert!(turn.has_unfinished_turn());

        turn.response_started(Some("next_response".into()));
        let previews =
            turn.complete_translation(Some("次の文。".into()), Some("next_response".into()));
        assert!(
            matches!(previews.as_slice(), [LiveTranslateServerEvent::UtteranceText { text, utterance_id, role: UtteranceRole::Translation, is_final: false, .. }] if text == "次の文。" && utterance_id == "next_source")
        );
        let pair = turn.response_done(Some("next_response".into()));
        assert!(matches!(
            pair.as_slice(),
            [LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { source, language, translation: final_translation, .. }]
                if source == "Next sentence."
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
        assert_eq!(turn.response_id.as_deref(), Some("combined_response"));
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

    #[test]
    fn successful_response_waits_for_its_source_and_cannot_clear_the_next_turn() {
        let mut turn = GrokTurnState::default();
        turn.input_boundary(Some("source_a".into()), true);
        turn.response_started(Some("response_a".into()));
        assert!(turn
            .complete_translation(Some("Translation A.".into()), Some("response_a".into()))
            .iter()
            .all(|event| !matches!(
                event,
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
            )));
        assert!(turn.response_done(Some("response_a".into())).is_empty());
        turn.input_boundary(Some("source_b".into()), false);
        let events = turn.update_source("Source A.".into(), Some("source_a".into()), None, true);
        assert!(events.iter().any(|event| matches!(event,
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {source, translation, ..}
                if source == "Source A." && translation == "Translation A.")));
        assert_eq!(turn.item_id.as_deref(), Some("source_b"));
        assert!(turn.has_unfinished_turn());
        assert!(turn.response_done(Some("response_a".into())).is_empty());
        assert!(!turn.response_started(Some("response_a".into())));
        assert_eq!(turn.item_id.as_deref(), Some("source_b"));
        assert!(turn.source.is_empty() && turn.translation.is_empty());
        assert!(turn.has_unfinished_turn());
    }

    #[test]
    fn a_response_without_a_known_input_boundary_cannot_borrow_a_later_source() {
        let mut turn = GrokTurnState::default();
        assert!(turn
            .append_translation("Unknown translation.".into(), Some("response_a".into()))
            .is_empty());
        assert!(turn
            .update_source("Later source.".into(), Some("source_b".into()), None, true)
            .is_empty());
        assert!(turn.response_done(Some("response_a".into())).is_empty());
        assert!(turn.source.is_empty() && turn.translation.is_empty());
    }

    #[test]
    fn identified_lanes_project_early_previews_without_crossing_overlapping_turns() {
        let mut turn = GrokTurnState::default();
        let mut controller = crate::core::session::TranslationSessionController::default();
        controller.set_atomic_preview(false);
        controller.did_connect();
        let apply = |controller: &mut crate::core::session::TranslationSessionController,
                     events: Vec<LiveTranslateServerEvent>| {
            for event in events {
                controller.handle(event);
            }
        };
        apply(
            &mut controller,
            turn.update_source(
                "Source A.".into(),
                Some("source_a".into()),
                Some("en".into()),
                true,
            ),
        );
        turn.response_started(Some("response_a".into()));
        apply(
            &mut controller,
            turn.append_translation("Translation A".into(), Some("response_a".into())),
        );
        let early = controller
            .state
            .subtitles
            .realtime_preview
            .as_ref()
            .unwrap();
        assert_eq!(early.source.text, "Source A.");
        assert_eq!(early.translation.text, "Translation A");
        assert!(controller.state.subtitles.history.is_empty());

        apply(
            &mut controller,
            turn.update_source(
                "Source B growing".into(),
                Some("source_b".into()),
                Some("ko".into()),
                false,
            ),
        );
        let overlapping = controller
            .state
            .subtitles
            .realtime_preview
            .as_ref()
            .unwrap();
        assert_eq!(overlapping.source.text, "Source B growing");
        assert!(
            overlapping.translation.text.is_empty(),
            "A translation cannot accompany B source"
        );
        apply(
            &mut controller,
            turn.complete_translation(Some("Translation A.".into()), Some("response_a".into())),
        );
        assert_eq!(controller.state.subtitles.source.text, "Source B growing");
        apply(
            &mut controller,
            turn.response_done(Some("response_a".into())),
        );
        assert_eq!(
            controller.state.subtitles.source.text, "Source B growing",
            "late A final must preserve B's current lane"
        );
        assert_eq!(controller.state.subtitles.history.len(), 1);
        assert_eq!(
            controller.state.detected_language.as_ref().unwrap().code,
            "ko"
        );

        apply(
            &mut controller,
            turn.update_source(
                "Source B.".into(),
                Some("source_b".into()),
                Some("ko".into()),
                true,
            ),
        );
        turn.response_started(Some("response_b".into()));
        apply(
            &mut controller,
            turn.append_translation("Translation B".into(), Some("response_b".into())),
        );
        let early_b = controller
            .state
            .subtitles
            .realtime_preview
            .as_ref()
            .unwrap();
        assert_eq!(early_b.source.utterance_id.as_deref(), Some("source_b"));
        assert_eq!(
            early_b.translation.utterance_id.as_deref(),
            Some("source_b")
        );
        assert_eq!(early_b.translation.text, "Translation B");
        assert_eq!(controller.state.subtitles.history.len(), 1);
        apply(
            &mut controller,
            turn.complete_translation(Some("Translation B.".into()), Some("response_b".into())),
        );
        apply(
            &mut controller,
            turn.response_done(Some("response_b".into())),
        );
        assert_eq!(controller.state.subtitles.history.len(), 2);
        let pair = controller.state.subtitles.display_pair.as_ref().unwrap();
        assert_eq!(pair.utterance_id.as_deref(), Some("source_b"));
        assert_eq!(pair.source, "Source B.");
        assert_eq!(pair.translation, "Translation B.");
    }

    #[test]
    fn clear_retires_pending_input_before_late_completions_reach_the_real_projection() {
        let mut turn = GrokTurnState::default();
        let mut controller = crate::core::session::TranslationSessionController::default();
        controller.set_atomic_preview(false);
        controller.did_connect();
        let apply = |controller: &mut crate::core::session::TranslationSessionController,
                     events: Vec<LiveTranslateServerEvent>| {
            for event in events {
                controller.handle(event);
            }
        };
        apply(
            &mut controller,
            turn.update_source("Source A.".into(), Some("source_a".into()), None, true),
        );
        turn.response_started(Some("response_a".into()));
        apply(
            &mut controller,
            turn.append_translation("Translation A".into(), Some("response_a".into())),
        );
        turn.input_boundary(Some("source_b".into()), false);
        apply(
            &mut controller,
            turn.update_source(
                "Source B growing".into(),
                Some("source_b".into()),
                None,
                false,
            ),
        );
        turn.clear_content();
        controller.clear_subtitles();

        assert!(
            turn.update_source("Source B.".into(), Some("source_b".into()), None, true)
                .is_empty(),
            "cleared pending B must not be admitted again"
        );
        apply(
            &mut controller,
            turn.response_done(Some("response_a".into())),
        );
        turn.response_started(Some("response_b".into()));
        apply(
            &mut controller,
            turn.complete_translation(Some("Translation B.".into()), Some("response_b".into())),
        );
        apply(
            &mut controller,
            turn.response_done(Some("response_b".into())),
        );
        assert!(controller.state.subtitles.history.is_empty());
        assert!(controller.state.subtitles.source.text.is_empty());
        assert!(controller.state.subtitles.translation.text.is_empty());

        apply(
            &mut controller,
            turn.update_source("Source C.".into(), Some("source_c".into()), None, true),
        );
        turn.response_started(Some("response_c".into()));
        apply(
            &mut controller,
            turn.complete_translation(Some("Translation C.".into()), Some("response_c".into())),
        );
        apply(
            &mut controller,
            turn.response_done(Some("response_c".into())),
        );
        assert_eq!(controller.state.subtitles.history.len(), 1);
        let pair = controller.state.subtitles.display_pair.as_ref().unwrap();
        assert_eq!(pair.source, "Source C.");
        assert_eq!(pair.translation, "Translation C.");
        assert!(
            turn.source.is_empty()
                && turn.translation.is_empty()
                && turn.pending_next_source.is_none()
        );
        assert!(turn.retired_source_ids.len() <= 4 && turn.retired_response_ids.len() <= 4);
    }

    #[tokio::test]
    async fn discarded_pending_input_never_lends_its_response_after_a_safety_limit() {
        for scenario in [
            "current_translation",
            "pending_source",
            "extra_pending_source",
        ] {
            let (client, mut events) = test_client(move |mut socket| {
                Box::pin(async move {
                    let _ = socket.next().await;
                    socket.send(setup_ack()).await.unwrap();
                    for event in [
                        r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Source A."}"#,
                        r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                        r#"{"type":"conversation.item.input_audio_transcription.updated","item_id":"source_b","transcript":"Source B growing"}"#,
                    ] { socket.send(Message::Text(event.into())).await.unwrap(); }
                    let oversized = "s".repeat(MAXIMUM_TRANSCRIPT_BYTES + 1);
                    let oversized_event = if scenario == "current_translation" {
                        serde_json::json!({"type":"response.output_audio_transcript.delta", "response_id":"response_a", "delta":oversized})
                    } else {
                        let id = if scenario == "pending_source" {"source_b"} else {"source_c"};
                        serde_json::json!({"type":"conversation.item.input_audio_transcription.updated", "item_id":id, "transcript":oversized})
                    };
                    socket.send(Message::Text(oversized_event.to_string().into())).await.unwrap();
                    for event in [
                        r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#,
                        r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_c","transcript":"Source C."}"#,
                        r#"{"type":"response.created","response":{"id":"response_b"}}"#,
                        r#"{"type":"response.output_audio_transcript.done","response_id":"response_b","transcript":"Translation B."}"#,
                        r#"{"type":"response.done","response":{"id":"response_b","status":"completed"}}"#,
                    ] { if socket.send(Message::Text(event.into())).await.is_err() { break; } }
                    while socket.next().await.is_some() {}
                })
            }).await;
            client.connect().await.unwrap();
            let mut controller = crate::core::session::TranslationSessionController::default();
            controller.did_connect();
            let mut recovery_errors = 0;
            while let Ok(Some(event)) =
                tokio::time::timeout(Duration::from_millis(50), events.recv()).await
            {
                if let LiveTranslateServerEvent::Error { code, .. } = &event {
                    if code == "transport_error" {
                        recovery_errors += 1;
                    } else {
                        assert_eq!(code, "xai_transcript_safety_limit");
                    }
                }
                controller.handle(event);
            }
            assert!(
                controller.state.subtitles.history.is_empty(),
                "{scenario}: C must never borrow discarded B's response"
            );
            assert_eq!(
                recovery_errors, 1,
                "{scenario}: recover once when response ownership is ambiguous"
            );
            let turn = client.inner.turn.lock().await;
            assert!(
                turn.source.is_empty()
                    && turn.translation.is_empty()
                    && turn.pending_next_source.is_none()
            );
            drop(turn);
            client.disconnect().await;
        }
    }

    #[tokio::test]
    async fn progress_transcripts_wait_for_actual_completion_after_response_done() {
        let (complete_sent, complete_received) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                for event in [
                    r#"{"type":"input_audio_buffer.committed","item_id":"source_a"}"#,
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Partial","status":"in_progress"}"#,
                    r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                    r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"Complete translation."}"#,
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Partial source","status":"in_progress"}"#,
                    r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#,
                ] { socket.send(Message::Text(event.into())).await.unwrap(); }
                complete_received.await.unwrap();
                socket.send(Message::Text(r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Actual complete source.","status":"completed"}"#.into())).await.unwrap();
                while socket.next().await.is_some() {}
            })
        }).await;
        let mut controller = crate::core::session::TranslationSessionController::default();
        controller.set_atomic_preview(false);
        controller.did_connect();
        client.connect().await.unwrap();
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), events.recv()).await
        {
            controller.handle(event);
        }
        assert!(
            controller.state.subtitles.history.is_empty(),
            "in_progress source must not confirm at response.done"
        );
        assert!(!client.inner.turn.lock().await.source_complete);
        complete_sent.send(()).unwrap();
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), events.recv()).await
        {
            controller.handle(event);
        }
        assert_eq!(controller.state.subtitles.history.len(), 1);
        let pair = controller.state.subtitles.display_pair.as_ref().unwrap();
        assert_eq!(pair.source, "Actual complete source.");
        assert_eq!(pair.translation, "Complete translation.");
        client.disconnect().await;
    }

    #[tokio::test]
    async fn progress_only_sources_never_become_durable_when_finishing() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                for event in [
                    r#"{"type":"input_audio_buffer.committed","item_id":"source_a"}"#,
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Partial","status":"in_progress"}"#,
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Partial source","status":"in_progress"}"#,
                    r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                    r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"Unconfirmed translation."}"#,
                    r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#,
                ] { socket.send(Message::Text(event.into())).await.unwrap(); }
                while socket.next().await.is_some() {}
            })
        }).await;
        client.connect().await.unwrap();
        let mut controller = crate::core::session::TranslationSessionController::default();
        controller.did_connect();
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), events.recv()).await
        {
            controller.handle(event);
        }
        client.finish(Duration::from_millis(40)).await;
        while let Ok(event) = events.try_recv() {
            controller.handle(event);
        }
        assert!(controller.state.subtitles.history.is_empty());
        assert!(matches!(
            controller.state.status,
            crate::core::models::SessionStatus::Error(_)
        ));
    }

    #[tokio::test]
    async fn clear_with_a_new_source_before_the_cleared_response_recovers_without_cross_pairing() {
        let (clear_sent, clear_received) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                for event in [
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"Source A."}"#,
                    r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                    r#"{"type":"input_audio_buffer.speech_started","item_id":"source_b"}"#,
                    r#"{"type":"conversation.item.input_audio_transcription.updated","item_id":"source_b","transcript":"Source B growing"}"#,
                ] { socket.send(Message::Text(event.into())).await.unwrap(); }
                clear_received.await.unwrap();
                for event in [
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_c","transcript":"Source C."}"#,
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_b","transcript":"Source B."}"#,
                    r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#,
                    r#"{"type":"response.created","response":{"id":"response_b"}}"#,
                    r#"{"type":"response.output_audio_transcript.done","response_id":"response_b","transcript":"Translation B."}"#,
                    r#"{"type":"response.done","response":{"id":"response_b","status":"completed"}}"#,
                ] { if socket.send(Message::Text(event.into())).await.is_err() { break; } }
                while socket.next().await.is_some() {}
            })
        }).await;
        let mut controller = crate::core::session::TranslationSessionController::default();
        controller.set_atomic_preview(false);
        controller.did_connect();
        client.connect().await.unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap()
                .unwrap();
            let pending_b = matches!(&event, LiveTranslateServerEvent::UtteranceText { utterance_id, role:UtteranceRole::Source, .. } if utterance_id == "source_b");
            controller.handle(event);
            if pending_b {
                break;
            }
        }
        client.clear_content().await;
        controller.clear_subtitles();
        clear_sent.send(()).unwrap();
        let mut recovery_errors = 0;
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), events.recv()).await
        {
            if let LiveTranslateServerEvent::Error { code, message } = &event {
                assert_eq!(code, "transport_error");
                assert_eq!(message, GENERIC_PROTOCOL_ERROR);
                recovery_errors += 1;
            }
            controller.handle(event);
        }
        assert!(
            controller.state.subtitles.history.is_empty(),
            "C cannot borrow the cleared B response"
        );
        assert_eq!(recovery_errors, 1);
        client.disconnect().await;

        // The normal reconnect resets only bounded provider state. The same
        // real C item can then preview and confirm with its own response.
        let (reconnected, mut new_events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                for event in [
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_c","transcript":"Source C."}"#,
                    r#"{"type":"response.created","response":{"id":"response_c"}}"#,
                    r#"{"type":"response.output_audio_transcript.done","response_id":"response_c","transcript":"Translation C."}"#,
                    r#"{"type":"response.done","response":{"id":"response_c","status":"completed"}}"#,
                ] { socket.send(Message::Text(event.into())).await.unwrap(); }
                while socket.next().await.is_some() {}
            })
        }).await;
        controller.begin_connecting();
        controller.did_connect();
        reconnected.connect().await.unwrap();
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), new_events.recv()).await
        {
            controller.handle(event);
        }
        assert_eq!(controller.state.subtitles.history.len(), 1);
        let pair = controller.state.subtitles.display_pair.as_ref().unwrap();
        assert_eq!(pair.utterance_id.as_deref(), Some("source_c"));
        assert_eq!(pair.source, "Source C.");
        assert_eq!(pair.translation, "Translation C.");
        reconnected.disconnect().await;
    }

    #[tokio::test]
    async fn ambiguous_following_items_trigger_one_recovery_without_publishing_a_wrong_final() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                for event in [
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"First source."}"#,
                    r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_b","transcript":"Second source."}"#,
                    r#"{"type":"input_audio_buffer.speech_started","item_id":"source_c"}"#,
                    r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"First translation."}"#,
                    r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#,
                ] {
                    if socket.send(Message::Text(event.into())).await.is_err() { break; }
                }
                while socket.next().await.is_some() {}
            })
        }).await;
        client.connect().await.unwrap();
        let mut recovery_errors = 0;
        let mut pairs = Vec::new();
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), events.recv()).await
        {
            match event {
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. } => pairs.push(event),
                LiveTranslateServerEvent::Error { code, message } => {
                    assert_eq!(code, "transport_error");
                    assert_eq!(message, GENERIC_PROTOCOL_ERROR);
                    assert!(crate::core::recovery::provider_error_is_retryable(&code));
                    recovery_errors += 1;
                }
                _ => {}
            }
        }
        assert_eq!(recovery_errors, 1);
        assert!(pairs.is_empty());
        let turn = client.inner.turn.lock().await;
        assert!(turn.source.is_empty() && turn.translation.is_empty());
        assert!(turn.pending_next_source.is_none());
        drop(turn);
        client.disconnect().await;
    }

    #[tokio::test]
    async fn an_old_response_done_cannot_close_before_trailing_audio_gets_its_input_boundary() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                let _first_audio = socket.next().await;
                for event in [
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_a","transcript":"First source."}"#,
                    r#"{"type":"response.created","response":{"id":"response_a"}}"#,
                    r#"{"type":"response.output_audio_transcript.done","response_id":"response_a","transcript":"First translation."}"#,
                ] { socket.send(Message::Text(event.into())).await.unwrap(); }
                let _trailing_audio = socket.next().await;
                socket.send(Message::Text(r#"{"type":"response.done","response":{"id":"response_a","status":"completed"}}"#.into())).await.unwrap();
                // Sending audio and completing A do not prove the server has
                // delivered B's VAD/input events. Delay them until after tail.
                for _ in 0..FINISH_TAIL_FRAME_COUNT {
                    let _tail = socket.next().await;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
                for event in [
                    r#"{"type":"input_audio_buffer.speech_started","item_id":"source_b"}"#,
                    r#"{"type":"input_audio_buffer.committed","item_id":"source_b"}"#,
                    r#"{"type":"response.created","response":{"id":"response_b"}}"#,
                    r#"{"type":"response.output_audio_transcript.done","response_id":"response_b","transcript":"Second translation."}"#,
                    r#"{"type":"response.done","response":{"id":"response_b","status":"completed"}}"#,
                    r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source_b","transcript":"Second source."}"#,
                ] { socket.send(Message::Text(event.into())).await.unwrap(); }
                while socket.next().await.is_some() {}
            })
        }).await;
        client.connect().await.unwrap();
        client
            .send_audio(&vec![1; XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT * 2])
            .await
            .unwrap();
        client.finish(Duration::from_millis(250)).await;
        let mut pairs = Vec::new();
        let mut finished = false;
        while let Ok(event) = events.try_recv() {
            match event {
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                    source,
                    translation,
                    ..
                } => pairs.push((source, translation)),
                LiveTranslateServerEvent::SessionFinished => finished = true,
                LiveTranslateServerEvent::Error { code, .. } => panic!("unexpected error: {code}"),
                _ => {}
            }
        }
        assert_eq!(
            pairs,
            [
                ("First source.".into(), "First translation.".into()),
                ("Second source.".into(), "Second translation.".into())
            ]
        );
        assert!(finished);
    }

    #[tokio::test]
    async fn disconnect_cancels_a_finish_wait_without_waiting_for_its_deadline() {
        let (tails_sent, tails_received) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket.send(setup_ack()).await.unwrap();
                for _ in 0..FINISH_TAIL_FRAME_COUNT {
                    let _ = socket.next().await;
                }
                let _ = tails_sent.send(());
                while socket.next().await.is_some() {}
            })
        })
        .await;
        client.connect().await.unwrap();
        let finishing_client = client.clone();
        let finish = tokio::spawn(async move {
            finishing_client.finish(Duration::from_secs(5)).await;
        });
        tails_received.await.unwrap();
        tokio::time::timeout(Duration::from_millis(500), async {
            client.disconnect().await;
            finish.await.unwrap();
        })
        .await
        .expect("cancellation must release the finish wait immediately");
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(
                event,
                LiveTranslateServerEvent::Error { .. }
                    | LiveTranslateServerEvent::SessionFinished
                    | LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
            ));
        }
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
                assert!(update["session"]["turn_detection"].get("silence_duration_ms").is_none());
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
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { source, translation, .. }
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

                for _ in 0..FINISH_TAIL_FRAME_COUNT {
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
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { source, translation, .. }
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
                socket.send(Message::Text(r#"{"type":"input_audio_buffer.speech_started","item_id":"unfinished_source"}"#.into())).await.unwrap();
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
