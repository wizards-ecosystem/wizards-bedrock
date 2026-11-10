use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use handle_space::Rights;
use slab_layout::AllocatorOwnership;
use stack_layout::StackLayout;
use wost::arch::current::cpu::Protections;
use wost::arch::current::paging::{self, AddressSpace, BootRegion, MapError, ReclaimError};
use wost::arch::current::usercopy::{self, UserCopyError};
use wost::arch::current::{interrupt, task, xstate};
use wost::boot;
use wost::mm;
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};
use x86_tables::exception;

use super::files::SMALL_NAME;
use super::threads::{context, yields_until};
use super::{runs_until, runs_until_deadline};
use crate::arch::current::trap;
use crate::arch::current::vector;
use crate::handle::{HandleError, HandleTable};
use crate::nvme;
use crate::object;
use crate::queue;
use crate::timepage;
use crate::user;
use crate::vfs;
use crate::{info, mem, thread, time};

pub(super) fn a_thread_crosses_into_ring_three_and_calls_back() -> bool {
    let before = trap::last_syscall();
    let Ok(spawned) = user::run() else {
        return false;
    };

    let retired = last_rites_for(&spawned, 2_000).is_some();
    if !retired {
        return false;
    }
    let call = trap::last_syscall();

    let this_child_called = call.count > before.count && call.slot == spawned.slot;
    let from_user = call.code_selector & 0b11 == 3;
    let through_the_gate = call.vector as u8 == vector::SYSCALL.get();
    let only_gate = interrupt::gate_privilege(vector::SYSCALL.get()) == 3
        && interrupt::gate_privilege(vector::TIMER.get()) == 0
        && interrupt::gate_privilege(vector::KEYBOARD.get()) == 0
        && interrupt::gate_privilege(exception::PAGE_FAULT) == 0;
    let on_a_user_stack =
        call.stack_selector & 0b11 == 3 && call.stack_pointer < usercopy::USER_END;

    this_child_called && from_user && through_the_gate && only_gate && on_a_user_stack
}

pub(super) fn a_program_cannot_bring_smap_down_with_it() -> bool {
    /// `EFLAGS.AC`.
    const AC: u64 = 1 << 18;

    let call = trap::last_syscall();
    if call.count == 0 {
        return false;
    }
    let the_program_tried = call.flags & AC != 0;
    let the_kernel_undid_it = trap::smap_survived_the_last_syscall();

    the_program_tried && the_kernel_undid_it && interrupt::access_clear_is_armed()
}

pub(super) fn a_faulting_program_dies_alone() -> bool {
    let before = trap::last_user_fault();
    let Ok(spawned) = user::run_faulting() else {
        return false;
    };

    if !runs_until(2000, || {
        let fault = trap::last_user_fault();
        fault.count > before.count && fault.slot == spawned.slot
    }) {
        return false;
    }
    let fault = trap::last_user_fault();

    // This process's own fault: its slot, and the FAULT argument (1) it left
    // in `rax`, which survives the dispatch to the `ud2` (D-327).
    let this_child = fault.slot == spawned.slot && fault.rax == 1;
    // From ring 3, by the CPU's own account.
    let from_user = fault.code_selector & 0b11 == 3;
    // The fault the program chose, and no other.
    let it_was_the_ud2 = fault.vector == u64::from(exception::INVALID_OPCODE);
    // Its stack pointer is in user memory, not the kernel's.
    let in_user_memory = fault.stack_pointer < usercopy::USER_END;
    // The exact incarnation reached its last rites, not a global tally.
    let retired = last_rites_for(&spawned, 2_000).is_some();

    this_child && from_user && it_was_the_ud2 && in_user_memory && retired
}

pub(super) fn a_noncanonical_user_return_stack_retires_only_its_program() -> bool {
    let (before, _) = trap::last_invalid_user_return();
    let expiries = time::expiries();
    let Ok(spawned) = user::run_with_invalid_return_stack() else {
        return false;
    };

    if !runs_until(2_000, || {
        let (refused, _) = trap::last_invalid_user_return();
        refused.count > before.count && refused.slot == spawned.slot
    }) {
        return false;
    }
    let (refused, kind) = trap::last_invalid_user_return();
    let this_child = refused.slot == spawned.slot
        && refused.stack_pointer == usercopy::USER_END
        && kind == trap::InvalidUserReturnKind::StackPointer;
    let retired = last_rites_for(&spawned, 2_000).is_some();

    // A refusal is process-local: the timer keeps arriving and a clean child
    // can still cross the same gate, return through the same tail and exit.
    let calls = trap::last_syscall();
    let Ok(successor) = user::run_for_bootstrap() else {
        return false;
    };
    let successor_retired = last_rites_for(&successor, 2_000).is_some();
    let successor_call = trap::last_syscall();
    let successor_ran = successor_call.count > calls.count && successor_call.slot == successor.slot;

    this_child && retired && successor_retired && successor_ran && time::expiries() > expiries
}

pub(super) fn unsafe_user_return_flags_retire_only_their_program() -> bool {
    const NESTED_TASK: u64 = 1 << 14;

    let (before, _) = trap::last_invalid_user_return();
    let expiries = time::expiries();
    let Ok(spawned) = user::run_with_invalid_return_flags() else {
        return false;
    };

    if !runs_until(2_000, || {
        let (refused, _) = trap::last_invalid_user_return();
        refused.count > before.count && refused.slot == spawned.slot
    }) {
        return false;
    }
    let (refused, kind) = trap::last_invalid_user_return();
    let this_child = refused.slot == spawned.slot
        && refused.flags & NESTED_TASK != 0
        && kind == trap::InvalidUserReturnKind::Flags;
    let retired = last_rites_for(&spawned, 2_000).is_some();

    let calls = trap::last_syscall();
    let Ok(successor) = user::run_for_bootstrap() else {
        return false;
    };
    let successor_retired = last_rites_for(&successor, 2_000).is_some();
    let successor_call = trap::last_syscall();
    let successor_ran = successor_call.count > calls.count && successor_call.slot == successor.slot;

    this_child && retired && successor_retired && successor_ran && time::expiries() > expiries
}

