//! Tencent Cloud realtime speech-translation WebSocket client.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::tencent_cloud::{
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
            super::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| TencentCloudClientError::TransportFailure)?
        .map_err(|error| {
            if super::connection_diagnostics::authentication_rejected(&error) {
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
mod tests {
    #[tokio::test]
    async fn setup_rejections_preserve_status_without_provider_message_content() {
        let contract: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../translation-contracts.json"
        )))
        .unwrap();
        for case in contract["tencentSpeechTranslation"]["errors"]
            .as_array()
            .unwrap()
        {
            let code = case["code"].as_i64().unwrap();
            let expected_label = case["label"].as_str().unwrap();
            let (client, _events) = test_client(move |mut socket| {
                Box::pin(async move {
                    let rejected = serde_json::json!({
                        "code": code,
                        "message": "private-provider-response-secret",
                    });
                    socket
                        .send(Message::Text(rejected.to_string().into()))
                        .await
                        .unwrap();
                })
            })
            .await;
            let error = client
                .connect_with_timeout(Duration::from_secs(2))
                .await
                .unwrap_err();
            assert_eq!(error, TencentCloudClientError::ProviderRejected { code });
            assert_eq!(error.to_string(), expected_label);
            assert!(!format!("{error:?}").contains("private-provider-response-secret"));
            assert!(!client.inner.ready.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn active_session_rejection_keeps_code_and_a_fixed_safe_label() {
        let (reject, rejected) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                socket
                    .send(Message::Text(
                        r#"{"code":0,"message":"success","voice_id":"v"}"#.into(),
                    ))
                    .await
                    .unwrap();
                rejected.await.unwrap();
                socket
                    .send(Message::Text(
                        r#"{"code":6005,"message":"private-provider-response-secret"}"#.into(),
                    ))
                    .await
                    .unwrap();
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_secs(2))
            .await
            .unwrap();
        reject.send(()).unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let LiveTranslateServerEvent::Error { code, message } = event {
                assert_eq!(code, "tencent_provider_6005");
                assert_eq!(message, "tencent_quota_exhausted");
                break;
            }
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while !client.inner.receive_ended.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            client.send_audio(&vec![0; 6400]).await,
            Err(TencentCloudClientError::NotConnected)
        );
        assert_eq!(
            client.ping(Duration::from_millis(50)).await,
            Err(TencentCloudClientError::NotConnected)
        );
        client.disconnect().await;
    }

    #[tokio::test]
    async fn rejection_during_finish_closes_without_a_second_timeout_error() {
        let (closed, close_observation) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                socket
                    .send(Message::Text(r#"{"code":0}"#.into()))
                    .await
                    .unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if matches!(&message, Message::Text(text) if text.contains(r#""type":"end""#)) {
                        socket
                            .send(Message::Text(
                                r#"{"code":6004,"message":"private provider detail"}"#.into(),
                            ))
                            .await
                            .unwrap();
                    }
                    if matches!(message, Message::Close(_)) {
                        closed.send(()).unwrap();
                        return;
                    }
                }
                panic!("client did not close its rejected session");
            })
        })
        .await;
        client.connect().await.unwrap();
        tokio::time::timeout(
            Duration::from_millis(500),
            client.finish(Duration::from_secs(2)),
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(1), close_observation)
            .await
            .unwrap()
            .unwrap();
        let mut errors = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let LiveTranslateServerEvent::Error { code, message } = event {
                errors.push((code, message));
            }
        }
        assert_eq!(
            errors,
            vec![(
                "tencent_provider_6004".into(),
                "tencent_quota_exhausted".into()
            )]
        );
        assert!(!client.inner.ready.load(Ordering::SeqCst));
        assert!(client.inner.sink.lock().await.is_none());
    }

    #[test]
    fn audio_frame_deadlines_do_not_accumulate_timer_overshoot() {
        let frame = Duration::from_millis(TencentCloudEndpoint::FRAME_DURATION_MS.into());
        let start = tokio::time::Instant::now();
        let mut scheduled = start;
        // Windows timers wake 1-14 ms late; 120 frames is 24 s of audio, about
        // when the relative cadence used to overflow the 20-frame queue.
        for index in 0..120u32 {
            let overshoot = Duration::from_millis(u64::from(1 + index % 14));
            let woke_at = scheduled + overshoot;
            scheduled = next_audio_frame_deadline(scheduled, woke_at);
            assert_eq!(scheduled, start + frame * (index + 1));
        }
    }

    #[test]
    fn a_late_audio_frame_is_sent_at_once_without_catch_up_credit() {
        let frame = Duration::from_millis(TencentCloudEndpoint::FRAME_DURATION_MS.into());
        let scheduled = tokio::time::Instant::now();
        let after_stall = scheduled + Duration::from_secs(2);
        let next = next_audio_frame_deadline(scheduled, after_stall);
        assert_eq!(next, after_stall);
        assert_eq!(next_audio_frame_deadline(next, next), next + frame);
    }

    #[tokio::test]
    async fn queued_pcm_frames_are_paced_at_the_documented_realtime_rate() {
        let (observed, observation) = tokio::sync::oneshot::channel();
        let (client, _events) = test_client(move |mut socket| {
            Box::pin(async move {
                socket
                    .send(Message::Text(r#"{"code":0}"#.into()))
                    .await
                    .unwrap();
                let mut frames = Vec::new();
                while let Some(Ok(message)) = socket.next().await {
                    if let Message::Binary(frame) = message {
                        assert_eq!(frame.len(), TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT);
                        frames.push(tokio::time::Instant::now());
                        if frames.len() == 3 {
                            observed.send(frames).unwrap();
                            return;
                        }
                    }
                }
                panic!("missing paced audio frames");
            })
        })
        .await;
        client.connect().await.unwrap();
        client
            .send_audio(&vec![0; TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT * 3])
            .await
            .unwrap();
        let frames = tokio::time::timeout(Duration::from_secs(2), observation)
            .await
            .unwrap()
            .unwrap();
        // Allow small local scheduling/arrival jitter without allowing a burst.
        assert!(frames
            .windows(2)
            .all(|pair| pair[1].duration_since(pair[0]) >= Duration::from_millis(180)));
        client.disconnect().await;
    }

    #[tokio::test]
    async fn a_blocked_socket_write_does_not_leave_an_extra_catch_up_frame() {
        let test_started = tokio::time::Instant::now();
        let (resume_reads, reads_resumed) = tokio::sync::oneshot::channel();
        let (observed, observation) = tokio::sync::oneshot::channel();
        const PRIMER_BYTES: usize = 16 * 1024 * 1024;
        let (client, _events) = test_client(move |mut socket| {
            Box::pin(async move {
                tokio::time::timeout(Duration::from_secs(5), async {
                    socket
                        .send(Message::Text(r#"{"code":0}"#.into()))
                        .await
                        .unwrap();
                    reads_resumed.await.unwrap();
                    let mut frames = 0;
                    let mut primer_decoded_at = None;
                    while let Some(Ok(message)) = socket.next().await {
                        if let Message::Binary(frame) = message {
                            if frame.len() == PRIMER_BYTES {
                                primer_decoded_at = Some(test_started.elapsed());
                                continue;
                            }
                            assert_eq!(frame.len(), TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT);
                            frames += 1;
                            if frames == 3 {
                                observed.send(primer_decoded_at.unwrap()).unwrap();
                                return;
                            }
                        }
                    }
                    panic!("missing paced frames after blocked write");
                })
                .await
                .unwrap();
            })
        })
        .await;
        client.connect().await.unwrap();
        // Fill the actual local WebSocket's write buffer while its peer is not
        // reading. This fixture-only message makes the first production send
        // await socket progress without adding a production delay/test hook.
        tokio::time::timeout(Duration::from_secs(2), async {
            client
                .inner
                .sink
                .lock()
                .await
                .as_mut()
                .unwrap()
                .feed(Message::Binary(vec![0; PRIMER_BYTES].into()))
                .await
                .unwrap();
        })
        .await
        .unwrap();
        let audio = vec![0; TencentCloudEndpoint::AUDIO_FRAME_BYTE_COUNT];
        let send = async {
            let mut completed = Vec::new();
            for _ in 0..3 {
                let previous_deadline = *client.inner.next_audio_frame_at.lock().await;
                let started_at = test_started.elapsed();
                client.send_audio(&audio).await?;
                let completed_at = tokio::time::Instant::now();
                let next_deadline = *client.inner.next_audio_frame_at.lock().await;
                eprintln!(
                    "frame={} started={started_at:?} completed={:?} previous_deadline={:?} next_deadline={:?}",
                    completed.len() + 1,
                    completed_at.duration_since(test_started),
                    previous_deadline.map(|at| at.duration_since(test_started)),
                    next_deadline.map(|at| at.duration_since(test_started)),
                );
                completed.push(completed_at);
            }
            Ok::<_, TencentCloudClientError>(completed)
        };
        tokio::pin!(send);
        tokio::select! {
            result = &mut send => panic!("socket did not stall: {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(500)) => {},
        }
        eprintln!("reader_resumed={:?}", test_started.elapsed());
        resume_reads.send(()).unwrap();
        let completed = tokio::time::timeout(Duration::from_secs(2), send)
            .await
            .unwrap()
            .unwrap();
        let primer_decoded_at = tokio::time::timeout(Duration::from_secs(2), observation)
            .await
            .unwrap()
            .unwrap();
        eprintln!("primer_decoded={primer_decoded_at:?}");
        client.disconnect().await;
        // The stalled completion can be followed by one immediately due frame;
        // the next frame must return to the 200-ms clock instead of bursting too.
        // Observe completed writes so parsing the large fixture message cannot
        // turn delayed server reads into a false burst report.
        let final_gap = completed[2].duration_since(completed[1]);
        assert!(
            final_gap >= Duration::from_millis(180),
            "third frame followed the second after {final_gap:?}"
        );
    }

    #[tokio::test]
    async fn handshake_authentication_rejections_preserve_transport_failures() {
        for (status, expected_auth_label) in
            crate::clients::connection_diagnostics::handshake_failure_cases()
        {
            let (endpoint, server) =
                crate::clients::connection_diagnostics::rejected_websocket_endpoint(status).await;
            let (events, _receiver) = provider_event_channel();
            let mut client = TencentCloudClient::with_endpoint(
                SourceLanguage::Japanese,
                TargetLanguage::English,
                events,
                endpoint,
            );
            client.network = super::super::provider_network::ProviderNetwork::resolve(
                &crate::core::network_proxy::ProxyConfig {
                    mode: crate::core::network_proxy::ProxyMode::Direct,
                    url: None,
                },
            )
            .unwrap();
            let error = client.connect().await.unwrap_err();
            server.await.unwrap();
            if let Some(expected_label) = expected_auth_label {
                assert_eq!(error, TencentCloudClientError::AuthenticationFailed);
                assert_eq!(error.to_string(), expected_label);
            } else {
                assert_eq!(error, TencentCloudClientError::TransportFailure);
            }
            assert!(!error.to_string().contains("private-handshake-body"));
            assert!(!client.inner.ready.load(Ordering::SeqCst));
        }
    }

    fn sentence(id: &str, start: u64, end: u64) -> TencentCloudSentence {
        TencentCloudSentence {
            id: id.into(),
            start_time_ms: start,
            end_time_ms: end,
        }
    }

    #[test]
    fn clear_observes_an_already_decoded_old_end_without_dropping_the_next_sentence() {
        let mut turn = SentenceContentState::default();
        assert!(turn.accepts_content(0, 0, sentence("A", 0, 800), false));
        turn.clear();
        assert!(!turn.accepts_content(0, 1, sentence("A", 0, 900), false));
        assert!(!turn.accepts_content(0, 1, sentence("A", 0, 1000), true));
        assert!(turn.accepts_content(1, 1, sentence("B", 1000, 1800), false));
        assert!(turn.accepts_content(1, 1, sentence("B", 1000, 2000), true));
    }

    #[test]
    fn sentence_metadata_rejects_old_drafts_duplicates_and_time_regressions() {
        let mut turn = SentenceContentState::default();
        assert!(turn.accepts_content(0, 0, sentence("A", 0, 800), false));
        assert!(!turn.accepts_content(0, 0, sentence("A", 0, 700), false));
        assert!(!turn.accepts_content(0, 0, sentence("A", 1, 900), true));
        assert!(turn.accepts_content(0, 0, sentence("B", 1000, 1800), false));
        assert!(!turn.accepts_content(0, 0, sentence("A", 0, 900), false));
        // A's first complete pair remains durable, without taking over B.
        assert!(turn.accepts_content(0, 0, sentence("A", 0, 1000), true));
        assert!(!turn.accepts_content(0, 0, sentence("A", 0, 1000), true));
        assert!(turn.accepts_content(0, 0, sentence("B", 1000, 2000), true));
        assert!(!turn.accepts_content(0, 0, sentence("B", 1000, 2100), false));
        assert!(!turn.accepts_content(0, 0, sentence("unseen-old", 500, 700), true));
    }

    #[test]
    fn clear_does_not_wait_for_a_missing_final_and_metadata_stays_bounded() {
        let mut turn = SentenceContentState::default();
        assert!(turn.accepts_content(0, 0, sentence("A", 0, 800), false));
        turn.clear();
        assert!(turn.accepts_content(1, 1, sentence("B", 1000, 1800), false));
        assert!(!turn.accepts_content(1, 1, sentence("A", 0, 1000), true));
        for index in 2..1000 {
            assert!(turn.accepts_content(
                1,
                1,
                sentence(&index.to_string(), index * 1000, index * 1000 + 800),
                true
            ));
        }
        assert_eq!(turn.sentences.len(), MAX_TRACKED_SENTENCES);
        assert!(!turn.accepts_content(1, 1, sentence("A", 0, 1000), true));
    }

    #[tokio::test]
    async fn clear_ignores_current_sentence_tail_but_keeps_socket_and_next_sentence() {
        let (resume, resumed) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                socket.send(Message::Text(r#"{"code":0,"message":"success","voice_id":"v"}"#.into())).await.unwrap();
                socket.send(Message::Text(r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"old source","target_text":"old translation","sentence_end":false,"start_time":0,"end_time":800},"sentence_id":"old-sentence"}"#.into())).await.unwrap();
                resumed.await.unwrap();
                for frame in [r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"old source final","target_text":"old translation final","sentence_end":true,"start_time":0,"end_time":800},"sentence_id":"old-sentence"}"#, r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"new source","target_text":"new translation","sentence_end":false,"start_time":1000,"end_time":1800},"sentence_id":"new-sentence"}"#, r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"new source final","target_text":"new translation final","sentence_end":true,"start_time":1000,"end_time":1800},"sentence_id":"new-sentence"}"#] {
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
            if matches!(
                event,
                LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair { .. }
            ) {
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
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                    source,
                    translation,
                    ..
                } => {
                    assert_eq!(source, "new source final");
                    assert_eq!(translation, "new translation final");
                    break;
                }
                LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair {
                    source,
                    translation,
                    ..
                } => {
                    assert!(!source.starts_with("old"));
                    assert!(!translation.starts_with("old"));
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
    async fn zero_final_handshake_is_ready_before_audio() {
        let (observed_audio, audio_observation) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                socket
                    .send(Message::Text(
                        r#"{"code":0,"message":"success","voice_id":"fixture-voice","final":0}"#
                            .into(),
                    ))
                    .await
                    .unwrap();
                let mut saw_audio = false;
                while let Some(Ok(message)) = socket.next().await {
                    saw_audio |= matches!(message, Message::Binary(_));
                    if matches!(message, Message::Close(_)) {
                        break;
                    }
                }
                observed_audio.send(saw_audio).unwrap();
            })
        })
        .await;

        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        assert!(client.inner.ready.load(Ordering::SeqCst));
        for expected in [
            LiveTranslateServerEvent::SessionCreated,
            LiveTranslateServerEvent::SessionUpdated,
        ] {
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), events.recv())
                    .await
                    .unwrap(),
                Some(expected)
            );
        }
        client.disconnect().await;
        assert!(
            !tokio::time::timeout(Duration::from_secs(1), audio_observation)
                .await
                .unwrap()
                .unwrap()
        );
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
                                r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"你好","target_text":"こんにちは","sentence_end":false,"start_time":0,"end_time":800},"sentence_id":"old-sentence"}"#.into()
                            )).await.unwrap();
                            socket.send(Message::Text(
                                r#"{"code":0,"result":{"source":"zh","target":"ja","source_text":"你好。","target_text":"こんにちは。","sentence_end":true,"start_time":0,"end_time":800},"sentence_id":"old-sentence"}"#.into()
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
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                    source,
                    language,
                    translation,
                    ..
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
    async fn wire_sentence_sequence_preserves_current_pairs_and_distinct_confirmations() {
        let (publish, publication) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let frame =
                    |id: &str, start: u64, end: u64, text: &str, final_sentence: bool| {
                        Message::Text(serde_json::json!({
                        "code": 0, "sentence_id": id,
                        "result": {
                            "source": "en", "target": "zh",
                            "source_text": text, "target_text": format!("Translated {text}"),
                            "start_time": start, "end_time": end, "sentence_end": final_sentence
                        }
                    }).to_string().into())
                    };
                socket
                    .send(Message::Text(r#"{"code":0}"#.into()))
                    .await
                    .unwrap();
                publication.await.unwrap();
                for message in [
                    frame("A", 0, 500, "Synthetic A draft", false),
                    frame("B", 1000, 1500, "Synthetic B draft", false),
                    frame("A", 0, 800, "Synthetic A final", true),
                    frame("A", 0, 800, "Synthetic A duplicate", true),
                    frame("A", 0, 900, "Synthetic A stale", false),
                ] {
                    socket.send(message).await.unwrap();
                }
                while let Some(Ok(message)) = socket.next().await {
                    if matches!(&message, Message::Text(text) if text.contains(r#""type":"end""#)) {
                        for message in [
                            frame("B", 1000, 1800, "Synthetic repeated final", true),
                            frame("B", 1000, 1900, "Synthetic B stale", false),
                            frame("C", 2000, 2800, "Synthetic repeated final", true),
                            frame("C", 2000, 2800, "Synthetic repeated final", true),
                            Message::Text(r#"{"code":0,"final":1}"#.into()),
                        ] {
                            socket.send(message).await.unwrap();
                        }
                        return;
                    }
                }
                panic!("missing end message");
            })
        })
        .await;
        let mut controller = crate::core::session::TranslationSessionController::default();
        client.connect().await.unwrap();
        publish.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            let mut saw_current_preview = false;
            loop {
                let event = events.recv().await.unwrap();
                saw_current_preview |= matches!(&event, LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair { utterance_id, .. } if utterance_id == "B");
                controller.handle(event);
                if saw_current_preview && !controller.state.subtitles.history.is_empty() { break; }
            }
        }).await.unwrap();
        assert_eq!(controller.state.subtitles.history.len(), 1);
        assert_eq!(
            controller.state.subtitles.history[0].source,
            "Synthetic A final"
        );
        assert_eq!(controller.state.subtitles.source.text, "Synthetic B draft");
        assert_eq!(
            controller.state.subtitles.translation.text,
            "Translated Synthetic B draft"
        );
        client.finish(Duration::from_secs(1)).await;
        while let Ok(event) = events.try_recv() {
            controller.handle(event);
        }
        assert_eq!(controller.state.subtitles.history.len(), 3);
        assert_eq!(
            controller.state.subtitles.history[1].source,
            "Synthetic repeated final"
        );
        assert_eq!(
            controller.state.subtitles.history[2].source,
            "Synthetic repeated final"
        );
        assert_eq!(
            controller.state.subtitles.source.utterance_id.as_deref(),
            Some("C")
        );
        assert!(controller.state.subtitles.source.is_final);
        assert!(controller.state.subtitles.realtime_preview.is_none());
    }

    #[tokio::test]
    async fn unsolicited_final_is_a_sanitized_error_not_a_clean_finish() {
        let (end, ended) = tokio::sync::oneshot::channel();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                socket
                    .send(Message::Text(
                        r#"{"code":0,"message":"success","voice_id":"v"}"#.into(),
                    ))
                    .await
                    .unwrap();
                ended.await.unwrap();
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
        end.send(()).unwrap();

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
    async fn a_terminal_immediately_after_handshake_cannot_revive_readiness() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                for frame in [r#"{"code":0}"#, r#"{"code":6005}"#] {
                    socket.send(Message::Text(frame.into())).await.unwrap();
                }
                while let Some(Ok(message)) = socket.next().await {
                    if matches!(message, Message::Close(_)) {
                        break;
                    }
                }
            })
        })
        .await;
        // Depending on task scheduling, readiness can precede the error or
        // connect can already report rejection. Neither may revive the socket.
        let result = client.connect().await;
        assert!(result.is_ok() || result == Err(TencentCloudClientError::SessionSetupRejected));
        tokio::time::timeout(Duration::from_secs(1), async {
            while !client.inner.receive_ended.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!client.inner.ready.load(Ordering::SeqCst));
        assert_eq!(
            client.send_audio(&vec![0; 6400]).await,
            Err(TencentCloudClientError::NotConnected)
        );
        let mut errors = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let LiveTranslateServerEvent::Error { code, .. } = event {
                errors.push(code);
            }
        }
        assert_eq!(errors, vec!["tencent_provider_6005"]);
        client.disconnect().await;
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

    #[tokio::test]
    async fn independent_review_late_final_keeps_newer_sentence_pair() {
        let (publish, mut commands) = tokio::sync::mpsc::unbounded_channel::<String>();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                socket
                    .send(Message::Text(r#"{"code":0}"#.into()))
                    .await
                    .unwrap();
                while let Some(frame) = commands.recv().await {
                    if frame.is_empty() {
                        break;
                    }
                    socket.send(Message::Text(frame.into())).await.unwrap();
                }
            })
        })
        .await;
        client.connect().await.unwrap();
        let mut controller = crate::core::session::TranslationSessionController::default();
        for (id, start, text) in [("A", 0, "Synthetic A"), ("B", 1000, "Synthetic B")] {
            publish.send(serde_json::json!({"code":0,"sentence_id":id,"result":{
                "source":"en","target":"zh","source_text":text,"target_text":format!("Translated {text}"),
                "start_time":start,"end_time":start+500,"sentence_end":false
            }}).to_string()).unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                while controller.state.subtitles.source.text != text
                    || controller.state.subtitles.translation.text != format!("Translated {text}")
                {
                    controller.handle(events.recv().await.unwrap());
                }
            })
            .await
            .unwrap();
        }
        publish.send(r#"{"code":0,"sentence_id":"A","result":{"source":"en","target":"zh","source_text":"Synthetic A final","target_text":"Translated A final","start_time":0,"end_time":800,"sentence_end":true}}"#.into()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while controller.state.subtitles.history.is_empty() {
                controller.handle(events.recv().await.unwrap());
            }
        })
        .await
        .unwrap();
        assert_eq!(
            controller.state.subtitles.history[0].source,
            "Synthetic A final"
        );
        assert_eq!(
            controller.state.subtitles.source.text, "Synthetic B",
            "A late final must not replace B's current source"
        );
        assert_eq!(
            controller.state.subtitles.translation.text,
            "Translated Synthetic B"
        );
        publish.send(String::new()).unwrap();
        client.disconnect().await;
    }
}
