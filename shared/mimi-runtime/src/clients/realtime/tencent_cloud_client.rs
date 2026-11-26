//! Tencent Cloud realtime speech-translation WebSocket client.

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::realtime::tencent_cloud::{
    TencentCloudEndpoint, TencentCloudRequestEncoder, TencentCloudSentence, TencentCloudServerEvent,
};
use futures_util::{SinkExt, StreamExt};
use std::collections::VecDeque;
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
const GENERIC_PROTOCOL_ERROR: &str =
    "Tencent Cloud realtime translation returned an invalid response.";
const GENERIC_TRANSPORT_ERROR: &str = "The Tencent Cloud realtime translation connection failed.";
const UNEXPECTED_SESSION_END_CODE: &str = "tencent_unexpected_session_end";
const UNEXPECTED_SESSION_END_ERROR: &str =
    "Tencent Cloud realtime translation ended the session unexpectedly.";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TencentCloudClientError {
    #[error("credential_authentication_failed")]
    AuthenticationFailed,
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
    #[error("{}", provider_rejection_label(*code))]
    ProviderRejected { code: i64 },
    #[error("Tencent Cloud realtime translation did not confirm the session in time.")]
    SessionSetupTimedOut,
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type Stream = futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected(Option<i64>),
}

const MAX_TRACKED_SENTENCES: usize = 64;

struct TrackedSentence {
    sentence: TencentCloudSentence,
    completed: bool,
    discarded: bool,
}

#[derive(Default)]
struct SentenceContentState {
    sentences: VecDeque<TrackedSentence>,
}

impl SentenceContentState {
    fn contains(&self, id: &str) -> bool {
        self.sentences
            .iter()
            .any(|tracked| tracked.sentence.id == id)
    }

    fn clear(&mut self) {
        // Every already-observed pending sentence belongs to the cleared
        // content revision. Keep only metadata so a late tail stays rejected.
        for tracked in &mut self.sentences {
            tracked.discarded = true;
        }
    }

    fn accepts_content(
        &mut self,
        expected: u64,
        current: u64,
        sentence: TencentCloudSentence,
        sentence_end: bool,
    ) -> bool {
        let latest_start = self
            .sentences
            .back()
            .map(|tracked| tracked.sentence.start_time_ms);
        if let Some(tracked) = self
            .sentences
            .iter_mut()
            .find(|tracked| tracked.sentence.id == sentence.id)
        {
            if tracked.completed
                || tracked.sentence.start_time_ms != sentence.start_time_ms
                || sentence.end_time_ms < tracked.sentence.end_time_ms
                || (!sentence_end
                    && latest_start.is_some_and(|latest| sentence.start_time_ms < latest))
            {
                return false;
            }
            tracked.sentence = sentence;
            tracked.completed = sentence_end;
            tracked.discarded |= expected != current;
            return !tracked.discarded;
        }
        // Once an old ID has left the bounded window it cannot become a new
        // sentence again. Known late finals remain reliable confirmations.
        if latest_start.is_some_and(|latest| sentence.start_time_ms <= latest) {
            return false;
        }
        self.sentences.push_back(TrackedSentence {
            sentence,
            completed: sentence_end,
            discarded: expected != current,
        });
        while self.sentences.len() > MAX_TRACKED_SENTENCES {
            self.sentences.pop_front();
        }
        expected == current
    }
}

struct Inner {
    content_lock: Mutex<SentenceContentState>,
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    audio_send_lock: Mutex<()>,
    pending_audio: Mutex<Vec<u8>>,
    next_audio_frame_at: Mutex<Option<tokio::time::Instant>>,
    ready: AtomicBool,
    is_closing: AtomicBool,
    received_session_finished: AtomicBool,
    receive_ended: AtomicBool,
    finish_notify: Notify,
    pong_notify: Notify,
    generation: AtomicU64,
}

