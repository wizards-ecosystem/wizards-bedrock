use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use boot_record::{Exit, Fault, Record};
use native_abi::witness::{self, AGREED, behaviour};
use world_tree::Status;
use wost::arch::current::interrupt;
use wost::arch::current::usercopy;
use wost::mm;
use x86_tables::exception;

use super::runs_until;
use super::threads::{STOP_SPINNING, light_spinner, yields_until};
use crate::arch::current::trap;
use crate::process::Rites;
use crate::thread::ThreadContext;
use crate::{thread, time, user};

pub(super) fn an_unserved_call_is_refused_without_stopping_the_machine(
    context: &mut ThreadContext,
) -> bool {
    let (refused_before, _) = user::refusals();
    let faults_before = trap::last_user_fault().count;
    let Ok(spawned) = user::spawn_probing(context) else {
        return false;
    };
    let exited = last_rites_for(&spawned, 2_000).is_some_and(|rites| agreed(&rites));
    let (refused, number) = user::refusals();

    exited
        && refused == refused_before + 1
        && number as u64 == witness::UNSERVED_CALL
        && trap::last_user_fault().count == faults_before
}

pub(super) fn a_faulting_program_dies_alone(context: &mut ThreadContext) -> bool {
    let before = trap::last_user_fault();
    let Ok(spawned) = user::spawn_faulting(context) else {
        return false;
    };

    if !runs_until(2000, || {
        let fault = trap::last_user_fault();
        fault.count > before.count && fault.slot == spawned.slot
    }) {
        return false;
    }
    let fault = trap::last_user_fault();

    // `rax` still holds the FAULT argument (1), which survives the dispatch to the `ud2`.
    let this_child = fault.slot == spawned.slot && fault.rax == behaviour::FAULT;
    let it_was_the_ud2 = fault.vector == u64::from(exception::INVALID_OPCODE);
    let in_user_memory = fault.stack_pointer < usercopy::USER_END;
    let retired = last_rites_for(&spawned, 2_000).is_some();
    let observed = observed_once(&spawned, Status::Faulted(exception::INVALID_OPCODE));

    this_child && it_was_the_ud2 && in_user_memory && retired && observed
}

// `rax` still holds the argument only if the trapped `rdtsc` never retired.
pub(super) fn a_direct_clock_read_retires_its_program(context: &mut ThreadContext) -> bool {
    retired_by_protection_fault(
        context,
        user::spawn_reading_the_counter,
        behaviour::READ_COUNTER,
    )
}

// At IOPL 0 with no I/O bitmap the `in` faults; a hypervisor that served it would replace `rax`.
pub(super) fn a_program_cannot_reach_the_hypervisor_backdoor(context: &mut ThreadContext) -> bool {
    retired_by_protection_fault(
        context,
        user::spawn_probing_the_backdoor,
        witness::BACKDOOR_MAGIC,
    )
}

fn retired_by_protection_fault(
    context: &mut ThreadContext,
    spawn: fn(&mut ThreadContext) -> Result<user::Spawned, user::SpawnError>,
    marker: u64,
) -> bool {
    let before = trap::last_user_fault();
    let Ok(spawned) = spawn(context) else {
        return false;
    };

    if !runs_until(2000, || {
        let fault = trap::last_user_fault();
        fault.count > before.count && fault.slot == spawned.slot
    }) {
        return false;
    }
    let fault = trap::last_user_fault();

    let this_child = fault.slot == spawned.slot && fault.rax == marker;
    let trapped = fault.vector == u64::from(exception::GENERAL_PROTECTION);
    let retired = last_rites_for(&spawned, 2_000).is_some();

    this_child && trapped && retired
}

