//! Owns blocking Windows capture work outside Tokio. The slot remains reserved
//! until native resources have actually dropped, even if start/stop is cancelled.
//! Included on other desktop platforms only for lifecycle tests.

use super::SystemAudioCaptureError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{oneshot, Notify};

/// Bound the caller's wait, not native ownership. A timed-out stop leaves the
/// worker reserved, so the next start cannot overlap its unfinished destruction.
pub(crate) async fn wait_for_previous_capture_release(
    stop: impl std::future::Future<Output = ()>,
    timeout: Duration,
) -> Result<(), SystemAudioCaptureError> {
    tokio::time::timeout(timeout, stop)
        .await
        .map_err(|_| SystemAudioCaptureError::PreviousCaptureStopping)
}

/// Per-worker metadata only. Native streams belong to the `start` closure, so
/// its completion flag cannot precede their destruction.
pub(crate) struct Worker<T> {
    pub(crate) data: T,
    cancelled: AtomicBool,
    finished: AtomicBool,
    completion: Notify,
}

impl<T> Worker<T> {
    pub(crate) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub(crate) fn finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    fn finish(&self) {
        self.finished.store(true, Ordering::SeqCst);
        self.completion.notify_waiters();
    }

    async fn wait_finished(&self) {
        // Create the notification before reading the flag, so a finish
        // between the check and the first poll wakes every registered waiter.
        let completion = self.completion.notified();
        if !self.finished() {
            completion.await;
        }
    }
}

pub(crate) struct WorkerSlot<T> {
    current: Mutex<Option<Arc<Worker<T>>>>,
}

impl<T> Default for WorkerSlot<T> {
    fn default() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }
}

impl<T> Drop for WorkerSlot<T> {
    fn drop(&mut self) {
        if let Some(worker) = self.current.get_mut().unwrap().as_ref() {
            worker.cancel();
        }
    }
}

struct CancelStartOnDrop<T>(Option<Arc<Worker<T>>>);

impl<T> Drop for CancelStartOnDrop<T> {
    fn drop(&mut self) {
        if let Some(worker) = &self.0 {
            worker.cancel();
        }
    }
}

struct FinishWorkerOnDrop<T>(Arc<Worker<T>>);

impl<T> Drop for FinishWorkerOnDrop<T> {
    fn drop(&mut self) {
        self.0.finish();
    }
}

