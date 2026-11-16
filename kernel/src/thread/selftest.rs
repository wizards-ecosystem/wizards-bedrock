use core::sync::atomic::{AtomicUsize, Ordering};

pub(super) static RETIREMENTS: AtomicUsize = AtomicUsize::new(0);

// Threads made and not yet reaped.
pub(super) static LIVE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn live() -> usize {
    LIVE.load(Ordering::Acquire)
}

pub(crate) fn retirements() -> usize {
    RETIREMENTS.load(Ordering::Acquire)
}
