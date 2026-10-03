//! Content-free pipeline diagnostics: timing, counts, language codes, and
//! sanitized error labels only. Recognized or translated text is forbidden.

/// Logs a pipeline diagnostic through `tracing`. Disabled when the
/// `MIMI_PIPELINE_DIAGNOSTICS` environment variable is exactly `"0"`.
#[macro_export]
macro_rules! pipeline_log {
    ($($arg:tt)*) => {{
        $crate::core::development_debug::record_pipeline(format_args!($($arg)*));
        if $crate::core::diagnostics::is_enabled() {
            tracing::info!($($arg)*);
        }
    }};
}

pub fn is_enabled() -> bool {
    static CELL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CELL.get_or_init(|| std::env::var("MIMI_PIPELINE_DIAGNOSTICS").as_deref() != Ok("0"))
}

/// Milliseconds elapsed between two `Instant`s.
pub fn milliseconds(start: std::time::Instant, end: std::time::Instant) -> u64 {
    end.saturating_duration_since(start).as_millis() as u64
}

/// The two measured translation boundaries are deliberately distinct:
/// a text request duration is not the same as waiting for a streamed final.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TranslationLatencyKind {
    Request,
    Follow,
}

/// A content-free measurement of the latest current successful translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslationLatency {
    pub milliseconds: u64,
    pub kind: TranslationLatencyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranslationRecoveryReason {
    RateLimited,
    TemporarilyUnavailable,
}

/// Local, nonterminal MT recovery. Contains no service response or content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRecovery {
    pub reason: TranslationRecoveryReason,
    pub retry_after_ms: u64,
    /// False when a replaceable preview exhausted its bounded attempts. No
    /// retry runs until new speech schedules work; capture continues.
    pub retry_scheduled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_backoff_has_a_content_free_camel_case_wire_contract() {
        assert_eq!(
            serde_json::to_value(TranslationRecovery {
                reason: TranslationRecoveryReason::RateLimited,
                retry_after_ms: 4_000,
                retry_scheduled: true,
            })
            .unwrap(),
            serde_json::json!({"reason":"rateLimited","retryAfterMs":4000,"retryScheduled":true})
        );
        assert_eq!(
            serde_json::to_value(TranslationRecoveryReason::TemporarilyUnavailable).unwrap(),
            "temporarilyUnavailable"
        );
        assert_eq!(
            serde_json::to_value(TranslationRecovery {
                reason: TranslationRecoveryReason::TemporarilyUnavailable,
                retry_after_ms: 0,
                retry_scheduled: false,
            })
            .unwrap(),
            serde_json::json!({"reason":"temporarilyUnavailable","retryAfterMs":0,"retryScheduled":false})
        );
    }

    #[test]
    fn milliseconds_measures_elapsed_time() {
        let start = std::time::Instant::now();
        let end = start + std::time::Duration::from_millis(150);
        assert_eq!(milliseconds(start, end), 150);
    }

    #[test]
    fn translation_latency_kinds_keep_distinct_wire_values() {
        assert_eq!(
            serde_json::to_value(TranslationLatencyKind::Request).unwrap(),
            "request"
        );
        assert_eq!(
            serde_json::to_value(TranslationLatencyKind::Follow).unwrap(),
            "follow"
        );
    }
}
