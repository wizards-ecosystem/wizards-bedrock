use wost::arch::current::usercopy;

use super::{BEHAVE, SpawnError, Spawned, spawn};

// Arguments that select a behaviour of the witness `xtask/src/image/program.rs` emits.
const FAULT: u64 = 1;
const PROBE: u64 = 2;
const DUPLICATE: u64 = 3;
const INVALID_RETURN_STACK: u64 = 6;
const INVALID_RETURN_FLAGS: u64 = 7;

pub(crate) fn spawn_faulting() -> Result<Spawned, SpawnError> {
    spawn("user-fault", 2, FAULT)
}

pub(crate) fn spawn_for_bootstrap() -> Result<Spawned, SpawnError> {
    spawn("user-bootstrap", 3, BEHAVE)
}

pub(crate) fn spawn_probing() -> Result<Spawned, SpawnError> {
    spawn("user-probe", 4, PROBE)
}

// A negative argument is a kernel address the witness jumps to from ring 3.
pub(crate) fn spawn_into_the_copy_window() -> Result<Spawned, SpawnError> {
    let (start, _) = usercopy::fault_window();
    spawn("user-window", 5, start)
}

pub(crate) fn spawn_duplicating() -> Result<Spawned, SpawnError> {
    spawn("user-duplicate", 6, DUPLICATE)
}

pub(crate) fn spawn_with_invalid_return_stack() -> Result<Spawned, SpawnError> {
    spawn("user-invalid-rsp", 10, INVALID_RETURN_STACK)
}

pub(crate) fn spawn_with_invalid_return_flags() -> Result<Spawned, SpawnError> {
    spawn("user-invalid-flags", 11, INVALID_RETURN_FLAGS)
}
