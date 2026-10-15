//! Experimental, revocable translation units. Only provider finals are durable.
//!
//! Punctuation is a conservative candidate signal, not semantic understanding.
//! State contains only a bounded current utterance and its replaceable pair.

use crate::core::models::subtitle_text_within_limit;

pub const UNIT_STABLE_MS: u64 = 600;
pub const UNIT_MAX_WAIT_MS: u64 = 2_000;

#[derive(Clone, Debug)]
pub struct UnitRequest {
    pub source: String,
    pub suffix: String,
    generation: u64,
    base_len: usize,
    pin: bool,
}

#[derive(Default)]
pub struct StablePreviewUnits {
    generation: u64,
    latest: String,
    candidate: String,
    candidate_since: u64,
    opened_at: Option<u64>,
    pinned_source: String,
    pinned_translation: String,
    displayed: Option<(String, String)>,
}

impl StablePreviewUnits {
    pub fn reset(&mut self) {
        let generation = self.generation.wrapping_add(1);
        *self = Self {
            generation,
            ..Self::default()
        };
    }

    /// Returns true when a previously published pair must be retracted.
    /// Empty ASR updates are not clear operations.
    pub fn observe(&mut self, text: &str, now_ms: u64) -> bool {
        let text = text.trim();
        if text.is_empty() || !subtitle_text_within_limit(text) {
            return false;
        }
        let retract = self
            .displayed
            .as_ref()
            .is_some_and(|(source, _)| !text.starts_with(source));
        if !text.starts_with(&self.latest) {
            // Even an A -> B -> A correction cannot resurrect an old response.
            self.generation = self.generation.wrapping_add(1);
        }
        if !text.starts_with(&self.pinned_source) {
            self.pinned_source.clear();
            self.pinned_translation.clear();
        }
        if retract {
            self.displayed = None;
        }
        self.latest = text.to_owned();
        self.opened_at.get_or_insert(now_ms);
        self.observe_boundary(now_ms);
        retract
    }

    fn observe_boundary(&mut self, now_ms: u64) {
        let prefix = complete_prefix(&self.latest);
        // A growing later sentence must not restart the dwell of an earlier
        // complete unit. Once accepted, the next boundary starts its own dwell.
        if self.candidate.len() > self.pinned_source.len() && prefix.starts_with(&self.candidate) {
            return;
        }
        if self.candidate != prefix {
            self.candidate = prefix.to_owned();
            self.candidate_since = now_ms;
        }
    }

    fn stable(&self, now_ms: u64) -> bool {
        self.candidate.len() > self.pinned_source.len()
            && now_ms.saturating_sub(self.candidate_since) >= UNIT_STABLE_MS
    }

    pub fn next(&mut self, now_ms: u64) -> Option<UnitRequest> {
        // A successful fallback can mature without paying for identical MT.
        if self.stable(now_ms) {
            if let Some((source, translation)) = &self.displayed {
                if source == &self.candidate {
                    self.pinned_source.clone_from(source);
                    self.pinned_translation.clone_from(translation);
                    self.observe_boundary(now_ms);
                }
            }
        }
        let source = if self.stable(now_ms) {
            &self.candidate
        } else if self
            .opened_at
            .is_some_and(|at| now_ms.saturating_sub(at) >= UNIT_MAX_WAIT_MS)
        {
            &self.latest
        } else {
            return None;
        };
        if self
            .displayed
            .as_ref()
            .is_some_and(|(previous, _)| previous == source)
        {
            return None;
        }
        self.request(source, now_ms)
    }

    pub fn request(&self, source: &str, now_ms: u64) -> Option<UnitRequest> {
        if !self.latest.starts_with(source) || !source.starts_with(&self.pinned_source) {
            return None;
        }
        let suffix = source[self.pinned_source.len()..].trim();
        if !suffix.chars().any(char::is_alphanumeric) {
            return None;
        }
        Some(UnitRequest {
            source: source.to_owned(),
            suffix: suffix.to_owned(),
            generation: self.generation,
            base_len: self.pinned_source.len(),
            pin: source == self.candidate && self.stable(now_ms),
        })
    }

