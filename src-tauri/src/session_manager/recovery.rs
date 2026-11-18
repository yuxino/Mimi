//! Native failure handoff, connection health and generation-owned recovery.
//! Portable retry/backoff policy remains in mimi-runtime.

use super::lifecycle::session_error_requires_user_action;
use super::ownership::{
    invalidate_audio_attempt_atoms, invalidate_generation_atoms, SESSION_START_CANCELLED,
};
use super::publication::HealthCheckLatency;
use super::recovery_cleanup::{cancel_recovery_task, prepare_recovery_cleanup};
use super::SessionManager;
use crate::audio::send_pipeline::AudioPipelineFailure;
use crate::audio::SystemAudioCaptureFailure;
use crate::clients::transport::provider_events::ProviderEvent;
use crate::core::audio::audio_input::AudioSource;
use crate::core::diagnostics::support_diagnostics::{RecoveryAction, SafeFailure};
use crate::core::protocols::realtime::live_translate::LiveTranslateServerEvent;
use crate::core::services::provider::ProviderKind;
use crate::pipeline_log;
use mimi_runtime::core::lifecycle::health::{probe_connection_health, HEALTH_CHECK_INTERVAL};
use mimi_runtime::core::lifecycle::recovery::{
    advance_lifecycle_sequence_if_current, clear_recovery_atoms, RecoveryContinuity, NO_GENERATION,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

/// The caller holds generation_transition, also used by recovery completion.
pub(super) fn claim_generation_recovery_atoms(
    active_generation: &AtomicU64,
    lifecycle_sequence: &AtomicU64,
    is_recovering: &AtomicBool,
    retry_generation: &AtomicU64,
    generation: u64,
) -> Option<bool> {
    invalidate_generation_atoms(active_generation, lifecycle_sequence, generation)?;
    let recovering = is_recovering.load(Ordering::SeqCst);
    if recovering {
        retry_generation.store(generation, Ordering::SeqCst);
    }
    Some(recovering)
}

/// The caller holds generation_transition. Retire the successful task before
/// releasing its recovery flag, so the next failure can queue a new owner.
pub(super) fn commit_recovery_success_atoms(
    active_generation: &AtomicU64,
    lifecycle_sequence: &AtomicU64,
    is_recovering: &AtomicBool,
    retry_generation: &AtomicU64,
    generation: u64,
    retire_task: impl FnOnce() -> bool,
) -> bool {
    if active_generation.load(Ordering::SeqCst) != generation
        || lifecycle_sequence.load(Ordering::SeqCst) != generation
        || !is_recovering.load(Ordering::SeqCst)
        || !retire_task()
    {
        return false;
    }
    clear_recovery_atoms(is_recovering, retry_generation);
    true
}

/// Called under the generation and health-task gates only after every selected
/// source has completed its probe. Stability duration and retry counts remain
/// the shared runtime's policy.
pub(super) fn complete_recovery_health_round(
    continuity: &mut RecoveryContinuity,
    current: (u64, u64, u64),
    probed: (u64, u64),
    sources: (usize, usize),
    now: Instant,
) -> bool {
    let (generation, task_id) = probed;
    if generation == NO_GENERATION
        || task_id == NO_GENERATION
        || current != (generation, generation, task_id)
        || sources.1 == 0
        || sources.0 != sources.1
    {
        return false;
    }
    continuity.health_succeeded(generation, now);
    true
}

/// Recovery may claim only the invalidation immediately following its failed
/// generation. A delayed task cannot adopt a newer manual request's budget.
pub(super) fn advance_recovery_request_atoms(
    active_generation: &AtomicU64,
    lifecycle_sequence: &AtomicU64,
    failed_generation: u64,
) -> Option<u64> {
    if active_generation.load(Ordering::SeqCst) != NO_GENERATION {
        return None;
    }
    let failed_epoch = failed_generation.wrapping_add(1).max(1);
    advance_lifecycle_sequence_if_current(lifecycle_sequence, failed_epoch)
}

impl SessionManager {
    pub(super) fn next_recovery_request(&self, failed_generation: u64) -> Option<u64> {
        let _transition = self.generation_transition.lock().unwrap();
        let generation = advance_recovery_request_atoms(
            &self.active_generation,
            &self.lifecycle_sequence,
            failed_generation,
        )?;
        self.lifecycle_notify.notify_waiters();
        Some(generation)
    }

    pub(super) async fn handle_capture_failure(
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
            None,
            None,
        )
        .await;
    }

    pub(super) async fn handle_audio_transport_failure(
        self: &Arc<Self>,
        generation: u64,
        attempt: u64,
        source: AudioSource,
        failure: AudioPipelineFailure,
        classification: Option<SafeFailure>,
    ) {
        self.handle_recoverable_runtime_failure(
            generation,
            attempt,
            failure.to_string(),
            failure.diagnostic_label(),
            Some(source),
            classification,
        )
        .await;
    }

    pub(super) async fn handle_recoverable_runtime_failure(
        self: &Arc<Self>,
        generation: u64,
        attempt: u64,
        message: String,
        diagnostic_label: &'static str,
        source: Option<AudioSource>,
        classification: Option<SafeFailure>,
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
            // Keep the accepted failing source attached to this generation
            // before a concurrent manual start can replace its diagnostics.
            self.record_source_failure(
                source,
                classification.unwrap_or_else(|| SafeFailure::from_error(diagnostic_label)),
            );
            self.lifecycle_notify.notify_waiters();
            let recovering = self.is_recovering.load(Ordering::SeqCst);
            if recovering {
                self.recovery_retry_generation
                    .store(generation, Ordering::SeqCst);
            }
            recovering
        };
        pipeline_log!("runtime stream failed label={}", diagnostic_label);
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

    pub(super) async fn start_health_checks(self: &Arc<Self>, generation: u64) {
        self.stop_health_checks().await;
        let task_id = self.next_background_task_id();
        let self_arc = self.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(HEALTH_CHECK_INTERVAL).await;
                if !self_arc.check_connection_health(generation, task_id).await {
                    return;
                }
            }
        });
        let mut slot = self.health_task.lock().unwrap();
        self.health_task_id.store(task_id, Ordering::SeqCst);
        *slot = Some(task);
    }

    pub(super) async fn stop_health_checks(&self) {
        let mut slot = self.health_task.lock().unwrap();
        self.health_task_id.store(NO_GENERATION, Ordering::SeqCst);
        if let Some(task) = slot.take() {
            task.abort();
        }
    }

    pub(super) async fn check_connection_health(
        self: &Arc<Self>,
        generation: u64,
        task_id: u64,
    ) -> bool {
        if self.is_paused()
            || self.is_recovering.load(Ordering::SeqCst)
            || !self.is_generation_current(generation)
            || self.health_task_id.load(Ordering::SeqCst) != task_id
        {
            return false;
        }
        let records_network_latency = self
            .active_settings
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|configuration| configuration.provider != ProviderKind::AppleSpeech);
        let mut maximum_latency_ms = 0;
        let sources = self.sources();
        let mut completed_sources = 0;
        for &source in sources {
            let Some(client) = self.client_for_generation(source, generation) else {
                return false;
            };
            match probe_connection_health(&client).await {
                Ok(elapsed) => {
                    let elapsed_ms = elapsed.as_millis().min(u64::MAX as u128) as u64;
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
                    completed_sources += 1;
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
                    if let Some(label) = error.recognition_failure_label() {
                        // The reader, PCM sender and health probe must agree
                        // on permanent versus retryable local failures. Clear
                        // this task's slot before event teardown stops health.
                        self.handle_event(
                            source,
                            generation,
                            ProviderEvent::unsequenced(
                                LiveTranslateServerEvent::Error {
                                    code: label.into(),
                                    message: label.into(),
                                },
                                client.content_revision(),
                            ),
                        )
                        .await;
                        return false;
                    }
                    let invalidated = {
                        let _transition = self.generation_transition.lock().unwrap();
                        if invalidate_generation_atoms(
                            &self.active_generation,
                            &self.lifecycle_sequence,
                            generation,
                        )
                        .is_some()
                        {
                            // Record the trigger before recovery can succeed or
                            // a new generation can replace this observation.
                            self.record_source_failure(
                                Some(source),
                                SafeFailure::from_error(&error.to_string()),
                            );
                            self.lifecycle_notify.notify_waiters();
                            true
                        } else {
                            false
                        }
                    };
                    if invalidated {
                        self.queue_recovery(generation, error.to_string()).await;
                    }
                    return false;
                }
            }
        }
        {
            let _transition = self.generation_transition.lock().unwrap();
            let _health_task = self.health_task.lock().unwrap();
            if !complete_recovery_health_round(
                &mut self.recovery_continuity.lock().unwrap(),
                (
                    self.active_generation.load(Ordering::SeqCst),
                    self.lifecycle_sequence.load(Ordering::SeqCst),
                    self.health_task_id.load(Ordering::SeqCst),
                ),
                (generation, task_id),
                (completed_sources, sources.len()),
                Instant::now(),
            ) {
                return false;
            }
        }
        self.publish_state();
        true
    }

    pub(super) async fn queue_recovery(
        self: &Arc<Self>,
        failed_generation: u64,
        failure_message: String,
    ) {
        self.queue_recovery_with_delay(failed_generation, failure_message, Duration::ZERO)
            .await;
    }

    pub(super) async fn queue_recovery_with_delay(
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
                .recover_connection(task_id, failed_generation, failure_message, minimum_delay)
                .await;
            self_arc.clear_recovery_task_if_id(task_id);
        });
        *slot = Some(task);
    }

    pub(super) async fn recover_connection(
        self: &Arc<Self>,
        task_id: u64,
        failed_generation: u64,
        failure_message: String,
        minimum_delay: Duration,
    ) {
        if self.is_paused() || self.is_recovering.load(Ordering::SeqCst) {
            return;
        }
        let _operation = self.begin_lifecycle_operation();
        let Some(mut recovery_generation) = self.next_recovery_request(failed_generation) else {
            return;
        };
        let lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        let mut schedule = {
            let _transition = self.generation_transition.lock().unwrap();
            if !self.is_lifecycle_request_current(recovery_generation) {
                return;
            }
            // A retained budget may already be exhausted, so this owner must
            // also own settings when it terminates without a new connect.
            self.retag_active_settings(recovery_generation);
            self.recovery_continuity.lock().unwrap().take_for_failure(
                failed_generation,
                recovery_generation,
                minimum_delay,
            )
        };
        self.record_recovery_if_current(RecoveryAction::Retrying, recovery_generation);
        pipeline_log!("session recovery started");
        prepare_recovery_cleanup(
            lifecycle,
            &self.is_recovering,
            &self.recovery_retry_generation,
            || self.cleanup_generation(failed_generation),
            || async {
                self.stop_health_checks().await;
                self.controller.lock().unwrap().begin_connecting();
                self.publish_state();
            },
        )
        .await;

        let mut terminal_failure = None;
        while let Some(retry) = schedule.next_attempt() {
            let attempt = retry.index;
            let delay = retry.delay;
            pipeline_log!(
                "session recovery attempt={} delayMs={}",
                retry.budget_index + 1,
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
                let _transition = self.generation_transition.lock().unwrap();
                let Some(next_generation) = schedule.advance_for_retry(&self.lifecycle_sequence)
                else {
                    clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
                    return;
                };
                recovery_generation = next_generation;
                self.lifecycle_notify.notify_waiters();
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
            let error = match self.establish_session(recovery_generation).await {
                Ok(()) => {
                    let _transition = self.generation_transition.lock().unwrap();
                    if commit_recovery_success_atoms(
                        &self.active_generation,
                        &self.lifecycle_sequence,
                        &self.is_recovering,
                        &self.recovery_retry_generation,
                        recovery_generation,
                        || self.clear_recovery_task_if_id(task_id),
                    ) {
                        self.recovery_continuity
                            .lock()
                            .unwrap()
                            .record_connected(schedule, Instant::now());
                        self.record_recovery_action(RecoveryAction::Recovered);
                        return;
                    }
                    // A source can fail after establish's last validity check
                    // but before this success claim. Keep its retry handoff.
                    SESSION_START_CANCELLED.to_string()
                }
                Err(error) => error,
            };
            if error == SESSION_START_CANCELLED {
                let retry_accepted = {
                    let _transition = self.generation_transition.lock().unwrap();
                    let retry_generation = self
                        .recovery_retry_generation
                        .swap(NO_GENERATION, Ordering::SeqCst);
                    let current_epoch = self.lifecycle_sequence.load(Ordering::SeqCst);
                    self.is_recovering.load(Ordering::SeqCst)
                        && schedule.accept_cancelled_attempt(
                            retry_generation,
                            current_epoch,
                            self.active_settings.lock().unwrap().is_some(),
                        )
                };
                if retry_accepted {
                    continue;
                }
                clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
                return;
            }
            if !schedule.accept_failure(self.lifecycle_sequence.load(Ordering::SeqCst)) {
                clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
                return;
            }
            if session_error_requires_user_action(&error) {
                terminal_failure = Some(error);
                break;
            }
        }

        let _lifecycle = Arc::clone(&self.lifecycle_lock).lock_owned().await;
        if !schedule.owns_exhaustion(
            self.lifecycle_sequence.load(Ordering::SeqCst),
            self.active_generation.load(Ordering::SeqCst),
        ) {
            clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
            return;
        }
        if terminal_failure.is_none() {
            self.record_recovery_if_current(RecoveryAction::RetriesExhausted, schedule.epoch());
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
    }

    pub(super) fn take_recovery_for_cancellation(&self) -> Option<JoinHandle<()>> {
        let mut slot = self.recovery_task.lock().unwrap();
        self.recovery_task_id.store(NO_GENERATION, Ordering::SeqCst);
        clear_recovery_atoms(&self.is_recovering, &self.recovery_retry_generation);
        slot.take()
    }

    pub(super) async fn cancel_recovery(&self) {
        let task = self.take_recovery_for_cancellation();
        cancel_recovery_task(task, &self.cleanup_owners).await;
    }
}
