//! Provider-neutral session state controller.

use crate::core::audio::audio_input::{AudioInput, AudioSource};
use crate::core::models::{
    DetectedLanguage, SessionStatus, SourceSubtitleSnapshot, SubtitleSnapshot, UtteranceRole,
};
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::subtitles::subtitle_reducer::SubtitleReducer;
use std::time::{Duration, Instant};

/// Clear a provider's stuck waiting-for-final indicator without changing text.
pub const TRANSLATION_PENDING_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Default, Clone)]
pub struct ProviderEventApplication {
    /// The accepted durable pair from this event, independent of display retention.
    pub confirmed_pair: Option<crate::core::models::SubtitlePair>,
    pub resolves_translation_timeout: bool,
    pub translation_pending_deadline: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslationSessionState {
    pub status: SessionStatus,
    pub atomic_preview: Option<bool>,
    pub subtitles: SubtitleSnapshot,
    pub detected_language: Option<DetectedLanguage>,
    pub is_translation_pending: bool,
    pub is_translation_preview_pending: bool,
    pub is_translation_timed_out: bool,
    pub translation_recovery: Option<crate::core::diagnostics::latency::TranslationRecovery>,
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

    fn start_translation(&mut self, now: Instant) {
        self.clear_preview_pending();
        self.state.translation_recovery = None;
        self.state.is_translation_pending = true;
        self.translation_pending_deadline = Some(now + TRANSLATION_PENDING_TIMEOUT);
        self.state.is_translation_timed_out = false;
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

    #[cfg(test)]
    fn handle_at(&mut self, event: LiveTranslateServerEvent, now: Instant) {
        let _ = self.handle_ordered_at(event, now, None, false);
    }

    fn handle_ordered_at(
        &mut self,
        event: LiveTranslateServerEvent,
        now: Instant,
        source_ordinal: Option<u64>,
        starts_translation_hint: bool,
    ) -> Option<ProviderEventApplication> {
        if !event.text_within_limit() {
            return None;
        }
        // Alibaba teardown can emit synthetic source/translation cleanup
        // finals, which are intentionally ignored. OpenAI has no separate
        // final events: a real `session.closed` may flush one client-aligned
        // atomic pair, and that verified tail is safe to keep.
        if self.state.status == SessionStatus::Stopping
            && !matches!(
                event,
                LiveTranslateServerEvent::SubtitleSourceBoundary { .. }
                    | LiveTranslateServerEvent::SubtitleFinalPair { .. }
                    | LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
                    | LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
                    | LiveTranslateServerEvent::UtteranceRegistered { .. }
            )
        {
            return None;
        }

        if let Some(order) = source_ordinal
            .and_then(|ordinal| ordinal.checked_add(1))
            .filter(|order| *order <= i64::MAX as u64)
        {
            if let LiveTranslateServerEvent::UtteranceText { utterance_id, .. }
            | LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { utterance_id, .. } = &event
            {
                if !self
                    .subtitle_reducer
                    .register_source_order(utterance_id, order)
                {
                    return None;
                }
            }
        }

        let resolves_translation_timeout = match &event {
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { utterance_id, .. } => self
                .subtitle_reducer
                .identified_source_is_current(utterance_id),
            _ => matches!(
                event,
                LiveTranslateServerEvent::TranslationFinal(_)
                    | LiveTranslateServerEvent::SubtitleFinalPair { .. }
                    | LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
                    | LiveTranslateServerEvent::UtteranceText {
                        role: UtteranceRole::Translation,
                        is_final: true,
                        ..
                    }
                    | LiveTranslateServerEvent::Error { .. }
                    | LiveTranslateServerEvent::TranslationDeferred(_)
            ),
        };
        let mut starts_translation = matches!(event, LiveTranslateServerEvent::TranslationStarted);

        match event {
            LiveTranslateServerEvent::SubtitleSourceBoundary {
                utterance_id,
                source_order,
            } => {
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::SourceBoundary {
                        utterance_id,
                        source_order,
                    });
            }
            LiveTranslateServerEvent::SessionCreated => {}
            LiveTranslateServerEvent::SessionUpdated => self.did_connect(),
            LiveTranslateServerEvent::UtteranceRegistered { utterance_id } => {
                self.subtitle_reducer
                    .register_source_identity(&utterance_id);
            }
            LiveTranslateServerEvent::SubtitleIdentifiedPreviewPair {
                utterance_id,
                source,
                translation,
                language,
            } => {
                // Publish one snapshot after both lines have been replaced.
                for (role, text) in [
                    (UtteranceRole::Source, source),
                    (UtteranceRole::Translation, translation),
                ] {
                    self.subtitle_reducer.apply(
                        crate::core::models::SubtitleEvent::UtteranceText {
                            utterance_id: utterance_id.clone(),
                            role,
                            text,
                            is_final: false,
                        },
                    );
                }
                if self
                    .subtitle_reducer
                    .snapshot
                    .source
                    .utterance_id
                    .as_deref()
                    == Some(&utterance_id)
                    && !self.subtitle_reducer.snapshot.source.is_final
                {
                    self.update_detected_language(language.as_deref());
                }
            }
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
                self.start_translation(now);
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
            LiveTranslateServerEvent::SubtitlePreviewWithdrawn {
                source_utterance_id,
            } => {
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::WithdrawPreview {
                        source_utterance_id,
                    });
            }
            LiveTranslateServerEvent::UtteranceText {
                utterance_id,
                role,
                text,
                is_final,
                language,
            } => {
                // The order/lifecycle checks above and caption reduction below
                // share the same controller admission as this activity hint.
                let accepted_start = starts_translation_hint
                    && role == UtteranceRole::Source
                    && is_final
                    && self
                        .subtitle_reducer
                        .can_start_identified_translation(&utterance_id, &text);
                self.subtitle_reducer
                    .apply(crate::core::models::SubtitleEvent::UtteranceText {
                        utterance_id: utterance_id.clone(),
                        role,
                        text,
                        is_final,
                    });
                if accepted_start {
                    self.start_translation(now);
                    starts_translation = true;
                }
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
                    return None;
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
                    return None;
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
            LiveTranslateServerEvent::Ignored { .. } => return None,
        }

        self.state.subtitles = self.subtitle_reducer.snapshot.clone();
        if !self.state.is_translation_pending {
            self.translation_pending_deadline = None;
        }
        Some(ProviderEventApplication {
            confirmed_pair: None,
            resolves_translation_timeout,
            translation_pending_deadline: starts_translation
                .then_some(self.translation_pending_deadline)
                .flatten(),
        })
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
        self.handle_ordered_from_at(audio_source, event, now, None, false);
    }

    pub(crate) fn handle_ordered_from_at(
        &mut self,
        audio_source: AudioSource,
        event: LiveTranslateServerEvent,
        now: Instant,
        source_ordinal: Option<u64>,
        starts_translation_hint: bool,
    ) -> ProviderEventApplication {
        if !self.audio_input.sources().contains(&audio_source) || !event.text_within_limit() {
            return ProviderEventApplication::default();
        }
        let lane = &mut self.sources[source_index(audio_source)];
        let before = lane
            .state
            .subtitles
            .history
            .last()
            .map(|pair| pair.created_at_ms);
        let mut application = lane
            .handle_ordered_at(event, now, source_ordinal, starts_translation_hint)
            .unwrap_or_default();
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
                application.confirmed_pair = Some(pair.clone());
                self.archive.append(pair);
                self.state.subtitles.history.push(pair.clone());
                if self.state.subtitles.history.len() > 20 {
                    self.state.subtitles.history.remove(0);
                }
            }
        }
        self.refresh();
        application
    }

    fn refresh(&mut self) {
        let history = std::mem::take(&mut self.state.subtitles.history);
        let active: Vec<_> = self
            .audio_input
            .sources()
            .iter()
            .map(|source| (*source, &self.sources[source_index(*source)].state))
            .collect();
        self.state = aggregate_source_states(&active, history, self.atomic_preview);
    }
}

