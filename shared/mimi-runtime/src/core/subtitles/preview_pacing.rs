//! Budgets replaceable MT work by actual HTTP start, input size and ASR change.
//! Only fixed-size fingerprints and timing metadata are retained here.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

pub const MIN_PREVIEW_REQUEST_SPACING: Duration = Duration::from_millis(1_500);
const MAX_PREVIEW_REQUEST_SPACING: Duration = Duration::from_secs(8);
const REVISION_REFRESH_SPACING: Duration = Duration::from_secs(5);
pub const RATE_LIMIT_PREVIEW_PAUSE: Duration = Duration::from_secs(30);
const MIN_SHARED_REQUEST_SPACING: Duration = Duration::from_millis(1_100);
const REQUEST_OVERHEAD_UNITS: usize = 512;
const SHARED_UNITS_PER_SECOND: usize = 450;

/// A local work estimate, not a tokenizer or a claim about provider quotas.
/// Non-ASCII input is charged per character; ASCII input per four characters.
pub fn estimated_token_units(text: &str) -> usize {
    let mut ascii = 0usize;
    let mut non_ascii = 0usize;
    for character in text.chars() {
        if character.is_ascii() {
            ascii = ascii.saturating_add(1);
        } else {
            non_ascii = non_ascii.saturating_add(1);
        }
    }
    ascii.div_ceil(4).saturating_add(non_ascii)
}

#[derive(Clone, Copy)]
pub struct PreviewCandidate {
    fingerprint: u64,
    meaningful_chars: usize,
    input_units: usize,
}

impl PreviewCandidate {
    pub fn new(text: &str, language: Option<&str>, context_units: usize) -> Self {
        let mut hasher = DefaultHasher::new();
        language.hash(&mut hasher);
        let mut meaningful_chars = 0usize;
        // Ignore presentation punctuation/spacing churn, while retaining
        // numeric signs/separators and word connectors that change the input.
        let mut previous_ascii_word = false;
        let mut word_boundary = false;
        let mut previous_character = None;
        let mut characters = text.chars().peekable();
        while let Some(character) = characters.next() {
            if character.is_alphanumeric() {
                // Preserve ASCII word boundaries: "now here" and "nowhere"
                // are different content, unlike repeated spaces or commas.
                if previous_ascii_word && word_boundary && character.is_ascii() {
                    '\0'.hash(&mut hasher);
                }
                character.hash(&mut hasher);
                meaningful_chars = meaningful_chars.saturating_add(1);
                previous_ascii_word = character.is_ascii();
                word_boundary = false;
            } else if character.is_whitespace() {
                word_boundary = true;
            } else if let Some(symbol) = preview_identity_symbol(
                character,
                previous_character,
                characters.peek().copied(),
                characters.clone().nth(1),
            ) {
                symbol.hash(&mut hasher);
            }
            previous_character = Some(character);
        }
        Self {
            fingerprint: hasher.finish(),
            meaningful_chars,
            // Include a conservative output allowance as well as MT memory.
            input_units: estimated_token_units(text)
                .saturating_mul(2)
                .saturating_add(context_units)
                .saturating_add(REQUEST_OVERHEAD_UNITS),
        }
    }

    pub fn matches_content(self, other: Self) -> bool {
        self.fingerprint == other.fingerprint
    }

    fn request_spacing(self) -> Duration {
        self.shared_spacing()
            .max(MIN_PREVIEW_REQUEST_SPACING)
            .min(MAX_PREVIEW_REQUEST_SPACING)
    }

    fn shared_spacing(self) -> Duration {
        let milliseconds = self
            .input_units
            .saturating_mul(1_000)
            .div_ceil(SHARED_UNITS_PER_SECOND);
        Duration::from_millis(milliseconds as u64).max(MIN_SHARED_REQUEST_SPACING)
    }
}

