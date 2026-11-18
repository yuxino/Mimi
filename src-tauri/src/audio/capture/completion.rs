use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

/// One native worker's completion. The owner calls `finish` after releasing
/// the stream. Dropping a wait does not release ownership or consume the signal.
#[derive(Default)]
pub(super) struct CaptureCompletion {
    finished: AtomicBool,
    notification: Notify,
}

impl CaptureCompletion {
    pub(super) fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    pub(super) fn finish(&self) {
        self.finished.store(true, Ordering::SeqCst);
        self.notification.notify_waiters();
    }

    pub(super) async fn wait(&self) {
        // Register interest before checking the flag so a concurrent finish
        // between the check and await is visible to every waiter.
        let notification = self.notification.notified();
        if !self.is_finished() {
            notification.await;
        }
    }
}
