//! Baidu Cloud realtime speech-translation WebSocket client.

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::realtime::baidu_translate::{
    BaiduTranslateEndpoint, BaiduTranslateRequestEncoder, BaiduTranslateServerEvent,
};
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(250);
const MAXIMUM_AUDIO_CHUNK_BYTES: usize = BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT * 64;
const GENERIC_PROVIDER_ERROR: &str = "Baidu realtime translation rejected the session.";
const GENERIC_PROTOCOL_ERROR: &str = "Baidu realtime translation returned an invalid response.";
const GENERIC_TRANSPORT_ERROR: &str = "The Baidu realtime translation connection failed.";
const UNEXPECTED_SESSION_END_CODE: &str = "baidu_unexpected_session_end";
const UNEXPECTED_SESSION_END_ERROR: &str =
    "Baidu realtime translation ended the session unexpectedly.";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BaiduTranslateClientError {
    #[error("credential_authentication_failed")]
    AuthenticationFailed,
    #[error("Add a Baidu Cloud AppID and AppKey in Settings.")]
    MissingCredentials,
    #[error("Baidu realtime translation requires an explicit supported source language.")]
    InvalidSourceLanguage,
    #[error("Baidu realtime translation requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("The Baidu realtime translation session is not connected.")]
    NotConnected,
    #[error("The Baidu realtime translation connection stopped responding.")]
    HealthCheckTimedOut,
    #[error("The Baidu realtime translation connection failed.")]
    TransportFailure,
    #[error("Baidu realtime translation rejected the session configuration.")]
    SessionSetupRejected,
    #[error("Baidu realtime translation rejected the session configuration (code {0}).")]
    SessionSetupRejectedWithCode(i64),
    #[error("Baidu realtime translation did not confirm the session in time.")]
    SessionSetupTimedOut,
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type Stream = futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected(BaiduTranslateClientError),
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

// A server rejection can arrive just after speech resumes. Retain recent
// delivered silence briefly without changing ordinary invalid-audio failures.
#[derive(Default)]
struct DeliveredSilence {
    continuous_ms: u64,
    last_qualifying_frame: Option<tokio::time::Instant>,
}

impl DeliveredSilence {
    fn observe(&mut self, silent: bool, now: tokio::time::Instant) {
        if silent {
            self.continuous_ms = self
                .continuous_ms
                .saturating_add(u64::from(BaiduTranslateEndpoint::FRAME_DURATION_MS));
            if self.continuous_ms >= 20_000 {
                self.last_qualifying_frame = Some(now);
            }
        } else {
            self.continuous_ms = 0;
        }
    }

    fn qualifies(&self, now: tokio::time::Instant) -> bool {
        self.last_qualifying_frame
            .is_some_and(|last| now.saturating_duration_since(last) <= Duration::from_secs(5))
    }
}

struct Inner {
    content_lock: Mutex<SentenceContentState>,
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    audio_send_lock: Mutex<Option<tokio::time::Instant>>,
    pending_audio: Mutex<Vec<u8>>,
    delivered_silence: Mutex<DeliveredSilence>,
    ready: AtomicBool,
    is_closing: AtomicBool,
    received_session_finished: AtomicBool,
    receive_ended: AtomicBool,
    finish_notify: Notify,
    pong_notify: Notify,
    generation: AtomicU64,
}

/// Credentials are intentionally stored in a non-`Debug` type. They are sent
/// only in the provider's required START frame and never included in errors.
#[derive(Clone)]
pub struct BaiduTranslateClient {
    network: crate::clients::transport::provider_network::ProviderNetwork,
    inner: Arc<Inner>,
    endpoint: url::Url,
    app_id: String,
    app_key: String,
    source_language: SourceLanguage,
    target_language: TargetLanguage,
    events: ProviderEventSender,
}

impl BaiduTranslateClient {
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.clone()
    }

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
        network: crate::clients::transport::provider_network::ProviderNetwork,
    ) -> Result<(), crate::clients::transport::provider_network::ProviderNetworkError> {
        self.network = network;
        Ok(())
    }

    pub fn new(
        app_id: &str,
        app_key: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        events: ProviderEventSender,
    ) -> Result<Self, BaiduTranslateClientError> {
        BaiduTranslateRequestEncoder::start(app_id, app_key, source_language, target_language)
            .map_err(map_protocol_configuration_error)?;
        let endpoint = BaiduTranslateEndpoint::url()
            .map_err(|_| BaiduTranslateClientError::TransportFailure)?;
        Ok(Self::from_parts(
            endpoint,
            app_id,
            app_key,
            source_language,
            target_language,
            events,
        ))
    }

    fn from_parts(
        endpoint: url::Url,
        app_id: &str,
        app_key: &str,
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
                audio_send_lock: Mutex::new(None),
                pending_audio: Mutex::new(Vec::new()),
                delivered_silence: Mutex::new(DeliveredSilence::default()),
                ready: AtomicBool::new(false),
                is_closing: AtomicBool::new(false),
                received_session_finished: AtomicBool::new(false),
                receive_ended: AtomicBool::new(false),
                finish_notify: Notify::new(),
                pong_notify: Notify::new(),
                generation: AtomicU64::new(0),
            }),
            endpoint,
            app_id: app_id.trim().to_string(),
            app_key: app_key.trim().to_string(),
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
            endpoint,
            "test-app-id",
            "test-app-key-not-real",
            source_language,
            target_language,
            events,
        )
    }

    pub async fn connect(&self) -> Result<(), BaiduTranslateClientError> {
        self.connect_with_timeout(Duration::from_secs(5)).await
    }

    async fn connect_with_timeout(
        &self,
        readiness_timeout: Duration,
    ) -> Result<(), BaiduTranslateClientError> {
        self.disconnect().await;
        *self.inner.content_lock.lock().await = SentenceContentState::default();
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let request = self
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|_| BaiduTranslateClientError::TransportFailure)?;
        let (socket, _) = tokio::time::timeout(
            CONNECT_TIMEOUT,
            crate::clients::transport::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| BaiduTranslateClientError::TransportFailure)?
        .map_err(|error| {
            if crate::clients::transport::connection_diagnostics::authentication_rejected(&error) {
                BaiduTranslateClientError::AuthenticationFailed
            } else {
                BaiduTranslateClientError::TransportFailure
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
        *self.inner.audio_send_lock.lock().await = None;

        let (setup_tx, mut setup_rx) = watch::channel(SetupState::Awaiting);
        let task = tokio::spawn(receive_loop(ReceiveContext {
            inner: Arc::clone(&self.inner),
            stream,
            events: self.events.clone(),
            setup: setup_tx,
            generation,
            normalized_source_language: self.source_language.raw_value().to_string(),
        }));
        *self.inner.receive_task.lock().await = Some(task);

        let start = BaiduTranslateRequestEncoder::start(
            &self.app_id,
            &self.app_key,
            self.source_language,
            self.target_language,
        )
        .map_err(map_protocol_configuration_error)?;
        let wait_for_ready = async {
            self.send_text(start.to_string(), generation).await?;
            loop {
                match setup_rx.borrow().clone() {
                    SetupState::Ready => return Ok(()),
                    SetupState::Rejected(error) => return Err(error),
                    SetupState::Awaiting => {}
                }
                if setup_rx.changed().await.is_err() {
                    return Err(BaiduTranslateClientError::TransportFailure);
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
                Err(BaiduTranslateClientError::SessionSetupTimedOut)
            }
        }
    }

    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), BaiduTranslateClientError> {
        if pcm_data.is_empty() {
            return Ok(());
        }
        if pcm_data.len() > MAXIMUM_AUDIO_CHUNK_BYTES {
            return Err(BaiduTranslateClientError::TransportFailure);
        }
        tokio::time::timeout(SEND_TIMEOUT, self.send_audio_operation(pcm_data))
            .await
            .map_err(|_| BaiduTranslateClientError::TransportFailure)?
    }

    async fn send_audio_operation(&self, pcm_data: &[u8]) -> Result<(), BaiduTranslateClientError> {
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let mut next_send_at = self.inner.audio_send_lock.lock().await;
        if !self.inner.ready.load(Ordering::SeqCst)
            || self.inner.is_closing.load(Ordering::SeqCst)
            || self.inner.generation.load(Ordering::SeqCst) != generation
        {
            return Err(BaiduTranslateClientError::NotConnected);
        }
        let frames = {
            let mut pending = self.inner.pending_audio.lock().await;
            pending.extend_from_slice(pcm_data);
            take_complete_frames(&mut pending)
        };
        for frame in frames {
            self.send_paced_binary(frame, &mut next_send_at, generation)
                .await?;
        }
        Ok(())
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), BaiduTranslateClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(BaiduTranslateClientError::NotConnected);
        }
        let operation = async {
            let pong = self.inner.pong_notify.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(BaiduTranslateClientError::NotConnected);
                };
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| BaiduTranslateClientError::TransportFailure)?;
            }
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| BaiduTranslateClientError::HealthCheckTimedOut)?
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
            let mut next_send_at = self.inner.audio_send_lock.lock().await;
            let final_frame = {
                let mut pending = self.inner.pending_audio.lock().await;
                take_padded_frame(&mut pending)
            };
            if let Some(frame) = final_frame {
                self.send_paced_binary(frame, &mut next_send_at, generation)
                    .await?;
            }
            self.send_text(
                BaiduTranslateRequestEncoder::finish().to_string(),
                generation,
            )
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
                return Err(BaiduTranslateClientError::TransportFailure);
            }
            Ok::<(), BaiduTranslateClientError>(())
        };
        match tokio::time::timeout(timeout, operation).await {
            Ok(Ok(())) => {}
            Ok(Err(_))
                if self.inner.generation.load(Ordering::SeqCst) == generation
                    && !self.inner.receive_ended.load(Ordering::SeqCst) =>
            {
                self.emit_error("baidu_finish_failed", GENERIC_TRANSPORT_ERROR, generation)
            }
            Err(_)
                if self.inner.generation.load(Ordering::SeqCst) == generation
                    && !self.inner.receive_ended.load(Ordering::SeqCst) =>
            {
                self.emit_error(
                    "baidu_finish_timeout",
                    "Baidu realtime translation did not finish in time.",
                    generation,
                )
            }
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
        *self.inner.delivered_silence.lock().await = DeliveredSilence::default();
    }

    async fn send_paced_binary(
        &self,
        frame: Vec<u8>,
        next_send_at: &mut Option<tokio::time::Instant>,
        generation: u64,
    ) -> Result<(), BaiduTranslateClientError> {
        BaiduTranslateRequestEncoder::validate_audio_frame(&frame)
            .map_err(|_| BaiduTranslateClientError::TransportFailure)?;
        let scheduled_at = next_send_at.unwrap_or_else(tokio::time::Instant::now);
        tokio::time::sleep_until(scheduled_at).await;
        let mut sink = self.inner.sink.lock().await;
        if self.inner.generation.load(Ordering::SeqCst) != generation {
            return Err(BaiduTranslateClientError::NotConnected);
        }
        let Some(sink) = sink.as_mut() else {
            return Err(BaiduTranslateClientError::NotConnected);
        };
        let evidence =
            crate::development_audio::begin_pcm(&frame, BaiduTranslateEndpoint::SAMPLE_RATE_HZ);
        let silent = frame.iter().all(|byte| *byte == 0);
        tokio::time::timeout(
            SEND_TIMEOUT,
            evidence.observe(sink.send(Message::Binary(frame.into()))),
        )
        .await
        .map_err(|_| BaiduTranslateClientError::TransportFailure)?
        .map_err(|_| BaiduTranslateClientError::TransportFailure)?;
        self.inner
            .delivered_silence
            .lock()
            .await
            .observe(silent, tokio::time::Instant::now());
        *next_send_at = Some(next_audio_frame_deadline(
            scheduled_at,
            tokio::time::Instant::now(),
        ));
        Ok(())
    }

    async fn send_text(
        &self,
        text: String,
        generation: u64,
    ) -> Result<(), BaiduTranslateClientError> {
        let mut sink = self.inner.sink.lock().await;
        if self.inner.generation.load(Ordering::SeqCst) != generation {
            return Err(BaiduTranslateClientError::NotConnected);
        }
        let Some(sink) = sink.as_mut() else {
            return Err(BaiduTranslateClientError::NotConnected);
        };
        tokio::time::timeout(SEND_TIMEOUT, sink.send(Message::Text(text.into())))
            .await
            .map_err(|_| BaiduTranslateClientError::TransportFailure)?
            .map_err(|_| BaiduTranslateClientError::TransportFailure)
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

fn next_audio_frame_deadline(
    scheduled_at: tokio::time::Instant,
    completed_at: tokio::time::Instant,
) -> tokio::time::Instant {
    // Preserve the 40-ms audio clock instead of adding every scheduler/write
    // delay to it. Do not accumulate catch-up credit after a long network stall.
    (scheduled_at + Duration::from_millis(BaiduTranslateEndpoint::FRAME_DURATION_MS.into()))
        .max(completed_at)
}

fn map_protocol_configuration_error(
    error: crate::core::protocols::realtime::baidu_translate::BaiduTranslateProtocolError,
) -> BaiduTranslateClientError {
    use crate::core::protocols::realtime::baidu_translate::BaiduTranslateProtocolError as ProtocolError;
    match error {
        ProtocolError::MissingAppID | ProtocolError::MissingAppKey => {
            BaiduTranslateClientError::MissingCredentials
        }
        ProtocolError::InvalidSourceLanguage => BaiduTranslateClientError::InvalidSourceLanguage,
        ProtocolError::InvalidTargetLanguage => BaiduTranslateClientError::InvalidTargetLanguage,
        _ => BaiduTranslateClientError::TransportFailure,
    }
}

fn take_complete_frames(pending: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let frame_size = BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT;
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
    frame.resize(BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT, 0);
    Some(frame)
}

struct ReceiveContext {
    inner: Arc<Inner>,
    stream: Stream,
    events: ProviderEventSender,
    setup: watch::Sender<SetupState>,
    generation: u64,
    normalized_source_language: String,
}

struct ReceiveEndedGuard {
    inner: Arc<Inner>,
    generation: u64,
}

impl Drop for ReceiveEndedGuard {
    fn drop(&mut self) {
        if self.inner.generation.load(Ordering::SeqCst) == self.generation {
            self.inner.receive_ended.store(true, Ordering::SeqCst);
            self.inner.finish_notify.notify_waiters();
        }
    }
}

async fn receive_loop(mut context: ReceiveContext) {
    let _receive_ended = ReceiveEndedGuard {
        inner: Arc::clone(&context.inner),
        generation: context.generation,
    };
    while let Some(message) = context.stream.next().await {
        if context.inner.generation.load(Ordering::SeqCst) != context.generation {
            return;
        }
        let event = match message {
            Ok(Message::Text(text)) => BaiduTranslateServerEvent::decode(&text),
            Ok(Message::Pong(_)) => {
                context.inner.pong_notify.notify_waiters();
                continue;
            }
            // TTS is disabled in START, so unexpected binary output is
            // discarded immediately and never retained.
            Ok(Message::Binary(_)) | Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => {
                fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR);
                return;
            }
        };
        let event = match event {
            Ok(event) => event,
            Err(_) => {
                fail_receive_loop(&context, "baidu_protocol_error", GENERIC_PROTOCOL_ERROR);
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

async fn handle_server_event(context: &ReceiveContext, event: BaiduTranslateServerEvent) -> bool {
    let revision = context.events.content_revision();
    let mut turn = context.inner.content_lock.lock().await;
    if context.inner.generation.load(Ordering::SeqCst) != context.generation {
        return false;
    }

    match event {
        BaiduTranslateServerEvent::SessionReady => {
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Ready);
                emit_if_current(context, LiveTranslateServerEvent::SessionCreated);
                emit_if_current(context, LiveTranslateServerEvent::SessionUpdated);
            }
        }
        BaiduTranslateServerEvent::TranscriptDraft {
            source_text,
            target_text,
        } => {
            let in_progress = turn.in_progress;
            if !turn.accepts_content(revision, context.events.content_revision(), false) {
                return false;
            }
            // An empty MID is not a sentence boundary: a subsequent clear
            // still excludes the tail of a sentence that already began.
            turn.in_progress =
                in_progress || !source_text.trim().is_empty() || !target_text.trim().is_empty();
            emit_draft(context, source_text, target_text);
        }
        BaiduTranslateServerEvent::TranscriptFinal {
            source_text,
            target_text,
            next_source_text,
            next_target_text,
        } => {
            let current_revision = context.events.content_revision();
            let accepts_final = turn.accepts_content(revision, current_revision, true);
            if accepts_final && !source_text.trim().is_empty() && !target_text.trim().is_empty() {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::SubtitleFinalPair {
                        source: source_text.trim().to_string(),
                        language: Some(context.normalized_source_language.clone()),
                        translation: target_text.trim().to_string(),
                    },
                );
            } else if accepts_final {
                clear_draft(context);
            }
            // Consume the previous sentence's boundary even after a clear,
            // then begin the next sentence independently. Never attach its
            // ASR text to the previous confirmed translation.
            if !next_source_text.trim().is_empty() || !next_target_text.trim().is_empty() {
                turn.in_progress = true;
                if revision == current_revision {
                    emit_draft(context, next_source_text, next_target_text);
                } else {
                    // A clear raced with this decoded packet. Its next draft
                    // is obsolete too, so exclude that sentence's tail.
                    turn.discard_until_end = true;
                }
            }
        }
        BaiduTranslateServerEvent::SessionFinished => {
            *turn = SentenceContentState::default();
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Rejected(
                    BaiduTranslateClientError::TransportFailure,
                ));
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
        BaiduTranslateServerEvent::ProviderError {
            code,
            is_recoverable,
        } => {
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Rejected(
                    BaiduTranslateClientError::SessionSetupRejectedWithCode(code),
                ));
                return true;
            }
            if is_recoverable {
                // Official sentence-local failures do not invalidate the
                // stream. Do not turn them into a session-fatal Error event.
                // These errors end the failed sentence without a FIN; clear
                // must not discard the next healthy sentence waiting for it.
                if matches!(code, 20_311 | 20_312 | 20_315) {
                    *turn = SentenceContentState::default();
                    clear_draft(context);
                }
                return false;
            }
            // Production replays showed 20316 followed by END after 25 seconds
            // of valid zero PCM. Recover only with delivered-silence evidence;
            // malformed audio and setup rejection retain their fatal meaning.
            let rejected_idle_audio = code == 20_316
                && context
                    .inner
                    .delivered_silence
                    .lock()
                    .await
                    .qualifies(tokio::time::Instant::now());
            emit_if_current(
                context,
                LiveTranslateServerEvent::Error {
                    code: if code == 20_314 || rejected_idle_audio {
                        // The documented idle disconnect uses the existing
                        // bounded transport recovery policy.
                        "transport_error".into()
                    } else {
                        format!("baidu_provider_{code}")
                    },
                    message: GENERIC_PROVIDER_ERROR.into(),
                },
            );
            return true;
        }
        BaiduTranslateServerEvent::Ignored { kind } => {
            emit_if_current(context, LiveTranslateServerEvent::Ignored { kind });
        }
    }
    false
}

fn clear_draft(context: &ReceiveContext) {
    emit_if_current(context, LiveTranslateServerEvent::SubtitlePreviewCleared);
    emit_draft(context, String::new(), String::new());
}

fn emit_draft(context: &ReceiveContext, source_text: String, target_text: String) {
    emit_if_current(
        context,
        LiveTranslateServerEvent::SourceDraft {
            text: source_text,
            language: Some(context.normalized_source_language.clone()),
        },
    );
    emit_if_current(
        context,
        LiveTranslateServerEvent::TranslationDraft(target_text),
    );
}

fn fail_receive_loop(context: &ReceiveContext, code: &str, message: &str) {
    if *context.setup.borrow() == SetupState::Awaiting {
        let failure = if code == "transport_error" {
            BaiduTranslateClientError::TransportFailure
        } else {
            BaiduTranslateClientError::SessionSetupRejected
        };
        let _ = context.setup.send(SetupState::Rejected(failure));
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
mod tests;
