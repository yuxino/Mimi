//! Recovery behavior regressions.

use super::*;

#[tokio::test]
async fn startup_rollback_keeps_the_provider_pump_alive_until_it_queues_recovery() {
    let active = Arc::new(AtomicU64::new(7));
    let sequence = Arc::new(AtomicU64::new(7));
    let notify = Arc::new(Notify::new());
    let recovery_queued = Arc::new(AtomicBool::new(false));
    let (failure_tx, failure_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let pump = {
        let active = Arc::clone(&active);
        let sequence = Arc::clone(&sequence);
        let notify = Arc::clone(&notify);
        let recovery_queued = Arc::clone(&recovery_queued);
        tokio::spawn(async move {
            failure_rx.await.unwrap();
            assert!(invalidate_generation_atoms(&active, &sequence, 7).is_some());
            notify.notify_waiters();
            // Provider-error handling waits on lifecycle/resource teardown
            // after invalidation and before queue_recovery_with_delay.
            release_rx.await.unwrap();
            recovery_queued.store(true, Ordering::SeqCst);
        })
    };
    let mut failure_tx = Some(failure_tx);
    let cleaned = AtomicBool::new(false);
    let result = connect_source_group(
        AudioInput::Both.sources(),
        |source| {
            if source == AudioSource::Microphone {
                failure_tx.take().unwrap().send(()).unwrap();
            }
            let active = Arc::clone(&active);
            let sequence = Arc::clone(&sequence);
            let notify = Arc::clone(&notify);
            async move {
                if source == AudioSource::System {
                    return Ok(());
                }
                run_generation_bound_operation(
                    active,
                    sequence,
                    notify,
                    7,
                    std::future::pending::<()>(),
                )
                .await
                .map(|_| ())
            }
        },
        |pumps| {
            cleaned.store(true, Ordering::SeqCst);
            if matches!(pumps, EventPumpCleanup::Abort) {
                pump.abort();
            }
            std::future::ready(())
        },
    )
    .await;
    assert_eq!(result, Err(SESSION_START_CANCELLED.into()));
    assert!(cleaned.load(Ordering::SeqCst));
    // The startup cannot claim terminal failure after the pump has already
    // claimed retry ownership. Its stale-error path must preserve the pump.
    assert!(invalidate_generation_atoms(&active, &sequence, 7).is_none());
    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), pump)
        .await
        .unwrap()
        .unwrap();
    assert!(recovery_queued.load(Ordering::SeqCst));
}

#[test]
fn late_health_checks_cannot_supply_another_generation_or_task_measurement() {
    let health = Some(HealthCheckLatency {
        generation: 41,
        task_id: 9,
        milliseconds: 80,
    });
    for (generation, task_id) in [(42, 9), (41, 10), (41, NO_GENERATION)] {
        assert_eq!(
            visible_session_latencies(
                &SessionStatus::Listening,
                false,
                false,
                generation,
                task_id,
                health,
                None,
            ),
            (None, None, None)
        );
    }
}

#[test]
fn recovery_backoff_is_bounded_exponential_and_deterministic() {
    let generation = 42;
    assert_eq!(recovery_delay(0, generation), Duration::ZERO);

    for (attempt, base_ms) in [(1, 500_u128), (2, 1_000), (3, 2_000)] {
        let first = recovery_delay(attempt, generation);
        let second = recovery_delay(attempt, generation);
        assert_eq!(first, second);
        assert!(first.as_millis() >= base_ms);
        assert!(first.as_millis() <= base_ms + base_ms / 4);
    }

    assert!(recovery_delay(2, generation) > recovery_delay(1, generation));
    assert!(recovery_delay(3, generation) > recovery_delay(2, generation));
}

