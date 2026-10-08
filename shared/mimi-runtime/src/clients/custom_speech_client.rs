//! Standalone OpenAI-compatible realtime transcription. Text translation is
//! supplied by the existing bounded pipeline, never requested on this socket.

use crate::clients::provider_events::ProviderEventSender;
use crate::clients::provider_network::{ProviderNetwork, ProviderNetworkError};
use crate::core::models::SourceLanguage;
use crate::core::pending_pcm::PendingPcmGate;
use crate::core::protocols::custom_speech::{
    self, AudioTurnAction, PcmTurnGate, ServerEvent, TranscriptOutput, TurnSequencer,
};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::provider::ProviderKind;
use futures_util::{SinkExt, StreamExt};
use std::collections::VecDeque;
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
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
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
}

#[derive(Clone)]
pub struct CustomSpeechClient {
    network: ProviderNetwork,
    endpoint: url::Url,
    model: String,
    api_key: String,
    source: SourceLanguage,
    inner: Arc<Inner>,
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
            inner: Arc::new(Inner {
                sink: Mutex::new(None),
                receive_task: Mutex::new(None),
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
            }),
        })
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
        self.disconnect().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);
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
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                .map_err(|_| CustomSpeechClientError::MissingAPIKey)?,
        );
        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            super::provider_network::websocket_with_message_limit(
                request,
                &self.network,
                custom_speech::MAX_CUSTOM_SPEECH_MESSAGE_BYTES,
            ),
        )
        .await
        .map_err(|_| CustomSpeechClientError::Unreachable)?
        .map_err(|error| {
            if super::connection_diagnostics::authentication_rejected(&error) {
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
        *self.inner.receive_task.lock().await = Some(task);
        let update = custom_speech::session_update(
            &self.model,
            self.source,
            &format!("mimi-asr-setup-{generation}"),
        );
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
            self.disconnect().await;
        }
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
        if !self.inner.ready.load(Ordering::SeqCst) {
            self.disconnect().await;
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
        self.disconnect().await;
    }

    pub async fn disconnect(&self) {
        // Invalidate and cancel the receiver before waiting for audio work:
        // its idle timer may itself be holding audio_operation during a send.
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.closing.store(true, Ordering::SeqCst);
        self.inner.clearing.store(false, Ordering::SeqCst);
        if let Some(task) = self.inner.receive_task.lock().await.take() {
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
                    let output = {
                        let mut content = self.inner.content.lock().unwrap();
                        let revision = content.revision;
                        match event {
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
                        })
                    };
                    let Ok(output) = output else {
                        self.fail(CustomSpeechClientError::InvalidProtocol, generation);
                        return;
                    };
                    for output in output {
                        if self.emit_transcript(output, &events, generation).is_err() {
                            return;
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
    ) -> Result<(), super::provider_events::ProviderEventSendError> {
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
        if !self.is_current(generation) || self.inner.failed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner.ready.store(false, Ordering::SeqCst);
        *self.inner.failure_reason.lock().unwrap() = Some(error.clone());
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
    use crate::clients::provider_events::{provider_event_channel, ProviderEventReceiver};
    use crate::core::network_proxy::{ProxyConfig, ProxyMode};
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, accept_hdr_async};

    type ServerSocket = WebSocketStream<TcpStream>;
    const MODEL: &str = "gpt-4o-mini-transcribe";

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
        *client.inner.receive_task.lock().await = Some(task);
        entered_rx.await.unwrap();
        tokio::time::timeout(Duration::from_millis(100), client.disconnect())
            .await
            .unwrap();
        assert!(!client.is_current(old_generation));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
        assert!(client.inner.receive_task.lock().await.is_none());
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
}
