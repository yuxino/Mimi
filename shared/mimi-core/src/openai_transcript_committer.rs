//! Aligns OpenAI deltas and assembles Gemini's independent continuous transcripts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum TranscriptEvent {
    SourceDraft {
        text: String,
        language: Option<String>,
    },
    TranslationDraft(String),
    SubtitleFinalPair {
        source: String,
        language: Option<String>,
        translation: String,
    },
    Error {
        code: String,
        message: String,
    },
}

pub const GEMINI_TRANSCRIPT_QUIET_MS: u64 = 2_000;
pub const GEMINI_TURN_TAIL_QUIET_MS: u64 = 500;
const MAXIMUM_ALIGNMENT_SKEW_MS: u64 = 400;
const MINIMUM_BUFFER_SAFETY_LIMIT: usize = 4_096;
const BUFFER_SAFETY_LIMIT_MULTIPLIER: usize = 16;
const SENTENCE_DELIMITERS: [char; 7] = ['.', '!', '?', '。', '！', '？', '\n'];

#[derive(Default, Serialize, Deserialize)]
struct TimedTextBuffer {
    text: String,
    boundaries: Vec<TimedBoundary>,
    #[serde(default)]
    cumulative_snapshots: bool,
    #[serde(default)]
    checkpoint_prefix: String,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct TimedBoundary {
    character_count: usize,
    elapsed_ms: u64,
}

impl TimedTextBuffer {
    fn character_count(&self) -> usize {
        self.text.chars().count()
    }

    fn has_timing_metadata(&self) -> bool {
        !self.boundaries.is_empty()
    }

    fn append(&mut self, delta: &str, elapsed_ms: Option<u64>) {
        self.text.push_str(delta);
        let Some(elapsed_ms) = elapsed_ms else { return };
        let elapsed_ms = self
            .boundaries
            .last()
            .map_or(elapsed_ms, |last| elapsed_ms.max(last.elapsed_ms));
        self.boundaries.push(TimedBoundary {
            character_count: self.character_count(),
            elapsed_ms,
        });
    }

    /// A strict extension of the whole pending buffer is a cumulative snapshot.
    /// Otherwise preserve the fragment exactly, including subword boundaries and
    /// whitespace. A matching tail alone cannot distinguish a resend from speech
    /// repetition. Equal snapshots are ignored only after cumulative evidence.
    fn merge_gemini(&mut self, chunk: &str) -> bool {
        // A local checkpoint does not end the provider's cumulative stream.
        let continuation = (!self.checkpoint_prefix.is_empty())
            .then(|| chunk.strip_prefix(&self.checkpoint_prefix))
            .flatten();
        let chunk = continuation.unwrap_or(chunk);
        if chunk.is_empty() {
            return false;
        }
        if !self.text.is_empty() && chunk.len() > self.text.len() && chunk.starts_with(&self.text) {
            if continuation.is_none() {
                // A new cumulative block has its own prefix. Plain fragments
                // still append verbatim and retain the prior cumulative base.
                self.checkpoint_prefix.clear();
            }
            self.text.clear();
            self.text.push_str(chunk);
            self.cumulative_snapshots = true;
        } else if self.cumulative_snapshots && chunk == self.text {
            return false;
        } else {
            self.text.push_str(chunk);
        }
        true
    }

    fn aligned_prefixes(&self, other: &Self, maximum_skew_ms: u64) -> Option<(usize, usize)> {
        let own_latest = self.boundaries.last()?.elapsed_ms;
        let other_latest = other.boundaries.last()?.elapsed_ms;
        let shared_coverage = own_latest.min(other_latest);
        let own = self.boundary_nearest_to(shared_coverage, maximum_skew_ms)?;
        let other = other.boundary_nearest_to(shared_coverage, maximum_skew_ms)?;
        if own.elapsed_ms.abs_diff(other.elapsed_ms) > maximum_skew_ms {
            return None;
        }
        Some((own.character_count, other.character_count))
    }

    fn boundary_nearest_to(
        &self,
        elapsed_ms: u64,
        maximum_forward_skew_ms: u64,
    ) -> Option<TimedBoundary> {
        self.boundaries
            .iter()
            .rev()
            .find(|boundary| boundary.elapsed_ms <= elapsed_ms)
            .copied()
            .or_else(|| {
                self.boundaries.first().copied().filter(|boundary| {
                    boundary.elapsed_ms.saturating_sub(elapsed_ms) <= maximum_forward_skew_ms
                })
            })
    }

