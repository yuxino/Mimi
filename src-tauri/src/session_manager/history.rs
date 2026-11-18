//! Opt-in local history, selected-input audio recording, bounded metadata and export finalization.

use super::fixtures::finish_ui_test_archive;
use super::recording_state::RecordingEvidence;
use super::SessionManager;
use crate::core::audio_input::AudioSource;
use crate::pipeline_log;
use crate::session_history::SessionHistory;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Content lives in the local journal. Only size/limit metadata stays here.
#[derive(Default)]
pub(super) struct LocalCaptureStats {
    pub(super) transcript_count: usize,
    pub(super) transcript_bytes: usize,
    pub(super) transcript_limited: bool,
    pub(super) audio_bytes: usize,
    pub(super) audio_limited: bool,
    pub(super) sample_rate: u32,
    pub(super) audio_evidence: RecordingEvidence,
}

#[derive(Default)]
pub(super) struct AudioWriteOutcome {
    pub(super) status_changed: bool,
    pub(super) write_failed: bool,
}

/// Update only bounded metadata after the owning source's actual append.
pub(super) fn append_recorded_audio(
    stats: &mut LocalCaptureStats,
    owner: (u64, u64),
    source: AudioSource,
    sample_rate: u32,
    data: &[u8],
    append: impl FnOnce(&[u8]) -> std::io::Result<()>,
) -> AudioWriteOutcome {
    if !stats.audio_evidence.owns(owner.0, owner.1, source)
        || stats.audio_limited
        || data.is_empty()
    {
        return AudioWriteOutcome::default();
    }
    let before = stats.audio_evidence.write_status(stats.audio_limited);
    let mut write_failed = false;
    if !matches!(sample_rate, 16_000 | 24_000)
        || !data.len().is_multiple_of(2)
        || (stats.sample_rate != 0 && stats.sample_rate != sample_rate)
    {
        stats.audio_limited = true;
        // Invalid PCM stops retention too, but must not look like a full disk
        // quota to the indicator.
        stats.audio_evidence.record(owner.0, owner.1, source, false);
    } else {
        let available =
            crate::core::session_archive::AUDIO_BYTE_LIMIT.saturating_sub(stats.audio_bytes);
        let accepted = data.len().min(available);
        if accepted == 0 {
            stats.audio_limited = true;
        } else if append(&data[..accepted]).is_err() {
            write_failed = true;
            stats.audio_evidence.record(owner.0, owner.1, source, false);
        } else {
            stats.sample_rate = sample_rate;
            stats.audio_bytes += accepted;
            stats.audio_limited =
                stats.audio_bytes == crate::core::session_archive::AUDIO_BYTE_LIMIT;
            stats.audio_evidence.record(owner.0, owner.1, source, true);
        }
    }
    AudioWriteOutcome {
        status_changed: stats.audio_evidence.write_status(stats.audio_limited) != before,
        write_failed,
    }
}

pub(super) fn finalize_pending_history(
    history: &SessionHistory,
    id: &str,
    ended_at_ms: u64,
    limited: bool,
    retain_text: bool,
    retain_audio: bool,
) -> std::io::Result<bool> {
    // An opt-out takes effect even when its initial filesystem cleanup fails.
    // Retry that cleanup before making any current content saved history.
    if !retain_text && !retain_audio {
        history.discard(id)?;
        return Ok(false);
    }
    if !retain_text {
        history.clear_text(id)?;
    }
    if !retain_audio {
        history.clear_audio(id)?;
    }
    history.finalize(id, ended_at_ms, limited)
}

impl SessionManager {
    pub(super) fn record_audio(
        self: &Arc<Self>,
        source: AudioSource,
        generation: u64,
        attempt: u64,
        sample_rate: u32,
        data: &[u8],
    ) {
        let outcome = {
            let pending = self.history_pending_id.lock().unwrap();
            // Recheck after acquiring the archive owner: a delayed old queue
            // must not write into a newly started session's files or evidence.
            if !self.history_pending_audio.load(Ordering::SeqCst)
                || self.is_paused()
                || !(self.is_generation_current(generation)
                    || self.stopping_tail_generation.load(Ordering::SeqCst) == generation)
            {
                return;
            }
            let Some(id) = pending.as_deref() else { return };
            let mut stats = self.history_stats.lock().unwrap();
            let outcome = append_recorded_audio(
                &mut stats,
                (generation, attempt),
                source,
                sample_rate,
                data,
                |accepted| {
                    self.history
                        .append_pcm_from(id, source, sample_rate, accepted)
                },
            );
            if outcome.write_failed {
                // Commit failure to this archive before releasing its owner;
                // a later start may reset the shared error flag.
                self.history_save_error.store(true, Ordering::SeqCst);
            }
            outcome
        };
        if outcome.write_failed {
            pipeline_log!("session history audio append failed label=write_failed");
        }
        if outcome.status_changed {
            // Steady PCM writes do not publish full subtitle snapshots.
            self.publish_state();
        }
    }

