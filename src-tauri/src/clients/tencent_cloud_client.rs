//! Tencent Cloud realtime speech-translation WebSocket client.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::tencent_cloud::{
    TencentCloudEndpoint, TencentCloudRequestEncoder, TencentCloudServerEvent,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(250);
const SIGNATURE_LIFETIME_SECONDS: u64 = 24 * 60 * 60;
const MAXIMUM_AUDIO_CHUNK_BYTES: usize = TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT * 64;
const GENERIC_PROVIDER_ERROR: &str = "Tencent Cloud realtime translation rejected the session.";
const GENERIC_PROTOCOL_ERROR: &str =
    "Tencent Cloud realtime translation returned an invalid response.";
const GENERIC_TRANSPORT_ERROR: &str = "The Tencent Cloud realtime translation connection failed.";
const UNEXPECTED_SESSION_END_CODE: &str = "tencent_unexpected_session_end";
const UNEXPECTED_SESSION_END_ERROR: &str =
    "Tencent Cloud realtime translation ended the session unexpectedly.";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TencentCloudClientError {
    #[error("Add a Tencent Cloud AppID, SecretID, and SecretKey in Settings.")]
    MissingCredentials,
    #[error("Tencent Cloud realtime translation requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("The Tencent Cloud realtime translation session is not connected.")]
    NotConnected,
    #[error("The Tencent Cloud realtime translation connection stopped responding.")]
    HealthCheckTimedOut,
    #[error("The Tencent Cloud realtime translation connection failed.")]
    TransportFailure,
    #[error("Tencent Cloud realtime translation rejected the session configuration.")]
    SessionSetupRejected,
    #[error("Tencent Cloud realtime translation did not confirm the session in time.")]
    SessionSetupTimedOut,
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type Stream = futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected,
}

#[derive(Default)]
struct SentenceContentState {
    in_progress: bool,
    discard_until_end: bool,
}

impl SentenceContentState {
    fn accepts_content(&mut self, expected: u64, current: u64, sentence_end: bool) -> bool {
        if expected != current || self.discard_until_end {
            // Even an already decoded old final remains a real turn boundary.
            // Consume that boundary without publishing its obsolete content.
            if sentence_end {
                *self = Self::default();
            }
            return false;
        }
        self.in_progress = !sentence_end;
        true
    }
}

struct Inner {
    content_lock: Mutex<SentenceContentState>,
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    audio_send_lock: Mutex<()>,
    pending_audio: Mutex<Vec<u8>>,
    ready: AtomicBool,
    is_closing: AtomicBool,
    received_session_finished: AtomicBool,
    finish_notify: Notify,
    pong_notify: Notify,
    generation: AtomicU64,
}

/// Secret fields intentionally live in a non-`Debug` client and are never
/// interpolated into an error or diagnostic event.
#[derive(Clone)]
pub struct TencentCloudClient {
    network: super::provider_network::ProviderNetwork,
    inner: Arc<Inner>,
    endpoint_override: Option<url::Url>,
    app_id: String,
    secret_id: String,
    secret_key: String,
    source_language: SourceLanguage,
    target_language: TargetLanguage,
    events: ProviderEventSender,
}

impl TencentCloudClient {
    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    pub async fn clear_content(&self) -> u64 {
        let mut turn = self.inner.content_lock.lock().await;
        turn.discard_until_end |= turn.in_progress;
        turn.in_progress = false;
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
        app_id: &str,
        secret_id: &str,
        secret_key: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        events: ProviderEventSender,
    ) -> Result<Self, TencentCloudClientError> {
        // Validate the static credential/language shape without retaining a
        // signed URL. Production creates a fresh single-use URL in `connect`.
        TencentCloudEndpoint::new(
            app_id,
            secret_id,
            secret_key,
            source_language,
            target_language,
            1,
            2,
            1,
            "validation",
        )
        .map_err(map_protocol_configuration_error)?;
        Ok(Self::from_parts(
            None,
            app_id,
            secret_id,
            secret_key,
            source_language,
            target_language,
            events,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        endpoint_override: Option<url::Url>,
        app_id: &str,
        secret_id: &str,
        secret_key: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        events: ProviderEventSender,
    ) -> Self {
        Self {
            network: super::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                content_lock: Mutex::new(SentenceContentState::default()),
                sink: Mutex::new(None),
                receive_task: Mutex::new(None),
                audio_send_lock: Mutex::new(()),
                pending_audio: Mutex::new(Vec::new()),
                ready: AtomicBool::new(false),
                is_closing: AtomicBool::new(false),
                received_session_finished: AtomicBool::new(false),
                finish_notify: Notify::new(),
                pong_notify: Notify::new(),
                generation: AtomicU64::new(0),
            }),
            endpoint_override,
            app_id: app_id.trim().to_string(),
            secret_id: secret_id.trim().to_string(),
            secret_key: secret_key.trim().to_string(),
            source_language,
            target_language,
            events,
        }
    }

