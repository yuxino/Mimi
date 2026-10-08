//! Session lifecycle: start/stop/pause/resume, capability-aware switching,
//! health checks, automatic reconnection, and state-event broadcasting.
//!
//! The manager is always shared behind `Arc<SessionManager>`; spawned tasks
//! hold clones of the same Arc so they observe one piece of session state.

use crate::audio::echo_pipeline::EchoPipeline;
use crate::audio::send_pipeline::{AudioPipelineFailure, AudioSendPipeline};
use crate::audio::{
    AudioCapture, AudioCaptureFormat, CaptureFailureSender, SystemAudioCaptureFailure,
};
use crate::clients::provider_events::{provider_event_channel, ProviderEvent};
use crate::clients::translation_client::TranslationClient;
use crate::core::audio_input::{AudioInput, AudioSource};
use crate::core::configuration::LiveTranslationConfiguration;
use crate::core::credentials::{ProviderCredentials, TextTranslationCredentials};
use crate::core::diagnostics::{
    milliseconds, TranslationLatency, TranslationLatencyKind, TranslationRecovery,
};
use crate::core::models::{SessionStatus, SourceLanguage, TranslationMode, UtteranceRole};
use crate::core::preview_pacing::MTRequestBudget;
use crate::core::protocols::live_translate::LiveTranslateServerEvent;
use crate::core::protocols::qwen_mt::QwenMTModel;
#[cfg(test)]
use crate::core::protocols::qwen_mt::REALTIME_MT_MODEL;
use crate::core::provider::ProviderKind;
use crate::core::session::{TranslationSessionController, TranslationSessionState};
use crate::core::support_diagnostics::{
    Availability, CaptureObservation, DiagnosticEvent, DiagnosticFacts, DiagnosticJournal,
    DiagnosticStatus, LifecycleAction, OutputSelection, RecoveryAction, SafeFailure, TextEventKind,
};
use crate::core::system_audio_target::SystemAudioTarget;
use crate::pipeline_log;
use crate::session_history::SessionHistory;
use crate::settings_store::SettingsStore;
use crate::windows::OverlayWindowManager;
use serde::Serialize;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex as TokioMutex, Notify, OwnedMutexGuard};
use tokio::task::JoinHandle;