    pub fn archive_revision(&self) -> u64 {
        self.archive_revision.load(Ordering::SeqCst)
    }

    pub fn apply_archive_opt_out(self: &Arc<Self>, history: Option<bool>, audio: Option<bool>) {
        if history == Some(false) || audio == Some(false) {
            self.archive_revision.fetch_add(1, Ordering::SeqCst);
        }
        if history == Some(false) {
            self.history_pending_text.store(false, Ordering::SeqCst);
            if let Some(id) = self.history_pending_id.lock().unwrap().as_deref() {
                if self.history.clear_text(id).is_err() {
                    self.history_save_error.store(true, Ordering::SeqCst);
                }
            }
            self.controller.lock().unwrap().archive_mut().disable();
            let mut stats = self.history_stats.lock().unwrap();
            stats.transcript_count = 0;
            stats.transcript_bytes = 0;
            stats.transcript_limited = false;
        }
        if audio == Some(false) {
            self.history_pending_audio.store(false, Ordering::SeqCst);
            if let Some(id) = self.history_pending_id.lock().unwrap().as_deref() {
                if self.history.clear_audio(id).is_err() {
                    self.history_save_error.store(true, Ordering::SeqCst);
                }
            }
            for recording in self.recording.lock().unwrap().iter_mut() {
                recording.begin(false);
            }
            let mut stats = self.history_stats.lock().unwrap();
            stats.audio_bytes = 0;
            stats.audio_limited = false;
            stats.sample_rate = 0;
            stats.audio_evidence = RecordingEvidence::default();
            drop(stats);
            self.publish_state();
        }
    }

    pub fn archive_state(&self) -> crate::core::session_archive::ArchiveState {
        if self.is_ui_test() {
            let controller = self.controller.lock().unwrap();
            let recording = self.recording.lock().unwrap();
            return crate::core::session_archive::ArchiveState {
                transcript_count: controller.archive().count(),
                transcript_limited: controller.archive().limited,
                audio_sources: [AudioSource::System, AudioSource::Microphone]
                    .into_iter()
                    .zip(recording.iter())
                    .filter_map(|(source, recording)| (!recording.is_empty()).then_some(source))
                    .collect(),
                audio_bytes: recording.iter().map(|recording| recording.len()).sum(),
                audio_limited: recording.iter().any(|recording| recording.limited),
                sample_rate: recording
                    .iter()
                    .find(|recording| !recording.is_empty())
                    .map_or(0, |recording| recording.sample_rate),
                history_save_error: false,
            };
        }
        let audio_sources = self
            .history_pending_id
            .lock()
            .unwrap()
            .as_deref()
            .filter(|_| self.history_pending_audio.load(Ordering::SeqCst))
            .and_then(|id| self.history.audio_sources(id).ok())
            .unwrap_or_default();
        let stats = self.history_stats.lock().unwrap();
        crate::core::session_archive::ArchiveState {
            transcript_count: stats.transcript_count,
            transcript_limited: stats.transcript_limited,
            audio_sources,
            audio_bytes: stats.audio_bytes,
            audio_limited: stats.audio_limited,
            sample_rate: stats.sample_rate,
            history_save_error: self.history_save_error.load(Ordering::SeqCst),
        }
    }

    pub fn history(&self) -> Arc<SessionHistory> {
        Arc::clone(&self.history)
    }

    pub fn current_history_id(&self) -> Option<String> {
        self.history_pending_id.lock().unwrap().clone()
    }

