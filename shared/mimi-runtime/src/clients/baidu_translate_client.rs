//! Baidu Cloud realtime speech-translation WebSocket client.

use crate::clients::provider_events::ProviderEventSender;
use crate::core::models::{SourceLanguage, TargetLanguage};
use crate::core::protocols::baidu_translate::{
    BaiduTranslateEndpoint, BaiduTranslateRequestEncoder, BaiduTranslateServerEvent,
};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
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

struct Inner {
    content_lock: Mutex<SentenceContentState>,
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    audio_send_lock: Mutex<Option<tokio::time::Instant>>,
    pending_audio: Mutex<Vec<u8>>,
    ready: AtomicBool,
    is_closing: AtomicBool,
    received_session_finished: AtomicBool,
    finish_notify: Notify,
    pong_notify: Notify,
    generation: AtomicU64,
}

/// Credentials are intentionally stored in a non-`Debug` type. They are sent
/// only in the provider's required START frame and never included in errors.
#[derive(Clone)]
pub struct BaiduTranslateClient {
    network: super::provider_network::ProviderNetwork,
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
        network: super::provider_network::ProviderNetwork,
    ) -> Result<(), super::provider_network::ProviderNetworkError> {
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
            network: super::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                content_lock: Mutex::new(SentenceContentState::default()),
                sink: Mutex::new(None),
                receive_task: Mutex::new(None),
                audio_send_lock: Mutex::new(None),
                pending_audio: Mutex::new(Vec::new()),
                ready: AtomicBool::new(false),
                is_closing: AtomicBool::new(false),
                received_session_finished: AtomicBool::new(false),
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
            super::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| BaiduTranslateClientError::TransportFailure)?
        .map_err(|error| {
            if super::connection_diagnostics::authentication_rejected(&error) {
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
            if !self.inner.received_session_finished.load(Ordering::SeqCst) {
                finished.await;
            }
            Ok::<(), BaiduTranslateClientError>(())
        };
        match tokio::time::timeout(timeout, operation).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                self.emit_error("baidu_finish_failed", GENERIC_TRANSPORT_ERROR, generation)
            }
            Err(_) => self.emit_error(
                "baidu_finish_timeout",
                "Baidu realtime translation did not finish in time.",
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

    async fn send_paced_binary(
        &self,
        frame: Vec<u8>,
        next_send_at: &mut Option<tokio::time::Instant>,
        generation: u64,
    ) -> Result<(), BaiduTranslateClientError> {
        BaiduTranslateRequestEncoder::validate_audio_frame(&frame)
            .map_err(|_| BaiduTranslateClientError::TransportFailure)?;
        if let Some(deadline) = *next_send_at {
            tokio::time::sleep_until(deadline).await;
        }
        let mut sink = self.inner.sink.lock().await;
        if self.inner.generation.load(Ordering::SeqCst) != generation {
            return Err(BaiduTranslateClientError::NotConnected);
        }
        let Some(sink) = sink.as_mut() else {
            return Err(BaiduTranslateClientError::NotConnected);
        };
        let evidence =
            crate::development_audio::begin_pcm(&frame, BaiduTranslateEndpoint::SAMPLE_RATE_HZ);
        tokio::time::timeout(
            SEND_TIMEOUT,
            evidence.observe(sink.send(Message::Binary(frame.into()))),
        )
        .await
        .map_err(|_| BaiduTranslateClientError::TransportFailure)?
        .map_err(|_| BaiduTranslateClientError::TransportFailure)?;
        *next_send_at = Some(
            tokio::time::Instant::now()
                + Duration::from_millis(BaiduTranslateEndpoint::FRAME_DURATION_MS.into()),
        );
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

fn map_protocol_configuration_error(
    error: crate::core::protocols::baidu_translate::BaiduTranslateProtocolError,
) -> BaiduTranslateClientError {
    use crate::core::protocols::baidu_translate::BaiduTranslateProtocolError as ProtocolError;
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

async fn receive_loop(mut context: ReceiveContext) {
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
            emit_if_current(
                context,
                LiveTranslateServerEvent::Error {
                    code: if code == 20_314 {
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
mod tests {
    #[tokio::test]
    async fn handshake_authentication_rejections_preserve_transport_failures() {
        for (status, expected_auth_label) in
            crate::clients::connection_diagnostics::handshake_failure_cases()
        {
            let (endpoint, server) =
                crate::clients::connection_diagnostics::rejected_websocket_endpoint(status).await;
            let (events, _receiver) = provider_event_channel();
            let mut client = BaiduTranslateClient::with_endpoint(
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
                assert_eq!(error, BaiduTranslateClientError::AuthenticationFailed);
                assert_eq!(error.to_string(), expected_label);
            } else {
                assert_eq!(error, BaiduTranslateClientError::TransportFailure);
            }
            assert!(!error.to_string().contains("private-handshake-body"));
            assert!(!client.inner.ready.load(Ordering::SeqCst));
        }
    }

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
                let _ = socket.next().await;
                socket.send(Message::Text(r#"{"code":0,"msg":"Success","data":{"status":"STA"}}"#.into())).await.unwrap();
                socket.send(Message::Text(r#"{"code":0,"data":{"status":"TRN","result":{"type":"MID","asr":"old source","asr_trans":"old translation","sentence":"","sentence_trans":""}}}"#.into())).await.unwrap();
                resumed.await.unwrap();
                for frame in [r#"{"code":0,"data":{"status":"TRN","result":{"type":"FIN","asr":"","asr_trans":"","sentence":"old source final","sentence_trans":"old translation final"}}}"#, r#"{"code":0,"data":{"status":"TRN","result":{"type":"MID","asr":"new source","asr_trans":"new translation","sentence":"","sentence_trans":""}}}"#, r#"{"code":0,"data":{"status":"TRN","result":{"type":"FIN","asr":"","asr_trans":"","sentence":"new source final","sentence_trans":"new translation final"}}}"#] {
                    socket.send(Message::Text(frame.into())).await.unwrap();
                }
                while let Some(Ok(message)) = socket.next().await {
                    if matches!(&message, Message::Text(text) if text.contains(r#""type":"FINISH""#)) {
                        socket.send(Message::Text(r#"{"code":0,"msg":"Success","data":{"status":"END"}}"#.into())).await.unwrap();
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
    use serde_json::Value;
    use tokio::net::TcpListener;

    async fn test_client(
        server: impl FnOnce(
                WebSocketStream<TcpStream>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + 'static,
    ) -> (BaiduTranslateClient, ProviderEventReceiver) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server(socket).await;
        });
        let (events, receiver) = provider_event_channel();
        let endpoint = url::Url::parse(&format!("ws://{address}/realtime_speech_trans")).unwrap();
        let client = BaiduTranslateClient::with_endpoint(
            SourceLanguage::Japanese,
            TargetLanguage::English,
            events,
            endpoint,
        );
        (client, receiver)
    }

    #[test]
    fn audio_buffer_is_bounded_to_complete_frames_and_one_tail() {
        let mut pending = vec![9; BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT * 2 + 17];
        let frames = take_complete_frames(&mut pending);
        assert_eq!(frames.len(), 2);
        assert!(frames
            .iter()
            .all(|frame| frame.len() == BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT));
        assert_eq!(pending.len(), 17);
        let padded = take_padded_frame(&mut pending).unwrap();
        assert_eq!(padded.len(), BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT);
        assert_eq!(&padded[..17], &[9; 17]);
        assert!(padded[17..].iter().all(|byte| *byte == 0));
    }

    async fn next_event(events: &mut ProviderEventReceiver) -> LiveTranslateServerEvent {
        tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap()
    }

    fn transcript_frame(kind: &str, source: &str, translation: &str, next: &str) -> String {
        serde_json::json!({"code":0,"data":{"status":"TRN","result":{
            "type":kind,
            "asr": if kind == "MID" { source } else { next },
            "asr_trans": if kind == "MID" { translation } else { "" },
            "sentence": if kind == "FIN" { source } else { "" },
            "sentence_trans": if kind == "FIN" { translation } else { "" }
        }}})
        .to_string()
    }

    #[tokio::test]
    async fn fin_preserves_the_next_draft_and_clear_discards_its_tail() {
        for clear_previous in [false, true] {
            let (send, mut commands) = tokio::sync::mpsc::unbounded_channel::<Vec<String>>();
            let (client, mut events) = test_client(move |mut socket| {
                Box::pin(async move {
                    let _ = socket.next().await;
                    socket
                        .send(Message::Text(
                            r#"{"code":0,"data":{"status":"STA"}}"#.into(),
                        ))
                        .await
                        .unwrap();
                    while let Some(frames) = commands.recv().await {
                        if frames.is_empty() {
                            break;
                        }
                        for frame in frames {
                            socket.send(Message::Text(frame.into())).await.unwrap();
                        }
                    }
                    let finish = socket.next().await.unwrap().unwrap();
                    assert!(finish.to_text().unwrap().contains("FINISH"));
                    socket
                        .send(Message::Text(
                            r#"{"code":0,"data":{"status":"END"}}"#.into(),
                        ))
                        .await
                        .unwrap();
                })
            })
            .await;
            client.connect().await.unwrap();
            assert_eq!(
                next_event(&mut events).await,
                LiveTranslateServerEvent::SessionCreated
            );
            assert_eq!(
                next_event(&mut events).await,
                LiveTranslateServerEvent::SessionUpdated
            );
            send.send(vec![transcript_frame(
                "MID",
                "first draft",
                "first translation",
                "",
            )])
            .unwrap();
            assert!(matches!(
                next_event(&mut events).await,
                LiveTranslateServerEvent::SourceDraft { .. }
            ));
            assert!(matches!(
                next_event(&mut events).await,
                LiveTranslateServerEvent::TranslationDraft(_)
            ));
            send.send(vec![transcript_frame("MID", "", "", "")])
                .unwrap();
            assert!(
                matches!(next_event(&mut events).await, LiveTranslateServerEvent::SourceDraft { text, .. } if text.is_empty())
            );
            assert_eq!(
                next_event(&mut events).await,
                LiveTranslateServerEvent::TranslationDraft(String::new())
            );
            assert!(client.inner.content_lock.lock().await.in_progress);
            if clear_previous {
                client.clear_content().await;
            }
            send.send(vec![transcript_frame(
                "FIN",
                "first final",
                "first final translation",
                "next draft",
            )])
            .unwrap();
            if !clear_previous {
                assert!(
                    matches!(next_event(&mut events).await, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. }
                    if source == "first final" && translation == "first final translation")
                );
            }
            assert!(
                matches!(next_event(&mut events).await, LiveTranslateServerEvent::SourceDraft { text, .. } if text == "next draft")
            );
            assert_eq!(
                next_event(&mut events).await,
                LiveTranslateServerEvent::TranslationDraft(String::new())
            );
            assert!(client.inner.content_lock.lock().await.in_progress);
            client.clear_content().await;
            send.send(vec![
                transcript_frame("MID", "next cleared tail", "next cleared translation", ""),
                transcript_frame(
                    "FIN",
                    "next cleared final",
                    "next cleared final translation",
                    "",
                ),
                transcript_frame("FIN", "healthy final", "healthy translation", ""),
            ])
            .unwrap();
            assert!(
                matches!(next_event(&mut events).await, LiveTranslateServerEvent::SubtitleFinalPair { source, translation, .. }
                if source == "healthy final" && translation == "healthy translation")
            );
            send.send(Vec::new()).unwrap();
            client.finish(Duration::from_secs(2)).await;
            assert_eq!(
                next_event(&mut events).await,
                LiveTranslateServerEvent::SessionFinished
            );
            assert!(events.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn sentence_failure_releases_a_cleared_turn_and_idle_timeout_uses_recovery() {
        for (code, clear_previous) in [
            (20_311, true),
            (20_312, true),
            (20_315, true),
            (20_314, true),
            (20_312, false),
        ] {
            let (resume, resumed) = tokio::sync::oneshot::channel();
            let (healthy, healthy_allowed) = tokio::sync::oneshot::channel();
            let (client, mut events) = test_client(move |mut socket| {
                Box::pin(async move {
                    let _ = socket.next().await;
                    socket
                        .send(Message::Text(
                            r#"{"code":0,"data":{"status":"STA"}}"#.into(),
                        ))
                        .await
                        .unwrap();
                    socket
                        .send(Message::Text(
                            transcript_frame("MID", "obsolete", "obsolete translation", "").into(),
                        ))
                        .await
                        .unwrap();
                    resumed.await.unwrap();
                    socket
                        .send(Message::Text(
                            serde_json::json!({"code":code,"msg":"private provider content"})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                    if code == 20_314 {
                        return;
                    }
                    healthy_allowed.await.unwrap();
                    socket
                        .send(Message::Text(
                            transcript_frame("FIN", "healthy final", "healthy translation", "")
                                .into(),
                        ))
                        .await
                        .unwrap();
                    let _ = socket.next().await;
                    socket
                        .send(Message::Text(
                            r#"{"code":0,"data":{"status":"END"}}"#.into(),
                        ))
                        .await
                        .unwrap();
                })
            })
            .await;
            client.connect().await.unwrap();
            let mut controller = crate::core::session::TranslationSessionController::default();
            controller.handle(LiveTranslateServerEvent::SubtitleFinalPair {
                source: "retained final".into(),
                language: Some("ja".into()),
                translation: "retained translation".into(),
            });
            for _ in 0..4 {
                controller.handle(next_event(&mut events).await);
            }
            if clear_previous {
                client.clear_content().await;
                controller.clear_subtitles();
            }
            resume.send(()).unwrap();
            if code == 20_314 {
                let LiveTranslateServerEvent::Error { code, message } =
                    next_event(&mut events).await
                else {
                    panic!("expected timeout recovery");
                };
                assert!(crate::core::recovery::provider_error_is_retryable(&code));
                assert!(!message.contains("private"));
                client.disconnect().await;
            } else {
                for expected in [
                    LiveTranslateServerEvent::SubtitlePreviewCleared,
                    LiveTranslateServerEvent::SourceDraft {
                        text: String::new(),
                        language: Some("ja".into()),
                    },
                    LiveTranslateServerEvent::TranslationDraft(String::new()),
                ] {
                    let event = next_event(&mut events).await;
                    assert_eq!(event, expected);
                    controller.handle(event);
                }
                assert!(controller.state.subtitles.realtime_preview.is_none());
                if !clear_previous {
                    assert!(controller.state.subtitles.display_pair_final);
                    assert_eq!(
                        controller
                            .state
                            .subtitles
                            .display_pair
                            .as_ref()
                            .unwrap()
                            .translation,
                        "retained translation"
                    );
                }
                healthy.send(()).unwrap();
                assert!(
                    matches!(next_event(&mut events).await, LiveTranslateServerEvent::SubtitleFinalPair { source, .. } if source == "healthy final")
                );
                client.finish(Duration::from_secs(2)).await;
                assert_eq!(
                    next_event(&mut events).await,
                    LiveTranslateServerEvent::SessionFinished
                );
            }
        }
    }

    #[tokio::test]
    async fn batched_audio_is_paced_and_finish_flushes_one_padded_tail() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"code":0,"data":{"status":"STA"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                for index in 0..4 {
                    let frame = socket.next().await.unwrap().unwrap().into_data();
                    assert_eq!(frame.len(), BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT);
                    if index == 3 {
                        assert_eq!(&frame[..2], &[9, 9]);
                        assert!(frame[2..].iter().all(|byte| *byte == 0));
                    } else {
                        assert!(frame.iter().all(|byte| *byte == 9));
                    }
                }
                assert!(socket
                    .next()
                    .await
                    .unwrap()
                    .unwrap()
                    .to_text()
                    .unwrap()
                    .contains("FINISH"));
                socket
                    .send(Message::Text(
                        r#"{"code":0,"data":{"status":"END"}}"#.into(),
                    ))
                    .await
                    .unwrap();
            })
        })
        .await;
        client.connect().await.unwrap();
        let stale_generation = client.inner.generation.load(Ordering::SeqCst) - 1;
        assert_eq!(
            client
                .send_text(
                    BaiduTranslateRequestEncoder::finish().to_string(),
                    stale_generation
                )
                .await,
            Err(BaiduTranslateClientError::NotConnected)
        );
        assert_eq!(
            client
                .send_paced_binary(
                    vec![0; BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT],
                    &mut None,
                    stale_generation
                )
                .await,
            Err(BaiduTranslateClientError::NotConnected)
        );
        let started = tokio::time::Instant::now();
        client
            .send_audio(&vec![
                9;
                BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT * 3 + 2
            ])
            .await
            .unwrap();
        client.finish(Duration::from_secs(2)).await;
        assert!(started.elapsed() >= Duration::from_millis(120));
        assert_eq!(
            next_event(&mut events).await,
            LiveTranslateServerEvent::SessionCreated
        );
        assert_eq!(
            next_event(&mut events).await,
            LiveTranslateServerEvent::SessionUpdated
        );
        assert_eq!(
            next_event(&mut events).await,
            LiveTranslateServerEvent::SessionFinished
        );
        assert!(client.inner.pending_audio.lock().await.is_empty());
    }

    #[tokio::test]
    async fn mock_websocket_covers_start_audio_ping_transcript_and_finish() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let start = socket.next().await.unwrap().unwrap();
                let start: Value = serde_json::from_str(start.to_text().unwrap()).unwrap();
                assert_eq!(start["type"], "START");
                assert_eq!(start["from"], "jp");
                assert_eq!(start["to"], "en");
                assert_eq!(start["sampling_rate"], 16_000);
                socket
                    .send(Message::Text(
                        r#"{"code":0,"msg":"Success","data":{"status":"STA"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                let mut saw_audio = false;
                while let Some(Ok(message)) = socket.next().await {
                    match message {
                        Message::Binary(frame) => {
                            assert_eq!(frame.len(), BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT);
                            assert!(frame.iter().all(|byte| *byte == 9));
                            saw_audio = true;
                        }
                        Message::Ping(payload) => {
                            socket.send(Message::Pong(payload)).await.unwrap();
                        }
                        Message::Text(text) if text.contains(r#""type":"FINISH""#) => {
                            assert!(saw_audio);
                            socket.send(Message::Text(
                                r#"{"code":0,"data":{"status":"TRN","result":{"type":"MID","asr":"こんにちは","asr_trans":"Hello","sentence":"","sentence_trans":""}}}"#.into()
                            )).await.unwrap();
                            socket.send(Message::Text(
                                r#"{"code":0,"data":{"status":"TRN","result":{"type":"FIN","asr":"","asr_trans":"","sentence":"こんにちは。","sentence_trans":"Hello."}}}"#.into()
                            )).await.unwrap();
                            socket.send(Message::Text(
                                r#"{"code":0,"msg":"Success","data":{"status":"END"}}"#.into()
                            )).await.unwrap();
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
            .send_audio(&vec![9; BaiduTranslateEndpoint::AUDIO_FRAME_BYTE_COUNT])
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
                    assert_eq!(source, "こんにちは。");
                    assert_eq!(language.as_deref(), Some("ja"));
                    assert_eq!(translation, "Hello.");
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
    async fn unsolicited_end_is_a_sanitized_error_not_a_clean_finish() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"code":0,"msg":"Success","data":{"status":"STA"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"code":0,"msg":"private provider detail","data":{"status":"END"}}"#
                            .into(),
                    ))
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
                    assert!(!message.contains("private provider detail"));
                    saw_error = true;
                }
                LiveTranslateServerEvent::SessionFinished => {
                    panic!("unsolicited END must not look like a clean finish")
                }
                _ => {}
            }
        }
        assert!(saw_error);
    }

    #[tokio::test]
    async fn recoverable_sentence_error_keeps_the_stream_alive() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"code":0,"data":{"status":"STA"}}"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"{"code":20312,"msg":"private provider detail"}"#.into(),
                    ))
                    .await
                    .unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if message.to_text().is_ok_and(|text| text.contains("FINISH")) {
                        socket
                            .send(Message::Text(
                                r#"{"code":0,"data":{"status":"END"}}"#.into(),
                            ))
                            .await
                            .unwrap();
                        break;
                    }
                }
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.finish(Duration::from_millis(500)).await;
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(50), events.recv()).await
        {
            assert!(!matches!(event, LiveTranslateServerEvent::Error { .. }));
            if matches!(event, LiveTranslateServerEvent::SessionFinished) {
                return;
            }
        }
        panic!("missing SessionFinished");
    }

    #[tokio::test]
    async fn baidu_setup_preserves_explicit_rejection_and_transport_categories() {
        use crate::clients::translation_client::ConnectError;
        for (frame, expected, safe_code) in [
            (
                None,
                BaiduTranslateClientError::TransportFailure,
                "transport_error",
            ),
            (
                Some(r#"{"code":0,"data":{"status":"END"}}"#),
                BaiduTranslateClientError::TransportFailure,
                "transport_error",
            ),
            (
                Some(r#"{"code":20314,"msg":"private provider detail"}"#),
                BaiduTranslateClientError::SessionSetupRejectedWithCode(20_314),
                "transport_error",
            ),
            (
                Some(r#"{"code":20316,"msg":"private provider detail"}"#),
                BaiduTranslateClientError::SessionSetupRejectedWithCode(20_316),
                "provider_error",
            ),
            (
                Some(r#"{"code":99999,"msg":"private provider detail"}"#),
                BaiduTranslateClientError::SessionSetupRejectedWithCode(99_999),
                "provider_error",
            ),
            (
                Some("invalid-json"),
                BaiduTranslateClientError::SessionSetupRejected,
                "provider_error",
            ),
        ] {
            let (client, mut events) = test_client(move |mut socket| {
                Box::pin(async move {
                    let _ = socket.next().await.unwrap().unwrap();
                    if let Some(frame) = frame {
                        socket.send(Message::Text(frame.into())).await.unwrap();
                        while socket.next().await.is_some() {}
                    } else {
                        socket.close(None).await.unwrap();
                    }
                })
            })
            .await;
            let error = client
                .connect_with_timeout(Duration::from_millis(500))
                .await
                .unwrap_err();
            assert_eq!(error, expected, "frame={frame:?}");
            assert!(!error.to_string().contains("private provider detail"));
            assert_eq!(ConnectError::BaiduTranslate(error).safe_code(), safe_code);
            assert!(!client.inner.ready.load(Ordering::SeqCst));
            assert!(events.try_recv().is_err());
            client.disconnect().await;
        }
    }

    #[tokio::test]
    async fn setup_timeout_is_finite() {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                tokio::time::sleep(Duration::from_secs(1)).await;
            })
        })
        .await;
        assert_eq!(
            client
                .connect_with_timeout(Duration::from_millis(30))
                .await
                .unwrap_err(),
            BaiduTranslateClientError::SessionSetupTimedOut
        );
    }

    #[tokio::test]
    async fn setup_rejection_preserves_only_the_provider_error_code() {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"code":31003,"msg":"private credential detail"}"#.into(),
                    ))
                    .await
                    .unwrap();
            })
        })
        .await;
        let error = client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            BaiduTranslateClientError::SessionSetupRejectedWithCode(31_003)
        );
        assert!(!error.to_string().contains("private credential detail"));
    }

    #[tokio::test]
    async fn independent_review_fin_retains_its_next_draft() {
        let (publish, mut commands) = tokio::sync::mpsc::unbounded_channel::<String>();
        let (client, mut events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        r#"{"code":0,"data":{"status":"STA"}}"#.into(),
                    ))
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
        publish.send(r#"{"code":0,"data":{"status":"TRN","result":{"type":"FIN","sentence":"Synthetic A final","sentence_trans":"Translated A final","asr":"Synthetic B draft","asr_trans":"Translated B draft"}}}"#.into()).unwrap();
        let mut controller = crate::core::session::TranslationSessionController::default();
        tokio::time::timeout(Duration::from_secs(2), async {
            while controller.state.subtitles.history.is_empty() {
                controller.handle(events.recv().await.unwrap());
            }
        })
        .await
        .unwrap();
        while let Ok(event) = events.try_recv() {
            controller.handle(event);
        }
        assert_eq!(
            controller.state.subtitles.history[0].source,
            "Synthetic A final"
        );
        assert_eq!(
            controller.state.subtitles.history[0].translation,
            "Translated A final"
        );
        assert_eq!(
            controller.state.subtitles.source.text, "Synthetic B draft",
            "FIN must carry the next sentence independently"
        );
        assert_eq!(
            controller.state.subtitles.translation.text,
            "Translated B draft"
        );
        publish.send(String::new()).unwrap();
        client.disconnect().await;
    }
}
