//! Resource teardown outlives the lifecycle task that requested it.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use tokio::runtime::Handle;
use tokio::sync::watch;

pub type CleanupAfter = Box<dyn FnOnce() + Send + 'static>;
type CleanupFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type CleanupFactory = Box<dyn FnOnce() -> CleanupFuture + Send + 'static>;

/// A registry of independent cleanup lifetimes, scoped to its owning adapter.
/// Use distinct nonzero keys for resources that must close independently;
/// registering the same key joins the unfinished cleanup already registered.
#[derive(Clone, Default)]
pub struct CleanupOwners {
    registry: Arc<Registry>,
}

#[derive(Default)]
struct Registry {
    state: Mutex<RegistryState>,
}

#[derive(Default)]
struct RegistryState {
    next_id: u64,
    owners: HashMap<u64, Weak<Owner>>,
}

struct Owner {
    generation: u64,
    id: u64,
    registry: Weak<Registry>,
    runtime: Handle,
    state: Mutex<OwnerState>,
    completed: watch::Sender<bool>,
}

struct OwnerState {
    cleanup: Option<CleanupFactory>,
    guards: Vec<Box<dyn Send>>,
    after: Vec<CleanupAfter>,
    finishing: bool,
}

/// Registering is synchronous. Polling or dropping the ticket starts its owner;
/// dropping a waiter after that cannot cancel resource release.
pub struct CleanupTicket {
    owner: Option<Arc<Owner>>,
    completion: Option<CleanupFuture>,
}

impl CleanupOwners {
    /// Reserve cleanup while inside a Tokio runtime. Zero is a no-op key.
    /// The ticket may subsequently be polled or dropped outside that context.
    pub fn register<G, F, Fut>(
        &self,
        generation: u64,
        guard: G,
        cleanup: F,
        after: Option<CleanupAfter>,
    ) -> CleanupTicket
    where
        G: Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if generation == 0 {
            return CleanupTicket::completed();
        }
        let mut registry = self.registry.state.lock().unwrap();
        if let Some(owner) = registry.owners.get(&generation).and_then(Weak::upgrade) {
            let mut state = owner.state.lock().unwrap();
            if !state.finishing {
                state.guards.push(Box::new(guard));
                if let Some(after) = after {
                    state.after.push(after);
                }
                drop(state);
                drop(registry);
                return CleanupTicket::for_owner(owner);
            }
        }
        registry.next_id = registry.next_id.wrapping_add(1).max(1);
        let owner = Arc::new(Owner {
            generation,
            id: registry.next_id,
            registry: Arc::downgrade(&self.registry),
            runtime: Handle::current(),
            state: Mutex::new(OwnerState {
                cleanup: Some(Box::new(move || Box::pin(cleanup()))),
                guards: vec![Box::new(guard)],
                after: after.into_iter().collect(),
                finishing: false,
            }),
            completed: watch::channel(false).0,
        });
        registry.owners.insert(generation, Arc::downgrade(&owner));
        drop(registry);
        CleanupTicket::for_owner(owner)
    }

    /// Snapshot and start every registered owner before returning the waiter.
    /// This waits for those owners, including pending tickets held elsewhere;
    /// it does not wait for unrelated caller-owned teardown guards.
    pub fn wait_registered(&self) -> impl Future<Output = ()> + Send + 'static {
        let owners: Vec<_> = self
            .registry
            .state
            .lock()
            .unwrap()
            .owners
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        let mut tickets: Vec<_> = owners.into_iter().map(CleanupTicket::for_owner).collect();
        for ticket in &mut tickets {
            ticket.start();
        }
        async move {
            for ticket in tickets {
                ticket.await;
            }
        }
    }
}

impl CleanupTicket {
    pub fn completed() -> Self {
        Self {
            owner: None,
            completion: None,
        }
    }

    fn for_owner(owner: Arc<Owner>) -> Self {
        let mut completed = owner.completed.subscribe();
        Self {
            owner: Some(owner),
            completion: Some(Box::pin(async move {
                while !*completed.borrow_and_update() {
                    if completed.changed().await.is_err() {
                        return;
                    }
                }
            })),
        }
    }

    fn start(&mut self) {
        if let Some(owner) = self.owner.take() {
            owner.start();
        }
    }
}

impl Future for CleanupTicket {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        this.start();
        match this.completion.as_mut() {
            Some(completion) => completion.as_mut().poll(cx),
            None => Poll::Ready(()),
        }
    }
}

impl Drop for CleanupTicket {
    fn drop(&mut self) {
        self.start();
    }
}

impl Owner {
    fn start(self: &Arc<Self>) {
        let cleanup = self.state.lock().unwrap().cleanup.take();
        let Some(cleanup) = cleanup else { return };
        // Construct the completion guard before spawning. It also runs if the
        // runtime drops this task before its first poll or the factory panics.
        let completion = CompleteOwner(Arc::clone(self));
        drop(self.runtime.spawn(async move {
            let _completion = completion;
            cleanup().await;
        }));
    }

