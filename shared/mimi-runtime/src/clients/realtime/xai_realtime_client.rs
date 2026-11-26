//! xAI Grok Voice WebSocket adapter for turn-based translation.

mod turn_state;

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::core::models::TargetLanguage;
#[cfg(test)]
use crate::core::models::UtteranceRole;
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::realtime::xai_realtime::{
    XAIRealtimeEndpoint, XAIRealtimeRequestEncoder, XAIRealtimeServerEvent,
};
use futures_util::{SinkExt, StreamExt};
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
use turn_state::GrokTurnState;
#[cfg(test)]
#[allow(unused_imports)]
use turn_state::MAXIMUM_TRANSCRIPT_BYTES;

const GENERIC_PROVIDER_ERROR: &str = "xAI Grok Voice rejected the session.";
const GENERIC_PROTOCOL_ERROR: &str = "xAI Grok Voice returned an invalid response.";
const GENERIC_TRANSPORT_ERROR: &str = "The xAI Grok Voice connection failed.";
const GENERIC_FINISH_TIMEOUT_ERROR: &str = "xAI Grok Voice did not finish the final turn in time.";
const GENERIC_RESPONSE_FAILED_ERROR: &str = "xAI Grok Voice did not complete the current turn.";
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
// The official live-interpreter cookbook sends 1.5 seconds of trailing audio.
// This is a local finish strategy, not a configured/known server-VAD threshold.
const FINISH_TAIL_SILENCE_MS: u32 = 1_500;
const FINISH_TAIL_FRAME_COUNT: usize =
    (FINISH_TAIL_SILENCE_MS as usize).div_ceil(XAIRealtimeEndpoint::FRAME_DURATION_MS as usize);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum XAIRealtimeClientError {
    #[error("credential_authentication_failed")]
    AuthenticationFailed,
    #[error("Add an xAI API key in Settings.")]
    MissingAPIKey,
    #[error("xAI Grok Voice requires a translated output language.")]
    InvalidTargetLanguage,
    #[error("The xAI Grok Voice session is not connected.")]
    NotConnected,
    #[error("The xAI Grok Voice connection stopped responding.")]
    HealthCheckTimedOut,
    #[error("The xAI Grok Voice connection failed.")]
    TransportFailure,
    #[error("xAI Grok Voice rejected the session configuration.")]
    SessionSetupRejected,
    #[error("xAI Grok Voice did not confirm the session configuration in time.")]
    SessionSetupTimedOut,
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected(XAIRealtimeClientError),
}

fn setup_provider_failure(code: &str) -> XAIRealtimeClientError {
    match code {
        "invalid_api_key"
        | "authentication_error"
        | "authorization_error"
        | "unauthorized"
        | "forbidden"
        | "permission_denied" => XAIRealtimeClientError::AuthenticationFailed,
        _ => XAIRealtimeClientError::SessionSetupRejected,
    }
}

struct Inner {
    // Serializes the content barrier with local assembly, never socket/audio state.
    content_lock: Mutex<()>,
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    audio_send_lock: Mutex<()>,
    pending_audio: Mutex<Vec<u8>>,
    turn: Mutex<GrokTurnState>,
    ready: AtomicBool,
    is_closing: AtomicBool,
    last_response_failed: AtomicBool,
    finish_transport_failed: AtomicBool,
    response_done_notify: Notify,
    pong_notify: Notify,
    generation: AtomicU64,
}

#[derive(Clone)]
pub struct XAIRealtimeClient {
    network: crate::clients::transport::provider_network::ProviderNetwork,
    inner: Arc<Inner>,
    endpoint: url::Url,
    api_key: String,
    source_hint: crate::core::models::SourceLanguage,
    target_language: TargetLanguage,
    events: ProviderEventSender,
}

