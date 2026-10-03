//! Live-translate WebSocket client (`qwen3.5-livetranslate-flash-realtime`).

use crate::clients::provider_events::ProviderEventSender;
use crate::core::diagnostics::{milliseconds, TranslationLatency, TranslationLatencyKind};
use crate::core::models::{SourceLanguage, TargetLanguage, UtteranceRole};
use crate::core::protocols::live_translate::{
    LiveTranslateEndpoint, LiveTranslateEventIdentity, LiveTranslateRequestEncoder,
    LiveTranslateServerEvent,
};
use crate::pipeline_log;
use futures_util::{SinkExt, StreamExt};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SETUP_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(250);
const GENERIC_TRANSPORT_ERROR: &str = "The live translation connection closed.";
const GENERIC_PROTOCOL_ERROR: &str = "The live translation service returned invalid data.";
const MAX_TRACKED_ITEMS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LiveTranslateClientError {
    #[error("credential_authentication_failed")]
    AuthenticationFailed,
    #[error("Add an Alibaba Cloud Model Studio API key in Settings.")]
    MissingAPIKey,
    #[error("The live translation session is not connected.")]
    NotConnected,
    #[error("The live translation connection stopped responding.")]
    HealthCheckTimedOut,
    #[error("The live translation connection could not be established in time.")]
    ConnectionTimedOut,
    #[error("The live translation session setup timed out.")]
    SessionSetupTimedOut,
    #[error("The live translation session setup was rejected.")]
    SessionSetupRejected,
    #[error("The live translation transport failed.")]
    TransportFailure,
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
    content_lock: Mutex<()>,
    aligner: Mutex<LiveTranslatePairAligner>,
    sink: Mutex<Option<Sink>>,
    received_session_finished: AtomicBool,
    pong_notify: Notify,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    translation_latency: Arc<std::sync::Mutex<StreamTranslationLatency>>,
}

#[derive(Default)]
struct StreamTranslationLatency {
    generation: u64,
    value: Option<TranslationLatency>,
}

// Keep only monotonic timings and fixed labels. Dropping a send during
// recovery must still expose which phase was waiting, without logging the
// message, request, URL, credentials, or transport error.
struct SendTiming {
    started_at: Instant,
    sink_locked_at: Option<Instant>,
    finished_at: Option<Instant>,
    outcome: &'static str,
}

impl SendTiming {
    fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            sink_locked_at: None,
            finished_at: None,
            outcome: "cancelled",
        }
    }

    fn slow_phases_at(&self, now: Instant) -> Option<(u64, u64)> {
        let end = self.finished_at.unwrap_or(now);
        let lock_ms = milliseconds(self.started_at, self.sink_locked_at.unwrap_or(end));
        let send_ms = self
            .sink_locked_at
            .map_or(0, |start| milliseconds(start, end));
        (lock_ms > 100 || send_ms > 100).then_some((lock_ms, send_ms))
    }
}

impl Drop for SendTiming {
    fn drop(&mut self) {
        if let Some((lock_ms, send_ms)) = self.slow_phases_at(Instant::now()) {
            pipeline_log!(
                "live translate send slow sinkLockWaitMs={} socketSendMs={} sinkAcquired={} outcome={}",
                lock_ms,
                send_ms,
                self.sink_locked_at.is_some(),
                self.outcome
            );
        }
    }
}

/// An async client whose receive loop emits decoded server events onto the
/// session event channel. `disconnect` is idempotent and cancels the task.
#[derive(Clone)]
pub struct LiveTranslateClient {
    network: super::provider_network::ProviderNetwork,
    inner: Arc<Inner>,
    endpoint: LiveTranslateEndpoint,
    api_key: String,
    source_language: SourceLanguage,
    target_language: TargetLanguage,
    hotwords: BTreeMap<String, String>,
    events: ProviderEventSender,
}

