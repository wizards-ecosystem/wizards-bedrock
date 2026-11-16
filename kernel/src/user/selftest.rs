use wost::arch::current::usercopy;

use super::{BEHAVE, SpawnError, Spawned, spawn};

// Arguments that select a behaviour of the witness `witness/ring3`.
const FAULT: u64 = 1;
const PROBE: u64 = 2;
const DUPLICATE: u64 = 3;
const INVALID_RETURN_STACK: u64 = 6;
const INVALID_RETURN_FLAGS: u64 = 7;
const READ_COUNTER: u64 = 8;
const READ_KERNEL: u64 = 9;
const RING_WAIT: u64 = 10;
const THREAD_BASE: u64 = 11;
const SYSCALL: u64 = 12;

pub(crate) fn spawn_faulting() -> Result<Spawned, SpawnError> {
    spawn("user-fault", 2, FAULT, 0)
}

pub(crate) fn spawn_for_bootstrap() -> Result<Spawned, SpawnError> {
    spawn("user-bootstrap", 3, BEHAVE, 0)
}

pub(crate) fn spawn_probing() -> Result<Spawned, SpawnError> {
    spawn("user-probe", 4, PROBE, 0)
}

// A negative argument is a kernel address the witness jumps to from ring 3.
pub(crate) fn spawn_into_the_copy_window() -> Result<Spawned, SpawnError> {
    let (start, _) = usercopy::fault_window();
    spawn("user-window", 5, start, 0)
}

pub(crate) fn spawn_duplicating() -> Result<Spawned, SpawnError> {
    spawn("user-duplicate", 6, DUPLICATE, 0)
}

pub(crate) fn spawn_with_invalid_return_stack() -> Result<Spawned, SpawnError> {
    spawn("user-invalid-rsp", 10, INVALID_RETURN_STACK, 0)
}

pub(crate) fn spawn_with_invalid_return_flags() -> Result<Spawned, SpawnError> {
    spawn("user-invalid-flags", 11, INVALID_RETURN_FLAGS, 0)
}

pub(crate) fn spawn_reading_the_counter() -> Result<Spawned, SpawnError> {
    spawn("user-rdtsc", 12, READ_COUNTER, 0)
}

// The operand is the kernel-half address the witness reads from ring 3.
pub(crate) fn spawn_reading_the_kernel(address: u64) -> Result<Spawned, SpawnError> {
    spawn("user-kernel-read", 13, READ_KERNEL, address)
}

pub(crate) fn spawn_waiting_on_the_ring() -> Result<Spawned, SpawnError> {
    spawn("user-ring-wait", 14, RING_WAIT, 0)
}

pub(crate) fn spawn_setting_its_thread_base() -> Result<Spawned, SpawnError> {
    spawn("user-thread-base", 15, THREAD_BASE, 0)
}

pub(crate) fn spawn_executing_syscall() -> Result<Spawned, SpawnError> {
    spawn("user-syscall", 16, SYSCALL, 0)
}
