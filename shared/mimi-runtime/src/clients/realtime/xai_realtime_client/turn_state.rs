//! Synchronous turn pairing state machine for xAI Grok Voice.

use crate::core::models::UtteranceRole;
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use std::collections::VecDeque;

pub(super) const MAXIMUM_TRANSCRIPT_BYTES: usize = 128 * 1_024;

#[derive(Default)]
pub(super) struct GrokTurnState {
    pub(super) item_id: Option<String>,
    pub(super) source: String,
    source_language: Option<String>,
    pub(super) source_complete: bool,
    source_committed: bool,
    speech_active: bool,
    pub(super) response_id: Option<String>,
    pub(super) translation: String,
    translation_complete: bool,
    response_complete: bool,
    pub(super) discard_current_turn: bool,
    pub(super) discarded_item_id: Option<String>,
    pub(super) pending_next_source: Option<PendingGrokSource>,
    pub(super) retired_source_ids: VecDeque<String>,
    pub(super) retired_response_ids: VecDeque<String>,
    pub(super) pairing_ambiguous: bool,
    discarded_pending_response: bool,
}

pub(super) struct PendingGrokSource {
    item_id: String,
    transcript: String,
    language: Option<String>,
    completed: bool,
    committed: bool,
    speech_active: bool,
}

impl GrokTurnState {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    fn reset_content(&mut self) {
        let retired_source_ids = std::mem::take(&mut self.retired_source_ids);
        let retired_response_ids = std::mem::take(&mut self.retired_response_ids);
        let discarded_pending_response = self.discarded_pending_response;
        self.reset();
        self.retired_source_ids = retired_source_ids;
        self.retired_response_ids = retired_response_ids;
        self.discarded_pending_response = discarded_pending_response;
    }

    pub(super) fn clear_content(&mut self) {
        let discard = self.discard_current_turn
            || self.item_id.is_some()
            || self.response_id.is_some()
            || !self.source.is_empty()
            || !self.translation.is_empty();
        let item_id = self
            .item_id
            .clone()
            .or_else(|| self.discarded_item_id.clone());
        let response_id = self.response_id.clone();
        self.retire_pending_source();
        self.reset_content();
        self.response_id = response_id;
        self.discard_current_turn = discard;
        self.discarded_item_id = item_id;
    }

    pub(super) fn update_source(
        &mut self,
        transcript: String,
        item_id: Option<String>,
        language: Option<String>,
        completed: bool,
    ) -> Vec<LiveTranslateServerEvent> {
        if item_id
            .as_ref()
            .is_some_and(|id| self.retired_source_ids.contains(id))
        {
            return Vec::new();
        }
        if self.reject_source_while_discarded_response_is_pending(item_id.as_deref()) {
            return Vec::new();
        }
        if self.discard_current_turn {
            return self.update_pending_next_source(transcript, item_id, language, completed);
        }
        if identifiers_conflict(self.item_id.as_deref(), item_id.as_deref()) {
            // VAD may start the next input before the preceding response ends.
            // Keep its source separate until that response's terminal event.
            if self.response_id.is_some() || self.source_committed || self.source_complete {
                return self.update_pending_next_source(transcript, item_id, language, completed);
            }
            self.advance_turn();
        }
        if item_id.is_some() {
            self.item_id = item_id;
        }
        self.source = transcript;
        if language.is_some() {
            self.source_language = language;
        }
        self.source_complete |= completed;
        self.source_committed |= completed;
        if completed {
            self.speech_active = false;
        }

        if self.exceeded_safety_limit() {
            return self.safety_limit_error();
        }

        let mut events = self
            .item_id
            .as_ref()
            .filter(|_| !self.source.is_empty())
            .map(|item_id| {
                vec![LiveTranslateServerEvent::UtteranceText {
                    utterance_id: item_id.clone(),
                    role: UtteranceRole::Source,
                    text: self.source.clone(),
                    is_final: false,
                    language: self.source_language.clone(),
                }]
            })
            .unwrap_or_default();
        if let Some(pair) = self.take_pair_if_complete() {
            events.push(pair);
        }
        events
    }

    pub(super) fn response_started(&mut self, response_id: Option<String>) -> bool {
        let Some(response_id) = response_id else {
            return false;
        };
        if self.retired_response_ids.contains(&response_id) {
            return false;
        }
        if identifiers_conflict(self.response_id.as_deref(), Some(&response_id)) {
            self.advance_turn();
        }
        self.response_id = Some(response_id);
        if self.item_id.is_none() {
            // The response carries no input item identity. Without a known
            // input boundary it cannot safely borrow a later source transcript.
            self.discard_current_turn = true;
        }
        !self.discard_current_turn
    }

    fn accepts_response(&mut self, response_id: Option<String>) -> bool {
        let Some(id) = response_id else { return false };
        if self.retired_response_ids.contains(&id)
            || identifiers_conflict(self.response_id.as_deref(), Some(&id))
        {
            return false;
        }
        if self.response_id.is_none() {
            self.response_id = Some(id);
        }
        if self.item_id.is_none() {
            self.discard_current_turn = true;
        }
        !self.discard_current_turn
    }

