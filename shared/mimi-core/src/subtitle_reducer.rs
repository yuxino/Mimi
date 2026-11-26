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
        if source_id.is_some_and(|id| {
            self.display_pair_source_utterance_id
                .is_some_and(|owner| owner > id)
        }) {
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
        let duplicate = self.has_session_confirmation
            && !is_new
            && self.snapshot.display_pair_final
            && self
                .snapshot
                .display_pair
                .as_ref()
                .is_some_and(|pair| pair.source == source && pair.translation == translation);
        let promoted = self.update_final_display_pair(&source, &translation, source_id);
        // Display history may be disabled or cleared independently of the
        // platform archive. Its absence must not admit a replay of the
        // complete pair still on screen. A new recognition cycle or user
        // session remains a distinct confirmation, even with identical text.
        let time = if duplicate {
            None
        } else {
            self.append_confirmed_history(source, translation, is_new)
        };
        if promoted && time.is_some() {
            self.snapshot.display_pair_created_at_ms = time;
        }
    }

    fn append_confirmed_history(
        &mut self,
        source: String,
        translation: String,
        is_new_utterance: bool,
    ) -> Option<u64> {
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
mod tests;

#[cfg(test)]
mod common_state_tests;
