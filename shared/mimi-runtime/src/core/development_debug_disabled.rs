//! Stateless production API: never inspect subtitle content or allocate trace data.

use crate::core::models::SubtitleSnapshot;

pub const fn is_enabled() -> bool {
    false
}

pub fn record_snapshot(_generation: u64, _subtitles: &SubtitleSnapshot) -> Option<u64> {
    is_enabled().then_some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_trace_api_stays_disabled_and_does_not_assign_snapshot_ids() {
        let subtitles = SubtitleSnapshot::default();
        assert!(!is_enabled());
        for generation in [0, 1, u64::MAX] {
            assert_eq!(record_snapshot(generation, &subtitles), None);
        }
    }
}
