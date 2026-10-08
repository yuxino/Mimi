//! Provider-neutral session state controller.

use crate::core::audio_input::{AudioInput, AudioSource};
use crate::core::models::{
    DetectedLanguage, SessionStatus, SourceSubtitleSnapshot, SubtitleSnapshot, UtteranceRole,
};
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::subtitle_reducer::SubtitleReducer;
use std::time::{Duration, Instant};

/// Clear a provider's stuck waiting-for-final indicator without changing text.
pub const TRANSLATION_PENDING_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslationSessionState {
    pub status: SessionStatus,
    pub atomic_preview: Option<bool>,
    pub subtitles: SubtitleSnapshot,
    pub detected_language: Option<DetectedLanguage>,
    pub is_translation_pending: bool,
    pub is_translation_preview_pending: bool,
    pub is_translation_timed_out: bool,
    pub translation_recovery: Option<crate::core::diagnostics::TranslationRecovery>,
}

impl Default for TranslationSessionState {
    fn default() -> Self {
        Self {
            status: SessionStatus::Idle,
            atomic_preview: None,
            subtitles: SubtitleSnapshot::empty(),
            detected_language: None,
            is_translation_pending: false,
            is_translation_preview_pending: false,
            is_translation_timed_out: false,
            translation_recovery: None,
        }
    }
}

#[derive(Default)]
struct SourceSessionController {
    pub state: TranslationSessionState,
    subtitle_reducer: SubtitleReducer,
    preview_pending_id: Option<u64>,
    translation_pending_deadline: Option<Instant>,
}

impl SourceSessionController {
    pub fn accepts_confirmed_pair(
        &self,
        utterance_id: u64,
        source: &str,
        translation: &str,
    ) -> bool {
        !source.trim().is_empty()
            && !translation.trim().is_empty()
            && crate::core::models::subtitle_text_within_limit(source)
            && crate::core::models::subtitle_text_within_limit(translation)
            && self.subtitle_reducer.is_new_confirmation_id(utterance_id)
    }

    fn accepts_identified_final_pair(&self, id: &str, source: &str, translation: &str) -> bool {
        self.subtitle_reducer
            .accepts_identified_final_pair(id, source, translation)
    }

    fn clear_preview_pending(&mut self) {
        self.preview_pending_id = None;
        self.state.is_translation_preview_pending = false;
    }

    pub fn begin_session(&mut self) {
        self.subtitle_reducer.begin_new_session();
        self.begin_connecting();
    }

    pub fn begin_connecting(&mut self) {
        self.clear_preview_pending();
        self.state.translation_recovery = None;
        self.subtitle_reducer.reset_transient();
        self.state.subtitles = self.subtitle_reducer.snapshot.clone();
        self.state.status = SessionStatus::Connecting;
        self.state.detected_language = None;
        self.state.is_translation_pending = false;
        self.translation_pending_deadline = None;
        self.state.is_translation_timed_out = false;
    }

    pub fn did_connect(&mut self) {
        self.clear_preview_pending();
        self.state.translation_recovery = None;
        self.state.status = SessionStatus::Listening;
    }

    pub fn did_pause(&mut self) {
        self.clear_preview_pending();
        self.state.translation_recovery = None;
        self.state.status = SessionStatus::Listening;
        self.state.is_translation_pending = false;
        self.translation_pending_deadline = None;
    }

    /// Marks the active translation as timed out and clears its
    /// waiting-for-final indicator without touching status or subtitles. The
    /// explicit timeout bit lets presentation distinguish an identical new
    /// utterance from the latest already-committed history pair.
    pub fn clear_translation_pending(&mut self) {
        self.state.is_translation_pending = false;
        self.state.is_translation_timed_out = true;
        self.translation_pending_deadline = None;
    }

    pub fn begin_stopping(&mut self) {
        self.clear_preview_pending();
        self.state.translation_recovery = None;
        self.state.status = SessionStatus::Stopping;
        self.state.is_translation_pending = false;
        self.translation_pending_deadline = None;
    }

    pub fn did_stop(&mut self) {
        self.clear_preview_pending();
        self.state.translation_recovery = None;
        self.state.status = SessionStatus::Idle;
        self.state.is_translation_pending = false;
        self.translation_pending_deadline = None;
        self.state.is_translation_timed_out = false;
    }

    pub fn did_fail(&mut self, message: impl Into<String>) {
        self.clear_preview_pending();
        self.state.translation_recovery = None;
        self.state.status = SessionStatus::Error(message.into());
        self.state.is_translation_pending = false;
        self.translation_pending_deadline = None;
    }

    pub fn clear_subtitles(&mut self) {
        self.clear_preview_pending();
        self.state.is_translation_pending = false;
        self.translation_pending_deadline = None;
        self.state.translation_recovery = None;
        self.subtitle_reducer
            .apply(crate::core::models::SubtitleEvent::Clear);
        self.state.subtitles = self.subtitle_reducer.snapshot.clone();
        self.state.is_translation_timed_out = false;
    }

    #[cfg(test)]
    pub fn handle(&mut self, event: LiveTranslateServerEvent) {
        self.handle_at(event, Instant::now());
    }

