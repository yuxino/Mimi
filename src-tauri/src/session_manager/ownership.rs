//! Lifecycle request epochs, admission guards and generation-bound operations.
//! These gates prevent stale work from committing into a replacement session.

use super::SessionManager;
use crate::core::audio_input::AudioSource;
use crate::core::models::SessionStatus;
use mimi_runtime::core::recovery::{
    advance_lifecycle_sequence_if_current, lifecycle_sequence_matches, NO_GENERATION,
};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{Mutex as TokioMutex, Notify, OwnedMutexGuard};
use tokio::task::JoinHandle;

pub(super) const SESSION_START_CANCELLED: &str =
    "The session start was superseded by a newer request.";

pub(super) struct LifecycleOperationGuard {
    pub(super) count: Arc<AtomicUsize>,
}

impl Drop for LifecycleOperationGuard {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) struct SessionRequestGuard {
    pub(super) in_progress: Arc<AtomicBool>,
}

impl SessionRequestGuard {
    pub(super) fn acquire(in_progress: &Arc<AtomicBool>) -> Option<Self> {
        try_begin_start(in_progress).then(|| Self {
            in_progress: Arc::clone(in_progress),
        })
    }
}

pub(super) struct PauseRequest {
    pub(super) resume: bool,
    pub(super) _toggle: SessionRequestGuard,
    pub(super) _start: Option<SessionRequestGuard>,
}

pub(super) fn begin_pause_request(
    toggle_in_progress: &Arc<AtomicBool>,
    start_in_progress: &Arc<AtomicBool>,
    is_paused: impl FnOnce() -> bool,
) -> Result<Option<PauseRequest>, &'static str> {
    // Settings and the floating overlay have independent pending controls.
    // Claim before reading direction, so an in-flight Resume cannot turn a
    // second window's duplicate request into Pause (or the reverse).
    let Some(toggle) = SessionRequestGuard::acquire(toggle_in_progress) else {
        return Ok(None);
    };
    let resume = is_paused();
    let start = if resume {
        Some(SessionRequestGuard::acquire(start_in_progress).ok_or("session_resume_busy")?)
    } else {
        // Pause must remain able to invalidate a connecting Start. Stop also
        // bypasses these admission flags and can always claim a newer epoch.
        None
    };
    Ok(Some(PauseRequest {
        resume,
        _toggle: toggle,
        _start: start,
    }))
}

pub(super) struct TeardownOperationGuard {
    pub(super) count: Arc<AtomicUsize>,
    pub(super) notify: Arc<Notify>,
}

impl Drop for TeardownOperationGuard {
    fn drop(&mut self) {
        if self.count.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.notify.notify_waiters();
        }
    }
}

pub(super) fn lifecycle_activity_is_active(
    has_active_session: bool,
    lifecycle_operations: usize,
) -> bool {
    has_active_session || lifecycle_operations > 0
}

pub(super) fn start_request_can_proceed(
    has_active_session: bool,
    is_recovering: bool,
    active_generation: u64,
) -> bool {
    !has_active_session || (is_recovering && active_generation == NO_GENERATION)
}

pub(super) fn start_request_can_proceed_after_teardown(
    has_active_session: bool,
    is_recovering: bool,
    active_generation: u64,
    accepted_recovery_replacement: bool,
) -> bool {
    start_request_can_proceed(has_active_session, is_recovering, active_generation)
        || (accepted_recovery_replacement && active_generation == NO_GENERATION)
}

pub(super) fn pause_transition_is_valid(
    status: &SessionStatus,
    is_paused: bool,
    active_generation: u64,
) -> bool {
    !is_paused && *status == SessionStatus::Listening && active_generation != NO_GENERATION
}