    fn consume(&mut self, prefix_character_count: usize) -> String {
        let count = prefix_character_count.min(self.character_count());
        let byte_index = self
            .text
            .char_indices()
            .nth(count)
            .map_or(self.text.len(), |(index, _)| index);
        let suffix = self.text.split_off(byte_index);
        let prefix = std::mem::replace(&mut self.text, suffix);
        self.boundaries = self
            .boundaries
            .iter()
            .filter(|boundary| boundary.character_count > count)
            .map(|boundary| TimedBoundary {
                character_count: boundary.character_count - count,
                elapsed_ms: boundary.elapsed_ms,
            })
            .collect();
        prefix
    }

    fn checkpoint(&mut self) {
        if self.cumulative_snapshots {
            self.checkpoint_prefix.push_str(&self.text);
        }
        self.text.clear();
        self.boundaries.clear();
    }

    fn retained_character_count(&self) -> usize {
        self.character_count() + self.checkpoint_prefix.chars().count()
    }

    fn reset(&mut self) {
        self.text.clear();
        self.boundaries.clear();
        self.cumulative_snapshots = false;
        self.checkpoint_prefix.clear();
    }
}

#[derive(Serialize, Deserialize)]
pub struct OpenAITranscriptPairCommitter {
    maximum_pending_characters: usize,
    maximum_buffer_characters: usize,
    source_language: Option<String>,
    #[serde(default)]
    stable_block_mode: bool,
    #[serde(default)]
    last_delta_ms: Option<u64>,
    #[serde(default)]
    explicit_turn: bool,
    source: TimedTextBuffer,
    translation: TimedTextBuffer,
}

impl OpenAITranscriptPairCommitter {
    /// Validate opaque restored state before a platform bridge accepts it.
    pub fn validate_state(&self, maximum_pending_characters: usize) -> Result<(), &'static str> {
        let expected_limit = maximum_pending_characters.max(8);
        let buffer_limit = expected_limit
            .saturating_mul(BUFFER_SAFETY_LIMIT_MULTIPLIER)
            .max(MINIMUM_BUFFER_SAFETY_LIMIT);
        let valid_buffer = |buffer: &TimedTextBuffer| {
            let characters = buffer.character_count();
            crate::subtitle_text_within_limit(&buffer.text)
                && crate::subtitle_text_within_limit(&buffer.checkpoint_prefix)
                && buffer.retained_character_count() <= buffer_limit
                && (buffer.checkpoint_prefix.is_empty() || buffer.cumulative_snapshots)
                && (!buffer.cumulative_snapshots
                    || (self.stable_block_mode && buffer.boundaries.is_empty()))
                && buffer.boundaries.len() <= characters
                && buffer.boundaries.iter().all(|boundary| {
                    boundary.character_count > 0 && boundary.character_count <= characters
                })
                && buffer.boundaries.windows(2).all(|pair| {
                    pair[0].character_count <= pair[1].character_count
                        && pair[0].elapsed_ms <= pair[1].elapsed_ms
                })
        };
        if self.maximum_pending_characters == expected_limit
            && self.maximum_buffer_characters == buffer_limit
            && self
                .source_language
                .as_deref()
                .is_none_or(crate::subtitle_text_within_limit)
            && valid_buffer(&self.source)
            && valid_buffer(&self.translation)
        {
            Ok(())
        } else {
            Err("openai_stream_state_invalid")
        }
    }

    pub fn new(maximum_pending_characters: usize, source_language: Option<String>) -> Self {
        let maximum_pending_characters = maximum_pending_characters.max(8);
        let maximum_buffer_characters = maximum_pending_characters
            .saturating_mul(BUFFER_SAFETY_LIMIT_MULTIPLIER)
            .max(MINIMUM_BUFFER_SAFETY_LIMIT);
        Self {
            maximum_pending_characters,
            maximum_buffer_characters,
            source_language,
            stable_block_mode: false,
            last_delta_ms: None,
            explicit_turn: false,
            source: TimedTextBuffer::default(),
            translation: TimedTextBuffer::default(),
        }
    }

    /// Gemini supplies fragments or cumulative extensions without utterance IDs
    /// or audio timing. Draft presentation is independent of local finalization.
    /// Keep whole blocks together: equal punctuation counts do not prove alignment.
    pub fn new_gemini() -> Self {
        Self {
            stable_block_mode: true,
            ..Self::default()
        }
    }

    pub fn is_stable_block_mode(&self) -> bool {
        self.stable_block_mode
    }

