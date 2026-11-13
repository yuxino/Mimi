//! Audio 3.0 high-quality ASR WebSocket client.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::models::SourceLanguage;
use crate::core::pending_pcm::PendingPcmGate;
use crate::core::protocols::audio3::{
    Audio3ASREndpoint, Audio3ASRRequestEncoder, Audio3ASRServerEvent, Audio3ASRServerEventDecoder,
};
use crate::core::protocols::live_translate::{
    LiveTranslateProtocolError, LiveTranslateServerEvent,
};
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

// A native callback can deliver an entire elapsed audio batch after an idle
// timer has already run. Count successful PCM duration as well as send time so
// those gaps cannot repeatedly add synthetic audio to a continuous recording.
#[derive(Default)]
struct Audio3SendClock {
    started_at: Option<tokio::time::Instant>,
    audio_end_ns: u128,
    last_sent_at: Option<tokio::time::Instant>,
}

impl Audio3SendClock {
    fn start(&mut self, now: tokio::time::Instant) {
        self.started_at = Some(now);
        self.audio_end_ns = 0;
        self.last_sent_at = Some(now);
    }

    fn can_send_silence(&self, now: tokio::time::Instant) -> bool {
        let (Some(started_at), Some(last_sent_at)) = (self.started_at, self.last_sent_at) else {
            return false;
        };
        now.saturating_duration_since(last_sent_at) >= SILENCE_INTERVAL
            && now.saturating_duration_since(started_at).as_nanos()
                >= self
                    .audio_end_ns
                    .saturating_add(SILENCE_INTERVAL.as_nanos())
    }

    fn sent(
        &mut self,
        bytes: usize,
        send_started_at: tokio::time::Instant,
        completed_at: tokio::time::Instant,
    ) {
        let Some(started_at) = self.started_at else {
            return;
        };
        // Count whole PCM16 frames, not rounded milliseconds. At 16 kHz every
        // frame is exactly 62,500 ns, including arbitrarily small native blocks.
        let duration_ns = (bytes / 2) as u128 * 1_000_000_000 / 16_000;
        let elapsed_ns = send_started_at
            .saturating_duration_since(started_at)
            .as_nanos();
        // Missed idle ticks are not audio debt. Keep any already-sent lead, but
        // do not let old empty time fund more silence after real audio resumes.
        self.audio_end_ns = self
            .audio_end_ns
            .max(elapsed_ns.saturating_sub(duration_ns))
            .saturating_add(duration_ns);
        self.last_sent_at = Some(completed_at);
    }
}