/// Secret fields intentionally live in a non-`Debug` client and are never
/// interpolated into an error or diagnostic event.
#[derive(Clone)]
pub struct TencentCloudClient {
    network: crate::clients::transport::provider_network::ProviderNetwork,
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
    pub(crate) fn network_endpoint(&self) -> Result<url::Url, TencentCloudClientError> {
        self.endpoint_for_connection()
    }

    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    pub async fn clear_content(&self) -> u64 {
        let mut turn = self.inner.content_lock.lock().await;
        turn.clear();
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
            network: crate::clients::transport::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                content_lock: Mutex::new(SentenceContentState::default()),
                sink: Mutex::new(None),
                receive_task: Mutex::new(None),
                audio_send_lock: Mutex::new(()),
                pending_audio: Mutex::new(Vec::new()),
                next_audio_frame_at: Mutex::new(None),
                ready: AtomicBool::new(false),
                is_closing: AtomicBool::new(false),
                received_session_finished: AtomicBool::new(false),
                receive_ended: AtomicBool::new(false),
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
            crate::clients::transport::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| TencentCloudClientError::TransportFailure)?
        .map_err(|error| {
            if crate::clients::transport::connection_diagnostics::authentication_rejected(&error) {
                TencentCloudClientError::AuthenticationFailed
            } else {
                TencentCloudClientError::TransportFailure
            }
        })?;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .received_session_finished
            .store(false, Ordering::SeqCst);
        self.inner.receive_ended.store(false, Ordering::SeqCst);
        self.inner.pending_audio.lock().await.clear();
        *self.inner.next_audio_frame_at.lock().await = None;

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
                    SetupState::Rejected(code) => {
                        return Err(match code {
                            Some(code) => TencentCloudClientError::ProviderRejected { code },
                            None => TencentCloudClientError::SessionSetupRejected,
                        })
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
                // Readiness is published by the receive task. Do not revive
                // a socket that ended immediately after its handshake.
                if self.inner.receive_ended.load(Ordering::SeqCst) {
                    self.disconnect().await;
                    Err(TencentCloudClientError::SessionSetupRejected)
                } else {
                    Ok(())
                }
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
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let _send_guard = self.inner.audio_send_lock.lock().await;
        if self.inner.generation.load(Ordering::SeqCst) != generation
            || !self.inner.ready.load(Ordering::SeqCst)
            || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(TencentCloudClientError::NotConnected);
        }
        let frames = {
            let mut pending = self.inner.pending_audio.lock().await;
            if self.inner.generation.load(Ordering::SeqCst) != generation
                || !self.inner.ready.load(Ordering::SeqCst)
                || self.inner.is_closing.load(Ordering::SeqCst)
            {
                return Err(TencentCloudClientError::NotConnected);
            }
            pending.extend_from_slice(pcm_data);
            take_complete_frames(&mut pending)
        };
        for frame in frames {
            self.send_binary(frame, generation).await?;
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
            if self.inner.generation.load(Ordering::SeqCst) != generation
                || !self.inner.ready.load(Ordering::SeqCst)
            {
                return Err(TencentCloudClientError::NotConnected);
            }
            let final_frame = {
                let mut pending = self.inner.pending_audio.lock().await;
                if self.inner.generation.load(Ordering::SeqCst) != generation
                    || !self.inner.ready.load(Ordering::SeqCst)
                {
                    return Err(TencentCloudClientError::NotConnected);
                }
                take_padded_frame(&mut pending)
            };
            if let Some(frame) = final_frame {
                self.send_binary(frame, generation).await?;
            }
            self.send_text(TencentCloudRequestEncoder::finish().to_string(), generation)
                .await?;
            let finished = self.inner.finish_notify.notified();
            tokio::pin!(finished);
            finished.as_mut().enable();
            if !self.inner.received_session_finished.load(Ordering::SeqCst)
                && !self.inner.receive_ended.load(Ordering::SeqCst)
            {
                finished.await;
            }
            if !self.inner.received_session_finished.load(Ordering::SeqCst) {
                return Err(TencentCloudClientError::NotConnected);
            }
            Ok::<(), TencentCloudClientError>(())
        };
        match tokio::time::timeout(timeout, operation).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) if !self.inner.receive_ended.load(Ordering::SeqCst) => {
                self.emit_error("tencent_finish_failed", GENERIC_TRANSPORT_ERROR, generation)
            }
            Err(_) if !self.inner.receive_ended.load(Ordering::SeqCst) => self.emit_error(
                "tencent_finish_timeout",
                "Tencent Cloud realtime translation did not finish in time.",
                generation,
            ),
            _ => {}
        }
        self.disconnect_if_current(generation).await;
    }

    pub async fn disconnect(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.receive_ended.store(true, Ordering::SeqCst);
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

    async fn send_binary(
        &self,
        frame: Vec<u8>,
        generation: u64,
    ) -> Result<(), TencentCloudClientError> {
        TencentCloudRequestEncoder::validate_audio_frame(&frame)
            .map_err(|_| TencentCloudClientError::TransportFailure)?;
        // Capture normally supplies one frame at a time. Preserve its audio
        // clock without accumulating overdue deadlines after a slow write.
        let mut next_frame = self.inner.next_audio_frame_at.lock().await;
        let scheduled_at = match *next_frame {
            Some(deadline) => {
                tokio::time::sleep_until(deadline).await;
                deadline
            }
            None => tokio::time::Instant::now(),
        };
        let mut sink = self.inner.sink.lock().await;
        if self.inner.generation.load(Ordering::SeqCst) != generation
            || !self.inner.ready.load(Ordering::SeqCst)
        {
            return Err(TencentCloudClientError::NotConnected);
        }
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
        .map_err(|_| TencentCloudClientError::TransportFailure)?;
        *next_frame = Some(next_audio_frame_deadline(
            scheduled_at,
            tokio::time::Instant::now(),
        ));
        Ok(())
    }

    async fn send_text(
        &self,
        text: String,
        generation: u64,
    ) -> Result<(), TencentCloudClientError> {
        let mut sink = self.inner.sink.lock().await;
        if self.inner.generation.load(Ordering::SeqCst) != generation
            || !self.inner.ready.load(Ordering::SeqCst)
        {
            return Err(TencentCloudClientError::NotConnected);
        }
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
    error: crate::core::protocols::realtime::tencent_cloud::TencentCloudProtocolError,
) -> TencentCloudClientError {
    use crate::core::protocols::realtime::tencent_cloud::TencentCloudProtocolError as ProtocolError;
    match error {
        ProtocolError::MissingAppID
        | ProtocolError::MissingSecretID
        | ProtocolError::MissingSecretKey
        | ProtocolError::InvalidSessionIdentity => TencentCloudClientError::MissingCredentials,
        ProtocolError::InvalidTargetLanguage => TencentCloudClientError::InvalidTargetLanguage,
        _ => TencentCloudClientError::TransportFailure,
    }
}

/// Fixed labels from the speech-translation API's status table. Never expose
/// its free-form message: it can include request details or user content.
fn provider_rejection_label(code: i64) -> &'static str {
    match code {
        6001 => "tencent_configuration_rejected",
        6002 => "credential_authentication_failed",
        6003 => "tencent_service_activation_required",
        6004 | 6005 => "tencent_quota_exhausted",
        6006 => "tencent_capacity_exceeded",
        _ => "tencent_provider_rejected",
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
    loop {
        let Some(message) = context.stream.next().await else {
            fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR);
            break;
        };
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
                break;
            }
        };
        let event = match event {
            Ok(event) => event,
            Err(_) => {
                fail_receive_loop(&context, "tencent_protocol_error", GENERIC_PROTOCOL_ERROR);
                break;
            }
        };
        if handle_server_event(&context, event).await {
            break;
        }
    }
    // Take only this generation's sink. A delayed old receive task must never
    // close a replacement connection. Keep the close bounded even if the peer
    // ignores the closing handshake.
    if context.inner.generation.load(Ordering::SeqCst) != context.generation {
        return;
    }
    context.inner.ready.store(false, Ordering::SeqCst);
    context.inner.receive_ended.store(true, Ordering::SeqCst);
    context.inner.finish_notify.notify_waiters();
    let close = async {
        let mut sink = context.inner.sink.lock().await;
        if context.inner.generation.load(Ordering::SeqCst) != context.generation {
            return;
        }
        if let Some(mut sink) = sink.take() {
            let _ = sink.close().await;
        }
    };
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, close).await;
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
                context.inner.ready.store(true, Ordering::SeqCst);
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
            sentence,
        } => {
            let utterance_id = sentence.id.clone();
            let registered = turn.contains(&utterance_id);
            if !turn.accepts_content(
                revision,
                context.events.content_revision(),
                sentence,
                sentence_end,
            ) {
                return false;
            }
            if !registered {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::UtteranceRegistered {
                        utterance_id: utterance_id.clone(),
                    },
                );
            }
            if sentence_end {
                if !source_text.trim().is_empty() && !target_text.trim().is_empty() {
                    emit_if_current(
                        context,
                        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                            utterance_id,
                            source: source_text.trim().to_string(),
                            language: Some(source_language),
                            translation: target_text.trim().to_string(),
                        },
                    );
                }
            } else {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair {
                        utterance_id,
                        source: source_text,
                        language: Some(source_language),
                        translation: target_text,
                    },
                );
            }
        }
        TencentCloudServerEvent::SessionFinished => {
            *turn = SentenceContentState::default();
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Rejected(None));
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
                let _ = context.setup.send(SetupState::Rejected(Some(code)));
            } else {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: format!("tencent_provider_{code}"),
                        message: provider_rejection_label(code).into(),
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
        let _ = context.setup.send(SetupState::Rejected(None));
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

/// Keeps a fixed 200-ms audio clock anchored to the previous deadline.
///
/// Anchoring to `now` would add every timer overshoot (1-14 ms per frame on
/// Windows) to the cadence, so the sender falls behind realtime, the bounded
/// capture queue overflows and the session reconnects. A late completion makes
/// only the next frame immediately due; later frames resume the 200-ms clock.
fn next_audio_frame_deadline(
    scheduled_at: tokio::time::Instant,
    completed_at: tokio::time::Instant,
) -> tokio::time::Instant {
    (scheduled_at + Duration::from_millis(TencentCloudEndpoint::FRAME_DURATION_MS.into()))
        .max(completed_at)
}

#[cfg(test)]
mod tests;
