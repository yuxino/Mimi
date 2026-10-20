//! Shared DashScope live translation client.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::diagnostics::{milliseconds, TranslationLatency, TranslationLatencyKind};
use crate::core::models::{SourceLanguage, TargetLanguage, UtteranceRole};
use crate::core::protocols::live_translate::{
    AlibabaRealtimeEventDecoder, LiveTranslateEndpoint, LiveTranslateEventIdentity,
    LiveTranslateRequestEncoder, LiveTranslateServerEvent,
};
use crate::core::provider::AlibabaRealtimeMode;
use crate::pipeline_log;
use futures_util::{SinkExt, StreamExt};
use std::collections::BTreeMap;
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
#[cfg(test)]
const MAX_TRACKED_ITEMS: usize = mimi_core::live_pair_aligner::MAX_TRACKED_ITEMS;

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
    #[error("Enter the Beijing Model Studio workspace ID for this realtime model.")]
    MissingWorkspace,
    #[error("The Alibaba realtime configuration is not supported.")]
    InvalidConfiguration,
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
    translates_audio: bool,
    content_lock: Mutex<()>,
    aligner: Mutex<LiveTranslatePairAligner>,
    decoder: Mutex<AlibabaRealtimeEventDecoder>,
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
    mode: AlibabaRealtimeMode,
    events: ProviderEventSender,
}

