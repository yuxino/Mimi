//! xAI streaming recognition, independent of its conversational Voice API.

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::clients::transport::provider_network::{ProviderNetwork, ProviderNetworkError};
use crate::core::audio::pending_pcm::PendingPcmGate;
use crate::core::models::SourceLanguage;
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::recognition::xai_stt::{self, ServerEvent, TranscriptAssembler};
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
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Sink = futures_util::stream::SplitSink<Socket, Message>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum XaiSttClientError {
    #[error("xai_stt_credentials_missing")]
    MissingAPIKey,
    #[error("xai_stt_language_unsupported")]
    UnsupportedLanguage,
    #[error("xai_stt_authentication_failed")]
    AuthenticationFailed,
    #[error("xai_stt_unreachable")]
    Unreachable,
    #[error("xai_stt_session_rejected")]
    SessionRejected,
    #[error("xai_stt_setup_timeout")]
    SetupTimeout,
    #[error("xai_stt_protocol_invalid")]
    InvalidProtocol,
    #[error("xai_stt_audio_invalid")]
    InvalidAudio,
    #[error("xai_stt_not_connected")]
    NotConnected,
    #[error("xai_stt_health_timeout")]
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
    failed: AtomicBool,
    failure: std::sync::Mutex<Option<XaiSttClientError>>,
    pong: Notify,
    progress: Notify,
}

#[derive(Clone)]
pub struct XaiSttClient {
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
        let retired = self.generation.wrapping_add(1);
        if !self.complete
            && self
                .inner
                .generation
                .compare_exchange(self.generation, retired, Ordering::SeqCst, Ordering::SeqCst)
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

impl XaiSttClient {
    pub fn new(api_key: &str, source: SourceLanguage) -> Result<Self, XaiSttClientError> {
        let endpoint =
            xai_stt::endpoint(source).map_err(|_| XaiSttClientError::UnsupportedLanguage)?;
        let api_key = api_key.trim();
        if api_key.is_empty() || HeaderValue::from_str(&format!("Bearer {api_key}")).is_err() {
            return Err(XaiSttClientError::MissingAPIKey);
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
    pub async fn connect(&self, _task_id: &str) -> Result<(), XaiSttClientError> {
        self.connect_with_timeout(SETUP_TIMEOUT).await
    }
    pub async fn connect_for_probe(&self, task_id: &str) -> Result<(), XaiSttClientError> {
        self.connect(task_id).await
    }

    async fn connect_with_timeout(&self, timeout: Duration) -> Result<(), XaiSttClientError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        self.disconnect_locked().await;
        self.connect_locked(timeout).await
    }

    async fn connect_locked(&self, setup_timeout: Duration) -> Result<(), XaiSttClientError> {
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
            .ok_or(XaiSttClientError::NotConnected)?;
        self.inner.content.lock().unwrap().revision = events.content_revision();
        let mut request = self
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|_| XaiSttClientError::InvalidProtocol)?;
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                .map_err(|_| XaiSttClientError::MissingAPIKey)?,
        );
        let socket = tokio::time::timeout(
            Duration::from_secs(15),
            crate::clients::transport::provider_network::websocket_with_message_limit(
                request,
                &self.network,
                xai_stt::MAX_MESSAGE_BYTES,
            ),
        )
        .await
        .map_err(|_| XaiSttClientError::Unreachable)?
        .map_err(|error| {
            if crate::clients::transport::connection_diagnostics::authentication_rejected(&error) {
                XaiSttClientError::AuthenticationFailed
            } else {
                XaiSttClientError::Unreachable
            }
        })?
        .0;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.closing.store(false, Ordering::SeqCst);
        self.inner.done.store(false, Ordering::SeqCst);
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
        .unwrap_or(Err(XaiSttClientError::SetupTimeout));
        if result.is_err() {
            self.disconnect_locked().await;
        }
        attempt.complete = true;
        result
    }

