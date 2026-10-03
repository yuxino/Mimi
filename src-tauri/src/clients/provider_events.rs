//! Bounded provider-to-session event transport.
//!
//! Drafts are latest-value snapshots; lifecycle and confirmed subtitle events
//! use a reliable bounded lane. If the reliable lane fills, a separate
//! one-shot control signal forces generation recovery instead of silently
//! dropping a final or growing an unbounded queue.

use crate::core::models::UtteranceRole;
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};

const DEFAULT_RELIABLE_CAPACITY: usize = 64;
const OVERFLOW_CODE: &str = "provider_event_backlog_overflow";
const OVERFLOW_MESSAGE: &str = "Translation event processing fell behind. mimi is reconnecting.";

#[derive(Debug, Clone)]
struct SequencedEvent {
    sequence: u64,
    content_revision: u64,
    event: LiveTranslateServerEvent,
}

/// A local content boundary, independent of the socket/session generation.
/// Lifecycle and service failures remain meaningful across a content clear.
#[derive(Debug, Clone)]
pub struct ProviderEvent {
    /// Connects queue publication to session admission in development traces.
    /// Locally generated control signals do not have a publication sequence.
    pub transport_sequence: Option<u64>,
    pub content_revision: u64,
    pub event: LiveTranslateServerEvent,
}

impl ProviderEvent {
    pub fn is_content(&self) -> bool {
        is_content_event(&self.event)
    }
}

pub fn is_content_event(event: &LiveTranslateServerEvent) -> bool {
    !matches!(
        event,
        LiveTranslateServerEvent::SessionCreated
            | LiveTranslateServerEvent::SessionUpdated
            | LiveTranslateServerEvent::SessionFinished
            | LiveTranslateServerEvent::Error { .. }
            | LiveTranslateServerEvent::Ignored { .. }
    )
}

struct SenderInner {
    debug_context: Mutex<
        Option<(
            crate::core::audio_input::AudioSource,
            u64,
            crate::core::development_debug::DebugProducer,
        )>,
    >,
    reliable: mpsc::Sender<SequencedEvent>,
    source_draft: watch::Sender<Option<SequencedEvent>>,
    translation_draft: watch::Sender<Option<SequencedEvent>>,
    overflow: watch::Sender<bool>,
    dispatch: Mutex<()>,
    sequence: AtomicU64,
    failed: AtomicBool,
    content_revision: Arc<AtomicU64>,
}

#[derive(Clone)]
pub struct ProviderEventSender {
    inner: Arc<SenderInner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProviderEventSendError {
    #[error("The session event receiver is closed.")]
    Closed,
    #[error("The reliable session event queue is full.")]
    Backpressure,
    #[error("The subtitle service returned too much text.")]
    TextTooLarge,
}

pub struct ProviderEventReceiver {
    reliable: mpsc::Receiver<SequencedEvent>,
    source_draft: watch::Receiver<Option<SequencedEvent>>,
    translation_draft: watch::Receiver<Option<SequencedEvent>>,
    overflow: watch::Receiver<bool>,
    reliable_open: bool,
    source_open: bool,
    translation_open: bool,
    overflow_open: bool,
    overflow_delivered: bool,
    source_pending: Option<SequencedEvent>,
    translation_pending: Option<SequencedEvent>,
    last_delivered_sequence: u64,
    last_source_final: Option<(u64, u64)>,
    content_revision: Arc<AtomicU64>,
}

pub fn provider_event_channel() -> (ProviderEventSender, ProviderEventReceiver) {
    provider_event_channel_with_capacity(DEFAULT_RELIABLE_CAPACITY)
}

fn provider_event_channel_with_capacity(
    reliable_capacity: usize,
) -> (ProviderEventSender, ProviderEventReceiver) {
    assert!(reliable_capacity > 0);
    let content_revision = Arc::new(AtomicU64::new(0));
    let (reliable_tx, reliable_rx) = mpsc::channel(reliable_capacity);
    let (source_tx, source_rx) = watch::channel(None);
    let (translation_tx, translation_rx) = watch::channel(None);
    let (overflow_tx, overflow_rx) = watch::channel(false);
    (
        ProviderEventSender {
            inner: Arc::new(SenderInner {
                debug_context: Mutex::new(None),
                reliable: reliable_tx,
                source_draft: source_tx,
                translation_draft: translation_tx,
                overflow: overflow_tx,
                dispatch: Mutex::new(()),
                sequence: AtomicU64::new(0),
                failed: AtomicBool::new(false),
                content_revision: Arc::clone(&content_revision),
            }),
        },
        ProviderEventReceiver {
            reliable: reliable_rx,
            source_draft: source_rx,
            translation_draft: translation_rx,
            overflow: overflow_rx,
            reliable_open: true,
            source_open: true,
            translation_open: true,
            overflow_open: true,
            overflow_delivered: false,
            source_pending: None,
            translation_pending: None,
            last_delivered_sequence: 0,
            last_source_final: None,
            content_revision,
        },
    )
}

impl ProviderEventSender {
    pub fn set_debug_context(
        &self,
        source: crate::core::audio_input::AudioSource,
        generation: u64,
    ) {
        *self.inner.debug_context.lock().unwrap() = Some((
            source,
            generation,
            crate::core::development_debug::DebugProducer::Provider,
        ));
    }
    pub fn debug_context(&self) -> Option<(crate::core::audio_input::AudioSource, u64)> {
        self.inner
            .debug_context
            .lock()
            .unwrap()
            .map(|(source, generation, _)| (source, generation))
    }
    pub fn set_recognition_debug_context(
        &self,
        source: crate::core::audio_input::AudioSource,
        generation: u64,
    ) {
        *self.inner.debug_context.lock().unwrap() = Some((
            source,
            generation,
            crate::core::development_debug::DebugProducer::Recognition,
        ));
    }
    fn debug_event(
        &self,
        event: &LiveTranslateServerEvent,
        admission: crate::core::development_debug::Admission,
        sequence: Option<u64>,
    ) {
        use crate::core::development_debug::{self as debug, DebugEvent, ProviderObservation};
        if !debug::is_enabled() {
            return;
        }
        if let Some((source, generation, producer)) = *self.inner.debug_context.lock().unwrap() {
            crate::development_content::provider(
                source,
                generation,
                self.content_revision(),
                sequence,
                producer,
                event,
                admission,
            );
            let mut observation = ProviderObservation::new(
                source,
                generation,
                self.content_revision(),
                event,
                admission,
            );
            observation.transport_sequence = sequence;
            observation.producer = producer;
            debug::record(DebugEvent::Provider { observation });
        }
    }
    pub fn content_revision(&self) -> u64 {
        self.inner.content_revision.load(Ordering::SeqCst)
    }

