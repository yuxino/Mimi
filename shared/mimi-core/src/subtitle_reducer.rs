//! Subtitle assembly state machine for drafts, confirmed pairs, and bounded
//! history.
//!
//! Providers that stream recognition and translation independently pair them by
//! the identity their own protocol carries and commit one atomic
//! [`SubtitleEvent::FinalPair`]. [`SubtitleEvent::TranslationFinal`] remains the
//! best-effort path for a stream without identity: the translation is paired
//! with the recognition line currently on screen.

use crate::models::{
    PreviewSubtitlePair, RealtimeSubtitlePreview, SubtitleEvent, SubtitleLine, SubtitlePair,
    SubtitleSnapshot, UtteranceRole,
};
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_LAYOUT_EPOCH: AtomicU64 = AtomicU64::new(0);

fn fresh_layout_epoch() -> u64 {
    NEXT_LAYOUT_EPOCH
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1)
}

// Keep one complete pair rather than truncating a sentence for presentation.
// This matches the MT clients' translated-text bound and also bounds its ASR
// counterpart when retained independently of the latest raw recognition.
#[cfg(test)]
const MAX_PREVIEW_TEXT_BYTES: usize = crate::models::MAX_SUBTITLE_TEXT_BYTES;
const MAX_IDENTIFIED_SOURCES: usize = 64;