impl XAIRealtimeClient {
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.clone()
    }

    pub fn with_source_language(mut self, source: crate::core::models::SourceLanguage) -> Self {
        self.source_hint = source;
        self
    }

    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    pub async fn clear_content(&self) -> u64 {
        let _content = self.inner.content_lock.lock().await;
        self.inner.turn.lock().await.clear_content();
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
        api_key: &str,
        target_language: TargetLanguage,
        events: ProviderEventSender,
    ) -> Result<Self, XAIRealtimeClientError> {
        let endpoint =
            XAIRealtimeEndpoint::url().map_err(|_| XAIRealtimeClientError::TransportFailure)?;
        Self::with_endpoint(api_key, target_language, events, endpoint)
    }

    fn with_endpoint(
        api_key: &str,
        target_language: TargetLanguage,
        events: ProviderEventSender,
        endpoint: url::Url,
    ) -> Result<Self, XAIRealtimeClientError> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(XAIRealtimeClientError::MissingAPIKey);
        }
        if !target_language.translates_audio() {
            return Err(XAIRealtimeClientError::InvalidTargetLanguage);
        }
        Ok(Self {
            network: crate::clients::transport::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                content_lock: Mutex::new(()),
                sink: Mutex::new(None),
                receive_task: Mutex::new(None),
                audio_send_lock: Mutex::new(()),
                pending_audio: Mutex::new(Vec::new()),
                turn: Mutex::new(GrokTurnState::default()),
                ready: AtomicBool::new(false),
                is_closing: AtomicBool::new(false),
                last_response_failed: AtomicBool::new(false),
                finish_transport_failed: AtomicBool::new(false),
                response_done_notify: Notify::new(),
                pong_notify: Notify::new(),
                generation: AtomicU64::new(0),
            }),
            endpoint,
            api_key: api_key.to_string(),
            source_hint: crate::core::models::SourceLanguage::Automatic,
            target_language,
            events,
        })
    }

    pub async fn connect(&self) -> Result<(), XAIRealtimeClientError> {
        self.connect_with_timeout(Duration::from_secs(5)).await
    }

    async fn connect_with_timeout(
        &self,
        readiness_timeout: Duration,
    ) -> Result<(), XAIRealtimeClientError> {
        self.disconnect().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let setup_event_id = format!("mimi-xai-session-update-{generation}");

        let mut request = self
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|_| XAIRealtimeClientError::TransportFailure)?;
        let authorization = format!("Bearer {}", self.api_key);
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&authorization)
                .map_err(|_| XAIRealtimeClientError::MissingAPIKey)?,
        );

        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            crate::clients::transport::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| XAIRealtimeClientError::TransportFailure)?
        .map_err(|error| {
            if crate::clients::transport::connection_diagnostics::authentication_rejected(&error) {
                XAIRealtimeClientError::AuthenticationFailed
            } else {
                XAIRealtimeClientError::TransportFailure
            }
        })?;
        let (sink, stream) = socket.split();
        *self.inner.sink.lock().await = Some(sink);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .last_response_failed
            .store(false, Ordering::SeqCst);
        self.inner
            .finish_transport_failed
            .store(false, Ordering::SeqCst);
        self.inner.pending_audio.lock().await.clear();
        self.inner.turn.lock().await.reset();

        let (setup_tx, mut setup_rx) = watch::channel(SetupState::Awaiting);
        let task = tokio::spawn(receive_loop(ReceiveContext {
            inner: Arc::clone(&self.inner),
            stream,
            events: self.events.clone(),
            setup: setup_tx,
            setup_event_id: setup_event_id.clone(),
            generation,
        }));
        *self.inner.receive_task.lock().await = Some(task);

        let update = XAIRealtimeRequestEncoder::session_update(
            self.source_hint,
            self.target_language,
            Some(&setup_event_id),
        )
        .map_err(|_| XAIRealtimeClientError::InvalidTargetLanguage)?;
        let complete_setup = async {
            self.send_text(update.to_string()).await?;
            loop {
                match setup_rx.borrow().clone() {
                    SetupState::Ready => return Ok(()),
                    SetupState::Rejected(error) => return Err(error),
                    SetupState::Awaiting => {}
                }
                if setup_rx.changed().await.is_err() {
                    return Err(XAIRealtimeClientError::SessionSetupRejected);
                }
            }
        };
        match tokio::time::timeout(readiness_timeout, complete_setup).await {
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
                Err(XAIRealtimeClientError::SessionSetupTimedOut)
            }
        }
    }

    /// Accepts arbitrary PCM chunks and sends only exact 200 ms frames.
    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), XAIRealtimeClientError> {
        if pcm_data.is_empty() {
            return Ok(());
        }
        tokio::time::timeout(SEND_TIMEOUT, self.send_audio_operation(pcm_data))
            .await
            .map_err(|_| XAIRealtimeClientError::TransportFailure)?
    }

    async fn send_audio_operation(&self, pcm_data: &[u8]) -> Result<(), XAIRealtimeClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(XAIRealtimeClientError::NotConnected);
        }
        let _guard = self.inner.audio_send_lock.lock().await;
        let messages = {
            let mut pending = self.inner.pending_audio.lock().await;
            pending.extend_from_slice(pcm_data);
            take_complete_audio_messages(&mut pending)?
        };
        for message in messages {
            self.send_text(message).await?;
        }
        Ok(())
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), XAIRealtimeClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(XAIRealtimeClientError::NotConnected);
        }
        let operation = async {
            let pong = self.inner.pong_notify.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(XAIRealtimeClientError::NotConnected);
                };
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| XAIRealtimeClientError::TransportFailure)?;
            }
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| XAIRealtimeClientError::HealthCheckTimedOut)?
    }

    /// In server-VAD mode xAI explicitly disallows
    /// `input_audio_buffer.commit`. Drain the local frame buffer, append the
    /// cookbook's bounded silence tail, then wait a bounded
    /// amount of time for trailing VAD/input/response events before closing.
    /// A preceding `response.done` cannot acknowledge subsequently sent audio.
    pub async fn finish(&self, timeout: Duration) {
        if !self.inner.ready.load(Ordering::SeqCst)
            || self.inner.is_closing.swap(true, Ordering::SeqCst)
        {
            return;
        }
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let finished_cleanly =
            match tokio::time::timeout(timeout, self.finish_operation(generation)).await {
                Ok(Err(_)) if self.is_current_generation(generation) => {
                    self.emit(
                        LiveTranslateServerEvent::Error {
                            code: "xai_session_finish_failed".into(),
                            message: GENERIC_TRANSPORT_ERROR.into(),
                        },
                        generation,
                    );
                    false
                }
                Err(_) if self.is_current_generation(generation) => {
                    // A PCM frame does not imply a VAD speech turn. Keep the
                    // socket alive for the existing deadline so a preceding
                    // response.done cannot hide late events for trailing audio.
                    let unfinished = self.inner.turn.lock().await.has_unfinished_turn();
                    if unfinished {
                        self.emit(
                            LiveTranslateServerEvent::Error {
                                code: "xai_session_finish_timeout".into(),
                                message: GENERIC_FINISH_TIMEOUT_ERROR.into(),
                            },
                            generation,
                        );
                    }
                    !unfinished
                        && !self.inner.last_response_failed.load(Ordering::SeqCst)
                        && !self.inner.finish_transport_failed.load(Ordering::SeqCst)
                }
                _ => false,
            };
        if self.is_current_generation(generation) {
            self.inner.turn.lock().await.reset();
            if finished_cleanly {
                self.emit(LiveTranslateServerEvent::SessionFinished, generation);
            }
        }
        self.disconnect_if_current(generation).await;
    }

    async fn finish_operation(&self, generation: u64) -> Result<(), XAIRealtimeClientError> {
        let _guard = self.inner.audio_send_lock.lock().await;
        if !self.is_current_generation(generation) {
            return Err(XAIRealtimeClientError::NotConnected);
        }
        let partial = {
            let mut pending = self.inner.pending_audio.lock().await;
            take_padded_audio_message(&mut pending)?
        };
        if let Some(message) = partial {
            self.send_text(message).await?;
        }
        let silence = XAIRealtimeRequestEncoder::audio_append(&vec![
            0;
            XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT
        ])
        .map_err(|_| XAIRealtimeClientError::TransportFailure)?
        .to_string();
        for _ in 0..FINISH_TAIL_FRAME_COUNT {
            self.send_text(silence.clone()).await?;
        }

        while self.is_current_generation(generation) {
            let done = self.inner.response_done_notify.notified();
            tokio::pin!(done);
            done.as_mut().enable();
            if !self.is_current_generation(generation) {
                break;
            }
            if self.inner.finish_transport_failed.load(Ordering::SeqCst) {
                return Err(XAIRealtimeClientError::TransportFailure);
            }
            done.await;
        }
        Ok(())
    }

    pub async fn disconnect(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.inner.ready.store(false, Ordering::SeqCst);
        self.inner.is_closing.store(false, Ordering::SeqCst);
        self.inner
            .last_response_failed
            .store(false, Ordering::SeqCst);
        self.inner
            .finish_transport_failed
            .store(false, Ordering::SeqCst);
        if let Some(task) = self.inner.receive_task.lock().await.take() {
            task.abort();
        }
        if let Some(mut sink) = self.inner.sink.lock().await.take() {
            let _ = tokio::time::timeout(Duration::from_millis(250), sink.close()).await;
        }
        self.inner.pending_audio.lock().await.clear();
        self.inner.turn.lock().await.reset();
        self.inner.response_done_notify.notify_waiters();
    }

    async fn send_text(&self, text: String) -> Result<(), XAIRealtimeClientError> {
        let mut sink = self.inner.sink.lock().await;
        let Some(sink) = sink.as_mut() else {
            return Err(XAIRealtimeClientError::NotConnected);
        };
        let evidence =
            crate::development_audio::begin_json(&text, XAIRealtimeEndpoint::SAMPLE_RATE_HZ);
        tokio::time::timeout(
            SEND_TIMEOUT,
            evidence.observe(sink.send(Message::Text(text.into()))),
        )
        .await
        .map_err(|_| XAIRealtimeClientError::TransportFailure)?
        .map_err(|_| XAIRealtimeClientError::TransportFailure)
    }

    fn emit(&self, event: LiveTranslateServerEvent, generation: u64) {
        if self.is_current_generation(generation) {
            let _ = self.events.send(event);
        }
    }

    fn is_current_generation(&self, generation: u64) -> bool {
        self.inner.generation.load(Ordering::SeqCst) == generation
    }

    async fn disconnect_if_current(&self, generation: u64) {
        if self.is_current_generation(generation) {
            self.disconnect().await;
        }
    }
}