    /// The caller supplies a monotonic receipt clock, never an audio timestamp.
    /// Silence is a heuristic caption checkpoint, not an aligned utterance or a
    /// provider terminal event. Require both lanes; punctuation is not required.
    /// Unmatched text remains live until its counterpart or an explicit boundary.
    pub fn settle(&mut self, now_ms: u64) -> Vec<TranscriptEvent> {
        let quiet_ms = if self.explicit_turn {
            GEMINI_TURN_TAIL_QUIET_MS
        } else {
            GEMINI_TRANSCRIPT_QUIET_MS
        };
        if !self.stable_block_mode
            || self
                .last_delta_ms
                .is_none_or(|last| now_ms.saturating_sub(last) < quiet_ms)
        {
            return Vec::new();
        }
        if self.explicit_turn {
            return self.finish();
        }
        if !is_meaningful(&self.source.text) || !is_meaningful(&self.translation.text) {
            return Vec::new();
        }
        let event = self.final_pair(self.source.text.clone(), self.translation.text.clone());
        self.source.checkpoint();
        self.translation.checkpoint();
        self.last_delta_ms = None;
        event.into_iter().collect()
    }

    pub fn note_turn_complete(&mut self, now_ms: u64) {
        if self.stable_block_mode {
            self.explicit_turn = true;
            self.last_delta_ms = Some(now_ms);
        }
    }

    pub fn append_source_delta(
        &mut self,
        delta: &str,
        elapsed_ms: Option<u64>,
    ) -> Vec<TranscriptEvent> {
        if delta.is_empty() {
            return Vec::new();
        }
        if self.stable_block_mode {
            if !self.source.merge_gemini(delta) {
                return Vec::new();
            }
            self.last_delta_ms = elapsed_ms;
        } else {
            self.source.append(delta, elapsed_ms);
        }
        let preview = TranscriptEvent::SourceDraft {
            text: self.source.text.clone(),
            language: self.source_language.clone(),
        };
        self.events_after_append(preview)
    }

    pub fn append_translation_delta(
        &mut self,
        delta: &str,
        elapsed_ms: Option<u64>,
    ) -> Vec<TranscriptEvent> {
        if delta.is_empty() {
            return Vec::new();
        }
        if self.stable_block_mode {
            if !self.translation.merge_gemini(delta) {
                return Vec::new();
            }
            self.last_delta_ms = elapsed_ms;
        } else {
            self.translation.append(delta, elapsed_ms);
        }
        let preview = TranscriptEvent::TranslationDraft(self.translation.text.clone());
        self.events_after_append(preview)
    }

    fn events_after_append(&mut self, preview: TranscriptEvent) -> Vec<TranscriptEvent> {
        let committed = self.commit_available_aligned_blocks();
        if self.exceeded_safety_limit() {
            self.reset();
            return vec![TranscriptEvent::Error {
                code: "openai_transcript_safety_limit".into(),
                message:
                    "OpenAI Realtime Translation transcript alignment exceeded its safety limit."
                        .into(),
            }];
        }
        let mut events = vec![preview];
        events.extend(committed);
        events
    }

    /// A graceful provider close is the only point where unmatched tails can
    /// safely be treated as describing the same session interval.
    pub fn finish(&mut self) -> Vec<TranscriptEvent> {
        let event = self.final_pair(self.source.text.clone(), self.translation.text.clone());
        self.reset();
        event.into_iter().collect()
    }

    pub fn has_pending(&self) -> bool {
        is_meaningful(&self.source.text) || is_meaningful(&self.translation.text)
    }

    pub fn reset(&mut self) {
        self.source.reset();
        self.translation.reset();
        self.last_delta_ms = None;
        self.explicit_turn = false;
    }

    #[cfg(test)]
    fn pending_source_character_count(&self) -> usize {
        self.source.character_count()
    }

    #[cfg(test)]
    fn pending_translation_character_count(&self) -> usize {
        self.translation.character_count()
    }

    fn commit_available_aligned_blocks(&mut self) -> Vec<TranscriptEvent> {
        let mut events = Vec::new();
        let mut committed_any = false;
        while let Some((source_length, translation_length)) = self.next_safe_commit_lengths() {
            let source = self.source.consume(source_length);
            let translation = self.translation.consume(translation_length);
            if let Some(final_pair) = self.final_pair(source, translation) {
                events.push(final_pair);
                committed_any = true;
            }
        }
        if committed_any {
            if is_meaningful(&self.source.text) {
                events.push(TranscriptEvent::SourceDraft {
                    text: self.source.text.clone(),
                    language: self.source_language.clone(),
                });
            }
            if is_meaningful(&self.translation.text) {
                events.push(TranscriptEvent::TranslationDraft(
                    self.translation.text.clone(),
                ));
            }
        }
        events
    }

