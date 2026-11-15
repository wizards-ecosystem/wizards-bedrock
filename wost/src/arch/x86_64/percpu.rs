//! Per-processor lock accounting.
//!
//! Every caller goes through [`current`], keeping today's singleton behind the lookup a future
//! multiprocessor implementation must replace.

use core::sync::atomic::{AtomicUsize, Ordering};

/// One processor's own state, in atomics so trap and panic paths observe interrupted updates.
#[derive(Debug)]
pub(crate) struct PerCpu {
    /// Spin locks this CPU is holding.
    spinlocks_held: AtomicUsize,
}

impl PerCpu {
    const fn new() -> Self {
        Self {
            spinlocks_held: AtomicUsize::new(0),
        }
    }

    /// Locks this CPU holds.
    pub(crate) fn spinlocks_held(&self) -> usize {
        self.spinlocks_held.load(Ordering::Relaxed)
    }

    /// Note one more lock taken.
    pub(crate) fn took_lock(&self) {
        self.spinlocks_held.fetch_add(1, Ordering::Relaxed);
    }

    /// Note one lock released.
    pub(crate) fn released_lock(&self) {
        self.spinlocks_held.fetch_sub(1, Ordering::Relaxed);
    }
}

/// The one processor's block.
static BOOTSTRAP: PerCpu = PerCpu::new();

/// This processor's own state.
pub(crate) fn current() -> &'static PerCpu {
    &BOOTSTRAP
}
