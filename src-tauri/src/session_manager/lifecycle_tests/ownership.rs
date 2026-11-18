//! Ownership behavior regressions.

use super::*;

#[test]
fn accepted_pause_cannot_overwrite_stopped_failed_or_unowned_connecting_sessions() {
    for status in [
        SessionStatus::Idle,
        SessionStatus::Stopping,
        SessionStatus::Error("synthetic failure".into()),
    ] {
        assert!(!accepted_pause_transition_is_valid(
            &status,
            false,
            NO_GENERATION,
            true
        ));
    }
    assert!(!accepted_pause_transition_is_valid(
        &SessionStatus::Connecting,
        false,
        NO_GENERATION,
        false
    ));
    assert!(!accepted_pause_transition_is_valid(
        &SessionStatus::Connecting,
        true,
        NO_GENERATION,
        true
    ));
    assert!(!accepted_pause_transition_is_valid(
        &SessionStatus::Connecting,
        false,
        99,
        true
    ));
    assert!(
        !pause_transition_is_valid(&SessionStatus::Connecting, false, NO_GENERATION),
        "ordinary new pause requests still cannot enter while connecting"
    );
}

#[test]
fn only_transient_transport_and_bounded_queue_errors_recover() {
    for code in [
        "transport_error",
        // The desktop event pump receives recognizer error labels before
        // the adapter has normalized their code.
        "custom_speech_unreachable",
        "custom_speech_not_connected",
        "custom_speech_health_timeout",
        "gemini_transcribe_unreachable",
        "gemini_transcribe_not_connected",
        "gemini_transcribe_health_timeout",
        "provider_event_backlog_overflow",
        "translation_backlog_overflow",
        "translation_rate_limited",
        "translation_temporarily_unavailable",
    ] {
        assert!(provider_error_is_retryable(code));
    }
    assert!(!provider_error_is_retryable("invalid_api_key"));
    assert!(!provider_error_is_retryable("invalid_configuration"));
    assert!(!provider_error_is_retryable(
        "translation_authentication_failed"
    ));
    assert!(!provider_error_is_retryable("translation_failed"));
    for code in [
        "custom_speech_authentication_failed",
        "gemini_transcribe_authentication_failed",
        "custom_speech_endpoint_invalid",
        "gemini_transcribe_language_unsupported",
        "custom_speech_protocol_invalid",
        "gemini_transcribe_protocol_invalid",
        "unrecognized_provider_error",
    ] {
        assert!(!provider_error_is_retryable(code), "{code}");
    }
}

#[test]
fn stale_translation_backoff_cannot_change_a_replacement_or_stopping_session() {
    let event = LiveTranslateServerEvent::TranslationDeferred(TranslationRecovery {
        reason: crate::core::diagnostics::TranslationRecoveryReason::RateLimited,
        retry_after_ms: 4_000,
        retry_scheduled: true,
    });
    assert!(generation_accepts_event(41, NO_GENERATION, 41, &event));
    assert!(!generation_accepts_event(42, 41, 41, &event));
    assert!(!generation_accepts_event(NO_GENERATION, 41, 41, &event));
    assert!(!generation_accepts_event(42, NO_GENERATION, 41, &event));
}

#[test]
fn terminal_after_setup_ack_invalidates_startup_before_listening_commit() {
    let active = AtomicU64::new(41);
    let sequence = AtomicU64::new(41);

    assert_eq!(
        invalidate_generation_atoms(&active, &sequence, 41),
        Some(42)
    );
    assert_eq!(active.load(Ordering::SeqCst), NO_GENERATION);
    assert_ne!(sequence.load(Ordering::SeqCst), 41);
    assert!(!generation_accepts_event(
        active.load(Ordering::SeqCst),
        NO_GENERATION,
        41,
        &LiveTranslateServerEvent::TranslationStarted,
    ));
}

