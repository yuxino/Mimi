//! An opt-in file journal. Live state retains only counts and limit metadata.
use crate::core::{
    lifecycle::session_archive::TRANSCRIPT_BYTE_LIMIT,
    lifecycle::session_archive::TRANSCRIPT_COUNT_LIMIT, models::SubtitlePair,
};
use crate::session_history::SessionHistory;
use serde::Serialize;
use std::io;

#[derive(Default, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalState {
    pub count: usize,
    pub limited: bool,
    pub save_error: bool,
}

pub struct SessionJournal {
    history: SessionHistory,
    id: Option<String>,
    enabled: bool,
    bytes: usize,
    last_confirmation_ms: u64,
    state: JournalState,
}

impl SessionJournal {
    pub fn begin(history: SessionHistory, started_at_ms: u64) -> io::Result<Self> {
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, started_at_ms)?;
        Ok(Self {
            history,
            id: Some(id),
            enabled: true,
            bytes: 0,
            last_confirmation_ms: started_at_ms,
            state: JournalState::default(),
        })
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn state(&self) -> JournalState {
        self.state
    }

    /// Call only with the controller's accepted confirmation, never a preview
    /// or a periodically polled display snapshot. The caller serializes lanes.
    pub fn append(&mut self, pair: &SubtitlePair) -> io::Result<()> {
        let Some(id) = self.id.as_deref().filter(|_| self.enabled) else {
            return Ok(());
        };
        let bytes = pair.source.len().saturating_add(pair.translation.len());
        if self.state.limited {
            return Ok(());
        }
        if self.state.count >= TRANSCRIPT_COUNT_LIMIT
            || bytes > TRANSCRIPT_BYTE_LIMIT.saturating_sub(self.bytes)
        {
            self.state.limited = true;
            return Ok(());
        }
        let mut pair = pair.clone();
        pair.created_at_ms = pair
            .created_at_ms
            .max(self.last_confirmation_ms.saturating_add(1));
        if let Err(error) = self.history.append_pair(id, &pair) {
            self.state.save_error = true;
            return Err(error);
        }
        self.last_confirmation_ms = pair.created_at_ms;
        self.state.count += 1;
        self.bytes += bytes;
        Ok(())
    }

    pub fn finish(&mut self, ended_at_ms: u64) -> io::Result<()> {
        let Some(id) = self.id.as_deref() else {
            return Ok(());
        };
        let result = if self.enabled {
            self.history
                .finalize(
                    id,
                    ended_at_ms.max(self.last_confirmation_ms),
                    self.state.limited,
                )
                .map(|_| ())
        } else {
            self.history.discard(id)
        };
        if result.is_err() {
            self.state.save_error = true;
        } else {
            self.id = None;
        }
        result
    }

    /// Opting out erases this session and stops all subsequent writes. Older
    /// finalized sessions remain available until explicitly deleted.
    pub fn disable(&mut self) -> io::Result<()> {
        self.enabled = false;
        self.state = JournalState::default();
        self.bytes = 0;
        self.finish(self.last_confirmation_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        audio::audio_input::AudioSource, lifecycle::session_archive::TranscriptExportOptions,
    };

    #[test]
    fn journal_saves_every_accepted_pair_and_serializes_source_identity() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let mut journal = SessionJournal::begin(history.clone(), 10).unwrap();
        let id = journal.id().unwrap().to_string();
        for index in 0..100 {
            let mut pair =
                SubtitlePair::new(format!("source {index}"), format!("target {index}"), 11);
            pair.audio_source = if index % 2 == 0 {
                AudioSource::System
            } else {
                AudioSource::Microphone
            };
            journal.append(&pair).unwrap();
        }
        assert_eq!(history.page(&id, "", 0).unwrap().total, 100);
        journal.finish(20).unwrap();
        journal.finish(30).unwrap();
        let page = history.page(&id, "", 0).unwrap();
        assert_eq!(page.entries[0].audio_source, AudioSource::Microphone);
        assert_eq!(page.entries[1].audio_source, AudioSource::System);
        assert!(page.entries[0].created_at_ms > page.entries[1].created_at_ms);
        let exported = history
            .export_transcript_with_options(&id, TranscriptExportOptions::default())
            .unwrap();
        assert!(String::from_utf8(exported).unwrap().contains("source 0"));
        assert_eq!(history.list().unwrap()[0].count, 100);
    }

