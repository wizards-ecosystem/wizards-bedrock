use boot_record::Record;
use native_abi::witness::behaviour;
use wost::mm::PhysicalAddress;
use x86_paging::PAGE_SIZE;

use crate::process::{ObservationRef, ProcessRef};
use crate::thread;

mod call;
mod construction;
mod load;
#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
pub(crate) use call::refusals;
pub(crate) use call::{call, report_execution, report_refusals};
pub(crate) use construction::{SpawnError, spawn};
pub(crate) use load::keep;
#[cfg(feature = "selftest")]
pub(crate) use load::{LoadError, image, plan_image};
#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    spawn_duplicating, spawn_executing_syscall, spawn_faulting, spawn_for_bootstrap,
    spawn_into_the_copy_window, spawn_moving_handles, spawn_probing, spawn_probing_the_backdoor,
    spawn_reading_the_counter, spawn_reading_the_kernel, spawn_refusing_memory,
    spawn_setting_its_thread_base, spawn_waiting_on_the_ring, spawn_with_invalid_return_flags,
    spawn_with_invalid_return_stack,
};

#[cfg(feature = "selftest")]
pub(crate) const CODE: u64 = 0x0000_4000_0000_0000;

const STACK_BYTES: u64 = PAGE_SIZE;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Launch {
    entry: u64,
    stack: u64,
    argument: u64,
    operand: u64,
    bootstrap_handle: u64,
    which: u64,
    root: PhysicalAddress,
    ring: u64,
    data: u64,
    time: u64,
}

#[derive(Debug)]
pub(crate) struct Spawned {
    pub(crate) slot: usize,
    #[cfg_attr(
        not(feature = "selftest"),
        expect(
            dead_code,
            reason = "only the self-tests wait on a status the kernel spawned"
        )
    )]
    pub(crate) observation: ObservationRef,
}

// Kept by the program's process for its teardown to turn into the status, once.
pub(crate) fn record_ending(ending: Record) {
    if let Some((_, process)) = thread::current_process().and_then(ProcessRef::hold) {
        process.record_ending(ending);
    }
}

pub(crate) fn run(context: &mut thread::ThreadContext) -> Result<Spawned, SpawnError> {
    spawn(context, "user", 0, behaviour::ECHO, 0)
}