pub(super) fn a_program_cannot_aim_at_the_copy_window() -> bool {
    let recoveries_before = trap::user_copy_recoveries();
    let before = trap::last_user_fault();

    let Ok(spawned) = user::run_into_the_copy_window() else {
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

    // This process's own fault: its slot, and the window address it jumped to,
    // still in `rax` when the fetch faulted (D-327).
    let this_child = fault.slot == spawned.slot && fault.rax == start;
    // A page fault on the copy window, the case the arm must refuse.
    let page_fault = fault.vector == u64::from(exception::PAGE_FAULT);
    // From ring 3, which is what makes this the case the arm must refuse.
    let from_user = fault.code_selector & 0b11 == 3;
    // The exact incarnation reached its last rites, rather than being resumed
    // at the fixup.
    let retired = last_rites_for(&spawned, 2_000).is_some();
    // The counter did not move: nothing was recovered.
    let not_recovered = trap::user_copy_recoveries() == recoveries_before;
    // And the window is a real range: an empty `fault_window` would make
    // the check pass with the arm deleted.
    let real_window = start < end;

    this_child && page_fault && from_user && retired && not_recovered && real_window
}

pub(super) fn the_loader_refuses_what_it_may_not_map() -> bool {
    let Some(original) = user::image() else {
        return false;
    };
    /// Offset of the first program header's flags, from the ELF64 layout.
    const FLAGS: usize = elf64::HEADER_BYTES + 4;
    /// The image entry point in the ELF64 header.
    const ENTRY: usize = 24;
    /// Its `p_offset`, `p_filesz` and `p_vaddr`.
    const OFFSET: usize = elf64::HEADER_BYTES + 8;
    const VADDR: usize = elf64::HEADER_BYTES + 16;
    const FILE_SIZE: usize = elf64::HEADER_BYTES + 32;
    const MEMORY_SIZE: usize = elf64::HEADER_BYTES + 40;
    const ALIGNMENT: usize = elf64::HEADER_BYTES + 48;

    let doctor = |at: usize, value: u64| -> Vec<u8> {
        let mut copy = original.to_vec();
        copy[at..at + 8].copy_from_slice(&value.to_le_bytes());
        copy
    };
    let field = |at: usize| {
        let mut value = [0u8; 8];
        value.copy_from_slice(&original[at..at + 8]);
        u64::from_le_bytes(value)
    };
    let load = |bytes: &[u8], shift: u64| {
        let allocator_before = AllocatorSnapshot::capture();
        let Ok(mut space) = AddressSpace::for_user() else {
            return (Err(user::LoadError::OutOfMemory), false, false);
        };
        let held = mm::free_frames();
        let result = user::load_image(&mut space, bytes, shift);
        let refused_before_mapping = mm::free_frames() == held;
        let mappings_cleaned = result.as_ref().map_or(true, |image| {
            image
                .regions()
                .all(|(at, len)| space.unmap(at, len).map(drop).is_ok())
        });
        let tables_cleaned = space.reclaim(None).is_ok();
        drop(space);
        let cleaned = mappings_cleaned
            && tables_cleaned
            && AllocatorSnapshot::capture().has_same_ownership(&allocator_before);
        (result, cleaned, refused_before_mapping)
    };

    // Writable and executable at once.
    let mut write_execute = original.to_vec();
    write_execute[FLAGS..FLAGS + 4].copy_from_slice(&0b111u32.to_le_bytes());
    let wx_refused = matches!(
        load(&write_execute, 0).0,
        Err(user::LoadError::WriteExecute(_))
    );

    // Moving only `p_offset` isolates the contents bound from the independent
    // file-size-versus-memory-size refusal (D-105).
    let overrun = doctor(OFFSET, original.len() as u64);
    let overrun_refused = matches!(load(&overrun, 0).0, Err(user::LoadError::Truncated(_)));

    // More file than memory to put it in, and only that.
    let mut oversized = doctor(FILE_SIZE, 8);
    oversized[MEMORY_SIZE..MEMORY_SIZE + 8].copy_from_slice(&4u64.to_le_bytes());
    let oversized_refused = matches!(load(&oversized, 0).0, Err(user::LoadError::Malformed(_)));

    // Placed in the kernel's half.
    let mut intruding = doctor(VADDR, 0xffff_8000_0000_0000);
    intruding[ALIGNMENT..ALIGNMENT + 8].copy_from_slice(&1u64.to_le_bytes());
    let intruding_refused = matches!(
        load(&intruding, 0).0,
        Err(user::LoadError::NotUserMemory(_))
    );

    // Code there could trap with a non-canonical return address (D-348).
    let mut topmost = doctor(VADDR, x86_paging::USER_MAPPING_END);
    topmost[ALIGNMENT..ALIGNMENT + 8].copy_from_slice(&1u64.to_le_bytes());
    let (topmost_result, topmost_cleaned, topmost_preflight) = load(&topmost, 0);
    let topmost_refused = matches!(topmost_result, Err(user::LoadError::NotUserMemory(_)))
        && topmost_cleaned
        && topmost_preflight;

    // ELF congruence is still valid, but this loader deliberately places only
    // whole-page segments and refuses that narrower contract before mapping.
    let mut unaligned = doctor(VADDR, field(VADDR) + 1);
    unaligned[ALIGNMENT..ALIGNMENT + 8].copy_from_slice(&1u64.to_le_bytes());
    let (unaligned_result, unaligned_cleaned, unaligned_preflight) = load(&unaligned, 0);
    let unaligned_refused = matches!(unaligned_result, Err(user::LoadError::Unaligned(_)))
        && unaligned_cleaned
        && unaligned_preflight;

    let invalid_alignment = doctor(ALIGNMENT, 3);
    let (alignment_result, alignment_cleaned, alignment_preflight) = load(&invalid_alignment, 0);
    let alignment_refused = matches!(
        alignment_result,
        Err(user::LoadError::InvalidAlignment { alignment: 3, .. })
    ) && alignment_cleaned
        && alignment_preflight;

    let mut incongruent = doctor(VADDR, field(VADDR) + 1);
    incongruent[ALIGNMENT..ALIGNMENT + 8].copy_from_slice(&PAGE_SIZE.to_le_bytes());
    let (congruence_result, congruence_cleaned, congruence_preflight) = load(&incongruent, 0);
    let congruence_refused = matches!(
        congruence_result,
        Err(user::LoadError::InvalidAlignment { .. })
    ) && congruence_cleaned
        && congruence_preflight;

    // And something that is not an ELF at all.
    let not_an_elf = [0u8; 64];
    let refused_outright = matches!(load(&not_an_elf, 0).0, Err(user::LoadError::NotAnElf(_)));

    // Validated before the first placement: zero is a valid user address but
    // outside the executable segment, so a late refusal is observable.
    let invalid_entry = doctor(ENTRY, 0);
    let (entry_result, entry_cleaned, entry_preflight) = load(&invalid_entry, 0);
    let entry_refused = matches!(entry_result, Err(user::LoadError::InvalidEntry(0)))
        && entry_cleaned
        && entry_preflight;

    // Each success carries its own immutable placement record. Loading again
    // cannot overwrite the first call's cleanup evidence.
    let (first, first_cleaned, _) = load(original, 0);
    let (second, second_cleaned, _) = load(original, 0x1_0000);
    let independent = match (first, second) {
        (Ok(first), Ok(second)) => {
            let first: Vec<_> = first.regions().collect();
            let second: Vec<_> = second.regions().collect();
            !first.is_empty()
                && first.len() == second.len()
                && first
                    .iter()
                    .zip(&second)
                    .all(|((one, one_len), (two, two_len))| {
                        *two == *one + 0x1_0000 && one_len == two_len
                    })
        }
        _ => false,
    };

    wx_refused
        && overrun_refused
        && oversized_refused
        && intruding_refused
        && topmost_refused
        && unaligned_refused
        && alignment_refused
        && congruence_refused
        && refused_outright
        && entry_refused
        && first_cleaned
        && second_cleaned
        && independent
}

pub(super) fn a_second_thread_crosses_onto_its_own_stack() -> bool {
    let first = trap::last_syscall();
    if first.count == 0 {
        return false;
    }
    let first_landing = first.frame_address;

    if user::run_second().is_err() {
        return false;
    }
    if !runs_until(2000, || trap::last_syscall().count > first.count) {
        return false;
    }
    let second = trap::last_syscall();

    // Ring 3 again, through the same one gate.
    let from_user = second.code_selector & 0b11 == 3;
    let through_the_gate = second.vector as u8 == vector::SYSCALL.get();
    // At its own user addresses, not the first program's.
    let elsewhere =
        second.stack_pointer != first.stack_pointer && second.stack_pointer < usercopy::USER_END;
    // Slot identity is the witness because sequential threads may correctly
    // reuse a stack (D-117).
    let landed_on_its_own = thread::layout()
        .and_then(|layout| layout.slot(second.slot.checked_sub(1)?))
        .is_some_and(|slot| {
            second.frame_address > slot.stack_base() && second.frame_address <= slot.stack_top()
        });
    // Both are kernel stacks, which a shared or stale `RSP0` pointing at user
    // memory would not be.
    let kernel_stacks =
        second.frame_address >= usercopy::USER_END && first_landing >= usercopy::USER_END;

    from_user && through_the_gate && elsewhere && landed_on_its_own && kernel_stacks
}

pub(super) fn the_return_to_ring_three_clears_buffers() -> bool {
    let armed = interrupt::buffer_clear_is_armed();
    let executed = interrupt::buffer_clears();
    if !armed || executed == 0 {
        return false;
    }
    // Once per crossing, not once at boot: every syscall came through this
    // branch, so the tally cannot lag the crossings.
    executed >= trap::last_syscall().count as u64
}

pub(super) fn a_faulting_user_copy_resumes_and_reports() -> bool {
    let recoveries_before = trap::user_copy_recoveries();
    let exact = usercopy::drive_fault_recovery_for_test();
    let recovered = trap::user_copy_recoveries() == recoveries_before + 1;
    exact && recovered
}

pub(super) fn boundary_probe() -> Option<(u64, u64)> {
    /// Well clear of anything a program is loaded at.
    const CODE: u64 = user::CODE + 0x20_0000;
    const STACK: u64 = user::STACK + 0x20_0000;

    static PROBE: SpinLock<Option<(u64, u64)>> = SpinLock::new(None);

    if let Some(pages) = *PROBE.lock_leaf() {
        return Some(pages);
    }

    let mut code = mm::allocate_contiguous(1, 1)?;
    if !code.zero() || !code.write_at(0, &user::PROGRAM) {
        return None;
    }
    let mut stack = mm::allocate_contiguous(1, 1)?;
    if !stack.zero() {
        return None;
    }

    let mut context = context();
    // Read-only and executable, as a program's text is: the write refusal
    // below is about the leaf's own permission and needs a page that has it.
    mem::map_user_for_test(&mut context, CODE, code.into(), Flags::NONE)?.ok()?;
    mem::map_user_for_test(
        &mut context,
        STACK,
        stack.into(),
        Flags::WRITABLE.union(Flags::NO_EXECUTE),
    )?
    .ok()?;

    *PROBE.lock_leaf() = Some((CODE, STACK));
    Some((CODE, STACK))
}

pub(super) fn bytes_cross_the_boundary_only_where_they_may() -> bool {
    let Some((code, stack)) = boundary_probe() else {
        return false;
    };

    let mut read = [0u8; user::PROGRAM.len()];
    if usercopy::copy_from_user(code, &mut read).is_err() {
        return false;
    }
    let program_matches = read == user::PROGRAM;

    let written = [0xA5u8; 8];
    let mut echoed = [0u8; 8];
    let round_trip = usercopy::copy_to_user(stack, &written).is_ok()
        && usercopy::copy_from_user(stack, &mut echoed).is_ok()
        && echoed == written;

    let mut scratch = [0u8; 8];
    // Kernel memory, named as if it were the user's.
    let kernel_refused = matches!(
        usercopy::copy_from_user(boot::sections().text.start, &mut scratch),
        Err(UserCopyError::NotUserMemory(_))
    );
    // A range that starts inside user space and ends outside it.
    let straddling_refused = matches!(
        usercopy::copy_from_user(usercopy::USER_END - 4, &mut scratch),
        Err(UserCopyError::NotUserMemory(_))
    );
    // One that wraps rather than describing a span at all.
    let wrap_refused = matches!(
        usercopy::copy_from_user(u64::MAX - 2, &mut scratch),
        Err(UserCopyError::Wraps(_))
    );
    // User memory that is not there.
    let hole_refused = matches!(
        usercopy::copy_from_user(0x0000_2000_0000_0000, &mut scratch),
        Err(UserCopyError::Unmapped(_))
    );
    // And a write to the program's own page, which ring 3 may execute and
    // neither ring may write.
    let read_only_refused = matches!(
        usercopy::copy_to_user(code, &written),
        Err(UserCopyError::ReadOnly(_))
    );
    let supervisor_refused = walked_copy_refuses_supervisor_leaf();

    program_matches
        && round_trip
        && kernel_refused
        && straddling_refused
        && wrap_refused
        && hole_refused
        && read_only_refused
        && supervisor_refused
}

fn walked_copy_refuses_supervisor_leaf() -> bool {
    const SUPERVISOR: u64 = user::CODE + 0x30_0000;
    const PRIVATE: [u8; 8] = [0x3c; 8];
    const UNCHANGED: [u8; 8] = [0xa5; 8];

    let Some(mut run) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !run.zero() || !run.write_at(0, &PRIVATE) {
        return false;
    }
    let physical = run.address();
    let mut context = context();
    let Some(Ok(mapping)) =
        mem::map_owned_for_test(&mut context, SUPERVISOR, run, Flags::NO_EXECUTE)
    else {
        return false;
    };

    let mut scratch = UNCHANGED;
    let refused = matches!(
        usercopy::copy_from_user(SUPERVISOR, &mut scratch),
        Err(UserCopyError::Unmapped(found)) if found == SUPERVISOR
    ) && scratch == UNCHANGED;
    let cleaned = mem::unmap_for_test(&mut context, mapping).is_some_and(|result| {
        result
            .ok()
            .and_then(|unmapped| unmapped.into_kernel())
            .is_some_and(|segment| {
                let same = segment.address() == physical && segment.frames() == 1;
                drop(segment);
                same
            })
    });

    refused && cleaned
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    fn ownership(&self) -> AllocatorOwnership {
        AllocatorOwnership::new(
            self.free_frames,
            self.heap.slab_frames,
            self.heap.large_frames,
            self.heap.live,
        )
    }

    pub(super) fn accounted_physical_frames(&self) -> Option<usize> {
        self.ownership().accounted_physical_frames()
    }

    pub(super) fn has_same_ownership(&self, other: &Self) -> bool {
        self.ownership().is_preserved_since(other.ownership())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SpawnResources {
    frames: usize,
    heap: mm::heap::Stats,
    arena: (usize, usize),
    channels: usize,
    rings: usize,
    pub(super) thread_slots: usize,
    process_records: (usize, usize, usize),
    mapping_refusals: usize,
    table_refusals: usize,
    task_retirements: usize,
}

impl SpawnResources {
    fn has_same_live_owners(&self, other: &Self) -> bool {
        self.arena == other.arena
            && self.channels == other.channels
            && self.rings == other.rings
            && self.thread_slots == other.thread_slots
            && self.process_records == other.process_records
    }

    fn has_same_refusal_tallies(&self, other: &Self) -> bool {
        self.mapping_refusals == other.mapping_refusals
            && self.table_refusals == other.table_refusals
            && self.task_retirements == other.task_retirements
    }

    fn allocator_snapshot(&self) -> AllocatorSnapshot {
        AllocatorSnapshot {
            free_frames: self.frames,
            heap: self.heap,
        }
    }

    pub(super) fn has_same_ownership(&self, other: &Self) -> bool {
        self.has_same_live_owners(other)
            && self.has_same_refusal_tallies(other)
            && self
                .allocator_snapshot()
                .has_same_ownership(&other.allocator_snapshot())
    }
}

pub(super) fn spawn_resources() -> SpawnResources {
    let disabled = interrupt::disable();
    let allocator = AllocatorSnapshot::capture();
    let resources = SpawnResources {
        frames: allocator.free_frames,
        heap: allocator.heap,
        arena: HandleTable::arena_usage_for_test(),
        channels: object::live_channels_for_test(),
        rings: queue::live_for_test(),
        thread_slots: thread::free_slots(),
        process_records: user::process_records_for_test(),
        mapping_refusals: user::refused_mapping_reclaims(),
        table_refusals: user::refused_table_reclaims(),
        task_retirements: user::refused_task_retirements(),
    };
    disabled.restore();
    resources
}

fn refused_at_spawn_edge(edge: user::SpawnEdge) -> bool {
    match user::spawn_refusing(edge) {
        Err(user::SpawnError::Injected(found)) => found == edge,
        Err(user::SpawnError::NoThread(thread::SpawnError::Full)) => matches!(
            edge,
            user::SpawnEdge::ThreadReserved
                | user::SpawnEdge::ThreadFabricated
                | user::SpawnEdge::ThreadBound
        ),
        Err(user::SpawnError::NoThread(thread::SpawnError::Arrangement)) => {
            edge == user::SpawnEdge::ThreadArrangement
        }
        _ => false,
    }
}

pub(super) fn every_refused_spawn_returns_every_resource() -> bool {
    while user::collect().is_some() {}

    let baseline = spawn_resources();
    for edge in user::SpawnEdge::ALL {
        let (pages_before, tables_before) = user::spawn_rollbacks();
        if !refused_at_spawn_edge(edge) {
            return false;
        }
        let (pages_after, tables_after) = user::spawn_rollbacks();
        let pages_returned = matches!(edge, user::SpawnEdge::Space) || pages_after > pages_before;
        if tables_after <= tables_before
            || !pages_returned
            || !spawn_resources().has_same_ownership(&baseline)
        {
            return false;
        }
    }
    true
}

pub(super) fn a_commit_refusal_reunites_cleanup_authority() -> bool {
    while user::collect().is_some() {}
    let before = spawn_resources();
    let rollback_before = user::spawn_rollbacks();
    let result = user::spawn_refusing(user::SpawnEdge::ThreadArrangement);
    matches!(
        result,
        Err(user::SpawnError::NoThread(thread::SpawnError::Arrangement))
    ) && user::spawn_rollbacks().0 > rollback_before.0
        && user::spawn_rollbacks().1 > rollback_before.1
        && spawn_resources().has_same_ownership(&before)
}

pub(super) fn a_maximum_image_and_stack_fit_the_cleanup_ledger() -> bool {
    user::maximum_cleanup_ledger_is_total_for_test()
}

pub(super) fn cleanup_refusals_retain_reachable_tables() -> bool {
    const FIRST: u64 = 0x0000_4100_0000_0000;
    const SECOND: u64 = FIRST + 2 * PAGE_SIZE;
    const BLOCKER: u64 = FIRST + 4 * PAGE_SIZE;
    const SEGMENTS: usize = 3;
    const PHNUM: usize = 56;

    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let Some(mut blocker) = mm::allocate_contiguous(1, 1) else {
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a missing blocker did not return its address-space tables"
        );
        return false;
    };
    if !blocker.zero() {
        drop(blocker);
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a refused blocker zero did not return setup resources"
        );
        return false;
    }
    if space
        .map_user(
            BLOCKER,
            blocker.into(),
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )
        .is_err()
    {
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a refused blocker map did not return setup resources"
        );
        return false;
    }

    let data = elf64::HEADER_BYTES + SEGMENTS * elf64::PROGRAM_HEADER_BYTES;
    let mut image = alloc::vec![0u8; data + SEGMENTS];
    image[0..4].copy_from_slice(&elf64::MAGIC);
    image[4] = elf64::CLASS_64;
    image[5] = elf64::LITTLE_ENDIAN;
    image[6] = 1;
    image[16..18].copy_from_slice(&2u16.to_le_bytes());
    image[18..20].copy_from_slice(&elf64::MACHINE_X86_64.to_le_bytes());
    image[20..24].copy_from_slice(&1u32.to_le_bytes());
    image[24..32].copy_from_slice(&FIRST.to_le_bytes());
    image[32..40].copy_from_slice(&(elf64::HEADER_BYTES as u64).to_le_bytes());
    image[52..54].copy_from_slice(&(elf64::HEADER_BYTES as u16).to_le_bytes());
    image[54..56].copy_from_slice(&(elf64::PROGRAM_HEADER_BYTES as u16).to_le_bytes());
    image[PHNUM..PHNUM + 2].copy_from_slice(&(SEGMENTS as u16).to_le_bytes());
    for (index, (at, flags)) in [(FIRST, 5u32), (SECOND, 6u32), (BLOCKER, 6u32)]
        .into_iter()
        .enumerate()
    {
        let header = elf64::HEADER_BYTES + index * elf64::PROGRAM_HEADER_BYTES;
        image[header..header + 4].copy_from_slice(&1u32.to_le_bytes());
        image[header + 4..header + 8].copy_from_slice(&flags.to_le_bytes());
        image[header + 8..header + 16].copy_from_slice(&((data + index) as u64).to_le_bytes());
        image[header + 16..header + 24].copy_from_slice(&at.to_le_bytes());
        image[header + 32..header + 40].copy_from_slice(&1u64.to_le_bytes());
        image[header + 40..header + 48].copy_from_slice(&PAGE_SIZE.to_le_bytes());
    }

    user::refuse_cleanup_unmap_after_for_test(1);
    let load_result = user::load_image(&mut space, &image, 0);
    user::restore_cleanup_unmap_for_test();
    let typed = matches!(
        load_result,
        Err(user::LoadError::Rollback {
            reason: MapError::Pinned(SECOND),
            pages: 1,
        })
    );
    let retained = space.lookup(FIRST).is_none()
        && space.lookup(SECOND).is_some()
        && space.lookup(BLOCKER).is_some()
        && matches!(
            space.reclaim(None),
            Err(paging::ReclaimFailure {
                reason: ReclaimError::Mapped(2),
                ..
            })
        )
        && !space.is_reclaimed();
    let mappings_cleaned = [SECOND, BLOCKER]
        .into_iter()
        .all(|at| space.unmap(at, PAGE_SIZE).map(drop).is_ok());
    let tables_cleaned = space.reclaim(None).is_ok();
    drop(space);
    drop(image);
    let cleaned = mappings_cleaned
        && tables_cleaned
        && AllocatorSnapshot::capture().has_same_ownership(&allocator_before);

    let construction_before = spawn_resources();
    user::refuse_cleanup_unmap_after_for_test(0);
    let spawn_result = user::spawn_refusing(user::SpawnEdge::Image);
    user::restore_cleanup_unmap_for_test();
    let construction_after = spawn_resources();
    let expected_construction = SpawnResources {
        mapping_refusals: construction_before.mapping_refusals + 1,
        ..construction_before
    };
    let construction_refused = matches!(
        spawn_result,
        Err(user::SpawnError::Injected(user::SpawnEdge::Image))
    ) && construction_after.has_same_ownership(&expected_construction);

    typed && retained && cleaned && construction_refused
}

