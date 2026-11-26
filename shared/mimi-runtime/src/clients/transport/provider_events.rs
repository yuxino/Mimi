//! Bounded provider-to-session event transport.
//!
//! Drafts are latest-value snapshots; lifecycle and confirmed subtitle events
//! use a reliable bounded lane. If the reliable lane fills, a separate
//! one-shot control signal forces generation recovery instead of silently
//! dropping a final or growing an unbounded queue.

use crate::core::models::UtteranceRole;
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
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
    // Optional admission evidence from the shared identity aligner, never
    // inferred from opaque provider IDs or from delivery order.
    source_ordinal: Option<u64>,
    starts_translation: bool,
    event: LiveTranslateServerEvent,
}

struct IdentifiedFinal {
    sequence: u64,
    content_revision: u64,
    utterance_id: String,
    source_ordinal: Option<u64>,
}

struct DeliveredSource {
    content_revision: u64,
    ordinal: u64,
    source_final: bool,
    pair_final: bool,
}

/// A local content boundary, independent of the socket/session generation.
/// Lifecycle and service failures remain meaningful across a content clear.
#[derive(Debug, Clone)]
pub struct ProviderEvent {
    /// Connects queue publication to session admission in development traces.
    /// Locally generated control signals do not have a publication sequence.
    #[cfg(any(test, feature = "development-debugger"))]
    pub transport_sequence: Option<u64>,
    pub content_revision: u64,
    pub event: LiveTranslateServerEvent,
    source_ordinal: Option<u64>,
    starts_translation: bool,
}

impl ProviderEvent {
    pub fn is_content(&self) -> bool {
        is_content_event(&self.event)
    }

    /// Local controls and fixtures have no aligner ordering evidence.
    pub fn unsequenced(event: LiveTranslateServerEvent, content_revision: u64) -> Self {
        Self {
            #[cfg(any(test, feature = "development-debugger"))]
            transport_sequence: None,
            content_revision,
            event,
            source_ordinal: None,
            starts_translation: false,
        }
    }

    pub fn apply_to(
        self,
        controller: &mut crate::core::lifecycle::session::TranslationSessionController,
        source: crate::core::audio::audio_input::AudioSource,
    ) -> crate::core::lifecycle::session::ProviderEventApplication {
        self.apply_to_at(controller, source, std::time::Instant::now())
    }

    pub fn apply_to_at(
        self,
        controller: &mut crate::core::lifecycle::session::TranslationSessionController,
        source: crate::core::audio::audio_input::AudioSource,
        now: std::time::Instant,
    ) -> crate::core::lifecycle::session::ProviderEventApplication {
        controller.handle_ordered_from_at(
            source,
            self.event,
            now,
            self.source_ordinal,
            self.starts_translation,
        )
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
    #[cfg(any(test, feature = "development-debugger"))]
    debug_context: Mutex<
        Option<(
            crate::core::audio::audio_input::AudioSource,
            u64,
            crate::core::development::development_debug::DebugProducer,
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
    recognition_finish_drain: bool,
    pending_finish: Option<SequencedEvent>,
    source_pending: Option<SequencedEvent>,
    translation_pending: Option<SequencedEvent>,
    last_delivered_sequence: u64,
    last_source_final: Option<(u64, u64)>,
    // Revision scopes final freezing across content clears.
    last_identified_final: Option<IdentifiedFinal>,
    latest_delivered_source: Option<DeliveredSource>,
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
                #[cfg(any(test, feature = "development-debugger"))]
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
            recognition_finish_drain: false,
            pending_finish: None,
            source_pending: None,
            translation_pending: None,
            last_delivered_sequence: 0,
            last_source_final: None,
            last_identified_final: None,
            latest_delivered_source: None,
            content_revision,
        },
    )
}

