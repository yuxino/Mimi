//! Recovery admission and cancellation share the same resource owners as Stop.

use super::cleanup_owner::{CleanupOwners, CleanupTicket};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::OwnedMutexGuard;
use tokio::task::JoinHandle;

/// A Start admitted during recovery owns its newer request even if the old
/// recovery clears its flag or begins its next attempt while Start is waiting.
/// Retire that attempt before aborting it: startup can otherwise install a late
/// resource after the cleanup owner has already inspected its slots.
pub(super) fn retire_recovery_for_start<Reserve>(
    transition: &Mutex<()>,
    sequence: &AtomicU64,
    active_generation: &AtomicU64,
    request: u64,
    reserve: Reserve,
) -> Option<CleanupTicket>
where
    Reserve: FnOnce(u64) -> CleanupTicket,
{
    let _transition = transition.lock().unwrap();
    if sequence.load(Ordering::SeqCst) != request {
        return None;
    }
    let retired = active_generation.swap(0, Ordering::SeqCst);
    Some(reserve(retired))
}

pub(super) fn activate_start_if_current(
    transition: &Mutex<()>,
    sequence: &AtomicU64,
    active_generation: &AtomicU64,
    request: u64,
) -> bool {
    let _transition = transition.lock().unwrap();
    if sequence.load(Ordering::SeqCst) != request {
        return false;
    }
    active_generation.store(request, Ordering::SeqCst);
    true
}

/// Reserve the retired generation before a newer Start can observe recovery.
/// Even cancellation during `prepare` must transfer the unpolled ticket to its
/// owner; releasing the lifecycle lock does not release the teardown guard.
pub(super) async fn prepare_recovery_cleanup<Reserve, Prepare, Prepared>(
    lifecycle: OwnedMutexGuard<()>,
    recovering: &AtomicBool,
    retry_generation: &AtomicU64,
    reserve: Reserve,
    prepare: Prepare,
) where
    Reserve: FnOnce() -> CleanupTicket,
    Prepare: FnOnce() -> Prepared,
    Prepared: Future<Output = ()>,
{
    let cleanup = reserve();
    recovering.store(true, Ordering::SeqCst);
    retry_generation.store(0, Ordering::SeqCst);
    prepare().await;
    drop(lifecycle);
    cleanup.await;
}

pub(super) async fn cancel_recovery_task(
    task: Option<JoinHandle<()>>,
    cleanup_owners: &CleanupOwners,
) {
    if let Some(task) = task {
        task.abort();
        // Destruction of the recovery future drops any reserved, unpolled
        // cleanup ticket before we snapshot owners below.
        let _ = task.await;
    }
    // A terminal-event caller may itself own a teardown guard. Wait concrete
    // owners rather than the global counter, which would wait for that caller.
    cleanup_owners.wait_registered().await;
}

#[cfg(test)]
mod tests {
    use super::super::{
        lock_after_operations, start_request_can_proceed, start_request_can_proceed_after_teardown,
        TeardownOperationGuard,
    };
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::{Mutex as TokioMutex, Notify};

    fn teardown_guard(count: &Arc<AtomicUsize>, notify: &Arc<Notify>) -> TeardownOperationGuard {
        count.fetch_add(1, Ordering::SeqCst);
        TeardownOperationGuard {
            count: Arc::clone(count),
            notify: Arc::clone(notify),
        }
    }

    async fn reached(notify: &Notify) {
        tokio::time::timeout(Duration::from_secs(1), notify.notified())
            .await
            .expect("the expected lifecycle boundary was not reached");
    }

    async fn finished<T>(task: JoinHandle<T>) -> T {
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("the lifecycle operation did not finish")
            .unwrap()
    }