fn legacy_has_session_confirmation() -> bool {
    true
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentifiedSource {
    id: String,
    ordinal: u64,
    confirmed: bool,
}

/// Platform persistence receives confirmations only; the core retains only its bounded display.
pub trait ArchiveSink: Default {
    fn append(&mut self, pair: &SubtitlePair);
    fn clear(&mut self);
}

#[derive(Default, Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct NoopArchive;
impl ArchiveSink for NoopArchive {
    fn append(&mut self, _pair: &SubtitlePair) {}
    fn clear(&mut self) {}
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(bound(serialize = "", deserialize = ""))]
pub struct SubtitleReducer<A: ArchiveSink = NoopArchive> {
    pub audio_source: crate::models::AudioSource,
    pub snapshot: SubtitleSnapshot,
    #[serde(skip)]
    pub archive: A,
    max_history_count: usize,
    source_draft_since_confirmation: bool,
    /// A manually started session may begin with an identical ID-less final.
    /// Reconnection alone must not rearm that admission boundary.
    #[serde(default = "legacy_has_session_confirmation")]
    has_session_confirmation: bool,
    last_confirmed_id: Option<u64>,
    latest_source_utterance_id: Option<u64>,
    last_confirmed_source_id: Option<u64>,
    preview_source_utterance_id: Option<u64>,
    #[serde(default)]
    display_pair_source_utterance_id: Option<u64>,
    /// Source observation order controls presentation only. Pairing always
    /// uses the provider's explicit ID, never this order or text equality.
    #[serde(default)]
    identified_sources: VecDeque<IdentifiedSource>,
    #[serde(default)]
    discarded_source_ids: VecDeque<String>,
    #[serde(default)]
    next_source_ordinal: u64,
    #[serde(default)]
    latest_source_ordinal: Option<u64>,
    #[serde(default)]
    display_source_ordinal: Option<u64>,
    layout_epoch: u64,
}

impl<A: ArchiveSink> SubtitleReducer<A> {
    pub fn new(max_history_count: usize) -> Self {
        Self {
            audio_source: crate::models::AudioSource::System,
            snapshot: SubtitleSnapshot::empty(),
            archive: Default::default(),
            max_history_count,
            source_draft_since_confirmation: false,
            has_session_confirmation: false,
            last_confirmed_id: None,
            latest_source_utterance_id: None,
            last_confirmed_source_id: None,
            preview_source_utterance_id: None,
            display_pair_source_utterance_id: None,
            identified_sources: VecDeque::new(),
            discarded_source_ids: VecDeque::new(),
            next_source_ordinal: 0,
            latest_source_ordinal: None,
            display_source_ordinal: None,
            layout_epoch: fresh_layout_epoch(),
        }
    }

    pub fn is_new_confirmation_id(&self, utterance_id: u64) -> bool {
        self.last_confirmed_id.is_none_or(|last| {
            let distance = utterance_id.wrapping_sub(last);
            distance != 0 && distance < 1_u64 << 63
        })
    }

    pub fn accepts_identified_final_pair(&self, id: &str, source: &str, translation: &str) -> bool {
        crate::models::subtitle_text_within_limit(id)
            && crate::models::subtitle_text_within_limit(source)
            && crate::models::subtitle_text_within_limit(translation)
            && !source.trim().is_empty()
            && !translation.trim().is_empty()
            && !self.discarded_source_ids.iter().any(|old| old == id)
            && !self
                .identified_sources
                .iter()
                .any(|item| item.id == id && item.confirmed)
    }

    pub fn identified_source_is_current(&self, id: &str) -> bool {
        self.identified_sources
            .iter()
            .find(|item| item.id == id)
            .is_none_or(|item| {
                self.latest_source_ordinal
                    .is_none_or(|latest| item.ordinal >= latest)
            })
    }

    /// Presentation retention is platform-selected; persistence is independent.
    pub fn set_history_limit(&mut self, limit: usize) {
        self.max_history_count = limit;
        let overflow = self.snapshot.history.len().saturating_sub(limit);
        self.snapshot.history.drain(0..overflow);
    }

    /// Reject oversized opaque bridge state before applying any new operation.
    pub fn validate_state(&self, max_history_limit: usize) -> Result<(), &'static str> {
        let line_valid = |line: &SubtitleLine| {
            crate::models::subtitle_text_within_limit(&line.text)
                && line
                    .utterance_id
                    .as_deref()
                    .is_none_or(crate::models::subtitle_text_within_limit)
        };
        let pair_valid = |pair: &SubtitlePair| {
            crate::models::subtitle_text_within_limit(&pair.source)
                && crate::models::subtitle_text_within_limit(&pair.translation)
        };
        let preview_valid = |pair: &PreviewSubtitlePair| {
            crate::models::subtitle_text_within_limit(&pair.source)
                && crate::models::subtitle_text_within_limit(&pair.translation)
                && pair
                    .utterance_id
                    .as_deref()
                    .is_none_or(crate::models::subtitle_text_within_limit)
        };
        if self.max_history_count <= max_history_limit
            && self.snapshot.history.len() <= self.max_history_count
            && line_valid(&self.snapshot.source)
            && line_valid(&self.snapshot.translation)
            && self.snapshot.history.iter().all(pair_valid)
            && self
                .snapshot
                .realtime_preview
                .as_ref()
                .is_none_or(|preview| {
                    line_valid(&preview.source) && line_valid(&preview.translation)
                })
            && self
                .snapshot
                .preview_pair
                .as_ref()
                .is_none_or(preview_valid)
            && self
                .snapshot
                .display_pair
                .as_ref()
                .is_none_or(preview_valid)
            && self.snapshot.tracks.len() <= 2
            && self.snapshot.tracks.iter().all(|track| {
                line_valid(&track.source)
                    && line_valid(&track.translation)
                    && track.history.len() <= self.max_history_count
                    && track.history.iter().all(pair_valid)
                    && track.realtime_preview.as_ref().is_none_or(|preview| {
                        line_valid(&preview.source) && line_valid(&preview.translation)
                    })
                    && track.preview_pair.as_ref().is_none_or(preview_valid)
                    && track.display_pair.as_ref().is_none_or(preview_valid)
                    && track
                        .detected_language
                        .as_deref()
                        .is_none_or(crate::models::subtitle_text_within_limit)
            })
            && self.identified_sources.len() <= MAX_IDENTIFIED_SOURCES
            && self.discarded_source_ids.len() <= MAX_IDENTIFIED_SOURCES
            && self.next_source_ordinal < u64::MAX
            && self
                .latest_source_ordinal
                .is_none_or(|id| id <= self.next_source_ordinal)
            && self
                .display_source_ordinal
                .is_none_or(|id| id <= self.next_source_ordinal)
            && self.identified_sources.iter().all(|item| {
                crate::models::subtitle_text_within_limit(&item.id)
                    && item.ordinal > 0
                    && item.ordinal <= self.next_source_ordinal
                    && !self.discarded_source_ids.contains(&item.id)
            })
            && self
                .identified_sources
                .iter()
                .map(|item| &item.id)
                .collect::<HashSet<_>>()
                .len()
                == self.identified_sources.len()
            && self
                .identified_sources
                .iter()
                .map(|item| item.ordinal)
                .collect::<Vec<_>>()
                .windows(2)
                .all(|pair| pair[0] < pair[1])
            && self
                .discarded_source_ids
                .iter()
                .all(|id| crate::models::subtitle_text_within_limit(id))
            && self
                .discarded_source_ids
                .iter()
                .collect::<HashSet<_>>()
                .len()
                == self.discarded_source_ids.len()
        {
            Ok(())
        } else {
            Err("subtitle_state_invalid")
        }
    }

    pub fn apply(&mut self, event: SubtitleEvent) {
        if !event.text_within_limit() {
            return;
        }
        let realtime = matches!(
            &event,
            SubtitleEvent::SourceDraft(_)
                | SubtitleEvent::SourceFinal(_)
                | SubtitleEvent::TranslationDraft(_)
                | SubtitleEvent::TranslationFinal(_)
                | SubtitleEvent::UtteranceText { .. }
                | SubtitleEvent::IdentifiedFinalPair { .. }
                | SubtitleEvent::FinalPair { .. }
        );
        let atomic = matches!(
            &event,
            SubtitleEvent::SourceUtteranceDraft { .. }
                | SubtitleEvent::PreviewPair { .. }
                | SubtitleEvent::ConfirmedPair { .. }
        );
        match event {
            SubtitleEvent::SourceDraft(text) => {
                self.latest_source_ordinal = None;
                let text = trim(&text);
                // A new recognition cycle may repeat the preceding lyric or
                // sentence. Its final is distinct from a duplicate final sent
                // without any intervening recognition draft.
                self.source_draft_since_confirmation |= !text.is_empty();
                self.snapshot.source = SubtitleLine::new(text, false);
            }
            SubtitleEvent::SourceUtteranceDraft { utterance_id, text } => {
                if self
                    .last_confirmed_source_id
                    .is_some_and(|id| utterance_id <= id)
                    || self
                        .latest_source_utterance_id
                        .is_some_and(|id| utterance_id < id)
                    || self
                        .preview_source_utterance_id
                        .is_some_and(|id| utterance_id < id)
                {
                    return;
                }
                self.latest_source_utterance_id = Some(utterance_id);
                self.source_draft_since_confirmation |= !text.trim().is_empty();
                self.snapshot.source = SubtitleLine::for_utterance(
                    trim(&text),
                    false,
                    self.source_layout_identity(utterance_id),
                );
            }
            SubtitleEvent::SourceFinal(text) => {
                self.latest_source_ordinal = None;
                self.snapshot.source = SubtitleLine::new(trim(&text), true);
            }
            SubtitleEvent::TranslationDraft(text) => {
                let trimmed = trim(&text);
                // A blank draft must not overwrite an already-confirmed final.
                if trimmed.is_empty() && self.snapshot.translation.is_final {
                    return;
                }
                self.snapshot.translation = SubtitleLine::new(trimmed, false);
            }
            SubtitleEvent::PreviewPair {
                source_utterance_id,
                source,
                translation,
            } => {
                let source = trim(&source);
                let translation = trim(&translation);
                if source.is_empty() || translation.is_empty() {
                    return;
                }
                if source_utterance_id.is_some_and(|preview_id| {
                    self.last_confirmed_source_id
                        .is_some_and(|id| preview_id <= id)
                        || self
                            .latest_source_utterance_id
                            .is_some_and(|id| preview_id < id)
                }) {
                    return;
                }
                // This is a replaceable display pair, never a confirmation.
                // Raw ASR can continue independently while a later request
                // waits; retaining its last complete pair avoids a blank and
                // a new short SSE prefix on every request.
                self.snapshot.preview_pair = Some(PreviewSubtitlePair {
                    utterance_id: source_utterance_id.map(|id| self.source_layout_identity(id)),
                    source,
                    translation,
                });
                self.update_display_pair(
                    self.snapshot.preview_pair.as_ref().unwrap().clone(),
                    source_utterance_id,
                    false,
                );
                self.preview_source_utterance_id = source_utterance_id;
                if let Some(id) = source_utterance_id {
                    self.latest_source_utterance_id = Some(
                        self.latest_source_utterance_id
                            .map_or(id, |last| last.max(id)),
                    );
                }
            }
            SubtitleEvent::ClearPreview => {
                self.snapshot.preview_pair = None;
                self.preview_source_utterance_id = None;
                self.clear_unconfirmed_display_pair();
            }
            SubtitleEvent::UtteranceText {
                utterance_id,
                role,
                text,
                is_final,
            } => {
                let text = trim(&text);
                if self.discarded_source_ids.contains(&utterance_id) {
                    return;
                }
                match role {
                    UtteranceRole::Source => {
                        let (ordinal, confirmed) = self.identify_source(&utterance_id);
                        if confirmed
                            || self
                                .latest_source_ordinal
                                .is_some_and(|latest| ordinal < latest)
                        {
                            return;
                        }
                        self.latest_source_ordinal = Some(ordinal);
                        self.snapshot.source =
                            SubtitleLine::for_utterance(text, is_final, utterance_id);
                    }
                    UtteranceRole::Translation => {
                        if self
                            .identified_sources
                            .iter()
                            .find(|item| item.id == utterance_id)
                            .is_some_and(|item| {
                                item.confirmed
                                    || self
                                        .latest_source_ordinal
                                        .is_some_and(|latest| item.ordinal < latest)
                            })
                        {
                            return;
                        }
                        if text.is_empty() && self.snapshot.translation.is_final {
                            return;
                        }
                        self.snapshot.translation =
                            SubtitleLine::for_utterance(text.clone(), is_final, utterance_id);
                        if is_final {
                            let source = self.snapshot.source.text.clone();
                            if !source.is_empty() && !text.is_empty() {
                                self.snapshot.preview_pair = None;
                            }
                            self.update_final_display_pair(&source, &text, None);
                            self.append_history_if_possible(source, text);
                        }
                    }
                }
            }
            SubtitleEvent::TranslationFinal(text) => {
                let translation = trim(&text);
                self.snapshot.translation = SubtitleLine::new(translation.clone(), true);
                let source = self.snapshot.source.text.clone();
                if !source.is_empty() && !translation.is_empty() {
                    self.snapshot.preview_pair = None;
                }
                self.update_final_display_pair(&source, &translation, None);
                self.append_history_if_possible(source, translation);
            }
            SubtitleEvent::FinalPair {
                source,
                translation,
            } => {
                let source = trim(&source);
                let translation = trim(&translation);
                self.snapshot.source = SubtitleLine::new(source.clone(), true);
                self.snapshot.translation = SubtitleLine::new(translation.clone(), true);
                // HQ finals own the durable lane and exclude later preview
                // work until completion. A corrected final therefore replaces
                // its preview even when the provider revised the source text.
                if !source.is_empty() && !translation.is_empty() {
                    self.snapshot.preview_pair = None;
                }
                self.update_final_display_pair(&source, &translation, None);
                self.append_history_if_possible(source, translation);
            }
            SubtitleEvent::IdentifiedFinalPair {
                utterance_id,
                source,
                translation,
            } => {
                let source = trim(&source);
                let translation = trim(&translation);
                if source.is_empty()
                    || translation.is_empty()
                    || self.discarded_source_ids.contains(&utterance_id)
                {
                    return;
                }
                let (ordinal, confirmed) = self.identify_source(&utterance_id);
                if confirmed {
                    return;
                }
                self.identified_sources
                    .iter_mut()
                    .find(|item| item.id == utterance_id)
                    .unwrap()
                    .confirmed = true;
                if self
                    .latest_source_ordinal
                    .is_none_or(|latest| latest <= ordinal)
                {
                    self.latest_source_ordinal = Some(ordinal);
                    self.snapshot.source =
                        SubtitleLine::for_utterance(source.clone(), true, utterance_id.clone());
                    self.snapshot.translation = SubtitleLine::for_utterance(
                        translation.clone(),
                        true,
                        utterance_id.clone(),
                    );
                    self.snapshot.preview_pair = None;
                }
                if self
                    .display_source_ordinal
                    .is_none_or(|display| display <= ordinal)
                {
                    self.snapshot.display_pair = Some(PreviewSubtitlePair {
                        utterance_id: Some(utterance_id),
                        source: source.clone(),
                        translation: translation.clone(),
                    });
                    self.snapshot.display_pair_final = true;
                    self.display_pair_source_utterance_id = None;
                    self.display_source_ordinal = Some(ordinal);
                }
                self.source_draft_since_confirmation = false;
                self.append_confirmed_history(source, translation, true);
            }
            SubtitleEvent::ConfirmedPair {
                utterance_id,
                source_utterance_id,
                source,
                translation,
            } => {
                if !self.is_new_confirmation_id(utterance_id) {
                    return;
                }
                let source = trim(&source);
                let translation = trim(&translation);
                if source.is_empty() || translation.is_empty() {
                    return;
                }
                self.last_confirmed_id = Some(utterance_id);
                self.source_draft_since_confirmation = false;
                let newer_source_visible = source_utterance_id.is_some_and(|final_id| {
                    self.latest_source_utterance_id
                        .is_some_and(|id| id > final_id)
                        || self
                            .preview_source_utterance_id
                            .is_some_and(|id| id > final_id)
                });
                let newer_preview_visible = source_utterance_id.is_some_and(|final_id| {
                    self.preview_source_utterance_id
                        .is_some_and(|id| id > final_id)
                });
                if let Some(id) = source_utterance_id {
                    self.last_confirmed_source_id = Some(
                        self.last_confirmed_source_id
                            .map_or(id, |last| last.max(id)),
                    );
                }
                // A previous sentence's HTTP final remains durable, but is
                // not the owner of a newer real sentence already on screen.
                if !newer_source_visible {
                    self.snapshot.source = SubtitleLine::new(source.clone(), true);
                    self.snapshot.translation = SubtitleLine::new(translation.clone(), true);
                }
                // Raw B may arrive (including an empty begin) before A's
                // final HTTP completes, while the complete preview still
                // belongs to A. Only that preview's own identity can keep it
                // out of A's confirmation cleanup; a newer raw owner cannot.
                if !newer_preview_visible {
                    self.snapshot.preview_pair = None;
                    self.preview_source_utterance_id = None;
                }
                self.update_final_display_pair(&source, &translation, source_utterance_id);
                self.append_confirmed_history(source, translation, true);
            }
            SubtitleEvent::Clear => {
                let old_ids: Vec<_> = self
                    .identified_sources
                    .iter()
                    .map(|item| item.id.clone())
                    .collect();
                for id in old_ids {
                    self.discard_source_id(id);
                }
                self.identified_sources.clear();
                self.latest_source_ordinal = None;
                self.display_source_ordinal = None;
                self.archive.clear();
                self.snapshot = SubtitleSnapshot::empty();
                self.latest_source_utterance_id = None;
                self.preview_source_utterance_id = None;
                self.display_pair_source_utterance_id = None;
                self.snapshot.display_pair_final = false;
                self.layout_epoch = fresh_layout_epoch();
                self.source_draft_since_confirmation = false;
                self.has_session_confirmation = false;
                // Clearing the display is not a new generation. Keep its
                // watermark so a replay cannot restore explicitly cleared text.
            }
        }
        if realtime {
            self.refresh_realtime_preview();
        } else if atomic {
            self.snapshot.realtime_preview = None;
        }
    }

    /// Select realtime lanes without changing the retained complete pair or history.
    fn refresh_realtime_preview(&mut self) {
        let mut source = self.snapshot.source.clone();
        let mut translation = self.snapshot.translation.clone();
        if let Some(pair) = self.snapshot.display_pair.as_ref() {
            let previous = |line: &SubtitleLine, text: &str| {
                self.snapshot.display_pair_final
                    && line.is_final
                    && line.text == text
                    && line.utterance_id == pair.utterance_id
            };
            if previous(&source, &pair.source) {
                source = SubtitleLine::new("", false);
            }
            if previous(&translation, &pair.translation) {
                translation = SubtitleLine::new("", false);
            }
        }
        // A translation with a different/unknown owner cannot accompany the
        // current original. Translation-first streams can still show one lane.
        if !source.text.is_empty()
            && !translation.text.is_empty()
            && source.utterance_id != translation.utterance_id
        {
            translation = SubtitleLine::new("", false);
        }
        self.snapshot.realtime_preview = if source.text.is_empty() && translation.text.is_empty() {
            None
        } else {
            Some(RealtimeSubtitlePreview {
                source,
                translation,
            })
        };
    }

    /// Begins a distinct user session without erasing its bounded display.
    /// Its first valid confirmation is new even if an ID-less provider repeats
    /// the previous session's final text. Archive lifetime remains caller-owned.
    pub fn begin_new_session(&mut self) {
        self.reset_transient();
        self.has_session_confirmation = false;
    }

    /// Drops generation-local state while preserving confirmed history and any
    /// fully confirmed line still displayed: an unconfirmed line belongs to the
    /// connection that produced it.
    pub fn reset_transient(&mut self) {
        self.identified_sources.clear();
        self.discarded_source_ids.clear();
        self.next_source_ordinal = 0;
        self.latest_source_ordinal = None;
        self.display_source_ordinal = None;
        self.last_confirmed_id = None;
        self.latest_source_utterance_id = None;
        self.last_confirmed_source_id = None;
        self.preview_source_utterance_id = None;
        self.display_pair_source_utterance_id = None;
        self.layout_epoch = fresh_layout_epoch();
        self.snapshot.preview_pair = None;
        self.snapshot.realtime_preview = None;
        self.clear_unconfirmed_display_pair();
        self.source_draft_since_confirmation = false;
        if !self.snapshot.source.is_final {
            self.snapshot.source = SubtitleLine::new("", false);
        }
        if !self.snapshot.translation.is_final {
            self.snapshot.translation = SubtitleLine::new("", false);
        }
    }

    fn source_layout_identity(&self, source_id: u64) -> String {
        // This is a local presentation owner, not a new provider identity.
        // The epoch isolates reducer creation, Clear and connection resets.
        format!("audio3:{}:{source_id}", self.layout_epoch)
    }

    fn identify_source(&mut self, id: &str) -> (u64, bool) {
        if let Some(item) = self.identified_sources.iter().find(|item| item.id == id) {
            return (item.ordinal, item.confirmed);
        }
        self.next_source_ordinal = self.next_source_ordinal.saturating_add(1);
        let ordinal = self.next_source_ordinal;
        self.identified_sources.push_back(IdentifiedSource {
            id: id.into(),
            ordinal,
            confirmed: false,
        });
        if self.identified_sources.len() > MAX_IDENTIFIED_SOURCES {
            let old = self.identified_sources.pop_front().unwrap();
            self.discard_source_id(old.id);
        }
        (ordinal, false)
    }

    fn discard_source_id(&mut self, id: String) {
        if !self.discarded_source_ids.contains(&id) {
            self.discarded_source_ids.push_back(id);
            if self.discarded_source_ids.len() > MAX_IDENTIFIED_SOURCES {
                self.discarded_source_ids.pop_front();
            }
        }
    }

    fn update_final_display_pair(
        &mut self,
        source: &str,
        translation: &str,
        source_id: Option<u64>,
    ) {
        let utterance_id = source_id
            .map(|id| self.source_layout_identity(id))
            .or_else(|| self.snapshot.source.utterance_id.clone());
        self.update_display_pair(
            PreviewSubtitlePair {
                utterance_id,
                source: source.into(),
                translation: translation.into(),
            },
            source_id,
            true,
        );
    }

    fn update_display_pair(
        &mut self,
        pair: PreviewSubtitlePair,
        source_id: Option<u64>,
        is_final: bool,
    ) {
        if pair.source.is_empty()
            || pair.translation.is_empty()
            || source_id.is_some_and(|id| {
                self.display_pair_source_utterance_id
                    .is_some_and(|owner| owner > id)
            })
        {
            return;
        }
        self.snapshot.display_pair = Some(pair);
        self.display_source_ordinal = None;
        self.display_pair_source_utterance_id = source_id;
        self.snapshot.display_pair_final = is_final;
    }

    fn clear_unconfirmed_display_pair(&mut self) {
        if !self.snapshot.display_pair_final {
            self.snapshot.display_pair = None;
            self.display_source_ordinal = None;
            self.display_pair_source_utterance_id = None;
        }
    }

    fn append_history_if_possible(&mut self, source: String, translation: String) {
        if source.is_empty() || translation.is_empty() {
            return;
        }
        let new_recognition = std::mem::take(&mut self.source_draft_since_confirmation);
        self.append_confirmed_history(source, translation, new_recognition);
    }

    fn append_confirmed_history(
        &mut self,
        source: String,
        translation: String,
        is_new_utterance: bool,
    ) {
        if source.is_empty() || translation.is_empty() {
            return;
        }
        // Timestamps also identify the frontend's bounded history rows. Two
        // legitimate repeated finals can finish within one millisecond.
        let created_at_ms = self
            .snapshot
            .history
            .last()
            .map_or_else(now_epoch_ms, |last| {
                now_epoch_ms().max(last.created_at_ms.saturating_add(1))
            });
        let mut pair = SubtitlePair::new(source, translation, created_at_ms);
        pair.audio_source = self.audio_source;
        if self.has_session_confirmation
            && !is_new_utterance
            && self.snapshot.history.last() == Some(&pair)
        {
            return;
        }
        self.has_session_confirmation = true;
        self.archive.append(&pair);
        self.snapshot.history.push(pair);
        if self.snapshot.history.len() > self.max_history_count {
            let overflow = self.snapshot.history.len() - self.max_history_count;
            self.snapshot.history.drain(0..overflow);
        }
    }
}