    /// Advances only content: existing lifecycle/terminal events are retained.
    /// A worker must retain its original revision rather than read this at emit.
    pub fn advance_content_revision(&self) -> u64 {
        let _dispatch = self.inner.dispatch.lock().unwrap();
        let revision = self
            .inner
            .content_revision
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        self.inner.source_draft.send_replace(None);
        self.inner.translation_draft.send_replace(None);
        revision
    }

    pub fn send_content(
        &self,
        expected_revision: u64,
        event: LiveTranslateServerEvent,
    ) -> Result<(), ProviderEventSendError> {
        self.send_if(event, || self.content_revision() == expected_revision)
    }

    pub fn send(&self, event: LiveTranslateServerEvent) -> Result<(), ProviderEventSendError> {
        self.send_if(event, || true)
    }

    /// Checks validity in the same critical section as sequence allocation and
    /// publication. The predicate must be short, synchronous, and must not
    /// re-enter this sender. An obsolete event is discarded without error.
    pub fn send_if(
        &self,
        event: LiveTranslateServerEvent,
        is_current: impl FnOnce() -> bool,
    ) -> Result<(), ProviderEventSendError> {
        // Sequence allocation and lane publication are one tiny synchronous
        // critical section. Provider timers and socket tasks can publish from
        // different Tokio workers; without serialization, sequence 2 could
        // become visible before sequence 1 and let an old draft regress the
        // UI after a newer event.
        let _dispatch = self.inner.dispatch.lock().unwrap();
        if !is_current() {
            self.debug_event(
                &event,
                crate::core::development_debug::Admission::ObsoleteProducer,
                None,
            );
            return Ok(());
        }
        if self.inner.failed.load(Ordering::SeqCst) {
            self.debug_event(
                &event,
                crate::core::development_debug::Admission::ProducerBackpressure,
                None,
            );
            return Err(ProviderEventSendError::Backpressure);
        }
        if matches!(event, LiveTranslateServerEvent::Ignored { .. }) {
            return Ok(());
        }
        // Reject before retaining a draft/queued caption. The fixed error uses
        // the reliable lane so clients that stop on send failure remain visible.
        let rejected_text = !event.text_within_limit();
        if rejected_text {
            self.debug_event(
                &event,
                crate::core::development_debug::Admission::OversizedText,
                None,
            );
        }
        let event = if rejected_text {
            LiveTranslateServerEvent::text_limit_error()
        } else {
            event
        };
        let event = SequencedEvent {
            content_revision: self.content_revision(),
            sequence: self
                .inner
                .sequence
                .fetch_add(1, Ordering::SeqCst)
                .wrapping_add(1),
            event,
        };
        self.debug_event(
            &event.event,
            match &event.event {
                LiveTranslateServerEvent::SourceDraft { .. }
                | LiveTranslateServerEvent::SourceUtteranceDraft { .. }
                | LiveTranslateServerEvent::TranslationDraft(_)
                | LiveTranslateServerEvent::UtteranceText {
                    is_final: false, ..
                } => crate::core::development_debug::Admission::QueueDraftAttempt,
                _ => crate::core::development_debug::Admission::QueueReliableAttempt,
            },
            Some(event.sequence),
        );
        let result = match &event.event {
            LiveTranslateServerEvent::SourceDraft { .. }
            | LiveTranslateServerEvent::SourceUtteranceDraft { .. } => {
                if self.inner.source_draft.receiver_count() == 0 {
                    self.debug_event(
                        &event.event,
                        crate::core::development_debug::Admission::ProducerClosed,
                        Some(event.sequence),
                    );
                    return Err(ProviderEventSendError::Closed);
                }
                self.inner.source_draft.send_replace(Some(event));
                Ok(())
            }
            LiveTranslateServerEvent::TranslationDraft(_) => {
                if self.inner.translation_draft.receiver_count() == 0 {
                    self.debug_event(
                        &event.event,
                        crate::core::development_debug::Admission::ProducerClosed,
                        Some(event.sequence),
                    );
                    return Err(ProviderEventSendError::Closed);
                }
                self.inner.translation_draft.send_replace(Some(event));
                Ok(())
            }
            // Stamped drafts must land on the same latest-value lanes as the
            // unstamped drafts they replace: otherwise every intermediate
            // recognition or translation update would occupy the bounded
            // reliable lane and could starve finals and lifecycle events.
            // Only finals stay on the reliable lane.
            LiveTranslateServerEvent::UtteranceText {
                role,
                is_final: false,
                ..
            } => {
                let lane = match role {
                    UtteranceRole::Source => &self.inner.source_draft,
                    UtteranceRole::Translation => &self.inner.translation_draft,
                };
                if lane.receiver_count() == 0 {
                    self.debug_event(
                        &event.event,
                        crate::core::development_debug::Admission::ProducerClosed,
                        Some(event.sequence),
                    );
                    return Err(ProviderEventSendError::Closed);
                }
                lane.send_replace(Some(event));
                Ok(())
            }
            _ => match self.inner.reliable.try_send(event) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Closed(event)) => {
                    self.debug_event(
                        &event.event,
                        crate::core::development_debug::Admission::ProducerClosed,
                        Some(event.sequence),
                    );
                    Err(ProviderEventSendError::Closed)
                }
                Err(mpsc::error::TrySendError::Full(event)) => {
                    self.debug_event(
                        &event.event,
                        crate::core::development_debug::Admission::ProducerBackpressureResult,
                        Some(event.sequence),
                    );
                    if !self.inner.failed.swap(true, Ordering::SeqCst) {
                        self.inner.overflow.send_replace(true);
                    }
                    Err(ProviderEventSendError::Backpressure)
                }
            },
        };
        if rejected_text && result.is_ok() {
            Err(ProviderEventSendError::TextTooLarge)
        } else {
            result
        }
    }
}

