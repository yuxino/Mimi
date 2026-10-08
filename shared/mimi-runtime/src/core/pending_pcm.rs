//! Counts PCM work while it is queued or being sent. Every accepted work item
//! owns one guard, so cancellation and normal completion release it identically.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct PendingPcmGate {
    count: Arc<AtomicUsize>,
}

impl PendingPcmGate {
    #[must_use = "Keep the guard alive until the PCM work completes or is cancelled"]
    pub fn acquire(&self) -> PendingPcmGuard {
        self.count.fetch_add(1, Ordering::SeqCst);
        PendingPcmGuard {
            count: Arc::clone(&self.count),
        }
    }

    pub fn has_pending(&self) -> bool {
        self.count.load(Ordering::SeqCst) != 0
    }

    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

/// A guard cannot be cloned: exactly one release corresponds to its acquire.
/// It keeps its original gate alive when a newer generation replaces the gate.
#[must_use = "Keep the guard alive until the PCM work completes or is cancelled"]
pub struct PendingPcmGuard {
    count: Arc<AtomicUsize>,
}

impl Drop for PendingPcmGuard {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloned_gate_observes_the_same_pending_work() {
        let gate = PendingPcmGate::default();
        let clone = gate.clone();
        assert!(!gate.has_pending());
        let guard = clone.acquire();
        assert!(gate.has_pending());
        assert_eq!(gate.pending_count(), 1);
        assert_eq!(clone.pending_count(), 1);
        drop(guard);
        assert!(!gate.has_pending());
        assert!(!clone.has_pending());
    }

    #[test]
    fn each_guard_releases_only_its_own_pending_work() {
        let gate = PendingPcmGate::default();
        let first = gate.acquire();
        let second = gate.acquire();
        let third = gate.clone().acquire();
        assert_eq!(gate.pending_count(), 3);
        drop(second);
        assert_eq!(gate.pending_count(), 2);
        drop(first);
        assert!(gate.has_pending());
        assert_eq!(gate.pending_count(), 1);
        drop(third);
        assert_eq!(gate.pending_count(), 0);
        assert!(!gate.has_pending());
    }

    #[test]
    fn an_old_generation_release_cannot_clear_a_new_generation() {
        let old = PendingPcmGate::default();
        let old_guard = old.acquire();
        let new = PendingPcmGate::default();
        assert!(!new.has_pending());
        let new_guard = new.acquire();
        drop(old_guard);
        assert!(!old.has_pending());
        assert!(new.has_pending());
        assert_eq!(new.pending_count(), 1);
        drop(new_guard);
        assert!(!new.has_pending());
    }

    #[test]
    fn a_guard_keeps_its_counter_alive_after_the_gate_owner_is_dropped() {
        let gate = PendingPcmGate::default();
        let observer = gate.clone();
        let guard = gate.acquire();
        drop(gate);
        assert!(observer.has_pending());
        drop(guard);
        assert_eq!(observer.pending_count(), 0);
    }
}
