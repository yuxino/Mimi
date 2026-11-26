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
const UNPAIRED_BLOCK_IDLE_MS: u64 = 20_000;
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
    #[serde(default)]
    source_isolated: bool,
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
            && (!self.source_isolated
                || (!self.stable_block_mode
                    && self.source.text.is_empty()
                    && self.translation.character_count() <= self.maximum_pending_characters))
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
            source_isolated: false,
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

    fn retire_unpaired_block_before_append(&mut self, now_ms: Option<u64>) -> Vec<TranscriptEvent> {
        if !self.stable_block_mode
            || !(is_meaningful(&self.source.text) ^ is_meaningful(&self.translation.text))
            || now_ms
                .zip(self.last_delta_ms)
                .is_none_or(|(now, last)| now.saturating_sub(last) < UNPAIRED_BLOCK_IDLE_MS)
        {
            return Vec::new();
        }
        // A long-idle single lane has no bilingual confirmation. Keep it
        // visible during the wait, but never prepend it to a later clip.
        self.reset();
        vec![
            TranscriptEvent::SourceDraft {
                text: String::new(),
                language: self.source_language.clone(),
            },
            TranscriptEvent::TranslationDraft(String::new()),
        ]
    }

    pub fn append_source_delta(
        &mut self,
        delta: &str,
        elapsed_ms: Option<u64>,
    ) -> Vec<TranscriptEvent> {
        if delta.is_empty() || (self.source_isolated && delta.trim().is_empty()) {
            return Vec::new();
        }
        let mut retired = self.retire_unpaired_block_before_append(elapsed_ms);
        if self.source_isolated {
            // The returning original cannot own translation accumulated while
            // originals were absent. Retire that preview before starting anew.
            self.reset();
            retired.push(TranscriptEvent::TranslationDraft(String::new()));
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
        retired.extend(self.events_after_append(preview));
        retired
    }

    pub fn append_translation_delta(
        &mut self,
        delta: &str,
        elapsed_ms: Option<u64>,
    ) -> Vec<TranscriptEvent> {
        if delta.is_empty() {
            return Vec::new();
        }
        let mut retired = self.retire_unpaired_block_before_append(elapsed_ms);
        if self.source_isolated {
            self.translation.append(delta, elapsed_ms);
            self.bound_isolated_translation();
            return vec![TranscriptEvent::TranslationDraft(
                self.translation.text.clone(),
            )];
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
        retired.extend(self.events_after_append(preview));
        retired
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

    /// Optional source transcription can omit music while translation continues.
    /// Hide its stale original without terminating usable translated output or
    /// promoting an unpaired preview into confirmed history.
    pub fn isolate_source(&mut self) -> Vec<TranscriptEvent> {
        if self.stable_block_mode || self.source_isolated {
            return Vec::new();
        }
        self.source.reset();
        self.source_isolated = true;
        self.bound_isolated_translation();
        vec![TranscriptEvent::SourceDraft {
            text: String::new(),
            language: self.source_language.clone(),
        }]
    }

    fn bound_isolated_translation(&mut self) {
        let excess = self
            .translation
            .character_count()
            .saturating_sub(self.maximum_pending_characters);
        if excess > 0 {
            self.translation.consume(excess);
        }
    }

    pub fn reset(&mut self) {
        self.source.reset();
        self.translation.reset();
        self.last_delta_ms = None;
        self.explicit_turn = false;
        self.source_isolated = false;
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
            // The two prefixes cover one interval. Cutting each at its own
            // last punctuation can permanently pair source sentence N + 1
            // with translation N. Keep the complete interval together.
            if ends_at_sentence_boundary(&source_aligned)
                && ends_at_sentence_boundary(&translation_aligned)
            {
                return Some((source_aligned_length, translation_aligned_length));
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
        if source_sentence_count > 0
            && source_sentence_count == translation_sentence_count
            && ends_at_sentence_boundary(&self.source.text)
            && ends_at_sentence_boundary(&self.translation.text)
        {
            return Some((
                self.source.character_count(),
                self.translation.character_count(),
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
        (ends_at_sentence_boundary(&self.source.text)
            && ends_at_sentence_boundary(&self.translation.text))
        .then(|| {
            (
                self.source.character_count(),
                self.translation.character_count(),
            )
        })
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

fn ends_at_sentence_boundary(text: &str) -> bool {
    text.trim_end_matches(|character: char| {
        (character.is_whitespace() && character != '\n')
            || matches!(character, '"' | '\'' | '”' | '’' | '」' | '』' | ')' | '）')
    })
    .chars()
    .next_back()
    .is_some_and(|character| SENTENCE_DELIMITERS.contains(&character))
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
mod tests;
