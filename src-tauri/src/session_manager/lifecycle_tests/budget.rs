//! Budget behavior regressions.

use super::*;

#[test]
fn mt_recovery_cannot_bypass_service_cooldown_by_replacing_the_client() {
    for code in ["translation_rate_limited", "translation_backlog_overflow"] {
        assert!(provider_error_is_retryable(code));
        assert_eq!(
            provider_recovery_minimum_delay(code),
            Duration::from_secs(8)
        );
        for attempt in 0..RECOVERY_ATTEMPTS {
            assert!(
                recovery_delay(attempt, 42).max(provider_recovery_minimum_delay(code))
                    >= Duration::from_secs(8)
            );
        }
    }
    assert_eq!(
        provider_recovery_minimum_delay("translation_temporarily_unavailable"),
        Duration::from_millis(600)
    );
    assert_eq!(
        provider_recovery_minimum_delay("transport_error"),
        Duration::ZERO
    );
}

#[test]
fn desktop_recovery_budget_survives_short_success_across_task_owners() {
    let active = AtomicU64::new(1);
    let sequence = AtomicU64::new(1);
    let recovering = AtomicBool::new(false);
    let retry = AtomicU64::new(NO_GENERATION);
    let mut continuity = RecoveryContinuity::default();
    let mut failed_generation = 1;
    let now = Instant::now();

    for budget_index in 0..=RECOVERY_ATTEMPTS {
        assert_eq!(
            claim_generation_recovery_atoms(
                &active,
                &sequence,
                &recovering,
                &retry,
                failed_generation,
            ),
            Some(false)
        );
        let generation =
            advance_recovery_request_atoms(&active, &sequence, failed_generation).unwrap();
        let mut schedule =
            continuity.take_for_failure(failed_generation, generation, Duration::from_millis(600));
        if budget_index == RECOVERY_ATTEMPTS {
            assert!(schedule.next_attempt().is_none());
            assert!(schedule.owns_exhaustion(
                sequence.load(Ordering::SeqCst),
                active.load(Ordering::SeqCst),
            ));
            break;
        }
        let attempt = schedule.next_attempt().unwrap();
        assert_eq!(
            attempt.index, 0,
            "each task starts with its own first attempt"
        );
        assert_eq!(attempt.budget_index, budget_index);
        active.store(generation, Ordering::SeqCst);
        recovering.store(true, Ordering::SeqCst);
        let task_present = AtomicBool::new(true);
        assert!(commit_recovery_success_atoms(
            &active,
            &sequence,
            &recovering,
            &retry,
            generation,
            || task_present.swap(false, Ordering::SeqCst),
        ));
        continuity.record_connected(schedule, now);
        assert!(!task_present.load(Ordering::SeqCst));
        failed_generation = generation;
    }
}

#[test]
fn desktop_recovery_budget_rejects_old_owners_after_manual_replacement() {
    let now = Instant::now();
    let mut continuity = RecoveryContinuity::default();
    let mut old = RecoverySchedule::new(1, 3, Duration::ZERO);
    assert!(old.next_attempt().is_some());
    continuity.record_connected(old, now);
    // Accepted manual Start/Stop/Retry/configuration changes clear under
    // the same generation gate before the replacement can connect.
    continuity.clear();
    let mut current = continuity.take_for_failure(5, 7, Duration::ZERO);
    assert_eq!(current.next_attempt().unwrap().budget_index, 0);
    continuity.record_connected(current, now);
    let active = AtomicU64::new(7);
    let sequence = AtomicU64::new(7);
    let recovering = AtomicBool::new(false);
    let retry = AtomicU64::new(NO_GENERATION);
    assert!(!commit_recovery_success_atoms(
        &active,
        &sequence,
        &recovering,
        &retry,
        3,
        || panic!("a stale success cannot retire the replacement task"),
    ));
    assert!(advance_recovery_request_atoms(&active, &sequence, 3).is_none());
    active.store(NO_GENERATION, Ordering::SeqCst);
    assert!(advance_recovery_request_atoms(&active, &sequence, 3).is_none());
    assert!(!complete_recovery_health_round(
        &mut continuity,
        (7, 7, 9),
        (3, 8),
        (2, 2),
        now + HEALTH_CHECK_INTERVAL,
    ));
    let mut next = continuity.take_for_failure(7, 9, Duration::ZERO);
    assert_eq!(next.next_attempt().unwrap().budget_index, 1);
}