pub(super) fn partial_stack_rollback_is_retryable() -> bool {
    const BASE: u64 = 0x0000_4200_0000_0000;
    const STACK_BYTES: u64 = 2 * PAGE_SIZE;
    const SLOTS: usize = 3;

    let Some(layout) = StackLayout::new(BASE, STACK_BYTES, PAGE_SIZE, SLOTS) else {
        return false;
    };
    let (Some(first), Some(second), Some(third)) = (layout.slot(0), layout.slot(1), layout.slot(2))
    else {
        return false;
    };
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    paging::refuse_stack_mapping_after_for_test(2);
    paging::refuse_stack_rollback_after_for_test(1);
    let result = space.map_stacks(&layout);
    paging::restore_stack_refusals_for_test();

    let cleanup = match result {
        Err(paging::StackMapError::Rollback {
            cause: MapError::OutOfMemory,
            cleanup,
        }) => cleanup,
        _ => return false,
    };
    let typed = cleanup.reason == MapError::Pinned(first.stack_base())
        && cleanup.mapped_stacks() == 2
        && cleanup.returned_pages == 2
        && cleanup.remaining_stacks() == 1;
    let retained = space.lookup(first.stack_base()).is_some()
        && space.lookup(second.stack_base()).is_none()
        && space.lookup(third.stack_base()).is_none()
        && matches!(
            space.reclaim(None),
            Err(paging::ReclaimFailure {
                reason: ReclaimError::Mapped(2),
                ..
            })
        )
        && !space.is_reclaimed();
    let resumed = space.resume_stack_rollback(cleanup);
    let reclaimed = space.reclaim(None).is_ok();
    drop(space);
    let cleaned = matches!(resumed, Ok(4))
        && reclaimed
        && AllocatorSnapshot::capture().has_same_ownership(&allocator_before);

    typed && retained && cleaned
}

