//! Volcano Engine Doubao Simultaneous Interpretation 2.0 WebSocket client.

use crate::clients::transport::provider_events::ProviderEventSender;
use crate::core::models::{SourceLanguage, TargetLanguage, UtteranceRole};
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::realtime::volcano_engine::{
    VolcanoEngineEndpoint, VolcanoEngineProtocolError, VolcanoEngineRequestEncoder,
    VolcanoEngineServerEvent,
};
use futures_util::{SinkExt, StreamExt};
use std::collections::VecDeque;
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
use uuid::Uuid;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SETUP_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(250);
const MAXIMUM_AUDIO_INPUT_BYTES_PER_CALL: usize = 1_024 * 1_024;
const MAXIMUM_PENDING_FINALS_PER_SIDE: usize = 16;
const MAXIMUM_PENDING_FINAL_TEXT_BYTES: usize = 128 * 1_024;
const MAXIMUM_RECENT_COMMITTED_TIMINGS: usize = 32;
const MAXIMUM_SOURCE_BLOCKS: usize = 48;
const GENERIC_PROVIDER_ERROR: &str = "Volcano Engine rejected the translation session.";
const GENERIC_PROTOCOL_ERROR: &str = "Volcano Engine returned an invalid response.";
const GENERIC_TRANSPORT_ERROR: &str = "The Volcano Engine connection failed.";
const TRANSCRIPT_SAFETY_LIMIT_ERROR: &str =
    "Volcano Engine transcript buffering exceeded its safety limit.";