impl<T: Default + Send + Sync + 'static> WorkerSlot<T> {
    pub(crate) fn current(&self) -> Option<Arc<Worker<T>>> {
        self.current.lock().unwrap().clone()
    }

    fn reserve(&self) -> Result<Arc<Worker<T>>, SystemAudioCaptureError> {
        let mut slot = self.current.lock().unwrap();
        if let Some(worker) = slot.as_ref() {
            if !worker.finished() {
                return Err(if worker.cancelled() {
                    SystemAudioCaptureError::PreviousCaptureStopping
                } else {
                    SystemAudioCaptureError::AlreadyRunning
                });
            }
        }
        let worker = Arc::new(Worker {
            data: T::default(),
            cancelled: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            completion: Notify::new(),
        });
        *slot = Some(Arc::clone(&worker));
        Ok(worker)
    }

    /// The closure owns resolve/build/play/rebind/drop and must check cancellation
    /// before starting native work and after any blocking native operation. It
    /// sends readiness once, then keeps its native resources on this thread.
    /// Never capture the slot itself: its final Drop must remain able to cancel.
    pub(crate) async fn start(
        &self,
        run: impl FnOnce(Arc<Worker<T>>, oneshot::Sender<Result<(), SystemAudioCaptureError>>)
            + Send
            + 'static,
    ) -> Result<(), SystemAudioCaptureError> {
        let worker = self.reserve()?;
        let mut cancel_start = CancelStartOnDrop(Some(Arc::clone(&worker)));
        let (ready_tx, ready_rx) = oneshot::channel();
        let thread_worker = Arc::clone(&worker);
        if std::thread::Builder::new()
            .name("mimi-windows-audio".into())
            .spawn(move || {
                // `run` owns all native locals and captured resources. Its
                // return/unwind drops them before this completion is published.
                let _finished = FinishWorkerOnDrop(Arc::clone(&thread_worker));
                run(thread_worker, ready_tx);
            })
            .is_err()
        {
            worker.finish();
            return Err(SystemAudioCaptureError::NativeStartFailed);
        }
        ready_rx
            .await
            .map_err(|_| SystemAudioCaptureError::NativeStartFailed)??;
        if worker.cancelled() {
            return Err(SystemAudioCaptureError::StartCancelled);
        }
        cancel_start.0 = None;
        Ok(())
    }

    /// Cancellation is immediate; native destruction is cooperative and may
    /// remain blocked. Dropping this wait never detaches ownership from the slot.
    pub(crate) async fn stop(&self) {
        if let Some(worker) = self.current() {
            worker.cancel();
            worker.wait_finished().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Condvar;

    #[derive(Default)]
    struct Gate {
        released: Mutex<bool>,
        changed: Condvar,
    }

    impl Gate {
        fn wait(&self) {
            let mut released = self.released.lock().unwrap();
            while !*released {
                released = self.changed.wait(released).unwrap();
            }
        }

        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.changed.notify_all();
        }
    }

    struct ReleaseOnDrop(Arc<Gate>);

    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    struct BlockingDrop {
        entered: Option<oneshot::Sender<()>>,
        release: Arc<Gate>,
    }

    impl Drop for BlockingDrop {
        fn drop(&mut self) {
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
            }
            self.release.wait();
        }
    }

    fn run_until_cancelled<T>(worker: Arc<Worker<T>>) {
        while !worker.cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    async fn await_stopped<T: Default + Send + Sync + 'static>(slot: &WorkerSlot<T>) {
        tokio::time::timeout(Duration::from_secs(2), slot.stop())
            .await
            .expect("released native resources must finish");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completion_wakes_all_stops_only_after_native_drop() {
        let slot = WorkerSlot::<()>::default();
        let release = Arc::new(Gate::default());
        let _release_on_failure = ReleaseOnDrop(Arc::clone(&release));
        let native_release = Arc::clone(&release);
        let (entered_tx, entered_rx) = oneshot::channel();
        slot.start(move |worker, ready| {
            let _native = BlockingDrop {
                entered: Some(entered_tx),
                release: native_release,
            };
            let _ = ready.send(Ok(()));
            run_until_cancelled(worker);
        })
        .await
        .unwrap();

        let mut first = Box::pin(slot.stop());
        let mut second = Box::pin(slot.stop());
        std::future::poll_fn(|cx| {
            use std::future::Future;
            assert!(first.as_mut().poll(cx).is_pending());
            assert!(second.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        tokio::time::timeout(Duration::from_secs(2), entered_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(!slot.current().unwrap().finished());
        release.release();
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(first, second)
        })
        .await
        .expect("both stops must observe native resource release");
        assert!(slot.current().unwrap().finished());
        await_stopped(&slot).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocked_native_drop_allows_stop_timeout_but_prevents_a_new_start() {
        let slot = WorkerSlot::<()>::default();
        let release = Arc::new(Gate::default());
        let _release_on_failure = ReleaseOnDrop(Arc::clone(&release));
        let native_release = Arc::clone(&release);
        let (entered_tx, entered_rx) = oneshot::channel();
        slot.start(move |worker, ready| {
            let _native = BlockingDrop {
                entered: Some(entered_tx),
                release: native_release,
            };
            let _ = ready.send(Ok(()));
            run_until_cancelled(worker);
        })
        .await
        .unwrap();
        let original = slot.current().unwrap();
        assert_eq!(
            slot.start(|_, _| panic!("a second worker must not run"))
                .await,
            Err(SystemAudioCaptureError::AlreadyRunning)
        );
        assert!(tokio::time::timeout(Duration::from_millis(30), slot.stop())
            .await
            .is_err());
        tokio::time::timeout(Duration::from_secs(2), entered_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(original.cancelled());
        assert!(!original.finished());
        assert_eq!(
            slot.start(|_, _| panic!("native drop still owns the slot"))
                .await,
            Err(SystemAudioCaptureError::PreviousCaptureStopping)
        );
        release.release();
        await_stopped(&slot).await;
        assert!(original.finished());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restart_wait_fails_without_releasing_a_blocked_native_owner() {
        let slot = WorkerSlot::<()>::default();
        let release = Arc::new(Gate::default());
        let _release_on_failure = ReleaseOnDrop(Arc::clone(&release));
        let native_release = Arc::clone(&release);
        let (entered_tx, entered_rx) = oneshot::channel();
        slot.start(move |worker, ready| {
            let _native = BlockingDrop {
                entered: Some(entered_tx),
                release: native_release,
            };
            let _ = ready.send(Ok(()));
            run_until_cancelled(worker);
        })
        .await
        .unwrap();
        let original = slot.current().unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(30), slot.stop())
            .await
            .is_err());
        tokio::time::timeout(Duration::from_secs(2), entered_rx)
            .await
            .unwrap()
            .unwrap();

        // AudioCapture::start releases previous backends before starting the
        // selected source. This wait must return an error while native Drop is
        // blocked, rather than waiting forever before WorkerSlot::reserve.
        let restart = async {
            wait_for_previous_capture_release(slot.stop(), Duration::from_millis(30)).await?;
            slot.start(|_, _| panic!("native drop still owns the slot"))
                .await
        };
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), restart)
                .await
                .expect("restart must return the bounded release error"),
            Err(SystemAudioCaptureError::PreviousCaptureStopping)
        );
        assert!(Arc::ptr_eq(&original, &slot.current().unwrap()));
        assert!(original.cancelled());
        assert!(!original.finished());

        release.release();
        wait_for_previous_capture_release(slot.stop(), Duration::from_secs(2))
            .await
            .unwrap();
        assert!(original.finished());
        slot.start(|worker, ready| {
            let _ = ready.send(Ok(()));
            run_until_cancelled(worker);
        })
        .await
        .unwrap();
        assert!(!Arc::ptr_eq(&original, &slot.current().unwrap()));
        await_stopped(&slot).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelling_a_pending_start_keeps_ownership_until_its_worker_releases() {
        let slot = Arc::new(WorkerSlot::<AtomicUsize>::default());
        let release = Arc::new(Gate::default());
        let _release_on_failure = ReleaseOnDrop(Arc::clone(&release));
        let native_release = Arc::clone(&release);
        let (entered_tx, entered_rx) = oneshot::channel();
        let starter = Arc::clone(&slot);
        let starting = tokio::spawn(async move {
            starter
                .start(move |worker, ready| {
                    let _ = entered_tx.send(());
                    native_release.wait();
                    if worker.cancelled() {
                        let _ = ready.send(Err(SystemAudioCaptureError::StartCancelled));
                        return;
                    }
                    worker.data.fetch_add(1, Ordering::SeqCst);
                    let _ = ready.send(Ok(()));
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), entered_rx)
            .await
            .unwrap()
            .unwrap();
        let original = slot.current().unwrap();
        starting.abort();
        assert!(starting.await.unwrap_err().is_cancelled());
        assert!(original.cancelled());
        assert!(!original.finished());
        assert_eq!(
            slot.start(|_, _| panic!("cancelled native build still owns the slot"))
                .await,
            Err(SystemAudioCaptureError::PreviousCaptureStopping)
        );
        release.release();
        await_stopped(&slot).await;
        assert_eq!(original.data.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failed_start_must_finish_native_cleanup_before_restart() {
        let slot = WorkerSlot::<()>::default();
        let release = Arc::new(Gate::default());
        let _release_on_failure = ReleaseOnDrop(Arc::clone(&release));
        let native_release = Arc::clone(&release);
        assert_eq!(
            slot.start(move |_, ready| {
                let _native = BlockingDrop {
                    entered: None,
                    release: native_release,
                };
                let _ = ready.send(Err(SystemAudioCaptureError::NativeStartFailed));
            })
            .await,
            Err(SystemAudioCaptureError::NativeStartFailed)
        );
        let failed = slot.current().unwrap();
        assert_eq!(
            slot.start(|_, _| panic!("failed stream has not dropped"))
                .await,
            Err(SystemAudioCaptureError::PreviousCaptureStopping)
        );
        release.release();
        await_stopped(&slot).await;
        slot.start(|worker, ready| {
            let _ = ready.send(Ok(()));
            run_until_cancelled(worker);
        })
        .await
        .unwrap();
        let replacement = slot.current().unwrap();
        assert!(!Arc::ptr_eq(&failed, &replacement));
        failed.cancel();
        assert!(!replacement.cancelled());
        await_stopped(&slot).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn repeated_stop_and_slot_drop_do_not_create_or_join_workers() {
        let slot = WorkerSlot::<()>::default();
        slot.stop().await;
        slot.stop().await;
        assert!(slot.current().is_none());
        slot.start(|worker, ready| {
            let _ = ready.send(Ok(()));
            run_until_cancelled(worker);
        })
        .await
        .unwrap();
        let original = slot.current().unwrap();
        await_stopped(&slot).await;
        slot.stop().await;
        assert!(Arc::ptr_eq(&original, &slot.current().unwrap()));
        assert!(original.finished());

        let release = Arc::new(Gate::default());
        let _release_on_failure = ReleaseOnDrop(Arc::clone(&release));
        let native_release = Arc::clone(&release);
        slot.start(move |worker, ready| {
            let _native = BlockingDrop {
                entered: None,
                release: native_release,
            };
            let _ = ready.send(Ok(()));
            run_until_cancelled(worker);
        })
        .await
        .unwrap();
        let last = slot.current().unwrap();
        drop(slot);
        assert!(last.cancelled());
        assert!(!last.finished());
        release.release();
        tokio::time::timeout(Duration::from_secs(2), last.wait_finished())
            .await
            .unwrap();
        assert!(last.finished());
    }
}