pub(super) fn guard_rollback_refusal_retains_the_stack() -> bool {
    const BASE: u64 = 0x0000_4300_0000_0000;
    const STACK_BYTES: u64 = 2 * PAGE_SIZE;

    let Some(layout) = StackLayout::new(BASE, STACK_BYTES, PAGE_SIZE, 1) else {
        return false;
    };
    let Some(physical) = mm::first_memory_run(PAGE_SIZE) else {
        return false;
    };
    let Some(slot) = layout.slot(0) else {
        return false;
    };
    let prepare = |space: &mut AddressSpace| {
        let Some(guard) = BootRegion::describe(physical, PAGE_SIZE) else {
            return false;
        };
        space.map_boot(BASE, guard, Flags::NO_EXECUTE).is_ok()
    };
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    if !prepare(&mut space) {
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a refused guard setup did not return its tables"
        );
        return false;
    }
    paging::refuse_stack_rollback_after_for_test(0);
    let result = space.map_stack(&slot);
    paging::restore_stack_refusals_for_test();

    let cleanup = match result {
        Err(paging::StackMapError::Rollback { cause, cleanup })
            if cause == MapError::Conflict(BASE) =>
        {
            cleanup
        }
        _ => return false,
    };
    let typed = cleanup.reason == MapError::Pinned(slot.stack_base())
        && cleanup.mapped_stacks() == 1
        && cleanup.returned_pages == 0
        && cleanup.remaining_stacks() == 1;
    let retained = space.lookup(slot.stack_base()).is_some()
        && matches!(
            space.reclaim(None),
            Err(paging::ReclaimFailure {
                reason: ReclaimError::Mapped(2),
                ..
            })
        )
        && !space.is_reclaimed();
    let resumed = space.resume_stack_rollback(cleanup);
    let reclaimed = space.reclaim(None).is_ok();
    drop(space);
    let cleaned = matches!(resumed, Ok(2))
        && reclaimed
        && AllocatorSnapshot::capture().has_same_ownership(&allocator_before);

    typed && retained && cleaned
}

pub(super) fn a_retirement_refusal_is_settled_before_slot_reuse() -> bool {
    let resources = spawn_resources();
    task::refuse_next_retirement_for_test();
    let result = user::spawn_refusing(user::SpawnEdge::ThreadFabricated);
    task::restore_retirement_for_test();
    let slot = match result {
        Err(user::SpawnError::NoThread(thread::SpawnError::Retirement {
            slot,
            reason: task::TaskError::Current(found),
        })) if slot == found => slot,
        _ => return false,
    };
    let expected = SpawnResources {
        task_retirements: resources.task_retirements + 1,
        ..resources
    };
    !task::slot_is_live_for_test(slot) && spawn_resources().has_same_ownership(&expected)
}

pub(super) fn cleanup_unexpected_spawn(spawned: user::Spawned) -> bool {
    let slot = spawned.slot;
    let retired = runs_until(2_000, || !task::slot_is_live_for_test(slot));
    while user::collect().is_some() {}
    retired && !user::has_zombies()
}

pub(super) fn an_unguarded_boot_stack_closes_the_user_boundary() -> bool {
    while user::collect().is_some() {}
    if !crate::user_launch_is_safe() {
        return false;
    }

    let resources_before = spawn_resources();
    let crossings_before = trap::last_syscall().count;
    let attempt = crate::with_unguarded_boot_stack_for_test(user::run);
    let denied = matches!(&attempt, Err(user::SpawnError::UnguardedBootStack));

    let cleaned = match attempt {
        Ok(spawned) => cleanup_unexpected_spawn(spawned),
        Err(_) => true,
    };

    denied
        && cleaned
        && crate::user_launch_is_safe()
        && trap::last_syscall().count == crossings_before
        && spawn_resources() == resources_before
}

