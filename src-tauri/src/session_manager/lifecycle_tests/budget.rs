//! Budget behavior regressions.

use super::*;

#[test]
fn custom_speech_budget_tracks_only_the_independent_translation_route() {
    use crate::core::models::TargetLanguage;
    for provider in [
        ProviderKind::CustomDashScopeASR,
        ProviderKind::CustomOpenAIASR,
    ] {
        let configuration = LiveTranslationConfiguration::with_credentials(
            provider,
            ProviderCredentials::CustomSpeech {
                endpoint: "wss://example.com/recognition".into(),
                model: "synthetic-recognition-model".into(),
                api_key: "synthetic-recognition-key".into(),
            },
            SourceLanguage::Automatic,
            TargetLanguage::Japanese,
            TranslationMode::Turbo,
        );
        assert!(
            MTBudgetScope::for_configuration("synthetic-profile".into(), &configuration).is_none()
        );
        for (credentials, expected_route) in [
            (
                TextTranslationCredentials::DeepL {
                    api_key: "synthetic-text-key".into(),
                },
                MTBudgetRoute::DeepL,
            ),
            (
                TextTranslationCredentials::DeepLX {
                    endpoint: "https://example.com/translate".into(),
                    token: String::new(),
                },
                MTBudgetRoute::DeepLX,
            ),
            (
                TextTranslationCredentials::OpenAICompatible {
                    endpoint: "https://example.com/v1".into(),
                    model: "synthetic-text-model".into(),
                    api_key: "synthetic-text-key".into(),
                },
                MTBudgetRoute::OpenAICompatible,
            ),
            (
                TextTranslationCredentials::ChatMock {
                    endpoint: "https://example.com/v1".into(),
                    model: "synthetic-text-model".into(),
                    api_key: "synthetic-text-key".into(),
                },
                MTBudgetRoute::ChatMock,
            ),
        ] {
            let mut selected = configuration.clone().with_text_credentials(credentials);
            let scope =
                MTBudgetScope::for_configuration("synthetic-profile".into(), &selected).unwrap();
            assert!(scope.route == expected_route);
            assert_eq!(scope.provider, provider);
            assert_eq!(scope.profile_id, "synthetic-profile");
            selected.target_language = TargetLanguage::Original;
            assert!(
                MTBudgetScope::for_configuration("synthetic-profile".into(), &selected).is_none()
            );
        }
    }
}

#[test]
fn chatmock_and_generic_compatible_routes_never_share_budget_continuations() {
    use crate::core::models::TargetLanguage;
    use crate::core::preview_pacing::PreviewRequestPacer;
    let scope = |chatmock: bool| {
        let credentials = if chatmock {
            ProviderCredentials::ChatMock {
                asr_api_key: "synthetic-asr".into(),
                endpoint: "https://example.com/v1".into(),
                api_key: "synthetic-text-key".into(),
                model: "synthetic-model".into(),
            }
        } else {
            ProviderCredentials::OpenAICompatible {
                asr_api_key: "synthetic-asr".into(),
                endpoint: "https://example.com/v1".into(),
                api_key: "synthetic-text-key".into(),
                model: "synthetic-model".into(),
            }
        };
        MTBudgetScope::for_configuration(
            "same-profile".into(),
            &LiveTranslationConfiguration::with_credentials(
                ProviderKind::AlibabaCloud,
                credentials,
                SourceLanguage::Automatic,
                TargetLanguage::Japanese,
                TranslationMode::Turbo,
            ),
        )
        .unwrap()
    };
    for chatmock_first in [false, true] {
        let first = scope(chatmock_first);
        let other = scope(!chatmock_first);
        assert!(first.route != other.route);
        let mut continuity = MTBudgetContinuity::default();
        assert!(continuity.prepare(1, Some(first.clone())).is_none());
        let (old_token, old_scope) = continuity.take_lease(1).unwrap();
        continuity.remember(
            old_token,
            old_scope,
            PreviewRequestPacer::default().export_budget(),
        );
        assert!(continuity.prepare(2, Some(other.clone())).is_none());
        continuity.remember(
            old_token,
            first,
            PreviewRequestPacer::default().export_budget(),
        );
        assert!(
            continuity.retained.is_none(),
            "old route teardown cannot restore its budget"
        );
        let (token, owned_scope) = continuity.take_lease(2).unwrap();
        continuity.remember(
            token,
            owned_scope,
            PreviewRequestPacer::default().export_budget(),
        );
        assert!(
            continuity.prepare(3, Some(other)).is_some(),
            "same route keeps its own continuation"
        );
    }
}

#[test]
fn pause_and_reconnect_keep_same_profile_budget_but_stop_rejects_late_teardown() {
    use crate::core::preview_pacing::{
        PreviewCandidate, PreviewRequestPacer, RATE_LIMIT_PREVIEW_PAUSE,
    };
    let now = Instant::now();
    let mut pacer = PreviewRequestPacer::default();
    pacer.record_candidate_start(
        now,
        PreviewCandidate::new("Synthetic request", Some("en"), 0),
    );
    pacer.suppress_after_rate_limit(now);
    pacer.set_shared_cooldown(Some(now + Duration::from_secs(8)));
    let mut continuity = MTBudgetContinuity::default();
    let scope = synthetic_mt_scope("fixture-profile", MTBudgetRoute::Qwen(REALTIME_MT_MODEL));
    assert!(continuity.prepare(7, Some(scope.clone())).is_none());
    let (token, owned_scope) = continuity.take_lease(7).unwrap();
    continuity.remember(token, owned_scope, pacer.export_budget());
    let restored = continuity.prepare(8, Some(scope.clone())).unwrap();
    let mut resumed = PreviewRequestPacer::default();
    resumed.restore_budget(restored);
    assert_eq!(resumed.suppression_remaining(now), RATE_LIMIT_PREVIEW_PAUSE);
    assert_eq!(
        resumed.next_shared_start_at(now),
        now + Duration::from_secs(8)
    );
    let (old_token, old_scope) = continuity.take_lease(8).unwrap();
    continuity.reset();
    continuity.remember(old_token, old_scope, restored);
    assert!(
        continuity.prepare(9, Some(scope)).is_none(),
        "late paused/recovery export must not undo explicit stop"
    );
}

#[test]
fn mt_budget_continuation_never_crosses_profiles_routes_models_or_new_client_tokens() {
    use crate::core::preview_pacing::PreviewRequestPacer;
    let scope = synthetic_mt_scope("fixture-profile", MTBudgetRoute::Qwen(REALTIME_MT_MODEL));
    for different in [
        synthetic_mt_scope("other-profile", MTBudgetRoute::Qwen(REALTIME_MT_MODEL)),
        synthetic_mt_scope("fixture-profile", MTBudgetRoute::DeepL),
        synthetic_mt_scope("fixture-profile", MTBudgetRoute::Qwen(QwenMTModel::Plus)),
    ] {
        let mut continuity = MTBudgetContinuity::default();
        continuity.prepare(7, Some(scope.clone()));
        let (token, owned_scope) = continuity.take_lease(7).unwrap();
        continuity.remember(
            token,
            owned_scope,
            PreviewRequestPacer::default().export_budget(),
        );
        assert!(continuity.prepare(8, Some(different)).is_none());
        continuity.remember(
            token,
            scope.clone(),
            PreviewRequestPacer::default().export_budget(),
        );
        assert!(
            continuity.retained.is_none(),
            "superseded old-client export must remain rejected"
        );
    }
}

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
