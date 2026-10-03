//! Audio 3.0 high-quality ASR WebSocket client.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::models::SourceLanguage;
use crate::core::pending_pcm::PendingPcmGate;
use crate::core::protocols::audio3::{
    Audio3ASREndpoint, Audio3ASRRequestEncoder, Audio3ASRServerEvent, Audio3ASRServerEventDecoder,
};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(250);
const GENERIC_TRANSPORT_ERROR: &str = "The speech recognition connection closed.";
const GENERIC_PROTOCOL_ERROR: &str = "The speech recognition service returned invalid data.";
const SILENCE_INTERVAL: Duration = Duration::from_millis(100);
const SILENCE_PCM: [u8; 3200] = [0; 3200]; // 100 ms of 16 kHz mono PCM16.

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Audio3ASRClientError {
    #[error("Add an Alibaba Cloud Model Studio API key in Settings.")]
    MissingAPIKey,
    #[error("custom_speech_endpoint_invalid")]
    InvalidCustomEndpoint,
    #[error("custom_speech_model_invalid")]
    InvalidCustomModel,
    #[error("The speech recognition session is not connected.")]
    NotConnected,
    #[error("The speech recognition connection stopped responding.")]
    HealthCheckTimedOut,
    #[error("{0}")]
    Task(String),
    #[error("The speech recognition connection could not be established in time.")]
    ConnectionTimedOut,
    #[error("audio3_error.setup.timeout.LOCAL_TIMEOUT")]
    TaskSetupTimedOut,
    #[error("The speech recognition transport failed.")]
    TransportFailure,
}

impl Audio3ASRClientError {
    pub fn is_unreachable(&self) -> bool {
        matches!(self, Self::TransportFailure | Self::NotConnected)
            || matches!(self, Self::Task(label) if label == GENERIC_TRANSPORT_ERROR)
    }
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

/// Clear follows the recognizer's real sentence boundary; it never subtracts
/// a guessed text prefix. An unseen server sentence cannot be classified by
/// capture time, so only the currently observed sentence/ID watermark is cut.
#[derive(Default)]
struct Audio3ContentGate {
    revision: u64,
    active: bool,
    highest_sentence_id: Option<u64>,
    discard_through_id: Option<u64>,
    discard_unknown_sentence: bool,
}

impl Audio3ContentGate {
    fn clear(&mut self, revision: u64) {
        self.revision = revision;
        self.discard_through_id = self.highest_sentence_id;
        self.discard_unknown_sentence |= self.active;
        self.active = false;
    }

    fn accepts(&mut self, event: &Audio3ASRServerEvent) -> bool {
        let Audio3ASRServerEvent::Transcription {
            is_final,
            sentence_id,
            ..
        } = event
        else {
            return true;
        };
        if self.discard_unknown_sentence {
            let next_identified_sentence = sentence_id.is_some_and(|id| {
                self.discard_through_id
                    .is_some_and(|watermark| id > watermark)
            });
            if next_identified_sentence {
                self.discard_unknown_sentence = false;
            } else {
                if *is_final {
                    self.discard_unknown_sentence = false;
                    if let Some(id) = sentence_id {
                        let watermark = self.highest_sentence_id.map_or(*id, |seen| seen.max(*id));
                        self.highest_sentence_id = Some(watermark);
                        self.discard_through_id = Some(watermark);
                    }
                }
                return false;
            }
        }
        if let Some(id) = sentence_id {
            if self
                .discard_through_id
                .is_some_and(|watermark| *id <= watermark)
            {
                return false;
            }
            self.highest_sentence_id =
                Some(self.highest_sentence_id.map_or(*id, |seen| seen.max(*id)));
        }
        self.active = !*is_final;
        true
    }
}

struct Inner {
    generation: AtomicU64,
    content_gate: std::sync::Mutex<Audio3ContentGate>,
    sink: Mutex<Option<Sink>>,
    task_started: AtomicBool,
    task_finished: AtomicBool,
    finishing: AtomicBool,
    last_audio_sent: Mutex<tokio::time::Instant>,
    pending_pcm: std::sync::Mutex<Option<PendingPcmGate>>,
    terminal_error: Mutex<Option<String>>,
    pong_notify: Notify,
    receive_task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone)]
pub struct Audio3ASRClient {
    network: super::provider_network::ProviderNetwork,
    inner: Arc<Inner>,
    endpoint: Audio3ASREndpoint,
    model: String,
    custom_endpoint: bool,
    api_key: String,
    source_language: SourceLanguage,
    events: Arc<Mutex<Option<ProviderEventSender>>>,
    task_id: Arc<Mutex<Option<String>>>,
}