    fn handle_at(&mut self, event: LiveTranslateServerEvent, now: Instant) {
        if !event.text_within_limit() {
            return;
        }
        // Alibaba teardown can emit synthetic source/translation cleanup
        // finals, which are intentionally ignored. OpenAI has no separate
        // final events: a real `session.closed` may flush one client-aligned
        // atomic pair, and that verified tail is safe to keep.
        if self.state.status == SessionStatus::Stopping
            && !matches!(
                event,
                LiveTranslateServerEvent::SubtitleFinalPair { .. }
                    | LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
                    | LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
            )
        {
            return;
        }

        match event {
            LiveTranslateServerEvent::SessionCreated => {}
            LiveTranslateServerEvent::SessionUpdated => self.did_connect(),
            LiveTranslateServerEvent::SourceDraft { text, language } => {
                self.update_detected_language(language.as_deref());
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::SourceDraft(text));
            }
            LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id,
                text,
                language,
            } => {
                self.update_detected_language(language.as_deref());
                self.subtitle_reducer.apply(
                    crate::core::models::SubtitleEvent::SourceUtteranceDraft { utterance_id, text },
                );
            }
            LiveTranslateServerEvent::SourceFinal { text, language }
            | LiveTranslateServerEvent::SourceUtteranceFinal { text, language, .. } => {
                self.update_detected_language(language.as_deref());
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::SourceFinal(text));
            }
            LiveTranslateServerEvent::TranslationStarted => {
                self.clear_preview_pending();
                self.state.translation_recovery = None;
                self.state.is_translation_pending = true;
                self.translation_pending_deadline = Some(now + TRANSLATION_PENDING_TIMEOUT);
                self.state.is_translation_timed_out = false;
            }
            LiveTranslateServerEvent::PreviewTranslationStarted { request_id } => {
                self.preview_pending_id = Some(request_id);
                self.state.is_translation_preview_pending = true;
                self.state.translation_recovery = None;
            }
            LiveTranslateServerEvent::PreviewTranslationFinished { request_id } => {
                if self.preview_pending_id == Some(request_id) {
                    self.clear_preview_pending();
                }
            }
            LiveTranslateServerEvent::TranslationDeferred(recovery) => {
                self.clear_preview_pending();
                self.state.translation_recovery = Some(recovery);
                self.state.is_translation_pending = false;
                self.state.is_translation_timed_out = false;
            }
            LiveTranslateServerEvent::TranslationDraft(text) => {
                if !text.trim().is_empty() {
                    self.state.translation_recovery = None;
                }
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::TranslationDraft(text));
            }
            LiveTranslateServerEvent::SubtitlePreviewPair {
                source_utterance_id,
                source,
                language,
                translation,
            } => {
                self.update_detected_language(language.as_deref());
                self.state.translation_recovery = None;
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::PreviewPair {
                        source_utterance_id,
                        source,
                        translation,
                    });
            }
            LiveTranslateServerEvent::SubtitlePreviewCleared => {
                self.clear_preview_pending();
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::ClearPreview);
            }
            LiveTranslateServerEvent::UtteranceText {
                utterance_id,
                role,
                text,
                is_final,
                language,
            } => {
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::UtteranceText {
                        utterance_id: utterance_id.clone(),
                        role,
                        text,
                        is_final,
                    });
                if role == UtteranceRole::Source
                    && self
                        .subtitle_reducer
                        .snapshot
                        .source
                        .utterance_id
                        .as_deref()
                        == Some(&utterance_id)
                {
                    self.update_detected_language(language.as_deref());
                }
            }
            LiveTranslateServerEvent::TranslationFinal(text) => {
                self.clear_preview_pending();
                self.state.translation_recovery = None;
                self.state.is_translation_pending = false;
                self.state.is_translation_timed_out = false;
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::TranslationFinal(text));
            }
            LiveTranslateServerEvent::SubtitleFinalPair {
                source,
                language,
                translation,
            } => {
                self.clear_preview_pending();
                self.state.translation_recovery = None;
                self.update_detected_language(language.as_deref());
                self.state.is_translation_pending = false;
                self.state.is_translation_timed_out = false;
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::FinalPair {
                        source,
                        translation,
                    });
            }
            LiveTranslateServerEvent::SubtitleConfirmedPair {
                utterance_id,
                source_utterance_id,
                source,
                language,
                translation,
            } => {
                if !self.accepts_confirmed_pair(utterance_id, &source, &translation) {
                    return;
                }
                self.clear_preview_pending();
                self.state.translation_recovery = None;
                self.update_detected_language(language.as_deref());
                self.state.is_translation_pending = false;
                self.state.is_translation_timed_out = false;
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::ConfirmedPair {
                        utterance_id,
                        source_utterance_id,
                        source,
                        translation,
                    });
            }
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id,
                source,
                language,
                translation,
            } => {
                if !self.accepts_identified_final_pair(&utterance_id, &source, &translation) {
                    return;
                }
                if self
                    .subtitle_reducer
                    .identified_source_is_current(&utterance_id)
                {
                    self.clear_preview_pending();
                    self.state.translation_recovery = None;
                    self.state.is_translation_pending = false;
                    self.state.is_translation_timed_out = false;
                }
                self.subtitle_reducer.apply(
                    crate::core::models::SubtitleEvent::IdentifiedFinalPair {
                        utterance_id: utterance_id.clone(),
                        source,
                        translation,
                    },
                );
                if self
                    .subtitle_reducer
                    .snapshot
                    .source
                    .utterance_id
                    .as_deref()
                    == Some(&utterance_id)
                {
                    self.update_detected_language(language.as_deref());
                }
            }
            LiveTranslateServerEvent::SessionFinished => self.did_stop(),
            LiveTranslateServerEvent::Error { message, .. } => self.did_fail(message),
            LiveTranslateServerEvent::Ignored { .. } => return,
        }

        self.state.subtitles = self.subtitle_reducer.snapshot.clone();
        if !self.state.is_translation_pending {
            self.translation_pending_deadline = None;
        }
    }

    fn update_detected_language(&mut self, reported_language: Option<&str>) {
        if let Some(language) = DetectedLanguage::from_reported(reported_language) {
            self.state.detected_language = Some(language);
        }
    }
}

/// A session owns two independent subtitle state machines, selected explicitly.
/// Its public history is ordered by final confirmation; provider IDs remain local
/// to each input, and each input retains a separate bounded preview/history.
pub struct TranslationSessionController {
    pub state: TranslationSessionState,
    atomic_preview: Option<bool>,
    audio_input: AudioInput,
    sources: [SourceSessionController; 2],
    archive: super::session_archive::TranscriptArchive,
}

impl Default for TranslationSessionController {
    fn default() -> Self {
        let system = SourceSessionController::default();
        let mut microphone = SourceSessionController::default();
        microphone.subtitle_reducer.audio_source = AudioSource::Microphone;
        Self {
            state: TranslationSessionState::default(),
            atomic_preview: None,
            audio_input: AudioInput::System,
            sources: [system, microphone],
            archive: Default::default(),
        }
    }
}

fn source_index(source: AudioSource) -> usize {
    match source {
        AudioSource::System => 0,
        AudioSource::Microphone => 1,
    }
}

impl TranslationSessionController {
    pub fn set_atomic_preview(&mut self, paired: bool) {
        self.atomic_preview = Some(paired);
        self.state.atomic_preview = Some(paired);
    }

    pub fn set_audio_input(&mut self, audio_input: AudioInput) {
        if self.audio_input != audio_input {
            self.reconfigure_audio_input(audio_input);
        }
    }

    /// Reconfigure a live/paused session without erasing confirmed subtitles
    /// or opt-in transcript history. Old transport previews cannot survive.
    /// The same source selection may now capture a different application;
    /// every accepted capture restart needs fresh provider ID watermarks.
    pub fn reconfigure_audio_input(&mut self, audio_input: AudioInput) {
        let status = self.state.status.clone();
        for source in &mut self.sources {
            source.begin_connecting();
            source.state.status = status.clone();
        }
        self.audio_input = audio_input;
        self.refresh();
    }

    pub fn archive(&self) -> &super::session_archive::TranscriptArchive {
        &self.archive
    }

    pub fn archive_mut(&mut self) -> &mut super::session_archive::TranscriptArchive {
        &mut self.archive
    }

    pub fn accepts_confirmed_pair_from(
        &self,
        audio_source: AudioSource,
        utterance_id: u64,
        source: &str,
        translation: &str,
    ) -> bool {
        self.audio_input.sources().contains(&audio_source)
            && self.sources[source_index(audio_source)].accepts_confirmed_pair(
                utterance_id,
                source,
                translation,
            )
    }

    pub fn accepts_identified_final_pair_from(
        &self,
        source: AudioSource,
        id: &str,
        text: &str,
        translation: &str,
    ) -> bool {
        self.audio_input.sources().contains(&source)
            && self.sources[source_index(source)].accepts_identified_final_pair(
                id,
                text,
                translation,
            )
    }

    pub fn identified_source_is_current_from(&self, source: AudioSource, id: &str) -> bool {
        self.audio_input.sources().contains(&source)
            && self.sources[source_index(source)]
                .subtitle_reducer
                .identified_source_is_current(id)
    }

    fn apply_to_sources(&mut self, action: impl Fn(&mut SourceSessionController)) {
        for source in self.audio_input.sources() {
            action(&mut self.sources[source_index(*source)]);
        }
        self.refresh();
    }

    pub fn begin_session(&mut self) {
        // A source enabled later must share this session's admission boundary,
        // even when it is dormant at the manual start.
        for source in &mut self.sources {
            source.begin_session();
        }
        self.refresh();
    }

    pub fn begin_connecting(&mut self) {
        self.apply_to_sources(SourceSessionController::begin_connecting);
    }

    pub fn did_connect(&mut self) {
        self.apply_to_sources(SourceSessionController::did_connect);
    }

    pub fn did_pause(&mut self) {
        self.apply_to_sources(SourceSessionController::did_pause);
    }

    pub fn begin_stopping(&mut self) {
        self.apply_to_sources(SourceSessionController::begin_stopping);
    }

    pub fn did_stop(&mut self) {
        self.apply_to_sources(SourceSessionController::did_stop);
    }

