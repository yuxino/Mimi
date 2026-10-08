//! Explicitly opted-in, bounded session buffers and export formatting. No disk or OS APIs.
use crate::core::audio_input::AudioSource;
use crate::core::models::SubtitlePair;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

pub const TRANSCRIPT_BYTE_LIMIT: usize = 2 * 1024 * 1024;
pub const TRANSCRIPT_COUNT_LIMIT: usize = 10_000;
pub const AUDIO_BYTE_LIMIT: usize = 64 * 1024 * 1024;
pub const TRANSCRIPT_PAGE_SIZE: usize = 30;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPageEntry {
    pub audio_source: AudioSource,
    pub index: usize,
    pub source: String,
    pub translation: String,
    pub created_at_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPage {
    pub total: usize,
    pub page: usize,
    pub entries: Vec<TranscriptPageEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTranscript {
    pub version: u32,
    pub id: String,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub limited: bool,
    pub entries: Vec<SubtitlePair>,
}

impl SavedTranscript {
    pub fn page(&self, query: &str, requested_page: usize) -> TranscriptPage {
        transcript_page(&self.entries, query, requested_page)
    }

    pub fn export(&self) -> Option<String> {
        format_transcript(self.started_at_ms, &self.entries, self.limited)
    }
}

fn transcript_page(entries: &[SubtitlePair], query: &str, requested_page: usize) -> TranscriptPage {
    let query = query.trim().to_lowercase();
    let matches: Vec<_> = entries
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, pair)| {
            query.is_empty()
                || pair.source.to_lowercase().contains(&query)
                || pair.translation.to_lowercase().contains(&query)
        })
        .collect();
    let total = matches.len();
    let page = requested_page.min(total.saturating_sub(1) / TRANSCRIPT_PAGE_SIZE);
    let entries = matches
        .into_iter()
        .skip(page * TRANSCRIPT_PAGE_SIZE)
        .take(TRANSCRIPT_PAGE_SIZE)
        .map(|(index, pair)| TranscriptPageEntry {
            audio_source: pair.audio_source,
            index: index + 1,
            source: pair.source.clone(),
            translation: pair.translation.clone(),
            created_at_ms: pair.created_at_ms,
        })
        .collect();
    TranscriptPage {
        total,
        page,
        entries,
    }
}

#[derive(Default)]
pub struct TranscriptArchive {
    enabled: bool,
    entries: Vec<SubtitlePair>,
    bytes: usize,
    pub limited: bool,
    started_at_ms: u64,
}

impl TranscriptArchive {
    pub fn begin(&mut self, enabled: bool, started_at_ms: u64) {
        *self = Self {
            enabled,
            started_at_ms,
            ..Self::default()
        };
    }

    pub fn disable(&mut self) {
        *self = Self::default();
    }
    pub fn clear(&mut self) {
        self.begin(self.enabled, self.started_at_ms);
    }
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    pub fn page(&self, query: &str, requested_page: usize) -> TranscriptPage {
        transcript_page(&self.entries, query, requested_page)
    }

    pub fn append(&mut self, pair: &SubtitlePair) {
        if !self.enabled || self.limited {
            return;
        }
        let bytes = pair.source.len().saturating_add(pair.translation.len());
        if self.entries.len() >= TRANSCRIPT_COUNT_LIMIT
            || bytes > TRANSCRIPT_BYTE_LIMIT.saturating_sub(self.bytes)
        {
            self.limited = true;
            return;
        }
        self.bytes += bytes;
        self.entries.push(pair.clone());
    }

    pub fn export(&self) -> Option<String> {
        format_transcript(self.started_at_ms, &self.entries, self.limited)
    }
}

fn format_transcript(
    started_at_ms: u64,
    entries: &[SubtitlePair],
    limited: bool,
) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let mut text = format!("Mimi transcript\nSession started: {}\nTimestamps mark final confirmation, not media playback positions.\n\n", utc_timestamp(started_at_ms));
    for pair in entries {
        let elapsed = pair.created_at_ms.saturating_sub(started_at_ms);
        let _ = writeln!(
            text,
            "[{} | +{:02}:{:02}:{:02}.{:03} | {}]\n{}\n{}\n",
            utc_timestamp(pair.created_at_ms),
            elapsed / 3_600_000,
            elapsed / 60_000 % 60,
            elapsed / 1_000 % 60,
            elapsed % 1_000,
            match pair.audio_source {
                AudioSource::System => "System audio",
                AudioSource::Microphone => "Microphone",
            },
            pair.source,
            pair.translation
        );
    }
    if limited {
        text.push_str("[Transcript limit reached; later entries were not retained.]\n");
    }
    Some(text)
}

fn utc_timestamp(ms: u64) -> String {
    let Ok(timestamp) =
        time::OffsetDateTime::from_unix_timestamp((ms / 1000).min(i64::MAX as u64) as i64)
    else {
        return "unknown".into();
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        timestamp.year(),
        u8::from(timestamp.month()),
        timestamp.day(),
        timestamp.hour(),
        timestamp.minute(),
        timestamp.second(),
        ms % 1000
    )
}

#[derive(Default)]
pub struct AudioRecording {
    enabled: bool,
    pcm: Vec<u8>,
    pub sample_rate: u32,
    pub limited: bool,
}

