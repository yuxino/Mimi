//! WebSocket receive loop and server event dispatch for Volcano Engine.

use super::committer::{PendingFinalTextLimitExceeded, SubtitleCommitResult, SubtitleTiming};
use super::{Inner, SetupState, Stream, GENERIC_TRANSPORT_ERROR};
use crate::clients::transport::provider_events::ProviderEventSender;
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::realtime::volcano_engine::VolcanoEngineServerEvent;
use futures_util::StreamExt;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

const GENERIC_PROVIDER_ERROR: &str = "Volcano Engine rejected the translation session.";
pub(super) const GENERIC_PROTOCOL_ERROR: &str = "Volcano Engine returned an invalid response.";
pub(super) const TRANSCRIPT_SAFETY_LIMIT_ERROR: &str =
    "Volcano Engine transcript buffering exceeded its safety limit.";
pub(super) const UNEXPECTED_SESSION_FINISHED_ERROR: &str =
    "Volcano Engine ended the translation session unexpectedly.";

pub(super) struct ReceiveContext {
    pub(super) inner: Arc<Inner>,
    pub(super) stream: Stream,
    pub(super) events: ProviderEventSender,
    pub(super) setup: watch::Sender<SetupState>,
    pub(super) generation: u64,
}

struct ReceiveEndedGuard {
    inner: Arc<Inner>,
    generation: u64,
}

impl Drop for ReceiveEndedGuard {
    fn drop(&mut self) {
        if self.inner.generation.load(Ordering::SeqCst) == self.generation {
            self.inner.receive_ended.store(true, Ordering::SeqCst);
            self.inner.session_finished_notify.notify_waiters();
        }
    }
}