    #[cfg(test)]
    fn with_endpoint(
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        events: ProviderEventSender,
        endpoint: url::Url,
    ) -> Self {
        Self::from_parts(
            Some(endpoint),
            "1250000000",
            "AKIDTEST",
            "test-secret-not-real",
            source_language,
            target_language,
            events,
        )
    }

    pub async fn connect(&self) -> Result<(), TencentCloudClientError> {
        self.connect_with_timeout(Duration::from_secs(5)).await
    }

    async fn connect_with_timeout(
        &self,
        readiness_timeout: Duration,
    ) -> Result<(), TencentCloudClientError> {
        self.disconnect().await;
        *self.inner.content_lock.lock().await = SentenceContentState::default();
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let endpoint = self.endpoint_for_connection()?;
        let request = endpoint
            .into_client_request()
            .map_err(|_| TencentCloudClientError::TransportFailure)?;
        let (socket, _) = tokio::time::timeout(
            CONNECT_TIMEOUT,
            super::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| TencentCloudClientError::TransportFailure)?
        .map_err(|_| TencentCloudClientError::TransportFailure)?;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .received_session_finished
            .store(false, Ordering::SeqCst);
        self.inner.pending_audio.lock().await.clear();

        let (setup_tx, mut setup_rx) = watch::channel(SetupState::Awaiting);
        let task = tokio::spawn(receive_loop(ReceiveContext {
            inner: Arc::clone(&self.inner),
            stream,
            events: self.events.clone(),
            setup: setup_tx,
            generation,
        }));
        *self.inner.receive_task.lock().await = Some(task);

        let wait_for_ready = async {
            loop {
                match *setup_rx.borrow() {
                    SetupState::Ready => return Ok(()),
                    SetupState::Rejected => {
                        return Err(TencentCloudClientError::SessionSetupRejected)
                    }
                    SetupState::Awaiting => {}
                }
                if setup_rx.changed().await.is_err() {
                    return Err(TencentCloudClientError::SessionSetupRejected);
                }
            }
        };
        match tokio::time::timeout(readiness_timeout, wait_for_ready).await {
            Ok(Ok(())) => {
                self.inner.ready.store(true, Ordering::SeqCst);
                Ok(())
            }
            Ok(Err(error)) => {
                self.disconnect().await;
                Err(error)
            }
            Err(_) => {
                self.disconnect().await;
                Err(TencentCloudClientError::SessionSetupTimedOut)
            }
        }
    }