impl AudioRecording {
    pub fn begin(&mut self, enabled: bool) {
        *self = Self {
            enabled,
            ..Self::default()
        };
    }
    pub fn clear(&mut self) {
        self.begin(self.enabled);
    }
    pub fn len(&self) -> usize {
        self.pcm.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pcm.is_empty()
    }
    pub fn append(&mut self, sample_rate: u32, data: &[u8]) {
        if !self.enabled || self.limited || data.is_empty() {
            return;
        }
        if !matches!(sample_rate, 16_000 | 24_000)
            || !data.len().is_multiple_of(2)
            || (self.sample_rate != 0 && self.sample_rate != sample_rate)
        {
            self.limited = true;
            return;
        }
        self.sample_rate = sample_rate;
        let available = AUDIO_BYTE_LIMIT.saturating_sub(self.pcm.len());
        let count = data.len().min(available);
        self.pcm.extend_from_slice(&data[..count]);
        self.limited = self.pcm.len() == AUDIO_BYTE_LIMIT;
    }

    pub fn export(&self) -> Option<Vec<u8>> {
        if self.pcm.is_empty() {
            return None;
        }
        let size = self.pcm.len() as u32;
        let mut wav = Vec::with_capacity(self.pcm.len() + 44);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(size + 36).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1_u16.to_le_bytes()); // mono
        wav.extend_from_slice(&self.sample_rate.to_le_bytes());
        wav.extend_from_slice(&(self.sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&size.to_le_bytes());
        wav.extend_from_slice(&self.pcm);
        Some(wav)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveState {
    pub audio_sources: Vec<AudioSource>,
    pub transcript_count: usize,
    pub transcript_limited: bool,
    pub audio_bytes: usize,
    pub audio_limited: bool,
    pub sample_rate: u32,
    pub history_save_error: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair(at: u64) -> SubtitlePair {
        SubtitlePair::new("日本語".into(), "中文".into(), at)
    }

    #[test]
    fn default_retains_nothing_and_opt_out_erases_content() {
        let mut transcript = TranscriptArchive::default();
        transcript.append(&pair(1500));
        assert!(transcript.export().is_none());
        transcript.begin(true, 1000);
        transcript.append(&pair(1500));
        assert!(transcript.export().unwrap().contains("+00:00:00.500"));
        transcript.disable();
        assert!(transcript.export().is_none());
        let mut audio = AudioRecording::default();
        audio.append(16000, &[1, 0]);
        assert!(audio.export().is_none());
        audio.begin(true);
        audio.append(16000, &[1, 0]);
        audio.begin(false);
        assert!(audio.export().is_none());
    }

    #[test]
    fn page_searches_confirmed_pairs_and_clamps_after_clear() {
        let mut archive = TranscriptArchive::default();
        archive.begin(true, 0);
        for index in 0..35 {
            archive.append(&SubtitlePair::new(
                format!("Source {index}"),
                format!("译文 {index}"),
                index,
            ));
        }
        let page = archive.page("", 1);
        assert_eq!((page.total, page.page, page.entries.len()), (35, 1, 5));
        assert_eq!(page.entries[0].index, 5);
        let page = archive.page("译文 3", 20);
        assert_eq!((page.total, page.page), (6, 0));
        archive.clear();
        let page = archive.page("", 20);
        assert_eq!((page.total, page.page, page.entries.len()), (0, 0, 0));
    }

    #[test]
    fn timestamps_are_utc_and_clock_reversal_does_not_underflow() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(utc_timestamp(86_400_123), "1970-01-02T00:00:00.123Z");
        let mut archive = TranscriptArchive::default();
        archive.begin(true, 2000);
        archive.append(&pair(1000));
        assert!(archive.export().unwrap().contains("+00:00:00.000"));
    }

    #[test]
    fn transcript_limit_stops_retention_and_new_session_resets_it() {
        let mut archive = TranscriptArchive::default();
        archive.begin(true, 0);
        archive.append(&pair(1));
        archive.append(&SubtitlePair::new(
            "x".repeat(TRANSCRIPT_BYTE_LIMIT),
            "y".into(),
            2,
        ));
        archive.append(&pair(3));
        assert_eq!(archive.count(), 1);
        assert!(archive.limited);
        archive.begin(true, 4);
        assert_eq!(archive.count(), 0);
        assert!(!archive.limited);
        for i in 0..=TRANSCRIPT_COUNT_LIMIT {
            archive.append(&pair(i as u64));
        }
        assert_eq!(archive.count(), TRANSCRIPT_COUNT_LIMIT);
        assert!(archive.limited);
    }

    #[test]
    fn wav_header_preserves_samples_at_each_provider_rate() {
        for rate in [16_000, 24_000] {
            let mut recording = AudioRecording::default();
            recording.begin(true);
            recording.append(rate, &[1, 0, 255, 127]);
            let wav = recording.export().unwrap();
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            assert_eq!(&wav[24..28], &rate.to_le_bytes());
            assert_eq!(&wav[40..44], &4_u32.to_le_bytes());
            assert_eq!(&wav[44..], &[1, 0, 255, 127]);
        }
    }

    #[test]
    fn audio_limit_and_format_change_never_corrupt_pcm() {
        let mut recording = AudioRecording::default();
        recording.begin(true);
        recording.append(16000, &[0, 0]);
        recording.append(24000, &[1, 0]);
        assert_eq!(recording.len(), 2);
        assert!(recording.limited);
        recording.clear();
        recording.pcm.resize(AUDIO_BYTE_LIMIT - 2, 0);
        recording.append(16000, &[1, 0, 2, 0]);
        assert_eq!(recording.len(), AUDIO_BYTE_LIMIT);
        assert_eq!(&recording.pcm[AUDIO_BYTE_LIMIT - 2..], &[1, 0]);
        assert!(recording.limited);
        recording.append(16000, &[3, 0]);
        assert_eq!(recording.len(), AUDIO_BYTE_LIMIT);
    }
}