fn take_complete_audio_messages(
    pending: &mut Vec<u8>,
) -> Result<Vec<String>, XAIRealtimeClientError> {
    let frame_size = XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let complete_bytes = pending.len() / frame_size * frame_size;
    let result = pending[..complete_bytes]
        .chunks_exact(frame_size)
        .map(|frame| {
            XAIRealtimeRequestEncoder::audio_append(frame)
                .map(|value| value.to_string())
                .map_err(|_| XAIRealtimeClientError::TransportFailure)
        })
        .collect::<Result<Vec<_>, _>>();
    // Preserve the existing failure semantics: a complete batch is consumed
    // even if encoding one of its frames fails.
    pending.drain(..complete_bytes);
    result
}

fn take_padded_audio_message(
    pending: &mut Vec<u8>,
) -> Result<Option<String>, XAIRealtimeClientError> {
    if pending.is_empty() {
        return Ok(None);
    }
    let mut frame = std::mem::take(pending);
    frame.resize(XAIRealtimeEndpoint::AUDIO_FRAME_BYTE_COUNT, 0);
    XAIRealtimeRequestEncoder::audio_append(&frame)
        .map(|value| Some(value.to_string()))
        .map_err(|_| XAIRealtimeClientError::TransportFailure)
}

struct ReceiveContext {
    inner: Arc<Inner>,
    stream: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    events: ProviderEventSender,
    setup: watch::Sender<SetupState>,
    setup_event_id: String,
    generation: u64,
}