impl<A: ArchiveSink> Default for SubtitleReducer<A> {
    fn default() -> Self {
        Self::new(20)
    }
}

/// Trims leading and trailing Unicode whitespace.
pub fn trim(text: &str) -> String {
    text.trim().to_string()
}

/// Current wall-clock time as epoch milliseconds.
pub fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{SubtitleLine, SubtitlePair};

    type SubtitleReducer = super::SubtitleReducer<BoundedTestArchive>;

    #[derive(Default)]
    struct BoundedTestArchive {
        enabled: bool,
        pairs: Vec<SubtitlePair>,
        bytes: usize,
    }
    impl BoundedTestArchive {
        fn begin(&mut self, enabled: bool, _started_at_ms: u64) {
            *self = Self {
                enabled,
                ..Self::default()
            };
        }
        fn count(&self) -> usize {
            self.pairs.len()
        }
    }
    impl ArchiveSink for BoundedTestArchive {
        fn append(&mut self, pair: &SubtitlePair) {
            let bytes = pair.source.len().saturating_add(pair.translation.len());
            if self.enabled
                && self.pairs.len() < 128
                && bytes <= (1024usize * 1024).saturating_sub(self.bytes)
            {
                self.bytes += bytes;
                self.pairs.push(pair.clone());
            }
        }
        fn clear(&mut self) {
            self.begin(self.enabled, 0);
        }
    }

    fn confirmed(utterance_id: u64, text: &str) -> SubtitleEvent {
        SubtitleEvent::ConfirmedPair {
            source_utterance_id: None,
            utterance_id,
            source: text.into(),
            translation: text.into(),
        }
    }

    #[test]
    fn an_older_confirmation_appends_history_without_replacing_a_newer_complete_pair() {
        let mut reducer = SubtitleReducer::default();
        reducer.archive.begin(true, 0);
        reducer.apply(SubtitleEvent::SourceUtteranceDraft {
            utterance_id: 8,
            text: "Synthetic next source".into(),
        });
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: Some(8),
            source: "Synthetic next source".into(),
            translation: "Synthetic next translation".into(),
        });
        let source = reducer.snapshot.source.clone();
        let preview = reducer.snapshot.preview_pair.clone();
        assert_eq!(preview.as_ref().unwrap().utterance_id, source.utterance_id);
        reducer.apply(SubtitleEvent::ConfirmedPair {
            utterance_id: 1,
            source_utterance_id: Some(7),
            source: "Synthetic previous source".into(),
            translation: "Synthetic previous translation".into(),
        });
        assert_eq!(reducer.snapshot.source, source);
        assert_eq!(reducer.snapshot.preview_pair, preview);
        assert_eq!(reducer.snapshot.history.len(), 1);
        assert_eq!(
            reducer.snapshot.history[0].source,
            "Synthetic previous source"
        );
        assert_eq!(reducer.archive.count(), 1);
        for obsolete in [
            SubtitleEvent::SourceUtteranceDraft {
                utterance_id: 7,
                text: "Synthetic stale draft".into(),
            },
            SubtitleEvent::PreviewPair {
                source_utterance_id: Some(7),
                source: "Synthetic stale source".into(),
                translation: "Synthetic stale translation".into(),
            },
        ] {
            reducer.apply(obsolete);
            assert_eq!(reducer.snapshot.source, source);
            assert_eq!(reducer.snapshot.preview_pair, preview);
        }
        reducer.apply(SubtitleEvent::ConfirmedPair {
            utterance_id: 2,
            source_utterance_id: Some(8),
            source: "Synthetic next source".into(),
            translation: "Synthetic next translation".into(),
        });
        assert!(reducer.snapshot.preview_pair.is_none());
        assert!(reducer.snapshot.source.is_final);
        assert_eq!(reducer.snapshot.source.text, "Synthetic next source");
        assert!(reducer.snapshot.source.utterance_id.is_none());
        assert_eq!(reducer.snapshot.history.len(), 2);
        reducer.apply(SubtitleEvent::Clear);
        reducer.apply(SubtitleEvent::SourceUtteranceDraft {
            utterance_id: 8,
            text: "Synthetic cleared replay".into(),
        });
        assert!(reducer.snapshot.source.text.is_empty());
        reducer.apply(SubtitleEvent::SourceUtteranceDraft {
            utterance_id: 9,
            text: "Synthetic repeated lyric".into(),
        });
        assert_eq!(reducer.snapshot.source.text, "Synthetic repeated lyric");
        let before_reset = reducer.snapshot.source.utterance_id.clone();
        reducer.reset_transient();
        reducer.apply(SubtitleEvent::SourceUtteranceDraft {
            utterance_id: 1,
            text: "Synthetic new task".into(),
        });
        assert_eq!(reducer.snapshot.source.text, "Synthetic new task");
        assert_ne!(reducer.snapshot.source.utterance_id, before_reset);
    }

    #[test]
    fn confirming_a_preview_clears_it_while_retaining_the_next_raw_sentence() {
        for next_source in ["", "Next sentence begins"] {
            let mut reducer = SubtitleReducer::default();
            reducer.apply(SubtitleEvent::SourceUtteranceDraft {
                utterance_id: 7,
                text: "Earlier partial sentence".into(),
            });
            reducer.apply(SubtitleEvent::PreviewPair {
                source_utterance_id: Some(7),
                source: "Earlier partial sentence".into(),
                translation: "较早的完整预览".into(),
            });
            reducer.apply(SubtitleEvent::SourceUtteranceDraft {
                utterance_id: 8,
                text: next_source.into(),
            });
            let newer_raw = reducer.snapshot.source.clone();

            reducer.apply(SubtitleEvent::ConfirmedPair {
                utterance_id: 1,
                source_utterance_id: Some(7),
                source: "Earlier corrected complete sentence.".into(),
                translation: "较早句子的最终译文。".into(),
            });

            assert_eq!(reducer.snapshot.source, newer_raw);
            assert!(!reducer.snapshot.source.is_final);
            assert!(
                reducer.snapshot.preview_pair.is_none(),
                "a newer raw sentence cannot keep the confirmed sentence's old preview"
            );
            assert_eq!(reducer.snapshot.history.len(), 1);
            assert_eq!(
                reducer.snapshot.history[0].source,
                "Earlier corrected complete sentence."
            );
            assert_eq!(
                reducer.snapshot.history[0].translation,
                "较早句子的最终译文。"
            );
            assert_eq!(reducer.preview_source_utterance_id, None);
            assert_eq!(reducer.latest_source_utterance_id, Some(8));
        }
    }

    #[test]
    fn completed_pair_ownership_is_stable_on_revisions_and_changes_after_clear_or_reset() {
        let mut reducer = SubtitleReducer::default();
        let draft = || SubtitleEvent::SourceUtteranceDraft {
            utterance_id: 8,
            text: "Synthetic source".into(),
        };
        let preview = || SubtitleEvent::PreviewPair {
            source_utterance_id: Some(8),
            source: "Synthetic source".into(),
            translation: "Synthetic translation".into(),
        };
        reducer.apply(draft());
        reducer.apply(preview());
        let first = reducer
            .snapshot
            .preview_pair
            .as_ref()
            .unwrap()
            .utterance_id
            .clone();
        assert_eq!(first, reducer.snapshot.source.utterance_id);
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: Some(8),
            source: "Synthetic revised source".into(),
            translation: "Synthetic revised translation".into(),
        });
        assert_eq!(
            reducer.snapshot.preview_pair.as_ref().unwrap().utterance_id,
            first
        );
        reducer.apply(SubtitleEvent::Clear);
        reducer.apply(draft());
        reducer.apply(preview());
        let second = reducer
            .snapshot
            .preview_pair
            .as_ref()
            .unwrap()
            .utterance_id
            .clone();
        assert_ne!(second, first);
        assert_eq!(second, reducer.snapshot.source.utterance_id);
        reducer.reset_transient();
        reducer.apply(draft());
        reducer.apply(preview());
        assert_ne!(
            reducer.snapshot.preview_pair.as_ref().unwrap().utterance_id,
            second
        );
        let json = serde_json::to_value(&reducer.snapshot.preview_pair).unwrap();
        assert!(json["utteranceId"].is_string());
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source: "Synthetic legacy source".into(),
            translation: "Synthetic legacy translation".into(),
        });
        let json = serde_json::to_value(&reducer.snapshot.preview_pair).unwrap();
        assert!(json.get("utteranceId").is_none());
    }

    #[test]
    fn every_oversized_caption_preserves_complete_pair_and_confirmation_identity() {
        let mut reducer = SubtitleReducer::default();
        reducer.archive.begin(true, 0);
        reducer.apply(confirmed(1, "Synthetic final"));
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source: "Synthetic complete preview".into(),
            translation: "Synthetic complete translation".into(),
        });
        let expected = reducer.snapshot.clone();
        let oversized = "x".repeat(crate::models::MAX_SUBTITLE_TEXT_BYTES + 1);
        let events = [
            SubtitleEvent::SourceDraft(oversized.clone()),
            SubtitleEvent::SourceFinal(oversized.clone()),
            SubtitleEvent::TranslationDraft(oversized.clone()),
            SubtitleEvent::TranslationFinal(oversized.clone()),
            SubtitleEvent::UtteranceText {
                utterance_id: "synthetic-id".into(),
                role: UtteranceRole::Source,
                text: oversized.clone(),
                is_final: false,
            },
            SubtitleEvent::UtteranceText {
                utterance_id: "synthetic-id".into(),
                role: UtteranceRole::Translation,
                text: oversized.clone(),
                is_final: true,
            },
            SubtitleEvent::PreviewPair {
                source_utterance_id: None,
                source: oversized.clone(),
                translation: "valid".into(),
            },
            SubtitleEvent::FinalPair {
                source: "valid".into(),
                translation: oversized.clone(),
            },
            SubtitleEvent::ConfirmedPair {
                source_utterance_id: None,
                utterance_id: 2,
                source: oversized.clone(),
                translation: "valid".into(),
            },
            SubtitleEvent::ConfirmedPair {
                source_utterance_id: None,
                utterance_id: 2,
                source: "valid".into(),
                translation: oversized,
            },
        ];
        for event in events {
            reducer.apply(event);
            assert_eq!(reducer.snapshot, expected);
            assert_eq!(reducer.archive.count(), 1);
        }
        let exact = "🙂".repeat(crate::models::MAX_SUBTITLE_TEXT_BYTES / 4);
        reducer.apply(confirmed(2, &exact));
        assert_eq!(reducer.snapshot.source.text, exact);
        assert_eq!(reducer.snapshot.history.len(), 2);
        assert_eq!(reducer.archive.count(), 2);
    }

    #[test]
    fn confirmed_identity_preserves_repeated_text_and_rejects_older_or_duplicate_ids() {
        let mut reducer = SubtitleReducer::new(2);
        reducer.archive.begin(true, 0);
        for id in [1, 2, 3] {
            reducer.apply(confirmed(id, "Synthetic repeated line"));
        }
        assert_eq!(reducer.snapshot.history.len(), 2);
        assert_eq!(reducer.archive.count(), 3);
        let snapshot = reducer.snapshot.clone();
        for id in [3, 1, 2] {
            reducer.apply(confirmed(id, "Stale synthetic revision"));
        }
        assert_eq!(reducer.snapshot, snapshot);
        assert_eq!(reducer.archive.count(), 3);
    }

    #[test]
    fn clear_keeps_identity_watermark_and_reconnect_resets_it_with_history_preserved() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(confirmed(9, "Synthetic line"));
        reducer.apply(SubtitleEvent::Clear);
        reducer.apply(confirmed(9, "Synthetic replay"));
        assert!(reducer.snapshot.history.is_empty());
        reducer.apply(confirmed(10, "New synthetic line"));
        assert_eq!(reducer.snapshot.history.len(), 1);
        reducer.reset_transient();
        reducer.apply(confirmed(0, "New generation synthetic line"));
        assert_eq!(reducer.snapshot.history.len(), 2);
    }

    #[test]
    fn empty_confirmation_does_not_claim_identity_and_wrapped_serials_remain_ordered() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(confirmed(u64::MAX, " \n"));
        reducer.apply(confirmed(u64::MAX, "Synthetic line"));
        reducer.apply(confirmed(1, "Synthetic line"));
        reducer.apply(confirmed(u64::MAX, "Obsolete line"));
        assert_eq!(reducer.snapshot.history.len(), 2);
        assert_eq!(reducer.snapshot.source.text, "Synthetic line");
    }

    #[test]
    fn subtitle_reducer_starts_empty() {
        let reducer = SubtitleReducer::default();
        assert_eq!(reducer.snapshot, SubtitleSnapshot::empty());
    }

    #[test]
    fn drafts_remain_visibly_unconfirmed() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceDraft("Hello wor".into()));
        reducer.apply(SubtitleEvent::TranslationDraft("你好，世".into()));

        assert_eq!(
            reducer.snapshot.source,
            SubtitleLine::new("Hello wor", false)
        );
        assert_eq!(
            reducer.snapshot.translation,
            SubtitleLine::new("你好，世", false)
        );
    }

    #[test]
    fn a_complete_preview_survives_new_raw_drafts_without_pairing_them() {
        let mut reducer = SubtitleReducer::default();
        reducer.archive.begin(true, 0);
        reducer.apply(SubtitleEvent::SourceDraft("Old recognized source".into()));
        reducer.apply(SubtitleEvent::TranslationDraft(
            "Old raw translation".into(),
        ));
        let stable = PreviewSubtitlePair {
            utterance_id: None,
            source: "Old recognized source".into(),
            translation: "Complete translated preview".into(),
        };
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source: stable.source.clone(),
            translation: stable.translation.clone(),
        });
        reducer.apply(SubtitleEvent::SourceDraft(
            "New recognition still growing".into(),
        ));
        reducer.apply(SubtitleEvent::TranslationDraft(String::new()));
        reducer.apply(SubtitleEvent::SourceFinal(
            "New recognition revised.".into(),
        ));

        assert_eq!(reducer.snapshot.preview_pair, Some(stable));
        assert_eq!(reducer.snapshot.source.text, "New recognition revised.");
        assert!(reducer.snapshot.translation.text.is_empty());
        assert!(reducer.snapshot.history.is_empty());
        assert_eq!(reducer.archive.count(), 0);
    }

    #[test]
    fn preview_pairs_replace_one_slot_without_rewinding_raw_asr_or_entering_history() {
        let mut reducer = SubtitleReducer::new(2);
        reducer.archive.begin(true, 0);
        reducer.apply(SubtitleEvent::SourceDraft("Latest ASR draft".into()));
        reducer.apply(SubtitleEvent::TranslationDraft("Latest raw draft".into()));
        for index in 0..2_000 {
            reducer.apply(SubtitleEvent::PreviewPair {
                source_utterance_id: None,
                source: format!("Earlier candidate {index}"),
                translation: format!("Completed preview {index}"),
            });
        }
        assert_eq!(
            reducer.snapshot.preview_pair,
            Some(PreviewSubtitlePair {
                utterance_id: None,
                source: "Earlier candidate 1999".into(),
                translation: "Completed preview 1999".into(),
            })
        );
        assert_eq!(reducer.snapshot.source.text, "Latest ASR draft");
        assert_eq!(reducer.snapshot.translation.text, "Latest raw draft");
        assert!(reducer.snapshot.history.is_empty());
        assert_eq!(reducer.archive.count(), 0);
    }

    #[test]
    fn empty_or_oversized_preview_pairs_keep_the_last_complete_pair() {
        let mut reducer = SubtitleReducer::default();
        let stable = PreviewSubtitlePair {
            utterance_id: None,
            source: "Stable source".into(),
            translation: "Stable translation".into(),
        };
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source: stable.source.clone(),
            translation: stable.translation.clone(),
        });
        for (source, translation) in [
            (" \n\t".into(), "Translation".into()),
            ("Source".into(), " \n\t".into()),
            ("x".repeat(MAX_PREVIEW_TEXT_BYTES + 1), "Translation".into()),
            ("Source".into(), "字".repeat(MAX_PREVIEW_TEXT_BYTES / 3 + 1)),
        ] {
            reducer.apply(SubtitleEvent::PreviewPair {
                source_utterance_id: None,
                source,
                translation,
            });
            assert_eq!(reducer.snapshot.preview_pair, Some(stable.clone()));
        }
        assert!(reducer.snapshot.history.is_empty());
    }

    #[test]
    fn complete_preview_text_at_the_byte_limit_is_not_truncated() {
        let mut reducer = SubtitleReducer::default();
        let source = format!("{}a", "字".repeat(MAX_PREVIEW_TEXT_BYTES / 3));
        let translation = "🙂".repeat(MAX_PREVIEW_TEXT_BYTES / 4);
        assert_eq!(source.len(), MAX_PREVIEW_TEXT_BYTES);
        assert_eq!(translation.len(), MAX_PREVIEW_TEXT_BYTES);
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source: source.clone(),
            translation: translation.clone(),
        });
        assert_eq!(
            reducer.snapshot.preview_pair,
            Some(PreviewSubtitlePair {
                utterance_id: None,
                source,
                translation
            })
        );
    }

    #[test]
    fn valid_final_boundaries_replace_the_preview_even_when_source_was_revised() {
        for final_event in [
            SubtitleEvent::FinalPair {
                source: "Corrected source.".into(),
                translation: "Confirmed translation.".into(),
            },
            SubtitleEvent::TranslationFinal("Confirmed translation.".into()),
            SubtitleEvent::UtteranceText {
                utterance_id: "confirmed-source".into(),
                role: UtteranceRole::Translation,
                text: "Confirmed translation.".into(),
                is_final: true,
            },
        ] {
            let mut reducer = SubtitleReducer::default();
            reducer.apply(SubtitleEvent::SourceFinal("Corrected source.".into()));
            reducer.apply(SubtitleEvent::PreviewPair {
                source_utterance_id: None,
                source: "Provisional source".into(),
                translation: "Provisional translation".into(),
            });
            reducer.apply(final_event);
            assert!(reducer.snapshot.preview_pair.is_none());
            assert_eq!(reducer.snapshot.history.len(), 1);
            assert_eq!(reducer.snapshot.history[0].source, "Corrected source.");
            assert_eq!(
                reducer.snapshot.history[0].translation,
                "Confirmed translation."
            );
        }
    }

    #[test]
    fn an_empty_final_cannot_erase_a_complete_preview_without_replacing_it() {
        let mut reducer = SubtitleReducer::default();
        let stable = PreviewSubtitlePair {
            utterance_id: None,
            source: "Stable source".into(),
            translation: "Stable translation".into(),
        };
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source: stable.source.clone(),
            translation: stable.translation.clone(),
        });
        reducer.apply(SubtitleEvent::FinalPair {
            source: "Final source".into(),
            translation: " \n".into(),
        });
        assert_eq!(reducer.snapshot.preview_pair, Some(stable));
        assert!(reducer.snapshot.history.is_empty());
    }

    #[test]
    fn transient_reset_and_manual_clear_remove_the_preview_with_distinct_history_lifetimes() {
        let mut reducer = SubtitleReducer::default();
        reducer.archive.begin(true, 0);
        reducer.apply(SubtitleEvent::FinalPair {
            source: "Confirmed source".into(),
            translation: "Confirmed translation".into(),
        });
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source: "Current source".into(),
            translation: "Current preview".into(),
        });
        reducer.reset_transient();
        assert!(reducer.snapshot.preview_pair.is_none());
        assert_eq!(reducer.snapshot.history.len(), 1);
        assert_eq!(reducer.archive.count(), 1);
        assert!(reducer.snapshot.source.is_final);
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: None,
            source: "Current source".into(),
            translation: "Current preview".into(),
        });
        reducer.apply(SubtitleEvent::Clear);
        assert_eq!(reducer.snapshot, SubtitleSnapshot::empty());
        assert_eq!(reducer.archive.count(), 0);
    }

    #[test]
    fn a_new_recognition_cycle_preserves_repeated_lyrics_but_duplicate_finals_stay_idempotent() {
        let mut reducer = SubtitleReducer::default();
        reducer.archive.begin(true, 0);
        for _ in 0..2 {
            reducer.apply(SubtitleEvent::SourceDraft("Repeated lyric".into()));
            // Revising one recognition cycle does not create several finals.
            reducer.apply(SubtitleEvent::SourceDraft("Repeated lyric".into()));
            for _ in 0..2 {
                reducer.apply(SubtitleEvent::FinalPair {
                    source: "Repeated lyric".into(),
                    translation: "Repeated translation".into(),
                });
            }
        }
        assert_eq!(reducer.snapshot.history.len(), 2);
        assert_eq!(reducer.archive.count(), 2);
        assert!(
            reducer.snapshot.history[0].created_at_ms < reducer.snapshot.history[1].created_at_ms
        );
    }

    #[test]
    fn blank_drafts_or_reset_cannot_rearm_a_duplicate_confirmation() {
        let mut reducer = SubtitleReducer::default();
        for _ in 0..2 {
            reducer.apply(SubtitleEvent::FinalPair {
                source: "Confirmed source".into(),
                translation: "Confirmed translation".into(),
            });
            reducer.apply(SubtitleEvent::SourceDraft(" \n".into()));
            reducer.apply(SubtitleEvent::TranslationDraft(
                "A translation update".into(),
            ));
        }
        assert_eq!(reducer.snapshot.history.len(), 1);
        reducer.apply(SubtitleEvent::SourceDraft("Confirmed source".into()));
        reducer.reset_transient();
        reducer.apply(SubtitleEvent::FinalPair {
            source: "Confirmed source".into(),
            translation: "Confirmed translation".into(),
        });
        assert_eq!(reducer.snapshot.history.len(), 1);
    }

    #[test]
    fn new_session_admits_one_identical_idless_final_after_empty_events_and_reconnect() {
        let mut reducer = SubtitleReducer::default();
        let pair = || SubtitleEvent::FinalPair {
            source: "Repeated confirmation".into(),
            translation: "Repeated translation".into(),
        };
        reducer.apply(pair());
        reducer.begin_new_session();
        reducer.archive.begin(true, 1);
        assert_eq!(reducer.snapshot.history.len(), 1);
        for (source, translation) in [("", "Repeated translation"), ("Repeated confirmation", "")] {
            reducer.apply(SubtitleEvent::FinalPair {
                source: source.into(),
                translation: translation.into(),
            });
        }
        reducer.apply(SubtitleEvent::SourceDraft(" \n".into()));
        // Start may perform several connection attempts before a valid final.
        reducer.reset_transient();
        reducer.reset_transient();
        reducer.apply(pair());
        assert_eq!(reducer.snapshot.history.len(), 2);
        assert_eq!(reducer.archive.count(), 1);
        reducer.apply(pair());
        reducer.reset_transient();
        reducer.apply(pair());
        assert_eq!(reducer.snapshot.history.len(), 2);
        assert_eq!(reducer.archive.count(), 1);
    }

    #[test]
    fn identified_confirmations_consume_the_new_session_idless_admission() {
        for event in [
            SubtitleEvent::ConfirmedPair {
                utterance_id: 1,
                source_utterance_id: Some(1),
                source: "Same confirmation".into(),
                translation: "Same translation".into(),
            },
            SubtitleEvent::IdentifiedFinalPair {
                utterance_id: "provider-item-1".into(),
                source: "Same confirmation".into(),
                translation: "Same translation".into(),
            },
        ] {
            let mut reducer = SubtitleReducer::default();
            reducer.begin_new_session();
            reducer.apply(event);
            reducer.reset_transient();
            reducer.apply(SubtitleEvent::FinalPair {
                source: "Same confirmation".into(),
                translation: "Same translation".into(),
            });
            assert_eq!(reducer.snapshot.history.len(), 1);
        }
    }

    #[test]
    fn older_serialized_states_keep_idless_duplicate_suppression() {
        let mut reducer = SubtitleReducer::default();
        let pair = || SubtitleEvent::FinalPair {
            source: "Existing confirmation".into(),
            translation: "Existing translation".into(),
        };
        reducer.apply(pair());
        let mut state = serde_json::to_value(&reducer).unwrap();
        state
            .as_object_mut()
            .unwrap()
            .remove("has_session_confirmation");
        let mut restored: SubtitleReducer = serde_json::from_value(state).unwrap();
        restored.reset_transient();
        restored.apply(pair());
        assert_eq!(restored.snapshot.history.len(), 1);
    }

    #[test]
    fn final_translation_creates_a_history_pair() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceFinal("Hello world.".into()));
        reducer.apply(SubtitleEvent::TranslationFinal("你好，世界。".into()));

        assert_eq!(
            reducer.snapshot.source,
            SubtitleLine::new("Hello world.", true)
        );
        assert_eq!(
            reducer.snapshot.translation,
            SubtitleLine::new("你好，世界。", true)
        );
        assert_eq!(
            reducer.snapshot.history,
            vec![SubtitlePair::new(
                "Hello world.".into(),
                "你好，世界。".into(),
                0
            )]
        );
    }

    #[test]
    fn an_atomic_final_pair_alone_enters_history() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceFinal("legacy pending".into()));
        reducer.apply(SubtitleEvent::FinalPair {
            source: "OpenAI source".into(),
            translation: "OpenAI translation".into(),
        });

        assert_eq!(reducer.snapshot.history.len(), 1);
        assert_eq!(reducer.snapshot.history[0].source, "OpenAI source");
        assert_eq!(
            reducer.snapshot.history[0].translation,
            "OpenAI translation"
        );
    }

    #[test]
    fn a_new_draft_keeps_confirmed_history_available() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceFinal("Hello.".into()));
        reducer.apply(SubtitleEvent::TranslationFinal("你好。".into()));
        reducer.apply(SubtitleEvent::SourceDraft("How are".into()));
        reducer.apply(SubtitleEvent::TranslationDraft("你最近".into()));

        assert_eq!(
            reducer.snapshot.history,
            vec![SubtitlePair::new("Hello.".into(), "你好。".into(), 0)]
        );
        assert_eq!(
            reducer.snapshot.translation,
            SubtitleLine::new("你最近", false)
        );
    }

    #[test]
    fn a_plus_final_replaces_its_preview_and_alone_enters_history() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceDraft("今日は晴れです".into()));
        reducer.apply(SubtitleEvent::TranslationDraft("今天晴天".into()));
        assert!(reducer.snapshot.history.is_empty());

        reducer.apply(SubtitleEvent::SourceFinal("今日は晴れです。".into()));
        reducer.apply(SubtitleEvent::TranslationFinal("今天天气很好。".into()));

        assert_eq!(
            reducer.snapshot.history,
            vec![SubtitlePair::new(
                "今日は晴れです。".into(),
                "今天天气很好。".into(),
                0
            )]
        );
        assert_eq!(
            reducer.snapshot.translation,
            SubtitleLine::new("今天天气很好。", true)
        );
    }

    #[test]
    fn a_late_final_translation_uses_the_recognition_line_on_screen() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceFinal("First sentence.".into()));
        reducer.apply(SubtitleEvent::SourceDraft("Second sentence".into()));
        reducer.apply(SubtitleEvent::TranslationFinal("第一句。".into()));

        assert_eq!(
            reducer.snapshot.history,
            vec![SubtitlePair::new(
                "Second sentence".into(),
                "第一句。".into(),
                0
            )]
        );
        assert_eq!(
            reducer.snapshot.source,
            SubtitleLine::new("Second sentence", false)
        );
    }

    #[test]
    fn duplicate_finals_do_not_duplicate_history() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceFinal("Hello.".into()));
        reducer.apply(SubtitleEvent::TranslationFinal("你好。".into()));
        reducer.apply(SubtitleEvent::TranslationFinal("你好。".into()));
        assert_eq!(reducer.snapshot.history.len(), 1);
    }

    #[test]
    fn identical_source_and_translation_remain_in_history() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceFinal("嗯啊".into()));
        reducer.apply(SubtitleEvent::TranslationFinal("嗯啊".into()));

        assert_eq!(
            reducer.snapshot.history,
            vec![SubtitlePair::new("嗯啊".into(), "嗯啊".into(), 0)]
        );
    }

    #[test]
    fn history_is_bounded() {
        let mut reducer = SubtitleReducer::new(2);
        for index in 1..=3 {
            reducer.apply(SubtitleEvent::SourceFinal(format!("source {index}")));
            reducer.apply(SubtitleEvent::TranslationFinal(format!(
                "translation {index}"
            )));
        }

        assert_eq!(
            reducer.snapshot.history,
            vec![
                SubtitlePair::new("source 2".into(), "translation 2".into(), 0),
                SubtitlePair::new("source 3".into(), "translation 3".into(), 0),
            ]
        );
    }

    #[test]
    fn recognition_finals_without_translations_never_enter_history() {
        let mut reducer = SubtitleReducer::new(2);
        reducer.apply(SubtitleEvent::SourceFinal("source 1".into()));
        reducer.apply(SubtitleEvent::SourceFinal("source 2".into()));
        reducer.apply(SubtitleEvent::SourceFinal("source 3".into()));

        assert!(reducer.snapshot.history.is_empty());
        assert_eq!(reducer.snapshot.source, SubtitleLine::new("source 3", true));

        reducer.apply(SubtitleEvent::TranslationFinal("late translation".into()));
        assert_eq!(
            reducer.snapshot.history,
            vec![SubtitlePair::new(
                "source 3".into(),
                "late translation".into(),
                0
            )]
        );
    }

    #[test]
    fn clear_resets_all_subtitle_state() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceFinal("Hello.".into()));
        reducer.apply(SubtitleEvent::TranslationFinal("你好。".into()));
        reducer.apply(SubtitleEvent::Clear);

        assert_eq!(reducer.snapshot, SubtitleSnapshot::empty());
    }

    #[test]
    fn reconnect_clears_unconfirmed_lines_before_a_late_translation_final() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceDraft("generation A".into()));

        reducer.reset_transient();
        assert_eq!(reducer.snapshot.source, SubtitleLine::new("", false));
        reducer.apply(SubtitleEvent::TranslationFinal("译文 A".into()));
        assert!(reducer.snapshot.history.is_empty());

        reducer.apply(SubtitleEvent::SourceFinal("generation B".into()));
        reducer.apply(SubtitleEvent::TranslationFinal("译文 B".into()));
        assert_eq!(
            reducer.snapshot.history,
            vec![SubtitlePair::new("generation B".into(), "译文 B".into(), 0)]
        );
    }

    #[test]
    fn blank_draft_does_not_overwrite_confirmed_final() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceFinal("Hello.".into()));
        reducer.apply(SubtitleEvent::TranslationFinal("你好。".into()));
        reducer.apply(SubtitleEvent::TranslationDraft("   ".into()));

        assert_eq!(
            reducer.snapshot.translation,
            SubtitleLine::new("你好。", true)
        );
    }

    #[test]
    fn stamped_text_keeps_one_utterance_identity_on_both_lines() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::UtteranceText {
            utterance_id: "item_source".into(),
            role: UtteranceRole::Source,
            text: "Hello.".into(),
            is_final: true,
        });
        reducer.apply(SubtitleEvent::UtteranceText {
            utterance_id: "item_source".into(),
            role: UtteranceRole::Translation,
            text: "你好。".into(),
            is_final: false,
        });

        assert_eq!(
            reducer.snapshot.source.utterance_id.as_deref(),
            Some("item_source")
        );
        assert_eq!(
            reducer.snapshot.translation.utterance_id.as_deref(),
            Some("item_source")
        );
        assert!(reducer.snapshot.history.is_empty());

        reducer.apply(SubtitleEvent::UtteranceText {
            utterance_id: "item_source".into(),
            role: UtteranceRole::Translation,
            text: "你好。".into(),
            is_final: true,
        });
        assert_eq!(
            reducer.snapshot.history,
            vec![SubtitlePair::new("Hello.".into(), "你好。".into(), 0)]
        );
    }

    #[test]
    fn stamped_blank_draft_does_not_overwrite_confirmed_final() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::UtteranceText {
            utterance_id: "item_source".into(),
            role: UtteranceRole::Translation,
            text: "你好。".into(),
            is_final: true,
        });
        reducer.apply(SubtitleEvent::UtteranceText {
            utterance_id: "item_source".into(),
            role: UtteranceRole::Translation,
            text: "   ".into(),
            is_final: false,
        });

        assert_eq!(
            reducer.snapshot.translation,
            SubtitleLine::for_utterance("你好。", true, "item_source".into())
        );
    }
}