    pub fn is_current(&self, request: &UnitRequest) -> bool {
        request.generation == self.generation
            && request.base_len == self.pinned_source.len()
            && request.source.starts_with(&self.pinned_source)
            && self.latest.starts_with(&request.source)
    }

    /// Accept only a nonempty, current completion. A fallback never becomes a
    /// durable subtitle; a later correction or authoritative final can replace it.
    pub fn complete(&mut self, request: &UnitRequest, output: &str, now_ms: u64) -> Option<String> {
        let output = output.trim();
        if output.is_empty() || !subtitle_text_within_limit(output) || !self.is_current(request) {
            return None;
        }
        let translation = join_translation(&self.pinned_translation, output);
        if !subtitle_text_within_limit(&translation) {
            return None;
        }
        self.displayed = Some((request.source.clone(), translation.clone()));
        if request.pin || (request.source == self.candidate && self.stable(now_ms)) {
            self.pinned_source.clone_from(&request.source);
            self.pinned_translation.clone_from(&translation);
            self.observe_boundary(now_ms);
        }
        Some(translation)
    }

    pub fn pinned_pair(&self) -> Option<(&str, &str)> {
        (!self.pinned_source.is_empty()).then_some((&self.pinned_source, &self.pinned_translation))
    }
}

fn join_translation(prefix: &str, tail: &str) -> String {
    if prefix.is_empty() {
        return tail.to_owned();
    }
    let cjk = |c: char| matches!(c as u32, 0x3000..=0x9fff | 0xac00..=0xd7af | 0xff00..=0xffef);
    let needs_space = prefix.chars().last().is_some_and(|c| !cjk(c))
        && tail.chars().next().is_some_and(|c| !cjk(c));
    format!("{prefix}{}{tail}", if needs_space { " " } else { "" })
}