impl ProviderEventSender {
    #[cfg(any(test, feature = "development-debugger"))]
    pub fn set_debug_context(
        &self,
        source: crate::core::audio::audio_input::AudioSource,
        generation: u64,
    ) {
        *self.inner.debug_context.lock().unwrap() = Some((
            source,
            generation,
            crate::core::development::development_debug::DebugProducer::Provider,
        ));
    }
    #[cfg(any(test, feature = "development-debugger"))]
    pub fn debug_context(&self) -> Option<(crate::core::audio::audio_input::AudioSource, u64)> {
        self.inner
            .debug_context
            .lock()
            .unwrap()
            .map(|(source, generation, _)| (source, generation))
    }
    #[cfg(any(test, feature = "development-debugger"))]
    pub fn set_recognition_debug_context(
        &self,
        source: crate::core::audio::audio_input::AudioSource,
        generation: u64,
    ) {
        *self.inner.debug_context.lock().unwrap() = Some((
            source,
            generation,
            crate::core::development::development_debug::DebugProducer::Recognition,
        ));
    }
    #[cfg(any(test, feature = "development-debugger"))]
    fn debug_event(
        &self,
        event: &LiveTranslateServerEvent,
        admission: crate::core::development::development_debug::Admission,
        sequence: Option<u64>,
    ) {
        use crate::core::development::development_debug::{
            self as debug, DebugEvent, ProviderObservation,
        };
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
    #[cfg(not(any(test, feature = "development-debugger")))]
    pub fn set_debug_context(
        &self,
        _source: crate::core::audio::audio_input::AudioSource,
        _generation: u64,
    ) {
    }

    #[cfg(not(any(test, feature = "development-debugger")))]
    pub fn debug_context(&self) -> Option<(crate::core::audio::audio_input::AudioSource, u64)> {
        None
    }

    #[cfg(not(any(test, feature = "development-debugger")))]
    pub fn set_recognition_debug_context(
        &self,
        _source: crate::core::audio::audio_input::AudioSource,
        _generation: u64,
    ) {
    }

    #[cfg(not(any(test, feature = "development-debugger")))]
    fn debug_event(
        &self,
        _event: &LiveTranslateServerEvent,
        _admission: crate::core::development::development_debug::Admission,
        _sequence: Option<u64>,
    ) {
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

    pub(crate) fn send_with_source_order(
        &self,
        event: LiveTranslateServerEvent,
        source_ordinal: Option<u64>,
    ) -> Result<(), ProviderEventSendError> {
        self.send_if_with_source_order(event, source_ordinal, false, || true)
    }

    /// Commit source text and its activity hint in one publication. The
    /// controller still decides whether this identity can start a new wait.
    pub(crate) fn send_translating_source_final(
        &self,
        event: LiveTranslateServerEvent,
        source_ordinal: Option<u64>,
    ) -> Result<(), ProviderEventSendError> {
        self.send_if_with_source_order(event, source_ordinal, true, || true)
    }

    /// Checks validity in the same critical section as sequence allocation and
    /// publication. The predicate must be short, synchronous, and must not
    /// re-enter this sender. An obsolete event is discarded without error.
    pub fn send_if(
        &self,
        event: LiveTranslateServerEvent,
        is_current: impl FnOnce() -> bool,
    ) -> Result<(), ProviderEventSendError> {
        self.send_if_with_source_order(event, None, false, is_current)
    }

    fn send_if_with_source_order(
        &self,
        event: LiveTranslateServerEvent,
        source_ordinal: Option<u64>,
        starts_translation: bool,
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
                crate::core::development::development_debug::Admission::ObsoleteProducer,
                None,
            );
            return Ok(());
        }
        if self.inner.failed.load(Ordering::SeqCst) {
            self.debug_event(
                &event,
                crate::core::development::development_debug::Admission::ProducerBackpressure,
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
                crate::core::development::development_debug::Admission::OversizedText,
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
            source_ordinal,
            starts_translation: starts_translation
                && matches!(
                    &event,
                    LiveTranslateServerEvent::UtteranceText {
                        role: UtteranceRole::Source,
                        is_final: true,
                        ..
                    }
                ),
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
                | LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair { .. }
                | LiveTranslateServerEvent::TranslationDraft(_)
                | LiveTranslateServerEvent::UtteranceText {
                    is_final: false, ..
                } => crate::core::development::development_debug::Admission::QueueDraftAttempt,
                _ => crate::core::development::development_debug::Admission::QueueReliableAttempt,
            },
            Some(event.sequence),
        );
        let result = match &event.event {
            LiveTranslateServerEvent::SourceDraft { .. }
            | LiveTranslateServerEvent::SourceUtteranceDraft { .. }
            | LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair { .. } => {
                if self.inner.source_draft.receiver_count() == 0 {
                    self.debug_event(
                        &event.event,
                        crate::core::development::development_debug::Admission::ProducerClosed,
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
                        crate::core::development::development_debug::Admission::ProducerClosed,
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
                        crate::core::development::development_debug::Admission::ProducerClosed,
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
                        crate::core::development::development_debug::Admission::ProducerClosed,
                        Some(event.sequence),
                    );
                    Err(ProviderEventSendError::Closed)
                }
                Err(mpsc::error::TrySendError::Full(event)) => {
                    self.debug_event(
                        &event.event,
                        crate::core::development::development_debug::Admission::ProducerBackpressureResult,
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
    /// Recognition bridges need the latest unfinished source before their
    /// finish acknowledgement so the bounded session-finish fallback can use
    /// it. Ordinary subtitle consumers retain their terminal-barrier behavior.
    pub(crate) fn enable_recognition_finish_drain(&mut self) {
        self.recognition_finish_drain = true;
    }

    #[cfg(test)]
    pub fn try_recv(&mut self) -> Result<LiveTranslateServerEvent, mpsc::error::TryRecvError> {
        self.try_recv_with_revision().map(|event| event.event)
    }

    pub fn try_recv_with_revision(&mut self) -> Result<ProviderEvent, mpsc::error::TryRecvError> {
        loop {
            let event = self.try_recv_unfiltered()?;
            if self.accepts(&event) {
                return Ok(event);
            }
        }
    }

    fn try_recv_unfiltered(&mut self) -> Result<ProviderEvent, mpsc::error::TryRecvError> {
        if !self.overflow_delivered && *self.overflow.borrow() {
            self.overflow_delivered = true;
            return Ok(self.control_event(overflow_event()));
        }
        if let Some(event) = self.pending_finish.take() {
            return Ok(self.take_reliable_event(event));
        }
        if self.reliable_open {
            match self.reliable.try_recv() {
                Ok(event) => {
                    return Ok(self.take_reliable_event_or_recognition_tail(event));
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
        ProviderEvent::unsequenced(event, self.content_revision.load(Ordering::SeqCst))
    }

    async fn recv_unfiltered(&mut self) -> Option<ProviderEvent> {
        loop {
            if !self.overflow_delivered && *self.overflow.borrow() {
                self.overflow_delivered = true;
                return Some(self.control_event(overflow_event()));
            }
            if let Some(event) = self.pending_finish.take() {
                return Some(self.take_reliable_event(event));
            }
            if self.reliable_open {
                match self.reliable.try_recv() {
                    Ok(event) => {
                        return Some(self.take_reliable_event_or_recognition_tail(event));
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
                            return Some(self.take_reliable_event_or_recognition_tail(event));
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

    fn take_reliable_event_or_recognition_tail(&mut self, event: SequencedEvent) -> ProviderEvent {
        if self.recognition_finish_drain
            && matches!(event.event, LiveTranslateServerEvent::SessionFinished)
        {
            refresh_pending(
                &mut self.source_draft,
                &mut self.source_open,
                &mut self.source_pending,
            );
            let has_tail = self.source_pending.as_ref().is_some_and(|draft| {
                draft.sequence < event.sequence
                    && draft.content_revision == self.content_revision.load(Ordering::SeqCst)
                    && matches!(
                        draft.event,
                        LiveTranslateServerEvent::SourceDraft { .. }
                            | LiveTranslateServerEvent::SourceUtteranceDraft { .. }
                    )
                    && !self.source_draft_is_obsolete(draft)
            });
            if has_tail {
                let draft = self.source_pending.take().unwrap();
                // At most one already-received terminal is retained. Deliver
                // it before consulting normal lanes; overflow stays urgent.
                self.pending_finish = Some(event);
                return self.take_draft_event(draft);
            }
        }
        self.take_reliable_event(event)
    }

    fn take_reliable_event(&mut self, event: SequencedEvent) -> ProviderEvent {
        self.note_delivered_source(&event);
        match &event.event {
            LiveTranslateServerEvent::SubtitlePreviewWithdrawn {
                source_utterance_id,
            } => {
                // Reliable controls are read before latest-value drafts. Consume
                // the matching earlier slot now; a later withdrawal must never
                // replace a marker and make this withdrawn draft visible again.
                refresh_pending(
                    &mut self.source_draft,
                    &mut self.source_open,
                    &mut self.source_pending,
                );
                drop(self.source_pending.take_if(|draft| {
                    draft.content_revision == event.content_revision
                        && draft.sequence <= event.sequence
                        && matches!(draft.event, LiveTranslateServerEvent::SourceUtteranceDraft { utterance_id, .. } if utterance_id == *source_utterance_id)
                }));
            }
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { utterance_id, .. } => {
                self.last_identified_final = Some(IdentifiedFinal {
                    sequence: event.sequence,
                    content_revision: event.content_revision,
                    utterance_id: utterance_id.clone(),
                    source_ordinal: event.source_ordinal,
                });
            }
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
                | LiveTranslateServerEvent::SubtitlePreviewWithdrawn { .. }
                | LiveTranslateServerEvent::TranslationStarted
                | LiveTranslateServerEvent::TranslationDeferred(_)
                | LiveTranslateServerEvent::UtteranceRegistered { .. }
        ) {
            self.last_delivered_sequence = self.last_delivered_sequence.max(event.sequence);
        }
        ProviderEvent {
            #[cfg(any(test, feature = "development-debugger"))]
            transport_sequence: Some(event.sequence),
            content_revision: event.content_revision,
            source_ordinal: event.source_ordinal,
            starts_translation: event.starts_translation,
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
        if self.translation_pending.as_ref().is_some_and(|event| {
            self.stamped_draft_is_obsolete(event)
                .unwrap_or(event.sequence <= self.last_delivered_sequence)
        }) {
            self.translation_pending = None;
        }

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
        Some(self.take_draft_event(event))
    }

    fn take_draft_event(&mut self, event: SequencedEvent) -> ProviderEvent {
        self.note_delivered_source(&event);
        self.last_delivered_sequence = self.last_delivered_sequence.max(event.sequence);
        ProviderEvent {
            #[cfg(any(test, feature = "development-debugger"))]
            transport_sequence: Some(event.sequence),
            content_revision: event.content_revision,
            source_ordinal: event.source_ordinal,
            starts_translation: event.starts_translation,
            event: event.event,
        }
    }

    fn source_draft_is_obsolete(&self, event: &SequencedEvent) -> bool {
        if let Some(obsolete) = self.stamped_draft_is_obsolete(event) {
            return obsolete;
        }
        if let (
            LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair { utterance_id, .. },
            Some(final_event),
        ) = (&event.event, &self.last_identified_final)
        {
            if event.content_revision == final_event.content_revision {
                if utterance_id == &final_event.utterance_id {
                    return true;
                }
                // A queued complete preview for another registered sentence
                // survives an older final. The reducer knows the actual order.
                if final_event.sequence == self.last_delivered_sequence {
                    return false;
                }
            }
        }
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

    fn stamped_draft_is_obsolete(&self, event: &SequencedEvent) -> Option<bool> {
        let LiveTranslateServerEvent::UtteranceText {
            utterance_id,
            role,
            is_final: false,
            ..
        } = &event.event
        else {
            return None;
        };
        let final_event = self
            .last_identified_final
            .as_ref()
            .filter(|final_event| event.content_revision == final_event.content_revision);
        if final_event.is_some_and(|final_event| utterance_id == &final_event.utterance_id) {
            return Some(true);
        }
        if event.sequence > self.last_delivered_sequence {
            return Some(false);
        }
        // Only a queued draft proven to belong to a later source may cross
        // this final. Other lifecycle barriers and unknown identities keep
        // their conservative sequence-based admission rule.
        Some(!final_event.is_some_and(|final_event| {
            final_event.sequence == self.last_delivered_sequence
                && event
                    .source_ordinal
                    .zip(final_event.source_ordinal)
                    .is_some_and(|(draft, final_order)| {
                        draft > final_order
                            && self
                                .latest_delivered_source
                                .as_ref()
                                .filter(|source| source.content_revision == event.content_revision)
                                .is_none_or(|source| {
                                    draft > source.ordinal
                                        || (draft == source.ordinal
                                            && match role {
                                                UtteranceRole::Source => !source.source_final,
                                                UtteranceRole::Translation => !source.pair_final,
                                            })
                                })
                    })
        }))
    }

    fn note_delivered_source(&mut self, event: &SequencedEvent) {
        if event.content_revision != self.content_revision.load(Ordering::SeqCst) {
            return;
        }
        let Some(ordinal) = event.source_ordinal else {
            return;
        };
        let (source_final, pair_final) = match &event.event {
            LiveTranslateServerEvent::UtteranceText {
                role: UtteranceRole::Source,
                is_final,
                ..
            } => (*is_final, false),
            // A known ordinal also proves that a translation's source was
            // observed by the aligner, even if its source draft coalesced.
            LiveTranslateServerEvent::UtteranceText { .. } => (false, false),
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. } => (true, true),
            _ => return,
        };
        match &mut self.latest_delivered_source {
            Some(source) if source.content_revision == event.content_revision => {
                if ordinal > source.ordinal {
                    *source = DeliveredSource {
                        content_revision: event.content_revision,
                        ordinal,
                        source_final,
                        pair_final,
                    };
                } else if ordinal == source.ordinal {
                    source.source_final |= source_final;
                    source.pair_final |= pair_final;
                }
            }
            slot => {
                *slot = Some(DeliveredSource {
                    content_revision: event.content_revision,
                    ordinal,
                    source_final,
                    pair_final,
                })
            }
        }
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
mod tests;