#[cfg(test)]
mod common_state_tests {
    use super::*;

    #[test]
    fn identified_owner_indexes_and_clear_tombstones_are_bounded_and_validated() {
        let mut reducer = SubtitleReducer::<NoopArchive>::new(6);
        for index in 0..MAX_IDENTIFIED_SOURCES * 3 {
            reducer.apply(SubtitleEvent::UtteranceText {
                utterance_id: format!("source-{index}"),
                role: UtteranceRole::Source,
                text: "Synthetic source".into(),
                is_final: false,
            });
        }
        assert_eq!(reducer.identified_sources.len(), MAX_IDENTIFIED_SOURCES);
        assert_eq!(reducer.discarded_source_ids.len(), MAX_IDENTIFIED_SOURCES);
        assert!(reducer.validate_state(6).is_ok());
        reducer.apply(SubtitleEvent::Clear);
        assert!(reducer.identified_sources.is_empty());
        assert_eq!(reducer.discarded_source_ids.len(), MAX_IDENTIFIED_SOURCES);
        assert!(reducer.validate_state(6).is_ok());
        let mut state = serde_json::to_value(&reducer).unwrap();
        state["discarded_source_ids"] = serde_json::json!(["duplicate", "duplicate"]);
        let restored: SubtitleReducer<NoopArchive> = serde_json::from_value(state).unwrap();
        assert_eq!(restored.validate_state(6), Err("subtitle_state_invalid"));
        reducer.reset_transient();
        assert!(reducer.discarded_source_ids.is_empty());
        assert!(reducer.validate_state(6).is_ok());
    }

