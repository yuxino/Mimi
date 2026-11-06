//! Content-free stale-original detection for independent transcript streams.
//! Missing optional originals are not proof of a failed service connection.

const SOURCE_STALE_MS: u64 = 20_000;
const MAX_TRANSLATION_GAP_MS: u64 = 5_000;
const MIN_TRANSLATION_UPDATES: usize = 8;
const MIN_TRANSLATION_CHARACTERS: usize = 64;

#[derive(Default)]
pub struct SourceTranscriptProgress {
    translation_only_since: Option<u64>,
    last_translation: Option<u64>,
    updates: usize,
    characters: usize,
    stale: bool,
}

impl SourceTranscriptProgress {
    pub fn source(&mut self) {
        self.translation_only_since = None;
        self.updates = 0;
        self.characters = 0;
        self.stale = false;
    }

    /// `now_ms` is local monotonic receipt time, never provider alignment time.
    /// Isolated late tails and silence cannot accumulate a stale-preview window.
    pub fn translation(&mut self, now_ms: u64, characters: usize) -> bool {
        if characters == 0 || self.stale {
            return self.stale;
        }
        if self
            .last_translation
            .is_none_or(|previous| now_ms.saturating_sub(previous) > MAX_TRANSLATION_GAP_MS)
        {
            self.translation_only_since = None;
            self.updates = 0;
            self.characters = 0;
        }
        let since = *self.translation_only_since.get_or_insert(now_ms);
        self.last_translation = Some(now_ms);
        self.updates = self.updates.saturating_add(1);
        self.characters = self.characters.saturating_add(characters);
        self.stale = now_ms.saturating_sub(since) >= SOURCE_STALE_MS
            && self.updates >= MIN_TRANSLATION_UPDATES
            && self.characters >= MIN_TRANSLATION_CHARACTERS;
        self.stale
    }

    pub fn stale(&self) -> bool {
        self.stale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continued_translation_without_originals_has_a_finite_deadline() {
        for source_seen in [false, true] {
            let mut progress = SourceTranscriptProgress::default();
            if source_seen {
                progress.source();
            }
            for time in (0..SOURCE_STALE_MS).step_by(2_000) {
                assert!(!progress.translation(time, 10));
            }
            assert!(progress.translation(SOURCE_STALE_MS, 10));
        }
    }

    #[test]
    fn silence_short_tails_and_source_only_same_language_do_not_isolate() {
        let mut progress = SourceTranscriptProgress::default();
        for time in (0..120_000).step_by(10_000) {
            progress.source();
            assert!(!progress.stale());
            assert!(!progress.translation(time, 8));
        }
        // A fresh translation after a long quiet interval gets a new window.
        assert!(!progress.translation(600_000, 100));
        assert!(!progress.translation(602_000, 100));
        assert!(!progress.stale());
    }

    #[test]
    fn resumed_source_starts_a_new_progress_window() {
        let mut progress = SourceTranscriptProgress::default();
        for time in (0..18_000).step_by(2_000) {
            assert!(!progress.translation(time, 10));
        }
        progress.source();
        assert!(!progress.stale());
        for time in (18_000..38_000).step_by(2_000) {
            assert!(!progress.translation(time, 10));
        }
        assert!(progress.translation(38_000, 10));
        progress.source();
        assert!(!progress.stale());
        assert!(!progress.translation(40_000, 10));
    }
}