    fn next_safe_commit_lengths(&self) -> Option<(usize, usize)> {
        if self.stable_block_mode {
            return None;
        }
        if let Some((source_aligned_length, translation_aligned_length)) = self
            .source
            .aligned_prefixes(&self.translation, MAXIMUM_ALIGNMENT_SKEW_MS)
        {
            let source_aligned = char_prefix(&self.source.text, source_aligned_length);
            let translation_aligned =
                char_prefix(&self.translation.text, translation_aligned_length);
            if !is_meaningful(&source_aligned) || !is_meaningful(&translation_aligned) {
                return None;
            }
            if let (Some(source_sentence), Some(translation_sentence)) = (
                complete_prefix_boundary(&source_aligned),
                complete_prefix_boundary(&translation_aligned),
            ) {
                return Some((source_sentence, translation_sentence));
            }
            if self.reached_alignment_checkpoint() {
                return Some((source_aligned_length, translation_aligned_length));
            }
            return None;
        }

        if self.source.has_timing_metadata() && self.translation.has_timing_metadata() {
            return None;
        }
        let source_sentence_count = sentence_delimiter_count(&self.source.text);
        let translation_sentence_count = sentence_delimiter_count(&self.translation.text);
        if source_sentence_count > 0 && source_sentence_count == translation_sentence_count {
            return Some((
                complete_prefix_boundary(&self.source.text)?,
                complete_prefix_boundary(&self.translation.text)?,
            ));
        }
        self.bounded_fallback_lengths()
    }

    fn final_pair(&self, source: String, translation: String) -> Option<TranscriptEvent> {
        let source = source.trim().to_string();
        let translation = translation.trim().to_string();
        if !is_meaningful(&source) || !is_meaningful(&translation) {
            return None;
        }
        Some(TranscriptEvent::SubtitleFinalPair {
            source,
            language: self.source_language.clone(),
            translation,
        })
    }

    fn reached_alignment_checkpoint(&self) -> bool {
        self.source.character_count() >= self.maximum_pending_characters
            || self.translation.character_count() >= self.maximum_pending_characters
    }

    fn both_reached_alignment_checkpoint(&self) -> bool {
        self.source.character_count() >= self.maximum_pending_characters
            && self.translation.character_count() >= self.maximum_pending_characters
    }

    fn exceeded_safety_limit(&self) -> bool {
        self.source.retained_character_count() > self.maximum_buffer_characters
            || self.translation.retained_character_count() > self.maximum_buffer_characters
    }

    /// Timing metadata is optional in the official protocol. Unequal sentence
    /// segmentation may still be paired once both streams have accumulated a
    /// substantial, completed prefix. Requiring both checkpoint and boundary
    /// evidence avoids forging a durable pair from a long source and a tiny,
    /// lagging translation fragment.
    fn bounded_fallback_lengths(&self) -> Option<(usize, usize)> {
        if !self.both_reached_alignment_checkpoint()
            || !is_meaningful(&self.source.text)
            || !is_meaningful(&self.translation.text)
        {
            return None;
        }
        let source_length = complete_prefix_boundary(&self.source.text)?;
        let translation_length = complete_prefix_boundary(&self.translation.text)?;
        (source_length > 0 && translation_length > 0).then_some((source_length, translation_length))
    }
}

impl Default for OpenAITranscriptPairCommitter {
    fn default() -> Self {
        Self::new(320, None)
    }
}

