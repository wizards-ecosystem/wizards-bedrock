use native_abi::witness::behaviour::{
    BACKDOOR, CALLS, CHANNELS, DUPLICATE, ECHO, FAULT, INVALID_RETURN_FLAGS, INVALID_RETURN_STACK,
    MEMORY, PROBE, READ_COUNTER, READ_KERNEL, RING_WAIT, SYSCALL, THREAD_BASE,
};
use wost::arch::current::usercopy;

use world_tree::NodeRef;

use super::{SpawnError, Spawned, spawn, spawn_in};
use crate::channel::Peer;
use crate::thread::ThreadContext;

pub(crate) fn spawn_faulting(context: &mut ThreadContext) -> Result<Spawned, SpawnError> {
    spawn(context, "user-fault", 2, FAULT, 0)
}

pub(crate) fn spawn_for_bootstrap(context: &mut ThreadContext) -> Result<Spawned, SpawnError> {
    spawn(context, "user-bootstrap", 3, ECHO, 0)
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

pub(crate) fn spawn_refusing_memory(context: &mut ThreadContext) -> Result<Spawned, SpawnError> {
    spawn(context, "user-memory", 17, MEMORY, 0)
}

pub(crate) fn spawn_moving_handles(
    context: &mut ThreadContext,
    world: NodeRef,
    birth: Peer,
    role: u64,
) -> Result<Spawned, SpawnError> {
    spawn_in(
        context,
        world,
        Some(birth),
        "user-channels",
        19,
        CHANNELS,
        role,
    )
}

pub(crate) fn spawn_calling(
    context: &mut ThreadContext,
    world: NodeRef,
    birth: Peer,
    role: u64,
) -> Result<Spawned, SpawnError> {
    spawn_in(context, world, Some(birth), "user-calls", 20, CALLS, role)
}

pub(crate) fn spawn_probing_the_backdoor(
    context: &mut ThreadContext,
) -> Result<Spawned, SpawnError> {
    spawn(context, "user-backdoor", 18, BACKDOOR, 0)
}
