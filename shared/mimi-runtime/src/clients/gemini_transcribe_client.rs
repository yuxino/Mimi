//! Dedicated Gemini Live transcription with bounded, independent MT events.

use crate::clients::provider_events::ProviderEventSender;
use crate::clients::provider_network::{ProviderNetwork, ProviderNetworkError};
use crate::core::models::SourceLanguage;
use crate::core::pending_pcm::PendingPcmGate;
use crate::core::protocols::gemini_transcribe::{self, ServerEvent, TranscriptAssembler};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use futures_util::{SinkExt, StreamExt};
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
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(250);
// The Live API can deliver authoritative transcription after turnComplete.
const TAIL_QUIET_PERIOD: Duration =
    Duration::from_millis(mimi_core::openai_transcript_committer::GEMINI_TURN_TAIL_QUIET_MS);
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Sink = futures_util::stream::SplitSink<Socket, Message>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GeminiTranscribeClientError {
    #[error("gemini_transcribe_credentials_missing")]
    MissingAPIKey,
    #[error("gemini_transcribe_language_unsupported")]
    UnsupportedLanguage,
    #[error("gemini_transcribe_authentication_failed")]
    AuthenticationFailed,
    #[error("gemini_transcribe_unreachable")]
    Unreachable,
    #[error("gemini_transcribe_session_rejected")]
    SessionRejected,
    #[error("gemini_transcribe_setup_timeout")]
    SetupTimeout,
    #[error("gemini_transcribe_protocol_invalid")]
    InvalidProtocol,
    #[error("gemini_transcribe_audio_invalid")]
    InvalidAudio,
    #[error("gemini_transcribe_not_connected")]
    NotConnected,
    #[error("gemini_transcribe_health_timeout")]
    HealthTimeout,
}

#[derive(Clone, Copy)]
enum Setup {
    Awaiting,
    Ready,
    Rejected,
}

#[derive(Default)]
struct Content {
    revision: u64,
    transcript: TranscriptAssembler,
    last_received: Option<tokio::time::Instant>,
}

struct Inner {
    lifecycle: Mutex<()>,
    audio_operation: Mutex<()>,
    sink: Mutex<Option<Sink>>,
    receiver: std::sync::Mutex<Option<AbortHandle>>,
    content: std::sync::Mutex<Content>,
    events: std::sync::Mutex<Option<ProviderEventSender>>,
    generation: AtomicU64,
    ready: AtomicBool,
    closing: AtomicBool,
    done: AtomicBool,
    end_sent: AtomicBool,
    failed: AtomicBool,
    failure: std::sync::Mutex<Option<GeminiTranscribeClientError>>,
    pong: Notify,
    progress: Notify,
}

#[derive(Clone)]
pub struct GeminiTranscribeClient {
    endpoint: url::Url,
    api_key: String,
    source: SourceLanguage,
    network: ProviderNetwork,
    inner: Arc<Inner>,
}

/// Cancellation invalidates the socket synchronously; the next operation can
/// reconnect and dispose of its old sink without reviving its receiver.
struct ConnectAttempt {
    inner: Arc<Inner>,
    generation: u64,
    complete: bool,
}
impl Drop for ConnectAttempt {
    fn drop(&mut self) {
        if !self.complete
            && self
                .inner
                .generation
                .compare_exchange(
                    self.generation,
                    self.generation.wrapping_add(1),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
        {
            self.inner.ready.store(false, Ordering::SeqCst);
            if let Some(receiver) = self.inner.receiver.lock().unwrap().take() {
                receiver.abort();
            }
            if let Ok(mut sink) = self.inner.sink.try_lock() {
                sink.take();
            } else if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let inner = self.inner.clone();
                let retired = self.generation.wrapping_add(1);
                runtime.spawn(async move {
                    let mut sink = inner.sink.lock().await;
                    if inner.generation.load(Ordering::SeqCst) == retired {
                        sink.take();
                    }
                });
            }
        }
    }
}