    pub(super) fn append_translation(
        &mut self,
        delta: String,
        response_id: Option<String>,
    ) -> Vec<LiveTranslateServerEvent> {
        if !self.accepts_response(response_id) {
            return Vec::new();
        }
        if delta.is_empty() {
            return Vec::new();
        }
        self.translation.push_str(&delta);
        if self.exceeded_safety_limit() {
            return self.safety_limit_error();
        }
        self.translation_preview().into_iter().collect()
    }

    pub(super) fn complete_translation(
        &mut self,
        final_transcript: Option<String>,
        response_id: Option<String>,
    ) -> Vec<LiveTranslateServerEvent> {
        if !self.accepts_response(response_id) {
            return Vec::new();
        }
        let mut events = Vec::new();
        if let Some(final_transcript) = final_transcript.filter(|text| !text.is_empty()) {
            if self.translation != final_transcript {
                self.translation = final_transcript;
                if self.exceeded_safety_limit() {
                    return self.safety_limit_error();
                }
                events.extend(self.translation_preview());
            }
        }
        self.translation_complete = true;
        if let Some(pair) = self.take_pair_if_complete() {
            events.push(pair);
        }
        events
    }

    fn translation_preview(&self) -> Option<LiveTranslateServerEvent> {
        self.item_id
            .as_ref()
            .map(|item_id| LiveTranslateServerEvent::UtteranceText {
                utterance_id: item_id.clone(),
                role: UtteranceRole::Translation,
                text: self.translation.clone(),
                // Transcript completion is still a preview until response.done
                // confirms success. Marking this final would bypass that gate.
                is_final: false,
                language: None,
            })
    }

    #[cfg(test)]
    pub(super) fn response_done(
        &mut self,
        response_id: Option<String>,
    ) -> Vec<LiveTranslateServerEvent> {
        self.finish_response(response_id, true).unwrap_or_default()
    }

    pub(super) fn finish_response(
        &mut self,
        response_id: Option<String>,
        successful: bool,
    ) -> Option<Vec<LiveTranslateServerEvent>> {
        let response_id = response_id?;
        if self.retired_response_ids.contains(&response_id)
            || identifiers_conflict(self.response_id.as_deref(), Some(&response_id))
        {
            return None;
        }
        self.response_id = Some(response_id);
        if self.item_id.is_none() {
            self.discard_current_turn = true;
        }
        if self.discard_current_turn || !successful {
            // A new source item can arrive before the discarded response has
            // finished. Promote that source only after the provider's
            // definitive response boundary so late translation events from the
            // oversized turn can never be paired with it.
            // The active/discarded A boundary comes first. The next discarded
            // response, which has no retained input, resolves discarded B.
            if self.item_id.is_none() && self.discarded_item_id.is_none() {
                self.discarded_pending_response = false;
            }
            self.advance_turn();
            return Some(Vec::new());
        }
        self.translation_complete = true;
        self.response_complete = true;
        Some(self.take_pair_if_complete().into_iter().collect())
    }