pub(super) async fn receive_loop(mut context: ReceiveContext) {
    let _receive_ended = ReceiveEndedGuard {
        inner: Arc::clone(&context.inner),
        generation: context.generation,
    };
    while let Some(message) = context.stream.next().await {
        if context.inner.generation.load(Ordering::SeqCst) != context.generation {
            return;
        }
        let event = match message {
            Ok(Message::Binary(frame)) => VolcanoEngineServerEvent::decode(&frame),
            Ok(Message::Text(frame)) => VolcanoEngineServerEvent::decode(frame.as_bytes()),
            Ok(Message::Pong(_)) => {
                context.inner.pong_notify.notify_waiters();
                continue;
            }
            Ok(Message::Ping(_)) | Ok(Message::Frame(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => {
                if context.inner.is_closing.load(Ordering::SeqCst)
                    && context
                        .inner
                        .received_session_finished
                        .load(Ordering::SeqCst)
                {
                    return;
                }
                fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR);
                return;
            }
        };
        let event = match event {
            Ok(event) => event,
            Err(_) => {
                fail_receive_loop(&context, "volcano_protocol_error", GENERIC_PROTOCOL_ERROR);
                return;
            }
        };
        if handle_server_event(&context, event).await {
            return;
        }
    }

    if context.inner.generation.load(Ordering::SeqCst) == context.generation
        && !(context.inner.is_closing.load(Ordering::SeqCst)
            && context
                .inner
                .received_session_finished
                .load(Ordering::SeqCst))
    {
        fail_receive_loop(&context, "transport_error", GENERIC_TRANSPORT_ERROR);
    }
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

/// Returns true when the receive loop must stop.
pub(super) async fn handle_server_event(
    context: &ReceiveContext,
    event: VolcanoEngineServerEvent,
) -> bool {
    let revision = context.events.content_revision();
    let _content = context.inner.content_lock.lock().await;
    if context.inner.generation.load(Ordering::SeqCst) != context.generation {
        return false;
    }
    if revision != context.events.content_revision()
        && matches!(
            &event,
            VolcanoEngineServerEvent::SourceSubtitleStarted { .. }
                | VolcanoEngineServerEvent::SourceSubtitleDraft(_)
                | VolcanoEngineServerEvent::SourceSubtitleFinal { .. }
                | VolcanoEngineServerEvent::TranslationSubtitleStarted { .. }
                | VolcanoEngineServerEvent::TranslationSubtitleDraft(_)
                | VolcanoEngineServerEvent::TranslationSubtitleFinal { .. }
        )
    {
        return false;
    }

    let setup_is_awaiting = *context.setup.borrow() == SetupState::Awaiting;
    if setup_is_awaiting
        && !matches!(
            &event,
            VolcanoEngineServerEvent::SessionStarted
                | VolcanoEngineServerEvent::SessionFailed { .. }
        )
    {
        let _ = context.setup.send(SetupState::Rejected);
        return true;
    }

    match event {
        VolcanoEngineServerEvent::SessionStarted => {
            if setup_is_awaiting {
                if !emit_if_current(context, LiveTranslateServerEvent::SessionCreated)
                    || !emit_if_current(context, LiveTranslateServerEvent::SessionUpdated)
                {
                    let _ = context.setup.send(SetupState::Rejected);
                    return true;
                }
                let _ = context.setup.send(SetupState::Ready);
            }
        }
        VolcanoEngineServerEvent::SourceSubtitleStarted { start_time_ms } => {
            let mut committer = context.inner.committer.lock().await;
            if committer.start_source(start_time_ms) {
                let Some(owner) = committer.source_owner else {
                    committer.reset();
                    emit_commit_result(context, Err(PendingFinalTextLimitExceeded));
                    return true;
                };
                if !emit_if_current(context, committer.source_boundary_event(owner).unwrap()) {
                    return true;
                }
                if committer.translation_in_progress
                    && committer.translation_owner == Some(owner)
                    && !committer.translation_draft.is_empty()
                    && !emit_if_current(context, committer.translation_text_event())
                {
                    return true;
                }
            }
        }
        VolcanoEngineServerEvent::SourceSubtitleDraft(text) => {
            let mut committer = context.inner.committer.lock().await;
            if committer.discard_source {
                return false;
            }
            if committer.source_draft.len().saturating_add(text.len())
                > crate::core::models::MAX_SUBTITLE_TEXT_BYTES
            {
                committer.reset();
                emit_commit_result(context, Err(PendingFinalTextLimitExceeded));
                return true;
            }
            let previous = committer.source_owner;
            let Some(owner) = committer.ensure_source_owner() else {
                committer.reset();
                emit_commit_result(context, Err(PendingFinalTextLimitExceeded));
                return true;
            };
            if previous != Some(owner)
                && !emit_if_current(context, committer.source_boundary_event(owner).unwrap())
            {
                return true;
            }
            committer.source_in_progress = true;
            committer.source_draft.push_str(&text);
            let text = committer.source_draft.clone();
            if !emit_if_current(context, committer.source_text_event(owner, text, false)) {
                return true;
            }
        }
        VolcanoEngineServerEvent::SourceSubtitleFinal {
            text,
            start_time_ms,
            end_time_ms,
        } => {
            let mut committer = context.inner.committer.lock().await;
            if committer.start_is_retired(start_time_ms)
                || (committer.discard_source && committer.source_owner.is_none())
            {
                return false;
            }
            let owner = committer.source_end_owner(start_time_ms, end_time_ms);
            if let Some(owner) = owner {
                if !emit_if_current(context, committer.source_boundary_event(owner).unwrap()) {
                    return true;
                }
                if !text.trim().is_empty()
                    && crate::core::models::subtitle_text_within_limit(&text)
                    && !emit_if_current(
                        context,
                        committer.source_text_event(owner, text.clone(), true),
                    )
                {
                    return true;
                }
                if committer.source_owner == Some(owner) {
                    committer.source_in_progress = false;
                    committer.source_draft.clear();
                }
            } else if start_time_ms.is_none() && committer.source_owner.is_none() {
                committer.source_in_progress = false;
                committer.source_draft.clear();
            } else {
                committer.finish_implicit_source();
            }
            let commit = committer.push_source_with_owner(text, start_time_ms, end_time_ms, owner);
            let matched = matches!(&commit, Ok(Some(_)));
            if !emit_commit_result(context, commit) {
                return true;
            }
            if matched {
                for event in committer.republish_after_final() {
                    if !emit_if_current(context, event) {
                        return true;
                    }
                }
            } else if let Some(owner) = owner {
                for event in committer.republish_after_owner(owner) {
                    if !emit_if_current(context, event) {
                        return true;
                    }
                }
                // Source END is reliable but its still-progressive translation
                // remains a latest-value draft, including translation-first wire.
                if committer.translation_in_progress
                    && !committer.discard_translation
                    && committer.translation_owner == Some(owner)
                    && !committer.translation_draft.is_empty()
                    && !emit_if_current(context, committer.translation_text_event())
                {
                    return true;
                }
            }
        }
        VolcanoEngineServerEvent::TranslationSubtitleStarted { start_time_ms } => {
            let mut committer = context.inner.committer.lock().await;
            if committer.start_translation(start_time_ms)
                && !emit_if_current(context, LiveTranslateServerEvent::TranslationStarted)
            {
                return true;
            }
        }
        VolcanoEngineServerEvent::TranslationSubtitleDraft(text) => {
            let mut committer = context.inner.committer.lock().await;
            committer.translation_in_progress = true;
            if committer.discard_translation {
                return false;
            }
            if committer.translation_draft.len().saturating_add(text.len())
                > crate::core::models::MAX_SUBTITLE_TEXT_BYTES
            {
                committer.reset();
                emit_commit_result(context, Err(PendingFinalTextLimitExceeded));
                return true;
            }
            committer.bind_translation_owner();
            committer.translation_draft.push_str(&text);
            if !emit_if_current(context, committer.translation_text_event()) {
                return true;
            }
        }
        VolcanoEngineServerEvent::TranslationSubtitleFinal {
            text,
            start_time_ms,
            end_time_ms,
        } => {
            let mut committer = context.inner.committer.lock().await;
            if committer.start_is_retired(start_time_ms)
                || (committer.discard_translation && committer.translation_start_time_ms.is_none())
            {
                return false;
            }
            if committer.translation_end_is_current(SubtitleTiming::new(start_time_ms, end_time_ms))
            {
                committer.translation_in_progress = false;
                committer.translation_draft.clear();
                committer.translation_start_time_ms = None;
                committer.translation_has_start = false;
                committer.translation_owner = None;
            }
            let commit = committer.push_translation(text, start_time_ms, end_time_ms);
            let matched = matches!(&commit, Ok(Some(_)));
            if !emit_commit_result(context, commit) {
                return true;
            }
            if matched {
                for event in committer.republish_after_final() {
                    if !emit_if_current(context, event) {
                        return true;
                    }
                }
            }
        }
        VolcanoEngineServerEvent::SessionFinished => {
            // An unmatched source or translation final is an incomplete tail,
            // not evidence that two different sentence intervals belong
            // together. Drop it before publishing the terminal event.
            context.inner.committer.lock().await.reset();
            if context.inner.is_closing.load(Ordering::SeqCst) {
                context
                    .inner
                    .received_session_finished
                    .store(true, Ordering::SeqCst);
                context.inner.session_finished_notify.notify_waiters();
                emit_if_current(context, LiveTranslateServerEvent::SessionFinished);
            } else {
                context.inner.ready.store(false, Ordering::SeqCst);
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code: "volcano_unexpected_session_finished".into(),
                        message: UNEXPECTED_SESSION_FINISHED_ERROR.into(),
                    },
                );
            }
            return true;
        }
        VolcanoEngineServerEvent::SessionFailed { status_code } => {
            context.inner.committer.lock().await.reset();
            if setup_is_awaiting {
                let _ = context.setup.send(SetupState::Rejected);
            } else {
                let code = status_code
                    .map(|status| format!("volcano_provider_error.{status}"))
                    .unwrap_or_else(|| "volcano_provider_error".into());
                emit_if_current(
                    context,
                    LiveTranslateServerEvent::Error {
                        code,
                        message: GENERIC_PROVIDER_ERROR.into(),
                    },
                );
            }
            return true;
        }
        VolcanoEngineServerEvent::Ignored { event } => {
            if !emit_if_current(
                context,
                LiveTranslateServerEvent::Ignored {
                    kind: format!("volcanoEvent{event}"),
                },
            ) {
                return true;
            }
        }
    }
    false
}

fn emit_commit_result(context: &ReceiveContext, result: SubtitleCommitResult) -> bool {
    match result {
        Ok(Some(pair)) => emit_if_current(context, pair),
        Ok(None) => true,
        Err(PendingFinalTextLimitExceeded) => emit_if_current(
            context,
            LiveTranslateServerEvent::Error {
                code: "volcano_transcript_safety_limit".into(),
                message: TRANSCRIPT_SAFETY_LIMIT_ERROR.into(),
            },
        ),
    }
}

fn emit_if_current(context: &ReceiveContext, event: LiveTranslateServerEvent) -> bool {
    context.inner.generation.load(Ordering::SeqCst) == context.generation
        && context.events.send(event).is_ok()
}