    pub async fn send_audio(&self, pcm: &[u8]) -> Result<(), XaiSttClientError> {
        if pcm.is_empty() {
            return Ok(());
        }
        if !pcm.len().is_multiple_of(2) || pcm.len() > xai_stt::MAX_PCM_CHUNK_BYTES {
            return Err(XaiSttClientError::InvalidAudio);
        }
        let operation = async {
            let _audio = self.inner.audio_operation.lock().await;
            if !self.inner.ready.load(Ordering::SeqCst) || self.inner.closing.load(Ordering::SeqCst)
            {
                return Err(XaiSttClientError::NotConnected);
            }
            self.send_message(Message::Binary(pcm.to_vec().into()))
                .await
        };
        tokio::time::timeout(SEND_TIMEOUT, operation)
            .await
            .unwrap_or(Err(XaiSttClientError::Unreachable))
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), XaiSttClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) {
            return Err(XaiSttClientError::NotConnected);
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
        .unwrap_or(Err(XaiSttClientError::HealthTimeout))
    }

    pub async fn clear_content(&self) -> u64 {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let reconnect =
            self.inner.ready.load(Ordering::SeqCst) && !self.inner.closing.load(Ordering::SeqCst);
        // xAI has no audio-clear acknowledgement. A fresh socket is the only
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
                    self.send_message(Message::Text("{\"type\":\"audio.done\"}".into()))
                        .await?;
                }
                loop {
                    let progress = self.inner.progress.notified();
                    tokio::pin!(progress);
                    progress.as_mut().enable();
                    if self.inner.done.load(Ordering::SeqCst)
                        || self.inner.failed.load(Ordering::SeqCst)
                    {
                        break;
                    }
                    progress.await;
                }
                Ok::<(), XaiSttClientError>(())
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
        self.inner.content.lock().unwrap().transcript.reset();
        self.inner.failed.store(false, Ordering::SeqCst);
        *self.inner.failure.lock().unwrap() = None;
    }

    async fn send_message(&self, message: Message) -> Result<(), XaiSttClientError> {
        tokio::time::timeout(SEND_TIMEOUT, async {
            let mut sink = self.inner.sink.lock().await;
            let sink = sink.as_mut().ok_or(XaiSttClientError::NotConnected)?;
            let pcm = match &message {
                Message::Binary(pcm) => pcm.as_ref(),
                _ => &[],
            };
            crate::development_audio::begin_pcm(pcm, xai_stt::PCM_SAMPLE_RATE)
                .observe(sink.send(message))
                .await
                .map_err(|_| XaiSttClientError::Unreachable)
        })
        .await
        .unwrap_or(Err(XaiSttClientError::Unreachable))
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
                Ok(Message::Text(text)) => match xai_stt::decode(&text) {
                    Ok(event) => event,
                    Err(_) => {
                        self.fail(XaiSttClientError::InvalidProtocol, generation);
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
                Ok(Message::Binary(_)) => {
                    self.fail(XaiSttClientError::InvalidProtocol, generation);
                    let _ = setup.send(Setup::Rejected);
                    return;
                }
                _ => {
                    if !self.inner.done.load(Ordering::SeqCst) {
                        self.fail(XaiSttClientError::Unreachable, generation);
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
                    self.fail(XaiSttClientError::InvalidProtocol, generation);
                    let _ = setup.send(Setup::Rejected);
                    return;
                }
                ServerEvent::AuthenticationRejected | ServerEvent::Rejected => {
                    self.fail(
                        if event == ServerEvent::AuthenticationRejected {
                            XaiSttClientError::AuthenticationFailed
                        } else {
                            XaiSttClientError::SessionRejected
                        },
                        generation,
                    );
                    let _ = setup.send(Setup::Rejected);
                    return;
                }
                ServerEvent::Ignored => {}
                event if created => {
                    let done = matches!(event, ServerEvent::Done { .. });
                    if done && !self.inner.closing.load(Ordering::SeqCst) {
                        self.fail(XaiSttClientError::InvalidProtocol, generation);
                        let _ = setup.send(Setup::Rejected);
                        return;
                    }
                    let mut content = self.inner.content.lock().unwrap();
                    let output = content.transcript.apply(event);
                    match output {
                        Ok(Some(output)) => {
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
                        Ok(None) => {}
                        Err(_) => {
                            drop(content);
                            self.fail(XaiSttClientError::InvalidProtocol, generation);
                            return;
                        }
                    }
                    if done {
                        self.inner.done.store(true, Ordering::SeqCst);
                        self.inner.ready.store(false, Ordering::SeqCst);
                    }
                    drop(content);
                    self.inner.progress.notify_waiters();
                    if done {
                        return;
                    }
                }
                _ => {
                    self.fail(XaiSttClientError::InvalidProtocol, generation);
                    let _ = setup.send(Setup::Rejected);
                    return;
                }
            }
        }
        if self.current(generation) && !self.inner.done.load(Ordering::SeqCst) {
            self.fail(XaiSttClientError::Unreachable, generation);
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
    fn fail(&self, error: XaiSttClientError, generation: u64) {
        if !self.current(generation) || self.inner.failed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner.ready.store(false, Ordering::SeqCst);
        *self.inner.failure.lock().unwrap() = Some(error.clone());
        self.emit(
            LiveTranslateServerEvent::Error {
                code: error.to_string(),
                message: error.to_string(),
            },
            generation,
        );
        self.inner.progress.notify_waiters();
    }
    fn failure(&self) -> XaiSttClientError {
        self.inner
            .failure
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(XaiSttClientError::SessionRejected)
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
    use tokio::sync::oneshot;
    use tokio_tungstenite::{accept_async, accept_hdr_async};
    type Server = WebSocketStream<TcpStream>;

    async fn fixture() -> (XaiSttClient, ProviderEventReceiver, TcpListener) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = XaiSttClient::new("synthetic-key", SourceLanguage::English).unwrap();
        let mut endpoint = client.endpoint.clone();
        endpoint.set_scheme("ws").unwrap();
        endpoint.set_host(Some("127.0.0.1")).unwrap();
        endpoint
            .set_port(Some(listener.local_addr().unwrap().port()))
            .unwrap();
        client.endpoint = endpoint;
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
    async fn send(socket: &mut Server, event: Value) {
        socket
            .send(Message::Text(event.to_string().into()))
            .await
            .unwrap();
    }
    fn partial(text: &str, is_final: bool, speech_final: bool, start: f64, duration: f64) -> Value {
        json!({"type":"transcript.partial", "text":text, "is_final":is_final,"speech_final":speech_final,"start":start,"duration":duration})
    }
    async fn accept(listener: &TcpListener) -> Server {
        accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap()
    }
    async fn next_content(
        receiver: &mut ProviderEventReceiver,
    ) -> crate::clients::transport::provider_events::ProviderEvent {
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
    async fn next_message(socket: &mut Server) -> Message {
        tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    #[allow(clippy::result_large_err)] // Tungstenite requires this callback signature.
    async fn waits_for_created_sends_pcm_and_commits_chunks_without_a_pause() {
        let (client, mut receiver, listener) = fixture().await;
        let (connected, await_connected) = oneshot::channel();
        let (ready, await_ready) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut socket = accept_hdr_async(
                listener.accept().await.unwrap().0,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(
                        request.headers().get("Authorization").unwrap(),
                        "Bearer synthetic-key"
                    );
                    let url = url::Url::parse(&format!("ws://fixture{}", request.uri())).unwrap();
                    let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
                    assert_eq!(query.get("encoding").unwrap(), "pcm");
                    assert_eq!(query.get("sample_rate").unwrap(), "16000");
                    assert_eq!(query.get("interim_results").unwrap(), "true");
                    assert_eq!(query.get("language").unwrap(), "en");
                    Ok(response)
                },
            )
            .await
            .unwrap();
            connected.send(()).unwrap();
            await_ready.await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(30), socket.next())
                    .await
                    .is_err()
            );
            send(&mut socket, json!({"type":"transcript.created"})).await;
            assert_eq!(
                next_message(&mut socket).await,
                Message::Binary(vec![1, 0, 2, 0].into())
            );
            send(
                &mut socket,
                partial("synthetic chunk", true, false, 0.0, 3.0),
            )
            .await;
            let message = next_message(&mut socket).await;
            assert_eq!(message, Message::Text("{\"type\":\"audio.done\"}".into()));
            send(
                &mut socket,
                json!({"type":"transcript.done","text":"synthetic chunk","duration":3.0}),
            )
            .await;
        });
        let connecting = tokio::spawn({
            let client = client.clone();
            async move { client.connect("fixture").await }
        });
        await_connected.await.unwrap();
        assert_eq!(
            client.send_audio(&[1, 0]).await,
            Err(XaiSttClientError::NotConnected)
        );
        ready.send(()).unwrap();
        connecting.await.unwrap().unwrap();
        client.send_audio(&[1, 0, 2, 0]).await.unwrap();
        assert!(
            matches!(next_content(&mut receiver).await.event, LiveTranslateServerEvent::SourceUtteranceFinal { utterance_id:1, text, language:Some(language) } if text == "synthetic chunk" && language == "en")
        );
        client.finish(Duration::from_secs(1)).await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn revisions_keep_identity_and_cumulative_finals_do_not_repeat_chunks() {
        let (client, mut receiver, listener) = fixture().await;
        let (advance, mut steps) = tokio::sync::mpsc::channel::<()>(8);
        let server = tokio::spawn(async move {
            let mut socket = accept(&listener).await;
            send(&mut socket, json!({"type":"transcript.created"})).await;
            for event in [
                partial("synthetic dra", false, false, 0.0, 1.0),
                partial("synthetic draft", false, false, 0.0, 2.0),
                partial("synthetic first", true, false, 0.0, 3.0),
                partial("second", false, false, 3.0, 1.0),
                partial("synthetic first second", true, true, 0.0, 4.0),
                partial("synthetic first second", true, true, 0.0, 4.0),
                partial("third", true, true, 4.0, 2.0),
            ] {
                steps.recv().await.unwrap();
                send(&mut socket, event).await;
            }
            assert_eq!(
                next_message(&mut socket).await,
                Message::Text("{\"type\":\"audio.done\"}".into())
            );
            send(&mut socket, json!({"type":"transcript.done","text":"synthetic first second third tail","duration":7.0})).await;
        });
        client.connect("fixture").await.unwrap();
        for (text, id, final_event) in [
            ("synthetic dra", 1, false),
            ("synthetic draft", 1, false),
            ("synthetic first", 1, true),
            ("second", 2, false),
            ("second", 2, true),
        ] {
            advance.send(()).await.unwrap();
            let event = next_content(&mut receiver).await.event;
            match event {
                LiveTranslateServerEvent::SourceUtteranceDraft {
                    utterance_id,
                    text: actual,
                    ..
                } if !final_event => {
                    assert_eq!(utterance_id, id);
                    assert_eq!(actual, text);
                }
                LiveTranslateServerEvent::SourceUtteranceFinal {
                    utterance_id,
                    text: actual,
                    ..
                } if final_event => {
                    assert_eq!(utterance_id, id);
                    assert_eq!(actual, text);
                }
                other => panic!("Unexpected synthetic event: {other:?}"),
            }
        }
        advance.send(()).await.unwrap();
        advance.send(()).await.unwrap();
        assert!(
            matches!(next_content(&mut receiver).await.event, LiveTranslateServerEvent::SourceUtteranceFinal { utterance_id:3, text,.. } if text == "third")
        );
        client.finish(Duration::from_secs(1)).await;
        assert!(
            matches!(next_content(&mut receiver).await.event, LiveTranslateServerEvent::SourceUtteranceFinal { utterance_id:4, text,.. } if text == "tail")
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn empty_final_withdraws_a_draft_and_stop_flushes_before_finishing() {
        let (client, mut receiver, listener) = fixture().await;
        let (advance, proceed) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut socket = accept(&listener).await;
            send(&mut socket, json!({"type":"transcript.created"})).await;
            send(
                &mut socket,
                partial("synthetic draft", false, false, 0.0, 1.0),
            )
            .await;
            proceed.await.unwrap();
            send(&mut socket, partial("", true, false, 0.0, 1.0)).await;
            assert_eq!(
                next_message(&mut socket).await,
                Message::Text("{\"type\":\"audio.done\"}".into())
            );
            send(&mut socket, partial("last", true, false, 1.0, 1.0)).await;
            send(
                &mut socket,
                json!({"type":"transcript.done","text":"last","duration":2.0}),
            )
            .await;
            let _ = socket
                .send(Message::Text(
                    partial("late", true, false, 2.0, 1.0).to_string().into(),
                ))
                .await;
        });
        client.connect("fixture").await.unwrap();
        assert!(matches!(
            next_content(&mut receiver).await.event,
            LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: 1,
                ..
            }
        ));
        advance.send(()).unwrap();
        assert!(
            matches!(next_content(&mut receiver).await.event, LiveTranslateServerEvent::SourceUtteranceFinal { utterance_id:1,text,.. } if text.is_empty())
        );
        client.finish(Duration::from_secs(1)).await;
        assert!(
            matches!(next_content(&mut receiver).await.event, LiveTranslateServerEvent::SourceUtteranceFinal { utterance_id:2,text,.. } if text == "last")
        );
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(20), receiver.recv_with_revision()).await
        {
            assert!(!event.is_content());
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn clear_reconnects_and_old_socket_content_cannot_acquire_new_revision() {
        let (client, mut receiver, listener) = fixture().await;
        let (send_old, old_sent) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut old = accept(&listener).await;
            send(&mut old, json!({"type":"transcript.created"})).await;
            send(&mut old, partial("old draft", false, false, 0.0, 1.0)).await;
            old_sent.await.unwrap();
            let _ = old
                .send(Message::Text(
                    partial("old late final", true, true, 0.0, 2.0)
                        .to_string()
                        .into(),
                ))
                .await;
            let mut new = accept(&listener).await;
            send(&mut new, json!({"type":"transcript.created"})).await;
            send(&mut new, partial("new final", true, false, 0.0, 3.0)).await;
            while new
                .next()
                .await
                .is_some_and(|message| !matches!(message, Ok(Message::Close(_))))
            {}
        });
        client.connect("fixture").await.unwrap();
        assert_eq!(next_content(&mut receiver).await.content_revision, 0);
        let clearing = tokio::spawn({
            let client = client.clone();
            async move { client.clear_content().await }
        });
        send_old.send(()).unwrap();
        assert_eq!(clearing.await.unwrap(), 1);
        let event = next_content(&mut receiver).await;
        assert_eq!(event.content_revision, 1);
        assert!(
            matches!(event.event,LiveTranslateServerEvent::SourceUtteranceFinal{utterance_id:2,text,..} if text == "new final")
        );
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn setup_rejects_content_without_created_and_never_exposes_error_messages() {
        for event in [
            json!({"type":"error","code":"invalid_api_key","message":"synthetic-private-content"}),
            partial("private", true, false, 0.0, 1.0),
        ] {
            let (client, mut receiver, listener) = fixture().await;
            let authentication = event["type"] == "error";
            let server = tokio::spawn(async move {
                let mut socket = accept(&listener).await;
                send(&mut socket, event).await;
                let _ = socket.next().await;
            });
            let error = client.connect("fixture").await.unwrap_err();
            assert_eq!(
                error,
                if authentication {
                    XaiSttClientError::AuthenticationFailed
                } else {
                    XaiSttClientError::InvalidProtocol
                }
            );
            assert!(!error.to_string().contains("private"));
            let event = receiver.recv_with_revision().await.unwrap();
            assert!(
                matches!(event.event,LiveTranslateServerEvent::Error { code,message } if code == error.to_string() && message == code)
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn setup_timeout_and_cancellation_can_reconnect() {
        let (client, mut receiver, listener) = fixture().await;
        let (accepted, connected) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut first = accept(&listener).await;
            let _ = first.next().await;
            let mut second = accept(&listener).await;
            accepted.send(()).unwrap();
            let mut third = accept(&listener).await;
            let _ = second
                .send(Message::Text(
                    partial("old cancelled", true, false, 0.0, 1.0)
                        .to_string()
                        .into(),
                ))
                .await;
            send(&mut third, json!({"type":"transcript.created"})).await;
            send(&mut third, partial("recovered", true, false, 0.0, 1.0)).await;
            let _ = third.next().await;
        });
        assert_eq!(
            client.connect_with_timeout(Duration::from_millis(30)).await,
            Err(XaiSttClientError::SetupTimeout)
        );
        let connecting = tokio::spawn({
            let client = client.clone();
            async move { client.connect("cancelled").await }
        });
        connected.await.unwrap();
        connecting.abort();
        let _ = connecting.await;
        client.connect("retry").await.unwrap();
        assert!(
            matches!(next_content(&mut receiver).await.event,LiveTranslateServerEvent::SourceUtteranceFinal {text,..} if text == "recovered")
        );
        client.disconnect().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn ping_uses_pong_and_finish_wait_is_bounded() {
        let (client, _receiver, listener) = fixture().await;
        let server = tokio::spawn(async move {
            let mut socket = accept(&listener).await;
            send(&mut socket, json!({"type":"transcript.created"})).await;
            match next_message(&mut socket).await {
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                other => panic!("Unexpected synthetic message: {other:?}"),
            }
            assert_eq!(
                next_message(&mut socket).await,
                Message::Text("{\"type\":\"audio.done\"}".into())
            );
            let _ = socket.next().await;
        });
        client.connect("fixture").await.unwrap();
        client.ping(Duration::from_secs(1)).await.unwrap();
        let started = tokio::time::Instant::now();
        client.finish(Duration::from_millis(30)).await;
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(
            client.send_audio(&[0, 0]).await,
            Err(XaiSttClientError::NotConnected)
        );
        server.await.unwrap();
    }
}