impl ProviderEventReceiver {
    #[cfg(test)]
    pub fn try_recv(&mut self) -> Result<LiveTranslateServerEvent, mpsc::error::TryRecvError> {
        loop {
            let event = self.try_recv_unfiltered()?;
            if self.accepts(&event) {
                return Ok(event.event);
            }
        }
    }

    #[cfg(test)]
    fn try_recv_unfiltered(&mut self) -> Result<ProviderEvent, mpsc::error::TryRecvError> {
        if !self.overflow_delivered && *self.overflow.borrow() {
            self.overflow_delivered = true;
            return Ok(self.control_event(overflow_event()));
        }
        if self.reliable_open {
            match self.reliable.try_recv() {
                Ok(event) => {
                    return Ok(self.take_reliable_event(event));
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.reliable_open = false;
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
            }
        }
        self.refresh_draft_pending();
        if let Some(event) = self.take_next_draft() {
            return Ok(event);
        }
        if self.overflow_open && self.overflow.has_changed().is_err() {
            self.overflow_open = false;
        }
        if !self.reliable_open && !self.source_open && !self.translation_open && !self.overflow_open
        {
            Err(mpsc::error::TryRecvError::Disconnected)
        } else {
            Err(mpsc::error::TryRecvError::Empty)
        }
    }

    #[cfg(test)]
    pub async fn recv(&mut self) -> Option<LiveTranslateServerEvent> {
        self.recv_with_revision().await.map(|event| event.event)
    }

    pub async fn recv_with_revision(&mut self) -> Option<ProviderEvent> {
        loop {
            let event = self.recv_unfiltered().await?;
            if self.accepts(&event) {
                return Some(event);
            }
        }
    }

    fn accepts(&self, event: &ProviderEvent) -> bool {
        !event.is_content()
            || event.content_revision == self.content_revision.load(Ordering::SeqCst)
    }

    fn control_event(&self, event: LiveTranslateServerEvent) -> ProviderEvent {
        ProviderEvent {
            transport_sequence: None,
            content_revision: self.content_revision.load(Ordering::SeqCst),
            event,
        }
    }

    async fn recv_unfiltered(&mut self) -> Option<ProviderEvent> {
        loop {
            if !self.overflow_delivered && *self.overflow.borrow() {
                self.overflow_delivered = true;
                return Some(self.control_event(overflow_event()));
            }
            if self.reliable_open {
                match self.reliable.try_recv() {
                    Ok(event) => {
                        return Some(self.take_reliable_event(event));
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        self.reliable_open = false;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                }
            }
            self.refresh_draft_pending();
            if let Some(event) = self.take_next_draft() {
                return Some(event);
            }
            if !self.reliable_open
                && !self.source_open
                && !self.translation_open
                && (!self.overflow_open || self.overflow_delivered)
            {
                return None;
            }

            tokio::select! {
                biased;

                result = self.overflow.changed(), if self.overflow_open && !self.overflow_delivered => {
                    match result {
                        Ok(()) if *self.overflow.borrow_and_update() => {
                            self.overflow_delivered = true;
                            return Some(self.control_event(overflow_event()));
                        }
                        Ok(()) => {}
                        Err(_) => self.overflow_open = false,
                    }
                }
                event = self.reliable.recv(), if self.reliable_open => {
                    match event {
                        Some(event) => {
                            return Some(self.take_reliable_event(event));
                        }
                        None => self.reliable_open = false,
                    }
                }
                result = self.source_draft.changed(), if self.source_open => {
                    match result {
                        Ok(()) => {
                            self.source_pending = self.source_draft.borrow().clone();
                        }
                        Err(_) => self.source_open = false,
                    }
                }
                result = self.translation_draft.changed(), if self.translation_open => {
                    match result {
                        Ok(()) => {
                            self.translation_pending = self.translation_draft.borrow().clone();
                        }
                        Err(_) => self.translation_open = false,
                    }
                }
            }
        }
    }

    fn refresh_draft_pending(&mut self) {
        refresh_pending(
            &mut self.source_draft,
            &mut self.source_open,
            &mut self.source_pending,
        );
        refresh_pending(
            &mut self.translation_draft,
            &mut self.translation_open,
            &mut self.translation_pending,
        );
    }

    fn take_reliable_event(&mut self, event: SequencedEvent) -> ProviderEvent {
        match &event.event {
            LiveTranslateServerEvent::SourceUtteranceFinal { utterance_id, .. }
            | LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: Some(utterance_id),
                ..
            } => {
                self.last_source_final = Some((event.sequence, *utterance_id));
            }
            _ => {}
        }
        // Preview activity and completed pairs are FIFO reliable signals,
        // but do not supersede the independently latest ASR/draft lanes.
        // In particular, a Finished immediately after a TranslationDraft
        // must not make that successful draft obsolete before it is read.
        if !matches!(
            event.event,
            LiveTranslateServerEvent::PreviewTranslationStarted { .. }
                | LiveTranslateServerEvent::PreviewTranslationFinished { .. }
                | LiveTranslateServerEvent::SubtitlePreviewPair { .. }
                | LiveTranslateServerEvent::SubtitlePreviewCleared
                | LiveTranslateServerEvent::TranslationStarted
                | LiveTranslateServerEvent::TranslationDeferred(_)
        ) {
            self.last_delivered_sequence = self.last_delivered_sequence.max(event.sequence);
        }
        ProviderEvent {
            transport_sequence: Some(event.sequence),
            content_revision: event.content_revision,
            event: event.event,
        }
    }

    fn take_next_draft(&mut self) -> Option<ProviderEvent> {
        if self
            .source_pending
            .as_ref()
            .is_some_and(|event| self.source_draft_is_obsolete(event))
        {
            self.source_pending = None;
        }
        self.translation_pending
            .take_if(|event| event.sequence <= self.last_delivered_sequence);

        let take_source = match (&self.source_pending, &self.translation_pending) {
            (Some(source), Some(translation)) => source.sequence < translation.sequence,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => return None,
        };
        let event = if take_source {
            self.source_pending.take().unwrap()
        } else {
            self.translation_pending.take().unwrap()
        };
        self.last_delivered_sequence = self.last_delivered_sequence.max(event.sequence);
        Some(ProviderEvent {
            transport_sequence: Some(event.sequence),
            content_revision: event.content_revision,
            event: event.event,
        })
    }

    fn source_draft_is_obsolete(&self, event: &SequencedEvent) -> bool {
        if event.sequence > self.last_delivered_sequence {
            return false;
        }
        // A previous sentence's final can arrive after the next real begin.
        // Preserve that newer identity even though its draft sequence is lower;
        // all other reliable content/lifecycle barriers retain their old rules.
        !matches!(
            (&event.event, self.last_source_final),
            (LiveTranslateServerEvent::SourceUtteranceDraft { utterance_id, .. }, Some((sequence, final_id)))
                if sequence == self.last_delivered_sequence && *utterance_id > final_id
        )
    }
}

fn refresh_pending(
    receiver: &mut watch::Receiver<Option<SequencedEvent>>,
    is_open: &mut bool,
    pending: &mut Option<SequencedEvent>,
) {
    if !*is_open {
        return;
    }
    match receiver.has_changed() {
        Ok(true) => *pending = receiver.borrow_and_update().clone(),
        Ok(false) => {}
        Err(_) => {
            *is_open = false;
        }
    }
}

fn overflow_event() -> LiveTranslateServerEvent {
    LiveTranslateServerEvent::Error {
        code: OVERFLOW_CODE.into(),
        message: OVERFLOW_MESSAGE.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn admission_preserves_queue_identity_across_reliable_draft_and_content_boundaries() {
        let (sender, mut receiver) = provider_event_channel();
        sender
            .send(LiveTranslateServerEvent::TranslationStarted)
            .unwrap();
        sender
            .send(LiveTranslateServerEvent::TranslationDraft(
                "Synthetic draft".into(),
            ))
            .unwrap();
        let reliable = receiver.recv_with_revision().await.unwrap();
        let draft = receiver.recv_with_revision().await.unwrap();
        assert_eq!(reliable.transport_sequence, Some(1));
        assert_eq!(draft.transport_sequence, Some(2));
        assert_eq!(draft.content_revision, 0);
        sender.advance_content_revision();
        sender
            .send(LiveTranslateServerEvent::TranslationFinal(
                "Synthetic final".into(),
            ))
            .unwrap();
        let next = receiver.recv_with_revision().await.unwrap();
        assert_eq!(next.transport_sequence, Some(3));
        assert_eq!(next.content_revision, 1);
    }

    #[tokio::test]
    async fn generated_overflow_has_no_fabricated_queue_identity() {
        let (sender, mut receiver) = provider_event_channel_with_capacity(1);
        sender
            .send(LiveTranslateServerEvent::TranslationStarted)
            .unwrap();
        assert_eq!(
            sender.send(LiveTranslateServerEvent::SessionFinished),
            Err(ProviderEventSendError::Backpressure)
        );
        let overflow = receiver.recv_with_revision().await.unwrap();
        assert_eq!(overflow.transport_sequence, None);
        assert!(matches!(
            overflow.event,
            LiveTranslateServerEvent::Error { .. }
        ));
        assert_eq!(
            receiver
                .recv_with_revision()
                .await
                .unwrap()
                .transport_sequence,
            Some(1)
        );
    }

    #[tokio::test]
    async fn a_previous_identified_final_keeps_the_newer_sentence_draft_but_not_its_own() {
        for draft_id in [7, 8] {
            let (sender, mut receiver) = provider_event_channel();
            let draft = LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: draft_id,
                text: "Synthetic repeated lyric".into(),
                language: None,
            };
            sender.send(draft.clone()).unwrap();
            let final_event = LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 7,
                text: "Synthetic repeated lyric".into(),
                language: None,
            };
            sender.send(final_event.clone()).unwrap();
            assert_eq!(receiver.try_recv().unwrap(), final_event);
            if draft_id == 8 {
                assert_eq!(receiver.try_recv().unwrap(), draft);
            }
            assert!(receiver.try_recv().is_err());
            sender
                .send(LiveTranslateServerEvent::SourceUtteranceDraft {
                    utterance_id: 9,
                    text: "Synthetic cleared draft".into(),
                    language: None,
                })
                .unwrap();
            sender.advance_content_revision();
            sender
                .send(LiveTranslateServerEvent::SessionFinished)
                .unwrap();
            assert_eq!(
                receiver.try_recv().unwrap(),
                LiveTranslateServerEvent::SessionFinished
            );
            assert!(receiver.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn an_older_hq_confirmation_and_activity_keep_a_queued_newer_source() {
        let (sender, mut receiver) = provider_event_channel();
        let draft = LiveTranslateServerEvent::SourceUtteranceDraft {
            utterance_id: 8,
            text: "Synthetic newer source".into(),
            language: None,
        };
        sender.send(draft.clone()).unwrap();
        let confirmed = LiveTranslateServerEvent::SubtitleConfirmedPair {
            utterance_id: 1,
            source_utterance_id: Some(7),
            source: "Synthetic previous source".into(),
            translation: "Synthetic previous translation".into(),
            language: None,
        };
        for event in [
            confirmed.clone(),
            LiveTranslateServerEvent::TranslationStarted,
            LiveTranslateServerEvent::SubtitlePreviewCleared,
        ] {
            sender.send(event.clone()).unwrap();
            assert_eq!(receiver.try_recv().unwrap(), event);
        }
        assert_eq!(receiver.try_recv().unwrap(), draft);
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn clear_revision_drops_queued_content_but_keeps_lifecycle_and_popped_stamp() {
        let (sender, mut receiver) = provider_event_channel();
        sender
            .send(LiveTranslateServerEvent::SubtitleFinalPair {
                source: "old".into(),
                language: None,
                translation: "old translation".into(),
            })
            .unwrap();
        let already_popped = receiver.recv_with_revision().await.unwrap();
        sender
            .send(LiveTranslateServerEvent::SessionUpdated)
            .unwrap();
        sender
            .send(LiveTranslateServerEvent::SubtitleFinalPair {
                source: "queued old".into(),
                language: None,
                translation: "old".into(),
            })
            .unwrap();
        sender
            .send(LiveTranslateServerEvent::SourceDraft {
                text: "old draft".into(),
                language: None,
            })
            .unwrap();
        let revision = sender.advance_content_revision();
        assert!(already_popped.is_content());
        assert_ne!(already_popped.content_revision, revision);
        sender
            .send_content(
                already_popped.content_revision,
                LiveTranslateServerEvent::TranslationFinal("late old callback".into()),
            )
            .unwrap();
        sender
            .send_content(
                revision,
                LiveTranslateServerEvent::SourceDraft {
                    text: "new".into(),
                    language: None,
                },
            )
            .unwrap();
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionUpdated)
        );
        let new = receiver.recv_with_revision().await.unwrap();
        assert_eq!(new.content_revision, revision);
        assert!(
            matches!(new.event, LiveTranslateServerEvent::SourceDraft { text, .. } if text == "new")
        );
        assert!(receiver.try_recv().is_err());
        sender
            .send(LiveTranslateServerEvent::Error {
                code: "authentication_error".into(),
                message: "fixed".into(),
            })
            .unwrap();
        sender.advance_content_revision();
        assert!(matches!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::Error { .. })
        ));
    }