impl LiveTranslateClient {
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.url.clone()
    }

    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    pub async fn clear_content(&self) -> u64 {
        let _content = self.inner.content_lock.lock().await;
        self.inner.aligner.lock().await.clear_content();
        self.inner.decoder.lock().await.clear_content();
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
        Self::build(
            api_key,
            source_language,
            target_language,
            hotwords,
            events,
            AlibabaRealtimeMode::Disabled,
            LiveTranslateEndpoint::new()
                .map_err(|_| LiveTranslateClientError::InvalidConfiguration)?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_mode(
        api_key: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        hotwords: BTreeMap<String, String>,
        events: ProviderEventSender,
        mode: AlibabaRealtimeMode,
        workspace_id: &str,
    ) -> Result<Self, LiveTranslateClientError> {
        // Validate before opening a socket, including language capabilities.
        LiveTranslateRequestEncoder::session_update_for_mode(
            mode,
            source_language,
            target_language,
            &hotwords,
            None,
        )
        .map_err(|_| LiveTranslateClientError::InvalidConfiguration)?;
        let endpoint = LiveTranslateEndpoint::for_mode(mode, workspace_id).map_err(|error| match error {
            crate::core::protocols::live_translate::LiveTranslateProtocolError::MissingWorkspace => LiveTranslateClientError::MissingWorkspace,
            _ => LiveTranslateClientError::InvalidConfiguration,
        })?;
        Self::build(
            api_key,
            source_language,
            target_language,
            hotwords,
            events,
            mode,
            endpoint,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        api_key: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        hotwords: BTreeMap<String, String>,
        events: ProviderEventSender,
        mode: AlibabaRealtimeMode,
        endpoint: LiveTranslateEndpoint,
    ) -> Result<Self, LiveTranslateClientError> {
        let trimmed_key = api_key.trim();
        if trimmed_key.is_empty() {
            return Err(LiveTranslateClientError::MissingAPIKey);
        }
        Ok(Self {
            network: super::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                translates_audio: target_language.translates_audio(),
                content_lock: Mutex::new(()),
                aligner: Mutex::new(LiveTranslatePairAligner::default()),
                decoder: Mutex::new(AlibabaRealtimeEventDecoder::new(mode)),
                sink: Mutex::new(None),
                received_session_finished: AtomicBool::new(false),
                pong_notify: Notify::new(),
                receive_task: Mutex::new(None),
                translation_latency: Default::default(),
            }),
            endpoint,
            api_key: trimmed_key.to_string(),
            source_language,
            target_language,
            hotwords,
            mode,
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
            self.target_language,
        ));
        *self.inner.receive_task.lock().await = Some(task);

        let update = if self.mode == AlibabaRealtimeMode::Disabled {
            LiveTranslateRequestEncoder::session_update(
                self.source_language,
                self.target_language,
                &self.hotwords,
                None,
            )
        } else {
            LiveTranslateRequestEncoder::session_update_for_mode(
                self.mode,
                self.source_language,
                self.target_language,
                &self.hotwords,
                None,
            )
        }
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

/// Desktop timing/transport adapter for the single shared identity algorithm.
#[derive(Default)]
struct LiveTranslatePairAligner {
    core: mimi_core::live_pair_aligner::LivePairAligner,
    translation_latency: Arc<std::sync::Mutex<StreamTranslationLatency>>,
    latency_generation: u64,
    clock_epoch: Option<Instant>,
}

impl LiveTranslatePairAligner {
    fn clear_content(&mut self) {
        self.core.clear_content();
    }

    #[cfg(test)]
    fn discard_item(&mut self, item: String) {
        self.core.discard_item(item);
    }

    #[cfg(test)]
    fn observe(
        &mut self,
        event: &LiveTranslateServerEvent,
        identity: &LiveTranslateEventIdentity,
    ) -> Vec<LiveTranslateServerEvent> {
        self.observe_at(event, identity, Instant::now())
    }

    #[cfg(test)]
    fn observe_at(
        &mut self,
        event: &LiveTranslateServerEvent,
        identity: &LiveTranslateEventIdentity,
        received_at: Instant,
    ) -> Vec<LiveTranslateServerEvent> {
        self.observe_at_with_source_order(event, identity, received_at)
            .into_iter()
            .map(|(event, _)| event)
            .collect()
    }

    fn observe_with_source_order(
        &mut self,
        event: &LiveTranslateServerEvent,
        identity: &LiveTranslateEventIdentity,
    ) -> Vec<(LiveTranslateServerEvent, Option<u64>)> {
        self.observe_at_with_source_order(event, identity, Instant::now())
    }

    fn observe_at_with_source_order(
        &mut self,
        event: &LiveTranslateServerEvent,
        identity: &LiveTranslateEventIdentity,
        received_at: Instant,
    ) -> Vec<(LiveTranslateServerEvent, Option<u64>)> {
        use mimi_core::live_pair_aligner::{LivePairEvent as E, LivePairIdentity};
        let input = match event {
            LiveTranslateServerEvent::SourceDraft { text, language } => E::SourceDraft {
                text: text.clone(),
                language: language.clone(),
            },
            LiveTranslateServerEvent::SourceFinal { text, language } => E::SourceFinal {
                text: text.clone(),
                language: language.clone(),
            },
            LiveTranslateServerEvent::TranslationDraft(text) => {
                E::TranslationDraft { text: text.clone() }
            }
            LiveTranslateServerEvent::TranslationFinal(text) => {
                E::TranslationFinal { text: text.clone() }
            }
            LiveTranslateServerEvent::Ignored { kind } if kind == "conversation.item.created" => {
                E::ItemCreated
            }
            LiveTranslateServerEvent::SessionFinished => E::SessionFinished,
            _ => E::Passthrough {
                is_content: crate::clients::provider_events::is_content_event(event),
            },
        };
        let identity = LivePairIdentity {
            item_id: identity.item_id.clone(),
            previous_item_id: identity.previous_item_id.clone(),
        };
        let epoch = self.clock_epoch.get_or_insert(received_at);
        let received_at_ns = received_at
            .saturating_duration_since(*epoch)
            .as_nanos()
            .min(u64::MAX as u128) as u64;
        self.core
            .observe_at(&input, &identity, received_at_ns)
            .into_iter()
            .map(|aligned| {
                let source_ordinal = match &aligned {
                    E::UtteranceText { utterance_id, .. } | E::FinalPair { utterance_id, .. } => {
                        self.core.source_ordinal(utterance_id)
                    }
                    _ => None,
                };
                let event = match aligned {
                    E::UtteranceText {
                        utterance_id,
                        role,
                        text,
                        is_final,
                        language,
                    } => LiveTranslateServerEvent::UtteranceText {
                        utterance_id,
                        role,
                        text,
                        is_final,
                        language,
                    },
                    E::FinalPair {
                        utterance_id,
                        source,
                        translation,
                        language,
                        follow_latency_ms,
                    } => {
                        let mut latency = self.translation_latency.lock().unwrap();
                        if latency.generation == self.latency_generation {
                            latency.value =
                                follow_latency_ms.map(|milliseconds| TranslationLatency {
                                    milliseconds,
                                    kind: TranslationLatencyKind::Follow,
                                });
                        }
                        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                            utterance_id,
                            source,
                            language,
                            translation,
                        }
                    }
                    _ => event.clone(),
                };
                (event, source_ordinal)
            })
            .collect()
    }
}

async fn receive_loop(
    mut stream: Stream,
    inner: Arc<Inner>,
    events: ProviderEventSender,
    setup: watch::Sender<SetupState>,
    latency_generation: u64,
    target_language: TargetLanguage,
) {
    {
        let _content = inner.content_lock.lock().await;
        *inner.aligner.lock().await = LiveTranslatePairAligner {
            translation_latency: Arc::clone(&inner.translation_latency),
            latency_generation,
            ..Default::default()
        };
        inner.decoder.lock().await.clear_content();
    }
    while let Some(message) = stream.next().await {
        let wire = match message {
            Ok(Message::Text(text)) => text.to_string(),
            Ok(Message::Binary(data)) => String::from_utf8_lossy(&data).into_owned(),
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

        let revision = events.content_revision();
        let _content = inner.content_lock.lock().await;
        let decoded = inner.decoder.lock().await.decode(&wire);
        let decoded = match decoded {
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
        for (event, identity) in decoded {
            if (revision != events.content_revision()
                && crate::clients::provider_events::is_content_event(&event))
                || (!target_language.translates_audio()
                    && matches!(
                        event,
                        LiveTranslateServerEvent::TranslationDraft(_)
                            | LiveTranslateServerEvent::TranslationFinal(_)
                    ))
            {
                continue;
            }
            for (event, source_ordinal) in inner
                .aligner
                .lock()
                .await
                .observe_with_source_order(&event, &identity)
            {
                if !emit_server_event_with_source_order(
                    &inner,
                    &events,
                    &setup,
                    event,
                    source_ordinal,
                ) {
                    return;
                }
            }
        }
    }

    if should_report_transport_end(inner.received_session_finished.load(Ordering::SeqCst)) {
        fail_receive_loop(&events, &setup, "transport_error", GENERIC_TRANSPORT_ERROR);
    }
}

#[cfg(test)]
fn emit_server_event(
    inner: &Inner,
    events: &ProviderEventSender,
    setup: &watch::Sender<SetupState>,
    event: LiveTranslateServerEvent,
) -> bool {
    emit_server_event_with_source_order(inner, events, setup, event, None)
}

fn emit_server_event_with_source_order(
    inner: &Inner,
    events: &ProviderEventSender,
    setup: &watch::Sender<SetupState>,
    event: LiveTranslateServerEvent,
    source_ordinal: Option<u64>,
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
    let starts_identified_translation = inner.translates_audio
        && matches!(
            &event,
            LiveTranslateServerEvent::UtteranceText {
                role: UtteranceRole::Source,
                is_final: true,
                ..
            }
        );
    if source_final_arrived
        && inner.translates_audio
        && !starts_identified_translation
        && events
            .send(LiveTranslateServerEvent::TranslationStarted)
            .is_err()
    {
        return false;
    }
    let result = if starts_identified_translation {
        events.send_translating_source_final(event, source_ordinal)
    } else {
        events.send_with_source_order(event, source_ordinal)
    };
    if result.is_err() {
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
            matches!(events.as_slice(), [LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {source,translation,..}, LiveTranslateServerEvent::SessionFinished]
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
        assert!(aligner.core.tracked_utterance_count() == 0 && aligner.core.response_count() == 0);
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
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {source,translation,..} if source == "new source" && translation == "new translation")));
        for i in 0..MAX_TRACKED_ITEMS * 3 {
            aligner.discard_item(format!("old-{i}"));
        }
        assert!(aligner.core.discarded_item_count() <= MAX_TRACKED_ITEMS);
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
    fn livetranslate_wire_pairs_actual_item_identity_after_response_success() {
        let mut decoder = AlibabaRealtimeEventDecoder::new(AlibabaRealtimeMode::LiveTranslate);
        let mut aligner = LiveTranslatePairAligner::default();
        let mut published = Vec::new();
        for wire in [
            r#"{"type":"conversation.item.input_audio_transcription.delta","item_id":"source","delta":"Hello."}"#,
            r#"{"type":"conversation.item.created","previous_item_id":"source","item":{"id":"out","role":"assistant"}}"#,
            r#"{"type":"response.text.delta","item_id":"out","response_id":"response","delta":"你好。"}"#,
            r#"{"type":"response.text.done","item_id":"out","response_id":"response","text":"你好。"}"#,
            r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"source","transcript":"Hello."}"#,
        ] {
            for (event, identity) in decoder.decode(wire).unwrap() {
                published.extend(aligner.observe(&event, &identity));
            }
        }
        assert!(!published.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
        )));
        for (event, identity) in decoder.decode(r#"{"type":"response.done","response":{"id":"response","status":"completed","output":[]}}"#).unwrap() {
            published.extend(aligner.observe(&event, &identity));
        }
        let pairs: Vec<_> = published
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
                )
            })
            .collect();
        assert!(
            matches!(pairs.as_slice(), [LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {utterance_id,source,translation,..}] if utterance_id == "source" && source == "Hello." && translation == "你好。")
        );
    }

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
            translates_audio: true,
            content_lock: Mutex::new(()),
            aligner: Mutex::new(LiveTranslatePairAligner::default()),
            decoder: Mutex::new(AlibabaRealtimeEventDecoder::new(
                AlibabaRealtimeMode::Disabled,
            )),
            sink: Mutex::new(None),
            received_session_finished: AtomicBool::new(false),
            pong_notify: Notify::new(),
            receive_task: Mutex::new(None),
            translation_latency: Default::default(),
        }
    }

    #[test]
    fn identified_source_final_does_not_restart_a_newer_translation_wait() {
        use crate::core::audio_input::AudioSource;
        use crate::core::session::TranslationSessionController;

        for completed in [false, true] {
            let inner = test_inner();
            let (sender, mut receiver) = provider_event_channel();
            let (setup, _setup_rx) = watch::channel(SetupState::Ready);
            let mut controller = TranslationSessionController::default();
            controller.did_connect();
            let now = Instant::now();
            let source = |id: &str, is_final| LiveTranslateServerEvent::UtteranceText {
                utterance_id: id.into(),
                role: UtteranceRole::Source,
                text: format!("Synthetic {id} source."),
                is_final,
                language: None,
            };
            for (event, order) in [(source("old", false), 0), (source("current", true), 1)] {
                assert!(emit_server_event_with_source_order(
                    &inner,
                    &sender,
                    &setup,
                    event,
                    Some(order),
                ));
                while let Ok(event) = receiver.try_recv_with_revision() {
                    event.apply_to_at(&mut controller, AudioSource::System, now);
                }
            }
            if completed {
                assert!(emit_server_event_with_source_order(
                    &inner,
                    &sender,
                    &setup,
                    LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                        utterance_id: "current".into(),
                        source: "Synthetic current source.".into(),
                        translation: "Synthetic current translation.".into(),
                        language: None,
                    },
                    Some(1),
                ));
                receiver.try_recv_with_revision().unwrap().apply_to_at(
                    &mut controller,
                    AudioSource::System,
                    now,
                );
            }
            let before = controller.state.clone();
            let deadline = controller.translation_pending_deadline_from(AudioSource::System);
            assert!(emit_server_event_with_source_order(
                &inner,
                &sender,
                &setup,
                source("old", true),
                Some(0),
            ));
            while let Ok(event) = receiver.try_recv_with_revision() {
                let application = event.apply_to_at(
                    &mut controller,
                    AudioSource::System,
                    now + Duration::from_secs(5),
                );
                assert!(application.translation_pending_deadline.is_none());
                assert!(!application.resolves_translation_timeout);
            }
            assert_eq!(controller.state, before);
            assert_eq!(
                controller.translation_pending_deadline_from(AudioSource::System),
                deadline
            );
            assert_eq!(controller.tick(now + Duration::from_secs(6)), !completed);
        }
    }

    #[test]
    fn real_emission_owns_activity_through_late_source_and_graceful_tail_replay() {
        use crate::core::audio_input::AudioSource;
        use crate::core::session::TranslationSessionController;
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../live-pair-contracts.json")).unwrap();
        let replay = &fixture["translationStartActivity"];
        let inner = test_inner();
        let (sender, mut receiver) = provider_event_channel();
        let (setup, _setup_rx) = watch::channel(SetupState::Ready);
        let mut decoder = AlibabaRealtimeEventDecoder::new(AlibabaRealtimeMode::LiveTranslate);
        let mut aligner = LiveTranslatePairAligner::default();
        let mut controller = TranslationSessionController::default();
        controller.did_connect();
        let now = Instant::now();
        for phase in 0..4 {
            if phase == 3 {
                controller.begin_stopping();
            }
            let frames = if phase == 3 {
                &replay["finishFrames"]
            } else {
                &replay["phases"][phase]
            };
            let mut armed = 0;
            for frame in frames.as_array().unwrap() {
                for (event, identity) in decoder.decode(&frame.to_string()).unwrap() {
                    for (event, order) in aligner.observe_with_source_order(&event, &identity) {
                        assert!(emit_server_event_with_source_order(
                            &inner, &sender, &setup, event, order
                        ));
                    }
                }
            }
            while let Ok(event) = receiver.try_recv_with_revision() {
                assert!(!matches!(
                    event.event,
                    LiveTranslateServerEvent::TranslationStarted
                ));
                armed += usize::from(
                    event
                        .apply_to_at(
                            &mut controller,
                            AudioSource::System,
                            now + Duration::from_secs(phase as u64),
                        )
                        .translation_pending_deadline
                        .is_some(),
                );
            }
            assert_eq!(armed, usize::from(phase == 0 || phase == 2));
            assert_eq!(controller.state.is_translation_pending, phase == 2);
            assert!(!controller.state.is_translation_timed_out);
            assert_eq!(
                controller.state.subtitles.history.len(),
                [1, 2, 2, 3][phase]
            );
            assert_eq!(
                controller.state.subtitles.source.utterance_id.as_deref(),
                Some(if phase < 2 {
                    "source_current"
                } else {
                    "source_next"
                })
            );
        }
        assert_eq!(
            controller.state.status,
            // Native adapters finalize Stopping after draining this tail.
            crate::core::models::SessionStatus::Stopping
        );
        assert_eq!(
            controller
                .state
                .subtitles
                .display_pair
                .as_ref()
                .unwrap()
                .translation,
            "Synthetic next translation."
        );
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
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                    utterance_id: "item_source".into(),
                    source: "Hello.".into(),
                    language: Some("en".into()),
                    translation: "你好。".into(),
                },
            ]
        );
    }

    #[test]
    fn source_observation_contracts_keep_complete_pairs_through_provider_decoding() {
        use mimi_core::live_pair_aligner::{LivePairAligner, LivePairEvent};
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../live-pair-contracts.json")).unwrap();
        let ids = [
            "late_response_uses_exact_source_identity_and_orphan_final_never_guesses",
            "late_previous_source_final_preserves_new_pending_final",
            "legacy_next_translation_waits_for_unobserved_source_after_retirement",
            "legacy_empty_source_final_does_not_move_current_back",
            "legacy_translation_placeholder_before_current_waits_for_actual_source",
        ];
        let mut checked = 0;
        for case in fixtures["cases"].as_array().unwrap() {
            if !ids.contains(&case["id"].as_str().unwrap()) {
                continue;
            }
            checked += 1;
            let mut aligner = LiveTranslatePairAligner::default();
            let epoch = Instant::now();
            for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
                if step["legacySourceOrder"] == true {
                    let mut state = serde_json::to_value(&aligner.core).unwrap();
                    state.as_object_mut().unwrap().remove("source_order");
                    aligner.core = serde_json::from_value::<LivePairAligner>(state).unwrap();
                    continue;
                }
                let (event, identity) =
                    LiveTranslateServerEvent::decode_value_with_identity(&step["providerFrame"])
                        .unwrap();
                let output = aligner.observe_at(
                    &event,
                    &identity,
                    epoch + Duration::from_nanos(step["atNs"].as_u64().unwrap()),
                );
                let expected: Vec<_> = step["expected"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|event| {
                        match serde_json::from_value::<LivePairEvent>(event.clone()).unwrap() {
                            LivePairEvent::UtteranceText {
                                utterance_id,
                                role,
                                text,
                                is_final,
                                language,
                            } => LiveTranslateServerEvent::UtteranceText {
                                utterance_id,
                                role,
                                text,
                                is_final,
                                language,
                            },
                            LivePairEvent::FinalPair {
                                utterance_id,
                                source,
                                translation,
                                language,
                                ..
                            } => LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                                utterance_id,
                                source,
                                translation,
                                language,
                            },
                            other => {
                                panic!("unexpected source-observation fixture output: {other:?}")
                            }
                        }
                    })
                    .collect();
                assert_eq!(output, expected, "{} step {index}", case["id"]);
            }
        }
        assert_eq!(checked, ids.len());
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
            vec![LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id: "item_source".into(),
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
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                    utterance_id: "item_source".into(),
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
            vec![LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id: "item_source_2".into(),
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
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                    utterance_id: "item_source_1".into(),
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
    fn queued_late_translation_keeps_the_following_source_draft() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../live-pair-contracts.json")).unwrap();
        let case = fixtures["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["id"] == "queued_late_translation_preserves_following_draft")
            .unwrap();
        assert_eq!(case["queueBeforeDrain"], true);
        let mut aligner = LiveTranslatePairAligner::default();
        let (sender, mut receiver) = provider_event_channel();
        for step in case["steps"].as_array().unwrap() {
            let (event, identity) =
                LiveTranslateServerEvent::decode_value_with_identity(&step["providerFrame"])
                    .unwrap();
            for (event, source_ordinal) in aligner.observe_with_source_order(&event, &identity) {
                sender
                    .send_with_source_order(event, source_ordinal)
                    .unwrap();
            }
        }
        // The consumer was busy while both sentences arrived. The previous
        // sentence's reliable final must not erase the latest source preview.
        let mut controller = crate::core::session::TranslationSessionController::default();
        controller.did_connect();
        while let Ok(event) = receiver.try_recv() {
            controller.handle(event);
        }
        assert_eq!(
            serde_json::to_value(&controller.state.subtitles.source).unwrap(),
            case["expectedQueuedSource"]
        );
        assert_eq!(
            serde_json::to_value(controller.state.subtitles.history.iter().map(|pair| {
                serde_json::json!({ "source": pair.source, "translation": pair.translation })
            }).collect::<Vec<_>>()).unwrap(),
            case["expectedQueuedHistory"]
        );
    }

    #[test]
    fn reliable_old_source_final_keeps_the_aligner_order_after_drafts_coalesce() {
        for completions in [vec![], vec!["a", "c"], vec!["c", "a"]] {
            let mut aligner = LiveTranslatePairAligner::default();
            let (sender, mut receiver) = provider_event_channel();
            let mut direct = crate::core::session::TranslationSessionController::default();
            let fixture: serde_json::Value =
                serde_json::from_str(include_str!("../../../live-pair-contracts.json")).unwrap();
            let case = fixture["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|case| case["id"] == "queued_reliable_source_finals_keep_observation_order")
                .unwrap();
            let mut inputs: Vec<_> = case["steps"]
                .as_array()
                .unwrap()
                .iter()
                .map(|step| {
                    LiveTranslateServerEvent::decode_value_with_identity(&step["providerFrame"])
                        .unwrap()
                })
                .collect();
            for id in &completions {
                inputs.push((
                    LiveTranslateServerEvent::TranslationFinal(format!(
                        "Synthetic {id} translation."
                    )),
                    identity(&format!("response_{id}"), None),
                ));
            }
            for (event, identity) in inputs {
                for (event, order) in aligner.observe_with_source_order(&event, &identity) {
                    direct.handle(event.clone());
                    sender.send_with_source_order(event, order).unwrap();
                }
            }
            let mut held = crate::core::session::TranslationSessionController::default();
            while let Ok(event) = receiver.try_recv_with_revision() {
                event.apply_to(&mut held, crate::core::audio_input::AudioSource::System);
            }
            assert_eq!(
                direct.state.subtitles.source.utterance_id.as_deref(),
                Some("source_c")
            );
            assert_eq!(
                serde_json::to_value(&held.state.subtitles.source).unwrap(),
                case["expectedQueuedSource"]
            );
            assert_eq!(held.state.subtitles.source, direct.state.subtitles.source);
            assert!(held.state.subtitles.source.is_final);
            assert_eq!(held.state.subtitles.history.len(), completions.len());
            for (pair, id) in held.state.subtitles.history.iter().zip(&completions) {
                assert_eq!(pair.translation, format!("Synthetic {id} translation."));
            }
            if !completions.is_empty() {
                assert_eq!(
                    held.state
                        .subtitles
                        .display_pair
                        .as_ref()
                        .unwrap()
                        .utterance_id
                        .as_deref(),
                    Some("source_c")
                );
            }
        }
    }

    #[test]
    fn a_previous_final_preserves_source_final_without_blocking_its_translation_preview() {
        for complete_pair in [false, true] {
            let mut aligner = LiveTranslatePairAligner::default();
            let (sender, mut receiver) = provider_event_channel();
            let mut inputs = vec![
                (created_item(), identity("response_a", Some("source_a"))),
                (created_item(), identity("response_b", Some("source_b"))),
                (
                    LiveTranslateServerEvent::SourceFinal {
                        text: "Synthetic A.".into(),
                        language: None,
                    },
                    identity("source_a", None),
                ),
                (
                    LiveTranslateServerEvent::SourceDraft {
                        text: "Synthetic B".into(),
                        language: None,
                    },
                    identity("source_b", None),
                ),
                (
                    LiveTranslateServerEvent::SourceFinal {
                        text: "Synthetic B complete.".into(),
                        language: None,
                    },
                    identity("source_b", None),
                ),
                (
                    LiveTranslateServerEvent::TranslationDraft(
                        "Synthetic B translation preview".into(),
                    ),
                    identity("response_b", None),
                ),
            ];
            if complete_pair {
                inputs.push((
                    LiveTranslateServerEvent::TranslationFinal(
                        "Synthetic B translation final.".into(),
                    ),
                    identity("response_b", None),
                ));
            }
            inputs.push((
                LiveTranslateServerEvent::TranslationFinal("Synthetic A translation.".into()),
                identity("response_a", None),
            ));
            for (event, identity) in inputs {
                for (event, order) in aligner.observe_with_source_order(&event, &identity) {
                    sender.send_with_source_order(event, order).unwrap();
                }
            }
            let mut controller = crate::core::session::TranslationSessionController::default();
            while let Ok(event) = receiver.try_recv() {
                controller.handle(event);
            }
            assert_eq!(
                controller.state.subtitles.source.text,
                "Synthetic B complete."
            );
            assert!(controller.state.subtitles.source.is_final);
            assert_eq!(
                controller.state.subtitles.translation.text,
                if complete_pair {
                    "Synthetic B translation final."
                } else {
                    "Synthetic B translation preview"
                }
            );
            assert_eq!(
                controller.state.subtitles.translation.is_final,
                complete_pair
            );
            assert_eq!(
                controller.state.subtitles.history.len(),
                if complete_pair { 2 } else { 1 }
            );
        }
    }

    #[test]
    fn a_late_final_cannot_reopen_a_source_older_than_the_delivered_translation_preview() {
        let mut aligner = LiveTranslatePairAligner::default();
        let (sender, mut receiver) = provider_event_channel();
        for (event, identity) in [
            (created_item(), identity("response_a", Some("source_a"))),
            (created_item(), identity("response_c", Some("source_c"))),
            (
                LiveTranslateServerEvent::SourceFinal {
                    text: "Synthetic A.".into(),
                    language: None,
                },
                identity("source_a", None),
            ),
            (
                LiveTranslateServerEvent::SourceDraft {
                    text: "Synthetic B draft".into(),
                    language: None,
                },
                identity("source_b", None),
            ),
            (
                LiveTranslateServerEvent::SourceDraft {
                    text: "Synthetic C draft".into(),
                    language: None,
                },
                identity("source_c", None),
            ),
            (
                LiveTranslateServerEvent::TranslationDraft(
                    "Synthetic C translation preview".into(),
                ),
                identity("response_c", None),
            ),
            (
                LiveTranslateServerEvent::SourceDraft {
                    text: "Synthetic stale B".into(),
                    language: None,
                },
                identity("source_b", None),
            ),
            (
                LiveTranslateServerEvent::TranslationFinal("Synthetic A translation.".into()),
                identity("response_a", None),
            ),
        ] {
            for (event, order) in aligner.observe_with_source_order(&event, &identity) {
                sender.send_with_source_order(event, order).unwrap();
            }
        }
        let mut controller = crate::core::session::TranslationSessionController::default();
        let mut received_translation = false;
        while let Ok(event) = receiver.try_recv() {
            assert!(
                !matches!(&event, LiveTranslateServerEvent::UtteranceText { utterance_id, role: UtteranceRole::Source, .. } if utterance_id == "source_b")
            );
            received_translation |= matches!(&event, LiveTranslateServerEvent::UtteranceText { utterance_id, role: UtteranceRole::Translation, .. } if utterance_id == "source_c");
            controller.handle(event);
        }
        assert!(received_translation);
        assert_ne!(
            controller.state.subtitles.source.utterance_id.as_deref(),
            Some("source_b")
        );
        assert_eq!(controller.state.subtitles.history.len(), 1);
    }

    #[test]
    fn a_late_older_final_cannot_reopen_a_coalesced_middle_source() {
        for complete_current in [false, true] {
            let mut aligner = LiveTranslatePairAligner::default();
            let (sender, mut receiver) = provider_event_channel();
            let mut inputs = vec![
                (created_item(), identity("response_a", Some("source_a"))),
                (created_item(), identity("response_c", Some("source_c"))),
                (
                    LiveTranslateServerEvent::SourceFinal {
                        text: "Synthetic A.".into(),
                        language: None,
                    },
                    identity("source_a", None),
                ),
                (
                    LiveTranslateServerEvent::SourceDraft {
                        text: "Synthetic B draft".into(),
                        language: None,
                    },
                    identity("source_b", None),
                ),
                (
                    LiveTranslateServerEvent::SourceFinal {
                        text: "Synthetic C.".into(),
                        language: None,
                    },
                    identity("source_c", None),
                ),
            ];
            if complete_current {
                inputs.push((
                    LiveTranslateServerEvent::TranslationFinal("Synthetic C translation.".into()),
                    identity("response_c", None),
                ));
            }
            inputs.extend([
                (
                    LiveTranslateServerEvent::SourceDraft {
                        text: "Synthetic stale B".into(),
                        language: None,
                    },
                    identity("source_b", None),
                ),
                (
                    LiveTranslateServerEvent::TranslationFinal("Synthetic A translation.".into()),
                    identity("response_a", None),
                ),
            ]);
            for (event, identity) in inputs {
                for (event, order) in aligner.observe_with_source_order(&event, &identity) {
                    sender.send_with_source_order(event, order).unwrap();
                }
            }
            let mut controller = crate::core::session::TranslationSessionController::default();
            while let Ok(event) = receiver.try_recv() {
                controller.handle(event);
            }
            // B never reached the reducer, so its identity cannot be trusted
            // there to reject a preview replay after the already delivered C.
            assert_eq!(
                controller.state.subtitles.source.utterance_id.as_deref(),
                Some("source_c")
            );
            assert!(controller.state.subtitles.source.is_final);
            assert_eq!(
                controller.state.subtitles.history.len(),
                if complete_current { 2 } else { 1 }
            );
        }
    }

    #[test]
    fn a_late_final_cannot_rewind_an_already_delivered_newer_source_draft() {
        let mut aligner = LiveTranslatePairAligner::default();
        let (sender, mut receiver) = provider_event_channel();
        let mut controller = crate::core::session::TranslationSessionController::default();
        for (event, identity) in [
            (created_item(), identity("response_a", Some("source_a"))),
            (
                LiveTranslateServerEvent::SourceFinal {
                    text: "Synthetic A.".into(),
                    language: None,
                },
                identity("source_a", None),
            ),
            (
                LiveTranslateServerEvent::SourceDraft {
                    text: "Synthetic B draft".into(),
                    language: None,
                },
                identity("source_b", None),
            ),
            (
                LiveTranslateServerEvent::SourceDraft {
                    text: "Synthetic C draft".into(),
                    language: None,
                },
                identity("source_c", None),
            ),
        ] {
            for (event, order) in aligner.observe_with_source_order(&event, &identity) {
                sender.send_with_source_order(event, order).unwrap();
            }
        }
        while let Ok(event) = receiver.try_recv() {
            controller.handle(event);
        }
        assert_eq!(
            controller.state.subtitles.source.utterance_id.as_deref(),
            Some("source_c")
        );
        for (event, identity) in [
            (
                LiveTranslateServerEvent::SourceDraft {
                    text: "Synthetic stale B".into(),
                    language: None,
                },
                identity("source_b", None),
            ),
            (
                LiveTranslateServerEvent::TranslationFinal("Synthetic A translation.".into()),
                identity("response_a", None),
            ),
        ] {
            for (event, order) in aligner.observe_with_source_order(&event, &identity) {
                sender.send_with_source_order(event, order).unwrap();
            }
        }
        while let Ok(event) = receiver.try_recv() {
            controller.handle(event);
        }
        assert_eq!(
            controller.state.subtitles.source.utterance_id.as_deref(),
            Some("source_c")
        );
        assert!(!controller.state.subtitles.source.is_final);
        assert_eq!(controller.state.subtitles.history.len(), 1);
    }

    #[test]
    fn queued_source_order_is_not_reassigned_when_the_aligner_window_moves() {
        let mut aligner = LiveTranslatePairAligner::default();
        let (sender, mut receiver) = provider_event_channel();
        let source = LiveTranslateServerEvent::SourceDraft {
            text: "Synthetic old draft".into(),
            language: None,
        };
        for (event, order) in aligner.observe_with_source_order(&source, &identity("old", None)) {
            sender.send_with_source_order(event, order).unwrap();
        }
        // The mailbox retains the old draft while alignment advances beyond
        // its bounded identity window. Do not look up or reassign its order at
        // delivery time and mistake it for a new source.
        for index in 0..mimi_core::live_pair_aligner::MAX_TRACKED_ITEMS {
            aligner.observe_with_source_order(&source, &identity(&format!("new-{index}"), None));
        }
        assert_eq!(aligner.core.source_ordinal("old"), None);
        let current = format!(
            "new-{}",
            mimi_core::live_pair_aligner::MAX_TRACKED_ITEMS - 1
        );
        aligner.observe(&created_item(), &identity("response", Some(&current)));
        for (event, identity) in [
            (
                LiveTranslateServerEvent::SourceFinal {
                    text: "Synthetic current final.".into(),
                    language: None,
                },
                identity(&current, None),
            ),
            (
                LiveTranslateServerEvent::TranslationFinal("Synthetic current translation.".into()),
                identity("response", None),
            ),
        ] {
            for (event, order) in aligner.observe_with_source_order(&event, &identity) {
                sender.send_with_source_order(event, order).unwrap();
            }
        }
        let mut controller = crate::core::session::TranslationSessionController::default();
        while let Ok(event) = receiver.try_recv() {
            assert!(!matches!(
                event,
                LiveTranslateServerEvent::UtteranceText {
                    is_final: false,
                    ..
                }
            ));
            controller.handle(event);
        }
        assert_eq!(
            controller.state.subtitles.source.utterance_id.as_deref(),
            Some(current.as_str())
        );
        assert!(controller.state.subtitles.source.is_final);
        assert_eq!(controller.state.subtitles.history.len(), 1);
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
            vec![LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id: "source_a".into(),
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
        assert!(aligner.core.response_count() <= 64);
        assert!(aligner.core.tracked_utterance_count() <= 64);
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
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                    utterance_id: "item_source".into(),
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

        assert!(aligner.core.tracked_utterance_count() <= 1);
        assert!(aligner.core.response_count() <= 2);
    }

    /// Replays the captured Alibaba live-translate session the documentation
    /// harness uses: the two finals of one utterance arrive tens of
    /// milliseconds apart in either order, one utterance is never translated,
    /// and drafts must keep streaming untouched.
    #[test]
    fn the_real_alibaba_capture_pairs_every_utterance_by_identity() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/demos/english-film/response.json"
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
                    LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
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