    fn endpoint_for_connection(&self) -> Result<url::Url, TencentCloudClientError> {
        if let Some(endpoint) = &self.endpoint_override {
            return Ok(endpoint.clone());
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| TencentCloudClientError::TransportFailure)?
            .as_secs();
        let expired = timestamp
            .checked_add(SIGNATURE_LIFETIME_SECONDS)
            .ok_or(TencentCloudClientError::TransportFailure)?;
        let random = Uuid::new_v4().as_u128();
        let nonce = (random % 9_999_999_999) as u64 + 1;
        let voice_id = Uuid::new_v4().hyphenated().to_string();
        TencentCloudEndpoint::new(
            &self.app_id,
            &self.secret_id,
            &self.secret_key,
            self.source_language,
            self.target_language,
            timestamp,
            expired,
            nonce,
            &voice_id,
        )
        .map(|endpoint| endpoint.url().clone())
        .map_err(map_protocol_configuration_error)
    }

    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), TencentCloudClientError> {
        if pcm_data.is_empty() {
            return Ok(());
        }
        if pcm_data.len() > MAXIMUM_AUDIO_CHUNK_BYTES {
            return Err(TencentCloudClientError::TransportFailure);
        }
        tokio::time::timeout(SEND_TIMEOUT, self.send_audio_operation(pcm_data))
            .await
            .map_err(|_| TencentCloudClientError::TransportFailure)?
    }

    async fn send_audio_operation(&self, pcm_data: &[u8]) -> Result<(), TencentCloudClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(TencentCloudClientError::NotConnected);
        }
        let _send_guard = self.inner.audio_send_lock.lock().await;
        let frames = {
            let mut pending = self.inner.pending_audio.lock().await;
            pending.extend_from_slice(pcm_data);
            take_complete_frames(&mut pending)
        };
        for frame in frames {
            self.send_binary(frame).await?;
        }
        Ok(())
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), TencentCloudClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(TencentCloudClientError::NotConnected);
        }
        let operation = async {
            let pong = self.inner.pong_notify.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(TencentCloudClientError::NotConnected);
                };
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| TencentCloudClientError::TransportFailure)?;
            }
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| TencentCloudClientError::HealthCheckTimedOut)?
    }

    pub async fn finish(&self, timeout: Duration) {
        if !self.inner.ready.load(Ordering::SeqCst) {
            return;
        }
        if self.inner.is_closing.swap(true, Ordering::SeqCst) {
            return;
        }
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let operation = async {
            let _send_guard = self.inner.audio_send_lock.lock().await;
            let final_frame = {
                let mut pending = self.inner.pending_audio.lock().await;
                take_padded_frame(&mut pending)
            };
            if let Some(frame) = final_frame {
                self.send_binary(frame).await?;
            }
            self.send_text(TencentCloudRequestEncoder::finish().to_string())
                .await?;
            let finished = self.inner.finish_notify.notified();
            tokio::pin!(finished);
            finished.as_mut().enable();
            if !self.inner.received_session_finished.load(Ordering::SeqCst) {
                finished.await;
            }
            Ok::<(), TencentCloudClientError>(())
        };
        match tokio::time::timeout(timeout, operation).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                self.emit_error("tencent_finish_failed", GENERIC_TRANSPORT_ERROR, generation)
            }
            Err(_) => self.emit_error(
                "tencent_finish_timeout",
                "Tencent Cloud realtime translation did not finish in time.",
                generation,
            ),
        }
        self.disconnect_if_current(generation).await;
    }

    pub async fn disconnect(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .received_session_finished
            .store(false, Ordering::SeqCst);
        if let Some(task) = self.inner.receive_task.lock().await.take() {
            task.abort();
        }
        if let Some(mut sink) = self.inner.sink.lock().await.take() {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.close()).await;
        }
        self.inner.pending_audio.lock().await.clear();
        self.inner.finish_notify.notify_waiters();
    }

    async fn send_binary(&self, frame: Vec<u8>) -> Result<(), TencentCloudClientError> {
        TencentCloudRequestEncoder::validate_audio_frame(&frame)
            .map_err(|_| TencentCloudClientError::TransportFailure)?;
        let mut sink = self.inner.sink.lock().await;
        let Some(sink) = sink.as_mut() else {
            return Err(TencentCloudClientError::NotConnected);
        };
        let evidence =
            crate::development_audio::begin_pcm(&frame, TencentCloudEndpoint::SAMPLE_RATE_HZ);
        tokio::time::timeout(
            SEND_TIMEOUT,
            evidence.observe(sink.send(Message::Binary(frame.into()))),
        )
        .await
        .map_err(|_| TencentCloudClientError::TransportFailure)?
        .map_err(|_| TencentCloudClientError::TransportFailure)
    }

    async fn send_text(&self, text: String) -> Result<(), TencentCloudClientError> {
        let mut sink = self.inner.sink.lock().await;
        let Some(sink) = sink.as_mut() else {
            return Err(TencentCloudClientError::NotConnected);
        };
        tokio::time::timeout(SEND_TIMEOUT, sink.send(Message::Text(text.into())))
            .await
            .map_err(|_| TencentCloudClientError::TransportFailure)?
            .map_err(|_| TencentCloudClientError::TransportFailure)
    }

    fn emit_error(&self, code: &str, message: &str, generation: u64) {
        if self.inner.generation.load(Ordering::SeqCst) == generation {
            let _ = self.events.send(LiveTranslateServerEvent::Error {
                code: code.into(),
                message: message.into(),
            });
        }
    }

    async fn disconnect_if_current(&self, generation: u64) {
        if self.inner.generation.load(Ordering::SeqCst) == generation {
            self.disconnect().await;
        }
    }
}