    fn take_pair_if_complete(&mut self) -> Option<LiveTranslateServerEvent> {
        if !self.source_complete
            || !self.translation_complete
            || !self.response_complete
            || self.item_id.is_none()
        {
            return None;
        }
        let event = (is_meaningful(&self.source) && is_meaningful(&self.translation)).then(|| {
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id: self.item_id.clone().unwrap(),
                source: self.source.trim().to_string(),
                language: self.source_language.clone(),
                translation: self.translation.trim().to_string(),
            }
        });
        self.advance_turn();
        event
    }

    fn exceeded_safety_limit(&self) -> bool {
        self.source.len().saturating_add(self.translation.len()) > MAXIMUM_TRANSCRIPT_BYTES
    }

    pub(super) fn has_unfinished_turn(&self) -> bool {
        self.speech_active
            || self.discarded_pending_response
            || self.source_committed
            || self.response_id.is_some()
            || self.pending_next_source.is_some()
    }

    fn advance_turn(&mut self) {
        let pending = self.pending_next_source.take();
        if let Some(id) = self
            .item_id
            .take()
            .or_else(|| self.discarded_item_id.take())
        {
            remember_retired_id(&mut self.retired_source_ids, id);
        }
        if let Some(id) = self.response_id.take() {
            remember_retired_id(&mut self.retired_response_ids, id);
        }
        self.reset_content();
        if let Some(pending) = pending {
            self.item_id = Some(pending.item_id);
            self.source = pending.transcript;
            self.source_language = pending.language;
            self.source_complete = pending.completed;
            self.source_committed = pending.committed;
            self.speech_active = pending.speech_active;
        }
    }

    pub(super) fn input_boundary(&mut self, item_id: Option<String>, committed: bool) {
        let Some(item_id) = item_id else { return };
        if self.retired_source_ids.contains(&item_id) {
            return;
        }
        if self.reject_source_while_discarded_response_is_pending(Some(&item_id)) {
            return;
        }
        if self.item_id.as_deref() == Some(&item_id) {
            self.source_committed |= committed;
            self.speech_active = !committed;
            return;
        }
        if self.discard_current_turn
            || self.response_id.is_some()
            || self.source_committed
            || self.source_complete
        {
            self.update_pending_next_source(String::new(), Some(item_id.clone()), None, false);
            if let Some(pending) = self
                .pending_next_source
                .as_mut()
                .filter(|pending| pending.item_id == item_id)
            {
                pending.committed |= committed;
                pending.speech_active = !committed;
            }
        } else {
            self.advance_turn();
            self.item_id = Some(item_id);
            self.source_committed = committed;
            self.speech_active = !committed;
        }
    }

    fn safety_limit_error(&mut self) -> Vec<LiveTranslateServerEvent> {
        let discarded_item_id = self.item_id.clone();
        let response_id = self.response_id.clone();
        self.retire_pending_source();
        self.reset_content();
        self.response_id = response_id;
        self.discard_current_turn = true;
        self.discarded_item_id = discarded_item_id;
        vec![LiveTranslateServerEvent::Error {
            code: "xai_transcript_safety_limit".into(),
            message: "xAI Grok Voice transcript buffering exceeded its safety limit.".into(),
        }]
    }

    fn retire_pending_source(&mut self) {
        if let Some(pending) = self.pending_next_source.take() {
            self.discarded_pending_response = true;
            remember_retired_id(&mut self.retired_source_ids, pending.item_id);
        }
    }

    fn reject_source_while_discarded_response_is_pending(&mut self, item_id: Option<&str>) -> bool {
        if !self.discarded_pending_response {
            return false;
        }
        let Some(item_id) = item_id else {
            return true;
        };
        if self.item_id.as_deref() == Some(item_id) {
            return false;
        }
        if self.discarded_item_id.as_deref() == Some(item_id)
            || self.retired_source_ids.iter().any(|id| id == item_id)
        {
            return true;
        }
        // C precedes a response for discarded B. Since responses carry no input
        // identity, borrowing C would be a guess. Recover once with no text.
        self.reset_content();
        self.discard_current_turn = true;
        self.pairing_ambiguous = true;
        true
    }

    fn update_pending_next_source(
        &mut self,
        transcript: String,
        item_id: Option<String>,
        language: Option<String>,
        completed: bool,
    ) -> Vec<LiveTranslateServerEvent> {
        let Some(item_id) = item_id else {
            return Vec::new();
        };
        if (self.discard_current_turn && self.discarded_item_id.is_none())
            || self.discarded_item_id.as_deref() == Some(&item_id)
            || self.retired_source_ids.contains(&item_id)
        {
            return Vec::new();
        }
        if self
            .pending_next_source
            .as_ref()
            .is_some_and(|pending| pending.item_id != item_id)
        {
            // Check identity before size: an oversized second pending item
            // cannot be collapsed into a single discarded-response tombstone.
            self.reset_content();
            self.discard_current_turn = true;
            self.pairing_ambiguous = true;
            return Vec::new();
        }
        if transcript.len() > MAXIMUM_TRANSCRIPT_BYTES {
            self.retire_pending_source();
            self.discarded_pending_response = true;
            remember_retired_id(&mut self.retired_source_ids, item_id);
            return vec![LiveTranslateServerEvent::Error {
                code: "xai_transcript_safety_limit".into(),
                message: "xAI Grok Voice transcript buffering exceeded its safety limit.".into(),
            }];
        }

        match self.pending_next_source.as_mut() {
            Some(pending) => {
                if !transcript.is_empty() || completed {
                    pending.transcript = transcript;
                }
                if language.is_some() {
                    pending.language = language;
                }
                pending.completed |= completed;
                pending.committed |= completed;
                if completed {
                    pending.speech_active = false;
                }
            }
            _ => {
                self.pending_next_source = Some(PendingGrokSource {
                    item_id,
                    transcript,
                    language,
                    completed,
                    committed: completed,
                    speech_active: !completed,
                });
            }
        }

        self.pending_next_source
            .as_ref()
            .filter(|pending| !pending.transcript.is_empty())
            .map(|pending| {
                vec![LiveTranslateServerEvent::UtteranceText {
                    utterance_id: pending.item_id.clone(),
                    role: UtteranceRole::Source,
                    text: pending.transcript.clone(),
                    is_final: false,
                    language: pending.language.clone(),
                }]
            })
            .unwrap_or_default()
    }
}

fn remember_retired_id(ids: &mut VecDeque<String>, id: String) {
    if ids.len() == 4 {
        ids.pop_front();
    }
    ids.push_back(id);
}

fn identifiers_conflict(current: Option<&str>, incoming: Option<&str>) -> bool {
    matches!((current, incoming), (Some(current), Some(incoming)) if current != incoming)
}

fn is_meaningful(text: &str) -> bool {
    text.chars()
        .any(|character| !character.is_whitespace() && character.is_alphanumeric())
}
