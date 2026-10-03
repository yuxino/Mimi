//! Pure final-translation scheduling policy shared by desktop and Android.
//! Adapters own clocks, HTTP, timers and cancellation; this module owns bounds.

use serde::{Deserialize, Serialize};

/// Waiting entries only; the one active request is not counted here.
pub const MAX_FINAL_QUEUE_DEPTH: usize = 3;
pub const FINAL_DEADLINE_MS: u64 = 45_000;
pub const MAX_TRANSLATION_ATTEMPTS: usize = 3;
pub const FINISH_DRAIN_TIMEOUT_MS: u64 = 3_000;
pub const RECOGNITION_FINISH_TIMEOUT_MS: u64 = 1_000;
pub const REALTIME_FINISH_TIMEOUT_MS: u64 = 2_000;
pub const PROVIDER_FINISH_TIMEOUT_MS: u64 = 6_000;
pub const STARTUP_AUDIO_LIMIT_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslationPolicy {
    pub max_final_queue_depth: usize,
    pub final_deadline_ms: u64,
    pub max_translation_attempts: usize,
    pub finish_drain_timeout_ms: u64,
    pub recognition_finish_timeout_ms: u64,
    pub realtime_finish_timeout_ms: u64,
    pub provider_finish_timeout_ms: u64,
    pub startup_audio_limit_ms: u64,
}