fn preview_identity_symbol(
    character: char,
    previous: Option<char>,
    next: Option<char>,
    after_next: Option<char>,
) -> Option<char> {
    let character = match character {
        '．' => '.',
        '，' => ',',
        '：' => ':',
        '／' => '/',
        '＋' => '+',
        '－' => '-',
        '％' => '%',
        character => character,
    };
    let numeric_before = previous.is_some_and(char::is_numeric);
    let numeric_after = next.is_some_and(char::is_numeric);
    let fractional_after =
        matches!(next, Some('.' | ',' | '．' | '，')) && after_next.is_some_and(char::is_numeric);
    let inside_word =
        previous.is_some_and(char::is_alphanumeric) && next.is_some_and(char::is_alphanumeric);
    match character {
        // A leading decimal has no preceding digit; a sentence-ending dot
        // still has no following digit and remains presentation punctuation.
        '.' | ',' if numeric_after => Some(character),
        ':' | '/' if numeric_before && numeric_after => Some(character),
        '+' if numeric_after || fractional_after => Some('+'),
        '-' | '−' | '‐' | '‑' if numeric_after || fractional_after => Some('-'),
        '%' if numeric_before => Some('%'),
        '\'' | '’' if inside_word => Some('\''),
        '-' | '‐' | '‑' if inside_word => Some('-'),
        _ => None,
    }
}

pub struct PreviewRequestPacer {
    last_started_at: Option<Instant>,
    last_spacing: Duration,
    last_candidate: Option<PreviewCandidate>,
    suppressed_until: Option<Instant>,
    last_http_started_at: Option<Instant>,
    last_http_spacing: Duration,
    shared_cooldown_until: Option<Instant>,
}

/// Transfers only content-free request accounting across pause/reconnect.
/// No draft fingerprints or translation memory leave the old client.
#[derive(Clone, Copy, Debug)]
pub struct MTRequestBudget {
    last_started_at: Option<Instant>,
    last_spacing: Duration,
    suppressed_until: Option<Instant>,
    last_http_started_at: Option<Instant>,
    last_http_spacing: Duration,
    shared_cooldown_until: Option<Instant>,
}

impl MTRequestBudget {
    pub fn shared_cooldown_until(self) -> Option<Instant> {
        self.shared_cooldown_until
    }

    pub fn is_preview_suppressed(self, now: Instant) -> bool {
        self.suppressed_until.is_some_and(|until| until > now)
    }
}

impl Default for PreviewRequestPacer {
    fn default() -> Self {
        Self {
            last_started_at: None,
            last_spacing: MIN_PREVIEW_REQUEST_SPACING,
            last_candidate: None,
            suppressed_until: None,
            last_http_started_at: None,
            last_http_spacing: MIN_SHARED_REQUEST_SPACING,
            shared_cooldown_until: None,
        }
    }
}

impl PreviewRequestPacer {
    /// Only the current utterance's consumed preview candidate, never text.
    pub fn has_previewed_utterance(&self) -> bool {
        self.last_candidate.is_some()
    }

    pub fn next_start_at(&self, now: Instant) -> Instant {
        let paced = self
            .last_started_at
            .map_or(now, |previous| (previous + self.last_spacing).max(now));
        let paced = self.next_shared_start_at(now).max(paced);
        self.suppressed_until
            .map_or(paced, |until| until.max(paced))
    }

    pub fn next_shared_start_at(&self, now: Instant) -> Instant {
        let paced = self
            .last_http_started_at
            .map_or(now, |previous| (previous + self.last_http_spacing).max(now));
        self.shared_cooldown_until
            .map_or(paced, |until| until.max(paced))
    }

    pub fn has_changed(&self, candidate: PreviewCandidate) -> bool {
        candidate.meaningful_chars > 0
            && self
                .last_candidate
                .is_none_or(|previous| previous.fingerprint != candidate.fingerprint)
    }

    pub fn next_candidate_start_at(
        &self,
        now: Instant,
        candidate: PreviewCandidate,
    ) -> Option<Instant> {
        if !self.has_changed(candidate) {
            return None;
        }
        let mut next = self.next_start_at(now);
        if let Some(previous_start) = self.last_started_at {
            next = next.max(previous_start + candidate.request_spacing());
            if let Some(previous) = self.last_candidate {
                let growth = (previous.meaningful_chars / 8).clamp(4, 24);
                if candidate.meaningful_chars < previous.meaningful_chars.saturating_add(growth) {
                    // Real corrections remain eligible, but equal-length churn
                    // cannot consume a request on every 250 ms stable timer.
                    next = next.max(previous_start + REVISION_REFRESH_SPACING);
                }
            }
        }
        Some(next)
    }