impl Audio3ASRClient {
    /// Bound to this generation's real PCM queue before native capture starts.
    pub fn set_audio_pending_gate(&self, gate: PendingPcmGate) {
        *self.inner.pending_pcm.lock().unwrap() = Some(gate);
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
    ) -> Result<Self, Audio3ASRClientError> {
        let trimmed_key = api_key.trim();
        if trimmed_key.is_empty() {
            return Err(Audio3ASRClientError::MissingAPIKey);
        }
        Ok(Self {
            network: super::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                generation: AtomicU64::new(0),
                content_gate: Default::default(),
                sink: Mutex::new(None),
                task_started: AtomicBool::new(false),
                task_finished: AtomicBool::new(false),
                finishing: AtomicBool::new(false),
                last_audio_sent: Mutex::new(tokio::time::Instant::now()),
                pending_pcm: Default::default(),
                terminal_error: Mutex::new(None),
                pong_notify: Notify::new(),
                receive_task: Mutex::new(None),
            }),
            endpoint: Audio3ASREndpoint::new().map_err(|_| Audio3ASRClientError::MissingAPIKey)?,
            model: Audio3ASREndpoint::MODEL.to_owned(),
            custom_endpoint: false,
            api_key: trimmed_key.to_string(),
            source_language,
            events: Arc::new(Mutex::new(None)),
            task_id: Arc::new(Mutex::new(None)),
        })
    }

    pub fn new_custom(
        endpoint: &str,
        model: &str,
        api_key: &str,
        source_language: SourceLanguage,
    ) -> Result<Self, Audio3ASRClientError> {
        let endpoint = crate::core::protocols::custom_speech::endpoint(
            endpoint,
            crate::core::provider::ProviderKind::CustomDashScopeASR,
        )
        .map_err(|_| Audio3ASRClientError::InvalidCustomEndpoint)?;
        let model = crate::core::protocols::custom_speech::validate_model(model)
            .map_err(|_| Audio3ASRClientError::InvalidCustomModel)?;
        let mut client = Self::new(api_key, source_language)?;
        client.endpoint.url = endpoint;
        client.model = model;
        client.custom_endpoint = true;
        Ok(client)
    }

    /// Sets the channel the receive loop emits decoded events onto.
    pub async fn set_event_sender(&self, sender: ProviderEventSender) {
        *self.inner.content_gate.lock().unwrap() = Audio3ContentGate {
            revision: sender.content_revision(),
            ..Default::default()
        };
        *self.events.lock().await = Some(sender);
    }

    pub fn content_revision(&self) -> u64 {
        self.inner.content_gate.lock().unwrap().revision
    }

    pub async fn clear_content(&self) -> u64 {
        let events = self.events.lock().await.clone();
        let mut gate = self.inner.content_gate.lock().unwrap();
        let revision = events.as_ref().map_or(
            gate.revision.wrapping_add(1),
            ProviderEventSender::advance_content_revision,
        );
        gate.clear(revision);
        revision
    }

    /// Opens the socket, sends `run-task`, and waits for `task-started`.
    pub async fn connect(&self, task_id: &str) -> Result<(), Audio3ASRClientError> {
        self.connect_with_heartbeat(task_id, true).await
    }

    /// Authenticated task setup without sending captured or synthetic PCM.
    pub async fn connect_for_probe(&self, task_id: &str) -> Result<(), Audio3ASRClientError> {
        self.connect_with_heartbeat(task_id, false).await
    }

    async fn connect_with_heartbeat(
        &self,
        task_id: &str,
        silence_heartbeat: bool,
    ) -> Result<(), Audio3ASRClientError> {
        self.disconnect().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let events = self
            .events
            .lock()
            .await
            .clone()
            .ok_or(Audio3ASRClientError::NotConnected)?;
        let context = Some(
            crate::core::protocols::audio3::Audio3ASRContext::audiovisual_dialogue(
                self.source_language,
            ),
        );
        let run_task = if self.model == Audio3ASREndpoint::MODEL {
            Audio3ASRRequestEncoder::run_task(task_id, self.source_language, context)
        } else {
            Audio3ASRRequestEncoder::run_task_for_model(
                task_id,
                self.source_language,
                context,
                &self.model,
            )
        }
        .map_err(|_| Audio3ASRClientError::NotConnected)?;
        crate::development_content::asr_request(
            events
                .debug_context()
                .map(|(source, generation)| (source, generation, events.content_revision())),
            &run_task,
        );
        *self.task_id.lock().await = Some(task_id.to_string());

        let mut request = self
            .endpoint
            .url
            .clone()
            .into_client_request()
            .map_err(|_| Audio3ASRClientError::NotConnected)?;
        let auth = format!("Bearer {}", self.api_key);
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&auth).map_err(|_| Audio3ASRClientError::MissingAPIKey)?,
        );
        request
            .headers_mut()
            .insert("User-Agent", HeaderValue::from_static("mimi-tauri"));

        let (socket, _response) = tokio::time::timeout(
            CONNECT_TIMEOUT,
            super::provider_network::websocket_with_message_limit(
                request,
                &self.network,
                crate::core::protocols::audio3::MAX_AUDIO3_MESSAGE_BYTES,
            ),
        )
        .await
        .map_err(|_| Audio3ASRClientError::ConnectionTimedOut)?
        .map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response)
                if matches!(response.status().as_u16(), 401 | 403) =>
            {
                Audio3ASRClientError::Task(
                    "audio3_error.connection.authentication.HTTP_AUTH".into(),
                )
            }
            _ => Audio3ASRClientError::TransportFailure,
        })?;
        let (sink, mut stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.task_started.store(false, Ordering::SeqCst);
        self.inner.task_finished.store(false, Ordering::SeqCst);
        self.inner.finishing.store(false, Ordering::SeqCst);
        *self.inner.last_audio_sent.lock().await = tokio::time::Instant::now();
        *self.inner.terminal_error.lock().await = None;

        let inner = self.inner.clone();
        let source_language = self.source_language;
        let expected_task_id = self.custom_endpoint.then(|| task_id.to_owned());
        let evidence_context = crate::development_audio::context();
        let task = tokio::spawn(async move {
            let mut heartbeat = tokio::time::interval(SILENCE_INTERVAL);
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                if inner.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                let message = tokio::select! {
                    message = stream.next() => message,
                    _ = heartbeat.tick(), if silence_heartbeat && inner.task_started.load(Ordering::SeqCst)
                        && !inner.task_finished.load(Ordering::SeqCst)
                        && !inner.finishing.load(Ordering::SeqCst) => {
                        if crate::development_audio::scope_context(evidence_context, send_silence_if_idle(&inner)).await.is_err() {
                            fail_transport(&inner, &events, generation).await;
                            return;
                        }
                        continue;
                    }
                };
                if inner.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                let Some(message) = message else {
                    if should_report_transport_end(inner.task_finished.load(Ordering::SeqCst)) {
                        fail_transport(&inner, &events, generation).await;
                    }
                    return;
                };
                let text = match message {
                    Ok(Message::Text(text)) => text.to_string(),
                    Ok(Message::Binary(data)) => String::from_utf8_lossy(&data).to_string(),
                    Ok(Message::Pong(_)) => {
                        inner.pong_notify.notify_waiters();
                        continue;
                    }
                    Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => {
                        continue;
                    }
                    Ok(Message::Close(_)) => {
                        if should_report_transport_end(inner.task_finished.load(Ordering::SeqCst)) {
                            fail_transport(&inner, &events, generation).await;
                        }
                        return;
                    }
                    Err(_) => {
                        if should_report_transport_end(inner.task_finished.load(Ordering::SeqCst)) {
                            fail_transport(&inner, &events, generation).await;
                        }
                        return;
                    }
                };
                {
                    let decoded = match expected_task_id.as_deref() {
                        Some(task_id) => {
                            Audio3ASRServerEventDecoder::decode_for_task(&text, task_id)
                        }
                        None => Audio3ASRServerEventDecoder::decode(&text),
                    };
                    let event = match decoded {
                        Ok(event) => event,
                        Err(_) => {
                            *inner.terminal_error.lock().await =
                                Some(GENERIC_PROTOCOL_ERROR.into());
                            let _ = events.send_if(
                                LiveTranslateServerEvent::Error {
                                    code: "audio3_protocol_error".into(),
                                    message: GENERIC_PROTOCOL_ERROR.into(),
                                },
                                || inner.generation.load(Ordering::SeqCst) == generation,
                            );
                            return;
                        }
                    };
                    let is_task_finished = matches!(&event, Audio3ASRServerEvent::TaskFinished);
                    match &event {
                        Audio3ASRServerEvent::TaskStarted => {
                            inner.task_started.store(true, Ordering::SeqCst);
                        }
                        Audio3ASRServerEvent::TaskFinished => {}
                        Audio3ASRServerEvent::TaskFailed { code, message } => {
                            crate::pipeline_log!(
                                "audio3 task failed phase={} category={} code={}",
                                if inner.task_started.load(Ordering::SeqCst) {
                                    "recognition"
                                } else {
                                    "setup"
                                },
                                message,
                                code
                            );
                            *inner.terminal_error.lock().await = Some(task_failure_token(
                                code,
                                message,
                                inner.task_started.load(Ordering::SeqCst),
                            ));
                        }
                        _ => {}
                    }
                    let subtitle_event = match &event {
                        Audio3ASRServerEvent::TaskFailed { .. } => {
                            let token = inner.terminal_error.lock().await.clone().unwrap();
                            LiveTranslateServerEvent::Error {
                                code: token.clone(),
                                message: token,
                            }
                        }
                        _ => event.subtitle_event(source_language),
                    };
                    let is_task_failed =
                        matches!(subtitle_event, LiveTranslateServerEvent::Error { .. });
                    let send_result = {
                        let mut gate = inner.content_gate.lock().unwrap();
                        if inner.generation.load(Ordering::SeqCst) != generation {
                            return;
                        }
                        if gate.accepts(&event) {
                            let revision = gate.revision;
                            let is_content =
                                super::provider_events::is_content_event(&subtitle_event);
                            events.send_if(subtitle_event, || {
                                inner.generation.load(Ordering::SeqCst) == generation
                                    && (!is_content || events.content_revision() == revision)
                            })
                        } else {
                            Ok(())
                        }
                    };
                    // `finish` disconnects the socket as soon as this flag is
                    // visible. Publish SessionFinished first so that cleanup
                    // cannot abort the receive task between provider ack and
                    // the bridge event needed to drain an authoritative tail.
                    if is_task_finished {
                        inner.task_finished.store(true, Ordering::SeqCst);
                    }
                    if send_result.is_err() {
                        break;
                    }
                    if is_task_failed {
                        return;
                    }
                }
            }
        });
        *self.inner.receive_task.lock().await = Some(task);

        let result = async {
            self.send_text(run_task.to_string()).await?;
            self.wait_for_task_start(Duration::from_secs(10)).await
        }
        .await;
        if result.is_err() {
            self.disconnect().await;
        }
        result
    }

    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), Audio3ASRClientError> {
        if pcm_data.is_empty() {
            return Ok(());
        }
        if pcm_data.len() > crate::core::protocols::custom_speech::MAX_PCM_CHUNK_BYTES
            || !pcm_data.len().is_multiple_of(2)
        {
            return Err(Audio3ASRClientError::TransportFailure);
        }
        // The reliable provider-error event owns teardown. Avoid a secondary
        // pipeline transport failure racing it and masking authentication.
        if self.inner.terminal_error.lock().await.is_some() {
            return Ok(());
        }
        if !self.inner.task_started.load(Ordering::SeqCst)
            || self.inner.finishing.load(Ordering::SeqCst)
        {
            return Err(Audio3ASRClientError::NotConnected);
        }
        let operation = async {
            let mut sink = self.inner.sink.lock().await;
            let Some(sink) = sink.as_mut() else {
                return Err(Audio3ASRClientError::NotConnected);
            };
            let evidence = crate::development_audio::begin_pcm(pcm_data, 16_000);
            evidence
                .observe(sink.send(Message::Binary(pcm_data.to_vec().into())))
                .await
                .map_err(|_| Audio3ASRClientError::TransportFailure)?;
            *self.inner.last_audio_sent.lock().await = tokio::time::Instant::now();
            Ok(())
        };
        tokio::time::timeout(SEND_TIMEOUT, operation)
            .await
            .map_err(|_| Audio3ASRClientError::TransportFailure)?
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), Audio3ASRClientError> {
        let operation = async {
            let pong = self.inner.pong_notify.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(Audio3ASRClientError::NotConnected);
                };
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| Audio3ASRClientError::TransportFailure)?;
            }
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| Audio3ASRClientError::HealthCheckTimedOut)?
    }

    /// Sends `finish-task`, waits briefly for `task-finished`, then
    /// disconnects.
    pub async fn finish(&self, timeout: Duration) {
        if self.inner.sink.lock().await.is_none() {
            return;
        }
        self.inner.finishing.store(true, Ordering::SeqCst);
        let task_id = self.task_id.lock().await.clone().unwrap_or_default();
        if let Ok(command) = Audio3ASRRequestEncoder::finish_task(&task_id) {
            let _ = self.send_text(command.to_string()).await;
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.inner.task_finished.load(Ordering::SeqCst) {
                break;
            }
            if self.inner.terminal_error.lock().await.is_some() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.disconnect().await;
    }

    pub async fn disconnect(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.finishing.store(true, Ordering::SeqCst);
        if let Some(task) = self.inner.receive_task.lock().await.take() {
            task.abort();
        }
        let sink = self.inner.sink.lock().await.take();
        if let Some(mut sink) = sink {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.close()).await;
        }
        self.inner.task_started.store(false, Ordering::SeqCst);
        self.inner.task_finished.store(false, Ordering::SeqCst);
        *self.inner.terminal_error.lock().await = None;
    }

    async fn wait_for_task_start(&self, timeout: Duration) -> Result<(), Audio3ASRClientError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.inner.task_started.load(Ordering::SeqCst) {
                return Ok(());
            }
            if let Some(error) = self.inner.terminal_error.lock().await.clone() {
                return Err(Audio3ASRClientError::Task(error));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Audio3ASRClientError::TaskSetupTimedOut);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn send_text(&self, text: String) -> Result<(), Audio3ASRClientError> {
        let operation = async {
            let mut sink = self.inner.sink.lock().await;
            let Some(sink) = sink.as_mut() else {
                return Err(Audio3ASRClientError::NotConnected);
            };
            sink.send(Message::Text(text.into()))
                .await
                .map_err(|_| Audio3ASRClientError::TransportFailure)
        };
        tokio::time::timeout(SEND_TIMEOUT, operation)
            .await
            .map_err(|_| Audio3ASRClientError::TransportFailure)?
    }
}