fn char_prefix(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

fn complete_prefix_boundary(text: &str) -> Option<usize> {
    text.chars()
        .enumerate()
        .filter_map(|(index, character)| {
            SENTENCE_DELIMITERS
                .contains(&character)
                .then_some(index + 1)
        })
        .last()
}

fn sentence_delimiter_count(text: &str) -> usize {
    text.chars()
        .filter(|character| SENTENCE_DELIMITERS.contains(character))
        .count()
}

fn is_meaningful(text: &str) -> bool {
    text.chars()
        .any(|character| !character.is_whitespace() && character.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_keeps_split_translation_with_its_original_block() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        stream.append_source_delta("First sentence.", Some(0));
        let events = stream.append_translation_delta("第一部分。", Some(100));
        assert!(!events
            .iter()
            .any(|e| matches!(e, TranscriptEvent::SubtitleFinalPair { .. })));
        assert!(stream.settle(1_900).is_empty());
        stream.append_translation_delta("后半部分。", Some(1_000));
        stream.append_source_delta(" Next sentence.", Some(1_100));
        stream.append_translation_delta("下一句。", Some(1_900));
        assert!(stream.settle(3_899).is_empty());
        assert!(
            matches!(stream.settle(3_900).as_slice(), [TranscriptEvent::SubtitleFinalPair { source, translation, .. }] if source == "First sentence. Next sentence." && translation == "第一部分。后半部分。下一句。")
        );
        assert!(!stream.has_pending());
        // A genuine repetition belongs to a new block, not a deduplication filter.
        stream.append_source_delta("First sentence.", Some(4_000));
        stream.append_translation_delta("第一句。", Some(4_100));
        assert_eq!(stream.settle(6_100).len(), 1);
    }

    #[test]
    fn gemini_explicit_boundary_absorbs_late_unpunctuated_tail() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        stream.append_source_delta("Source ", Some(0));
        stream.append_translation_delta("翻訳", Some(50));
        stream.note_turn_complete(100);
        assert!(stream.settle(599).is_empty());
        stream.append_source_delta("tail", Some(300));
        stream.append_translation_delta("末尾", Some(350));
        assert!(stream.settle(849).is_empty());
        assert!(
            matches!(stream.settle(850).as_slice(), [TranscriptEvent::SubtitleFinalPair { source, translation, .. }] if source == "Source tail" && translation == "翻訳末尾")
        );
    }

    #[test]
    fn gemini_quiet_closes_paired_unpunctuated_text_but_retains_unmatched_text() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        stream.append_source_delta("Complete.", Some(0));
        assert!(stream.settle(5_000).is_empty());
        stream.append_translation_delta("尚未完成", Some(5_100));
        assert!(stream.settle(7_099).is_empty());
        assert!(matches!(stream.settle(7_100).as_slice(),
            [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
            if source == "Complete." && translation == "尚未完成"));
        assert!(!stream.has_pending());
        stream.append_translation_delta("Translation only", Some(8_000));
        assert!(stream.settle(10_000).is_empty());
        assert!(stream.has_pending());
    }

    #[test]
    fn gemini_merges_fragments_and_cumulative_extensions_in_independent_lanes() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        for (index, chunk) in ["我", "今天", "不想", "出去。"].iter().enumerate() {
            stream.append_source_delta(chunk, Some(index as u64));
        }
        for (index, (chunk, expected)) in [
            ("私", "私"),
            ("私は今日", "私は今日"),
            ("私は今日外に", "私は今日外に"),
            ("出たくない。", "私は今日外に出たくない。"),
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(
                stream.append_translation_delta(chunk, Some(index as u64)),
                vec![TranscriptEvent::TranslationDraft((*expected).into())]
            );
        }
        assert!(matches!(stream.settle(2_003).as_slice(),
            [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
            if source == "我今天不想出去。" && translation == "私は今日外に出たくない。"));
    }

    #[test]
    fn gemini_preserves_repetition_subwords_and_provider_whitespace() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        for chunk in ["trans", "lation", " is very", " very", " good", "."] {
            stream.append_source_delta(chunk, Some(0));
        }
        for chunk in ["哈", "哈", "！"] {
            stream.append_translation_delta(chunk, Some(0));
        }
        assert!(matches!(stream.finish().as_slice(),
            [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
            if source == "translation is very very good." && translation == "哈哈！"));
    }

    #[test]
    fn gemini_equal_cumulative_snapshots_do_not_postpone_quiet_or_cross_a_reset() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        stream.append_source_delta("Hello", Some(0));
        stream.append_source_delta("Hello world", Some(100));
        stream.append_translation_delta("你好", Some(0));
        stream.append_translation_delta("你好世界", Some(100));
        assert!(stream
            .append_source_delta("Hello world", Some(1_900))
            .is_empty());
        assert!(stream
            .append_translation_delta("你好世界", Some(2_000))
            .is_empty());
        assert!(stream.settle(2_099).is_empty());
        assert_eq!(stream.settle(2_100).len(), 1);
        // Local silence is not a provider boundary; a resend stays ignored.
        assert!(stream
            .append_translation_delta("你好世界", Some(3_000))
            .is_empty());
        stream.reset();
        assert_eq!(
            stream.append_translation_delta("你好世界", Some(4_000)),
            vec![TranscriptEvent::TranslationDraft("你好世界".into())]
        );
    }

    #[test]
    fn gemini_cumulative_extensions_after_quiet_checkpoint_only_confirm_new_text() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        stream.append_source_delta("Hello", Some(0));
        stream.append_source_delta("Hello world", Some(100));
        stream.append_translation_delta("你好", Some(0));
        stream.append_translation_delta("你好世界", Some(100));
        assert_eq!(stream.settle(2_100).len(), 1);
        let state = serde_json::to_string(&stream).unwrap();
        let mut stream: OpenAITranscriptPairCommitter = serde_json::from_str(&state).unwrap();
        stream.validate_state(320).unwrap();
        assert_eq!(
            stream.append_source_delta("Hello world again", Some(3_000)),
            vec![TranscriptEvent::SourceDraft {
                text: " again".into(),
                language: None
            }]
        );
        assert!(stream
            .append_translation_delta("你好世界", Some(3_100))
            .is_empty());
        assert_eq!(
            stream.append_translation_delta("你好世界再见", Some(3_200)),
            vec![TranscriptEvent::TranslationDraft("再见".into())]
        );
        assert!(matches!(stream.settle(5_200).as_slice(),
            [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
            if source == "again" && translation == "再见"));
        assert!(stream
            .append_source_delta("Hello world again", Some(5_300))
            .is_empty());
        assert!(stream
            .append_translation_delta("你好世界再见", Some(5_400))
            .is_empty());
        assert!(!stream.has_pending());
    }

    #[test]
    fn gemini_explicit_turn_keeps_late_cumulative_tail_then_clears_checkpoint() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        stream.append_source_delta("A", Some(0));
        stream.append_source_delta("AB", Some(100));
        stream.append_translation_delta("甲", Some(0));
        stream.append_translation_delta("甲乙", Some(100));
        assert_eq!(stream.settle(2_100).len(), 1);
        stream.note_turn_complete(2_200);
        stream.append_source_delta("ABC", Some(2_400));
        stream.append_translation_delta("甲乙丙", Some(2_400));
        assert!(stream.settle(2_899).is_empty());
        assert!(matches!(stream.settle(2_900).as_slice(),
            [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
            if source == "C" && translation == "丙"));
        assert_eq!(
            stream.append_translation_delta("甲乙丙", Some(3_000)),
            vec![TranscriptEvent::TranslationDraft("甲乙丙".into())]
        );
    }

    #[test]
    fn gemini_empty_explicit_turn_clears_checkpoint_and_preserves_new_repetition() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        stream.append_source_delta("A", Some(0));
        stream.append_source_delta("AB", Some(100));
        stream.append_translation_delta("甲", Some(0));
        stream.append_translation_delta("甲乙", Some(100));
        assert_eq!(stream.settle(2_100).len(), 1);
        stream.note_turn_complete(2_200);
        assert!(stream.settle(2_700).is_empty());
        assert_eq!(
            stream.append_source_delta("AB", Some(3_000)),
            vec![TranscriptEvent::SourceDraft {
                text: "AB".into(),
                language: None
            }]
        );
        assert_eq!(
            stream.append_translation_delta("甲乙", Some(3_000)),
            vec![TranscriptEvent::TranslationDraft("甲乙".into())]
        );
        assert_eq!(stream.finish().len(), 1);
    }

    #[test]
    fn gemini_checkpoint_preserves_independent_delta_and_cumulative_lanes() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        stream.append_source_delta("Hello", Some(0));
        stream.append_translation_delta("你", Some(0));
        stream.append_translation_delta("你好", Some(100));
        assert_eq!(stream.settle(2_100).len(), 1);
        stream.append_source_delta("Hello again", Some(3_000));
        stream.append_translation_delta("你好世界", Some(3_000));
        assert!(matches!(stream.finish().as_slice(),
            [TranscriptEvent::SubtitleFinalPair { source, translation, .. }]
            if source == "Hello again" && translation == "世界"));
    }

    #[test]
    fn gemini_checkpoint_and_pending_text_share_the_existing_safety_bound() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        stream.append_source_delta("a", Some(0));
        stream.append_source_delta(&"a".repeat(5_118), Some(100));
        stream.append_translation_delta("translation", Some(100));
        assert_eq!(stream.settle(2_100).len(), 1);
        stream.validate_state(320).unwrap();
        assert!(matches!(
            stream.append_source_delta("xyz", Some(3_000)).as_slice(),
            [TranscriptEvent::Error { .. }]
        ));
        assert!(!stream.has_pending());
        stream.validate_state(320).unwrap();
    }

    #[test]
    fn gemini_cumulative_state_roundtrips_without_changing_openai_delta_semantics() {
        let mut stream = OpenAITranscriptPairCommitter::new_gemini();
        stream.append_translation_delta("合成", Some(0));
        stream.append_translation_delta("合成文本", Some(1));
        let state = serde_json::to_string(&stream).unwrap();
        let mut restored: OpenAITranscriptPairCommitter = serde_json::from_str(&state).unwrap();
        restored.validate_state(320).unwrap();
        assert!(restored
            .append_translation_delta("合成文本", Some(2))
            .is_empty());
        assert_eq!(
            restored.append_translation_delta("后续", Some(3)),
            vec![TranscriptEvent::TranslationDraft("合成文本后续".into())]
        );
        let mut openai = OpenAITranscriptPairCommitter::default();
        openai.append_translation_delta("合成", None);
        assert_eq!(
            openai.append_translation_delta("合成文本", None),
            vec![TranscriptEvent::TranslationDraft("合成合成文本".into())]
        );
    }

    #[test]
    fn pending_status_tracks_unmatched_content_without_counting_whitespace() {
        let mut stream = OpenAITranscriptPairCommitter::default();
        assert!(!stream.has_pending());
        stream.append_source_delta("Hello.", None);
        assert!(stream.has_pending());
        stream.append_translation_delta("こんにちは。 ", None);
        assert!(!stream.has_pending());
        stream.append_source_delta("Unmatched", None);
        assert!(stream.has_pending());
        stream.reset();
        assert!(!stream.has_pending());
    }

    #[test]
    fn accumulates_append_only_drafts() {
        let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
        assert_eq!(
            committer.append_source_delta("Hello", None),
            vec![TranscriptEvent::SourceDraft {
                text: "Hello".into(),
                language: Some("en".into())
            }]
        );
        assert_eq!(
            committer.append_source_delta(" world", None),
            vec![TranscriptEvent::SourceDraft {
                text: "Hello world".into(),
                language: Some("en".into())
            }]
        );
    }

    #[test]
    fn finalizes_a_pair_as_one_atomic_event() {
        let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
        let _ = committer.append_source_delta("Hello world.", None);
        let events = committer.append_translation_delta("你好世界。", None);
        assert_eq!(
            events,
            vec![
                TranscriptEvent::TranslationDraft("你好世界。".into()),
                TranscriptEvent::SubtitleFinalPair {
                    source: "Hello world.".into(),
                    language: Some("en".into()),
                    translation: "你好世界。".into()
                }
            ]
        );
    }

    #[test]
    fn restores_unmatched_tails_after_a_pair() {
        let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
        let _ = committer.append_source_delta("First. Tail", None);
        let events = committer.append_translation_delta("第一句。尾巴", None);
        assert_eq!(events.len(), 4);
        assert!(matches!(
            &events[1],
            TranscriptEvent::SubtitleFinalPair { source, translation, .. }
                if source == "First." && translation == "第一句。"
        ));
        assert_eq!(committer.pending_source_character_count(), 5);
        assert_eq!(committer.pending_translation_character_count(), 2);
    }

    #[test]
    fn timing_checkpoint_can_align_unequal_punctuation() {
        let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
        let _ = committer.append_source_delta("A. B.", Some(1_200));
        let events = committer.append_translation_delta("合并译文。", Some(1_200));
        assert!(events.iter().any(|event| matches!(
            event,
            TranscriptEvent::SubtitleFinalPair { source, translation, .. }
                if source == "A. B." && translation == "合并译文。"
        )));
    }

    #[test]
    fn timed_source_deltas_commit_once_and_keep_the_unicode_tail_aligned() {
        let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
        let _ = committer.append_source_delta("Hello", Some(500));
        let _ = committer.append_source_delta(" world.", Some(1_000));
        let _ = committer.append_source_delta(" 次の文", Some(2_000));

        assert_eq!(
            committer.append_translation_delta("你好世界。", Some(1_000)),
            vec![
                TranscriptEvent::TranslationDraft("你好世界。".into()),
                TranscriptEvent::SubtitleFinalPair {
                    source: "Hello world.".into(),
                    language: Some("en".into()),
                    translation: "你好世界。".into(),
                },
                TranscriptEvent::SourceDraft {
                    text: " 次の文".into(),
                    language: Some("en".into()),
                },
            ]
        );

        let _ = committer.append_source_delta("。", Some(2_000));
        assert_eq!(
            committer.append_translation_delta("下一句。", Some(2_000)),
            vec![
                TranscriptEvent::TranslationDraft("下一句。".into()),
                TranscriptEvent::SubtitleFinalPair {
                    source: "次の文。".into(),
                    language: Some("en".into()),
                    translation: "下一句。".into(),
                },
            ]
        );
        assert!(committer.finish().is_empty());
    }

    #[test]
    fn timed_translation_deltas_commit_once_and_keep_the_unicode_tail_aligned() {
        let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
        let _ = committer.append_translation_delta("你好", Some(500));
        let _ = committer.append_translation_delta("世界。", Some(1_000));
        let _ = committer.append_translation_delta(" 后文", Some(2_000));

        assert_eq!(
            committer.append_source_delta("Hello world.", Some(1_000)),
            vec![
                TranscriptEvent::SourceDraft {
                    text: "Hello world.".into(),
                    language: Some("en".into()),
                },
                TranscriptEvent::SubtitleFinalPair {
                    source: "Hello world.".into(),
                    language: Some("en".into()),
                    translation: "你好世界。".into(),
                },
                TranscriptEvent::TranslationDraft(" 后文".into()),
            ]
        );

        let _ = committer.append_translation_delta("。", Some(2_000));
        assert_eq!(
            committer.append_source_delta("Next.", Some(2_000)),
            vec![
                TranscriptEvent::SourceDraft {
                    text: "Next.".into(),
                    language: Some("en".into()),
                },
                TranscriptEvent::SubtitleFinalPair {
                    source: "Next.".into(),
                    language: Some("en".into()),
                    translation: "后文。".into(),
                },
            ]
        );
        assert!(committer.finish().is_empty());
    }

    #[test]
    fn one_sided_growth_is_retained_until_the_counterpart_arrives() {
        let mut committer = OpenAITranscriptPairCommitter::new(8, None);
        let _ = committer.append_source_delta("abcdefgh", None);
        let events = committer.append_source_delta("i", None);
        assert!(events
            .iter()
            .all(|event| !matches!(event, TranscriptEvent::Error { .. })));
        assert_eq!(committer.pending_source_character_count(), 9);

        let paired = committer.append_translation_delta("译文。", None);
        assert!(paired
            .iter()
            .all(|event| !matches!(event, TranscriptEvent::SubtitleFinalPair { .. })));
        assert_eq!(
            committer.finish(),
            vec![TranscriptEvent::SubtitleFinalPair {
                source: "abcdefghi".into(),
                language: None,
                translation: "译文。".into(),
            }]
        );
    }

    #[test]
    fn paired_single_deltas_larger_than_the_alignment_checkpoint_remain_complete() {
        let mut committer = OpenAITranscriptPairCommitter::new(320, Some("en".into()));
        let source = "a".repeat(384);
        let translation = "译".repeat(384);

        let _ = committer.append_source_delta(&source, None);
        let events = committer.append_translation_delta(&translation, None);

        assert!(events
            .iter()
            .all(|event| !matches!(event, TranscriptEvent::SubtitleFinalPair { .. })));
        assert_eq!(
            committer.finish(),
            vec![TranscriptEvent::SubtitleFinalPair {
                source,
                language: Some("en".into()),
                translation,
            }]
        );
    }

    #[test]
    fn unpaired_growth_fails_closed_only_at_the_absolute_safety_limit() {
        let mut committer = OpenAITranscriptPairCommitter::new(8, None);
        let source = "a".repeat(committer.maximum_buffer_characters + 1);

        assert_eq!(
            committer.append_source_delta(&source, None),
            vec![TranscriptEvent::Error {
                code: "openai_transcript_safety_limit".into(),
                message:
                    "OpenAI Realtime Translation transcript alignment exceeded its safety limit."
                        .into(),
            }]
        );
        assert_eq!(committer.pending_source_character_count(), 0);
        assert_eq!(committer.pending_translation_character_count(), 0);
    }

    #[test]
    fn missing_timing_and_unequal_sentence_segmentation_commit_at_the_bound() {
        let mut committer = OpenAITranscriptPairCommitter::new(32, Some("en".into()));
        let _ = committer.append_source_delta(&"A. B. ".repeat(8), None);
        let events = committer.append_translation_delta(&"合并译文。".repeat(7), None);

        assert!(events
            .iter()
            .all(|event| !matches!(event, TranscriptEvent::Error { .. })));
        assert!(events.iter().any(|event| matches!(
            event,
            TranscriptEvent::SubtitleFinalPair { source, translation, .. }
                if source.contains("A. B.") && translation.contains("合并译文。")
        )));
        assert!(committer.pending_source_character_count() <= 32);
        assert!(committer.pending_translation_character_count() <= 32);
    }

    #[test]
    fn a_short_lagging_translation_cannot_finalize_a_long_source() {
        let mut committer = OpenAITranscriptPairCommitter::new(32, Some("en".into()));
        let source = format!("{}.", "a".repeat(40));
        let translation = format!("你{}。", "译".repeat(39));
        let _ = committer.append_source_delta(&source, None);

        let short = committer.append_translation_delta("你", None);
        assert!(short
            .iter()
            .all(|event| !matches!(event, TranscriptEvent::SubtitleFinalPair { .. })));

        let completed = committer.append_translation_delta(&format!("{}。", "译".repeat(39)), None);
        assert!(completed.iter().any(|event| matches!(
            event,
            TranscriptEvent::SubtitleFinalPair {
                source: committed_source,
                translation: committed_translation,
                ..
            } if committed_source == &source && committed_translation == &translation
        )));
    }

    #[test]
    fn graceful_close_flushes_only_a_meaningful_pair() {
        let mut committer = OpenAITranscriptPairCommitter::new(320, Some("ja".into()));
        let _ = committer.append_source_delta("こんにちは", None);
        let _ = committer.append_translation_delta("你好", None);
        assert_eq!(
            committer.finish(),
            vec![TranscriptEvent::SubtitleFinalPair {
                source: "こんにちは".into(),
                language: Some("ja".into()),
                translation: "你好".into()
            }]
        );

        let mut one_sided = OpenAITranscriptPairCommitter::default();
        let _ = one_sided.append_source_delta("private source tail", None);
        assert!(one_sided.finish().is_empty());
    }
}