/// UI-only buffers contain synthetic fixtures. An explicit export fixture keeps
/// them in memory after stop so QA can reach the real picker and write path.
/// This helper is never used by production history finalization.
fn finish_ui_test_archive(
    controller: &mut TranslationSessionController,
    recording: &mut crate::core::session_archive::AudioRecording,
    retain_for_export: bool,
) {
    if !retain_for_export {
        controller.archive_mut().clear();
        recording.clear();
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StatusPayload {
    Idle,
    Connecting,
    Listening,
    Stopping,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStateEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_snapshot_id: Option<u64>,
    pub status: StatusPayload,
    #[serde(rename = "isActive")]
    pub is_active: bool,
    #[serde(rename = "isPaused")]
    pub is_paused: bool,
    #[serde(rename = "isOverlayCollapsed")]
    pub is_overlay_collapsed: bool,
    pub subtitles: crate::core::models::SubtitleSnapshot,
    #[serde(rename = "detectedLanguage")]
    pub detected_language: Option<String>,
    #[serde(rename = "isTranslationPending")]
    pub is_translation_pending: bool,
    pub is_translation_preview_pending: bool,
    #[serde(rename = "isTranslationTimedOut")]
    pub is_translation_timed_out: bool,
    /// Existing WebSocket health-check round trip, including local send wait.
    /// HQ/DeepLX probe their ASR socket, not the text-translation HTTP service.
    pub api_latency_ms: Option<u64>,
    /// Latest current successful preview/final; the kind identifies boundaries.
    pub translation_latency_ms: Option<u64>,
    pub translation_latency_kind: Option<TranslationLatencyKind>,
    pub translation_recovery: Option<TranslationRecovery>,
}

#[derive(Clone, Copy)]
struct HealthCheckLatency {
    generation: u64,
    task_id: u64,
    milliseconds: u64,
}

fn visible_session_latencies(
    status: &SessionStatus,
    paused: bool,
    recovering: bool,
    generation: u64,
    health_task_id: u64,
    health_latency: Option<HealthCheckLatency>,
    translation_latency: Option<TranslationLatency>,
) -> (Option<u64>, Option<u64>, Option<TranslationLatencyKind>) {
    if !matches!(status, SessionStatus::Listening)
        || paused
        || recovering
        || generation == NO_GENERATION
    {
        return (None, None, None);
    }
    let api_latency = health_latency
        .filter(|sample| {
            sample.generation == generation
                && sample.task_id == health_task_id
                && health_task_id != NO_GENERATION
        })
        .map(|sample| sample.milliseconds);
    (
        api_latency,
        translation_latency.map(|sample| sample.milliseconds),
        translation_latency.map(|sample| sample.kind),
    )
}

const NO_GENERATION: u64 = 0;
const SESSION_START_CANCELLED: &str = "The session start was superseded by a newer request.";
const RECOVERY_ATTEMPTS: usize = 4;

#[derive(Clone, PartialEq, Eq)]
enum MTBudgetRoute {
    Apple,
    Qwen(QwenMTModel),
    DeepL,
    DeepLX,
    OpenAICompatible,
    ChatMock,
}

#[derive(Clone, PartialEq, Eq)]
struct MTBudgetScope {
    profile_id: String,
    provider: ProviderKind,
    route: MTBudgetRoute,
}

impl MTBudgetScope {
    fn for_configuration(
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
struct MTBudgetContinuity {
    token: u64,
    client_scope: Option<(u64, u64, MTBudgetScope)>,
    retained: Option<(MTBudgetScope, MTRequestBudget)>,
}

impl MTBudgetContinuity {
    fn reset(&mut self) {
        self.token = self.token.wrapping_add(1).max(1);
        self.client_scope = None;
        self.retained = None;
    }

    fn prepare(
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

    fn take_lease(&mut self, generation: u64) -> Option<(u64, MTBudgetScope)> {
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

    fn remember(&mut self, token: u64, scope: MTBudgetScope, budget: MTRequestBudget) {
        if token == self.token {
            self.retained = Some((scope, budget));
        }
    }
}

struct LifecycleOperationGuard {
    count: Arc<AtomicUsize>,
}

impl Drop for LifecycleOperationGuard {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::SeqCst);
    }
}

struct StartRequestGuard {
    in_progress: Arc<AtomicBool>,
}

struct TeardownOperationGuard {
    count: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

impl Drop for TeardownOperationGuard {
    fn drop(&mut self) {
        if self.count.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.notify.notify_waiters();
        }
    }
}

fn translation_mode_after_source_switch(
    _provider: ProviderKind,
    _source_language: SourceLanguage,
    _current_mode: TranslationMode,
) -> TranslationMode {
    TranslationMode::Turbo
}

fn pipeline_settings_mutation_is_allowed(
    status: &SessionStatus,
    lifecycle_operations: usize,
) -> bool {
    lifecycle_operations == 0
        && !matches!(status, SessionStatus::Connecting | SessionStatus::Stopping)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileSwitchAction {
    SelectOnly,
    KeepPaused,
    Reconnect,
}

fn profile_switch_action(
    status: &SessionStatus,
    paused: bool,
    recovering: bool,
    lifecycle_operations: usize,
) -> Result<ProfileSwitchAction, &'static str> {
    if recovering || !pipeline_settings_mutation_is_allowed(status, lifecycle_operations) {
        return Err("profile_switch_busy");
    }
    if paused {
        Ok(ProfileSwitchAction::KeepPaused)
    } else if matches!(status, SessionStatus::Listening) {
        Ok(ProfileSwitchAction::Reconnect)
    } else {
        Ok(ProfileSwitchAction::SelectOnly)
    }
}

fn ensure_profile_switch_audio_format(
    recording: bool,
    current_sample_rate: u32,
    next_sample_rate: u32,
) -> Result<(), String> {
    if recording && current_sample_rate != next_sample_rate {
        Err("profile_switch_recording_requires_stop".into())
    } else {
        Ok(())
    }
}

/// Caller holds the lifecycle and generation-transition gates. Persist before
/// replacing the resumable configuration, retaining its current owner (which
/// can be NO_GENERATION after a failed resume leaves the session paused).
fn commit_profile_selection(
    lifecycle_sequence: &AtomicU64,
    expected_epoch: u64,
    active_settings: &Mutex<Option<LiveTranslationConfiguration>>,
    proposed: Option<LiveTranslationConfiguration>,
    select: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if !lifecycle_sequence_matches(lifecycle_sequence, expected_epoch) {
        return Err("profile_switch_superseded".into());
    }
    select()?;
    if let Some(proposed) = proposed {
        *active_settings.lock().unwrap() = Some(proposed);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioInputSwitchAction {
    ReconfigureOnly,
    Reconnect,
}

fn validate_system_audio_target(
    target: &SystemAudioTarget,
    ui_test: bool,
    application_capture_supported: impl FnOnce() -> bool,
) -> Result<(), String> {
    if !target.validate() {
        return Err("application_audio_invalid_target".into());
    }
    if target.application_id().is_some() && !ui_test && !application_capture_supported() {
        return Err(crate::audio::SystemAudioCaptureError::ApplicationUnsupported.to_string());
    }
    Ok(())
}

/// Selecting an application while only the microphone is enabled is a future
/// system-lane preference; it must not interrupt the unrelated microphone.
fn audio_capture_selection_needs_restart(previous: AudioInput, selected: AudioInput) -> bool {
    previous != selected || selected.sources().contains(&AudioSource::System)
}

fn audio_input_switch_action(
    status: &SessionStatus,
    paused: bool,
    recovering: bool,
    lifecycle_operations: usize,
) -> Result<AudioInputSwitchAction, &'static str> {
    if recovering || !pipeline_settings_mutation_is_allowed(status, lifecycle_operations) {
        return Err("audio_input_switch_busy");
    }
    if paused || !matches!(status, SessionStatus::Listening) {
        Ok(AudioInputSwitchAction::ReconfigureOnly)
    } else {
        Ok(AudioInputSwitchAction::Reconnect)
    }
}

/// Caller owns both the content gate and generation-transition lock. No
/// provider event can enter between sealing the old generation and resetting
/// its confirmation watermarks; a superseded/save-failed request resets none.
fn commit_audio_input_switch_boundary(
    active_generation: &AtomicU64,
    lifecycle_sequence: &AtomicU64,
    expected_epoch: u64,
    controller: &mut TranslationSessionController,
    input: AudioInput,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<(u64, u64), String> {
    if !lifecycle_sequence_matches(lifecycle_sequence, expected_epoch) {
        return Err("audio_input_switch_superseded".into());
    }
    persist()?;
    let generation = advance_lifecycle_sequence_if_current(lifecycle_sequence, expected_epoch)
        .ok_or_else(|| "audio_input_switch_superseded".to_string())?;
    let previous = active_generation.swap(NO_GENERATION, Ordering::SeqCst);
    controller.reconfigure_audio_input(input);
    Ok((generation, previous))
}

/// Switching off a source must confirm native release even when the session
/// is already paused, idle, or failed. Those states may still be tearing down.
async fn release_reconfigured_audio_sources<Stop, Stopped>(
    previous: AudioInput,
    selected: AudioInput,
    system_target_changed: bool,
    mut stop: Stop,
) -> Result<(), String>
where
    Stop: FnMut(AudioSource) -> Stopped,
    Stopped: Future<Output = Result<(), String>>,
{
    for &source in previous.sources() {
        if !selected.sources().contains(&source)
            || (system_target_changed && source == AudioSource::System)
        {
            stop(source).await?;
        }
    }
    Ok(())
}

fn lifecycle_activity_is_active(has_active_session: bool, lifecycle_operations: usize) -> bool {
    has_active_session || lifecycle_operations > 0
}

fn start_request_can_proceed(
    has_active_session: bool,
    is_recovering: bool,
    active_generation: u64,
) -> bool {
    !has_active_session || (is_recovering && active_generation == NO_GENERATION)
}

fn apply_establish_failure_state(
    controller: &mut TranslationSessionController,
    error: String,
    is_recovering: bool,
) {
    if is_recovering && !session_error_requires_user_action(&error) {
        controller.begin_connecting();
    } else {
        controller.did_fail(error);
    }
}

/// These fixed application errors cannot improve through automatic reconnect.
/// Starting again is an explicit user action after authorization or system stop.
fn configuration_failure_requires_repair(error: &str) -> bool {
    crate::core::protocols::audio3::failure_requires_configuration(error)
        || matches!(
            error,
            "apple_translation_unavailable"
                | "apple_translation_assets_missing"
                | "apple_translation_language_unsupported"
                | "apple_translation_invalid_input"
        )
}

fn session_error_requires_user_action(error: &str) -> bool {
    configuration_failure_requires_repair(error)
        || matches!(
            error,
            "System audio capture permission was denied."
                | "Microphone capture permission was denied."
                | "System audio capture was stopped by the user."
                | "credential_authentication_failed"
                | "custom_speech_authentication_failed"
                | "custom_speech_credentials_missing"
                | "custom_speech_endpoint_invalid"
                | "custom_speech_model_invalid"
                | "text_translation_credentials_missing"
                | "invalid_configuration"
        )
}

/// A successful repair may retire only the error observed before that write.
fn clear_stale_configuration_error(
    controller: &mut TranslationSessionController,
    expected: Option<&(u64, String)>,
    current_epoch: u64,
    changed: bool,
) -> bool {
    let Some((epoch, error)) = expected else {
        return false;
    };
    if !changed
        || *epoch != current_epoch
        || !configuration_failure_requires_repair(error)
        || controller.state.status != SessionStatus::Error(error.clone())
    {
        return false;
    }
    controller.did_stop();
    true
}

fn source_switch_requires_reconnect(
    is_listening: bool,
    current_source: SourceLanguage,
    current_target: crate::core::models::TargetLanguage,
    current_mode: TranslationMode,
    next_source: SourceLanguage,
    next_target: crate::core::models::TargetLanguage,
    next_mode: TranslationMode,
) -> bool {
    is_listening
        && (current_source != next_source
            || current_target != next_target
            || current_mode != next_mode)
}

fn pause_transition_is_valid(
    status: &SessionStatus,
    is_paused: bool,
    active_generation: u64,
) -> bool {
    !is_paused && *status == SessionStatus::Listening && active_generation != NO_GENERATION
}

/// A pause accepted while listening may claim its epoch after a source
/// switch/recovery seals the old generation. It must finish that handoff,
/// rather than cancelling the reconnect and leaving Connecting ownerless.
fn accepted_pause_transition_is_valid(
    status: &SessionStatus,
    is_paused: bool,
    active_generation: u64,
    has_active_settings: bool,
) -> bool {
    pause_transition_is_valid(status, is_paused, active_generation)
        || (!is_paused
            && *status == SessionStatus::Connecting
            && active_generation == NO_GENERATION
            && has_active_settings)
}

fn resume_transition_is_valid(
    status: &SessionStatus,
    is_paused: bool,
    active_generation: u64,
    has_active_settings: bool,
) -> bool {
    is_paused
        && *status == SessionStatus::Listening
        && active_generation == NO_GENERATION
        && has_active_settings
}

fn try_begin_start(in_progress: &AtomicBool) -> bool {
    in_progress
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

fn invalidate_generation_atoms(
    active_generation: &AtomicU64,
    lifecycle_sequence: &AtomicU64,
    generation: u64,
) -> Option<u64> {
    if lifecycle_sequence.load(Ordering::SeqCst) != generation {
        return None;
    }
    if active_generation
        .compare_exchange(
            generation,
            NO_GENERATION,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_ok()
    {
        let mut owned_epoch = generation.wrapping_add(1);
        if owned_epoch == NO_GENERATION {
            owned_epoch = 1;
        }
        lifecycle_sequence
            .compare_exchange(generation, owned_epoch, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| owned_epoch)
    } else {
        None
    }
}

/// The caller holds generation_transition through owner validation and claim.
/// Rebuilding settings can replace audio resources without changing generation.
fn invalidate_audio_attempt_atoms(
    active_generation: &AtomicU64,
    lifecycle_sequence: &AtomicU64,
    owner: &mut Option<(u64, u64)>,
    generation: u64,
    attempt: u64,
) -> Option<u64> {
    if *owner != Some((generation, attempt)) {
        return None;
    }
    let epoch = invalidate_generation_atoms(active_generation, lifecycle_sequence, generation)?;
    *owner = None;
    Some(epoch)
}

fn advance_lifecycle_sequence_if_current(
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

fn lifecycle_sequence_matches(lifecycle_sequence: &AtomicU64, expected: u64) -> bool {
    lifecycle_sequence.load(Ordering::SeqCst) == expected
}

fn resume_failure_is_still_owned(
    error: &str,
    failure_epoch: u64,
    current_epoch: u64,
    status: &SessionStatus,
    active_generation: u64,
) -> bool {
    error != SESSION_START_CANCELLED
        && failure_epoch == current_epoch
        && matches!(status, SessionStatus::Error(message) if message == error)
        && active_generation == NO_GENERATION
}

/// Ownership validation and paused-state restoration share the lifecycle gate.
/// Stop may claim a newer epoch immediately, but its final state must run after
/// this restoration or make the ownership check fail before any state is changed.
async fn restore_failed_resume(
    lifecycle_lock: Arc<TokioMutex<()>>,
    owns_failure: impl FnOnce() -> bool,
    restore: impl FnOnce(),
) -> bool {
    let _lifecycle = lifecycle_lock.lock_owned().await;
    if !owns_failure() {
        return false;
    }
    restore();
    true
}

fn cancelled_recovery_attempt_is_retryable(
    retry_generation: u64,
    attempt_generation: u64,
    current_epoch: u64,
    has_active_settings: bool,
) -> bool {
    retry_generation == attempt_generation
        && current_epoch == attempt_generation.wrapping_add(1)
        && has_active_settings
}

fn recovery_exhaustion_is_still_owned(
    recovery_epoch: u64,
    current_epoch: u64,
    active_generation: u64,
) -> bool {
    recovery_epoch == current_epoch && active_generation == NO_GENERATION
}

fn clear_recovery_atoms(is_recovering: &AtomicBool, retry_generation: &AtomicU64) {
    retry_generation.store(NO_GENERATION, Ordering::SeqCst);
    is_recovering.store(false, Ordering::SeqCst);
}

/// Bounded exponential recovery delay with deterministic per-generation
/// jitter. Determinism keeps lifecycle tests reliable while preventing two
/// mimi instances from reconnecting in lockstep after a shared outage.
fn recovery_delay(attempt: usize, generation: u64) -> Duration {
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

fn generation_accepts_event(
    active_generation: u64,
    stopping_tail_generation: u64,
    generation: u64,
    event: &LiveTranslateServerEvent,
) -> bool {
    generation != NO_GENERATION
        && (active_generation == generation
            || (stopping_tail_generation == generation
                && matches!(
                    event,
                    LiveTranslateServerEvent::SubtitleFinalPair { .. }
                        | LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
                        | LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
                )))
}

fn subtitle_content_is_current(
    generation: u64,
    event: &ProviderEvent,
    current: (u64, u64),
) -> bool {
    !event.is_content() || current == (generation, event.content_revision)
}

fn confirmed_history_tail_changed(
    previous: Option<&crate::core::models::SubtitlePair>,
    current: Option<&crate::core::models::SubtitlePair>,
) -> bool {
    current.is_some_and(|current| {
        previous.is_none_or(|previous| {
            current.created_at_ms != previous.created_at_ms || current != previous
        })
    })
}

fn provider_error_is_retryable(code: &str) -> bool {
    matches!(
        code,
        "transport_error"
            | "provider_event_backlog_overflow"
            | "translation_backlog_overflow"
            | "translation_rate_limited"
            | "translation_temporarily_unavailable"
    )
}

fn provider_recovery_minimum_delay(code: &str) -> Duration {
    match code {
        // A new client must not bypass the MT cooldown by replacing the
        // generation after finite retries or bounded final-queue pressure.
        "translation_rate_limited" | "translation_backlog_overflow" => Duration::from_secs(8),
        "translation_temporarily_unavailable" => Duration::from_millis(600),
        _ => Duration::ZERO,
    }
}

fn clear_task_slot_if_id(
    slot: &Mutex<Option<JoinHandle<()>>>,
    current_id: &AtomicU64,
    task_id: u64,
) -> bool {
    let mut slot = slot.lock().unwrap();
    if current_id.load(Ordering::SeqCst) != task_id {
        return false;
    }
    current_id.store(NO_GENERATION, Ordering::SeqCst);
    drop(slot.take());
    true
}

fn clear_owned_value_if_generation<T>(
    slot: &Mutex<Option<T>>,
    owner_generation: &AtomicU64,
    generation: u64,
) -> bool {
    let mut slot = slot.lock().unwrap();
    if owner_generation.load(Ordering::SeqCst) != generation {
        return false;
    }
    *slot = None;
    owner_generation.store(NO_GENERATION, Ordering::SeqCst);
    true
}

fn update_owned_value<T>(
    slot: &Mutex<Option<T>>,
    owner_generation: &AtomicU64,
    update: impl FnOnce(&mut T),
) -> bool {
    let mut slot = slot.lock().unwrap();
    if owner_generation.load(Ordering::SeqCst) == NO_GENERATION {
        return false;
    }
    let Some(value) = slot.as_mut() else {
        return false;
    };
    update(value);
    true
}

async fn lock_after_operations(
    lock: Arc<TokioMutex<()>>,
    operation_count: Arc<AtomicUsize>,
    notify: Arc<Notify>,
) -> OwnedMutexGuard<()> {
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        // Register before inspecting the counter. `notify_waiters` does not
        // retain a permit, so awaiting an unregistered future after the last
        // teardown completes could otherwise sleep forever.
        notified.as_mut().enable();
        let guard = Arc::clone(&lock).lock_owned().await;
        if operation_count.load(Ordering::SeqCst) == 0 {
            return guard;
        }
        drop(guard);
        notified.await;
    }
}

/// A newer pause may claim its epoch immediately, but cannot publish Paused
/// or let Resume install replacements before the retired resources are gone.
async fn retain_lifecycle_during_teardown(
    lifecycle: OwnedMutexGuard<()>,
    cleanup: impl Future<Output = ()>,
) -> OwnedMutexGuard<()> {
    cleanup.await;
    lifecycle
}

async fn wait_until_generation_changes(
    active_generation: Arc<AtomicU64>,
    lifecycle_sequence: Arc<AtomicU64>,
    lifecycle_notify: Arc<Notify>,
    generation: u64,
) {
    loop {
        let notified = lifecycle_notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if lifecycle_sequence.load(Ordering::SeqCst) != generation
            || active_generation.load(Ordering::SeqCst) != generation
        {
            return;
        }
        notified.await;
    }
}

async fn run_generation_bound_operation<T>(
    active_generation: Arc<AtomicU64>,
    lifecycle_sequence: Arc<AtomicU64>,
    lifecycle_notify: Arc<Notify>,
    generation: u64,
    operation: impl Future<Output = T>,
) -> Result<T, String> {
    tokio::select! {
        result = operation => Ok(result),
        () = wait_until_generation_changes(
            active_generation,
            lifecycle_sequence,
            lifecycle_notify,
            generation,
        ) => Err(SESSION_START_CANCELLED.into()),
    }
}

impl Drop for StartRequestGuard {
    fn drop(&mut self) {
        self.in_progress.store(false, Ordering::SeqCst);
    }
}

impl From<&TranslationSessionState> for SessionStateEvent {
    fn from(state: &TranslationSessionState) -> Self {
        let status = match &state.status {
            SessionStatus::Idle => StatusPayload::Idle,
            SessionStatus::Connecting => StatusPayload::Connecting,
            SessionStatus::Listening => StatusPayload::Listening,
            SessionStatus::Stopping => StatusPayload::Stopping,
            SessionStatus::Error(message) => StatusPayload::Error {
                message: message.clone(),
            },
        };
        Self {
            debug_snapshot_id: None,
            is_active: state.status.is_active(),
            status,
            is_paused: false,
            is_overlay_collapsed: false,
            subtitles: state.subtitles.clone(),
            detected_language: state
                .detected_language
                .as_ref()
                .map(|language| language.code.clone()),
            is_translation_pending: state.is_translation_pending,
            is_translation_preview_pending: state.is_translation_preview_pending,
            is_translation_timed_out: state.is_translation_timed_out,
            api_latency_ms: None,
            translation_latency_ms: None,
            translation_latency_kind: None,
            translation_recovery: state.translation_recovery,
        }
    }
}

fn status_should_show_overlay(status: &SessionStatus) -> bool {
    status.is_active() || matches!(status, SessionStatus::Error(_))
}

/// Stop is a completion boundary: retain normal diagnostic scheduling and
/// publish the fully drained state before the caller can seal evidence or quit.
async fn publish_stop_boundary(
    request_publish: impl FnOnce(),
    publish_now: impl Future<Output = ()>,
) {
    request_publish();
    publish_now.await;
}

/// Content lives in the local journal. Only size/limit metadata stays here.
#[derive(Default)]
struct LocalCaptureStats {
    transcript_count: usize,
    transcript_bytes: usize,
    transcript_limited: bool,
    audio_bytes: usize,
    audio_limited: bool,
    sample_rate: u32,
}

/// Capture presentation never borrows a retired generation's recent activity.
/// Unlike diagnostics, this represents currently selected live input only.
fn source_capture_observation(
    pipeline_slot: &Mutex<Option<Arc<AudioSendPipeline>>>,
    pipeline_generation: &AtomicU64,
    active_generation: u64,
) -> Option<crate::audio::CaptureSignal> {
    let pipeline = pipeline_slot.lock().unwrap();
    if active_generation == NO_GENERATION
        || pipeline_generation.load(Ordering::SeqCst) != active_generation
    {
        return None;
    }
    pipeline.as_ref().map(|pipeline| {
        let (pcm_data_recent, sound_recent) = pipeline.input_activity();
        crate::audio::CaptureSignal {
            pcm_data_recent,
            sound_recent,
        }
    })
}

/// A provider terminating either stream terminates the selected session.
/// Synchronize every reducer before publishing or tearing down transports.
fn apply_terminal_event_to_all_sources(
    controller: &mut TranslationSessionController,
    event: &LiveTranslateServerEvent,
) {
    match event {
        LiveTranslateServerEvent::SessionFinished => controller.did_stop(),
        LiveTranslateServerEvent::Error { message, .. } => controller.did_fail(message.clone()),
        _ => {}
    }
}

#[derive(Clone, Copy)]
enum EventPumpCleanup {
    Preserve,
    Abort,
}

/// Establishing a selection is transactional: a later source's failure must
/// release the earlier source too, including partially installed resources.
async fn connect_source_group<Start, Started, Rollback, RolledBack>(
    sources: &[AudioSource],
    mut start: Start,
    rollback: Rollback,
) -> Result<(), String>
where
    Start: FnMut(AudioSource) -> Started,
    Started: Future<Output = Result<(), String>>,
    Rollback: FnOnce(EventPumpCleanup) -> RolledBack,
    RolledBack: Future<Output = ()>,
{
    for &source in sources {
        if let Err(error) = start(source).await {
            // The event pump may have invalidated this startup and still own
            // a recovery handoff. Release captures/clients without aborting it.
            rollback(EventPumpCleanup::Preserve).await;
            return Err(error);
        }
    }
    Ok(())
}

/// A source owns every transport, capture, revision and timer it can mutate.
/// Lifecycle generations are shared so either lane failing tears down both.
#[derive(Clone)]
struct RuntimeLane {
    audio: Arc<Mutex<AudioCapture>>,
    client: Arc<Mutex<Option<TranslationClient>>>,
    client_generation: Arc<AtomicU64>,
    subtitle_content_revision: Arc<Mutex<(u64, u64)>>,
    mt_budget_continuity: Arc<Mutex<MTBudgetContinuity>>,
    audio_pipeline: Arc<Mutex<Option<Arc<AudioSendPipeline>>>>,
    audio_pipeline_generation: Arc<AtomicU64>,
    capture_generation: Arc<AtomicU64>,
    pump_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    pump_generation: Arc<AtomicU64>,
    translation_timeout_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    translation_timeout_task_id: Arc<AtomicU64>,
}

impl RuntimeLane {
    fn new(app: &AppHandle) -> Self {
        let audio_capture = AudioCapture::for_app(app);
        Self {
            audio: Arc::new(Mutex::new(audio_capture)),
            client: Arc::new(Mutex::new(None)),
            client_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            subtitle_content_revision: Default::default(),
            mt_budget_continuity: Default::default(),
            audio_pipeline: Arc::new(Mutex::new(None)),
            audio_pipeline_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            capture_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            pump_task: Arc::new(Mutex::new(None)),
            pump_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            translation_timeout_task: Arc::new(Mutex::new(None)),
            translation_timeout_task_id: Arc::new(AtomicU64::new(NO_GENERATION)),
        }
    }
}

type EchoPipelineSlot = Arc<Mutex<Option<(u64, Arc<EchoPipeline>)>>>;

#[derive(Clone)]
pub struct SessionManager {
    app: AppHandle,
    lanes: [RuntimeLane; 2],
    /// One DSP stage per connection attempt; a same-generation settings
    /// rebuild also replaces the reference history and adaptive filter.
    echo_pipeline: EchoPipelineSlot,
    audio_attempt: Arc<Mutex<Option<(u64, u64)>>>,
    diagnostic_epoch: Instant,
    diagnostic_error: Arc<Mutex<Option<(SafeFailure, Instant)>>>,
    diagnostic_recovery: Arc<Mutex<Option<(RecoveryAction, Instant)>>>,
    diagnostic_capture: Arc<Mutex<Option<(CaptureObservation, Instant)>>>,
    diagnostic_journal: Arc<Mutex<DiagnosticJournal>>,
    health_latency: Arc<Mutex<Option<HealthCheckLatency>>>,
    settings: Arc<SettingsStore>,
    controller: Arc<Mutex<TranslationSessionController>>,
    /// Captured at manual start; pause, reconnect and recovery preserve it.
    active_audio_input: Arc<Mutex<AudioInput>>,
    recording: Arc<Mutex<[crate::core::session_archive::AudioRecording; 2]>>,
    archive_revision: Arc<AtomicU64>,
    history: Arc<SessionHistory>,
    history_pending_id: Arc<Mutex<Option<String>>>,
    history_pending_text: Arc<AtomicBool>,
    history_pending_audio: Arc<AtomicBool>,
    history_stats: Arc<Mutex<LocalCaptureStats>>,
    history_save_error: Arc<AtomicBool>,
    /// Clear and event application (including local-history append) are atomic.
    subtitle_content_lock: Arc<TokioMutex<()>>,
    active_settings: Arc<Mutex<Option<LiveTranslationConfiguration>>>,
    active_settings_generation: Arc<AtomicU64>,
    is_paused: Arc<AtomicBool>,
    is_recovering: Arc<AtomicBool>,
    is_overlay_collapsed: Arc<AtomicBool>,
    /// Set whenever a session-state broadcast is requested; the single
    /// scheduled publisher clears it after each publish and keeps looping
    /// until no newer request has arrived.
    publish_dirty: Arc<AtomicBool>,
    /// Serializes publisher scheduling: exactly one task owns this lock.
    publish_lock: Arc<TokioMutex<()>>,
    health_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    health_task_id: Arc<AtomicU64>,
    recovery_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    recovery_task_id: Arc<AtomicU64>,
    /// Marks a provider transport failure that cancelled the current recovery
    /// attempt. The existing recovery owner consumes it and performs the next
    /// retry instead of trying to enqueue a second recovery task.
    recovery_retry_generation: Arc<AtomicU64>,
    background_task_sequence: Arc<AtomicU64>,
    /// Explicit lifecycle operations and settings mutations share this lock,
    /// eliminating check-then-mutate races around credential/profile reads.
    lifecycle_lock: Arc<TokioMutex<()>>,
    lifecycle_operations: Arc<AtomicUsize>,
    lifecycle_sequence: Arc<AtomicU64>,
    lifecycle_notify: Arc<Notify>,
    active_generation: Arc<AtomicU64>,
    stopping_tail_generation: Arc<AtomicU64>,
    start_in_progress: Arc<AtomicBool>,
    generation_transition: Arc<Mutex<()>>,
    teardown_operations: Arc<AtomicUsize>,
    teardown_notify: Arc<Notify>,
}

impl SessionManager {
    fn lane(&self, source: AudioSource) -> &RuntimeLane {
        &self.lanes[match source {
            AudioSource::System => 0,
            AudioSource::Microphone => 1,
        }]
    }

    fn sources(&self) -> &'static [AudioSource] {
        self.active_audio_input.lock().unwrap().sources()
    }

    pub fn new(app: AppHandle, settings: Arc<SettingsStore>) -> Arc<Self> {
        let history_directory = app.path().app_data_dir().ok();
        let history = SessionHistory::new(
            history_directory
                .clone()
                .unwrap_or_default()
                .join("session-history"),
            settings.is_ui_test() || history_directory.is_none(),
        );
        Arc::new(Self {
            lanes: [RuntimeLane::new(&app), RuntimeLane::new(&app)],
            echo_pipeline: Default::default(),
            audio_attempt: Default::default(),
            app,
            diagnostic_epoch: Instant::now(),
            diagnostic_error: Default::default(),
            diagnostic_recovery: Default::default(),
            diagnostic_capture: Default::default(),
            diagnostic_journal: Default::default(),
            health_latency: Default::default(),
            settings,
            controller: Arc::new(Mutex::new(TranslationSessionController::default())),
            active_audio_input: Default::default(),
            recording: Default::default(),
            archive_revision: Default::default(),
            history: Arc::new(history),
            history_pending_id: Arc::new(Mutex::new(None)),
            history_pending_text: Arc::new(AtomicBool::new(false)),
            history_pending_audio: Arc::new(AtomicBool::new(false)),
            history_stats: Arc::new(Mutex::new(LocalCaptureStats::default())),
            history_save_error: Arc::new(AtomicBool::new(false)),
            subtitle_content_lock: Default::default(),
            active_settings: Arc::new(Mutex::new(None)),
            active_settings_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            is_paused: Arc::new(AtomicBool::new(false)),
            is_recovering: Arc::new(AtomicBool::new(false)),
            is_overlay_collapsed: Arc::new(AtomicBool::new(false)),
            publish_dirty: Arc::new(AtomicBool::new(false)),
            publish_lock: Arc::new(TokioMutex::new(())),
            health_task: Arc::new(Mutex::new(None)),
            health_task_id: Arc::new(AtomicU64::new(NO_GENERATION)),
            recovery_task: Arc::new(Mutex::new(None)),
            recovery_task_id: Arc::new(AtomicU64::new(NO_GENERATION)),
            recovery_retry_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            background_task_sequence: Arc::new(AtomicU64::new(0)),
            lifecycle_lock: Arc::new(TokioMutex::new(())),
            lifecycle_operations: Arc::new(AtomicUsize::new(0)),
            lifecycle_sequence: Arc::new(AtomicU64::new(0)),
            lifecycle_notify: Arc::new(Notify::new()),
            active_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            stopping_tail_generation: Arc::new(AtomicU64::new(NO_GENERATION)),
            start_in_progress: Arc::new(AtomicBool::new(false)),
            generation_transition: Arc::new(Mutex::new(())),
            teardown_operations: Arc::new(AtomicUsize::new(0)),
            teardown_notify: Arc::new(Notify::new()),
        })
    }

    pub fn is_active(&self) -> bool {
        lifecycle_activity_is_active(
            self.has_active_session(),
            self.lifecycle_operations.load(Ordering::SeqCst),
        )
    }

    /// Errors remain visible in the overlay until the user retries or stops.
    /// This is intentionally separate from `is_active`: tray and shortcut
    /// actions must still treat an error as a stopped session that can retry.
    pub fn should_show_overlay(&self) -> bool {
        self.is_active()
            || status_should_show_overlay(&self.controller.lock().unwrap().state.status)
    }

    /// True only for an established/in-flight session, excluding a settings
    /// mutation that merely owns the shared lifecycle lock.
    pub fn has_active_session(&self) -> bool {
        self.controller.lock().unwrap().state.status.is_active()
            || self.active_generation.load(Ordering::SeqCst) != NO_GENERATION
    }

    /// Serializes a profile or listening-settings mutation with start/stop.
    /// The caller must retain the returned guard until its synchronous store
    /// mutation and persistence have both completed.
    pub async fn settings_mutation_guard(
        self: &Arc<Self>,
        require_inactive: bool,
    ) -> Result<OwnedMutexGuard<()>, String> {
        let guard = self.lock_after_teardown().await;
        if require_inactive && self.is_active() {
            Err("Listening settings cannot be changed while a session is active.".into())
        } else {
            Ok(guard)
        }
    }

    /// Called under the settings mutation guard before a persisted repair.
    pub fn configuration_failure_snapshot(&self) -> Option<(u64, String)> {
        let controller = self.controller.lock().unwrap();
        match &controller.state.status {
            SessionStatus::Error(error)
                if configuration_failure_requires_repair(error)
                    && self.active_generation.load(Ordering::SeqCst) == NO_GENERATION =>
            {
                Some((
                    self.lifecycle_sequence.load(Ordering::SeqCst),
                    error.clone(),
                ))
            }
            _ => None,
        }
    }

    /// Does not start capture, erase subtitles or clear a newer session's error.
    pub fn configuration_saved(self: &Arc<Self>, expected: Option<(u64, String)>, changed: bool) {
        if self.active_generation.load(Ordering::SeqCst) != NO_GENERATION {
            return;
        }
        let cleared = clear_stale_configuration_error(
            &mut self.controller.lock().unwrap(),
            expected.as_ref(),
            self.lifecycle_sequence.load(Ordering::SeqCst),
            changed,
        );
        if cleared {
            self.publish_state();
        }
    }

    fn record_diagnostic_failure(&self, error: &str) {
        self.record_safe_failure(SafeFailure::from_error(error));
    }

    fn record_safe_failure(&self, classification: SafeFailure) {
        *self.diagnostic_error.lock().unwrap() = Some((classification, Instant::now()));
        self.record_diagnostic_event(DiagnosticEvent::Failure { classification });
    }

    fn record_diagnostic_event(&self, event: DiagnosticEvent) {
        self.diagnostic_journal
            .lock()
            .unwrap()
            .record(event, milliseconds(self.diagnostic_epoch, Instant::now()));
    }

    /// Recheck event admission under the generation gate before recording.
    /// No journal or generation lock crosses an await.
    fn record_accepted_provider_event(
        &self,
        generation: u64,
        event: &LiveTranslateServerEvent,
    ) -> bool {
        let _transition = self.generation_transition.lock().unwrap();
        if !self.accepts_event(generation, event) || self.is_paused() {
            return false;
        }
        if let LiveTranslateServerEvent::Error { code, message } = event {
            self.record_safe_failure(SafeFailure::from_provider_error(code, message));
        }
        let mut journal = self.diagnostic_journal.lock().unwrap();
        let text_kind = match event {
            LiveTranslateServerEvent::SourceDraft { .. }
            | LiveTranslateServerEvent::SourceUtteranceDraft { .. } => {
                Some(TextEventKind::SourceDraft)
            }
            LiveTranslateServerEvent::SourceFinal { .. }
            | LiveTranslateServerEvent::SourceUtteranceFinal { .. } => {
                Some(TextEventKind::SourceFinal)
            }
            LiveTranslateServerEvent::TranslationDraft(_) => Some(TextEventKind::TranslationDraft),
            LiveTranslateServerEvent::TranslationFinal(_) => Some(TextEventKind::TranslationFinal),
            LiveTranslateServerEvent::SubtitleFinalPair { .. }
            | LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { .. }
            | LiveTranslateServerEvent::SubtitleConfirmedPair { .. } => {
                Some(TextEventKind::ConfirmedPair)
            }
            LiveTranslateServerEvent::UtteranceText { role, is_final, .. } => {
                Some(match (role, is_final) {
                    (UtteranceRole::Source, false) => TextEventKind::SourceDraft,
                    (UtteranceRole::Source, true) => TextEventKind::SourceFinal,
                    (UtteranceRole::Translation, false) => TextEventKind::TranslationDraft,
                    (UtteranceRole::Translation, true) => TextEventKind::TranslationFinal,
                })
            }
            _ => None,
        };
        if let Some(kind) = text_kind {
            journal.observe_text(kind);
        }
        if let LiveTranslateServerEvent::TranslationDeferred(recovery) = event {
            journal.record(
                DiagnosticEvent::TranslationBackoff {
                    recovery: *recovery,
                },
                milliseconds(self.diagnostic_epoch, Instant::now()),
            );
        }
        true
    }

    fn record_recovery_action(&self, action: RecoveryAction) {
        *self.diagnostic_recovery.lock().unwrap() = Some((action, Instant::now()));
        self.record_diagnostic_event(DiagnosticEvent::Recovery { action });
    }

    fn record_recovery_if_current(&self, action: RecoveryAction, epoch: u64) {
        let _transition = self.generation_transition.lock().unwrap();
        if self.is_lifecycle_request_current(epoch) {
            self.record_recovery_action(action);
        }
    }

    fn current_capture_observation(&self) -> Option<CaptureObservation> {
        self.lanes
            .iter()
            .filter_map(|lane| {
                lane.audio_pipeline
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|pipeline| pipeline.input_activity())
            })
            .fold(None, |previous, (pcm_data_recent, sound_recent)| {
                let mut observation = previous.unwrap_or(CaptureObservation {
                    pcm_data_recent: false,
                    sound_recent: false,
                });
                observation.pcm_data_recent |= pcm_data_recent;
                observation.sound_recent |= sound_recent;
                Some(observation)
            })
    }

    pub async fn audio_applications(
        &self,
    ) -> Result<
        crate::audio::applications::ApplicationSnapshot,
        crate::audio::SystemAudioCaptureError,
    > {
        let capture = self.lane(AudioSource::System).audio.lock().unwrap().clone();
        capture.audio_applications().await
    }

    pub fn capture_status(&self) -> crate::audio::CaptureStatus {
        let preferences = self.settings.preferences();
        let generation = self.active_generation.load(Ordering::SeqCst);
        crate::audio::CaptureStatus::for_input(preferences.audio_input, |source| {
            let lane = self.lane(source);
            let observation = source_capture_observation(
                &lane.audio_pipeline,
                &lane.audio_pipeline_generation,
                generation,
            );
            if source == AudioSource::System {
                if let Some(name) = preferences.system_audio_target.application_name() {
                    return crate::audio::CaptureDetails {
                        kind: "application",
                        strategy: "selected_application",
                        actual_device_name: Some(name.to_string()),
                        system_output_device_name: None,
                        observation,
                    };
                }
            }
            self.capture_details(source, observation, &preferences.windows_audio_source)
        })
    }

    fn capture_details(
        &self,
        source: AudioSource,
        observation: Option<crate::audio::CaptureSignal>,
        windows_audio_source: &str,
    ) -> crate::audio::CaptureDetails {
        if source == AudioSource::Microphone {
            return crate::audio::CaptureDetails {
                kind: "microphone",
                strategy: "default_input",
                actual_device_name: self
                    .lane(source)
                    .audio
                    .lock()
                    .unwrap()
                    .microphone_device_name(),
                system_output_device_name: None,
                observation,
            };
        }
        #[cfg(target_os = "windows")]
        {
            let snapshot = self.windows_audio_status().ok().flatten();
            let actual_device_name = snapshot.and_then(|snapshot| {
                snapshot.current_device.and_then(|id| {
                    snapshot
                        .devices
                        .into_iter()
                        .find(|device| device.id == id)
                        .map(|device| device.name)
                })
            });
            crate::audio::CaptureDetails {
                kind: "windows_output",
                strategy: if windows_audio_source.is_empty() {
                    "follow_system"
                } else {
                    "manual_output"
                },
                actual_device_name,
                system_output_device_name: None,
                observation,
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = windows_audio_source;
            crate::audio::CaptureDetails {
                kind: if cfg!(target_os = "macos") {
                    "macos_system_mix"
                } else if cfg!(target_os = "linux") {
                    "linux_output_monitor"
                } else {
                    "unknown"
                },
                strategy: "platform_capture",
                actual_device_name: None,
                system_output_device_name: {
                    #[cfg(target_os = "macos")]
                    {
                        crate::audio::macos_output::default_output_device_name()
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        None
                    }
                },
                observation,
            }
        }
    }

    pub fn support_diagnostics(&self) -> String {
        crate::core::support_diagnostics::render(self.diagnostic_facts())
    }

    pub fn support_issue(&self) -> crate::core::support_diagnostics::SupportIssue {
        crate::core::support_diagnostics::issue_link(self.diagnostic_facts())
    }

    fn diagnostic_facts(&self) -> DiagnosticFacts {
        let now = Instant::now();
        let prefs = self.settings.preferences();
        let provider = self
            .settings
            .active_profile()
            .ok()
            .map(|profile| profile.effective_provider());
        let (provider, mode) = self
            .active_settings
            .lock()
            .unwrap()
            .as_ref()
            .map(|config| (Some(config.provider), config.effective_translation_mode()))
            .unwrap_or((provider, prefs.translation_mode));
        let generation = self.active_generation.load(Ordering::SeqCst);
        let (
            status,
            translation_pending,
            translation_preview_pending,
            translation_timed_out,
            session_status,
            translation_recovery,
        ) = {
            let controller = self.controller.lock().unwrap();
            let status = match controller.state.status {
                SessionStatus::Idle => DiagnosticStatus::Idle,
                SessionStatus::Connecting => DiagnosticStatus::Connecting,
                SessionStatus::Stopping => DiagnosticStatus::Stopping,
                SessionStatus::Listening if self.is_paused() => DiagnosticStatus::Paused,
                SessionStatus::Listening => DiagnosticStatus::Listening,
                SessionStatus::Error(_) => DiagnosticStatus::Error,
            };
            (
                status,
                controller.state.is_translation_pending,
                controller.state.is_translation_preview_pending,
                controller.state.is_translation_timed_out,
                controller.state.status.clone(),
                controller.state.translation_recovery,
            )
        };
        let capture = self
            .current_capture_observation()
            .map(|capture| (capture, 0))
            .or_else(|| {
                self.diagnostic_capture
                    .lock()
                    .unwrap()
                    .map(|(capture, at)| (capture, crate::core::diagnostics::milliseconds(at, now)))
            });
        #[cfg(target_os = "windows")]
        let (output_selection, output_availability) = {
            let selected = prefs.windows_audio_source;
            let availability = self
                .windows_audio_status()
                .ok()
                .flatten()
                .map(|snapshot| {
                    if selected.is_empty() {
                        // An active bound endpoint is known; before start, the
                        // default role cannot be inferred from the device list.
                        if snapshot.current_device.is_some() {
                            Availability::Available
                        } else {
                            Availability::Unknown
                        }
                    } else if snapshot.devices.iter().any(|device| device.id == selected) {
                        Availability::Available
                    } else {
                        Availability::Unavailable
                    }
                })
                .unwrap_or(Availability::Unknown);
            (
                if selected.is_empty() {
                    OutputSelection::SystemDefault
                } else {
                    OutputSelection::ManualOutput
                },
                availability,
            )
        };
        #[cfg(not(target_os = "windows"))]
        let (output_selection, output_availability) =
            (OutputSelection::PlatformSystemAudio, Availability::Unknown);
        let (output_selection, output_availability) = if prefs.audio_input == AudioInput::Microphone
        {
            (OutputSelection::DefaultMicrophone, Availability::Unknown)
        } else if prefs.audio_input == AudioInput::Both {
            (OutputSelection::SystemAndMicrophone, Availability::Unknown)
        } else if prefs.system_audio_target.application_id().is_some() {
            (OutputSelection::SelectedApplication, Availability::Unknown)
        } else {
            (output_selection, output_availability)
        };
        let translation_latency = self
            .sources()
            .iter()
            .filter_map(|&source| {
                self.client_for_generation(source, generation)
                    .and_then(|client| client.translation_latency())
            })
            .max_by_key(|latency| latency.milliseconds);
        let (mut api_latency_ms, mut translation_latency_ms, mut translation_latency_kind) =
            visible_session_latencies(
                &session_status,
                self.is_paused(),
                self.is_recovering.load(Ordering::SeqCst),
                generation,
                self.health_task_id.load(Ordering::SeqCst),
                *self.health_latency.lock().unwrap(),
                translation_latency,
            );
        let translation_recovery = if self.is_generation_current(generation) {
            translation_recovery
        } else {
            api_latency_ms = None;
            translation_latency_ms = None;
            translation_latency_kind = None;
            None
        };
        let journal = self.diagnostic_journal.lock().unwrap().snapshot();
        DiagnosticFacts {
            provider,
            mode,
            status,
            output_selection,
            output_availability,
            capture,
            translation_pending,
            translation_preview_pending,
            translation_timed_out,
            last_error: self
                .diagnostic_error
                .lock()
                .unwrap()
                .map(|(error, at)| (error, crate::core::diagnostics::milliseconds(at, now))),
            recovery: self
                .diagnostic_recovery
                .lock()
                .unwrap()
                .map(|(action, at)| (action, crate::core::diagnostics::milliseconds(at, now))),
            api_latency_ms,
            translation_latency_ms,
            translation_latency_kind,
            translation_recovery,
            journal,
            elapsed_ms: crate::core::diagnostics::milliseconds(
                self.diagnostic_epoch,
                Instant::now(),
            ),
        }
    }

    pub fn windows_audio_status(
        &self,
    ) -> Result<Option<crate::audio::AudioSourceSnapshot>, String> {
        if self.settings.preferences().audio_input == AudioInput::Microphone {
            return Ok(None);
        }
        #[cfg(target_os = "windows")]
        {
            let mut snapshot = self
                .lane(AudioSource::System)
                .audio
                .lock()
                .unwrap()
                .snapshot()?;
            // Sound reaching the provider is measured after mono/resampling,
            // independently of raw callback arrival or synthetic keepalive.
            let system = self.lane(AudioSource::System);
            snapshot.receiving_sound = snapshot.current_device.is_some()
                && source_capture_observation(
                    &system.audio_pipeline,
                    &system.audio_pipeline_generation,
                    self.active_generation.load(Ordering::SeqCst),
                )
                .is_some_and(|capture| capture.sound_recent);
            Ok(Some(snapshot))
        }
        #[cfg(not(target_os = "windows"))]
        {
            Ok(None)
        }
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::SeqCst)
    }

    fn can_pause_current_session(&self) -> bool {
        let status = self.controller.lock().unwrap().state.status.clone();
        pause_transition_is_valid(
            &status,
            self.is_paused(),
            self.active_generation.load(Ordering::SeqCst),
        )
    }

    fn can_resume_current_session(&self) -> bool {
        let status = self.controller.lock().unwrap().state.status.clone();
        let has_active_settings = self.active_settings.lock().unwrap().is_some();
        resume_transition_is_valid(
            &status,
            self.is_paused(),
            self.active_generation.load(Ordering::SeqCst),
            has_active_settings,
        )
    }

    pub fn is_overlay_collapsed(&self) -> bool {
        self.is_overlay_collapsed.load(Ordering::SeqCst)
    }

    /// Starts a listening session while preserving the bounded confirmed display.
    /// Only the explicit clear action erases subtitles; new sessions reset their
    /// transport state and opt-in archives independently.
    pub async fn start(self: &Arc<Self>) -> Result<(), String> {
        self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
            action: LifecycleAction::StartRequested,
        });
        let status_label = {
            let controller = self.controller.lock().unwrap();
            match &controller.state.status {
                SessionStatus::Idle => "idle",
                SessionStatus::Connecting => "connecting",
                SessionStatus::Listening => "listening",
                SessionStatus::Stopping => "stopping",
                SessionStatus::Error(_) => "error",
            }
        };
        pipeline_log!(
            "session start entered status={} generation={} recovering={} startBusy={} teardownOperations={}",
            status_label,
            self.active_generation.load(Ordering::SeqCst),
            u8::from(self.is_recovering.load(Ordering::SeqCst)),
            u8::from(self.start_in_progress.load(Ordering::SeqCst)),
            self.teardown_operations.load(Ordering::SeqCst),
        );
        if !try_begin_start(&self.start_in_progress) {
            self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
                action: LifecycleAction::StartBusy,
            });
            pipeline_log!("session start skipped label=start_in_progress");
            return Ok(());
        }
        let _start_request = StartRequestGuard {
            in_progress: Arc::clone(&self.start_in_progress),
        };
        if !start_request_can_proceed(
            self.has_active_session(),
            self.is_recovering.load(Ordering::SeqCst),
            self.active_generation.load(Ordering::SeqCst),
        ) {
            self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
                action: LifecycleAction::StartAlreadyActive,
            });
            pipeline_log!("session start skipped label=active_session");
            return Ok(());
        }
        let _operation = self.begin_lifecycle_operation();
        let request_generation = self.next_lifecycle_request();
        let lifecycle = self.lock_after_teardown().await;
        if !self.is_lifecycle_request_current(request_generation)
            || !start_request_can_proceed(
                self.has_active_session(),
                self.is_recovering.load(Ordering::SeqCst),
                self.active_generation.load(Ordering::SeqCst),
            )
        {
            self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
                action: if self.is_lifecycle_request_current(request_generation) {
                    LifecycleAction::StartAlreadyActive
                } else {
                    LifecycleAction::StartSuperseded
                },
            });
            pipeline_log!(
                "session start skipped label={}",
                if self.is_lifecycle_request_current(request_generation) {
                    "active_session_after_teardown"
                } else {
                    "superseded"
                }
            );
            return Ok(());
        }
        // A manual start is a new session; server quota can still be exhausted,
        // but no old client accounting may be restored into this new intent.
        for lane in &self.lanes {
            lane.mt_budget_continuity.lock().unwrap().reset();
        }
        // A manual start during recovery backoff is the newer user intent.
        // Cancel the old owner before installing this generation so its
        // global recovery flag cannot affect the new session's error path.
        self.cancel_recovery().await;
        self.persist_current_history()
            .map_err(|_| "Could not save the previous session history.".to_string())?;
        let preferences = self.settings.preferences();
        *self.active_audio_input.lock().unwrap() = preferences.audio_input;
        self.controller
            .lock()
            .unwrap()
            .set_audio_input(preferences.audio_input);
        let started_at_ms = crate::core::subtitle_reducer::now_epoch_ms();
        let history_id = (preferences.retain_session_history || preferences.record_session_audio)
            .then(|| uuid::Uuid::new_v4().to_string());
        if let Some(id) = &history_id {
            if !self.history.available() && !self.is_ui_test() {
                return Err("Local session history is unavailable.".to_string());
            }
            self.history
                .begin(id, started_at_ms)
                .map_err(|_| "Could not start local session history.".to_string())?;
        }
        self.active_generation
            .store(request_generation, Ordering::SeqCst);
        self.stopping_tail_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        self.is_paused.store(false, Ordering::SeqCst);
        *self.history_pending_id.lock().unwrap() = history_id;
        self.history_pending_text
            .store(preferences.retain_session_history, Ordering::SeqCst);
        self.history_pending_audio
            .store(preferences.record_session_audio, Ordering::SeqCst);
        *self.history_stats.lock().unwrap() = LocalCaptureStats::default();
        self.history_save_error.store(false, Ordering::SeqCst);
        self.archive_revision.fetch_add(1, Ordering::SeqCst);
        self.controller.lock().unwrap().archive_mut().begin(
            self.is_ui_test() && preferences.retain_session_history,
            started_at_ms,
        );
        for recording in self.recording.lock().unwrap().iter_mut() {
            recording.begin(false);
        }
        self.controller.lock().unwrap().begin_session();
        self.publish_state();
        // UI fixtures must never read credentials, open a socket, or touch
        // native audio capture. This branch intentionally runs before resolving
        // settings because that resolution reads the OS credential store.
        if self.is_ui_test() {
            if self.is_generation_current(request_generation) {
                self.establish_ui_test_session();
            }
            return Ok(());
        }
        if let Err(error) = self.settings.prepare_for_listening() {
            if self.invalidate_generation(request_generation) {
                self.record_diagnostic_failure(&error);
                self.controller.lock().unwrap().did_fail(error.clone());
                self.publish_state();
                return Err(error);
            }
            return Err(SESSION_START_CANCELLED.into());
        }
        let configuration = match self.settings.configuration() {
            Ok(configuration) => configuration,
            Err(error) => {
                pipeline_log!("session settings failed label=settings_configuration");
                if self.invalidate_generation(request_generation) {
                    self.record_diagnostic_failure(&error);
                    self.controller.lock().unwrap().did_fail(error.clone());
                    self.publish_state();
                    return Err(error);
                }
                return Err(SESSION_START_CANCELLED.into());
            }
        };
        pipeline_log!(
            "session start requested provider={} source={} target={} mode={:?}",
            configuration.provider.wire_value(),
            configuration.source_language.raw_value(),
            configuration.target_language.raw_value(),
            configuration.effective_translation_mode()
        );

        self.set_active_settings(request_generation, Some(configuration));
        // All shared state is now generation-tagged. Do not hold the
        // lifecycle gate across socket setup or capture authorization: stop
        // must be able to invalidate this generation immediately.
        drop(lifecycle);
        self.establish_session(request_generation).await
    }

    async fn establish_session(self: &Arc<Self>, generation: u64) -> Result<(), String> {
        self.stop_health_checks().await;

        self.ensure_generation_current(generation)?;

        if self.is_ui_test() {
            self.establish_ui_test_session();
            return Ok(());
        }

        // A language/mode switch may land while connecting (the picker stays
        // usable during connecting); at most one rebuild picks up the change
        // so a rapid switch storm cannot loop forever.
        let mut attempts = 0usize;
        loop {
            attempts += 1;
            let configuration = match self.active_settings.lock().unwrap().clone() {
                Some(configuration) => configuration,
                None => {
                    let configuration = self.settings.configuration()?;
                    self.set_active_settings(generation, Some(configuration.clone()));
                    configuration
                }
            };

            pipeline_log!("session connecting");
            self.ensure_generation_current(generation)?;
            self.controller.lock().unwrap().begin_connecting();
            self.publish_state();

            let result = Arc::clone(self)
                .connect_and_listen(configuration.clone(), generation)
                .await;
            if let Err(error) = result {
                // Apple setup refreshes the resource cache before starting.
                // Publish even rejected or superseded starts so every picker
                // drops resources that disappeared since its last snapshot.
                if configuration.provider == ProviderKind::AppleSpeech {
                    self.publish_settings();
                }
                pipeline_log!("session establish failed label=provider_or_capture_setup");
                // Claim failure before cleanup can await. Otherwise a provider
                // pump can claim recovery during teardown and then be aborted
                // before it installs the retry task.
                let Some(failure_epoch) = self.invalidate_generation_with_epoch(generation) else {
                    self.cleanup_generation_without_pump(generation).await;
                    return Err(SESSION_START_CANCELLED.into());
                };
                self.cleanup_generation(generation).await;
                let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
                if self.lifecycle_sequence.load(Ordering::SeqCst) != failure_epoch {
                    return Err(SESSION_START_CANCELLED.into());
                }
                let is_recovering = self.is_recovering.load(Ordering::SeqCst);
                if !is_recovering {
                    self.clear_active_settings_for_generation(generation);
                }
                self.record_diagnostic_failure(&error);
                apply_establish_failure_state(
                    &mut self.controller.lock().unwrap(),
                    error.clone(),
                    is_recovering,
                );
                self.publish_state();
                return Err(error);
            }

            self.ensure_generation_current(generation)?;

            // The picker allows switching while connecting; if the settings
            // changed under the in-flight connect, rebuild the session with
            // the fresh configuration instead of going live with the stale
            // one (the user sees the capsule state and the session diverge
            // otherwise).
            let fresh = self.settings.configuration().ok();
            let stale = fresh.as_ref().is_none_or(|fresh| fresh != &configuration);
            if !stale || attempts >= 2 {
                return Ok(());
            }
            if let Some(fresh) = fresh {
                self.set_active_settings(generation, Some(fresh));
            }
            pipeline_log!("session rebuild for settings changed mid-connect");
            self.cleanup_generation(generation).await;
            self.ensure_generation_current(generation)?;
        }
    }

    fn connect_and_listen(
        self: Arc<Self>,
        configuration: LiveTranslationConfiguration,
        generation: u64,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
        Box::pin(async move {
            let attempt = self.next_background_task_id();
            {
                let _transition = self.generation_transition.lock().unwrap();
                self.ensure_generation_current(generation)?;
                *self.audio_attempt.lock().unwrap() = Some((generation, attempt));
            }
            if *self.active_audio_input.lock().unwrap() == AudioInput::Both {
                let echo = Arc::new(
                    EchoPipeline::spawn(
                        configuration.capabilities().input_sample_rate_hz,
                        self.capture_failure_channel(generation, attempt),
                    )
                    .map_err(|error| error.to_string())?,
                );
                let _transition = self.generation_transition.lock().unwrap();
                self.ensure_generation_current(generation)?;
                let mut slot = self.echo_pipeline.lock().unwrap();
                if slot.is_some() {
                    return Err("An audio processor is already assigned to a session.".into());
                }
                *slot = Some((generation, echo));
            }
            connect_source_group(
                self.sources(),
                |source| {
                    Arc::clone(&self).connect_source(
                        configuration.clone(),
                        generation,
                        attempt,
                        source,
                    )
                },
                |pumps| self.cleanup_generation_resources(generation, pumps),
            )
            .await?;
            self.commit_listening(generation)?;
            self.publish_state();
            self.start_health_checks(generation).await;
            self.ensure_generation_current(generation)?;
            Ok(())
        })
    }

    fn connect_source(
        self: Arc<Self>,
        configuration: LiveTranslationConfiguration,
        generation: u64,
        attempt: u64,
        source: AudioSource,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
        Box::pin(async move {
            // Create the client and consume its events through this manager.
            let (event_tx, mut event_rx) = provider_event_channel();
            event_tx.set_debug_context(source, generation);
            let new_client = TranslationClient::new(&configuration, event_tx).map_err(|error| {
                pipeline_log!(
                    "provider client creation failed label={}",
                    error.diagnostic_label()
                );
                error.to_string()
            })?;
            self.ensure_generation_current(generation)?;
            let scope =
                self.settings.active_profile().ok().and_then(|profile| {
                    MTBudgetScope::for_configuration(profile.id, &configuration)
                });
            let budget = {
                let _transition = self.generation_transition.lock().unwrap();
                self.ensure_generation_current(generation)?;
                if !self.is_lifecycle_request_current(generation) {
                    return Err(SESSION_START_CANCELLED.into());
                }
                self.lane(source)
                    .mt_budget_continuity
                    .lock()
                    .unwrap()
                    .prepare(generation, scope)
            };
            new_client.restore_mt_request_budget(budget).await;
            self.ensure_generation_current(generation)?;
            self.install_client(source, generation, new_client).await?;

            // Start consuming before awaiting setup: a provider may acknowledge
            // setup and immediately send a terminal error/close in the same
            // socket read. The terminal event invalidates this generation, and
            // every later startup step checks it before installing resources.
            let self_arc = Arc::clone(&self);
            let pump = tokio::spawn(async move {
                while let Some(event) = event_rx.recv_with_revision().await {
                    self_arc.handle_event(source, generation, event).await;
                }
            });
            self.install_pump(source, generation, pump);

            let client = self
                .client_for_generation(source, generation)
                .ok_or_else(|| SESSION_START_CANCELLED.to_string())?;
            let connect_result = self
                .run_while_generation_current(
                    generation,
                    crate::development_audio::scope(source, generation, client.connect()),
                )
                .await;
            let connect_result = match connect_result {
                Ok(result) => result,
                Err(error) => {
                    self.remember_mt_request_budget(source, generation, &client)
                        .await;
                    let _ = tokio::time::timeout(Duration::from_secs(1), client.disconnect()).await;
                    return Err(error);
                }
            };
            connect_result.map_err(|error| error.to_string())?;
            tokio::task::yield_now().await;
            if let Err(error) = self.ensure_generation_current(generation) {
                self.remember_mt_request_budget(source, generation, &client)
                    .await;
                let _ = tokio::time::timeout(Duration::from_secs(1), client.disconnect()).await;
                return Err(error);
            }
            pipeline_log!("asr websocket connected");

            // Create the sole bounded audio queue before capture starts. The
            // native callback writes directly to this synchronous ingress;
            // there is no unbounded bridge ahead of the network sender.
            let audio_format =
                AudioCaptureFormat::pcm16_mono(configuration.capabilities().input_sample_rate_hz)
                    .map_err(|error| error.to_string())?;
            let send_manager = Arc::clone(&self);
            let on_error_self = Arc::clone(&self);
            let pipeline = Arc::new(AudioSendPipeline::spawn(
                move |data| {
                    let manager = Arc::clone(&send_manager);
                    Box::pin(async move {
                        manager.record_audio(
                            source,
                            generation,
                            audio_format.sample_rate_hz,
                            &data,
                        );
                        let client = manager.client_for_generation(source, generation);
                        match client {
                            Some(client) => crate::development_audio::scope(
                                source,
                                generation,
                                client.send_audio(&data),
                            )
                            .await
                            .map_err(|_| ()),
                            None => Err(()),
                        }
                    })
                },
                move |failure| {
                    let manager = Arc::clone(&on_error_self);
                    tokio::spawn(async move {
                        manager
                            .handle_audio_transport_failure(generation, attempt, failure)
                            .await;
                    });
                },
            ));
            let audio_ingress = pipeline
                .ingress()
                .ok_or_else(|| "The bounded audio pipeline is unavailable.".to_string())?;
            let audio_ingress = {
                let echo = self.echo_pipeline.lock().unwrap();
                match echo.as_ref().filter(|(owner, _)| *owner == generation) {
                    Some((_, echo)) => echo.ingress(source, audio_ingress),
                    None => audio_ingress,
                }
            };
            self.ensure_generation_current(generation)?;
            self.install_pipeline(source, generation, Arc::clone(&pipeline))?;
            client.set_audio_pending_gate(pipeline.pending_pcm_gate());

            let audio_failure_tx = self.capture_failure_channel(generation, attempt);
            self.ensure_generation_current(generation)?;
            self.lane(source)
                .capture_generation
                .compare_exchange(
                    NO_GENERATION,
                    generation,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .map_err(|_| "Audio capture is already assigned to a session.".to_string())?;
            let capture = self.lane(source).audio.lock().unwrap().clone();
            #[cfg(target_os = "windows")]
            capture.set_source(self.settings.preferences().windows_audio_source);
            match self
                .run_while_generation_current(
                    generation,
                    capture.start(
                        audio_ingress,
                        audio_failure_tx,
                        audio_format,
                        source,
                        self.settings.preferences().system_audio_target,
                    ),
                )
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    // A platform capture may fail after it has installed a
                    // native stream but before its start acknowledgement is
                    // delivered. Always request generation-scoped teardown;
                    // the platform implementation also cleans up its own
                    // start token before returning this error.
                    self.stop_capture_for_generation(source, generation).await;
                    return Err(error.to_string());
                }
                Err(error) => {
                    self.stop_capture_for_generation(source, generation).await;
                    return Err(error);
                }
            }
            if let Err(error) = self.ensure_generation_current(generation) {
                self.stop_capture_for_generation(source, generation).await;
                return Err(error);
            }
            pipeline_log!("audio capture started");

            Ok(())
        })
    }

    pub async fn stop(self: &Arc<Self>) {
        self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
            action: LifecycleAction::StopRequested,
        });
        self.record_recovery_action(RecoveryAction::UserStopped);
        let _operation = self.begin_lifecycle_operation();
        let _stop_request = self.next_lifecycle_request();
        for lane in &self.lanes {
            lane.mt_budget_continuity.lock().unwrap().reset();
        }
        let stopping_generation = self.active_generation.swap(NO_GENERATION, Ordering::SeqCst);
        if stopping_generation != NO_GENERATION {
            self.stopping_tail_generation
                .store(stopping_generation, Ordering::SeqCst);
        }
        let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        self.retire_audio_attempt(stopping_generation);
        if !self.controller.lock().unwrap().state.status.is_active()
            && self.lanes.iter().all(|lane| {
                lane.client.lock().unwrap().is_none()
                    && lane.capture_generation.load(Ordering::SeqCst) == NO_GENERATION
            })
        {
            if let Some(echo) = self.take_echo_pipeline(stopping_generation) {
                echo.stop();
            }
            self.stop_health_checks().await;
            self.cancel_recovery().await;
            self.cancel_translation_timeout();
            self.clear_active_settings();
            self.is_recovering.store(false, Ordering::SeqCst);
            self.is_paused.store(false, Ordering::SeqCst);
            self.stopping_tail_generation
                .store(NO_GENERATION, Ordering::SeqCst);
            self.controller.lock().unwrap().did_stop();
            if self.persist_current_history().is_err() {
                pipeline_log!("session history save failed label=write_failed");
            }
            publish_stop_boundary(|| self.publish_state(), self.publish_state_now()).await;
            return;
        }
        pipeline_log!("session stop requested");

        self.stop_health_checks().await;
        self.cancel_recovery().await;
        self.cancel_translation_timeout();
        self.clear_active_settings();
        self.is_recovering.store(false, Ordering::SeqCst);
        self.is_paused.store(false, Ordering::SeqCst);
        self.controller.lock().unwrap().begin_stopping();
        self.publish_state();

        if let Some((_, echo)) = self.echo_pipeline.lock().unwrap().as_ref() {
            echo.seal();
        }

        // Stop capture first so the queue has a fixed upper bound, then give
        // buffers already accepted by the network pipeline a finite drain
        // window before asking the provider to close its session.
        if stopping_generation != NO_GENERATION {
            tokio::join!(
                self.stop_capture_for_generation(AudioSource::System, stopping_generation),
                self.stop_capture_for_generation(AudioSource::Microphone, stopping_generation),
            );
        } else {
            tokio::join!(
                self.stop_any_capture(AudioSource::System),
                self.stop_any_capture(AudioSource::Microphone)
            );
        }
        if let Some(echo) = self.take_echo_pipeline(stopping_generation) {
            if !echo.finish(Duration::from_secs(1)).await {
                pipeline_log!("audio processing drain timed out");
            }
        }
        for source in [AudioSource::System, AudioSource::Microphone] {
            if stopping_generation != NO_GENERATION {
                self.finish_pipeline_for_generation(
                    source,
                    stopping_generation,
                    Duration::from_secs(1),
                )
                .await;
            } else {
                self.finish_source_pipeline(source, Duration::from_secs(1))
                    .await;
            }
            let taken = if stopping_generation == NO_GENERATION {
                self.take_any_client(source)
            } else {
                self.take_client_for_generation(source, stopping_generation)
            };
            if let Some(client) = taken {
                if tokio::time::timeout(
                    Duration::from_millis(
                        mimi_core::translation_policy::PROVIDER_FINISH_TIMEOUT_MS,
                    ),
                    crate::development_audio::scope(source, stopping_generation, client.finish()),
                )
                .await
                .is_err()
                {
                    pipeline_log!("provider finish timed out");
                    let _ = tokio::time::timeout(Duration::from_secs(1), client.disconnect()).await;
                }
            }
            if stopping_generation != NO_GENERATION {
                self.finish_pump_for_generation(
                    source,
                    stopping_generation,
                    Duration::from_millis(500),
                )
                .await;
            } else {
                self.stop_any_pump(source);
            }
        }
        self.stopping_tail_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        self.controller.lock().unwrap().did_stop();
        if self.persist_current_history().is_err() {
            pipeline_log!("session history save failed label=write_failed");
        }
        publish_stop_boundary(|| self.publish_state(), self.publish_state_now()).await;
        pipeline_log!("session stopped");
    }

    pub async fn toggle_paused(self: &Arc<Self>) -> Result<(), String> {
        if self.is_ui_test() {
            let paused = self.is_paused.load(Ordering::SeqCst);
            self.is_paused.store(!paused, Ordering::SeqCst);
            if paused {
                self.controller.lock().unwrap().did_connect();
            } else {
                self.controller.lock().unwrap().did_pause();
            }
            self.publish_state();
            return Ok(());
        }
        if self.is_paused() {
            self.resume().await
        } else {
            self.pause().await;
            Ok(())
        }
    }

    pub async fn pause(self: &Arc<Self>) {
        if !self.can_pause_current_session() {
            return;
        }
        let _operation = self.begin_lifecycle_operation();
        let pause_request = self.next_lifecycle_request();
        let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        if !self.is_lifecycle_request_current(pause_request) {
            return;
        }
        let status = self.controller.lock().unwrap().state.status.clone();
        if !accepted_pause_transition_is_valid(
            &status,
            self.is_paused(),
            self.active_generation.load(Ordering::SeqCst),
            self.active_settings.lock().unwrap().is_some(),
        ) {
            return;
        }
        let paused_generation = self.active_generation.swap(NO_GENERATION, Ordering::SeqCst);
        self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
            action: LifecycleAction::PauseRequested,
        });
        pipeline_log!("session pause requested");
        self.is_paused.store(true, Ordering::SeqCst);
        self.stop_health_checks().await;
        self.cancel_recovery().await;
        self.cancel_translation_timeout();
        self.is_recovering.store(false, Ordering::SeqCst);
        self.controller.lock().unwrap().did_pause();
        self.publish_state();
        if paused_generation != NO_GENERATION {
            self.cleanup_generation(paused_generation).await;
        }
        pipeline_log!("session paused");
    }

    pub async fn resume(self: &Arc<Self>) -> Result<(), String> {
        if !self.can_resume_current_session() {
            return Ok(());
        }
        let _operation = self.begin_lifecycle_operation();
        let resume_generation = self.next_lifecycle_request();
        let lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        if !self.is_lifecycle_request_current(resume_generation)
            || !self.can_resume_current_session()
        {
            return Ok(());
        }
        pipeline_log!("session resume requested");
        self.record_diagnostic_event(DiagnosticEvent::Lifecycle {
            action: LifecycleAction::ResumeRequested,
        });
        let paused_configuration = self.active_settings.lock().unwrap().clone();
        self.is_paused.store(false, Ordering::SeqCst);
        self.active_generation
            .store(resume_generation, Ordering::SeqCst);
        self.retag_active_settings(resume_generation);
        drop(lifecycle);
        let error = match self.establish_session(resume_generation).await {
            Ok(()) => {
                pipeline_log!("session resumed");
                return Ok(());
            }
            Err(error) if error == SESSION_START_CANCELLED => return Ok(()),
            Err(error) => error,
        };
        let restored = restore_failed_resume(
            Arc::clone(&self.lifecycle_lock),
            || {
                resume_failure_is_still_owned(
                    &error,
                    resume_generation.wrapping_add(1),
                    self.lifecycle_sequence.load(Ordering::SeqCst),
                    &self.controller.lock().unwrap().state.status,
                    self.active_generation.load(Ordering::SeqCst),
                )
            },
            || {
                self.set_active_settings(NO_GENERATION, paused_configuration);
                self.is_paused.store(true, Ordering::SeqCst);
                self.controller.lock().unwrap().did_pause();
            },
        )
        .await;
        if !restored {
            return Ok(());
        }
        // Deliver the restored paused state before rejecting the IPC action.
        // A newer stop/resume can still win; publish only its current snapshot.
        self.publish_state_now().await;
        pipeline_log!("session resume failed; remaining paused");
        Err(error)
    }

    /// The compact audio switches are explicit live reconfiguration. Preserve
    /// confirmed subtitles, revoke recording consent, and restart only when
    /// currently listening; a paused session must never briefly open a mic.
    pub async fn switch_audio_input(self: &Arc<Self>, input: AudioInput) -> Result<(), String> {
        self.switch_audio_capture(Some(input), None).await
    }

    pub async fn switch_system_audio_target(
        self: &Arc<Self>,
        target: SystemAudioTarget,
    ) -> Result<(), String> {
        validate_system_audio_target(
            &target,
            self.is_ui_test(),
            crate::audio::applications::supported,
        )?;
        self.switch_audio_capture(None, Some(target)).await
    }

    async fn switch_audio_capture(
        self: &Arc<Self>,
        input: Option<AudioInput>,
        target: Option<SystemAudioTarget>,
    ) -> Result<(), String> {
        let switch_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
        let lifecycle = self.settings_mutation_guard(false).await?;
        if !self.is_lifecycle_request_current(switch_epoch) {
            return Err("audio_input_switch_superseded".into());
        }
        let status = self.controller.lock().unwrap().state.status.clone();
        let action = audio_input_switch_action(
            &status,
            self.is_paused(),
            self.is_recovering.load(Ordering::SeqCst),
            self.lifecycle_operations.load(Ordering::SeqCst),
        )
        .map_err(str::to_owned)?;
        let preferences = self.settings.preferences();
        let input = input.unwrap_or(preferences.audio_input);
        let target = target.unwrap_or_else(|| preferences.system_audio_target.clone());
        if preferences.audio_input == input && preferences.system_audio_target == target {
            return Ok(());
        }
        let persist = || {
            self.settings
                .save_preferences(|prefs| {
                    prefs.apply_audio_preferences(Some(input), None);
                    prefs.apply_system_audio_target(target.clone());
                })
                .map_err(|_| "audio_input_switch_save_failed".to_string())
        };
        if !audio_capture_selection_needs_restart(preferences.audio_input, input) {
            {
                let _transition = self.generation_transition.lock().unwrap();
                if !self.is_lifecycle_request_current(switch_epoch) {
                    return Err("audio_input_switch_superseded".into());
                }
                persist()?;
            }
            self.apply_archive_opt_out(None, Some(false));
            self.publish_settings();
            return Ok(());
        }
        let previous_input = *self.active_audio_input.lock().unwrap();
        let _operation = self.begin_lifecycle_operation();
        let (generation, old_generation) = {
            let _content = self.subtitle_content_lock.lock().await;
            let _transition = self.generation_transition.lock().unwrap();
            let boundary = commit_audio_input_switch_boundary(
                &self.active_generation,
                &self.lifecycle_sequence,
                switch_epoch,
                &mut self.controller.lock().unwrap(),
                input,
                persist,
            )?;
            *self.active_audio_input.lock().unwrap() = input;
            self.lifecycle_notify.notify_waiters();
            boundary
        };
        self.apply_archive_opt_out(None, Some(false));
        self.publish_settings();
        self.stop_health_checks().await;
        self.cancel_recovery().await;
        self.cancel_translation_timeout();
        if action == AudioInputSwitchAction::Reconnect {
            self.controller.lock().unwrap().begin_connecting();
        }
        self.publish_state();
        let lifecycle = retain_lifecycle_during_teardown(lifecycle, async {
            if old_generation != NO_GENERATION {
                self.cleanup_generation(old_generation).await;
            }
        })
        .await;
        if !self.is_lifecycle_request_current(generation) {
            return Err("audio_input_switch_superseded".into());
        }
        if !self.is_ui_test() {
            // A target change keeps the system lane selected but retires its
            // former native application capture just like disabling that lane.
            let released = release_reconfigured_audio_sources(
                previous_input,
                input,
                preferences.system_audio_target != target,
                |source| {
                    let capture = self.lane(source).audio.lock().unwrap().clone();
                    async move {
                        match tokio::time::timeout(Duration::from_secs(5), capture.stop_and_wait())
                            .await
                        {
                            Ok(Ok(())) => Ok(()),
                            _ => Err("audio_input_switch_stop_failed".into()),
                        }
                    }
                },
            )
            .await;
            if let Err(error) = released {
                if self.is_lifecycle_request_current(generation) {
                    self.clear_active_settings();
                    self.is_paused.store(false, Ordering::SeqCst);
                    self.controller.lock().unwrap().did_fail(error.clone());
                    self.publish_state();
                }
                return Err(error);
            }
        }
        if !self.is_lifecycle_request_current(generation) {
            return Err("audio_input_switch_superseded".into());
        }
        if action == AudioInputSwitchAction::ReconfigureOnly {
            self.publish_state();
            return Ok(());
        }
        self.active_generation.store(generation, Ordering::SeqCst);
        self.retag_active_settings(generation);
        drop(lifecycle);
        self.establish_session(generation).await.map_err(|error| {
            if error == SESSION_START_CANCELLED {
                "audio_input_switch_superseded".into()
            } else {
                error
            }
        })
    }

    /// Selects a saved profile while retaining the current session and its
    /// selected sources. A live selection reconnects; a paused one only
    /// replaces the configuration used by Resume.
    pub async fn switch_profile(self: &Arc<Self>, profile_id: &str) -> Result<(), String> {
        self.switch_profile_with_source(profile_id, None).await
    }

    pub async fn switch_profile_with_source(
        self: &Arc<Self>,
        profile_id: &str,
        source_language: Option<SourceLanguage>,
    ) -> Result<(), String> {
        let switch_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
        let lifecycle = self
            .settings_mutation_guard(source_language.is_some())
            .await?;
        if !self.is_lifecycle_request_current(switch_epoch) {
            return Err("profile_switch_superseded".into());
        }
        let status = self.controller.lock().unwrap().state.status.clone();
        let action = profile_switch_action(
            &status,
            self.is_paused(),
            self.is_recovering.load(Ordering::SeqCst),
            self.lifecycle_operations.load(Ordering::SeqCst),
        )
        .map_err(str::to_owned)?;
        let current_profile = self.settings.active_profile()?;
        if current_profile.id == profile_id
            && source_language.is_none()
            && current_profile.language_preset.is_none()
        {
            return Ok(());
        }
        let configuration_failure = self.configuration_failure_snapshot();
        let (_, profiles) = self.settings.profile_catalog()?;
        let profile = profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| "The service profile does not exist.".to_string())?;
        let selection = self
            .settings
            .preferences_for_profile_selection(profile, source_language)?;
        if profile.provider == ProviderKind::LocalSpeech {
            let _lease = crate::local_models::manager()
                .and_then(|manager| manager.acquire(profile.local_speech_model))
                .map_err(str::to_owned)?;
        }
        let apple_support = if profile.provider == ProviderKind::AppleSpeech {
            if self.is_ui_test() {
                return Err("apple_speech_ui_test_unavailable".into());
            }
            let support = crate::apple_speech_support::refresh()
                .await
                .and_then(|support| {
                    if support.available {
                        Ok(support)
                    } else {
                        Err("apple_speech_unavailable".into())
                    }
                })
                .inspect_err(|_| self.publish_settings())?;
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("profile_switch_superseded".into());
            }
            Some(support)
        } else {
            None
        };
        let live = action != ProfileSwitchAction::SelectOnly;
        if profile.text_translation() == crate::core::provider::TextTranslation::Apple {
            if self.is_ui_test() && selection.target_language.translates_audio() {
                return Err("apple_translation_unavailable".into());
            }
            if !live
                && selection.source_language == SourceLanguage::Automatic
                && selection.target_language.translates_audio()
            {
                // Idle metadata can await an explicit source choice; do not
                // guess another language merely to activate the editor.
                if !crate::apple_translation_support::refresh().await?.available {
                    return Err("apple_translation_unavailable".into());
                }
            } else {
                crate::apple_translation_support::validate_pair(
                    selection.source_language,
                    selection.target_language,
                    live || (source_language.is_none() && profile.language_preset.is_some()),
                )
                .await?;
            }
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("profile_switch_superseded".into());
            }
        }
        // Validate before persistence: an incomplete profile must not replace
        // the working session. UI fixtures never resolve credentials.
        let proposed = if live && !self.is_ui_test() {
            Some(match source_language {
                Some(source) => self
                    .settings
                    .configuration_for_profile_selection_with_source(profile, Some(source))?,
                None => self.settings.configuration_for_profile_selection(profile)?,
            })
        } else {
            None
        };
        if let Some(support) = apple_support {
            crate::apple_speech_support::validate_refreshed_profile_source(
                Ok(support),
                profile,
                selection.source_language,
                selection.target_language,
                live || source_language.is_some() || profile.language_preset.is_some(),
                || self.publish_settings(),
            )?;
        }
        if live {
            let target = self.settings.preferences().target_language;
            let current_rate = self
                .active_settings
                .lock()
                .unwrap()
                .as_ref()
                .map(|configuration| configuration.capabilities().input_sample_rate_hz)
                .unwrap_or_else(|| current_profile.capabilities(target).input_sample_rate_hz);
            let next_rate = proposed
                .as_ref()
                .map(|configuration| configuration.capabilities().input_sample_rate_hz)
                .unwrap_or_else(|| profile.capabilities(target).input_sample_rate_hz);
            // Every session audio track has one fixed PCM rate. Keep its
            // existing bytes and consent intact rather than dropping a track
            // or silently ceasing recording after a 16/24 kHz switch.
            ensure_profile_switch_audio_format(
                self.history_pending_audio.load(Ordering::SeqCst),
                current_rate,
                next_rate,
            )?;
        }
        {
            let _transition = self.generation_transition.lock().unwrap();
            commit_profile_selection(
                &self.lifecycle_sequence,
                switch_epoch,
                &self.active_settings,
                proposed,
                || {
                    match source_language {
                        Some(source) => self
                            .settings
                            .select_profile_with_source(profile_id, Some(source)),
                        None => self.settings.select_profile(profile_id),
                    }
                    .map_err(|_| "profile_switch_save_failed".to_string())
                },
            )?;
        }
        // The persisted selection repairs only the failure observed under
        // this lifecycle guard. Keep subtitles and never start an idle session.
        self.configuration_saved(configuration_failure, true);
        self.publish_settings();
        if action == ProfileSwitchAction::Reconnect {
            drop(lifecycle);
            return self
                .try_reconnect_if_current(switch_epoch)
                .await
                .map_err(|error| {
                    if error == "language_switch_superseded" {
                        "profile_switch_superseded".into()
                    } else {
                        error
                    }
                });
        }
        Ok(())
    }

    /// Quick-switches the source language, reconnecting when needed.
    /// Source changes preserve the single Turbo path for every provider.
    pub async fn switch_source_language(
        self: &Arc<Self>,
        language: SourceLanguage,
    ) -> Result<(), String> {
        let switch_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
        let lifecycle = self
            .settings_mutation_guard(false)
            .await
            .map_err(|_| "source_switch_busy".to_string())?;
        if !self.is_lifecycle_request_current(switch_epoch) {
            return Err("source_switch_superseded".into());
        }
        let status = self.controller.lock().unwrap().state.status.clone();
        if !pipeline_settings_mutation_is_allowed(
            &status,
            self.lifecycle_operations.load(Ordering::SeqCst),
        ) {
            return Err("source_switch_busy".into());
        }
        let configuration_failure = self.configuration_failure_snapshot();
        let profile = self
            .settings
            .active_profile()
            .map_err(|_| "source_switch_profile".to_string())?;
        let provider = profile.effective_provider();
        let prefs = self.settings.preferences();
        if provider == ProviderKind::AppleSpeech {
            if self.is_ui_test() {
                return Err("apple_speech_ui_test_unavailable".into());
            }
            // Validate once while holding the lifecycle guard. Refusals must
            // reach IPC instead of looking like a successful language change.
            let support = crate::apple_speech_support::refresh().await;
            let installed_required = self.has_active_session() || self.is_paused();
            crate::apple_speech_support::validate_refreshed_profile_source(
                support,
                &profile,
                language,
                prefs.target_language,
                installed_required,
                || self.publish_settings(),
            )?;
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("source_switch_superseded".into());
            }
        }
        let capabilities = profile.capabilities(prefs.target_language);
        if !capabilities.source_languages.contains(&language) {
            return Err("source_switch_unsupported".into());
        }
        let (target_language, next_mode, needs_reconnect) = {
            // Menus and settings share explicit source-selection semantics:
            // retain the target unless this route requires normalization.
            let target =
                capabilities.target_language_after_source_switch(language, prefs.target_language);
            if !profile
                .capabilities(target)
                .source_languages
                .contains(&language)
            {
                return Err("source_switch_unsupported".into());
            }
            let mode =
                translation_mode_after_source_switch(provider, language, prefs.translation_mode);
            let needs_reconnect = source_switch_requires_reconnect(
                self.controller.lock().unwrap().state.status == SessionStatus::Listening,
                prefs.source_language,
                prefs.target_language,
                prefs.translation_mode,
                language,
                target,
                mode,
            );
            (target, mode, needs_reconnect)
        };
        if profile.text_translation() == crate::core::provider::TextTranslation::Apple {
            if self.is_ui_test() && target_language.translates_audio() {
                return Err("apple_translation_unavailable".into());
            }
            crate::apple_translation_support::validate_pair(
                language,
                target_language,
                self.has_active_session() || self.is_paused(),
            )
            .await?;
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("source_switch_superseded".into());
            }
        }
        if self
            .settings
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = language;
                prefs.target_language = target_language;
                prefs.translation_mode = next_mode;
            })
            .is_err()
        {
            pipeline_log!("preferences unavailable label=source_switch_write_failed");
            return Err("source_switch_save_failed".into());
        }
        self.configuration_saved(configuration_failure, prefs.source_language != language);
        // A pause/stop can claim a newer lifecycle epoch while waiting for
        // this guard. Keep the current session's immutable settings snapshot
        // in sync before releasing the guard so a later pause/resume cannot
        // revive the pre-switch configuration even if reconnect loses that
        // newer-intent race.
        update_owned_value(
            &self.active_settings,
            &self.active_settings_generation,
            |configuration| {
                configuration.source_language = language;
                configuration.target_language = target_language;
                configuration.translation_mode = next_mode;
            },
        );
        // Broadcast immediately: the reconnect below can take seconds, and
        // every window (including the overlay control) must see the new
        // selection right away.
        self.publish_settings();
        if self.is_paused() || !needs_reconnect {
            return Ok(());
        }

        pipeline_log!(
            "session language switch source={} target={}",
            language.raw_value(),
            target_language.raw_value()
        );
        drop(lifecycle);
        // Preferences already identify the selected language. A reconnect
        // failure remains a session error and is also returned to its control.
        self.try_reconnect_if_current(switch_epoch).await
    }

    /// Switches to a supported translation target or recognition-only mode.
    /// Shares the source-switch lifecycle: paused sessions stay paused, live
    /// sessions reconnect, and newer stop/pause requests supersede reconnect.
    pub async fn switch_target_language(
        self: &Arc<Self>,
        target: crate::core::models::TargetLanguage,
    ) -> Result<(), String> {
        use crate::core::provider::ProviderPreferences;
        let switch_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
        let lifecycle = self.settings_mutation_guard(false).await?;
        let status = self.controller.lock().unwrap().state.status.clone();
        if !self.is_lifecycle_request_current(switch_epoch)
            || !pipeline_settings_mutation_is_allowed(
                &status,
                self.lifecycle_operations.load(Ordering::SeqCst),
            )
        {
            return Err("target_switch_busy".into());
        }
        let profile = self
            .settings
            .active_profile()
            .map_err(|_| "target_switch_profile")?;
        let prefs = self.settings.preferences();
        let selection = profile
            .preferences_after_target_switch(
                ProviderPreferences {
                    source_language: prefs.source_language,
                    target_language: prefs.target_language,
                    translation_mode: prefs.translation_mode,
                },
                target,
            )
            .ok_or("target_switch_unsupported")?;
        if prefs.target_language == target {
            return Ok(());
        }
        if profile.provider == ProviderKind::AppleSpeech {
            if self.is_ui_test() {
                return Err("apple_speech_ui_test_unavailable".into());
            }
            crate::apple_speech_support::validate_refreshed_profile_source(
                crate::apple_speech_support::refresh().await,
                &profile,
                prefs.source_language,
                target,
                self.has_active_session() || self.is_paused(),
                || self.publish_settings(),
            )?;
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("language_switch_superseded".into());
            }
        }
        if profile.text_translation() == crate::core::provider::TextTranslation::Apple {
            if self.is_ui_test() && target.translates_audio() {
                return Err("apple_translation_unavailable".into());
            }
            crate::apple_translation_support::validate_pair(
                prefs.source_language,
                target,
                self.has_active_session() || self.is_paused(),
            )
            .await?;
            if !self.is_lifecycle_request_current(switch_epoch) {
                return Err("language_switch_superseded".into());
            }
        }
        // A session started in Original mode can lack translation credentials.
        // Validate its resumed route before saving, so a missing text key cannot
        // leave the preference and running session on different selections.
        let proposed_configuration = self.active_settings.lock().unwrap().clone();
        let proposed_configuration = proposed_configuration
            .map(|configuration| {
                self.settings
                    .configuration_for_target_switch(&profile, configuration, selection)
            })
            .transpose()?;
        self.settings
            .save_preferences_for_active_profile(|prefs| {
                prefs.source_language = selection.source_language;
                prefs.target_language = selection.target_language;
                prefs.translation_mode = selection.translation_mode;
            })
            .map_err(|_| "target_switch_save_failed")?;
        update_owned_value(
            &self.active_settings,
            &self.active_settings_generation,
            |configuration| {
                if let Some(proposed) = &proposed_configuration {
                    *configuration = proposed.clone();
                }
            },
        );
        self.publish_settings();
        if self.is_paused() || status != SessionStatus::Listening {
            return Ok(());
        }
        drop(lifecycle);
        self.try_reconnect_if_current(switch_epoch).await
    }

    /// Quick-switches the translation mode, reconnecting when needed.
    pub async fn switch_translation_mode(self: &Arc<Self>, mode: TranslationMode) {
        let switch_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
        let lifecycle = match self.settings_mutation_guard(false).await {
            Ok(guard) => guard,
            Err(_) => return,
        };
        if !self.is_lifecycle_request_current(switch_epoch) {
            return;
        }
        let status = self.controller.lock().unwrap().state.status.clone();
        if !pipeline_settings_mutation_is_allowed(
            &status,
            self.lifecycle_operations.load(Ordering::SeqCst),
        ) {
            return;
        }
        let provider = self
            .settings
            .active_profile()
            .map(|profile| profile.effective_provider())
            .unwrap_or(ProviderKind::AlibabaCloud);
        if !provider.capabilities().translation_modes.contains(&mode) {
            return;
        }
        let current = self.settings.preferences().translation_mode;
        if current == mode {
            return;
        }
        if self
            .settings
            .save_preferences_for_active_profile(|prefs| prefs.translation_mode = mode)
            .is_err()
        {
            pipeline_log!("preferences unavailable label=mode_switch_write_failed");
            return;
        }
        update_owned_value(
            &self.active_settings,
            &self.active_settings_generation,
            |configuration| configuration.translation_mode = mode,
        );
        // Broadcast immediately: the reconnect below can take seconds.
        self.publish_settings();
        if self.is_paused() {
            return;
        }
        let is_listening = self.controller.lock().unwrap().state.status == SessionStatus::Listening;
        if !is_listening {
            return;
        }

        pipeline_log!("session translation mode switch mode={:?}", mode);
        drop(lifecycle);
        self.reconnect_if_current(switch_epoch).await;
    }

    /// Tears down and re-establishes the session, keeping subtitles.
    async fn reconnect_if_current(self: &Arc<Self>, expected_epoch: u64) {
        let _ = self.try_reconnect_if_current(expected_epoch).await;
    }

    /// Profile and language controls need the actual reconnect outcome.
    /// Other lifecycle callers retain session-state-only error handling.
    async fn try_reconnect_if_current(self: &Arc<Self>, expected_epoch: u64) -> Result<(), String> {
        let _operation = self.begin_lifecycle_operation();
        let Some(reconnect_generation) = self.advance_lifecycle_request_if_current(expected_epoch)
        else {
            return Err("language_switch_superseded".into());
        };
        let lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        if !self.is_lifecycle_request_current(reconnect_generation) {
            return Err("language_switch_superseded".into());
        }
        let old_generation = self.active_generation.swap(NO_GENERATION, Ordering::SeqCst);
        self.stop_health_checks().await;
        self.cancel_recovery().await;
        self.controller.lock().unwrap().begin_connecting();
        self.publish_state();
        drop(lifecycle);
        if old_generation != NO_GENERATION {
            self.cleanup_generation(old_generation).await;
        }
        let lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        if !self.is_lifecycle_request_current(reconnect_generation) {
            return Err("language_switch_superseded".into());
        }
        self.active_generation
            .store(reconnect_generation, Ordering::SeqCst);
        self.retag_active_settings(reconnect_generation);
        drop(lifecycle);
        self.establish_session(reconnect_generation)
            .await
            .map_err(|error| {
                if error == SESSION_START_CANCELLED {
                    "language_switch_superseded".into()
                } else {
                    error
                }
            })
    }

    /// Machine-readable status kind for the global shortcut gate.
    pub fn status_kind(&self) -> String {
        match self.controller.lock().unwrap().state.status {
            SessionStatus::Idle => "idle".into(),
            SessionStatus::Connecting => "connecting".into(),
            SessionStatus::Listening => "listening".into(),
            SessionStatus::Stopping => "stopping".into(),
            SessionStatus::Error(_) => "error".into(),
        }
    }

    fn record_audio(&self, source: AudioSource, generation: u64, sample_rate: u32, data: &[u8]) {
        if (self.is_generation_current(generation)
            || self.stopping_tail_generation.load(Ordering::SeqCst) == generation)
            && !self.is_paused()
        {
            let pending = self.history_pending_id.lock().unwrap();
            if !self.history_pending_audio.load(Ordering::SeqCst) {
                return;
            }
            let Some(id) = pending.as_deref() else { return };
            let mut stats = self.history_stats.lock().unwrap();
            if stats.audio_limited || data.is_empty() {
                return;
            }
            if !matches!(sample_rate, 16_000 | 24_000)
                || !data.len().is_multiple_of(2)
                || (stats.sample_rate != 0 && stats.sample_rate != sample_rate)
            {
                stats.audio_limited = true;
                return;
            }
            let available =
                crate::core::session_archive::AUDIO_BYTE_LIMIT.saturating_sub(stats.audio_bytes);
            let accepted = data.len().min(available);
            if accepted == 0 {
                stats.audio_limited = true;
                return;
            }
            if self
                .history
                .append_pcm_from(id, source, sample_rate, &data[..accepted])
                .is_err()
            {
                self.history_save_error.store(true, Ordering::SeqCst);
                pipeline_log!("session history audio append failed label=write_failed");
                return;
            }
            stats.sample_rate = sample_rate;
            stats.audio_bytes += accepted;
            stats.audio_limited =
                stats.audio_bytes == crate::core::session_archive::AUDIO_BYTE_LIMIT;
        }
    }

    pub fn archive_revision(&self) -> u64 {
        self.archive_revision.load(Ordering::SeqCst)
    }

    pub fn apply_archive_opt_out(&self, history: Option<bool>, audio: Option<bool>) {
        if history == Some(false) || audio == Some(false) {
            self.archive_revision.fetch_add(1, Ordering::SeqCst);
        }
        if history == Some(false) {
            self.history_pending_text.store(false, Ordering::SeqCst);
            if let Some(id) = self.history_pending_id.lock().unwrap().as_deref() {
                if self.history.clear_text(id).is_err() {
                    self.history_save_error.store(true, Ordering::SeqCst);
                }
            }
            self.controller.lock().unwrap().archive_mut().disable();
            let mut stats = self.history_stats.lock().unwrap();
            stats.transcript_count = 0;
            stats.transcript_bytes = 0;
            stats.transcript_limited = false;
        }
        if audio == Some(false) {
            self.history_pending_audio.store(false, Ordering::SeqCst);
            if let Some(id) = self.history_pending_id.lock().unwrap().as_deref() {
                if self.history.clear_audio(id).is_err() {
                    self.history_save_error.store(true, Ordering::SeqCst);
                }
            }
            for recording in self.recording.lock().unwrap().iter_mut() {
                recording.begin(false);
            }
            let mut stats = self.history_stats.lock().unwrap();
            stats.audio_bytes = 0;
            stats.audio_limited = false;
            stats.sample_rate = 0;
        }
    }

    pub fn archive_state(&self) -> crate::core::session_archive::ArchiveState {
        if self.is_ui_test() {
            let controller = self.controller.lock().unwrap();
            let recording = self.recording.lock().unwrap();
            return crate::core::session_archive::ArchiveState {
                transcript_count: controller.archive().count(),
                transcript_limited: controller.archive().limited,
                audio_sources: [AudioSource::System, AudioSource::Microphone]
                    .into_iter()
                    .zip(recording.iter())
                    .filter_map(|(source, recording)| (recording.len() > 0).then_some(source))
                    .collect(),
                audio_bytes: recording.iter().map(|recording| recording.len()).sum(),
                audio_limited: recording.iter().any(|recording| recording.limited),
                sample_rate: recording
                    .iter()
                    .find(|recording| recording.len() > 0)
                    .map_or(0, |recording| recording.sample_rate),
                history_save_error: false,
            };
        }
        let audio_sources = self
            .history_pending_id
            .lock()
            .unwrap()
            .as_deref()
            .and_then(|id| self.history.audio_sources(id).ok())
            .unwrap_or_default();
        let stats = self.history_stats.lock().unwrap();
        crate::core::session_archive::ArchiveState {
            transcript_count: stats.transcript_count,
            transcript_limited: stats.transcript_limited,
            audio_sources,
            audio_bytes: stats.audio_bytes,
            audio_limited: stats.audio_limited,
            sample_rate: stats.sample_rate,
            history_save_error: self.history_save_error.load(Ordering::SeqCst),
        }
    }

    pub fn history(&self) -> Arc<SessionHistory> {
        Arc::clone(&self.history)
    }

    pub fn current_history_id(&self) -> Option<String> {
        self.history_pending_id.lock().unwrap().clone()
    }

    pub fn persist_current_history(&self) -> std::io::Result<()> {
        let mut pending = self.history_pending_id.lock().unwrap();
        let Some(id) = pending.as_ref() else {
            return Ok(());
        };
        if self.is_ui_test() {
            // Opt-in only within UI-test mode: no history writes, credentials,
            // provider connections or captured audio. New start/clear/opt-out
            // still reset these bounded, synthetic buffers normally.
            let mut controller = self.controller.lock().unwrap();
            for recording in self.recording.lock().unwrap().iter_mut() {
                finish_ui_test_archive(
                    &mut controller,
                    recording,
                    std::env::var("MIMI_UI_TEST_EXPORT").as_deref() == Ok("1"),
                );
            }
            *pending = None;
            self.history_pending_text.store(false, Ordering::SeqCst);
            self.history_pending_audio.store(false, Ordering::SeqCst);
            return Ok(());
        }
        let limited = self.history_stats.lock().unwrap().transcript_limited;
        if let Err(error) =
            self.history
                .finalize(id, crate::core::subtitle_reducer::now_epoch_ms(), limited)
        {
            self.history_save_error.store(true, Ordering::SeqCst);
            return Err(error);
        }
        self.history_save_error.store(false, Ordering::SeqCst);
        *self.history_stats.lock().unwrap() = LocalCaptureStats::default();
        *pending = None;
        self.history_pending_text.store(false, Ordering::SeqCst);
        self.history_pending_audio.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub fn transcript_page(
        &self,
        query: &str,
        page: usize,
    ) -> std::io::Result<crate::core::session_archive::TranscriptPage> {
        if self.is_ui_test() {
            return Ok(self.controller.lock().unwrap().archive().page(query, page));
        }
        let pending = self.history_pending_id.lock().unwrap();
        let Some(id) = pending.as_deref() else {
            return Ok(crate::core::session_archive::TranscriptPage {
                total: 0,
                page: 0,
                entries: Vec::new(),
            });
        };
        self.history.page(id, query, page)
    }

    pub fn export_transcript(&self) -> std::io::Result<Option<Vec<u8>>> {
        if self.is_ui_test() {
            return Ok(self
                .controller
                .lock()
                .unwrap()
                .archive()
                .export()
                .map(String::into_bytes));
        }
        let pending = self.history_pending_id.lock().unwrap();
        match pending.as_deref() {
            Some(id) => self.history.export_transcript(id).map(Some),
            None => Ok(None),
        }
    }

    pub fn export_audio_from(
        &self,
        source: Option<AudioSource>,
    ) -> std::io::Result<Option<Vec<u8>>> {
        if self.is_ui_test() {
            let recordings = self.recording.lock().unwrap();
            let index = match source {
                Some(AudioSource::System) => Some(0),
                Some(AudioSource::Microphone) => Some(1),
                None => {
                    let mut available = recordings
                        .iter()
                        .enumerate()
                        .filter(|(_, recording)| recording.len() > 0);
                    let first = available.next().map(|(index, _)| index);
                    if available.next().is_some() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "Select an audio source to export.",
                        ));
                    }
                    first
                }
            };
            return Ok(index.and_then(|index| recordings[index].export()));
        }
        let pending = self.history_pending_id.lock().unwrap();
        match pending.as_deref() {
            Some(id) => self.history.audio_selected(id, source).map(Some),
            None => Ok(None),
        }
    }

    pub fn clear_archive(&self) -> std::io::Result<()> {
        let mut pending = self.history_pending_id.lock().unwrap();
        if let Some(id) = pending.as_deref() {
            self.history.discard(id)?;
        }
        *pending = None;
        self.history_pending_text.store(false, Ordering::SeqCst);
        self.history_pending_audio.store(false, Ordering::SeqCst);
        *self.history_stats.lock().unwrap() = LocalCaptureStats::default();
        self.history_save_error.store(false, Ordering::SeqCst);
        self.archive_revision.fetch_add(1, Ordering::SeqCst);
        self.controller.lock().unwrap().archive_mut().clear();
        for recording in self.recording.lock().unwrap().iter_mut() {
            recording.clear();
        }
        Ok(())
    }

    pub async fn clear_subtitles(self: &Arc<Self>) -> std::io::Result<()> {
        let _content = self.subtitle_content_lock.lock().await;
        self.clear_current_subtitle_history()?;
        for source in [AudioSource::System, AudioSource::Microphone] {
            let generation = self.lane(source).client_generation.load(Ordering::SeqCst);
            if let Some(client) = self.client_for_generation(source, generation) {
                let revision = client.clear_content().await;
                *self.lane(source).subtitle_content_revision.lock().unwrap() =
                    (generation, revision);
            } else {
                // Keep the retired connection cut even after teardown has taken
                // its client. A previously popped stopping tail must stay stale.
                let mut boundary = self.lane(source).subtitle_content_revision.lock().unwrap();
                boundary.1 = boundary.1.wrapping_add(1);
            }
        }
        self.cancel_translation_timeout();
        self.controller.lock().unwrap().clear_subtitles();
        self.publish_state();
        Ok(())
    }

    fn clear_current_subtitle_history(&self) -> std::io::Result<()> {
        let pending = self.history_pending_id.lock().unwrap();
        if let Some(id) = pending.as_deref() {
            self.history.clear_text(id)?;
        }
        let mut stats = self.history_stats.lock().unwrap();
        stats.transcript_count = 0;
        stats.transcript_bytes = 0;
        stats.transcript_limited = false;
        drop(stats);
        self.archive_revision.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    pub fn set_overlay_collapsed(&self, collapsed: bool) {
        self.is_overlay_collapsed.store(collapsed, Ordering::SeqCst);
    }

    // MARK: event handling

    async fn handle_event(
        self: &Arc<Self>,
        source: AudioSource,
        generation: u64,
        envelope: ProviderEvent,
    ) {
        let content = self.subtitle_content_lock.lock().await;
        #[cfg(any(test, feature = "development-debugger"))]
        let debug_event = |event: &LiveTranslateServerEvent, admission| {
            if crate::core::development_debug::is_enabled() {
                let mut observation = crate::core::development_debug::ProviderObservation::new(
                    source,
                    generation,
                    envelope.content_revision,
                    event,
                    admission,
                );
                observation.transport_sequence = envelope.transport_sequence;
                crate::core::development_debug::record(
                    crate::core::development_debug::DebugEvent::Provider { observation },
                );
            }
        };
        #[cfg(not(any(test, feature = "development-debugger")))]
        let debug_event =
            |_event: &LiveTranslateServerEvent,
             _admission: crate::core::development_debug::Admission| {};
        if !subtitle_content_is_current(
            generation,
            &envelope,
            *self.lane(source).subtitle_content_revision.lock().unwrap(),
        ) {
            debug_event(
                &envelope.event,
                crate::core::development_debug::Admission::StaleContent,
            );
            return;
        }
        let mut event = envelope.event;
        if !self.accepts_event(generation, &event) || self.is_paused() {
            debug_event(
                &event,
                if self.is_paused() {
                    crate::core::development_debug::Admission::Paused
                } else {
                    crate::core::development_debug::Admission::StaleGeneration
                },
            );
            return;
        }

        // Setup acknowledgements are consumed by `connect`; only
        // `connect_and_listen` may transition to Listening after audio and the
        // send pipeline for the same generation are installed.
        if matches!(
            event,
            LiveTranslateServerEvent::SessionCreated | LiveTranslateServerEvent::SessionUpdated
        ) {
            debug_event(
                &event,
                crate::core::development_debug::Admission::SetupAcknowledgement,
            );
            return;
        }

        if let LiveTranslateServerEvent::SubtitleConfirmedPair {
            utterance_id,
            source: source_text,
            translation,
            ..
        } = &event
        {
            // A replay is not an accepted final: it must not cancel a newer
            // request's timeout or inflate the content-free journal counts.
            if !self.controller.lock().unwrap().accepts_confirmed_pair_from(
                source,
                *utterance_id,
                source_text,
                translation,
            ) {
                debug_event(
                    &event,
                    crate::core::development_debug::Admission::DuplicateFinal,
                );
                return;
            }
        }

        if let LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
            utterance_id,
            source: source_text,
            translation,
            ..
        } = &event
        {
            if !self
                .controller
                .lock()
                .unwrap()
                .accepts_identified_final_pair_from(source, utterance_id, source_text, translation)
            {
                debug_event(
                    &event,
                    crate::core::development_debug::Admission::DuplicateFinal,
                );
                return;
            }
        }

        if let LiveTranslateServerEvent::Error { code, message } = &mut event {
            if matches!(
                code.as_str(),
                "invalid_api_key"
                    | "authentication_error"
                    | "unauthorized"
                    | "translation_authentication_failed"
            ) {
                *message = "credential_authentication_failed".into();
            }
        }

        if !self.record_accepted_provider_event(generation, &event) {
            debug_event(
                &event,
                crate::core::development_debug::Admission::StaleGeneration,
            );
            return;
        }
        debug_event(&event, crate::core::development_debug::Admission::Accepted);

        if let LiveTranslateServerEvent::Error { code, message } = &event {
            if provider_error_is_retryable(code) {
                let _teardown = self.begin_teardown_operation();
                let _operation = self.begin_lifecycle_operation();
                let recovery_owns_attempt = self.is_recovering.load(Ordering::SeqCst);
                if recovery_owns_attempt {
                    // Register the handoff before invalidation wakes the
                    // generation-bound connect future. The current recovery
                    // task consumes this marker and owns the next retry.
                    self.recovery_retry_generation
                        .store(generation, Ordering::SeqCst);
                }
                if !self.invalidate_generation(generation) {
                    if recovery_owns_attempt {
                        let _ = self.recovery_retry_generation.compare_exchange(
                            generation,
                            NO_GENERATION,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        );
                    }
                    return;
                }
                pipeline_log!("session recoverable provider error code={}", code);
                self.cancel_translation_timeout();
                self.controller.lock().unwrap().begin_connecting();
                self.publish_state();
                drop(content);
                let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
                self.stop_health_checks().await;
                self.cleanup_generation_without_pump(generation).await;
                if !recovery_owns_attempt {
                    let minimum_delay = provider_recovery_minimum_delay(code);
                    self.queue_recovery_with_delay(generation, message.clone(), minimum_delay)
                        .await;
                }
                return;
            }
        }

        // A final translation resolves any pending timeout; a fresh
        // TranslationStarted arms a new one.
        let resolves_translation_timeout = match &event {
            LiveTranslateServerEvent::SubtitleIdentifiedFinalPair { utterance_id, .. } => self
                .controller
                .lock()
                .unwrap()
                .identified_source_is_current_from(source, utterance_id),
            _ => matches!(
                event,
                LiveTranslateServerEvent::TranslationFinal(_)
                    | LiveTranslateServerEvent::SubtitleFinalPair { .. }
                    | LiveTranslateServerEvent::SubtitleConfirmedPair { .. }
                    | LiveTranslateServerEvent::UtteranceText {
                        role: UtteranceRole::Translation,
                        is_final: true,
                        ..
                    }
                    | LiveTranslateServerEvent::Error { .. }
                    | LiveTranslateServerEvent::TranslationDeferred(_)
            ),
        };
        if resolves_translation_timeout {
            self.cancel_source_translation_timeout(source);
        }
        if matches!(event, LiveTranslateServerEvent::TranslationStarted) {
            self.arm_translation_timeout(source, generation);
        }

        let is_terminal = matches!(
            event,
            LiveTranslateServerEvent::Error { .. } | LiveTranslateServerEvent::SessionFinished
        );
        let terminal_teardown = is_terminal.then(|| self.begin_teardown_operation());
        let terminal_operation = is_terminal.then(|| self.begin_lifecycle_operation());
        if is_terminal && !self.invalidate_generation(generation) {
            return;
        }
        if is_terminal {
            self.cancel_translation_timeout();
        }
        let newly_confirmed = {
            let mut controller = self.controller.lock().unwrap();
            #[cfg(any(test, feature = "development-debugger"))]
            let debug_before = crate::core::development_debug::is_enabled()
                .then(|| controller.state.subtitles.clone());
            let previous = controller.state.subtitles.history.last().cloned();
            controller.handle_from(source, event.clone());
            apply_terminal_event_to_all_sources(&mut controller, &event);
            #[cfg(any(test, feature = "development-debugger"))]
            if let Some(before) = debug_before {
                crate::core::development_debug::record(
                    crate::core::development_debug::DebugEvent::Reduced {
                        source,
                        generation,
                        changed: before != controller.state.subtitles,
                        before: (&before).into(),
                        after: (&controller.state.subtitles).into(),
                    },
                );
            }
            let current = controller.state.subtitles.history.last();
            confirmed_history_tail_changed(previous.as_ref(), current)
                .then(|| current.cloned())
                .flatten()
        };
        if let Some(pair) = newly_confirmed {
            let pending = self.history_pending_id.lock().unwrap();
            if let Some(id) = pending.as_deref() {
                if self.history_pending_text.load(Ordering::SeqCst) {
                    let mut stats = self.history_stats.lock().unwrap();
                    let bytes = pair.source.len().saturating_add(pair.translation.len());
                    if !stats.transcript_limited {
                        if stats.transcript_count
                            >= crate::core::session_archive::TRANSCRIPT_COUNT_LIMIT
                            || bytes
                                > crate::core::session_archive::TRANSCRIPT_BYTE_LIMIT
                                    .saturating_sub(stats.transcript_bytes)
                        {
                            stats.transcript_limited = true;
                        } else if self.history.append_pair(id, &pair).is_ok() {
                            stats.transcript_count += 1;
                            stats.transcript_bytes += bytes;
                        } else {
                            self.history_save_error.store(true, Ordering::SeqCst);
                            pipeline_log!(
                                "session history subtitle append failed label=write_failed"
                            );
                        }
                    }
                }
            }
        }
        self.publish_state();
        drop(content);

        if is_terminal {
            // Provider error codes are not universally trustworthy (some
            // protocols carry arbitrary server strings). Keep diagnostics on
            // a fixed local label just like free-text messages.
            let label = match &event {
                LiveTranslateServerEvent::Error { .. } => "provider_terminal_error",
                LiveTranslateServerEvent::SessionFinished => "session_finished",
                _ => "terminal_event",
            };
            pipeline_log!("session terminal event label={label}");
            let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
            self.stop_health_checks().await;
            self.cancel_recovery().await;
            self.cleanup_generation_without_pump(generation).await;
            self.clear_active_settings_for_generation(generation);
            drop(terminal_operation);
            drop(terminal_teardown);
        }
    }

    /// Arms a timer that clears a stuck "正在翻译" state. The low-latency
    /// stream protocol synthesizes TranslationStarted at the source final,
    /// then waits for the server's `response.text.done`; if the audio stops
    /// mid-sentence the server may never finalize, so without this the UI
    /// would stay pending forever. The shown subtitle is left untouched.
    fn arm_translation_timeout(self: &Arc<Self>, source: AudioSource, generation: u64) {
        // Install/cancel under one slot lock. Otherwise a concurrent cancel
        // can run after `spawn` but before the handle is stored, leaving a
        // detached stale handle in the slot even though its ownership id was
        // already invalidated.
        let mut slot = self.lane(source).translation_timeout_task.lock().unwrap();
        if let Some(task) = slot.take() {
            task.abort();
        }
        let task_id = self.next_background_task_id();
        self.lane(source)
            .translation_timeout_task_id
            .store(task_id, Ordering::SeqCst);
        let this = Arc::clone(self);
        let task = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(6)).await;
            let _content = this.subtitle_content_lock.lock().await;
            if !this.is_generation_current(generation)
                || !this.clear_translation_timeout_task_if_id(source, task_id)
            {
                return;
            }
            pipeline_log!("translation pending timed out; clearing");
            this.controller
                .lock()
                .unwrap()
                .clear_translation_pending_from(source);
            this.publish_state();
        });
        *slot = Some(task);
    }

    fn cancel_translation_timeout(self: &Arc<Self>) {
        for source in [AudioSource::System, AudioSource::Microphone] {
            self.cancel_source_translation_timeout(source);
        }
    }

    fn cancel_source_translation_timeout(self: &Arc<Self>, source: AudioSource) {
        let mut slot = self.lane(source).translation_timeout_task.lock().unwrap();
        self.lane(source)
            .translation_timeout_task_id
            .store(NO_GENERATION, Ordering::SeqCst);
        if let Some(task) = slot.take() {
            task.abort();
        }
    }

    async fn handle_capture_failure(
        self: &Arc<Self>,
        generation: u64,
        attempt: u64,
        failure: SystemAudioCaptureFailure,
    ) {
        if !failure.is_recoverable() {
            let _teardown = self.begin_teardown_operation();
            let _operation = self.begin_lifecycle_operation();
            let epoch = {
                let _transition = self.generation_transition.lock().unwrap();
                let Some(epoch) = invalidate_audio_attempt_atoms(
                    &self.active_generation,
                    &self.lifecycle_sequence,
                    &mut self.audio_attempt.lock().unwrap(),
                    generation,
                    attempt,
                ) else {
                    return;
                };
                self.lifecycle_notify.notify_waiters();
                epoch
            };
            self.cancel_recovery().await;
            let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
            self.stop_health_checks().await;
            self.cleanup_generation(generation).await;
            if self.lifecycle_sequence.load(Ordering::SeqCst) != epoch {
                return;
            }
            self.clear_active_settings_for_generation(generation);
            self.record_diagnostic_failure(failure.diagnostic_label());
            self.controller
                .lock()
                .unwrap()
                .did_fail(failure.to_string());
            self.publish_state();
            return;
        }
        self.handle_recoverable_runtime_failure(
            generation,
            attempt,
            failure.to_string(),
            failure.diagnostic_label(),
        )
        .await;
    }

    async fn handle_audio_transport_failure(
        self: &Arc<Self>,
        generation: u64,
        attempt: u64,
        failure: AudioPipelineFailure,
    ) {
        self.handle_recoverable_runtime_failure(
            generation,
            attempt,
            failure.to_string(),
            failure.diagnostic_label(),
        )
        .await;
    }

    async fn handle_recoverable_runtime_failure(
        self: &Arc<Self>,
        generation: u64,
        attempt: u64,
        message: String,
        diagnostic_label: &'static str,
    ) {
        let _teardown = self.begin_teardown_operation();
        let _operation = self.begin_lifecycle_operation();
        if self.is_paused() || self.active_settings.lock().unwrap().is_none() {
            return;
        }
        let recovery_owns_attempt = {
            let _transition = self.generation_transition.lock().unwrap();
            if invalidate_audio_attempt_atoms(
                &self.active_generation,
                &self.lifecycle_sequence,
                &mut self.audio_attempt.lock().unwrap(),
                generation,
                attempt,
            )
            .is_none()
            {
                return;
            }
            self.lifecycle_notify.notify_waiters();
            let recovering = self.is_recovering.load(Ordering::SeqCst);
            if recovering {
                self.recovery_retry_generation
                    .store(generation, Ordering::SeqCst);
            }
            recovering
        };
        pipeline_log!("runtime stream failed label={}", diagnostic_label);
        self.record_diagnostic_failure(diagnostic_label);
        self.controller.lock().unwrap().begin_connecting();
        self.publish_state();
        let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        self.stop_health_checks().await;
        self.cleanup_generation(generation).await;
        if !recovery_owns_attempt {
            self.queue_recovery(generation, message).await;
        }
    }

    // MARK: health checks and recovery

    async fn start_health_checks(self: &Arc<Self>, generation: u64) {
        self.stop_health_checks().await;
        let task_id = self.next_background_task_id();
        let self_arc = self.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if !self_arc.check_connection_health(generation, task_id).await {
                    return;
                }
            }
        });
        let mut slot = self.health_task.lock().unwrap();
        self.health_task_id.store(task_id, Ordering::SeqCst);
        *slot = Some(task);
    }

    async fn stop_health_checks(&self) {
        let mut slot = self.health_task.lock().unwrap();
        self.health_task_id.store(NO_GENERATION, Ordering::SeqCst);
        if let Some(task) = slot.take() {
            task.abort();
        }
    }

    async fn check_connection_health(self: &Arc<Self>, generation: u64, task_id: u64) -> bool {
        if self.is_paused()
            || self.is_recovering.load(Ordering::SeqCst)
            || !self.is_generation_current(generation)
            || self.health_task_id.load(Ordering::SeqCst) != task_id
        {
            return false;
        }
        let records_network_latency =
            self.active_settings
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|configuration| {
                    !matches!(
                        configuration.provider,
                        ProviderKind::AppleSpeech | ProviderKind::LocalSpeech
                    )
                });
        let mut maximum_latency_ms = 0;
        for &source in self.sources() {
            let Some(client) = self.client_for_generation(source, generation) else {
                return false;
            };
            let started_at = Instant::now();
            match client.ping(Duration::from_secs(4)).await {
                Ok(()) => {
                    let elapsed_ms = milliseconds(started_at, Instant::now());
                    {
                        // Pair the validity check and write with health-task
                        // replacement, so a late old probe cannot overwrite a
                        // newer task's measurement after stop/reconnect.
                        let _health_task = self.health_task.lock().unwrap();
                        if !self.is_generation_current(generation)
                            || self.health_task_id.load(Ordering::SeqCst) != task_id
                        {
                            return false;
                        }
                        *self.health_latency.lock().unwrap() =
                            records_network_latency.then_some(HealthCheckLatency {
                                generation,
                                task_id,
                                milliseconds: maximum_latency_ms.max(elapsed_ms),
                            });
                    }
                    maximum_latency_ms = maximum_latency_ms.max(elapsed_ms);
                }
                Err(error) => {
                    if !self.is_generation_current(generation)
                        || self.health_task_id.load(Ordering::SeqCst) != task_id
                    {
                        return false;
                    }
                    pipeline_log!(
                        "connection health failed label={}",
                        error.diagnostic_label()
                    );
                    self.clear_health_task_if_id(task_id);
                    if self.invalidate_generation(generation) {
                        self.queue_recovery(generation, error.to_string()).await;
                    }
                    return false;
                }
            }
        }
        self.publish_state();
        true
    }

    async fn queue_recovery(self: &Arc<Self>, failed_generation: u64, failure_message: String) {
        self.queue_recovery_with_delay(failed_generation, failure_message, Duration::ZERO)
            .await;
    }

    async fn queue_recovery_with_delay(
        self: &Arc<Self>,
        failed_generation: u64,
        failure_message: String,
        minimum_delay: Duration,
    ) {
        if self.is_paused() || self.active_settings.lock().unwrap().is_none() {
            return;
        }
        let mut slot = self.recovery_task.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let task_id = self.next_background_task_id();
        self.recovery_task_id.store(task_id, Ordering::SeqCst);
        let self_arc = self.clone();
        let task = tokio::spawn(async move {
            self_arc
                .recover_connection(failed_generation, failure_message, minimum_delay)
                .await;
            self_arc.clear_recovery_task_if_id(task_id);
        });
        *slot = Some(task);
    }

    async fn recover_connection(
        self: &Arc<Self>,
        failed_generation: u64,
        failure_message: String,
        minimum_delay: Duration,
    ) {
        if self.is_paused() || self.is_recovering.load(Ordering::SeqCst) {
            return;
        }
        let _operation = self.begin_lifecycle_operation();
        let mut recovery_generation = self.next_lifecycle_request();
        let lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        if !self.is_lifecycle_request_current(recovery_generation) {
            return;
        }
        self.record_recovery_if_current(RecoveryAction::Retrying, recovery_generation);
        pipeline_log!("session recovery started");
        self.is_recovering.store(true, Ordering::SeqCst);
        self.recovery_retry_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        self.stop_health_checks().await;
        self.controller.lock().unwrap().begin_connecting();
        self.publish_state();
        drop(lifecycle);
        self.cleanup_generation(failed_generation).await;

        let mut recovered = false;
        let mut terminal_failure = None;
        let mut recovery_epoch = recovery_generation;
        for attempt in 0..RECOVERY_ATTEMPTS {
            let delay = recovery_delay(attempt, failed_generation).max(minimum_delay);
            pipeline_log!(
                "session recovery attempt={} delayMs={}",
                attempt + 1,
                delay.as_millis()
            );
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if self.active_settings.lock().unwrap().is_none() {
                clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
                return;
            }
            if attempt > 0 {
                let Some(next_generation) =
                    self.advance_lifecycle_request_if_current(recovery_epoch)
                else {
                    clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
                    return;
                };
                recovery_generation = next_generation;
            }
            let lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
            if !self.is_lifecycle_request_current(recovery_generation) {
                clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
                return;
            }
            self.active_generation
                .store(recovery_generation, Ordering::SeqCst);
            self.retag_active_settings(recovery_generation);
            drop(lifecycle);
            match self.establish_session(recovery_generation).await {
                Ok(()) => {
                    self.record_recovery_if_current(RecoveryAction::Recovered, recovery_generation);
                    recovered = true;
                    break;
                }
                Err(error) if error == SESSION_START_CANCELLED => {
                    let retry_generation = self
                        .recovery_retry_generation
                        .swap(NO_GENERATION, Ordering::SeqCst);
                    let current_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
                    if self.is_recovering.load(Ordering::SeqCst)
                        && cancelled_recovery_attempt_is_retryable(
                            retry_generation,
                            recovery_generation,
                            current_epoch,
                            self.active_settings.lock().unwrap().is_some(),
                        )
                    {
                        recovery_epoch = current_epoch;
                        continue;
                    }
                    clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
                    return;
                }
                Err(error) => {
                    let failure_epoch = recovery_generation.wrapping_add(1);
                    if self.lifecycle_sequence.load(Ordering::SeqCst) != failure_epoch {
                        clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
                        return;
                    }
                    recovery_epoch = failure_epoch;
                    if session_error_requires_user_action(&error) {
                        terminal_failure = Some(error);
                        break;
                    }
                }
            }
        }

        if !recovered {
            let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
            if !recovery_exhaustion_is_still_owned(
                recovery_epoch,
                self.lifecycle_sequence.load(Ordering::SeqCst),
                self.active_generation.load(Ordering::SeqCst),
            ) {
                clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
                return;
            }
            if terminal_failure.is_none() {
                self.record_recovery_if_current(RecoveryAction::RetriesExhausted, recovery_epoch);
                pipeline_log!("session recovery exhausted");
            }
            // Preserve the permission/stop result instead of covering it with
            // the transient error that originally initiated this recovery.
            let failure_message = terminal_failure.unwrap_or(failure_message);
            self.record_diagnostic_failure(&failure_message);
            self.clear_active_settings_for_generation(recovery_generation);
            clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
            self.controller.lock().unwrap().did_fail(failure_message);
            self.publish_state();
            return;
        }
        clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
    }

    async fn cancel_recovery(&self) {
        let mut slot = self.recovery_task.lock().unwrap();
        self.recovery_task_id.store(NO_GENERATION, Ordering::SeqCst);
        clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
        if let Some(task) = slot.take() {
            task.abort();
        }
    }

    async fn finish_source_pipeline(&self, source: AudioSource, timeout: Duration) {
        let pipeline = self.lane(source).audio_pipeline.lock().unwrap().take();
        if let Some(pipeline) = pipeline {
            self.lane(source)
                .audio_pipeline_generation
                .store(NO_GENERATION, Ordering::SeqCst);
            if !pipeline.finish(timeout).await {
                pipeline_log!("audio pipeline drain timed out");
            }
        }
    }

    /// Capacity-one channel through which a native callback reports its first
    /// fatal failure without blocking the real-time audio thread.
    fn capture_failure_channel(
        self: &Arc<Self>,
        generation: u64,
        attempt: u64,
    ) -> CaptureFailureSender {
        let (tx, mut rx) = CaptureFailureSender::channel();
        let self_arc = Arc::clone(self);
        tokio::spawn(async move {
            if let Some(failure) = rx.recv().await {
                self_arc
                    .clone()
                    .handle_capture_failure(generation, attempt, failure)
                    .await;
            }
        });
        tx
    }

    fn begin_lifecycle_operation(&self) -> LifecycleOperationGuard {
        self.lifecycle_operations.fetch_add(1, Ordering::SeqCst);
        LifecycleOperationGuard {
            count: Arc::clone(&self.lifecycle_operations),
        }
    }

    fn begin_teardown_operation(&self) -> TeardownOperationGuard {
        self.teardown_operations.fetch_add(1, Ordering::SeqCst);
        TeardownOperationGuard {
            count: Arc::clone(&self.teardown_operations),
            notify: Arc::clone(&self.teardown_notify),
        }
    }

    async fn lock_after_teardown(&self) -> OwnedMutexGuard<()> {
        lock_after_operations(
            Arc::clone(&self.lifecycle_lock),
            Arc::clone(&self.teardown_operations),
            Arc::clone(&self.teardown_notify),
        )
        .await
    }

    fn next_lifecycle_request(&self) -> u64 {
        let _transition = self.generation_transition.lock().unwrap();
        let generation = self
            .lifecycle_sequence
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        let generation = if generation == NO_GENERATION {
            self.lifecycle_sequence.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            generation
        };
        self.lifecycle_notify.notify_waiters();
        generation
    }

    fn advance_lifecycle_request_if_current(&self, expected: u64) -> Option<u64> {
        let _transition = self.generation_transition.lock().unwrap();
        let generation = advance_lifecycle_sequence_if_current(&self.lifecycle_sequence, expected)?;
        self.lifecycle_notify.notify_waiters();
        Some(generation)
    }

    fn next_background_task_id(&self) -> u64 {
        let task_id = self
            .background_task_sequence
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        if task_id == NO_GENERATION {
            self.background_task_sequence.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            task_id
        }
    }

    fn clear_health_task_if_id(&self, task_id: u64) -> bool {
        clear_task_slot_if_id(&self.health_task, &self.health_task_id, task_id)
    }

    fn clear_recovery_task_if_id(&self, task_id: u64) -> bool {
        clear_task_slot_if_id(&self.recovery_task, &self.recovery_task_id, task_id)
    }

    fn clear_translation_timeout_task_if_id(&self, source: AudioSource, task_id: u64) -> bool {
        clear_task_slot_if_id(
            &self.lane(source).translation_timeout_task,
            &self.lane(source).translation_timeout_task_id,
            task_id,
        )
    }

    fn is_lifecycle_request_current(&self, generation: u64) -> bool {
        lifecycle_sequence_matches(&self.lifecycle_sequence, generation)
    }

    fn is_generation_current(&self, generation: u64) -> bool {
        generation != NO_GENERATION && self.active_generation.load(Ordering::SeqCst) == generation
    }

    fn ensure_generation_current(&self, generation: u64) -> Result<(), String> {
        if self.is_generation_current(generation) {
            Ok(())
        } else {
            Err(SESSION_START_CANCELLED.into())
        }
    }

    async fn run_while_generation_current<T>(
        &self,
        generation: u64,
        operation: impl Future<Output = T>,
    ) -> Result<T, String> {
        run_generation_bound_operation(
            Arc::clone(&self.active_generation),
            Arc::clone(&self.lifecycle_sequence),
            Arc::clone(&self.lifecycle_notify),
            generation,
            operation,
        )
        .await
    }

    /// Invalidates only the specified generation. The sequence bump also
    /// cancels a startup that has not yet reached its next generation check.
    fn invalidate_generation(&self, generation: u64) -> bool {
        self.invalidate_generation_with_epoch(generation).is_some()
    }

    fn invalidate_generation_with_epoch(&self, generation: u64) -> Option<u64> {
        let _transition = self.generation_transition.lock().unwrap();
        let owned_epoch = invalidate_generation_atoms(
            &self.active_generation,
            &self.lifecycle_sequence,
            generation,
        );
        if owned_epoch.is_some() {
            self.lifecycle_notify.notify_waiters();
        }
        owned_epoch
    }

    fn commit_listening(&self, generation: u64) -> Result<(), String> {
        let _transition = self.generation_transition.lock().unwrap();
        self.ensure_generation_current(generation)?;
        self.controller.lock().unwrap().did_connect();
        Ok(())
    }

    fn accepts_event(&self, generation: u64, event: &LiveTranslateServerEvent) -> bool {
        generation_accepts_event(
            self.active_generation.load(Ordering::SeqCst),
            self.stopping_tail_generation.load(Ordering::SeqCst),
            generation,
            event,
        )
    }

    async fn install_client(
        &self,
        source: AudioSource,
        generation: u64,
        client: TranslationClient,
    ) -> Result<(), String> {
        // Recovery may install outside the command lifecycle guard. Keep its
        // owner/revision atomic with a Clear that is still awaiting the old client.
        let _content = self.subtitle_content_lock.lock().await;
        let _transition = self.generation_transition.lock().unwrap();
        self.ensure_generation_current(generation)?;
        let mut slot = self.lane(source).client.lock().unwrap();
        if slot.is_some() {
            return Err("A live translation client is already installed.".into());
        }
        *self.lane(source).subtitle_content_revision.lock().unwrap() =
            (generation, client.content_revision());
        *slot = Some(client);
        self.lane(source)
            .client_generation
            .store(generation, Ordering::SeqCst);
        // Settings can rebuild a client within the same lifecycle generation.
        // Its new socket must not inherit the previous socket's probe time.
        *self.health_latency.lock().unwrap() = None;
        Ok(())
    }

    fn set_active_settings(
        &self,
        generation: u64,
        configuration: Option<LiveTranslationConfiguration>,
    ) {
        let mut slot = self.active_settings.lock().unwrap();
        *slot = configuration;
        self.active_settings_generation.store(
            if slot.is_some() {
                generation
            } else {
                NO_GENERATION
            },
            Ordering::SeqCst,
        );
    }

    fn retag_active_settings(&self, generation: u64) {
        let slot = self.active_settings.lock().unwrap();
        if slot.is_some() {
            self.active_settings_generation
                .store(generation, Ordering::SeqCst);
        }
    }

    fn clear_active_settings_for_generation(&self, generation: u64) -> bool {
        clear_owned_value_if_generation(
            &self.active_settings,
            &self.active_settings_generation,
            generation,
        )
    }

    fn clear_active_settings(&self) {
        *self.active_settings.lock().unwrap() = None;
        self.active_settings_generation
            .store(NO_GENERATION, Ordering::SeqCst);
    }

    fn client_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
    ) -> Option<TranslationClient> {
        let slot = self.lane(source).client.lock().unwrap();
        if self.lane(source).client_generation.load(Ordering::SeqCst) == generation {
            slot.clone()
        } else {
            None
        }
    }

    fn take_client_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
    ) -> Option<TranslationClient> {
        let mut slot = self.lane(source).client.lock().unwrap();
        if self.lane(source).client_generation.load(Ordering::SeqCst) != generation {
            return None;
        }
        self.lane(source)
            .client_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        slot.take()
    }

    fn take_any_client(&self, source: AudioSource) -> Option<TranslationClient> {
        let mut slot = self.lane(source).client.lock().unwrap();
        self.lane(source)
            .client_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        slot.take()
    }

    fn install_pipeline(
        &self,
        source: AudioSource,
        generation: u64,
        pipeline: Arc<AudioSendPipeline>,
    ) -> Result<(), String> {
        let _transition = self.generation_transition.lock().unwrap();
        self.ensure_generation_current(generation)?;
        if !self.is_lifecycle_request_current(generation) {
            return Err(SESSION_START_CANCELLED.into());
        }
        let mut slot = self.lane(source).audio_pipeline.lock().unwrap();
        if slot.is_some() {
            return Err("An audio send pipeline is already installed.".into());
        }
        *slot = Some(pipeline);
        self.lane(source)
            .audio_pipeline_generation
            .store(generation, Ordering::SeqCst);
        Ok(())
    }

    fn take_pipeline_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
    ) -> Option<Arc<AudioSendPipeline>> {
        let mut slot = self.lane(source).audio_pipeline.lock().unwrap();
        if self
            .lane(source)
            .audio_pipeline_generation
            .load(Ordering::SeqCst)
            != generation
        {
            return None;
        }
        self.lane(source)
            .audio_pipeline_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        let pipeline = slot.take();
        if let Some(pipeline) = &pipeline {
            let (pcm_data_recent, sound_recent) = pipeline.input_activity();
            *self.diagnostic_capture.lock().unwrap() = Some((
                CaptureObservation {
                    pcm_data_recent,
                    sound_recent,
                },
                Instant::now(),
            ));
        }
        pipeline
    }

    fn install_pump(&self, source: AudioSource, generation: u64, pump: JoinHandle<()>) {
        let _transition = self.generation_transition.lock().unwrap();
        if !self.is_generation_current(generation) || !self.is_lifecycle_request_current(generation)
        {
            pump.abort();
            return;
        }
        let mut slot = self.lane(source).pump_task.lock().unwrap();
        if let Some(old) = slot.replace(pump) {
            old.abort();
        }
        self.lane(source)
            .pump_generation
            .store(generation, Ordering::SeqCst);
    }

    fn stop_pump_for_generation(&self, source: AudioSource, generation: u64) {
        if let Some(task) = self.take_pump_for_generation(source, generation) {
            task.abort();
        }
    }

    fn take_pump_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
    ) -> Option<JoinHandle<()>> {
        let mut slot = self.lane(source).pump_task.lock().unwrap();
        if self.lane(source).pump_generation.load(Ordering::SeqCst) != generation {
            return None;
        }
        self.lane(source)
            .pump_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        slot.take()
    }

    async fn finish_pump_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
        timeout: Duration,
    ) {
        let Some(mut task) = self.take_pump_for_generation(source, generation) else {
            return;
        };
        if tokio::time::timeout(timeout, &mut task).await.is_err() {
            task.abort();
        }
    }

    fn stop_any_pump(&self, source: AudioSource) {
        self.lane(source)
            .pump_generation
            .store(NO_GENERATION, Ordering::SeqCst);
        if let Some(task) = self.lane(source).pump_task.lock().unwrap().take() {
            task.abort();
        }
    }

    async fn stop_capture_for_generation(&self, source: AudioSource, generation: u64) {
        if self
            .lane(source)
            .capture_generation
            .compare_exchange(
                generation,
                NO_GENERATION,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {
            return;
        }
        if let Some(observation) = self.current_capture_observation() {
            *self.diagnostic_capture.lock().unwrap() = Some((observation, Instant::now()));
        }
        let capture = self.lane(source).audio.lock().unwrap().clone();
        if tokio::time::timeout(Duration::from_secs(2), capture.stop())
            .await
            .is_err()
        {
            pipeline_log!("audio capture stop timed out");
        }
    }

    async fn stop_any_capture(&self, source: AudioSource) {
        if self
            .lane(source)
            .capture_generation
            .swap(NO_GENERATION, Ordering::SeqCst)
            == NO_GENERATION
        {
            return;
        }
        let capture = self.lane(source).audio.lock().unwrap().clone();
        if tokio::time::timeout(Duration::from_secs(2), capture.stop())
            .await
            .is_err()
        {
            pipeline_log!("audio capture stop timed out");
        }
    }

    async fn finish_pipeline_for_generation(
        &self,
        source: AudioSource,
        generation: u64,
        timeout: Duration,
    ) {
        if let Some(pipeline) = self.take_pipeline_for_generation(source, generation) {
            if !pipeline.finish(timeout).await {
                pipeline_log!("audio pipeline drain timed out");
            }
        }
    }

    async fn cleanup_generation(&self, generation: u64) {
        self.cleanup_generation_resources(generation, EventPumpCleanup::Abort)
            .await;
    }

    async fn cleanup_generation_without_pump(&self, generation: u64) {
        self.cleanup_generation_resources(generation, EventPumpCleanup::Preserve)
            .await;
    }

    async fn cleanup_generation_resources(&self, generation: u64, pumps: EventPumpCleanup) {
        self.retire_audio_attempt(generation);
        if let Some(echo) = self.take_echo_pipeline(generation) {
            echo.stop();
        }
        for source in [AudioSource::System, AudioSource::Microphone] {
            if let Some(pipeline) = self.take_pipeline_for_generation(source, generation) {
                pipeline.stop();
            }
        }
        tokio::join!(
            self.stop_capture_for_generation(AudioSource::System, generation),
            self.stop_capture_for_generation(AudioSource::Microphone, generation),
        );
        for source in [AudioSource::System, AudioSource::Microphone] {
            if let Some(client) = self.take_client_for_generation(source, generation) {
                self.remember_mt_request_budget(source, generation, &client)
                    .await;
                let _ = tokio::time::timeout(Duration::from_secs(2), client.disconnect()).await;
            }
            if matches!(pumps, EventPumpCleanup::Abort) {
                self.stop_pump_for_generation(source, generation);
            }
        }
    }

    fn retire_audio_attempt(&self, generation: u64) {
        let _transition = self.generation_transition.lock().unwrap();
        let mut owner = self.audio_attempt.lock().unwrap();
        if generation == NO_GENERATION || owner.is_some_and(|(active, _)| active == generation) {
            *owner = None;
        }
    }

    fn take_echo_pipeline(&self, generation: u64) -> Option<Arc<EchoPipeline>> {
        let mut slot = self.echo_pipeline.lock().unwrap();
        if generation == NO_GENERATION
            || slot.as_ref().is_some_and(|(owner, _)| *owner == generation)
        {
            slot.take().map(|(_, echo)| echo)
        } else {
            None
        }
    }

    async fn remember_mt_request_budget(
        &self,
        source: AudioSource,
        generation: u64,
        client: &TranslationClient,
    ) {
        let lease = self
            .lane(source)
            .mt_budget_continuity
            .lock()
            .unwrap()
            .take_lease(generation);
        let Some((token, scope)) = lease else {
            return;
        };
        if let Some(budget) = client.suspend_mt_request_budget().await {
            self.lane(source)
                .mt_budget_continuity
                .lock()
                .unwrap()
                .remember(token, scope, budget);
        }
    }

    // MARK: state publishing

    /// Builds the current session state snapshot without emitting it (used by
    /// windows that boot after the last broadcast, e.g. the overlay control).
    pub fn current_state_event(&self) -> SessionStateEvent {
        let state = self.controller.lock().unwrap().state.clone();
        let mut event = SessionStateEvent::from(&state);
        event.is_active |= self.is_recovering.load(Ordering::SeqCst);
        event.is_paused = self.is_paused();
        event.is_overlay_collapsed = self.is_overlay_collapsed();
        let generation = self.active_generation.load(Ordering::SeqCst);
        let translation_latency = self
            .sources()
            .iter()
            .filter_map(|&source| {
                self.client_for_generation(source, generation)
                    .and_then(|client| client.translation_latency())
            })
            .max_by_key(|latency| latency.milliseconds);
        let (api_latency_ms, translation_latency_ms, translation_latency_kind) =
            visible_session_latencies(
                &state.status,
                event.is_paused,
                self.is_recovering.load(Ordering::SeqCst),
                generation,
                self.health_task_id.load(Ordering::SeqCst),
                *self.health_latency.lock().unwrap(),
                translation_latency,
            );
        if self.is_generation_current(generation) {
            event.api_latency_ms = api_latency_ms;
            event.translation_latency_ms = translation_latency_ms;
            event.translation_latency_kind = translation_latency_kind;
        }
        event.debug_snapshot_id =
            crate::core::development_debug::record_snapshot(generation, &event.subtitles);
        event
    }

    /// Broadcasts the current session state to the frontend, coalescing
    /// high-frequency calls: subtitle chunks stream in at tens of events per
    /// second and every snapshot carries the full subtitle history, so
    /// emitting per chunk is the main UI-lag cost during live listening. The
    /// first call schedules a single trailing broadcast ~60ms later that
    /// always carries the latest snapshot; intermediate calls only set the
    /// dirty flag. Status-only changes therefore reach the UI within 60ms
    /// and bursty chunks are folded into one emit.
    pub fn publish_state(self: &Arc<Self>) {
        let status = {
            let controller = self.controller.lock().unwrap();
            match controller.state.status {
                SessionStatus::Idle => DiagnosticStatus::Idle,
                SessionStatus::Connecting => DiagnosticStatus::Connecting,
                SessionStatus::Stopping => DiagnosticStatus::Stopping,
                SessionStatus::Listening if self.is_paused() => DiagnosticStatus::Paused,
                SessionStatus::Listening => DiagnosticStatus::Listening,
                SessionStatus::Error(_) => DiagnosticStatus::Error,
            }
        };
        self.diagnostic_journal
            .lock()
            .unwrap()
            .observe_status(status, milliseconds(self.diagnostic_epoch, Instant::now()));
        self.publish_dirty.store(true, Ordering::SeqCst);
        let Ok(guard) = Arc::clone(&self.publish_lock).try_lock_owned() else {
            return;
        };
        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let guard = guard;
            // The first caller's dirty bit is represented by this task itself;
            // clear it before waiting so the first publish below is not
            // followed by a duplicate one.
            this.publish_dirty.store(false, Ordering::SeqCst);
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                this.publish_state_now().await;
                if !this.publish_dirty.swap(false, Ordering::SeqCst) {
                    // Release the scheduling lock first, then check for a
                    // caller that set the dirty bit while the lock was still
                    // held (that caller could not schedule its own task).
                    drop(guard);
                    if this.publish_dirty.load(Ordering::SeqCst) {
                        this.publish_state_now().await;
                    }
                    return;
                }
            }
        });
    }

    async fn publish_state_now(self: &Arc<Self>) {
        // Snapshot and delivery share the clear/apply gate, so a snapshot
        // captured before Clear cannot be emitted after the cleared one.
        let _content = self.subtitle_content_lock.lock().await;
        let event = self.current_state_event();
        #[cfg(any(test, feature = "development-debugger"))]
        crate::development_debugger::record_snapshot(&event, &self.settings);
        self.write_ui_test_session_state(&event);
        let should_show_overlay =
            event.is_active || matches!(&event.status, StatusPayload::Error { .. });
        let is_collapsed = event.is_overlay_collapsed;
        let preferences = self.settings.preferences();
        let click_through =
            preferences.overlay_locked || preferences.subtitle_blends_with_background;
        #[cfg(any(test, feature = "development-debugger"))]
        let debug_snapshot_id = event.debug_snapshot_id;
        let _delivered = self.app.emit("session-state", &event).is_ok();
        #[cfg(any(test, feature = "development-debugger"))]
        if let Some(snapshot_id) = debug_snapshot_id {
            crate::core::development_debug::record(
                crate::core::development_debug::DebugEvent::Published {
                    snapshot_id,
                    delivered: _delivered,
                    overlay_requested: should_show_overlay,
                    collapsed: is_collapsed,
                },
            );
        }
        OverlayWindowManager::sync_presentation(
            &self.app,
            should_show_overlay,
            is_collapsed,
            click_through,
            preferences.subtitle_blends_with_background,
        );
    }

    fn write_ui_test_session_state(&self, event: &SessionStateEvent) {
        if !self.is_ui_test() {
            return;
        }
        let Some(path) = std::env::var_os("MIMI_UI_TEST_SESSION_STATE_FILE") else {
            return;
        };
        let state = match &event.status {
            StatusPayload::Idle => "idle",
            StatusPayload::Connecting => "connecting",
            StatusPayload::Listening => "listening",
            StatusPayload::Stopping => "stopping",
            StatusPayload::Error { .. } => "error",
        };
        if let Err(error) = std::fs::write(path, state) {
            tracing::warn!(error = %error, "could not write UI-test session state marker");
        }
    }

    /// Broadcasts the current settings snapshot to every window (used after
    /// preference writes so no window keeps a stale selection, and when a
    /// window re-shows in case its webview missed events while hidden).
    pub fn publish_settings(&self) {
        crate::commands::sync_overlay_minimum(&self.app);
        let _ = self.app.emit(
            "settings-changed",
            crate::commands::SettingsSnapshotPayload::from_store(&self.settings),
        );
    }

    fn is_ui_test(&self) -> bool {
        self.settings.is_ui_test()
    }

    fn establish_ui_test_session(self: &Arc<Self>) {
        self.cancel_translation_timeout();
        self.clear_active_settings();
        self.controller.lock().unwrap().begin_connecting();
        self.publish_state();
        self.controller.lock().unwrap().did_connect();
        // Exercise the real archive and native save dialog in credential-free
        // UI QA. These are explicitly synthetic samples, never captured audio.
        let preferences = self.settings.preferences();
        let dual_fixture = std::env::var("MIMI_UI_TEST_DUAL_SUBTITLES").as_deref() == Ok("1");
        let dual_live_fixture =
            std::env::var("MIMI_UI_TEST_DUAL_LIVE_SUBTITLES").as_deref() == Ok("1");
        self.controller
            .lock()
            .unwrap()
            .set_audio_input(preferences.audio_input);
        if preferences.retain_session_history || dual_fixture {
            for &source in preferences.audio_input.sources() {
                let (original, translation) = match source {
                    AudioSource::System => (
                        "Mimi system audio test: can you see the shared screen?",
                        "Mimi 系统声音测试：你能看到共享屏幕吗？",
                    ),
                    AudioSource::Microphone => (
                        "Mimi microphone test: yes, I can see it.",
                        "Mimi 麦克风测试：可以，我能看到。",
                    ),
                };
                self.controller.lock().unwrap().handle_from(
                    source,
                    LiveTranslateServerEvent::SubtitleFinalPair {
                        source: original.into(),
                        language: Some("en".into()),
                        translation: translation.into(),
                    },
                );
            }
        }
        if dual_live_fixture {
            seed_ui_test_live_subtitles(
                &mut self.controller.lock().unwrap(),
                preferences.audio_input,
            );
        }
        if preferences.record_session_audio && !dual_live_fixture {
            for &source in preferences.audio_input.sources() {
                let (slot, frequency) = match source {
                    AudioSource::System => (0, 440.0),
                    AudioSource::Microphone => (1, 613.0),
                };
                let samples: Vec<u8> = (0..16_000)
                    .flat_map(|index| {
                        let phase = index as f32 * std::f32::consts::TAU * frequency / 16_000.0;
                        ((phase.sin() * 1_000.0) as i16).to_le_bytes()
                    })
                    .collect();
                let mut recordings = self.recording.lock().unwrap();
                recordings[slot].begin(true);
                recordings[slot].append(16_000, &samples);
            }
        }
        self.publish_state();
        pipeline_log!("ui-test synthetic session listening");
    }
}

