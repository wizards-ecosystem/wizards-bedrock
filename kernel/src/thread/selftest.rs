use core::sync::atomic::{AtomicUsize, Ordering};

use eevdf::BASE_WEIGHT;
use wost::arch::current::paging::Binding;
use wost::arch::current::task::Bound;

use super::{Prepared, SpaceSpawnError, SpawnPoint, State, THREADS, spawn_prepared};

/// Times the idle thread has reached its halt: times the machine had nothing to do.
#[cfg(feature = "selftest")]
pub(super) static IDLE_HALTS: AtomicUsize = AtomicUsize::new(0);

/// Slots with no live thread in them.
#[cfg(feature = "selftest")]
pub(crate) fn free_slots() -> usize {
    let threads = THREADS.lock_leaf();
    threads.queue.slots[1..=threads.spawnable]
        .iter()
        .filter(|slot| slot.state == State::Free)
        .count()
}

/// How many times the idle thread has had the CPU with nothing to do on it.
#[cfg(feature = "selftest")]
pub(crate) fn idle_halts() -> usize {
    IDLE_HALTS.load(Ordering::Relaxed)
}

#[cfg(feature = "selftest")]
pub(crate) fn spawn_in_space_refusing(
    name: &'static str,
    entry: fn(),
    binding: Binding,
    point: SpawnPoint,
    arrange: impl FnOnce(usize, Bound) -> Result<(), Bound>,
) -> Result<usize, SpaceSpawnError> {
    spawn_prepared(
        Prepared {
            name,
            entry,
            weight: BASE_WEIGHT,
            binding: Some(binding),
        },
        move |slot, bound| {
            let Some(bound) = bound else {
                return Err(None);
            };
            arrange(slot, bound).map_err(Some)
        },
        move |seen| seen == point,
    )
    .map_err(|failure| SpaceSpawnError {
        reason: failure.reason,
        authority: failure.authority,
    })
}

/// How many threads have been retired from trap context, monotonic so self-tests can observe a
/// retirement even after its slot is reused.
#[cfg(feature = "selftest")]
pub(super) static RETIREMENTS: AtomicUsize = AtomicUsize::new(0);

/// Threads retired from trap context since boot.
#[cfg(feature = "selftest")]
pub(crate) fn retirements() -> usize {
    RETIREMENTS.load(Ordering::Acquire)
}

/// Handler wakeups whose runnable thread needed an immediate timer arm.
#[cfg(feature = "selftest")]
pub(super) static HANDLER_ARMS: AtomicUsize = AtomicUsize::new(0);
