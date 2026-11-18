//! Standalone OpenAI-compatible realtime transcription. Text translation is
//! supplied by the existing bounded pipeline, never requested on this socket.

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::clients::transport::provider_network::{ProviderNetwork, ProviderNetworkError};
use crate::core::audio::pending_pcm::PendingPcmGate;
use crate::core::models::SourceLanguage;
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::recognition::custom_speech::{
    self, AudioTurnAction, PcmTurnGate, ServerEvent, TranscriptOutput, TurnSequencer,
};
use crate::core::services::provider::ProviderKind;
use futures_util::{SinkExt, StreamExt};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex, Notify};
use tokio::task::AbortHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(250);
const IDLE_COMMIT_DELAY: Duration = Duration::from_millis(600);
const MAX_FINAL_WAIT: Duration = Duration::from_secs(45);
const CLEAR_TIMEOUT: Duration = Duration::from_secs(1);
type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CustomSpeechClientError {
    #[error("custom_speech_credentials_missing")]
    MissingAPIKey,
    #[error("custom_speech_endpoint_invalid")]
    InvalidEndpoint,
    #[error("custom_speech_model_invalid")]
    InvalidModel,
    #[error("custom_speech_authentication_failed")]
    AuthenticationFailed,
    #[error("custom_speech_unreachable")]
    Unreachable,
    #[error("custom_speech_session_rejected")]
    SessionRejected,
    #[error("custom_speech_setup_timeout")]
    SetupTimeout,
    #[error("custom_speech_protocol_invalid")]
    InvalidProtocol,
    #[error("custom_speech_audio_invalid")]
    InvalidAudio,
    #[error("custom_speech_not_connected")]
    NotConnected,
    #[error("custom_speech_health_timeout")]
    HealthTimeout,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected,
}

#[derive(Default)]
struct ContentState {
    revision: u64,
    pcm: PcmTurnGate,
    turns: TurnSequencer,
    last_pcm_at: Option<tokio::time::Instant>,
    pending_started: VecDeque<(u64, tokio::time::Instant)>,
}

struct Inner {
    lifecycle: Mutex<()>,
    sink: Mutex<Option<Sink>>,
    receive_task: std::sync::Mutex<Option<AbortHandle>>,
    audio_operation: Mutex<()>,
    content: std::sync::Mutex<ContentState>,
    generation: AtomicU64,
    ready: AtomicBool,
    closing: AtomicBool,
    failed: AtomicBool,
    failure_reason: std::sync::Mutex<Option<CustomSpeechClientError>>,
    clearing: AtomicBool,
    pong: Notify,
    cleared: Notify,
    progress: Notify,
    events: std::sync::Mutex<Option<ProviderEventSender>>,
    #[cfg(test)]
    before_final_publication: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

#[derive(Clone)]
pub struct CustomSpeechClient {
    network: ProviderNetwork,
    endpoint: url::Url,
    model: String,
    api_key: String,
    source: SourceLanguage,
    azure_api_key: bool,
    builtin_asr: bool,
    inner: Arc<Inner>,
}

/// Dropping an in-flight connection must close its socket even when the
/// caller keeps the client for a later retry (for example a cancelled probe).
struct ConnectAttempt {
    inner: Arc<Inner>,
    generation: u64,
    complete: bool,
}
impl Drop for ConnectAttempt {
    fn drop(&mut self) {
        let retired = self.generation.wrapping_add(1);
        if self.complete
            || self
                .inner
                .generation
                .compare_exchange(self.generation, retired, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            return;
        }
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.closing.store(true, Ordering::SeqCst);
        if let Some(receiver) = self.inner.receive_task.lock().unwrap().take() {
            receiver.abort();
        }
        if let Ok(mut sink) = self.inner.sink.try_lock() {
            sink.take();
        } else if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let inner = self.inner.clone();
            runtime.spawn(async move {
                let mut sink = inner.sink.lock().await;
                if inner.generation.load(Ordering::SeqCst) == retired {
                    sink.take();
                }
            });
        }
    }
}

