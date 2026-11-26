//! GoAway replacement connection pre-establishment, quiet drain, and handoff.

use super::turn_boundary::{cancel_normal_turn_boundary, TAIL_QUIET_PERIOD};
#[cfg(test)]
use super::ProbeConnectionEvent;
use super::{
    connection_error, decode_binary_event, emit_all_if_current, handle_server_event,
    take_complete_audio_messages, take_padded_audio_message, GeminiLiveClient,
    GeminiLiveClientError, Inner, ReceiveContext, Sink, Stream, TransportHealth,
};
use crate::core::protocols::realtime::gemini_live::{
    GeminiLiveRequestEncoder, GeminiLiveServerEvent,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const ROTATION_SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const ROTATION_DRAIN_TIMEOUT: Duration = Duration::from_millis(1_500);

/// Establish the replacement while capture/audio and old transcripts continue.
/// No resumption handle or audio replay: neither has reliable alignment metadata
/// for this continuous translation model. Only one socket receives input audio.
pub(super) async fn prepare_replacement(
    client: &GeminiLiveClient,
) -> Result<(Sink, Stream), GeminiLiveClientError> {
    let request = client
        .authenticated_endpoint()
        .into_client_request()
        .map_err(|_| GeminiLiveClientError::TransportFailure)?;
    let (socket, _) =
        crate::clients::transport::provider_network::websocket(request, &client.network)
            .await
            .map_err(connection_error)?;
    let (mut sink, mut stream) = socket.split();
    let setup = GeminiLiveRequestEncoder::setup(client.target_language)
        .map_err(|_| GeminiLiveClientError::InvalidTargetLanguage)?;
    sink.send(Message::Text(setup.to_string().into()))
        .await
        .map_err(|_| GeminiLiveClientError::TransportFailure)?;
    while let Some(message) = stream.next().await {
        let events = match message.map_err(|_| GeminiLiveClientError::TransportFailure)? {
            Message::Text(text) => GeminiLiveServerEvent::decode(&text)
                .map_err(|_| GeminiLiveClientError::SessionSetupRejected)?,
            Message::Binary(data) => decode_binary_event(&data)
                .map_err(|_| GeminiLiveClientError::SessionSetupRejected)?,
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            Message::Close(_) => return Err(GeminiLiveClientError::TransportFailure),
        };
        return match events.as_slice() {
            [GeminiLiveServerEvent::SetupComplete] => Ok((sink, stream)),
            [GeminiLiveServerEvent::ProviderError { code, .. }] => {
                Err(GeminiLiveClientError::from_provider_code(code))
            }
            [GeminiLiveServerEvent::GoAway] => Err(GeminiLiveClientError::TransportFailure),
            _ => Err(GeminiLiveClientError::SessionSetupRejected),
        };
    }
    Err(GeminiLiveClientError::TransportFailure)
}

pub(super) enum RotationFailure {
    Failed(GeminiLiveClientError),
    AlreadyReported,
}

impl From<GeminiLiveClientError> for RotationFailure {
    fn from(error: GeminiLiveClientError) -> Self {
        Self::Failed(error)
    }
}

pub(super) async fn rotate_connection(context: &mut ReceiveContext) -> Result<(), RotationFailure> {
    let client = context.client.clone();
    let replacement = tokio::time::timeout(ROTATION_SETUP_TIMEOUT, prepare_replacement(&client));
    tokio::pin!(replacement);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let (mut next_sink, next_stream) = loop {
        tokio::select! {
            result = &mut replacement => {
                #[cfg(test)]
                if !matches!(&result, Ok(Ok(_))) {
                    record_rotation(&context.inner, "geminiReplacementSetupFailed");
                }
                let prepared = result.map_err(|_| GeminiLiveClientError::SessionSetupTimedOut).and_then(|result| result);
                if prepared.is_err() && context.inner.is_closing.load(Ordering::SeqCst) {
                    // Stop already owns the old socket. A rejected replacement
                    // that never received audio cannot discard its closing tail.
                    return Ok(());
                }
                break prepared?;
            },
            message = context.stream.next() => {
                process_rotation_message(context, message).await?;
            },
            _ = tick.tick() => {
                let _content = context.inner.content_lock.lock().await;
                if context.inner.generation.load(Ordering::SeqCst) != context.generation { return Err(RotationFailure::AlreadyReported); }
                let mut committer = context.inner.committer.lock().await;
                if !committer.turn_complete_received {
                    emit_all_if_current(context, committer.settle());
                }
            }
        }
    };

    let inner = Arc::clone(&context.inner);
    let _send = inner.audio_send_lock.lock().await;
    if inner.generation.load(Ordering::SeqCst) != context.generation {
        return Err(RotationFailure::AlreadyReported);
    }
    if inner.is_closing.load(Ordering::SeqCst) {
        // Stop owns the old socket; leave its real closing boundary intact.
        let _ = tokio::time::timeout(Duration::from_millis(250), next_sink.close()).await;
        return Ok(());
    }
    let partial = take_padded_audio_message(&mut *inner.pending_audio.lock().await)?;
    if let Some(message) = partial {
        client.send_text(message).await?;
    }
    client
        .send_text(GeminiLiveRequestEncoder::audio_stream_end().to_string())
        .await?;
    inner.rotating.store(true, Ordering::SeqCst);
    let handoff = RotationHandoff {
        inner: Arc::clone(&inner),
        generation: context.generation,
    };
    drop(_send);

    // Continue reading late old-session output without mixing it into the new
    // session. A finite quiet drain precedes finalization and the socket switch.
    let deadline = tokio::time::Instant::now() + ROTATION_DRAIN_TIMEOUT;
    let mut quiet_since = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(quiet_since + TAIL_QUIET_PERIOD) => break,
            _ = tokio::time::sleep_until(deadline) => return Err(GeminiLiveClientError::TransportFailure.into()),
            message = context.stream.next() => {
                let revision = inner.transcript_revision.load(Ordering::SeqCst);
                process_rotation_message(context, message).await?;
                if inner.transcript_revision.load(Ordering::SeqCst) != revision {
                    quiet_since = tokio::time::Instant::now();
                }
            }
        }
    }

    let _send = inner.audio_send_lock.lock().await;
    let _content = inner.content_lock.lock().await;
    if inner.generation.load(Ordering::SeqCst) != context.generation {
        return Err(RotationFailure::AlreadyReported);
    }
    // Once audioStreamEnd was sent, Stop must finish the replacement and its
    // staged audio; it must never send those buffers into the ended old stream.
    cancel_normal_turn_boundary(&inner);
    emit_all_if_current(context, inner.committer.lock().await.finish_turn());
    // Stop may have observed the old socket's turnComplete during the drain.
    // The replacement must acknowledge its own audioStreamEnd before closing.
    inner.received_final_turn.store(false, Ordering::SeqCst);
    let old_sink = {
        let mut sink = inner.sink.lock().await;
        *inner.transport_health.lock().await = Some(TransportHealth::connected(context.generation));
        sink.replace(next_sink)
    };
    context.stream = next_stream;
    drop(_content);
    let messages = {
        let mut pending = inner.pending_audio.lock().await;
        take_complete_audio_messages(&mut pending)?
    };
    for message in messages {
        client.send_text(message).await?;
    }
    record_rotation(&inner, "geminiConnectionRotated");
    drop(handoff);
    // Close the retired transport without extending the audio-send barrier.
    drop(_send);
    if let Some(mut sink) = old_sink {
        let _ = tokio::time::timeout(Duration::from_millis(250), sink.close()).await;
    }
    Ok(())
}