#[test]
fn recovery_success_handoff_preserves_an_already_accepted_failure() {
    let transition = Arc::new(Mutex::new(()));
    let active = Arc::new(AtomicU64::new(41));
    let sequence = Arc::new(AtomicU64::new(41));
    let recovering = Arc::new(AtomicBool::new(true));
    let retry = Arc::new(AtomicU64::new(NO_GENERATION));
    let accepted = Arc::new(std::sync::Barrier::new(2));
    let failure = std::thread::spawn({
        let transition = Arc::clone(&transition);
        let active = Arc::clone(&active);
        let sequence = Arc::clone(&sequence);
        let recovering = Arc::clone(&recovering);
        let retry = Arc::clone(&retry);
        let accepted = Arc::clone(&accepted);
        move || {
            {
                let _transition = transition.lock().unwrap();
                assert_eq!(
                    claim_generation_recovery_atoms(&active, &sequence, &recovering, &retry, 41,),
                    Some(true)
                );
            }
            accepted.wait();
        }
    });
    // Establish already returned Ok, but either lane fails before the
    // recovery task claims success. The original owner must keep retrying.
    accepted.wait();
    let task_present = AtomicBool::new(true);
    {
        let _transition = transition.lock().unwrap();
        assert!(!commit_recovery_success_atoms(
            &active,
            &sequence,
            &recovering,
            &retry,
            41,
            || task_present.swap(false, Ordering::SeqCst),
        ));
    }
    failure.join().unwrap();
    assert!(task_present.load(Ordering::SeqCst));
    assert!(recovering.load(Ordering::SeqCst));
    let mut schedule = RecoverySchedule::new(39, 41, Duration::ZERO);
    assert!(schedule.next_attempt().is_some());
    assert!(schedule.accept_cancelled_attempt(
        retry.swap(NO_GENERATION, Ordering::SeqCst),
        sequence.load(Ordering::SeqCst),
        true,
    ));
    assert_eq!(schedule.advance_for_retry(&sequence), Some(43));
}

#[test]
fn recovery_success_handoff_retires_the_task_before_a_later_failure() {
    let transition = Arc::new(Mutex::new(()));
    let active = Arc::new(AtomicU64::new(41));
    let sequence = Arc::new(AtomicU64::new(41));
    let recovering = Arc::new(AtomicBool::new(true));
    let retry = Arc::new(AtomicU64::new(NO_GENERATION));
    let task_present = Arc::new(AtomicBool::new(true));
    let (retiring, retired) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let completion = std::thread::spawn({
        let transition = Arc::clone(&transition);
        let active = Arc::clone(&active);
        let sequence = Arc::clone(&sequence);
        let recovering = Arc::clone(&recovering);
        let retry = Arc::clone(&retry);
        let task_present = Arc::clone(&task_present);
        move || {
            let _transition = transition.lock().unwrap();
            commit_recovery_success_atoms(&active, &sequence, &recovering, &retry, 41, || {
                let retired = task_present.swap(false, Ordering::SeqCst);
                retiring.send(()).unwrap();
                released.recv_timeout(Duration::from_secs(2)).unwrap();
                retired
            })
        }
    });
    retired.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(transition.try_lock().is_err());
    // No failure may observe the still-recovering flag after the task was
    // retired; both operations use this gate.
    release.send(()).unwrap();
    assert!(completion.join().unwrap());
    let _transition = transition.lock().unwrap();
    assert_eq!(
        claim_generation_recovery_atoms(&active, &sequence, &recovering, &retry, 41),
        Some(false)
    );
    assert!(!task_present.load(Ordering::SeqCst));
    assert_eq!(retry.load(Ordering::SeqCst), NO_GENERATION);
    // queue_recovery now has a vacant task slot for the newly failed lane.
    assert!(!task_present.swap(true, Ordering::SeqCst));
}

#[test]
fn recovery_success_handoff_cannot_retire_a_stop_or_new_generation() {
    for (active, epoch) in [(NO_GENERATION, 42), (43, 43), (41, 42)] {
        let active = AtomicU64::new(active);
        let sequence = AtomicU64::new(epoch);
        let recovering = AtomicBool::new(true);
        let retry = AtomicU64::new(43);
        let task_present = AtomicBool::new(true);
        assert!(!commit_recovery_success_atoms(
            &active,
            &sequence,
            &recovering,
            &retry,
            41,
            || task_present.swap(false, Ordering::SeqCst),
        ));
        assert_eq!(
            claim_generation_recovery_atoms(&active, &sequence, &recovering, &retry, 41),
            None,
        );
        assert!(task_present.load(Ordering::SeqCst));
        assert!(recovering.load(Ordering::SeqCst));
        assert_eq!(retry.load(Ordering::SeqCst), 43);
        assert_eq!(sequence.load(Ordering::SeqCst), epoch);
    }
}