#[test]
fn stopped_start_does_not_publish_a_late_configuration_failure() {
    let generation = 51;
    let active = AtomicU64::new(generation);
    let sequence = AtomicU64::new(generation);
    let mut controller = TranslationSessionController::default();
    controller.begin_connecting();

    // Stop claims the generation before the in-flight settings read
    // returns its error.
    sequence.store(generation + 1, Ordering::SeqCst);
    assert_eq!(active.swap(NO_GENERATION, Ordering::SeqCst), generation);
    if invalidate_generation_atoms(&active, &sequence, generation).is_some() {
        controller.did_fail("late settings failure");
    }

    controller.did_stop();
    assert_eq!(controller.state.status, SessionStatus::Idle);
}

#[tokio::test]
async fn stop_after_error_invalidation_cannot_be_adopted_by_the_old_failure() {
    let generation = 61;
    let active = Arc::new(AtomicU64::new(generation));
    let sequence = Arc::new(AtomicU64::new(generation));
    let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
    controller.lock().unwrap().begin_connecting();
    let (invalidated_tx, invalidated_rx) = tokio::sync::oneshot::channel();
    let (continue_tx, continue_rx) = tokio::sync::oneshot::channel();

    let old_failure = {
        let active = Arc::clone(&active);
        let sequence = Arc::clone(&sequence);
        let controller = Arc::clone(&controller);
        tokio::spawn(async move {
            let owned_epoch = invalidate_generation_atoms(&active, &sequence, generation).unwrap();
            invalidated_tx.send(owned_epoch).unwrap();
            continue_rx.await.unwrap();
            if sequence.load(Ordering::SeqCst) == owned_epoch {
                controller.lock().unwrap().did_fail("stale connect failure");
            }
        })
    };

    let failure_epoch = invalidated_rx.await.unwrap();
    assert_eq!(failure_epoch, generation + 1);
    // Stop is the newer owner and reaches Idle before the old connect
    // failure resumes between invalidation and publication.
    sequence.fetch_add(1, Ordering::SeqCst);
    active.store(NO_GENERATION, Ordering::SeqCst);
    controller.lock().unwrap().did_stop();
    continue_tx.send(()).unwrap();
    old_failure.await.unwrap();

    assert_eq!(controller.lock().unwrap().state.status, SessionStatus::Idle);
    assert_eq!(sequence.load(Ordering::SeqCst), generation + 2);
}

#[test]
fn generation_invalidation_cannot_claim_an_epoch_already_owned_by_stop() {
    let generation = 71;
    let active = AtomicU64::new(generation);
    let sequence = AtomicU64::new(generation + 1);

    assert_eq!(
        invalidate_generation_atoms(&active, &sequence, generation),
        None
    );
    assert_eq!(active.load(Ordering::SeqCst), generation);
    assert_eq!(sequence.load(Ordering::SeqCst), generation + 1);
}

#[test]
fn concurrent_double_start_has_one_owner() {
    let in_progress = Arc::new(AtomicBool::new(false));
    assert!(try_begin_start(&in_progress));
    assert!(!try_begin_start(&in_progress));

    let owner = SessionRequestGuard {
        in_progress: Arc::clone(&in_progress),
    };
    drop(owner);
    assert!(try_begin_start(&in_progress));
    in_progress.store(false, Ordering::SeqCst);
}

#[test]
fn concurrent_pause_requests_are_admitted_before_reading_the_toggle_direction() {
    let toggle = Arc::new(AtomicBool::new(false));
    let start = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    let (reading, read_started) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let first = std::thread::spawn({
        let toggle = Arc::clone(&toggle);
        let start = Arc::clone(&start);
        let reads = Arc::clone(&reads);
        move || {
            begin_pause_request(&toggle, &start, || {
                reads.fetch_add(1, Ordering::SeqCst);
                reading.send(()).unwrap();
                released.recv_timeout(Duration::from_secs(2)).unwrap();
                true
            })
        }
    });
    read_started.recv_timeout(Duration::from_secs(2)).unwrap();
    let duplicate = begin_pause_request(&toggle, &start, || {
        reads.fetch_add(1, Ordering::SeqCst);
        false
    })
    .unwrap();
    let duplicate_was_admitted = duplicate.is_some();
    drop(duplicate);
    // Release and join before asserting, including on the failing old
    // implementation, so no test thread remains blocked on failure.
    release.send(()).unwrap();
    let owner = first.join().unwrap().unwrap().unwrap();
    assert!(!duplicate_was_admitted);
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert!(owner.resume);
    drop(owner);
    assert!(!toggle.load(Ordering::SeqCst));
    assert!(!start.load(Ordering::SeqCst));
}

