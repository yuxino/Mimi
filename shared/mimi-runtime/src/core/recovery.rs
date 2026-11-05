//! Shared generation ownership, recovery backoff and MT budget continuation.
//! Extracted from the desktop lifecycle; no capture, storage or window APIs.

use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::credentials::{ProviderCredentials, TextTranslationCredentials};
use crate::core::models::TranslationMode;
use crate::core::preview_pacing::MTRequestBudget;
use crate::core::protocols::qwen_mt::QwenMTModel;
use crate::core::provider::ProviderKind;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const NO_GENERATION: u64 = 0;
pub const RECOVERY_ATTEMPTS: usize = 4;

/// Never forward arbitrary vendor strings as adapter error codes. Both native
/// drivers use the same authentication aliases and finite recovery codes.
pub fn normalize_provider_error_code(code: &str) -> &'static str {
    match code {
        "transport_error"
        | "custom_speech_unreachable"
        | "custom_speech_not_connected"
        | "custom_speech_health_timeout"
        | "xai_stt_unreachable"
        | "xai_stt_not_connected"
        | "xai_stt_health_timeout"
        | "gemini_transcribe_unreachable"
        | "gemini_transcribe_not_connected"
        | "gemini_transcribe_health_timeout" => "transport_error",
        "provider_event_backlog_overflow" => "provider_event_backlog_overflow",
        "live_translate_unexpected_session_end" => "live_translate_unexpected_session_end",
        "alibaba_realtime_service_unavailable" => "alibaba_realtime_service_unavailable",
        "alibaba_realtime_idle_timeout" => "alibaba_realtime_idle_timeout",
        "gemini_live_service_unavailable" => "gemini_live_service_unavailable",
        "translation_backlog_overflow" => "translation_backlog_overflow",
        "translation_rate_limited" => "translation_rate_limited",
        "translation_temporarily_unavailable" => "translation_temporarily_unavailable",
        "apple_speech_runtime_service_unavailable" => "apple_speech_runtime_service_unavailable",
        "apple_speech_runtime_timeout" => "apple_speech_runtime_timeout",
        "apple_speech_stream_closed" => "apple_speech_stream_closed",
        "apple_speech_result_backlog" => "apple_speech_result_backlog",
        "invalid_api_key"
        | "authentication_error"
        | "unauthorized"
        | "translation_authentication_failed"
        | "credential_authentication_failed"
        | "custom_speech_authentication_failed"
        | "xai_stt_authentication_failed"
        | "gemini_transcribe_authentication_failed" => "credential_authentication_failed",
        "invalid_configuration" => "invalid_configuration",
        code if provider_error_requires_user_action(code) => "invalid_configuration",
        _ => "provider_error",
    }
}

/// Provider/configuration errors shared by native session adapters. Capture
/// permissions remain a platform effect and do not belong in this policy.
pub fn provider_error_requires_user_action(error: &str) -> bool {
    crate::core::protocols::audio3::failure_requires_configuration(error)
        || matches!(
            error,
            "apple_translation_unavailable"
                | "apple_translation_assets_missing"
                | "apple_translation_language_unsupported"
                | "apple_translation_invalid_input"
                | "credential_authentication_failed"
                | "custom_speech_authentication_failed"
                | "custom_speech_credentials_missing"
                | "custom_speech_endpoint_invalid"
                | "custom_speech_model_invalid"
                | "gemini_transcribe_authentication_failed"
                | "gemini_transcribe_credentials_missing"
                | "gemini_transcribe_language_unsupported"
                | "xai_stt_authentication_failed"
                | "xai_stt_credentials_missing"
                | "xai_stt_language_unsupported"
                | "text_translation_credentials_missing"
                | "tencent_configuration_rejected"
                | "tencent_service_activation_required"
                | "tencent_quota_exhausted"
                | "invalid_configuration"
        )
}