    pub fn record_start(&mut self, now: Instant) {
        self.last_started_at = Some(now);
        self.last_http_started_at = Some(now);
    }

    pub fn record_candidate_start(&mut self, now: Instant, candidate: PreviewCandidate) {
        self.record_start(now);
        self.last_spacing = candidate.request_spacing();
        self.last_candidate = Some(candidate);
        self.last_http_spacing = candidate.shared_spacing();
    }

    pub fn record_final_start(&mut self, now: Instant, candidate: PreviewCandidate) {
        self.last_http_started_at = Some(now);
        self.last_http_spacing = candidate.shared_spacing();
    }

    pub fn suppress_after_rate_limit(&mut self, now: Instant) {
        let until = now + RATE_LIMIT_PREVIEW_PAUSE;
        self.suppressed_until = Some(self.suppressed_until.map_or(until, |old| old.max(until)));
    }

    pub fn set_shared_cooldown(&mut self, until: Option<Instant>) {
        self.shared_cooldown_until = until;
    }

    pub fn export_budget(&self) -> MTRequestBudget {
        MTRequestBudget {
            last_started_at: self.last_started_at,
            last_spacing: self.last_spacing,
            suppressed_until: self.suppressed_until,
            last_http_started_at: self.last_http_started_at,
            last_http_spacing: self.last_http_spacing,
            shared_cooldown_until: self.shared_cooldown_until,
        }
    }

    pub fn restore_budget(&mut self, budget: MTRequestBudget) {
        self.last_started_at = budget.last_started_at;
        self.last_spacing = budget.last_spacing;
        self.suppressed_until = budget.suppressed_until;
        self.last_http_started_at = budget.last_http_started_at;
        self.last_http_spacing = budget.last_http_spacing;
        self.shared_cooldown_until = budget.shared_cooldown_until;
        self.last_candidate = None;
    }

    pub fn suppression_remaining(&self, now: Instant) -> Duration {
        self.suppressed_until
            .map_or(Duration::ZERO, |until| until.saturating_duration_since(now))
    }

