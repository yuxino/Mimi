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
mod tests {
    #[tokio::test]
    async fn handshake_authentication_rejections_preserve_transport_failures() {
        for (status, expected_auth_label) in
            crate::clients::transport::connection_diagnostics::handshake_failure_cases()
        {
            let (endpoint, server) =
                crate::clients::transport::connection_diagnostics::rejected_websocket_endpoint(
                    status,
                )
                .await;
            let (events, _receiver) = provider_event_channel();
            let mut client = VolcanoEngineClient::with_endpoint(
                "test-key-not-real",
                SourceLanguage::English,
                TargetLanguage::Japanese,
                events,
                endpoint,
            )
            .unwrap();
            client.network = crate::clients::transport::provider_network::ProviderNetwork::resolve(
                &crate::core::services::network_proxy::ProxyConfig {
                    mode: crate::core::services::network_proxy::ProxyMode::Direct,
                    url: None,
                },
            )
            .unwrap();
            let error = client.connect().await.unwrap_err();
            server.await.unwrap();
            if let Some(expected_label) = expected_auth_label {
                assert_eq!(error, VolcanoEngineClientError::AuthenticationFailed);
                assert_eq!(error.to_string(), expected_label);
            } else {
                assert_eq!(error, VolcanoEngineClientError::TransportFailure);
            }
            assert!(!error.to_string().contains("private-handshake-body"));
            assert!(!client.inner.ready.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn clear_removes_pending_text_but_tombstones_its_real_interval() {
        let (sender, mut receiver) = provider_event_channel();
        let client = VolcanoEngineClient::with_endpoint(
            "test-key-not-real",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender.clone(),
            url::Url::parse("ws://127.0.0.1:1/translate").unwrap(),
        )
        .unwrap();
        {
            let mut committer = client.inner.committer.lock().await;
            committer.source_in_progress = true;
            committer.source_draft = "old source preview".into();
            committer.translation_draft = "old translation preview".into();
            committer
                .push_source("old source".into(), Some(0), Some(10))
                .unwrap();
        }
        sender
            .send(LiveTranslateServerEvent::SessionFinished)
            .unwrap();
        assert_eq!(client.clear_content().await, 1);
        assert_eq!(client.content_revision(), 1);
        let mut committer = client.inner.committer.lock().await;
        assert!(committer.sources.is_empty() && committer.translations.is_empty());
        assert!(committer.source_draft.is_empty() && committer.translation_draft.is_empty());
        assert_eq!(committer.pending_text_bytes, 0);
        assert!(committer.discard_source && committer.discard_translation);
        assert!(committer
            .push_translation("late old".into(), Some(0), Some(10))
            .unwrap()
            .is_none());
        assert!(committer
            .push_source("old replay".into(), Some(0), Some(10))
            .unwrap()
            .is_none());
        committer.discard_source = false;
        committer.discard_translation = false;
        committer
            .push_source("new source".into(), Some(11), Some(20))
            .unwrap();
        let pair = committer
            .push_translation("new translation".into(), Some(11), Some(20))
            .unwrap();
        assert!(
            matches!(pair, Some(LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {source,translation,..} | LiveTranslateServerEvent::SubtitleFinalPair {source,translation,..})
            if source == "new source" && translation == "new translation")
        );
        drop(committer);
        assert_eq!(
            receiver.recv().await,
            Some(LiveTranslateServerEvent::SessionFinished)
        );
    }

    use super::*;
    use crate::clients::transport::provider_events::{
        provider_event_channel, ProviderEventReceiver,
    };
    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpListener;

    const EVENT_START_SESSION: u32 = 100;
    const EVENT_FINISH_SESSION: u32 = 102;
    const EVENT_SESSION_STARTED: u32 = 150;
    const EVENT_SESSION_FINISHED: u32 = 152;
    const EVENT_SESSION_FAILED: u32 = 153;
    const EVENT_TASK_REQUEST: u32 = 200;
    const EVENT_SOURCE_SUBTITLE_START: u32 = 650;
    const EVENT_SOURCE_SUBTITLE_RESPONSE: u32 = 651;
    const EVENT_SOURCE_SUBTITLE_END: u32 = 652;
    const EVENT_TRANSLATION_SUBTITLE_START: u32 = 653;
    const EVENT_TRANSLATION_SUBTITLE_RESPONSE: u32 = 654;
    const EVENT_TRANSLATION_SUBTITLE_END: u32 = 655;

    async fn test_client(
        server: impl FnOnce(
                WebSocketStream<TcpStream>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + 'static,
    ) -> (VolcanoEngineClient, ProviderEventReceiver) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server(socket).await;
        });
        let (events, receiver) = provider_event_channel();
        let endpoint = url::Url::parse(&format!("ws://{address}/translate")).unwrap();
        let client = VolcanoEngineClient::with_endpoint(
            "volcano-test-key-not-real",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            events,
            endpoint,
        )
        .unwrap();
        (client, receiver)
    }

    fn write_varint(output: &mut Vec<u8>, mut value: u64) {
        while value >= 0x80 {
            output.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        output.push(value as u8);
    }

    fn write_varint_field(output: &mut Vec<u8>, field: u32, value: u64) {
        write_varint(output, u64::from(field) << 3);
        write_varint(output, value);
    }

    fn write_bytes_field(output: &mut Vec<u8>, field: u32, value: &[u8]) {
        write_varint(output, (u64::from(field) << 3) | 2);
        write_varint(output, value.len() as u64);
        output.extend_from_slice(value);
    }

    fn server_event(event: u64, text: Option<&str>) -> Vec<u8> {
        let mut message = Vec::new();
        write_varint_field(&mut message, 2, event);
        if let Some(text) = text {
            write_bytes_field(&mut message, 4, text.as_bytes());
        }
        message
    }

    fn started_server_event(event: u64, start_time_ms: u64) -> Vec<u8> {
        let mut message = server_event(event, None);
        write_varint_field(&mut message, 5, start_time_ms);
        message
    }

    async fn ownership_context() -> (
        VolcanoEngineClient,
        ReceiveContext,
        ProviderEventReceiver,
        WebSocketStream<TcpStream>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = url::Url::parse(&format!(
            "ws://{}/translate",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let (connected, accepted) =
            tokio::join!(tokio_tungstenite::connect_async(endpoint.clone()), async {
                let (stream, _) = listener.accept().await.unwrap();
                tokio_tungstenite::accept_async(stream).await
            });
        let (socket, _) = connected.unwrap();
        let server = accepted.unwrap();
        let (_, stream) = socket.split();
        let (sender, receiver) = provider_event_channel();
        let client = VolcanoEngineClient::with_endpoint(
            "test-key-not-real",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender.clone(),
            endpoint,
        )
        .unwrap();
        let (setup, _) = watch::channel(SetupState::Ready);
        let context = ReceiveContext {
            inner: Arc::clone(&client.inner),
            stream,
            events: sender,
            setup,
            generation: 0,
        };
        (client, context, receiver, server)
    }

    async fn handle_wire(context: &ReceiveContext, wire: Vec<u8>) {
        assert!(
            !handle_server_event(context, VolcanoEngineServerEvent::decode(&wire).unwrap()).await
        );
    }

    fn drain_into(
        events: &mut ProviderEventReceiver,
        controller: &mut crate::core::lifecycle::session::TranslationSessionController,
    ) {
        while let Ok(event) = events.try_recv() {
            controller.handle(event);
        }
    }

    #[tokio::test]
    async fn ownership_keeps_admitted_and_queued_b_through_late_a_source_and_translation_end() {
        for drain_before_final in [false, true] {
            let (_client, context, mut events, _server) = ownership_context().await;
            let mut controller =
                crate::core::lifecycle::session::TranslationSessionController::default();
            for wire in [
                started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 100),
                server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic A")),
                started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
                server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
                started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, 300),
                server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成 B")),
            ] {
                handle_wire(&context, wire).await;
                if drain_before_final {
                    drain_into(&mut events, &mut controller);
                }
            }
            handle_wire(
                &context,
                timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Synthetic A.", 100, 200),
            )
            .await;
            handle_wire(
                &context,
                timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成 A。", 100, 200),
            )
            .await;
            drain_into(&mut events, &mut controller);
            let snapshot = &controller.state.subtitles;
            assert_eq!(snapshot.source.text, "Synthetic B");
            assert_eq!(snapshot.translation.text, "合成 B");
            assert_eq!(
                snapshot.source.utterance_id,
                snapshot.translation.utterance_id
            );
            assert_eq!(snapshot.history.len(), 1);
            assert_eq!(snapshot.history[0].source, "Synthetic A.");
            assert!(controller.state.is_translation_pending);
            let committer = context.inner.committer.lock().await;
            assert_eq!(committer.source_draft, "Synthetic B");
            assert_eq!(committer.translation_draft, "合成 B");
            assert!(committer.source_in_progress && committer.translation_in_progress);
        }
    }

    #[tokio::test]
    async fn ownership_proven_late_unseen_clock_does_not_become_a_newer_source() {
        let (_client, context, mut events, _server) = ownership_context().await;
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        for wire in [
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
        ] {
            handle_wire(&context, wire).await;
            drain_into(&mut events, &mut controller);
        }
        for wire in [
            timed_server_event(
                EVENT_SOURCE_SUBTITLE_END as u64,
                "Synthetic unseen A.",
                100,
                200,
            ),
            timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成 A。", 100, 200),
        ] {
            handle_wire(&context, wire).await;
            drain_into(&mut events, &mut controller);
        }
        assert_eq!(controller.state.subtitles.source.text, "Synthetic B");
        assert_eq!(controller.state.subtitles.history.len(), 1);
        assert_eq!(
            controller.state.subtitles.history[0].source,
            "Synthetic unseen A."
        );
    }

    #[tokio::test]
    async fn ownership_omitted_zero_a_before_b_preserves_admitted_queued_and_stopping_b() {
        for (drain_before_final, stopping) in [(false, false), (true, false), (true, true)] {
            let (_client, context, mut events, _server) = ownership_context().await;
            let mut controller =
                crate::core::lifecycle::session::TranslationSessionController::default();
            for wire in [
                server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
                server_event(
                    EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                    Some("Synthetic zero A"),
                ),
                started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 1000),
                server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
            ] {
                handle_wire(&context, wire).await;
                if drain_before_final {
                    drain_into(&mut events, &mut controller);
                }
            }
            let b_owner = context.inner.committer.lock().await.source_owner.unwrap();
            if stopping {
                controller.begin_stopping();
            }
            for (event, text) in [
                (EVENT_SOURCE_SUBTITLE_END, "Synthetic zero A."),
                (EVENT_TRANSLATION_SUBTITLE_END, "合成 A。"),
            ] {
                let mut wire = server_event(event as u64, Some(text));
                write_varint_field(&mut wire, 6, 500);
                handle_wire(&context, wire).await;
            }
            drain_into(&mut events, &mut controller);
            let snapshot = &controller.state.subtitles;
            assert_eq!(snapshot.source.text, "Synthetic B");
            assert_eq!(snapshot.history.len(), 1);
            assert_eq!(snapshot.history[0].source, "Synthetic zero A.");
            let pair = snapshot.display_pair.as_ref().unwrap();
            assert_eq!(pair.source, "Synthetic zero A.");
            assert_ne!(snapshot.source.utterance_id, pair.utterance_id);
            let committer = context.inner.committer.lock().await;
            assert_eq!(committer.source_owner, Some(b_owner));
            assert_eq!(committer.source_draft, "Synthetic B");
            assert!(committer.source_in_progress);
            let b = committer
                .source_blocks
                .iter()
                .find(|block| block.owner == b_owner)
                .unwrap();
            assert_eq!(b.start_time_ms, Some(1000));
            assert!(!b.finished);
            assert!(
                committer.owner_for_start(Some(500)).is_none(),
                "END must not become START"
            );
        }
    }

    #[tokio::test]
    async fn ownership_omitted_and_explicit_zero_starts_keep_first_preview_and_late_final_owned() {
        for omitted in [false, true] {
            for translation_first in [false, true] {
                let (_client, context, mut events, _server) = ownership_context().await;
                let mut controller =
                    crate::core::lifecycle::session::TranslationSessionController::default();
                let start = |event| {
                    if omitted {
                        server_event(event, None)
                    } else {
                        started_server_event(event, 0)
                    }
                };
                let source = [
                    start(EVENT_SOURCE_SUBTITLE_START as u64),
                    server_event(
                        EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                        Some("Synthetic zero A"),
                    ),
                ];
                let translation = [
                    start(EVENT_TRANSLATION_SUBTITLE_START as u64),
                    server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成零 A")),
                ];
                let lanes = if translation_first {
                    [translation, source]
                } else {
                    [source, translation]
                };
                for lane in lanes {
                    for wire in lane {
                        handle_wire(&context, wire).await;
                        drain_into(&mut events, &mut controller);
                    }
                }
                let snapshot = &controller.state.subtitles;
                assert_eq!(snapshot.source.text, "Synthetic zero A");
                assert_eq!(snapshot.translation.text, "合成零 A");
                assert!(snapshot.source.utterance_id.is_some());
                assert_eq!(
                    snapshot.source.utterance_id,
                    snapshot.translation.utterance_id
                );
                let preview = snapshot.realtime_preview.as_ref().unwrap();
                assert_eq!(preview.source.text, "Synthetic zero A");
                assert_eq!(preview.translation.text, "合成零 A");
                assert_eq!(
                    preview.source.utterance_id,
                    preview.translation.utterance_id
                );
                for wire in [
                    started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
                    server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
                    started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, 300),
                    server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成 B")),
                ] {
                    handle_wire(&context, wire).await;
                }
                for (event, text) in [
                    (EVENT_SOURCE_SUBTITLE_END, "Synthetic zero A."),
                    (EVENT_TRANSLATION_SUBTITLE_END, "合成零 A。"),
                ] {
                    let mut wire = server_event(event as u64, Some(text));
                    if !omitted {
                        write_varint_field(&mut wire, 5, 0);
                    }
                    write_varint_field(&mut wire, 6, 200);
                    handle_wire(&context, wire).await;
                }
                drain_into(&mut events, &mut controller);
                let snapshot = &controller.state.subtitles;
                assert_eq!(snapshot.source.text, "Synthetic B");
                assert_eq!(snapshot.translation.text, "合成 B");
                assert_eq!(
                    snapshot.source.utterance_id,
                    snapshot.translation.utterance_id
                );
                assert_eq!(snapshot.history.len(), 1);
                assert_eq!(snapshot.history[0].source, "Synthetic zero A.");
                assert_eq!(snapshot.history[0].translation, "合成零 A。");
            }
        }
    }