    #[test]
    fn oversized_caption_is_not_retained_and_only_a_fixed_error_reaches_the_receiver() {
        let (sender, mut receiver) = provider_event_channel();
        let oversized = "x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1);
        for event in [
            LiveTranslateServerEvent::SourceDraft {
                text: oversized.clone(),
                language: None,
            },
            LiveTranslateServerEvent::TranslationDraft(oversized.clone()),
            LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: None,
                utterance_id: 1,
                source: oversized,
                language: None,
                translation: "valid".into(),
            },
        ] {
            assert_eq!(
                sender.send(event),
                Err(ProviderEventSendError::TextTooLarge)
            );
            assert_eq!(
                receiver.try_recv(),
                Ok(LiveTranslateServerEvent::text_limit_error())
            );
        }
        assert!(sender.inner.source_draft.borrow().is_none());
        assert!(sender.inner.translation_draft.borrow().is_none());
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn invalidated_draft_cannot_overtake_a_final_during_publication() {
        let (sender, mut receiver) = provider_event_channel();
        let epoch = AtomicU64::new(1);
        let final_pair = LiveTranslateServerEvent::SubtitleFinalPair {
            source: "confirmed".into(),
            language: Some("en".into()),
            translation: "已确认".into(),
        };
        let (checked_tx, checked_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let timeout = std::time::Duration::from_secs(5);

        std::thread::scope(|scope| {
            let sender = &sender;
            let epoch = &epoch;
            let final_pair = &final_pair;
            scope.spawn(move || {
                checked_rx.recv_timeout(timeout).unwrap();
                epoch.store(2, Ordering::SeqCst);
                // Force a final into the old check-then-send gap if publication
                // is unlocked. With an atomic check/send it must wait instead.
                let can_publish = sender.inner.dispatch.try_lock().is_ok();
                if can_publish {
                    sender.send(final_pair.clone()).unwrap();
                    resume_tx.send(()).unwrap();
                } else {
                    resume_tx.send(()).unwrap();
                    sender.send(final_pair.clone()).unwrap();
                }
            });

            sender
                .send_if(
                    LiveTranslateServerEvent::TranslationDraft("obsolete preview".into()),
                    || {
                        let was_current = epoch.load(Ordering::SeqCst) == 1;
                        checked_tx.send(()).unwrap();
                        resume_rx.recv_timeout(timeout).unwrap();
                        was_current
                    },
                )
                .unwrap();
        });

        assert_eq!(receiver.try_recv(), Ok(final_pair));
        assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));
    }

    #[test]
    fn stale_drafts_are_discarded_without_suppressing_current_drafts() {
        for draft in [
            LiveTranslateServerEvent::SourceDraft {
                text: "current source".into(),
                language: Some("en".into()),
            },
            LiveTranslateServerEvent::TranslationDraft("当前译文".into()),
        ] {
            let (sender, mut receiver) = provider_event_channel();
            let epoch = AtomicU64::new(1);
            sender
                .send_if(draft.clone(), || epoch.load(Ordering::SeqCst) == 1)
                .unwrap();
            assert_eq!(receiver.try_recv(), Ok(draft.clone()));

            epoch.store(2, Ordering::SeqCst);
            sender
                .send(LiveTranslateServerEvent::TranslationStarted)
                .unwrap();
            sender
                .send_if(draft.clone(), || epoch.load(Ordering::SeqCst) == 1)
                .unwrap();
            assert_eq!(
                receiver.try_recv(),
                Ok(LiveTranslateServerEvent::TranslationStarted)
            );
            assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));

            sender
                .send_if(draft.clone(), || epoch.load(Ordering::SeqCst) == 2)
                .unwrap();
            assert_eq!(receiver.try_recv(), Ok(draft));
        }
    }

    #[tokio::test]
    async fn drafts_are_latest_only() {
        let (sender, mut receiver) = provider_event_channel();
        sender
            .send(LiveTranslateServerEvent::SourceDraft {
                text: "first".into(),
                language: Some("en".into()),
            })
            .unwrap();
        sender
            .send(LiveTranslateServerEvent::SourceDraft {
                text: "latest".into(),
                language: Some("en".into()),
            })
            .unwrap();

        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SourceDraft {
                text: "latest".into(),
                language: Some("en".into()),
            })
        );
    }

    #[tokio::test]
    async fn interleaved_draft_lanes_are_delivered_in_publish_order() {
        let (sender, mut receiver) = provider_event_channel();
        let translation = LiveTranslateServerEvent::TranslationDraft("older preview".into());
        let source = LiveTranslateServerEvent::SourceDraft {
            text: "new source".into(),
            language: Some("en".into()),
        };
        sender.send(translation.clone()).unwrap();
        sender.send(source.clone()).unwrap();

        assert_eq!(receiver.recv().await, Some(translation));
        assert_eq!(receiver.recv().await, Some(source));
    }

    #[tokio::test]
    async fn reliable_events_take_priority_and_suppress_older_drafts() {
        let (sender, mut receiver) = provider_event_channel();
        sender
            .send(LiveTranslateServerEvent::SourceDraft {
                text: "stale".into(),
                language: Some("en".into()),
            })
            .unwrap();
        let final_pair = LiveTranslateServerEvent::SubtitleFinalPair {
            source: "confirmed".into(),
            language: Some("en".into()),
            translation: "已确认".into(),
        };
        sender.send(final_pair.clone()).unwrap();

        assert_eq!(receiver.recv().await, Some(final_pair));
        let no_stale =
            tokio::time::timeout(std::time::Duration::from_millis(20), receiver.recv()).await;
        assert!(no_stale.is_err());
    }

    #[tokio::test]
    async fn preview_lifecycle_and_pair_preserve_independent_latest_drafts() {
        for use_async_receive in [false, true] {
            let (sender, mut receiver) = provider_event_channel();
            let started = LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 1 };
            let translation =
                LiveTranslateServerEvent::TranslationDraft("synthetic translation".into());
            let pair = LiveTranslateServerEvent::SubtitlePreviewPair {
                source_utterance_id: None,
                source: "synthetic source".into(),
                language: Some("en".into()),
                translation: "synthetic translation".into(),
            };
            let finished = LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 1 };
            let latest_source = LiveTranslateServerEvent::SourceDraft {
                text: "newer synthetic source".into(),
                language: Some("en".into()),
            };
            sender.send(started.clone()).unwrap();
            sender.send(translation.clone()).unwrap();
            sender.send(pair.clone()).unwrap();
            sender.send(finished.clone()).unwrap();
            sender.send(latest_source.clone()).unwrap();

            // Reliable activity must be observable without swallowing raw
            // recognition or the successful latest-value translation lane.
            for expected in [started, pair, finished, translation, latest_source] {
                let actual = if use_async_receive {
                    receiver.recv().await.unwrap()
                } else {
                    receiver.try_recv().unwrap()
                };
                assert_eq!(actual, expected);
            }
            assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));
        }
    }

    #[test]
    fn final_barrier_remains_ordered_after_preview_signals_and_discards_older_drafts() {
        let (sender, mut receiver) = provider_event_channel();
        let pair = LiveTranslateServerEvent::SubtitlePreviewPair {
            source_utterance_id: None,
            source: "synthetic source".into(),
            language: Some("en".into()),
            translation: "synthetic preview".into(),
        };
        let finished = LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 1 };
        let final_pair = LiveTranslateServerEvent::SubtitleFinalPair {
            source: "corrected synthetic final".into(),
            language: Some("en".into()),
            translation: "synthetic final".into(),
        };
        sender.send(pair.clone()).unwrap();
        sender
            .send(LiveTranslateServerEvent::TranslationDraft(
                "obsolete draft".into(),
            ))
            .unwrap();
        sender.send(finished.clone()).unwrap();
        sender.send(final_pair.clone()).unwrap();

        assert_eq!(receiver.try_recv(), Ok(pair));
        assert_eq!(receiver.try_recv(), Ok(finished));
        assert_eq!(receiver.try_recv(), Ok(final_pair));
        assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));
    }

    #[tokio::test]
    async fn reliable_overflow_emits_one_recoverable_control_event() {
        let (sender, mut receiver) = provider_event_channel_with_capacity(1);
        sender
            .send(LiveTranslateServerEvent::TranslationStarted)
            .unwrap();
        assert_eq!(
            sender.send(LiveTranslateServerEvent::SessionFinished),
            Err(ProviderEventSendError::Backpressure)
        );

        assert_eq!(receiver.recv().await, Some(overflow_event()));
        assert_eq!(
            sender.send(LiveTranslateServerEvent::SessionFinished),
            Err(ProviderEventSendError::Backpressure)
        );
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::TranslationStarted)
        );
    }

    #[test]
    fn overflow_diagnostic_is_content_free() {
        let LiveTranslateServerEvent::Error { code, message } = overflow_event() else {
            unreachable!();
        };
        assert_eq!(code, OVERFLOW_CODE);
        assert!(!message.contains("subtitle"));
        assert!(!message.contains("translation text"));
    }

    fn stamped_draft(
        utterance_id: &str,
        role: UtteranceRole,
        text: &str,
    ) -> LiveTranslateServerEvent {
        LiveTranslateServerEvent::UtteranceText {
            utterance_id: utterance_id.into(),
            role,
            text: text.into(),
            is_final: false,
            language: None,
        }
    }

    #[test]
    fn stamped_drafts_use_the_latest_value_lanes() {
        let (sender, mut receiver) = provider_event_channel();
        // Far more drafts than the reliable lane could hold: an accidental
        // reliable-lane send would fill it and fail with Backpressure.
        for index in 0..200 {
            sender
                .send(stamped_draft(
                    &format!("source_{index}"),
                    UtteranceRole::Source,
                    &format!("source draft {index}"),
                ))
                .unwrap();
            sender
                .send(stamped_draft(
                    &format!("source_{index}"),
                    UtteranceRole::Translation,
                    &format!("译文草稿 {index}"),
                ))
                .unwrap();
        }

        assert_eq!(
            receiver.try_recv(),
            Ok(stamped_draft(
                "source_199",
                UtteranceRole::Source,
                "source draft 199"
            ))
        );
        assert_eq!(
            receiver.try_recv(),
            Ok(stamped_draft(
                "source_199",
                UtteranceRole::Translation,
                "译文草稿 199"
            ))
        );
        assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));
    }

    #[test]
    fn stamped_finals_stay_on_the_reliable_lane() {
        let (sender, mut receiver) = provider_event_channel();
        for index in 0..200 {
            sender
                .send(stamped_draft(
                    &format!("source_{index}"),
                    UtteranceRole::Source,
                    &format!("draft {index}"),
                ))
                .unwrap();
        }

        let source_final = LiveTranslateServerEvent::UtteranceText {
            utterance_id: "source_199".into(),
            role: UtteranceRole::Source,
            text: "final".into(),
            is_final: true,
            language: Some("en".into()),
        };
        let pair = LiveTranslateServerEvent::SubtitleFinalPair {
            source: "final".into(),
            language: Some("en".into()),
            translation: "定稿".into(),
        };
        sender.send(source_final.clone()).unwrap();
        sender.send(pair.clone()).unwrap();

        assert_eq!(receiver.try_recv(), Ok(source_final));
        assert_eq!(receiver.try_recv(), Ok(pair));
        // The draft published before them cannot resurface after the final.
        assert_eq!(receiver.try_recv(), Err(mpsc::error::TryRecvError::Empty));
    }
}