pub(super) fn unadmitted_interrupts_close_the_user_boundary() -> bool {
    while user::collect().is_some() {}
    if !crate::maskable_interrupts_are_ready() {
        return false;
    }

    let resources_before = spawn_resources();
    let crossings_before = trap::last_syscall().count;
    let attempt = crate::without_maskable_interrupt_admission_for_test(user::run);
    let construction_denied = matches!(&attempt, Err(user::SpawnError::NoInterruptAdmission));
    let construction_cleaned = match attempt {
        Ok(spawned) => cleanup_unexpected_spawn(spawned),
        Err(_) => true,
    };
    if !construction_denied
        || !construction_cleaned
        || trap::last_syscall().count != crossings_before
        || !spawn_resources().has_same_ownership(&resources_before)
    {
        return false;
    }

    let Ok(spawned) = user::run_gated_for_wake() else {
        return false;
    };
    let slot = spawned.slot;
    let waiting = runs_until(500, user::gated_cross_is_waiting);
    let late_refusal = crate::without_maskable_interrupt_admission_for_test(|| {
        let released = user::release_gated_cross() == 1;
        let retired = runs_until(2_000, || !task::slot_is_live_for_test(slot));
        released && retired && trap::last_syscall().count == crossings_before
    });
    while user::collect().is_some() {}
    let cleanup_exact = spawn_resources().has_same_ownership(&resources_before);
    if !cleanup_exact {
        user::orphan(slot);
        while user::collect().is_some() {}
    }

    waiting
        && late_refusal
        && cleanup_exact
        && crate::maskable_interrupts_are_ready()
        && task::final_interrupt_admission_refusal_for_test()
        && trap::last_syscall().count == crossings_before
        && spawn_resources().has_same_ownership(&resources_before)
}

pub(super) fn shared_kernel_mappings_without_proof_close_the_user_boundary() -> bool {
    while user::collect().is_some() {}
    let shared_before = crate::shared_kernel_mappings_are_safe();
    let isolation_before = crate::user_isolation();
    if isolation_before == crate::UserIsolation::Closed {
        return false;
    }

    let resources_before = spawn_resources();
    let crossings_before = trap::last_syscall().count;
    let attempt = crate::without_shared_kernel_mapping_proof_for_test(user::run);
    let denied = matches!(&attempt, Err(user::SpawnError::NoUserIsolation));
    let cleaned = match attempt {
        Ok(spawned) => cleanup_unexpected_spawn(spawned),
        Err(_) => true,
    };

    denied
        && cleaned
        && crate::shared_kernel_mappings_are_safe() == shared_before
        && crate::user_isolation() == isolation_before
        && task::final_shared_mapping_refusal_for_test()
        && trap::last_syscall().count == crossings_before
        && spawn_resources() == resources_before
}

pub(super) fn no_execute_protection_closes_the_user_boundary() -> bool {
    while user::collect().is_some() {}
    if !Protections::enabled().nx {
        return false;
    }

    let resources_before = spawn_resources();
    let crossings_before = trap::last_syscall().count;
    let attempt = user::without_no_execute_for_test(user::run);
    let denied = matches!(&attempt, Err(user::SpawnError::NoExecuteProtection));

    let cleaned = match attempt {
        Ok(spawned) => cleanup_unexpected_spawn(spawned),
        Err(_) => true,
    };

    denied
        && cleaned
        && Protections::enabled().nx
        && task::final_no_execute_refusal_for_test()
        && trap::last_syscall().count == crossings_before
        && spawn_resources() == resources_before
}

pub(super) fn missing_extended_state_closes_the_user_boundary() -> bool {
    while user::collect().is_some() {}
    if !xstate::is_enabled() {
        return false;
    }

    let resources_before = spawn_resources();
    let crossings_before = trap::last_syscall().count;
    let attempt = user::without_extended_state_for_test(user::run);
    let denied = matches!(&attempt, Err(user::SpawnError::NoExtendedState));

    let cleaned = match attempt {
        Ok(spawned) => cleanup_unexpected_spawn(spawned),
        Err(_) => true,
    };

    denied
        && cleaned
        && xstate::is_enabled()
        && task::final_extended_state_refusal_for_test()
        && trap::last_syscall().count == crossings_before
        && spawn_resources() == resources_before
}

pub(super) fn an_uncalibrated_clock_admits_no_user_or_timer() -> bool {
    while user::collect().is_some() {}
    if !time::is_calibrated() {
        return false;
    }
    let Some(ring) = queue::Queue::new() else {
        return false;
    };
    let table = shared_handle_table_or_false!(1);
    ring.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0xC10C, 1_000_000),
    );
    ring.publish_tail_for_test(1);

    let resources_before = spawn_resources();
    let crossings_before = trap::last_syscall().count;
    let (attempt, served) = time::without_calibration_for_test(|| {
        let attempt = user::run();
        let served = ring.serve(&table);
        (attempt, served)
    });
    let denied = matches!(&attempt, Err(user::SpawnError::NoClock));

    let cleaned = match attempt {
        Ok(spawned) => cleanup_unexpected_spawn(spawned),
        Err(_) => true,
    };

    let completion = ring.read_completion(0);
    denied
        && cleaned
        && served == 1
        && completion.token == 0xC10C
        && completion.status == queue::STATUS_UNAVAILABLE
        && completion.value == 0
        && ring.in_flight() == 0
        && ring.outstanding() == 0
        && ring.is_consistent()
        && time::is_calibrated()
        && trap::last_syscall().count == crossings_before
        && spawn_resources() == resources_before
}

pub(super) fn a_clock_disagreement_revokes_time_without_stranding_timers() -> bool {
    if !time::is_calibrated() || time::Revocation::current() != time::Revocation::None {
        return false;
    }
    let Some(ring) = queue::Queue::new() else {
        return false;
    };
    let table = shared_handle_table_or_false!(1);
    ring.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0xC10D, 10_000_000_000),
    );
    ring.publish_tail_for_test(1);
    if ring.serve(&table) != 0 || ring.in_flight() != 1 {
        return false;
    }

    let before = time::now_ns();
    let sequence = timepage::sequence();
    let deadline = time::uptime_ms() + 10_000;
    let during = time::with_watchdog_disagreement_for_test(|| {
        let frozen = time::now_ns();
        let predicate_calls = core::cell::Cell::new(0_u32);
        let wait_aborted = !runs_until_deadline(deadline, || {
            let calls = predicate_calls.get() + 1;
            predicate_calls.set(calls);
            calls >= 2
        });
        let scheduler_before = time::scheduler_now_ns();
        time::advance_scheduler_for_test();
        let settled = ring.expire(frozen);
        let completion = ring.read_completion(0);
        let page_revoked =
            timepage::read().is_some_and(|reading| reading.mode == timepage::MODE_NONE);
        time::Revocation::current() == time::Revocation::WatchdogDisagreed
            && !time::is_calibrated()
            && frozen >= before
            && time::now_ns() == frozen
            && frozen / time::NANOSECONDS_PER_MS < deadline
            && wait_aborted
            && predicate_calls.get() == 0
            && time::scheduler_now_ns() > scheduler_before
            && settled == 1
            && completion.token == 0xC10D
            && completion.status == queue::STATUS_UNAVAILABLE
            && completion.value == 0
            && ring.in_flight() == 0
            && ring.is_consistent()
            && page_revoked
    });
    thread::rearm_current_for_test();
    let page_restored =
        timepage::read().is_some_and(|reading| reading.mode == timepage::MODE_COUNTER);

    during
        && time::is_calibrated()
        && time::Revocation::current() == time::Revocation::None
        && page_restored
        && sequence
            .zip(timepage::sequence())
            .is_some_and(|(before, after)| after.wrapping_sub(before) == 4)
}

fn exact_zombie(spawned: &user::Spawned) -> bool {
    user::last_rites_of(spawned.slot)
        .is_some_and(|rites| rites.table == spawned.observed_table_incarnation)
        && zombie_record_is_stable(spawned.slot)
}

fn zombie_record_is_stable(slot: usize) -> bool {
    user::last_rites_of(slot).is_some()
        && user::zombie_owns_slot_for_test(slot)
        && !task::slot_is_live_for_test(slot)
}

fn wait_for_exact_zombie(spawned: &user::Spawned) -> bool {
    yields_until(|| zombie_record_is_stable(spawned.slot)) && exact_zombie(spawned)
}