#[test]
fn duplicate_pause_or_resume_does_not_reverse_an_in_flight_toggle() {
    for was_paused in [false, true] {
        let toggle = Arc::new(AtomicBool::new(false));
        let start = Arc::new(AtomicBool::new(false));
        let paused = AtomicBool::new(was_paused);
        let request = begin_pause_request(&toggle, &start, || paused.load(Ordering::SeqCst))
            .unwrap()
            .unwrap();
        assert_eq!(request.resume, was_paused);
        // Pause publishes paused=true before teardown finishes; Resume
        // sets it false before its connection finishes. A second window
        // must not turn that intermediate state into the opposite action.
        paused.store(!was_paused, Ordering::SeqCst);
        assert!(
            begin_pause_request(&toggle, &start, || paused.load(Ordering::SeqCst))
                .unwrap()
                .is_none()
        );
        drop(request);
        let next = begin_pause_request(&toggle, &start, || paused.load(Ordering::SeqCst))
            .unwrap()
            .unwrap();
        assert_eq!(next.resume, !was_paused);
    }
}

#[test]
fn resume_and_start_share_admission_while_pause_can_cancel_a_start() {
    let toggle = Arc::new(AtomicBool::new(false));
    let start = Arc::new(AtomicBool::new(false));
    let starting = SessionRequestGuard::acquire(&start).unwrap();
    assert!(matches!(
        begin_pause_request(&toggle, &start, || true),
        Err("session_resume_busy")
    ));
    assert!(!toggle.load(Ordering::SeqCst));
    let pause = begin_pause_request(&toggle, &start, || false)
        .unwrap()
        .unwrap();
    assert!(!pause.resume);
    drop(pause);
    assert!(start.load(Ordering::SeqCst));
    drop(starting);

    let resume = begin_pause_request(&toggle, &start, || true)
        .unwrap()
        .unwrap();
    assert!(SessionRequestGuard::acquire(&start).is_none());
    drop(resume);
    assert!(SessionRequestGuard::acquire(&start).is_some());
}

#[test]
fn pause_and_resume_admission_do_not_block_a_newer_stop_epoch() {
    for was_paused in [false, true] {
        let toggle = Arc::new(AtomicBool::new(false));
        let start = Arc::new(AtomicBool::new(false));
        let request = begin_pause_request(&toggle, &start, || was_paused)
            .unwrap()
            .unwrap();
        let sequence = AtomicU64::new(71);
        assert_eq!(
            advance_lifecycle_sequence_if_current(&sequence, 71),
            Some(72)
        );
        assert!(!lifecycle_sequence_matches(&sequence, 71));
        drop(request);
        assert!(!toggle.load(Ordering::SeqCst));
        assert!(!start.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn settings_mutation_and_lifecycle_operation_are_serialized() {
    let lock = Arc::new(TokioMutex::new(()));
    let settings_guard = Arc::clone(&lock).lock_owned().await;
    let lock_for_stop = Arc::clone(&lock);
    let mut stop_waiter = tokio::spawn(async move {
        let _stop_guard = lock_for_stop.lock_owned().await;
    });

    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut stop_waiter)
            .await
            .is_err()
    );
    drop(settings_guard);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), stop_waiter)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn teardown_notification_registered_before_await_is_not_lost() {
    let notify = Notify::new();
    let notified = notify.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();

    notify.notify_waiters();

    assert!(tokio::time::timeout(Duration::from_millis(50), notified)
        .await
        .is_ok());
}