    #[tokio::test]
    async fn cancellation_before_cleanup_poll_keeps_start_behind_the_registered_owner() {
        let owners = CleanupOwners::default();
        let lifecycle = Arc::new(TokioMutex::new(()));
        let count = Arc::new(AtomicUsize::new(0));
        let notify = Arc::new(Notify::new());
        let recovering = Arc::new(AtomicBool::new(false));
        let prepared = Arc::new(Notify::new());
        let cleaning = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let disconnected = Arc::new(AtomicBool::new(false));
        let recovery = tokio::spawn({
            let owners = owners.clone();
            let lifecycle = Arc::clone(&lifecycle);
            let count = Arc::clone(&count);
            let notify = Arc::clone(&notify);
            let recovering = Arc::clone(&recovering);
            let prepared = Arc::clone(&prepared);
            let cleaning = Arc::clone(&cleaning);
            let release = Arc::clone(&release);
            let disconnected = Arc::clone(&disconnected);
            async move {
                let lock = Arc::clone(&lifecycle).lock_owned().await;
                prepare_recovery_cleanup(
                    lock,
                    &recovering,
                    &AtomicU64::new(7),
                    || {
                        assert!(!recovering.load(Ordering::SeqCst));
                        assert!(lifecycle.try_lock().is_err());
                        owners.register(
                            71,
                            teardown_guard(&count, &notify),
                            move || async move {
                                cleaning.notify_one();
                                release.notified().await;
                                disconnected.store(true, Ordering::SeqCst);
                            },
                            None,
                        )
                    },
                    || async {
                        assert!(recovering.load(Ordering::SeqCst));
                        assert_eq!(count.load(Ordering::SeqCst), 1);
                        prepared.notify_one();
                        std::future::pending().await
                    },
                )
                .await;
            }
        });
        reached(&prepared).await;
        assert!(recovering.load(Ordering::SeqCst));
        let cancellation = tokio::spawn({
            let owners = owners.clone();
            async move { cancel_recovery_task(Some(recovery), &owners).await }
        });
        reached(&cleaning).await;
        let start = tokio::spawn(lock_after_operations(lifecycle, Arc::clone(&count), notify));
        tokio::task::yield_now().await;
        assert!(!start.is_finished());
        assert!(!cancellation.is_finished());
        release.notify_one();
        finished(cancellation).await;
        let _new_lifecycle = finished(start).await;
        assert!(disconnected.load(Ordering::SeqCst));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    async fn cancellation_at_resource_boundary(after_client_take: bool) {
        let owners = CleanupOwners::default();
        let lifecycle = Arc::new(TokioMutex::new(()));
        let count = Arc::new(AtomicUsize::new(0));
        let notify = Arc::new(Notify::new());
        let capture_stopping = Arc::new(Notify::new());
        let capture_stopped = Arc::new(Notify::new());
        let client_taken = Arc::new(Notify::new());
        let disconnect = Arc::new(Notify::new());
        let disconnected = Arc::new(AtomicBool::new(false));
        let pump_aborted = Arc::new(AtomicBool::new(false));
        let client_slot = Arc::new(Mutex::new(Some(Arc::clone(&disconnected))));
        let ticket = owners.register(
            71,
            teardown_guard(&count, &notify),
            {
                let capture_stopping = Arc::clone(&capture_stopping);
                let capture_stopped = Arc::clone(&capture_stopped);
                let client_taken = Arc::clone(&client_taken);
                let disconnect = Arc::clone(&disconnect);
                let client_slot = Arc::clone(&client_slot);
                move || async move {
                    capture_stopping.notify_one();
                    capture_stopped.notified().await;
                    let client = client_slot.lock().unwrap().take().unwrap();
                    client_taken.notify_one();
                    disconnect.notified().await;
                    client.store(true, Ordering::SeqCst);
                }
            },
            Some(Box::new({
                let pump_aborted = Arc::clone(&pump_aborted);
                move || pump_aborted.store(true, Ordering::SeqCst)
            })),
        );
        let recovery = tokio::spawn(ticket);
        reached(&capture_stopping).await;
        if after_client_take {
            capture_stopped.notify_one();
            reached(&client_taken).await;
            assert!(client_slot.lock().unwrap().is_none());
        }
        // Stop owns the lifecycle lock while cancelling recovery. The owner
        // must not need that lock to finish a capture or a detached client.
        let stop = tokio::spawn({
            let owners = owners.clone();
            let lifecycle = Arc::clone(&lifecycle);
            async move {
                let _lifecycle = lifecycle.lock_owned().await;
                cancel_recovery_task(Some(recovery), &owners).await;
            }
        });
        tokio::task::yield_now().await;
        let start = tokio::spawn(lock_after_operations(lifecycle, Arc::clone(&count), notify));
        tokio::task::yield_now().await;
        assert!(!start.is_finished());
        assert!(!stop.is_finished());
        assert!(!disconnected.load(Ordering::SeqCst));
        assert!(!pump_aborted.load(Ordering::SeqCst));
        if !after_client_take {
            capture_stopped.notify_one();
            reached(&client_taken).await;
        }
        disconnect.notify_one();
        finished(stop).await;
        let _new_lifecycle = finished(start).await;
        assert!(disconnected.load(Ordering::SeqCst));
        assert!(pump_aborted.load(Ordering::SeqCst));
        assert_eq!(count.load(Ordering::SeqCst), 0);
        // A new start can now install its client without an old occupied slot.
        assert!(client_slot
            .lock()
            .unwrap()
            .replace(Arc::new(AtomicBool::new(false)))
            .is_none());
    }

    #[tokio::test]
    async fn stop_and_start_wait_for_capture_stop_after_recovery_is_cancelled() {
        cancellation_at_resource_boundary(false).await;
    }

    #[tokio::test]
    async fn stop_and_start_wait_for_a_client_already_taken_by_cancelled_recovery() {
        cancellation_at_resource_boundary(true).await;
    }

    #[tokio::test]
    async fn cancelling_idle_recovery_preserves_resources_for_graceful_stop() {
        let owners = CleanupOwners::default();
        let disconnected = Arc::new(AtomicBool::new(false));
        let healthy_client = Mutex::new(Some(Arc::clone(&disconnected)));
        let healthy_pump = tokio::spawn(std::future::pending::<()>());
        let backoff = tokio::spawn(std::future::pending::<()>());
        cancel_recovery_task(Some(backoff), &owners).await;
        assert!(healthy_client.lock().unwrap().is_some());
        assert!(!healthy_pump.is_finished());
        assert!(!disconnected.load(Ordering::SeqCst));
        // With no registered hard teardown, Stop still owns the client/pump
        // and can request provider finish and drain accepted events itself.
        healthy_pump.abort();
    }

    #[tokio::test]
    async fn cancellation_does_not_wait_for_its_callers_teardown_guard() {
        let owners = CleanupOwners::default();
        let count = Arc::new(AtomicUsize::new(0));
        let notify = Arc::new(Notify::new());
        let caller_guard = teardown_guard(&count, &notify);
        let ticket = owners.register(71, teardown_guard(&count, &notify), || async {}, None);
        tokio::time::timeout(Duration::from_secs(1), cancel_recovery_task(None, &owners))
            .await
            .expect("terminal-event cancellation must not wait for its own guard");
        assert_eq!(count.load(Ordering::SeqCst), 1);
        ticket.await;
        drop(caller_guard);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn accepted_recovery_start_survives_the_old_recovery_clearing_its_flag_during_teardown() {
        let owners = CleanupOwners::default();
        let lifecycle = Arc::new(TokioMutex::new(()));
        let transition = Arc::new(Mutex::new(()));
        let sequence = Arc::new(AtomicU64::new(73));
        let active = Arc::new(AtomicU64::new(0));
        let recovering = Arc::new(AtomicBool::new(true));
        let count = Arc::new(AtomicUsize::new(0));
        let notify = Arc::new(Notify::new());
        let cleaning = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let disconnected = Arc::new(AtomicBool::new(false));
        let prior_cleanup = owners.register(
            71,
            teardown_guard(&count, &notify),
            {
                let cleaning = Arc::clone(&cleaning);
                let release = Arc::clone(&release);
                let disconnected = Arc::clone(&disconnected);
                let recovering = Arc::clone(&recovering);
                move || async move {
                    cleaning.notify_one();
                    release.notified().await;
                    disconnected.store(true, Ordering::SeqCst);
                    recovering.store(false, Ordering::SeqCst);
                }
            },
            None,
        );
        drop(prior_cleanup);
        reached(&cleaning).await;
        let accepted_recovery_replacement = recovering.load(Ordering::SeqCst);
        assert!(start_request_can_proceed(
            true,
            accepted_recovery_replacement,
            active.load(Ordering::SeqCst),
        ));
        let start = tokio::spawn({
            let transition = Arc::clone(&transition);
            let sequence = Arc::clone(&sequence);
            let active = Arc::clone(&active);
            let recovering = Arc::clone(&recovering);
            let disconnected = Arc::clone(&disconnected);
            let count = Arc::clone(&count);
            async move {
                let _lifecycle = lock_after_operations(lifecycle, count, notify).await;
                assert!(disconnected.load(Ordering::SeqCst));
                let cleanup =
                    retire_recovery_for_start(&transition, &sequence, &active, 73, |generation| {
                        assert_eq!(generation, 0);
                        CleanupTicket::completed()
                    })
                    .expect("the accepted Start still owns its request");
                assert_eq!(active.load(Ordering::SeqCst), 0);
                assert!(!start_request_can_proceed(
                    true,
                    recovering.load(Ordering::SeqCst),
                    0,
                ));
                assert!(start_request_can_proceed_after_teardown(
                    true,
                    recovering.load(Ordering::SeqCst),
                    0,
                    accepted_recovery_replacement,
                ));
                cleanup.await;
                activate_start_if_current(&transition, &sequence, &active, 73)
            }
        });
        tokio::task::yield_now().await;
        assert!(!start.is_finished());
        release.notify_one();
        assert!(finished(start).await);
        assert!(!recovering.load(Ordering::SeqCst));
        assert_eq!(active.load(Ordering::SeqCst), 73);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn accepted_start_retires_a_raced_recovery_attempt_before_cancel_or_cleanup_execution() {
        struct ObserveAbort {
            active: Arc<AtomicU64>,
            reserved: Arc<AtomicBool>,
            observed_active: Arc<AtomicU64>,
            observed_reserved: Arc<AtomicBool>,
        }
        impl Drop for ObserveAbort {
            fn drop(&mut self) {
                self.observed_active
                    .store(self.active.load(Ordering::SeqCst), Ordering::SeqCst);
                self.observed_reserved
                    .store(self.reserved.load(Ordering::SeqCst), Ordering::SeqCst);
            }
        }
        let owners = CleanupOwners::default();
        let transition = Arc::new(Mutex::new(()));
        let sequence = Arc::new(AtomicU64::new(73));
        let active = Arc::new(AtomicU64::new(0));
        let count = Arc::new(AtomicUsize::new(0));
        let notify = Arc::new(Notify::new());
        let reserved = Arc::new(AtomicBool::new(false));
        let factory_ran = Arc::new(AtomicBool::new(false));
        let observed_active = Arc::new(AtomicU64::new(u64::MAX));
        let observed_reserved = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(Notify::new());
        let cleaning = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let disconnected = Arc::new(AtomicBool::new(false));
        let client_slot = Arc::new(Mutex::new(Some(81)));
        assert!(start_request_can_proceed(true, true, 0));
        // The old recovery passed its own admission before Start claimed the
        // newer request, then installed its attempt while Start waited.
        active.store(81, Ordering::SeqCst);
        let recovery = tokio::spawn({
            let active = Arc::clone(&active);
            let reserved = Arc::clone(&reserved);
            let observed_active = Arc::clone(&observed_active);
            let observed_reserved = Arc::clone(&observed_reserved);
            let ready = Arc::clone(&ready);
            async move {
                let _observe = ObserveAbort {
                    active,
                    reserved,
                    observed_active,
                    observed_reserved,
                };
                ready.notify_one();
                std::future::pending::<()>().await;
            }
        });
        reached(&ready).await;
        let ticket = retire_recovery_for_start(&transition, &sequence, &active, 73, |generation| {
            assert_eq!(generation, 81);
            assert_eq!(active.load(Ordering::SeqCst), 0);
            reserved.store(true, Ordering::SeqCst);
            owners.register(
                generation,
                teardown_guard(&count, &notify),
                {
                    let active = Arc::clone(&active);
                    let factory_ran = Arc::clone(&factory_ran);
                    let cleaning = Arc::clone(&cleaning);
                    let release = Arc::clone(&release);
                    let client_slot = Arc::clone(&client_slot);
                    let disconnected = Arc::clone(&disconnected);
                    move || {
                        assert_eq!(active.load(Ordering::SeqCst), 0);
                        factory_ran.store(true, Ordering::SeqCst);
                        async move {
                            cleaning.notify_one();
                            release.notified().await;
                            assert_eq!(client_slot.lock().unwrap().take(), Some(81));
                            disconnected.store(true, Ordering::SeqCst);
                        }
                    }
                },
                None,
            )
        })
        .unwrap();
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert!(!factory_ran.load(Ordering::SeqCst));
        assert_eq!(observed_active.load(Ordering::SeqCst), u64::MAX);
        let cancellation = tokio::spawn({
            let owners = owners.clone();
            async move { cancel_recovery_task(Some(recovery), &owners).await }
        });
        reached(&cleaning).await;
        assert_eq!(observed_active.load(Ordering::SeqCst), 0);
        assert!(observed_reserved.load(Ordering::SeqCst));
        assert!(!cancellation.is_finished());
        let start = tokio::spawn({
            let transition = Arc::clone(&transition);
            let sequence = Arc::clone(&sequence);
            let active = Arc::clone(&active);
            let count = Arc::clone(&count);
            let disconnected = Arc::clone(&disconnected);
            async move {
                let _lifecycle =
                    lock_after_operations(Arc::new(TokioMutex::new(())), count, notify).await;
                assert!(disconnected.load(Ordering::SeqCst));
                activate_start_if_current(&transition, &sequence, &active, 73)
            }
        });
        tokio::task::yield_now().await;
        assert!(!start.is_finished());
        assert_eq!(*client_slot.lock().unwrap(), Some(81));
        assert_eq!(active.load(Ordering::SeqCst), 0);
        release.notify_one();
        finished(cancellation).await;
        assert!(finished(start).await);
        ticket.await;
        assert!(client_slot.lock().unwrap().is_none());
        assert_eq!(active.load(Ordering::SeqCst), 73);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn superseded_start_does_not_retire_the_attempt_reserved_for_stop() {
        let transition = Mutex::new(());
        let sequence = AtomicU64::new(73);
        let active = AtomicU64::new(0);
        let reserved = AtomicBool::new(false);
        assert!(start_request_can_proceed(true, true, 0));
        active.store(81, Ordering::SeqCst);
        // Stop has claimed its newer epoch and will take this attempt itself.
        sequence.store(74, Ordering::SeqCst);
        assert!(
            retire_recovery_for_start(&transition, &sequence, &active, 73, |_| {
                reserved.store(true, Ordering::SeqCst);
                CleanupTicket::completed()
            })
            .is_none()
        );
        assert!(!reserved.load(Ordering::SeqCst));
        assert_eq!(active.load(Ordering::SeqCst), 81);
    }

    #[tokio::test]
    async fn stop_claimed_during_cancellation_prevents_the_accepted_start_from_activating() {
        let owners = CleanupOwners::default();
        let lifecycle = Arc::new(TokioMutex::new(()));
        let transition = Arc::new(Mutex::new(()));
        let sequence = Arc::new(AtomicU64::new(73));
        let active = Arc::new(AtomicU64::new(81));
        let cleaning = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let cleaned = Arc::new(AtomicBool::new(false));
        let ticket = retire_recovery_for_start(&transition, &sequence, &active, 73, |generation| {
            owners.register(
                generation,
                (),
                {
                    let cleaning = Arc::clone(&cleaning);
                    let release = Arc::clone(&release);
                    let cleaned = Arc::clone(&cleaned);
                    move || async move {
                        cleaning.notify_one();
                        release.notified().await;
                        cleaned.store(true, Ordering::SeqCst);
                    }
                },
                None,
            )
        })
        .unwrap();
        let recovery = tokio::spawn(std::future::pending::<()>());
        let start = tokio::spawn({
            let transition = Arc::clone(&transition);
            let sequence = Arc::clone(&sequence);
            let active = Arc::clone(&active);
            let cleaned = Arc::clone(&cleaned);
            async move {
                let _lifecycle = lifecycle.lock_owned().await;
                cancel_recovery_task(Some(recovery), &owners).await;
                ticket.await;
                assert!(cleaned.load(Ordering::SeqCst));
                activate_start_if_current(&transition, &sequence, &active, 73)
            }
        });
        reached(&cleaning).await;
        assert!(!start.is_finished());
        {
            let _transition = transition.lock().unwrap();
            sequence.store(74, Ordering::SeqCst);
            active.store(0, Ordering::SeqCst);
        }
        release.notify_one();
        assert!(!finished(start).await);
        assert!(cleaned.load(Ordering::SeqCst));
        assert_eq!(sequence.load(Ordering::SeqCst), 74);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }
}
