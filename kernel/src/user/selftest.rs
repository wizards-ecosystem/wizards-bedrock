use wost::arch::current::usercopy;

use super::{BEHAVE, SpawnError, Spawned, spawn};
use crate::thread::ThreadContext;

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

pub(crate) fn spawn_faulting(context: &mut ThreadContext) -> Result<Spawned, SpawnError> {
    spawn(context, "user-fault", 2, FAULT, 0)
}

pub(crate) fn spawn_for_bootstrap(context: &mut ThreadContext) -> Result<Spawned, SpawnError> {
    spawn(context, "user-bootstrap", 3, BEHAVE, 0)
}

pub(crate) fn spawn_probing(context: &mut ThreadContext) -> Result<Spawned, SpawnError> {
    spawn(context, "user-probe", 4, PROBE, 0)
}

// A negative argument is a kernel address the witness jumps to from ring 3.
pub(crate) fn spawn_into_the_copy_window(
    context: &mut ThreadContext,
) -> Result<Spawned, SpawnError> {
    let (start, _) = usercopy::fault_window();
    spawn(context, "user-window", 5, start, 0)
}

pub(crate) fn spawn_duplicating(context: &mut ThreadContext) -> Result<Spawned, SpawnError> {
    spawn(context, "user-duplicate", 6, DUPLICATE, 0)
}

pub(crate) fn spawn_with_invalid_return_stack(
    context: &mut ThreadContext,
) -> Result<Spawned, SpawnError> {
    spawn(context, "user-invalid-rsp", 10, INVALID_RETURN_STACK, 0)
}

pub(crate) fn spawn_with_invalid_return_flags(
    context: &mut ThreadContext,
) -> Result<Spawned, SpawnError> {
    spawn(context, "user-invalid-flags", 11, INVALID_RETURN_FLAGS, 0)
}

pub(crate) fn spawn_reading_the_counter(
    context: &mut ThreadContext,
) -> Result<Spawned, SpawnError> {
    spawn(context, "user-rdtsc", 12, READ_COUNTER, 0)
}

// The operand is the kernel-half address the witness reads from ring 3.
pub(crate) fn spawn_reading_the_kernel(
    context: &mut ThreadContext,
    address: u64,
) -> Result<Spawned, SpawnError> {
    spawn(context, "user-kernel-read", 13, READ_KERNEL, address)
}

pub(crate) fn spawn_waiting_on_the_ring(
    context: &mut ThreadContext,
) -> Result<Spawned, SpawnError> {
    spawn(context, "user-ring-wait", 14, RING_WAIT, 0)
}

pub(crate) fn spawn_setting_its_thread_base(
    context: &mut ThreadContext,
) -> Result<Spawned, SpawnError> {
    spawn(context, "user-thread-base", 15, THREAD_BASE, 0)
}

pub(crate) fn spawn_executing_syscall(context: &mut ThreadContext) -> Result<Spawned, SpawnError> {
    spawn(context, "user-syscall", 16, SYSCALL, 0)
}