    pub fn did_fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.apply_to_sources(|source| source.did_fail(message.clone()));
    }

    pub fn clear_translation_pending_from(&mut self, source: AudioSource) {
        if self.audio_input.sources().contains(&source) {
            self.sources[source_index(source)].clear_translation_pending();
            self.refresh();
        }
    }

    pub fn translation_pending_deadline_from(&self, source: AudioSource) -> Option<Instant> {
        self.audio_input
            .sources()
            .contains(&source)
            .then(|| self.sources[source_index(source)].translation_pending_deadline)
            .flatten()
    }

    /// Expire only the currently pending request; a late timer cannot clear a
    /// newer request whose deadline has not elapsed.
    pub fn expire_translation_pending_from(&mut self, source: AudioSource, now: Instant) -> bool {
        if self
            .translation_pending_deadline_from(source)
            .is_none_or(|deadline| now < deadline)
        {
            return false;
        }
        self.sources[source_index(source)].clear_translation_pending();
        self.refresh();
        true
    }

    /// Advance shared session deadlines. Native drivers may call this while
    /// polling snapshots, without reproducing the timeout policy.
    pub fn tick(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for source in [AudioSource::System, AudioSource::Microphone] {
            changed |= self.expire_translation_pending_from(source, now);
        }
        changed
    }

    pub fn clear_subtitles(&mut self) {
        for source in &mut self.sources {
            source.clear_subtitles();
        }
        self.archive.clear();
        self.state.subtitles.history.clear();
        self.refresh();
    }

    pub fn handle(&mut self, event: LiveTranslateServerEvent) {
        self.handle_from(self.audio_input.sources()[0], event);
    }

    pub fn handle_from(&mut self, audio_source: AudioSource, event: LiveTranslateServerEvent) {
        self.handle_from_at(audio_source, event, Instant::now());
    }

    /// Injectable monotonic clock for both native drivers and offline replay.
    pub fn handle_from_at(
        &mut self,
        audio_source: AudioSource,
        event: LiveTranslateServerEvent,
        now: Instant,
    ) {
        if !self.audio_input.sources().contains(&audio_source) || !event.text_within_limit() {
            return;
        }
        let lane = &mut self.sources[source_index(audio_source)];
        let before = lane
            .state
            .subtitles
            .history
            .last()
            .map(|pair| pair.created_at_ms);
        lane.handle_at(event, now);
        if let Some(pair) = lane.state.subtitles.history.last_mut() {
            if before != Some(pair.created_at_ms) {
                // The aggregate list must stay monotonically ordered even when
                // both independent providers confirm within one millisecond.
                if let Some(previous) = self.state.subtitles.history.last() {
                    pair.created_at_ms = pair
                        .created_at_ms
                        .max(previous.created_at_ms.saturating_add(1));
                }
                if let Some(reduced) = lane.subtitle_reducer.snapshot.history.last_mut() {
                    reduced.created_at_ms = pair.created_at_ms;
                }
                self.archive.append(pair);
                self.state.subtitles.history.push(pair.clone());
                if self.state.subtitles.history.len() > 20 {
                    self.state.subtitles.history.remove(0);
                }
            }
        }
        self.refresh();
    }

    fn refresh(&mut self) {
        let history = std::mem::take(&mut self.state.subtitles.history);
        let active: Vec<_> = self
            .audio_input
            .sources()
            .iter()
            .map(|source| (*source, &self.sources[source_index(*source)].state))
            .collect();
        self.state = active[0].1.clone();
        self.state.atomic_preview = self.atomic_preview;
        self.state.subtitles.history = history;
        self.state.subtitles.tracks = active
            .iter()
            .map(|(source, state)| SourceSubtitleSnapshot {
                audio_source: *source,
                source: state.subtitles.source.clone(),
                translation: state.subtitles.translation.clone(),
                history: state.subtitles.history.clone(),
                preview_pair: state.subtitles.preview_pair.clone(),
                realtime_preview: state.subtitles.realtime_preview.clone(),
                display_pair: state.subtitles.display_pair.clone(),
                display_pair_final: state.subtitles.display_pair_final,
                detected_language: state
                    .detected_language
                    .as_ref()
                    .map(|language| language.code.clone()),
                is_translation_pending: state.is_translation_pending,
                is_translation_preview_pending: state.is_translation_preview_pending,
                is_translation_timed_out: state.is_translation_timed_out,
                translation_recovery: state.translation_recovery,
            })
            .collect();
        self.state.status = active
            .iter()
            .max_by_key(|(_, state)| match &state.status {
                SessionStatus::Error(_) => 4,
                SessionStatus::Stopping => 3,
                SessionStatus::Connecting => 2,
                SessionStatus::Listening => 1,
                SessionStatus::Idle => 0,
            })
            .map(|(_, state)| state.status.clone())
            .unwrap_or(SessionStatus::Idle);
        self.state.is_translation_pending =
            active.iter().any(|(_, state)| state.is_translation_pending);
        self.state.is_translation_preview_pending = active
            .iter()
            .any(|(_, state)| state.is_translation_preview_pending);
        self.state.is_translation_timed_out = active
            .iter()
            .any(|(_, state)| state.is_translation_timed_out);
        self.state.translation_recovery = active
            .iter()
            .find_map(|(_, state)| state.translation_recovery);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_deadline_is_shared_and_drafts_do_not_extend_it() {
        let now = Instant::now();
        let mut controller = TranslationSessionController::default();
        controller.did_connect();
        controller.handle_from_at(
            AudioSource::System,
            LiveTranslateServerEvent::TranslationStarted,
            now,
        );
        let deadline = now + TRANSLATION_PENDING_TIMEOUT;
        assert_eq!(
            controller.translation_pending_deadline_from(AudioSource::System),
            Some(deadline)
        );
        controller.handle_from_at(
            AudioSource::System,
            LiveTranslateServerEvent::TranslationDraft("草稿".into()),
            now + Duration::from_secs(5),
        );
        let mut before = controller.state.subtitles.clone();
        before.tracks.clear();
        assert!(!controller.tick(deadline - Duration::from_nanos(1)));
        assert!(controller.tick(deadline));
        assert!(!controller.state.is_translation_pending);
        assert!(controller.state.is_translation_timed_out);
        assert_eq!(controller.state.status, SessionStatus::Listening);
        let mut after = controller.state.subtitles.clone();
        after.tracks.clear();
        assert_eq!(after, before);
        assert!(!controller.tick(deadline + Duration::from_secs(1)));
    }

    #[test]
    fn a_new_request_replaces_the_deadline_and_final_or_stop_cancels_it() {
        let now = Instant::now();
        let mut controller = TranslationSessionController::default();
        controller.handle_from_at(
            AudioSource::System,
            LiveTranslateServerEvent::TranslationStarted,
            now,
        );
        controller.handle_from_at(
            AudioSource::System,
            LiveTranslateServerEvent::TranslationStarted,
            now + Duration::from_secs(5),
        );
        assert!(!controller.expire_translation_pending_from(
            AudioSource::System,
            now + TRANSLATION_PENDING_TIMEOUT
        ));
        controller.handle_from_at(
            AudioSource::System,
            LiveTranslateServerEvent::TranslationFinal("译文".into()),
            now + Duration::from_secs(7),
        );
        assert_eq!(
            controller.translation_pending_deadline_from(AudioSource::System),
            None
        );
        assert!(!controller.tick(now + Duration::from_secs(12)));
        controller.handle_from_at(
            AudioSource::System,
            LiveTranslateServerEvent::TranslationStarted,
            now + Duration::from_secs(13),
        );
        controller.did_stop();
        assert_eq!(
            controller.translation_pending_deadline_from(AudioSource::System),
            None
        );
    }

    #[test]
    fn pending_deadlines_remain_independent_per_audio_source() {
        let now = Instant::now();
        let mut controller = TranslationSessionController::default();
        controller.set_audio_input(AudioInput::Both);
        controller.handle_from_at(
            AudioSource::System,
            LiveTranslateServerEvent::TranslationStarted,
            now,
        );
        controller.handle_from_at(
            AudioSource::Microphone,
            LiveTranslateServerEvent::TranslationStarted,
            now + Duration::from_secs(2),
        );
        assert!(controller.tick(now + TRANSLATION_PENDING_TIMEOUT));
        assert!(controller.state.is_translation_pending);
        assert_eq!(
            controller.translation_pending_deadline_from(AudioSource::System),
            None
        );
        assert_eq!(
            controller.translation_pending_deadline_from(AudioSource::Microphone),
            Some(now + Duration::from_secs(8))
        );
        controller.clear_subtitles();
        assert!(!controller.tick(now + Duration::from_secs(20)));
    }

    #[test]
    fn identified_late_final_keeps_newer_text_language_and_pending_owner() {
        let mut controller = SourceSessionController::default();
        let source =
            |id: &str, text: &str, language: &str| LiveTranslateServerEvent::UtteranceText {
                utterance_id: id.into(),
                role: UtteranceRole::Source,
                text: text.into(),
                is_final: true,
                language: Some(language.into()),
            };
        let pair = |id: &str, text: &str, language: &str| {
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id: id.into(),
                source: text.into(),
                translation: format!("译文 {text}"),
                language: Some(language.into()),
            }
        };
        controller.handle(source("A", "Older sentence", "en"));
        controller.handle(source("B", "새로운 문장", "ko"));
        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        controller.handle(pair("A", "Older sentence", "en"));
        assert!(controller.state.is_translation_pending);
        assert_eq!(
            controller.state.subtitles.source.utterance_id.as_deref(),
            Some("B")
        );
        assert_eq!(
            controller.state.detected_language.as_ref().unwrap().code,
            "ko"
        );
        controller.handle(pair("B", "새로운 문장", "ko"));
        let expected = controller.state.clone();
        controller.handle(source("A", "Older sentence", "en"));
        controller.handle(pair("A", "Older sentence", "en"));
        assert_eq!(controller.state, expected);
        assert_eq!(
            controller
                .state
                .subtitles
                .display_pair
                .as_ref()
                .unwrap()
                .utterance_id
                .as_deref(),
            Some("B")
        );
        assert_eq!(controller.state.subtitles.history.len(), 2);
        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        let pending = controller.state.clone();
        controller.handle(pair("A", "Older sentence", "en"));
        assert_eq!(controller.state, pending);
    }

    #[test]
    fn identified_pair_is_atomic_while_stopping_and_repeated_ids_remain_distinct() {
        let mut controller = TranslationSessionController::default();
        for id in ["first", "second"] {
            controller.handle(LiveTranslateServerEvent::UtteranceText {
                utterance_id: id.into(),
                role: UtteranceRole::Source,
                text: "Repeat sentence".into(),
                is_final: true,
                language: Some("en".into()),
            });
            controller.begin_stopping();
            controller.handle(LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
                utterance_id: id.into(),
                source: "Repeat sentence".into(),
                translation: "重复句子".into(),
                language: Some("en".into()),
            });
            controller.did_connect();
        }
        assert_eq!(controller.state.subtitles.history.len(), 2);
        assert!(
            controller.state.subtitles.history[0].created_at_ms
                < controller.state.subtitles.history[1].created_at_ms
        );
    }

    #[test]
    fn replayed_confirmation_cannot_clear_pending_work_or_rewind_a_live_pair() {
        let mut controller = SourceSessionController::default();
        let replay = LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: None,
            utterance_id: 1,
            source: "Synthetic final".into(),
            language: Some("en".into()),
            translation: "Synthetic final translation".into(),
        };
        controller.handle(replay.clone());
        controller.handle(LiveTranslateServerEvent::SubtitlePreviewPair {
            source_utterance_id: None,
            source: "New synthetic source".into(),
            language: Some("ja".into()),
            translation: "New synthetic translation".into(),
        });
        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        let expected = controller.state.clone();
        controller.handle(replay);
        assert_eq!(controller.state, expected);
        controller.handle(LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: None,
            utterance_id: 2,
            source: " ".into(),
            language: Some("en".into()),
            translation: "Synthetic incomplete result".into(),
        });
        assert_eq!(controller.state, expected);
    }

    #[test]
    fn oversized_events_cannot_clear_pending_recovery_or_claim_a_confirmation() {
        let mut controller = SourceSessionController::default();
        controller.handle(LiveTranslateServerEvent::SubtitlePreviewPair {
            source_utterance_id: None,
            source: "Synthetic complete source".into(),
            language: Some("ja".into()),
            translation: "Synthetic complete translation".into(),
        });
        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 7 });
        controller.state.translation_recovery =
            Some(crate::core::diagnostics::TranslationRecovery {
                reason: crate::core::diagnostics::TranslationRecoveryReason::RateLimited,
                retry_after_ms: 4_000,
                retry_scheduled: true,
            });
        let expected = controller.state.clone();
        let oversized = "x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1);
        for event in [
            LiveTranslateServerEvent::SourceUtteranceFinal {
                utterance_id: 1,
                text: oversized.clone(),
                language: Some("en".into()),
            },
            LiveTranslateServerEvent::TranslationDraft(oversized.clone()),
            LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: None,
                utterance_id: 1,
                source: "valid".into(),
                language: Some("en".into()),
                translation: oversized,
            },
        ] {
            controller.handle(event);
            assert_eq!(controller.state, expected);
        }
        controller.handle(LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: None,
            utterance_id: 1,
            source: "Synthetic valid final".into(),
            language: Some("en".into()),
            translation: "Synthetic valid translation".into(),
        });
        assert_eq!(controller.state.subtitles.history.len(), 1);
        assert!(!controller.state.is_translation_pending);
        assert!(!controller.state.is_translation_preview_pending);
    }

    #[test]
    fn preview_http_pending_is_owner_matched_and_does_not_change_final_pairing() {
        let mut controller = SourceSessionController::default();
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 1 });
        assert!(controller.state.is_translation_preview_pending);
        assert!(!controller.state.is_translation_pending);
        controller.handle(LiveTranslateServerEvent::SourceDraft {
            text: "Synthetic source".into(),
            language: Some("en".into()),
        });
        controller.handle(LiveTranslateServerEvent::TranslationDraft(
            "Synthetic partial".into(),
        ));
        assert!(controller.state.is_translation_preview_pending);
        assert!(controller.state.subtitles.history.is_empty());
        controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 2 });
        controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 1 });
        assert!(controller.state.is_translation_preview_pending);
        controller.clear_translation_pending();
        assert!(controller.state.is_translation_preview_pending);
        controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 2 });
        assert!(!controller.state.is_translation_preview_pending);
        assert!(!controller.state.is_translation_pending);
        assert!(controller.state.subtitles.history.is_empty());
    }

    #[test]
    fn preview_cleanup_cannot_clear_a_final_and_final_start_clears_old_preview() {
        let mut controller = SourceSessionController::default();
        controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 1 });
        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        assert!(!controller.state.is_translation_preview_pending);
        assert!(controller.state.is_translation_pending);
        controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 1 });
        assert!(controller.state.is_translation_pending);
        controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 2 });
        controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 2 });
        assert!(!controller.state.is_translation_preview_pending);
        assert!(controller.state.is_translation_pending);
    }

    #[test]
    fn preview_pending_is_cleared_on_each_session_lifecycle_boundary() {
        let boundaries: [fn(&mut SourceSessionController); 6] = [
            SourceSessionController::begin_connecting,
            SourceSessionController::did_connect,
            SourceSessionController::did_pause,
            SourceSessionController::begin_stopping,
            SourceSessionController::did_stop,
            |controller| controller.did_fail("synthetic failure"),
        ];
        for boundary in boundaries {
            let mut controller = SourceSessionController::default();
            controller
                .handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 1 });
            boundary(&mut controller);
            assert!(!controller.state.is_translation_preview_pending);
            assert!(controller.preview_pending_id.is_none());
        }
    }

    #[test]
    fn translation_backoff_is_nonterminal_and_lifecycle_or_valid_text_clears_it() {
        use crate::core::diagnostics::{TranslationRecovery, TranslationRecoveryReason};
        let recovery = TranslationRecovery {
            reason: TranslationRecoveryReason::RateLimited,
            retry_after_ms: 4_000,
            retry_scheduled: true,
        };
        let mut controller = SourceSessionController::default();
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        controller.handle(LiveTranslateServerEvent::TranslationDeferred(recovery));
        assert_eq!(controller.state.status, SessionStatus::Listening);
        assert!(controller.state.status.is_active());
        assert!(!controller.state.is_translation_pending);
        assert!(!controller.state.is_translation_timed_out);
        assert_eq!(controller.state.translation_recovery, Some(recovery));
        let idle_recovery = TranslationRecovery {
            retry_scheduled: false,
            ..recovery
        };
        controller.handle(LiveTranslateServerEvent::TranslationDeferred(idle_recovery));
        controller.handle(LiveTranslateServerEvent::SourceDraft {
            text: "recognition keeps running without a scheduled retry".into(),
            language: Some("en".into()),
        });
        assert_eq!(controller.state.translation_recovery, Some(idle_recovery));
        assert_eq!(controller.state.status, SessionStatus::Listening);
        assert!(!controller.state.is_translation_pending);
        controller.handle(LiveTranslateServerEvent::TranslationDeferred(recovery));
        controller.handle(LiveTranslateServerEvent::SourceDraft {
            text: "new synthetic recognition".into(),
            language: Some("en".into()),
        });
        assert_eq!(controller.state.translation_recovery, Some(recovery));
        controller.handle(LiveTranslateServerEvent::TranslationDraft(
            "valid synthetic preview".into(),
        ));
        assert_eq!(controller.state.translation_recovery, None);
        for finish in [
            SourceSessionController::did_pause,
            SourceSessionController::begin_stopping,
            SourceSessionController::did_stop,
            SourceSessionController::begin_connecting,
        ] {
            controller.handle(LiveTranslateServerEvent::TranslationDeferred(recovery));
            finish(&mut controller);
            assert_eq!(controller.state.translation_recovery, None);
        }
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::TranslationDeferred(recovery));
        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        assert_eq!(controller.state.translation_recovery, None);
        controller.handle(LiveTranslateServerEvent::TranslationDeferred(recovery));
        controller.did_fail("credential_authentication_failed");
        assert_eq!(controller.state.translation_recovery, None);
        assert!(!controller.state.status.is_active());
    }

    #[test]
    fn session_follows_the_happy_path_lifecycle() {
        let mut controller = SourceSessionController::default();

        controller.begin_connecting();
        assert_eq!(controller.state.status, SessionStatus::Connecting);

        controller.did_connect();
        assert_eq!(controller.state.status, SessionStatus::Listening);

        controller.begin_stopping();
        assert_eq!(controller.state.status, SessionStatus::Stopping);

        controller.did_stop();
        assert_eq!(controller.state.status, SessionStatus::Idle);
    }

    #[test]
    fn server_events_update_subtitle_state() {
        let mut controller = SourceSessionController::default();
        controller.handle(LiveTranslateServerEvent::SourceDraft {
            text: "Hello wor".into(),
            language: Some("en".into()),
        });
        controller.handle(LiveTranslateServerEvent::TranslationDraft(
            "你好，世".into(),
        ));

        assert_eq!(controller.state.subtitles.source.text, "Hello wor");
        assert!(!controller.state.subtitles.source.is_final);
        assert_eq!(controller.state.subtitles.translation.text, "你好，世");

        controller.handle(LiveTranslateServerEvent::SourceFinal {
            text: "Hello world.".into(),
            language: Some("en".into()),
        });
        controller.handle(LiveTranslateServerEvent::TranslationFinal(
            "你好，世界。".into(),
        ));

        assert_eq!(controller.state.subtitles.history.len(), 1);
        assert!(controller.state.subtitles.translation.is_final);
        assert_eq!(
            controller.state.detected_language.as_ref().unwrap().code,
            "en"
        );
    }

    #[test]
    fn a_new_connection_clears_the_previously_detected_language() {
        let mut controller = SourceSessionController::default();
        controller.handle(LiveTranslateServerEvent::SourceDraft {
            text: "こんにちは".into(),
            language: Some("ja".into()),
        });
        assert_eq!(
            controller.state.detected_language.as_ref().unwrap().code,
            "ja"
        );

        controller.begin_connecting();
        assert_eq!(controller.state.detected_language, None);
    }

    #[test]
    fn service_errors_move_the_session_to_error() {
        let mut controller = SourceSessionController::default();
        controller.begin_connecting();
        controller.handle(LiveTranslateServerEvent::Error {
            code: "invalid_value".into(),
            message: "Bad language".into(),
        });

        assert_eq!(
            controller.state.status,
            SessionStatus::Error("Bad language".into())
        );
    }

    #[test]
    fn translation_activity_follows_the_real_plus_request_lifecycle() {
        let mut controller = SourceSessionController::default();
        controller.did_connect();

        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        assert!(controller.state.is_translation_pending);

        controller.handle(LiveTranslateServerEvent::TranslationFinal(
            "翻译完成。".into(),
        ));
        assert!(!controller.state.is_translation_pending);

        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        controller.did_fail("Request failed");
        assert!(!controller.state.is_translation_pending);
    }

    #[test]
    fn pausing_clears_translation_activity_without_discarding_subtitles() {
        let mut controller = SourceSessionController::default();
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::SourceFinal {
            text: "Please wait.".into(),
            language: Some("en".into()),
        });
        controller.handle(LiveTranslateServerEvent::TranslationFinal(
            "请稍等。".into(),
        ));
        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        let subtitles_before_pause = controller.state.subtitles.clone();

        controller.did_pause();

        assert_eq!(controller.state.status, SessionStatus::Listening);
        assert!(!controller.state.is_translation_pending);
        assert_eq!(controller.state.subtitles, subtitles_before_pause);
    }

    #[test]
    fn clearing_translation_pending_keeps_status_and_subtitles() {
        let mut controller = SourceSessionController::default();
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::SourceFinal {
            text: "Hello.".into(),
            language: Some("en".into()),
        });
        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        assert!(controller.state.is_translation_pending);
        let subtitles_before = controller.state.subtitles.clone();

        // The timeout guard clears only the waiting-for-final flag.
        controller.clear_translation_pending();

        assert_eq!(controller.state.status, SessionStatus::Listening);
        assert!(!controller.state.is_translation_pending);
        assert!(controller.state.is_translation_timed_out);
        assert_eq!(controller.state.subtitles, subtitles_before);

        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        assert!(!controller.state.is_translation_timed_out);
    }

    #[test]
    fn clearing_subtitles_does_not_change_session_status() {
        let mut controller = SourceSessionController::default();
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::SourceFinal {
            text: "Hello.".into(),
            language: Some("en".into()),
        });
        controller.handle(LiveTranslateServerEvent::TranslationFinal("你好。".into()));

        controller.clear_subtitles();

        assert_eq!(controller.state.status, SessionStatus::Listening);
        assert_eq!(controller.state.subtitles, SubtitleSnapshot::empty());
    }

    #[test]
    fn clear_resets_pending_work_without_stopping_and_new_preview_remains_owned() {
        let mut controller = SourceSessionController::default();
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::TranslationStarted);
        controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 1 });
        controller.state.is_translation_timed_out = true;
        controller.state.translation_recovery =
            Some(crate::core::diagnostics::TranslationRecovery {
                reason: crate::core::diagnostics::TranslationRecoveryReason::RateLimited,
                retry_after_ms: 4_000,
                retry_scheduled: true,
            });
        controller.clear_subtitles();
        assert_eq!(controller.state.status, SessionStatus::Listening);
        assert!(!controller.state.is_translation_pending);
        assert!(!controller.state.is_translation_preview_pending);
        assert!(!controller.state.is_translation_timed_out);
        assert!(controller.state.translation_recovery.is_none());
        controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 2 });
        controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 1 });
        assert!(controller.state.is_translation_preview_pending);
        controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 2 });
        assert!(!controller.state.is_translation_preview_pending);
    }

    #[test]
    fn stopping_ignores_flushed_tail_subtitles() {
        let mut controller = SourceSessionController::default();
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::SourceFinal {
            text: "Last real line.".into(),
            language: Some("en".into()),
        });
        controller.handle(LiveTranslateServerEvent::TranslationFinal(
            "最后一句正常字幕。".into(),
        ));
        let subtitles_before_stopping = controller.state.subtitles.clone();

        controller.begin_stopping();
        controller.handle(LiveTranslateServerEvent::SourceFinal {
            text: "Translation mode ended.".into(),
            language: Some("en".into()),
        });
        controller.handle(LiveTranslateServerEvent::TranslationFinal(
            "翻译模式已结束。".into(),
        ));
        controller.handle(LiveTranslateServerEvent::SessionFinished);

        assert_eq!(controller.state.status, SessionStatus::Stopping);
        assert_eq!(controller.state.subtitles, subtitles_before_stopping);
    }

    #[test]
    fn stopping_accepts_a_provider_confirmed_atomic_tail_pair() {
        let mut controller = SourceSessionController::default();
        controller.did_connect();
        controller.begin_stopping();

        controller.handle(LiveTranslateServerEvent::SubtitleFinalPair {
            source: "Confirmed tail".into(),
            language: Some("en".into()),
            translation: "已确认的尾句".into(),
        });

        assert_eq!(controller.state.status, SessionStatus::Stopping);
        assert_eq!(controller.state.subtitles.history.len(), 1);
        assert_eq!(
            controller.state.subtitles.history[0].source,
            "Confirmed tail"
        );
        assert_eq!(
            controller.state.subtitles.history[0].translation,
            "已确认的尾句"
        );
    }

    #[test]
    fn unknown_server_events_leave_state_unchanged() {
        let mut controller = SourceSessionController::default();
        let before = controller.state.clone();
        controller.handle(LiveTranslateServerEvent::Ignored {
            kind: "response.created".into(),
        });
        assert_eq!(controller.state, before);
    }

    #[test]
    fn atomic_pair_updates_history_and_detected_language_together() {
        let mut controller = SourceSessionController::default();
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::SubtitleFinalPair {
            source: "Hello.".into(),
            language: Some("en".into()),
            translation: "你好。".into(),
        });

        assert_eq!(controller.state.subtitles.history.len(), 1);
        assert_eq!(controller.state.subtitles.history[0].source, "Hello.");
        assert_eq!(controller.state.subtitles.history[0].translation, "你好。");
        assert_eq!(
            controller
                .state
                .detected_language
                .as_ref()
                .map(|value| value.code.as_str()),
            Some("en")
        );
    }

    #[test]
    fn stamped_utterance_text_reaches_the_snapshot_with_its_identity() {
        let mut controller = SourceSessionController::default();
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::UtteranceText {
            utterance_id: "item_source".into(),
            role: UtteranceRole::Source,
            text: "Hello.".into(),
            is_final: true,
            language: Some("en".into()),
        });
        controller.handle(LiveTranslateServerEvent::UtteranceText {
            utterance_id: "item_source".into(),
            role: UtteranceRole::Translation,
            text: "你好。".into(),
            is_final: false,
            language: None,
        });

        assert_eq!(
            controller.state.subtitles.source.utterance_id.as_deref(),
            Some("item_source")
        );
        assert_eq!(
            controller
                .state
                .subtitles
                .translation
                .utterance_id
                .as_deref(),
            Some("item_source")
        );
        assert_eq!(
            controller
                .state
                .detected_language
                .as_ref()
                .map(|value| value.code.as_str()),
            Some("en")
        );
    }
}

