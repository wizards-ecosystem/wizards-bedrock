use core::sync::atomic::{AtomicUsize, Ordering};

use super::{State, THREADS};

pub(super) static RETIREMENTS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn free_slots() -> usize {
    let threads = THREADS.lock_leaf();
    threads.queue.slots[1..=threads.spawnable]
        .iter()
        .filter(|slot| slot.state == State::Free)
        .count()
}

pub(crate) fn retirements() -> usize {
    RETIREMENTS.load(Ordering::Acquire)
}