async fn receive_loop(mut context: ReceiveContext) {
    while let Some(message) = context.stream.next().await {
        if context.inner.generation.load(Ordering::SeqCst) != context.generation {
            return;
        }
        let event = match message {
            Ok(Message::Text(text)) => XAIRealtimeServerEvent::decode(&text),
            // JSON transport makes binary frames output audio only. mimi does
            // not play provider output audio, so discard them without parsing
            // or retaining their contents.
            Ok(Message::Binary(_)) => continue,
            Ok(Message::Pong(_)) => {
                context.inner.pong_notify.notify_waiters();
                continue;
            }
            Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => {
                fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR).await;
                return;
            }
        };
        let event = match event {
            Ok(event) => event,
            Err(_) => {
                fail_receive_loop(&context, "xai_protocol_error", GENERIC_PROTOCOL_ERROR).await;
                return;
            }
        };
        if handle_server_event(&context, event).await {
            return;
        }
    }
    if context.inner.generation.load(Ordering::SeqCst) == context.generation
        && !context.inner.is_closing.load(Ordering::SeqCst)
    {
        fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR).await;
    }
}

async fn fail_receive_loop(context: &ReceiveContext, code: &str, message: &str) {
    if context.inner.is_closing.load(Ordering::SeqCst) {
        context
            .inner
            .finish_transport_failed
            .store(true, Ordering::SeqCst);
    }
    context.inner.response_done_notify.notify_waiters();
    if *context.setup.borrow() == SetupState::Awaiting {
        let failure = if code == "transport_error" {
            XAIRealtimeClientError::TransportFailure
        } else {
            XAIRealtimeClientError::SessionSetupRejected
        };
        let _ = context.setup.send(SetupState::Rejected(failure));
    } else if !context.inner.is_closing.load(Ordering::SeqCst) {
        emit_if_current(
            context,
            LiveTranslateServerEvent::Error {
                code: code.into(),
                message: message.into(),
            },
        );
    }
}