pub(super) fn a_task_release_refusal_retains_the_zombie_for_retry() -> bool {
    task::restore_release_for_test();
    queue::pause_worker_for_test(true);
    if !yields_until(queue::worker_is_paused_for_test) {
        queue::pause_worker_for_test(false);
        return false;
    }
    let settled = yields_until(|| {
        while user::collect().is_some() {}
        !user::has_zombies() && user::process_records_for_test() == (0, 0, 0)
    });
    if !settled {
        queue::pause_worker_for_test(false);
        return false;
    }
    let baseline = spawn_resources();
    let reclaimed_before = user::reclaimed();

    let checked = (|| -> bool {
        let Ok(spawned) = user::run_for_bootstrap() else {
            return false;
        };
        if !wait_for_exact_zombie(&spawned) {
            return false;
        }

        let crossings = trap::last_syscall().count;
        task::refuse_releases_for_test();
        let refused = matches!(user::collect(), user::CollectOutcome::Retained);
        let retained_resources = spawn_resources();
        let retained = refused
            && user::has_zombies()
            && exact_zombie(&spawned)
            && trap::last_syscall().count == crossings
            && user::reclaimed() == reclaimed_before;

        let Ok(probe) = user::run_for_bootstrap() else {
            return false;
        };
        let admission_closed = probe.slot != spawned.slot && exact_zombie(&spawned);
        let probe_retired = wait_for_exact_zombie(&probe);

        task::restore_release_for_test();
        let mut old_reclaimed = false;
        let mut probe_reclaimed = false;
        let mut returned_pages = 0usize;
        let mut returned_tables = 0usize;
        let mut exact_reclaims = probe_retired && exact_zombie(&spawned) && exact_zombie(&probe);
        for _ in 0..2 {
            if !exact_reclaims {
                break;
            }
            let old_before = exact_zombie(&spawned);
            let probe_before = exact_zombie(&probe);
            let user::CollectOutcome::Reclaimed(result) = user::collect() else {
                exact_reclaims = false;
                break;
            };
            let old_after = exact_zombie(&spawned);
            let probe_after = exact_zombie(&probe);
            let reclaimed_old = old_before && !old_after;
            let reclaimed_probe = probe_before && !probe_after;
            if reclaimed_old == reclaimed_probe
                || (reclaimed_old && old_reclaimed)
                || (reclaimed_probe && probe_reclaimed)
            {
                exact_reclaims = false;
                break;
            }
            let Some(pages) = returned_pages.checked_add(result.pages) else {
                exact_reclaims = false;
                break;
            };
            let Some(tables) = returned_tables.checked_add(result.tables) else {
                exact_reclaims = false;
                break;
            };
            returned_pages = pages;
            returned_tables = tables;
            old_reclaimed |= reclaimed_old;
            probe_reclaimed |= reclaimed_probe;
        }
        let both_exact_zombies_reclaimed = exact_reclaims
            && old_reclaimed
            && probe_reclaimed
            && returned_pages > 0
            && returned_tables > 0
            && !user::zombie_owns_slot_for_test(spawned.slot)
            && !user::zombie_owns_slot_for_test(probe.slot);

        let reclaimed_after = user::reclaimed();
        let resources_after = spawn_resources();
        let one_retained_record = retained_resources.process_records == (0, 0, 1);
        let resources_restored = resources_after.has_same_ownership(&baseline);
        let page_tally_is_exact =
            reclaimed_after.0.checked_sub(reclaimed_before.0) == Some(returned_pages);
        let table_tally_is_exact =
            reclaimed_after.1.checked_sub(reclaimed_before.1) == Some(returned_tables);
        let complete = retained
            && one_retained_record
            && admission_closed
            && probe_retired
            && both_exact_zombies_reclaimed
            && !user::has_zombies()
            && resources_restored
            && page_tally_is_exact
            && table_tally_is_exact;
        if !complete {
            info!(
                "selftest: task-release detail retained={retained} one-record={one_retained_record} admission={admission_closed} probe-retired={probe_retired} exact-reclaims={both_exact_zombies_reclaimed} zombies={} resources={resources_restored} pages={page_tally_is_exact} tables={table_tally_is_exact} before={baseline:?} after={resources_after:?}",
                user::has_zombies()
            );
        }
        complete
    })();
    task::restore_release_for_test();
    queue::pause_worker_for_test(false);
    let cleaned = yields_until(|| {
        while user::collect().is_some() {}
        !user::has_zombies()
    });
    checked && cleaned
}

static PAUSED_COLLECTOR_OUTCOME: AtomicUsize = AtomicUsize::new(0);
static PAUSED_COLLECTOR_READY: AtomicUsize = AtomicUsize::new(0);
static PAUSED_COLLECTOR_START: AtomicUsize = AtomicUsize::new(0);

fn collect_while_the_owner_is_in_flight() {
    PAUSED_COLLECTOR_READY.store(1, Ordering::Release);
    while PAUSED_COLLECTOR_START.load(Ordering::Acquire) == 0 {
        thread::yield_now(&mut context());
    }
    let outcome = match user::collect() {
        user::CollectOutcome::Empty => 1,
        user::CollectOutcome::Retained => 2,
        user::CollectOutcome::Reclaimed(_) => 3,
    };
    PAUSED_COLLECTOR_OUTCOME.store(outcome, Ordering::Release);
}

pub(super) fn one_zombie_has_one_settlement_collector() -> bool {
    user::pause_collection_before_settlement_for_test(false);
    user::pause_collection_after_take_for_test(false);
    queue::pause_worker_for_test(true);
    let settled = runs_until(2_000, || {
        while user::collect().is_some() {}
        !user::has_zombies() && user::process_records_for_test() == (0, 0, 0)
    });
    if !settled {
        queue::pause_worker_for_test(false);
        return false;
    }

    let baseline = spawn_resources();
    let retirements = thread::retirements();
    PAUSED_COLLECTOR_OUTCOME.store(0, Ordering::Release);
    PAUSED_COLLECTOR_READY.store(0, Ordering::Release);
    PAUSED_COLLECTOR_START.store(0, Ordering::Release);

    let checked = (|| {
        thread::spawn("settlement-owner", collect_while_the_owner_is_in_flight).ok()?;
        if !runs_until(2_000, || {
            PAUSED_COLLECTOR_READY.load(Ordering::Acquire) != 0
        }) {
            return None;
        }
        let spawned = user::run_for_bootstrap().ok()?;
        let slot = spawned.slot;
        if !runs_until(2_000, || {
            thread::retirements() > retirements && user::zombie_owns_slot_for_test(slot)
        }) {
            return None;
        }

        user::pause_collection_before_settlement_for_test(true);
        PAUSED_COLLECTOR_START.store(1, Ordering::Release);
        if !runs_until(2_000, user::collection_is_settling_for_test) {
            return None;
        }
        let second_was_refused = matches!(user::collect(), user::CollectOutcome::Retained)
            && user::zombie_owns_slot_for_test(slot)
            && user::process_records_for_test().2 == 1;

        user::pause_collection_before_settlement_for_test(false);
        let first_finished = runs_until(2_000, || {
            PAUSED_COLLECTOR_OUTCOME.load(Ordering::Acquire) != 0
        }) && PAUSED_COLLECTOR_OUTCOME.load(Ordering::Acquire) == 3;
        Some(second_was_refused && first_finished && !user::has_zombies())
    })()
    .unwrap_or(false);

    user::pause_collection_before_settlement_for_test(false);
    PAUSED_COLLECTOR_START.store(1, Ordering::Release);
    queue::pause_worker_for_test(false);
    let cleaned = runs_until(2_000, || {
        while user::collect().is_some() {}
        !user::has_zombies() && thread::free_slots() == baseline.thread_slots
    });

    checked && cleaned && spawn_resources().has_same_ownership(&baseline)
}

pub(super) fn an_in_flight_collector_keeps_the_worker_retry_live() -> bool {
    task::restore_release_for_test();
    user::pause_collection_after_take_for_test(false);
    queue::pause_worker_for_test(true);
    let settled = runs_until(2_000, || {
        while user::collect().is_some() {}
        !user::has_zombies() && user::process_records_for_test() == (0, 0, 0)
    });
    if !settled {
        queue::pause_worker_for_test(false);
        return false;
    }

    let baseline = spawn_resources();
    let retirements = thread::retirements();
    let retained = queue::worker_retained_for_test();
    let reaped = queue::worker_reaps_for_test();
    PAUSED_COLLECTOR_OUTCOME.store(0, Ordering::Release);
    PAUSED_COLLECTOR_READY.store(0, Ordering::Release);
    PAUSED_COLLECTOR_START.store(0, Ordering::Release);

    let checked = (|| {
        thread::spawn("paused-collector", collect_while_the_owner_is_in_flight).ok()?;
        if !runs_until(2_000, || {
            PAUSED_COLLECTOR_READY.load(Ordering::Acquire) != 0
        }) {
            return None;
        }
        let spawned = user::run_for_bootstrap().ok()?;
        let slot = spawned.slot;
        if !runs_until(2_000, || {
            thread::retirements() > retirements && user::zombie_owns_slot_for_test(slot)
        }) {
            return None;
        }
        task::refuse_releases_for_test();
        user::pause_collection_after_take_for_test(true);
        PAUSED_COLLECTOR_START.store(1, Ordering::Release);
        if !runs_until(2_000, user::collection_is_paused_for_test) {
            return None;
        }
        queue::pause_worker_for_test(false);
        let worker_saw_the_owner_in_flight = runs_until(2_000, || {
            queue::worker_retained_for_test() > retained
                && user::collection_is_paused_for_test()
                && PAUSED_COLLECTOR_OUTCOME.load(Ordering::Acquire) == 0
        });
        user::pause_collection_after_take_for_test(false);
        let collector_restored_the_owner = runs_until(2_000, || {
            PAUSED_COLLECTOR_OUTCOME.load(Ordering::Acquire) != 0
        }) && PAUSED_COLLECTOR_OUTCOME.load(Ordering::Acquire)
            == 2
            && user::zombie_owns_slot_for_test(slot);
        task::restore_release_for_test();

        // Nothing else exits, spawns or collects after the refusal lifts, so
        // the worker can finish only through its production retry.
        let worker_finished_its_retry = runs_until(2_000, || {
            queue::worker_reaps_for_test() > reaped && !user::has_zombies()
        });
        Some(
            worker_saw_the_owner_in_flight
                && collector_restored_the_owner
                && worker_finished_its_retry,
        )
    })()
    .unwrap_or(false);

    user::pause_collection_after_take_for_test(false);
    PAUSED_COLLECTOR_START.store(1, Ordering::Release);
    task::restore_release_for_test();
    queue::pause_worker_for_test(false);
    let cleaned = runs_until(2_000, || {
        !user::has_zombies() && thread::free_slots() == baseline.thread_slots
    });

    checked && cleaned && spawn_resources().has_same_ownership(&baseline)
}