const UNEXPECTED_SESSION_FINISHED_ERROR: &str =
    "Volcano Engine ended the translation session unexpectedly.";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VolcanoEngineClientError {
    #[error("credential_authentication_failed")]
    AuthenticationFailed,
    #[error("Add a Volcano Engine API key in Settings.")]
    MissingAPIKey,
    #[error("Volcano Engine requires an explicit supported source language.")]
    UnsupportedSourceLanguage,
    #[error("Volcano Engine does not support this translation direction.")]
    UnsupportedTargetLanguage,
    #[error("The Volcano Engine translation session is not connected.")]
    NotConnected,
    #[error("The Volcano Engine connection stopped responding.")]
    HealthCheckTimedOut,
    #[error("The Volcano Engine connection could not be established in time.")]
    ConnectionTimedOut,
    #[error("Volcano Engine did not confirm the session configuration in time.")]
    SessionSetupTimedOut,
    #[error("Volcano Engine rejected the session configuration.")]
    SessionSetupRejected,
    #[error("The Volcano Engine transport failed.")]
    TransportFailure,
    #[error("The Volcano Engine audio input exceeded its bounded call limit.")]
    AudioInputTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupState {
    Awaiting,
    Ready,
    Rejected,
}

type Sink = futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type Stream = futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SubtitleTiming {
    start_time_ms: Option<i32>,
    end_time_ms: Option<i32>,
}

impl SubtitleTiming {
    fn new(start_time_ms: Option<i32>, end_time_ms: Option<i32>) -> Self {
        Self {
            start_time_ms,
            end_time_ms,
        }
    }

    fn is_usable(self) -> bool {
        self.start_time_ms.is_some() || self.end_time_ms.is_some()
    }

    /// The official source and translation finals describe the same input
    /// interval. Require the full interval when both sides provide it; fall
    /// back to one shared boundary only when the other boundary is absent.
    fn matches(self, other: Self) -> bool {
        match (
            self.start_time_ms,
            self.end_time_ms,
            other.start_time_ms,
            other.end_time_ms,
        ) {
            (Some(left_start), Some(left_end), Some(right_start), Some(right_end)) => {
                left_start == right_start && left_end == right_end
            }
            (_, Some(left), _, Some(right)) => left == right,
            (Some(left), _, Some(right), _) => left == right,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingFinal {
    timing: SubtitleTiming,
    text: String,
    source_owner: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct SourceBlock {
    owner: u64,
    start_time_ms: Option<i32>,
    has_start: bool,
    source_order: u64,
    finished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingFinalTextLimitExceeded;

type SubtitleCommitResult = Result<Option<LiveTranslateServerEvent>, PendingFinalTextLimitExceeded>;

struct VolcanoSubtitlePairCommitter {
    source_draft: String,
    translation_draft: String,
    source_in_progress: bool,
    translation_in_progress: bool,
    discard_source: bool,
    discard_translation: bool,
    source_language: String,
    sources: VecDeque<PendingFinal>,
    translations: VecDeque<PendingFinal>,
    pending_text_bytes: usize,
    recently_committed: VecDeque<SubtitleTiming>,
    owner_epoch: Uuid,
    next_source_owner: u64,
    source_blocks: VecDeque<SourceBlock>,
    retired_starts: VecDeque<i32>,
    retired_start_floor: Option<i32>,
    source_owner: Option<u64>,
    translation_start_time_ms: Option<i32>,
    translation_has_start: bool,
    translation_owner: Option<u64>,
    last_matched_source_owner: Option<u64>,
    latest_start_time_ms: i32,
}

impl VolcanoSubtitlePairCommitter {
    fn new(source_language: SourceLanguage) -> Self {
        Self {
            source_draft: String::new(),
            translation_draft: String::new(),
            source_in_progress: false,
            translation_in_progress: false,
            discard_source: false,
            discard_translation: false,
            source_language: source_language.raw_value().into(),
            sources: VecDeque::new(),
            translations: VecDeque::new(),
            pending_text_bytes: 0,
            recently_committed: VecDeque::new(),
            owner_epoch: Uuid::new_v4(),
            next_source_owner: 0,
            source_blocks: VecDeque::new(),
            retired_starts: VecDeque::new(),
            retired_start_floor: None,
            source_owner: None,
            translation_start_time_ms: None,
            translation_has_start: false,
            translation_owner: None,
            last_matched_source_owner: None,
            latest_start_time_ms: 0,
        }
    }

    fn owner_id(&self, owner: u64) -> String {
        format!("volcano:{}:{owner}", self.owner_epoch)
    }

    fn owner_for_start(&self, start_time_ms: Option<i32>) -> Option<u64> {
        let start = start_time_ms?;
        self.source_blocks
            .iter()
            .find(|block| block.start_time_ms == Some(start))
            .map(|block| block.owner)
    }

    fn start_is_retired(&self, start_time_ms: Option<i32>) -> bool {
        start_time_ms.is_some_and(|start| {
            self.retired_start_floor.is_some_and(|floor| start <= floor)
                || self.retired_starts.contains(&start)
                || self
                    .recently_committed
                    .iter()
                    .any(|timing| timing.start_time_ms == Some(start))
        })
    }

    fn retire_start(&mut self, start: i32) {
        if !self.retired_starts.contains(&start) {
            self.retired_starts.push_back(start);
            while self.retired_starts.len() > MAXIMUM_SOURCE_BLOCKS {
                if let Some(retired) = self.retired_starts.pop_front() {
                    self.retired_start_floor = Some(
                        self.retired_start_floor
                            .map_or(retired, |floor| floor.max(retired)),
                    );
                }
            }
        }
    }

    fn new_source_owner(&mut self, start_time_ms: Option<i32>) -> Option<u64> {
        let next = self.next_source_owner.checked_add(1)?;
        u32::try_from(next).ok()?;
        self.next_source_owner = next;
        let owner = self.next_source_owner;
        let start = start_time_ms.unwrap_or(self.latest_start_time_ms);
        self.latest_start_time_ms = self.latest_start_time_ms.max(start);
        let source_order = (u64::try_from(start).ok()? << 32) | owner;
        self.source_blocks.push_back(SourceBlock {
            owner,
            start_time_ms,
            has_start: false,
            source_order,
            finished: false,
        });
        while self.source_blocks.len() > MAXIMUM_SOURCE_BLOCKS {
            if let Some(start) = self
                .source_blocks
                .pop_front()
                .and_then(|block| block.start_time_ms)
            {
                self.retire_start(start);
            }
        }
        Some(owner)
    }

    fn start_source(&mut self, start_time_ms: Option<i32>) -> bool {
        if self.start_is_retired(start_time_ms) {
            self.discard_source = true;
            return false;
        }
        if let (Some(start), Some(active)) = (start_time_ms, self.source_owner) {
            if self.source_blocks.iter().any(|block| {
                block.owner == active && block.start_time_ms.is_some_and(|time| start < time)
            }) {
                if self.owner_for_start(start_time_ms).is_none() {
                    self.new_source_owner(start_time_ms);
                }
                self.discard_source = true;
                return false;
            }
        }
        if let Some(owner) = self.owner_for_start(start_time_ms) {
            if self.source_owner == Some(owner) && self.source_in_progress {
                self.discard_source = false;
                return false;
            }
            if self.source_owner == Some(owner)
                && !self.source_in_progress
                && !self
                    .sources
                    .iter()
                    .any(|pending| pending.source_owner == Some(owner))
                && self
                    .source_blocks
                    .iter()
                    .any(|block| block.owner == owner && block.finished)
            {
                // An empty END supplied no final text or durable pair. The
                // same current clock may resume without creating a new owner.
                // Retired clocks were rejected above; older owners and any
                // pending source final remain closed.
                self.source_blocks
                    .iter_mut()
                    .find(|block| block.owner == owner)
                    .unwrap()
                    .finished = false;
                self.source_in_progress = true;
                self.discard_source = false;
                return true;
            }
            // A known old START cannot reopen its text or replace an active B.
            self.discard_source = true;
            return false;
        }
        self.source_owner = self.new_source_owner(start_time_ms);
        if let Some(block) = self
            .source_blocks
            .iter_mut()
            .find(|block| Some(block.owner) == self.source_owner)
        {
            block.has_start = true;
        }
        self.source_draft.clear();
        self.source_in_progress = true;
        self.discard_source = false;
        self.bind_translation_owner();
        true
    }

    fn ensure_source_owner(&mut self) -> Option<u64> {
        if self.source_owner.is_none_or(|owner| {
            self.source_blocks
                .iter()
                .any(|block| block.owner == owner && block.finished)
        }) {
            self.source_owner = self.new_source_owner(None);
        }
        self.source_owner
    }

    fn source_end_owner(
        &mut self,
        start_time_ms: Option<i32>,
        end_time_ms: Option<i32>,
    ) -> Option<u64> {
        let timing = SubtitleTiming::new(start_time_ms, end_time_ms);
        if self
            .recently_committed
            .iter()
            .any(|old| old.matches(timing))
        {
            return None;
        }
        let pending_owner = self
            .sources
            .iter()
            .find(|pending| pending.timing.matches(timing))
            .and_then(|pending| pending.source_owner);
        let owner = self
            .owner_for_start(start_time_ms)
            .or(pending_owner)
            .or_else(|| {
                // An anonymous active B cannot be identified by an unseen A END.
                // Without its clock, the END cannot prove that it precedes B.
                if self.source_owner.is_some_and(|owner| {
                    self.source_blocks.iter().any(|block| {
                        block.owner == owner && block.start_time_ms.is_none() && !block.finished
                    })
                }) {
                    None
                } else if let Some(start) = start_time_ms {
                    self.new_source_owner(Some(start))
                } else if let Some(end) = end_time_ms.filter(|end| {
                    self.source_owner.is_none_or(|owner| {
                        self.source_blocks
                            .iter()
                            .find(|block| block.owner == owner)
                            .and_then(|block| block.start_time_ms)
                            .is_some_and(|start| *end < start)
                    })
                }) {
                    // A missing START may still have an END proven before B's
                    // START. Give that final its own owner and clock order; the
                    // END is never stored as a fabricated START correlation.
                    let owner = self.new_source_owner(None)?;
                    let block = self
                        .source_blocks
                        .iter_mut()
                        .find(|block| block.owner == owner)?;
                    block.source_order = (u64::try_from(end).ok()? << 32) | owner;
                    Some(owner)
                } else {
                    None
                }
            });
        if let Some(owner) = owner {
            if let Some(block) = self
                .source_blocks
                .iter_mut()
                .find(|block| block.owner == owner)
            {
                block.finished = true;
            }
            self.bind_translation_owner();
        }
        owner
    }

    fn finish_implicit_source(&mut self) {
        if let Some(block) = self.source_blocks.iter_mut().find(|block| {
            Some(block.owner) == self.source_owner
                && !block.has_start
                && block.start_time_ms.is_none()
        }) {
            // A stream omitting every START retains its legacy END barrier.
            // This clears an implicit block without assigning that END's clock
            // or identity to whichever anonymous text happened to be active.
            block.finished = true;
            self.source_in_progress = false;
            self.source_draft.clear();
        }
    }

    fn translation_end_is_current(&self, timing: SubtitleTiming) -> bool {
        if self.translation_in_progress && !self.translation_has_start {
            return true;
        }
        if self.translation_start_time_ms.is_some()
            && self.translation_start_time_ms == timing.start_time_ms
        {
            return true;
        }
        let owner = self.owner_for_start(timing.start_time_ms).or_else(|| {
            self.sources
                .iter()
                .find(|pending| pending.timing.matches(timing))
                .and_then(|pending| pending.source_owner)
        });
        owner.is_some() && owner == self.translation_owner
    }

    fn bind_translation_owner(&mut self) {
        if self.translation_owner.is_none() {
            self.translation_owner = self.owner_for_start(self.translation_start_time_ms);
        }
    }

    fn start_translation(&mut self, start_time_ms: Option<i32>) -> bool {
        if self.start_is_retired(start_time_ms) {
            self.discard_translation = true;
            return false;
        }
        if start_time_ms.is_some()
            && self.translation_in_progress
            && self.translation_start_time_ms == start_time_ms
        {
            self.discard_translation = false;
            return false;
        }
        let owner = self.owner_for_start(start_time_ms);
        if owner.is_some_and(|owner| {
            self.translation_owner.is_some_and(|active| {
                self.source_order(owner)
                    .zip(self.source_order(active))
                    .is_some_and(|(order, current)| order < current)
            })
        }) {
            self.discard_translation = true;
            return false;
        }
        self.translation_start_time_ms = start_time_ms;
        self.translation_has_start = true;
        self.translation_owner = owner;
        self.translation_draft.clear();
        self.translation_in_progress = true;
        self.discard_translation = false;
        true
    }

    fn source_text_event(
        &self,
        owner: u64,
        text: String,
        is_final: bool,
    ) -> LiveTranslateServerEvent {
        if !is_final
            && self
                .source_blocks
                .iter()
                .any(|block| block.owner == owner && block.start_time_ms.is_none())
        {
            return LiveTranslateServerEvent::SourceDraft {
                text,
                language: Some(self.source_language.clone()),
            };
        }
        LiveTranslateServerEvent::UtteranceText {
            utterance_id: self.owner_id(owner),
            role: UtteranceRole::Source,
            text,
            is_final,
            language: Some(self.source_language.clone()),
        }
    }

    fn source_order(&self, owner: u64) -> Option<u64> {
        self.source_blocks
            .iter()
            .find(|block| block.owner == owner)
            .map(|block| block.source_order)
    }

    fn source_boundary_event(&self, owner: u64) -> Option<LiveTranslateServerEvent> {
        Some(LiveTranslateServerEvent::SubtitleSourceBoundary {
            utterance_id: self.owner_id(owner),
            source_order: self.source_order(owner)?,
        })
    }

    fn translation_text_event(&self) -> LiveTranslateServerEvent {
        if let Some(owner) = self.translation_owner {
            LiveTranslateServerEvent::UtteranceText {
                utterance_id: self.owner_id(owner),
                role: UtteranceRole::Translation,
                text: self.translation_draft.clone(),
                is_final: false,
                language: None,
            }
        } else {
            LiveTranslateServerEvent::TranslationDraft(self.translation_draft.clone())
        }
    }

    fn republish_after_final(&self) -> Vec<LiveTranslateServerEvent> {
        let Some(final_owner) = self.last_matched_source_owner else {
            return Vec::new();
        };
        self.republish_after_owner(final_owner)
    }

    fn republish_after_owner(&self, final_owner: u64) -> Vec<LiveTranslateServerEvent> {
        let Some(final_order) = self.source_order(final_owner) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        if let Some(owner) = self.source_owner.filter(|owner| {
            self.source_in_progress
                && !self.discard_source
                && !self.source_draft.trim().is_empty()
                && self
                    .source_order(*owner)
                    .is_some_and(|order| order > final_order)
        }) {
            events.push(self.source_text_event(owner, self.source_draft.clone(), false));
        }
        if self.translation_in_progress
            && !self.discard_translation
            && self.translation_owner.is_some_and(|owner| {
                self.source_order(owner)
                    .is_some_and(|order| order > final_order)
            })
        {
            events.push(LiveTranslateServerEvent::TranslationStarted);
            if !self.translation_draft.is_empty() {
                events.push(self.translation_text_event());
            }
        }
        events
    }

    #[cfg(test)]
    fn push_source(
        &mut self,
        text: String,
        start_time_ms: Option<i32>,
        end_time_ms: Option<i32>,
    ) -> SubtitleCommitResult {
        let owner = self.source_end_owner(start_time_ms, end_time_ms);
        self.push_source_with_owner(text, start_time_ms, end_time_ms, owner)
    }

    fn push_source_with_owner(
        &mut self,
        text: String,
        start_time_ms: Option<i32>,
        end_time_ms: Option<i32>,
        source_owner: Option<u64>,
    ) -> SubtitleCommitResult {
        let pending = PendingFinal {
            timing: SubtitleTiming::new(start_time_ms, end_time_ms),
            text: text.trim().to_string(),
            source_owner: None,
        };
        if !self.accepts(&pending) {
            return Ok(None);
        }
        let pending = PendingFinal {
            source_owner,
            ..pending
        };
        if let Some(index) = matching_index(&self.translations, pending.timing) {
            let Some(translation) = self.translations.remove(index) else {
                return Ok(None);
            };
            self.pending_text_bytes = self
                .pending_text_bytes
                .saturating_sub(translation.text.len());
            self.mark_committed(pending.timing);
            return Ok(Some(self.final_pair(pending, translation.text)));
        }
        if upsert_bounded(&mut self.sources, pending, &mut self.pending_text_bytes) {
            self.reset();
            return Err(PendingFinalTextLimitExceeded);
        }
        Ok(None)
    }

    fn push_translation(
        &mut self,
        text: String,
        start_time_ms: Option<i32>,
        end_time_ms: Option<i32>,
    ) -> SubtitleCommitResult {
        let pending = PendingFinal {
            timing: SubtitleTiming::new(start_time_ms, end_time_ms),
            text: text.trim().to_string(),
            source_owner: None,
        };
        if !self.accepts(&pending) {
            return Ok(None);
        }
        if let Some(index) = matching_index(&self.sources, pending.timing) {
            let Some(source) = self.sources.remove(index) else {
                return Ok(None);
            };
            self.pending_text_bytes = self.pending_text_bytes.saturating_sub(source.text.len());
            self.mark_committed(pending.timing);
            return Ok(Some(self.final_pair(source, pending.text)));
        }
        if upsert_bounded(
            &mut self.translations,
            pending,
            &mut self.pending_text_bytes,
        ) {
            self.reset();
            return Err(PendingFinalTextLimitExceeded);
        }
        Ok(None)
    }

    fn accepts(&self, pending: &PendingFinal) -> bool {
        !pending.text.is_empty()
            && pending.timing.is_usable()
            && !self.start_is_retired(pending.timing.start_time_ms)
            && !self
                .recently_committed
                .iter()
                .any(|timing| timing.matches(pending.timing))
    }

    fn final_pair(
        &mut self,
        source: PendingFinal,
        translation: String,
    ) -> LiveTranslateServerEvent {
        self.last_matched_source_owner = source.source_owner;
        if let Some(owner) = source.source_owner {
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id: self.owner_id(owner),
                source: source.text,
                language: Some(self.source_language.clone()),
                translation,
            }
        } else {
            LiveTranslateServerEvent::SubtitleFinalPair {
                source: source.text,
                language: Some(self.source_language.clone()),
                translation,
            }
        }
    }

    fn mark_committed(&mut self, timing: SubtitleTiming) {
        self.recently_committed.push_back(timing);
        while self.recently_committed.len() > MAXIMUM_RECENT_COMMITTED_TIMINGS {
            self.recently_committed.pop_front();
        }
    }

    fn clear_content(&mut self) {
        let active = self.source_in_progress
            || self.translation_in_progress
            || !self.sources.is_empty()
            || !self.translations.is_empty();
        // Known old final intervals remain tombstoned, without retaining text.
        let timings: Vec<_> = self
            .sources
            .iter()
            .chain(self.translations.iter())
            .map(|pending| pending.timing)
            .collect();
        for timing in timings {
            self.mark_committed(timing);
        }
        let starts: Vec<_> = self
            .source_blocks
            .iter()
            .filter_map(|block| block.start_time_ms)
            .chain(self.translation_start_time_ms)
            .collect();
        for start in starts {
            self.retire_start(start);
        }
        self.source_blocks.clear();
        self.source_owner = None;
        self.translation_owner = None;
        self.translation_start_time_ms = None;
        self.translation_has_start = false;
        self.last_matched_source_owner = None;
        self.latest_start_time_ms = 0;
        self.sources.clear();
        self.translations.clear();
        self.source_draft.clear();
        self.translation_draft.clear();
        self.pending_text_bytes = 0;
        self.discard_source |= active;
        self.discard_translation |= active;
    }

    fn reset(&mut self) {
        self.source_draft.clear();
        self.translation_draft.clear();
        self.source_in_progress = false;
        self.translation_in_progress = false;
        self.discard_source = false;
        self.discard_translation = false;
        self.sources.clear();
        self.translations.clear();
        self.pending_text_bytes = 0;
        self.recently_committed.clear();
        self.owner_epoch = Uuid::new_v4();
        self.next_source_owner = 0;
        self.source_blocks.clear();
        self.retired_starts.clear();
        self.retired_start_floor = None;
        self.source_owner = None;
        self.translation_owner = None;
        self.translation_start_time_ms = None;
        self.translation_has_start = false;
        self.last_matched_source_owner = None;
        self.latest_start_time_ms = 0;
    }
}

fn matching_index(queue: &VecDeque<PendingFinal>, timing: SubtitleTiming) -> Option<usize> {
    queue
        .iter()
        .position(|pending| pending.timing.matches(timing))
}

/// Returns true when the combined retained source/translation text exceeds
/// the safety limit. The caller resets both queues before returning control.
fn upsert_bounded(
    queue: &mut VecDeque<PendingFinal>,
    pending: PendingFinal,
    pending_text_bytes: &mut usize,
) -> bool {
    if let Some(index) = matching_index(queue, pending.timing) {
        if let Some(replaced) = queue.remove(index) {
            *pending_text_bytes = pending_text_bytes.saturating_sub(replaced.text.len());
        }
    }
    *pending_text_bytes = pending_text_bytes.saturating_add(pending.text.len());
    queue.push_back(pending);
    while queue.len() > MAXIMUM_PENDING_FINALS_PER_SIDE {
        if let Some(evicted) = queue.pop_front() {
            *pending_text_bytes = pending_text_bytes.saturating_sub(evicted.text.len());
        }
    }
    *pending_text_bytes > MAXIMUM_PENDING_FINAL_TEXT_BYTES
}

struct Inner {
    // Serializes the content barrier with local assembly, never socket/audio state.
    content_lock: Mutex<()>,
    sink: Mutex<Option<Sink>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    audio_send_lock: Mutex<()>,
    pending_audio: Mutex<Vec<u8>>,
    committer: Mutex<VolcanoSubtitlePairCommitter>,
    session_id: Mutex<Option<String>>,
    ready: AtomicBool,
    is_closing: AtomicBool,
    received_session_finished: AtomicBool,
    session_finished_notify: Notify,
    pong_notify: Notify,
    generation: AtomicU64,
}

/// The API key is kept out of `Debug` output and attached only to the
/// short-lived WebSocket upgrade request. Provider text and provider error
/// messages are never logged or surfaced by this client.
#[derive(Clone)]
pub struct VolcanoEngineClient {
    network: crate::clients::transport::provider_network::ProviderNetwork,
    inner: Arc<Inner>,
    endpoint: url::Url,
    api_key: String,
    source_language: SourceLanguage,
    target_language: TargetLanguage,
    events: ProviderEventSender,
}

impl VolcanoEngineClient {
    pub(crate) fn network_endpoint(&self) -> url::Url {
        self.endpoint.clone()
    }

    pub fn content_revision(&self) -> u64 {
        self.events.content_revision()
    }

    pub async fn clear_content(&self) -> u64 {
        let _content = self.inner.content_lock.lock().await;
        self.inner.committer.lock().await.clear_content();
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
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        events: ProviderEventSender,
    ) -> Result<Self, VolcanoEngineClientError> {
        let endpoint =
            VolcanoEngineEndpoint::url().map_err(|_| VolcanoEngineClientError::TransportFailure)?;
        Self::with_endpoint(api_key, source_language, target_language, events, endpoint)
    }

    fn with_endpoint(
        api_key: &str,
        source_language: SourceLanguage,
        target_language: TargetLanguage,
        events: ProviderEventSender,
        endpoint: url::Url,
    ) -> Result<Self, VolcanoEngineClientError> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(VolcanoEngineClientError::MissingAPIKey);
        }
        VolcanoEngineRequestEncoder::validate_languages(source_language, target_language)
            .map_err(map_language_error)?;
        Ok(Self {
            network: crate::clients::transport::provider_network::ProviderNetwork::default(),
            inner: Arc::new(Inner {
                content_lock: Mutex::new(()),
                sink: Mutex::new(None),
                receive_task: Mutex::new(None),
                audio_send_lock: Mutex::new(()),
                pending_audio: Mutex::new(Vec::new()),
                committer: Mutex::new(VolcanoSubtitlePairCommitter::new(source_language)),
                session_id: Mutex::new(None),
                ready: AtomicBool::new(false),
                is_closing: AtomicBool::new(false),
                received_session_finished: AtomicBool::new(false),
                session_finished_notify: Notify::new(),
                pong_notify: Notify::new(),
                generation: AtomicU64::new(0),
            }),
            endpoint,
            api_key: api_key.to_string(),
            source_language,
            target_language,
            events,
        })
    }

    pub async fn connect(&self) -> Result<(), VolcanoEngineClientError> {
        self.connect_with_timeout(SETUP_TIMEOUT).await
    }

    async fn connect_with_timeout(
        &self,
        readiness_timeout: Duration,
    ) -> Result<(), VolcanoEngineClientError> {
        self.disconnect().await;
        let generation = self.inner.generation.load(Ordering::SeqCst);

        let mut request = self
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|_| VolcanoEngineClientError::TransportFailure)?;
        request.headers_mut().insert(
            "X-Api-Key",
            HeaderValue::from_str(&self.api_key)
                .map_err(|_| VolcanoEngineClientError::MissingAPIKey)?,
        );
        request.headers_mut().insert(
            "X-Api-Resource-Id",
            HeaderValue::from_static(VolcanoEngineEndpoint::RESOURCE_ID),
        );

        let (socket, _) = tokio::time::timeout(
            CONNECT_TIMEOUT,
            crate::clients::transport::provider_network::websocket(request, &self.network),
        )
        .await
        .map_err(|_| VolcanoEngineClientError::ConnectionTimedOut)?
        .map_err(|error| {
            if crate::clients::transport::connection_diagnostics::authentication_rejected(&error) {
                VolcanoEngineClientError::AuthenticationFailed
            } else {
                VolcanoEngineClientError::TransportFailure
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
        self.inner.committer.lock().await.reset();
        let session_id = Uuid::new_v4().to_string();
        *self.inner.session_id.lock().await = Some(session_id.clone());

        let (setup_tx, setup_rx) = watch::channel(SetupState::Awaiting);
        let task = tokio::spawn(receive_loop(ReceiveContext {
            inner: Arc::clone(&self.inner),
            stream,
            events: self.events.clone(),
            setup: setup_tx,
            generation,
        }));
        *self.inner.receive_task.lock().await = Some(task);

        let start = VolcanoEngineRequestEncoder::start_session(
            &session_id,
            self.source_language,
            self.target_language,
        )
        .map_err(map_language_error)?;
        let setup = async {
            self.send_binary(start).await?;
            wait_for_setup(setup_rx).await
        };
        let result = match tokio::time::timeout(readiness_timeout, setup).await {
            Ok(result) => result,
            Err(_) => Err(VolcanoEngineClientError::SessionSetupTimedOut),
        };
        match result {
            Ok(()) => {
                self.inner.ready.store(true, Ordering::SeqCst);
                Ok(())
            }
            Err(error) => {
                self.disconnect().await;
                Err(error)
            }
        }
    }

    /// Accepts arbitrary PCM chunks and emits exact 80 ms provider frames.
    pub async fn send_audio(&self, pcm_data: &[u8]) -> Result<(), VolcanoEngineClientError> {
        if pcm_data.is_empty() {
            return Ok(());
        }
        if pcm_data.len() > MAXIMUM_AUDIO_INPUT_BYTES_PER_CALL {
            return Err(VolcanoEngineClientError::AudioInputTooLarge);
        }
        tokio::time::timeout(SEND_TIMEOUT, self.send_audio_operation(pcm_data))
            .await
            .map_err(|_| VolcanoEngineClientError::TransportFailure)?
    }

    async fn send_audio_operation(&self, pcm_data: &[u8]) -> Result<(), VolcanoEngineClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(VolcanoEngineClientError::NotConnected);
        }
        let _send_guard = self.inner.audio_send_lock.lock().await;
        let session_id = self.current_session_id().await?;
        let frames = {
            let mut pending = self.inner.pending_audio.lock().await;
            pending.extend_from_slice(pcm_data);
            take_complete_audio_messages(&session_id, &mut pending)?
        };
        for frame in frames {
            self.send_binary(frame).await?;
        }
        Ok(())
    }

    pub async fn ping(&self, timeout: Duration) -> Result<(), VolcanoEngineClientError> {
        if !self.inner.ready.load(Ordering::SeqCst) || self.inner.is_closing.load(Ordering::SeqCst)
        {
            return Err(VolcanoEngineClientError::NotConnected);
        }
        let operation = async {
            let pong = self.inner.pong_notify.notified();
            tokio::pin!(pong);
            pong.as_mut().enable();
            {
                let mut sink = self.inner.sink.lock().await;
                let Some(sink) = sink.as_mut() else {
                    return Err(VolcanoEngineClientError::NotConnected);
                };
                sink.send(Message::Ping(tokio_tungstenite::tungstenite::Bytes::new()))
                    .await
                    .map_err(|_| VolcanoEngineClientError::TransportFailure)?;
            }
            pong.await;
            Ok(())
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| VolcanoEngineClientError::HealthCheckTimedOut)?
    }

    /// Flushes a final zero-padded 80 ms frame, sends FinishSession, waits for
    /// SessionFinished, and then tears down the transport.
    pub async fn finish(&self, timeout: Duration) {
        if !self.inner.ready.load(Ordering::SeqCst) {
            return;
        }
        if self.inner.is_closing.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner
            .received_session_finished
            .store(false, Ordering::SeqCst);
        let generation = self.inner.generation.load(Ordering::SeqCst);
        match tokio::time::timeout(timeout, self.finish_operation(generation)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) if self.is_current_generation(generation) => {
                self.emit(
                    LiveTranslateServerEvent::Error {
                        code: "volcano_session_close_failed".into(),
                        message: GENERIC_TRANSPORT_ERROR.into(),
                    },
                    generation,
                );
            }
            Err(_) if self.is_current_generation(generation) => {
                self.emit(
                    LiveTranslateServerEvent::Error {
                        code: "volcano_close_timeout".into(),
                        message: "Volcano Engine did not finish closing in time.".into(),
                    },
                    generation,
                );
            }
            _ => {}
        }
        self.disconnect_if_current(generation).await;
    }

    async fn finish_operation(&self, generation: u64) -> Result<(), VolcanoEngineClientError> {
        let send_guard = self.inner.audio_send_lock.lock().await;
        if !self.is_current_generation(generation) {
            return Err(VolcanoEngineClientError::NotConnected);
        }
        let session_id = self.current_session_id().await?;
        let partial = {
            let mut pending = self.inner.pending_audio.lock().await;
            take_padded_audio_message(&session_id, &mut pending)?
        };
        if let Some(frame) = partial {
            self.send_binary(frame).await?;
        }
        let finish = VolcanoEngineRequestEncoder::finish_session(&session_id)
            .map_err(|_| VolcanoEngineClientError::TransportFailure)?;
        self.send_binary(finish).await?;
        drop(send_guard);

        loop {
            if !self.is_current_generation(generation) {
                return Err(VolcanoEngineClientError::NotConnected);
            }
            let finished = self.inner.session_finished_notify.notified();
            tokio::pin!(finished);
            finished.as_mut().enable();
            if self.inner.received_session_finished.load(Ordering::SeqCst) {
                return Ok(());
            }
            finished.await;
        }
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
        let sink = self.inner.sink.lock().await.take();
        if let Some(mut sink) = sink {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, sink.close()).await;
        }
        self.inner.pending_audio.lock().await.clear();
        self.inner.committer.lock().await.reset();
        *self.inner.session_id.lock().await = None;
        self.inner.session_finished_notify.notify_waiters();
        self.inner.pong_notify.notify_waiters();
    }

    async fn send_binary(&self, frame: Vec<u8>) -> Result<(), VolcanoEngineClientError> {
        let mut sink = self.inner.sink.lock().await;
        let Some(sink) = sink.as_mut() else {
            return Err(VolcanoEngineClientError::NotConnected);
        };
        let evidence =
            crate::development_audio::begin_volcano(&frame, VolcanoEngineEndpoint::SAMPLE_RATE_HZ);
        tokio::time::timeout(
            SEND_TIMEOUT,
            evidence.observe(sink.send(Message::Binary(frame.into()))),
        )
        .await
        .map_err(|_| VolcanoEngineClientError::TransportFailure)?
        .map_err(|_| VolcanoEngineClientError::TransportFailure)
    }

    async fn current_session_id(&self) -> Result<String, VolcanoEngineClientError> {
        self.inner
            .session_id
            .lock()
            .await
            .clone()
            .ok_or(VolcanoEngineClientError::NotConnected)
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

fn map_language_error(error: VolcanoEngineProtocolError) -> VolcanoEngineClientError {
    match error {
        VolcanoEngineProtocolError::UnsupportedSourceLanguage => {
            VolcanoEngineClientError::UnsupportedSourceLanguage
        }
        VolcanoEngineProtocolError::UnsupportedTargetLanguage => {
            VolcanoEngineClientError::UnsupportedTargetLanguage
        }
        _ => VolcanoEngineClientError::TransportFailure,
    }
}

fn take_complete_audio_messages(
    session_id: &str,
    pending: &mut Vec<u8>,
) -> Result<Vec<Vec<u8>>, VolcanoEngineClientError> {
    let frame_size = VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT;
    let complete_bytes = pending.len() / frame_size * frame_size;
    let result = pending[..complete_bytes]
        .chunks_exact(frame_size)
        .map(|frame| {
            VolcanoEngineRequestEncoder::audio(session_id, frame)
                .map_err(|_| VolcanoEngineClientError::TransportFailure)
        })
        .collect::<Result<Vec<_>, _>>();
    // Preserve the existing failure semantics: a complete batch is consumed
    // even if encoding one of its frames fails.
    pending.drain(..complete_bytes);
    result
}

fn take_padded_audio_message(
    session_id: &str,
    pending: &mut Vec<u8>,
) -> Result<Option<Vec<u8>>, VolcanoEngineClientError> {
    if pending.is_empty() {
        return Ok(None);
    }
    let mut frame = std::mem::take(pending);
    frame.resize(VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT, 0);
    VolcanoEngineRequestEncoder::audio(session_id, &frame)
        .map(Some)
        .map_err(|_| VolcanoEngineClientError::TransportFailure)
}

async fn wait_for_setup(
    mut setup: watch::Receiver<SetupState>,
) -> Result<(), VolcanoEngineClientError> {
    loop {
        match *setup.borrow() {
            SetupState::Ready => return Ok(()),
            SetupState::Rejected => return Err(VolcanoEngineClientError::SessionSetupRejected),
            SetupState::Awaiting => {}
        }
        if setup.changed().await.is_err() {
            return Err(VolcanoEngineClientError::SessionSetupRejected);
        }
    }
}

struct ReceiveContext {
    inner: Arc<Inner>,
    stream: Stream,
    events: ProviderEventSender,
    setup: watch::Sender<SetupState>,
    generation: u64,
}

async fn receive_loop(mut context: ReceiveContext) {
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
async fn handle_server_event(context: &ReceiveContext, event: VolcanoEngineServerEvent) -> bool {
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

#[cfg(test)]
mod tests;