// A refusal is process-local: the timer keeps arriving and a clean successor still crosses.
pub(super) fn a_noncanonical_user_return_stack_retires_only_its_program(
    context: &mut ThreadContext,
) -> bool {
    const NESTED_TASK: u64 = 1 << 14;

    let expiries = time::expiries();
    let stack_refused =
        invalid_return_is_refused(context, user::spawn_with_invalid_return_stack, |frame| {
            frame.stack_pointer == usercopy::USER_END
        });
    let flags_refused =
        invalid_return_is_refused(context, user::spawn_with_invalid_return_flags, |frame| {
            frame.flags & NESTED_TASK != 0
        });

    let calls = trap::last_syscall();
    let Ok(successor) = user::spawn_for_bootstrap(context) else {
        return false;
    };
    let successor_retired = last_rites_for(&successor, 2_000).is_some();
    let successor_call = trap::last_syscall();
    let successor_ran = successor_call.count > calls.count && successor_call.slot == successor.slot;

    stack_refused
        && flags_refused
        && successor_retired
        && successor_ran
        && time::expiries() > expiries
}

fn invalid_return_is_refused(
    context: &mut ThreadContext,
    spawn: fn(&mut ThreadContext) -> Result<user::Spawned, user::SpawnError>,
    names_the_fault: impl Fn(&trap::Record) -> bool,
) -> bool {
    let before = trap::last_invalid_user_return();
    let Ok(spawned) = spawn(context) else {
        return false;
    };
    if !runs_until(2_000, || {
        let refused = trap::last_invalid_user_return();
        refused.count > before.count && refused.slot == spawned.slot
    }) {
        return false;
    }
    names_the_fault(&trap::last_invalid_user_return()) && last_rites_for(&spawned, 2_000).is_some()
}

// A ring-3 fetch fault in the copy window retires the program; `rax` holds where it jumped.
pub(super) fn a_program_cannot_aim_at_the_copy_window(context: &mut ThreadContext) -> bool {
    let recoveries_before = trap::user_copy_recoveries();
    let before = trap::last_user_fault();

    let Ok(spawned) = user::spawn_into_the_copy_window(context) else {
        return false;
    };
    if !runs_until(2000, || {
        let fault = trap::last_user_fault();
        fault.count > before.count && fault.slot == spawned.slot
    }) {
        return false;
    }

    let fault = trap::last_user_fault();
    let (start, end) = usercopy::fault_window();

    let this_child = fault.slot == spawned.slot && fault.rax == start;
    let page_fault = fault.vector == u64::from(exception::PAGE_FAULT);
    let retired = last_rites_for(&spawned, 2_000).is_some();
    let not_recovered = trap::user_copy_recoveries() == recoveries_before;
    let real_window = start < end;

    this_child && page_fault && retired && not_recovered && real_window
}

// The witness's ending once it retired, or `None` if it never did within `ms`.
fn ending_of(spawned: &user::Spawned, ms: u64) -> Option<Record> {
    last_rites_for(spawned, ms)?.ending
}

// The fault was raised in ring 3 code, not on the address the program reached for.
fn in_user_code(rip: u64) -> bool {
    rip < usercopy::USER_END
}

// Present, user, read: the page is there and only its supervisor bit refuses ring 3.
const PRESENT_USER_READ: u64 = 0b101;

// The entry enclave's text is present in every user root, shared or paired, as supervisor pages.
pub(super) fn a_program_cannot_read_the_kernel(context: &mut ThreadContext) -> bool {
    let address = wost::boot::sections().entry_enclave.text.start;
    let Ok(spawned) = user::spawn_reading_the_kernel(context, address) else {
        return false;
    };
    matches!(
        ending_of(&spawned, 2_000),
        Some(Record::Fault(Fault { vector, error, address: faulted, rip }))
            if vector == u64::from(exception::PAGE_FAULT)
                && error == PRESENT_USER_READ
                && faulted == address
                && in_user_code(rip)
    )
}