/// Shared presentation aggregation for desktop lanes and independent native handles.
/// The caller owns bounded history and source identity; provider text is never mixed.
fn aggregate_source_states(
    active: &[(AudioSource, &TranslationSessionState)],
    history: Vec<crate::core::models::SubtitlePair>,
    atomic_preview: Option<bool>,
) -> TranslationSessionState {
    if active.is_empty() {
        return TranslationSessionState::default();
    }
    let mut state = TranslationSessionState::default();
    let primary = active[0].1;
    state.atomic_preview = atomic_preview;
    state.detected_language = primary.detected_language.clone();
    // The aggregate history belongs to the session, not its first lane.
    // Clone only the primary presentation; cloning the whole lane state
    // would allocate its complete history just to discard it here.
    state.subtitles = SubtitleSnapshot {
        tracks: Vec::new(),
        source: primary.subtitles.source.clone(),
        translation: primary.subtitles.translation.clone(),
        history,
        preview_pair: primary.subtitles.preview_pair.clone(),
        realtime_preview: primary.subtitles.realtime_preview.clone(),
        display_pair: primary.subtitles.display_pair.clone(),
        display_pair_final: primary.subtitles.display_pair_final,
        display_pair_created_at_ms: primary.subtitles.display_pair_created_at_ms,
    };
    state.subtitles.tracks = active
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
            display_pair_created_at_ms: state.subtitles.display_pair_created_at_ms,
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
    state.status = active
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
    state.is_translation_pending = active.iter().any(|(_, state)| state.is_translation_pending);
    state.is_translation_preview_pending = active
        .iter()
        .any(|(_, state)| state.is_translation_preview_pending);
    state.is_translation_timed_out = active
        .iter()
        .any(|(_, state)| state.is_translation_timed_out);
    state.translation_recovery = active
        .iter()
        .find_map(|(_, state)| state.translation_recovery);
    state
}

