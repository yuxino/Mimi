//! Synchronous subtitle timing and pair-commitment state machine for Volcano Engine.

use crate::core::models::{SourceLanguage, UtteranceRole};
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use std::collections::VecDeque;
use uuid::Uuid;

pub(super) const MAXIMUM_PENDING_FINALS_PER_SIDE: usize = 16;
pub(super) const MAXIMUM_PENDING_FINAL_TEXT_BYTES: usize = 128 * 1_024;
pub(super) const MAXIMUM_RECENT_COMMITTED_TIMINGS: usize = 32;
pub(super) const MAXIMUM_SOURCE_BLOCKS: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SubtitleTiming {
    pub(super) start_time_ms: Option<i32>,
    pub(super) end_time_ms: Option<i32>,
}

impl SubtitleTiming {
    pub(super) fn new(start_time_ms: Option<i32>, end_time_ms: Option<i32>) -> Self {
        Self {
            start_time_ms,
            end_time_ms,
        }
    }

    pub(super) fn is_usable(self) -> bool {
        self.start_time_ms.is_some() || self.end_time_ms.is_some()
    }

    /// The official source and translation finals describe the same input
    /// interval. Require the full interval when both sides provide it; fall
    /// back to one shared boundary only when the other boundary is absent.
    pub(super) fn matches(self, other: Self) -> bool {
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
pub(super) struct PendingFinal {
    pub(super) timing: SubtitleTiming,
    pub(super) text: String,
    pub(super) source_owner: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SourceBlock {
    pub(super) owner: u64,
    pub(super) start_time_ms: Option<i32>,
    pub(super) has_start: bool,
    pub(super) source_order: u64,
    pub(super) finished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingFinalTextLimitExceeded;

pub(super) type SubtitleCommitResult =
    Result<Option<LiveTranslateServerEvent>, PendingFinalTextLimitExceeded>;

pub(super) struct VolcanoSubtitlePairCommitter {
    pub(super) source_draft: String,
    pub(super) translation_draft: String,
    pub(super) source_in_progress: bool,
    pub(super) translation_in_progress: bool,
    pub(super) discard_source: bool,
    pub(super) discard_translation: bool,
    source_language: String,
    pub(super) sources: VecDeque<PendingFinal>,
    pub(super) translations: VecDeque<PendingFinal>,
    pub(super) pending_text_bytes: usize,
    pub(super) recently_committed: VecDeque<SubtitleTiming>,
    owner_epoch: Uuid,
    pub(super) next_source_owner: u64,
    pub(super) source_blocks: VecDeque<SourceBlock>,
    pub(super) retired_starts: VecDeque<i32>,
    retired_start_floor: Option<i32>,
    pub(super) source_owner: Option<u64>,
    pub(super) translation_start_time_ms: Option<i32>,
    pub(super) translation_has_start: bool,
    pub(super) translation_owner: Option<u64>,
    pub(super) last_matched_source_owner: Option<u64>,
    latest_start_time_ms: i32,
}

impl VolcanoSubtitlePairCommitter {
    pub(super) fn new(source_language: SourceLanguage) -> Self {
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

    pub(super) fn owner_id(&self, owner: u64) -> String {
        format!("volcano:{}:{owner}", self.owner_epoch)
    }

    pub(super) fn owner_for_start(&self, start_time_ms: Option<i32>) -> Option<u64> {
        let start = start_time_ms?;
        self.source_blocks
            .iter()
            .find(|block| block.start_time_ms == Some(start))
            .map(|block| block.owner)
    }

    pub(super) fn start_is_retired(&self, start_time_ms: Option<i32>) -> bool {
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

    pub(super) fn new_source_owner(&mut self, start_time_ms: Option<i32>) -> Option<u64> {
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

    pub(super) fn start_source(&mut self, start_time_ms: Option<i32>) -> bool {
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

    pub(super) fn ensure_source_owner(&mut self) -> Option<u64> {
        if self.source_owner.is_none_or(|owner| {
            self.source_blocks
                .iter()
                .any(|block| block.owner == owner && block.finished)
        }) {
            self.source_owner = self.new_source_owner(None);
        }
        self.source_owner
    }

    pub(super) fn source_end_owner(
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

    pub(super) fn finish_implicit_source(&mut self) {
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

    pub(super) fn translation_end_is_current(&self, timing: SubtitleTiming) -> bool {
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

    pub(super) fn bind_translation_owner(&mut self) {
        if self.translation_owner.is_none() {
            self.translation_owner = self.owner_for_start(self.translation_start_time_ms);
        }
    }

    pub(super) fn start_translation(&mut self, start_time_ms: Option<i32>) -> bool {
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

    pub(super) fn source_text_event(
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

    pub(super) fn source_boundary_event(&self, owner: u64) -> Option<LiveTranslateServerEvent> {
        Some(LiveTranslateServerEvent::SubtitleSourceBoundary {
            utterance_id: self.owner_id(owner),
            source_order: self.source_order(owner)?,
        })
    }

    pub(super) fn translation_text_event(&self) -> LiveTranslateServerEvent {
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

    pub(super) fn republish_after_final(&self) -> Vec<LiveTranslateServerEvent> {
        let Some(final_owner) = self.last_matched_source_owner else {
            return Vec::new();
        };
        self.republish_after_owner(final_owner)
    }

    pub(super) fn republish_after_owner(&self, final_owner: u64) -> Vec<LiveTranslateServerEvent> {
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
    pub(super) fn push_source(
        &mut self,
        text: String,
        start_time_ms: Option<i32>,
        end_time_ms: Option<i32>,
    ) -> SubtitleCommitResult {
        let owner = self.source_end_owner(start_time_ms, end_time_ms);
        self.push_source_with_owner(text, start_time_ms, end_time_ms, owner)
    }

    pub(super) fn push_source_with_owner(
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
            let translation = self
                .translations
                .remove(index)
                .expect("matching_index returns an in-bounds index");
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

    pub(super) fn push_translation(
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
            let source = self
                .sources
                .remove(index)
                .expect("matching_index returns an in-bounds index");
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

    pub(super) fn clear_content(&mut self) {
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

    pub(super) fn reset(&mut self) {
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
        let replaced = queue
            .remove(index)
            .expect("matching_index returns an in-bounds index");
        *pending_text_bytes = pending_text_bytes.saturating_sub(replaced.text.len());
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