// `syscall` is a closed door: it faults where it stands and the kernel is never entered.
pub(super) fn a_syscall_instruction_never_enters_the_kernel(context: &mut ThreadContext) -> bool {
    let calls_before = trap::last_syscall().count;
    let faults_before = trap::last_user_fault().count;
    let Ok(spawned) = user::spawn_executing_syscall(context) else {
        return false;
    };
    let ended = matches!(
        ending_of(&spawned, 2_000),
        Some(Record::Fault(Fault { vector, rip, .. }))
            if vector == u64::from(exception::INVALID_OPCODE) && in_user_code(rip)
    );
    let fault = trap::last_user_fault();

    ended
        && fault.count > faults_before
        && fault.slot == spawned.slot
        && fault.rax == behaviour::SYSCALL
        && trap::last_syscall().count == calls_before
}

// Idle, the wait returns at once; with a timer in flight, only that timer's completion wakes it.
pub(super) fn ring_wait_sleeps_only_while_work_is_in_flight(context: &mut ThreadContext) -> bool {
    let Ok(spawned) = user::spawn_waiting_on_the_ring(context) else {
        return false;
    };
    last_rites_for(&spawned, 2_000).is_some_and(|rites| agreed(&rites) && rites.served == 1)
}

// Every memory refusal names its rule, and a seal leaves exactly one handle without `WRITE`.
pub(super) fn memory_refusals_name_their_rule(context: &mut ThreadContext) -> bool {
    let Ok(spawned) = user::spawn_refusing_memory(context) else {
        return false;
    };
    last_rites_for(&spawned, 2_000)
        .is_some_and(|rites| agreed(&rites) && rites.served == 16 && rites.handles == 5)
}

// A user base is read back through `fs`, and a kernel-half or noncanonical one is refused.
pub(super) fn a_thread_base_is_taken_only_inside_user_memory(context: &mut ThreadContext) -> bool {
    let Ok(spawned) = user::spawn_setting_its_thread_base(context) else {
        return false;
    };
    last_rites_for(&spawned, 2_000).is_some_and(|rites| agreed(&rites))
}

const SEGMENT_FLAGS: usize = elf64::HEADER_BYTES + 4;
const SEGMENT_VADDR: usize = elf64::HEADER_BYTES + 16;
const SEGMENT_ALIGNMENT: usize = elf64::HEADER_BYTES + 48;
const ELF_ENTRY: usize = 24;

fn doctored(original: &[u8], at: usize, value: u64) -> Vec<u8> {
    let mut copy = original.to_vec();
    copy[at..at + 8].copy_from_slice(&value.to_le_bytes());
    copy
}

// Byte alignment keeps the moved address congruent with its offset, so only the address is wrong.
fn relocated(original: &[u8], address: u64) -> Vec<u8> {
    let mut copy = doctored(original, SEGMENT_VADDR, address);
    copy[SEGMENT_ALIGNMENT..SEGMENT_ALIGNMENT + 8].copy_from_slice(&1u64.to_le_bytes());
    copy
}

fn refusal(bytes: &[u8]) -> Option<user::LoadError> {
    user::plan_image(bytes).err()
}