    #[derive(Default)]
    struct CountingArchive(usize);
    impl ArchiveSink for CountingArchive {
        fn append(&mut self, _pair: &SubtitlePair) {
            self.0 = self.0.saturating_add(1);
        }
        fn clear(&mut self) {
            self.0 = 0;
        }
    }

    fn confirmed(id: u64, source_id: u64, text: &str) -> SubtitleEvent {
        SubtitleEvent::ConfirmedPair {
            utterance_id: id,
            source_utterance_id: Some(source_id),
            source: text.into(),
            translation: format!("完整译文 {text}"),
        }
    }

    #[test]
    fn history_limit_zero_keeps_one_current_pair_and_does_not_disable_platform_archive() {
        let mut reducer = SubtitleReducer::<CountingArchive>::new(2);
        for id in 1..=3 {
            reducer.apply(confirmed(id, id, &format!("Complete sentence {id}")));
        }
        assert_eq!(reducer.archive.0, 3);
        assert_eq!(reducer.snapshot.history.len(), 2);
        reducer.set_history_limit(0);
        assert!(reducer.snapshot.history.is_empty());
        assert_eq!(reducer.archive.0, 3);
        assert_eq!(
            reducer.snapshot.display_pair.as_ref().unwrap().source,
            "Complete sentence 3"
        );
        reducer.apply(SubtitleEvent::SourceUtteranceDraft {
            utterance_id: 4,
            text: "Unfinished next sentence".into(),
        });
        assert_eq!(
            reducer.snapshot.display_pair.as_ref().unwrap().source,
            "Complete sentence 3"
        );
        reducer.apply(confirmed(4, 4, "Next complete sentence"));
        assert!(reducer.snapshot.history.is_empty());
        assert_eq!(reducer.archive.0, 4);
        assert_eq!(
            reducer.snapshot.display_pair.as_ref().unwrap().source,
            "Next complete sentence"
        );
    }