pub(super) fn a_dead_program_gives_its_memory_back() -> bool {
    // Drain anything earlier checks left, so the measurement below is about
    // this program and not about theirs.
    while user::collect().is_some() {}

    let resources_before = spawn_resources();
    let (pages_before, tables_before) = user::reclaimed();
    let retirements_before = thread::retirements();

    if user::run_for_bootstrap().is_err() {
        return false;
    }
    if !runs_until(2000, || thread::retirements() > retirements_before) {
        return false;
    }

    // Its slot came back with it: the thread is gone from both tables, which
    // is what makes the index usable again.
    let slot_returned = runs_until(500, || {
        thread::free_slots() >= resources_before.thread_slots
    });

    // And its memory, once collected: asked directly, because waiting for
    // idle in a busy self-test may wait for ever.
    let collected_directly = user::collect().is_some();
    let (pages_after, tables_after) = user::reclaimed();
    let gave_back = pages_after > pages_before && tables_after > tables_before;
    let collected = collected_directly || (gave_back && !user::has_zombies());
    let resources_after = spawn_resources();
    let owners_returned = resources_after.has_same_ownership(&resources_before);

    // And nothing is left waiting: one program made one zombie, and it is
    // collected.
    let drained = !user::has_zombies();

    // Sixteen slots, forty programs: without reclamation every spawn after
    // the sixteenth is refused, and the snapshot says the memory came back.
    let repetition_before = spawn_resources();
    let mut ran = 0;
    for _ in 0..40 {
        let retirements = thread::retirements();
        if user::run_for_bootstrap().is_err() {
            break;
        }
        if !runs_until(2000, || thread::retirements() > retirements) {
            break;
        }
        while user::collect().is_some() {}
        ran += 1;
    }
    let repeated = ran == 40;
    let steady = spawn_resources().has_same_ownership(&repetition_before);

    slot_returned && collected && gave_back && owners_returned && drained && repeated && steady
}

pub(super) fn an_unserved_call_is_refused_without_stopping_the_machine() -> bool {
    // Before the stateful ring-3 group, and waited for: the trap-context
    // logging mutation must assert before later state can hide the cause.
    let isolated = runs_until(2_000, || {
        while user::collect().is_some() {}
        !user::has_zombies()
    });
    let resources_before = if isolated {
        Some(spawn_resources())
    } else {
        None
    };
    let live_owner_was_visible = resources_before
        .as_ref()
        .is_some_and(a_gated_process_holds_spawn_ownership);
    let result = isolated
        && live_owner_was_visible
        && (|| {
            let (refused_before, _, _) = user::refusals();
            let faults_before = trap::last_user_fault().count;

            let Ok(first) = user::run_probing() else {
                return false;
            };
            if last_rites_for(&first, 2_000).is_none() {
                return false;
            }

            let (refused_after, number, unanswered) = user::refusals();
            // Exactly one refusal, of the number the program asked for.
            let counted = refused_after == refused_before + 1 && number == 0x0BAD_0BAD;
            // Every answer was written back: a program that got nothing cannot tell a
            // call it made wrong from one the kernel dropped.
            let answered = unanswered == 0;
            // And it left by exiting, not by faulting — so the refusal really did
            // return to ring 3 and the program ran on.
            let exited = trap::last_user_fault().count == faults_before;

            // Whichever observer wins says the one warning. A second real refusal
            // still advances accounting but must not re-arm UART output.
            let _first_observer_may_already_have_reported = user::report_refusals();
            let Ok(second) = user::run_probing() else {
                return false;
            };
            let second_finished = last_rites_for(&second, 2_000).is_some();
            let (refused_final, final_number, unanswered_final) = user::refusals();
            let second_counted = refused_final == refused_after + 1
                && final_number == 0x0BAD_0BAD
                && unanswered_final == 0;
            let warning_is_one_shot =
                !user::report_refusals() && user::refusal_report_latch_for_test() == 1;

            counted
                && answered
                && exited
                && second_finished
                && second_counted
                && warning_is_one_shot
        })();
    let cleaned = runs_until(2_000, || {
        while user::collect().is_some() {}
        if user::has_zombies() {
            return false;
        }
        // A process that has not reached orphaning is not a zombie yet. The
        // ownership baseline keeps every early exit waiting for its live
        // process records, thread slot, and allocated resources to return.
        match resources_before {
            Some(before) => spawn_resources_are_restored(&before),
            None => true,
        }
    });

    result && cleaned
}

fn a_gated_process_holds_spawn_ownership(before: &SpawnResources) -> bool {
    let Ok(spawned) = user::run_gated_for_wake() else {
        return false;
    };
    let waiting = runs_until(500, user::gated_cross_is_waiting);
    let live_owner_visible = waiting && !spawn_resources_are_restored(before);
    // Release after every successful spawn, even when the observations above
    // fail, so the witness cannot strand the process it uses.
    let released = user::release_gated_cross() == 1;
    let finished = last_rites_for(&spawned, 2_000).is_some();

    waiting && live_owner_visible && released && finished
}

fn spawn_resources_are_restored(before: &SpawnResources) -> bool {
    !user::has_zombies() && spawn_resources().has_same_ownership(before)
}

pub(super) fn last_rites_for(spawned: &user::Spawned, ms: u64) -> Option<user::LastRites> {
    let table = spawned.observed_table_incarnation;
    let exact = || user::last_rites_of(spawned.slot).is_some_and(|rites| rites.table == table);
    if !runs_until(ms, exact) {
        return None;
    }
    user::last_rites_of(spawned.slot).filter(|rites| rites.table == table)
}

pub(super) fn a_program_drives_its_own_ring() -> bool {
    let faults_before = trap::last_user_fault().count;

    let Ok(spawned) = user::run_for_bootstrap() else {
        return false;
    };

    // Read from the exit record, not the program: its ring pages are already
    // reclaimed, and the record was taken at death (D-117).
    let Some(rites) = last_rites_for(&spawned, 2_000) else {
        return false;
    };
    let (served, submitted, completed) = (rites.served, rites.submitted, rites.completed);

    // The kernel's own tally, which ring 3 cannot write.
    let one_served = served == 1;

    // The program wrote a submission, and it is the one the ABI describes.
    let asked = submitted.opcode == queue::Opcode::Echo && submitted.token != 0;

    // And the completion answers *that* submission.
    let answered = completed.token == submitted.token
        && completed.status == queue::STATUS_OK
        && completed.value == submitted.argument;

    // It left by exit, not by fault: a program disagreeing with its answer
    // executes `ud2`.
    let exited = trap::last_user_fault().count == faults_before;

    one_served && asked && answered && exited
}

pub(super) fn a_ring_enter_round_trip_is_measurable() -> bool {
    let faults_before = trap::last_user_fault().count;
    let Ok(spawned) = user::run_measuring() else {
        return false;
    };
    let Some(rites) = last_rites_for(&spawned, 2_000) else {
        return false;
    };
    let ticks = rites.submitted.offset;
    let complete = rites.served == 1
        && rites.submitted.opcode == queue::Opcode::Echo
        && rites.completed.token == rites.submitted.token
        && rites.completed.status == queue::STATUS_OK
        && rites.completed.value == rites.submitted.argument
        && trap::last_user_fault().count == faults_before;
    if complete && ticks > 0 {
        RING_ENTER_TICKS.store(ticks, Ordering::Relaxed);
        true
    } else {
        false
    }
}

static RING_ENTER_TICKS: AtomicU64 = AtomicU64::new(0);

pub(super) fn a_full_buffer_nvme_read_is_measurable() -> bool {
    let Some((bytes, ticks)) = nvme::measure_buffered_read_for_test() else {
        return false;
    };
    STORAGE_BYTES.store(bytes, Ordering::Relaxed);
    STORAGE_TICKS.store(ticks, Ordering::Relaxed);
    bytes == PAGE_SIZE && ticks > 0
}

static STORAGE_BYTES: AtomicU64 = AtomicU64::new(0);
static STORAGE_TICKS: AtomicU64 = AtomicU64::new(0);
pub(super) static SELFTEST_STARTED_NS: AtomicU64 = AtomicU64::new(0);

