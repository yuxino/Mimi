//! Dedicated Gemini Live transcription with bounded, independent MT events.

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::clients::transport::provider_network::{ProviderNetwork, ProviderNetworkError};
use crate::core::audio::pending_pcm::PendingPcmGate;
use crate::core::models::SourceLanguage;
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::recognition::gemini_transcribe::{
    self, ServerEvent, TranscriptAssembler,
};
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
            crate::clients::transport::provider_network::websocket_with_message_limit(
                request,
                &self.network,
                gemini_transcribe::MAX_MESSAGE_BYTES,
            ),
        )
        .await
        .map_err(|_| GeminiTranscribeClientError::Unreachable)?
        .map_err(|error| {
            if crate::clients::transport::connection_diagnostics::authentication_rejected(&error) {
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
mod tests;