    fn finish(&self) {
        let (after, guards) = {
            let mut state = self.state.lock().unwrap();
            state.finishing = true;
            (
                std::mem::take(&mut state.after),
                std::mem::take(&mut state.guards),
            )
        };
        for after in after {
            // A failing callback must not skip other requests or prevent the
            // completion notification, including while unwinding cleanup.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(after));
        }
        for guard in guards {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(guard)));
        }
        self.completed.send_replace(true);
        if let Some(registry) = self.registry.upgrade() {
            let mut registry = registry.state.lock().unwrap();
            if registry
                .owners
                .get(&self.generation)
                .and_then(Weak::upgrade)
                .is_some_and(|owner| owner.id == self.id)
            {
                registry.owners.remove(&self.generation);
            }
        }
    }
}

struct CompleteOwner(Arc<Owner>);

impl Drop for CompleteOwner {
    fn drop(&mut self) {
        self.0.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::oneshot;

    struct CountGuard(Arc<AtomicUsize>);

    impl CountGuard {
        fn new(count: &Arc<AtomicUsize>) -> Self {
            count.fetch_add(1, Ordering::SeqCst);
            Self(Arc::clone(count))
        }
    }

    impl Drop for CountGuard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn dropping_an_unpolled_ticket_starts_its_registered_cleanup() {
        let owners = CleanupOwners::default();
        let count = Arc::new(AtomicUsize::new(0));
        let factory_ran = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let cleanup_factory_ran = Arc::clone(&factory_ran);
        let cleanup_started = Arc::clone(&started);
        let ticket = owners.register(
            7,
            CountGuard::new(&count),
            move || {
                cleanup_factory_ran.store(true, Ordering::SeqCst);
                async move { cleanup_started.store(true, Ordering::SeqCst) }
            },
            None,
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(!factory_ran.load(Ordering::SeqCst));
        assert!(!started.load(Ordering::SeqCst));
        drop(ticket);
        owners.wait_registered().await;
        assert!(factory_ran.load(Ordering::SeqCst));
        assert!(started.load(Ordering::SeqCst));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn aborting_a_waiter_cannot_drop_a_taken_resource_or_its_cleanup_guard() {
        struct Resource(Arc<AtomicBool>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let owners = CleanupOwners::default();
        let count = Arc::new(AtomicUsize::new(0));
        let released = Arc::new(AtomicBool::new(false));
        let slot = Arc::new(Mutex::new(Some(Resource(Arc::clone(&released)))));
        let cleanup_slot = Arc::clone(&slot);
        let (taken_tx, taken_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let ticket = owners.register(
            7,
            CountGuard::new(&count),
            move || async move {
                let resource = cleanup_slot.lock().unwrap().take().unwrap();
                taken_tx.send(()).unwrap();
                release_rx.await.unwrap();
                drop(resource);
            },
            None,
        );
        let waiter = tokio::spawn(ticket);
        taken_rx.await.unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        assert!(slot.lock().unwrap().is_none());
        assert!(!released.load(Ordering::SeqCst));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        release_tx.send(()).unwrap();
        owners.wait_registered().await;
        assert!(released.load(Ordering::SeqCst));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn same_generation_requests_join_one_cleanup_and_retain_all_guards() {
        let owners = CleanupOwners::default();
        let count = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = Arc::clone(&calls);
        let (release_tx, release_rx) = oneshot::channel();
        let first = owners.register(
            7,
            CountGuard::new(&count),
            move || async move {
                first_calls.fetch_add(1, Ordering::SeqCst);
                release_rx.await.unwrap();
            },
            None,
        );
        let second_calls = Arc::clone(&calls);
        let second = owners.register(
            7,
            CountGuard::new(&count),
            move || async move {
                second_calls.fetch_add(1, Ordering::SeqCst);
            },
            None,
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
        release_tx.send(()).unwrap();
        first.await;
        second.await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn preserve_and_abort_requests_run_each_after_callback_after_work() {
        let owners = CleanupOwners::default();
        let count = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(Mutex::new(Vec::new()));
        let work_order = Arc::clone(&order);
        let preserve = owners.register(
            7,
            CountGuard::new(&count),
            move || async move { work_order.lock().unwrap().push(0) },
            None,
        );
        let mut aborts = Vec::new();
        for marker in [1, 2] {
            let after_order = Arc::clone(&order);
            let after_count = Arc::clone(&count);
            aborts.push(owners.register(
                7,
                CountGuard::new(&count),
                || async { panic!("joined request cannot replace the original cleanup") },
                Some(Box::new(move || {
                    assert_eq!(after_count.load(Ordering::SeqCst), 3);
                    after_order.lock().unwrap().push(marker);
                })),
            ));
        }
        preserve.await;
        for abort in aborts {
            abort.await;
        }
        assert_eq!(*order.lock().unwrap(), [0, 1, 2]);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn finishing_owner_cannot_absorb_or_remove_a_new_owner_of_the_same_generation() {
        let owners = CleanupOwners::default();
        let (finishing_tx, finishing_rx) = oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let first = owners.register(
            7,
            (),
            || async {},
            Some(Box::new(move || {
                finishing_tx.send(()).unwrap();
                finish_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            })),
        );
        let first_waiter = tokio::spawn(first);
        finishing_rx.await.unwrap();
        let (new_started_tx, new_started_rx) = oneshot::channel();
        let (new_finish_tx, new_finish_rx) = oneshot::channel();
        let second = owners.register(
            7,
            (),
            move || async move {
                new_started_tx.send(()).unwrap();
                new_finish_rx.await.unwrap();
            },
            None,
        );
        drop(second);
        new_started_rx.await.unwrap();
        finish_tx.send(()).unwrap();
        first_waiter.await.unwrap();
        let mut waiting_for_new = Box::pin(owners.wait_registered());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut waiting_for_new)
                .await
                .is_err()
        );
        new_finish_tx.send(()).unwrap();
        waiting_for_new.await;
    }

    #[tokio::test]
    async fn registered_wait_starts_pending_owners_without_waiting_for_a_callers_global_guard() {
        let owners = CleanupOwners::default();
        let count = Arc::new(AtomicUsize::new(0));
        let caller_guard = CountGuard::new(&count);
        let ticket = owners.register(7, CountGuard::new(&count), || async {}, None);
        assert_eq!(count.load(Ordering::SeqCst), 2);
        tokio::time::timeout(Duration::from_secs(1), owners.wait_registered())
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        ticket.await;
        drop(caller_guard);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn registered_wait_starts_all_generations_before_waiting_for_any_one() {
        let owners = CleanupOwners::default();
        let (first_started_tx, first_started_rx) = oneshot::channel();
        let (second_started_tx, second_started_rx) = oneshot::channel();
        let first = owners.register(
            7,
            (),
            move || async move {
                first_started_tx.send(()).unwrap();
                second_started_rx.await.unwrap();
            },
            None,
        );
        let second = owners.register(
            8,
            (),
            move || async move {
                second_started_tx.send(()).unwrap();
                first_started_rx.await.unwrap();
            },
            None,
        );
        tokio::time::timeout(Duration::from_secs(1), owners.wait_registered())
            .await
            .unwrap();
        first.await;
        second.await;
    }

    #[tokio::test]
    async fn panicking_factory_or_work_releases_guards_and_notifies_waiters() {
        for factory_panics in [true, false] {
            let owners = CleanupOwners::default();
            let count = Arc::new(AtomicUsize::new(0));
            let after_ran = Arc::new(AtomicBool::new(false));
            let after_completed = Arc::clone(&after_ran);
            let ticket = owners.register(
                7,
                CountGuard::new(&count),
                move || -> CleanupFuture {
                    assert!(!factory_panics, "synthetic cleanup factory panic");
                    Box::pin(async { panic!("synthetic cleanup work panic") })
                },
                Some(Box::new(move || {
                    after_completed.store(true, Ordering::SeqCst)
                })),
            );
            tokio::time::timeout(Duration::from_secs(1), ticket)
                .await
                .unwrap();
            assert!(after_ran.load(Ordering::SeqCst));
            assert_eq!(count.load(Ordering::SeqCst), 0);
            owners.wait_registered().await;
        }
    }

    #[tokio::test]
    async fn panicking_after_or_guard_does_not_skip_other_callbacks_or_completion() {
        struct PanickingGuard;
        impl Drop for PanickingGuard {
            fn drop(&mut self) {
                panic!("synthetic cleanup guard panic");
            }
        }
        let owners = CleanupOwners::default();
        let after_ran = Arc::new(AtomicBool::new(false));
        let after_completed = Arc::clone(&after_ran);
        let first = owners.register(
            7,
            PanickingGuard,
            || async {},
            Some(Box::new(|| panic!("synthetic cleanup callback panic"))),
        );
        let second = owners.register(
            7,
            (),
            || async {},
            Some(Box::new(move || {
                after_completed.store(true, Ordering::SeqCst)
            })),
        );
        tokio::time::timeout(Duration::from_secs(1), first)
            .await
            .unwrap();
        second.await;
        assert!(after_ran.load(Ordering::SeqCst));
        owners.wait_registered().await;
    }

    #[tokio::test]
    async fn zero_generation_and_completed_tickets_do_not_run_cleanup_or_callbacks() {
        let owners = CleanupOwners::default();
        CleanupTicket::completed().await;
        owners
            .register(
                0,
                (),
                || async { panic!("generation zero has no retired resources") },
                Some(Box::new(|| panic!("generation zero must not abort a pump"))),
            )
            .await;
        owners.wait_registered().await;
    }
}