/// Handles complete sentence punctuation, excluding decimal points, initials,
/// common abbreviations and ellipses. Deliberately excludes commas and pauses.
pub fn complete_prefix(text: &str) -> &str {
    let mut end = 0;
    for (index, character) in text.char_indices() {
        if !matches!(character, '.' | '?' | '!' | '。' | '？' | '！') {
            continue;
        }
        let after = &text[index + character.len_utf8()..];
        if character == '.' {
            let before = &text[..index];
            let word = before
                .rsplit(|c: char| !c.is_ascii_alphabetic())
                .next()
                .unwrap_or_default();
            if after
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '.')
                || before.ends_with('.')
                || (!word.is_empty()
                    && (word.len() <= 1
                        || matches!(
                            word.to_ascii_lowercase().as_str(),
                            "mr" | "mrs" | "ms" | "dr" | "prof" | "sr" | "jr" | "vs" | "etc"
                        )))
            {
                continue;
            }
        }
        end = index + character.len_utf8();
        for close in after
            .chars()
            .take_while(|c| matches!(c, '"' | '\'' | '”' | '’' | '」' | '』' | '）' | ')'))
        {
            end += close.len_utf8();
        }
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_contract_matches_direct_rust() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../../../translation-contracts.json")).unwrap();
        for case in contract["stableUnitPreviews"].as_array().unwrap() {
            let mut units = StablePreviewUnits::default();
            let mut rows = Vec::new();
            for step in case["steps"].as_array().unwrap() {
                let now = step["atMs"].as_u64().unwrap();
                if step["reset"].as_bool().unwrap_or(false) {
                    units.reset();
                }
                units.observe(step["draft"].as_str().unwrap(), now);
                let request = units.next(now);
                let translation = request.as_ref().and_then(|request| {
                    step["reply"]
                        .as_str()
                        .and_then(|reply| units.complete(request, reply, now))
                });
                rows.push(serde_json::json!({"source":request.as_ref().map(|r| &r.source), "suffix":request.as_ref().map(|r| &r.suffix),
                    "translation":translation,"pinnedSource":units.pinned_pair().map(|(source, _)| source)}));
            }
            assert_eq!(serde_json::json!(rows), case["expected"], "{}", case["id"]);
        }
    }

    #[test]
    fn stable_units_translate_only_new_tail_and_keep_the_prefix_exact() {
        let mut units = StablePreviewUnits::default();
        units.observe("Hello world. Later", 0);
        assert!(units.next(599).is_none());
        let first = units.next(600).unwrap();
        assert_eq!(first.suffix, "Hello world.");
        assert_eq!(
            units.complete(&first, "你好。", 620).as_deref(),
            Some("你好。")
        );
        units.observe("Hello world. Later sentence. Unfinished", 800);
        let second = units.next(1400).unwrap();
        assert_eq!(second.suffix, "Later sentence.");
        assert_eq!(
            units.complete(&second, "后一句。", 1500).as_deref(),
            Some("你好。后一句。")
        );
    }

    #[test]
    fn growing_tail_does_not_restart_complete_prefix_dwell() {
        let mut units = StablePreviewUnits::default();
        units.observe("One sentence. Tail", 0);
        units.observe("One sentence. Tail continues.", 500);
        assert_eq!(units.next(600).unwrap().source, "One sentence.");
    }

    #[test]
    fn corrections_revoke_display_and_reject_late_a_b_a_response() {
        let mut units = StablePreviewUnits::default();
        units.observe("I accept.", 0);
        let request = units.next(600).unwrap();
        units.complete(&request, "我接受。", 610).unwrap();
        units.observe("I accept. Another sentence.", 700);
        let stale = units.next(1300).unwrap();
        assert!(units.observe("I do not accept.", 1400));
        assert!(units.pinned_pair().is_none());
        units.observe("I accept. Another sentence.", 1450);
        assert!(units.complete(&stale, "迟到结果。", 1500).is_none());
    }

    #[test]
    fn incomplete_rescue_is_replaceable_and_empty_failure_cannot_pin() {
        let mut units = StablePreviewUnits::default();
        units.observe("これは行くとは限ら", 0);
        assert!(units.next(1999).is_none());
        let first = units.next(2000).unwrap();
        assert!(units.complete(&first, "  ", 2010).is_none());
        units.complete(&first, "未完成预览", 2020).unwrap();
        assert!(units.pinned_pair().is_none());
        assert!(!units.observe("これは行くとは限らない", 2100));
        let corrected = units.next(2100).unwrap();
        assert_eq!(corrected.suffix, "これは行くとは限らない");
    }

    #[test]
    fn successful_fallback_matures_without_an_identical_request() {
        let mut units = StablePreviewUnits::default();
        units.observe("First words", 0);
        units.observe("First words.", 1900);
        let fallback = units.next(2000).unwrap();
        units.complete(&fallback, "第一句。", 2100).unwrap();
        assert!(units.pinned_pair().is_none());
        assert!(units.next(2500).is_none());
        assert_eq!(units.pinned_pair(), Some(("First words.", "第一句。")));
    }

    #[test]
    fn reset_and_empty_or_oversized_inputs_cannot_corrupt_current_state() {
        let mut units = StablePreviewUnits::default();
        units.observe("A sentence.", 0);
        let request = units.next(600).unwrap();
        units.observe("", 650);
        units.observe(
            &"x".repeat(crate::core::models::MAX_SUBTITLE_TEXT_BYTES + 1),
            660,
        );
        assert!(units.complete(&request, "句子。", 700).is_some());
        units.reset();
        assert!(units.complete(&request, "迟到。", 800).is_none());
        assert!(units.next(9000).is_none());
    }

    #[test]
    fn punctuation_handles_decimal_abbreviation_ellipsis_and_cjk_quotes() {
        for text in [
            "Dr. Smith",
            "Value 3.14",
            "Wait...",
            "A. Smith",
            "etc. and more",
            "如果明天，",
        ] {
            assert_eq!(complete_prefix(text), "", "{text}");
        }
        assert_eq!(complete_prefix("「そうです。」まだ"), "「そうです。」");
        assert_eq!(complete_prefix("Hello world. Next"), "Hello world.");
        assert_eq!(join_translation("First.", "Second."), "First. Second.");
    }
}
