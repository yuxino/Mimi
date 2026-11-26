//! Quiet-tail turn boundary scheduling, cancellation, and commit barriers.

use super::{Inner, ReceiveContext};
use crate::clients::transport::provider_events::ProviderEventSender;
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

// Gemini can legally deliver transcript chunks after `turnComplete` and does
// not provide a separate transcript-terminal event. Treat every normal turn
// boundary (and the final close boundary) as provisional until transcripts
// have stayed quiet long enough to absorb ordinary network jitter.
pub(super) const TAIL_QUIET_PERIOD: Duration =
    Duration::from_millis(mimi_core::openai_transcript_committer::GEMINI_TURN_TAIL_QUIET_MS);

#[derive(Clone, Copy)]
pub(super) enum TranscriptBoundary {
    Normal(u64),
    Closing,
}

pub(super) fn transcript_boundary_is_current(
    inner: &Inner,
    generation: u64,
    boundary: TranscriptBoundary,
) -> bool {
    if inner.generation.load(Ordering::SeqCst) != generation {
        return false;
    }
    match boundary {
        TranscriptBoundary::Normal(epoch) => {
            !inner.is_closing.load(Ordering::SeqCst)
                && inner.turn_boundary_epoch.load(Ordering::SeqCst) == epoch
        }
        TranscriptBoundary::Closing => inner.is_closing.load(Ordering::SeqCst),
    }
}

pub(super) async fn publish_turn_after_transcript_quiet(
    inner: &Arc<Inner>,
    events: &ProviderEventSender,
    generation: u64,
    boundary: TranscriptBoundary,
    publish_session_finished: bool,
) -> Option<()> {
    loop {
        let revision = wait_for_transcript_quiet(inner, generation, boundary).await?;
        let _content = inner.content_lock.lock().await;
        let mut committer = inner.committer.lock().await;
        if !transcript_boundary_is_current(inner, generation, boundary) {
            return None;
        }
        if inner.transcript_revision.load(Ordering::SeqCst) != revision {
            drop(committer);
            continue;
        }
        // Commit and publish while holding the same guard used by transcript,
        // interruption, and terminal handlers. A reset can therefore never
        // slip between clearing the buffers and publishing their final pair.
        for event in committer.finish_turn() {
            let _ = events.send(event);
        }
        if publish_session_finished {
            let _ = events.send(LiveTranslateServerEvent::SessionFinished);
        }
        return Some(());
    }
}

async fn wait_for_transcript_quiet(
    inner: &Arc<Inner>,
    generation: u64,
    boundary: TranscriptBoundary,
) -> Option<u64> {
    loop {
        if !transcript_boundary_is_current(inner, generation, boundary) {
            return None;
        }
        let revision = inner.transcript_revision.load(Ordering::SeqCst);
        let changed = inner.transcript_notify.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if !transcript_boundary_is_current(inner, generation, boundary)
            || inner.transcript_revision.load(Ordering::SeqCst) != revision
        {
            continue;
        }
        if tokio::time::timeout(TAIL_QUIET_PERIOD, changed)
            .await
            .is_ok()
        {
            continue;
        }
        return Some(revision);
    }
}

async fn emit_normal_turn_after_transcript_quiet(
    inner: &Arc<Inner>,
    events: &ProviderEventSender,
    generation: u64,
    epoch: u64,
) {
    let _ = publish_turn_after_transcript_quiet(
        inner,
        events,
        generation,
        TranscriptBoundary::Normal(epoch),
        false,
    )
    .await;
}

pub(super) fn schedule_normal_turn_commit(context: &ReceiveContext) {
    schedule_normal_turn_commit_for(&context.inner, &context.events, context.generation);
}

pub(super) fn schedule_normal_turn_commit_for(
    inner: &Arc<Inner>,
    events: &ProviderEventSender,
    generation: u64,
) {
    let epoch = inner
        .turn_boundary_epoch
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1);
    inner.transcript_notify.notify_waiters();
    let inner = Arc::clone(inner);
    let events = events.clone();
    drop(tokio::spawn(async move {
        emit_normal_turn_after_transcript_quiet(&inner, &events, generation, epoch).await;
    }));
}

pub(super) fn cancel_normal_turn_boundary(inner: &Inner) {
    inner.turn_boundary_epoch.fetch_add(1, Ordering::SeqCst);
    inner.transcript_notify.notify_waiters();
}