impl CustomSpeechClient {
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.clone()
    }

    pub fn new(
        endpoint: &str,
        model: &str,
        api_key: &str,
        source: SourceLanguage,
    ) -> Result<Self, CustomSpeechClientError> {
        let mut endpoint = custom_speech::endpoint(endpoint, ProviderKind::CustomOpenAIASR)
            .map_err(|_| CustomSpeechClientError::InvalidEndpoint)?;
        endpoint
            .query_pairs_mut()
            .append_pair("intent", "transcription");
        let model = custom_speech::validate_model(model)
            .map_err(|_| CustomSpeechClientError::InvalidModel)?;
        let api_key = api_key.trim();
        if api_key.is_empty() || HeaderValue::from_str(&format!("Bearer {api_key}")).is_err() {
            return Err(CustomSpeechClientError::MissingAPIKey);
        }
        Ok(Self {
            network: ProviderNetwork::default(),
            endpoint,
            model,
            api_key: api_key.to_owned(),
            source,
            azure_api_key: false,
            builtin_asr: false,
            inner: Arc::new(Inner {
                lifecycle: Mutex::new(()),
                sink: Mutex::new(None),
                receive_task: std::sync::Mutex::new(None),
                audio_operation: Mutex::new(()),
                content: Default::default(),
                generation: AtomicU64::new(0),
                ready: AtomicBool::new(false),
                closing: AtomicBool::new(false),
                failed: AtomicBool::new(false),
                failure_reason: Default::default(),
                clearing: AtomicBool::new(false),
                pong: Notify::new(),
                cleared: Notify::new(),
                progress: Notify::new(),
                events: Default::default(),
                #[cfg(test)]
                before_final_publication: Default::default(),
            }),
        })
    }

    /// First-party OpenAI uses the same bounded transcription transport.
    pub fn openai(api_key: &str, source: SourceLanguage) -> Result<Self, CustomSpeechClientError> {
        if !crate::core::protocols::recognition::realtime_asr::supports_language(
            ProviderKind::OpenAIRealtime,
            source,
        ) {
            return Err(CustomSpeechClientError::InvalidModel);
        }
        let mut client = Self::new(
            "wss://api.openai.com/v1/realtime",
            "gpt-live-transcribe",
            api_key,
            source,
        )?;
        client.builtin_asr = true;
        Ok(client)
    }

    /// Azure uses a resource-local transcription deployment and API-key header.
    pub fn azure(
        resource: &str,
        deployment: &str,
        api_key: &str,
        source: SourceLanguage,
    ) -> Result<Self, CustomSpeechClientError> {
        let mut endpoint =
            crate::core::protocols::realtime::azure_openai_realtime::AzureOpenAIRealtimeEndpoint::new(
                resource, deployment,
            )
            .map_err(|_| CustomSpeechClientError::InvalidEndpoint)?
            .url()
            .clone();
        endpoint.set_path("/openai/v1/realtime");
        endpoint.set_query(None);
        endpoint
            .query_pairs_mut()
            .append_pair("intent", "transcription");
        let mut client = Self::new(
            "wss://api.openai.com/v1/realtime",
            deployment,
            api_key,
            source,
        )?;
        client.endpoint = endpoint;
        client.azure_api_key = true;
        client.builtin_asr = true;
        Ok(client)
    }

    pub fn set_network(&mut self, network: ProviderNetwork) -> Result<(), ProviderNetworkError> {
        self.network = network;
        Ok(())
    }
    pub fn set_audio_pending_gate(&self, _gate: PendingPcmGate) { /* Only Audio3 synthesizes idle PCM. */
    }

    pub async fn set_event_sender(&self, events: ProviderEventSender) {
        self.inner.content.lock().unwrap().revision = events.content_revision();
        *self.inner.events.lock().unwrap() = Some(events);
    }

    pub fn content_revision(&self) -> u64 {
        self.inner.content.lock().unwrap().revision
    }

    pub async fn connect(&self, _task_id: &str) -> Result<(), CustomSpeechClientError> {
        self.connect_with_timeout(Duration::from_secs(10)).await
    }
    pub async fn connect_for_probe(&self, task_id: &str) -> Result<(), CustomSpeechClientError> {
        self.connect(task_id).await
    }

    async fn connect_with_timeout(
        &self,
        setup_timeout: Duration,
    ) -> Result<(), CustomSpeechClientError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        self.disconnect_locked().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let mut attempt = ConnectAttempt {
            inner: self.inner.clone(),
            generation,
            complete: false,
        };
        let events = self
            .inner
            .events
            .lock()
            .unwrap()
            .clone()
            .ok_or(CustomSpeechClientError::NotConnected)?;
        self.inner.content.lock().unwrap().revision = events.content_revision();
        let mut request = self
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|_| CustomSpeechClientError::InvalidEndpoint)?;
        request.headers_mut().insert(
            if self.azure_api_key {
                "api-key"
            } else {
                "Authorization"
            },
            HeaderValue::from_str(&if self.azure_api_key {
                self.api_key.clone()
            } else {
                format!("Bearer {}", self.api_key)
            })
            .map_err(|_| CustomSpeechClientError::MissingAPIKey)?,
        );
        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            crate::clients::transport::provider_network::websocket_with_message_limit(
                request,
                &self.network,
                custom_speech::MAX_CUSTOM_SPEECH_MESSAGE_BYTES,
            ),
        )
        .await
        .map_err(|_| CustomSpeechClientError::Unreachable)?
        .map_err(|error| {
            if crate::clients::transport::connection_diagnostics::authentication_rejected(&error) {
                CustomSpeechClientError::AuthenticationFailed
            } else {
                CustomSpeechClientError::Unreachable
            }
        })?;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.closing.store(false, Ordering::SeqCst);
        self.inner.failed.store(false, Ordering::SeqCst);
        let (setup, mut acknowledged) = watch::channel(SetupState::Awaiting);
        let client = self.clone();
        let evidence_context = crate::development_audio::context();
        let task = tokio::spawn(crate::development_audio::scope_context(
            evidence_context,
            async move {
                client.receive(stream, events, setup, generation).await;
            },
        ));
        *self.inner.receive_task.lock().unwrap() = Some(task.abort_handle());
        let mut update = custom_speech::session_update(
            &self.model,
            self.source,
            &format!("mimi-asr-setup-{generation}"),
        );
        if self.builtin_asr && self.source != SourceLanguage::Automatic {
            let transcription = update
                .pointer_mut("/session/audio/input/transcription")
                .unwrap()
                .as_object_mut()
                .unwrap();
            transcription.remove("language");
            transcription.insert(
                "languages".into(),
                serde_json::json!([
                    crate::core::protocols::recognition::realtime_asr::language_hint(
                        ProviderKind::OpenAIRealtime,
                        self.source
                    )
                    .ok_or(CustomSpeechClientError::InvalidModel)?
                ]),
            );
        }
        let setup = async {
            self.send_message(Message::Text(update.to_string().into()))
                .await?;
            loop {
                let status = *acknowledged.borrow();
                match status {
                    SetupState::Ready => {
                        if !self.is_current(generation) || self.inner.failed.load(Ordering::SeqCst)
                        {
                            return Err(self.failure_reason());
                        }
                        self.inner.ready.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                    SetupState::Rejected => return Err(self.failure_reason()),
                    SetupState::Awaiting => {}
                }
                acknowledged
                    .changed()
                    .await
                    .map_err(|_| CustomSpeechClientError::SessionRejected)?;
            }
        };
        let result = tokio::time::timeout(setup_timeout, setup)
            .await
            .unwrap_or(Err(CustomSpeechClientError::SetupTimeout));
        if result.is_err() {
            self.disconnect_locked().await;
        }
        attempt.complete = true;
        result
    }

    pub async fn send_audio(&self, pcm: &[u8]) -> Result<(), CustomSpeechClientError> {
        if pcm.is_empty() || self.inner.failed.load(Ordering::SeqCst) {
            return Ok(());
        }
        let operation = async {
            let _audio = self.inner.audio_operation.lock().await;
            if !self.inner.ready.load(Ordering::SeqCst) || self.inner.closing.load(Ordering::SeqCst)
            {
                return Err(CustomSpeechClientError::NotConnected);
            }
            let actions = {
                let mut content = self.inner.content.lock().unwrap();
                content.last_pcm_at = Some(tokio::time::Instant::now());
                content
                    .pcm
                    .push(pcm)
                    .map_err(|_| CustomSpeechClientError::InvalidAudio)?
            };
            self.send_actions(actions).await
        };
        tokio::time::timeout(SEND_TIMEOUT, operation)
            .await
            .unwrap_or(Err(CustomSpeechClientError::Unreachable))
    }

    async fn send_actions(
        &self,
        actions: Vec<AudioTurnAction>,
    ) -> Result<(), CustomSpeechClientError> {
        for action in actions {
            match action {
                AudioTurnAction::Start => {
                    let mut content = self.inner.content.lock().unwrap();
                    let revision = content.revision;
                    let sequence = content
                        .turns
                        .start(revision)
                        .map_err(|_| CustomSpeechClientError::InvalidProtocol)?;
                    content
                        .pending_started
                        .push_back((sequence, tokio::time::Instant::now()));
                }
                AudioTurnAction::Append(pcm) => {
                    self.send_message(Message::Text(
                        custom_speech::audio_append(&pcm).to_string().into(),
                    ))
                    .await?
                }
                AudioTurnAction::Commit => {
                    self.inner
                        .content
                        .lock()
                        .unwrap()
                        .turns
                        .commit()
                        .map_err(|_| CustomSpeechClientError::InvalidProtocol)?;
                    self.send_message(Message::Text(
                        custom_speech::audio_commit().to_string().into(),
                    ))
                    .await?;
                }
            }
        }
        Ok(())
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), CustomSpeechClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) {
            return Err(CustomSpeechClientError::NotConnected);
        }
        let operation = async {
            let pong = self.inner.pong.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            self.send_message(Message::Ping(Default::default())).await?;
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .unwrap_or(Err(CustomSpeechClientError::HealthTimeout))
    }

    pub async fn clear_content(&self) -> u64 {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let _audio = self.inner.audio_operation.lock().await;
        let revision = self.inner.events.lock().unwrap().as_ref().map_or_else(
            || self.content_revision().wrapping_add(1),
            ProviderEventSender::advance_content_revision,
        );
        {
            let mut content = self.inner.content.lock().unwrap();
            content.revision = revision;
            content.pcm = PcmTurnGate::default();
            content.turns.clear();
            trim_pending_started(&mut content);
        }
        if self.inner.ready.load(Ordering::SeqCst) && !self.inner.failed.load(Ordering::SeqCst) {
            self.inner.clearing.store(true, Ordering::SeqCst);
            let cleared = self.inner.cleared.notified();
            tokio::pin!(cleared);
            cleared.as_mut().enable();
            let result = tokio::time::timeout(CLEAR_TIMEOUT, async {
                self.send_message(Message::Text(
                    custom_speech::audio_clear().to_string().into(),
                ))
                .await?;
                cleared.await;
                Ok::<(), CustomSpeechClientError>(())
            })
            .await;
            if !matches!(result, Ok(Ok(()))) {
                self.fail(
                    CustomSpeechClientError::InvalidProtocol,
                    self.inner.generation.load(Ordering::SeqCst),
                );
            }
        }
        revision
    }

    pub async fn finish(&self, timeout: Duration) {
        let _lifecycle = self.inner.lifecycle.lock().await;
        if !self.inner.ready.load(Ordering::SeqCst) {
            self.disconnect_locked().await;
            return;
        }
        self.inner.closing.store(true, Ordering::SeqCst);
        let operation = async {
            {
                let _audio = self.inner.audio_operation.lock().await;
                let actions = self.inner.content.lock().unwrap().pcm.finish();
                let _ = self.send_actions(actions).await;
            }
            loop {
                let progress = self.inner.progress.notified();
                tokio::pin!(progress);
                progress.as_mut().enable();
                if self.inner.content.lock().unwrap().turns.is_empty()
                    || self.inner.failed.load(Ordering::SeqCst)
                {
                    break;
                }
                progress.await;
            }
        };
        let _ = tokio::time::timeout(timeout, operation).await;
        let generation = self.inner.generation.load(Ordering::SeqCst);
        self.emit(LiveTranslateServerEvent::SessionFinished, None, generation);
        self.disconnect_locked().await;
    }

    pub async fn disconnect(&self) {
        let _lifecycle = self.inner.lifecycle.lock().await;
        self.disconnect_locked().await;
    }

    async fn disconnect_locked(&self) {
        // Invalidate and cancel the receiver before waiting for audio work:
        // its idle timer may itself be holding audio_operation during a send.
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.closing.store(true, Ordering::SeqCst);
        self.inner.clearing.store(false, Ordering::SeqCst);
        if let Some(task) = self.inner.receive_task.lock().unwrap().take() {
            task.abort();
        }
        let _audio = self.inner.audio_operation.lock().await;
        if let Some(mut sink) = self.inner.sink.lock().await.take() {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.close()).await;
        }
        let revision = self.content_revision();
        *self.inner.content.lock().unwrap() = ContentState {
            revision,
            ..Default::default()
        };
        self.inner.failed.store(false, Ordering::SeqCst);
        *self.inner.failure_reason.lock().unwrap() = None;
    }

    async fn send_message(&self, message: Message) -> Result<(), CustomSpeechClientError> {
        let operation = async {
            let mut sink = self.inner.sink.lock().await;
            let sink = sink.as_mut().ok_or(CustomSpeechClientError::NotConnected)?;
            let evidence = match &message {
                Message::Text(text) => {
                    crate::development_audio::begin_json(text, custom_speech::PCM_SAMPLE_RATE)
                }
                _ => crate::development_audio::begin_pcm(&[], custom_speech::PCM_SAMPLE_RATE),
            };
            evidence
                .observe(sink.send(message))
                .await
                .map_err(|_| CustomSpeechClientError::Unreachable)
        };
        tokio::time::timeout(SEND_TIMEOUT, operation)
            .await
            .unwrap_or(Err(CustomSpeechClientError::Unreachable))
    }

    async fn receive(
        &self,
        mut stream: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        events: ProviderEventSender,
        setup: watch::Sender<SetupState>,
        generation: u64,
    ) {
        let mut idle = tokio::time::interval(Duration::from_millis(100));
        idle.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let message = tokio::select! {
                message = stream.next() => message,
                _ = idle.tick() => {
                    if !self.is_current(generation) { return; }
                    if let Err(error) = self.tick().await { self.fail(error, generation); let _ = setup.send(SetupState::Rejected); return; }
                    continue;
                }
            };
            if !self.is_current(generation) {
                return;
            }
            let event = match message {
                Some(Ok(Message::Text(text))) => match custom_speech::decode(&text) {
                    Ok(event) => event,
                    Err(_) => {
                        self.fail(CustomSpeechClientError::InvalidProtocol, generation);
                        let _ = setup.send(SetupState::Rejected);
                        return;
                    }
                },
                Some(Ok(Message::Pong(_))) => {
                    self.inner.pong.notify_waiters();
                    continue;
                }
                Some(Ok(Message::Ping(bytes))) => {
                    let _ = self.send_message(Message::Pong(bytes)).await;
                    continue;
                }
                Some(Ok(Message::Frame(_))) => continue,
                Some(Ok(Message::Binary(_))) => {
                    self.fail(CustomSpeechClientError::InvalidProtocol, generation);
                    let _ = setup.send(SetupState::Rejected);
                    return;
                }
                _ => {
                    if !self.inner.closing.load(Ordering::SeqCst) {
                        self.fail(CustomSpeechClientError::Unreachable, generation);
                    }
                    let _ = setup.send(SetupState::Rejected);
                    return;
                }
            };
            match event {
                ServerEvent::Created | ServerEvent::Ignored => {}
                ServerEvent::Updated { model } => {
                    if model != self.model {
                        self.fail(CustomSpeechClientError::SessionRejected, generation);
                        let _ = setup.send(SetupState::Rejected);
                        return;
                    }
                    self.emit(LiveTranslateServerEvent::SessionCreated, None, generation);
                    let _ = setup.send(SetupState::Ready);
                }
                ServerEvent::Cleared => {
                    if self.inner.clearing.swap(false, Ordering::SeqCst) {
                        self.inner.content.lock().unwrap().turns.buffer_cleared();
                    }
                    self.inner.cleared.notify_waiters();
                }
                ServerEvent::AuthenticationRejected => {
                    self.fail(CustomSpeechClientError::AuthenticationFailed, generation);
                    let _ = setup.send(SetupState::Rejected);
                    return;
                }
                ServerEvent::Rejected => {
                    self.fail(CustomSpeechClientError::SessionRejected, generation);
                    let _ = setup.send(SetupState::Rejected);
                    return;
                }
                event => {
                    if *setup.borrow() != SetupState::Ready {
                        self.fail(CustomSpeechClientError::InvalidProtocol, generation);
                        let _ = setup.send(SetupState::Rejected);
                        return;
                    }
                    {
                        let mut content = self.inner.content.lock().unwrap();
                        if !self.is_current(generation) {
                            return;
                        }
                        let revision = content.revision;
                        let output = match event {
                            ServerEvent::Committed { item_id } => {
                                content.turns.acknowledge(item_id).map(|()| Vec::new())
                            }
                            ServerEvent::Delta { item_id, text }
                                if !self.inner.clearing.load(Ordering::SeqCst) =>
                            {
                                content
                                    .turns
                                    .delta(item_id, &text, revision)
                                    .map(|event| event.into_iter().collect())
                            }
                            ServerEvent::Delta { .. } => Ok(Vec::new()),
                            ServerEvent::Completed { item_id, text } => {
                                content.turns.complete(item_id, text, revision)
                            }
                            _ => unreachable!(),
                        }
                        .inspect(|_| {
                            trim_pending_started(&mut content);
                        });
                        let Ok(output) = output else {
                            drop(content);
                            self.fail(CustomSpeechClientError::InvalidProtocol, generation);
                            return;
                        };
                        // Retiring a turn and publishing its authoritative
                        // result are one synchronous operation. Otherwise
                        // finish can observe an empty turn queue and invalidate
                        // this receiver before its final reaches the event lane.
                        for output in output {
                            #[cfg(test)]
                            if output.is_final {
                                if let Some(pause) =
                                    self.inner.before_final_publication.lock().unwrap().take()
                                {
                                    pause();
                                }
                            }
                            if self.emit_transcript(output, &events, generation).is_err() {
                                return;
                            }
                        }
                    }
                    self.inner.progress.notify_waiters();
                }
            }
        }
    }

    async fn tick(&self) -> Result<(), CustomSpeechClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.closing.load(Ordering::SeqCst) {
            return Ok(());
        }
        let Ok(_audio) = self.inner.audio_operation.try_lock() else {
            return Ok(());
        };
        let actions = {
            let mut content = self.inner.content.lock().unwrap();
            if content
                .pending_started
                .front()
                .is_some_and(|(_, at)| at.elapsed() >= MAX_FINAL_WAIT)
            {
                return Err(CustomSpeechClientError::InvalidProtocol);
            }
            if content.pcm.is_active()
                && content
                    .last_pcm_at
                    .is_some_and(|at| at.elapsed() >= IDLE_COMMIT_DELAY)
            {
                content.pcm.finish()
            } else {
                Vec::new()
            }
        };
        self.send_actions(actions).await
    }

    fn is_current(&self, generation: u64) -> bool {
        self.inner.generation.load(Ordering::SeqCst) == generation
    }

    fn emit_transcript(
        &self,
        output: TranscriptOutput,
        events: &ProviderEventSender,
        generation: u64,
    ) -> Result<(), crate::clients::transport::provider_events::ProviderEventSendError> {
        let language =
            (self.source != SourceLanguage::Automatic).then(|| self.source.raw_value().to_owned());
        let event = if output.is_final {
            LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: output.sequence,
                text: output.text,
                language,
            }
        } else {
            LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: output.sequence,
                text: output.text,
                language,
            }
        };
        events.send_if(event, || {
            self.is_current(generation) && events.content_revision() == output.revision
        })
    }

    fn emit(&self, event: LiveTranslateServerEvent, revision: Option<u64>, generation: u64) {
        if let Some(events) = self.inner.events.lock().unwrap().as_ref() {
            let _ = events.send_if(event, || {
                self.is_current(generation)
                    && revision.is_none_or(|revision| events.content_revision() == revision)
            });
        }
    }

    fn fail(&self, error: CustomSpeechClientError, generation: u64) {
        let content = self.inner.content.lock().unwrap();
        if !self.is_current(generation) || self.inner.failed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner.ready.store(false, Ordering::SeqCst);
        *self.inner.failure_reason.lock().unwrap() = Some(error.clone());
        drop(content);
        self.emit(
            LiveTranslateServerEvent::Error {
                code: error.to_string(),
                message: error.to_string(),
            },
            None,
            generation,
        );
        self.inner.progress.notify_waiters();
        self.inner.cleared.notify_waiters();
    }

    fn failure_reason(&self) -> CustomSpeechClientError {
        self.inner
            .failure_reason
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(CustomSpeechClientError::SessionRejected)
    }
}