impl LiveTranslateClient {
    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    pub async fn clear_content(&self) -> u64 {
        let _content = self.inner.content_lock.lock().await;
        self.inner.aligner.lock().await.clear_content();
        self.inner.translation_latency.lock().unwrap().value = None;
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
        hotwords: BTreeMap<String, String>,
        events: ProviderEventSender,
    ) -> Result<Self, LiveTranslateClientError> {
        let trimmed_key = api_key.trim();
        if trimmed_key.is_empty() {
            return Err(LiveTranslateClientError::MissingAPIKey);
        }
        Ok(Self {
            network: super::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                content_lock: Mutex::new(()),
                aligner: Mutex::new(LiveTranslatePairAligner::default()),
                sink: Mutex::new(None),
                received_session_finished: AtomicBool::new(false),
                pong_notify: Notify::new(),
                receive_task: Mutex::new(None),
                translation_latency: Default::default(),
            }),
            endpoint: LiveTranslateEndpoint::new()
                .map_err(|_| LiveTranslateClientError::MissingAPIKey)?,
            api_key: trimmed_key.to_string(),
            source_language,
            target_language,
            hotwords,
            events,
        })
    }

    /// Opens the socket, sends `session.update`, and waits for the matching
    /// server readiness acknowledgement.
    pub async fn connect(&self) -> Result<(), LiveTranslateClientError> {
        self.disconnect().await;
        let latency_generation = self.inner.translation_latency.lock().unwrap().generation;

        let mut request = self
            .endpoint
            .url
            .clone()
            .into_client_request()
            .map_err(|_| LiveTranslateClientError::NotConnected)?;
        let auth = format!("Bearer {}", self.api_key);
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&auth).map_err(|_| LiveTranslateClientError::MissingAPIKey)?,
        );

        let (socket, _response) = tokio::time::timeout(
            CONNECT_TIMEOUT,
            super::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| LiveTranslateClientError::ConnectionTimedOut)?
        .map_err(|error| {
            if super::connection_diagnostics::authentication_rejected(&error) {
                LiveTranslateClientError::AuthenticationFailed
            } else {
                LiveTranslateClientError::TransportFailure
            }
        })?;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner
            .received_session_finished
            .store(false, Ordering::SeqCst);

        let (setup_tx, setup_rx) = watch::channel(SetupState::Awaiting);
        let task = tokio::spawn(receive_loop(
            stream,
            Arc::clone(&self.inner),
            self.events.clone(),
            setup_tx,
            latency_generation,
        ));
        *self.inner.receive_task.lock().await = Some(task);

        let update = LiveTranslateRequestEncoder::session_update(
            self.source_language,
            self.target_language,
            &self.hotwords,
            None,
        )
        .expect("session.update encoding cannot fail");
        let setup = async {
            self.send_text(update.to_string()).await?;
            wait_for_setup(setup_rx).await
        };
        let result = match tokio::time::timeout(SETUP_TIMEOUT, setup).await {
            Ok(result) => result,
            Err(_) => Err(LiveTranslateClientError::SessionSetupTimedOut),
        };
        if result.is_err() {
            self.disconnect().await;
        }
        result
    }

    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), LiveTranslateClientError> {
        if pcm_data.is_empty() {
            return Ok(());
        }
        let message = LiveTranslateRequestEncoder::audio_append(pcm_data, None)
            .expect("audio append encoding cannot fail");
        self.send_text(message.to_string()).await
    }

    /// Sends a WebSocket ping and waits for the matching pong (4s timeout).
    pub async fn ping(&self, timeout: Duration) -> Result<(), LiveTranslateClientError> {
        let operation = async {
            let pong = self.inner.pong_notify.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(LiveTranslateClientError::NotConnected);
                };
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| LiveTranslateClientError::TransportFailure)?;
            }
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| LiveTranslateClientError::HealthCheckTimedOut)?
    }

    /// Waiting after the same utterance's source final arrives until its
    /// translation final arrives. A translation that arrived first needs 0ms.
    pub fn translation_latency(&self) -> Option<TranslationLatency> {
        if !self.target_language.translates_audio() {
            return None;
        }
        self.inner.translation_latency.lock().unwrap().value
    }

    /// Sends `session.finish`, waits briefly for `session.finished`, then
    /// disconnects.
    pub async fn finish(&self, timeout: Duration) {
        if self.inner.sink.lock().await.is_none() {
            return;
        }
        if let Ok(message) = LiveTranslateRequestEncoder::finish(None) {
            let _ = self.send_text(message.to_string()).await;
        }
        let deadline = tokio::time::Instant::now() + timeout;
        while !self.inner.received_session_finished.load(Ordering::SeqCst)
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.disconnect().await;
    }

    pub async fn disconnect(&self) {
        {
            let mut latency = self.inner.translation_latency.lock().unwrap();
            latency.generation = latency.generation.wrapping_add(1);
            latency.value = None;
        }
        if let Some(task) = self.inner.receive_task.lock().await.take() {
            task.abort();
        }
        let sink = self.inner.sink.lock().await.take();
        if let Some(mut sink) = sink {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.close()).await;
        }
        self.inner
            .received_session_finished
            .store(false, Ordering::SeqCst);
    }

    async fn send_text(&self, text: String) -> Result<(), LiveTranslateClientError> {
        let mut timing = SendTiming::new(Instant::now());
        let operation = async {
            let mut sink = self.inner.sink.lock().await;
            timing.sink_locked_at = Some(Instant::now());
            let Some(sink) = sink.as_mut() else {
                return Err(LiveTranslateClientError::NotConnected);
            };
            let evidence = crate::development_audio::begin_json(&text, 16_000);
            evidence
                .observe(sink.send(Message::Text(text.into())))
                .await
                .map_err(|_| LiveTranslateClientError::TransportFailure)
        };
        let result = tokio::time::timeout(SEND_TIMEOUT, operation).await;
        timing.finished_at = Some(Instant::now());
        timing.outcome = match &result {
            Ok(Ok(())) => "sent",
            Ok(Err(_)) => "failed",
            Err(_) => "timeout",
        };
        result.map_err(|_| LiveTranslateClientError::TransportFailure)?
    }
}

async fn wait_for_setup(
    mut setup: watch::Receiver<SetupState>,
) -> Result<(), LiveTranslateClientError> {
    loop {
        match *setup.borrow() {
            SetupState::Ready => return Ok(()),
            SetupState::Rejected => return Err(LiveTranslateClientError::SessionSetupRejected),
            SetupState::Awaiting => {}
        }
        if setup.changed().await.is_err() {
            return Err(LiveTranslateClientError::SessionSetupRejected);
        }
    }
}

/// One utterance being paired: its recognized text, whether that text is the
/// authoritative final, and the translation final of the response answering it.
#[derive(Default)]
struct TrackedUtterance {
    source_text: String,
    source_language: Option<String>,
    source_final: bool,
    translation_final: Option<String>,
    source_final_at: Option<Instant>,
    translation_final_at: Option<Instant>,
}

/// Pairs live-translate recognition and translation finals by the provider's own
/// conversation-item identity.
///
/// The protocol streams both sides independently and the two finals of one
/// utterance arrive tens of milliseconds apart in either order, so arrival order
/// cannot identify an utterance. Drafts never pass through this type: they keep
/// streaming to the session exactly as the provider emits them, and only the
/// durable pair waits for both authoritative finals. Tracking follows the
/// recognition stream. Pending items have a fixed capacity so delayed finals
/// can still pair after the next source begins without unbounded retention.
#[derive(Default)]
struct LiveTranslatePairAligner {
    // Fixed-size identity tombstones only; never retain cleared subtitle text.
    discarded_items: VecDeque<String>,
    /// The utterance the recognition stream is currently filling.
    current_source_id: Option<String>,
    utterances: HashMap<String, TrackedUtterance>,
    utterance_order: VecDeque<String>,
    /// Response item id -> the input item it answers.
    responses: HashMap<String, String>,
    response_order: VecDeque<String>,
    translation_latency: Arc<std::sync::Mutex<StreamTranslationLatency>>,
    latency_generation: u64,
}

impl LiveTranslatePairAligner {
    fn discard_item(&mut self, item: String) {
        if !self.discarded_items.contains(&item) {
            self.discarded_items.push_back(item);
            while self.discarded_items.len() > MAX_TRACKED_ITEMS {
                self.discarded_items.pop_front();
            }
        }
    }

    fn clear_content(&mut self) {
        let old_ids: Vec<_> = self
            .utterance_order
            .iter()
            .chain(self.response_order.iter())
            .chain(self.current_source_id.iter())
            .cloned()
            .collect();
        for item in old_ids {
            self.discard_item(item);
        }
        self.current_source_id = None;
        self.utterances.clear();
        self.utterance_order.clear();
        self.responses.clear();
        self.response_order.clear();
    }

    /// Observes one decoded frame and returns the events to forward downstream.
    fn observe(
        &mut self,
        event: &LiveTranslateServerEvent,
        identity: &LiveTranslateEventIdentity,
    ) -> Vec<LiveTranslateServerEvent> {
        self.observe_at(event, identity, Instant::now())
    }