#[test]
fn desktop_recovery_health_requires_all_sources_and_the_current_round() {
    let now = Instant::now();
    for (sources, current, elapsed, round_is_current, resets_budget) in [
        ((1, 2), (3, 3, 8), HEALTH_CHECK_INTERVAL, false, false),
        ((2, 2), (3, 3, 8), HEALTH_CHECK_INTERVAL, true, true),
        ((1, 1), (3, 3, 8), HEALTH_CHECK_INTERVAL, true, true),
        ((0, 0), (3, 3, 8), HEALTH_CHECK_INTERVAL, false, false),
        ((2, 2), (5, 5, 8), HEALTH_CHECK_INTERVAL, false, false),
        ((2, 2), (3, 4, 8), HEALTH_CHECK_INTERVAL, false, false),
        ((2, 2), (3, 3, 9), HEALTH_CHECK_INTERVAL, false, false),
        ((2, 2), (3, 3, 8), Duration::ZERO, true, false),
    ] {
        let mut continuity = RecoveryContinuity::default();
        let mut schedule = RecoverySchedule::new(1, 3, Duration::ZERO);
        assert!(schedule.next_attempt().is_some());
        continuity.record_connected(schedule, now);
        assert_eq!(
            complete_recovery_health_round(
                &mut continuity,
                current,
                (3, 8),
                sources,
                now + elapsed,
            ),
            round_is_current,
        );
        let mut next = continuity.take_for_failure(3, 5, Duration::ZERO);
        assert_eq!(
            next.next_attempt().unwrap().budget_index,
            usize::from(!resets_budget),
            "sources={sources:?} current={current:?} elapsed={elapsed:?}",
        );
    }
}

#[test]
fn manual_start_during_recovery_backoff_wins_the_epoch() {
    let recovery_failure_epoch = 72;
    let sequence = AtomicU64::new(recovery_failure_epoch);

    assert!(start_request_can_proceed(true, true, NO_GENERATION));
    assert!(!start_request_can_proceed(true, true, 71));
    assert!(!start_request_can_proceed(true, false, NO_GENERATION));

    let manual_start_generation = sequence.fetch_add(1, Ordering::SeqCst) + 1;
    assert_eq!(manual_start_generation, 73);
    assert_eq!(
        advance_lifecycle_sequence_if_current(&sequence, recovery_failure_epoch),
        None
    );
    assert_eq!(sequence.load(Ordering::SeqCst), manual_start_generation);
}

#[test]
fn transport_error_during_recovery_is_retried_only_by_the_existing_owner() {
    let generation = 81;
    assert!(cancelled_recovery_attempt_is_retryable(
        generation,
        generation,
        generation + 1,
        true,
    ));
    assert!(!cancelled_recovery_attempt_is_retryable(
        NO_GENERATION,
        generation,
        generation + 1,
        true,
    ));
    assert!(!cancelled_recovery_attempt_is_retryable(
        generation,
        generation,
        generation + 2,
        true,
    ));
}

#[test]
fn terminal_cancellation_clears_global_recovery_state() {
    let recovering = AtomicBool::new(true);
    let retry_generation = AtomicU64::new(82);

    clear_recovery_atoms(&recovering, &retry_generation);

    assert!(!recovering.load(Ordering::SeqCst));
    assert_eq!(retry_generation.load(Ordering::SeqCst), NO_GENERATION);
}

#[tokio::test]
async fn manual_start_before_recovery_exhaustion_prevents_stale_error_publish() {
    let lifecycle = Arc::new(TokioMutex::new(()));
    let sequence = Arc::new(AtomicU64::new(91));
    let active = Arc::new(AtomicU64::new(NO_GENERATION));
    let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
    controller.lock().unwrap().did_fail("old recovery failure");

    let manual_start = Arc::clone(&lifecycle).lock_owned().await;
    sequence.store(92, Ordering::SeqCst);
    active.store(92, Ordering::SeqCst);
    controller.lock().unwrap().begin_connecting();

    let recovery_publish = {
        let lifecycle = Arc::clone(&lifecycle);
        let sequence = Arc::clone(&sequence);
        let active = Arc::clone(&active);
        let controller = Arc::clone(&controller);
        tokio::spawn(async move {
            let _guard = lifecycle.lock_owned().await;
            if recovery_exhaustion_is_still_owned(
                91,
                sequence.load(Ordering::SeqCst),
                active.load(Ordering::SeqCst),
            ) {
                controller
                    .lock()
                    .unwrap()
                    .did_fail("stale recovery failure");
                true
            } else {
                false
            }
        })
    };
    drop(manual_start);

    assert!(!recovery_publish.await.unwrap());
    assert_eq!(
        controller.lock().unwrap().state.status,
        SessionStatus::Connecting
    );
}

