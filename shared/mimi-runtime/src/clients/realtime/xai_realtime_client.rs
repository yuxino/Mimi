//! xAI Grok Voice WebSocket adapter for turn-based translation.

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::core::models::{TargetLanguage, UtteranceRole};
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::realtime::xai_realtime::{
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
    Rejected(XAIRealtimeClientError),
}

fn setup_provider_failure(code: &str) -> XAIRealtimeClientError {
    match code {
        "invalid_api_key"
        | "authentication_error"
        | "authorization_error"
        | "unauthorized"
        | "forbidden"
        | "permission_denied" => XAIRealtimeClientError::AuthenticationFailed,
        _ => XAIRealtimeClientError::SessionSetupRejected,
    }
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
    network: crate::clients::transport::provider_network::ProviderNetwork,
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
        network: crate::clients::transport::provider_network::ProviderNetwork,
    ) -> Result<(), crate::clients::transport::provider_network::ProviderNetworkError> {
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
            network: crate::clients::transport::provider_network::ProviderNetwork::default(),
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
            crate::clients::transport::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| XAIRealtimeClientError::TransportFailure)?
        .map_err(|error| {
            if crate::clients::transport::connection_diagnostics::authentication_rejected(&error) {
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
            setup: setup_tx,
            setup_event_id: setup_event_id.clone(),
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
                    SetupState::Rejected(error) => return Err(error),
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
    setup: watch::Sender<SetupState>,
    setup_event_id: String,
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
        let failure = if code == "transport_error" {
            XAIRealtimeClientError::TransportFailure
        } else {
            XAIRealtimeClientError::SessionSetupRejected
        };
        let _ = context.setup.send(SetupState::Rejected(failure));
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
                if *context.setup.borrow() == SetupState::Awaiting {
                    let _ = context.setup.send(SetupState::Rejected(
                        XAIRealtimeClientError::SessionSetupRejected,
                    ));
                } else {
                    emit_if_current(
                        context,
                        LiveTranslateServerEvent::Error {
                            code: "xai_session_configuration_mismatch".into(),
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
            if *context.setup.borrow() == SetupState::Awaiting {
                if is_recoverable
                    && related_event_id
                        .as_deref()
                        .is_some_and(|id| id != context.setup_event_id)
                {
                    // An explicit error for another request does not reject
                    // our pending session.update acknowledgement.
                    return false;
                }
                // Before readiness, a provider rejection is not a missing ack.
                // Keep its finite typed category when the related id is
                // missing or identifies this session.update.
                let _ = context
                    .setup
                    .send(SetupState::Rejected(setup_provider_failure(&code)));
                return true;
            }
            if !is_recoverable {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: if setup_provider_failure(&code)
                            == XAIRealtimeClientError::AuthenticationFailed
                        {
                            "credential_authentication_failed".into()
                        } else {
                            format!("xai_provider_error.{code}")
                        },
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
mod tests;