fn map_protocol_configuration_error(
    error: crate::core::protocols::tencent_cloud::TencentCloudProtocolError,
) -> TencentCloudClientError {
    use crate::core::protocols::tencent_cloud::TencentCloudProtocolError as ProtocolError;
    match error {
        ProtocolError::MissingAppID
        | ProtocolError::MissingSecretID
        | ProtocolError::MissingSecretKey
        | ProtocolError::InvalidSessionIdentity => TencentCloudClientError::MissingCredentials,
        ProtocolError::InvalidTargetLanguage => TencentCloudClientError::InvalidTargetLanguage,
        _ => TencentCloudClientError::TransportFailure,
    }
}

fn take_complete_frames(pending: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let frame_size = TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let complete_bytes = pending.len() / frame_size * frame_size;
    let frames = pending[..complete_bytes]
        .chunks_exact(frame_size)
        .map(<[u8]>::to_vec)
        .collect::<Vec<_>>();
    pending.drain(..complete_bytes);
    frames
}

fn take_padded_frame(pending: &mut Vec<u8>) -> Option<Vec<u8>> {
    if pending.is_empty() {
        return None;
    }
    let mut frame = std::mem::take(pending);
    frame.resize(TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT, 0);
    Some(frame)
}

struct ReceiveContext {
    inner: Arc<Inner>,
    stream: Stream,
    events: ProviderEventSender,
    setup: watch::Sender<SetupState>,
    generation: u64,
}

async fn receive_loop(mut context: ReceiveContext) {
    while let Some(message) = context.stream.next().await {
        if context.inner.generation.load(Ordering::SeqCst) != context.generation {
            return;
        }
        let event = match message {
            Ok(Message::Text(text)) => TencentCloudServerEvent::decode(&text),
            Ok(Message::Pong(_)) => {
                context.inner.pong_notify.notify_waiters();
                continue;
            }
            // Binary responses are TTS audio. Mimi never enables TTS and does
            // not retain provider audio, so an unexpected payload is ignored.
            Ok(Message::Binary(_)) | Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => {
                fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR);
                return;
            }
        };
        let event = match event {
            Ok(event) => event,
            Err(_) => {
                fail_receive_loop(&context, "tencent_protocol_error", GENERIC_PROTOCOL_ERROR);
                return;
            }
        };
        if handle_server_event(&context, event).await {
            return;
        }
    }
    if context.inner.generation.load(Ordering::SeqCst) == context.generation
        && !context
            .inner
            .received_session_finished
            .load(Ordering::SeqCst)
    {
        fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR);
    }
}

async fn handle_server_event(context: &ReceiveContext, event: TencentCloudServerEvent) -> bool {
    let revision = context.events.content_revision();
    let mut turn = context.inner.content_lock.lock().await;
    if context.inner.generation.load(Ordering::SeqCst) != context.generation {
        return false;
    }

    match event {
        TencentCloudServerEvent::SessionReady => {
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Ready);
                emit_if_current(context, LiveTranslateServerEvent::SessionCreated);
                emit_if_current(context, LiveTranslateServerEvent::SessionUpdated);
            }
        }
        TencentCloudServerEvent::Transcript {
            source_text,
            target_text,
            source_language,
            sentence_end,
        } => {
            if !turn.accepts_content(revision, context.events.content_revision(), sentence_end) {
                return false;
            }
            if sentence_end {
                if !source_text.trim().is_empty() && !target_text.trim().is_empty() {
                    emit_if_current(
                        context,
                        LiveTranslateServerEvent::SubtitleFinalPair {
                            source: source_text.trim().to_string(),
                            language: Some(source_language),
                            translation: target_text.trim().to_string(),
                        },
                    );
                }
            } else {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::SourceDraft {
                        text: source_text,
                        language: Some(source_language),
                    },
                );
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::TranslationDraft(target_text),
                );
            }
        }
        TencentCloudServerEvent::SessionFinished => {
            *turn = SentenceContentState::default();
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Rejected);
                return true;
            }
            if !context.inner.is_closing.load(Ordering::SeqCst) {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: UNEXPECTED_SESSION_END_CODE.into(),
                        message: UNEXPECTED_SESSION_END_ERROR.into(),
                    },
                );
                return true;
            }
            emit_if_current(context, LiveTranslateServerEvent::SessionFinished);
            context
                .inner
                .received_session_finished
                .store(true, Ordering::SeqCst);
            context.inner.finish_notify.notify_waiters();
            return true;
        }
        TencentCloudServerEvent::ProviderError { code } => {
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Rejected);
            } else {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: format!("tencent_{code}"),
                        message: GENERIC_PROVIDER_ERROR.into(),
                    },
                );
            }
            return true;
        }
        TencentCloudServerEvent::Ignored { kind } => {
            emit_if_current(context, LiveTranslateServerEvent::Ignored { kind });
        }
    }
    false
}

