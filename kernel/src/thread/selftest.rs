use core::sync::atomic::{AtomicUsize, Ordering};

use super::{State, THREADS};

pub(super) static RETIREMENTS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn free_slots() -> usize {
    let threads = THREADS.lock_leaf();
    threads.queue.slots[1..=threads.spawnable]
        .iter()
        .filter(|slot| matches!(slot.state, State::Free | State::Exited))
        .count()
}

pub(crate) fn retirements() -> usize {
    RETIREMENTS.load(Ordering::Acquire)
}