impl GeminiTranscribeClient {
    pub fn new(api_key: &str, source: SourceLanguage) -> Result<Self, GeminiTranscribeClientError> {
        let endpoint = gemini_transcribe::endpoint(source)
            .map_err(|_| GeminiTranscribeClientError::UnsupportedLanguage)?;
        let api_key = api_key.trim();
        if api_key.is_empty() || HeaderValue::from_str(&format!("Bearer {api_key}")).is_err() {
            return Err(GeminiTranscribeClientError::MissingAPIKey);
        }
        Ok(Self {
            endpoint,
            api_key: api_key.to_owned(),
            source,
            network: ProviderNetwork::default(),
            inner: Arc::new(Inner {
                lifecycle: Mutex::new(()),
                audio_operation: Mutex::new(()),
                sink: Mutex::new(None),
                receiver: Default::default(),
                content: Default::default(),
                events: Default::default(),
                generation: AtomicU64::new(0),
                ready: AtomicBool::new(false),
                closing: AtomicBool::new(false),
                done: AtomicBool::new(false),
                end_sent: AtomicBool::new(false),
                failed: AtomicBool::new(false),
                failure: Default::default(),
                pong: Notify::new(),
                progress: Notify::new(),
            }),
        })
    }

    /// Production endpoints stay fixed. Behavioral tests may replace only
    /// the transport host with an explicitly local fixture address.
    #[cfg(test)]
    pub(crate) fn set_loopback_for_test(&mut self, address: std::net::SocketAddr) {
        assert!(address.ip().is_loopback());
        self.endpoint.set_scheme("ws").unwrap();
        self.endpoint
            .set_host(Some(&address.ip().to_string()))
            .unwrap();
        self.endpoint.set_port(Some(address.port())).unwrap();
    }

    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.clone()
    }
    pub fn set_network(&mut self, network: ProviderNetwork) -> Result<(), ProviderNetworkError> {
        self.network = network;
        Ok(())
    }
    pub fn set_audio_pending_gate(&self, _gate: PendingPcmGate) {}
    pub async fn set_event_sender(&self, events: ProviderEventSender) {
        self.inner.content.lock().unwrap().revision = events.content_revision();
        *self.inner.events.lock().unwrap() = Some(events);
    }
    pub fn content_revision(&self) -> u64 {
        self.inner.content.lock().unwrap().revision
    }
    pub async fn connect(&self, _task_id: &str) -> Result<(), GeminiTranscribeClientError> {
        self.connect_with_timeout(SETUP_TIMEOUT).await
    }
    pub async fn connect_for_probe(
        &self,
        task_id: &str,
    ) -> Result<(), GeminiTranscribeClientError> {
        self.connect(task_id).await
    }

    async fn connect_with_timeout(
        &self,
        timeout: Duration,
    ) -> Result<(), GeminiTranscribeClientError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        self.disconnect_locked().await;
        self.connect_locked(timeout).await
    }

    async fn connect_locked(
        &self,
        setup_timeout: Duration,
    ) -> Result<(), GeminiTranscribeClientError> {
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
            .ok_or(GeminiTranscribeClientError::NotConnected)?;
        self.inner.content.lock().unwrap().revision = events.content_revision();
        let mut request = self
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|_| GeminiTranscribeClientError::InvalidProtocol)?;
        request.headers_mut().insert(
            "x-goog-api-key",
            HeaderValue::from_str(&self.api_key)
                .map_err(|_| GeminiTranscribeClientError::MissingAPIKey)?,
        );
        let socket = tokio::time::timeout(
            Duration::from_secs(15),
            super::provider_network::websocket_with_message_limit(
                request,
                &self.network,
                gemini_transcribe::MAX_MESSAGE_BYTES,
            ),
        )
        .await
        .map_err(|_| GeminiTranscribeClientError::Unreachable)?
        .map_err(|error| {
            if super::connection_diagnostics::authentication_rejected(&error) {
                GeminiTranscribeClientError::AuthenticationFailed
            } else {
                GeminiTranscribeClientError::Unreachable
            }
        })?
        .0;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.closing.store(false, Ordering::SeqCst);
        self.inner.done.store(false, Ordering::SeqCst);
        self.inner.end_sent.store(false, Ordering::SeqCst);
        self.inner.failed.store(false, Ordering::SeqCst);
        *self.inner.failure.lock().unwrap() = None;
        let (setup, mut acknowledgement) = watch::channel(Setup::Awaiting);
        let client = self.clone();
        let task = tokio::spawn(crate::development_audio::scope_context(
            crate::development_audio::context(),
            async move {
                client.receive(stream, events, setup, generation).await;
            },
        ));
        *self.inner.receiver.lock().unwrap() = Some(task.abort_handle());
        let result = tokio::time::timeout(setup_timeout, async {
            self.send_message(Message::Text(
                gemini_transcribe::setup(self.source)
                    .map_err(|_| GeminiTranscribeClientError::UnsupportedLanguage)?
                    .to_string()
                    .into(),
            ))
            .await?;
            loop {
                let state = *acknowledgement.borrow();
                match state {
                    Setup::Ready
                        if self.current(generation)
                            && !self.inner.failed.load(Ordering::SeqCst)
                            && !self.inner.done.load(Ordering::SeqCst) =>
                    {
                        self.inner.ready.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                    Setup::Ready | Setup::Rejected => return Err(self.failure()),
                    Setup::Awaiting => {}
                }
                acknowledgement
                    .changed()
                    .await
                    .map_err(|_| self.failure())?;
            }
        })
        .await
        .unwrap_or(Err(GeminiTranscribeClientError::SetupTimeout));
        if result.is_err() {
            self.disconnect_locked().await;
        }
        attempt.complete = true;
        result
    }

    pub async fn send_audio(&self, pcm: &[u8]) -> Result<(), GeminiTranscribeClientError> {
        if pcm.is_empty() {
            return Ok(());
        }
        if !pcm.len().is_multiple_of(2) || pcm.len() > gemini_transcribe::MAX_PCM_CHUNK_BYTES {
            return Err(GeminiTranscribeClientError::InvalidAudio);
        }
        let operation = async {
            let _audio = self.inner.audio_operation.lock().await;
            if !self.inner.ready.load(Ordering::SeqCst) || self.inner.closing.load(Ordering::SeqCst)
            {
                return Err(GeminiTranscribeClientError::NotConnected);
            }
            self.send_message(Message::Text(
                gemini_transcribe::audio(pcm).to_string().into(),
            ))
            .await
        };
        tokio::time::timeout(SEND_TIMEOUT, operation)
            .await
            .unwrap_or(Err(GeminiTranscribeClientError::Unreachable))
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), GeminiTranscribeClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) {
            return Err(GeminiTranscribeClientError::NotConnected);
        }
        tokio::time::timeout(timeout, async {
            let pong = self.inner.pong.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            self.send_message(Message::Ping(Default::default())).await?;
            pong.await;
            Ok(())
        })
        .await
        .unwrap_or(Err(GeminiTranscribeClientError::HealthTimeout))
    }

    pub async fn clear_content(&self) -> u64 {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let reconnect =
            self.inner.ready.load(Ordering::SeqCst) && !self.inner.closing.load(Ordering::SeqCst);
        // Gemini has no audio-clear acknowledgement. A fresh socket is the only
        // boundary that proves old, untranscribed audio cannot leak back.
        self.disconnect_locked().await;
        let revision = self.inner.events.lock().unwrap().as_ref().map_or_else(
            || self.content_revision().wrapping_add(1),
            ProviderEventSender::advance_content_revision,
        );
        self.inner.content.lock().unwrap().revision = revision;
        if reconnect {
            if let Err(error) = self.connect_locked(SETUP_TIMEOUT).await {
                self.fail(error, self.inner.generation.load(Ordering::SeqCst));
            }
        }
        revision
    }

    pub async fn finish(&self, timeout: Duration) {
        let _lifecycle = self.inner.lifecycle.lock().await;
        if self.inner.ready.load(Ordering::SeqCst) {
            self.inner.closing.store(true, Ordering::SeqCst);
            let operation = async {
                {
                    let _audio = self.inner.audio_operation.lock().await;
                    self.send_message(Message::Text(
                        gemini_transcribe::finish().to_string().into(),
                    ))
                    .await?;
                    // A turn marker observed while the sink/write was blocked
                    // belongs to earlier audio. Start the drain only after the
                    // end signal has actually been written. An unusually fast
                    // response before this point is still published; without a
                    // later marker finish waits its finite outer deadline.
                    let mut content = self.inner.content.lock().unwrap();
                    content.last_received = Some(tokio::time::Instant::now());
                    self.inner.end_sent.store(true, Ordering::SeqCst);
                }
                loop {
                    let progress = self.inner.progress.notified();
                    tokio::pin!(progress);
                    progress.as_mut().enable();
                    if self.inner.failed.load(Ordering::SeqCst) {
                        break;
                    }
                    let quiet_since = self.inner.content.lock().unwrap().last_received;
                    if self.inner.done.load(Ordering::SeqCst) {
                        if let Some(quiet_since) = quiet_since {
                            if quiet_since.elapsed() >= TAIL_QUIET_PERIOD {
                                break;
                            }
                            tokio::select! {
                                _ = progress => {},
                                _ = tokio::time::sleep_until(quiet_since + TAIL_QUIET_PERIOD) => {},
                            }
                            continue;
                        }
                    }
                    progress.await;
                }
                Ok::<(), GeminiTranscribeClientError>(())
            };
            let _ = tokio::time::timeout(timeout, operation).await;
            let generation = self.inner.generation.load(Ordering::SeqCst);
            self.emit(LiveTranslateServerEvent::SessionFinished, generation);
        }
        self.disconnect_locked().await;
    }

    pub async fn disconnect(&self) {
        let _lifecycle = self.inner.lifecycle.lock().await;
        self.disconnect_locked().await;
    }
    async fn disconnect_locked(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.closing.store(true, Ordering::SeqCst);
        if let Some(receiver) = self.inner.receiver.lock().unwrap().take() {
            receiver.abort();
        }
        let _audio = self.inner.audio_operation.lock().await;
        if let Some(mut sink) = self.inner.sink.lock().await.take() {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.close()).await;
        }
        {
            let mut content = self.inner.content.lock().unwrap();
            content.transcript.reset();
            content.last_received = None;
        }
        self.inner.end_sent.store(false, Ordering::SeqCst);
        self.inner.failed.store(false, Ordering::SeqCst);
        *self.inner.failure.lock().unwrap() = None;
    }

    async fn send_message(&self, message: Message) -> Result<(), GeminiTranscribeClientError> {
        tokio::time::timeout(SEND_TIMEOUT, async {
            let mut sink = self.inner.sink.lock().await;
            let sink = sink
                .as_mut()
                .ok_or(GeminiTranscribeClientError::NotConnected)?;
            let evidence = match &message {
                Message::Text(text) => {
                    crate::development_audio::begin_json(text, gemini_transcribe::PCM_SAMPLE_RATE)
                }
                _ => crate::development_audio::begin_pcm(&[], gemini_transcribe::PCM_SAMPLE_RATE),
            };
            evidence
                .observe(sink.send(message))
                .await
                .map_err(|_| GeminiTranscribeClientError::Unreachable)
        })
        .await
        .unwrap_or(Err(GeminiTranscribeClientError::Unreachable))
    }

    async fn receive(
        &self,
        mut stream: futures_util::stream::SplitStream<Socket>,
        events: ProviderEventSender,
        setup: watch::Sender<Setup>,
        generation: u64,
    ) {
        let mut created = false;
        while let Some(message) = stream.next().await {
            if !self.current(generation) {
                return;
            }
            let event = match message {
                Ok(Message::Text(text)) => match gemini_transcribe::decode(&text) {
                    Ok(event) => event,
                    Err(_) => {
                        self.fail(GeminiTranscribeClientError::InvalidProtocol, generation);
                        let _ = setup.send(Setup::Rejected);
                        return;
                    }
                },
                Ok(Message::Pong(_)) => {
                    self.inner.pong.notify_waiters();
                    continue;
                }
                Ok(Message::Ping(bytes)) => {
                    let _ = self.send_message(Message::Pong(bytes)).await;
                    continue;
                }
                Ok(Message::Frame(_)) => continue,
                Ok(Message::Binary(data)) => match std::str::from_utf8(&data)
                    .map_err(|_| gemini_transcribe::ProtocolError)
                    .and_then(gemini_transcribe::decode)
                {
                    Ok(event) => event,
                    Err(_) => {
                        self.fail(GeminiTranscribeClientError::InvalidProtocol, generation);
                        let _ = setup.send(Setup::Rejected);
                        return;
                    }
                },
                _ => {
                    if !self.inner.done.load(Ordering::SeqCst) {
                        self.fail(GeminiTranscribeClientError::Unreachable, generation);
                    }
                    let _ = setup.send(Setup::Rejected);
                    return;
                }
            };
            match event {
                ServerEvent::Created if !created => {
                    created = true;
                    self.emit(LiveTranslateServerEvent::SessionCreated, generation);
                    let _ = setup.send(Setup::Ready);
                }
                ServerEvent::Created => {
                    self.fail(GeminiTranscribeClientError::InvalidProtocol, generation);
                    let _ = setup.send(Setup::Rejected);
                    return;
                }
                ServerEvent::AuthenticationRejected | ServerEvent::Rejected => {
                    self.fail(
                        if event == ServerEvent::AuthenticationRejected {
                            GeminiTranscribeClientError::AuthenticationFailed
                        } else {
                            GeminiTranscribeClientError::SessionRejected
                        },
                        generation,
                    );
                    let _ = setup.send(Setup::Rejected);
                    return;
                }
                ServerEvent::GoAway => {
                    self.fail(GeminiTranscribeClientError::Unreachable, generation);
                    let _ = setup.send(Setup::Rejected);
                    return;
                }
                ServerEvent::Ignored => {}
                event if created => {
                    let done = self.inner.end_sent.load(Ordering::SeqCst)
                        && matches!(
                            event,
                            ServerEvent::Transcript {
                                turn_complete: true,
                                ..
                            }
                        );
                    let mut content = self.inner.content.lock().unwrap();
                    if !self.current(generation) {
                        return;
                    }
                    content.last_received = Some(tokio::time::Instant::now());
                    let output = content.transcript.apply(event);
                    match output {
                        Ok(outputs) => {
                            for output in outputs {
                                let language = (self.source != SourceLanguage::Automatic)
                                    .then(|| self.source.raw_value().to_owned());
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
                                if events
                                    .send_if(event, || {
                                        self.current(generation)
                                            && events.content_revision() == content.revision
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                        Err(_) => {
                            drop(content);
                            self.fail(GeminiTranscribeClientError::InvalidProtocol, generation);
                            return;
                        }
                    }
                    if done {
                        self.inner.done.store(true, Ordering::SeqCst);
                    }
                    drop(content);
                    self.inner.progress.notify_waiters();
                }
                _ => {
                    self.fail(GeminiTranscribeClientError::InvalidProtocol, generation);
                    let _ = setup.send(Setup::Rejected);
                    return;
                }
            }
        }
        if self.current(generation) && !self.inner.done.load(Ordering::SeqCst) {
            self.fail(GeminiTranscribeClientError::Unreachable, generation);
            let _ = setup.send(Setup::Rejected);
        }
    }
    fn current(&self, generation: u64) -> bool {
        self.inner.generation.load(Ordering::SeqCst) == generation
    }
    fn emit(&self, event: LiveTranslateServerEvent, generation: u64) {
        if let Some(events) = self.inner.events.lock().unwrap().as_ref() {
            let _ = events.send_if(event, || self.current(generation));
        }
    }
    fn fail(&self, error: GeminiTranscribeClientError, generation: u64) {
        let content = self.inner.content.lock().unwrap();
        if !self.current(generation) || self.inner.failed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner.ready.store(false, Ordering::SeqCst);
        *self.inner.failure.lock().unwrap() = Some(error.clone());
        drop(content);
        self.emit(
            LiveTranslateServerEvent::Error {
                code: error.to_string(),
                message: error.to_string(),
            },
            generation,
        );
        self.inner.progress.notify_waiters();
    }
    fn failure(&self) -> GeminiTranscribeClientError {
        self.inner
            .failure
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(GeminiTranscribeClientError::SessionRejected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::provider_events::{provider_event_channel, ProviderEventReceiver};
    use crate::core::network_proxy::{ProxyConfig, ProxyMode};
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_tungstenite::{accept_async, accept_hdr_async};
    type Server = WebSocketStream<TcpStream>;

    async fn fixture() -> (GeminiTranscribeClient, ProviderEventReceiver, TcpListener) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client =
            GeminiTranscribeClient::new("synthetic-speech-key", SourceLanguage::English).unwrap();
        client.set_loopback_for_test(listener.local_addr().unwrap());
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
    async fn send(socket: &mut Server, value: Value) {
        socket
            .send(Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }
    async fn message(socket: &mut Server) -> Value {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let Message::Text(text) = message {
                return serde_json::from_str(&text).unwrap();
            }
        }
    }
    async fn setup(socket: &mut Server) {
        assert_eq!(
            message(socket).await["setup"]["model"],
            "models/gemini-3.5-transcribe-live"
        );
        send(socket, json!({"setupComplete":{}})).await;
    }
    async fn next_content(
        receiver: &mut ProviderEventReceiver,
    ) -> crate::clients::provider_events::ProviderEvent {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = receiver.recv_with_revision().await.unwrap();
                if event.is_content() {
                    return event;
                }
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn binary_json_setup_and_transcripts_work_with_text_or_binary_frames() {
        let contract: Value =
            serde_json::from_str(include_str!("../../../translation-contracts.json")).unwrap();
        let contract = &contract["geminiTranscribeTransport"];
        for (binary_setup, binary_content) in [(true, true), (true, false), (false, true)] {
            let (client, mut receiver, listener) = fixture().await;
            let wire = contract.clone();
            let server = tokio::spawn(async move {
                let mut socket = accept_async(listener.accept().await.unwrap().0)
                    .await
                    .unwrap();
                let frame = |value: &Value, binary| {
                    let text = value.to_string();
                    if binary {
                        Message::Binary(text.into_bytes().into())
                    } else {
                        Message::Text(text.into())
                    }
                };
                let _ = message(&mut socket).await;
                socket
                    .send(frame(&wire["setup"], binary_setup))
                    .await
                    .unwrap();
                let audio = message(&mut socket).await;
                assert_eq!(
                    audio["realtimeInput"]["audio"]["mimeType"],
                    "audio/pcm;rate=16000"
                );
                socket
                    .send(frame(&wire["draft"], binary_content))
                    .await
                    .unwrap();
                assert_eq!(message(&mut socket).await, gemini_transcribe::finish());
                socket
                    .send(frame(&wire["final"], binary_content))
                    .await
                    .unwrap();
                while socket.next().await.is_some() {}
            });
            client.connect("binary-json").await.unwrap();
            client.send_audio(&[0; 3200]).await.unwrap();
            let draft = next_content(&mut receiver).await;
            assert!(
                matches!(draft.event, LiveTranslateServerEvent::SourceUtteranceDraft { ref text, .. }
                if text == contract["expectedDraft"].as_str().unwrap())
            );
            client.finish(Duration::from_secs(2)).await;
            let tail = next_content(&mut receiver).await;
            assert!(
                matches!(tail.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. }
                if text == contract["expectedFinal"].as_str().unwrap())
            );
            assert!(matches!(
                receiver.recv().await,
                Some(LiveTranslateServerEvent::SessionFinished)
            ));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn invalid_binary_payloads_fail_before_and_after_setup() {
        let payloads = [
            vec![0xff, 0xfe],
            b"{invalid json".to_vec(),
            b"{}".to_vec(),
            br#"{"serverContent":{"inputTranscription":{}}}"#.to_vec(),
        ];
        for ready in [false, true] {
            for payload in &payloads {
                let (client, mut receiver, listener) = fixture().await;
                let payload = payload.clone();
                let server = tokio::spawn(async move {
                    let mut socket = accept_async(listener.accept().await.unwrap().0)
                        .await
                        .unwrap();
                    if ready {
                        setup(&mut socket).await;
                        let _ = message(&mut socket).await;
                    } else {
                        let _ = message(&mut socket).await;
                    }
                    socket.send(Message::Binary(payload.into())).await.unwrap();
                    while socket.next().await.is_some() {}
                });
                if ready {
                    client.connect("invalid-binary").await.unwrap();
                    client.send_audio(&[0; 3200]).await.unwrap();
                    tokio::time::timeout(Duration::from_secs(2), async {
                        loop {
                            match receiver.recv().await.unwrap() {
                                LiveTranslateServerEvent::SessionCreated => {}
                                LiveTranslateServerEvent::Error { code, .. } => {
                                    assert_eq!(
                                        code,
                                        GeminiTranscribeClientError::InvalidProtocol.to_string()
                                    );
                                    break;
                                }
                                event => panic!(
                                    "Unexpected event after invalid binary payload: {event:?}"
                                ),
                            }
                        }
                    })
                    .await
                    .unwrap();
                } else {
                    assert!(matches!(
                        client.connect("invalid-binary").await,
                        Err(GeminiTranscribeClientError::InvalidProtocol)
                    ));
                }
                client.disconnect().await;
                server.await.unwrap();
            }
        }
    }

    #[tokio::test]
    async fn binary_server_errors_preserve_authentication_and_rejection_classification() {
        for (code, expected) in [
            (401, GeminiTranscribeClientError::AuthenticationFailed),
            (403, GeminiTranscribeClientError::AuthenticationFailed),
            (429, GeminiTranscribeClientError::SessionRejected),
        ] {
            let (client, _receiver, listener) = fixture().await;
            let server = tokio::spawn(async move {
                let mut socket = accept_async(listener.accept().await.unwrap().0)
                    .await
                    .unwrap();
                let _ = message(&mut socket).await;
                socket
                    .send(Message::Binary(
                        json!({"error":{"code":code}})
                            .to_string()
                            .into_bytes()
                            .into(),
                    ))
                    .await
                    .unwrap();
                while socket.next().await.is_some() {}
            });
            assert_eq!(
                client
                    .connect("binary-error")
                    .await
                    .unwrap_err()
                    .to_string(),
                expected.to_string()
            );
            client.disconnect().await;
            server.await.unwrap();
        }
    }

    #[tokio::test]
    #[allow(clippy::result_large_err)]
    async fn speech_only_handshake_streams_and_stop_preserves_authoritative_tail() {
        let (client, mut receiver, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let mut socket = accept_hdr_async(
                listener.accept().await.unwrap().0,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(
                        request.headers().get("x-goog-api-key").unwrap(),
                        "synthetic-speech-key"
                    );
                    assert!(request.headers().get("Authorization").is_none());
                    assert!(request.uri().query().is_none());
                    Ok(response)
                },
            )
            .await
            .unwrap();
            setup(&mut socket).await;
            let audio = message(&mut socket).await;
            assert_eq!(
                audio["realtimeInput"]["audio"]["mimeType"],
                "audio/pcm;rate=16000"
            );
            send(
                &mut socket,
                json!({"serverContent":{"inputTranscription":{"text":"First segment."}}}),
            )
            .await;
            assert_eq!(message(&mut socket).await, gemini_transcribe::finish());
            send(&mut socket, json!({"serverContent":{"inputTranscription":{"text":"Final tail."},"turnComplete":true}})).await;
            while socket.next().await.is_some() {}
        });
        client.connect("test").await.unwrap();
        client.send_audio(&[0; 3200]).await.unwrap();
        let first = next_content(&mut receiver).await;
        assert!(
            matches!(first.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "First segment.")
        );
        client.finish(Duration::from_secs(2)).await;
        let tail = next_content(&mut receiver).await;
        assert!(
            matches!(tail.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "Final tail.")
        );
        assert!(matches!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn stop_drains_authoritative_transcription_after_turn_complete() {
        let (client, mut receiver, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let mut socket = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            setup(&mut socket).await;
            assert_eq!(message(&mut socket).await, gemini_transcribe::finish());
            send(&mut socket, json!({"serverContent":{"turnComplete":true}})).await;
            tokio::time::sleep(TAIL_QUIET_PERIOD / 2).await;
            send(
                &mut socket,
                json!({"serverContent":{"inputTranscription":{"text":"Late authoritative tail."}}}),
            )
            .await;
            while socket.next().await.is_some() {}
        });
        client.connect("test").await.unwrap();
        client.finish(Duration::from_secs(2)).await;
        let tail = next_content(&mut receiver).await;
        assert!(
            matches!(tail.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "Late authoritative tail.")
        );
        assert!(matches!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn prior_turn_complete_cannot_finish_before_the_audio_end_signal() {
        let (client, mut receiver, listener) = fixture().await;
        let (send_prior, wait_prior) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut socket = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            setup(&mut socket).await;
            wait_prior.await.unwrap();
            send(&mut socket, json!({"serverContent":{"interimInputTranscription":{"text":"Prior preview"},"turnComplete":true}})).await;
            assert_eq!(message(&mut socket).await, gemini_transcribe::finish());
            send(&mut socket, json!({"serverContent":{"inputTranscription":{"text":"Flushed final."},"turnComplete":true}})).await;
            while socket.next().await.is_some() {}
        });
        client.connect("test").await.unwrap();
        let audio_guard = client.inner.audio_operation.lock().await;
        let closing_client = client.clone();
        let closing = tokio::spawn(async move {
            closing_client.finish(Duration::from_secs(2)).await;
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !client.inner.closing.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        send_prior.send(()).unwrap();
        let _ = next_content(&mut receiver).await;
        assert!(!client.inner.done.load(Ordering::SeqCst));
        drop(audio_guard);
        closing.await.unwrap();
        let tail = next_content(&mut receiver).await;
        assert!(
            matches!(tail.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "Flushed final.")
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn old_marker_while_end_write_is_blocked_cannot_consume_the_drain_window() {
        let (client, mut receiver, listener) = fixture().await;
        let (send_prior, wait_prior) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut socket = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            setup(&mut socket).await;
            wait_prior.await.unwrap();
            send(&mut socket, json!({"serverContent":{"interimInputTranscription":{"text":"Earlier preview"},"turnComplete":true}})).await;
            assert_eq!(message(&mut socket).await, gemini_transcribe::finish());
            tokio::time::sleep(TAIL_QUIET_PERIOD / 2).await;
            send(
                &mut socket,
                json!({"serverContent":{"inputTranscription":{"text":"Post-write final."}}}),
            )
            .await;
            while socket.next().await.is_some() {}
        });
        client.connect("test").await.unwrap();
        let sink_guard = client.inner.sink.lock().await;
        let closing_client = client.clone();
        let closing = tokio::spawn(async move {
            closing_client.finish(Duration::from_secs(2)).await;
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !client.inner.closing.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        send_prior.send(()).unwrap();
        let _ = next_content(&mut receiver).await;
        tokio::time::sleep(TAIL_QUIET_PERIOD + Duration::from_millis(20)).await;
        assert!(!client.inner.end_sent.load(Ordering::SeqCst));
        assert!(!client.inner.done.load(Ordering::SeqCst));
        drop(sink_guard);
        closing.await.unwrap();
        let tail = next_content(&mut receiver).await;
        assert!(
            matches!(tail.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "Post-write final.")
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn clear_opens_a_new_socket_and_cannot_commit_old_audio() {
        let (client, mut receiver, listener) = fixture().await;
        let (first_ready, wait_first) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut first = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            setup(&mut first).await;
            send(
                &mut first,
                json!({"serverContent":{"interimInputTranscription":{"text":"Old draft"}}}),
            )
            .await;
            first_ready.send(()).unwrap();
            while let Some(Ok(message)) = first.next().await {
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
            let _ = first
                .send(Message::Text(
                    json!({"serverContent":{"inputTranscription":{"text":"Stale final"}}})
                        .to_string()
                        .into(),
                ))
                .await;
            let mut second = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            setup(&mut second).await;
            send(
                &mut second,
                json!({"serverContent":{"inputTranscription":{"text":"New final"}}}),
            )
            .await;
            while second.next().await.is_some() {}
        });
        client.connect("test").await.unwrap();
        wait_first.await.unwrap();
        let _ = next_content(&mut receiver).await;
        assert_eq!(client.clear_content().await, 1);
        let next = next_content(&mut receiver).await;
        assert_eq!(next.content_revision, 1);
        assert!(
            matches!(next.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "New final")
        );
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_probe_closes_its_socket_while_client_is_retained() {
        let (client, _receiver, listener) = fixture().await;
        let (setup_seen, wait_setup) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut socket = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            let _ = message(&mut socket).await;
            setup_seen.send(()).unwrap();
            let closed = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .expect("cancelled Gemini probe retained its WebSocket");
            assert!(matches!(
                closed,
                None | Some(Err(_)) | Some(Ok(Message::Close(_)))
            ));
        });
        let mut pending = Box::pin(client.connect_for_probe("cancelled"));
        tokio::select! { result = &mut pending => panic!("unexpected setup: {result:?}"), _ = wait_setup => {} }
        drop(pending);
        server.await.unwrap();
        assert!(client.inner.sink.lock().await.is_none());
        client.disconnect().await;
    }

    #[tokio::test]
    async fn cancelled_setup_retires_receiver_before_the_next_connection() {
        let (client, mut receiver, listener) = fixture().await;
        let (old_setup, wait_old_setup) = oneshot::channel();
        let (send_late, wait_send_late) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut first = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            let _ = message(&mut first).await;
            old_setup.send(()).unwrap();
            wait_send_late.await.unwrap();
            let _ = first
                .send(Message::Text(
                    json!({"setupComplete":{}}).to_string().into(),
                ))
                .await;
            let _ = first
                .send(Message::Text(
                    json!({"serverContent":{"inputTranscription":{"text":"Late old final"}}})
                        .to_string()
                        .into(),
                ))
                .await;
            let mut second = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            setup(&mut second).await;
            send(
                &mut second,
                json!({"serverContent":{"inputTranscription":{"text":"Fresh final"}}}),
            )
            .await;
            while second.next().await.is_some() {}
        });
        let mut connect = Box::pin(client.connect("cancelled"));
        tokio::select! { _ = &mut connect => panic!("setup must wait for acknowledgement"), _ = wait_old_setup => {} }
        drop(connect);
        send_late.send(()).unwrap();
        client.connect("fresh").await.unwrap();
        let event = next_content(&mut receiver).await;
        assert!(
            matches!(event.event, LiveTranslateServerEvent::SourceUtteranceFinal { ref text, .. } if text == "Fresh final")
        );
        client.disconnect().await;
        server.await.unwrap();
    }
}