fn fail_receive_loop(context: &ReceiveContext, code: &str, message: &str) {
    if *context.setup.borrow() == SetupState::Awaiting {
        let _ = context.setup.send(SetupState::Rejected);
    } else {
        emit_if_current(
            context,
            LiveTranslateServerEvent::Error {
                code: code.into(),
                message: message.into(),
            },
        );
    }
}

fn emit_if_current(context: &ReceiveContext, event: LiveTranslateServerEvent) {
    if context.inner.generation.load(Ordering::SeqCst) == context.generation {
        let _ = context.events.send(event);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn clear_observes_an_already_decoded_old_end_without_dropping_the_next_sentence() {
        let mut turn = SentenceContentState {
            in_progress: false,
            discard_until_end: true,
        };
        assert!(!turn.accepts_content(0, 1, false));
        assert!(turn.discard_until_end);
        assert!(!turn.accepts_content(0, 1, true));
        assert!(!turn.discard_until_end);
        assert!(turn.accepts_content(1, 1, false));
        assert!(turn.in_progress);
        assert!(turn.accepts_content(1, 1, true));
        assert!(!turn.in_progress);
    }

    #[tokio::test]
    async fn clear_ignores_current_sentence_tail_but_keeps_socket_and_next_sentence() {
        let (resume, resumed) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                socket.send(Message::Text(r#"{"code":0,"message":"success","voice_id":"v"}"#.into())).await.unwrap();
                socket.send(Message::Text(r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"old source","target_text":"old translation","sentence_end":false}}"#.into())).await.unwrap();
                resumed.await.unwrap();
                for frame in [r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"old source final","target_text":"old translation final","sentence_end":true}}"#, r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"new source","target_text":"new translation","sentence_end":false}}"#, r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"new source final","target_text":"new translation final","sentence_end":true}}"#] {
                    socket.send(Message::Text(frame.into())).await.unwrap();
                }
                while let Some(Ok(message)) = socket.next().await {
                    if matches!(&message, Message::Text(text) if text.contains(r#""type":"end""#)) {
                        socket.send(Message::Text(r#"{"code":0,"final":1}"#.into())).await.unwrap();
                        break;
                    }
                }
            })
        }).await;
        client
            .connect_with_timeout(Duration::from_secs(2))
            .await
            .unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap();
            if matches!(event, LiveTranslateServerEvent::SourceDraft { .. }) {
                break;
            }
        }
        let generation = client.inner.generation.load(Ordering::SeqCst);
        *client.inner.pending_audio.lock().await = vec![1, 2];
        assert_eq!(client.clear_content().await, 1);
        assert_eq!(client.content_revision(), 1);
        assert_eq!(client.inner.generation.load(Ordering::SeqCst), generation);
        assert!(client.inner.ready.load(Ordering::SeqCst));
        assert!(client.inner.sink.lock().await.is_some());
        assert_eq!(*client.inner.pending_audio.lock().await, vec![1, 2]);
        resume.send(()).unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap();
            match event {
                LiveTranslateServerEvent::SubtitleFinalPair {
                    source,
                    translation,
                    ..
                } => {
                    assert_eq!(source, "new source final");
                    assert_eq!(translation, "new translation final");
                    break;
                }
                LiveTranslateServerEvent::SourceDraft { text, .. }
                | LiveTranslateServerEvent::TranslationDraft(text) => {
                    assert!(!text.starts_with("old"));
                }
                LiveTranslateServerEvent::Error { code, .. } => {
                    panic!("unexpected fixed error: {code}")
                }
                _ => {}
            }
        }
        client.finish(Duration::from_secs(2)).await;
        assert_eq!(
            events.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        );
        client.disconnect().await;
    }

    use super::*;
    use crate::clients::provider_events::{provider_event_channel, ProviderEventReceiver};
    use tokio::net::TcpListener;

    async fn test_client(
        server: impl FnOnce(
                WebSocketStream<TcpStream>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + 'static,
    ) -> (TencentCloudClient, ProviderEventReceiver) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server(socket).await;
        });
        let (events, receiver) = provider_event_channel();
        let endpoint = url::Url::parse(&format!("ws://{address}/speech_translate")).unwrap();
        let client = TencentCloudClient::with_endpoint(
            SourceLanguage::Chinese,
            TargetLanguage::Japanese,
            events,
            endpoint,
        );
        (client, receiver)
    }

    #[test]
    fn audio_buffer_is_bounded_to_complete_frames_and_one_tail() {
        let mut pending = vec![7; TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT * 2 + 19];
        let frames = take_complete_frames(&mut pending);
        assert_eq!(frames.len(), 2);
        assert!(frames
            .iter()
            .all(|frame| frame.len() == TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT));
        assert_eq!(pending.len(), 19);
        let padded = take_padded_frame(&mut pending).unwrap();
        assert_eq!(padded.len(), TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT);
        assert_eq!(&padded[..19], &[7; 19]);
        assert!(padded[19..].iter().all(|byte| *byte == 0));
    }

    #[tokio::test]
    async fn mock_websocket_covers_setup_audio_ping_transcript_and_finish() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                socket
                    .send(Message::Text(
                        r#"{"code":0,"message":"success","voice_id":"v"}"#.into(),
                    ))
                    .await
                    .unwrap();
                let mut saw_audio = false;
                while let Some(Ok(message)) = socket.next().await {
                    match message {
                        Message::Binary(frame) => {
                            assert_eq!(frame.len(), TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT);
                            assert!(frame.iter().all(|byte| *byte == 7));
                            saw_audio = true;
                        }
                        Message::Ping(payload) => {
                            socket.send(Message::Pong(payload)).await.unwrap();
                        }
                        Message::Text(text) if text.contains(r#""type":"end""#) => {
                            assert!(saw_audio);
                            socket.send(Message::Text(
                                r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"你好","target_text":"こんにちは","sentence_end":false}}"#.into()
                            )).await.unwrap();
                            socket.send(Message::Text(
                                r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"你好。","target_text":"こんにちは。","sentence_end":true}}"#.into()
                            )).await.unwrap();
                            socket
                                .send(Message::Text(r#"{"code":0,"final":1}"#.into()))
                                .await
                                .unwrap();
                            break;
                        }
                        _ => {}
                    }
                }
            })
        })
        .await;

        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client
            .send_audio(&vec![7; TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        client.ping(Duration::from_millis(500)).await.unwrap();
        client.finish(Duration::from_millis(500)).await;

        let mut saw_pair = false;
        let mut saw_finished = false;
        for _ in 0..8 {
            let Ok(Some(event)) =
                tokio::time::timeout(Duration::from_millis(100), events.recv()).await
            else {
                break;
            };
            match event {
                LiveTranslateServerEvent::SubtitleFinalPair {
                    source,
                    language,
                    translation,
                } => {
                    assert_eq!(source, "你好。");
                    assert_eq!(language.as_deref(), Some("zh"));
                    assert_eq!(translation, "こんにちは。");
                    saw_pair = true;
                }
                LiveTranslateServerEvent::SessionFinished => saw_finished = true,
                LiveTranslateServerEvent::Error { code, .. } => {
                    panic!("unexpected error event: {code}")
                }
                _ => {}
            }
        }
        assert!(saw_pair);
        assert!(saw_finished);
    }

    #[tokio::test]
    async fn unsolicited_final_is_a_sanitized_error_not_a_clean_finish() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                socket
                    .send(Message::Text(
                        r#"{"code":0,"message":"success","voice_id":"v"}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(r#"{"code":0,"final":1}"#.into()))
                    .await
                    .unwrap();
            })
        })
        .await;

        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();

        let mut saw_error = false;
        for _ in 0..4 {
            let Ok(Some(event)) =
                tokio::time::timeout(Duration::from_millis(100), events.recv()).await
            else {
                break;
            };
            match event {
                LiveTranslateServerEvent::Error { code, message } => {
                    assert_eq!(code, UNEXPECTED_SESSION_END_CODE);
                    assert_eq!(message, UNEXPECTED_SESSION_END_ERROR);
                    saw_error = true;
                }
                LiveTranslateServerEvent::SessionFinished => {
                    panic!("unsolicited final must not look like a clean finish")
                }
                _ => {}
            }
        }
        assert!(saw_error);
    }

    #[tokio::test]
    async fn setup_timeout_is_finite() {
        let (client, _events) = test_client(|socket| {
            Box::pin(async move {
                let _keep_open = socket;
                tokio::time::sleep(Duration::from_secs(1)).await;
                drop(_keep_open);
            })
        })
        .await;
        assert_eq!(
            client
                .connect_with_timeout(Duration::from_millis(30))
                .await
                .unwrap_err(),
            TencentCloudClientError::SessionSetupTimedOut
        );
    }
}