#[tokio::test]
async fn stop_generation_change_cancels_a_slow_connect_immediately() {
    let generation = 41;
    let active = Arc::new(AtomicU64::new(generation));
    let sequence = Arc::new(AtomicU64::new(generation));
    let notify = Arc::new(Notify::new());
    let mut operation = tokio::spawn(run_generation_bound_operation(
        Arc::clone(&active),
        Arc::clone(&sequence),
        Arc::clone(&notify),
        generation,
        std::future::pending::<()>(),
    ));

    tokio::task::yield_now().await;
    sequence.store(generation + 1, Ordering::SeqCst);
    active.store(NO_GENERATION, Ordering::SeqCst);
    notify.notify_waiters();

    assert_eq!(
        tokio::time::timeout(Duration::from_millis(100), &mut operation)
            .await
            .unwrap()
            .unwrap(),
        Err(SESSION_START_CANCELLED.into())
    );
}

#[tokio::test]
async fn stopped_slow_resume_cannot_restore_a_paused_listening_state() {
    let generation = 61;
    let active = Arc::new(AtomicU64::new(generation));
    let sequence = Arc::new(AtomicU64::new(generation));
    let notify = Arc::new(Notify::new());
    let operation = tokio::spawn(run_generation_bound_operation(
        Arc::clone(&active),
        Arc::clone(&sequence),
        Arc::clone(&notify),
        generation,
        std::future::pending::<()>(),
    ));
    let mut controller = TranslationSessionController::default();
    controller.begin_connecting();

    tokio::task::yield_now().await;
    sequence.store(generation + 1, Ordering::SeqCst);
    active.store(NO_GENERATION, Ordering::SeqCst);
    controller.did_stop();
    notify.notify_waiters();

    let error = operation.await.unwrap().unwrap_err();
    assert!(!resume_failure_is_still_owned(
        &error,
        generation.wrapping_add(1),
        sequence.load(Ordering::SeqCst),
        &controller.state.status,
        active.load(Ordering::SeqCst),
    ));
    assert_eq!(controller.state.status, SessionStatus::Idle);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_resume_keeps_the_gate_through_restore_so_a_waiting_stop_wins() {
    let lifecycle = Arc::new(TokioMutex::new(()));
    let sequence = Arc::new(AtomicU64::new(72));
    let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
    let paused = Arc::new(AtomicBool::new(false));
    let error = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
    controller.lock().unwrap().did_fail(error);
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let restore = {
        let lifecycle = Arc::clone(&lifecycle);
        let sequence = Arc::clone(&sequence);
        let controller = Arc::clone(&controller);
        let paused = Arc::clone(&paused);
        let failed_controller = Arc::clone(&controller);
        tokio::spawn(async move {
            restore_failed_resume(
                lifecycle,
                move || {
                    resume_failure_is_still_owned(
                        error,
                        72,
                        sequence.load(Ordering::SeqCst),
                        &failed_controller.lock().unwrap().state.status,
                        NO_GENERATION,
                    )
                },
                move || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    paused.store(true, Ordering::SeqCst);
                    controller.lock().unwrap().did_pause();
                },
            )
            .await
        })
    };
    entered_rx.await.unwrap();
    let (claimed_tx, claimed_rx) = tokio::sync::oneshot::channel();
    let stop = {
        let lifecycle = Arc::clone(&lifecycle);
        let sequence = Arc::clone(&sequence);
        let controller = Arc::clone(&controller);
        let paused = Arc::clone(&paused);
        tokio::spawn(async move {
            sequence.fetch_add(1, Ordering::SeqCst);
            claimed_tx.send(lifecycle.try_lock().is_err()).unwrap();
            let _stop_guard = lifecycle.lock_owned().await;
            paused.store(false, Ordering::SeqCst);
            controller.lock().unwrap().did_stop();
        })
    };
    let stop_waited_for_restore = claimed_rx.await.unwrap();
    release_tx.send(()).unwrap();
    assert!(restore.await.unwrap());
    stop.await.unwrap();
    assert!(stop_waited_for_restore);
    assert_eq!(controller.lock().unwrap().state.status, SessionStatus::Idle);
    assert!(!paused.load(Ordering::SeqCst));
    assert_eq!(sequence.load(Ordering::SeqCst), 73);
}