async fn handle_server_event(context: &ReceiveContext, event: XAIRealtimeServerEvent) -> bool {
    let revision = context.events.content_revision();
    let _content = context.inner.content_lock.lock().await;
    if context.inner.generation.load(Ordering::SeqCst) != context.generation {
        return false;
    }
    if revision != context.events.content_revision()
        && matches!(
            &event,
            XAIRealtimeServerEvent::SourceTranscriptUpdated { .. }
                | XAIRealtimeServerEvent::SourceTranscriptCompleted { .. }
                | XAIRealtimeServerEvent::ResponseStarted { .. }
                | XAIRealtimeServerEvent::TranslationDelta { .. }
                | XAIRealtimeServerEvent::TranslationDone { .. }
                | XAIRealtimeServerEvent::SpeechStarted { .. }
                | XAIRealtimeServerEvent::SourceCommitted { .. }
        )
    {
        return false;
    }

    match event {
        XAIRealtimeServerEvent::SessionCreated => {
            emit_if_current(context, LiveTranslateServerEvent::SessionCreated);
        }
        XAIRealtimeServerEvent::SessionUpdated {
            input_format,
            input_rate,
            transcription_model,
            turn_detection,
            reasoning_effort,
        } => {
            let valid = input_format == "audio/pcm"
                && input_rate == XAIRealtimeEndpoint::SAMPLE_RATE_HZ
                && transcription_model == XAIRealtimeEndpoint::TRANSCRIPTION_MODEL
                && turn_detection == "server_vad"
                && reasoning_effort == "none";
            if !valid {
                if *context.setup.borrow() == SetupState::Awaiting {
                    let _ = context.setup.send(SetupState::Rejected(
                        XAIRealtimeClientError::SessionSetupRejected,
                    ));
                } else {
                    emit_if_current(
                        context,
                        LiveTranslateServerEvent::Error {
                            code: "xai_session_configuration_mismatch".into(),
                            message: GENERIC_PROVIDER_ERROR.into(),
                        },
                    );
                }
                return true;
            }
            if *context.setup.borrow() == SetupState::Awaiting {
                let _ = context.setup.send(SetupState::Ready);
            } else {
                emit_if_current(context, LiveTranslateServerEvent::SessionUpdated);
            }
        }
        XAIRealtimeServerEvent::SourceTranscriptUpdated {
            transcript,
            item_id,
            language,
        } => {
            let events = context
                .inner
                .turn
                .lock()
                .await
                .update_source(transcript, item_id, language, false);
            emit_all_if_current(context, events);
        }
        XAIRealtimeServerEvent::SpeechStarted { item_id } => {
            context
                .inner
                .turn
                .lock()
                .await
                .input_boundary(item_id, false);
        }
        XAIRealtimeServerEvent::SourceCommitted { item_id } => {
            context
                .inner
                .turn
                .lock()
                .await
                .input_boundary(item_id, true);
        }
        XAIRealtimeServerEvent::SourceTranscriptCompleted {
            transcript,
            item_id,
            language,
        } => {
            let events = context
                .inner
                .turn
                .lock()
                .await
                .update_source(transcript, item_id, language, true);
            emit_all_if_current(context, events);
        }
        XAIRealtimeServerEvent::ResponseStarted { response_id } => {
            let mut turn = context.inner.turn.lock().await;
            if turn.response_started(response_id) {
                context
                    .inner
                    .last_response_failed
                    .store(false, Ordering::SeqCst);
                emit_if_current(context, LiveTranslateServerEvent::TranslationStarted);
            }
        }
        XAIRealtimeServerEvent::TranslationDelta { delta, response_id } => {
            let events = context
                .inner
                .turn
                .lock()
                .await
                .append_translation(delta, response_id);
            emit_all_if_current(context, events);
        }
        XAIRealtimeServerEvent::TranslationDone {
            transcript,
            response_id,
        } => {
            let events = context
                .inner
                .turn
                .lock()
                .await
                .complete_translation(transcript, response_id);
            emit_all_if_current(context, events);
        }
        XAIRealtimeServerEvent::OutputAudioDelta => {}
        XAIRealtimeServerEvent::ResponseDone {
            response_id,
            status,
        } => {
            let successful = status.as_deref() == Some("completed");
            let cancelled = status.as_deref() == Some("cancelled");
            let events = context
                .inner
                .turn
                .lock()
                .await
                .finish_response(response_id, successful);
            let Some(events) = events else {
                return false;
            };
            emit_all_if_current(context, events);
            if successful || cancelled {
                context
                    .inner
                    .last_response_failed
                    .store(false, Ordering::SeqCst);
            } else {
                context
                    .inner
                    .last_response_failed
                    .store(true, Ordering::SeqCst);
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: "xai_response_failed".into(),
                        message: GENERIC_RESPONSE_FAILED_ERROR.into(),
                    },
                );
            }
            context.inner.response_done_notify.notify_waiters();
        }
        XAIRealtimeServerEvent::ProviderError {
            code,
            is_recoverable,
            related_event_id,
        } => {
            if *context.setup.borrow() == SetupState::Awaiting {
                if is_recoverable
                    && related_event_id
                        .as_deref()
                        .is_some_and(|id| id != context.setup_event_id)
                {
                    // An explicit error for another request does not reject
                    // our pending session.update acknowledgement.
                    return false;
                }
                // Before readiness, a provider rejection is not a missing ack.
                // Keep its finite typed category when the related id is
                // missing or identifies this session.update.
                let _ = context
                    .setup
                    .send(SetupState::Rejected(setup_provider_failure(&code)));
                return true;
            }
            if !is_recoverable {
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: if setup_provider_failure(&code)
                            == XAIRealtimeClientError::AuthenticationFailed
                        {
                            "credential_authentication_failed".into()
                        } else {
                            format!("xai_provider_error.{code}")
                        },
                        message: GENERIC_PROVIDER_ERROR.into(),
                    },
                );
                return true;
            }
        }
        XAIRealtimeServerEvent::Ignored { kind } => {
            emit_if_current(context, LiveTranslateServerEvent::Ignored { kind });
        }
    }
    if context.inner.turn.lock().await.pairing_ambiguous {
        fail_receive_loop(context, "transport_error", GENERIC_PROTOCOL_ERROR).await;
        return true;
    }
    false
}

fn emit_all_if_current(context: &ReceiveContext, events: Vec<LiveTranslateServerEvent>) {
    for event in events {
        emit_if_current(context, event);
    }
}

fn emit_if_current(context: &ReceiveContext, event: LiveTranslateServerEvent) {
    if context.inner.generation.load(Ordering::SeqCst) == context.generation {
        let _ = context.events.send(event);
    }
}

#[cfg(test)]
mod tests;