    /// A durable server boundary permits the next utterance, but cannot refund
    /// HTTP work or the independent rate-limit pause.
    pub fn finish_utterance(&mut self) {
        self.last_candidate = None;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_request_is_immediate_and_cancellation_does_not_refund_its_slot() {
        let now = Instant::now();
        let mut pacer = PreviewRequestPacer::default();
        assert_eq!(pacer.next_start_at(now), now);
        pacer.record_start(now);
        // A new owner uses the same pacer after replacing the old draft.
        assert_eq!(
            pacer.next_start_at(now + Duration::from_millis(250)),
            now + MIN_PREVIEW_REQUEST_SPACING
        );
        let next = now + MIN_PREVIEW_REQUEST_SPACING;
        pacer.record_start(next);
        assert_eq!(
            pacer.next_start_at(next),
            next + MIN_PREVIEW_REQUEST_SPACING
        );
        let later = next + Duration::from_secs(2);
        assert_eq!(pacer.next_start_at(later), later);
    }

    #[test]
    fn long_cumulative_input_and_context_cost_more_without_unbounded_metadata() {
        let now = Instant::now();
        let short = PreviewCandidate::new("短句", Some("zh"), 0);
        let long = PreviewCandidate::new(&"字".repeat(2_000), Some("zh"), 100);
        let mut pacer = PreviewRequestPacer::default();
        assert_eq!(pacer.next_candidate_start_at(now, long), Some(now));
        pacer.record_candidate_start(now, short);
        assert_eq!(
            pacer.next_candidate_start_at(now, long),
            Some(now + Duration::from_secs(8))
        );
        pacer.record_candidate_start(now, long);
        assert!(pacer.has_previewed_utterance());
        pacer.finish_utterance();
        assert!(!pacer.has_previewed_utterance());
        assert!(pacer.next_candidate_start_at(now, short).unwrap() > now + Duration::from_secs(10));
        assert!(std::mem::size_of::<PreviewRequestPacer>() < 192);
        assert_eq!(estimated_token_units("abcd四字"), 3);
    }

    #[test]
    fn punctuation_churn_is_skipped_and_real_same_length_corrections_are_bounded() {
        let now = Instant::now();
        let original = PreviewCandidate::new("Hello world", Some("en"), 0);
        let mut pacer = PreviewRequestPacer::default();
        pacer.record_candidate_start(now, original);
        assert_eq!(
            pacer.next_candidate_start_at(
                now + Duration::from_secs(20),
                PreviewCandidate::new("Hello, world!", Some("en"), 0)
            ),
            None
        );
        let correction = PreviewCandidate::new("Hello there", Some("en"), 0);
        assert!(original.matches_content(PreviewCandidate::new(
            "Hello, world!",
            Some("en"),
            1_000,
        )));
        assert!(!original.matches_content(correction));
        assert!(!original.matches_content(PreviewCandidate::new("Hello world", Some("ja"), 0,)));
        assert_eq!(
            pacer.next_candidate_start_at(now, correction),
            Some(now + REVISION_REFRESH_SPACING)
        );
        let growth = PreviewCandidate::new("Hello world today", Some("en"), 0);
        assert!(
            pacer.next_candidate_start_at(now, growth).unwrap() < now + REVISION_REFRESH_SPACING
        );
        pacer.finish_utterance();
        assert!(
            pacer.has_changed(original),
            "a repeated spoken line after a final is new work"
        );
    }

    #[test]
    fn semantic_number_and_word_punctuation_remains_eligible_for_a_bounded_revision() {
        let now = Instant::now();
        for (original, correction) in [
            ("Use 1.5 liters", "Use 15 liters"),
            ("Use 1,5 liters", "Use 15 liters"),
            ("Use .5 liters", "Use 5 liters"),
            ("Use ,5 liters", "Use 5 liters"),
            ("Set -5 degrees", "Set 5 degrees"),
            ("Set +5 degrees", "Set 5 degrees"),
            ("Set -.5 degrees", "Set .5 degrees"),
            ("Set +.5 degrees", "Set .5 degrees"),
            ("Set -,5 degrees", "Set ,5 degrees"),
            ("Set +,5 degrees", "Set ,5 degrees"),
            ("Mix 1:5", "Mix 15"),
            ("Use 1/5 cup", "Use 15 cup"),
            ("Set 50% power", "Set 50 power"),
            ("I can't stay", "I cant stay"),
            ("Please re-sign here", "Please resign here"),
        ] {
            for (original, correction) in [(original, correction), (correction, original)] {
                let original = PreviewCandidate::new(original, Some("en"), 0);
                let correction = PreviewCandidate::new(correction, Some("en"), 0);
                assert!(
                    !original.matches_content(correction),
                    "A meaning-changing separator must not be cosmetic"
                );
                let mut pacer = PreviewRequestPacer::default();
                pacer.record_candidate_start(now, original);
                assert_eq!(
                    pacer.next_candidate_start_at(now, correction),
                    Some(now + REVISION_REFRESH_SPACING),
                    "A semantic correction keeps the existing revision cooldown"
                );
            }
        }
    }

    #[test]
    fn cosmetic_punctuation_and_connector_glyphs_do_not_retranslate_a_number() {
        for (original, cosmetic) in [
            ("Use 1.5 liters", "Use 1.5 liters!"),
            ("Use 1,5 liters", "Use 1,5   liters."),
            ("Set 5", "Set 5."),
            ("Use .5 liters", "Use ．5 liters."),
            ("Use ,5 liters", "Use ，5 liters."),
            ("Set -.5 degrees", "Set −．5 degrees!"),
            ("Set +,5 degrees", "Set ＋，5 degrees!"),
            ("Set -5 degrees", "Set －5 degrees!"),
            ("Use 1.5 liters", "Use 1．5 liters!"),
            ("Use 1,5 liters", "Use 1，5 liters!"),
            ("Mix 1:5", "Mix 1：5."),
            ("Use 1/5 cup", "Use 1／5 cup."),
            ("Set 50% power", "Set 50％ power."),
            (
                "I can't re-sign at -5 degrees",
                "I can’t re‐sign at −5 degrees!",
            ),
        ] {
            assert!(PreviewCandidate::new(original, Some("en"), 0)
                .matches_content(PreviewCandidate::new(cosmetic, Some("en"), 0)));
        }
    }

    #[test]
    fn rate_limit_pause_survives_draft_and_utterance_replacement_until_session_reset() {
        let now = Instant::now();
        let mut pacer = PreviewRequestPacer::default();
        pacer.suppress_after_rate_limit(now);
        pacer.finish_utterance();
        assert_eq!(pacer.next_start_at(now), now + RATE_LIMIT_PREVIEW_PAUSE);
        assert_eq!(
            pacer.suppression_remaining(now + Duration::from_secs(4)),
            Duration::from_secs(26)
        );
        pacer.suppress_after_rate_limit(now + Duration::from_secs(3));
        assert_eq!(pacer.suppression_remaining(now), Duration::from_secs(33));
        pacer.reset();
        assert_eq!(pacer.next_start_at(now), now);
        assert_eq!(pacer.suppression_remaining(now), Duration::ZERO);
    }

    #[test]
    fn finals_skip_preview_revision_wait_but_share_http_floor_and_input_budget() {
        let now = Instant::now();
        let mut pacer = PreviewRequestPacer::default();
        let preview = PreviewCandidate::new("Hello world", Some("en"), 0);
        pacer.record_candidate_start(now, preview);
        let final_at = pacer.next_shared_start_at(now);
        assert!(final_at >= now + MIN_SHARED_REQUEST_SPACING);
        assert!(final_at < now + MIN_PREVIEW_REQUEST_SPACING);
        let long_final = PreviewCandidate::new(&"字".repeat(1_000), Some("zh"), 1_000);
        pacer.record_final_start(final_at, long_final);
        assert!(pacer.next_shared_start_at(final_at) > final_at + Duration::from_secs(7));
        pacer.suppress_after_rate_limit(final_at);
        assert!(pacer.next_shared_start_at(final_at) < final_at + RATE_LIMIT_PREVIEW_PAUSE);
    }

    #[test]
    fn thirty_seconds_of_growing_drafts_cannot_retranslate_on_every_stable_timer() {
        let now = Instant::now();
        let mut pacer = PreviewRequestPacer::default();
        let mut starts = 0;
        for update in 1..=60 {
            let at = now + Duration::from_millis(update * 500);
            let candidate =
                PreviewCandidate::new(&"字".repeat(update as usize * 10), Some("zh"), 300);
            if pacer
                .next_candidate_start_at(at, candidate)
                .is_some_and(|next| next <= at)
            {
                pacer.record_candidate_start(at, candidate);
                starts += 1;
            }
        }
        assert!(
            starts <= 13,
            "cumulative length must reduce repeated input work"
        );
        assert!(starts > 1, "growing speech still receives previews");
    }

    #[test]
    fn pause_and_reconnect_transfer_budget_without_text_or_refunding_cooldown() {
        let now = Instant::now();
        let mut old = PreviewRequestPacer::default();
        let candidate = PreviewCandidate::new("Synthetic preview", Some("en"), 20);
        old.record_candidate_start(now, candidate);
        old.suppress_after_rate_limit(now);
        old.set_shared_cooldown(Some(now + Duration::from_secs(8)));
        let budget = old.export_budget();
        old.reset();
        let mut resumed = PreviewRequestPacer::default();
        resumed.restore_budget(budget);
        assert!(
            resumed.has_changed(candidate),
            "old draft fingerprint must not transfer"
        );
        assert_eq!(resumed.next_start_at(now), now + RATE_LIMIT_PREVIEW_PAUSE);
        assert_eq!(
            resumed.next_shared_start_at(now),
            now + Duration::from_secs(8)
        );
        resumed.set_shared_cooldown(None);
        assert!(resumed.next_shared_start_at(now) >= now + MIN_SHARED_REQUEST_SPACING);
        assert_eq!(resumed.suppression_remaining(now), RATE_LIMIT_PREVIEW_PAUSE);
    }
}