    fn observe_at(
        &mut self,
        event: &LiveTranslateServerEvent,
        identity: &LiveTranslateEventIdentity,
        received_at: Instant,
    ) -> Vec<LiveTranslateServerEvent> {
        if crate::clients::provider_events::is_content_event(event)
            && identity
                .item_id
                .as_ref()
                .is_some_and(|item| self.discarded_items.contains(item))
        {
            return Vec::new();
        }
        // A created item links a response item to the input item it answers.
        if let (true, Some(item_id), Some(previous_item_id)) = (
            matches!(event, LiveTranslateServerEvent::Ignored { kind } if kind == "conversation.item.created"),
            identity.item_id.as_deref(),
            identity.previous_item_id.as_deref(),
        ) {
            if !self.responses.contains_key(item_id) {
                self.response_order.push_back(item_id.to_string());
            }
            self.responses
                .insert(item_id.to_string(), previous_item_id.to_string());
            while self.responses.len() > MAX_TRACKED_ITEMS {
                if let Some(oldest) = self.response_order.pop_front() {
                    self.responses.remove(&oldest);
                }
            }
            return Vec::new();
        }

        match event {
            LiveTranslateServerEvent::SourceDraft { text, language }
            | LiveTranslateServerEvent::SourceFinal { text, language } => {
                let Some(item_id) = identity.item_id.as_deref() else {
                    return vec![event.clone()];
                };
                // A new source can legitimately follow an old conversation item.
                // Only an actual translation may use the response -> source link.
                self.responses.remove(item_id);
                self.response_order
                    .retain(|item| self.responses.contains_key(item));
                let is_final = matches!(event, LiveTranslateServerEvent::SourceFinal { .. });
                let mut events = self.start_utterance(item_id);
                let utterance = self.track(item_id);
                utterance.source_text = text.clone();
                utterance.source_language = language.clone();
                utterance.source_final |= is_final;
                if is_final && utterance.source_final_at.is_none() {
                    utterance.source_final_at = Some(received_at);
                }
                events.push(LiveTranslateServerEvent::UtteranceText {
                    utterance_id: item_id.to_string(),
                    role: UtteranceRole::Source,
                    text: text.clone(),
                    is_final,
                    language: language.clone(),
                });
                if is_final {
                    events.extend(self.take_pair(item_id, false));
                }
                events
            }
            LiveTranslateServerEvent::TranslationDraft(text) => {
                let Some(response_id) = identity.item_id.as_deref() else {
                    return vec![event.clone()];
                };
                let Some(source_id) = self.responses.get(response_id).cloned() else {
                    return vec![event.clone()];
                };
                if self.discarded_items.contains(&source_id) {
                    return Vec::new();
                }
                vec![LiveTranslateServerEvent::UtteranceText {
                    utterance_id: source_id,
                    role: UtteranceRole::Translation,
                    text: text.clone(),
                    is_final: false,
                    language: None,
                }]
            }
            LiveTranslateServerEvent::TranslationFinal(text) => {
                let Some(response_id) = identity.item_id.as_deref() else {
                    // Without identity the legacy best-effort path still applies.
                    return vec![event.clone()];
                };
                let Some(source_id) = self.responses.get(response_id).cloned() else {
                    // A known response with no source link must not fall back
                    // to arrival-order pairing with another utterance.
                    return Vec::new();
                };
                if self.discarded_items.contains(&source_id) {
                    return Vec::new();
                }
                // An utterance the recognition stream has already left can no
                // longer receive its final, so its own text is the best source.
                let allow_draft_source =
                    matches!(&self.current_source_id, Some(current) if current != &source_id);
                let utterance = self.track(&source_id);
                utterance.translation_final = Some(text.clone());
                utterance.translation_final_at = Some(received_at);
                self.take_pair(&source_id, allow_draft_source)
            }
            // A graceful close is the last point where an unmatched translation
            // can be paired with the text its own utterance produced.
            LiveTranslateServerEvent::SessionFinished => {
                let mut events = self
                    .current_source_id
                    .clone()
                    .map(|source_id| self.flush(&source_id))
                    .unwrap_or_default();
                events.push(event.clone());
                events
            }
            _ => vec![event.clone()],
        }
    }

    /// Moves the recognition stream to `source_id`. Keep a previous source
    /// without a translation so a late response can still find its own text.
    fn start_utterance(&mut self, source_id: &str) -> Vec<LiveTranslateServerEvent> {
        if self.current_source_id.as_deref() == Some(source_id) {
            return Vec::new();
        }
        let previous = self.current_source_id.replace(source_id.to_string());
        previous
            .filter(|source_id| {
                self.utterances
                    .get(source_id)
                    .is_some_and(|utterance| utterance.translation_final.is_some())
            })
            .map(|source_id| self.flush(&source_id))
            .unwrap_or_default()
    }

    fn flush(&mut self, source_id: &str) -> Vec<LiveTranslateServerEvent> {
        let pair = self.take_pair(source_id, true);
        self.retire(source_id);
        pair
    }

    /// Emits the pair for one utterance once both sides are authoritative. An
    /// empty translation or an empty recognized result retires the utterance
    /// without a pair and never consumes another utterance's text.
    fn take_pair(
        &mut self,
        source_id: &str,
        allow_draft_source: bool,
    ) -> Vec<LiveTranslateServerEvent> {
        let Some(utterance) = self.utterances.get(source_id) else {
            return Vec::new();
        };
        let Some(translation) = utterance.translation_final.clone() else {
            return Vec::new();
        };
        if !utterance.source_final && !allow_draft_source {
            return Vec::new();
        }
        let source = utterance.source_text.trim().to_string();
        let language = utterance.source_language.clone();
        let measured_latency = utterance
            .source_final_at
            .zip(utterance.translation_final_at)
            .map(|(source_at, translation_at)| TranslationLatency {
                milliseconds: milliseconds(source_at, translation_at),
                kind: TranslationLatencyKind::Follow,
            });
        self.retire(source_id);
        if source.is_empty() || translation.trim().is_empty() {
            return Vec::new();
        }
        {
            let mut latency = self.translation_latency.lock().unwrap();
            if latency.generation == self.latency_generation {
                latency.value = measured_latency;
            }
        }
        vec![LiveTranslateServerEvent::SubtitleFinalPair {
            source,
            language,
            translation: translation.trim().to_string(),
        }]
    }

    fn track(&mut self, source_id: &str) -> &mut TrackedUtterance {
        if !self.utterances.contains_key(source_id) {
            self.utterance_order.push_back(source_id.to_string());
            while self.utterance_order.len() > MAX_TRACKED_ITEMS {
                if let Some(oldest) = self.utterance_order.pop_front() {
                    self.retire(&oldest);
                }
            }
        }
        self.utterances.entry(source_id.to_string()).or_default()
    }

    fn retire(&mut self, source_id: &str) {
        self.utterances.remove(source_id);
        self.utterance_order.retain(|item| item != source_id);
        self.responses.remove(source_id);
        self.responses.retain(|_, source| source != source_id);
        self.response_order
            .retain(|item| self.responses.contains_key(item));
    }
}