#[cfg(test)]
mod dual_source_tests {
    use super::*;
    use crate::core::models::SubtitlePair;

    fn final_pair(id: u64, source: &str, translation: &str) -> LiveTranslateServerEvent {
        LiveTranslateServerEvent::SubtitleConfirmedPair {
            utterance_id: id,
            source_utterance_id: Some(id),
            source: source.into(),
            translation: translation.into(),
            language: Some("en".into()),
        }
    }

    #[test]
    fn stopped_session_restart_keeps_confirmations_and_starts_a_separate_archive() {
        let mut controller = TranslationSessionController::default();
        controller.set_audio_input(AudioInput::Both);
        controller.archive_mut().begin(true, 0);
        controller.begin_session();
        controller.did_connect();
        for source in [AudioSource::System, AudioSource::Microphone] {
            controller.handle_from(source, final_pair(1, "Old confirmation", "Old translation"));
            controller.handle_from(
                source,
                LiveTranslateServerEvent::SourceDraft {
                    text: "Obsolete draft".into(),
                    language: Some("ja".into()),
                },
            );
            controller.handle_from(
                source,
                LiveTranslateServerEvent::SubtitlePreviewPair {
                    source_utterance_id: Some(2),
                    source: "Obsolete draft".into(),
                    language: Some("ja".into()),
                    translation: "Obsolete preview".into(),
                },
            );
            controller.handle_from(source, LiveTranslateServerEvent::TranslationStarted);
            controller.handle_from(
                source,
                LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 2 },
            );
        }
        controller.clear_translation_pending_from(AudioSource::Microphone);
        let confirmed = controller.state.subtitles.history.clone();
        assert_eq!(controller.archive().count(), 2);
        controller.begin_stopping();
        controller.did_stop();

