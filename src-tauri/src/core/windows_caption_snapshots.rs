//! Bounded reconciliation of rolling Windows UI Automation caption snapshots.
//! UIA exposes display text, not recognition finals. Sentence boundaries and
//! 1.2 seconds without a revision are explicitly heuristic commit points.

const MAX_SNAPSHOT_BYTES: usize = 24_576;
const MAX_PENDING_BYTES: usize = 24_576;
pub const SETTLE_MS: u64 = 1_200;

#[derive(Debug, PartialEq, Eq)]
pub struct CaptionUpdate {
    pub utterance_id: u64,
    pub text: String,
    pub is_final: bool,
}

#[derive(Default)]
pub struct CaptionSnapshots {
    window: Option<String>,
    consumed: usize,
    carried: String,
    draft: String,
    active_id: Option<u64>,
    next_id: u64,
    changed_at_ms: u64,
}

impl CaptionSnapshots {
    pub fn update(
        &mut self,
        snapshot: &str,
        now_ms: u64,
    ) -> Result<Vec<CaptionUpdate>, &'static str> {
        if snapshot.len() > MAX_SNAPSHOT_BYTES {
            return Err("windows_live_captions_snapshot_limit");
        }
        // Windows layout line breaks are not authoritative sentence boundaries.
        let next = snapshot.split_whitespace().collect::<Vec<_>>().join(" ");
        let Some(previous) = self.window.as_ref() else {
            self.consumed = next.len();
            self.window = Some(next);
            return Ok(Vec::new()); // Attach/start never replays historical captions.
        };
        if previous == &next {
            return Ok(self.tick(now_ms));
        }
        let previous = previous.clone();
        let mut updates = Vec::new();
        if next.is_empty() {
            self.commit_draft(&mut updates);
            self.consumed = 0;
            self.carried.clear();
        } else if next.starts_with(&previous) {
            // Append-only growth keeps the admitted boundary unchanged.
        } else {
            let prefix = common_prefix(&previous, &next);
            let overlap = suffix_prefix_overlap(&previous, &next);
            let rolled = overlap >= 8
                && overlap > prefix
                && (prefix < self.consumed || prefix * 2 < previous.len().min(next.len()));
            if rolled {
                let removed = previous.len() - overlap;
                if removed > self.consumed {
                    let removed_draft = &previous[self.consumed..removed];
                    if self.carried.len() + removed_draft.len() > MAX_PENDING_BYTES {
                        return Err("windows_live_captions_snapshot_limit");
                    }
                    self.carried.push_str(removed_draft);
                    self.consumed = 0;
                } else {
                    self.consumed -= removed;
                }
            } else if prefix >= self.consumed
                && (prefix > 0 || self.consumed == 0 && self.draft.is_empty())
            {
                // A correction in the current draft revises its existing ID.
            } else if (prefix + common_suffix(&previous[prefix..], &next[prefix..])) * 2
                >= previous.len().min(next.len())
            {
                // A correction before the committed boundary must not replay
                // already-admitted content. Map through its unchanged suffix.
                let suffix = common_suffix(&previous[prefix..], &next[prefix..]);
                let old_suffix = previous.len() - suffix;
                let new_suffix = next.len() - suffix;
                self.consumed = if self.consumed >= old_suffix {
                    new_suffix + self.consumed - old_suffix
                } else {
                    new_suffix
                };
                self.consumed = self.consumed.min(next.len());
            } else {
                // An unrelated replacement/reset closes the previous visible
                // utterance. It is never treated as a correction to a final.
                self.commit_draft(&mut updates);
                self.carried.clear();
                self.consumed = 0;
            }
        }
        self.window = Some(next);
        self.refresh_draft(now_ms)?;
        while let Some(boundary) = completed_sentence_prefix(&self.draft) {
            let text = self.draft[..boundary].trim().to_string();
            self.emit_final(text, &mut updates);
            self.consume(boundary);
            self.refresh_draft(now_ms)?;
        }
        if !self.draft.is_empty() {
            let utterance_id = self.id();
            updates.push(CaptionUpdate {
                utterance_id,
                text: self.draft.clone(),
                is_final: false,
            });
        }
        Ok(updates)
    }

    pub fn tick(&mut self, now_ms: u64) -> Vec<CaptionUpdate> {
        let mut updates = Vec::new();
        if !self.draft.is_empty() && now_ms.saturating_sub(self.changed_at_ms) >= SETTLE_MS {
            self.commit_draft(&mut updates);
        }
        updates
    }

    /// Clear keeps the current window as the baseline; the next newly appended
    /// text is retained even if Windows never emits the unchanged old snapshot.
    pub fn reset_baseline(&mut self) {
        self.clear();
        self.window = None;
        self.consumed = 0;
    }

    pub fn clear(&mut self) {
        self.consumed = self.window.as_ref().map_or(0, String::len);
        self.carried.clear();
        self.draft.clear();
        self.active_id = None;
    }

    pub fn finish(&mut self) -> Vec<CaptionUpdate> {
        let mut updates = Vec::new();
        self.commit_draft(&mut updates);
        updates
    }

    fn refresh_draft(&mut self, now_ms: u64) -> Result<(), &'static str> {
        let window = self.window.as_deref().unwrap_or_default();
        let current = format!("{}{}", self.carried, &window[self.consumed..]);
        if current.len() > MAX_PENDING_BYTES {
            return Err("windows_live_captions_snapshot_limit");
        }
        let leading = current.len() - current.trim_start().len();
        self.consume(leading);
        let current = current.trim().to_string();
        if current != self.draft {
            self.changed_at_ms = now_ms;
            self.draft = current;
        }
        Ok(())
    }

    fn consume(&mut self, bytes: usize) {
        if bytes <= self.carried.len() {
            self.carried.drain(..bytes);
        } else {
            self.consumed += bytes - self.carried.len();
            self.carried.clear();
        }
    }

    fn id(&mut self) -> u64 {
        *self.active_id.get_or_insert_with(|| {
            self.next_id += 1;
            self.next_id
        })
    }

    fn emit_final(&mut self, text: String, updates: &mut Vec<CaptionUpdate>) {
        if text.chars().any(|character| {
            !character.is_whitespace() && !matches!(character, '.' | '!' | '?' | '。' | '！' | '？')
        }) {
            let utterance_id = self.id();
            updates.push(CaptionUpdate {
                utterance_id,
                text,
                is_final: true,
            });
        }
        self.active_id = None;
    }

    fn commit_draft(&mut self, updates: &mut Vec<CaptionUpdate>) {
        let text = std::mem::take(&mut self.draft);
        self.emit_final(text, updates);
        self.carried.clear();
        self.consumed = self.window.as_ref().map_or(0, String::len);
    }
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum()
}

