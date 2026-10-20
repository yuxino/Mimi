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
    #[serde(default)]
    known_order: Option<u64>,
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
    #[serde(default)]
    latest_known_source_order: Option<u64>,
    #[serde(default)]
    display_known_source_order: Option<u64>,
    #[serde(default)]
    source_owner_order: Option<u64>,
    #[serde(default)]
    translation_owner_order: Option<u64>,
    #[serde(default)]
    display_owner_order: Option<u64>,
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
            latest_known_source_order: None,
            display_known_source_order: None,
            source_owner_order: None,
            translation_owner_order: None,
            display_owner_order: None,
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
            .is_none_or(|item| self.can_present_identity(item, false))
    }

    /// Activity may start only for a meaningful source the reducer can accept.
    /// Register any provider order before querying this admission decision.
    pub fn can_start_identified_translation(&self, id: &str, text: &str) -> bool {
        if id.trim().is_empty()
            || text.trim().is_empty()
            || !crate::models::subtitle_text_within_limit(id)
            || !crate::models::subtitle_text_within_limit(text)
            || self.discarded_source_ids.iter().any(|old| old == id)
        {
            return false;
        }
        self.identified_sources
            .iter()
            .find(|item| item.id == id)
            .map_or_else(
                || {
                    self.next_source_ordinal
                        .checked_add(1)
                        .is_some_and(|next| next < u64::MAX)
                },
                |item| !item.confirmed && self.can_present_identity(item, false),
            )
    }

    /// Retains provider sentence order even when replaceable drafts coalesce.
    /// Registration carries no caption, confirmation, or display mutation.
    pub fn register_source_identity(&mut self, id: &str) {
        if !id.trim().is_empty()
            && crate::models::subtitle_text_within_limit(id)
            && !self.discarded_source_ids.iter().any(|old| old == id)
        {
            self.identify_source(id);
        }
    }

    /// Registers provider observation order without mixing it with the local
    /// arrival counter. Missing metadata never erases an identity's known order.
    pub fn register_source_order(&mut self, id: &str, order: u64) -> bool {
        if id.trim().is_empty()
            || !crate::models::subtitle_text_within_limit(id)
            || order == 0
            || order > i64::MAX as u64
            || self.discarded_source_ids.iter().any(|old| old == id)
            || self.identified_sources.iter().any(|item| {
                (item.id != id && item.known_order == Some(order))
                    || (item.id == id && item.known_order.is_some_and(|known| known != order))
            })
        {
            return false;
        }
        self.identify_source(id);
        let Some(item) = self
            .identified_sources
            .iter_mut()
            .find(|item| item.id == id)
        else {
            return false;
        };
        item.known_order = Some(order);
        let ordinal = item.ordinal;
        if self.snapshot.source.utterance_id.as_deref() == Some(id) {
            self.source_owner_order = Some(order);
        }
        if self.snapshot.translation.utterance_id.as_deref() == Some(id) {
            self.translation_owner_order = Some(order);
        }
        // A boundary can arrive after its caption. Retain that already visible
        // owner's order without changing the caption or its confirmation.
        if self.snapshot.source.utterance_id.as_deref() == Some(id)
            || self
                .snapshot
                .realtime_preview
                .as_ref()
                .is_some_and(|preview| {
                    !preview.translation.text.is_empty()
                        && preview.translation.utterance_id.as_deref() == Some(id)
                })
        {
            self.note_live_identity(ordinal, Some(order));
        }
        if self
            .snapshot
            .display_pair
            .as_ref()
            .and_then(|pair| pair.utterance_id.as_deref())
            == Some(id)
        {
            self.display_known_source_order =
                Some(self.display_known_source_order.unwrap_or(0).max(order));
            self.display_owner_order = Some(order);
        }
        true
    }

    fn retained_line(&self, line: &SubtitleLine, role: UtteranceRole) -> bool {
        self.snapshot.display_pair.as_ref().is_some_and(|pair| {
            self.snapshot.display_pair_final
                && line.is_final
                && line.utterance_id == pair.utterance_id
                && line.text
                    == match role {
                        UtteranceRole::Source => &pair.source,
                        UtteranceRole::Translation => &pair.translation,
                    }
                    .as_str()
        })
    }

    fn visible_live_owner_order(&self) -> Option<u64> {
        if !self.snapshot.source.text.is_empty()
            && !self.retained_line(&self.snapshot.source, UtteranceRole::Source)
        {
            self.source_owner_order
        } else if !self.snapshot.translation.text.is_empty()
            && !self.retained_line(&self.snapshot.translation, UtteranceRole::Translation)
        {
            self.translation_owner_order
        } else {
            self.source_owner_order
        }
    }

    fn can_present_identity(&self, item: &IdentifiedSource, display: bool) -> bool {
        let (arrival, known, owner_order) = if display {
            (
                self.display_source_ordinal,
                self.display_known_source_order,
                self.display_owner_order,
            )
        } else {
            (
                self.latest_source_ordinal,
                self.latest_known_source_order,
                self.visible_live_owner_order(),
            )
        };
        if let Some(order) = item.known_order {
            if known.is_some_and(|latest| order < latest) {
                return false;
            }
            if let Some(owner_order) = owner_order {
                return order >= owner_order;
            }
        }
        arrival.is_none_or(|latest| item.ordinal >= latest)
    }

    fn note_live_identity(&mut self, ordinal: u64, order: Option<u64>) {
        self.latest_source_ordinal = Some(self.latest_source_ordinal.unwrap_or(0).max(ordinal));
        if let Some(order) = order {
            self.latest_known_source_order =
                Some(self.latest_known_source_order.unwrap_or(0).max(order));
        }
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
        let owner_valid = |owner: Option<&str>, order: Option<u64>| {
            order.is_none_or(|_| owner.is_some())
                && owner
                    .and_then(|id| self.identified_sources.iter().find(|item| item.id == id))
                    .and_then(|item| item.known_order)
                    .is_none_or(|known| order == Some(known))
        };
        let frontier_covers = |order: Option<u64>, frontier: Option<u64>| {
            order.is_none_or(|order| frontier.is_some_and(|frontier| frontier >= order))
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
            && owner_valid(
                self.snapshot.source.utterance_id.as_deref(),
                self.source_owner_order,
            )
            && owner_valid(
                self.snapshot.translation.utterance_id.as_deref(),
                self.translation_owner_order,
            )
            && owner_valid(
                self.snapshot
                    .display_pair
                    .as_ref()
                    .and_then(|pair| pair.utterance_id.as_deref()),
                self.display_owner_order,
            )
            && frontier_covers(self.source_owner_order, self.latest_known_source_order)
            && frontier_covers(
                self.visible_live_owner_order(),
                self.latest_known_source_order,
            )
            && frontier_covers(self.display_owner_order, self.display_known_source_order)
            && [
                self.latest_known_source_order,
                self.display_known_source_order,
                self.source_owner_order,
                self.translation_owner_order,
                self.display_owner_order,
            ]
            .iter()
            .all(|order| order.is_none_or(|order| order > 0 && order <= i64::MAX as u64))
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
                    && item
                        .known_order
                        .is_none_or(|order| order > 0 && order <= i64::MAX as u64)
            })
            && self
                .identified_sources
                .iter()
                .filter_map(|item| item.known_order)
                .collect::<HashSet<_>>()
                .len()
                == self
                    .identified_sources
                    .iter()
                    .filter(|item| item.known_order.is_some())
                    .count()
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
                | SubtitleEvent::WithdrawPreview { .. }
        );
        match event {
            SubtitleEvent::SourceBoundary {
                utterance_id,
                source_order,
            } => {
                // Audio-time boundaries may correct a provisional clock for
                // an unconfirmed item. They retain their existing contract;
                // stable aligner ranks use register_source_order instead.
                if utterance_id.is_empty()
                    || source_order == 0
                    || source_order > i64::MAX as u64
                    || self.discarded_source_ids.contains(&utterance_id)
                {
                    return;
                }
                if self
                    .identified_sources
                    .iter()
                    .any(|item| item.id != utterance_id && item.ordinal == source_order)
                {
                    return;
                }
                if let Some(item) = self
                    .identified_sources
                    .iter_mut()
                    .find(|item| item.id == utterance_id)
                {
                    if item.confirmed || item.known_order.is_some() {
                        return;
                    }
                    if self.latest_source_ordinal == Some(item.ordinal) {
                        self.latest_source_ordinal = Some(source_order);
                    }
                    item.ordinal = source_order;
                } else {
                    self.identified_sources.push_back(IdentifiedSource {
                        id: utterance_id,
                        ordinal: source_order,
                        confirmed: false,
                        known_order: None,
                    });
                }
                self.identified_sources
                    .make_contiguous()
                    .sort_by_key(|item| item.ordinal);
                if self.identified_sources.len() > MAX_IDENTIFIED_SOURCES {
                    let old = self.identified_sources.pop_front().unwrap();
                    self.discard_source_id(old.id);
                }
                self.next_source_ordinal = self.next_source_ordinal.max(source_order);
            }
            SubtitleEvent::SourceDraft(text) => {
                self.latest_source_ordinal = None;
                self.source_owner_order = None;
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
                self.source_owner_order = None;
                self.source_draft_since_confirmation |= !text.trim().is_empty();
                self.snapshot.source = SubtitleLine::for_utterance(
                    trim(&text),
                    false,
                    self.source_layout_identity(utterance_id),
                );
            }
            SubtitleEvent::SourceFinal(text) => {
                self.latest_source_ordinal = None;
                self.source_owner_order = None;
                self.snapshot.source = SubtitleLine::new(trim(&text), true);
            }
            SubtitleEvent::TranslationDraft(text) => {
                let trimmed = trim(&text);
                // A blank draft must not overwrite an already-confirmed final.
                if trimmed.is_empty() && self.snapshot.translation.is_final {
                    return;
                }
                self.snapshot.translation = SubtitleLine::new(trimmed, false);
                self.translation_owner_order = None;
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
            SubtitleEvent::WithdrawPreview {
                source_utterance_id,
            } => {
                if source_utterance_id == 0 {
                    return;
                }
                let owner = self.source_layout_identity(source_utterance_id);
                let owns_source = !self.snapshot.source.is_final
                    && self.snapshot.source.utterance_id.as_deref() == Some(owner.as_str());
                let owns_preview = self.preview_source_utterance_id == Some(source_utterance_id);
                let owns_display = !self.snapshot.display_pair_final
                    && self.display_pair_source_utterance_id == Some(source_utterance_id);
                if !owns_source && !owns_preview && !owns_display {
                    return;
                }
                if owns_preview {
                    self.snapshot.preview_pair = None;
                    self.preview_source_utterance_id = None;
                }
                if owns_display {
                    self.clear_unconfirmed_display_pair();
                }
                if owns_source {
                    self.snapshot.source =
                        SubtitleLine::for_utterance(String::new(), false, owner.clone());
                }
                // Unstamped translation drafts may belong to an older final
                // still in flight. Only an identified line or complete owned
                // preview supplies enough evidence to withdraw that line.
                if !self.snapshot.translation.is_final
                    && ((owns_source && owns_preview)
                        || self.snapshot.translation.utterance_id.as_deref()
                            == Some(owner.as_str()))
                {
                    self.snapshot.translation =
                        SubtitleLine::for_utterance(String::new(), false, owner);
                }
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
                        if confirmed || !self.identified_source_is_current(&utterance_id) {
                            return;
                        }
                        let order = self
                            .identified_sources
                            .iter()
                            .find(|item| item.id == utterance_id)
                            .and_then(|item| item.known_order);
                        self.note_live_identity(ordinal, order);
                        self.source_owner_order = order;
                        self.snapshot.source =
                            SubtitleLine::for_utterance(text, is_final, utterance_id);
                    }
                    UtteranceRole::Translation => {
                        if self
                            .identified_sources
                            .iter()
                            .find(|item| item.id == utterance_id)
                            .is_some_and(|item| {
                                item.confirmed || !self.can_present_identity(item, false)
                            })
                        {
                            return;
                        }
                        if text.is_empty() && self.snapshot.translation.is_final {
                            return;
                        }
                        let known_identity = self
                            .identified_sources
                            .iter()
                            .find(|item| item.id == utterance_id)
                            .and_then(|item| item.known_order.map(|order| (item.ordinal, order)));
                        self.translation_owner_order = known_identity.map(|(_, order)| order);
                        if let Some((ordinal, order)) = known_identity {
                            self.note_live_identity(ordinal, Some(order));
                        }
                        self.snapshot.translation =
                            SubtitleLine::for_utterance(text.clone(), is_final, utterance_id);
                        if is_final {
                            let source = self.snapshot.source.text.clone();
                            if !source.is_empty() && !text.is_empty() {
                                self.snapshot.preview_pair = None;
                            }
                            self.confirm_legacy_pair(source, text);
                        }
                    }
                }
            }
            SubtitleEvent::TranslationFinal(text) => {
                self.translation_owner_order = None;
                let translation = trim(&text);
                self.snapshot.translation = SubtitleLine::new(translation.clone(), true);
                let source = self.snapshot.source.text.clone();
                if !source.is_empty() && !translation.is_empty() {
                    self.snapshot.preview_pair = None;
                }
                self.confirm_legacy_pair(source, translation);
            }
            SubtitleEvent::FinalPair {
                source,
                translation,
            } => {
                let source = trim(&source);
                let translation = trim(&translation);
                self.source_owner_order = None;
                self.translation_owner_order = None;
                self.snapshot.source = SubtitleLine::new(source.clone(), true);
                self.snapshot.translation = SubtitleLine::new(translation.clone(), true);
                // HQ finals own the durable lane and exclude later preview
                // work until completion. A corrected final therefore replaces
                // its preview even when the provider revised the source text.
                if !source.is_empty() && !translation.is_empty() {
                    self.snapshot.preview_pair = None;
                }
                self.confirm_legacy_pair(source, translation);
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
                let item = self
                    .identified_sources
                    .iter()
                    .find(|item| item.id == utterance_id)
                    .unwrap();
                let order = item.known_order;
                let promote_source = self.can_present_identity(item, false);
                let promoted = self.can_present_identity(item, true);
                if promote_source {
                    self.note_live_identity(ordinal, order);
                    self.source_owner_order = order;
                    self.translation_owner_order = order;
                    self.snapshot.source =
                        SubtitleLine::for_utterance(source.clone(), true, utterance_id.clone());
                    self.snapshot.translation = SubtitleLine::for_utterance(
                        translation.clone(),
                        true,
                        utterance_id.clone(),
                    );
                    self.snapshot.preview_pair = None;
                }
                if promoted {
                    self.display_owner_order = order;
                    self.snapshot.display_pair = Some(PreviewSubtitlePair {
                        utterance_id: Some(utterance_id),
                        source: source.clone(),
                        translation: translation.clone(),
                    });
                    self.snapshot.display_pair_final = true;
                    self.display_pair_source_utterance_id = None;
                    self.display_source_ordinal =
                        Some(self.display_source_ordinal.unwrap_or(0).max(ordinal));
                }
                if let Some(order) = order {
                    self.latest_known_source_order =
                        Some(self.latest_known_source_order.unwrap_or(0).max(order));
                    self.display_known_source_order =
                        Some(self.display_known_source_order.unwrap_or(0).max(order));
                }
                self.source_draft_since_confirmation = false;
                let time = self.append_confirmed_history(source, translation, true);
                if promoted && time.is_some() {
                    self.snapshot.display_pair_created_at_ms = time;
                }
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
                    self.source_owner_order = None;
                    self.translation_owner_order = None;
                    self.snapshot.source = SubtitleLine::new(source.clone(), true);
                    self.snapshot.translation = SubtitleLine::new(translation.clone(), true);
                    // Original-only readers use the raw final lane. Retain the
                    // same owner as its complete pair so confirmation/time is
                    // not lost when a numeric ASR sentence finishes HTTP MT.
                    if let Some(id) = source_utterance_id {
                        let owner = self.source_layout_identity(id);
                        self.snapshot.source.utterance_id = Some(owner.clone());
                        self.snapshot.translation.utterance_id = Some(owner);
                    }
                }
                // Raw B may arrive (including an empty begin) before A's
                // final HTTP completes, while the complete preview still
                // belongs to A. Only that preview's own identity can keep it
                // out of A's confirmation cleanup; a newer raw owner cannot.
                if !newer_preview_visible {
                    self.snapshot.preview_pair = None;
                    self.preview_source_utterance_id = None;
                }
                self.confirm_display_pair(source, translation, source_utterance_id, true);
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
                self.latest_known_source_order = None;
                self.display_known_source_order = None;
                self.source_owner_order = None;
                self.translation_owner_order = None;
                self.display_owner_order = None;
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
        if self.retained_line(&source, UtteranceRole::Source) {
            source = SubtitleLine::new("", false);
        }
        if self.retained_line(&translation, UtteranceRole::Translation) {
            translation = SubtitleLine::new("", false);
        }
        // A translation with a different/unknown owner cannot accompany the
        // current original. Translation-first streams can still show one lane.
        if !source.text.is_empty()
            && !translation.text.is_empty()
            && source.utterance_id != translation.utterance_id
        {
            translation = SubtitleLine::new("", false);
        }
        let (owner, order) = if !source.text.is_empty() {
            (source.utterance_id.as_deref(), self.source_owner_order)
        } else {
            (
                translation.utterance_id.as_deref(),
                self.translation_owner_order,
            )
        };
        if let Some(order) = order {
            self.latest_known_source_order =
                Some(self.latest_known_source_order.unwrap_or(0).max(order));
        }
        // Pure legacy translation drafts keep their existing policy. In a
        // mixed stream an already registered visible owner also retains its
        // arrival frontier, so known-order corrections cannot reopen it.
        if self.latest_known_source_order.is_some() {
            if let Some(ordinal) = owner
                .and_then(|id| self.identified_sources.iter().find(|item| item.id == id))
                .map(|item| item.ordinal)
            {
                self.latest_source_ordinal =
                    Some(self.latest_source_ordinal.unwrap_or(0).max(ordinal));
            }
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
        self.latest_known_source_order = None;
        self.display_known_source_order = None;
        self.source_owner_order = None;
        self.translation_owner_order = None;
        self.display_owner_order = None;
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
        let Some(ordinal) = self
            .next_source_ordinal
            .checked_add(1)
            .filter(|ordinal| *ordinal < u64::MAX)
        else {
            // Treat exhausted new identities as inadmissible without mutating
            // a restorable state or fabricating a confirmation.
            return (self.next_source_ordinal, true);
        };
        self.next_source_ordinal = ordinal;
        self.identified_sources.push_back(IdentifiedSource {
            id: id.into(),
            ordinal,
            confirmed: false,
            known_order: None,
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
    ) -> bool {
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
        )
    }

    fn update_display_pair(
        &mut self,
        pair: PreviewSubtitlePair,
        source_id: Option<u64>,
        is_final: bool,
    ) -> bool {
        if pair.source.is_empty()
            || pair.translation.is_empty()
            || source_id.is_some_and(|id| {
                self.display_pair_source_utterance_id
                    .is_some_and(|owner| owner > id)
            })
        {
            return false;
        }
        let known_order = pair
            .utterance_id
            .as_deref()
            .and_then(|id| self.identified_sources.iter().find(|item| item.id == id))
            .and_then(|item| item.known_order)
            .or_else(|| {
                (pair.utterance_id.is_some()
                    && pair.utterance_id == self.snapshot.source.utterance_id)
                    .then_some(self.source_owner_order)
                    .flatten()
            });
        self.display_owner_order = known_order;
        if let Some(order) = known_order {
            self.display_known_source_order =
                Some(self.display_known_source_order.unwrap_or(0).max(order));
        }
        self.snapshot.display_pair = Some(pair);
        self.display_source_ordinal = None;
        self.display_pair_source_utterance_id = source_id;
        self.snapshot.display_pair_final = is_final;
        if !is_final {
            self.snapshot.display_pair_created_at_ms = None;
        }
        true
    }

    fn clear_unconfirmed_display_pair(&mut self) {
        if !self.snapshot.display_pair_final {
            self.display_owner_order = None;
            self.snapshot.display_pair = None;
            self.snapshot.display_pair_created_at_ms = None;
            self.display_source_ordinal = None;
            self.display_pair_source_utterance_id = None;
        }
    }

    fn confirm_legacy_pair(&mut self, source: String, translation: String) {
        if source.is_empty() || translation.is_empty() {
            return;
        }
        let new_recognition = std::mem::take(&mut self.source_draft_since_confirmation);
        self.confirm_display_pair(source, translation, None, new_recognition);
    }

    fn confirm_display_pair(
        &mut self,
        source: String,
        translation: String,
        source_id: Option<u64>,
        is_new: bool,
    ) {
        let duplicate = !is_new
            && self.snapshot.display_pair_final
            && self
                .snapshot
                .display_pair
                .as_ref()
                .is_some_and(|pair| pair.source == source && pair.translation == translation);
        let promoted = self.update_final_display_pair(&source, &translation, source_id);
        let time = self.append_confirmed_history(source, translation, is_new);
        if promoted && !duplicate && time.is_some() {
            self.snapshot.display_pair_created_at_ms = time;
        }
    }

    fn append_confirmed_history(
        &mut self,
        source: String,
        translation: String,
        is_new_utterance: bool,
    ) -> Option<u64> {
        if source.is_empty() || translation.is_empty() {
            return None;
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
            return None;
        }
        self.has_session_confirmation = true;
        self.archive.append(&pair);
        self.snapshot.history.push(pair);
        if self.snapshot.history.len() > self.max_history_count {
            let overflow = self.snapshot.history.len() - self.max_history_count;
            self.snapshot.history.drain(0..overflow);
        }
        Some(created_at_ms)
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

    #[test]
    fn translation_start_requires_a_meaningful_current_unconfirmed_source() {
        let mut reducer = SubtitleReducer::default();
        assert!(reducer.can_start_identified_translation("current", "Synthetic source"));
        assert!(reducer.register_source_order("current", 2));
        reducer.apply(identified_text(
            "current",
            UtteranceRole::Source,
            "Synthetic source",
        ));
        assert!(reducer.register_source_order("unseen-old", 1));
        assert!(!reducer.can_start_identified_translation("unseen-old", "Synthetic old source"));
        reducer.discard_source_id("discarded".into());
        let oversized = "x".repeat(crate::models::MAX_SUBTITLE_TEXT_BYTES + 1);
        for (id, text) in [
            ("", "Synthetic source"),
            (" \u{3000}", "Synthetic source"),
            ("current", ""),
            ("current", " \u{3000}"),
            ("discarded", "Synthetic source"),
            (oversized.as_str(), "Synthetic source"),
            ("current", oversized.as_str()),
        ] {
            assert!(!reducer.can_start_identified_translation(id, text));
        }
        reducer.apply(SubtitleEvent::IdentifiedFinalPair {
            utterance_id: "current".into(),
            source: "Synthetic source".into(),
            translation: "Synthetic translation".into(),
        });
        assert!(!reducer.can_start_identified_translation("current", "Synthetic revised source"));
        assert!(reducer.can_start_identified_translation("newer", "Synthetic next source"));
    }

    #[test]
    fn translation_start_cannot_admit_an_unknown_identity_after_ordinal_exhaustion() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(identified_text(
            "known",
            UtteranceRole::Source,
            "Synthetic source",
        ));
        reducer.next_source_ordinal = u64::MAX - 1;
        assert!(reducer.validate_state(100).is_ok());
        assert!(reducer.can_start_identified_translation("known", "Synthetic revised source"));
        assert!(!reducer.can_start_identified_translation("unknown", "Synthetic next source"));
        let before = reducer.snapshot.clone();
        reducer.apply(identified_text(
            "unknown",
            UtteranceRole::Source,
            "Synthetic next source",
        ));
        assert_eq!(reducer.snapshot, before);
    }

    #[test]
    fn source_order_limits_and_exhausted_auto_order_keep_state_restorable() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(SubtitleEvent::SourceBoundary {
            utterance_id: "clock-limit".into(),
            source_order: i64::MAX as u64,
        });
        reducer.apply(SubtitleEvent::SourceBoundary {
            utterance_id: "invalid-clock".into(),
            source_order: i64::MAX as u64 + 1,
        });
        assert_eq!(reducer.identified_sources.len(), 1);
        reducer.apply(SubtitleEvent::UtteranceText {
            utterance_id: "automatic".into(),
            role: UtteranceRole::Source,
            text: "Synthetic current".into(),
            is_final: false,
        });
        assert_eq!(reducer.next_source_ordinal, i64::MAX as u64 + 1);
        assert!(reducer.validate_state(100).is_ok());
        reducer.next_source_ordinal = u64::MAX - 1;
        let before = serde_json::to_value(&reducer).unwrap();
        reducer.apply(SubtitleEvent::UtteranceText {
            utterance_id: "exhausted".into(),
            role: UtteranceRole::Source,
            text: "Synthetic rejected".into(),
            is_final: false,
        });
        reducer.apply(SubtitleEvent::IdentifiedFinalPair {
            utterance_id: "exhausted".into(),
            source: "Synthetic rejected".into(),
            translation: "Synthetic rejected translation".into(),
        });
        assert_eq!(serde_json::to_value(&reducer).unwrap(), before);
        assert!(reducer.validate_state(100).is_ok());
    }

    fn identified_text(id: &str, role: UtteranceRole, text: &str) -> SubtitleEvent {
        SubtitleEvent::UtteranceText {
            utterance_id: id.into(),
            role,
            text: text.into(),
            is_final: false,
        }
    }

    fn identified_pair(id: &str, text: &str) -> SubtitleEvent {
        SubtitleEvent::IdentifiedFinalPair {
            utterance_id: id.into(),
            source: text.into(),
            translation: format!("Translated {text}"),
        }
    }

    #[test]
    fn known_order_is_independent_of_unknown_arrival_ordinals_and_cannot_be_retargeted() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(identified_text(
            "unknown",
            UtteranceRole::Source,
            "Synthetic unknown",
        ));
        assert!(reducer.register_source_order("known-z", 1));
        reducer.apply(identified_text(
            "known-z",
            UtteranceRole::Source,
            "Synthetic known",
        ));
        assert_eq!(
            reducer.snapshot.source.utterance_id.as_deref(),
            Some("known-z")
        );
        assert!(!reducer.register_source_order("known-z", 2));
        assert!(!reducer.register_source_order("other", 1));
        assert!(reducer.register_source_order("known-a", 2));
        reducer.apply(identified_text(
            "known-a",
            UtteranceRole::Source,
            "Synthetic newer",
        ));
        // Missing metadata cannot erase the rank learned for this identity.
        reducer.apply(identified_text(
            "known-z",
            UtteranceRole::Source,
            "Synthetic old replay",
        ));
        assert_eq!(reducer.snapshot.source.text, "Synthetic newer");
        assert!(reducer.validate_state(100).is_ok());
    }

    #[test]
    fn old_known_pairs_keep_history_and_the_retained_pair_without_rewinding_live_lanes() {
        let mut reducer = SubtitleReducer::default();
        assert!(reducer.register_source_order("current", 2));
        reducer.apply(identified_text(
            "current",
            UtteranceRole::Source,
            "Synthetic current",
        ));
        reducer.apply(identified_text(
            "current",
            UtteranceRole::Translation,
            "Current preview",
        ));
        assert!(reducer.register_source_order("previous", 1));
        for role in [UtteranceRole::Source, UtteranceRole::Translation] {
            reducer.apply(identified_text("previous", role, "Synthetic stale preview"));
        }
        assert_eq!(reducer.snapshot.source.text, "Synthetic current");
        assert_eq!(reducer.snapshot.translation.text, "Current preview");
        reducer.apply(identified_pair("previous", "Synthetic previous complete"));
        assert_eq!(reducer.snapshot.source.text, "Synthetic current");
        assert_eq!(
            reducer
                .snapshot
                .display_pair
                .as_ref()
                .unwrap()
                .utterance_id
                .as_deref(),
            Some("previous")
        );
        reducer.apply(identified_pair("current", "Synthetic current complete"));
        reducer.apply(identified_pair("previous", "Synthetic duplicate"));
        assert_eq!(reducer.snapshot.source.text, "Synthetic current complete");
        assert_eq!(
            reducer
                .snapshot
                .display_pair
                .as_ref()
                .unwrap()
                .utterance_id
                .as_deref(),
            Some("current")
        );
        assert_eq!(reducer.snapshot.history.len(), 2);
        assert!(reducer.validate_state(100).is_ok());
    }

    #[test]
    fn known_correction_does_not_reopen_an_older_unknown_source_or_display() {
        let mut reducer = SubtitleReducer::default();
        assert!(reducer.register_source_order("known-a", 3));
        reducer.register_source_identity("unknown-old");
        assert!(reducer.register_source_order("known-z", 2));
        reducer.apply(identified_text(
            "known-z",
            UtteranceRole::Source,
            "Synthetic second",
        ));
        reducer.apply(identified_pair("known-z", "Synthetic second complete"));
        reducer.apply(identified_text(
            "known-a",
            UtteranceRole::Source,
            "Synthetic newest",
        ));
        reducer.apply(identified_pair("known-a", "Synthetic newest complete"));
        reducer.apply(identified_text(
            "unknown-old",
            UtteranceRole::Source,
            "Synthetic old unknown",
        ));
        reducer.apply(identified_pair(
            "unknown-old",
            "Synthetic old unknown complete",
        ));
        assert_eq!(reducer.snapshot.source.text, "Synthetic newest complete");
        assert_eq!(
            reducer
                .snapshot
                .display_pair
                .as_ref()
                .unwrap()
                .utterance_id
                .as_deref(),
            Some("known-a")
        );
        assert_eq!(reducer.snapshot.history.len(), 3);
        reducer.apply(identified_text(
            "unknown-new",
            UtteranceRole::Source,
            "Synthetic new unknown",
        ));
        assert_eq!(reducer.snapshot.source.text, "Synthetic new unknown");
        reducer.apply(identified_text(
            "known-z",
            UtteranceRole::Source,
            "Synthetic late known",
        ));
        assert_eq!(reducer.snapshot.source.text, "Synthetic new unknown");
        assert!(reducer.validate_state(100).is_ok());
    }

    #[test]
    fn legacy_order_state_restores_and_rank_scope_resets_without_losing_confirmed_history() {
        let mut reducer = SubtitleReducer::default();
        reducer.apply(identified_text(
            "old",
            UtteranceRole::Source,
            "Synthetic old",
        ));
        reducer.apply(identified_text(
            "current",
            UtteranceRole::Source,
            "Synthetic current",
        ));
        let mut legacy = serde_json::to_value(&reducer).unwrap();
        for field in ["latest_known_source_order", "display_known_source_order"] {
            legacy.as_object_mut().unwrap().remove(field);
        }
        for identity in legacy["identified_sources"].as_array_mut().unwrap() {
            identity.as_object_mut().unwrap().remove("known_order");
        }
        let mut restored: SubtitleReducer = serde_json::from_value(legacy).unwrap();
        assert!(restored.validate_state(100).is_ok());
        assert!(restored.register_source_order("current", 100));
        assert!(restored.register_source_order("old", 99));
        restored.apply(identified_text(
            "old",
            UtteranceRole::Source,
            "Synthetic late old",
        ));
        assert_eq!(restored.snapshot.source.text, "Synthetic current");
        restored.apply(identified_pair("current", "Synthetic confirmed"));
        restored.reset_transient();
        assert_eq!(restored.snapshot.history.len(), 1);
        assert!(restored.register_source_order("next", 1));
        restored.apply(identified_text(
            "next",
            UtteranceRole::Source,
            "Synthetic new scope",
        ));
        assert_eq!(restored.snapshot.source.text, "Synthetic new scope");
        let state = serde_json::to_value(&restored).unwrap();
        let mut restored: SubtitleReducer = serde_json::from_value(state).unwrap();
        restored.apply(SubtitleEvent::Clear);
        assert!(restored.snapshot.history.is_empty());
        assert!(!restored.register_source_order("next", 1));
        assert!(restored.register_source_order("after-clear", 1));
        restored.apply(identified_text(
            "after-clear",
            UtteranceRole::Source,
            "Synthetic after clear",
        ));
        assert_eq!(restored.snapshot.source.text, "Synthetic after clear");
        assert!(restored.validate_state(100).is_ok());
    }

    #[test]
    fn late_order_for_translation_only_owner_covers_both_domains_and_its_successor() {
        let mut reducer = SubtitleReducer::default();
        assert!(reducer.register_source_order("future", 3));
        reducer.register_source_identity("old-unknown");
        reducer.apply(identified_text(
            "current",
            UtteranceRole::Translation,
            "Synthetic current translation",
        ));
        assert!(reducer.register_source_order("current", 2));
        assert!(reducer.register_source_order("previous", 1));
        for id in ["previous", "old-unknown"] {
            reducer.apply(identified_text(
                id,
                UtteranceRole::Translation,
                "Synthetic old translation",
            ));
            assert_eq!(
                reducer.snapshot.translation.text,
                "Synthetic current translation"
            );
        }
        reducer.apply(identified_text(
            "future",
            UtteranceRole::Translation,
            "Synthetic newer translation",
        ));
        assert_eq!(
            reducer.snapshot.translation.text,
            "Synthetic newer translation"
        );
        assert!(reducer.validate_state(100).is_ok());
    }

    #[test]
    fn retained_source_does_not_hide_the_visible_translation_comparison_domain() {
        for old_known in [false, true] {
            let mut reducer = SubtitleReducer::default();
            if old_known {
                assert!(reducer.register_source_order("retained", 1));
            }
            reducer.apply(identified_pair("retained", "Synthetic retained complete"));
            assert!(reducer.register_source_order("future", 3));
            assert!(reducer.register_source_order("current", 2));
            reducer.apply(identified_text(
                "current",
                UtteranceRole::Translation,
                "Synthetic current translation",
            ));
            assert_eq!(
                reducer
                    .snapshot
                    .realtime_preview
                    .as_ref()
                    .unwrap()
                    .translation
                    .text,
                "Synthetic current translation"
            );
            assert!(reducer.validate_state(100).is_ok());
            reducer.register_source_identity("unknown-new");
            reducer.apply(identified_text(
                "unknown-new",
                UtteranceRole::Translation,
                "Synthetic new unknown translation",
            ));
            reducer.apply(identified_text(
                "future",
                UtteranceRole::Translation,
                "Synthetic early unknown replacement",
            ));
            assert_eq!(
                reducer.snapshot.translation.text,
                "Synthetic new unknown translation"
            );
            assert!(reducer.validate_state(100).is_ok());
        }
    }

    #[test]
    fn evicting_the_current_known_identity_keeps_its_comparison_domain() {
        for display in [false, true] {
            let mut reducer = SubtitleReducer::default();
            assert!(reducer.register_source_order("current", 3));
            assert!(reducer.register_source_order("future", 4));
            for index in 3..=64 {
                reducer.register_source_identity(&format!("unknown-{index}"));
            }
            assert!(reducer.register_source_order("unknown-64", 2));
            reducer.apply(if display {
                identified_pair("unknown-64", "Synthetic earlier")
            } else {
                identified_text("unknown-64", UtteranceRole::Source, "Synthetic earlier")
            });
            reducer.apply(if display {
                identified_pair("current", "Synthetic current")
            } else {
                identified_text("current", UtteranceRole::Source, "Synthetic current")
            });
            reducer.register_source_identity("extra");
            assert!(!reducer
                .identified_sources
                .iter()
                .any(|item| item.id == "current"));
            reducer.apply(if display {
                identified_pair("future", "Synthetic newer")
            } else {
                identified_text("future", UtteranceRole::Source, "Synthetic newer")
            });
            assert_eq!(reducer.snapshot.source.text, "Synthetic newer");
            if display {
                assert_eq!(
                    reducer.snapshot.display_pair.as_ref().unwrap().source,
                    "Synthetic newer"
                );
            }
            assert!(reducer.validate_state(100).is_ok());
        }
    }

    #[test]
    fn restored_known_frontiers_cannot_lag_their_visible_owners() {
        let mut reducer = SubtitleReducer::default();
        assert!(reducer.register_source_order("current", 100));
        reducer.apply(identified_pair("current", "Synthetic current"));
        for field in ["latest_known_source_order", "display_known_source_order"] {
            for forged in [serde_json::Value::Null, serde_json::json!(1)] {
                let mut state = serde_json::to_value(&reducer).unwrap();
                state[field] = forged;
                let restored: SubtitleReducer = serde_json::from_value(state).unwrap();
                assert!(restored.validate_state(100).is_err(), "{field}");
            }
        }
        let mut stable = SubtitleReducer::default();
        assert!(stable.register_source_order("maximum-stable", i64::MAX as u64));
        assert_eq!(stable.next_source_ordinal, 1);
        assert!(stable.validate_state(100).is_ok());
    }

    #[test]
    fn known_order_state_stays_bounded_and_rejects_forged_duplicate_ranks() {
        let mut reducer = SubtitleReducer::default();
        for order in 1..=200 {
            let id = format!("synthetic-{order}");
            assert!(reducer.register_source_order(&id, order));
            reducer.apply(identified_text(
                &id,
                UtteranceRole::Source,
                "Synthetic bounded",
            ));
        }
        assert_eq!(reducer.identified_sources.len(), MAX_IDENTIFIED_SOURCES);
        assert_eq!(reducer.discarded_source_ids.len(), MAX_IDENTIFIED_SOURCES);
        assert!(reducer.validate_state(100).is_ok());
        let mut forged = serde_json::to_value(&reducer).unwrap();
        forged["identified_sources"][1]["known_order"] =
            forged["identified_sources"][0]["known_order"].clone();
        let forged: SubtitleReducer = serde_json::from_value(forged).unwrap();
        assert!(forged.validate_state(100).is_err());
    }

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
        assert_eq!(
            reducer.snapshot.source.utterance_id,
            reducer.snapshot.display_pair.as_ref().unwrap().utterance_id
        );
        assert_eq!(
            reducer.snapshot.translation.utterance_id,
            reducer.snapshot.source.utterance_id
        );
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
    fn current_confirmation_time_survives_disabled_history_and_ignores_duplicates() {
        let mut reducer = SubtitleReducer::new(0);
        let event = SubtitleEvent::FinalPair {
            source: "Synthetic source".into(),
            translation: "Synthetic translation".into(),
        };
        reducer.apply(event.clone());
        let time = reducer.snapshot.display_pair_created_at_ms.unwrap();
        assert!(time > 0);
        assert!(reducer.snapshot.history.is_empty());
        reducer.apply(event);
        assert_eq!(reducer.snapshot.display_pair_created_at_ms, Some(time));
        reducer.apply(SubtitleEvent::Clear);
        assert_eq!(reducer.snapshot.display_pair_created_at_ms, None);
    }

    #[test]
    fn an_older_identified_final_cannot_replace_the_current_time() {
        let mut reducer = SubtitleReducer::default();
        for id in ["A", "B"] {
            reducer.apply(SubtitleEvent::UtteranceText {
                utterance_id: id.into(),
                role: UtteranceRole::Source,
                text: "Same synthetic source".into(),
                is_final: false,
            });
        }
        let event = |id: &str| SubtitleEvent::IdentifiedFinalPair {
            utterance_id: id.into(),
            source: "Same synthetic source".into(),
            translation: "Same synthetic translation".into(),
        };
        reducer.apply(event("B"));
        let time = reducer.snapshot.display_pair_created_at_ms;
        assert_eq!(
            time,
            reducer
                .snapshot
                .history
                .last()
                .map(|pair| pair.created_at_ms)
        );
        reducer.apply(event("A"));
        assert_eq!(reducer.snapshot.display_pair_created_at_ms, time);
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
    fn reliable_identity_registration_is_bounded_and_does_not_change_subtitles() {
        let mut reducer = SubtitleReducer::<NoopArchive>::new(6);
        let before = reducer.snapshot.clone();
        for index in 0..MAX_IDENTIFIED_SOURCES * 3 {
            reducer.register_source_identity(&format!("sentence-{index}"));
        }
        assert_eq!(reducer.snapshot, before);
        assert_eq!(reducer.identified_sources.len(), MAX_IDENTIFIED_SOURCES);
        assert_eq!(reducer.discarded_source_ids.len(), MAX_IDENTIFIED_SOURCES);
        assert!(reducer
            .identified_sources
            .iter()
            .all(|item| !item.confirmed));
        assert!(reducer.validate_state(6).is_ok());
        reducer.register_source_identity("");
        reducer.register_source_identity(&"x".repeat(crate::models::MAX_SUBTITLE_TEXT_BYTES + 1));
        assert_eq!(reducer.identified_sources.len(), MAX_IDENTIFIED_SOURCES);
        let last_id = format!("sentence-{}", MAX_IDENTIFIED_SOURCES * 3 - 1);
        reducer.apply(SubtitleEvent::Clear);
        reducer.register_source_identity(&last_id);
        assert!(reducer.identified_sources.is_empty());
        assert_eq!(reducer.snapshot, before);
    }

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
