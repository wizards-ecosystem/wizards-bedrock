use core::sync::atomic::{AtomicUsize, Ordering};

// Atomics, so trap and panic paths observe interrupted updates.
#[derive(Debug)]
pub(crate) struct PerCpu {
    spinlocks_held: AtomicUsize,
}

impl PerCpu {
    const fn new() -> Self {
        Self {
            spinlocks_held: AtomicUsize::new(0),
        }
    }

    pub(crate) fn spinlocks_held(&self) -> usize {
        self.spinlocks_held.load(Ordering::Relaxed)
    }

    pub(crate) fn took_lock(&self) {
        self.spinlocks_held.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn released_lock(&self) {
        self.spinlocks_held.fetch_sub(1, Ordering::Relaxed);
    }
}

static BOOTSTRAP: PerCpu = PerCpu::new();

pub(crate) fn current() -> &'static PerCpu {
    &BOOTSTRAP
}