/// Combine already-reduced independent lanes without reducing their events again.
/// On Stop only accepted complete pairs remain; all transient activity is retired.
pub fn merge_independent_states(
    mut lanes: Vec<(AudioSource, TranslationSessionState)>,
    history_limit: usize,
    stopped: bool,
) -> TranslationSessionState {
    lanes.sort_by_key(|(source, _)| source_index(*source));
    let limit = history_limit.min(6);
    for (source, state) in &mut lanes {
        state.subtitles.tracks.clear();
        for pair in &mut state.subtitles.history {
            pair.audio_source = *source;
        }
        if stopped {
            let subtitles = &mut state.subtitles;
            if !subtitles.display_pair_final {
                subtitles.display_pair =
                    subtitles
                        .history
                        .last()
                        .map(|pair| mimi_core::PreviewSubtitlePair {
                            utterance_id: None,
                            source: pair.source.clone(),
                            translation: pair.translation.clone(),
                        });
                subtitles.display_pair_created_at_ms =
                    subtitles.history.last().map(|pair| pair.created_at_ms);
                subtitles.display_pair_final = subtitles.display_pair.is_some();
            }
            let pair = subtitles.display_pair.as_ref();
            subtitles.source = mimi_core::SubtitleLine::new(
                pair.map_or("", |pair| pair.source.as_str()),
                pair.is_some(),
            );
            subtitles.translation = mimi_core::SubtitleLine::new(
                pair.map_or("", |pair| pair.translation.as_str()),
                pair.is_some(),
            );
            if let Some(pair) = pair {
                subtitles.source.utterance_id = pair.utterance_id.clone();
                subtitles.translation.utterance_id = pair.utterance_id.clone();
            }
            subtitles.preview_pair = None;
            subtitles.realtime_preview = None;
            state.status = SessionStatus::Idle;
            state.is_translation_pending = false;
            state.is_translation_preview_pending = false;
            state.is_translation_timed_out = false;
            state.translation_recovery = None;
        }
        // Resolve the last accepted pair before applying a zero display limit.
        if state.subtitles.history.len() > limit {
            state
                .subtitles
                .history
                .drain(..state.subtitles.history.len() - limit);
        }
    }
    let mut history: Vec<_> = lanes
        .iter()
        .flat_map(|(_, state)| state.subtitles.history.iter().cloned())
        .collect();
    history.sort_by_key(|pair| pair.created_at_ms);
    if history.len() > limit {
        history.drain(..history.len() - limit);
    }
    let atomic = lanes.first().and_then(|(_, state)| state.atomic_preview);
    let active: Vec<_> = lanes
        .iter()
        .map(|(source, state)| (*source, state))
        .collect();
    aggregate_source_states(&active, history, atomic)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod dual_source_tests;