    #[tokio::test]
    async fn ownership_empty_zero_duration_end_never_confirms_and_stop_keeps_a_real_tail() {
        for stopping in [false, true] {
            for tail_start in [0, 300] {
                let (_client, context, mut events, _server) = ownership_context().await;
                let mut controller =
                    crate::core::lifecycle::session::TranslationSessionController::default();
                for event in [
                    EVENT_SOURCE_SUBTITLE_START,
                    EVENT_TRANSLATION_SUBTITLE_START,
                    EVENT_SOURCE_SUBTITLE_END,
                    EVENT_TRANSLATION_SUBTITLE_END,
                ] {
                    handle_wire(&context, server_event(event as u64, None)).await;
                    drain_into(&mut events, &mut controller);
                }
                assert!(controller.state.subtitles.history.is_empty());
                assert!(controller.state.subtitles.display_pair.is_none());
                for wire in [
                    started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, tail_start),
                    server_event(
                        EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                        Some("Synthetic real tail"),
                    ),
                    started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, tail_start),
                    server_event(
                        EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64,
                        Some("合成真实尾句"),
                    ),
                ] {
                    handle_wire(&context, wire).await;
                    drain_into(&mut events, &mut controller);
                }
                let preview = controller
                    .state
                    .subtitles
                    .realtime_preview
                    .as_ref()
                    .unwrap();
                assert_eq!(preview.source.text, "Synthetic real tail");
                assert_eq!(preview.translation.text, "合成真实尾句");
                assert_eq!(
                    preview.source.utterance_id,
                    preview.translation.utterance_id
                );
                if stopping {
                    controller.begin_stopping();
                }
                for wire in [
                    server_event(EVENT_SOURCE_SUBTITLE_END as u64, None),
                    server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, None),
                    timed_server_event(
                        EVENT_SOURCE_SUBTITLE_END as u64,
                        "Synthetic real tail.",
                        tail_start,
                        400,
                    ),
                    timed_server_event(
                        EVENT_TRANSLATION_SUBTITLE_END as u64,
                        "合成真实尾句。",
                        tail_start,
                        400,
                    ),
                ] {
                    handle_wire(&context, wire).await;
                    drain_into(&mut events, &mut controller);
                }
                assert_eq!(controller.state.subtitles.history.len(), 1);
                assert_eq!(
                    controller.state.subtitles.history[0].source,
                    "Synthetic real tail."
                );
                assert!(controller.state.subtitles.display_pair_final);
            }
        }
    }

    #[tokio::test]
    async fn ownership_zero_start_cannot_reopen_pending_confirmed_or_older_source() {
        let (_client, context, mut events, _server) = ownership_context().await;
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        for wire in [
            server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
            server_event(
                EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                Some("Synthetic zero A"),
            ),
            timed_server_event(
                EVENT_SOURCE_SUBTITLE_END as u64,
                "Synthetic zero A.",
                0,
                200,
            ),
            server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
            server_event(
                EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                Some("Synthetic pending replay"),
            ),
        ] {
            handle_wire(&context, wire).await;
            drain_into(&mut events, &mut controller);
        }
        assert_eq!(controller.state.subtitles.source.text, "Synthetic zero A.");
        assert!(context.inner.committer.lock().await.discard_source);
        handle_wire(
            &context,
            timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成零 A。", 0, 200),
        )
        .await;
        drain_into(&mut events, &mut controller);
        for wire in [
            server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
            server_event(
                EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                Some("Synthetic confirmed replay"),
            ),
        ] {
            handle_wire(&context, wire).await;
            drain_into(&mut events, &mut controller);
        }
        assert_eq!(controller.state.subtitles.history.len(), 1);
        assert_eq!(controller.state.subtitles.source.text, "Synthetic zero A.");
        for wire in [
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
            server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
            server_event(
                EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                Some("Synthetic older replay"),
            ),
        ] {
            handle_wire(&context, wire).await;
            drain_into(&mut events, &mut controller);
        }
        assert_eq!(controller.state.subtitles.source.text, "Synthetic B");
        assert_eq!(
            context.inner.committer.lock().await.source_draft,
            "Synthetic B"
        );
    }

    #[tokio::test]
    async fn ownership_internal_unknown_start_does_not_claim_late_a_translation_end() {
        let (_client, context, mut events, _server) = ownership_context().await;
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        for wire in [
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 100),
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
        ] {
            handle_wire(&context, wire).await;
            drain_into(&mut events, &mut controller);
        }
        // Internal unknown timing is a synthetic compatibility case. An
        // omitted scalar in an actual START wire frame means zero instead.
        assert!(
            !handle_server_event(
                &context,
                VolcanoEngineServerEvent::TranslationSubtitleStarted {
                    start_time_ms: None
                }
            )
            .await
        );
        for wire in [
            server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("匿名 B")),
            timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Synthetic A.", 100, 200),
            timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成 A。", 100, 200),
        ] {
            handle_wire(&context, wire).await;
            drain_into(&mut events, &mut controller);
        }
        assert_eq!(controller.state.subtitles.source.text, "Synthetic B");
        assert_eq!(controller.state.subtitles.history.len(), 1);
        {
            let committer = context.inner.committer.lock().await;
            assert_eq!(committer.translation_draft, "匿名 B");
            assert!(committer.translation_in_progress);
            assert!(committer.translation_owner.is_none());
            assert!(committer.translation_start_time_ms.is_none());
        }
        handle_wire(
            &context,
            server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some(" 后续")),
        )
        .await;
        let committer = context.inner.committer.lock().await;
        assert_eq!(committer.translation_draft, "匿名 B 后续");
        assert!(committer.translation_owner.is_none());
    }

    #[tokio::test]
    async fn ownership_no_start_legacy_end_barrier_keeps_successive_blocks_separate() {
        let (_client, context, mut events, _server) = ownership_context().await;
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        for (source, translation, start, end) in [
            ("Synthetic A", "合成 A", 100, 200),
            ("Synthetic B", "合成 B", 300, 400),
        ] {
            handle_wire(
                &context,
                server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some(source)),
            )
            .await;
            handle_wire(
                &context,
                server_event(
                    EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64,
                    Some(translation),
                ),
            )
            .await;
            drain_into(&mut events, &mut controller);
            assert_eq!(controller.state.subtitles.source.text, source);
            assert_eq!(controller.state.subtitles.translation.text, translation);
            assert!(controller.state.subtitles.source.utterance_id.is_none());
            assert!(controller
                .state
                .subtitles
                .translation
                .utterance_id
                .is_none());
            handle_wire(
                &context,
                timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, source, start, end),
            )
            .await;
            handle_wire(
                &context,
                timed_server_event(
                    EVENT_TRANSLATION_SUBTITLE_END as u64,
                    translation,
                    start,
                    end,
                ),
            )
            .await;
            drain_into(&mut events, &mut controller);
            assert_eq!(controller.state.subtitles.source.text, source);
            assert_eq!(controller.state.subtitles.translation.text, translation);
            let committer = context.inner.committer.lock().await;
            assert!(committer.source_draft.is_empty() && committer.translation_draft.is_empty());
            assert!(!committer.source_in_progress && !committer.translation_in_progress);
            assert!(committer.translation_owner.is_none());
            assert!(committer.translation_start_time_ms.is_none());
            assert!(
                committer.last_matched_source_owner.is_none(),
                "legacy END cannot infer ownership"
            );
        }
        assert_eq!(controller.state.subtitles.history.len(), 2);
        assert_eq!(controller.state.subtitles.history[1].source, "Synthetic B");
    }

    #[tokio::test]
    async fn ownership_empty_boundary_retains_completed_a_without_labeling_its_text_b() {
        let (_client, context, mut events, _server) = ownership_context().await;
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        for wire in [
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 100),
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic A")),
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
            timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Synthetic A.", 100, 200),
            timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成 A。", 100, 200),
        ] {
            handle_wire(&context, wire).await;
            drain_into(&mut events, &mut controller);
        }
        let snapshot = &controller.state.subtitles;
        assert_eq!(snapshot.source.text, "Synthetic A.");
        let pair = snapshot.display_pair.as_ref().unwrap();
        assert_eq!(pair.source, "Synthetic A.");
        assert_eq!(snapshot.source.utterance_id, pair.utterance_id);
        assert!(snapshot.display_pair_final);
    }

    #[tokio::test]
    async fn ownership_translation_first_start_binds_only_when_source_time_is_proved() {
        let (_client, context, mut events, _server) = ownership_context().await;
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        for wire in [
            started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, 0),
            server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成")),
        ] {
            handle_wire(&context, wire).await;
            drain_into(&mut events, &mut controller);
        }
        assert!(controller
            .state
            .subtitles
            .translation
            .utterance_id
            .is_none());
        for wire in [
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic")),
        ] {
            handle_wire(&context, wire).await;
            drain_into(&mut events, &mut controller);
        }
        let snapshot = &controller.state.subtitles;
        assert!(snapshot.source.utterance_id.is_some());
        assert_eq!(
            snapshot.source.utterance_id,
            snapshot.translation.utterance_id
        );
        assert_eq!(snapshot.translation.text, "合成");
    }

    #[test]
    fn ownership_internal_unknown_time_never_guesses_the_translation_owner() {
        let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(committer.start_source(None));
        let owner = committer.source_owner.unwrap();
        assert!(committer.start_translation(Some(100)));
        assert!(committer.translation_owner.is_none());
        assert_eq!(committer.source_end_owner(Some(100), Some(200)), None);
        assert_eq!(committer.translation_owner, None);
        assert_eq!(committer.source_owner, Some(owner));
        assert_eq!(committer.source_blocks[0].start_time_ms, None);
        assert!(!committer.source_blocks[0].finished);
        committer.source_draft = "Synthetic anonymous B".into();
        committer
            .push_source("Synthetic unknown A.".into(), Some(100), Some(200))
            .unwrap();
        assert!(matches!(
            committer
                .push_translation("合成 A。".into(), Some(100), Some(200))
                .unwrap(),
            Some(LiveTranslateServerEvent::SubtitleFinalPair { .. })
        ));
        assert_eq!(committer.source_draft, "Synthetic anonymous B");
        assert!(!committer.source_blocks[0].finished);
        committer.reset();
        let owner = committer.ensure_source_owner().unwrap();
        committer.translation_draft = "Unowned synthetic translation".into();
        committer.bind_translation_owner();
        assert!(committer.translation_owner.is_none());
        assert!(matches!(
            committer.translation_text_event(),
            LiveTranslateServerEvent::TranslationDraft(_)
        ));
        assert_eq!(committer.source_end_owner(Some(0), None), None);
        committer.bind_translation_owner();
        assert!(committer.translation_owner.is_none());
        assert_eq!(committer.source_blocks[0].start_time_ms, None);
        assert!(matches!(
            committer.source_text_event(owner, "Synthetic".into(), false),
            LiveTranslateServerEvent::SourceDraft { .. }
        ));
        assert!(committer.start_source(Some(0)));
        assert!(committer.start_translation(Some(0)));
        assert_eq!(committer.translation_owner, committer.source_owner);
    }

    #[test]
    fn ownership_duplicate_and_retired_starts_preserve_bounded_buffers() {
        let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(committer.start_source(Some(0)));
        committer.source_draft = "Synthetic".into();
        assert!(!committer.start_source(Some(0)));
        assert_eq!(committer.source_draft, "Synthetic");
        assert!(committer.start_translation(Some(0)));
        committer.translation_draft = "合成".into();
        assert!(!committer.start_translation(Some(0)));
        assert_eq!(committer.translation_draft, "合成");
        for start in 0..(MAXIMUM_SOURCE_BLOCKS as i32 * 4) {
            let start = start * 10;
            committer.start_source(Some(start));
            committer
                .push_source("S".into(), Some(start), Some(start + 1))
                .unwrap();
            committer
                .push_translation("T".into(), Some(start), Some(start + 1))
                .unwrap();
        }
        assert!(committer.source_blocks.len() <= MAXIMUM_SOURCE_BLOCKS);
        assert!(committer.retired_starts.len() <= MAXIMUM_SOURCE_BLOCKS);
        assert!(committer.recently_committed.len() <= MAXIMUM_RECENT_COMMITTED_TIMINGS);
        assert!(!committer.start_source(Some(0)));
        assert!(!committer.start_translation(Some(0)));
        committer.reset();
        committer.next_source_owner = u64::from(u32::MAX);
        assert!(committer.new_source_owner(Some(0)).is_none());
        assert!(committer.source_blocks.is_empty());
    }

    #[tokio::test]
    async fn ownership_retired_start_conservatively_drops_unidentified_drafts_until_known_start() {
        let (_client, context, mut events, _server) = ownership_context().await;
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        for wire in [
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
            timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Synthetic A.", 0, 10),
            timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成 A。", 0, 10),
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 20),
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic B")),
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
            server_event(
                EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                Some(" ambiguous tail"),
            ),
        ] {
            handle_wire(&context, wire).await;
            drain_into(&mut events, &mut controller);
        }
        assert_eq!(controller.state.subtitles.source.text, "Synthetic B");
        assert_eq!(
            context.inner.committer.lock().await.source_draft,
            "Synthetic B"
        );
        // With no ID on RESPONSE, the old A tail and a new B tail are
        // indistinguishable. Preserve B's buffer but suppress both until START.
        handle_wire(
            &context,
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 20),
        )
        .await;
        handle_wire(
            &context,
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some(" resumed")),
        )
        .await;
        drain_into(&mut events, &mut controller);
        assert_eq!(
            controller.state.subtitles.source.text,
            "Synthetic B resumed"
        );
    }

    #[tokio::test]
    async fn ownership_clear_and_reconnect_reject_retired_frames_and_refresh_epoch() {
        let (client, mut context, mut events, _server) = ownership_context().await;
        handle_wire(
            &context,
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
        )
        .await;
        handle_wire(
            &context,
            server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Synthetic old")),
        )
        .await;
        let old_id = context.inner.committer.lock().await.owner_id(1);
        while events.try_recv().is_ok() {}
        client.clear_content().await;
        for wire in [
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
            server_event(
                EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                Some("Synthetic replay"),
            ),
            timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Synthetic old.", 0, 10),
            timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "合成旧句。", 0, 10),
        ] {
            handle_wire(&context, wire).await;
        }
        assert!(events.try_recv().is_err());
        handle_wire(
            &context,
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 20),
        )
        .await;
        assert!(events.try_recv().is_ok());
        client.disconnect().await;
        handle_wire(
            &context,
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 30),
        )
        .await;
        assert!(events.try_recv().is_err());
        context.generation = context.inner.generation.load(Ordering::SeqCst);
        handle_wire(
            &context,
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 0),
        )
        .await;
        let new_id = context.inner.committer.lock().await.owner_id(1);
        assert_ne!(old_id, new_id);
        assert!(events.try_recv().is_ok());
    }

    fn timed_server_event(event: u64, text: &str, start_time_ms: u64, end_time_ms: u64) -> Vec<u8> {
        let mut message = server_event(event, Some(text));
        write_varint_field(&mut message, 5, start_time_ms);
        write_varint_field(&mut message, 6, end_time_ms);
        message
    }

    fn failed_server_event(status: u64, private_message: &str) -> Vec<u8> {
        let mut meta = Vec::new();
        write_varint_field(&mut meta, 3, status);
        write_bytes_field(&mut meta, 4, private_message.as_bytes());
        let mut message = Vec::new();
        write_bytes_field(&mut message, 1, &meta);
        write_varint_field(&mut message, 2, EVENT_SESSION_FAILED as u64);
        message
    }

    fn read_varint(input: &mut &[u8]) -> Option<u64> {
        let mut value = 0_u64;
        for shift in (0..70).step_by(7) {
            let byte = *input.first()?;
            *input = &input[1..];
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn find_varint(message: &[u8], wanted_field: u32) -> Option<u64> {
        let mut input = message;
        while !input.is_empty() {
            let key = read_varint(&mut input)?;
            let field = u32::try_from(key >> 3).ok()?;
            let wire = (key & 7) as u8;
            match wire {
                0 => {
                    let value = read_varint(&mut input)?;
                    if field == wanted_field {
                        return Some(value);
                    }
                }
                2 => {
                    let length = usize::try_from(read_varint(&mut input)?).ok()?;
                    if length > input.len() {
                        return None;
                    }
                    input = &input[length..];
                }
                _ => return None,
            }
        }
        None
    }

    fn find_bytes(message: &[u8], wanted_field: u32) -> Option<&[u8]> {
        let mut input = message;
        while !input.is_empty() {
            let key = read_varint(&mut input)?;
            let field = u32::try_from(key >> 3).ok()?;
            let wire = (key & 7) as u8;
            match wire {
                0 => {
                    read_varint(&mut input)?;
                }
                2 => {
                    let length = usize::try_from(read_varint(&mut input)?).ok()?;
                    if length > input.len() {
                        return None;
                    }
                    let (value, rest) = input.split_at(length);
                    input = rest;
                    if field == wanted_field {
                        return Some(value);
                    }
                }
                _ => return None,
            }
        }
        None
    }

    fn binary(message: Message) -> Vec<u8> {
        match message {
            Message::Binary(frame) => frame.to_vec(),
            other => panic!("expected binary protobuf frame, got {other:?}"),
        }
    }

    fn assert_pair(event: SubtitleCommitResult, expected_source: &str, expected_translation: &str) {
        assert!(matches!(
            event,
            Ok(Some(LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                source,
                language,
                translation,
                ..
            } | LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                language,
                translation,
                ..
            })) if source == expected_source
                && language.as_deref() == Some("en")
                && translation == expected_translation
        ));
    }

    #[test]
    fn final_pair_committer_accepts_either_final_arrival_order() {
        let mut source_first = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(source_first
            .push_source("one".into(), Some(10), Some(90))
            .unwrap()
            .is_none());
        assert_pair(
            source_first.push_translation("一".into(), Some(10), Some(90)),
            "one",
            "一",
        );

        let mut translation_first = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(translation_first
            .push_translation("二".into(), Some(100), Some(190))
            .unwrap()
            .is_none());
        assert_pair(
            translation_first.push_source("two".into(), Some(100), Some(190)),
            "two",
            "二",
        );
    }

    #[test]
    fn timing_identity_prevents_cross_sentence_pairing_during_reordering() {
        let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(committer
            .push_source("sentence A".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert!(committer
            .push_source("sentence B".into(), Some(100), Some(200))
            .unwrap()
            .is_none());

        // B completes first, followed by A. Each translation must select the
        // source with the same official input interval rather than the oldest
        // or most recently received source.
        assert_pair(
            committer.push_translation("译文 B".into(), Some(100), Some(200)),
            "sentence B",
            "译文 B",
        );
        assert_pair(
            committer.push_translation("译文 A".into(), Some(0), Some(100)),
            "sentence A",
            "译文 A",
        );
    }

    #[test]
    fn mismatched_or_incomplete_tails_are_never_forged_into_a_pair() {
        let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(committer
            .push_source("old source".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert!(committer
            .push_translation("new translation".into(), Some(100), Some(200))
            .unwrap()
            .is_none());
        assert!(committer
            .push_source("missing timing".into(), None, None)
            .unwrap()
            .is_none());

        committer.reset();
        assert!(committer.sources.is_empty());
        assert!(committer.translations.is_empty());
        assert!(committer
            .push_translation("late unmatched tail".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_committed_timing_cannot_emit_a_duplicate_pair() {
        let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        assert!(committer
            .push_source("once".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert_pair(
            committer.push_translation("一次".into(), Some(0), Some(100)),
            "once",
            "一次",
        );
        assert!(committer
            .push_source("duplicate source".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert!(committer
            .push_translation("重复译文".into(), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert!(committer.sources.is_empty());
        assert!(committer.translations.is_empty());
    }

    #[test]
    fn pending_final_text_budget_resets_and_allows_the_next_pair() {
        let mut committer = VolcanoSubtitlePairCommitter::new(SourceLanguage::English);
        let oversized_half = MAXIMUM_PENDING_FINAL_TEXT_BYTES / 2 + 1;
        assert!(committer
            .push_source("s".repeat(oversized_half), Some(0), Some(100))
            .unwrap()
            .is_none());
        assert_eq!(committer.pending_text_bytes, oversized_half);

        assert_eq!(
            committer.push_translation("t".repeat(oversized_half), Some(100), Some(200),),
            Err(PendingFinalTextLimitExceeded)
        );
        assert!(committer.sources.is_empty());
        assert!(committer.translations.is_empty());
        assert_eq!(committer.pending_text_bytes, 0);

        assert!(committer
            .push_source("next sentence".into(), Some(200), Some(300))
            .unwrap()
            .is_none());
        assert_pair(
            committer.push_translation("次の文".into(), Some(200), Some(300)),
            "next sentence",
            "次の文",
        );
        assert_eq!(committer.pending_text_bytes, 0);
    }

    #[test]
    fn constructor_validates_languages_without_exposing_the_key() {
        let secret = "volcano-private-key-never-log";
        let (events, _receiver) = provider_event_channel();
        let result = VolcanoEngineClient::with_endpoint(
            secret,
            SourceLanguage::Automatic,
            TargetLanguage::English,
            events,
            url::Url::parse("ws://127.0.0.1:9/translate").unwrap(),
        );
        let error = match result {
            Ok(_) => panic!("automatic source language must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error, VolcanoEngineClientError::UnsupportedSourceLanguage);
        assert!(!format!("{error:?}").contains(secret));
    }

    #[allow(clippy::result_large_err)]
    #[tokio::test]
    async fn websocket_upgrade_uses_the_current_api_key_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(
                        request.headers().get("X-Api-Key").unwrap(),
                        "volcano-current-api-key-not-real"
                    );
                    assert_eq!(
                        request.headers().get("X-Api-Resource-Id").unwrap(),
                        VolcanoEngineEndpoint::RESOURCE_ID
                    );
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let _ = socket.next().await;
            socket
                .send(Message::Binary(
                    server_event(EVENT_SESSION_STARTED as u64, None).into(),
                ))
                .await
                .unwrap();
            while socket.next().await.is_some() {}
        });

        let (events, _receiver) = provider_event_channel();
        let client = VolcanoEngineClient::with_endpoint(
            "volcano-current-api-key-not-real",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            events,
            url::Url::parse(&format!("ws://{address}/translate")).unwrap(),
        )
        .unwrap();
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn connect_waits_for_the_official_session_started_event() {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                let start = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&start, 2), Some(EVENT_START_SESSION as u64));
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
                while socket.next().await.is_some() {}
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.disconnect().await;
    }

    #[tokio::test]
    async fn offline_reading_boundary_volcano_staggered_drafts_preserve_newer_owner() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = url::Url::parse(&format!(
            "ws://{}/translate",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let (connected, accepted) =
            tokio::join!(tokio_tungstenite::connect_async(endpoint.clone()), async {
                let (stream, _) = listener.accept().await.unwrap();
                tokio_tungstenite::accept_async(stream).await
            });
        let (socket, _) = connected.unwrap();
        let _server = accepted.unwrap();
        let (_, stream) = socket.split();
        let (sender, mut events) = provider_event_channel();
        let client = VolcanoEngineClient::with_endpoint(
            "test-key-not-real",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            sender.clone(),
            endpoint,
        )
        .unwrap();
        let (setup, _) = watch::channel(SetupState::Ready);
        let context = ReceiveContext {
            inner: Arc::clone(&client.inner),
            stream,
            events: sender,
            setup,
            generation: 0,
        };
        let mut controller =
            crate::core::lifecycle::session::TranslationSessionController::default();
        let frames = [
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 100),
            server_event(
                EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                Some("Synthetic sentence A"),
            ),
            timed_server_event(
                EVENT_SOURCE_SUBTITLE_END as u64,
                "Synthetic sentence A.",
                100,
                200,
            ),
            started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, 100),
            server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成訳 A")),
            started_server_event(EVENT_SOURCE_SUBTITLE_START as u64, 300),
            server_event(
                EVENT_SOURCE_SUBTITLE_RESPONSE as u64,
                Some("Synthetic sentence B"),
            ),
            timed_server_event(
                EVENT_TRANSLATION_SUBTITLE_END as u64,
                "合成訳 A。",
                100,
                200,
            ),
            started_server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, 300),
            server_event(EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64, Some("合成訳 B")),
            timed_server_event(
                EVENT_SOURCE_SUBTITLE_END as u64,
                "Synthetic sentence B.",
                300,
                400,
            ),
            timed_server_event(
                EVENT_TRANSLATION_SUBTITLE_END as u64,
                "合成訳 B。",
                300,
                400,
            ),
        ];
        for (index, frame) in frames.into_iter().enumerate() {
            assert!(
                !handle_server_event(&context, VolcanoEngineServerEvent::decode(&frame).unwrap())
                    .await
            );
            while let Ok(event) = events.try_recv() {
                controller.handle(event);
            }
            if index == 6 {
                // A's owned translation cannot accompany B's current original.
                let preview = controller
                    .state
                    .subtitles
                    .realtime_preview
                    .as_ref()
                    .unwrap();
                assert_eq!(preview.source.text, "Synthetic sentence B");
                assert_eq!(preview.translation.text, "");
                assert!(preview.source.utterance_id.is_some());
                assert!(controller.state.subtitles.history.is_empty());
            }
            if index == 7 {
                // A stays durable while the newer B original remains live.
                assert_eq!(
                    controller.state.subtitles.source.text,
                    "Synthetic sentence B"
                );
                assert_eq!(controller.state.subtitles.history.len(), 1);
            }
        }
        let history = &controller.state.subtitles.history;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].source, "Synthetic sentence A.");
        assert_eq!(history[0].translation, "合成訳 A。");
        assert_eq!(history[1].source, "Synthetic sentence B.");
        assert_eq!(history[1].translation, "合成訳 B。");
        assert_eq!(
            controller.state.subtitles.source.text,
            "Synthetic sentence B."
        );
        assert_eq!(controller.state.subtitles.translation.text, "合成訳 B。");
    }

    #[tokio::test]
    async fn shared_incremental_subtitle_sequences_keep_full_previews_and_sentence_boundaries() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../../../../translation-contracts.json")).unwrap();
        for case in contract["volcanoSubtitleSequences"].as_array().unwrap() {
            let frames = case["frames"]
                .as_array()
                .unwrap()
                .iter()
                .map(|hex| {
                    let hex = hex.as_str().unwrap();
                    (0..hex.len())
                        .step_by(2)
                        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = url::Url::parse(&format!(
                "ws://{}/translate",
                listener.local_addr().unwrap()
            ))
            .unwrap();
            let (connected, accepted) =
                tokio::join!(tokio_tungstenite::connect_async(endpoint.clone()), async {
                    let (stream, _) = listener.accept().await.unwrap();
                    tokio_tungstenite::accept_async(stream).await
                });
            let (socket, _) = connected.unwrap();
            let _server = accepted.unwrap();
            let (_, stream) = socket.split();
            let (sender, mut events) = provider_event_channel();
            let client = VolcanoEngineClient::with_endpoint(
                "test-key-not-real",
                SourceLanguage::Japanese,
                TargetLanguage::SimplifiedChinese,
                sender.clone(),
                endpoint,
            )
            .unwrap();
            let (setup, _) = watch::channel(SetupState::Ready);
            let context = ReceiveContext {
                inner: Arc::clone(&client.inner),
                stream,
                events: sender,
                setup,
                generation: 0,
            };
            let mut source_drafts = Vec::new();
            let mut translation_drafts = Vec::new();
            let mut source_finals = Vec::new();
            let mut translation_finals = Vec::new();
            let expected = &case["expected"];
            // Drain after each real handler call. Draft transport intentionally
            // coalesces snapshots, so a burst socket test cannot assert every preview.
            for frame in frames {
                assert!(
                    !handle_server_event(
                        &context,
                        VolcanoEngineServerEvent::decode(&frame).unwrap()
                    )
                    .await
                );
                while let Ok(event) = events.try_recv() {
                    match event {
                        LiveTranslateServerEvent::UtteranceText {
                            role: UtteranceRole::Source,
                            is_final: false,
                            text,
                            ..
                        }
                        | LiveTranslateServerEvent::SourceDraft { text, .. } => {
                            source_drafts.push(text)
                        }
                        LiveTranslateServerEvent::UtteranceText {
                            role: UtteranceRole::Translation,
                            is_final: false,
                            text,
                            ..
                        }
                        | LiveTranslateServerEvent::TranslationDraft(text) => {
                            translation_drafts.push(text)
                        }
                        LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                            source,
                            translation,
                            ..
                        }
                        | LiveTranslateServerEvent::SubtitleFinalPair {
                            source,
                            translation,
                            ..
                        } => {
                            source_finals.push(source);
                            translation_finals.push(translation);
                        }
                        LiveTranslateServerEvent::Error { code, .. } => {
                            panic!("unexpected failure {code}")
                        }
                        _ => {}
                    }
                }
            }
            assert_eq!(
                serde_json::json!({
                    "sourceDrafts": source_drafts, "translationDrafts": translation_drafts,
                    "sourceFinals": source_finals, "translationFinals": translation_finals,
                }),
                *expected,
                "{}",
                case["id"]
            );
        }
    }

    #[tokio::test]
    async fn incremental_draft_overflow_is_bounded_on_both_sides() {
        for event in [
            EVENT_SOURCE_SUBTITLE_RESPONSE,
            EVENT_TRANSLATION_SUBTITLE_RESPONSE,
        ] {
            for character in ["a", "日"] {
                let (client, mut events) = test_client(move |mut socket| {
                    Box::pin(async move {
                        socket.next().await.unwrap().unwrap();
                        socket
                            .send(Message::Binary(
                                server_event(EVENT_SESSION_STARTED as u64, None).into(),
                            ))
                            .await
                            .unwrap();
                        socket.next().await.unwrap().unwrap();
                        for text in [
                            character.repeat(
                                crate::core::models::MAX_SUBTITLE_TEXT_BYTES / character.len(),
                            ),
                            character.into(),
                        ] {
                            socket
                                .send(Message::Binary(
                                    server_event(event as u64, Some(&text)).into(),
                                ))
                                .await
                                .unwrap();
                        }
                        while socket.next().await.is_some() {}
                    })
                })
                .await;
                client.connect().await.unwrap();
                client
                    .send_audio(&vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT])
                    .await
                    .unwrap();
                loop {
                    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                        .await
                        .unwrap()
                        .unwrap();
                    if let LiveTranslateServerEvent::Error { code, .. } = event {
                        assert_eq!(code, "volcano_transcript_safety_limit");
                        break;
                    }
                }
                client.disconnect().await;
            }
        }
    }

    #[tokio::test]
    async fn empty_subtitle_events_do_not_disconnect_or_block_the_next_final_pair() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                socket.next().await.unwrap().unwrap();
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
                // Send only after connect has completed, as with normal audio.
                socket.next().await.unwrap().unwrap();
                for event in [
                    EVENT_SOURCE_SUBTITLE_RESPONSE,
                    EVENT_SOURCE_SUBTITLE_END,
                    EVENT_TRANSLATION_SUBTITLE_RESPONSE,
                    EVENT_TRANSLATION_SUBTITLE_END,
                ] {
                    for text in [None, Some("")] {
                        socket
                            .send(Message::Binary(server_event(event as u64, text).into()))
                            .await
                            .unwrap();
                    }
                }
                for response in [
                    timed_server_event(
                        EVENT_SOURCE_SUBTITLE_END as u64,
                        "Synthetic source.",
                        100,
                        200,
                    ),
                    timed_server_event(
                        EVENT_TRANSLATION_SUBTITLE_END as u64,
                        "合成テスト。",
                        100,
                        200,
                    ),
                ] {
                    socket.send(Message::Binary(response.into())).await.unwrap();
                }
                let finish = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_FINISHED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
            })
        })
        .await;
        client.connect().await.unwrap();
        client
            .send_audio(&vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap()
                .unwrap();
            match event {
                LiveTranslateServerEvent::Error { code, .. } => panic!("unexpected failure {code}"),
                LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                    source,
                    translation,
                    ..
                }
                | LiveTranslateServerEvent::SubtitleFinalPair {
                    source,
                    translation,
                    ..
                } => {
                    assert_eq!(source, "Synthetic source.");
                    assert_eq!(translation, "合成テスト。");
                    break;
                }
                _ => {}
            }
        }
        client.finish(Duration::from_secs(1)).await;
        let remaining = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
        assert!(remaining
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!remaining.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::Error { .. }
                | LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
                | LiveTranslateServerEvent::SubtitleFinalPair { .. }
        )));
        client.disconnect().await;
    }

    #[tokio::test]
    async fn full_mock_lifecycle_frames_audio_and_emits_subtitles() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let start = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&start, 2), Some(EVENT_START_SESSION as u64));
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();

                let audio = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&audio, 2), Some(EVENT_TASK_REQUEST as u64));
                let source_audio = find_bytes(&audio, 4).unwrap();
                let pcm = find_bytes(source_audio, 14).unwrap();
                assert_eq!(pcm.len(), VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT);
                assert_eq!(&pcm[..3], &[1, 2, 3]);
                assert!(pcm[3..].iter().all(|byte| *byte == 0));

                for response in [
                    server_event(EVENT_SOURCE_SUBTITLE_START as u64, None),
                    server_event(EVENT_SOURCE_SUBTITLE_RESPONSE as u64, Some("Hello")),
                ] {
                    socket.send(Message::Binary(response.into())).await.unwrap();
                }

                let finish = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));
                for response in [
                    server_event(EVENT_TRANSLATION_SUBTITLE_START as u64, None),
                    server_event(
                        EVENT_TRANSLATION_SUBTITLE_RESPONSE as u64,
                        Some("こんにちは"),
                    ),
                    // Finals can arrive in either order. Sentence timing, not
                    // arrival order, is the pairing identity.
                    timed_server_event(
                        EVENT_TRANSLATION_SUBTITLE_END as u64,
                        "こんにちは。",
                        1_000,
                        1_800,
                    ),
                    timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "Hello.", 1_000, 1_800),
                    server_event(EVENT_SESSION_FINISHED as u64, None),
                ] {
                    socket.send(Message::Binary(response.into())).await.unwrap();
                }
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        let mut pcm = vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT];
        pcm[..3].copy_from_slice(&[1, 2, 3]);
        client.send_audio(&pcm).await.unwrap();

        let mut received = Vec::new();
        loop {
            let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
                .await
                .unwrap()
                .unwrap();
            let saw_source_draft = matches!(
                &event,
                LiveTranslateServerEvent::UtteranceText { role: UtteranceRole::Source, text, language, is_final: false, .. } | LiveTranslateServerEvent::SourceDraft {text, language, ..}
                    if text == "Hello" && language.as_deref() == Some("en")
            );
            received.push(event);
            if saw_source_draft {
                break;
            }
        }

        client.finish(Duration::from_secs(1)).await;
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::UtteranceText { role: UtteranceRole::Source, text, language, is_final: false, .. } | LiveTranslateServerEvent::SourceDraft {text, language, ..}
                if text == "Hello" && language.as_deref() == Some("en")
        )));
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { source, language, translation, .. } | LiveTranslateServerEvent::SubtitleFinalPair { source, language, translation, .. }
                if source == "Hello."
                    && language.as_deref() == Some("en")
                    && translation == "こんにちは。"
        )));
        let pair_index = received
            .iter()
            .position(|event| {
                matches!(
                    event,
                    LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
                        | LiveTranslateServerEvent::SubtitleFinalPair { .. }
                )
            })
            .unwrap();
        let finished_index = received
            .iter()
            .position(|event| matches!(event, LiveTranslateServerEvent::SessionFinished))
            .unwrap();
        assert!(pair_index < finished_index);
        assert!(!received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SourceFinal { .. }
                | LiveTranslateServerEvent::TranslationFinal(_)
        )));
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::Error { .. })));
    }

    #[tokio::test]
    async fn graceful_finish_drops_an_incomplete_confirmed_tail() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
                let finish = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));
                socket
                    .send(Message::Binary(
                        timed_server_event(
                            EVENT_SOURCE_SUBTITLE_END as u64,
                            "unmatched final source",
                            2_000,
                            2_800,
                        )
                        .into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_FINISHED as u64, None).into(),
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
        client.finish(Duration::from_secs(1)).await;

        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
                | LiveTranslateServerEvent::SubtitleFinalPair { .. }
                | LiveTranslateServerEvent::SourceFinal { .. }
                | LiveTranslateServerEvent::TranslationFinal(_)
        )));
    }

    #[tokio::test]
    async fn unsolicited_session_finished_is_a_safe_local_error() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();

                let audio = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&audio, 2), Some(EVENT_TASK_REQUEST as u64));
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_FINISHED as u64, None).into(),
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
        client
            .send_audio(&vec![0; VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT])
            .await
            .unwrap();

        let mut received = Vec::new();
        loop {
            let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
                .await
                .unwrap()
                .unwrap();
            let is_error = matches!(
                &event,
                LiveTranslateServerEvent::Error { code, message }
                    if code == "volcano_unexpected_session_finished"
                        && message == UNEXPECTED_SESSION_FINISHED_ERROR
            );
            received.push(event);
            if is_error {
                break;
            }
        }
        assert!(!received
            .iter()
            .any(|event| matches!(event, LiveTranslateServerEvent::SessionFinished)));
        assert!(!client.inner.ready.load(Ordering::SeqCst));
        client.disconnect().await;
    }

    #[tokio::test]
    async fn oversized_unmatched_final_buffer_emits_safe_error_and_recovers() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
                let finish = binary(socket.next().await.unwrap().unwrap());
                assert_eq!(find_varint(&finish, 2), Some(EVENT_FINISH_SESSION as u64));

                let oversized_half = MAXIMUM_PENDING_FINAL_TEXT_BYTES / 2 + 1;
                for response in [
                    timed_server_event(
                        EVENT_SOURCE_SUBTITLE_END as u64,
                        &"private source".repeat(oversized_half / "private source".len() + 1),
                        0,
                        100,
                    ),
                    timed_server_event(
                        EVENT_TRANSLATION_SUBTITLE_END as u64,
                        &"private translation"
                            .repeat(oversized_half / "private translation".len() + 1),
                        100,
                        200,
                    ),
                    timed_server_event(EVENT_SOURCE_SUBTITLE_END as u64, "next sentence", 200, 300),
                    timed_server_event(EVENT_TRANSLATION_SUBTITLE_END as u64, "次の文", 200, 300),
                    server_event(EVENT_SESSION_FINISHED as u64, None),
                ] {
                    socket.send(Message::Binary(response.into())).await.unwrap();
                }
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        client.finish(Duration::from_secs(1)).await;

        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::Error { code, message }
                if code == "volcano_transcript_safety_limit"
                    && message == TRANSCRIPT_SAFETY_LIMIT_ERROR
        )));
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { source, language, translation, .. } | LiveTranslateServerEvent::SubtitleFinalPair { source, language, translation, .. }
                if source == "next sentence"
                    && language.as_deref() == Some("en")
                    && translation == "次の文"
        )));
        assert!(!received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::SourceFinal { text, .. }
                if text.contains("private")
        ) || matches!(
            event,
            LiveTranslateServerEvent::TranslationFinal(translation)
                if translation.contains("private")
        )));
    }

    #[tokio::test]
    async fn provider_setup_failure_is_content_free() {
        let private_detail = "private transcript and volcano-secret-key";
        let (client, _events) = test_client(move |mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        failed_server_event(45_000_001, private_detail).into(),
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
        assert_eq!(error, VolcanoEngineClientError::SessionSetupRejected);
        assert!(!format!("{error:?}").contains(private_detail));
    }

    #[tokio::test]
    async fn setup_timeout_is_finite() {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                tokio::time::sleep(Duration::from_secs(2)).await;
            })
        })
        .await;
        assert_eq!(
            client
                .connect_with_timeout(Duration::from_millis(50))
                .await
                .unwrap_err(),
            VolcanoEngineClientError::SessionSetupTimedOut
        );
    }

    #[tokio::test]
    async fn an_immediate_pong_cannot_be_lost() {
        let (client, _events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if let Message::Ping(payload) = message {
                        socket.send(Message::Pong(payload)).await.unwrap();
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
        client.ping(Duration::from_millis(200)).await.unwrap();
        client.disconnect().await;
    }

    #[test]
    fn audio_buffer_keeps_only_a_partial_tail() {
        let frame_size = VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT;
        let mut pending = vec![0x22; frame_size * 2 + 17];
        let messages = take_complete_audio_messages("session-buffer", &mut pending).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(pending, vec![0x22; 17]);
        for message in messages {
            assert_eq!(find_varint(&message, 2), Some(EVENT_TASK_REQUEST as u64));
            assert_eq!(
                find_bytes(find_bytes(&message, 4).unwrap(), 14)
                    .unwrap()
                    .len(),
                frame_size
            );
        }
    }

    #[test]
    fn failed_audio_encoding_still_consumes_the_complete_batch() {
        let frame_size = VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT;
        let mut pending = vec![0x22; frame_size * 2 + 17];

        assert_eq!(
            take_complete_audio_messages("", &mut pending),
            Err(VolcanoEngineClientError::TransportFailure)
        );
        assert_eq!(pending, vec![0x22; 17]);
    }

    #[test]
    fn final_partial_audio_frame_is_zero_padded() {
        let mut pending = vec![1, 2, 3];
        let message = take_padded_audio_message("session-tail", &mut pending)
            .unwrap()
            .unwrap();
        let pcm = find_bytes(find_bytes(&message, 4).unwrap(), 14).unwrap();
        assert_eq!(pcm.len(), VolcanoEngineEndpoint::AUDIO_FRAME_BYTE_COUNT);
        assert_eq!(&pcm[..3], &[1, 2, 3]);
        assert!(pcm[3..].iter().all(|byte| *byte == 0));
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn malformed_response_after_setup_is_a_content_free_protocol_error() {
        let (client, mut events) = test_client(|mut socket| {
            Box::pin(async move {
                let _ = socket.next().await;
                socket
                    .send(Message::Binary(
                        server_event(EVENT_SESSION_STARTED as u64, None).into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Binary(vec![0x10, 0x80].into()))
                    .await
                    .unwrap();
            })
        })
        .await;
        client
            .connect_with_timeout(Duration::from_millis(500))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(event);
        }
        assert!(received.iter().any(|event| matches!(
            event,
            LiveTranslateServerEvent::Error { code, message }
                if code == "volcano_protocol_error" && message == GENERIC_PROTOCOL_ERROR
        )));
        client.disconnect().await;
    }
}
