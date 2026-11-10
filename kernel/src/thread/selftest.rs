use core::sync::atomic::{AtomicUsize, Ordering};

use eevdf::BASE_WEIGHT;
use wost::arch::current::paging::Binding;
use wost::arch::current::task::{self, Bound};

use super::{
    Class, IDLE_STARTED, IDLE_STARTING, IDLE_STATE, IDLE_UNSTARTED, Prepared, SpaceSpawnError,
    SpawnError, SpawnPoint, State, THREADS, reschedule_timer, spawn_prepared,
};
use crate::time;

/// Times the policy had no answer and round-robin stood in for it, which should stay zero: a
/// climbing count is the policy disagreeing with its own host tests.
#[cfg(feature = "selftest")]
pub(super) static POLICY_FALLBACKS: AtomicUsize = AtomicUsize::new(0);

/// Parked threads put back on the queue.
#[cfg(feature = "selftest")]
pub(super) static WAKES: AtomicUsize = AtomicUsize::new(0);

/// Threads that have actually left the run queue to wait on a queue.
#[cfg(feature = "selftest")]
pub(super) static BLOCKS: AtomicUsize = AtomicUsize::new(0);

/// Trap-frame waits which actually handed the CPU to another thread.
#[cfg(feature = "selftest")]
pub(super) static TRAP_WAITS: AtomicUsize = AtomicUsize::new(0);

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

/// How often the policy has been stood in for. See [`POLICY_FALLBACKS`].
#[cfg(feature = "selftest")]
pub(crate) fn policy_fallbacks() -> usize {
    POLICY_FALLBACKS.load(Ordering::Relaxed)
}

/// How many parked threads have been put back on the queue.
#[cfg(feature = "selftest")]
pub(crate) fn wakes() -> usize {
    WAKES.load(Ordering::Relaxed)
}

/// How many threads have blocked on a wait queue.
#[cfg(feature = "selftest")]
pub(crate) fn blocks() -> usize {
    BLOCKS.load(Ordering::Relaxed)
}

/// Ring-3 waits which switched through their saved trap frame.
#[cfg(feature = "selftest")]
pub(crate) fn trap_waits() -> usize {
    TRAP_WAITS.load(Ordering::Relaxed)
}

/// How many times the idle thread has had the CPU with nothing to do on it.
#[cfg(feature = "selftest")]
pub(crate) fn idle_halts() -> usize {
    IDLE_HALTS.load(Ordering::Relaxed)
}

/// Whether the idle thread is running.
#[cfg(feature = "selftest")]
pub(crate) fn idle_is_started() -> bool {
    IDLE_STATE.load(Ordering::Acquire) == IDLE_STARTED && THREADS.lock_leaf().queue.idle().is_some()
}

/// How many live slots are classified as idle.
#[cfg(feature = "selftest")]
pub(crate) fn idle_threads() -> usize {
    THREADS
        .lock_leaf()
        .queue
        .slots
        .iter()
        .filter(|slot| slot.class == Class::Idle && slot.state != State::Free)
        .count()
}

/// Release a deliberately burned in-progress reservation after its check.
#[cfg(feature = "selftest")]
pub(crate) fn restore_idle_retry_for_test() -> bool {
    IDLE_STATE
        .compare_exchange(
            IDLE_STARTING,
            IDLE_UNSTARTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

#[cfg(feature = "selftest")]
pub(crate) fn spawn_weighted_for_test(
    name: &'static str,
    entry: fn(),
    weight: u32,
) -> Result<usize, SpawnError> {
    spawn_prepared(
        Prepared {
            name,
            entry,
            weight,
            binding: None,
        },
        |_, bound| {
            if bound.is_none() { Ok(()) } else { Err(bound) }
        },
        |_| false,
    )
    .map_err(|failure| failure.reason)
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

/// Wakes from a handler that armed the timer. See [`HANDLER_ARMS`].
#[cfg(feature = "selftest")]
pub(crate) fn handler_arms() -> usize {
    HANDLER_ARMS.load(Ordering::Relaxed)
}

/// Restore the current scheduler appointment after a reversible clock check.
#[cfg(feature = "selftest")]
pub(crate) fn rearm_current_for_test() {
    let now = time::scheduler_now_ns();
    let event = {
        let threads = THREADS.lock_leaf();
        if threads.spawnable == 0 {
            return;
        }
        threads.queue.next_event(task::current(), now)
    };
    reschedule_timer(event);
}
