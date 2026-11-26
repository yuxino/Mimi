//! Audio 3.0 high-quality ASR WebSocket client.

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::core::audio::pending_pcm::PendingPcmGate;
use crate::core::models::SourceLanguage;
use crate::core::protocols::realtime::live_translate::{
    LiveTranslateProtocolError, LiveTranslateServerEvent,
};
use crate::core::protocols::recognition::audio3::{
    Audio3ASREndpoint, Audio3ASRRequestEncoder, Audio3ASRServerEvent, Audio3ASRServerEventDecoder,
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
    network: crate::clients::transport::provider_network::ProviderNetwork,
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
        network: crate::clients::transport::provider_network::ProviderNetwork,
    ) -> Result<(), crate::clients::transport::provider_network::ProviderNetworkError> {
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
            network: crate::clients::transport::provider_network::ProviderNetwork::default(),
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
        let endpoint = crate::core::protocols::recognition::custom_speech::endpoint(
            endpoint,
            crate::core::services::provider::ProviderKind::CustomDashScopeASR,
        )
        .map_err(|_| Audio3ASRClientError::InvalidCustomEndpoint)?;
        let model = crate::core::protocols::recognition::custom_speech::validate_model(model)
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
            crate::core::protocols::recognition::audio3::Audio3ASRContext::audiovisual_dialogue(
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
            crate::clients::transport::provider_network::websocket_with_message_limit(
                request,
                &self.network,
                crate::core::protocols::recognition::audio3::MAX_AUDIO3_MESSAGE_BYTES,
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
                                crate::clients::transport::provider_events::is_content_event(
                                    &subtitle_event,
                                );
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
        if pcm_data.len() > crate::core::protocols::recognition::custom_speech::MAX_PCM_CHUNK_BYTES
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
mod tests;

#[cfg(test)]
mod streaming_tests;