// Code on the topmost user page could trap with a non-canonical return address.
pub(super) fn the_loader_refuses_what_it_may_not_map(_: &mut ThreadContext) -> bool {
    let Some(original) = user::image() else {
        return false;
    };
    let mut write_execute = original.to_vec();
    write_execute[SEGMENT_FLAGS..SEGMENT_FLAGS + 4].copy_from_slice(&0b111u32.to_le_bytes());

    let write_execute_refused = matches!(
        refusal(&write_execute),
        Some(user::LoadError::WriteExecute(_))
    );
    let kernel_half_refused = matches!(
        refusal(&relocated(original, 0xffff_8000_0000_0000)),
        Some(user::LoadError::NotUserMemory(_))
    );
    let topmost_refused = matches!(
        refusal(&relocated(original, x86_paging::USER_MAPPING_END)),
        Some(user::LoadError::NotUserMemory(_))
    );
    let entry_refused = matches!(
        refusal(&doctored(original, ELF_ENTRY, 0)),
        Some(user::LoadError::InvalidEntry(0))
    );

    write_execute_refused && kernel_half_refused && topmost_refused && entry_refused
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct AllocatorSnapshot {
    free_frames: usize,
    heap: mm::heap::Stats,
}

impl AllocatorSnapshot {
    pub(super) fn capture() -> Self {
        let disabled = interrupt::disable();
        let snapshot = Self {
            free_frames: mm::free_frames(),
            heap: mm::heap::stats(),
        };
        disabled.restore();
        snapshot
    }

    fn physical_frames(&self) -> Option<usize> {
        self.free_frames
            .checked_add(self.heap.slab_frames)?
            .checked_add(self.heap.large_frames)
    }

    // Slabs may grow; live allocations and the physical total may not change.
    pub(super) fn has_same_ownership(&self, before: &Self) -> bool {
        self.heap.live == before.heap.live
            && self.heap.slab_frames >= before.heap.slab_frames
            && self
                .physical_frames()
                .is_some_and(|frames| before.physical_frames() == Some(frames))
    }
}

// Forty programs through sixteen slots beside a spinner: the worker must reclaim each one.
pub(super) fn a_dead_program_gives_its_memory_back(context: &mut ThreadContext) -> bool {
    if !runs_until(2000, crate::reaper::is_idle) {
        return false;
    }
    let before = AllocatorSnapshot::capture();
    let free_slots = thread::free_slots();
    STOP_SPINNING.store(0, Ordering::Release);
    if thread::spawn("reap-pressure", light_spinner, 0).is_err() {
        return false;
    }

    let mut ran = 0;
    for _ in 0..40 {
        let retirements = thread::retirements();
        if user::spawn_for_bootstrap(context).is_err()
            || !runs_until(2000, || {
                thread::retirements() > retirements && crate::reaper::is_idle()
            })
        {
            break;
        }
        ran += 1;
    }
    STOP_SPINNING.store(1, Ordering::Release);
    let settled = yields_until(context, || thread::free_slots() == free_slots);

    ran == 40 && settled && AllocatorSnapshot::capture().has_same_ownership(&before)
}

pub(super) fn last_rites_for(spawned: &user::Spawned, ms: u64) -> Option<Rites> {
    if !runs_until(ms, || spawned.observation.rites().is_some()) {
        return None;
    }
    spawned.observation.rites()
}

// Written by the teardown once, and refused for good after that.
fn observed_once(spawned: &user::Spawned, status: Status) -> bool {
    spawned.observation.status() == Some(status) && spawned.observation.recorded() == 1
}

// The witness exits 0 only when every answer it checked agreed.
fn agreed(rites: &Rites) -> bool {
    rites.ending == Some(Record::Exit(Exit { status: AGREED }))
}

pub(super) fn a_program_drives_its_own_ring(context: &mut ThreadContext) -> bool {
    const ACCESS_CHECK: u64 = 1 << 18;

    let faults_before = trap::last_user_fault().count;
    let calls_before = trap::last_syscall().count;

    let Ok(echoing) = user::run(context) else {
        return false;
    };
    let Some(echoed) = last_rites_for(&echoing, 2_000) else {
        return false;
    };
    let call = trap::last_syscall();
    let crossed = call.count > calls_before
        && call.slot == echoing.slot
        && call.stack_pointer < usercopy::USER_END;
    // The witness sets `AC` before its calls; the handler must still have run with SMAP on.
    let smap_held = call.flags & ACCESS_CHECK != 0 && trap::smap_survived_the_last_syscall();

    let Ok(duplicating) = user::spawn_duplicating(context) else {
        return false;
    };
    let Some(duplicated) = last_rites_for(&duplicating, 2_000) else {
        return false;
    };

    crossed
        && smap_held
        && agreed(&echoed)
        && agreed(&duplicated)
        && observed_once(&echoing, Status::Exited(0))
        && echoed.served == 1
        && duplicated.served == 1
        && duplicated.handles == 2
        && trap::last_user_fault().count == faults_before
}