struct RotationHandoff {
    inner: Arc<Inner>,
    generation: u64,
}

impl Drop for RotationHandoff {
    fn drop(&mut self) {
        if self.inner.generation.load(Ordering::SeqCst) == self.generation {
            self.inner.rotating.store(false, Ordering::SeqCst);
            self.inner.rotation_notify.notify_waiters();
        }
    }
}

pub(super) fn record_rotation(_inner: &Inner, kind: &'static str) {
    crate::pipeline_log!("gemini connection rotation status={}", kind);
    #[cfg(test)]
    {
        let elapsed_ms = _inner
            .probe_connection_epoch
            .lock()
            .unwrap()
            .elapsed()
            .as_millis();
        let mut events = _inner.probe_connection_events.lock().unwrap();
        if events.len() < 16 {
            events.push(ProbeConnectionEvent { elapsed_ms, kind });
        }
    }
}

async fn process_rotation_message(
    context: &ReceiveContext,
    message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
) -> Result<(), RotationFailure> {
    let message = message
        .ok_or(GeminiLiveClientError::TransportFailure)?
        .map_err(|_| GeminiLiveClientError::TransportFailure)?;
    let events = match message {
        Message::Text(text) => GeminiLiveServerEvent::decode(&text)
            .map_err(|_| GeminiLiveClientError::SessionSetupRejected)?,
        Message::Binary(data) => {
            decode_binary_event(&data).map_err(|_| GeminiLiveClientError::SessionSetupRejected)?
        }
        Message::Pong(_) => {
            context.inner.pong_notify.notify_waiters();
            return Ok(());
        }
        Message::Ping(_) | Message::Frame(_) => return Ok(()),
        Message::Close(_) => return Err(GeminiLiveClientError::TransportFailure.into()),
    };
    for event in events {
        if event != GeminiLiveServerEvent::GoAway && handle_server_event(context, event).await {
            return Err(RotationFailure::AlreadyReported);
        }
    }
    Ok(())
}