/// Shared recovery ownership and attempt schedule. Adapters supply connection,
/// capture and status effects; neither platform owns a second retry counter or
/// independently interprets a cancelled/failed connection generation.
pub struct RecoverySchedule {
    failed_generation: u64,
    minimum_delay: Duration,
    used_attempts_before_owner: usize,
    next_attempt_index: usize,
    generation: u64,
    epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryAttempt {
    /// Index within this native recovery owner; zero uses its reserved generation.
    pub index: usize,
    /// Index across connections that have not yet passed a stable health check.
    pub budget_index: usize,
    pub delay: Duration,
}

impl RecoverySchedule {
    pub fn new(failed_generation: u64, owner_generation: u64, minimum_delay: Duration) -> Self {
        Self {
            failed_generation,
            minimum_delay,
            used_attempts_before_owner: 0,
            next_attempt_index: 0,
            generation: owner_generation,
            epoch: owner_generation,
        }
    }

    pub fn next_attempt(&mut self) -> Option<RecoveryAttempt> {
        let budget_index = self.used_attempts_before_owner + self.next_attempt_index;
        if budget_index >= RECOVERY_ATTEMPTS {
            return None;
        }
        let index = self.next_attempt_index;
        self.next_attempt_index += 1;
        Some(RecoveryAttempt {
            index,
            budget_index,
            delay: recovery_delay(budget_index, self.failed_generation).max(self.minimum_delay),
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn owns_generation(&self, current_epoch: u64) -> bool {
        self.generation == current_epoch
    }

    /// Called for attempts after the initial one, just as the desktop loop
    /// advances its expected epoch before establishing the replacement client.
    pub fn advance_for_retry(&mut self, sequence: &AtomicU64) -> Option<u64> {
        self.generation = advance_lifecycle_sequence_if_current(sequence, self.epoch)?;
        Some(self.generation)
    }

    pub fn accept_cancelled_attempt(
        &mut self,
        retry_generation: u64,
        current_epoch: u64,
        has_active_settings: bool,
    ) -> bool {
        if !cancelled_recovery_attempt_is_retryable(
            retry_generation,
            self.generation,
            current_epoch,
            has_active_settings,
        ) {
            return false;
        }
        self.epoch = current_epoch;
        true
    }

    pub fn accept_failure(&mut self, current_epoch: u64) -> bool {
        let failure_epoch = self.generation.wrapping_add(1);
        if current_epoch != failure_epoch {
            return false;
        }
        self.epoch = failure_epoch;
        true
    }

    pub fn owns_exhaustion(&self, current_epoch: u64, active_generation: u64) -> bool {
        recovery_exhaustion_is_still_owned(self.epoch, current_epoch, active_generation)
    }
}

/// A successful health probe only proves stability for the connection it
/// measured, after the existing full health interval has elapsed.
pub fn connection_health_is_stable(
    connected_generation: u64,
    probe_generation: u64,
    connected_at: Instant,
    now: Instant,
) -> bool {
    connected_generation == probe_generation
        && now.saturating_duration_since(connected_at) >= crate::core::health::HEALTH_CHECK_INTERVAL
}

/// Moves the same retry budget between native recovery owners. A setup
/// acknowledgement is not evidence that a replacement connection is stable.
/// Adapters retain ownership checks and report an actual successful health
/// probe; only this shared policy decides when the budget may reset.
#[derive(Default)]
pub struct RecoveryContinuity {
    connected: Option<ConnectedRecovery>,
}

struct ConnectedRecovery {
    schedule: RecoverySchedule,
    connected_at: Instant,
}

impl RecoveryContinuity {
    /// The caller must still own the successful schedule's generation.
    pub fn record_connected(&mut self, schedule: RecoverySchedule, now: Instant) {
        self.connected = Some(ConnectedRecovery {
            schedule,
            connected_at: now,
        });
    }

    pub fn health_succeeded(&mut self, generation: u64, now: Instant) -> bool {
        let stable = self.connected.as_ref().is_some_and(|connected| {
            connection_health_is_stable(
                connected.schedule.generation(),
                generation,
                connected.connected_at,
                now,
            )
        });
        if stable {
            self.clear();
        }
        stable
    }

    pub fn take_for_failure(
        &mut self,
        failed_generation: u64,
        owner_generation: u64,
        minimum_delay: Duration,
    ) -> RecoverySchedule {
        let Some(connected) = self
            .connected
            .take()
            .filter(|connected| connected.schedule.generation() == failed_generation)
        else {
            return RecoverySchedule::new(failed_generation, owner_generation, minimum_delay);
        };
        let mut schedule = connected.schedule;
        schedule.used_attempts_before_owner += schedule.next_attempt_index;
        schedule.next_attempt_index = 0;
        schedule.minimum_delay = schedule.minimum_delay.max(minimum_delay);
        schedule.generation = owner_generation;
        schedule.epoch = owner_generation;
        schedule
    }

    /// Explicit Stop, Start/Retry and configuration replacement own a new budget.
    pub fn clear(&mut self) {
        self.connected = None;
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;

    #[test]
    fn shared_continuity_contract_bounds_connections_and_allows_a_healthy_new_window() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../translation-contracts.json")).unwrap();
        for case in fixtures["liveTranslateRecoveryContinuity"]
            .as_array()
            .unwrap()
        {
            let mut continuity = RecoveryContinuity::default();
            let now = Instant::now();
            let mut generation = 1;
            loop {
                let mut schedule =
                    continuity.take_for_failure(generation, generation + 1, Duration::ZERO);
                if schedule.next_attempt().is_none() {
                    break;
                }
                generation += 1;
                assert!(generation < 16, "a recovery budget must terminate");
                continuity.record_connected(schedule, now);
                if case["stableConnection"].as_u64() == Some(generation) {
                    assert!(continuity.health_succeeded(
                        generation,
                        now + crate::core::health::HEALTH_CHECK_INTERVAL
                    ));
                }
            }
            assert_eq!(
                generation,
                case["expectedConnections"].as_u64().unwrap(),
                "{}",
                case["id"]
            );
        }
    }

    #[test]
    fn setup_acknowledgements_cannot_reset_the_shared_recovery_budget_or_backoff() {
        let mut continuity = RecoveryContinuity::default();
        let now = Instant::now();
        let failed_generation = 10;
        for budget_index in 0..RECOVERY_ATTEMPTS {
            let generation = failed_generation + budget_index as u64;
            let mut schedule =
                continuity.take_for_failure(generation, generation + 1, Duration::ZERO);
            let attempt = schedule.next_attempt().unwrap();
            assert_eq!(
                attempt.index, 0,
                "each native owner uses its reserved generation first"
            );
            assert_eq!(attempt.budget_index, budget_index);
            assert_eq!(
                attempt.delay,
                recovery_delay(budget_index, failed_generation)
            );
            continuity.record_connected(schedule, now);
        }
        let generation = failed_generation + RECOVERY_ATTEMPTS as u64;
        assert!(continuity
            .take_for_failure(generation, generation + 1, Duration::ZERO)
            .next_attempt()
            .is_none());
    }

    #[test]
    fn only_a_current_successful_health_probe_after_the_shared_interval_resets_recovery() {
        let now = Instant::now();
        let interval = crate::core::health::HEALTH_CHECK_INTERVAL;
        let mut continuity = RecoveryContinuity::default();
        let mut schedule = continuity.take_for_failure(1, 2, Duration::ZERO);
        schedule.next_attempt().unwrap();
        continuity.record_connected(schedule, now);
        assert!(!continuity.health_succeeded(2, now + interval - Duration::from_nanos(1)));
        assert!(!continuity.health_succeeded(1, now + interval));
        let mut resumed = continuity.take_for_failure(2, 3, Duration::ZERO);
        assert_eq!(resumed.next_attempt().unwrap().budget_index, 1);
        continuity.record_connected(resumed, now + interval);
        assert!(!continuity.health_succeeded(2, now + interval * 2));
        assert!(continuity.health_succeeded(3, now + interval * 2));
        let mut fresh = continuity.take_for_failure(3, 4, Duration::ZERO);
        for budget_index in 0..RECOVERY_ATTEMPTS {
            assert_eq!(fresh.next_attempt().unwrap().budget_index, budget_index);
        }
        assert!(fresh.next_attempt().is_none());
    }

    #[test]
    fn elapsed_time_without_successful_health_does_not_reset_recovery() {
        let mut continuity = RecoveryContinuity::default();
        let now = Instant::now();
        let mut schedule = continuity.take_for_failure(1, 2, Duration::ZERO);
        schedule.next_attempt().unwrap();
        continuity.record_connected(schedule, now);
        assert!(!continuity.health_succeeded(1, now + Duration::from_secs(900)));
        assert_eq!(
            continuity
                .take_for_failure(2, 3, Duration::ZERO)
                .next_attempt()
                .unwrap()
                .budget_index,
            1
        );
    }

    #[test]
    fn continued_owners_count_failed_setup_attempts_and_preserve_service_cooldown() {
        let mut continuity = RecoveryContinuity::default();
        let mut schedule = continuity.take_for_failure(20, 21, Duration::from_secs(8));
        schedule.next_attempt().unwrap();
        assert!(schedule.accept_failure(22));
        let sequence = AtomicU64::new(22);
        assert_eq!(schedule.advance_for_retry(&sequence), Some(23));
        assert_eq!(schedule.next_attempt().unwrap().budget_index, 1);
        continuity.record_connected(schedule, Instant::now());
        let mut resumed = continuity.take_for_failure(23, 24, Duration::ZERO);
        let first = resumed.next_attempt().unwrap();
        assert_eq!(first.index, 0);
        assert_eq!(first.budget_index, 2);
        assert_eq!(first.delay, Duration::from_secs(8));
        assert!(resumed.accept_failure(25));
        let sequence = AtomicU64::new(25);
        assert_eq!(resumed.advance_for_retry(&sequence), Some(26));
        let second = resumed.next_attempt().unwrap();
        assert_eq!(second.index, 1);
        assert_eq!(second.budget_index, 3);
        assert!(resumed.next_attempt().is_none());
    }

    #[test]
    fn explicit_reset_and_replacement_owners_cannot_inherit_an_old_retry_budget() {
        for reset in [false, true] {
            let mut continuity = RecoveryContinuity::default();
            let mut schedule = continuity.take_for_failure(1, 2, Duration::ZERO);
            for _ in 0..RECOVERY_ATTEMPTS {
                schedule.next_attempt().unwrap();
            }
            continuity.record_connected(schedule, Instant::now());
            let failed_generation = if reset {
                continuity.clear();
                2
            } else {
                9
            };
            assert_eq!(
                continuity
                    .take_for_failure(failed_generation, failed_generation + 1, Duration::ZERO)
                    .next_attempt()
                    .unwrap()
                    .budget_index,
                0
            );
        }
    }

    #[test]
    fn recognition_recovery_contract_preserves_raw_and_normalized_decisions() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../translation-contracts.json")).unwrap();
        let cases = fixtures["recognitionRecoveryErrors"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let id = case["id"].as_str().unwrap();
            let code = case["code"].as_str().unwrap();
            let normalized = normalize_provider_error_code(code);
            assert_eq!(normalized, case["normalized"].as_str().unwrap(), "{id}");
            for code in [code, normalized] {
                assert_eq!(
                    provider_error_is_retryable(code),
                    case["retryable"].as_bool().unwrap(),
                    "{id}: {code}"
                );
            }
            assert_eq!(
                provider_error_requires_user_action(code),
                case["requiresUserAction"].as_bool().unwrap(),
                "{id}"
            );
        }
    }

    #[test]
    fn independent_asr_transport_expiry_recovers_but_bad_credentials_do_not() {
        for code in [
            "custom_speech_unreachable",
            "custom_speech_health_timeout",
            "gemini_transcribe_unreachable",
            "gemini_transcribe_health_timeout",
        ] {
            assert_eq!(normalize_provider_error_code(code), "transport_error");
            assert!(provider_error_is_retryable(normalize_provider_error_code(
                code
            )));
        }
        assert_eq!(
            normalize_provider_error_code("gemini_transcribe_authentication_failed"),
            "credential_authentication_failed"
        );
        for code in [
            "gemini_transcribe_authentication_failed",
            "gemini_transcribe_credentials_missing",
            "gemini_transcribe_language_unsupported",
        ] {
            assert!(provider_error_requires_user_action(code));
            assert!(!provider_error_is_retryable(normalize_provider_error_code(
                code
            )));
        }
    }

    #[test]
    fn raw_independent_asr_errors_use_the_same_recovery_policy_as_normalized_events() {
        for code in [
            "custom_speech_unreachable",
            "custom_speech_not_connected",
            "custom_speech_health_timeout",
            "gemini_transcribe_unreachable",
            "gemini_transcribe_not_connected",
            "gemini_transcribe_health_timeout",
        ] {
            assert!(provider_error_is_retryable(code), "{code}");
            let normalized = normalize_provider_error_code(code);
            assert!(provider_error_is_retryable(normalized), "{code}");
            assert_eq!(normalize_provider_error_code(normalized), normalized);
        }
        for code in [
            "custom_speech_authentication_failed",
            "custom_speech_credentials_missing",
            "custom_speech_endpoint_invalid",
            "custom_speech_model_invalid",
            "custom_speech_protocol_invalid",
            "gemini_transcribe_authentication_failed",
            "gemini_transcribe_credentials_missing",
            "gemini_transcribe_language_unsupported",
            "gemini_transcribe_protocol_invalid",
            "unrecognized_provider_error",
        ] {
            assert!(!provider_error_is_retryable(code), "{code}");
            let normalized = normalize_provider_error_code(code);
            assert!(!provider_error_is_retryable(normalized), "{code}");
            assert_eq!(normalize_provider_error_code(normalized), normalized);
        }
    }

    #[test]
    fn provider_error_codes_share_authentication_aliases_and_never_expose_vendor_text() {
        for code in [
            "invalid_api_key",
            "authentication_error",
            "unauthorized",
            "translation_authentication_failed",
            "custom_speech_authentication_failed",
            "credential_authentication_failed",
        ] {
            assert_eq!(
                normalize_provider_error_code(code),
                "credential_authentication_failed"
            );
        }
        for code in [
            "transport_error",
            "provider_event_backlog_overflow",
            "translation_backlog_overflow",
            "translation_rate_limited",
            "translation_temporarily_unavailable",
        ] {
            assert_eq!(normalize_provider_error_code(code), code);
            assert!(provider_error_is_retryable(normalize_provider_error_code(
                code
            )));
        }
        assert_eq!(
            normalize_provider_error_code("custom_speech_endpoint_invalid"),
            "invalid_configuration"
        );
        assert_eq!(
            normalize_provider_error_code("vendor supplied code with endpoint or content"),
            "provider_error"
        );
        assert!(!provider_error_is_retryable(normalize_provider_error_code(
            "unauthorized"
        )));
    }

    #[test]
    fn shared_schedule_is_bounded_preserves_service_cooldown_and_rejects_new_owner() {
        let sequence = AtomicU64::new(41);
        let mut recovery = RecoverySchedule::new(40, 41, Duration::from_secs(8));
        for index in 0..RECOVERY_ATTEMPTS {
            let attempt = recovery.next_attempt().unwrap();
            assert_eq!(attempt.index, index);
            assert!(attempt.delay >= Duration::from_secs(8));
            if index > 0 {
                assert_eq!(
                    recovery.advance_for_retry(&sequence),
                    Some(41 + index as u64 * 2)
                );
            }
            assert!(recovery.owns_generation(sequence.load(Ordering::SeqCst)));
            sequence.fetch_add(1, Ordering::SeqCst);
            assert!(recovery.accept_failure(sequence.load(Ordering::SeqCst)));
        }
        assert!(recovery.next_attempt().is_none());
        assert!(recovery.owns_exhaustion(sequence.load(Ordering::SeqCst), NO_GENERATION));
        sequence.fetch_add(1, Ordering::SeqCst);
        assert!(!recovery.owns_exhaustion(sequence.load(Ordering::SeqCst), NO_GENERATION));
        assert!(recovery.advance_for_retry(&sequence).is_none());
    }

    #[test]
    fn only_the_current_failed_attempt_can_continue_after_cancellation() {
        let mut recovery = RecoverySchedule::new(8, 9, Duration::ZERO);
        assert_eq!(recovery.next_attempt().unwrap().delay, Duration::ZERO);
        assert!(!recovery.accept_cancelled_attempt(8, 10, true));
        assert!(!recovery.accept_cancelled_attempt(9, 11, true));
        assert!(!recovery.accept_cancelled_attempt(9, 10, false));
        assert!(recovery.accept_cancelled_attempt(9, 10, true));
        assert_eq!(recovery.epoch(), 10);
        let sequence = AtomicU64::new(10);
        assert_eq!(recovery.advance_for_retry(&sequence), Some(11));
        assert!(!recovery.accept_failure(13));
        assert!(recovery.accept_failure(12));
    }

    #[test]
    fn apple_runtime_recovery_is_cooldown_bounded_and_excludes_setup_or_unknown_errors() {
        for code in [
            "apple_speech_runtime_service_unavailable",
            "apple_speech_runtime_timeout",
            "apple_speech_stream_closed",
            "apple_speech_result_backlog",
        ] {
            assert_eq!(normalize_provider_error_code(code), code);
            assert!(provider_error_is_retryable(code));
            let delay = provider_recovery_minimum_delay(code);
            assert_eq!(delay, Duration::from_millis(600));
            let mut schedule = RecoverySchedule::new(1, 2, delay);
            for _ in 0..RECOVERY_ATTEMPTS {
                assert!(schedule.next_attempt().unwrap().delay >= delay);
            }
            assert!(schedule.next_attempt().is_none());
        }
        for code in [
            "apple_speech_service_unavailable",
            "apple_speech_setup_timeout",
            "apple_speech_recognition_failed",
            "apple_speech_invalid_result",
            "apple_speech_audio_failed",
            "apple_speech_finalize_timeout",
            "apple_speech_not_connected",
        ] {
            assert!(!provider_error_is_retryable(code));
            assert!(!provider_error_is_retryable(normalize_provider_error_code(
                code
            )));
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum MTBudgetRoute {
    Apple,
    Qwen(QwenMTModel),
    DeepL,
    DeepLX,
    OpenAICompatible,
    ChatMock,
    XAI,
}

#[derive(Clone, PartialEq, Eq)]
pub struct MTBudgetScope {
    pub profile_id: String,
    pub provider: ProviderKind,
    pub route: MTBudgetRoute,
}

impl MTBudgetScope {
    pub fn for_configuration(
        profile_id: String,
        configuration: &LiveTranslationConfiguration,
    ) -> Option<Self> {
        let route = if let Some(credentials) = &configuration.text_credentials {
            if !configuration.target_language.translates_audio() {
                return None;
            }
            match credentials {
                TextTranslationCredentials::Apple => MTBudgetRoute::Apple,
                TextTranslationCredentials::DeepL { .. } => MTBudgetRoute::DeepL,
                TextTranslationCredentials::DeepLX { .. } => MTBudgetRoute::DeepLX,
                TextTranslationCredentials::OpenAICompatible { .. } => {
                    MTBudgetRoute::OpenAICompatible
                }
                TextTranslationCredentials::ChatMock { .. } => MTBudgetRoute::ChatMock,
                TextTranslationCredentials::XAI { .. } => MTBudgetRoute::XAI,
            }
        } else if configuration.is_standalone_asr() {
            return None;
        } else {
            match &configuration.credentials {
                ProviderCredentials::DeepL { .. } => MTBudgetRoute::DeepL,
                ProviderCredentials::DeepLX { .. } => MTBudgetRoute::DeepLX,
                ProviderCredentials::OpenAICompatible { .. } => MTBudgetRoute::OpenAICompatible,
                ProviderCredentials::ChatMock { .. } => MTBudgetRoute::ChatMock,
                ProviderCredentials::ApiKey { .. }
                    if configuration.provider == ProviderKind::AlibabaCloud =>
                {
                    let model = match configuration.effective_translation_mode() {
                        TranslationMode::Turbo => configuration.qwen_mt_model,
                        TranslationMode::HighQuality => QwenMTModel::Plus,
                        TranslationMode::LowLatency => return None,
                    };
                    MTBudgetRoute::Qwen(model)
                }
                _ => return None,
            }
        };
        Some(Self {
            profile_id,
            provider: configuration.provider,
            route,
        })
    }
}

/// One bounded continuation slot. Its token rejects late teardown snapshots
/// after stop/new-start or after another client/route has already been prepared.
#[derive(Default)]
pub struct MTBudgetContinuity {
    pub token: u64,
    pub client_scope: Option<(u64, u64, MTBudgetScope)>,
    pub retained: Option<(MTBudgetScope, MTRequestBudget)>,
}

impl MTBudgetContinuity {
    pub fn reset(&mut self) {
        self.token = self.token.wrapping_add(1).max(1);
        self.client_scope = None;
        self.retained = None;
    }

    pub fn prepare(
        &mut self,
        generation: u64,
        scope: Option<MTBudgetScope>,
    ) -> Option<MTRequestBudget> {
        let Some(scope) = scope else {
            self.reset();
            return None;
        };
        if self
            .retained
            .as_ref()
            .is_some_and(|(previous, _)| previous != &scope)
        {
            self.retained = None;
        }
        self.token = self.token.wrapping_add(1).max(1);
        self.client_scope = Some((generation, self.token, scope.clone()));
        self.retained
            .as_ref()
            .filter(|(previous, _)| previous == &scope)
            .map(|(_, budget)| *budget)
    }

    pub fn take_lease(&mut self, generation: u64) -> Option<(u64, MTBudgetScope)> {
        if self
            .client_scope
            .as_ref()
            .is_none_or(|(owner, _, _)| *owner != generation)
        {
            return None;
        }
        self.client_scope
            .take()
            .map(|(_, token, scope)| (token, scope))
    }

    pub fn remember(&mut self, token: u64, scope: MTBudgetScope, budget: MTRequestBudget) {
        if token == self.token {
            self.retained = Some((scope, budget));
        }
    }
}

pub fn advance_lifecycle_sequence_if_current(
    lifecycle_sequence: &AtomicU64,
    expected: u64,
) -> Option<u64> {
    let mut next = expected.wrapping_add(1);
    if next == NO_GENERATION {
        next = 1;
    }
    lifecycle_sequence
        .compare_exchange(expected, next, Ordering::SeqCst, Ordering::SeqCst)
        .ok()
        .map(|_| next)
}

pub fn lifecycle_sequence_matches(lifecycle_sequence: &AtomicU64, expected: u64) -> bool {
    lifecycle_sequence.load(Ordering::SeqCst) == expected
}

pub fn cancelled_recovery_attempt_is_retryable(
    retry_generation: u64,
    attempt_generation: u64,
    current_epoch: u64,
    has_active_settings: bool,
) -> bool {
    retry_generation == attempt_generation
        && current_epoch == attempt_generation.wrapping_add(1)
        && has_active_settings
}

pub fn recovery_exhaustion_is_still_owned(
    recovery_epoch: u64,
    current_epoch: u64,
    active_generation: u64,
) -> bool {
    recovery_epoch == current_epoch && active_generation == NO_GENERATION
}

pub fn clear_recovery_atoms(is_recovering: &AtomicBool, retry_generation: &AtomicU64) {
    retry_generation.store(NO_GENERATION, Ordering::SeqCst);
    is_recovering.store(false, Ordering::SeqCst);
}

/// Bounded exponential recovery delay with deterministic per-generation
/// jitter. Determinism keeps lifecycle tests reliable while preventing two
/// mimi instances from reconnecting in lockstep after a shared outage.
pub fn recovery_delay(attempt: usize, generation: u64) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    let exponent = u32::try_from(attempt.saturating_sub(1)).unwrap_or(u32::MAX);
    let base_ms = 500_u64.saturating_mul(2_u64.saturating_pow(exponent.min(3)));
    let mixed = generation
        .wrapping_add((attempt as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let jitter_ms = mixed % (base_ms / 4 + 1);
    Duration::from_millis(base_ms + jitter_ms)
}

pub fn provider_error_is_retryable(code: &str) -> bool {
    // Desktop receives provider labels directly while Android supplies their
    // normalized form. Both adapters must make the same recovery decision.
    matches!(
        normalize_provider_error_code(code),
        "transport_error"
            | "provider_event_backlog_overflow"
            | "live_translate_unexpected_session_end"
            | "alibaba_realtime_service_unavailable"
            | "alibaba_realtime_idle_timeout"
            | "gemini_live_service_unavailable"
            | "translation_backlog_overflow"
            | "translation_rate_limited"
            | "translation_temporarily_unavailable"
            | "apple_speech_runtime_service_unavailable"
            | "apple_speech_runtime_timeout"
            | "apple_speech_stream_closed"
            | "apple_speech_result_backlog"
    )
}

pub fn provider_recovery_minimum_delay(code: &str) -> Duration {
    match code {
        // A new client must not bypass the MT cooldown by replacing the
        // generation after finite retries or bounded final-queue pressure.
        "translation_rate_limited" | "translation_backlog_overflow" => Duration::from_secs(8),
        "translation_temporarily_unavailable"
        | "live_translate_unexpected_session_end"
        | "alibaba_realtime_idle_timeout"
        | "gemini_live_service_unavailable"
        | "alibaba_realtime_service_unavailable" => Duration::from_millis(600),
        "apple_speech_runtime_service_unavailable"
        | "apple_speech_runtime_timeout"
        | "apple_speech_stream_closed"
        | "apple_speech_result_backlog" => Duration::from_millis(600),
        _ => Duration::ZERO,
    }
}

#[cfg(test)]
mod independent_budget_tests {
    use super::*;
    use crate::core::models::{SourceLanguage, TargetLanguage};
    use crate::core::provider::XAISpeechMode;
    #[test]
    fn xai_asr_budget_uses_the_selected_text_route_and_original_has_no_budget() {
        let configuration = LiveTranslationConfiguration::for_provider(
            ProviderKind::XAIRealtime,
            "speech-key",
            SourceLanguage::English,
            TargetLanguage::Japanese,
            TranslationMode::Turbo,
        )
        .with_xai_speech(XAISpeechMode::AsrMt, "legacy-unused");
        assert!(MTBudgetScope::for_configuration("profile".into(), &configuration).is_none());
        for (credentials, route) in [
            (
                TextTranslationCredentials::DeepL {
                    api_key: "text:fx".into(),
                },
                MTBudgetRoute::DeepL,
            ),
            (
                TextTranslationCredentials::OpenAICompatible {
                    endpoint: "http://127.0.0.1:1".into(),
                    model: "model".into(),
                    api_key: "text".into(),
                },
                MTBudgetRoute::OpenAICompatible,
            ),
            (
                TextTranslationCredentials::XAI {
                    api_key: "text".into(),
                    model: "grok".into(),
                },
                MTBudgetRoute::XAI,
            ),
        ] {
            let mut configuration = configuration.clone().with_text_credentials(credentials);
            assert!(
                MTBudgetScope::for_configuration("profile".into(), &configuration)
                    .unwrap()
                    .route
                    == route
            );
            configuration.target_language = TargetLanguage::Original;
            assert!(MTBudgetScope::for_configuration("profile".into(), &configuration).is_none());
        }
    }
}