/// Only the credential-free UI-test start path calls this synthetic fixture.
/// Preserve the selected sources and publish both raw and complete paired
/// previews so native geometry QA works for independent and atomic routes.
fn seed_ui_test_live_subtitles(
    controller: &mut TranslationSessionController,
    audio_input: AudioInput,
) {
    for &audio_source in audio_input.sources() {
        let (source_id, source, translation) = match audio_source {
            AudioSource::System => (10_001, "System live test", "系统字幕测试"),
            AudioSource::Microphone => (10_002, "Microphone live test", "麦克风字幕测试"),
        };
        controller.handle_from(
            audio_source,
            LiveTranslateServerEvent::SourceUtteranceDraft {
                utterance_id: source_id,
                text: source.into(),
                language: Some("en".into()),
            },
        );
        // Use the actual reducer stamp, including its current display epoch.
        let Some(owner) = controller
            .state
            .subtitles
            .tracks
            .iter()
            .find(|track| track.audio_source == audio_source)
            .and_then(|track| track.source.utterance_id.clone())
        else {
            continue;
        };
        controller.handle_from(
            audio_source,
            LiveTranslateServerEvent::UtteranceText {
                utterance_id: owner,
                role: UtteranceRole::Translation,
                text: translation.into(),
                is_final: false,
                language: None,
            },
        );
        controller.handle_from(
            audio_source,
            LiveTranslateServerEvent::SubtitlePreviewPair {
                source_utterance_id: Some(source_id),
                source: source.into(),
                language: Some("en".into()),
                translation: translation.into(),
            },
        );
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn application_target_validation_is_bounded_and_ui_fixtures_never_probe_os_support() {
        let target = SystemAudioTarget::Application {
            id: "test.player".into(),
            name: "Test Player".into(),
        };
        assert_eq!(
            validate_system_audio_target(&target, false, || true),
            Ok(())
        );
        assert_eq!(
            validate_system_audio_target(&target, false, || false),
            Err("application_audio_unsupported".into())
        );
        assert_eq!(
            validate_system_audio_target(&target, true, || panic!("UI fixture probed OS")),
            Ok(())
        );
        assert_eq!(
            validate_system_audio_target(&SystemAudioTarget::System, false, || panic!(
                "System target probed app support"
            )),
            Ok(())
        );
        for ui_test in [false, true] {
            assert_eq!(
                validate_system_audio_target(
                    &SystemAudioTarget::Application {
                        id: "".into(),
                        name: "Test Player".into(),
                    },
                    ui_test,
                    || panic!("Invalid target probed OS")
                ),
                Err("application_audio_invalid_target".into())
            );
        }
    }

    #[test]
    fn target_switch_restarts_selected_system_capture_but_not_microphone_only() {
        assert!(!audio_capture_selection_needs_restart(
            AudioInput::Microphone,
            AudioInput::Microphone
        ));
        for previous in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
            for selected in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
                if previous != selected || selected != AudioInput::Microphone {
                    assert!(audio_capture_selection_needs_restart(previous, selected));
                }
            }
        }
    }

    #[tokio::test]
    async fn application_target_change_waits_for_old_system_capture_without_stopping_selected_microphone(
    ) {
        let mut released = Vec::new();
        release_reconfigured_audio_sources(AudioInput::Both, AudioInput::Both, true, |source| {
            released.push(source);
            async { Ok(()) }
        })
        .await
        .unwrap();
        assert_eq!(released, [AudioSource::System]);
        released.clear();
        release_reconfigured_audio_sources(
            AudioInput::Microphone,
            AudioInput::Microphone,
            true,
            |source| {
                released.push(source);
                async { Ok(()) }
            },
        )
        .await
        .unwrap();
        assert!(released.is_empty());
        assert_eq!(
            release_reconfigured_audio_sources(
                AudioInput::System,
                AudioInput::System,
                true,
                |_| async { Err("audio_input_switch_stop_failed".into()) }
            )
            .await,
            Err("audio_input_switch_stop_failed".into())
        );
    }

    #[tokio::test]
    async fn audio_attempt_failure_cannot_claim_a_rebuilt_session_with_the_same_generation() {
        let active = Arc::new(AtomicU64::new(7));
        let sequence = Arc::new(AtomicU64::new(7));
        let transition = Arc::new(Mutex::new(Some((7, 100))));
        let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
        let (dispatch_tx, dispatch_rx) = tokio::sync::oneshot::channel();
        let delayed_failure = {
            let active = Arc::clone(&active);
            let sequence = Arc::clone(&sequence);
            let transition = Arc::clone(&transition);
            tokio::spawn(async move {
                // The old native/DSP callback has already reported a failure,
                // but its receiver has not reached the invalidation boundary.
                queued_tx.send(()).unwrap();
                dispatch_rx.await.unwrap();
                let mut owner = transition.lock().unwrap();
                invalidate_audio_attempt_atoms(&active, &sequence, &mut owner, 7, 100)
            })
        };
        queued_rx.await.unwrap();
        {
            let mut owner = transition.lock().unwrap();
            *owner = None; // cleanup retires the old attempt under the gate
            *owner = Some((7, 101)); // settings rebuild keeps generation 7
        }
        dispatch_tx.send(()).unwrap();
        assert_eq!(delayed_failure.await.unwrap(), None);
        assert_eq!(active.load(Ordering::SeqCst), 7);
        assert_eq!(sequence.load(Ordering::SeqCst), 7);
        let mut owner = transition.lock().unwrap();
        assert_eq!(
            invalidate_audio_attempt_atoms(&active, &sequence, &mut owner, 7, 101),
            Some(8)
        );
        assert_eq!(*owner, None);
        assert_eq!(active.load(Ordering::SeqCst), NO_GENERATION);
    }

    #[tokio::test]
    async fn audio_input_switch_seals_old_events_before_resetting_confirmation_ids() {
        let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
        let confirmed = LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: Some(1),
            utterance_id: 1,
            source: "Synthetic confirmed system".into(),
            translation: "Synthetic confirmation".into(),
            language: Some("en".into()),
        };
        controller.lock().unwrap().did_connect();
        controller
            .lock()
            .unwrap()
            .handle_from(AudioSource::System, confirmed.clone());
        let expected_history = controller.lock().unwrap().state.subtitles.history.clone();
        let active = Arc::new(AtomicU64::new(7));
        let sequence = AtomicU64::new(7);
        let transition = Mutex::new(());
        let content = Arc::new(TokioMutex::new(()));
        let held_content = content.lock().await;
        let (popped_tx, popped_rx) = tokio::sync::oneshot::channel();
        let pump = {
            let controller = Arc::clone(&controller);
            let active = Arc::clone(&active);
            let content = Arc::clone(&content);
            tokio::spawn(async move {
                // These events have left the provider queue before the switch,
                // but cannot apply until its shared content gate is released.
                popped_tx.send(()).unwrap();
                let _content = content.lock().await;
                for event in [
                    confirmed,
                    LiveTranslateServerEvent::SourceDraft {
                        text: "Retired synthetic draft".into(),
                        language: Some("ja".into()),
                    },
                ] {
                    if generation_accepts_event(
                        active.load(Ordering::SeqCst),
                        NO_GENERATION,
                        7,
                        &event,
                    ) {
                        controller
                            .lock()
                            .unwrap()
                            .handle_from(AudioSource::System, event);
                    }
                }
            })
        };
        popped_rx.await.unwrap();
        let boundary = {
            let _transition = transition.lock().unwrap();
            commit_audio_input_switch_boundary(
                &active,
                &sequence,
                7,
                &mut controller.lock().unwrap(),
                AudioInput::Both,
                || Ok(()),
            )
        };
        assert_eq!(boundary.unwrap(), (8, 7));
        assert_eq!(active.load(Ordering::SeqCst), NO_GENERATION);
        drop(held_content);
        pump.await.unwrap();
        let state = &controller.lock().unwrap().state;
        assert_eq!(state.subtitles.history, expected_history);
        assert!(state
            .subtitles
            .tracks
            .iter()
            .all(|track| !track.source.text.contains("Retired")));
    }

    #[test]
    fn audio_input_switch_superseded_or_failed_save_never_resets_live_watermarks() {
        for (epoch, save_fails) in [(8, false), (7, true)] {
            let active = AtomicU64::new(7);
            let sequence = AtomicU64::new(epoch);
            let mut controller = TranslationSessionController::default();
            controller.did_connect();
            let final_event = LiveTranslateServerEvent::SubtitleConfirmedPair {
                source_utterance_id: Some(1),
                utterance_id: 1,
                source: "Synthetic original".into(),
                translation: "Synthetic translation".into(),
                language: Some("en".into()),
            };
            controller.handle_from(AudioSource::System, final_event.clone());
            let original_state = controller.state.clone();
            let persisted = AtomicBool::new(false);
            let result = commit_audio_input_switch_boundary(
                &active,
                &sequence,
                7,
                &mut controller,
                AudioInput::Both,
                || {
                    persisted.store(true, Ordering::SeqCst);
                    Err("audio_input_switch_save_failed".into())
                },
            );
            assert_eq!(
                result,
                Err(if save_fails {
                    "audio_input_switch_save_failed"
                } else {
                    "audio_input_switch_superseded"
                }
                .into())
            );
            assert_eq!(persisted.load(Ordering::SeqCst), save_fails);
            assert_eq!(active.load(Ordering::SeqCst), 7);
            assert_eq!(sequence.load(Ordering::SeqCst), epoch);
            assert_eq!(controller.state, original_state);
            controller.handle_from(AudioSource::System, final_event);
            assert_eq!(
                controller.state, original_state,
                "the old provider ID must still be rejected as a replay"
            );
        }
    }

    #[tokio::test]
    async fn audio_input_switch_paused_idle_and_error_wait_for_native_release() {
        for status in [
            SessionStatus::Listening,
            SessionStatus::Idle,
            SessionStatus::Error("synthetic failure".into()),
        ] {
            let action =
                audio_input_switch_action(&status, status == SessionStatus::Listening, false, 0)
                    .unwrap();
            assert_eq!(action, AudioInputSwitchAction::ReconfigureOnly);
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let mut started_tx = Some(started_tx);
                let mut finished_rx = Some(finished_rx);
                release_reconfigured_audio_sources(
                    AudioInput::Both,
                    AudioInput::System,
                    false,
                    |source| {
                        assert_eq!(source, AudioSource::Microphone);
                        started_tx.take().unwrap().send(()).unwrap();
                        let finished = finished_rx.take().unwrap();
                        async move {
                            finished.await.unwrap();
                            Ok(())
                        }
                    },
                )
                .await?;
                Ok::<_, String>(action)
            });
            started_rx.await.unwrap();
            tokio::task::yield_now().await;
            assert!(
                !task.is_finished(),
                "selection-only changes still wait for the native microphone to release"
            );
            finished_tx.send(()).unwrap();
            assert_eq!(
                task.await.unwrap(),
                Ok(AudioInputSwitchAction::ReconfigureOnly)
            );
            assert_eq!(
                release_reconfigured_audio_sources(
                    AudioInput::Both,
                    AudioInput::System,
                    false,
                    |_| async { Err("audio_input_switch_stop_failed".into()) }
                )
                .await,
                Err("audio_input_switch_stop_failed".into())
            );
        }
    }

    #[tokio::test]
    async fn audio_input_switch_hands_off_to_an_already_accepted_newer_pause() {
        let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
        controller.lock().unwrap().did_connect();
        let active = Arc::new(AtomicU64::new(7));
        let sequence = Arc::new(AtomicU64::new(7));
        let paused = Arc::new(AtomicBool::new(false));
        let lifecycle = Arc::new(TokioMutex::new(()));
        let transition = Arc::new(Mutex::new(()));
        let switch_guard = Arc::clone(&lifecycle).lock_owned().await;
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let (claim_tx, claim_rx) = tokio::sync::oneshot::channel();
        let (claimed_tx, claimed_rx) = tokio::sync::oneshot::channel();
        let pause = {
            let controller = Arc::clone(&controller);
            let active = Arc::clone(&active);
            let sequence = Arc::clone(&sequence);
            let paused = Arc::clone(&paused);
            let lifecycle = Arc::clone(&lifecycle);
            let transition = Arc::clone(&transition);
            tokio::spawn(async move {
                assert!(pause_transition_is_valid(
                    &controller.lock().unwrap().state.status,
                    false,
                    active.load(Ordering::SeqCst)
                ));
                accepted_tx.send(()).unwrap();
                claim_rx.await.unwrap();
                let pause_epoch = {
                    let _transition = transition.lock().unwrap();
                    let current = sequence.load(Ordering::SeqCst);
                    advance_lifecycle_sequence_if_current(&sequence, current).unwrap()
                };
                claimed_tx.send(pause_epoch).unwrap();
                let _lifecycle = lifecycle.lock_owned().await;
                if lifecycle_sequence_matches(&sequence, pause_epoch)
                    && accepted_pause_transition_is_valid(
                        &controller.lock().unwrap().state.status,
                        paused.load(Ordering::SeqCst),
                        active.load(Ordering::SeqCst),
                        true,
                    )
                {
                    active.store(NO_GENERATION, Ordering::SeqCst);
                    paused.store(true, Ordering::SeqCst);
                    controller.lock().unwrap().did_pause();
                    true
                } else {
                    false
                }
            })
        };
        accepted_rx.await.unwrap();
        {
            let _transition = transition.lock().unwrap();
            assert_eq!(
                commit_audio_input_switch_boundary(
                    &active,
                    &sequence,
                    7,
                    &mut controller.lock().unwrap(),
                    AudioInput::Both,
                    || Ok(())
                )
                .unwrap(),
                (8, 7)
            );
        }
        claim_tx.send(()).unwrap();
        assert_eq!(claimed_rx.await.unwrap(), 9);
        controller.lock().unwrap().begin_connecting();
        let resources_installed = Arc::new(AtomicBool::new(true));
        let (teardown_started_tx, teardown_started_rx) = tokio::sync::oneshot::channel();
        let (finish_teardown_tx, finish_teardown_rx) = tokio::sync::oneshot::channel();
        let teardown = {
            let resources_installed = Arc::clone(&resources_installed);
            tokio::spawn(async move {
                let _switch_guard = retain_lifecycle_during_teardown(switch_guard, async {
                    teardown_started_tx.send(()).unwrap();
                    finish_teardown_rx.await.unwrap();
                    resources_installed.store(false, Ordering::SeqCst);
                })
                .await;
            })
        };
        teardown_started_rx.await.unwrap();
        tokio::task::yield_now().await;
        assert!(
            !pause.is_finished(),
            "Pause cannot expose Resume before old capture/client teardown finishes"
        );
        assert!(resources_installed.load(Ordering::SeqCst));
        finish_teardown_tx.send(()).unwrap();
        teardown.await.unwrap();
        assert!(pause.await.unwrap());
        let _resume_guard = Arc::clone(&lifecycle).lock_owned().await;
        assert!(
            !resources_installed.load(Ordering::SeqCst),
            "Resume cannot meet an old installed client or capture"
        );
        assert!(paused.load(Ordering::SeqCst));
        assert_eq!(
            controller.lock().unwrap().state.status,
            SessionStatus::Listening
        );
        assert_eq!(active.load(Ordering::SeqCst), NO_GENERATION);
        assert!(
            !lifecycle_sequence_matches(&sequence, 8),
            "the superseded switch cannot reconnect after pause"
        );
        assert!(resume_transition_is_valid(
            &controller.lock().unwrap().state.status,
            paused.load(Ordering::SeqCst),
            active.load(Ordering::SeqCst),
            true
        ));
    }

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
    fn audio_input_switch_reconnects_only_a_running_unpaused_session() {
        assert_eq!(
            audio_input_switch_action(&SessionStatus::Listening, false, false, 0),
            Ok(AudioInputSwitchAction::Reconnect)
        );
        for status in [
            SessionStatus::Listening,
            SessionStatus::Idle,
            SessionStatus::Error("synthetic failure".into()),
        ] {
            assert_eq!(
                audio_input_switch_action(&status, true, false, 0),
                Ok(AudioInputSwitchAction::ReconfigureOnly)
            );
        }
        for status in [
            SessionStatus::Idle,
            SessionStatus::Error("synthetic failure".into()),
        ] {
            assert_eq!(
                audio_input_switch_action(&status, false, false, 0),
                Ok(AudioInputSwitchAction::ReconfigureOnly)
            );
        }
    }

    #[test]
    fn audio_input_switch_rejects_inflight_lifecycle_and_recovery() {
        for status in [SessionStatus::Connecting, SessionStatus::Stopping] {
            for paused in [false, true] {
                assert_eq!(
                    audio_input_switch_action(&status, paused, false, 0),
                    Err("audio_input_switch_busy")
                );
            }
        }
        assert_eq!(
            audio_input_switch_action(&SessionStatus::Listening, false, true, 0),
            Err("audio_input_switch_busy")
        );
        assert_eq!(
            audio_input_switch_action(&SessionStatus::Listening, true, false, 1),
            Err("audio_input_switch_busy")
        );
    }

    fn capture_status_test_pipeline() -> Arc<AudioSendPipeline> {
        Arc::new(AudioSendPipeline::spawn(
            |_| async { Ok::<(), ()>(()) },
            |_| {},
        ))
    }

    #[tokio::test]
    async fn capture_status_reports_sound_silence_and_missing_pcm_per_lane() {
        let system = capture_status_test_pipeline();
        let microphone = capture_status_test_pipeline();
        system.ingress().unwrap().try_send(vec![128, 0]).unwrap();
        let system_slot = Mutex::new(Some(Arc::clone(&system)));
        let microphone_slot = Mutex::new(Some(Arc::clone(&microphone)));
        let generation = AtomicU64::new(7);
        let sound = source_capture_observation(&system_slot, &generation, 7).unwrap();
        let no_pcm = source_capture_observation(&microphone_slot, &generation, 7).unwrap();
        assert_eq!(
            sound,
            crate::audio::CaptureSignal {
                pcm_data_recent: true,
                sound_recent: true
            }
        );
        assert_eq!(
            no_pcm,
            crate::audio::CaptureSignal {
                pcm_data_recent: false,
                sound_recent: false
            }
        );
        // Receiving silent PCM is different from receiving no samples. Sound
        // on the system lane must not leak into this microphone observation.
        microphone.ingress().unwrap().try_send(vec![0, 0]).unwrap();
        let silent = source_capture_observation(&microphone_slot, &generation, 7).unwrap();
        assert_eq!(
            silent,
            crate::audio::CaptureSignal {
                pcm_data_recent: true,
                sound_recent: false
            }
        );
        assert_eq!(
            source_capture_observation(&system_slot, &generation, 7),
            Some(sound)
        );
        system.stop();
        microphone.stop();
    }

    #[tokio::test]
    async fn capture_status_does_not_reuse_retired_or_removed_pipeline_activity() {
        let old = capture_status_test_pipeline();
        old.ingress().unwrap().try_send(vec![128, 0]).unwrap();
        let slot = Mutex::new(Some(Arc::clone(&old)));
        let generation = AtomicU64::new(7);
        assert!(
            source_capture_observation(&slot, &generation, 7)
                .unwrap()
                .sound_recent
        );
        // Stop/pause invalidates the active generation before teardown awaits.
        assert!(source_capture_observation(&slot, &generation, NO_GENERATION).is_none());
        assert!(source_capture_observation(&slot, &generation, 8).is_none());
        let removed = slot.lock().unwrap().take().unwrap();
        assert!(source_capture_observation(&slot, &generation, 7).is_none());
        // A new pipeline begins unobserved even while a retired Arc survives.
        let next = capture_status_test_pipeline();
        *slot.lock().unwrap() = Some(Arc::clone(&next));
        generation.store(8, Ordering::SeqCst);
        assert_eq!(
            source_capture_observation(&slot, &generation, 8),
            Some(crate::audio::CaptureSignal {
                pcm_data_recent: false,
                sound_recent: false,
            })
        );
        removed.stop();
        next.stop();
    }

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
    fn either_source_finishing_or_failing_ends_all_selected_sources() {
        for source in [AudioSource::System, AudioSource::Microphone] {
            for event in [
                LiveTranslateServerEvent::SessionFinished,
                LiveTranslateServerEvent::Error {
                    code: "fatal".into(),
                    message: "unavailable".into(),
                },
            ] {
                let mut controller = TranslationSessionController::default();
                controller.set_audio_input(AudioInput::Both);
                controller.did_connect();
                for &input in AudioInput::Both.sources() {
                    controller.handle_from(input, LiveTranslateServerEvent::TranslationStarted);
                }
                controller.handle_from(source, event.clone());
                apply_terminal_event_to_all_sources(&mut controller, &event);
                assert!(!controller.state.status.is_active());
                assert!(!controller.state.is_translation_pending);
                assert!(controller
                    .state
                    .subtitles
                    .tracks
                    .iter()
                    .all(|track| !track.is_translation_pending));
                // A normal start following terminal cleanup can establish both again.
                controller.begin_connecting();
                controller.did_connect();
                assert_eq!(controller.state.status, SessionStatus::Listening);
            }
        }
    }

    #[tokio::test]
    async fn dual_source_start_failure_rolls_back_every_installed_lane() {
        let installed = Arc::new(Mutex::new(Vec::new()));
        let rollbacks = AtomicUsize::new(0);
        let result = connect_source_group(
            AudioInput::Both.sources(),
            |source| {
                let installed = Arc::clone(&installed);
                async move {
                    installed.lock().unwrap().push(source);
                    if source == AudioSource::Microphone {
                        Err("microphone unavailable".into())
                    } else {
                        Ok(())
                    }
                }
            },
            |_| async {
                rollbacks.fetch_add(1, Ordering::SeqCst);
                assert_eq!(*installed.lock().unwrap(), AudioInput::Both.sources());
                installed.lock().unwrap().clear();
            },
        )
        .await;
        assert_eq!(result, Err("microphone unavailable".into()));
        assert!(installed.lock().unwrap().is_empty());
        assert_eq!(rollbacks.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dual_source_stop_cancels_pending_second_start_and_rolls_back_first() {
        let active = Arc::new(AtomicU64::new(1));
        let sequence = Arc::new(AtomicU64::new(1));
        let notify = Arc::new(Notify::new());
        let installed = Arc::new(Mutex::new(Vec::new()));
        let result = connect_source_group(
            AudioInput::Both.sources(),
            |source| {
                let active = Arc::clone(&active);
                let sequence = Arc::clone(&sequence);
                let notify = Arc::clone(&notify);
                let installed = Arc::clone(&installed);
                async move {
                    installed.lock().unwrap().push(source);
                    if source == AudioSource::System {
                        return Ok(());
                    }
                    active.store(NO_GENERATION, Ordering::SeqCst);
                    sequence.store(2, Ordering::SeqCst);
                    notify.notify_waiters();
                    run_generation_bound_operation(
                        active,
                        sequence,
                        notify,
                        1,
                        std::future::pending::<()>(),
                    )
                    .await
                    .map(|_| ())
                }
            },
            |_| async {
                installed.lock().unwrap().clear();
            },
        )
        .await;
        assert_eq!(result, Err(SESSION_START_CANCELLED.into()));
        assert!(installed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dual_source_success_keeps_both_lanes_and_never_rolls_back() {
        let started = Mutex::new(Vec::new());
        let rollback = AtomicBool::new(false);
        connect_source_group(
            AudioInput::Both.sources(),
            |source| {
                started.lock().unwrap().push(source);
                std::future::ready(Ok(()))
            },
            |_| async {
                rollback.store(true, Ordering::SeqCst);
            },
        )
        .await
        .unwrap();
        assert_eq!(*started.lock().unwrap(), AudioInput::Both.sources());
        assert!(!rollback.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn popped_pre_clear_event_cannot_restore_overlay_or_private_history() {
        let directory = tempfile::tempdir().unwrap();
        let history = SessionHistory::new(directory.path().to_path_buf(), false);
        let id = uuid::Uuid::new_v4().to_string();
        history.begin(&id, 0).unwrap();
        let mut controller = TranslationSessionController::default();
        controller.did_connect();
        let (sender, mut receiver) = provider_event_channel();
        let final_event = |source: &str| LiveTranslateServerEvent::SubtitleFinalPair {
            source: source.into(),
            translation: "synthetic translation".into(),
            language: None,
        };
        sender.send(final_event("synthetic old pair")).unwrap();
        // This is the race a receiver-only filter cannot fix: already popped.
        let popped = receiver.recv_with_revision().await.unwrap();
        let generation = 3;
        let content = TokioMutex::new((generation, sender.content_revision()));
        {
            let mut boundary = content.lock().await;
            history.clear_text(&id).unwrap();
            boundary.1 = sender.advance_content_revision();
            controller.clear_subtitles();
        }
        let boundary = content.lock().await;
        assert!(!subtitle_content_is_current(generation, &popped, *boundary));
        assert!(generation_accepts_event(
            NO_GENERATION,
            generation,
            generation,
            &popped.event
        ));
        // A valid stopping tail still needs the retained content revision.
        assert!(!subtitle_content_is_current(generation, &popped, *boundary));
        assert!(controller.state.subtitles.history.is_empty());
        drop(boundary);
        sender.send(final_event("synthetic next sentence")).unwrap();
        let next = receiver.recv_with_revision().await.unwrap();
        {
            let boundary = content.lock().await;
            assert!(subtitle_content_is_current(generation, &next, *boundary));
            controller.handle(next.event);
            history
                .append_pair(&id, controller.state.subtitles.history.last().unwrap())
                .unwrap();
        }
        assert_eq!(controller.state.subtitles.history.len(), 1);
        let stored = std::fs::read_to_string(directory.path().join(format!("{id}.jsonl"))).unwrap();
        assert!(!stored.contains("synthetic old pair"));
        assert!(stored.contains("synthetic next sentence"));
        // An older session's revision cannot match a new session by accident.
        assert!(!subtitle_content_is_current(
            generation + 1,
            &popped,
            (generation, 0)
        ));
        for event in [
            LiveTranslateServerEvent::SessionFinished,
            LiveTranslateServerEvent::Error {
                code: "authentication_error".into(),
                message: "fixed label".into(),
            },
        ] {
            assert!(subtitle_content_is_current(
                generation,
                &ProviderEvent {
                    transport_sequence: None,
                    content_revision: 0,
                    event
                },
                (generation, sender.content_revision())
            ));
        }
    }

    #[test]
    fn repeated_confirmations_trigger_private_file_append_even_when_text_is_equal() {
        let first = crate::core::models::SubtitlePair::new(
            "Synthetic source".into(),
            "Synthetic translation".into(),
            1,
        );
        let second = crate::core::models::SubtitlePair::new(
            first.source.clone(),
            first.translation.clone(),
            2,
        );
        assert_eq!(
            first, second,
            "legacy text equality intentionally ignores timestamps"
        );
        assert!(confirmed_history_tail_changed(Some(&first), Some(&second)));
        assert!(!confirmed_history_tail_changed(
            Some(&second),
            Some(&second)
        ));
        assert!(confirmed_history_tail_changed(None, Some(&first)));
        assert!(!confirmed_history_tail_changed(Some(&first), None));
    }

    #[test]
    fn preview_pending_is_serialized_separately_from_final_pending() {
        let mut controller = TranslationSessionController::default();
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::PreviewTranslationStarted { request_id: 42 });
        let payload = serde_json::to_value(SessionStateEvent::from(&controller.state)).unwrap();
        assert_eq!(payload["isTranslationPreviewPending"], true);
        assert_eq!(payload["isTranslationPending"], false);
        assert!(payload.get("requestId").is_none());
        controller.handle(LiveTranslateServerEvent::PreviewTranslationFinished { request_id: 42 });
        let payload = serde_json::to_value(SessionStateEvent::from(&controller.state)).unwrap();
        assert_eq!(payload["isTranslationPreviewPending"], false);
    }

    fn synthetic_mt_scope(profile_id: &str, route: MTBudgetRoute) -> MTBudgetScope {
        MTBudgetScope {
            profile_id: profile_id.into(),
            provider: ProviderKind::AlibabaCloud,
            route,
        }
    }

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
                MTBudgetScope::for_configuration("synthetic-profile".into(), &configuration)
                    .is_none()
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
                let scope = MTBudgetScope::for_configuration("synthetic-profile".into(), &selected)
                    .unwrap();
                assert!(scope.route == expected_route);
                assert_eq!(scope.provider, provider);
                assert_eq!(scope.profile_id, "synthetic-profile");
                selected.target_language = TargetLanguage::Original;
                assert!(
                    MTBudgetScope::for_configuration("synthetic-profile".into(), &selected)
                        .is_none()
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
    fn latency_payload_ignores_stopped_paused_reconnecting_and_failed_sessions() {
        let health = Some(HealthCheckLatency {
            generation: 41,
            task_id: 9,
            milliseconds: 80,
        });
        let translation = Some(TranslationLatency {
            milliseconds: 120,
            kind: TranslationLatencyKind::Request,
        });
        assert_eq!(
            visible_session_latencies(
                &SessionStatus::Listening,
                false,
                false,
                41,
                9,
                health,
                translation,
            ),
            (Some(80), Some(120), Some(TranslationLatencyKind::Request))
        );
        for (status, paused, recovering, generation) in [
            (SessionStatus::Idle, false, false, 41),
            (SessionStatus::Connecting, false, false, 41),
            (SessionStatus::Stopping, false, false, 41),
            (
                SessionStatus::Error("synthetic failure".into()),
                false,
                false,
                41,
            ),
            (SessionStatus::Listening, true, false, 41),
            (SessionStatus::Listening, false, true, 41),
            (SessionStatus::Listening, false, false, NO_GENERATION),
        ] {
            assert_eq!(
                visible_session_latencies(
                    &status,
                    paused,
                    recovering,
                    generation,
                    9,
                    health,
                    translation,
                ),
                (None, None, None)
            );
        }
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
    fn bootstrapping_state_serializes_unknown_latencies_as_nullable_fields() {
        let state = TranslationSessionState::default();
        let payload = serde_json::to_value(SessionStateEvent::from(&state)).unwrap();
        for field in [
            "apiLatencyMs",
            "translationLatencyMs",
            "translationLatencyKind",
            "translationRecovery",
        ] {
            assert_eq!(payload.get(field), Some(&serde_json::Value::Null));
        }
    }

    #[test]
    fn ui_live_fixture_keeps_selected_sources_paired_without_confirming_history() {
        for audio_input in [AudioInput::System, AudioInput::Microphone, AudioInput::Both] {
            let mut controller = TranslationSessionController::default();
            controller.set_audio_input(audio_input);
            controller.archive_mut().begin(true, 1);
            controller.did_connect();
            seed_ui_test_live_subtitles(&mut controller, audio_input);

            let subtitles = &controller.state.subtitles;
            let sources: Vec<_> = subtitles
                .tracks
                .iter()
                .map(|track| track.audio_source)
                .collect();
            assert_eq!(sources, audio_input.sources());
            assert!(subtitles.history.is_empty());
            assert_eq!(controller.archive().page("", 0).total, 0);
            for track in &subtitles.tracks {
                let pair = track.preview_pair.as_ref().unwrap();
                assert!(!track.source.is_final);
                assert!(!track.translation.is_final);
                assert!(track.source.utterance_id.is_some());
                assert_eq!(track.source.utterance_id, track.translation.utterance_id);
                assert_eq!(track.source.utterance_id, pair.utterance_id);
                assert_eq!(track.source.text, pair.source);
                assert_eq!(track.translation.text, pair.translation);
                assert!(track.history.is_empty());
            }
            if subtitles.tracks.len() == 2 {
                assert_ne!(
                    subtitles.tracks[0].source.utterance_id,
                    subtitles.tracks[1].source.utterance_id
                );
            }
        }
    }

    #[test]
    fn ui_export_fixture_survives_stop_and_repeated_finalization_but_resets_on_new_start() {
        let mut controller = TranslationSessionController::default();
        controller.archive_mut().begin(true, 1);
        controller.did_connect();
        controller.handle(LiveTranslateServerEvent::SubtitleFinalPair {
            source: "Synthetic export fixture".into(),
            language: Some("en".into()),
            translation: "合成导出示例".into(),
        });
        let mut recording = crate::core::session_archive::AudioRecording::default();
        recording.begin(true);
        recording.append(16_000, &[0, 0, 1, 0]);
        controller.did_stop();
        let transcript = controller.archive().export().unwrap();
        let audio = recording.export().unwrap();
        for _ in 0..2 {
            finish_ui_test_archive(&mut controller, &mut recording, true);
            assert_eq!(controller.state.status, SessionStatus::Idle);
            assert_eq!(controller.archive().page("Synthetic", 0).total, 1);
            assert_eq!(controller.archive().export().unwrap(), transcript);
            assert_eq!(recording.export().unwrap(), audio);
        }
        // The next session begins fresh; opting out must not retain a fixture.
        controller.archive_mut().begin(false, 2);
        recording.begin(false);
        assert!(controller.archive().export().is_none());
        assert!(recording.export().is_none());
    }

    #[test]
    fn ordinary_ui_test_finalization_still_clears_synthetic_buffers() {
        let mut controller = TranslationSessionController::default();
        controller.archive_mut().begin(true, 1);
        controller.handle(LiveTranslateServerEvent::SubtitleFinalPair {
            source: "Synthetic export fixture".into(),
            language: Some("en".into()),
            translation: "合成导出示例".into(),
        });
        let mut recording = crate::core::session_archive::AudioRecording::default();
        recording.begin(true);
        recording.append(16_000, &[0, 0]);
        finish_ui_test_archive(&mut controller, &mut recording, false);
        assert_eq!(controller.archive().page("", 0).total, 0);
        assert!(controller.archive().export().is_none());
        assert!(recording.export().is_none());
    }

    #[test]
    fn terminal_error_remains_visible_but_is_not_an_active_session() {
        let status = SessionStatus::Error("capture failed".into());

        assert!(!status.is_active());
        assert!(status_should_show_overlay(&status));
        assert!(!status_should_show_overlay(&SessionStatus::Idle));
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
    fn only_transient_transport_and_bounded_queue_errors_recover() {
        for code in [
            "transport_error",
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
                let owned_epoch =
                    invalidate_generation_atoms(&active, &sequence, generation).unwrap();
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

        let owner = StartRequestGuard {
            in_progress: Arc::clone(&in_progress),
        };
        drop(owner);
        assert!(try_begin_start(&in_progress));
        in_progress.store(false, Ordering::SeqCst);
    }

    #[tokio::test]
    async fn stop_publication_waits_until_the_last_accepted_tail_is_published() {
        let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
        controller.lock().unwrap().did_connect();
        controller.lock().unwrap().begin_stopping();
        let published = Arc::new(Mutex::new(vec![SessionStateEvent::from(
            &controller.lock().unwrap().state,
        )]));
        assert!(published.lock().unwrap()[0].subtitles.history.is_empty());
        let tail = LiveTranslateServerEvent::SubtitleConfirmedPair {
            source_utterance_id: Some(7),
            utterance_id: 1,
            source: "Synthetic stop tail".into(),
            translation: "Synthetic final translation".into(),
            language: Some("en".into()),
        };
        assert!(generation_accepts_event(NO_GENERATION, 9, 9, &tail));
        controller
            .lock()
            .unwrap()
            .handle_from(AudioSource::System, tail);
        controller.lock().unwrap().did_stop();

        // Hold the same kind of content gate used by publish_state_now. A
        // coalesced request alone cannot complete stop while delivery is gated.
        let content = Arc::new(TokioMutex::new(()));
        let held = Arc::clone(&content).lock_owned().await;
        let (requested, request_seen) = tokio::sync::oneshot::channel();
        let publication_controller = Arc::clone(&controller);
        let publication_output = Arc::clone(&published);
        let mut stopped = tokio::spawn(async move {
            publish_stop_boundary(|| requested.send(()).unwrap(), async move {
                let _content = content.lock().await;
                publication_output
                    .lock()
                    .unwrap()
                    .push(SessionStateEvent::from(
                        &publication_controller.lock().unwrap().state,
                    ));
            })
            .await;
        });
        request_seen.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut stopped)
                .await
                .is_err(),
            "stop returned while only the old stopping/history=0 snapshot was published"
        );
        // No std controller guard is held while waiting for publication.
        assert!(controller.try_lock().is_ok());
        drop(held);
        tokio::time::timeout(Duration::from_millis(200), stopped)
            .await
            .unwrap()
            .unwrap();
        let output = published.lock().unwrap();
        assert_eq!(output.len(), 2);
        let final_state = &output[1];
        assert!(matches!(final_state.status, StatusPayload::Idle));
        assert_eq!(final_state.subtitles.history.len(), 1);
        assert_eq!(
            final_state.subtitles.history[0].source,
            "Synthetic stop tail"
        );
        assert_eq!(
            final_state.subtitles.history[0].translation,
            "Synthetic final translation"
        );
    }

    #[test]
    fn stop_rejects_old_events_but_keeps_atomic_confirmed_tail() {
        let generation = 9;
        let final_pair = LiveTranslateServerEvent::SubtitleFinalPair {
            source: "tail".into(),
            language: Some("en".into()),
            translation: "尾句".into(),
        };
        assert!(generation_accepts_event(
            NO_GENERATION,
            generation,
            generation,
            &final_pair,
        ));
        assert!(!generation_accepts_event(
            NO_GENERATION,
            generation,
            generation,
            &LiveTranslateServerEvent::TranslationDraft("stale".into()),
        ));
        let identified_pair = LiveTranslateServerEvent::SubtitleConfirmedPair {
            utterance_id: 1,
            source_utterance_id: None,
            source: "Synthetic tail".into(),
            language: Some("en".into()),
            translation: "Synthetic translation".into(),
        };
        assert!(generation_accepts_event(
            NO_GENERATION,
            generation,
            generation,
            &identified_pair
        ));
        assert!(!generation_accepts_event(
            generation + 1,
            NO_GENERATION,
            generation,
            &identified_pair
        ));
        let dashscope_pair = LiveTranslateServerEvent::SubtitleIdentifiedFinalPair {
            utterance_id: "provider-source".into(),
            source: "Synthetic identified tail".into(),
            language: Some("en".into()),
            translation: "合成有身份尾句".into(),
        };
        assert!(generation_accepts_event(
            NO_GENERATION,
            generation,
            generation,
            &dashscope_pair
        ));
        assert!(!generation_accepts_event(
            generation + 1,
            NO_GENERATION,
            generation,
            &dashscope_pair
        ));
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
    async fn terminal_teardown_blocks_new_start_or_profile_mutation() {
        let lock = Arc::new(TokioMutex::new(()));
        let teardown_count = Arc::new(AtomicUsize::new(1));
        let notify = Arc::new(Notify::new());
        let mut waiter = tokio::spawn(lock_after_operations(
            Arc::clone(&lock),
            Arc::clone(&teardown_count),
            Arc::clone(&notify),
        ));

        assert!(tokio::time::timeout(Duration::from_millis(20), &mut waiter)
            .await
            .is_err());
        teardown_count.store(0, Ordering::SeqCst);
        notify.notify_waiters();
        assert!(tokio::time::timeout(Duration::from_millis(200), waiter)
            .await
            .is_ok());
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
    fn stop_after_settings_switch_prevents_late_reconnect_claim() {
        let sequence = AtomicU64::new(101);
        let switch_epoch = sequence.load(Ordering::SeqCst);

        let stop_epoch = sequence.fetch_add(1, Ordering::SeqCst) + 1;
        assert_eq!(stop_epoch, 102);
        assert_eq!(
            advance_lifecycle_sequence_if_current(&sequence, switch_epoch),
            None
        );
        assert_eq!(sequence.load(Ordering::SeqCst), stop_epoch);
    }

    #[test]
    fn profile_selection_reconnects_only_a_live_unpaused_session() {
        assert_eq!(
            profile_switch_action(&SessionStatus::Listening, false, false, 0),
            Ok(ProfileSwitchAction::Reconnect)
        );
        assert_eq!(
            profile_switch_action(&SessionStatus::Listening, true, false, 0),
            Ok(ProfileSwitchAction::KeepPaused)
        );
        for status in [
            SessionStatus::Idle,
            SessionStatus::Error("synthetic".into()),
        ] {
            assert_eq!(
                profile_switch_action(&status, false, false, 0),
                Ok(ProfileSwitchAction::SelectOnly)
            );
        }
        for status in [SessionStatus::Connecting, SessionStatus::Stopping] {
            for paused in [false, true] {
                assert_eq!(
                    profile_switch_action(&status, paused, false, 0),
                    Err("profile_switch_busy")
                );
            }
        }
        for (recovering, operations) in [(true, 0), (false, 1)] {
            assert_eq!(
                profile_switch_action(&SessionStatus::Listening, true, recovering, operations),
                Err("profile_switch_busy")
            );
        }
    }

    #[test]
    fn profile_selection_preserves_recordings_by_rejecting_sample_rate_changes() {
        for current_rate in [16_000, 24_000] {
            for next_rate in [16_000, 24_000] {
                assert!(ensure_profile_switch_audio_format(false, current_rate, next_rate).is_ok());
                assert_eq!(
                    ensure_profile_switch_audio_format(true, current_rate, next_rate),
                    if current_rate == next_rate {
                        Ok(())
                    } else {
                        Err("profile_switch_recording_requires_stop".into())
                    }
                );
            }
        }
    }

    #[test]
    fn profile_selection_commit_keeps_previous_configuration_on_failed_or_stale_selection() {
        let old = LiveTranslationConfiguration::for_provider(
            ProviderKind::AlibabaCloud,
            "synthetic-old",
            SourceLanguage::Japanese,
            crate::core::models::TargetLanguage::English,
            TranslationMode::Turbo,
        );
        let next = LiveTranslationConfiguration::for_provider(
            ProviderKind::OpenAIRealtime,
            "synthetic-next",
            SourceLanguage::Automatic,
            crate::core::models::TargetLanguage::English,
            TranslationMode::Turbo,
        );
        let active = Mutex::new(Some(old.clone()));
        let sequence = AtomicU64::new(11);
        assert_eq!(
            commit_profile_selection(&sequence, 11, &active, Some(next.clone()), || {
                Err("profile_switch_save_failed".into())
            }),
            Err("profile_switch_save_failed".into())
        );
        assert_eq!(*active.lock().unwrap(), Some(old.clone()));
        assert_eq!(
            commit_profile_selection(&sequence, 10, &active, Some(next.clone()), || {
                panic!("a superseded selection must never persist")
            }),
            Err("profile_switch_superseded".into())
        );
        assert_eq!(*active.lock().unwrap(), Some(old));
        commit_profile_selection(&sequence, 11, &active, Some(next.clone()), || Ok(())).unwrap();
        assert_eq!(*active.lock().unwrap(), Some(next));
        // The commit does not claim a new lifecycle operation; a later pause
        // or stop remains able to supersede the reconnect.
        assert_eq!(sequence.load(Ordering::SeqCst), 11);
    }

    #[tokio::test]
    async fn switch_waiting_on_the_gate_cannot_mutate_after_a_newer_session_connects() {
        let lifecycle = Arc::new(TokioMutex::new(()));
        let sequence = Arc::new(AtomicU64::new(111));
        let preference_changed = Arc::new(AtomicBool::new(false));
        let controller = Arc::new(Mutex::new(TranslationSessionController::default()));
        let switch_epoch = sequence.load(Ordering::SeqCst);
        let newer_lifecycle = Arc::clone(&lifecycle).lock_owned().await;

        let stale_switch = {
            let lifecycle = Arc::clone(&lifecycle);
            let sequence = Arc::clone(&sequence);
            let preference_changed = Arc::clone(&preference_changed);
            tokio::spawn(async move {
                let _guard = lifecycle.lock_owned().await;
                if lifecycle_sequence_matches(&sequence, switch_epoch) {
                    preference_changed.store(true, Ordering::SeqCst);
                }
            })
        };
        tokio::task::yield_now().await;

        sequence.store(switch_epoch + 1, Ordering::SeqCst);
        controller.lock().unwrap().begin_connecting();
        controller.lock().unwrap().did_connect();
        drop(newer_lifecycle);
        stale_switch.await.unwrap();

        assert!(!preference_changed.load(Ordering::SeqCst));
        assert_eq!(
            controller.lock().unwrap().state.status,
            SessionStatus::Listening
        );
    }

    #[tokio::test]
    async fn pause_claim_after_switch_validation_inherits_the_saved_configuration() {
        let lifecycle = Arc::new(TokioMutex::new(()));
        let sequence = Arc::new(AtomicU64::new(121));
        let operations = Arc::new(AtomicUsize::new(0));
        let owner = Arc::new(AtomicU64::new(121));
        let active_settings = Arc::new(Mutex::new(Some(
            LiveTranslationConfiguration::for_provider(
                crate::core::provider::ProviderKind::AlibabaCloud,
                "test-key",
                SourceLanguage::English,
                crate::core::models::TargetLanguage::SimplifiedChinese,
                TranslationMode::LowLatency,
            ),
        )));

        // The switch has acquired the lifecycle gate and passed both stale
        // intent checks. Pause then claims the next epoch but must wait for
        // the durable preference write to finish.
        let switch_guard = Arc::clone(&lifecycle).lock_owned().await;
        assert!(lifecycle_sequence_matches(&sequence, 121));
        assert!(pipeline_settings_mutation_is_allowed(
            &SessionStatus::Listening,
            operations.load(Ordering::SeqCst),
        ));
        let (pause_claimed_tx, pause_claimed_rx) = tokio::sync::oneshot::channel();
        let pause = {
            let lifecycle = Arc::clone(&lifecycle);
            let sequence = Arc::clone(&sequence);
            let operations = Arc::clone(&operations);
            let active_settings = Arc::clone(&active_settings);
            tokio::spawn(async move {
                operations.fetch_add(1, Ordering::SeqCst);
                sequence.fetch_add(1, Ordering::SeqCst);
                pause_claimed_tx.send(()).unwrap();
                let _pause_guard = lifecycle.lock_owned().await;
                operations.fetch_sub(1, Ordering::SeqCst);
                active_settings.lock().unwrap().clone().unwrap()
            })
        };
        pause_claimed_rx.await.unwrap();
        assert_eq!(sequence.load(Ordering::SeqCst), 122);

        assert!(update_owned_value(
            &active_settings,
            &owner,
            |configuration| {
                configuration.source_language = SourceLanguage::Japanese;
                configuration.target_language = crate::core::models::TargetLanguage::English;
                configuration.translation_mode = TranslationMode::Turbo;
            }
        ));
        drop(switch_guard);

        let resumed_configuration = pause.await.unwrap();
        assert_eq!(
            resumed_configuration.source_language,
            SourceLanguage::Japanese
        );
        assert_eq!(
            resumed_configuration.target_language,
            crate::core::models::TargetLanguage::English
        );
        assert_eq!(
            resumed_configuration.translation_mode,
            TranslationMode::Turbo
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
        ] {
            assert!(!session_error_requires_user_action(error));
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
    async fn queued_profile_mutation_is_rejected_during_resume_failure_handoff() {
        let lifecycle = Arc::new(TokioMutex::new(()));
        let lifecycle_operations = Arc::new(AtomicUsize::new(1));
        let status = Arc::new(Mutex::new(SessionStatus::Error("resume failure".into())));
        let resume_guard = Arc::clone(&lifecycle).lock_owned().await;

        let mutation = {
            let lifecycle = Arc::clone(&lifecycle);
            let lifecycle_operations = Arc::clone(&lifecycle_operations);
            let status = Arc::clone(&status);
            tokio::spawn(async move {
                let _guard = lifecycle.lock_owned().await;
                !lifecycle_activity_is_active(
                    status.lock().unwrap().is_active(),
                    lifecycle_operations.load(Ordering::SeqCst),
                )
            })
        };
        tokio::task::yield_now().await;
        drop(resume_guard);

        assert!(
            !mutation.await.unwrap(),
            "profile mutation must be rejected"
        );
        lifecycle_operations.store(0, Ordering::SeqCst);
        assert!(!lifecycle_activity_is_active(false, 0));
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

    #[test]
    fn automatic_source_preserves_supported_turbo_modes() {
        assert_eq!(
            translation_mode_after_source_switch(
                ProviderKind::OpenAIRealtime,
                SourceLanguage::Automatic,
                TranslationMode::HighQuality,
            ),
            TranslationMode::Turbo
        );
        assert_eq!(
            translation_mode_after_source_switch(
                ProviderKind::VolcanoEngine,
                SourceLanguage::Japanese,
                TranslationMode::HighQuality,
            ),
            TranslationMode::Turbo
        );
        assert_eq!(
            translation_mode_after_source_switch(
                ProviderKind::AlibabaCloud,
                SourceLanguage::Automatic,
                TranslationMode::HighQuality,
            ),
            TranslationMode::Turbo
        );
        assert_eq!(
            translation_mode_after_source_switch(
                ProviderKind::AlibabaCloud,
                SourceLanguage::Automatic,
                TranslationMode::Turbo,
            ),
            TranslationMode::Turbo
        );

        assert!(source_switch_requires_reconnect(
            true,
            SourceLanguage::Automatic,
            crate::core::models::TargetLanguage::SimplifiedChinese,
            TranslationMode::Turbo,
            SourceLanguage::Automatic,
            crate::core::models::TargetLanguage::SimplifiedChinese,
            TranslationMode::LowLatency,
        ));
    }
}