    pub fn persist_current_history(&self) -> std::io::Result<()> {
        let mut pending = self.history_pending_id.lock().unwrap();
        let Some(id) = pending.as_ref() else {
            return Ok(());
        };
        if self.is_ui_test() {
            // Opt-in only within UI-test mode: no history writes, credentials,
            // provider connections or captured audio. New start/clear/opt-out
            // still reset these bounded, synthetic buffers normally.
            let mut controller = self.controller.lock().unwrap();
            for recording in self.recording.lock().unwrap().iter_mut() {
                finish_ui_test_archive(
                    &mut controller,
                    recording,
                    std::env::var("MIMI_UI_TEST_EXPORT").as_deref() == Ok("1"),
                );
            }
            *pending = None;
            self.history_pending_text.store(false, Ordering::SeqCst);
            self.history_pending_audio.store(false, Ordering::SeqCst);
            return Ok(());
        }
        let limited = self.history_stats.lock().unwrap().transcript_limited;
        if let Err(error) = finalize_pending_history(
            &self.history,
            id,
            crate::core::subtitle_reducer::now_epoch_ms(),
            limited,
            self.history_pending_text.load(Ordering::SeqCst),
            self.history_pending_audio.load(Ordering::SeqCst),
        ) {
            self.history_save_error.store(true, Ordering::SeqCst);
            return Err(error);
        }
        self.history_save_error.store(false, Ordering::SeqCst);
        *self.history_stats.lock().unwrap() = LocalCaptureStats::default();
        *pending = None;
        self.history_pending_text.store(false, Ordering::SeqCst);
        self.history_pending_audio.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub fn transcript_page(
        &self,
        query: &str,
        page: usize,
    ) -> std::io::Result<crate::core::session_archive::TranscriptPage> {
        if self.is_ui_test() {
            return Ok(self.controller.lock().unwrap().archive().page(query, page));
        }
        let pending = self.history_pending_id.lock().unwrap();
        let Some(id) = pending
            .as_deref()
            .filter(|_| self.history_pending_text.load(Ordering::SeqCst))
        else {
            return Ok(crate::core::session_archive::TranscriptPage {
                total: 0,
                page: 0,
                entries: Vec::new(),
            });
        };
        self.history.page(id, query, page)
    }

    pub fn export_transcript(
        &self,
        options: crate::core::session_archive::TranscriptExportOptions,
    ) -> std::io::Result<Option<Vec<u8>>> {
        if self.is_ui_test() {
            return Ok(self
                .controller
                .lock()
                .unwrap()
                .archive()
                .export_with_options(options)
                .map(String::into_bytes));
        }
        let pending = self.history_pending_id.lock().unwrap();
        match pending
            .as_deref()
            .filter(|_| self.history_pending_text.load(Ordering::SeqCst))
        {
            Some(id) => self
                .history
                .export_transcript_with_options(id, options)
                .map(Some),
            None => Ok(None),
        }
    }

    pub fn export_audio_from(
        &self,
        source: Option<AudioSource>,
    ) -> std::io::Result<Option<Vec<u8>>> {
        if self.is_ui_test() {
            let recordings = self.recording.lock().unwrap();
            let index = match source {
                Some(AudioSource::System) => Some(0),
                Some(AudioSource::Microphone) => Some(1),
                None => {
                    let mut available = recordings
                        .iter()
                        .enumerate()
                        .filter(|(_, recording)| !recording.is_empty());
                    let first = available.next().map(|(index, _)| index);
                    if available.next().is_some() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "Select an audio source to export.",
                        ));
                    }
                    first
                }
            };
            return Ok(index.and_then(|index| recordings[index].export()));
        }
        let pending = self.history_pending_id.lock().unwrap();
        match pending
            .as_deref()
            .filter(|_| self.history_pending_audio.load(Ordering::SeqCst))
        {
            Some(id) => self.history.audio_selected(id, source).map(Some),
            None => Ok(None),
        }
    }

    pub fn clear_archive(self: &Arc<Self>) -> std::io::Result<()> {
        let mut pending = self.history_pending_id.lock().unwrap();
        if let Some(id) = pending.as_deref() {
            self.history.discard(id)?;
        }
        *pending = None;
        self.history_pending_text.store(false, Ordering::SeqCst);
        self.history_pending_audio.store(false, Ordering::SeqCst);
        *self.history_stats.lock().unwrap() = LocalCaptureStats::default();
        self.history_save_error.store(false, Ordering::SeqCst);
        self.archive_revision.fetch_add(1, Ordering::SeqCst);
        self.controller.lock().unwrap().archive_mut().clear();
        for recording in self.recording.lock().unwrap().iter_mut() {
            recording.clear();
        }
        drop(pending);
        self.publish_state();
        Ok(())
    }

    pub async fn clear_subtitles(self: &Arc<Self>) -> std::io::Result<()> {
        let _content = self.subtitle_content_lock.lock().await;
        self.clear_current_subtitle_history()?;
        for source in [AudioSource::System, AudioSource::Microphone] {
            let generation = self.lane(source).client_generation.load(Ordering::SeqCst);
            if let Some(client) = self.client_for_generation(source, generation) {
                let revision = client.clear_content().await;
                *self.lane(source).subtitle_content_revision.lock().unwrap() =
                    (generation, revision);
            } else {
                // Keep the retired connection cut even after teardown has taken
                // its client. A previously popped stopping tail must stay stale.
                let mut boundary = self.lane(source).subtitle_content_revision.lock().unwrap();
                boundary.1 = boundary.1.wrapping_add(1);
            }
        }
        self.cancel_translation_timeout();
        self.controller.lock().unwrap().clear_subtitles();
        self.publish_state();
        Ok(())
    }

    pub(super) fn clear_current_subtitle_history(&self) -> std::io::Result<()> {
        let pending = self.history_pending_id.lock().unwrap();
        if let Some(id) = pending.as_deref() {
            self.history.clear_text(id)?;
        }
        let mut stats = self.history_stats.lock().unwrap();
        stats.transcript_count = 0;
        stats.transcript_bytes = 0;
        stats.transcript_limited = false;
        drop(stats);
        self.archive_revision.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