        controller.set_audio_input(AudioInput::Both);
        controller.archive_mut().begin(true, 1);
        controller.begin_session();
        assert_eq!(controller.state.status, SessionStatus::Connecting);
        assert_eq!(controller.state.subtitles.history, confirmed);
        assert_eq!(controller.archive().count(), 0);
        assert!(controller.state.subtitles.tracks.iter().all(|track| {
            !track.is_translation_pending
                && !track.is_translation_preview_pending
                && !track.is_translation_timed_out
                && track.preview_pair.is_none()
                && track.source.text != "Obsolete draft"
                && track.translation.text != "Obsolete preview"
                && track.detected_language.is_none()
        }));

        controller.did_connect();
        for source in [AudioSource::System, AudioSource::Microphone] {
            controller.handle_from(source, final_pair(1, "New confirmation", "New translation"));
        }
        assert_eq!(controller.state.subtitles.history.len(), 4);
        assert_eq!(&controller.state.subtitles.history[..2], &confirmed);
        assert_eq!(controller.archive().count(), 2);
        let exported = controller.archive().export().unwrap();
        assert!(!exported.contains("Old confirmation"));
        assert_eq!(exported.matches("New confirmation").count(), 2);
        assert_eq!(
            controller.state.subtitles.history[2].audio_source,
            AudioSource::System
        );
        assert_eq!(
            controller.state.subtitles.history[3].audio_source,
            AudioSource::Microphone
        );