/// A pause accepted while listening may claim its epoch after a source
/// switch/recovery seals the old generation. It must finish that handoff,
/// rather than cancelling the reconnect and leaving Connecting ownerless.
pub(super) fn accepted_pause_transition_is_valid(
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

pub(super) fn resume_transition_is_valid(
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

pub(super) fn try_begin_start(in_progress: &AtomicBool) -> bool {
    in_progress
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

pub(super) fn invalidate_generation_atoms(
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
pub(super) fn invalidate_audio_attempt_atoms(
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

pub(super) fn resume_failure_is_still_owned(
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
pub(super) async fn restore_failed_resume(
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

pub(super) fn clear_task_slot_if_id(
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

pub(super) fn clear_owned_value_if_generation<T>(
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

pub(super) fn update_owned_value<T>(
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

pub(super) async fn lock_after_operations(
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
pub(super) async fn retain_lifecycle_during_teardown(
    lifecycle: OwnedMutexGuard<()>,
    cleanup: impl Future<Output = ()>,
) -> OwnedMutexGuard<()> {
    cleanup.await;
    lifecycle
}

pub(super) async fn wait_until_generation_changes(
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

pub(super) async fn run_generation_bound_operation<T>(
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

impl Drop for SessionRequestGuard {
    fn drop(&mut self) {
        self.in_progress.store(false, Ordering::SeqCst);
    }
}

impl SessionManager {
    pub(super) fn begin_lifecycle_operation(&self) -> LifecycleOperationGuard {
        self.lifecycle_operations.fetch_add(1, Ordering::SeqCst);
        LifecycleOperationGuard {
            count: Arc::clone(&self.lifecycle_operations),
        }
    }

    pub(super) fn begin_teardown_operation(&self) -> TeardownOperationGuard {
        self.teardown_operations.fetch_add(1, Ordering::SeqCst);
        TeardownOperationGuard {
            count: Arc::clone(&self.teardown_operations),
            notify: Arc::clone(&self.teardown_notify),
        }
    }

    pub(super) async fn lock_after_teardown(&self) -> OwnedMutexGuard<()> {
        lock_after_operations(
            Arc::clone(&self.lifecycle_lock),
            Arc::clone(&self.teardown_operations),
            Arc::clone(&self.teardown_notify),
        )
        .await
    }

    pub(super) fn next_lifecycle_request(&self) -> u64 {
        let _transition = self.generation_transition.lock().unwrap();
        self.recovery_continuity.lock().unwrap().clear();
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

    pub(super) fn advance_lifecycle_request_if_current(&self, expected: u64) -> Option<u64> {
        let _transition = self.generation_transition.lock().unwrap();
        let generation = advance_lifecycle_sequence_if_current(&self.lifecycle_sequence, expected)?;
        self.recovery_continuity.lock().unwrap().clear();
        self.lifecycle_notify.notify_waiters();
        Some(generation)
    }

    pub(super) fn next_background_task_id(&self) -> u64 {
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

    pub(super) fn clear_health_task_if_id(&self, task_id: u64) -> bool {
        clear_task_slot_if_id(&self.health_task, &self.health_task_id, task_id)
    }

    pub(super) fn clear_recovery_task_if_id(&self, task_id: u64) -> bool {
        clear_task_slot_if_id(&self.recovery_task, &self.recovery_task_id, task_id)
    }

    pub(super) fn clear_translation_timeout_task_if_id(
        &self,
        source: AudioSource,
        task_id: u64,
    ) -> bool {
        clear_task_slot_if_id(
            &self.lane(source).translation_timeout_task,
            &self.lane(source).translation_timeout_task_id,
            task_id,
        )
    }

    pub(super) fn is_lifecycle_request_current(&self, generation: u64) -> bool {
        lifecycle_sequence_matches(&self.lifecycle_sequence, generation)
    }

    pub(super) fn is_generation_current(&self, generation: u64) -> bool {
        generation != NO_GENERATION && self.active_generation.load(Ordering::SeqCst) == generation
    }

    pub(super) fn ensure_generation_current(&self, generation: u64) -> Result<(), String> {
        if self.is_generation_current(generation) {
            Ok(())
        } else {
            Err(SESSION_START_CANCELLED.into())
        }
    }

    pub(super) async fn run_while_generation_current<T>(
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
    pub(super) fn invalidate_generation(&self, generation: u64) -> bool {
        self.invalidate_generation_with_epoch(generation).is_some()
    }

    pub(super) fn invalidate_generation_with_epoch(&self, generation: u64) -> Option<u64> {
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

    pub(super) fn commit_listening(&self, generation: u64) -> Result<(), String> {
        let _transition = self.generation_transition.lock().unwrap();
        self.ensure_generation_current(generation)?;
        self.controller.lock().unwrap().did_connect();
        Ok(())
    }
}