#[test]
fn recovery_configuration_and_permission_failures_keep_the_actual_error() {
    for error in [
        "System audio capture permission was denied.",
        "Microphone capture permission was denied.",
        "System audio capture was stopped by the user.",
        "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE",
        "audio3_error.setup.request.CLIENT_ERROR",
        "audio3_error.setup.authentication.INVALID_API_KEY",
        "credential_authentication_failed",
        "custom_speech_authentication_failed",
        "custom_speech_credentials_missing",
        "custom_speech_endpoint_invalid",
        "custom_speech_model_invalid",
        "gemini_transcribe_authentication_failed",
        "gemini_transcribe_credentials_missing",
        "gemini_transcribe_language_unsupported",
        "xai_stt_authentication_failed",
        "xai_stt_credentials_missing",
        "xai_stt_language_unsupported",
        "text_translation_credentials_missing",
    ] {
        let mut controller = TranslationSessionController::default();
        controller.begin_connecting();
        apply_establish_failure_state(&mut controller, error.into(), true);
        assert_eq!(controller.state.status, SessionStatus::Error(error.into()));
        assert!(session_error_requires_user_action(error));
        assert!(!SessionStateEvent::from(&controller.state).is_active);
    }
    for error in [
        "Audio capture stopped unexpectedly.",
        "System audio capture could not be started.",
        "transport_error",
        "Audio capture setup timed out.",
        "audio3_error.setup.timeout.CLIENT_ERROR",
        "audio3_error.recognition.service.SERVER_ERROR",
        "custom_speech_unreachable",
        "custom_speech_setup_timeout",
        "custom_speech_authentication_failed: private-detail",
        "gemini_transcribe_unreachable",
        "gemini_transcribe_setup_timeout",
        "gemini_transcribe_authentication_failed: private-detail",
        "xai_stt_unreachable",
        "xai_stt_setup_timeout",
        "xai_stt_authentication_failed: private-detail",
    ] {
        assert!(!session_error_requires_user_action(error));
    }
}

#[test]
fn tencent_permanent_setup_rejection_does_not_retry_or_erase_the_new_reason() {
    use crate::clients::realtime::tencent_cloud_client::TencentCloudClientError;
    for code in [6001, 6003, 6004, 6005] {
        let error = session_connect_error(ConnectError::TencentCloud(
            TencentCloudClientError::ProviderRejected { code },
        ));
        assert!(session_error_requires_user_action(&error), "{error}");
        let mut controller = TranslationSessionController::default();
        controller.begin_connecting();
        apply_establish_failure_state(&mut controller, error.clone(), true);
        assert_eq!(controller.state.status, SessionStatus::Error(error));
    }
}

#[test]
fn built_in_setup_rejection_stops_recovery_with_the_current_failure() {
    use crate::clients::realtime::baidu_translate_client::BaiduTranslateClientError;
    use crate::clients::realtime::gemini_live_client::GeminiLiveClientError;
    use crate::clients::realtime::live_translate_client::LiveTranslateClientError;
    use crate::clients::realtime::openai_realtime_client::OpenAIRealtimeClientError;
    use crate::clients::realtime::xai_realtime_client::XAIRealtimeClientError;

    for (failure, expected) in [
        (
            ConnectError::Live(LiveTranslateClientError::SessionSetupRejected),
            "alibaba_realtime_provider_error",
        ),
        (
            ConnectError::Live(LiveTranslateClientError::AuthenticationFailed),
            "credential_authentication_failed",
        ),
        (
            ConnectError::OpenAI(OpenAIRealtimeClientError::SessionSetupRejected),
            "openai_realtime_provider_error",
        ),
        (
            ConnectError::Xai(XAIRealtimeClientError::SessionSetupRejected),
            "xai_realtime_provider_error",
        ),
        (
            ConnectError::Gemini(GeminiLiveClientError::SessionSetupRejected),
            "gemini_live_provider_error",
        ),
        (
            ConnectError::BaiduTranslate(BaiduTranslateClientError::SessionSetupRejectedWithCode(
                20316,
            )),
            "baidu_realtime_provider_error",
        ),
        (
            ConnectError::BaiduTranslate(BaiduTranslateClientError::SessionSetupRejectedWithCode(
                987654,
            )),
            "baidu_realtime_provider_error",
        ),
    ] {
        let error = session_connect_error(failure);
        assert_eq!(error, expected);
        // This is the recovery loop's terminal branch: it must retain the
        // setup rejection rather than exhaust retries and report the idle
        // timeout that originally caused recovery.
        assert!(session_error_requires_user_action(&error));
        let mut controller = TranslationSessionController::default();
        controller.begin_connecting();
        apply_establish_failure_state(&mut controller, error.clone(), true);
        assert_eq!(controller.state.status, SessionStatus::Error(error));
        assert!(!SessionStateEvent::from(&controller.state).is_active);
    }
}