    #[test]
    fn archive_contents_are_never_serialized_in_shared_opaque_state() {
        let mut reducer = SubtitleReducer::<CountingArchive>::new(1);
        reducer.apply(confirmed(1, 1, "Synthetic complete sentence"));
        assert_eq!(reducer.archive.0, 1);
        let state = serde_json::to_value(&reducer).unwrap();
        assert!(state.get("archive").is_none());
        let restored: SubtitleReducer<CountingArchive> = serde_json::from_value(state).unwrap();
        assert_eq!(restored.archive.0, 0);
        assert_eq!(restored.snapshot, reducer.snapshot);
        assert!(!restored.is_new_confirmation_id(1));
    }

    #[test]
    fn validation_rejects_unbounded_history_and_complete_display_without_echoing_text() {
        let mut reducer = SubtitleReducer::<NoopArchive>::new(6);
        assert!(reducer.validate_state(6).is_ok());
        reducer.set_history_limit(7);
        assert_eq!(reducer.validate_state(6), Err("subtitle_state_invalid"));
        reducer.set_history_limit(0);
        reducer.snapshot.display_pair = Some(PreviewSubtitlePair {
            utterance_id: None,
            source: "x".repeat(crate::models::MAX_SUBTITLE_TEXT_BYTES + 1),
            translation: "Allowed synthetic translation".into(),
        });
        assert_eq!(reducer.validate_state(6), Err("subtitle_state_invalid"));
        reducer.snapshot.display_pair = None;
        reducer.snapshot.history.push(SubtitlePair::new(
            "Unretained synthetic source".into(),
            "Unretained synthetic translation".into(),
            1,
        ));
        assert_eq!(reducer.validate_state(6), Err("subtitle_state_invalid"));
    }

    #[test]
    fn reconnect_retains_final_display_but_releases_owner_ids_for_the_next_generation() {
        let mut reducer = SubtitleReducer::<NoopArchive>::new(0);
        reducer.apply(confirmed(1, 99, "Previous generation final"));
        let old_display = reducer.snapshot.display_pair.clone();
        reducer.reset_transient();
        assert_eq!(reducer.snapshot.display_pair, old_display);
        reducer.apply(SubtitleEvent::SourceUtteranceDraft {
            utterance_id: 0,
            text: "New generation raw".into(),
        });
        reducer.apply(SubtitleEvent::PreviewPair {
            source_utterance_id: Some(0),
            source: "New generation complete preview".into(),
            translation: "新一代完整预览".into(),
        });
        assert_eq!(
            reducer.snapshot.display_pair.as_ref().unwrap().source,
            "New generation complete preview"
        );
        reducer.reset_transient();
        assert!(reducer.snapshot.display_pair.is_none());
        assert!(reducer.snapshot.history.is_empty());
    }
}