#[tokio::test]
async fn failed_resume_skips_restore_after_a_newer_stop_has_finished() {
    let lifecycle = Arc::new(TokioMutex::new(()));
    let controller = Mutex::new(TranslationSessionController::default());
    let restored = AtomicBool::new(false);
    assert!(
        !restore_failed_resume(
            lifecycle,
            || {
                resume_failure_is_still_owned(
                    "synthetic_failure",
                    72,
                    73,
                    &controller.lock().unwrap().state.status,
                    NO_GENERATION,
                )
            },
            || restored.store(true, Ordering::SeqCst),
        )
        .await
    );
    assert!(!restored.load(Ordering::SeqCst));
    assert_eq!(controller.lock().unwrap().state.status, SessionStatus::Idle);
}

#[tokio::test]
async fn pause_rechecks_state_after_a_concurrent_stop_owns_the_gate() {
    let lock = Arc::new(TokioMutex::new(()));
    let status = Arc::new(Mutex::new(SessionStatus::Listening));
    let paused = Arc::new(AtomicBool::new(false));
    let active = Arc::new(AtomicU64::new(7));
    let stop_guard = Arc::clone(&lock).lock_owned().await;

    let pause_waiter = {
        let lock = Arc::clone(&lock);
        let status = Arc::clone(&status);
        let paused = Arc::clone(&paused);
        let active = Arc::clone(&active);
        tokio::spawn(async move {
            let _pause_guard = lock.lock_owned().await;
            pause_transition_is_valid(
                &status.lock().unwrap(),
                paused.load(Ordering::SeqCst),
                active.load(Ordering::SeqCst),
            )
        })
    };

    *status.lock().unwrap() = SessionStatus::Idle;
    active.store(NO_GENERATION, Ordering::SeqCst);
    drop(stop_guard);

    assert!(!pause_waiter.await.unwrap());
    assert!(!resume_transition_is_valid(
        &SessionStatus::Idle,
        true,
        NO_GENERATION,
        true,
    ));
}

#[tokio::test]
async fn concurrent_pause_or_reconnect_has_exactly_one_cleanup_owner() {
    async fn claim_after_gate(
        lock: Arc<TokioMutex<()>>,
        sequence: Arc<AtomicU64>,
        active: Arc<AtomicU64>,
        ready: Arc<tokio::sync::Barrier>,
    ) -> u64 {
        let request = sequence.fetch_add(1, Ordering::SeqCst) + 1;
        ready.wait().await;
        let _guard = lock.lock_owned().await;
        if sequence.load(Ordering::SeqCst) != request {
            return NO_GENERATION;
        }
        active.swap(NO_GENERATION, Ordering::SeqCst)
    }

    for _operation in ["pause", "reconnect"] {
        let lock = Arc::new(TokioMutex::new(()));
        let sequence = Arc::new(AtomicU64::new(100));
        let active = Arc::new(AtomicU64::new(77));
        let ready = Arc::new(tokio::sync::Barrier::new(2));
        let first = tokio::spawn(claim_after_gate(
            Arc::clone(&lock),
            Arc::clone(&sequence),
            Arc::clone(&active),
            Arc::clone(&ready),
        ));
        let second = tokio::spawn(claim_after_gate(
            Arc::clone(&lock),
            Arc::clone(&sequence),
            Arc::clone(&active),
            Arc::clone(&ready),
        ));

        let claims = [first.await.unwrap(), second.await.unwrap()];
        assert_eq!(claims.iter().filter(|claim| **claim == 77).count(), 1);
        assert_eq!(active.load(Ordering::SeqCst), NO_GENERATION);
    }
}

#[test]
fn stale_terminal_cleanup_cannot_clear_new_generation_settings() {
    let settings = Mutex::new(Some("new-generation".to_string()));
    let owner = AtomicU64::new(8);

    assert!(!clear_owned_value_if_generation(&settings, &owner, 7));
    assert_eq!(settings.lock().unwrap().as_deref(), Some("new-generation"));
    assert_eq!(owner.load(Ordering::SeqCst), 8);
}