fn trim_pending_started(content: &mut ContentState) {
    let oldest = content.turns.oldest_sequence();
    while content
        .pending_started
        .front()
        .is_some_and(|(sequence, _)| oldest.is_none_or(|oldest| *sequence < oldest))
    {
        content.pending_started.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::transport::provider_events::{
        provider_event_channel, ProviderEventReceiver,
    };
    use crate::core::services::network_proxy::{ProxyConfig, ProxyMode};
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, accept_hdr_async};

    type ServerSocket = WebSocketStream<TcpStream>;
    const MODEL: &str = "gpt-4o-mini-transcribe";

    #[tokio::test]
    #[allow(clippy::result_large_err)]
    async fn builtin_openai_and_azure_use_transcription_intent_and_separate_authentication() {
        for azure in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut client = if azure {
                CustomSpeechClient::azure(
                    "https://synthetic.openai.azure.com",
                    "speech-deployment",
                    "synthetic-key",
                    SourceLanguage::Chinese,
                )
                .unwrap()
            } else {
                CustomSpeechClient::openai("synthetic-key", SourceLanguage::Chinese).unwrap()
            };
            let expected_path = if azure {
                "/openai/v1/realtime"
            } else {
                "/v1/realtime"
            };
            assert_eq!(client.endpoint.path(), expected_path);
            client.endpoint.set_scheme("ws").unwrap();
            client.endpoint.set_host(Some("127.0.0.1")).unwrap();
            client
                .endpoint
                .set_port(Some(listener.local_addr().unwrap().port()))
                .unwrap();
            client
                .set_network(
                    ProviderNetwork::resolve(&ProxyConfig {
                        mode: ProxyMode::Direct,
                        url: None,
                    })
                    .unwrap(),
                )
                .unwrap();
            let (events, _receiver) = provider_event_channel();
            client.set_event_sender(events).await;
            let server = tokio::spawn(async move {
                let mut socket = accept_hdr_async(
                    listener.accept().await.unwrap().0,
                    move |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                          response| {
                        assert_eq!(request.uri().path(), expected_path);
                        assert_eq!(request.uri().query(), Some("intent=transcription"));
                        assert_eq!(
                            request
                                .headers()
                                .get(if azure { "api-key" } else { "Authorization" })
                                .unwrap(),
                            if azure {
                                "synthetic-key"
                            } else {
                                "Bearer synthetic-key"
                            }
                        );
                        assert!(request
                            .headers()
                            .get(if azure { "Authorization" } else { "api-key" })
                            .is_none());
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                let update = wire(&mut socket).await;
                let model = if azure {
                    "speech-deployment"
                } else {
                    "gpt-live-transcribe"
                };
                assert_eq!(update["session"]["type"], "transcription");
                assert_eq!(
                    update["session"]["audio"]["input"]["transcription"]["model"],
                    model
                );
                assert_eq!(
                    update["session"]["audio"]["input"]["transcription"]["languages"],
                    json!(["zh-cn"])
                );
                assert!(update["session"]["audio"]["input"]["turn_detection"].is_null());
                send(
                    &mut socket,
                    json!({"type":"session.updated", "session":update["session"]}),
                )
                .await;
                while socket.next().await.is_some() {}
            });
            client.connect("test").await.unwrap();
            client.disconnect().await;
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn cancelled_builtin_probe_setup_closes_socket_without_dropping_the_client() {
        for azure in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut client = if azure {
                CustomSpeechClient::azure(
                    "https://synthetic.openai.azure.com",
                    "speech-deployment",
                    "synthetic-key",
                    SourceLanguage::English,
                )
                .unwrap()
            } else {
                CustomSpeechClient::openai("synthetic-key", SourceLanguage::English).unwrap()
            };
            client.endpoint.set_scheme("ws").unwrap();
            client.endpoint.set_host(Some("127.0.0.1")).unwrap();
            client
                .endpoint
                .set_port(Some(listener.local_addr().unwrap().port()))
                .unwrap();
            client
                .set_network(
                    ProviderNetwork::resolve(&ProxyConfig {
                        mode: ProxyMode::Direct,
                        url: None,
                    })
                    .unwrap(),
                )
                .unwrap();
            let (events, _receiver) = provider_event_channel();
            client.set_event_sender(events).await;
            let (setup_seen, wait_setup) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let mut socket = accept_async(listener.accept().await.unwrap().0)
                    .await
                    .unwrap();
                assert_eq!(wire(&mut socket).await["session"]["type"], "transcription");
                setup_seen.send(()).unwrap();
                let closed = tokio::time::timeout(Duration::from_secs(2), socket.next())
                    .await
                    .expect("cancelled probe retained its WebSocket");
                assert!(matches!(
                    closed,
                    None | Some(Err(_)) | Some(Ok(Message::Close(_)))
                ));
            });
            let mut pending = Box::pin(client.connect_for_probe("cancelled-probe"));
            tokio::select! {
                result = &mut pending => panic!("setup unexpectedly completed: {result:?}"),
                _ = wait_setup => {},
            }
            drop(pending);
            server.await.unwrap();
            assert!(!client.inner.ready.load(Ordering::SeqCst));
            assert!(client.inner.receive_task.lock().unwrap().is_none());
            assert!(client.inner.sink.lock().await.is_none());
            client.disconnect().await;
        }
    }

    async fn fixture() -> (CustomSpeechClient, ProviderEventReceiver, TcpListener) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = CustomSpeechClient::new(
            &format!("ws://{}/v1/realtime", listener.local_addr().unwrap()),
            MODEL,
            "synthetic-key",
            SourceLanguage::English,
        )
        .unwrap();
        client
            .set_network(
                ProviderNetwork::resolve(&ProxyConfig {
                    mode: ProxyMode::Direct,
                    url: None,
                })
                .unwrap(),
            )
            .unwrap();
        let (events, receiver) = provider_event_channel();
        client.set_event_sender(events).await;
        (client, receiver, listener)
    }

    async fn wire(socket: &mut ServerSocket) -> Value {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match socket.next().await.unwrap().unwrap() {
                    Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                    Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                    other => panic!("Unexpected synthetic wire message: {other:?}"),
                }
            }
        })
        .await
        .unwrap()
    }

    async fn send(socket: &mut ServerSocket, value: Value) {
        socket
            .send(Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }

    async fn setup(socket: &mut ServerSocket) {
        let mut update = wire(socket).await;
        assert_eq!(update["type"], "session.update");
        assert_eq!(update["session"]["type"], "transcription");
        assert_eq!(
            update["session"]["audio"]["input"]["format"],
            json!({"type":"audio/pcm","rate":24000})
        );
        assert_eq!(
            update["session"]["audio"]["input"]["transcription"]["model"],
            MODEL
        );
        assert_eq!(
            update["session"]["audio"]["input"]["transcription"]["language"],
            "en"
        );
        assert!(update["session"]["audio"]["input"]["turn_detection"].is_null());
        update["type"] = json!("session.updated");
        send(socket, update).await;
    }

    async fn committed(socket: &mut ServerSocket, item: &str) {
        assert_eq!(wire(socket).await["type"], "input_audio_buffer.append");
        loop {
            let event = wire(socket).await;
            if event["type"] == "input_audio_buffer.commit" {
                break;
            }
            assert_eq!(event["type"], "input_audio_buffer.append");
        }
        send(
            socket,
            json!({"type":"input_audio_buffer.committed","item_id":item}),
        )
        .await;
    }

    async fn completed(socket: &mut ServerSocket, item: &str, text: &str) {
        send(socket, json!({"type":"conversation.item.input_audio_transcription.completed","item_id":item,"content_index":0,"transcript":text})).await;
    }

    async fn next_final(receiver: &mut ProviderEventReceiver) -> (u64, String) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match receiver.recv().await.unwrap() {
                    LiveTranslateServerEvent::SourceUtteranceFinal {
                        utterance_id, text, ..
                    } => return (utterance_id, text),
                    LiveTranslateServerEvent::Error { code, .. } => {
                        panic!("Synthetic ASR failed: {code}")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap()
    }

    fn voice_then_silence() -> Vec<u8> {
        let mut pcm: Vec<u8> = 3000_i16
            .to_le_bytes()
            .into_iter()
            .cycle()
            .take(960)
            .collect();
        pcm.extend(vec![0; 30 * 960]);
        pcm
    }

    #[tokio::test]
    #[allow(clippy::result_large_err)] // Tungstenite fixes the handshake callback error type.
    async fn probe_waits_for_real_setup_ack_and_sends_no_pcm() {
        let (client, mut receiver, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_hdr_async(
                stream,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(
                        request.uri().path_and_query().unwrap().as_str(),
                        "/v1/realtime?intent=transcription"
                    );
                    assert_eq!(request.headers()["authorization"], "Bearer synthetic-key");
                    Ok(response)
                },
            )
            .await
            .unwrap();
            setup(&mut socket).await;
            assert!(matches!(
                socket.next().await,
                Some(Ok(Message::Close(_))) | None
            ));
        });
        client.connect_for_probe("synthetic-probe").await.unwrap();
        assert!(matches!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionCreated)
        ));
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn authentication_error_frames_preserve_fixed_diagnostic_classification() {
        let (client, mut receiver, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _ = wire(&mut socket).await;
            send(&mut socket, json!({"type":"error","error":{"code":"invalid_api_key","message":"Synthetic sensitive provider body"}})).await;
            let _ = socket.next().await;
        });
        assert_eq!(
            client.connect("synthetic-auth").await,
            Err(CustomSpeechClientError::AuthenticationFailed)
        );
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::Error {
                code: "custom_speech_authentication_failed".into(),
                message: "custom_speech_authentication_failed".into()
            })
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn disconnect_cancels_stalled_receive_timer_before_waiting_for_its_audio_lock() {
        let (client, _receiver, _listener) = fixture().await;
        let old_generation = client.inner.generation.load(Ordering::SeqCst);
        client.inner.ready.store(true, Ordering::SeqCst);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let inner = client.inner.clone();
        let task = tokio::spawn(async move {
            let _audio = inner.audio_operation.lock().await;
            entered_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        *client.inner.receive_task.lock().unwrap() = Some(task.abort_handle());
        entered_rx.await.unwrap();
        tokio::time::timeout(Duration::from_millis(100), client.disconnect())
            .await
            .unwrap();
        assert!(!client.is_current(old_generation));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
        assert!(client.inner.receive_task.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn socket_creation_alone_does_not_pass_setup_and_provider_body_is_not_retained() {
        let (client, _receiver, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _ = wire(&mut socket).await;
            send(&mut socket, json!({"type":"session.created"})).await;
            assert!(matches!(
                socket.next().await,
                Some(Ok(Message::Close(_))) | None
            ));
        });
        assert_eq!(
            client.connect_with_timeout(Duration::from_millis(40)).await,
            Err(CustomSpeechClientError::SetupTimeout)
        );
        server.await.unwrap();

        let (client, mut receiver, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let _ = wire(&mut socket).await;
            send(&mut socket, json!({"type":"error","error":{"code":"arbitrary-secret-code","message":"synthetic-secret-provider-body"}})).await;
            let _ = socket.next().await;
        });
        assert_eq!(
            client.connect("synthetic-rejected").await,
            Err(CustomSpeechClientError::SessionRejected)
        );
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::Error {
                code: "custom_speech_session_rejected".into(),
                message: "custom_speech_session_rejected".into()
            })
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn audio_turns_stream_bounded_pcm_and_out_of_order_finals_keep_capture_order() {
        let (client, mut receiver, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            setup(&mut socket).await;
            committed(&mut socket, "one").await;
            committed(&mut socket, "two").await;
            completed(&mut socket, "two", "Synthetic second final").await;
            completed(&mut socket, "one", "Synthetic first final").await;
            let _ = socket.next().await;
        });
        client.connect("synthetic-order").await.unwrap();
        // Idle system audio must not build an unlimited remote input buffer.
        for _ in 0..20 {
            client.send_audio(&vec![0; 9600]).await.unwrap();
        }
        client.send_audio(&voice_then_silence()).await.unwrap();
        client.send_audio(&voice_then_silence()).await.unwrap();
        assert_eq!(
            next_final(&mut receiver).await,
            (1, "Synthetic first final".into())
        );
        assert_eq!(
            next_final(&mut receiver).await,
            (2, "Synthetic second final".into())
        );
        assert!(client
            .inner
            .content
            .lock()
            .unwrap()
            .pending_started
            .is_empty());
        client.finish(Duration::from_secs(1)).await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn stop_flushes_short_voiced_tail_and_waits_for_final_before_finished() {
        use base64::Engine;
        let (client, mut receiver, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            setup(&mut socket).await;
            let append = wire(&mut socket).await;
            assert_eq!(append["type"], "input_audio_buffer.append");
            let pcm = base64::engine::general_purpose::STANDARD
                .decode(append["audio"].as_str().unwrap())
                .unwrap();
            assert_eq!(pcm.len(), 4800);
            assert_eq!(pcm[..2], 3000_i16.to_le_bytes());
            assert!(pcm[2..].iter().all(|byte| *byte == 0));
            assert_eq!(wire(&mut socket).await["type"], "input_audio_buffer.commit");
            send(
                &mut socket,
                json!({"type":"input_audio_buffer.committed","item_id":"tail"}),
            )
            .await;
            completed(&mut socket, "tail", "Synthetic stopped tail").await;
            assert!(matches!(
                socket.next().await,
                Some(Ok(Message::Close(_))) | None
            ));
        });
        client.connect("synthetic-stop").await.unwrap();
        client.send_audio(&3000_i16.to_le_bytes()).await.unwrap();
        client.finish(Duration::from_secs(1)).await;
        assert_eq!(
            next_final(&mut receiver).await,
            (1, "Synthetic stopped tail".into())
        );
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        );
        server.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn stop_cannot_overtake_a_retired_completion_before_publication() {
        for text in ["", " \t ", "Synthetic accepted final."] {
            let (client, mut receiver, listener) = fixture().await;
            let (complete_tx, complete_rx) = tokio::sync::oneshot::channel();
            let (publication_tx, publication_rx) = tokio::sync::oneshot::channel();
            let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
            *client.inner.before_final_publication.lock().unwrap() = Some(Box::new(move || {
                // Hold the exact interval after turn retirement and before
                // publication while another worker starts session finish.
                publication_tx.send(()).unwrap();
                resume_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            }));
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(stream).await.unwrap();
                setup(&mut socket).await;
                committed(&mut socket, "tail").await;
                complete_rx.await.unwrap();
                completed(&mut socket, "tail", text).await;
                assert!(matches!(
                    socket.next().await,
                    Some(Ok(Message::Close(_))) | None
                ));
            });
            client.connect("synthetic-publication-race").await.unwrap();
            assert_eq!(
                receiver.recv().await,
                Some(LiveTranslateServerEvent::SessionCreated)
            );
            client.send_audio(&voice_then_silence()).await.unwrap();
            complete_tx.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(2), publication_rx)
                .await
                .unwrap()
                .unwrap();
            let (finish_started_tx, finish_started_rx) = tokio::sync::oneshot::channel();
            let finishing = tokio::spawn(async move {
                finish_started_tx.send(()).unwrap();
                client.finish(Duration::from_secs(1)).await;
            });
            finish_started_rx.await.unwrap();
            let premature = tokio::time::timeout(Duration::from_millis(100), receiver.recv())
                .await
                .ok()
                .flatten();
            // Release before asserting so a failing regression cannot strand
            // the receiver or leave a loopback connection alive.
            resume_tx.send(()).unwrap();
            finishing.await.unwrap();
            server.await.unwrap();
            assert_eq!(premature, None, "finish overtook completion {text:?}");
            assert_eq!(
                receiver.recv().await,
                Some(LiveTranslateServerEvent::SourceUtteranceFinal {
                    utterance_id: 1,
                    text: text.into(),
                    language: Some("en".into()),
                })
            );
            assert_eq!(
                receiver.recv().await,
                Some(LiveTranslateServerEvent::SessionFinished)
            );
        }
    }

    #[tokio::test]
    async fn clear_waits_for_buffer_barrier_and_cannot_republish_old_callbacks() {
        let (client, mut receiver, listener) = fixture().await;
        let (old_committed_tx, old_committed_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            setup(&mut socket).await;
            committed(&mut socket, "old").await;
            old_committed_tx.send(()).unwrap();
            assert_eq!(wire(&mut socket).await["type"], "input_audio_buffer.clear");
            send(&mut socket, json!({"type":"conversation.item.input_audio_transcription.delta","item_id":"old","delta":"Synthetic stale preview"})).await;
            send(&mut socket, json!({"type":"input_audio_buffer.cleared"})).await;
            completed(&mut socket, "old", "Synthetic old final").await;
            committed(&mut socket, "new").await;
            completed(&mut socket, "new", "Synthetic new final").await;
            let _ = socket.next().await;
        });
        client.connect("synthetic-clear").await.unwrap();
        client.send_audio(&voice_then_silence()).await.unwrap();
        old_committed_rx.await.unwrap();
        assert_eq!(client.clear_content().await, 1);
        client.send_audio(&voice_then_silence()).await.unwrap();
        assert_eq!(
            next_final(&mut receiver).await,
            (2, "Synthetic new final".into())
        );
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn idle_capture_gap_commits_and_stop_deadline_cancels_missing_final() {
        let (client, mut receiver, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            setup(&mut socket).await;
            committed(&mut socket, "missing-final").await;
            assert!(matches!(
                socket.next().await,
                Some(Ok(Message::Close(_))) | None
            ));
        });
        client.connect("synthetic-idle").await.unwrap();
        client
            .send_audio(
                &3000_i16
                    .to_le_bytes()
                    .into_iter()
                    .cycle()
                    .take(960)
                    .collect::<Vec<_>>(),
            )
            .await
            .unwrap();
        {
            let mut content = client.inner.content.lock().unwrap();
            content.last_pcm_at = Some(tokio::time::Instant::now() - IDLE_COMMIT_DELAY);
        }
        client.tick().await.unwrap();
        let started = tokio::time::Instant::now();
        client.finish(Duration::from_millis(40)).await;
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(client.inner.content.lock().unwrap().turns.is_empty());
        while !matches!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        ) {}
        server.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_protocol_and_stale_generations_fail_without_content_or_body() {
        let (client, mut receiver, listener) = fixture().await;
        let (malformed_tx, malformed_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            setup(&mut socket).await;
            malformed_rx.await.unwrap();
            send(&mut socket, json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"unknown","transcript":"Synthetic uncommitted final"})).await;
            let _ = socket.next().await;
        });
        client.connect("synthetic-malformed").await.unwrap();
        let generation = client.inner.generation.load(Ordering::SeqCst);
        malformed_tx.send(()).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(LiveTranslateServerEvent::Error { code, message }) =
                    receiver.recv().await
                {
                    return (code, message);
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            error,
            (
                "custom_speech_protocol_invalid".into(),
                "custom_speech_protocol_invalid".into()
            )
        );
        client.disconnect().await;
        client.emit(
            LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 99,
                text: "Synthetic stale generation".into(),
                language: None,
            },
            None,
            generation,
        );
        client.fail(CustomSpeechClientError::SessionRejected, generation);
        assert!(
            tokio::time::timeout(Duration::from_millis(40), receiver.recv())
                .await
                .is_err()
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn every_pending_turn_has_its_own_bounded_completion_deadline() {
        let (client, _receiver, _listener) = fixture().await;
        let now = tokio::time::Instant::now();
        let mut content = client.inner.content.lock().unwrap();
        let first = content.turns.start(0).unwrap();
        content.turns.commit().unwrap();
        content.turns.acknowledge("first".into()).unwrap();
        let second = content.turns.start(0).unwrap();
        content.turns.commit().unwrap();
        content.turns.acknowledge("second".into()).unwrap();
        content
            .pending_started
            .push_back((first, now - MAX_FINAL_WAIT));
        content.pending_started.push_back((second, now));
        content
            .turns
            .complete("first".into(), "Synthetic first".into(), 0)
            .unwrap();
        trim_pending_started(&mut content);
        assert_eq!(content.pending_started.front(), Some(&(second, now)));
    }

    #[tokio::test]
    async fn empty_completion_retires_its_deadline_without_refunding_a_later_turn() {
        for final_text in ["", " \t "] {
            let (client, _receiver, _listener) = fixture().await;
            client.inner.ready.store(true, Ordering::SeqCst);
            let now = tokio::time::Instant::now();
            {
                let mut content = client.inner.content.lock().unwrap();
                let first = content.turns.start(0).unwrap();
                content.turns.commit().unwrap();
                content.turns.acknowledge("first".into()).unwrap();
                let second = content.turns.start(0).unwrap();
                content.turns.commit().unwrap();
                content.turns.acknowledge("second".into()).unwrap();
                content
                    .pending_started
                    .push_back((first, now - MAX_FINAL_WAIT));
                content.pending_started.push_back((second, now));
                let output = content
                    .turns
                    .complete("first".into(), final_text.into(), 0)
                    .unwrap();
                assert_eq!(output.len(), 1);
                assert_eq!(output[0].sequence, first);
                trim_pending_started(&mut content);
                assert_eq!(content.pending_started.front(), Some(&(second, now)));
            }
            assert!(client.tick().await.is_ok());
            client
                .inner
                .content
                .lock()
                .unwrap()
                .pending_started
                .front_mut()
                .unwrap()
                .1 = now - MAX_FINAL_WAIT;
            assert!(matches!(
                client.tick().await,
                Err(CustomSpeechClientError::InvalidProtocol)
            ));
        }
    }
}