fn common_suffix(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum()
}

fn suffix_prefix_overlap(previous: &str, next: &str) -> usize {
    // Linear matching keeps highly repetitive rolling text from making every
    // UIA update perform a quadratic scan. Both inputs are valid UTF-8; a
    // matching prefix/suffix ends on a character boundary in both strings.
    let pattern = next.as_bytes();
    if pattern.is_empty() {
        return 0;
    }
    let mut fallback = vec![0; pattern.len()];
    for index in 1..pattern.len() {
        let mut length = fallback[index - 1];
        while length > 0 && pattern[index] != pattern[length] {
            length = fallback[length - 1];
        }
        if pattern[index] == pattern[length] {
            length += 1;
        }
        fallback[index] = length;
    }
    let mut length = 0;
    for byte in previous.bytes() {
        while length > 0 && (length == pattern.len() || pattern[length] != byte) {
            length = fallback[length - 1];
        }
        if pattern[length] == byte {
            length += 1;
        }
    }
    length
}

fn completed_sentence_prefix(text: &str) -> Option<usize> {
    for (index, character) in text.char_indices() {
        if !matches!(character, '.' | '!' | '?' | '。' | '！' | '？') {
            continue;
        }
        let end = index + character.len_utf8();
        let tail = &text[end..];
        if tail.trim().is_empty() {
            return None;
        }
        if character == '.' {
            // Decimal points and common initials/titles are not sentence ends.
            let word = text[..index].split_whitespace().last().unwrap_or_default();
            if tail.starts_with(|c: char| c.is_ascii_digit())
                || matches!(
                    word,
                    "Mr" | "Mrs" | "Ms" | "Dr" | "Prof" | "Sr" | "Jr" | "St" | "vs" | "etc"
                )
                || word.len() == 1 && word.as_bytes()[0].is_ascii_alphabetic()
                || !tail.starts_with(char::is_whitespace)
            {
                continue;
            }
        }
        return Some(end);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(events: &[CaptionUpdate]) -> Vec<(&str, bool)> {
        events
            .iter()
            .map(|event| (event.text.as_str(), event.is_final))
            .collect()
    }

    #[test]
    fn overlap_is_bounded_and_unicode_safe_with_repeated_text() {
        assert_eq!(
            suffix_prefix_overlap("Synthetic 字幕猫", "字幕猫 continues"),
            "字幕猫".len()
        );
        assert_eq!(suffix_prefix_overlap("aaaaab", "aaaaac"), 0);
        assert_eq!(suffix_prefix_overlap("", "caption"), 0);
        assert_eq!(suffix_prefix_overlap("caption", ""), 0);
        let repeated = "a".repeat(MAX_SNAPSHOT_BYTES);
        assert_eq!(suffix_prefix_overlap(&repeated, &repeated), repeated.len());
        let mut state = CaptionSnapshots::default();
        assert!(state.update(&"字".repeat(8_192), 0).is_ok());
    }

    #[test]
    fn baseline_reset_keeps_ids_monotonic_and_committed_correction_does_not_replay() {
        let mut state = CaptionSnapshots::default();
        state.update("", 0).unwrap();
        let first = state.update("Synthetic old sentence.", 1).unwrap();
        state.tick(1_201);
        assert!(state
            .update("Synthetic OLD sentence.", 1_300)
            .unwrap()
            .is_empty());
        state.reset_baseline();
        assert!(state
            .update("Visible text at clear", 1_400)
            .unwrap()
            .is_empty());
        let next = state
            .update("Visible text at clear New content", 1_500)
            .unwrap();
        assert_eq!(text(&next), [("New content", false)]);
        assert!(next[0].utterance_id > first[0].utterance_id);
    }

    #[test]
    fn baseline_growth_revision_and_stability_use_one_draft_identity() {
        let mut state = CaptionSnapshots::default();
        assert!(state.update("Historical caption.", 0).unwrap().is_empty());
        let first = state
            .update("Historical caption. A synthetic cat", 10)
            .unwrap();
        assert_eq!(text(&first), [("A synthetic cat", false)]);
        let revision = state
            .update("Historical caption. A synthetic cap", 900)
            .unwrap();
        assert_eq!(first[0].utterance_id, revision[0].utterance_id);
        assert!(state.tick(2_099).is_empty());
        assert_eq!(text(&state.tick(2_100)), [("A synthetic cap", true)]);
        assert!(state
            .update("Historical caption. A synthetic cap", 4_000)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn rollover_keeps_unsettled_prefix_and_removes_committed_history() {
        let mut state = CaptionSnapshots::default();
        state.update("Old historical sentence.", 0).unwrap();
        state
            .update("Old historical sentence. Synthetic rolling text", 1)
            .unwrap();
        let rolled = state.update("Synthetic rolling text grows", 20).unwrap();
        assert_eq!(text(&rolled), [("Synthetic rolling text grows", false)]);
        let rolled_again = state.update("rolling text grows further", 30).unwrap();
        assert_eq!(
            text(&rolled_again),
            [("Synthetic rolling text grows further", false)]
        );
        assert_eq!(
            text(&state.tick(1_230)),
            [("Synthetic rolling text grows further", true)]
        );
        assert_eq!(
            text(&state.update("grows further. Next sentence", 1_240).unwrap()),
            [("Next sentence", false)]
        );
    }

    #[test]
    fn multiple_short_sentences_are_not_dropped_and_repetitions_have_new_ids() {
        let mut state = CaptionSnapshots::default();
        state.update("", 0).unwrap();
        let events = state.update("Yes. Yes. Synthetic tail", 1).unwrap();
        assert_eq!(
            text(&events),
            [("Yes.", true), ("Yes.", true), ("Synthetic tail", false)]
        );
        assert!(events
            .windows(2)
            .all(|pair| pair[0].utterance_id < pair[1].utterance_id));
    }

    #[test]
    fn unicode_newlines_empty_reset_and_clear_do_not_replay_old_text() {
        let mut state = CaptionSnapshots::default();
        state.update("历史字幕。", 0).unwrap();
        assert_eq!(
            text(&state.update("历史字幕。 新的\n测试字幕", 1).unwrap()),
            [("新的 测试字幕", false)]
        );
        state.clear();
        assert!(state.tick(9_000).is_empty());
        assert_eq!(
            text(&state.update("历史字幕。 新的\n测试字幕 增量", 10).unwrap()),
            [("增量", false)]
        );
        assert_eq!(text(&state.update("", 20).unwrap()), [("增量", true)]);
        assert!(state.update("", 30).unwrap().is_empty());
        assert_eq!(
            text(&state.update("全新字幕", 40).unwrap()),
            [("全新字幕", false)]
        );
    }

    #[test]
    fn punctuation_is_heuristic_and_size_failure_is_explicit() {
        let mut state = CaptionSnapshots::default();
        state.update("", 0).unwrap();
        assert_eq!(
            text(&state.update("Dr. Example has 3.14 units. Next", 1).unwrap()),
            [("Dr. Example has 3.14 units.", true), ("Next", false)]
        );
        assert_eq!(
            state.update(&"字".repeat(MAX_SNAPSHOT_BYTES), 2),
            Err("windows_live_captions_snapshot_limit")
        );
    }
}