async fn receive_loop(
    mut stream: Stream,
    inner: Arc<Inner>,
    events: ProviderEventSender,
    setup: watch::Sender<SetupState>,
    latency_generation: u64,
) {
    {
        let _content = inner.content_lock.lock().await;
        *inner.aligner.lock().await = LiveTranslatePairAligner {
            translation_latency: Arc::clone(&inner.translation_latency),
            latency_generation,
            ..Default::default()
        };
    }
    while let Some(message) = stream.next().await {
        let decoded = match message {
            Ok(Message::Text(text)) => LiveTranslateServerEvent::decode_with_identity(&text),
            Ok(Message::Binary(data)) => {
                LiveTranslateServerEvent::decode_with_identity(&String::from_utf8_lossy(&data))
            }
            Ok(Message::Pong(_)) => {
                inner.pong_notify.notify_waiters();
                continue;
            }
            Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => {
                if should_report_transport_end(
                    inner.received_session_finished.load(Ordering::SeqCst),
                ) {
                    fail_receive_loop(&events, &setup, "transport_error", GENERIC_TRANSPORT_ERROR);
                }
                return;
            }
        };

        let (event, identity) = match decoded {
            Ok(decoded) => decoded,
            Err(_) => {
                fail_receive_loop(
                    &events,
                    &setup,
                    "live_translate_protocol_error",
                    GENERIC_PROTOCOL_ERROR,
                );
                return;
            }
        };
        let revision = events.content_revision();
        let _content = inner.content_lock.lock().await;
        if revision != events.content_revision()
            && crate::clients::provider_events::is_content_event(&event)
        {
            continue;
        }
        for event in inner.aligner.lock().await.observe(&event, &identity) {
            if !emit_server_event(&inner, &events, &setup, event) {
                return;
            }
        }
    }

    if should_report_transport_end(inner.received_session_finished.load(Ordering::SeqCst)) {
        fail_receive_loop(&events, &setup, "transport_error", GENERIC_TRANSPORT_ERROR);
    }
}

fn emit_server_event(
    inner: &Inner,
    events: &ProviderEventSender,
    setup: &watch::Sender<SetupState>,
    event: LiveTranslateServerEvent,
) -> bool {
    if event == LiveTranslateServerEvent::SessionFinished {
        inner
            .received_session_finished
            .store(true, Ordering::SeqCst);
    }
    if *setup.borrow() == SetupState::Awaiting {
        if matches!(event, LiveTranslateServerEvent::Error { .. }) {
            let _ = setup.send(SetupState::Rejected);
            return false;
        }
        if event == LiveTranslateServerEvent::SessionUpdated {
            // Publishing the acknowledgement also proves the bounded session
            // receiver is still alive before setup is marked ready.
            if events.send(event).is_err() {
                let _ = setup.send(SetupState::Rejected);
                return false;
            }
            return setup.send(SetupState::Ready).is_ok();
        }
    }

    // The stream protocol has no "translation started" message: the source
    // final is the reliable boundary at which translation begins.
    let source_final_arrived = matches!(
        &event,
        LiveTranslateServerEvent::SourceFinal { .. }
            | LiveTranslateServerEvent::UtteranceText {
                role: UtteranceRole::Source,
                is_final: true,
                ..
            }
    );
    if source_final_arrived
        && events
            .send(LiveTranslateServerEvent::TranslationStarted)
            .is_err()
    {
        return false;
    }
    if events.send(event).is_err() {
        if *setup.borrow() == SetupState::Awaiting {
            let _ = setup.send(SetupState::Rejected);
        }
        return false;
    }
    true
}

fn fail_receive_loop(
    events: &ProviderEventSender,
    setup: &watch::Sender<SetupState>,
    code: &str,
    message: &str,
) {
    if *setup.borrow() == SetupState::Awaiting {
        let _ = setup.send(SetupState::Rejected);
    } else {
        let _ = events.send(LiveTranslateServerEvent::Error {
            code: code.into(),
            message: message.into(),
        });
    }
}

fn should_report_transport_end(received_session_finished: bool) -> bool {
    !received_session_finished
}