#[test]
fn alibaba_transient_setup_failures_keep_bounded_recovery() {
    use crate::clients::realtime::live_translate_client::LiveTranslateClientError;

    for (failure, expected) in [
        (
            LiveTranslateClientError::ServiceUnavailable,
            "alibaba_realtime_service_unavailable",
        ),
        (
            LiveTranslateClientError::IdleTimeout,
            "alibaba_realtime_idle_timeout",
        ),
        (
            LiveTranslateClientError::UnexpectedSessionEnd,
            "live_translate_unexpected_session_end",
        ),
    ] {
        let error = session_connect_error(ConnectError::Live(failure));
        assert_eq!(error, expected);
        assert!(provider_error_is_retryable(&error));
        assert!(!session_error_requires_user_action(&error));
        let mut controller = TranslationSessionController::default();
        controller.begin_connecting();
        apply_establish_failure_state(&mut controller, error, true);
        assert_eq!(controller.state.status, SessionStatus::Connecting);
    }
}

#[test]
fn persisted_configuration_repair_restores_idle_without_losing_subtitles() {
    let error = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
    let mut controller = TranslationSessionController::default();
    seed_ui_test_live_subtitles(&mut controller, AudioInput::System);
    controller.did_fail(error);
    let subtitles = controller.state.subtitles.clone();
    let expected = Some((42, error.to_string()));
    assert!(!clear_stale_configuration_error(
        &mut controller,
        expected.as_ref(),
        42,
        false
    ));
    assert!(clear_stale_configuration_error(
        &mut controller,
        expected.as_ref(),
        42,
        true
    ));
    assert_eq!(controller.state.status, SessionStatus::Idle);
    assert_eq!(controller.state.subtitles, subtitles);
    assert!(!SessionStateEvent::from(&controller.state).is_active);
}

#[test]
fn configuration_repair_never_clears_newer_or_unrelated_failures() {
    let old = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
    let expected = Some((42, old.to_string()));
    for (status, epoch) in [
        (SessionStatus::Error(old.into()), 43),
        (
            SessionStatus::Error("audio3_error.setup.authentication.INVALID_API_KEY".into()),
            42,
        ),
        (SessionStatus::Error("transport_error".into()), 42),
        (SessionStatus::Listening, 42),
        (SessionStatus::Connecting, 42),
    ] {
        let mut controller = TranslationSessionController::default();
        controller.state.status = status.clone();
        assert!(!clear_stale_configuration_error(
            &mut controller,
            expected.as_ref(),
            epoch,
            true
        ));
        assert_eq!(controller.state.status, status);
    }
}

#[test]
fn recovery_failure_stays_connecting_and_blocks_pipeline_mutations_during_backoff() {
    let mut controller = TranslationSessionController::default();
    controller.begin_connecting();

    apply_establish_failure_state(&mut controller, "retryable failure".into(), true);

    assert_eq!(controller.state.status, SessionStatus::Connecting);
    assert!(SessionStateEvent::from(&controller.state).is_active);
    assert!(lifecycle_activity_is_active(
        controller.state.status.is_active(),
        1,
    ));
    assert!(!pipeline_settings_mutation_is_allowed(
        &controller.state.status,
        1,
    ));
}

#[tokio::test]
async fn delayed_health_or_recovery_cleanup_cannot_erase_replacement_task() {
    for stale_id in [11, 21] {
        let current_id = AtomicU64::new(stale_id + 1);
        let slot = Mutex::new(Some(tokio::spawn(async {
            std::future::pending::<()>().await;
        })));

        assert!(!clear_task_slot_if_id(&slot, &current_id, stale_id));
        assert_eq!(current_id.load(Ordering::SeqCst), stale_id + 1);
        let replacement = slot.lock().unwrap().take().unwrap();
        replacement.abort();
    }
}
