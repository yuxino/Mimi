//! Desktop persistence adapter for the single shared subtitle reducer.

pub use mimi_core::subtitle_reducer::{now_epoch_ms, trim};
pub type SubtitleReducer = mimi_core::SubtitleReducer<super::session_archive::TranscriptArchive>;

impl mimi_core::ArchiveSink for super::session_archive::TranscriptArchive {
    fn append(&mut self, pair: &super::models::SubtitlePair) {
        super::session_archive::TranscriptArchive::append(self, pair);
    }
    fn clear(&mut self) {
        super::session_archive::TranscriptArchive::clear(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::SubtitleEvent;

    #[test]
    fn opted_in_archive_outlives_overlay_history_and_excludes_drafts() {
        let mut reducer = SubtitleReducer::new(2);
        reducer.archive.begin(true, 0);
        reducer.apply(SubtitleEvent::SourceDraft("unconfirmed source".into()));
        reducer.apply(SubtitleEvent::TranslationDraft(
            "unconfirmed translation".into(),
        ));
        assert_eq!(reducer.archive.count(), 0);
        for i in 0..25 {
            reducer.apply(SubtitleEvent::FinalPair {
                source: format!("synthetic source {i}"),
                translation: format!("synthetic translation {i}"),
            });
        }
        assert_eq!(reducer.snapshot.history.len(), 2);
        assert_eq!(reducer.archive.count(), 25);
        reducer.reset_transient();
        assert_eq!(reducer.archive.count(), 25);
        reducer.apply(SubtitleEvent::Clear);
        assert_eq!(reducer.archive.count(), 0);
    }

    #[test]
    fn final_duplicates_and_empty_pairs_do_not_enter_archive() {
        let mut reducer = SubtitleReducer::default();
        reducer.archive.begin(true, 0);
        for _ in 0..2 {
            reducer.apply(SubtitleEvent::FinalPair {
                source: "synthetic".into(),
                translation: "test".into(),
            });
        }
        reducer.apply(SubtitleEvent::FinalPair {
            source: "".into(),
            translation: "test".into(),
        });
        assert_eq!(reducer.archive.count(), 1);
    }
}