#[cfg(test)]
mod tests {
    #[test]
    fn cleared_item_identity_never_filters_lifecycle_or_provider_errors() {
        let mut aligner = LiveTranslatePairAligner::default();
        aligner.observe(
            &LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic old source".into(),
                language: None,
            },
            &identity("old-source", None),
        );
        aligner.clear_content();
        for wire in [
            r#"{"type":"session.created","item_id":"old-source","previous_item_id":"old-source"}"#,
            r#"{"type":"session.updated","item_id":"old-source","previous_item_id":"old-source"}"#,
            r#"{"type":"session.finished","item_id":"old-source","previous_item_id":"old-source"}"#,
            r#"{"type":"error","item_id":"old-source","previous_item_id":"old-source","error":{"code":"synthetic_failure","message":"Synthetic fixed error"}}"#,
        ] {
            let (event, item) = LiveTranslateServerEvent::decode_with_identity(wire).unwrap();
            assert_eq!(aligner.observe(&event, &item), vec![event]);
        }
        assert!(aligner
            .observe(
                &LiveTranslateServerEvent::SourceFinal {
                    text: "Synthetic old final".into(),
                    language: None
                },
                &identity("old-source", None)
            )
            .is_empty());
    }

    #[test]
    fn session_finish_with_cleared_identity_still_flushes_a_new_valid_tail() {
        let mut aligner = LiveTranslatePairAligner::default();
        aligner.observe(
            &LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic old source".into(),
                language: None,
            },
            &identity("old-source", None),
        );
        aligner.clear_content();
        aligner.observe(
            &created_item(),
            &identity("new-response", Some("new-source")),
        );
        aligner.observe(
            &LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic new source".into(),
                language: None,
            },
            &identity("new-source", None),
        );
        assert!(aligner
            .observe(
                &LiveTranslateServerEvent::TranslationFinal("Synthetic new translation".into()),
                &identity("new-response", None)
            )
            .is_empty());
        let events = aligner.observe(
            &LiveTranslateServerEvent::SessionFinished,
            &identity("old-source", Some("old-source")),
        );
        assert!(
            matches!(events.as_slice(), [LiveTranslateServerEvent::SubtitleFinalPair {source,translation,..}, LiveTranslateServerEvent::SessionFinished]
            if source == "Synthetic new source" && translation == "Synthetic new translation")
        );
    }

    #[tokio::test]
    async fn clear_drops_shared_alignment_and_known_old_identity_without_losing_lifecycle() {
        let (sender, mut receiver) = provider_event_channel();
        let client = LiveTranslateClient::new(
            "test-key-not-real",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            BTreeMap::new(),
            sender.clone(),
        )
        .unwrap();
        {
            let mut aligner = client.inner.aligner.lock().await;
            aligner.observe(
                &created_item(),
                &identity("old-response", Some("old-source")),
            );
            aligner.observe(
                &LiveTranslateServerEvent::SourceFinal {
                    text: "old source".into(),
                    language: None,
                },
                &identity("old-source", None),
            );
        }
        sender
            .send(LiveTranslateServerEvent::SessionFinished)
            .unwrap();
        assert_eq!(client.clear_content().await, 1);
        assert_eq!(client.content_revision(), 1);
        let mut aligner = client.inner.aligner.lock().await;
        assert!(aligner.utterances.is_empty() && aligner.responses.is_empty());
        assert!(aligner
            .observe(
                &LiveTranslateServerEvent::SourceFinal {
                    text: "old replay".into(),
                    language: None
                },
                &identity("old-source", None)
            )
            .is_empty());
        assert!(aligner
            .observe(
                &LiveTranslateServerEvent::TranslationFinal("late old".into()),
                &identity("old-response", None)
            )
            .is_empty());
        assert!(aligner
            .observe(
                &created_item(),
                &identity("late-old-response", Some("old-source"))
            )
            .is_empty());
        assert!(aligner
            .observe(
                &LiveTranslateServerEvent::TranslationDraft("late old".into()),
                &identity("late-old-response", None)
            )
            .is_empty());
        aligner.observe(&created_item(), &identity("new-source", Some("old-source")));
        assert!(!aligner
            .observe(
                &LiveTranslateServerEvent::SourceDraft {
                    text: "new source".into(),
                    language: None
                },
                &identity("new-source", None)
            )
            .is_empty());
        aligner.observe(
            &created_item(),
            &identity("new-response", Some("new-source")),
        );
        aligner.observe(
            &LiveTranslateServerEvent::SourceFinal {
                text: "new source".into(),
                language: None,
            },
            &identity("new-source", None),
        );
        assert!(aligner.observe(&LiveTranslateServerEvent::TranslationFinal("new translation".into()), &identity("new-response", None)).iter().any(|event| matches!(event,
            LiveTranslateServerEvent::SubtitleFinalPair {source,translation,..} if source == "new source" && translation == "new translation")));
        for i in 0..MAX_TRACKED_ITEMS * 3 {
            aligner.discard_item(format!("old-{i}"));
        }
        assert!(aligner.discarded_items.len() <= MAX_TRACKED_ITEMS);
        drop(aligner);
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        );
    }

    use super::*;
    use crate::clients::provider_events::provider_event_channel;
    use std::collections::HashSet;

    #[test]
    fn interrupted_send_timings_distinguish_lock_wait_from_socket_wait() {
        let start = Instant::now();
        let mut timing = SendTiming::new(start);
        assert_eq!(
            timing.slow_phases_at(start + Duration::from_millis(250)),
            Some((250, 0))
        );
        timing.sink_locked_at = Some(start + Duration::from_millis(20));
        assert_eq!(
            timing.slow_phases_at(start + Duration::from_millis(250)),
            Some((20, 230))
        );
    }

    #[test]
    fn completed_send_timings_exclude_later_work_and_logs() {
        let start = Instant::now();
        let mut timing = SendTiming::new(start);
        timing.sink_locked_at = Some(start + Duration::from_millis(10));
        timing.finished_at = Some(start + Duration::from_millis(20));
        assert_eq!(timing.slow_phases_at(start + Duration::from_secs(5)), None);
        timing.finished_at = Some(start + Duration::from_millis(240));
        assert_eq!(
            timing.slow_phases_at(start + Duration::from_secs(5)),
            Some((10, 230))
        );
    }

    fn test_inner() -> Inner {
        Inner {
            content_lock: Mutex::new(()),
            aligner: Mutex::new(LiveTranslatePairAligner::default()),
            sink: Mutex::new(None),
            received_session_finished: AtomicBool::new(false),
            pong_notify: Notify::new(),
            receive_task: Mutex::new(None),
            translation_latency: Default::default(),
        }
    }

    #[tokio::test]
    async fn setup_waits_for_server_updated_acknowledgement() {
        let inner = test_inner();
        let (events, mut event_rx) = provider_event_channel();
        let (tx, rx) = watch::channel(SetupState::Awaiting);
        let mut wait = Box::pin(wait_for_setup(rx));
        assert!(tokio::time::timeout(Duration::from_millis(5), &mut wait)
            .await
            .is_err());

        assert!(emit_server_event(
            &inner,
            &events,
            &tx,
            LiveTranslateServerEvent::SessionUpdated,
        ));
        assert_eq!(wait.await, Ok(()));
        assert_eq!(
            event_rx.try_recv(),
            Ok(LiveTranslateServerEvent::SessionUpdated)
        );
    }

    #[tokio::test]
    async fn rejected_setup_fails_without_waiting_for_health_check() {
        let inner = test_inner();
        let (events, mut event_rx) = provider_event_channel();
        let (tx, rx) = watch::channel(SetupState::Awaiting);
        assert!(!emit_server_event(
            &inner,
            &events,
            &tx,
            LiveTranslateServerEvent::Error {
                code: "provider-private-code".into(),
                message: "provider-private-message".into(),
            },
        ));

        assert_eq!(
            wait_for_setup(rx).await,
            Err(LiveTranslateClientError::SessionSetupRejected)
        );
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn setup_is_rejected_when_the_bounded_session_receiver_is_gone() {
        let inner = test_inner();
        let (events, event_rx) = provider_event_channel();
        drop(event_rx);
        let (tx, rx) = watch::channel(SetupState::Awaiting);

        assert!(!emit_server_event(
            &inner,
            &events,
            &tx,
            LiveTranslateServerEvent::SessionUpdated,
        ));
        assert_eq!(
            wait_for_setup(rx).await,
            Err(LiveTranslateClientError::SessionSetupRejected)
        );
    }

    #[test]
    fn only_a_confirmed_session_finish_makes_socket_end_expected() {
        assert!(should_report_transport_end(false));
        assert!(!should_report_transport_end(true));
    }

    fn identity(item_id: &str, previous_item_id: Option<&str>) -> LiveTranslateEventIdentity {
        LiveTranslateEventIdentity {
            item_id: Some(item_id.to_string()),
            previous_item_id: previous_item_id.map(String::from),
        }
    }

    fn created_item() -> LiveTranslateServerEvent {
        LiveTranslateServerEvent::Ignored {
            kind: "conversation.item.created".into(),
        }
    }

    #[test]
    fn follow_latency_uses_its_own_final_boundaries_in_either_arrival_order() {
        let start = Instant::now();
        for (translation_first, expected_ms) in [(false, 125), (true, 0)] {
            let mut aligner = LiveTranslatePairAligner::default();
            aligner.observe_at(
                &created_item(),
                &identity("response", Some("source")),
                start,
            );
            let source = LiveTranslateServerEvent::SourceFinal {
                text: "Synthetic source.".into(),
                language: None,
            };
            let translation = LiveTranslateServerEvent::TranslationFinal("合成译文。".into());
            let (first, first_id, second, second_id) = if translation_first {
                (&translation, "response", &source, "source")
            } else {
                (&source, "source", &translation, "response")
            };
            aligner.observe_at(first, &identity(first_id, None), start);
            assert!(aligner.translation_latency.lock().unwrap().value.is_none());
            aligner.observe_at(
                second,
                &identity(second_id, None),
                start + Duration::from_millis(125),
            );
            assert_eq!(
                aligner.translation_latency.lock().unwrap().value,
                Some(TranslationLatency {
                    milliseconds: expected_ms,
                    kind: TranslationLatencyKind::Follow,
                })
            );
        }
    }

    #[test]
    fn a_late_translation_measures_its_source_instead_of_the_next_utterance() {
        let start = Instant::now();
        let mut aligner = LiveTranslatePairAligner::default();
        aligner.observe_at(
            &created_item(),
            &identity("response_old", Some("source_old")),
            start,
        );
        for (id, offset) in [("source_old", 0), ("source_new", 100)] {
            aligner.observe_at(
                &LiveTranslateServerEvent::SourceFinal {
                    text: "Synthetic source.".into(),
                    language: None,
                },
                &identity(id, None),
                start + Duration::from_millis(offset),
            );
        }
        aligner.observe_at(
            &LiveTranslateServerEvent::TranslationFinal("合成译文。".into()),
            &identity("response_old", None),
            start + Duration::from_millis(250),
        );
        assert_eq!(
            aligner.translation_latency.lock().unwrap().value,
            Some(TranslationLatency {
                milliseconds: 250,
                kind: TranslationLatencyKind::Follow,
            })
        );
    }

    #[test]
    fn fallback_and_obsolete_streams_cannot_guess_or_publish_follow_latency() {
        for obsolete in [false, true] {
            let start = Instant::now();
            let mut aligner = LiveTranslatePairAligner::default();
            aligner.observe_at(
                &created_item(),
                &identity("response", Some("source")),
                start,
            );
            let source = if obsolete {
                LiveTranslateServerEvent::SourceFinal {
                    text: "Synthetic source.".into(),
                    language: None,
                }
            } else {
                LiveTranslateServerEvent::SourceDraft {
                    text: "Synthetic source.".into(),
                    language: None,
                }
            };
            aligner.observe_at(&source, &identity("source", None), start);
            if obsolete {
                aligner.translation_latency.lock().unwrap().generation += 1;
            }
            aligner.observe_at(
                &LiveTranslateServerEvent::TranslationFinal("合成译文。".into()),
                &identity("response", None),
                start + Duration::from_millis(200),
            );
            aligner.observe_at(
                &LiveTranslateServerEvent::SessionFinished,
                &LiveTranslateEventIdentity::default(),
                start + Duration::from_millis(300),
            );
            assert!(aligner.translation_latency.lock().unwrap().value.is_none());
        }
    }

    #[test]
    fn original_mode_does_not_report_a_translation_measurement() {
        let (events, _receiver) = provider_event_channel();
        let client = LiveTranslateClient::new(
            "synthetic-key",
            SourceLanguage::English,
            TargetLanguage::Original,
            BTreeMap::new(),
            events,
        )
        .unwrap();
        client.inner.translation_latency.lock().unwrap().value = Some(TranslationLatency {
            milliseconds: 125,
            kind: TranslationLatencyKind::Follow,
        });
        assert_eq!(client.translation_latency(), None);
    }

    #[test]
    fn a_translation_final_waits_for_its_own_recognition_final() {
        let mut aligner = LiveTranslatePairAligner::default();
        assert!(aligner
            .observe(
                &created_item(),
                &identity("item_response", Some("item_source"))
            )
            .is_empty());

        // The translation usually completes first; it must not be paired yet
        // and must not fall back to the legacy best-effort path.
        assert!(aligner
            .observe(
                &LiveTranslateServerEvent::TranslationFinal("你好。".into()),
                &identity("item_response", None),
            )
            .is_empty());

        let events = aligner.observe(
            &LiveTranslateServerEvent::SourceFinal {
                text: "Hello.".into(),
                language: Some("en".into()),
            },
            &identity("item_source", None),
        );
        assert_eq!(
            events,
            vec![
                LiveTranslateServerEvent::UtteranceText {
                    utterance_id: "item_source".into(),
                    role: UtteranceRole::Source,
                    text: "Hello.".into(),
                    is_final: true,
                    language: Some("en".into()),
                },
                LiveTranslateServerEvent::SubtitleFinalPair {
                    source: "Hello.".into(),
                    language: Some("en".into()),
                    translation: "你好。".into(),
                },
            ]
        );
    }

    #[test]
    fn a_recognition_final_waits_for_its_own_translation_final() {
        let mut aligner = LiveTranslatePairAligner::default();
        aligner.observe(
            &created_item(),
            &identity("item_response", Some("item_source")),
        );

        let events = aligner.observe(
            &LiveTranslateServerEvent::SourceFinal {
                text: "Hello.".into(),
                language: None,
            },
            &identity("item_source", None),
        );
        assert_eq!(
            events,
            vec![LiveTranslateServerEvent::UtteranceText {
                utterance_id: "item_source".into(),
                role: UtteranceRole::Source,
                text: "Hello.".into(),
                is_final: true,
                language: None,
            }]
        );

        let events = aligner.observe(
            &LiveTranslateServerEvent::TranslationFinal("你好。".into()),
            &identity("item_response", None),
        );
        assert_eq!(
            events,
            vec![LiveTranslateServerEvent::SubtitleFinalPair {
                source: "Hello.".into(),
                language: None,
                translation: "你好。".into(),
            }]
        );
    }

    #[test]
    fn drafts_stream_through_untouched_while_the_pair_waits() {
        let mut aligner = LiveTranslatePairAligner::default();
        aligner.observe(
            &created_item(),
            &identity("item_response", Some("item_source")),
        );

        assert_eq!(
            aligner.observe(
                &LiveTranslateServerEvent::SourceDraft {
                    text: "Hello wor".into(),
                    language: Some("en".into()),
                },
                &identity("item_source", None),
            ),
            vec![LiveTranslateServerEvent::UtteranceText {
                utterance_id: "item_source".into(),
                role: UtteranceRole::Source,
                text: "Hello wor".into(),
                is_final: false,
                language: Some("en".into()),
            }]
        );
        // A translation draft is stamped with the source utterance it answers.
        assert_eq!(
            aligner.observe(
                &LiveTranslateServerEvent::TranslationDraft("你好".into()),
                &identity("item_response", None),
            ),
            vec![LiveTranslateServerEvent::UtteranceText {
                utterance_id: "item_source".into(),
                role: UtteranceRole::Translation,
                text: "你好".into(),
                is_final: false,
                language: None,
            }]
        );
        assert!(aligner
            .observe(
                &LiveTranslateServerEvent::TranslationFinal("你好。".into()),
                &identity("item_response", None),
            )
            .is_empty());
        assert_eq!(
            aligner.observe(
                &LiveTranslateServerEvent::SourceDraft {
                    text: "Hello world".into(),
                    language: Some("en".into()),
                },
                &identity("item_source", None),
            ),
            vec![LiveTranslateServerEvent::UtteranceText {
                utterance_id: "item_source".into(),
                role: UtteranceRole::Source,
                text: "Hello world".into(),
                is_final: false,
                language: Some("en".into()),
            }]
        );

        // The pair keeps the authoritative recognition final, never a draft.
        let events = aligner.observe(
            &LiveTranslateServerEvent::SourceFinal {
                text: "Hello world.".into(),
                language: Some("en".into()),
            },
            &identity("item_source", None),
        );
        assert_eq!(
            events,
            vec![
                LiveTranslateServerEvent::UtteranceText {
                    utterance_id: "item_source".into(),
                    role: UtteranceRole::Source,
                    text: "Hello world.".into(),
                    is_final: true,
                    language: Some("en".into()),
                },
                LiveTranslateServerEvent::SubtitleFinalPair {
                    source: "Hello world.".into(),
                    language: Some("en".into()),
                    translation: "你好。".into(),
                },
            ]
        );
    }

    #[test]
    fn an_empty_translation_final_never_consumes_another_utterance() {
        let mut aligner = LiveTranslatePairAligner::default();
        aligner.observe(
            &created_item(),
            &identity("item_response_1", Some("item_source_1")),
        );
        aligner.observe(
            &LiveTranslateServerEvent::SourceFinal {
                text: "Um.".into(),
                language: None,
            },
            &identity("item_source_1", None),
        );
        assert!(aligner
            .observe(
                &LiveTranslateServerEvent::TranslationFinal("  ".into()),
                &identity("item_response_1", None),
            )
            .is_empty());

        aligner.observe(
            &created_item(),
            &identity("item_response_2", Some("item_source_2")),
        );
        aligner.observe(
            &LiveTranslateServerEvent::SourceFinal {
                text: "I'm searching for someone.".into(),
                language: None,
            },
            &identity("item_source_2", None),
        );
        assert_eq!(
            aligner.observe(
                &LiveTranslateServerEvent::TranslationFinal("我在寻找某人。".into()),
                &identity("item_response_2", None),
            ),
            vec![LiveTranslateServerEvent::SubtitleFinalPair {
                source: "I'm searching for someone.".into(),
                language: None,
                translation: "我在寻找某人。".into(),
            }]
        );
    }

    #[test]
    fn a_missing_recognition_final_commits_with_its_own_utterance_text() {
        let mut aligner = LiveTranslatePairAligner::default();
        aligner.observe(
            &created_item(),
            &identity("item_response_1", Some("item_source_1")),
        );
        aligner.observe(
            &LiveTranslateServerEvent::SourceDraft {
                text: "Hello wor".into(),
                language: None,
            },
            &identity("item_source_1", None),
        );
        assert!(aligner
            .observe(
                &LiveTranslateServerEvent::TranslationFinal("你好。".into()),
                &identity("item_response_1", None),
            )
            .is_empty());

        // The next utterance starts while the first one never produced a
        // recognition final: its translation is committed with its own text
        // instead of being dropped or borrowing the next utterance.
        assert_eq!(
            aligner.observe(
                &LiveTranslateServerEvent::SourceDraft {
                    text: "Next utterance".into(),
                    language: None,
                },
                &identity("item_source_2", None),
            ),
            vec![
                LiveTranslateServerEvent::SubtitleFinalPair {
                    source: "Hello wor".into(),
                    language: None,
                    translation: "你好。".into(),
                },
                LiveTranslateServerEvent::UtteranceText {
                    utterance_id: "item_source_2".into(),
                    role: UtteranceRole::Source,
                    text: "Next utterance".into(),
                    is_final: false,
                    language: None,
                },
            ]
        );
    }

    #[test]
    fn a_late_translation_after_the_next_source_still_pairs_with_its_own_source() {
        let mut aligner = LiveTranslatePairAligner::default();
        aligner.observe(&created_item(), &identity("response_a", Some("source_a")));
        aligner.observe(
            &LiveTranslateServerEvent::SourceFinal {
                text: "First sentence.".into(),
                language: Some("en".into()),
            },
            &identity("source_a", None),
        );

        // Recognition has moved to B before the response for A completes.
        aligner.observe(
            &LiveTranslateServerEvent::SourceDraft {
                text: "Second sentence".into(),
                language: Some("en".into()),
            },
            &identity("source_b", None),
        );
        assert_eq!(
            aligner.observe(
                &LiveTranslateServerEvent::TranslationFinal("第一句。".into()),
                &identity("response_a", None),
            ),
            vec![LiveTranslateServerEvent::SubtitleFinalPair {
                source: "First sentence.".into(),
                language: Some("en".into()),
                translation: "第一句。".into(),
            }]
        );
    }

    /// The cross-sentence case above, for the streaming side: a draft that
    /// arrives after the recognition stream moved on is still stamped with its
    /// own source, so the bilingual preview refuses to stack it under the next
    /// sentence's original.
    #[test]
    fn a_late_translation_draft_keeps_the_identity_of_its_own_source() {
        let mut aligner = LiveTranslatePairAligner::default();
        aligner.observe(&created_item(), &identity("response_a", Some("source_a")));
        aligner.observe(
            &LiveTranslateServerEvent::SourceFinal {
                text: "First sentence.".into(),
                language: Some("en".into()),
            },
            &identity("source_a", None),
        );
        aligner.observe(
            &LiveTranslateServerEvent::SourceDraft {
                text: "Second sentence".into(),
                language: Some("en".into()),
            },
            &identity("source_b", None),
        );

        assert_eq!(
            aligner.observe(
                &LiveTranslateServerEvent::TranslationDraft("第一句".into()),
                &identity("response_a", None),
            ),
            vec![LiveTranslateServerEvent::UtteranceText {
                utterance_id: "source_a".into(),
                role: UtteranceRole::Translation,
                text: "第一句".into(),
                is_final: false,
                language: None,
            }]
        );
    }

    #[test]
    fn orphaned_response_items_cannot_grow_without_bound() {
        let mut aligner = LiveTranslatePairAligner::default();
        for index in 0..256 {
            let response = format!("response_{index}");
            let source = format!("source_{index}");
            aligner.observe(&created_item(), &identity(&response, Some(&source)));
            aligner.observe(
                &LiveTranslateServerEvent::TranslationFinal("translated".into()),
                &identity(&response, None),
            );
        }

        // The provider may omit recognition events for some response items.
        // A long-running session must retain only a bounded number of them.
        assert!(aligner.responses.len() <= 64);
        assert!(aligner.utterances.len() <= 64);
    }

    #[test]
    fn a_graceful_close_commits_the_pending_translation() {
        let mut aligner = LiveTranslatePairAligner::default();
        aligner.observe(
            &created_item(),
            &identity("item_response", Some("item_source")),
        );
        aligner.observe(
            &LiveTranslateServerEvent::SourceDraft {
                text: "Hello".into(),
                language: None,
            },
            &identity("item_source", None),
        );
        assert!(aligner
            .observe(
                &LiveTranslateServerEvent::TranslationFinal("你好。".into()),
                &identity("item_response", None),
            )
            .is_empty());

        assert_eq!(
            aligner.observe(
                &LiveTranslateServerEvent::SessionFinished,
                &LiveTranslateEventIdentity::default(),
            ),
            vec![
                LiveTranslateServerEvent::SubtitleFinalPair {
                    source: "Hello".into(),
                    language: None,
                    translation: "你好。".into(),
                },
                LiveTranslateServerEvent::SessionFinished,
            ]
        );
    }

    #[test]
    fn a_translation_final_without_identity_keeps_the_legacy_path() {
        let mut aligner = LiveTranslatePairAligner::default();
        let event = LiveTranslateServerEvent::TranslationFinal("你好。".into());
        assert_eq!(
            aligner.observe(&event, &LiveTranslateEventIdentity::default()),
            vec![event.clone()]
        );
    }

    #[test]
    fn a_long_session_keeps_only_the_current_utterance_tracked() {
        let mut aligner = LiveTranslatePairAligner::default();
        for index in 0..100 {
            let response = format!("item_response_{index}");
            let source = format!("item_source_{index}");
            aligner.observe(&created_item(), &identity(&response, Some(&source)));
            aligner.observe(
                &LiveTranslateServerEvent::SourceDraft {
                    text: format!("draft {index}"),
                    language: None,
                },
                &identity(&source, None),
            );
            aligner.observe(
                &LiveTranslateServerEvent::SourceFinal {
                    text: format!("final {index}"),
                    language: None,
                },
                &identity(&source, None),
            );
            aligner.observe(
                &LiveTranslateServerEvent::TranslationFinal(format!("translation {index}")),
                &identity(&response, None),
            );
        }

        assert!(aligner.utterances.len() <= 1);
        assert!(aligner.responses.len() <= 2);
    }

    /// Replays the captured Alibaba live-translate session the documentation
    /// harness uses: the two finals of one utterance arrive tens of
    /// milliseconds apart in either order, one utterance is never translated,
    /// and drafts must keep streaming untouched.
    #[test]
    fn the_real_alibaba_capture_pairs_every_utterance_by_identity() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docs/demos/english-film/response.json"
        ))
        .expect("the captured Alibaba response fixture is available");
        let fixture: serde_json::Value = serde_json::from_str(&raw).unwrap();

        let mut aligner = LiveTranslatePairAligner::default();
        let mut pairs = Vec::new();
        let (mut source_drafts, mut translation_drafts) = (0, 0);
        let (mut source_finals, mut translation_finals) = (0, 0);
        let mut source_utterances = HashSet::new();
        let mut translation_utterances = HashSet::new();
        for event in fixture["events"].as_array().unwrap() {
            let (decoded, identity) = LiveTranslateServerEvent::decode_value_with_identity(event)
                .expect("the capture only contains documented events");
            for forwarded in aligner.observe(&decoded, &identity) {
                match forwarded {
                    LiveTranslateServerEvent::UtteranceText {
                        utterance_id,
                        role,
                        is_final,
                        ..
                    } => {
                        match (role, is_final) {
                            (UtteranceRole::Source, false) => source_drafts += 1,
                            (UtteranceRole::Source, true) => source_finals += 1,
                            (UtteranceRole::Translation, false) => translation_drafts += 1,
                            (UtteranceRole::Translation, true) => translation_finals += 1,
                        }
                        match role {
                            UtteranceRole::Source => {
                                source_utterances.insert(utterance_id);
                            }
                            UtteranceRole::Translation => {
                                translation_utterances.insert(utterance_id);
                            }
                        }
                    }
                    LiveTranslateServerEvent::SourceDraft { .. }
                    | LiveTranslateServerEvent::SourceFinal { .. }
                    | LiveTranslateServerEvent::TranslationDraft(_)
                    | LiveTranslateServerEvent::TranslationFinal(_) => {
                        panic!("identity-carrying text must be stamped: {forwarded:?}")
                    }
                    LiveTranslateServerEvent::SubtitleFinalPair {
                        source,
                        translation,
                        ..
                    } => pairs.push((source, translation)),
                    _ => {}
                }
            }
        }

        // Drafts keep streaming exactly as the provider emits them, now stamped
        // with the utterance they belong to.
        assert_eq!((source_drafts, translation_drafts), (72, 77));
        assert_eq!((source_finals, translation_finals), (8, 0));
        assert_eq!(source_utterances.len(), 8);
        assert!(translation_utterances.is_subset(&source_utterances));
        assert_eq!(
            pairs,
            vec![
                (
                    "So. What brings you to the land of the gatekeepers?".to_string(),
                    "那么，是什么风把你吹到了守门人的国度？".to_string()
                ),
                (
                    "I'm searching for someone.".to_string(),
                    "我在寻找某人。".to_string()
                ),
                (
                    "Someone very dear.".to_string(),
                    "一位非常亲爱的人。".to_string()
                ),
                ("A kindred spirit.".to_string(), "志同道合的人".to_string()),
                ("A dragon.".to_string(), "一条龙".to_string()),
                (
                    "A dangerous quest for a lone hunter.".to_string(),
                    "对独行猎手而言，这是一场危险的 quest。".to_string()
                ),
                (
                    "I've been alone for as long as I can remember.".to_string(),
                    "从我有记忆起，我就一直孤身一人。".to_string()
                ),
            ]
        );
    }
}