pub const fn policy() -> TranslationPolicy {
    TranslationPolicy {
        max_final_queue_depth: MAX_FINAL_QUEUE_DEPTH,
        final_deadline_ms: FINAL_DEADLINE_MS,
        max_translation_attempts: MAX_TRANSLATION_ATTEMPTS,
        finish_drain_timeout_ms: FINISH_DRAIN_TIMEOUT_MS,
        recognition_finish_timeout_ms: RECOGNITION_FINISH_TIMEOUT_MS,
        realtime_finish_timeout_ms: REALTIME_FINISH_TIMEOUT_MS,
        provider_finish_timeout_ms: PROVIDER_FINISH_TIMEOUT_MS,
        startup_audio_limit_ms: STARTUP_AUDIO_LIMIT_MS,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryClass {
    Permanent,
    Temporary,
    RateLimited,
}

/// HTTP classification uses status only, never provider messages or text.
pub const fn classify_http(status: u16) -> RetryClass {
    match status {
        429 => RetryClass::RateLimited,
        408 | 500..=u16::MAX => RetryClass::Temporary,
        _ => RetryClass::Permanent,
    }
}

/// Android adapters already emit this fixed, content-free error vocabulary.
/// Unknown/malformed codes and authentication/configuration errors fail closed.
pub fn classify_failure(code: &str) -> RetryClass {
    match code {
        "translation_timeout" | "translation_network" | "translation_invalid_http" => {
            RetryClass::Temporary
        }
        _ => {
            let status = code
                .strip_prefix("translation_http_")
                .or_else(|| code.strip_prefix("translation_rejected_"));
            let Some(status) = status else {
                return RetryClass::Permanent;
            };
            if status.len() != 3 || !status.bytes().all(|byte| byte.is_ascii_digit()) {
                return RetryClass::Permanent;
            }
            status
                .parse::<u16>()
                .ok()
                .filter(|status| (100..=599).contains(status))
                .map(classify_http)
                .unwrap_or(RetryClass::Permanent)
        }
    }
}

/// Preserve desktop's global failure-streak cooldown, even after a request's
/// attempts are exhausted. Per-entry attempt admission is a separate decision.
pub fn retry_delay_ms(class: RetryClass, failure_streak: usize) -> Option<u64> {
    let exponent = failure_streak.saturating_sub(1);
    match class {
        RetryClass::Permanent => None,
        RetryClass::RateLimited => Some(8_000u64.min(4_000u64 << exponent.min(1))),
        RetryClass::Temporary => Some(8_000u64.min(600u64 << exponent.min(4))),
    }
}

pub const fn remaining_budget_ms(elapsed_ms: u64) -> u64 {
    FINAL_DEADLINE_MS.saturating_sub(elapsed_ms)
}

pub const fn can_start(elapsed_ms: u64) -> bool {
    elapsed_ms < FINAL_DEADLINE_MS
}

pub const fn can_enqueue(waiting_depth: usize, accepting: bool) -> bool {
    accepting && waiting_depth < MAX_FINAL_QUEUE_DEPTH
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryDecision {
    pub retry: bool,
    pub delay_ms: u64,
    pub expired: bool,
    pub exhausted: bool,
}

/// `attempt` is the just-completed attempt, starting at one. The remaining
/// budget includes waiting, earlier calls and retry delays; never reset it.
pub fn retry_with_remaining(
    class: RetryClass,
    attempt: usize,
    failure_streak: usize,
    remaining_ms: u64,
) -> RetryDecision {
    let delay = retry_delay_ms(class, failure_streak);
    let exhausted = attempt == 0 || attempt >= MAX_TRANSLATION_ATTEMPTS;
    let expired = remaining_ms == 0 || delay.is_some_and(|delay| delay >= remaining_ms);
    let retry = !exhausted && !expired && delay.is_some();
    RetryDecision {
        retry,
        delay_ms: if retry { delay.unwrap_or(0) } else { 0 },
        expired,
        exhausted,
    }
}

pub fn retry_decision(code: &str, attempt: usize, elapsed_ms: u64) -> RetryDecision {
    retry_with_remaining(
        classify_failure(code),
        attempt,
        attempt,
        remaining_budget_ms(elapsed_ms),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_admission_has_one_original_budget_and_three_waiting_slots() {
        assert!(can_enqueue(2, true));
        assert!(!can_enqueue(3, true));
        assert!(!can_enqueue(0, false));
        assert!(can_start(44_999));
        assert!(!can_start(45_000));
        assert_eq!(remaining_budget_ms(44_999), 1);
        assert_eq!(remaining_budget_ms(u64::MAX), 0);
    }

    #[test]
    fn only_known_transient_failures_retry() {
        for code in [
            "translation_network",
            "translation_timeout",
            "translation_http_408",
            "translation_http_503",
            "translation_rejected_500",
        ] {
            assert_eq!(classify_failure(code), RetryClass::Temporary, "{code}");
        }
        assert_eq!(
            classify_failure("translation_http_429"),
            RetryClass::RateLimited
        );
        for code in [
            "translation_http_401",
            "translation_http_403",
            "translation_http_400",
            "translation_response",
            "translation_key",
            "translation_too_large",
            "translation_http_0500",
            "translation_http_+500",
            "translation_http_600",
            "provider said 429",
            "translation_rejected_429secret",
        ] {
            assert_eq!(classify_failure(code), RetryClass::Permanent, "{code}");
            assert!(!retry_decision(code, 1, 0).retry);
        }
    }

    #[test]
    fn retry_delays_preserve_rate_limit_and_failure_streak_bounds() {
        assert_eq!(retry_delay_ms(RetryClass::Temporary, 1), Some(600));
        assert_eq!(retry_delay_ms(RetryClass::Temporary, 2), Some(1_200));
        assert_eq!(
            retry_delay_ms(RetryClass::Temporary, usize::MAX),
            Some(8_000)
        );
        assert_eq!(retry_delay_ms(RetryClass::RateLimited, 1), Some(4_000));
        assert_eq!(retry_delay_ms(RetryClass::RateLimited, 2), Some(8_000));
        assert_eq!(
            retry_delay_ms(RetryClass::RateLimited, usize::MAX),
            Some(8_000)
        );
        assert_eq!(retry_delay_ms(RetryClass::Permanent, 1), None);
    }

    #[test]
    fn retries_do_not_reset_time_or_attempt_budget() {
        assert_eq!(
            retry_decision("translation_network", 1, 44_399).delay_ms,
            600
        );
        let late = retry_decision("translation_network", 1, 44_400);
        assert!(!late.retry);
        assert!(late.expired);
        assert!(!retry_decision("translation_network", 3, 0).retry);
        assert!(retry_decision("translation_network", 3, 0).exhausted);
        assert!(!retry_decision("translation_network", 0, 0).retry);
        assert!(!retry_decision("translation_http_429", 2, 37_000).retry);
    }
}
