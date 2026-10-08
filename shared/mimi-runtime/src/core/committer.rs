//! Tracks the latest cumulative ASR draft and derives replaceable preview
//! candidates without advancing any durable subtitle boundary.

pub struct ASRDraftCommitter {
    long_incomplete_commit_threshold: usize,
    latest_draft: String,
}

impl ASRDraftCommitter {
    pub fn new(long_incomplete_commit_threshold: usize) -> Self {
        Self {
            long_incomplete_commit_threshold: long_incomplete_commit_threshold.max(1),
            latest_draft: String::new(),
        }
    }

    pub fn has_pending_text(&self) -> bool {
        Self::is_meaningful(&self.latest_draft)
    }

    /// Returns the whole latest draft without making it durable. When
    /// `require_long` is true, short incomplete speech stays pending until it
    /// grows or the provider supplies an authoritative final.
    pub fn preview_latest_draft(&self, require_long: bool) -> Option<String> {
        if !Self::is_meaningful(&self.latest_draft) {
            return None;
        }
        if require_long && self.latest_draft.chars().count() < self.long_incomplete_commit_threshold
        {
            return None;
        }
        Some(self.latest_draft.clone())
    }

    /// Replaces the cumulative provider draft and returns its normalized text.
    pub fn update_draft(&mut self, text: &str) -> String {
        if !crate::core::models::subtitle_text_within_limit(text) {
            return self.latest_draft.clone();
        }
        self.latest_draft = text.trim().to_string();
        self.latest_draft.clone()
    }

    pub fn reset(&mut self) {
        self.latest_draft.clear();
    }

    /// Text is meaningful when it contains a non-whitespace, non-punctuation
    /// character.
    fn is_meaningful(text: &str) -> bool {
        text.chars()
            .any(|character| !character.is_whitespace() && !is_punctuation(character))
    }
}

fn is_punctuation(character: char) -> bool {
    !character.is_alphanumeric() && !character.is_whitespace() && !character.is_control()
        || character.is_ascii_punctuation()
        || matches!(
            character,
            '。' | '、'
                | '！'
                | '？'
                | '「'
                | '」'
                | '『'
                | '』'
                | '（'
                | '）'
                | '・'
                | '…'
                | '—'
                | '–'
        )
}

impl Default for ASRDraftCommitter {
    fn default() -> Self {
        Self::new(20)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_draft_keeps_the_previous_complete_candidate_and_exact_utf8_is_accepted() {
        let mut committer = ASRDraftCommitter::default();
        committer.update_draft("Synthetic previous candidate");
        let exact = "🙂".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES / 4);
        assert_eq!(
            committer.update_draft(&format!("{exact}a")),
            "Synthetic previous candidate"
        );
        assert_eq!(
            committer.preview_latest_draft(false).as_deref(),
            Some("Synthetic previous candidate")
        );
        assert_eq!(committer.update_draft(&exact), exact);
    }

    #[test]
    fn keeps_an_incomplete_draft_pending() {
        let mut committer = ASRDraftCommitter::default();

        assert_eq!(committer.update_draft("今日は"), "今日は");
        assert!(committer.has_pending_text());
        assert_eq!(
            committer.preview_latest_draft(false).as_deref(),
            Some("今日は")
        );
    }

    #[test]
    fn previews_complete_sentences_without_losing_the_tail() {
        let mut committer = ASRDraftCommitter::default();
        let draft = "こんにちは。今日は天気が";
        let _ = committer.update_draft(draft);

        assert_eq!(
            committer.preview_latest_draft(false).as_deref(),
            Some(draft)
        );
    }

    #[test]
    fn previews_multiple_sentences_with_the_incomplete_tail() {
        let mut committer = ASRDraftCommitter::default();
        let _ = committer.update_draft("あ！え？うん。まだ");

        assert_eq!(
            committer.preview_latest_draft(false).as_deref(),
            Some("あ！え？うん。まだ")
        );
    }

    #[test]
    fn supports_english_and_chinese_delimiters() {
        let mut english = ASRDraftCommitter::default();
        let _ = english.update_draft("Hello there. How are you?");
        assert_eq!(
            english.preview_latest_draft(false).as_deref(),
            Some("Hello there. How are you?")
        );

        let mut chinese = ASRDraftCommitter::default();
        let _ = chinese.update_draft("你好！今天天气不错。明天");
        assert_eq!(
            chinese.preview_latest_draft(false).as_deref(),
            Some("你好！今天天气不错。明天")
        );
    }

    #[test]
    fn maximum_wait_requires_the_configured_length() {
        let mut committer = ASRDraftCommitter::new(8);
        let _ = committer.update_draft("short");
        assert_eq!(committer.preview_latest_draft(true), None);

        let draft = "First sentence. trailing words";
        let _ = committer.update_draft(draft);
        assert_eq!(committer.preview_latest_draft(true).as_deref(), Some(draft));
    }

    #[test]
    fn punctuation_only_drafts_are_not_meaningful() {
        let mut committer = ASRDraftCommitter::default();
        let _ = committer.update_draft("……！？");

        assert!(!committer.has_pending_text());
        assert_eq!(committer.preview_latest_draft(false), None);
    }

    #[test]
    fn a_new_draft_replaces_the_previous_preview_candidate() {
        let mut committer = ASRDraftCommitter::default();
        let _ = committer.update_draft("こんにちは。");
        assert_eq!(
            committer.preview_latest_draft(false).as_deref(),
            Some("こんにちは。")
        );

        let extended = "こんにちは。まだ話しています";
        let _ = committer.update_draft(extended);
        assert_eq!(
            committer.preview_latest_draft(false).as_deref(),
            Some(extended)
        );
    }

    #[test]
    fn reset_clears_the_latest_draft() {
        let mut committer = ASRDraftCommitter::default();
        let _ = committer.update_draft("こんにちは。途中まで");
        committer.reset();

        assert!(!committer.has_pending_text());
        assert_eq!(committer.preview_latest_draft(false), None);
    }
}