async fn fail_transport(inner: &Inner, events: &ProviderEventSender, generation: u64) {
    if inner.generation.load(Ordering::SeqCst) != generation {
        return;
    }
    *inner.terminal_error.lock().await = Some(GENERIC_TRANSPORT_ERROR.into());
    let _ = events.send_if(
        LiveTranslateServerEvent::Error {
            code: "transport_error".into(),
            message: GENERIC_TRANSPORT_ERROR.into(),
        },
        || inner.generation.load(Ordering::SeqCst) == generation,
    );
}

fn should_report_transport_end(task_finished: bool) -> bool {
    !task_finished
}

fn task_failure_token(code: &str, category: &str, started: bool) -> String {
    let phase = if started { "recognition" } else { "setup" };
    // Inputs have already passed the protocol decoder's exact allowlists.
    format!("audio3_error.{phase}.{category}.{code}")
}

async fn send_silence_if_idle(inner: &Inner) -> Result<(), Audio3ASRClientError> {
    let operation = async {
        let mut sink = inner.sink.lock().await;
        if inner.finishing.load(Ordering::SeqCst)
            || inner.terminal_error.lock().await.is_some()
            || inner.last_audio_sent.lock().await.elapsed() < SILENCE_INTERVAL
        {
            return Ok(());
        }
        let Some(sink) = sink.as_mut() else {
            return Err(Audio3ASRClientError::NotConnected);
        };
        let pending_pcm = inner
            .pending_pcm
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(PendingPcmGate::has_pending);
        if pending_pcm {
            return Ok(());
        }
        let evidence = crate::development_audio::begin_pcm(&SILENCE_PCM, 16_000);
        evidence
            .observe(sink.send(Message::Binary(SILENCE_PCM.to_vec().into())))
            .await
            .map_err(|_| Audio3ASRClientError::TransportFailure)?;
        *inner.last_audio_sent.lock().await = tokio::time::Instant::now();
        Ok(())
    };
    tokio::time::timeout(SEND_TIMEOUT, operation)
        .await
        .map_err(|_| Audio3ASRClientError::TransportFailure)?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caption(id: Option<u64>, is_final: bool) -> Audio3ASRServerEvent {
        Audio3ASRServerEvent::Transcription {
            text: "Synthetic caption".into(),
            is_final,
            sentence_id: id,
        }
    }

    #[test]
    fn content_clear_suppresses_current_sentence_and_replays_but_accepts_next_id() {
        let mut gate = Audio3ContentGate::default();
        assert!(gate.accepts(&caption(Some(7), false)));
        gate.clear(1);
        assert!(!gate.accepts(&caption(Some(7), false)));
        gate.clear(2);
        assert!(!gate.accepts(&caption(None, false)));
        assert!(!gate.accepts(&caption(Some(7), false)));
        assert!(gate.accepts(&caption(Some(8), false)));
        assert!(!gate.accepts(&caption(Some(7), true)));
        assert!(gate.accepts(&caption(Some(8), true)));
        gate.clear(2);
        assert!(!gate.accepts(&caption(Some(8), true)));
        assert!(gate.accepts(&caption(Some(9), false)));
    }

    #[test]
    fn missing_identity_clear_waits_for_a_real_final_without_guessing_a_prefix() {
        let mut gate = Audio3ContentGate::default();
        assert!(gate.accepts(&caption(None, false)));
        gate.clear(1);
        gate.clear(2);
        assert!(!gate.accepts(&caption(None, false)));
        assert!(!gate.accepts(&caption(None, false)));
        assert!(!gate.accepts(&caption(Some(1), true)));
        assert!(!gate.accepts(&caption(Some(1), true)));
        assert!(gate.accepts(&caption(Some(2), false)));
        let mut unseen = Audio3ContentGate::default();
        unseen.clear(1);
        // No observed sentence means there is no honest capture-time cutoff.
        assert!(unseen.accepts(&caption(Some(1), false)));
    }

    #[test]
    fn socket_end_is_failure_only_before_task_finished() {
        assert!(should_report_transport_end(false));
        assert!(!should_report_transport_end(true));
    }

    #[test]
    fn failures_preserve_setup_or_recognition_stage() {
        assert_eq!(
            task_failure_token("CLIENT_ERROR", "timeout", false),
            "audio3_error.setup.timeout.CLIENT_ERROR"
        );
        assert_eq!(
            task_failure_token("SERVER_ERROR", "service", true),
            "audio3_error.recognition.service.SERVER_ERROR"
        );
    }
}

