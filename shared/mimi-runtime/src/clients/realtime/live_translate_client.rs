//! Shared DashScope live translation client.

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::core::diagnostics::latency::{milliseconds, TranslationLatency, TranslationLatencyKind};
use crate::core::models::{SourceLanguage, TargetLanguage, UtteranceRole};
use crate::core::protocols::realtime::live_translate::{
    AlibabaRealtimeEventDecoder, LiveTranslateEndpoint, LiveTranslateEventIdentity,
    LiveTranslateProtocolError, LiveTranslateRequestEncoder, LiveTranslateServerEvent,
};
use crate::core::services::provider::AlibabaRealtimeMode;
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
    #[error("alibaba_realtime_service_unavailable")]
    ServiceUnavailable,
    #[error("alibaba_realtime_idle_timeout")]
    IdleTimeout,
    #[error("live_translate_unexpected_session_end")]
    UnexpectedSessionEnd,
    #[error("The live translation transport failed.")]
    TransportFailure,
    #[error("Enter the Beijing Model Studio workspace ID for this realtime model.")]
    MissingWorkspace,
    #[error("The Alibaba realtime configuration is not supported.")]
    InvalidConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected(LiveTranslateClientError),
}

fn setup_failure(code: &str) -> LiveTranslateClientError {
    match crate::core::lifecycle::recovery::normalize_provider_error_code(code) {
        "credential_authentication_failed" => LiveTranslateClientError::AuthenticationFailed,
        "alibaba_realtime_service_unavailable" => LiveTranslateClientError::ServiceUnavailable,
        "alibaba_realtime_idle_timeout" => LiveTranslateClientError::IdleTimeout,
        "live_translate_unexpected_session_end" => LiveTranslateClientError::UnexpectedSessionEnd,
        "transport_error" => LiveTranslateClientError::TransportFailure,
        _ => LiveTranslateClientError::SessionSetupRejected,
    }
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type Stream = futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

struct Inner {
    translates_audio: bool,
    content_lock: Mutex<()>,
    aligner: Mutex<LiveTranslatePairAligner>,
    decoder: Mutex<AlibabaRealtimeEventDecoder>,
    sink: Mutex<Option<Sink>>,
    finish_requested: AtomicBool,
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
    network: crate::clients::transport::provider_network::ProviderNetwork,
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
        network: crate::clients::transport::provider_network::ProviderNetwork,
    ) -> Result<(), crate::clients::transport::provider_network::ProviderNetworkError> {
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
            crate::core::protocols::realtime::live_translate::LiveTranslateProtocolError::MissingWorkspace => LiveTranslateClientError::MissingWorkspace,
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
            network: crate::clients::transport::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                translates_audio: target_language.translates_audio(),
                content_lock: Mutex::new(()),
                aligner: Mutex::new(LiveTranslatePairAligner::default()),
                decoder: Mutex::new(AlibabaRealtimeEventDecoder::new(mode)),
                sink: Mutex::new(None),
                finish_requested: AtomicBool::new(false),
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
            crate::clients::transport::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| LiveTranslateClientError::ConnectionTimedOut)?
        .map_err(|error| {
            if crate::clients::transport::connection_diagnostics::authentication_rejected(&error) {
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
        self.inner.finish_requested.store(true, Ordering::SeqCst);
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
        self.inner.finish_requested.store(false, Ordering::SeqCst);
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
                .map_err(|error| {
                    pipeline_log!(
                        "live translate send failed kind={}",
                        transport_error_kind(&error)
                    );
                    LiveTranslateClientError::TransportFailure
                })
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
        match setup.borrow().clone() {
            SetupState::Ready => return Ok(()),
            SetupState::Rejected(error) => return Err(error),
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
                is_content: crate::clients::transport::provider_events::is_content_event(event),
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
            Ok(Message::Text(text)) => Ok(text.to_string()),
            Ok(Message::Binary(data)) => std::str::from_utf8(&data).map(str::to_owned),
            Ok(Message::Pong(_)) => {
                inner.pong_notify.notify_waiters();
                continue;
            }
            Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => continue,
            Ok(Message::Close(frame)) => {
                // A numeric protocol status is safe; the peer's arbitrary
                // reason can contain user data and must never enter traces.
                pipeline_log!(
                    "live translate receive close code={} requestedFinish={}",
                    frame.as_ref().map_or(1005, |frame| u16::from(frame.code)),
                    inner.finish_requested.load(Ordering::SeqCst)
                );
                if !inner.received_session_finished.load(Ordering::SeqCst) {
                    fail_receive_loop(&events, &setup, "transport_error", GENERIC_TRANSPORT_ERROR);
                }
                return;
            }
            Err(error) => {
                pipeline_log!(
                    "live translate receive failed kind={}",
                    transport_error_kind(&error)
                );
                if !inner.received_session_finished.load(Ordering::SeqCst) {
                    fail_receive_loop(&events, &setup, "transport_error", GENERIC_TRANSPORT_ERROR);
                }
                return;
            }
        };

        let revision = events.content_revision();
        let _content = inner.content_lock.lock().await;
        let decoded = match wire {
            Ok(wire) => inner.decoder.lock().await.decode(&wire),
            Err(_) => Err(LiveTranslateProtocolError::InvalidJSON),
        };
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
            if event == LiveTranslateServerEvent::SessionFinished
                && !inner.finish_requested.load(Ordering::SeqCst)
            {
                // Only a locally requested finish may seal the pending tail
                // and stop the native driver. An unsolicited end retires this
                // connection through the shared bounded recovery policy.
                inner
                    .received_session_finished
                    .store(true, Ordering::SeqCst);
                fail_receive_loop(
                    &events,
                    &setup,
                    "live_translate_unexpected_session_end",
                    "The live translation session ended unexpectedly.",
                );
                return;
            }
            if (revision != events.content_revision()
                && crate::clients::transport::provider_events::is_content_event(&event))
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

    if !inner.received_session_finished.load(Ordering::SeqCst) {
        fail_receive_loop(&events, &setup, "transport_error", GENERIC_TRANSPORT_ERROR);
    }
}

fn transport_error_kind(error: &tokio_tungstenite::tungstenite::Error) -> &'static str {
    use tokio_tungstenite::tungstenite::Error;
    match error {
        Error::ConnectionClosed => "connection_closed",
        Error::AlreadyClosed => "already_closed",
        Error::Io(error) => match error.kind() {
            std::io::ErrorKind::ConnectionReset => "io_connection_reset",
            std::io::ErrorKind::ConnectionAborted => "io_connection_aborted",
            std::io::ErrorKind::BrokenPipe => "io_broken_pipe",
            std::io::ErrorKind::TimedOut => "io_timed_out",
            std::io::ErrorKind::UnexpectedEof => "io_unexpected_eof",
            _ => "io",
        },
        Error::Tls(_) => "tls",
        Error::Protocol(_) => "protocol",
        Error::Capacity(_) => "capacity",
        _ => "other",
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
        if let LiveTranslateServerEvent::Error { code, .. } = &event {
            let _ = setup.send(SetupState::Rejected(setup_failure(code)));
            return false;
        }
        if event == LiveTranslateServerEvent::SessionUpdated {
            // Publishing the acknowledgement also proves the bounded session
            // receiver is still alive before setup is marked ready.
            if events.send(event).is_err() {
                let _ = setup.send(SetupState::Rejected(
                    LiveTranslateClientError::SessionSetupRejected,
                ));
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
            let _ = setup.send(SetupState::Rejected(
                LiveTranslateClientError::SessionSetupRejected,
            ));
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
        let _ = setup.send(SetupState::Rejected(setup_failure(code)));
    } else {
        let _ = events.send(LiveTranslateServerEvent::Error {
            code: code.into(),
            message: message.into(),
        });
    }
}

#[cfg(test)]
mod tests;