    #[test]
    fn opt_out_erases_only_current_session_and_prevents_late_appends() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let pair = SubtitlePair::new("source".into(), "target".into(), 10);
        let mut saved = SessionJournal::begin(history.clone(), 0).unwrap();
        saved.append(&pair).unwrap();
        saved.finish(20).unwrap();
        let mut current = SessionJournal::begin(history.clone(), 30).unwrap();
        current.append(&pair).unwrap();
        current.disable().unwrap();
        current.append(&pair).unwrap();
        current.finish(50).unwrap();
        assert_eq!(history.list().unwrap().len(), 1);
        assert_eq!(current.state().count, 0);
    }

    #[test]
    fn journal_is_bounded_and_preserves_the_limit_on_disk() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let mut journal = SessionJournal::begin(history.clone(), 0).unwrap();
        let id = journal.id().unwrap().to_string();
        let pair = SubtitlePair::new("a".repeat(4096), "b".repeat(4096), 10);
        for _ in 0..300 {
            journal.append(&pair).unwrap();
        }
        assert_eq!(journal.state().count, TRANSCRIPT_BYTE_LIMIT / 8192);
        assert!(journal.state().limited);
        journal.finish(20).unwrap();
        assert!(history.list().unwrap()[0].limited);
        assert!(history.page(&id, "", 0).unwrap().total < 300);
    }

    #[test]
    fn failed_write_is_reported_without_retaining_failed_caption_content() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let mut journal = SessionJournal::begin(history.clone(), 0).unwrap();
        let id = journal.id().unwrap().to_owned();
        history.discard(&id).unwrap();
        let pair = SubtitlePair::new("source".into(), "target".into(), 10);
        assert!(journal.append(&pair).is_err());
        assert!(journal.state().save_error);
        assert_eq!(journal.state().count, 0);
        assert!(journal.finish(20).is_err());
        assert_eq!(journal.id(), Some(id.as_str()));
    }

    #[test]
    fn accepted_event_output_keeps_every_final_and_never_replays_duplicates() {
        use crate::clients::transport::provider_events::ProviderEvent;
        use crate::core::{
            audio::audio_input::AudioInput, lifecycle::session::TranslationSessionController,
            protocols::realtime::live_translate::LiveTranslateServerEvent,
        };
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().join("history"), false);
        let mut journal = SessionJournal::begin(history.clone(), 0).unwrap();
        let id = journal.id().unwrap().to_owned();
        let mut controller = TranslationSessionController::default();
        controller.set_audio_input(AudioInput::Microphone);
        controller.begin_session();
        controller.did_connect();
        for index in 1..=40 {
            let event = LiveTranslateServerEvent::SubtitleConfirmedPair {
                utterance_id: index,
                source_utterance_id: None,
                source: format!("source {index}"),
                translation: format!("target {index}"),
                language: Some("en".into()),
            };
            for repeated in [false, true] {
                let application = ProviderEvent::unsequenced(event.clone(), 0)
                    .apply_to(&mut controller, AudioSource::Microphone);
                assert_eq!(application.confirmed_pair.is_none(), repeated);
                if let Some(pair) = application.confirmed_pair {
                    journal.append(&pair).unwrap();
                }
            }
        }
        assert_eq!(controller.state.subtitles.history.len(), 20);
        assert_eq!(controller.archive().count(), 0);
        journal.finish(100).unwrap();
        let page = history.page(&id, "", 0).unwrap();
        assert_eq!(page.total, 40);
        assert!(page
            .entries
            .iter()
            .all(|pair| pair.audio_source == AudioSource::Microphone));
    }
}