pub(super) fn report_baseline() {
    let resources = spawn_resources();
    info!(
        "selftest: baseline v1 boot_ns={} frames_free={} heap_slab={} heap_large={} heap_live={} handles_committed={} handles_live={} trap_ticks={} storage_bytes={} storage_ticks={} counter_hz={}",
        SELFTEST_STARTED_NS.load(Ordering::Relaxed),
        resources.frames,
        resources.heap.slab_frames,
        resources.heap.large_frames,
        resources.heap.live,
        resources.arena.0,
        resources.arena.1,
        RING_ENTER_TICKS.load(Ordering::Relaxed),
        STORAGE_BYTES.load(Ordering::Relaxed),
        STORAGE_TICKS.load(Ordering::Relaxed),
        time::counter_hz()
    );
}

pub(super) fn a_program_duplicates_its_bootstrap_handle() -> bool {
    let faults_before = trap::last_user_fault().count;

    let Ok(spawned) = user::run_duplicating() else {
        return false;
    };
    let Some(rites) = last_rites_for(&spawned, 2_000) else {
        return false;
    };
    let submitted = rites.submitted;
    let completed = rites.completed;

    // It began with one handle, so the nonzero handle ring 3 supplied can only
    // be the bootstrap entry. The requested right is exactly INSPECT.
    let asked = submitted.opcode == queue::Opcode::Duplicate
        && submitted.token != 0
        && submitted.handle != 0
        && submitted.argument == u64::from(Rights::INSPECT.bits());

    // The completion carries a distinct, representable handle, and the exit
    // record proves both entries were installed before teardown reclaimed them.
    let new_handle = u32::try_from(completed.value).ok();
    let answered = completed.token == submitted.token
        && completed.status == queue::STATUS_OK
        && new_handle.is_some_and(|handle| handle != 0 && handle != submitted.handle);
    let retained = rites.served == 1
        && rites.handles == 2
        && rites.bootstrap == Some(object::ObjectType::Channel);

    // Every emitted-program disagreement executes `ud2`, so a clean exit is
    // independent evidence that ring 3 accepted the same result.
    let exited = trap::last_user_fault().count == faults_before;

    asked && answered && retained && exited
}

pub(super) fn a_program_waits_through_its_trap_frame() -> bool {
    let faults_before = trap::last_user_fault().count;
    let waits_before = thread::trap_waits();

    let Ok(spawned) = user::run_waiting() else {
        return false;
    };
    let Some(rites) = last_rites_for(&spawned, 2_000) else {
        return false;
    };

    rites.submitted.opcode == queue::Opcode::Timer
        && rites.submitted.token != 0
        && rites.served == 1
        && rites.completed.token == rites.submitted.token
        && rites.completed.status == queue::STATUS_OK
        && rites.completed.value > 0
        && thread::trap_waits() == waits_before + 1
        && trap::last_user_fault().count == faults_before
}

/// The real ring a kernel thread waits on during the syscall wake check.
static SYSCALL_WAKE_RING: SpinLock<Option<Arc<queue::Queue>>> = SpinLock::new(None);

/// Zero while the waiter is live and one after its production wait returns.
static SYSCALL_WAITER_RETURNED: AtomicUsize = AtomicUsize::new(0);

/// Makes failed orchestration release a waiter instead of stranding its slot.
static CANCEL_SYSCALL_WAITER: AtomicUsize = AtomicUsize::new(0);

fn wait_on_the_program_ring() {
    let ring = SYSCALL_WAKE_RING.lock_leaf().as_ref().map(Arc::clone);
    let Some(ring) = ring else {
        return;
    };
    ring.wait_for_test(&mut context(), &CANCEL_SYSCALL_WAITER);
    SYSCALL_WAITER_RETURNED.store(1, Ordering::Release);
}

pub(super) fn a_syscall_wake_arms_the_timer_for_what_it_woke() -> bool {
    let free = thread::free_slots();
    let faults = trap::last_user_fault().count;
    let ring_arms = user::ring_enter_arms_for_test();
    let handler_arms = thread::handler_arms();
    SYSCALL_WAITER_RETURNED.store(0, Ordering::Release);
    CANCEL_SYSCALL_WAITER.store(0, Ordering::Release);
    *SYSCALL_WAKE_RING.lock_leaf() = None;

    let Ok(spawned) = user::run_gated_for_wake() else {
        return false;
    };
    finish_syscall_wake_check(spawned, free, faults, ring_arms, handler_arms)
}

fn finish_syscall_wake_check(
    spawned: user::Spawned,
    free: usize,
    faults: usize,
    ring_arms: usize,
    handler_arms: usize,
) -> bool {
    let Some((ring, _)) = user::authority_for_test(spawned.slot) else {
        assert_eq!(
            user::release_gated_cross(),
            1,
            "a gated child without visible authority was not released"
        );
        return false;
    };
    *SYSCALL_WAKE_RING.lock_leaf() = Some(Arc::clone(&ring));
    let waiter_started = thread::spawn("syscall-ring-waiter", wait_on_the_program_ring).is_ok();
    let registered = waiter_started
        && runs_until(500, || {
            user::gated_cross_is_waiting() && ring.has_waiter_for_test()
        });
    let served = ring.served_count();
    let child_released = user::release_gated_cross() == 1;
    let woke = registered
        && child_released
        && runs_until(2_000, || {
            ring.served_count() == served + 1
                && SYSCALL_WAITER_RETURNED.load(Ordering::Acquire) == 1
                && user::ring_enter_arms_for_test() == ring_arms + 1
        });
    let rites = last_rites_for(&spawned, 2_000);

    CANCEL_SYSCALL_WAITER.store(1, Ordering::Release);
    let _released_waiters = ring.release_waiter_for_test();
    *SYSCALL_WAKE_RING.lock_leaf() = None;
    let settled = runs_until(2_000, || thread::free_slots() == free);

    let answered = rites.is_some_and(|rites| {
        rites.submitted.opcode == queue::Opcode::Echo
            && rites.served == served + 1
            && rites.completed.token == rites.submitted.token
            && rites.completed.status == queue::STATUS_OK
    });
    info!(
        "selftest: syscall wake evidence registered={registered} released={child_released} woke={woke} answered={answered} ring-arms={}->{} handler-arms={}->{} returned={} settled={settled}",
        ring_arms,
        user::ring_enter_arms_for_test(),
        handler_arms,
        thread::handler_arms(),
        SYSCALL_WAITER_RETURNED.load(Ordering::Acquire)
    );

    woke && answered
        && trap::last_user_fault().count == faults
        && user::ring_enter_arms_for_test() == ring_arms + 1
        && settled
}

pub(super) fn a_spawned_program_holds_exactly_one_handle() -> bool {
    // Its own program: every ring-3 check counts crossings, and an extra one
    // would land on whichever check looked next.
    let Ok(spawned) = user::run_for_bootstrap() else {
        return false;
    };
    let Some(birth) = user::birth_of(&spawned) else {
        return false;
    };

    // Captured while the slot was still Reserved: observing after `spawn`
    // returns races this short program's exit (D-217).
    let exactly_one = birth.handles == 1;
    let a_channel = birth.bootstrap == object::ObjectType::Channel;

    // And a ring at the shared address, in place before its first
    // instruction (D-113); the kernel's record, not the rewritable header (D-115).
    let (version, submissions, completions) = birth.ring;
    let has_a_ring = version == ring_proto::VERSION
        && submissions > 0
        && completions >= submissions.saturating_mul(2);
    let mapped = birth.ring_flags.contains(Flags::USER);

    // Leaves with one handle, and is waited for: retiring during a later
    // check would move the free-slot count under it (D-117).
    let died_with_one = last_rites_for(&spawned, 2_000).is_some_and(|rites| {
        rites.handles == 1 && rites.bootstrap == Some(object::ObjectType::Channel)
    });

    exactly_one && a_channel && has_a_ring && mapped && died_with_one
}

pub(super) fn a_meaningless_right_is_refused() -> bool {
    let table = handle_table_or_false!(4);
    let channel = object::bootstrap_identity();

    let refused = table.insert(channel.clone(), Rights::READ) == Err(HandleError::Meaningless);
    let also_refused = table.insert(channel.clone(), Rights::ALL) == Err(HandleError::Meaningless);

    // Universal rights mean the same for every object.
    let allowed = table.insert(channel.clone(), Rights::UNIVERSAL).is_ok();
    let nothing_is_allowed = table.insert(channel, Rights::NONE).is_ok();

    let Some(root) = vfs::root() else {
        return false;
    };
    let Ok(vfs::Node::File(file)) = vfs::resolve(root, SMALL_NAME) else {
        return false;
    };
    let file_refuses = [Rights::WRITE, Rights::EXECUTE, Rights::MAP]
        .into_iter()
        .all(|right| table.insert(object::file(file), right) == Err(HandleError::Meaningless));
    let directory_refuses =
        table.insert(object::directory(root), Rights::MODIFY) == Err(HandleError::Meaningless);

    refused
        && also_refused
        && allowed
        && nothing_is_allowed
        && file_refuses
        && directory_refuses
        && table.live() == 2
}