        controller.clear_subtitles();
        assert!(controller.state.subtitles.history.is_empty());
        assert!(controller
            .state
            .subtitles
            .tracks
            .iter()
            .all(|track| track.history.is_empty()));
        assert_eq!(controller.archive().count(), 0);
    }

    #[test]
    fn stopped_session_source_change_keeps_history_and_filters_inactive_events() {
        let mut controller = TranslationSessionController::default();
        controller.begin_session();
        controller.did_connect();
        controller.handle_from(
            AudioSource::System,
            final_pair(1, "Previous system confirmation", "Previous translation"),
        );
        controller.begin_stopping();
        controller.did_stop();
        let confirmed = controller.state.subtitles.history.clone();

        controller.set_audio_input(AudioInput::Microphone);
        assert_eq!(controller.state.status, SessionStatus::Idle);
        assert_eq!(controller.state.subtitles.history, confirmed);
        assert_eq!(controller.state.subtitles.tracks.len(), 1);
        assert_eq!(
            controller.state.subtitles.tracks[0].audio_source,
            AudioSource::Microphone
        );
        controller.begin_session();
        controller.did_connect();
        controller.handle_from(
            AudioSource::System,
            final_pair(2, "Unselected system", "Ignored translation"),
        );
        assert_eq!(controller.state.subtitles.history, confirmed);
        controller.handle_from(
            AudioSource::Microphone,
            final_pair(1, "New microphone confirmation", "New translation"),
        );
        assert_eq!(controller.state.subtitles.history.len(), 2);
        assert_eq!(
            controller.state.subtitles.history[0].audio_source,
            AudioSource::System
        );
        assert_eq!(
            controller.state.subtitles.history[1].audio_source,
            AudioSource::Microphone
        );
    }

    #[test]
    fn repeated_session_restarts_keep_display_history_bounded_without_retention() {
        let mut controller = TranslationSessionController::default();
        controller.set_audio_input(AudioInput::Both);
        for session in 0..25 {
            controller.archive_mut().begin(false, session);
            controller.begin_session();
            controller.did_connect();
            for source in [AudioSource::System, AudioSource::Microphone] {
                controller.handle_from(
                    source,
                    final_pair(1, "Synthetic final", "Synthetic translation"),
                );
            }
            controller.begin_stopping();
            controller.did_stop();
            assert!(controller.state.subtitles.history.len() <= 20);
            assert!(controller
                .state
                .subtitles
                .tracks
                .iter()
                .all(|track| track.history.len() <= 20));
            assert_eq!(controller.archive().count(), 0);
        }
        assert_eq!(controller.state.subtitles.history.len(), 20);
        assert!(controller
            .state
            .subtitles
            .tracks
            .iter()
            .all(|track| track.history.len() == 20));
    }

    #[test]
    fn manual_restart_accepts_an_identical_idless_final_once_in_the_new_archive() {
        let mut controller = TranslationSessionController::default();
        let final_event = LiveTranslateServerEvent::SubtitleFinalPair {
            source: "Repeated confirmation".into(),
            language: Some("en".into()),
            translation: "Repeated translation".into(),
        };
        controller.archive_mut().begin(true, 0);
        controller.begin_session();
        controller.did_connect();
        controller.handle_from(AudioSource::System, final_event.clone());
        controller.begin_stopping();
        controller.did_stop();
        let previous_time = controller.state.subtitles.history[0].created_at_ms;

        controller.archive_mut().begin(true, 1);
        controller.begin_session();
        // Connecting can be retried before the new session produces any text.
        controller.begin_connecting();
        controller.begin_connecting();
        controller.did_connect();
        controller.handle_from(AudioSource::System, final_event.clone());
        controller.handle_from(AudioSource::System, final_event.clone());
        assert_eq!(controller.state.subtitles.history.len(), 2);
        assert!(controller.state.subtitles.history[1].created_at_ms > previous_time);
        assert_eq!(controller.archive().count(), 1);
        assert_eq!(
            controller
                .archive()
                .export()
                .unwrap()
                .matches("Repeated confirmation")
                .count(),
            1
        );

        // A reconnect within that session still suppresses duplicate finals.
        controller.begin_connecting();
        controller.did_connect();
        controller.handle_from(AudioSource::System, final_event);
        assert_eq!(controller.state.subtitles.history.len(), 2);
        assert_eq!(controller.archive().count(), 1);
    }

    #[test]
    fn enabling_a_dormant_source_uses_the_current_manual_session_boundary() {
        let mut controller = TranslationSessionController::default();
        let final_event = LiveTranslateServerEvent::SubtitleFinalPair {
            source: "Repeated microphone confirmation".into(),
            language: Some("en".into()),
            translation: "Repeated microphone translation".into(),
        };
        controller.set_audio_input(AudioInput::Both);
        controller.begin_session();
        controller.did_connect();
        controller.handle_from(AudioSource::Microphone, final_event.clone());
        controller.begin_stopping();
        controller.did_stop();

        controller.set_audio_input(AudioInput::System);
        controller.archive_mut().begin(true, 1);
        controller.begin_session();
        controller.did_connect();
        assert_eq!(controller.state.subtitles.tracks.len(), 1);
        assert_eq!(controller.state.subtitles.history.len(), 1);

        controller.reconfigure_audio_input(AudioInput::Both);
        controller.begin_connecting();
        controller.did_connect();
        controller.handle_from(AudioSource::Microphone, final_event.clone());
        controller.handle_from(AudioSource::Microphone, final_event);
        assert_eq!(controller.state.subtitles.history.len(), 2);
        assert!(controller
            .state
            .subtitles
            .history
            .iter()
            .all(|pair| pair.audio_source == AudioSource::Microphone));
        assert_eq!(controller.archive().count(), 1);
    }

    #[test]
    fn audio_input_switch_preserves_confirmed_history_and_disabled_source_labels() {
        let mut controller = TranslationSessionController::default();
        controller.set_audio_input(AudioInput::Both);
        controller.did_connect();
        controller.archive_mut().begin(true, 0);
        controller.handle_from(
            AudioSource::System,
            final_pair(1, "Synthetic system", "System translation"),
        );
        controller.handle_from(
            AudioSource::Microphone,
            final_pair(1, "Synthetic microphone", "Microphone translation"),
        );
        let confirmed = controller.state.subtitles.history.clone();
        let transcript = controller.archive().export();
        controller.handle_from(
            AudioSource::Microphone,
            LiveTranslateServerEvent::TranslationStarted,
        );
        controller.handle_from(
            AudioSource::Microphone,
            LiveTranslateServerEvent::SourceDraft {
                text: "Discard this synthetic draft".into(),
                language: Some("ja".into()),
            },
        );
        controller.reconfigure_audio_input(AudioInput::System);
        assert_eq!(controller.state.status, SessionStatus::Listening);
        assert_eq!(controller.state.subtitles.history, confirmed);
        assert_eq!(controller.archive().export(), transcript);
        assert_eq!(controller.state.subtitles.tracks.len(), 1);
        assert_eq!(
            controller.state.subtitles.tracks[0].audio_source,
            AudioSource::System
        );
        controller.handle_from(
            AudioSource::Microphone,
            final_pair(2, "Late microphone", "Late translation"),
        );
        assert_eq!(controller.state.subtitles.history, confirmed);
        controller.reconfigure_audio_input(AudioInput::Both);
        assert_eq!(controller.state.subtitles.history, confirmed);
        assert!(controller
            .state
            .subtitles
            .tracks
            .iter()
            .all(|track| !track.is_translation_pending && track.preview_pair.is_none()));
        assert!(!controller.state.subtitles.tracks[1]
            .source
            .text
            .contains("Discard"));
        // Fresh client IDs restart at one; old confirmed history remains intact.
        controller.handle_from(
            AudioSource::Microphone,
            final_pair(1, "New microphone", "New translation"),
        );
        assert_eq!(controller.state.subtitles.history.len(), 3);
        assert_eq!(
            controller.state.subtitles.history[2].audio_source,
            AudioSource::Microphone
        );
    }

    #[test]
    fn application_capture_restart_preserves_history_and_reaccepts_fresh_provider_ids() {
        let mut controller = TranslationSessionController::default();
        controller.did_connect();
        controller.archive_mut().begin(true, 0);
        let pair = |text: &str| LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: Some(1),
            utterance_id: 1,
            source: text.into(),
            translation: "Synthetic translation".into(),
            language: Some("en".into()),
        };
        controller.handle_from(AudioSource::System, pair("Synthetic first application"));
        let history = controller.state.subtitles.history.clone();
        let transcript = controller.archive().export();
        controller.handle_from(
            AudioSource::System,
            LiveTranslateServerEvent::SourceDraft {
                text: "Synthetic obsolete draft".into(),
                language: Some("en".into()),
            },
        );
        // App A -> B retains AudioInput::System, but starts a fresh provider.
        controller.reconfigure_audio_input(AudioInput::System);
        assert_eq!(controller.state.status, SessionStatus::Listening);
        assert_eq!(controller.state.subtitles.history, history);
        assert_eq!(controller.archive().export(), transcript);
        assert!(!controller.state.subtitles.source.text.contains("obsolete"));
        controller.handle_from(AudioSource::System, pair("Synthetic second application"));
        assert_eq!(controller.state.subtitles.history.len(), 2);
        assert_eq!(
            controller.state.subtitles.history[1].source,
            "Synthetic second application"
        );
    }

    #[test]
    fn audio_input_switch_preserves_paused_or_error_controller_status() {
        for status in [
            SessionStatus::Listening,
            SessionStatus::Error("synthetic permission error".into()),
            SessionStatus::Idle,
        ] {
            let mut controller = TranslationSessionController::default();
            controller.sources[0].state.status = status.clone();
            controller.refresh();
            controller.reconfigure_audio_input(AudioInput::Both);
            assert_eq!(controller.state.status, status);
            assert!(controller
                .state
                .subtitles
                .tracks
                .iter()
                .all(|track| !track.is_translation_pending));
        }
    }

    #[test]
    fn simultaneous_drafts_and_same_provider_ids_stay_with_their_inputs() {
        let mut controller = TranslationSessionController::default();
        controller.set_audio_input(AudioInput::Both);
        for (input, text, language) in [
            (AudioSource::System, "Synthetic system draft", "en"),
            (AudioSource::Microphone, "Synthetic microphone draft", "ja"),
        ] {
            controller.handle_from(
                input,
                LiveTranslateServerEvent::SourceUtteranceDraft {
                    utterance_id: 1,
                    text: text.into(),
                    language: Some(language.into()),
                },
            );
        }
        let tracks = &controller.state.subtitles.tracks;
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].source.text, "Synthetic system draft");
        assert_eq!(tracks[1].source.text, "Synthetic microphone draft");
        assert_eq!(tracks[0].detected_language.as_ref().unwrap(), "en");
        assert_eq!(tracks[1].detected_language.as_ref().unwrap(), "ja");
        assert_ne!(tracks[0].source.utterance_id, tracks[1].source.utterance_id);
        let wire = serde_json::to_value(&controller.state.subtitles).unwrap();
        assert_eq!(wire["tracks"][0]["detectedLanguage"], "en");
        assert_eq!(wire["tracks"][1]["detectedLanguage"], "ja");
        assert_eq!(wire["tracks"][0]["audioSource"], "system");
        assert_eq!(wire["tracks"][1]["audioSource"], "microphone");

        controller.handle_from(
            AudioSource::System,
            final_pair(1, "System final", "System translation"),
        );
        // One input's provider ID does not consume the other's ID or draft.
        assert!(controller.accepts_confirmed_pair_from(
            AudioSource::Microphone,
            1,
            "Mic final",
            "Mic translation"
        ));
        assert_eq!(
            controller.state.subtitles.tracks[1].source.text,
            "Synthetic microphone draft"
        );
        controller.handle_from(
            AudioSource::Microphone,
            final_pair(1, "Mic final", "Mic translation"),
        );
        assert_eq!(controller.state.subtitles.history.len(), 2);
        assert_eq!(
            controller.state.subtitles.history[0].audio_source,
            AudioSource::System
        );
        assert_eq!(
            controller.state.subtitles.history[1].audio_source,
            AudioSource::Microphone
        );
        assert!(
            controller.state.subtitles.history[0].created_at_ms
                < controller.state.subtitles.history[1].created_at_ms
        );
        controller.handle_from(AudioSource::System, final_pair(1, "Replay", "Replay"));
        controller.handle_from(AudioSource::Microphone, final_pair(1, "Replay", "Replay"));
        assert_eq!(controller.state.subtitles.history.len(), 2);
    }

    #[test]
    fn finals_in_one_lane_do_not_clear_pending_or_preview_in_the_other() {
        let mut controller = TranslationSessionController::default();
        controller.set_audio_input(AudioInput::Both);
        controller.begin_connecting();
        controller.handle_from(
            AudioSource::System,
            LiveTranslateServerEvent::SessionUpdated,
        );
        assert_eq!(controller.state.status, SessionStatus::Connecting);
        controller.handle_from(
            AudioSource::Microphone,
            LiveTranslateServerEvent::SessionUpdated,
        );
        assert_eq!(controller.state.status, SessionStatus::Listening);
        for source in [AudioSource::System, AudioSource::Microphone] {
            controller.handle_from(source, LiveTranslateServerEvent::TranslationStarted);
            controller.handle_from(
                source,
                LiveTranslateServerEvent::SubtitlePreviewPair {
                    source_utterance_id: Some(2),
                    source: "Synthetic preview".into(),
                    language: None,
                    translation: "Synthetic translation".into(),
                },
            );
        }
        controller.handle_from(
            AudioSource::Microphone,
            final_pair(1, "Mic final", "Mic translation"),
        );
        assert!(controller.state.is_translation_pending);
        assert!(controller.state.subtitles.tracks[0].is_translation_pending);
        assert!(!controller.state.subtitles.tracks[1].is_translation_pending);
        assert!(controller.state.subtitles.tracks[0].preview_pair.is_some());
        controller.clear_translation_pending_from(AudioSource::System);
        assert!(controller.state.subtitles.tracks[0].is_translation_timed_out);
        assert!(!controller.state.subtitles.tracks[1].is_translation_timed_out);
        assert!(!controller.state.is_translation_pending);
    }

    #[test]
    fn both_histories_and_the_aggregate_are_bounded_and_export_preserves_sources() {
        let mut controller = TranslationSessionController::default();
        controller.set_audio_input(AudioInput::Both);
        controller.archive_mut().begin(true, 0);
        for id in 1..=40 {
            for input in [AudioSource::System, AudioSource::Microphone] {
                controller.handle_from(
                    input,
                    final_pair(id, "Synthetic final", "Synthetic translation"),
                );
            }
        }
        assert_eq!(controller.state.subtitles.history.len(), 20);
        assert!(controller
            .state
            .subtitles
            .tracks
            .iter()
            .all(|track| track.history.len() == 20));
        assert_eq!(controller.archive().count(), 80);
        let exported = controller.archive().export().unwrap();
        assert_eq!(exported.matches("System audio").count(), 40);
        assert_eq!(exported.matches("Microphone").count(), 40);
        controller.clear_subtitles();
        assert!(controller.state.subtitles.history.is_empty());
        assert!(controller
            .state
            .subtitles
            .tracks
            .iter()
            .all(|track| track.history.is_empty() && track.preview_pair.is_none()));
        assert_eq!(controller.archive().count(), 0);
        controller.handle_from(
            AudioSource::Microphone,
            final_pair(40, "Replay after clear", "Replay"),
        );
        assert!(controller.state.subtitles.history.is_empty());
        controller.begin_connecting();
        controller.handle_from(
            AudioSource::Microphone,
            final_pair(1, "New generation", "New translation"),
        );
        assert_eq!(controller.state.subtitles.history.len(), 1);
    }

    #[test]
    fn unselected_inputs_cannot_produce_content_and_legacy_pairs_default_to_system() {
        let mut controller = TranslationSessionController::default();
        controller.handle_from(
            AudioSource::Microphone,
            final_pair(1, "Unexpected microphone", "Unexpected microphone"),
        );
        assert!(controller.state.subtitles.history.is_empty());
        controller.set_audio_input(AudioInput::Microphone);
        controller.handle_from(
            AudioSource::System,
            final_pair(1, "Unexpected system", "Unexpected system"),
        );
        assert!(controller.state.subtitles.history.is_empty());
        controller.handle(final_pair(
            1,
            "Synthetic microphone",
            "Synthetic translation",
        ));
        assert_eq!(
            controller.state.subtitles.history[0].audio_source,
            AudioSource::Microphone
        );
        assert_eq!(controller.state.subtitles.tracks.len(), 1);
        let legacy: SubtitlePair = serde_json::from_str(
            r#"{"source":"Legacy synthetic","translation":"Legacy translation","createdAt":100}"#,
        )
        .unwrap();
        assert_eq!(legacy.audio_source, AudioSource::System);
        let mut microphone = legacy.clone();
        microphone.audio_source = AudioSource::Microphone;
        assert_ne!(legacy, microphone);
    }
}
