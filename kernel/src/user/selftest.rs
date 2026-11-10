use core::sync::atomic::Ordering;

use wost::arch::current::usercopy;

use super::call::{CROSS_GATE, CROSS_MAY_ENTER, GATE_NEXT_CROSS};
use super::{BEHAVE, SpawnError, Spawned, spawn};

/// A second one, in a space of its own, at its own addresses.
#[cfg(feature = "selftest")]
pub(crate) fn run_second() -> Result<Spawned, SpawnError> {
    spawn("user-two", 1, BEHAVE)
}

/// A fifth, which asks for a call this kernel does not serve.
#[cfg(feature = "selftest")]
pub(crate) fn run_probing() -> Result<Spawned, SpawnError> {
    spawn("user-probe", 4, PROBE)
}

/// A program that asks the kernel to return to a non-canonical stack.
#[cfg(feature = "selftest")]
pub(crate) fn run_with_invalid_return_stack() -> Result<Spawned, SpawnError> {
    spawn("user-invalid-rsp", 10, INVALID_RETURN_STACK)
}

/// A program that asks the kernel to return with `RFLAGS.NT` set.
#[cfg(feature = "selftest")]
pub(crate) fn run_with_invalid_return_flags() -> Result<Spawned, SpawnError> {
    spawn("user-invalid-flags", 11, INVALID_RETURN_FLAGS)
}

/// The argument that makes the program ask for an unserved call; must match the behaviour
/// `xtask/src/image/program.rs` emits for it.
#[cfg(feature = "selftest")]
const PROBE: u64 = 2;

/// Must match the hostile return behaviours `xtask/src/image/program.rs` emits.
#[cfg(feature = "selftest")]
const INVALID_RETURN_STACK: u64 = 6;
#[cfg(feature = "selftest")]
const INVALID_RETURN_FLAGS: u64 = 7;

/// A fourth, for the check that counts what a program is given at birth.
#[cfg(feature = "selftest")]
pub(crate) fn run_for_bootstrap() -> Result<Spawned, SpawnError> {
    spawn("user-bootstrap", 3, BEHAVE)
}

/// A child which measures one real Echo call across the ring-3 gate.
#[cfg(feature = "selftest")]
pub(crate) fn run_measuring() -> Result<Spawned, SpawnError> {
    spawn("user-baseline", 8, MEASURE)
}

/// A seventh, which duplicates its bootstrap channel through its own ring.
#[cfg(feature = "selftest")]
pub(crate) fn run_duplicating() -> Result<Spawned, SpawnError> {
    spawn("user-duplicate", 6, DUPLICATE)
}

/// An eighth, which defers a timer and blocks in `RingWait` for its result.
#[cfg(feature = "selftest")]
pub(crate) fn run_waiting() -> Result<Spawned, SpawnError> {
    spawn("user-wait", 7, WAIT)
}

/// An echoing program held just before ring-3 entry for wake orchestration.
#[cfg(feature = "selftest")]
pub(crate) fn run_gated_for_wake() -> Result<Spawned, SpawnError> {
    if !CROSS_GATE.is_idle() || GATE_NEXT_CROSS.load(Ordering::Acquire) {
        return Err(SpawnError::Incomplete);
    }
    CROSS_MAY_ENTER.store(false, Ordering::Release);
    if GATE_NEXT_CROSS.swap(true, Ordering::AcqRel) {
        CROSS_MAY_ENTER.store(true, Ordering::Release);
        return Err(SpawnError::Incomplete);
    }
    let spawned = spawn("user-wake", 9, BEHAVE);
    if spawned.is_err() {
        GATE_NEXT_CROSS.store(false, Ordering::Release);
        CROSS_MAY_ENTER.store(true, Ordering::Release);
    }
    spawned
}

/// Whether the gated child has registered and stopped being runnable.
#[cfg(feature = "selftest")]
pub(crate) fn gated_cross_is_waiting() -> bool {
    !CROSS_GATE.is_idle()
}

/// Let the gated child enter ring 3 and report how many threads were released.
#[cfg(feature = "selftest")]
pub(crate) fn release_gated_cross() -> usize {
    CROSS_MAY_ENTER.store(true, Ordering::Release);
    CROSS_GATE.wake_all()
}

/// A sixth, which jumps to its argument, the kernel's user-copy window, from ring 3: the
/// instruction-fetch fault's `RIP` lies inside the window the recovery arm looks at, the state
/// that arm once accepted without checking who faulted.
#[cfg(feature = "selftest")]
pub(crate) fn run_into_the_copy_window() -> Result<Spawned, SpawnError> {
    let (start, _) = usercopy::fault_window();
    spawn("user-window", 5, start)
}

/// A third that faults on purpose, to show the machine survives it.
#[cfg(feature = "selftest")]
pub(crate) fn run_faulting() -> Result<Spawned, SpawnError> {
    spawn("user-fault", 2, FAULT)
}

/// The argument that makes it execute `ud2` instead.
#[cfg(feature = "selftest")]
const FAULT: u64 = 1;

/// The argument that makes it submit Duplicate for its bootstrap handle.
#[cfg(feature = "selftest")]
const DUPLICATE: u64 = 3;

/// The argument that makes it submit a Timer and enter `RingWait`.
#[cfg(feature = "selftest")]
const WAIT: u64 = 4;

/// The argument that brackets Echo's RingEnter with timestamp reads.
#[cfg(feature = "selftest")]
const MEASURE: u64 = 5;
