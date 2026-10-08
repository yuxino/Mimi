//! Shared generation ownership, recovery backoff and MT budget continuation.
//! Extracted from the desktop lifecycle; no capture, storage or window APIs.

use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::credentials::{ProviderCredentials, TextTranslationCredentials};
use crate::core::models::TranslationMode;
use crate::core::preview_pacing::MTRequestBudget;
use crate::core::protocols::qwen_mt::QwenMTModel;
use crate::core::provider::ProviderKind;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

pub const NO_GENERATION: u64 = 0;
pub const RECOVERY_ATTEMPTS: usize = 4;

/// Never forward arbitrary vendor strings as adapter error codes. Both native
/// drivers use the same authentication aliases and finite recovery codes.
pub fn normalize_provider_error_code(code: &str) -> &'static str {
    match code {
        "transport_error" => "transport_error",
        "provider_event_backlog_overflow" => "provider_event_backlog_overflow",
        "translation_backlog_overflow" => "translation_backlog_overflow",
        "translation_rate_limited" => "translation_rate_limited",
        "translation_temporarily_unavailable" => "translation_temporarily_unavailable",
        "invalid_api_key"
        | "authentication_error"
        | "unauthorized"
        | "translation_authentication_failed"
        | "credential_authentication_failed"
        | "custom_speech_authentication_failed" => "credential_authentication_failed",
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
                | "text_translation_credentials_missing"
                | "invalid_configuration"
        )
}

/// Shared recovery ownership and attempt schedule. Adapters supply connection,
/// capture and status effects; neither platform owns a second retry counter or
/// independently interprets a cancelled/failed connection generation.
pub struct RecoverySchedule {
    failed_generation: u64,
    minimum_delay: Duration,
    next_attempt_index: usize,
    generation: u64,
    epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryAttempt {
    pub index: usize,
    pub delay: Duration,
}

impl RecoverySchedule {
    pub fn new(failed_generation: u64, owner_generation: u64, minimum_delay: Duration) -> Self {
        Self {
            failed_generation,
            minimum_delay,
            next_attempt_index: 0,
            generation: owner_generation,
            epoch: owner_generation,
        }
    }

    pub fn next_attempt(&mut self) -> Option<RecoveryAttempt> {
        if self.next_attempt_index == RECOVERY_ATTEMPTS {
            return None;
        }
        let index = self.next_attempt_index;
        self.next_attempt_index += 1;
        Some(RecoveryAttempt {
            index,
            delay: recovery_delay(index, self.failed_generation).max(self.minimum_delay),
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

#[cfg(test)]
mod schedule_tests {
    use super::*;

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
}

#[derive(Clone, PartialEq, Eq)]
pub enum MTBudgetRoute {
    Apple,
    Qwen(QwenMTModel),
    DeepL,
    DeepLX,
    OpenAICompatible,
    ChatMock,
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
        let route = if matches!(
            configuration.text_credentials,
            Some(TextTranslationCredentials::Apple)
        ) {
            if !configuration.target_language.translates_audio() {
                return None;
            }
            MTBudgetRoute::Apple
        } else if configuration.provider.is_standalone_asr() {
            if !configuration.target_language.translates_audio() {
                return None;
            }
            match configuration.text_credentials.as_ref()? {
                TextTranslationCredentials::Apple => MTBudgetRoute::Apple,
                TextTranslationCredentials::DeepL { .. } => MTBudgetRoute::DeepL,
                TextTranslationCredentials::DeepLX { .. } => MTBudgetRoute::DeepLX,
                TextTranslationCredentials::OpenAICompatible { .. } => {
                    MTBudgetRoute::OpenAICompatible
                }
                TextTranslationCredentials::ChatMock { .. } => MTBudgetRoute::ChatMock,
            }
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
    matches!(
        code,
        "transport_error"
            | "provider_event_backlog_overflow"
            | "translation_backlog_overflow"
            | "translation_rate_limited"
            | "translation_temporarily_unavailable"
    )
}

pub fn provider_recovery_minimum_delay(code: &str) -> Duration {
    match code {
        // A new client must not bypass the MT cooldown by replacing the
        // generation after finite retries or bounded final-queue pressure.
        "translation_rate_limited" | "translation_backlog_overflow" => Duration::from_secs(8),
        "translation_temporarily_unavailable" => Duration::from_millis(600),
        _ => Duration::ZERO,
    }
}