#[cfg(test)]
mod streaming_tests {
    use super::*;
    use crate::clients::provider_events::provider_event_channel;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    #[tokio::test]
    #[allow(clippy::result_large_err)] // Tungstenite fixes the handshake callback error type.
    async fn custom_dashscope_uses_selected_model_and_real_task_boundary() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.uri().path(), "/api-ws/v1/inference");
                    assert!(request.uri().query().is_none());
                    assert_eq!(request.headers()["authorization"], "Bearer synthetic-key");
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let run_task: serde_json::Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(run_task["header"]["action"], "run-task");
            assert_eq!(run_task["header"]["task_id"], "synthetic-custom-task");
            assert_eq!(run_task["payload"]["task"], "asr");
            assert_eq!(run_task["payload"]["model"], "synthetic-custom-asr-model");
            assert_eq!(run_task["payload"]["parameters"]["sample_rate"], 16_000);
            socket.send(Message::Text(serde_json::json!({"header":{"event":"task-started","task_id":"synthetic-custom-task"}}).to_string().into())).await.unwrap();
            assert!(
                matches!(socket.next().await.unwrap().unwrap(), Message::Binary(bytes) if bytes.as_ref() == [1,2,3,4])
            );
            socket.send(Message::Text(serde_json::json!({"header":{"event":"result-generated","task_id":"synthetic-custom-task"},"payload":{"output":{"sentence":{"sentence_id":1,"sentence_end":true,"text":"Synthetic custom recognition"}}}}).to_string().into())).await.unwrap();
            let finish: serde_json::Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(finish["header"]["action"], "finish-task");
            assert_eq!(finish["header"]["task_id"], "synthetic-custom-task");
            socket.send(Message::Text(serde_json::json!({"header":{"event":"task-finished","task_id":"synthetic-custom-task"}}).to_string().into())).await.unwrap();
            assert!(matches!(
                socket.next().await,
                Some(Ok(Message::Close(_))) | None
            ));
        });
        let mut client = Audio3ASRClient::new_custom(
            &format!("ws://{address}/api-ws/v1/inference"),
            "synthetic-custom-asr-model",
            "synthetic-key",
            SourceLanguage::English,
        )
        .unwrap();
        client
            .set_network(
                super::super::provider_network::ProviderNetwork::resolve(
                    &crate::core::network_proxy::ProxyConfig {
                        mode: crate::core::network_proxy::ProxyMode::Direct,
                        url: None,
                    },
                )
                .unwrap(),
            )
            .unwrap();
        let (events, mut receiver) = provider_event_channel();
        client.set_event_sender(events).await;
        client
            .connect_for_probe("synthetic-custom-task")
            .await
            .unwrap();
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionCreated)
        );
        client.send_audio(&[1, 2, 3, 4]).await.unwrap();
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 1,
                text: "Synthetic custom recognition".into(),
                language: Some("en".into())
            })
        );
        assert!(client
            .send_audio(&vec![
                0;
                crate::core::protocols::custom_speech::MAX_PCM_CHUNK_BYTES
                    + 2
            ])
            .await
            .is_err());
        client.finish(Duration::from_secs(1)).await;
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn clear_keeps_one_socket_and_audio_sending_while_old_sentence_is_discarded() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            assert!(socket
                .next()
                .await
                .unwrap()
                .unwrap()
                .to_text()
                .unwrap()
                .contains("run-task"));
            socket
                .send(Message::Text(
                    r#"{"header":{"event":"task-started"}}"#.into(),
                ))
                .await
                .unwrap();
            let frame = |id, final_sentence| {
                Message::Text(serde_json::json!({
                "header":{"event":"result-generated"}, "payload":{"output":{"sentence":{
                    "text":"Synthetic test sentence", "sentence_id":id, "sentence_end":final_sentence
                }}}
            }).to_string().into())
            };
            socket.send(frame(7, false)).await.unwrap();
            release_rx.await.unwrap();
            let audio = socket.next().await.unwrap().unwrap();
            assert!(matches!(audio, Message::Binary(bytes) if bytes.as_ref() == [1, 2, 3, 4]));
            socket.send(frame(7, false)).await.unwrap();
            socket.send(frame(7, true)).await.unwrap();
            socket.send(frame(8, true)).await.unwrap();
            assert!(matches!(
                socket.next().await,
                None | Some(Ok(Message::Close(_)))
            ));
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err()
            );
        });
        let mut client = Audio3ASRClient::new("synthetic-key", SourceLanguage::English).unwrap();
        client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
        client
            .set_network(
                super::super::provider_network::ProviderNetwork::resolve(
                    &crate::core::network_proxy::ProxyConfig {
                        mode: crate::core::network_proxy::ProxyMode::Direct,
                        url: None,
                    },
                )
                .unwrap(),
            )
            .unwrap();
        let (sender, mut receiver) = provider_event_channel();
        client.set_event_sender(sender).await;
        client
            .connect_for_probe("synthetic-clear-task")
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = receiver
                    .recv()
                    .await
                    .expect("synthetic stream closed before its identified draft");
                if matches!(
                    event,
                    LiveTranslateServerEvent::SourceUtteranceDraft {
                        utterance_id: 7,
                        ..
                    }
                ) {
                    break;
                }
            }
        })
        .await
        .expect("synthetic identified draft was not delivered");
        assert_eq!(client.clear_content().await, 1);
        client.send_audio(&[1, 2, 3, 4]).await.unwrap();
        release_tx.send(()).unwrap();
        let next = tokio::time::timeout(Duration::from_secs(2), receiver.recv_with_revision())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.content_revision, 1);
        assert!(matches!(
            next.event,
            LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 8,
                ..
            }
        ));
        assert!(receiver.try_recv().is_err());
        client.disconnect().await;
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn service_probe_waits_for_task_ready_without_sending_silence_pcm() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let run = socket.next().await.unwrap().unwrap();
            assert!(run.to_text().unwrap().contains("run-task"));
            socket
                .send(Message::Text(
                    r#"{"header":{"event":"task-started"}}"#.into(),
                ))
                .await
                .unwrap();
            // Several normal heartbeat intervals pass; a probe sends no PCM.
            assert!(
                tokio::time::timeout(Duration::from_millis(350), socket.next())
                    .await
                    .is_err()
            );
            socket.next().await
        });
        let mut client = Audio3ASRClient::new("fixture-only", SourceLanguage::English).unwrap();
        client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
        let (events, _receiver) = provider_event_channel();
        client.set_event_sender(events).await;
        client
            .connect_for_probe("synthetic-probe-task")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(400)).await;
        client.disconnect().await;
        let close = server.await.unwrap();
        assert!(matches!(close, None | Some(Ok(Message::Close(_)))));
    }

    #[tokio::test]
    async fn idle_audio_sends_silence_and_finish_stops_heartbeat() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (silence_ready_tx, silence_ready_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let run = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let run: serde_json::Value = serde_json::from_str(&run).unwrap();
            assert_eq!(run["payload"]["parameters"]["heartbeat"], true);
            socket
                .send(Message::Text(
                    r#"{"header":{"event":"task-started"}}"#.into(),
                ))
                .await
                .unwrap();
            for _ in 0..2 {
                let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert!(
                    matches!(frame, Message::Binary(ref bytes) if bytes.len() == SILENCE_PCM.len() && bytes.iter().all(|byte| *byte == 0))
                );
            }
            silence_ready_tx.send(()).unwrap();
            loop {
                let frame = socket.next().await.unwrap().unwrap();
                if let Message::Text(text) = frame {
                    assert!(text.contains("finish-task"));
                    socket
                        .send(Message::Text(
                            r#"{"header":{"event":"task-finished"}}"#.into(),
                        ))
                        .await
                        .unwrap();
                    break;
                }
            }
            let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .unwrap();
            assert!(matches!(next, None | Some(Ok(Message::Close(_)))));
        });
        let mut client =
            Audio3ASRClient::new("synthetic-test-key", SourceLanguage::English).unwrap();
        client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
        let (sender, mut events) = provider_event_channel();
        client.set_event_sender(sender).await;
        client.connect("synthetic-task").await.unwrap();
        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::SessionCreated)
        );
        tokio::time::timeout(Duration::from_secs(2), silence_ready_rx)
            .await
            .unwrap()
            .unwrap();
        client.finish(Duration::from_secs(1)).await;
        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn setup_authentication_failure_remains_terminal_and_safe() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _run = socket.next().await.unwrap().unwrap();
            socket.send(Message::Text(r#"{"header":{"event":"task-failed","error_code":"INVALID_API_KEY","error_message":"synthetic-private-response"}}"#.into())).await.unwrap();
            let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .unwrap();
            assert!(matches!(next, None | Some(Ok(Message::Close(_)))));
        });
        let mut client =
            Audio3ASRClient::new("synthetic-test-key", SourceLanguage::English).unwrap();
        client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
        let (sender, mut events) = provider_event_channel();
        client.set_event_sender(sender).await;
        let error = client.connect("synthetic-task").await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "audio3_error.setup.authentication.INVALID_API_KEY"
        );
        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::Error {
                code: "audio3_error.setup.authentication.INVALID_API_KEY".into(),
                message: "audio3_error.setup.authentication.INVALID_API_KEY".into()
            })
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn real_pcm_is_preserved_and_terminal_error_does_not_become_transport_recovery() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _run = socket.next().await.unwrap().unwrap();
            socket
                .send(Message::Text(
                    r#"{"header":{"event":"task-started"}}"#.into(),
                ))
                .await
                .unwrap();
            let frame = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let frame = socket.next().await.unwrap().unwrap();
                    if matches!(&frame, Message::Binary(bytes) if bytes.iter().all(|byte| *byte == 0)) { continue; }
                    break frame;
                }
            }).await.unwrap();
            assert!(matches!(frame, Message::Binary(ref bytes) if bytes.as_ref() == [1, 2, 3, 4]));
            socket.send(Message::Text(r#"{"header":{"event":"task-failed","error_code":"CLIENT_ERROR","error_message":"request timeout after 23 seconds."}}"#.into())).await.unwrap();
            let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .unwrap();
            assert!(matches!(next, None | Some(Ok(Message::Close(_)))));
        });
        let mut client =
            Audio3ASRClient::new("synthetic-test-key", SourceLanguage::English).unwrap();
        client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
        let (sender, mut events) = provider_event_channel();
        client.set_event_sender(sender).await;
        client.connect("synthetic-task").await.unwrap();
        let _ready = events.recv().await;
        client.send_audio(&[1, 2, 3, 4]).await.unwrap();
        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::Error {
                code: "audio3_error.recognition.timeout.CLIENT_ERROR".into(),
                message: "audio3_error.recognition.timeout.CLIENT_ERROR".into()
            })
        );
        client.send_audio(&[1, 2]).await.unwrap();
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn idle_send_clock_can_insert_silence_before_real_pcm_already_in_the_pipeline() {
        controlled_pending_pcm_gap(false, false).await;
    }

    #[tokio::test]
    async fn a_bound_pending_pcm_gate_prevents_extra_silence_before_delayed_real_pcm() {
        controlled_pending_pcm_gap(true, false).await;
    }

    #[tokio::test]
    async fn idle_silence_is_still_sent_after_the_bound_real_pcm_pipeline_drains() {
        controlled_pending_pcm_gap(true, true).await;
    }

    async fn controlled_pending_pcm_gap(bind_gate: bool, idle_after_drain: bool) {
        use crate::audio::send_pipeline::AudioSendPipeline;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let first_pcm = [1_u8, 0].repeat(800); // 50 ms of nonzero 16 kHz PCM16.
        let delayed_pcm = [64_u8, 0].repeat(1600); // 100 ms, already captured and audible.
        let expected_first = first_pcm.clone();
        let expected_delayed = delayed_pcm.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let run = socket.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(run.contains("run-task"));
            socket
                .send(Message::Text(
                    r#"{"header":{"event":"task-started"}}"#.into(),
                ))
                .await
                .unwrap();
            let mut frames = Vec::new();
            while let Some(message) = socket.next().await {
                match message.unwrap() {
                    Message::Binary(bytes) => frames.push(bytes.to_vec()),
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            let expect_zero = !bind_gate || idle_after_drain;
            assert_eq!(frames.len(), if expect_zero { 3 } else { 2 });
            assert_eq!(frames[0], expected_first);
            if bind_gate {
                assert_eq!(frames[1], expected_delayed);
                if idle_after_drain {
                    assert_eq!(frames[2], SILENCE_PCM);
                }
            } else {
                assert_eq!(frames[1], SILENCE_PCM);
                assert_eq!(frames[2], expected_delayed);
            }
            let (zero_frames, zero_bytes, real_bytes) =
                frames
                    .iter()
                    .fold((0, 0, 0), |(count, zeros, real), bytes| {
                        if bytes.iter().all(|byte| *byte == 0) {
                            (count + 1, zeros + bytes.len(), real)
                        } else {
                            (count, zeros, real + bytes.len())
                        }
                    });
            assert_eq!(zero_frames, usize::from(expect_zero));
            assert_eq!(zero_bytes, if expect_zero { 3200 } else { 0 });
            assert_eq!(real_bytes, 4800); // All 150 ms of captured PCM survives.
            assert_eq!(
                zero_bytes + real_bytes,
                if expect_zero { 8000 } else { 4800 }
            );
        });

        let mut client =
            Audio3ASRClient::new("synthetic-test-key", SourceLanguage::English).unwrap();
        client.endpoint.url = url::Url::parse(&format!("ws://{address}")).unwrap();
        let (sender, mut events) = provider_event_channel();
        client.set_event_sender(sender).await;
        // Disable only the autonomous timer so the exact production idle-send
        // operation below runs at a deterministic simulated scheduling gap.
        client.connect_for_probe("synthetic-task").await.unwrap();
        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::SessionCreated)
        );
        client.send_audio(&first_pcm).await.unwrap();

        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let entered = Arc::new(std::sync::Mutex::new(Some(entered_tx)));
        let release = Arc::new(std::sync::Mutex::new(Some(release_rx)));
        let delayed_client = client.clone();
        let pipeline = AudioSendPipeline::spawn(
            move |data| {
                let client = delayed_client.clone();
                let entered = Arc::clone(&entered);
                let release = Arc::clone(&release);
                async move {
                    let entered = entered.lock().unwrap().take();
                    if let Some(entered) = entered {
                        let _ = entered.send(data.len());
                    }
                    // A producer/network-worker scheduling pause before it
                    // enters the socket. No captured audio is actually absent.
                    let release = release.lock().unwrap().take();
                    if let Some(release) = release {
                        let _ = release.await;
                    }
                    client.send_audio(&data).await
                }
            },
            |_| panic!("synthetic local audio transport failed"),
        );
        let pending_gate = pipeline.pending_pcm_gate();
        if bind_gate {
            client.set_audio_pending_gate(pending_gate.clone());
        }
        pipeline.ingress().unwrap().try_send(delayed_pcm).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), entered_rx)
                .await
                .unwrap()
                .unwrap(),
            3200
        );
        assert_eq!(pipeline.input_activity(), (true, true));
        assert_eq!(pending_gate.pending_count(), 1);

        // Backdating the successful-send clock models a >100 ms gap without
        // a real-time sleep/testutil. Real PCM is still pending in the pipeline.
        *client.inner.last_audio_sent.lock().await =
            tokio::time::Instant::now() - SILENCE_INTERVAL - Duration::from_millis(1);
        send_silence_if_idle(&client.inner).await.unwrap();
        release_tx.send(()).unwrap();
        assert!(pipeline.finish(Duration::from_secs(2)).await);
        assert_eq!(pending_gate.pending_count(), 0);
        if idle_after_drain {
            *client.inner.last_audio_sent.lock().await =
                tokio::time::Instant::now() - SILENCE_INTERVAL - Duration::from_millis(1);
            send_silence_if_idle(&client.inner).await.unwrap();
        }
        client.disconnect().await;
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}