struct Inner {
    generation: AtomicU64,
    content_gate: std::sync::Mutex<Audio3ContentGate>,
    sink: Mutex<Option<Sink>>,
    task_started: AtomicBool,
    task_finished: AtomicBool,
    finishing: AtomicBool,
    audio_clock: Mutex<Audio3SendClock>,
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
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.url.clone()
    }

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
                audio_clock: Mutex::new(Audio3SendClock::default()),
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
        *self.inner.audio_clock.lock().await = Audio3SendClock::default();
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
                    if !inner.task_finished.load(Ordering::SeqCst) {
                        fail_transport(&inner, &events, generation).await;
                    }
                    return;
                };
                let text = match message {
                    Ok(Message::Text(text)) => Ok(text.to_string()),
                    Ok(Message::Binary(data)) => std::str::from_utf8(&data).map(str::to_owned),
                    Ok(Message::Pong(_)) => {
                        inner.pong_notify.notify_waiters();
                        continue;
                    }
                    Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => {
                        continue;
                    }
                    Ok(Message::Close(_)) => {
                        if !inner.task_finished.load(Ordering::SeqCst) {
                            fail_transport(&inner, &events, generation).await;
                        }
                        return;
                    }
                    Err(_) => {
                        if !inner.task_finished.load(Ordering::SeqCst) {
                            fail_transport(&inner, &events, generation).await;
                        }
                        return;
                    }
                };
                {
                    let decoded = text
                        .map_err(|_| LiveTranslateProtocolError::InvalidJSON)
                        .and_then(|text| match expected_task_id.as_deref() {
                            Some(task_id) => {
                                Audio3ASRServerEventDecoder::decode_for_task(&text, task_id)
                            }
                            None => Audio3ASRServerEventDecoder::decode(&text),
                        });
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
                    if is_task_finished && !inner.finishing.load(Ordering::SeqCst) {
                        // Only an explicit finish may seal the recognition tail.
                        // Retire an unsolicited end before any late final can
                        // enter the bridge; native drivers own bounded recovery.
                        fail_transport(&inner, &events, generation).await;
                        return;
                    }
                    match &event {
                        Audio3ASRServerEvent::TaskStarted => {
                            if !inner.task_started.load(Ordering::SeqCst) {
                                inner
                                    .audio_clock
                                    .lock()
                                    .await
                                    .start(tokio::time::Instant::now());
                            }
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
            let send_started_at = tokio::time::Instant::now();
            let evidence = crate::development_audio::begin_pcm(pcm_data, 16_000);
            evidence
                .observe(sink.send(Message::Binary(pcm_data.to_vec().into())))
                .await
                .map_err(|_| Audio3ASRClientError::TransportFailure)?;
            self.inner.audio_clock.lock().await.sent(
                pcm_data.len(),
                send_started_at,
                tokio::time::Instant::now(),
            );
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
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let events = self.events.lock().await.clone();
        {
            let _content = self.inner.content_gate.lock().unwrap();
            if self.inner.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            self.inner.finishing.store(true, Ordering::SeqCst);
        }
        let operation = async {
            {
                let mut sink = self.inner.sink.lock().await;
                if self.inner.generation.load(Ordering::SeqCst) != generation {
                    return Ok(false);
                }
                let Some(sink) = sink.as_mut() else {
                    return Ok(false);
                };
                let task_id = self.task_id.lock().await.clone().unwrap_or_default();
                if let Ok(command) = Audio3ASRRequestEncoder::finish_task(&task_id) {
                    sink.send(Message::Text(command.to_string().into())).await?;
                }
            }
            loop {
                if self.inner.generation.load(Ordering::SeqCst) != generation
                    || self.inner.task_finished.load(Ordering::SeqCst)
                    || self.inner.terminal_error.lock().await.is_some()
                {
                    return Ok::<_, tokio_tungstenite::tungstenite::Error>(true);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        // Sending the command consumes the same budget as waiting for its ack.
        let outcome = tokio::time::timeout(timeout, operation).await;
        if matches!(outcome, Ok(Ok(false))) {
            return;
        }
        let send_failed = matches!(outcome, Ok(Err(_)));
        let task = {
            let mut task = self.inner.receive_task.lock().await;
            if self.inner.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            task.take()
        };
        let had_producer = task.is_some();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        // Join the sole producer before publishing a local drain barrier. A
        // provider error must remain an error, and a real ack is not duplicated.
        let failed = send_failed || self.inner.terminal_error.lock().await.is_some();
        let retired_generation = {
            let _content = self.inner.content_gate.lock().unwrap();
            if self.inner.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            if had_producer && !failed && !self.inner.task_finished.load(Ordering::SeqCst) {
                if let Some(events) = events {
                    let _ = events.send(LiveTranslateServerEvent::SessionFinished);
                }
            }
            self.inner
                .generation
                .fetch_add(1, Ordering::SeqCst)
                .wrapping_add(1)
        };
        self.disconnect_retired_generation(retired_generation).await;
    }

    pub async fn disconnect(&self) {
        let generation = {
            let _content = self.inner.content_gate.lock().unwrap();
            self.inner.finishing.store(true, Ordering::SeqCst);
            self.inner
                .generation
                .fetch_add(1, Ordering::SeqCst)
                .wrapping_add(1)
        };
        self.disconnect_retired_generation(generation).await;
    }

    async fn disconnect_retired_generation(&self, generation: u64) {
        let task = {
            let mut task = self.inner.receive_task.lock().await;
            if self.inner.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            task.take()
        };
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        let sink = {
            let mut sink = self.inner.sink.lock().await;
            if self.inner.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            sink.take()
        };
        if let Some(mut sink) = sink {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.close()).await;
        }
        let mut terminal_error = self.inner.terminal_error.lock().await;
        let _content = self.inner.content_gate.lock().unwrap();
        if self.inner.generation.load(Ordering::SeqCst) == generation {
            self.inner.task_started.store(false, Ordering::SeqCst);
            self.inner.task_finished.store(false, Ordering::SeqCst);
            *terminal_error = None;
        }
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

fn task_failure_token(code: &str, category: &str, started: bool) -> String {
    let phase = if started { "recognition" } else { "setup" };
    // Inputs have already passed the protocol decoder's exact allowlists.
    format!("audio3_error.{phase}.{category}.{code}")
}

fn has_pending_pcm(inner: &Inner) -> bool {
    inner
        .pending_pcm
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(PendingPcmGate::has_pending)
}

async fn send_silence_if_idle(inner: &Inner) -> Result<(), Audio3ASRClientError> {
    // The receive loop must keep reading while real PCM owns the writer.
    if has_pending_pcm(inner) {
        return Ok(());
    }
    let operation = async {
        let mut sink = inner.sink.lock().await;
        if inner.finishing.load(Ordering::SeqCst)
            || inner.terminal_error.lock().await.is_some()
            || !inner
                .audio_clock
                .lock()
                .await
                .can_send_silence(tokio::time::Instant::now())
        {
            return Ok(());
        }
        let Some(sink) = sink.as_mut() else {
            return Err(Audio3ASRClientError::NotConnected);
        };
        // Work can arrive while this heartbeat waits for the writer lock.
        if has_pending_pcm(inner) {
            return Ok(());
        }
        let send_started_at = tokio::time::Instant::now();
        let evidence = crate::development_audio::begin_pcm(&SILENCE_PCM, 16_000);
        evidence
            .observe(sink.send(Message::Binary(SILENCE_PCM.to_vec().into())))
            .await
            .map_err(|_| Audio3ASRClientError::TransportFailure)?;
        inner.audio_clock.lock().await.sent(
            SILENCE_PCM.len(),
            send_started_at,
            tokio::time::Instant::now(),
        );
        Ok(())
    };
    tokio::time::timeout(SEND_TIMEOUT, operation)
        .await
        .map_err(|_| Audio3ASRClientError::TransportFailure)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_clock_starts_at_task_ready_without_setup_debt() {
        let before_setup = tokio::time::Instant::now();
        let ready = before_setup + Duration::from_secs(60);
        let mut clock = Audio3SendClock::default();
        assert!(!clock.can_send_silence(ready));
        clock.start(ready);
        assert!(!clock.can_send_silence(ready + Duration::from_millis(99)));
        assert!(clock.can_send_silence(ready + SILENCE_INTERVAL));
    }

    fn continuous_batched_clock(
        clock: &mut Audio3SendClock,
        start: tokio::time::Instant,
        batch_ticks: u32,
    ) -> usize {
        let mut silence_bytes = 0;
        // One hour of continuous PCM delivered in batches. A native callback
        // runs after the timer at equal timestamps, exercising the worst order.
        for tick in 1..=36_000u32 {
            let now = start + SILENCE_INTERVAL * tick;
            if clock.can_send_silence(now) {
                clock.sent(SILENCE_PCM.len(), now, now);
                silence_bytes += SILENCE_PCM.len();
            }
            if tick.is_multiple_of(batch_ticks) {
                clock.sent(SILENCE_PCM.len() * batch_ticks as usize, now, now);
            }
        }
        silence_bytes
    }

    #[test]
    fn one_hour_of_batched_pcm_has_only_one_bounded_initial_silence_gap() {
        let start = tokio::time::Instant::now();
        for batch_ticks in [2, 5] {
            let mut clock = Audio3SendClock::default();
            clock.start(start);
            assert_eq!(
                continuous_batched_clock(&mut clock, start, batch_ticks),
                SILENCE_PCM.len() * batch_ticks as usize
            );
            assert_eq!(
                clock.audio_end_ns,
                Duration::from_millis(3_600_000 + 100 * u64::from(batch_ticks)).as_nanos()
            );
        }
    }

    #[test]
    fn missed_idle_ticks_cannot_fund_silence_after_continuous_pcm_resumes() {
        let start = tokio::time::Instant::now();
        let resumed = start + Duration::from_secs(600);
        let mut clock = Audio3SendClock::default();
        clock.start(start);
        // No task ran for ten minutes. A real batch resumes without making up
        // that lost time as synthetic audio during subsequent capture gaps.
        clock.sent(6400, resumed, resumed);
        assert_eq!(clock.audio_end_ns, Duration::from_secs(600).as_nanos());
        assert_eq!(continuous_batched_clock(&mut clock, resumed, 2), 6400);
    }

    #[test]
    fn silence_clock_preserves_submillisecond_pcm_frames() {
        let start = tokio::time::Instant::now();
        let mut clock = Audio3SendClock::default();
        clock.start(start);
        for _ in 0..16_000 {
            clock.sent(2, start, start);
        }
        assert_eq!(clock.audio_end_ns, Duration::from_secs(1).as_nanos());
        assert!(!clock.can_send_silence(start + Duration::from_secs(1)));
        assert!(clock.can_send_silence(start + Duration::from_millis(1100)));
    }

    #[test]
    fn slow_send_does_not_backfill_history_and_keeps_the_idle_guard() {
        let start = tokio::time::Instant::now();
        let mut clock = Audio3SendClock::default();
        clock.start(start);
        clock.sent(
            3200,
            start + SILENCE_INTERVAL,
            start + Duration::from_secs(1),
        );
        assert!(!clock.can_send_silence(start + Duration::from_secs(1)));
        let next = start + Duration::from_millis(1100);
        assert!(clock.can_send_silence(next));
        clock.sent(3200, next, next);
        assert_eq!(clock.audio_end_ns, Duration::from_millis(1100).as_nanos());
        assert!(!clock.can_send_silence(next));
    }

    #[test]
    fn a_new_task_resets_old_audio_lead() {
        let start = tokio::time::Instant::now();
        let mut clock = Audio3SendClock::default();
        clock.start(start);
        clock.sent(32_000, start, start);
        let next_task = start + Duration::from_millis(200);
        clock.start(next_task);
        assert_eq!(clock.audio_end_ns, 0);
        assert!(!clock.can_send_silence(next_task));
        assert!(clock.can_send_silence(next_task + SILENCE_INTERVAL));
    }

    #[tokio::test]
    async fn clearing_captions_keeps_the_audio_clock() {
        let client = Audio3ASRClient::new("fixture-only", SourceLanguage::English).unwrap();
        let start = tokio::time::Instant::now();
        {
            let mut clock = client.inner.audio_clock.lock().await;
            clock.start(start);
            clock.sent(6400, start, start);
        }
        client.clear_content().await;
        let clock = client.inner.audio_clock.lock().await;
        assert_eq!(clock.started_at, Some(start));
        assert_eq!(clock.audio_end_ns, Duration::from_millis(200).as_nanos());
        assert_eq!(clock.last_sent_at, Some(start));
    }

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
    async fn binary_json_preserves_unicode_and_rejects_invalid_utf8_transcripts() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../translation-contracts.json")).unwrap();
        let case = &fixtures["binaryJsonTransports"]["audio3"];
        let setup = case["setup"].to_string();
        let transcript = case["transcript"].to_string();
        let hex = case["invalidTranscriptHex"].as_str().unwrap();
        let invalid: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).unwrap())
            .collect();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = Audio3ASRClient::new_custom(
            &format!("ws://{}/asr", listener.local_addr().unwrap()),
            "synthetic-model",
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
        let (corrupt, corrupted) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _ = socket.next().await;
            socket
                .send(Message::Binary(setup.into_bytes().into()))
                .await
                .unwrap();
            socket
                .send(Message::Binary(transcript.into_bytes().into()))
                .await
                .unwrap();
            corrupted.await.unwrap();
            socket.send(Message::Binary(invalid.into())).await.unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
        });
        client
            .connect_for_probe("binary-json-fixture")
            .await
            .unwrap();
        let source = tokio::time::timeout(Duration::from_secs(2), async {
            assert_eq!(
                receiver.recv().await,
                Some(LiveTranslateServerEvent::SessionCreated)
            );
            receiver.recv().await.unwrap()
        })
        .await
        .unwrap();
        assert!(
            matches!(source, LiveTranslateServerEvent::SourceUtteranceFinal { text, .. }
            if text == case["expectedSource"].as_str().unwrap())
        );

        corrupt.send(()).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            error,
            LiveTranslateServerEvent::Error {
                code: "audio3_protocol_error".into(),
                message: GENERIC_PROTOCOL_ERROR.into(),
            }
        );
        let task = client.inner.receive_task.lock().await.take().unwrap();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert!(receiver.try_recv().is_err());
        client.disconnect().await;
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    async fn connected_finish_fixture() -> (
        Audio3ASRClient,
        crate::clients::provider_events::ProviderEventReceiver,
        WebSocketStream<tokio::net::TcpStream>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = Audio3ASRClient::new_custom(
            &format!("ws://{}/asr", listener.local_addr().unwrap()),
            "synthetic-model",
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
        let (sender, mut receiver) = provider_event_channel();
        client.set_event_sender(sender).await;
        let accept = async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let run = socket.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(run.contains("run-task"));
            socket
                .send(Message::Text(
                    r#"{"header":{"event":"task-started","task_id":"finish-fixture"}}"#.into(),
                ))
                .await
                .unwrap();
            socket
        };
        let (connected, socket) = tokio::join!(client.connect_for_probe("finish-fixture"), accept);
        connected.unwrap();
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionCreated)
        );
        (client, receiver, socket)
    }

    #[tokio::test]
    async fn unsolicited_task_finish_requests_recovery_without_promoting_a_draft() {
        let (client, mut receiver, mut socket) = connected_finish_fixture().await;
        socket.send(Message::Text(serde_json::json!({
            "header":{"event":"result-generated","task_id":"finish-fixture"},
            "payload":{"output":{"sentence":{"text":"Synthetic unfinished caption", "sentence_id":7,"sentence_end":false}}}
        }).to_string().into())).await.unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap(),
            Some(LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: 7,
                ..
            })
        ));
        socket
            .send(Message::Text(
                r#"{"header":{"event":"task-finished","task_id":"finish-fixture"}}"#.into(),
            ))
            .await
            .unwrap();
        socket.send(Message::Text(serde_json::json!({
            "header":{"event":"result-generated","task_id":"finish-fixture"},
            "payload":{"output":{"sentence":{"text":"Synthetic late final", "sentence_id":7,"sentence_end":true}}}
        }).to_string().into())).await.unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            event,
            LiveTranslateServerEvent::Error {
                code: "transport_error".into(),
                message: GENERIC_TRANSPORT_ERROR.into(),
            }
        );
        let task = client.inner.receive_task.lock().await.take().unwrap();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert!(receiver.try_recv().is_err());
        assert!(!client.inner.task_finished.load(Ordering::SeqCst));
        client.disconnect().await;
    }

    #[tokio::test]
    async fn requested_finish_preserves_final_tail_but_does_not_authorize_reused_task_end() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = Audio3ASRClient::new("synthetic-key", SourceLanguage::English).unwrap();
        client.endpoint.url =
            url::Url::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
        let (sender, mut receiver) = provider_event_channel();
        client.set_event_sender(sender).await;
        let (trigger, triggered) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut triggered = Some(triggered);
            for connection in 0..2 {
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
                if connection == 0 {
                    assert!(socket
                        .next()
                        .await
                        .unwrap()
                        .unwrap()
                        .to_text()
                        .unwrap()
                        .contains("finish-task"));
                    socket.send(Message::Text(serde_json::json!({
                        "header":{"event":"result-generated"},
                        "payload":{"output":{"sentence":{"text":"Synthetic final tail", "sentence_id":3,"sentence_end":true}}}
                    }).to_string().into())).await.unwrap();
                } else {
                    triggered.take().unwrap().await.unwrap();
                }
                socket
                    .send(Message::Text(
                        r#"{"header":{"event":"task-finished"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                assert!(matches!(
                    socket.next().await,
                    None | Some(Ok(Message::Close(_)))
                ));
            }
        });
        client
            .connect_for_probe("synthetic-reused-task")
            .await
            .unwrap();
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionCreated)
        );
        client.finish(Duration::from_secs(1)).await;
        assert!(matches!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 3,
                ..
            })
        ));
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        );
        client
            .connect_for_probe("synthetic-reused-task")
            .await
            .unwrap();
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionCreated)
        );
        trigger.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap(),
            Some(LiveTranslateServerEvent::Error {
                code: "transport_error".into(),
                message: GENERIC_TRANSPORT_ERROR.into(),
            })
        );
        client.disconnect().await;
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn unsolicited_task_finish_during_setup_is_an_immediate_transport_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = Audio3ASRClient::new("synthetic-key", SourceLanguage::English).unwrap();
        client.endpoint.url =
            url::Url::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
        let (sender, mut receiver) = provider_event_channel();
        client.set_event_sender(sender).await;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _run = socket.next().await.unwrap().unwrap();
            socket
                .send(Message::Text(
                    r#"{"header":{"event":"task-finished"}}"#.into(),
                ))
                .await
                .unwrap();
            assert!(matches!(
                socket.next().await,
                None | Some(Ok(Message::Close(_)))
            ));
        });
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            client.connect_for_probe("synthetic-setup"),
        )
        .await
        .expect("an unexpected finish must not wait for the setup deadline")
        .unwrap_err();
        assert!(error.is_unreachable());
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::Error {
                code: "transport_error".into(),
                message: GENERIC_TRANSPORT_ERROR.into(),
            })
        );
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn finish_is_inert_without_a_connection_and_after_a_completed_finish() {
        let client = Audio3ASRClient::new("synthetic-key", SourceLanguage::English).unwrap();
        let (sender, mut receiver) = provider_event_channel();
        client.set_event_sender(sender).await;
        client.finish(Duration::from_millis(40)).await;
        assert!(receiver.try_recv().is_err());

        let (client, mut receiver, mut socket) = connected_finish_fixture().await;
        let server = tokio::spawn(async move {
            assert!(socket
                .next()
                .await
                .unwrap()
                .unwrap()
                .into_text()
                .unwrap()
                .contains("finish-task"));
            assert!(matches!(
                socket.next().await,
                Some(Ok(Message::Close(_))) | None
            ));
        });
        client.finish(Duration::from_millis(40)).await;
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        );
        client.finish(Duration::from_millis(40)).await;
        client.disconnect().await;
        client.finish(Duration::from_millis(40)).await;
        assert!(receiver.try_recv().is_err());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn finish_preserves_provider_errors_without_a_synthetic_success() {
        let (client, mut receiver, mut socket) = connected_finish_fixture().await;
        let server = tokio::spawn(async move {
            assert!(socket
                .next()
                .await
                .unwrap()
                .unwrap()
                .into_text()
                .unwrap()
                .contains("finish-task"));
            socket.send(Message::Text(r#"{"header":{"event":"task-failed","task_id":"finish-fixture","error_code":"SERVER_ERROR","error_message":"synthetic failure"}}"#.into())).await.unwrap();
            assert!(matches!(
                socket.next().await,
                Some(Ok(Message::Close(_))) | None
            ));
        });
        client.finish(Duration::from_secs(1)).await;
        assert!(matches!(
            receiver.try_recv(),
            Ok(LiveTranslateServerEvent::Error { .. })
        ));
        assert!(receiver.try_recv().is_err());
        client.finish(Duration::from_millis(40)).await;
        assert!(receiver.try_recv().is_err());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn finish_send_failure_cannot_become_a_synthetic_success() {
        let (client, mut receiver, mut socket) = connected_finish_fixture().await;
        // Keep the peer unread so the receive task cannot independently turn
        // this into a transport error. The writer itself rejects further data.
        client
            .inner
            .sink
            .lock()
            .await
            .as_mut()
            .unwrap()
            .close()
            .await
            .unwrap();
        client.finish(Duration::from_secs(1)).await;
        assert!(receiver.try_recv().is_err());
        assert!(matches!(
            socket.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    }

    #[tokio::test]
    async fn finish_command_wait_uses_the_ack_budget_and_stops_its_producer() {
        let (client, mut receiver, mut socket) = connected_finish_fixture().await;
        let writer = client.inner.sink.lock().await;
        let finisher = {
            let client = client.clone();
            tokio::spawn(async move { client.finish(Duration::from_millis(40)).await })
        };
        // An occupied writer cannot make finish spend an additional send
        // timeout. Producer drain is observable before cleanup acquires it.
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap(),
            Some(LiveTranslateServerEvent::SessionFinished)
        );
        assert!(client.inner.receive_task.lock().await.is_none());
        drop(writer);
        finisher.await.unwrap();
        assert!(matches!(
            socket.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    }

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
    async fn custom_unsupported_language_remains_terminal_and_content_free() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let run = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let run: serde_json::Value = serde_json::from_str(&run).unwrap();
            assert_eq!(
                run["payload"]["parameters"]["language_hints"],
                serde_json::json!(["ja"])
            );
            socket.send(Message::Text(serde_json::json!({"header": {
                "event":"task-failed", "task_id":run["header"]["task_id"],
                "error_code":"UNSUPPORTED_LANGUAGE", "error_message":"synthetic-private-response"
            }}).to_string().into())).await.unwrap();
            let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .unwrap();
            assert!(matches!(next, None | Some(Ok(Message::Close(_)))));
        });
        let client = Audio3ASRClient::new_custom(
            &format!("ws://{address}/asr"),
            "fixture",
            "fixture-key",
            SourceLanguage::Japanese,
        )
        .unwrap();
        let (sender, mut events) = provider_event_channel();
        client.set_event_sender(sender).await;
        let token = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
        assert_eq!(
            client
                .connect("fixture-task")
                .await
                .unwrap_err()
                .to_string(),
            token
        );
        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::Error {
                code: token.into(),
                message: token.into()
            })
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
        {
            let now = tokio::time::Instant::now();
            let mut clock = client.inner.audio_clock.lock().await;
            clock.started_at = Some(now - Duration::from_millis(151));
            clock.last_sent_at = Some(now - SILENCE_INTERVAL - Duration::from_millis(1));
        }
        if bind_gate {
            // A real PCM writer can hold this lock while the receive loop's
            // heartbeat fires. Pending work must let that loop keep reading.
            let held_sink = client.inner.sink.lock().await;
            {
                let idle_send = send_silence_if_idle(&client.inner);
                tokio::pin!(idle_send);
                assert!(
                    matches!(
                        futures_util::poll!(idle_send.as_mut()),
                        std::task::Poll::Ready(Ok(()))
                    ),
                    "pending real PCM must skip the heartbeat without waiting for its writer"
                );
            }
            drop(held_sink);
        } else {
            send_silence_if_idle(&client.inner).await.unwrap();
        }
        release_tx.send(()).unwrap();
        assert!(pipeline.finish(Duration::from_secs(2)).await);
        assert_eq!(pending_gate.pending_count(), 0);
        if idle_after_drain {
            {
                let now = tokio::time::Instant::now();
                let mut clock = client.inner.audio_clock.lock().await;
                clock.started_at = Some(now - Duration::from_millis(350));
                clock.last_sent_at = Some(now - SILENCE_INTERVAL - Duration::from_millis(1));
            }
            send_silence_if_idle(&client.inner).await.unwrap();
        }
        client.disconnect().await;
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn failed_pcm_send_does_not_consume_silence_budget() {
        let (client, _receiver, mut socket) = connected_finish_fixture().await;
        client
            .inner
            .audio_clock
            .lock()
            .await
            .start(tokio::time::Instant::now() - Duration::from_secs(1));
        let before = {
            let clock = client.inner.audio_clock.lock().await;
            (clock.audio_end_ns, clock.last_sent_at)
        };
        client
            .inner
            .sink
            .lock()
            .await
            .as_mut()
            .unwrap()
            .close()
            .await
            .unwrap();
        assert!(client.send_audio(&[1, 0].repeat(1600)).await.is_err());
        assert!(send_silence_if_idle(&client.inner).await.is_err());
        let clock = client.inner.audio_clock.lock().await;
        assert_eq!((clock.audio_end_ns, clock.last_sent_at), before);
        drop(clock);
        client.disconnect().await;
        assert!(matches!(
            socket.next().await,
            Some(Ok(Message::Close(_))) | None
        ));
    }

    #[tokio::test]
    async fn repeated_task_ready_does_not_reset_successful_pcm_budget() {
        let (client, mut receiver, mut socket) = connected_finish_fixture().await;
        client.send_audio(&[1, 0].repeat(1600)).await.unwrap();
        assert!(matches!(socket.next().await, Some(Ok(Message::Binary(_)))));
        let before = {
            let clock = client.inner.audio_clock.lock().await;
            (clock.started_at, clock.audio_end_ns, clock.last_sent_at)
        };
        socket
            .send(Message::Text(
                r#"{"header":{"event":"task-started","task_id":"finish-fixture"}}"#.into(),
            ))
            .await
            .unwrap();
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionCreated)
        );
        let clock = client.inner.audio_clock.lock().await;
        assert_eq!(
            (clock.started_at, clock.audio_end_ns, clock.last_sent_at),
            before
        );
        drop(clock);
        client.disconnect().await;
    }

    #[tokio::test]
    async fn batched_continuous_pcm_does_not_accumulate_idle_silence() {
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
            let mut real_frames = Vec::new();
            let mut silence_bytes = 0usize;
            while let Some(message) = socket.next().await {
                match message.unwrap() {
                    Message::Binary(bytes) if bytes.iter().all(|byte| *byte == 0) => {
                        silence_bytes += bytes.len();
                    }
                    Message::Binary(bytes) => real_frames.push(bytes.to_vec()),
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            (real_frames, silence_bytes)
        });
        let mut client = Audio3ASRClient::new("fixture-only", SourceLanguage::English).unwrap();
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
        let (sender, mut events) = provider_event_channel();
        client.set_event_sender(sender).await;
        let gate = PendingPcmGate::default();
        client.set_audio_pending_gate(gate.clone());
        client.connect("batched-pcm-fixture").await.unwrap();
        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::SessionCreated)
        );
        let started = tokio::time::Instant::now();
        let mut previous_capture = started;
        let mut longest_gap = Duration::ZERO;
        for index in 1..=10u32 {
            tokio::time::sleep_until(started + Duration::from_millis(200) * index).await;
            let captured = tokio::time::Instant::now();
            longest_gap = longest_gap.max(captured.saturating_duration_since(previous_capture));
            previous_capture = captured;
            let pcm = [index as u8, 0].repeat(3200); // 200 ms of 16 kHz mono PCM16.
            let pending = gate.acquire();
            client.send_audio(&pcm).await.unwrap();
            drop(pending);
        }
        let elapsed = started.elapsed();
        client.disconnect().await;
        let (real_frames, silence_bytes) = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        let real_bytes: usize = real_frames.iter().map(Vec::len).sum();
        eprintln!("batched continuous PCM: realBytes={real_bytes} silenceBytes={silence_bytes} elapsedMs={}", elapsed.as_millis());
        assert_eq!(
            real_frames,
            (1..=10u8)
                .map(|value| [value, 0].repeat(3200))
                .collect::<Vec<_>>()
        );
        // A delayed CI executor may deliver a larger initial batch. Allow that
        // measured gap and genuinely elapsed idle time, never one extra gap per
        // callback. The clock tests above cover an exact virtual one-hour run.
        let allowed_gap =
            elapsed.saturating_sub(Duration::from_secs(2)) + longest_gap + SILENCE_INTERVAL;
        assert!(
            silence_bytes as u128 * 1_000_000_000 / 32_000 <= allowed_gap.as_nanos(),
            "idle silence accumulated beyond the measured capture gap: {silence_bytes} bytes"
        );
    }
}
