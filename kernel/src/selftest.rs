//! Boot-time self-checks.
//!
//! Hardware-dependent checks run on a fully booted `selftest` kernel and report
//! through [`crate::arch::current::debug_exit`]. Pure logic belongs in a host
//! `libs/` test instead. Boot assertions are not repeated because a panic is a
//! missing verdict, not a red check.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt::Write;
use core::hint::spin_loop;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use handle_space::Rights;
use ring_proto::SharedLayout;
use slab_layout::AllocatorOwnership;
use stack_layout::{Slot as StackSlot, StackLayout};
use wost::arch::current::cpu::{self, Protections};
use wost::arch::current::paging::{
    self, ActivateError, AddressSpace, BootRegion, MapError, ReclaimError,
};
use wost::arch::current::usercopy::{self, UserCopyError};
use wost::arch::current::{apic, interrupt, pic, port, segment, task, xstate};
use wost::boot::{self, FramebufferInfo, MemoryRegion, Rgb};
use wost::mm;
use wost::sync::{Once, SpinLock, SpinLockWitness};
use x86_paging::{CanonicalAddress, Flags, LARGE_PAGE_SIZE, PAGE_SIZE};
use x86_tables::exception;

use crate::arch::current::debug_exit;
use crate::arch::current::ioapic;
use crate::arch::current::trap::{self, Record};
use crate::arch::current::uart::{self, UartError};
use crate::arch::current::vector;
use crate::devfs;
use crate::fs;
use crate::handle::{HandleError, HandleTable};
use crate::iommu;
use crate::nvme;
use crate::object::{self, ObjectType};
use crate::pci;
use crate::queue;
use crate::thread::ThreadContext;
use crate::timepage;
use crate::tmpfs;
use crate::user;
use crate::vfs;
use crate::wait::WaitQueue;
use crate::{dev, error, info, input, mem, thread, time, warn};

macro_rules! handle_table_or_false {
    ($quota:expr) => {{
        let Ok(table) = HandleTable::new($quota) else {
            return false;
        };
        table
    }};
}

macro_rules! shared_handle_table_or_false {
    ($quota:expr) => {
        Arc::new(handle_table_or_false!($quota))
    };
}

fn frame_round_trips() -> bool {
    let before = mm::free_frames();
    let frame = mm::allocate();
    let taken = frame.is_some() && mm::free_frames() == before - 1;
    drop(frame);
    taken && mm::free_frames() == before
}

fn uncertain_dma_memory_is_never_recycled() -> bool {
    let before = mm::free_frames();
    let Some(segment) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let mut dma = mm::DmaSegment::from(segment);
    let reserved = mm::free_frames() == before - 1 && dma.address().is_some();

    dma.retain();
    let inaccessible = dma.address().is_none() && !dma.zero() && !dma.read_at(0, &mut [0u8; 1]);
    drop(dma);

    reserved && inaccessible && mm::free_frames() == before - 1
}

fn dma_byte_access_is_exact_and_bounded() -> bool {
    const FILL: u8 = 0x31;
    const TAIL: [u8; 2] = [0xa5, 0x5a];

    let Some(segment) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let mut dma = mm::DmaSegment::from(segment);
    let Some(len) = dma.len().and_then(|len| usize::try_from(len).ok()) else {
        return false;
    };
    let Some(tail) = len.checked_sub(TAIL.len()) else {
        return false;
    };

    let exact = dma.fill(FILL) && dma.write_at(tail, &TAIL) && {
        let mut observed = [0; TAIL.len()];
        dma.read_at(tail, &mut observed) && observed == TAIL
    };
    let mut refused_read = [0xcc; 2];
    let refused = !dma.write_at(len - 1, &TAIL)
        && !dma.write_at(usize::MAX, &TAIL)
        && !dma.read_at(len - 1, &mut refused_read)
        && refused_read == [0xcc; 2];
    let unchanged = {
        let mut observed = [0; TAIL.len()];
        dma.read_at(tail, &mut observed) && observed == TAIL
    };
    exact && refused && unchanged
}

fn frames_are_exclusive() -> bool {
    let (Some(first), Some(second)) = (mm::allocate(), mm::allocate()) else {
        return false;
    };
    first.address() != second.address()
}

fn dirty_memory_can_be_zeroed() -> bool {
    let Some(mut frame) = mm::allocate() else {
        return false;
    };

    let dirtied = frame.fill(0xAA) && frame.is_zero() == Some(false);
    let cleaned = frame.zero() && frame.is_zero() == Some(true);
    dirtied && cleaned
}

fn allocation_is_zeroed() -> bool {
    mm::allocate_zeroed().and_then(|frame| frame.is_zero()) == Some(true)
}

fn a_run_is_contiguous_and_aligned() -> bool {
    const FRAMES: usize = 8;
    /// Distinct per frame, so a page from somewhere else reads as the wrong one.
    const MARK: u8 = 0xC7;

    let before = mm::free_frames();
    let Some(mut run) = mm::allocate_contiguous(FRAMES, FRAMES) else {
        return false;
    };
    let aligned = run.address().is_multiple_of(FRAMES as u64 * 4096);
    let sized = run.frames() == FRAMES && mm::free_frames() == before - FRAMES;

    let mut spans = true;
    for frame in 0..FRAMES {
        let at = frame * 4096;
        spans &= run.write_at(at, &[MARK ^ frame as u8]);
    }

    // An overlapping allocation would disprove exclusive contiguity.
    let start = run.address();
    let end = start + (FRAMES as u64 * 4096);
    let elsewhere = mm::allocate_zeroed()
        .map(|frame| {
            let address = frame.address();
            !(start..end).contains(&address)
        })
        .unwrap_or(false);

    let mut intact = true;
    for frame in 0..FRAMES {
        let mut byte = [0u8; 1];
        intact &= run.read_at(frame * 4096, &mut byte) && byte[0] == MARK ^ frame as u8;
    }
    drop(run);

    aligned && sized && spans && elsewhere && intact && mm::free_frames() == before
}

fn the_loaded_table_is_ours() -> bool {
    paging::is_active()
        && cpu::page_table_base() == paging::active_root()
        && cpu::page_table_context_ids_are_disabled_for_test()
}

fn an_admitted_canonical_page_can_be_invalidated() -> bool {
    let raw = an_admitted_canonical_page_can_be_invalidated as *const () as u64;
    let Some(before) = paging::translate(raw) else {
        return false;
    };
    let Some(address) = CanonicalAddress::new(raw) else {
        return false;
    };
    cpu::invalidate_page(address);
    paging::translate(raw) == Some(before)
}

fn the_image_obeys_write_xor_execute() -> bool {
    let sections = boot::sections();
    let Some(direct) = mm::direct_map() else {
        return false;
    };

    [
        (sections.text, false, true),
        (sections.rodata, false, false),
        (sections.data, true, false),
    ]
    .into_iter()
    .all(|(section, writable, executable)| {
        // Offsets from the base, never a sum of two higher-half addresses:
        // those are within a hair of `u64::MAX` and adding a pair overflows.
        let len = section.mapped_len();
        let middle = section.start + ((len / 2) & !4095);
        [section.start, middle, section.start + len - 4096]
            .into_iter()
            .all(|address| {
                let Some((physical, flags)) = paging::translate(address) else {
                    return false;
                };
                let Some(alias) = direct.virtual_address(physical) else {
                    return false;
                };
                let Some((alias_physical, alias_flags)) = paging::translate(alias) else {
                    return false;
                };
                [flags, alias_flags].into_iter().all(|flags| {
                    !flags.is_write_execute()
                        && flags.contains(Flags::WRITABLE) == writable
                        && !flags.contains(Flags::NO_EXECUTE) == executable
                }) && alias_physical == physical
            })
    })
}

fn supervisor_writes_obey_read_only_pages() -> bool {
    cpu::drive_write_protection_for_test()
        && cpu::write_protection_probe()
        && Protections::enabled().write_protect
}

fn the_direct_map_is_not_executable() -> bool {
    let Some(frame) = mm::allocate() else {
        return false;
    };
    // Any frame the allocator owns is inside a region the direct map covers,
    // so its direct-map address is a fair sample of the whole mapping.
    let Some(address) = mm::direct_map().and_then(|map| map.virtual_address(frame.address()))
    else {
        return false;
    };

    match paging::translate(address) {
        Some((physical, flags)) => {
            physical == frame.address()
                && flags.contains(Flags::WRITABLE)
                && flags.contains(Flags::NO_EXECUTE)
                && !flags.is_write_execute()
        }
        None => false,
    }
}

fn firmware_reads_are_bounded() -> bool {
    const WORD: usize = 8;

    let Some(map) = mm::memory_map_for_test() else {
        return false;
    };
    let pick = |wanted: fn(&MemoryRegion) -> bool| {
        map.regions()
            .find(|region| wanted(region) && region.len >= WORD as u64)
            .map(|region| region.base)
    };
    let addresses = (
        pick(|region| region.kind.is_allocatable()),
        pick(|region| !region.kind.is_firmware_table() && !region.kind.is_allocatable()),
        pick(|region| region.kind.is_firmware_table()),
        // Inside the address space, so only the map can refuse it — an address
        // at the top would be caught by an overflow guard instead.
        map.regions()
            .map(|region| region.base + region.len)
            .max()
            .and_then(|end| end.checked_next_multiple_of(PAGE_SIZE)),
    );

    let (Some(usable), Some(unreachable), Some(readable), Some(undescribed)) = addresses else {
        return false;
    };

    let Some(firmware) = mm::firmware_memory() else {
        return false;
    };
    let mut buffer = [0; WORD];
    !wost::mm::FirmwareMemory::read(&firmware, usable, &mut buffer)
        && !wost::mm::FirmwareMemory::read(&firmware, undescribed, &mut buffer)
        && !wost::mm::FirmwareMemory::read(&firmware, unreachable, &mut buffer)
        && wost::mm::FirmwareMemory::read(&firmware, readable, &mut buffer)
}

fn in_reclaimed_space(
    constructor: impl FnOnce() -> Result<AddressSpace, MapError>,
    check: impl FnOnce(&mut AddressSpace) -> bool,
) -> bool {
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = constructor() else {
        return false;
    };
    let checked = check(&mut space);
    let reclaimed = space.reclaim(None).is_ok();
    drop(space);
    checked && reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

fn a_second_switch_is_refused() -> bool {
    in_reclaimed_space(AddressSpace::new, |space| {
        paging::activate(space) == Err(ActivateError::AlreadyActive)
    })
}

fn a_mapping_added_after_the_switch_is_live() -> bool {
    let Some(probe) =
        thread::boot_stack_layout().and_then(|layout| layout.end().checked_add(4 * PAGE_SIZE))
    else {
        return false;
    };

    let free_before = mm::free_frames();
    let unmapped_before = paging::translate(probe).is_none();

    let Some(run) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let physical = run.address();

    let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let Some(Ok(mapping)) = mem::map_owned_for_test(&mut context(), probe, run, wanted) else {
        return false;
    };

    // `OWNED` is expected because `map_owned` writes it: the entry is where
    // provenance lives (D-88).
    let expected = wanted.union(Flags::PRESENT).union(Flags::OWNED);
    let translated = paging::translate(probe) == Some((physical, expected));
    let on_the_live_root = paging::active_root() == cpu::page_table_base();
    let Some(Ok(returned)) = mem::unmap_for_test(&mut context(), mapping) else {
        return false;
    };
    drop(returned);
    let restored = paging::translate(probe).is_none() && mm::free_frames() == free_before;

    unmapped_before && translated && on_the_live_root && restored
}

static REREAD_QUEUE: WaitQueue = WaitQueue::new();
static REREAD_READY: AtomicUsize = AtomicUsize::new(0);

/// 0 before the thread runs, 1 once it has come back through `wait_unless`.
static REREAD_RETURNED: AtomicUsize = AtomicUsize::new(0);

fn wait_on_a_ready_condition() {
    REREAD_QUEUE.wait_unless(&mut context(), || REREAD_READY.load(Ordering::Acquire) != 0);
    REREAD_RETURNED.store(1, Ordering::Release);
}

fn a_waiter_rereads_its_condition_before_blocking() -> bool {
    REREAD_READY.store(1, Ordering::Release);
    REREAD_RETURNED.store(0, Ordering::Release);
    let free_before = thread::free_slots();
    if thread::spawn("wait-rereader", wait_on_a_ready_condition).is_err() {
        return false;
    }

    let came_back = runs_until(200, || REREAD_RETURNED.load(Ordering::Acquire) == 1);
    // `>=`: a program retiring elsewhere frees a slot too, and that does not
    // falsify this thread's slot coming back (D-117).
    let settled = runs_until(200, || thread::free_slots() >= free_before);
    // And nothing is left registered, so a later wake cannot name a thread
    // that is no longer waiting here.
    came_back && settled && REREAD_QUEUE.is_idle()
}

fn a_thread_crosses_into_ring_three_and_calls_back() -> bool {
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

fn a_program_cannot_bring_smap_down_with_it() -> bool {
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

fn a_faulting_program_dies_alone() -> bool {
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

fn a_noncanonical_user_return_stack_retires_only_its_program() -> bool {
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

fn unsafe_user_return_flags_retire_only_their_program() -> bool {
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

fn a_program_cannot_aim_at_the_copy_window() -> bool {
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

fn two_spaces_map_one_address_to_different_memory() -> bool {
    /// A user address neither space has anything else at.
    const SHARED: u64 = user::CODE + 0x10_0000;

    if !runs_until(2000, || !user::has_zombies()) {
        return false;
    }
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut first) = AddressSpace::for_user() else {
        return false;
    };
    let Ok(mut second) = AddressSpace::for_user() else {
        let reclaimed = first.reclaim(None).is_ok();
        drop(first);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a missing second space did not return the first space's tables"
        );
        return false;
    };

    let mut first_mapped = false;
    let mut second_mapped = false;
    let checked = (|| {
        if first.root() == second.root() {
            return false;
        }

        let writable = Flags::WRITABLE.union(Flags::NO_EXECUTE);
        let (Some(mut one), Some(mut two)) =
            (mm::allocate_contiguous(1, 1), mm::allocate_contiguous(1, 1))
        else {
            return false;
        };
        if !one.zero() || !two.zero() || !one.write_at(0, &[0xA5]) || !two.write_at(0, &[0x5A]) {
            return false;
        }
        let (one_at, two_at) = (one.address(), two.address());
        if one_at == two_at || first.map_user(SHARED, one.into(), writable).is_err() {
            return false;
        }
        first_mapped = true;
        if second.map_user(SHARED, two.into(), writable).is_err() {
            return false;
        }
        second_mapped = true;

        let (Some((first_frame, first_flags)), Some((second_frame, second_flags))) =
            (first.lookup(SHARED), second.lookup(SHARED))
        else {
            return false;
        };
        let separate = first_frame == one_at && second_frame == two_at;
        let both_user = first_flags.contains(Flags::USER) && second_flags.contains(Flags::USER);

        let text = boot::sections().text.start;
        let shared_kernel =
            first.lookup(text) == second.lookup(text) && first.lookup(text).is_some();
        separate && both_user && shared_kernel
    })();

    let first_unmapped = !first_mapped || first.unmap(SHARED, PAGE_SIZE).map(drop).is_ok();
    let second_unmapped = !second_mapped || second.unmap(SHARED, PAGE_SIZE).map(drop).is_ok();
    let first_reclaimed = first.reclaim(None).is_ok();
    let second_reclaimed = second.reclaim(None).is_ok();
    drop((first, second));

    checked
        && first_unmapped
        && second_unmapped
        && first_reclaimed
        && second_reclaimed
        && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

fn every_enabled_state_component_survives_a_switch() -> bool {
    /// What this thread puts there.
    const MINE: xstate::Probe = xstate::Probe {
        x87: 0x1234_5678_7654_3210,
        xmm: 0xA5A5_1234_5678_A5A5,
        ymm: 0xC3C3_0F0F_F0F0_3C3C,
    };
    if !xstate::is_enabled() {
        // The hardware state is coherent; user launch separately refuses it.
        return xstate::bytes() == 0 && xstate::extended_control() == 0;
    }

    // The write and read agree before anything else runs. Reading pops x87,
    // so install the complete probe again for the actual switch.
    if !xstate::write_probe(MINE)
        || xstate::read_probe() != Some(MINE)
        || !xstate::write_probe(MINE)
    {
        return false;
    }

    EXTENDED_INTRUDER_RAN.store(0, Ordering::Release);
    EXTENDED_INTRUDER_SAW_OWN.store(0, Ordering::Release);
    if thread::spawn("extended-intruder", clobber_extended_state).is_err() {
        return false;
    }
    if !runs_until(500, || EXTENDED_INTRUDER_RAN.load(Ordering::Acquire) == 1) {
        return false;
    }

    let kept = xstate::read_probe() == Some(MINE);
    let intruded = EXTENDED_INTRUDER_SAW_OWN.load(Ordering::Acquire) == 1;

    // Read live `XCR0` instead of restating `init`'s precondition (D-115).
    let mask = xstate::extended_control();
    let masked = mask & 0b111 == 0b111 && xstate::bytes() <= 1024;

    kept && intruded && masked
}

/// Set by the intruding thread once it has read its restored state.
static EXTENDED_INTRUDER_RAN: AtomicUsize = AtomicUsize::new(0);

/// Whether the intruding thread read back all three of its own values.
static EXTENDED_INTRUDER_SAW_OWN: AtomicUsize = AtomicUsize::new(0);

fn clobber_extended_state() {
    const THEIRS: xstate::Probe = xstate::Probe {
        x87: 0x0FED_CBA9_1357_2468,
        xmm: 0x5A5A_8765_4321_5A5A,
        ymm: 0x3C3C_F0F0_0F0F_C3C3,
    };
    if !xstate::write_probe(THEIRS) {
        return;
    }
    let mut context = context();
    thread::yield_now(&mut context);
    let saw_own = xstate::read_probe() == Some(THEIRS);
    EXTENDED_INTRUDER_SAW_OWN.store(saw_own as usize, Ordering::Release);
    EXTENDED_INTRUDER_RAN.store(1, Ordering::Release);
}

fn kernel_memory_cannot_be_published_to_ring_three() -> bool {
    /// A user address nothing else uses.
    const PROBE: u64 = user::STACK + 0x40_0000;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some(mut run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !run.zero() {
            return false;
        }

        // Recognizable kernel bytes keep accidental publication from looking zeroed.
        let secret = [0xDEu8; 8];
        if !run.write_at(0, &secret) {
            return false;
        }

        let refused = matches!(
            space.map_owned(PROBE, run, Flags::USER.union(Flags::WRITABLE)),
            Err(MapError::NotYours(PROBE))
        );
        if !refused {
            return false;
        }

        let Some(mut giving) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !giving.zero() || !giving.write_at(0, &secret) {
            return false;
        }
        let untyped: mm::USegment = giving.into();
        let address = untyped.address();

        let Some(back) = untyped.reclaim() else {
            return false;
        };
        // A successful reclaim alone does not prove that the bytes were cleared.
        let mut recovered = [0xFFu8; 8];
        let cleared =
            back.address() == address && back.read_at(0, &mut recovered) && recovered == [0u8; 8];
        drop(back);

        let Some(mut given) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !given.zero() {
            return false;
        }
        if space
            .map_user(
                PROBE,
                given.into(),
                Flags::WRITABLE.union(Flags::NO_EXECUTE),
            )
            .is_err()
        {
            return false;
        }
        let user_accessible = space
            .lookup(PROBE)
            .is_some_and(|(_, flags)| flags.contains(Flags::USER));
        let unmapped = space.unmap(PROBE, PAGE_SIZE).map(drop).is_ok();

        refused && cleared && user_accessible && unmapped
    })
}

fn mark_external_run(run: &mut mm::Segment, marker: u8) -> bool {
    let expected = [marker; 16];
    let mut found = [0; 16];
    run.write_at(0, &expected) && run.read_at(0, &mut found) && found == expected
}

fn external_user_bytes_are_zeroed_before_free() -> bool {
    const DIRECT: u8 = 0xa1;
    const SHARED: u8 = 0xb2;
    const REFUSED: u8 = 0xc3;
    const OUTSIDE_USER_HALF: u64 = 0xffff_8000_0010_0000;

    let direct = {
        let Some(mut run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !mark_external_run(&mut run, DIRECT) {
            return false;
        }
        let address = run.address();
        drop(mm::USegment::from(run));
        mm::freed_external_run_is_zero_for_test(address, 1)
    };

    let shared = {
        let Some(mut run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !mark_external_run(&mut run, SHARED) {
            return false;
        }
        let address = run.address();
        drop(mm::USegment::from(run).share());
        mm::freed_external_run_is_zero_for_test(address, 1)
    };

    let refused = in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some(mut run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !mark_external_run(&mut run, REFUSED) {
            return false;
        }
        let address = run.address();
        let Err(error) = space.map_user(
            OUTSIDE_USER_HALF,
            run.into(),
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        ) else {
            return false;
        };
        let reason = error.reason();
        let run = error.into_run();
        let retained = reason == MapError::NotYours(OUTSIDE_USER_HALF) && run.address() == address;
        drop(run);
        retained && mm::freed_external_run_is_zero_for_test(address, 1)
    });

    direct && shared && refused
}

fn a_refused_external_clear_withholds_its_run() -> bool {
    const MARKER: u8 = 0xd4;

    let free_before = mm::free_frames();
    let Some(mut run) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !mark_external_run(&mut run, MARKER) {
        return false;
    }
    let withheld = mm::refused_external_clear_withholds_for_test(run, &[MARKER; 16]);
    withheld && mm::free_frames() == free_before
}

fn only_a_space_that_matches_the_kernel_can_be_installed() -> bool {
    let bare_refused = in_reclaimed_space(AddressSpace::new, |bare| {
        matches!(bare.bind(), Err(ActivateError::NotAUserSpace))
    });

    let binds = {
        let free_before = mm::free_frames();
        let Ok(mut shared) = AddressSpace::for_user() else {
            return false;
        };
        let released = shared.bind().ok().map(task::release_uninstalled);
        let bound = released.is_some();
        let reclaimed = shared.reclaim(released).is_ok();
        bound && reclaimed && mm::free_frames() == free_before
    };

    let device_refused = in_reclaimed_space(AddressSpace::for_user, |probe| {
        matches!(
            paging::admit_device_range(0xfee0_0000, PAGE_SIZE)
                .and_then(|range| probe.map_device(range)),
            Err(MapError::NotYours(_))
        )
    });

    bare_refused && binds && device_refused
}

fn the_loader_refuses_what_it_may_not_map() -> bool {
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

fn a_second_thread_crosses_onto_its_own_stack() -> bool {
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

fn the_return_to_ring_three_clears_buffers() -> bool {
    let armed = interrupt::buffer_clear_is_armed();
    let executed = interrupt::buffer_clears();
    if !armed || executed == 0 {
        return false;
    }
    // Once per crossing, not once at boot: every syscall came through this
    // branch, so the tally cannot lag the crossings.
    executed >= trap::last_syscall().count as u64
}

fn a_faulting_user_copy_resumes_and_reports() -> bool {
    let recoveries_before = trap::user_copy_recoveries();
    let exact = usercopy::drive_fault_recovery_for_test();
    let recovered = trap::user_copy_recoveries() == recoveries_before + 1;
    exact && recovered
}

fn boundary_probe() -> Option<(u64, u64)> {
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

fn an_active_unmap_exposes_only_its_replacement() -> bool {
    const FIRST: [u8; 16] = [0xa5; 16];
    const SECOND: [u8; 16] = [0x5a; 16];

    let Some((_, stack)) = boundary_probe() else {
        return false;
    };
    let Some(at) = stack.checked_add(PAGE_SIZE) else {
        return false;
    };
    let Some(mut first) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !first.zero() || !first.write_at(0, &FIRST) {
        return false;
    }
    let first_physical = first.address();
    let first_flags = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let mut context = context();
    let Some(Ok(first_mapping)) = mem::map_user_page_for_test(&mut context, at, first, first_flags)
    else {
        return false;
    };

    let mut warmed = [0; FIRST.len()];
    let warmed_first = usercopy::copy_from_user(at, &mut warmed).is_ok() && warmed == FIRST;
    let Some(Ok(first_taken)) = mem::unmap_for_test(&mut context, first_mapping) else {
        return false;
    };

    let Some(mut second) = mm::allocate_contiguous(1, 1) else {
        drop(first_taken);
        return false;
    };
    if !second.zero() || !second.write_at(0, &SECOND) {
        drop(first_taken);
        return false;
    }
    let second_physical = second.address();
    let Some(Ok(second_mapping)) =
        mem::map_user_page_for_test(&mut context, at, second, Flags::NONE)
    else {
        drop(first_taken);
        return false;
    };

    let replacement_is_distinct = first_physical != second_physical
        && paging::translate(at).is_some_and(|(physical, flags)| {
            physical == second_physical
                && flags.contains(Flags::PRESENT)
                && flags.contains(Flags::USER)
                && flags.contains(Flags::OWNED)
                && !flags.contains(Flags::WRITABLE)
                && !flags.contains(Flags::NO_EXECUTE)
        });
    let mut observed = [0; SECOND.len()];
    let replacement_was_read =
        usercopy::copy_from_user(at, &mut observed).is_ok() && observed == SECOND;
    let replacement_is_read_only = matches!(
        usercopy::copy_to_user(at, &FIRST),
        Err(UserCopyError::ReadOnly(found)) if found == at
    );

    let second_removed = mem::unmap_for_test(&mut context, second_mapping)
        .is_some_and(|result| result.map(drop).is_ok());
    drop(first_taken);

    warmed_first
        && replacement_is_distinct
        && replacement_was_read
        && replacement_is_read_only
        && second_removed
}

fn a_shared_kernel_unmap_evicts_the_live_user_root() -> bool {
    const FIRST: u8 = 0xa6;
    const SECOND: u8 = 0x59;

    let Some(probe) =
        thread::boot_stack_layout().and_then(|layout| layout.end().checked_add(24 * PAGE_SIZE))
    else {
        return false;
    };

    let Ok(mut observer) = AddressSpace::for_user() else {
        return false;
    };
    let checked = (|| {
        if observer.lookup(probe).is_some() {
            return false;
        }
        let Some(mut first) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !first.zero() || !first.write_at(0, &[FIRST]) {
            return false;
        }
        let first_physical = first.address();
        let flags = Flags::WRITABLE.union(Flags::NO_EXECUTE);
        let mut context = context();
        let Some(Ok(mapping)) = mem::map_owned_for_test(&mut context, probe, first, flags) else {
            return false;
        };
        let mut live_mapping = Some(mapping);

        let observed = observer
            .with_root_for_test(|| {
                let warmed = paging::observe_owned_byte_for_test(probe) == Some(FIRST);
                let Some(mapping) = live_mapping.take() else {
                    return false;
                };
                let first_taken = match mem::unmap_for_test(&mut context, mapping) {
                    Some(Ok(taken)) => taken,
                    _ => panic!("the shared-root fixture could not remove its first leaf"),
                };

                let Some(mut second) = mm::allocate_contiguous(1, 1) else {
                    return false;
                };
                if !second.zero() || !second.write_at(0, &[SECOND]) {
                    return false;
                }
                let second_physical = second.address();
                let Some(Ok(second_mapping)) =
                    mem::map_owned_for_test(&mut context, probe, second, flags)
                else {
                    return false;
                };
                let table_changed = first_physical != second_physical
                    && paging::translate(probe)
                        .is_some_and(|(physical, _)| physical == second_physical);
                let replacement = paging::observe_owned_byte_for_test(probe) == Some(SECOND);
                let second_taken = match mem::unmap_for_test(&mut context, second_mapping) {
                    Some(Ok(taken)) => taken,
                    _ => panic!("the shared-root fixture could not remove its replacement leaf"),
                };
                drop(second_taken);
                drop(first_taken);
                warmed && table_changed && replacement
            })
            .unwrap_or(false);

        let cleaned = match live_mapping.take() {
            Some(mapping) => match mem::unmap_for_test(&mut context, mapping) {
                Some(Ok(taken)) => {
                    drop(taken);
                    true
                }
                _ => panic!("the shared-root fixture could not clean its unused leaf"),
            },
            None => true,
        };
        observed && cleaned && paging::translate(probe).is_none()
    })();
    let reclaimed = observer.reclaim(None).is_ok();
    checked && reclaimed
}

fn an_inactive_lower_root_unmap_preserves_the_live_observer() -> bool {
    const AT: u64 = 0x0000_0500_0000_0000;
    const OBSERVER: u8 = 0x3c;
    const VICTIM: u8 = 0xc3;

    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut observer) = AddressSpace::for_user() else {
        return false;
    };
    let Ok(mut victim) = AddressSpace::for_user() else {
        if let Err(reason) = observer.reclaim(None) {
            warn!("selftest: observer cleanup after victim-space setup refusal: {reason}");
        }
        return false;
    };

    let checked = (|| {
        if observer.root() == victim.root() {
            return false;
        }
        let (Some(mut observer_page), Some(mut victim_page)) =
            (mm::allocate_contiguous(1, 1), mm::allocate_contiguous(1, 1))
        else {
            return false;
        };
        if !observer_page.zero()
            || !observer_page.write_at(0, &[OBSERVER])
            || !victim_page.zero()
            || !victim_page.write_at(0, &[VICTIM])
        {
            return false;
        }
        let observer_physical = observer_page.address();
        let victim_physical = victim_page.address();
        let flags = Flags::WRITABLE.union(Flags::NO_EXECUTE);
        if observer.map_user(AT, observer_page.into(), flags).is_err()
            || victim.map_user(AT, victim_page.into(), flags).is_err()
        {
            return false;
        }
        let leaves_are_distinct = observer_physical != victim_physical
            && observer
                .lookup(AT)
                .is_some_and(|(physical, _)| physical == observer_physical)
            && victim
                .lookup(AT)
                .is_some_and(|(physical, _)| physical == victim_physical);
        if !leaves_are_distinct {
            return false;
        }

        observer
            .with_root_for_test(|| {
                let mut byte = [0];
                let warmed = usercopy::copy_from_user(AT, &mut byte).is_ok() && byte == [OBSERVER];
                let warmed_translation = paging::translate(AT);
                let warmed_leaf = observer.lookup(AT);
                let invalidations_before = cpu::page_invalidations_for_test();
                let removed = victim.unmap(AT, PAGE_SIZE).map(drop).is_ok();
                let no_eviction = cpu::page_invalidations_for_test() == invalidations_before;
                byte = [0];
                let observer_unchanged = paging::translate(AT) == warmed_translation
                    && observer.lookup(AT) == warmed_leaf
                    && usercopy::copy_from_user(AT, &mut byte).is_ok()
                    && byte == [OBSERVER];
                warmed
                    && warmed_translation.is_some()
                    && warmed_translation == warmed_leaf
                    && removed
                    && victim.lookup(AT).is_none()
                    && no_eviction
                    && observer_unchanged
            })
            .unwrap_or(false)
    })();

    let observer_cleaned =
        observer.lookup(AT).is_none() || observer.unmap(AT, PAGE_SIZE).map(drop).is_ok();
    let victim_cleaned =
        victim.lookup(AT).is_none() || victim.unmap(AT, PAGE_SIZE).map(drop).is_ok();
    let observer_reclaimed = observer.reclaim(None).is_ok();
    let victim_reclaimed = victim.reclaim(None).is_ok();
    drop(observer);
    drop(victim);
    let allocator_after = AllocatorSnapshot::capture();

    checked
        && observer_cleaned
        && victim_cleaned
        && observer_reclaimed
        && victim_reclaimed
        && allocator_after.has_same_ownership(&allocator_before)
}

fn bytes_cross_the_boundary_only_where_they_may() -> bool {
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

fn a_full_ring_admits_again_only_after_userspace_reaps() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let (_, submissions, completions) = queue.shape();

    // Fill the completion queue by serving as many rounds as it holds.
    let mut posted = 0u32;
    while posted < completions {
        let room = submissions as usize;
        for slot in 0..room {
            queue.post_submission_for_test(
                slot,
                queue::Submission::simple(queue::Opcode::Echo, posted, u64::from(posted)),
            );
        }
        queue.publish_tail_for_test(posted + room as u32);
        let served = queue.serve(&shared_handle_table_or_false!(8));
        if served == 0 {
            break;
        }
        posted += served as u32;
    }

    // Nothing has been reaped, so nothing more can be admitted: every slot is
    // either spoken for or already holds a completion.
    let refuses = queue.reserve_for_test(1) == 0 && queue.is_consistent();

    // A strict-prefix claim must not imply that unread completions were consumed.
    let head_before = queue.completion_head();
    let partial = posted / 2;
    queue.publish_completion_head_for_test(partial);
    let reaped = queue.reap_up_to(queue.completion_head());
    let followed = head_before == 0 && reaped == partial as usize;

    // Room for exactly what was reaped, and no more.
    let resumes = queue.reserve_for_test(partial as usize) == partial as usize
        && queue.reserve_for_test(1) == 0
        && queue.is_consistent();

    // The user head is an untrusted claim; an out-of-range claim reaps nothing.
    let Some(liar) = queue::Queue::new() else {
        return false;
    };
    for slot in 0..2 {
        liar.post_submission_for_test(
            slot,
            queue::Submission::simple(queue::Opcode::Echo, slot as u32, 0),
        );
    }
    liar.publish_tail_for_test(2);
    if liar.serve(&shared_handle_table_or_false!(8)) != 2 {
        return false;
    }
    liar.publish_completion_head_for_test(u32::MAX);
    let far_ahead = liar.reap_up_to(liar.completion_head()) == 0;
    let one = liar.reap_up_to(1) == 1;
    let just_behind = liar.reap_up_to(0) == 0;
    let lied = far_ahead && one && just_behind && liar.is_consistent();

    let quiet_completion_counted =
        queue::Queue::new().is_some_and(|queue| queue.quiet_completion_is_counted_for_test());
    posted > 0 && partial > 0 && refuses && followed && resumes && lied && quiet_completion_counted
}

fn a_ring_announces_its_shape_and_keeps_its_pages() -> bool {
    /// A user address nothing else uses.
    const AT: u64 = user::STACK + 0x60_0000;

    let Some(mut queue) = queue::Queue::new() else {
        return false;
    };
    // Compare the exported shape, not `Queue::new`'s constructor invariants.
    let (version, submissions, completions) = queue.shape();

    // Not yet decided: that the header page was published to match, which is
    // the whole ABI a program reads.
    let announced = queue.published_shape() == (version, submissions, completions)
        && version == ring_proto::VERSION;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        if Arc::get_mut(&mut queue).is_none_or(|queue| queue.install(space, AT).is_err()) {
            return false;
        }

        let reachable = space.lookup(AT).is_some_and(|(_, flags)| {
            flags.contains(Flags::USER) && flags.contains(Flags::WRITABLE)
        });

        // And cannot give them away. `unmap` reads the owned bit before minting a
        // `Segment`, and these frames were never the space's to own.
        let unowned = matches!(space.unmap(AT, PAGE_SIZE), Err(MapError::NotOwned(AT)));

        announced && reachable && unowned
    })
}

fn a_word_that_does_not_fit_is_refused_rather_than_truncated() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let held = crate::object::bootstrap_identity();
    let table = shared_handle_table_or_false!(8);
    let full = Rights::UNIVERSAL;
    let Ok(source) = table.insert(held, full) else {
        return false;
    };

    // A rights word whose low half is perfectly valid and whose high half is
    // not. A cast reads it as the low half; `try_from` refuses it whole.
    queue.post_submission_for_test(
        0,
        queue::Submission::about(
            queue::Opcode::Duplicate,
            0xC001,
            source,
            (0xFFFF_FFFFu64 << 32) | u64::from(full.bits()),
        ),
    );

    // An operation to cancel, and a token that is not its token because it
    // does not fit in one.
    queue.post_submission_for_test(
        1,
        queue::Submission::simple(queue::Opcode::Timer, 0xDEAD, 3_600_000_000_000),
    );
    queue.publish_tail_for_test(2);
    if queue.serve(&table) != 1 {
        return false;
    }
    let widened = queue.read_completion(0);

    queue.post_submission_for_test(
        2,
        queue::Submission::simple(queue::Opcode::Cancel, 0xC002, (1u64 << 32) | 0xDEAD),
    );
    queue.publish_tail_for_test(3);
    if queue.serve(&table) != 1 {
        return false;
    }
    let cancelled = queue.read_completion(1);

    // A file offset past every file this kernel can hold.
    let Some(file) = open_the_small_file_in(&queue, &table, 3) else {
        return false;
    };
    queue.post_submission_for_test(
        4,
        queue::Queue::about_at(
            queue::Opcode::Read,
            0xC003,
            file,
            u64::from(u32::MAX),
            512,
            16,
        ),
    );
    queue.post_submission_for_test(
        5,
        queue::Queue::about_at(queue::Opcode::Read, 0xC004, file, 1u64 << 32, 528, 16),
    );
    queue.publish_tail_for_test(6);
    let before_read = queue.served_count();
    if queue.serve(&table) != 0 || !runs_until(5_000, || queue.served_count() == before_read + 2) {
        return false;
    }
    let at_max = queue.read_completion(3);
    let past = queue.read_completion(4);

    let mut untouched = [0u8; 32];
    if !queue.read_data_for_test(512, &mut untouched) {
        return false;
    }

    widened.status == queue::STATUS_DENIED
        // The timer is still in flight: the cancellation named no operation,
        // which is what a token that does not fit names.
        && cancelled.status == queue::STATUS_NOT_FOUND
        && queue.in_flight() == 1
        // And the read answered "nothing", rather than the file's first bytes.
        && at_max.status == queue::STATUS_OK
        && at_max.value == 0
        && past.status == queue::STATUS_OK
        && past.value == 0
        && untouched == [0u8; 32]
}

fn open_the_small_file_in(
    queue: &Arc<queue::Queue>,
    table: &alloc::sync::Arc<crate::handle::HandleTable>,
    slot: usize,
) -> Option<u32> {
    let root = vfs::root()?;
    let object = crate::object::directory(root);
    let directory = table
        .insert(object, Rights::UNIVERSAL.union(Rights::TRAVERSE))
        .ok()?;

    if !queue.write_data_for_test(0, SMALL_NAME) {
        return None;
    }
    queue.post_submission_for_test(
        slot,
        queue::Queue::about_at(
            queue::Opcode::OpenAt,
            0x0F11,
            directory,
            0,
            0,
            SMALL_NAME.len() as u32,
        ),
    );
    queue.publish_tail_for_test(slot as u32 + 1);
    if queue.serve(table) != 1 {
        return None;
    }
    let completion = queue.read_completion(2);
    (completion.status == queue::STATUS_OK).then_some(completion.value as u32)
}

fn a_completion_carries_a_file_handle() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some(root) = vfs::root() else {
        return false;
    };
    let object = crate::object::directory(root);

    let table = shared_handle_table_or_false!(8);
    let held = Rights::UNIVERSAL
        .union(Rights::TRAVERSE)
        .union(Rights::ENUMERATE);
    let Ok(directory) = table.insert(object, held) else {
        return false;
    };

    if !queue.write_data_for_test(0, SMALL_NAME) {
        return false;
    }
    queue.post_submission_for_test(
        0,
        queue::Queue::about_at(
            queue::Opcode::OpenAt,
            0x09E4,
            directory,
            0,
            0,
            SMALL_NAME.len() as u32,
        ),
    );
    queue.publish_tail_for_test(1);

    if queue.serve(&table) != 1 {
        return false;
    }
    let completion = queue.read_completion(0);
    if completion.status != queue::STATUS_OK {
        return false;
    }

    let file = completion.value as u32;
    let Ok((object, granted)) = table.resolve(file) else {
        return false;
    };
    if object.as_file().is_none() {
        return false;
    }

    // **Read, and never write.** There is no write path, and a right that
    // grants nothing is a syscall name in disguise (D-78).
    granted == Rights::UNIVERSAL.union(Rights::READ)
        && !granted.contains(Rights::WRITE)
        && !granted.contains(Rights::TRAVERSE)
        && table.live() == 2
        && table.promised() == 0
}

fn opening_a_node_does_not_restore_universal_rights() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some(root) = vfs::root() else {
        return false;
    };
    let object = crate::object::directory(root);

    let table = shared_handle_table_or_false!(8);
    let Ok(directory) = table.insert(object, Rights::TRAVERSE) else {
        return false;
    };
    const DIRECTORY_AT: u64 = 64;
    if !queue.write_data_for_test(0, SMALL_NAME)
        || !queue.write_data_for_test(DIRECTORY_AT as usize, DIRECTORY_NAME)
    {
        return false;
    }
    for (slot, token, at, name) in [
        (0, 0xA771, 0, SMALL_NAME),
        (1, 0xA772, DIRECTORY_AT, DIRECTORY_NAME),
    ] {
        queue.post_submission_for_test(
            slot,
            queue::Queue::about_at(
                queue::Opcode::OpenAt,
                token,
                directory,
                0,
                at,
                name.len() as u32,
            ),
        );
    }
    queue.publish_tail_for_test(2);
    if queue.serve(&table) != 2 {
        return false;
    }

    let file = queue.read_completion(0);
    let subdirectory = queue.read_completion(1);
    if file.status != queue::STATUS_OK || subdirectory.status != queue::STATUS_OK {
        return false;
    }
    let Ok((file, file_rights)) = table.resolve(file.value as u32) else {
        return false;
    };
    let Ok((subdirectory, directory_rights)) = table.resolve(subdirectory.value as u32) else {
        return false;
    };

    file.as_file().is_some()
        && file_rights == Rights::READ
        && subdirectory.as_directory().is_some()
        && directory_rights == Rights::TRAVERSE
        && table.live() == 3
        && table.promised() == 0
}

fn a_read_is_a_submission_with_its_own_offset() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some(file) = open_the_small_file(&queue) else {
        return false;
    };
    let (table, handle) = file;

    // Two reads, at different offsets, into different parts of the data area.
    for (slot, token, offset, at, length) in [
        (1usize, 0x00A1u32, 0u64, 64u64, 16u32),
        (2, 0x00A2, 8, 128, 16),
    ] {
        queue.post_submission_for_test(
            slot,
            queue::Queue::about_at(queue::Opcode::Read, token, handle, offset, at, length),
        );
    }
    queue.publish_tail_for_test(3);
    let before = queue.served_count();
    if queue.serve(&table) != 0 || !runs_until(5_000, || queue.served_count() == before + 2) {
        return false;
    }

    let first = queue.read_completion(1);
    let second = queue.read_completion(2);
    if first.status != queue::STATUS_OK || second.status != queue::STATUS_OK {
        return false;
    }
    if first.value != 16 || second.value != 16 {
        return false;
    }

    let mut head = [0u8; 16];
    let mut later = [0u8; 16];
    if !queue.read_data_for_test(64, &mut head) || !queue.read_data_for_test(128, &mut later) {
        return false;
    }

    // The bytes are the file's, at the offsets asked for — and the two differ,
    // which is what a read that ignored the offset could not manage.
    head == SMALL_CONTENT[..16] && later == SMALL_CONTENT[8..24] && head != later
}

fn small_file_authority() -> Option<(Arc<HandleTable>, u32)> {
    let root = vfs::root()?;
    let vfs::Node::File(file) = vfs::resolve(root, SMALL_NAME).ok()? else {
        return None;
    };
    let table = Arc::new(HandleTable::new(16).ok()?);
    let handle = table
        .insert(
            crate::object::file(file),
            Rights::UNIVERSAL.union(Rights::READ),
        )
        .ok()?;
    Some((table, handle))
}

fn file_work_pressure_is_bounded_at_admission() -> bool {
    let Some((table, handle)) = small_file_authority() else {
        return false;
    };
    let Some(work) = queue::Queue::new() else {
        return false;
    };
    let Some(overflow) = queue::Queue::new() else {
        return false;
    };

    queue::pause_worker_for_test(true);
    let runs_before = queue::worker_runs_for_test();
    for slot in 0..8 {
        work.post_submission_for_test(
            slot,
            queue::Queue::about_at(
                queue::Opcode::Read,
                0x8100 + slot as u32,
                handle,
                slot as u64,
                0,
                16,
            ),
        );
    }
    work.publish_tail_for_test(8);
    let eight_wait =
        work.serve(&table) == 0 && work.in_flight() == 8 && queue::queued_work_for_test() == 8;

    overflow.post_submission_for_test(
        0,
        queue::Queue::about_at(queue::Opcode::Read, 0x8199, handle, 0, 0, 16),
    );
    overflow.publish_tail_for_test(1);
    let ninth_refused = overflow.serve(&table) == 1
        && overflow.read_completion(0).status == queue::STATUS_NO_ROOM
        && overflow.in_flight() == 0;

    queue::pause_worker_for_test(false);
    let drained = runs_until(10_000, || {
        work.is_quiet() && queue::queued_work_for_test() == 0
    });
    let mut exact = true;
    for slot in 0..8 {
        let completion = work.read_completion(slot);
        exact &= completion.token == 0x8100 + slot as u32
            && completion.status == queue::STATUS_OK
            && completion.value == 16;
    }

    eight_wait
        && ninth_refused
        && drained
        && exact
        && queue::worker_runs_for_test() == runs_before + 8
        && work.outstanding() == 0
        && work.is_consistent()
}

fn a_delayed_read_preserves_interrupt_admission_and_exact_teardown() -> bool {
    let Some((table, handle)) = small_file_authority() else {
        return false;
    };
    let Some(ring) = queue::Queue::new() else {
        return false;
    };
    ring.post_submission_for_test(
        0,
        queue::Queue::about_at(queue::Opcode::Read, 0xD200, handle, 0, 64, 16),
    );
    ring.publish_tail_for_test(1);

    nvme::hold_command_for_test(true);
    let served_before = ring.served_count();
    let deferred = ring.serve(&table) == 0 && ring.in_flight() == 1;
    let reached_device = runs_until(2_000, nvme::command_is_held_for_test);
    let retained = ring.abandon() == 0 && !ring.is_quiet() && Arc::strong_count(&ring) > 1;

    let interrupt_admission = interrupt::are_enabled();
    let key = input::arm_key(ps2_keys::Key::C);
    // Kept in step with STORAGE_DELAY_MARKER in `xtask/src/qemu.rs`.
    awaiting(
        "selftest: awaiting storage-delay keystroke",
        "  PRESS  c   to continue the storage round",
    );
    // This self-test runs before the permanent fallback starts so the scheduler
    // checks below retain their tickless baseline. Drain explicitly while the
    // storage worker is held, proving polling and unrelated IRQ admission can
    // progress together.
    let progressed = runs_until(input::HUMAN_WAIT_MS, || {
        input::poll();
        key.is_some_and(input::armed_key_arrived)
    });
    let mut context = context();
    let pressed = input::read_key(&mut context);
    let released = input::read_key(&mut context);
    let input_progress = progressed
        && interrupt_admission
        && interrupt::are_enabled()
        && pressed.event.key == ps2_keys::Key::C
        && pressed.event.state == ps2_keys::KeyState::Pressed
        && released.event.key == ps2_keys::Key::C
        && released.event.state == ps2_keys::KeyState::Released;

    nvme::hold_command_for_test(false);
    let completed = runs_until(5_000, || ring.is_quiet());
    let completion = ring.read_completion(0);
    let mut bytes = [0u8; 16];
    let copied = ring.read_data_for_test(64, &mut bytes);
    let exact = ring.served_count() == served_before + 1
        && completion.token == 0xD200
        && completion.status == queue::STATUS_OK
        && completion.value == 16
        && copied
        && bytes == SMALL_CONTENT[..16]
        && ring.outstanding() == 0
        && ring.abandon() == 0;
    info!(
        "selftest: delayed read evidence deferred={deferred} device={reached_device} retained={retained} polled={input_progress} completed={completed} exact={exact}"
    );

    deferred && reached_device && retained && input_progress && completed && exact
}

fn a_ring_operation_cannot_reach_past_its_rights_or_its_pages() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some(root) = vfs::root() else {
        return false;
    };
    let table = shared_handle_table_or_false!(8);

    // A directory held without TRAVERSE: it can be listed and not walked.
    let object = crate::object::directory(root);
    let Ok(unwalkable) = table.insert(object, Rights::UNIVERSAL.union(Rights::ENUMERATE)) else {
        return false;
    };
    // And one that can be walked, for the paths that are about the path.
    let object = crate::object::directory(root);
    let Ok(walkable) = table.insert(object, Rights::UNIVERSAL.union(Rights::TRAVERSE)) else {
        return false;
    };

    if !queue.write_data_for_test(0, SMALL_NAME) || !queue.write_data_for_test(256, b"..") {
        return false;
    }

    // A file held without READ, so a read of it is a refusal about the right
    // rather than about the file.
    let Some(root_node) = vfs::resolve(root, SMALL_NAME).ok() else {
        return false;
    };
    let vfs::Node::File(file) = root_node else {
        return false;
    };
    let object = crate::object::file(file);
    let Ok(unreadable) = table.insert(object, Rights::UNIVERSAL) else {
        return false;
    };
    // Grant `READ` so this case reaches the buffer bound (D-173).
    let object = crate::object::file(file);
    let Ok(readable) = table.insert(object, Rights::UNIVERSAL.union(Rights::READ)) else {
        return false;
    };

    let asked = [
        // No TRAVERSE.
        (
            0usize,
            0xD1u32,
            unwalkable,
            0u64,
            SMALL_NAME.len() as u32,
            queue::STATUS_DENIED,
        ),
        // A path that would leave the directory.
        (1, 0xD2, walkable, 256, 2, queue::STATUS_BAD_PATH),
        // A path buffer outside the data area.
        (2, 0xD3, walkable, 8192, 8, queue::STATUS_BAD_BUFFER),
        // A path of no length at all.
        (3, 0xD4, walkable, 0, 0, queue::STATUS_BAD_BUFFER),
    ];
    for (slot, token, handle, at, length, _) in asked {
        queue.post_submission_for_test(
            slot,
            queue::Queue::about_at(queue::Opcode::OpenAt, token, handle, 0, at, length),
        );
    }
    queue.publish_tail_for_test(asked.len() as u32);
    if queue.serve(&table) != asked.len() {
        return false;
    }

    let mut refused = true;
    for (slot, token, _, _, _, status) in asked {
        let completion = queue.read_completion(slot);
        refused &= completion.token == token && completion.status == status;
    }

    // Device-access counts distinguish preflight refusal from a late error
    // after I/O (D-173).
    let before = nvme::command_count_for_test();
    queue.post_submission_for_test(
        4,
        queue::Queue::about_at(queue::Opcode::Read, 0xD5, unreadable, 0, 0, 16),
    );
    queue.post_submission_for_test(
        5,
        queue::Queue::about_at(queue::Opcode::Read, 0xD6, unreadable, 0, 4090, 16),
    );
    // The one that reaches the bound: the device must still not be asked, or
    // the bound is checked with the work already done.
    queue.post_submission_for_test(
        6,
        queue::Queue::about_at(queue::Opcode::Read, 0xD7, readable, 0, 4090, 16),
    );
    queue.publish_tail_for_test(7);
    let served = queue.serve(&table) == 3;
    let after = nvme::command_count_for_test();

    let denied = queue.read_completion(4).status == queue::STATUS_DENIED
        && queue.read_completion(5).status == queue::STATUS_DENIED
        && queue.read_completion(6).status == queue::STATUS_BAD_BUFFER;
    let untouched = before == after;

    // Nothing was promised and kept: every refusal above gave its slot back.
    refused && served && denied && untouched && table.live() == 4 && table.promised() == 0
}

fn open_the_small_file(
    queue: &Arc<queue::Queue>,
) -> Option<(alloc::sync::Arc<crate::handle::HandleTable>, u32)> {
    let root = vfs::root()?;
    let object = crate::object::directory(root);
    let table = alloc::sync::Arc::new(crate::handle::HandleTable::new(8).ok()?);
    let held = Rights::UNIVERSAL.union(Rights::TRAVERSE);
    let directory = table.insert(object, held).ok()?;

    if !queue.write_data_for_test(0, SMALL_NAME) {
        return None;
    }
    queue.post_submission_for_test(
        0,
        queue::Queue::about_at(
            queue::Opcode::OpenAt,
            1,
            directory,
            0,
            0,
            SMALL_NAME.len() as u32,
        ),
    );
    queue.publish_tail_for_test(1);
    if queue.serve(&table) != 1 {
        return None;
    }
    let completion = queue.read_completion(0);
    (completion.status == queue::STATUS_OK).then_some((table, completion.value as u32))
}

fn a_second_backend_answers_the_same_three_operations() -> bool {
    let root = vfs::tmp_root();

    // Resolve and read a file at the top.
    let Ok(vfs::Node::File(motd)) = vfs::resolve(root, b"MOTD") else {
        return false;
    };
    let mut read = [0u8; 64];
    let Ok(got) = vfs::read(motd, 0, &mut read) else {
        return false;
    };
    if got == 0 || &read[..got] != b"the wizard's bedrock, from memory\n" {
        return false;
    }

    // A read from the middle starts where it was told, and stops at the end
    // rather than at the arena's.
    let mut window = [0u8; 8];
    let Ok(partial) = vfs::read(motd, 4, &mut window) else {
        return false;
    };
    if partial != window.len() || window[..] != read[4..12] {
        return false;
    }
    if vfs::read(motd, got as u32, &mut window) != Ok(0) {
        return false;
    }

    // Resolve through a directory, which is the walk rather than a lookup.
    let Ok(vfs::Node::File(nested)) = vfs::resolve(root, b"NOTES/ONE.TXT") else {
        return false;
    };
    let Ok(inside) = vfs::read(nested, 0, &mut read) else {
        return false;
    };
    if &read[..inside] != b"a file in a directory\n" {
        return false;
    }

    // A name in two directories resolves to the right one; a lookup ignoring
    // the parent is only wrong when names repeat, so here they do (D-172).
    let Ok(vfs::Node::File(shadowed)) = vfs::resolve(root, b"NOTES/MOTD") else {
        return false;
    };
    let Ok(shadow) = vfs::read(shadowed, 0, &mut read) else {
        return false;
    };
    if &read[..shadow] != b"a different motd\n" {
        return false;
    }

    // And enumeration agrees with what resolves.
    let mut names = 0usize;
    let mut directories = 0usize;
    if vfs::enumerate(root, |_, is_directory| {
        names += 1;
        if is_directory {
            directories += 1;
        }
        true
    })
    .is_err()
    {
        return false;
    }

    // A file walked through is refused here too, which is the rule being the
    // VFS's rather than FAT's.
    let through = vfs::resolve(root, b"MOTD/ONE.TXT");
    let hostile = vfs::read(
        vfs::File::Tmp {
            at: usize::MAX,
            len: usize::MAX,
        },
        1,
        &mut window,
    );
    names == 2
        && directories == 1
        && matches!(through, Err(vfs::VfsError::NotADirectory))
        && hostile == Ok(0)
        && tmpfs::unwritten_bytes_are_refused_for_test()
}

static ENUMERATION_SENTINEL: SpinLock<()> = SpinLock::new(());

fn enumeration_callbacks_run_without_backend_ownership() -> bool {
    let tmp = vfs::tmp_root();
    let mut tmp_reentered = false;
    let tmp_walked = vfs::enumerate(tmp, |name, _| {
        {
            let _sentinel = ENUMERATION_SENTINEL.lock_leaf();
        }
        if wost::sync::taken_under_a_leaf().is_some() {
            return false;
        }
        tmp_reentered = vfs::resolve(tmp, name).is_ok();
        false
    });

    let Some(fat) = vfs::root() else {
        return false;
    };
    let mut fat_reentered = false;
    let fat_walked = vfs::enumerate(fat, |name, _| {
        if !nvme::controller_is_unlocked_for_test() {
            return false;
        }
        fat_reentered = vfs::resolve(fat, name).is_ok();
        false
    });

    tmp_walked.is_ok() && tmp_reentered && fat_walked.is_ok() && fat_reentered
}

fn a_device_is_not_a_file() -> bool {
    let root = vfs::dev_root();

    let Ok(vfs::Node::Device(storage)) = vfs::resolve(root, b"storage") else {
        return false;
    };
    if storage.name() != b"storage" {
        return false;
    }
    // Not a file and not a directory: the *node* is a third thing, which is
    // what makes the distinction structural rather than a flag.
    if matches!(vfs::resolve(root, b"console"), Ok(vfs::Node::File(_))) {
        return false;
    }
    let Ok(vfs::Node::Device(console)) = vfs::resolve(root, b"console") else {
        return false;
    };
    if console == storage {
        return false;
    }

    // Every device is listed, and none of them claims to be a directory.
    let mut listed = 0usize;
    let mut any_directory = false;
    if vfs::enumerate(root, |_, is_directory| {
        listed += 1;
        any_directory |= is_directory;
        true
    })
    .is_err()
    {
        return false;
    }

    // A device has nothing inside it, so a path through one is refused for the
    // same reason a path through a file is.
    let through = vfs::resolve(root, b"storage/anything");
    let missing = vfs::resolve(root, b"nothing");

    listed == devfs::DEVICES.len()
        && !any_directory
        && matches!(through, Err(vfs::VfsError::NotADirectory))
        && matches!(missing, Err(vfs::VfsError::NotFound))
}

fn no_backend_lets_a_path_escape_or_cross() -> bool {
    let escaping: &[&[u8]] = &[b"..", b"../MOTD", b"NOTES/../..", b"./.."];
    let absolute: &[&[u8]] = &[b"/", b"/MOTD"];

    for root in [vfs::tmp_root(), vfs::dev_root()] {
        for path in escaping {
            if !matches!(
                vfs::resolve(root, path),
                Err(vfs::VfsError::Path(path_walk::PathError::Escapes))
            ) {
                return false;
            }
        }
        for path in absolute {
            if !matches!(
                vfs::resolve(root, path),
                Err(vfs::VfsError::Path(path_walk::PathError::Absolute))
            ) {
                return false;
            }
        }
    }

    // Names that exist on one backend and not another. Each of these resolves
    // from its own root and from nowhere else.
    let crossings: &[(vfs::Directory, &[u8])] = &[
        (vfs::tmp_root(), SMALL_NAME),
        (vfs::tmp_root(), b"storage"),
        (vfs::dev_root(), b"MOTD"),
        (vfs::dev_root(), SMALL_NAME),
    ];
    for (root, name) in crossings {
        if !matches!(vfs::resolve(*root, name), Err(vfs::VfsError::NotFound)) {
            return false;
        }
    }

    // And each name does resolve where it belongs, so the refusals above are
    // about the backend rather than about the name.
    vfs::resolve(vfs::tmp_root(), b"MOTD").is_ok()
        && vfs::resolve(vfs::dev_root(), b"storage").is_ok()
}

fn a_path_resolves_from_a_directory_handle() -> bool {
    let Some(root) = vfs::root() else {
        return false;
    };

    // A file at the top.
    let Ok(vfs::Node::File(small)) = vfs::resolve(root, SMALL_NAME) else {
        return false;
    };
    // The variant matters: a FAT root resolves to a FAT file, and the match
    // is the enum dispatch being exhaustive (D-172).
    let vfs::File::Fat { size, .. } = small else {
        return false;
    };
    if size as usize != SMALL_CONTENT.len() {
        return false;
    }
    let mut read = [0u8; fat32::SECTOR_BYTES];
    let Ok(got) = vfs::read(small, 0, &mut read) else {
        return false;
    };
    if got != SMALL_CONTENT.len() || &read[..got] != SMALL_CONTENT {
        return false;
    }

    // A directory, then a file inside it, through one path.
    let Ok(vfs::Node::Directory(_)) = vfs::resolve(root, DIRECTORY_NAME) else {
        return false;
    };
    let mut deeper = [0u8; 32];
    deeper[..DIRECTORY_NAME.len()].copy_from_slice(DIRECTORY_NAME);
    deeper[DIRECTORY_NAME.len()] = b'/';
    let tail = DIRECTORY_NAME.len() + 1;
    deeper[tail..tail + NESTED_NAME.len()].copy_from_slice(NESTED_NAME);
    let path = &deeper[..tail + NESTED_NAME.len()];

    let Ok(vfs::Node::File(nested)) = vfs::resolve(root, path) else {
        return false;
    };
    let mut inside = [0u8; 64];
    let Ok(got) = vfs::read(nested, 0, &mut inside) else {
        return false;
    };

    // And `.` and a cancelled `..` both name the directory they started from.
    let here = matches!(vfs::resolve(root, b"."), Ok(vfs::Node::Directory(_)));
    let round_trip = matches!(vfs::resolve(root, b"DATA/.."), Ok(vfs::Node::Directory(_)));

    got == NESTED_CONTENT.len() && &inside[..got] == NESTED_CONTENT && here && round_trip
}

fn no_path_can_name_anything_above_the_handle() -> bool {
    let Some(root) = vfs::root() else {
        return false;
    };

    let absolute: &[&[u8]] = &[b"/", b"/DATA", b"//DATA", b"/../DATA", b"/DATA/NESTED.TXT"];
    for path in absolute {
        if !matches!(
            vfs::resolve(root, path),
            Err(vfs::VfsError::Path(path_walk::PathError::Absolute))
        ) {
            return false;
        }
    }

    let escaping: &[&[u8]] = &[
        b"..",
        b"../DATA",
        b"DATA/../..",
        b"DATA/../../DATA",
        b"./..",
        b"DATA/./../..",
        b"DATA/../DATA/../..",
    ];
    for path in escaping {
        if !matches!(
            vfs::resolve(root, path),
            Err(vfs::VfsError::Path(path_walk::PathError::Escapes))
        ) {
            return false;
        }
    }

    // And an empty path names nothing rather than the directory itself, which
    // is what a resolver that treated it as `.` would answer.
    matches!(
        vfs::resolve(root, b""),
        Err(vfs::VfsError::Path(path_walk::PathError::Empty))
    )
}

fn a_file_cannot_be_walked_through() -> bool {
    let Some(root) = vfs::root() else {
        return false;
    };

    let mut path = [0u8; 32];
    path[..SMALL_NAME.len()].copy_from_slice(SMALL_NAME);
    path[SMALL_NAME.len()] = b'/';
    let tail = SMALL_NAME.len() + 1;
    path[tail..tail + NESTED_NAME.len()].copy_from_slice(NESTED_NAME);

    let through = vfs::resolve(root, &path[..tail + NESTED_NAME.len()]);
    let missing = vfs::resolve(root, b"NOPE.TXT");

    matches!(through, Err(vfs::VfsError::NotADirectory))
        && matches!(missing, Err(vfs::VfsError::NotFound))
}

fn enumeration_lists_what_traversal_would_find() -> bool {
    let Some(root) = vfs::root() else {
        return false;
    };

    let mut names = 0usize;
    let mut directories = 0usize;
    let mut saw_small = false;
    let mut saw_large = false;
    if vfs::enumerate(root, |name, is_directory| {
        names += 1;
        if is_directory {
            directories += 1;
        }
        saw_small |= name == SMALL_NAME;
        saw_large |= name == LARGE_NAME;
        true
    })
    .is_err()
    {
        return false;
    }

    // Resolve after enumeration because both paths take the controller leaf;
    // resolving in the callback would re-enter it (D-171).
    let mut listed = [[0u8; 12]; 4];
    let mut lengths = [0usize; 4];
    let mut count = 0usize;
    let listed_all = vfs::enumerate(root, |name, _| {
        if count < listed.len() && name.len() <= listed[count].len() {
            listed[count][..name.len()].copy_from_slice(name);
            lengths[count] = name.len();
            count += 1;
        }
        true
    })
    .is_ok();

    let mut all_resolve = listed_all && count == 3;
    for index in 0..count {
        all_resolve &= vfs::resolve(root, &listed[index][..lengths[index]]).is_ok();
    }

    names == 3 && directories == 1 && saw_small && saw_large && all_resolve
}

fn the_volume_mounts_and_says_what_a_fat32_volume_says() -> bool {
    matches!(fs::mount(), Some(Ok(volume)) if volume.layout_is_fat32())
}

fn the_kernel_reads_the_file_the_host_wrote() -> bool {
    matches!(
        fs::mount().map(|result| result.and_then(|mut volume| {
            let root = volume.root_cluster();
            let entry = volume.resolve(root, &[SMALL_NAME])?;
            if entry.size as usize != SMALL_CONTENT.len() || entry.is_directory() {
                return Ok(false);
            }

            // A deleted slot separates live entries, so traversal must continue.
            let mut live = 0usize;
            let mut cursor = fat32::DirectoryCursor::new(root);
            while !cursor.is_done() {
                live += volume.directory_chunk(&mut cursor)?.entries().len();
            }
            if live != 3 {
                return Ok(false);
            }

            // Extra buffer exposes a read incorrectly bounded by cluster size
            // instead of file size (D-170).
            let mut read = [0u8; fat32::SECTOR_BYTES];
            let got = volume.read_file(entry.cluster, entry.size, 0, &mut read)?;
            Ok(got == SMALL_CONTENT.len() && &read[..got] == SMALL_CONTENT)
        })),
        Some(Ok(true))
    )
}

fn a_file_longer_than_a_cluster_is_read_across_its_chain() -> bool {
    matches!(
        fs::mount().map(|result| result.and_then(|mut volume| {
            let root = volume.root_cluster();
            let entry = volume.resolve(root, &[LARGE_NAME])?;
            if entry.size as usize != LARGE_BYTES {
                return Ok(false);
            }
            let cluster = volume.shape().1 as usize;

            for offset in [
                0usize,
                1,
                fat32::SECTOR_BYTES - 1,
                cluster - 1,
                cluster,
                cluster + 1,
                cluster * 2 + 13,
                LARGE_BYTES - 64,
            ] {
                let mut window = [0u8; 64];
                let got =
                    volume.read_file(entry.cluster, entry.size, offset as u32, &mut window)?;
                if got != window.len() {
                    return Ok(false);
                }
                for (index, byte) in window.iter().enumerate() {
                    if *byte != large_byte(offset + index) {
                        return Ok(false);
                    }
                }
            }

            // And nothing past the end, rather than the next file's bytes.
            let mut past = [0u8; 16];
            Ok(volume.read_file(entry.cluster, entry.size, entry.size, &mut past)? == 0)
        })),
        Some(Ok(true))
    )
}

fn a_path_resolves_through_a_directory() -> bool {
    matches!(
        fs::mount().map(|result| result.and_then(|mut volume| {
            let root = volume.root_cluster();
            let nested = volume.resolve(root, &[DIRECTORY_NAME, NESTED_NAME])?;
            if nested.size as usize != NESTED_CONTENT.len() {
                return Ok(false);
            }
            let mut read = [0u8; 128];
            let got = volume.read_file(nested.cluster, nested.size, 0, &mut read)?;
            if got != NESTED_CONTENT.len() || &read[..got] != NESTED_CONTENT {
                return Ok(false);
            }

            // A file is not a directory, whatever its bytes look like.
            let through_a_file = volume.resolve(root, &[SMALL_NAME, NESTED_NAME]);
            let missing = volume.resolve(root, &[b"NOPE.TXT"]);
            Ok(through_a_file == Err(fat32::FatError::WrongKind)
                && missing == Err(fat32::FatError::NotFound))
        })),
        Some(Ok(true))
    )
}

/// What `xtask` put on the namespace.
///
/// Transcribed from `xtask/src/image/corpus.rs`, which writes it — two copies
/// of one table, so a host test compares them (D-158, D-170).
const SMALL_NAME: &[u8] = b"HELLO.TXT";
/// See [`SMALL_NAME`].
const SMALL_CONTENT: &[u8] = b"the wizard's bedrock reads a disk\n";
/// See [`SMALL_NAME`].
const LARGE_NAME: &[u8] = b"BIG.BIN";
/// See [`SMALL_NAME`].
const LARGE_BYTES: usize = 40 * 1024;
/// See [`SMALL_NAME`].
const DIRECTORY_NAME: &[u8] = b"DATA";
/// See [`SMALL_NAME`].
const NESTED_NAME: &[u8] = b"NESTED.TXT";
/// See [`SMALL_NAME`].
const NESTED_CONTENT: &[u8] = b"one directory down\n";

/// Byte `index` of [`LARGE_NAME`], as `xtask` computed it.
const fn large_byte(index: usize) -> u8 {
    ((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8
}

fn the_controller_was_read_rather_than_assumed() -> bool {
    nvme::controller_facts_hold_for_test() && nvme::bus_master_followed_disable_for_test()
}

fn nvme_candidates_preserve_boot_stack_headroom() -> bool {
    const CAPACITY: u64 = 32 * 1024;
    const MINIMUM_HEADROOM: u64 = 2 * PAGE_SIZE;

    let Some(stack) = nvme::candidate_stack_usage_for_test() else {
        return false;
    };
    info!(
        "selftest: nvme-stack v2 used={} capacity={} headroom={} leaf={}",
        stack.deepest.used(),
        stack.deepest.capacity(),
        stack.deepest.headroom(),
        stack.leaf.used()
    );
    stack.deepest.capacity() == CAPACITY && stack.deepest.headroom() >= MINIMUM_HEADROOM
}

fn a_failed_bus_master_admission_stops_before_nvme_start() -> bool {
    crate::refused_bus_master_retires_before_nvme_start()
}

fn invalid_nvme_namespace_stops_before_io_queues() -> bool {
    nvme::malformed_namespace_is_bounded_for_test()
}

fn invalid_nvme_reads_stop_before_submission() -> bool {
    nvme::malformed_reads_are_bounded_for_test()
}

fn a_block_read_returns_the_block_that_was_asked_for() -> bool {
    for lba in [0u64, 3, 1, 7] {
        let Some((block, head, beyond)) = nvme::read_block_probe_for_test(lba) else {
            return false;
        };
        if block < 8
            || u64::from_le_bytes(head) != disk_signature(lba)
            || u64::from_le_bytes(beyond) != 0
        {
            return false;
        }
    }
    true
}

fn the_queues_wrap_and_the_phase_tag_still_works() -> bool {
    let Some(entries) = nvme::queue_entries_for_test() else {
        return false;
    };
    let laps = entries * 3;
    for turn in 0..laps {
        let lba = u64::from(turn % 8);
        let Some((_, head, _)) = nvme::read_block_probe_for_test(lba) else {
            return false;
        };
        if u64::from_le_bytes(head) != disk_signature(lba) {
            return false;
        }
    }
    laps > entries
}

fn the_controllers_registers_are_uncacheable() -> bool {
    nvme::controller_window_for_test()
        .and_then(paging::translate)
        .is_some_and(|(_, flags)| {
            flags.contains(Flags::NO_CACHE) && !flags.contains(Flags::WRITE_THROUGH)
        })
}

fn an_nvme_timeout_quiesces_before_dma_memory_returns() -> bool {
    nvme::timeout_quiesces_for_test()
}

fn translation_is_active_for_the_storage_function() -> bool {
    nvme::isolation_is_active_for_test().is_some_and(|(minted, _)| minted)
}

fn every_shared_vtd_requester_was_fenced_across_controller_fallback() -> bool {
    iommu::requester_fences_cover_shared_unit_for_test()
}

fn the_unit_walks_the_kernels_root_table() -> bool {
    nvme::isolation_is_active_for_test().is_some_and(|(_, walks_kernel_root)| walks_kernel_root)
}

/// Read back from the second-level tables the unit walks, not from the
/// driver's index of them: the reachable set is exactly the live queue frames
/// while no transfer is published (D-357).
fn the_dma_domain_reaches_exactly_the_live_queue_frames() -> bool {
    nvme::reachable_frames_for_test()
        .is_some_and(|(reachable, expected)| !expected.is_empty() && reachable == expected)
}

fn a_domain_proof_is_refused_while_translation_is_off() -> bool {
    nvme::domain_proof_follows_translation_for_test()
}

/// The hook reads into the buffer once first, so a stale cached translation
/// would let the unmapped write through; the fault proves the unmap
/// invalidated the unit's cache and the device cannot reach the frame.
fn a_device_write_into_an_unmapped_frame_faults_and_changes_nothing() -> bool {
    nvme::unmapped_write_is_fenced_for_test()
        .is_some_and(|witness| witness.completed && witness.unchanged && witness.faulted)
}

fn an_uncertain_vtd_mapping_never_recycles_its_frames() -> bool {
    let before = mm::free_frames();
    let Some(segment) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let mut dma = mm::DmaSegment::from(segment);
    let reserved = mm::free_frames() == before - 1;
    let retained = {
        let mut installed = crate::iommu::installed();
        let Some(isolation) = installed.as_mut() else {
            return false;
        };
        // The first refusal makes the just-installed translation uncertain;
        // the second makes rollback uncertainty explicit.
        isolation.force_invalidation_failures_for_test(2);
        matches!(
            isolation.map_segment(&mut dma),
            Err(wost::arch::current::vtd::SegmentChangeError::Retained(_))
        ) && dma.is_retained()
    };
    drop(dma);
    reserved && retained && mm::free_frames() == before - 1
}

fn an_uncertain_vtd_unmap_never_recycles_its_frames() -> bool {
    let before = mm::free_frames();
    let Some(segment) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let mut dma = mm::DmaSegment::from(segment);
    let reserved = mm::free_frames() == before - 1;
    let retained = {
        let mut installed = crate::iommu::installed();
        let Some(isolation) = installed.as_mut() else {
            return false;
        };
        if isolation.map_segment(&mut dma).is_err() {
            return false;
        }
        // The leaf is removed, but a failed invalidation means the device may
        // still hold its old translation. Ownership must therefore disappear
        // into the DMA segment's boot-lifetime retention state.
        isolation.force_invalidation_failures_for_test(1);
        matches!(
            isolation.unmap_segment(&mut dma),
            Err(wost::arch::current::vtd::SegmentChangeError::Retained(_))
        ) && dma.is_retained()
    };
    drop(dma);
    reserved && retained && mm::free_frames() == before - 1
}

/// The screen `iommu::establish` applies admits a page wholly inside a device
/// hole and refuses a page of usable RAM; QEMU's DMAR names no region to try.
fn a_reserved_dma_region_over_kernel_memory_refuses_isolation() -> bool {
    let Some(map) = mm::memory_map_for_test() else {
        return false;
    };
    let device = |kind: boot::MemoryKind| {
        matches!(
            kind,
            boot::MemoryKind::Reserved | boot::MemoryKind::Unknown(_)
        )
    };
    let mut hole = None;
    let mut attempted = 0usize;
    'regions: for region in map.regions().filter(|region| device(region.kind)) {
        let (Some(mut physical), Some(region_end)) = (
            region.base.checked_next_multiple_of(PAGE_SIZE),
            region.base.checked_add(region.len),
        ) else {
            continue;
        };
        while let Some(end) = physical
            .checked_add(PAGE_SIZE)
            .filter(|&end| end <= region_end)
        {
            attempted += 1;
            if attempted > 256 {
                break 'regions;
            }
            if map.regions().all(|other| {
                let overlaps = physical < other.base + other.len && other.base < end;
                !overlaps || device(other.kind)
            }) {
                hole = Some(physical);
                break 'regions;
            }
            physical = end;
        }
    }
    let usable = map
        .regions()
        .filter(|region| region.kind == boot::MemoryKind::Usable)
        .find_map(|region| {
            let base = region.base.checked_next_multiple_of(PAGE_SIZE)?;
            (base.checked_add(PAGE_SIZE)? <= region.base.checked_add(region.len)?).then_some(base)
        });
    let (Some(hole), Some(usable)) = (hole, usable) else {
        return false;
    };
    iommu::reserved_regions_are_clear_for_test(&[(hole, hole + PAGE_SIZE - 1)])
        && !iommu::reserved_regions_are_clear_for_test(&[(usable, usable + PAGE_SIZE - 1)])
}

/// The first eight bytes of block `lba` on the check machine's namespace.
///
/// Written by `xtask` when it makes the backing file, and transcribed here —
/// which is two copies of one table, so a host test compares them (D-158).
const fn disk_signature(lba: u64) -> u64 {
    0x5749_5A44_0000_0000 | lba
}

fn the_bus_walk_descends_through_a_bridge() -> bool {
    let mut on_bus_zero = 0;
    let mut beyond = 0;
    let Some(survey) = pci::enumerate(|function| {
        if function.at.bus() == 0 {
            on_bus_zero += 1;
        } else {
            beyond += 1;
        }
    }) else {
        return false;
    };

    // More than one bus visited, and a bridge to explain why.
    survey.buses >= 2
        && survey.bridges >= 1
        && survey.highest_bus >= 1
        // And the counts agree with what the callback saw, so the survey is
        // reporting the walk rather than a number of its own.
        && survey.functions == on_bus_zero + beyond
        && on_bus_zero >= 6
}

static DEVICE_CLAIM_CHECK: AtomicUsize = AtomicUsize::new(0);
static DEVICE_PORT_CHECK: AtomicUsize = AtomicUsize::new(0);

/// Run the two checks that require a free PCI function before VT-d requester
/// takeover owns the complete unit. Each path returns every temporary owner;
/// only the immutable verdict crosses into the later report.
pub(crate) fn prepare_pci_claim_checks() {
    if DEVICE_CLAIM_CHECK.load(Ordering::Acquire) != 0 {
        return;
    }
    let claim = check_device_is_claimed_once_and_a_refusal_strands_nothing();
    let ports = check_claimed_devices_io_window_is_not_claimable_bare();
    DEVICE_CLAIM_CHECK.store(usize::from(claim) + 1, Ordering::Release);
    DEVICE_PORT_CHECK.store(usize::from(ports) + 1, Ordering::Release);
}

fn a_device_is_claimed_once_and_a_refusal_strands_nothing() -> bool {
    DEVICE_CLAIM_CHECK.load(Ordering::Acquire) == 2
}

fn check_device_is_claimed_once_and_a_refusal_strands_nothing() -> bool {
    // The bridge behind bus zero: it has no windows to disturb, and it is
    // there on both boot targets.
    let mut bridge = None;
    if pci::enumerate(|function| {
        if bridge.is_none() && function.class.is_bridge() && function.header.is_bridge() {
            bridge = Some(function.at);
        }
    })
    .is_none()
    {
        return false;
    }
    let Some(at) = bridge else {
        return false;
    };

    let before = pci::claims();
    let Some(alias) = at.at(pci_config::header::CLASS) else {
        return false;
    };
    pci::refuse_clean_preparation_for_test();
    let clean_refusal = matches!(pci::lease(alias), Err(pci::ClaimError::WindowTaken))
        && !pci::is_claimed(at)
        && pci::claims() == before;
    let Ok(held) = pci::lease(alias) else {
        return false;
    };
    let recorded = pci::is_claimed(at) && pci::claims() == before + 1;

    // A different register offset is still the same function and the same
    // claim, so a second driver cannot enter through an alias.
    let refused = matches!(
        pci::lease(at.at(pci_config::header::BAR0).expect("a BAR offset")),
        Err(pci::ClaimError::Taken)
    );

    // And the holder still holds it: a refusal that released what it found
    // would be indistinguishable from here without asking.
    let intact = pci::is_claimed(at)
        && pci::claims() == before + 1
        && held.at() == at.function_base()
        && held.identity().0 != 0xFFFF;

    // Bridge headers have two BARs; probing a third would overwrite bus numbers
    // before restoring them (D-168).
    let two_registers = held.window(2).is_none()
        && held.window(3).is_none()
        && held.window(4).is_none()
        && held.window(5).is_none();

    let released = held.release(&mut context()).into_retained().is_none()
        && !pci::is_claimed(at)
        && pci::claims() == before;

    clean_refusal && recorded && refused && intact && two_registers && released
}

fn a_claimed_devices_io_window_is_not_claimable_bare() -> bool {
    DEVICE_PORT_CHECK.load(Ordering::Acquire) == 2
}

fn check_claimed_devices_io_window_is_not_claimable_bare() -> bool {
    let claims_before = pci::claims();
    let mut candidate = None;
    if pci::enumerate(|function| {
        if candidate.is_none() && !function.header.is_bridge() && !pci::is_claimed(function.at) {
            candidate = Some(*function);
        }
    })
    .is_none()
    {
        return false;
    }

    // Walk the candidates until one with an I/O window is claimed; none
    // having one is a failure here, not a pass.
    let mut context = context();
    let mut found = None;
    let mut seen = 0;
    let mut retirement_clean = true;
    let surveyed = pci::enumerate(|function| {
        if found.is_some() || function.header.is_bridge() || pci::is_claimed(function.at) {
            return;
        }
        seen += 1;
        let Ok(device) = pci::lease(function.at) else {
            return;
        };
        let mut io = None;
        for index in 0..pci::BARS {
            if let Some(window) = device.window(index)
                && let pci_config::Bar::Io { port } = window.bar
                && window.size > 0
            {
                io = Some((port, window.size));
                break;
            }
        }
        if let Some((port, size)) = io {
            found = Some((device, port, size));
        } else {
            retirement_clean &= device.release(&mut context).into_retained().is_none();
        }
    })
    .is_some();

    let Some((device, port, size)) = found else {
        return false;
    };

    // Every port in the window is the device's. The first and the last, so a
    // claim that recorded only the base would be caught.
    let last = port + (size as u16 - 1);
    let base_refused = wost::arch::current::port::claim::<u8>(port).is_none();
    let last_refused = wost::arch::current::port::claim::<u8>(last).is_none();

    // Reacquiring the same function exercises both registries, then returns
    // it so the later unit-wide requester takeover can own the exact function.
    let at = device.at();
    wost::arch::current::port::refuse_span_release_for_test();
    let (reason, device) = match device.release(&mut context).into_retained() {
        None => return false,
        Some(retained) => retained,
    };
    retirement_clean &= reason == pci::ReleaseError::Ports
        && pci::is_claimed(at)
        && pci::claims() == claims_before + 1
        && wost::arch::current::port::claim::<u8>(port).is_none();
    retirement_clean &= device.release(&mut context).into_retained().is_none();
    let reclaimed = match pci::lease(at) {
        Ok(reclaimed) => {
            let same_window = (0..pci::BARS).any(|index| {
                reclaimed.window(index).is_some_and(|window| {
                    window.bar == pci_config::Bar::Io { port } && window.size == size
                })
            });
            same_window && reclaimed.release(&mut context).into_retained().is_none()
        }
        Err(_) => false,
    };

    // An outside claim proves refusal is limited to the device's window.
    let outside = port.checked_add(size as u16).is_some_and(|outside| {
        wost::arch::current::port::claim_spans::<1>(&[(outside, 1)])
            .is_some_and(|span| wost::arch::current::port::release_spans(span).is_ok())
    });

    surveyed && seen > 0 && base_refused && last_refused && retirement_clean && reclaimed && outside
}

fn work_in_flight_is_ended_before_the_ring_is_released() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let table = shared_handle_table_or_false!(8);

    // Two operations, an hour out, so nothing but the teardown can end them.
    for (slot, token) in [(0usize, 0xA1u32), (1, 0xA2)] {
        queue.post_submission_for_test(
            slot,
            queue::Submission::simple(queue::Opcode::Timer, token, 3_600_000_000_000),
        );
    }
    queue.publish_tail_for_test(2);

    // Admitted and deferred: no completions, and the ring's own accounting
    // says two operations are outstanding.
    let in_flight = queue.serve(&table) == 0 && queue.in_flight() == 2 && queue.outstanding() == 2;

    let ended = queue.abandon();

    // Each got the completion it was owed, and it says what happened rather
    // than pretending to be a result.
    let first = queue.read_completion(0);
    let second = queue.read_completion(1);
    let completed = first.token == 0xA1
        && first.status == queue::STATUS_CANCELLED
        && second.token == 0xA2
        && second.status == queue::STATUS_CANCELLED;

    in_flight
        && ended == 2
        && completed
        // Nothing left that could touch the pages, and `outstanding` would
        // otherwise say work is in flight for ever.
        && queue.is_quiet()
        && queue.in_flight() == 0
        && queue.outstanding() == 0
        && queue.is_consistent()
        // A second sweep finds nothing: teardown is idempotent, because
        // `reclaim` is not the only path to a zombie.
        && queue.abandon() == 0
}

fn a_refused_settlement_retains_every_owner() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }

    let Some(cancelled) = queue::Queue::new() else {
        return false;
    };
    let cancel_table = shared_handle_table_or_false!(8);
    cancelled.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0xCA11, u64::MAX),
    );
    cancelled.publish_tail_for_test(1);
    if cancelled.serve(&cancel_table) != 0 || cancelled.in_flight() != 1 {
        return false;
    }
    cancelled.post_submission_for_test(
        1,
        queue::Submission::simple(queue::Opcode::Cancel, 0xCA12, 0xCA11),
    );
    cancelled.publish_tail_for_test(2);
    cancelled.refuse_next_post_for_test();
    let cancel_refused = cancelled.serve(&cancel_table) == 1
        && cancelled.read_completion(0).token == 0xCA12
        && cancelled.read_completion(0).status == queue::STATUS_NO_ROOM
        && cancelled.in_flight() == 1
        && cancelled.outstanding() == 1
        && cancelled.settlement_refusals_for_test() == 1;
    let cancel_retried = cancelled.abandon() == 1
        && cancelled.read_completion(1).token == 0xCA11
        && cancelled.read_completion(1).status == queue::STATUS_CANCELLED
        && cancelled.is_quiet()
        && cancelled.outstanding() == 0;

    let Some(expired) = queue::Queue::new() else {
        return false;
    };
    let expire_table = shared_handle_table_or_false!(8);
    expired.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0xE771, u64::MAX),
    );
    expired.publish_tail_for_test(1);
    if expired.serve(&expire_table) != 0 || expired.in_flight() != 1 {
        return false;
    }
    expired.refuse_next_post_for_test();
    let expiry_refused = expired.expire(u64::MAX) == 0
        && expired.in_flight() == 1
        && expired.outstanding() == 1
        && expired.settlement_refusals_for_test() == 1;
    let expiry_retried = expired.expire(u64::MAX) == 1
        && expired.expire(u64::MAX) == 0
        && expired.read_completion(0).token == 0xE771
        && expired.read_completion(0).status == queue::STATUS_OK
        && expired.is_quiet()
        && expired.outstanding() == 0;

    while user::collect().is_some() {}
    let refusals_before = user::refused_ring_reclaims();
    let retirements_before = thread::retirements();
    let Ok(spawned) = user::run_faulting() else {
        return false;
    };
    let Some((ring, table)) = user::authority_for_test(spawned.slot) else {
        return false;
    };
    ring.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0xDEAD, u64::MAX),
    );
    ring.publish_tail_for_test(1);
    if ring.serve(&table) != 0 || ring.in_flight() != 1 {
        return false;
    }
    ring.refuse_next_post_for_test();
    queue::pause_worker_for_test(true);
    let retired =
        runs_until(2000, || thread::retirements() > retirements_before) && user::has_zombies();
    let first_refused = retired
        && matches!(user::collect(), user::CollectOutcome::Retained)
        && user::has_zombies()
        && user::refused_ring_reclaims() == refusals_before + 1
        && ring.in_flight() == 1
        && ring.outstanding() == 1
        && ring.settlement_refusals_for_test() == 1;
    let collected = user::collect().is_some_and(|reclaimed| reclaimed.abandoned == 1)
        && !user::has_zombies()
        && ring.read_completion(0).token == 0xDEAD
        && ring.read_completion(0).status == queue::STATUS_CANCELLED
        && ring.is_quiet()
        && ring.outstanding() == 0;
    queue::pause_worker_for_test(false);

    cancel_refused
        && cancel_retried
        && expiry_refused
        && expiry_retried
        && first_refused
        && collected
}

fn a_promise_outlives_the_program_that_made_it() -> bool {
    let held = crate::object::bootstrap_identity();
    let second = crate::object::bootstrap_identity();

    let table = shared_handle_table_or_false!(4);
    let foreign = shared_handle_table_or_false!(2);
    let watching = alloc::sync::Arc::downgrade(&table);

    let full = Rights::UNIVERSAL;
    if table.insert(held, full).is_err() {
        return false;
    }
    let Ok(reservation) = table.reserve_slot() else {
        return false;
    };
    let promised = table.live() == 1 && table.promised() == 1;

    // The program dies. Everything the kernel held of it is dropped, and the
    // only reference left in the world is the one inside the promise.
    drop(table);

    let Some(orphan) = watching.upgrade() else {
        // A table freed under an outstanding promise is a use-after-free
        // waiting for the completion to arrive, not merely a failed check.
        return false;
    };

    // Surviving is not working: the promise is spent against the orphaned
    // table, as a completion after the program's death would.
    let usage_before = HandleTable::arena_usage_for_test();
    let Ok(handle) = reservation.install(second, full, full) else {
        return false;
    };
    let Some(slot) = orphan.slot_of(handle) else {
        return false;
    };
    let foreign_handle = foreign.handle_for(slot);
    let usage_after = HandleTable::arena_usage_for_test();
    let installed = orphan.resolve(handle).is_ok()
        && matches!(foreign.resolve(foreign_handle), Err(HandleError::Stale));
    let accounted = orphan.live() == 2
        && orphan.promised() == 0
        && foreign.live() == 0
        && foreign.promised() == 0
        && usage_after.0 == usage_before.0
        && usage_after.1 == usage_before.1 + 1;

    // And with the promise gone, the table goes when the last reference does.
    // A `Weak` that still upgrades here is a table nothing will ever free.
    drop(orphan);
    drop(foreign);
    let released = watching.upgrade().is_none();

    promised && installed && accounted && released
}

fn the_time_page_states_the_clock_the_kernel_measured() -> bool {
    let Some((version, page_bytes, counter_hz)) = timepage::constants() else {
        return false;
    };

    version == timepage::VERSION
        && page_bytes == PAGE_SIZE as u32
        // The measured rate, not merely a non-zero one: a page publishing
        // somebody else's clock would still pass a "is it plausible" test.
        && counter_hz == crate::time::counter_hz()
        && counter_hz > 0
}

fn the_time_page_and_the_kernel_clock_tell_the_same_time() -> bool {
    let Some(reading) = timepage::read() else {
        return false;
    };
    if reading.mode != timepage::MODE_COUNTER {
        return false;
    }

    let before = crate::time::now_ns();
    let published = reading.at(cpu::timestamp());
    let after = crate::time::now_ns();

    let agrees = before <= published && published <= after;

    // And it *advances*: a page whose conversion is zero answers the same
    // number every time, which sits inside any sandwich taken quickly enough.
    let first = reading.at(cpu::timestamp());
    let moved = spins_until(1_000_000, || reading.at(cpu::timestamp()) > first);

    agrees && moved && reading.conversion.is_calibrated()
}

fn revoking_the_clock_sends_readers_to_uptime() -> bool {
    let Some(before) = timepage::read() else {
        return false;
    };

    let deferred = timepage::with_page_absent_for_test(|| {
        timepage::publish(
            timepage::MODE_NONE,
            crate::time::origin_ticks(),
            crate::time::conversion(),
        ) == timepage::Publication::Deferred
    });
    if deferred != Some(true) {
        return false;
    }

    let Some(revoked) = timepage::read() else {
        return false;
    };
    // Exactly two, begin and end: one would leave the page permanently odd
    // and every reader spinning.
    let stepped = revoked.seq == before.seq.wrapping_add(2);
    let closed = revoked.mode == timepage::MODE_NONE;

    // The fallback, on the path a program would actually use.
    let queue_answers = {
        let Some(queue) = queue::Queue::new() else {
            return false;
        };
        queue.post_submission_for_test(
            0,
            queue::Submission::simple(queue::Opcode::Uptime, 0x7EA, 0),
        );
        queue.publish_tail_for_test(1);
        let table = shared_handle_table_or_false!(8);
        let served = queue.serve(&table) == 1;
        let completion = queue.read_completion(0);
        served
            && completion.token == 0x7EA
            && completion.status == queue::STATUS_OK
            && completion.value > 0
    };

    let restored = timepage::publish(
        timepage::MODE_COUNTER,
        crate::time::origin_ticks(),
        crate::time::conversion(),
    ) == timepage::Publication::Published
        && timepage::read().is_some_and(|reading| reading.mode == timepage::MODE_COUNTER);

    stepped && closed && queue_answers && restored
}

fn the_clocks_update_window_is_never_preemptible() -> bool {
    // The revocation check has published twice by now, so this is not a count
    // that has never been exercised.
    let Some(seq) = timepage::sequence() else {
        return false;
    };

    timepage::is_published()
        && timepage::windows_left_preemptible() == 0
        // Read the odd marker to prove every update opened its seqlock window
        // (D-166).
        && timepage::windows_never_opened() == 0
        && timebase::Seq::is_stable(seq)
        // Publishing moves it, so "even" is not merely "never written".
        && seq >= 4
}

fn a_program_sees_the_time_page_and_cannot_write_it() -> bool {
    in_reclaimed_space(AddressSpace::for_user, |space| {
        if timepage::install(space).is_err() {
            return false;
        }

        let Some((_, flags)) = space.lookup(timepage::TIME) else {
            return false;
        };

        flags.contains(Flags::USER)
            && !flags.contains(Flags::WRITABLE)
            && flags.contains(Flags::NO_EXECUTE)
            // And not the space's to give away: the frame is the kernel's, so an
            // unmap must not hand it to the allocator.
            && matches!(
                space.unmap(timepage::TIME, PAGE_SIZE),
                Err(MapError::NotOwned(_))
            )
    })
}

fn a_completion_carries_a_handle_with_narrowed_rights() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let held = crate::object::bootstrap_identity();
    let table = shared_handle_table_or_false!(8);

    let offered = Rights::UNIVERSAL;
    let Ok(source) = table.insert(held, offered) else {
        return false;
    };

    let declared = Rights::INSPECT;
    queue.post_submission_for_test(
        0,
        queue::Submission::about(
            queue::Opcode::Duplicate,
            0x5150,
            source,
            u64::from(declared.bits()),
        ),
    );
    queue.publish_tail_for_test(1);

    if queue.serve(&table) != 1 {
        return false;
    }
    let completion = queue.read_completion(0);
    if completion.token != 0x5150 || completion.status != queue::STATUS_OK {
        return false;
    }

    let arrived = completion.value as u32;
    let Ok((_, granted)) = table.resolve(arrived) else {
        return false;
    };

    granted == declared
        && !granted.contains(Rights::DUPLICATE)
        && arrived != source
        && table.live() == 2
        // The promise became a handle: a table charged twice for one runs
        // out at half its quota and nothing else would say so.
        && table.promised() == 0
}

fn a_completion_cannot_widen_what_was_offered() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let held = crate::object::bootstrap_identity();
    let table = shared_handle_table_or_false!(8);

    // Held without WAIT or TRANSFER, so asking for all universal rights widens.
    let offered = Rights::DUPLICATE.union(Rights::INSPECT);
    let Ok(source) = table.insert(held, offered) else {
        return false;
    };

    let asked = Rights::UNIVERSAL;
    queue.post_submission_for_test(
        0,
        queue::Submission::about(
            queue::Opcode::Duplicate,
            0x0BAD,
            source,
            u64::from(asked.bits()),
        ),
    );
    queue.publish_tail_for_test(1);

    if queue.serve(&table) != 1 {
        return false;
    }
    let completion = queue.read_completion(0);

    completion.token == 0x0BAD
        && completion.status == queue::STATUS_DENIED
        && completion.value == 0
        && table.live() == 1
        && table.promised() == 0
}

fn a_refused_handle_operation_returns_its_promise() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let duplicable = crate::object::bootstrap_identity();
    let sealed = crate::object::bootstrap_identity();

    // Room to spare: a refusal must be reached *after* a promise is taken, and
    // a full table refuses before there is one.
    let table = shared_handle_table_or_false!(8);
    let full = Rights::UNIVERSAL;
    let Ok(source) = table.insert(duplicable, full) else {
        return false;
    };
    // Held without DUPLICATE, which is what makes copying it a refusal rather
    // than a mistake.
    let Ok(undupable) = table.insert(sealed, Rights::INSPECT) else {
        return false;
    };

    let asked = [
        // Refused at the lookup: a well-formed value naming nothing of ours.
        (
            0xF001u32,
            source ^ 0x40,
            u64::from(full.bits()),
            queue::STATUS_BAD_HANDLE,
        ),
        // Refused at the right: held, but not passable.
        (
            0xF002,
            undupable,
            u64::from(Rights::INSPECT.bits()),
            queue::STATUS_DENIED,
        ),
        // Refused at the attenuation: asking for more than is held.
        (
            0xF003,
            undupable,
            u64::from(Rights::INSPECT.union(Rights::WAIT).bits()),
            queue::STATUS_DENIED,
        ),
    ];

    for (slot, (token, handle, rights, _)) in asked.iter().enumerate() {
        queue.post_submission_for_test(
            slot,
            queue::Submission::about(queue::Opcode::Duplicate, *token, *handle, *rights),
        );
    }
    queue.publish_tail_for_test(asked.len() as u32);

    if queue.serve(&table) != asked.len() {
        return false;
    }

    let mut refused = true;
    for (slot, (token, _, _, status)) in asked.iter().enumerate() {
        let completion = queue.read_completion(slot);
        refused &= completion.token == *token && completion.status == *status;
    }

    // Three promises taken, three given back: a leak is invisible in the
    // completions and shows later as a table full of nothing.
    refused && table.live() == 2 && table.promised() == 0
}

fn a_full_table_refuses_a_handle_operation_at_submission() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let held = crate::object::bootstrap_identity();

    // Room for exactly one, which the source takes.
    let table = shared_handle_table_or_false!(1);
    let full = Rights::UNIVERSAL;
    let Ok(source) = table.insert(held, full) else {
        return false;
    };

    queue.post_submission_for_test(
        0,
        queue::Submission::about(
            queue::Opcode::Duplicate,
            0xFEED,
            source,
            u64::from(full.bits()),
        ),
    );
    // Neither malformed rights nor a malformed path may be inspected first,
    // or the result-producing operations have two admission orders (D-151).
    queue.post_submission_for_test(
        1,
        queue::Submission::about(
            queue::Opcode::Duplicate,
            0xFEE1,
            source,
            0xFFFF_FFFF_0000_0000,
        ),
    );
    queue.post_submission_for_test(
        2,
        queue::Queue::about_at(queue::Opcode::OpenAt, 0xFEE2, source, 0, u64::MAX, 1),
    );
    queue.publish_tail_for_test(3);

    if queue.serve(&table) != 3 {
        return false;
    }
    let duplicate = queue.read_completion(0);
    let malformed_duplicate = queue.read_completion(1);
    let open = queue.read_completion(2);

    duplicate.token == 0xFEED
        && duplicate.status == queue::STATUS_NO_HANDLE_SLOT
        && duplicate.value == 0
        && malformed_duplicate.token == 0xFEE1
        && malformed_duplicate.status == queue::STATUS_NO_HANDLE_SLOT
        && malformed_duplicate.value == 0
        && open.token == 0xFEE2
        && open.status == queue::STATUS_NO_HANDLE_SLOT
        && open.value == 0
        && table.live() == 1
        && table.promised() == 0
}

fn an_operation_completes_after_its_deadline_and_only_once() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let table = shared_handle_table_or_false!(8);

    const DELAY_NS: u64 = 2_000_000;
    let submitted_at = crate::time::now_ns();
    queue.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0x7117, DELAY_NS),
    );
    queue.publish_tail_for_test(1);

    // Admitted, and *not* completed.
    let deferred = queue.serve(&table) == 0 && queue.in_flight() == 1;
    let Some(deadline) = queue.earliest_deadline() else {
        return false;
    };
    // Before the deadline nothing is due, however often it is asked.
    let early = queue.expire(crate::time::now_ns()) == 0 && queue.in_flight() == 1;

    // A million reads: the deadline is two milliseconds out and the clock is
    // running, so a deadline this does not reach is not slow, it is wrong.
    if !spins_until(1_000_000, || crate::time::now_ns() >= deadline) {
        return false;
    }

    let due = queue.expire(crate::time::now_ns());
    let completion = queue.read_completion(0);

    deferred
        && early
        && due == 1
        // And only once: the second sweep finds nothing, which is D-79's rule
        // seen from the side a deferral could break it from.
        && queue.expire(crate::time::now_ns()) == 0
        && queue.in_flight() == 0
        && completion.token == 0x7117
        && completion.status == queue::STATUS_OK
        // Completed *after* the deadline, not merely after the submission.
        && completion.value >= submitted_at.saturating_add(DELAY_NS)
}

fn a_cancelled_operation_and_its_cancellation_complete_separately() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let table = shared_handle_table_or_false!(8);

    // An hour away, so nothing but the cancellation can end it.
    queue.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Timer, 0xDEAD, 3_600_000_000_000),
    );
    queue.publish_tail_for_test(1);
    if queue.serve(&table) != 0 || queue.in_flight() != 1 {
        return false;
    }

    queue.post_submission_for_test(
        1,
        queue::Submission::simple(queue::Opcode::Cancel, 0xC0DE, 0xDEAD),
    );
    queue.publish_tail_for_test(2);
    if queue.serve(&table) != 1 {
        return false;
    }

    // The target's, posted from inside the cancellation, then the
    // cancellation's own.
    let target = queue.read_completion(0);
    let canceller = queue.read_completion(1);

    let separate = target.token == 0xDEAD
        && target.status == queue::STATUS_CANCELLED
        && canceller.token == 0xC0DE
        && canceller.status == queue::STATUS_OK
        && canceller.value == u64::from(0xDEADu32);

    // A second cancellation finds nothing: advisory, as io_uring's
    // `-EALREADY`, for a race the program cannot avoid.
    queue.post_submission_for_test(
        2,
        queue::Submission::simple(queue::Opcode::Cancel, 0xC0DF, 0xDEAD),
    );
    queue.publish_tail_for_test(3);
    let again =
        queue.serve(&table) == 1 && queue.read_completion(2).status == queue::STATUS_NOT_FOUND;

    separate
        && again
        && queue.in_flight() == 0
        && queue.is_consistent()
        && queue::pending_identity_exhaustion_is_terminal_for_test()
}

fn every_submission_gets_exactly_one_completion() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };

    // Three operations: one served, one served differently, one refused.
    let asked = [
        queue::Submission::simple(queue::Opcode::Echo, 0x1111, 0xDEAD_BEEF),
        queue::Submission::simple(queue::Opcode::Uptime, 0x2222, 0),
        queue::Submission::simple(queue::Opcode::Unknown(0xFFFF), 0x3333, 0),
    ];

    for (slot, submission) in asked.iter().enumerate() {
        queue.post_submission_for_test(slot, *submission);
    }
    queue.publish_tail_for_test(asked.len() as u32);

    let served = queue.serve(&shared_handle_table_or_false!(8));
    // Exactly one each, and no more: a second call finds nothing left.
    let once = served == asked.len() && queue.serve(&shared_handle_table_or_false!(8)) == 0;
    let accounted = queue.outstanding() == 0 && queue.is_consistent();

    let echo = queue.read_completion(0);
    let uptime = queue.read_completion(1);
    let refused = queue.read_completion(2);

    let matched = echo.token == 0x1111
        && echo.status == queue::STATUS_OK
        && echo.value == 0xDEAD_BEEF
        && uptime.token == 0x2222
        && uptime.status == queue::STATUS_OK
        // The value, not only the token: a zeroed `now_ns()` once left this
        // green (D-115).
        && uptime.value > 0
        && uptime.value <= crate::time::now_ns()
        && refused.token == 0x3333
        // Refused *with* a completion, which is the rule: an operation the
        // kernel will not serve still reports, or a caller waits for ever.
        && refused.status == queue::STATUS_UNKNOWN_OPCODE;

    once && accounted && matched
}

fn admission_refuses_before_the_completion_queue_can_overflow() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let (_, submissions, completions) = queue.shape();

    // Ask for far more than either queue holds.
    let wanted = (completions as usize) * 4;
    let admitted = queue.reserve_for_test(wanted);

    // Bounded by the submission queue, which is the tighter of the two here,
    // and never by nothing.
    let bounded = admitted > 0 && admitted <= submissions as usize;
    let refused = admitted < wanted;
    let consistent = queue.is_consistent();

    // And a full ring admits nothing more until something reports.
    let full_refuses = queue.reserve_for_test(1) == 0;

    bounded && refused && consistent && full_refuses
}

fn a_rewritten_submission_cannot_change_what_was_served() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };

    queue.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Echo, 0xABCD, 0x0F0F_0F0F),
    );
    queue.publish_tail_for_test(1);
    if queue.serve(&shared_handle_table_or_false!(8)) != 1 {
        return false;
    }
    let posted = queue.read_completion(0);

    // Ring 3 rewrites the entry it already submitted, as a hostile program
    // would the instant it saw the head move.
    queue.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Unknown(0xDEAD), 0xBAAD, 0xFFFF_FFFF),
    );

    // The visible rewrite plus original completion proves service used the
    // copied submission (D-115).
    let rewritten = queue.read_submission(0);
    let took_effect = rewritten.token == 0xBAAD && rewritten.opcode != queue::Opcode::Echo;
    let unchanged = took_effect
        && posted.token == 0xABCD
        && posted.status == queue::STATUS_OK
        && posted.value == 0x0F0F_0F0F
        // And serving again does not re-serve the rewritten entry: the slot
        // was consumed, and the new bytes are a submission nobody made.
        && queue.serve(&shared_handle_table_or_false!(8)) == 0;

    // Reservations make this runaway tail reach the explicit posted bound
    // instead of an empty-queue early return (D-115).
    let Some(runaway) = queue::Queue::new() else {
        return false;
    };
    // Three written, and a tail claiming four billion. Without the bound the
    // kernel admits and serves all three from slots it was never told about.
    for slot in 0..3 {
        runaway.post_submission_for_test(
            slot,
            queue::Submission::simple(queue::Opcode::Echo, 0x9999, 0),
        );
    }
    runaway.publish_tail_for_test(u32::MAX);
    let runaway_bounded =
        runaway.serve(&shared_handle_table_or_false!(8)) == 0 && runaway.outstanding() == 0;

    queue.publish_tail_for_test(u32::MAX);
    let drained_bounded = queue.serve(&shared_handle_table_or_false!(8)) == 0;

    let Some(partial) = queue::Queue::new() else {
        return false;
    };
    for slot in 0..3 {
        partial.post_submission_for_test(
            slot,
            queue::Submission::simple(queue::Opcode::Echo, 0x5150 + slot as u32, 7),
        );
    }
    partial.publish_tail_for_test(1);
    let only_published = partial.serve(&shared_handle_table_or_false!(8)) == 1;
    let one_reported = partial.read_completion(0).token == 0x5150;
    // And nothing is held open: the one served reported, and the two unpublished
    // entries were never admitted, so no reservation outlives the round.
    let still_reserved = partial.outstanding() == 0 && partial.is_consistent();

    unchanged
        && runaway_bounded
        && drained_bounded
        && only_published
        && one_reported
        && still_reserved
}

fn thread_local_storage_follows_the_thread() -> bool {
    /// Somewhere in the low half, aligned, and not zero.
    const MINE: u64 = 0x0000_1234_5678_0000;
    /// What the other thread sets.
    const THEIRS: u64 = 0x0000_7654_3210_0000;

    let restore = task::read_thread_base();

    let checked = (|| {
        if !task::set_thread_base(MINE) || task::read_thread_base() != MINE {
            return false;
        }

        let kernel_refused =
            !task::set_thread_base(0xffff_8000_0000_0000) && task::read_thread_base() == MINE;
        let hole_refused =
            !task::set_thread_base(0x0000_8000_0000_0000) && task::read_thread_base() == MINE;

        BASE_INTRUDER_RAN.store(0, Ordering::Release);
        if thread::spawn("base-intruder", set_another_thread_base).is_err()
            || !runs_until(500, || BASE_INTRUDER_RAN.load(Ordering::Acquire) == 1)
        {
            return false;
        }

        let kept = task::read_thread_base() == MINE;
        let intruded = BASE_INTRUDER_SAW.load(Ordering::Acquire) == THEIRS as usize;
        kernel_refused && hole_refused && kept && intruded
    })();
    let restored = task::set_thread_base(restore) && task::read_thread_base() == restore;
    checked && restored
}

/// Set by the intruding thread once it has run.
static BASE_INTRUDER_RAN: AtomicUsize = AtomicUsize::new(0);

/// What the intruding thread read back from its own base.
static BASE_INTRUDER_SAW: AtomicUsize = AtomicUsize::new(0);

fn set_another_thread_base() {
    const THEIRS: u64 = 0x0000_7654_3210_0000;
    if task::set_thread_base(THEIRS) {
        let mut context = context();
        thread::yield_now(&mut context);
        BASE_INTRUDER_SAW.store(task::read_thread_base() as usize, Ordering::Release);
    }
    BASE_INTRUDER_RAN.store(1, Ordering::Release);
}

fn a_user_page_comes_back_cleared() -> bool {
    /// Two user addresses nothing else uses.
    const FIRST: u64 = user::STACK + 0xA0_0000;
    const SECOND: u64 = user::STACK + 0xA1_0000;
    /// What a program would have left in the page.
    const SECRET: [u8; 8] = [0xDE; 8];

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let writable = Flags::WRITABLE.union(Flags::NO_EXECUTE);

        let Some(mut run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !run.zero() || !run.write_at(0, &SECRET) {
            return false;
        }
        let address = run.address();
        if space.map_user(FIRST, run.into(), writable).is_err() {
            return false;
        }

        // It comes back untyped, and only a clearing conversion makes it the
        // kernel's again; the handle is followed, not the allocator asked.
        let Ok(unmapped) = space.unmap(FIRST, PAGE_SIZE) else {
            return false;
        };
        let knows = unmapped.was_user();
        let Some(untyped) = unmapped.into_user() else {
            return false;
        };
        let Some(back) = untyped.reclaim() else {
            return false;
        };
        let mut recovered = [0xFFu8; 8];
        let cleared = back.address() == address
            && back.read_at(0, &mut recovered)
            && recovered == [0u8; 8]
            && recovered != SECRET;
        drop(back);

        // The kernel conversion cannot hand a user range over as a `Segment`.
        let Some(mut other) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !other.zero() {
            return false;
        }
        if space.map_user(SECOND, other.into(), writable).is_err() {
            return false;
        }
        let Ok(second) = space.unmap(SECOND, PAGE_SIZE) else {
            return false;
        };
        let refused_as_kernel = second.into_kernel().is_none();

        // Reallocation need not return these frames, so a tally observes the
        // drop clear without relying on same-frame readback (D-119, D-123).
        let cleared_before = paging::cleared_on_drop();
        let Some(mut third) = mm::allocate_contiguous(2, 1) else {
            return false;
        };
        if !third.zero() {
            return false;
        }
        if space
            .map_user(SECOND + PAGE_SIZE * 4, third.into(), writable)
            .is_err()
        {
            return false;
        }
        let Ok(dropped) = space.unmap(SECOND + PAGE_SIZE * 4, 2 * PAGE_SIZE) else {
            return false;
        };
        drop(dropped);
        let counted = paging::cleared_on_drop() == cleared_before + 2;

        knows && cleared && refused_as_kernel && counted
    })
}

fn a_bound_space_is_not_reclaimed() -> bool {
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let root = space.root();
    let Ok(binding) = space.bind() else {
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a refused binding did not return its address-space tables"
        );
        return false;
    };

    // A thread to bind it to, which is what makes it un-reclaimable.
    let mut bound = None;
    let slot =
        match thread::spawn_in_space("bound-space", leave_at_once, binding, |_, installed| {
            bound = Some(installed);
            Ok(())
        }) {
            Ok(slot) => slot,
            Err(failure) => {
                let released = match failure.authority {
                    thread::SpaceAuthority::None => None,
                    thread::SpaceAuthority::Released(released) => Some(released),
                };
                let reclaimed = space.reclaim(released).is_ok();
                drop(space);
                assert!(
                    reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
                    "a refused bound-space setup did not return its tables"
                );
                return false;
            }
        };
    let bound = bound.expect("a successful arrangement retains its binding");

    let bound = match task::release_address_space(bound) {
        Ok(released) => {
            let reclaimed = space.reclaim(Some(released)).is_ok_and(|tables| tables > 0);
            drop(space);
            assert!(
                reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
                "an unexpected bound-space release did not return its tables"
            );
            return false;
        }
        Err(bound) => bound,
    };

    // Refused with no evidence and left untouched, so the same value can be
    // reclaimed once the task table supplies the evidence below.
    let held = mm::free_frames();
    let refused = matches!(
        space.reclaim(None),
        Err(paging::ReclaimFailure {
            reason: ReclaimError::StillBound(found),
            ..
        }) if found == root
    ) && !space.is_reclaimed()
        && mm::free_frames() == held;

    // And once the thread is gone, the task table says so and the space comes
    // back — so this is not simply "reclaim never works".
    let gone = runs_until(500, || !task::slot_is_live_for_test(slot));
    assert!(gone, "the bound-space witness did not retire");
    let released = task::release_address_space(bound)
        .expect("a retired bound-space witness releases its binding");

    // Release evidence survives a foreign and a repeated target, and the
    // rightful space is still reclaimable with it afterwards.
    let Ok(mut foreign) = AddressSpace::for_user() else {
        assert!(
            space.reclaim(Some(released)).is_ok(),
            "a failed foreign-space setup stranded release evidence"
        );
        return false;
    };
    let foreign_failure = match foreign.reclaim(Some(released)) {
        Err(failure) => failure,
        Ok(_) => panic!("a foreign space consumed release evidence"),
    };
    let foreign_refused = matches!(
        foreign_failure.reason,
        ReclaimError::UnexpectedRelease(found) if found == root
    ) && !foreign.is_reclaimed();
    let Some(released) = foreign_failure.released else {
        panic!("a foreign-space refusal lost release evidence");
    };
    let foreign_reclaimed = foreign.reclaim(None).is_ok() && foreign.is_reclaimed();
    let repeated_failure = match foreign.reclaim(Some(released)) {
        Err(failure) => failure,
        Ok(_) => panic!("a reclaimed space consumed release evidence"),
    };
    let repeated_refused = repeated_failure.reason == ReclaimError::Reclaimed;
    let Some(released) = repeated_failure.released else {
        panic!("a repeated reclaim refusal lost release evidence");
    };
    let reclaimed = foreign_refused
        && foreign_reclaimed
        && repeated_refused
        && space.reclaim(Some(released)).is_ok_and(|tables| tables > 0)
        && space.is_reclaimed();
    drop(foreign);
    drop(space);

    refused && reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

fn one_space_has_one_binding() -> bool {
    let free_before = mm::free_frames();
    let slots_before = thread::free_slots();
    let active_before = paging::active_root();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let root = space.root();
    let Ok(first) = space.bind() else {
        assert!(
            space.reclaim(None).is_ok() && mm::free_frames() == free_before,
            "a refused first binding did not return its address-space tables"
        );
        return false;
    };

    let (refused, duplicate) = match space.bind() {
        Err(ActivateError::AlreadyActive) => (true, None),
        Err(_) => (false, None),
        Ok(binding) => (false, Some(binding)),
    };
    let unchanged = first.root() == root
        && space.root() == root
        && paging::active_root() == active_before
        && thread::free_slots() == slots_before;

    if let Some(binding) = duplicate {
        let _duplicate_release = task::release_uninstalled(binding);
    }
    let released = task::release_uninstalled(first);
    let reclaimed = space.reclaim(Some(released)).is_ok_and(|tables| tables > 0)
        && space.is_reclaimed()
        && mm::free_frames() == free_before;

    refused && unchanged && reclaimed
}

fn binding_ignores_only_accessed_root_drift() -> bool {
    let free_before = mm::free_frames();
    let Ok(mut accessed) = AddressSpace::for_user() else {
        return false;
    };
    let (accepts_accessed, accessed_reclaimed) = match accessed.bind_with_accessed_drift_for_test()
    {
        Some(Ok(binding)) => {
            let released = task::release_uninstalled(binding);
            (
                true,
                accessed
                    .reclaim(Some(released))
                    .is_ok_and(|tables| tables > 0),
            )
        }
        Some(Err(_)) | None => (false, accessed.reclaim(None).is_ok_and(|tables| tables > 0)),
    };

    let Ok(mut cache) = AddressSpace::for_user() else {
        return false;
    };
    let (rejects_cache, cache_reclaimed) = match cache.bind_with_cache_drift_for_test() {
        Some(Err(ActivateError::KernelHalfDiffers(_))) => match cache.bind() {
            Ok(binding) => {
                let released = task::release_uninstalled(binding);
                (
                    true,
                    cache.reclaim(Some(released)).is_ok_and(|tables| tables > 0),
                )
            }
            Err(_) => (false, cache.reclaim(None).is_ok_and(|tables| tables > 0)),
        },
        Some(Ok(binding)) => {
            let released = task::release_uninstalled(binding);
            (
                false,
                cache.reclaim(Some(released)).is_ok_and(|tables| tables > 0),
            )
        }
        Some(Err(_)) | None => (false, cache.reclaim(None).is_ok_and(|tables| tables > 0)),
    };

    accepts_accessed
        && accessed_reclaimed
        && rejects_cache
        && cache_reclaimed
        && mm::free_frames() == free_before
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllocatorSnapshot {
    free_frames: usize,
    heap: mm::heap::Stats,
}

impl AllocatorSnapshot {
    fn capture() -> Self {
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

    fn accounted_physical_frames(&self) -> Option<usize> {
        self.ownership().accounted_physical_frames()
    }

    fn has_same_ownership(&self, other: &Self) -> bool {
        self.ownership().is_preserved_since(other.ownership())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SpawnResources {
    frames: usize,
    heap: mm::heap::Stats,
    arena: (usize, usize),
    channels: usize,
    rings: usize,
    thread_slots: usize,
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

    fn has_same_ownership(&self, other: &Self) -> bool {
        self.has_same_live_owners(other)
            && self.has_same_refusal_tallies(other)
            && self
                .allocator_snapshot()
                .has_same_ownership(&other.allocator_snapshot())
    }
}

fn spawn_resources() -> SpawnResources {
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

fn every_refused_spawn_returns_every_resource() -> bool {
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

fn a_commit_refusal_reunites_cleanup_authority() -> bool {
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

fn a_maximum_image_and_stack_fit_the_cleanup_ledger() -> bool {
    user::maximum_cleanup_ledger_is_total_for_test()
}

fn cleanup_refusals_retain_reachable_tables() -> bool {
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

fn partial_stack_rollback_is_retryable() -> bool {
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
        && cleanup.mapped_stacks == 2
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

fn guard_rollback_refusal_retains_the_stack() -> bool {
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
        && cleanup.mapped_stacks == 1
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

fn a_retirement_refusal_is_settled_before_slot_reuse() -> bool {
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

fn cleanup_unexpected_spawn(spawned: user::Spawned) -> bool {
    let slot = spawned.slot;
    let retired = runs_until(2_000, || !task::slot_is_live_for_test(slot));
    while user::collect().is_some() {}
    retired && !user::has_zombies()
}

fn an_unguarded_boot_stack_closes_the_user_boundary() -> bool {
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

fn unadmitted_interrupts_close_the_user_boundary() -> bool {
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

fn shared_kernel_mappings_without_proof_close_the_user_boundary() -> bool {
    while user::collect().is_some() {}
    if !crate::shared_kernel_mappings_are_safe() {
        return false;
    }

    let resources_before = spawn_resources();
    let crossings_before = trap::last_syscall().count;
    let attempt = crate::without_shared_kernel_mapping_proof_for_test(user::run);
    let denied = matches!(&attempt, Err(user::SpawnError::NoSharedKernelIsolation));
    let cleaned = match attempt {
        Ok(spawned) => cleanup_unexpected_spawn(spawned),
        Err(_) => true,
    };

    denied
        && cleaned
        && crate::shared_kernel_mappings_are_safe()
        && task::final_shared_mapping_refusal_for_test()
        && trap::last_syscall().count == crossings_before
        && spawn_resources() == resources_before
}

fn no_execute_protection_closes_the_user_boundary() -> bool {
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

fn missing_extended_state_closes_the_user_boundary() -> bool {
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

fn an_uncalibrated_clock_admits_no_user_or_timer() -> bool {
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

fn a_clock_disagreement_revokes_time_without_stranding_timers() -> bool {
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
    let during = time::with_watchdog_disagreement_for_test(|| {
        let frozen = time::now_ns();
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

fn a_task_release_refusal_retains_the_zombie_for_retry() -> bool {
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

fn one_zombie_has_one_settlement_collector() -> bool {
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

fn an_in_flight_collector_keeps_the_worker_retry_live() -> bool {
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

fn a_dead_program_gives_its_memory_back() -> bool {
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

fn an_unserved_call_is_refused_without_stopping_the_machine() -> bool {
    // Before the stateful ring-3 group, and waited for: the trap-context
    // logging mutation must assert before later state can hide the cause.
    let (refused_before, _, _) = user::refusals();
    let retirements_before = thread::retirements();
    let faults_before = trap::last_user_fault().count;

    let Ok(_) = user::run_probing() else {
        return false;
    };
    if !runs_until(2000, || thread::retirements() > retirements_before) {
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
    let second_retirements = thread::retirements();
    let second_started = user::run_probing().is_ok();
    let second_finished =
        second_started && runs_until(2000, || thread::retirements() > second_retirements);
    let (refused_final, final_number, unanswered_final) = user::refusals();
    let second_counted =
        refused_final == refused_after + 1 && final_number == 0x0BAD_0BAD && unanswered_final == 0;
    let warning_is_one_shot = !user::report_refusals();

    counted && answered && exited && second_finished && second_counted && warning_is_one_shot
}

fn last_rites_for(spawned: &user::Spawned, ms: u64) -> Option<user::LastRites> {
    let table = spawned.observed_table_incarnation;
    let exact = || user::last_rites_of(spawned.slot).is_some_and(|rites| rites.table == table);
    if !runs_until(ms, exact) {
        return None;
    }
    user::last_rites_of(spawned.slot).filter(|rites| rites.table == table)
}

fn a_program_drives_its_own_ring() -> bool {
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

fn a_ring_enter_round_trip_is_measurable() -> bool {
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

fn a_full_buffer_nvme_read_is_measurable() -> bool {
    let Some((bytes, ticks)) = nvme::measure_buffered_read_for_test() else {
        return false;
    };
    STORAGE_BYTES.store(bytes, Ordering::Relaxed);
    STORAGE_TICKS.store(ticks, Ordering::Relaxed);
    bytes == PAGE_SIZE && ticks > 0
}

static STORAGE_BYTES: AtomicU64 = AtomicU64::new(0);
static STORAGE_TICKS: AtomicU64 = AtomicU64::new(0);
static SELFTEST_STARTED_NS: AtomicU64 = AtomicU64::new(0);

fn report_baseline() {
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

fn a_program_duplicates_its_bootstrap_handle() -> bool {
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

fn a_program_waits_through_its_trap_frame() -> bool {
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

fn a_syscall_wake_arms_the_timer_for_what_it_woke() -> bool {
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

fn a_spawned_program_holds_exactly_one_handle() -> bool {
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

fn a_meaningless_right_is_refused() -> bool {
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

fn a_handle_reveals_no_foreign_occupancy() -> bool {
    let mine = handle_table_or_false!(4);
    let theirs = handle_table_or_false!(4);
    let object = object::bootstrap_identity();
    let Ok(handle) = mine.insert(object, Rights::UNIVERSAL) else {
        return false;
    };

    let Some(slot) = mine.slot_of(handle) else {
        return false;
    };
    let forged = theirs.handle_for(slot);
    let live_resolve = theirs.resolve(forged).err();
    let foreign_changed_nothing =
        mine.resolve(handle).is_ok() && mine.live() == 1 && theirs.live() == 0;

    drop(mine);
    let absent_resolve = theirs.resolve(forged).err();

    live_resolve == Some(HandleError::Stale)
        && live_resolve == absent_resolve
        && foreign_changed_nothing
        && theirs.live() == 0
}

fn table_identities_never_wrap_or_repeat() -> bool {
    let baseline = HandleTable::arena_usage_for_test();
    HandleTable::with_next_identity_for_test(u64::MAX - 1, || {
        let Ok(last) = HandleTable::new(1) else {
            return false;
        };
        let object = object::bootstrap_identity();
        let Ok(handle) = last.insert(object, Rights::UNIVERSAL) else {
            return false;
        };
        let live_commitment = HandleTable::arena_usage_for_test();

        let first = HandleTable::new(1);
        let second = HandleTable::new(1);
        let refused_twice = matches!(&first, Err(HandleError::IdentityExhausted))
            && matches!(&second, Err(HandleError::IdentityExhausted));
        let duplicate_authority = first
            .as_ref()
            .ok()
            .is_some_and(|table| table.resolve(handle).is_ok())
            || second
                .as_ref()
                .ok()
                .is_some_and(|table| table.resolve(handle).is_ok());
        let refusal_changed_nothing = HandleTable::arena_usage_for_test() == live_commitment;
        let last_stayed_live = last.identity_for_test() == u64::MAX - 1
            && last.kind_of(handle) == Ok(ObjectType::Channel);

        drop(first);
        drop(second);
        drop(last);

        refused_twice
            && !duplicate_authority
            && refusal_changed_nothing
            && last_stayed_live
            && HandleTable::arena_usage_for_test() == baseline
    })
}

fn a_table_cannot_exceed_its_quota() -> bool {
    const QUOTA: usize = 3;
    let table = handle_table_or_false!(QUOTA);
    for _ in 0..QUOTA {
        let object = object::bootstrap_identity();
        if table.insert(object, Rights::UNIVERSAL).is_err() {
            return false;
        }
    }

    let extra = object::bootstrap_identity();
    let refused = table.insert(extra, Rights::UNIVERSAL) == Err(HandleError::QuotaExhausted);
    refused && table.live() == QUOTA
}

fn the_handle_arena_backs_a_tables_whole_quota() -> bool {
    let (committed_before, capacity_before) = HandleTable::arena_backing_for_test();
    let Some(spare) = capacity_before.checked_sub(committed_before) else {
        return false;
    };
    let Some(quota) = spare.checked_add(1) else {
        return false;
    };
    if quota > (handle_space::SLOTS as usize).saturating_sub(committed_before) {
        return false;
    }

    let Ok(table) = HandleTable::new(quota) else {
        return false;
    };
    let (committed, capacity) = HandleTable::arena_backing_for_test();
    let backed = committed == committed_before + quota
        && capacity > capacity_before
        && capacity >= committed;

    for _ in 0..quota {
        let object = object::bootstrap_identity();
        if table.insert(object, Rights::UNIVERSAL).is_err() {
            return false;
        }
    }
    let (filled_commitment, filled_capacity) = HandleTable::arena_backing_for_test();
    let filled_without_growth =
        table.live() == quota && filled_commitment == committed && filled_capacity == capacity;

    drop(table);
    let (committed_after, capacity_after) = HandleTable::arena_backing_for_test();
    let returned = committed_after == committed_before && capacity_after == filled_capacity;

    backed && filled_without_growth && returned
}

fn an_object_lives_exactly_as_long_as_its_handles() -> bool {
    let first = handle_table_or_false!(1);
    let second = handle_table_or_false!(1);
    let object = object::bootstrap_identity();
    let Some(watcher) = object.downgrade_for_test() else {
        return false;
    };
    if first.insert(object.clone(), Rights::UNIVERSAL).is_err()
        || second.insert(object, Rights::UNIVERSAL).is_err()
    {
        return false;
    }
    let alive = watcher.upgrade().is_some();
    drop(first);
    let still_alive = watcher.upgrade().is_some();
    drop(second);
    let gone = watcher.upgrade().is_none();

    alive && still_alive && gone
}

fn a_dead_table_releases_its_objects() -> bool {
    let object = object::bootstrap_identity();
    let Some(watcher) = object.downgrade_for_test() else {
        return false;
    };
    {
        let table = handle_table_or_false!(4);
        if table.insert(object, Rights::UNIVERSAL).is_err() {
            return false;
        }
        if watcher.upgrade().is_none() {
            return false;
        }
    }
    watcher.upgrade().is_none()
}

fn the_boot_thread_stands_on_a_guarded_stack() -> bool {
    let Some(layout) = thread::boot_stack_layout() else {
        return false;
    };
    let Some(slot) = layout.slot(0) else {
        return false;
    };

    // Where this check is actually running.
    let pointer = cpu::stack_pointer();
    if pointer < slot.stack_base() || pointer >= slot.stack_top() {
        return false;
    }

    // Every guard page below it is absent.
    let mut guard = slot.guard_base();
    while guard < slot.stack_base() {
        if paging::translate(guard).is_some() {
            return false;
        }
        guard += PAGE_SIZE;
    }

    // And every page it stands on is writable and never executable.
    let mut page = slot.stack_base();
    while page < slot.stack_top() {
        match paging::translate(page) {
            Some((_, flags))
                if flags.contains(Flags::WRITABLE) && flags.contains(Flags::NO_EXECUTE) => {}
            _ => return false,
        }
        page += PAGE_SIZE;
    }
    pinned_guard_refuses_mapping(&slot)
}

fn the_double_fault_stack_is_guarded() -> bool {
    let Some(layout) = thread::fault_stack_layout() else {
        return false;
    };
    let Some(slot) = layout.slot(0) else {
        return false;
    };

    if segment::double_fault_stack() != slot.stack_top() {
        return false;
    }

    // Every guard page below it is absent.
    let mut guard = slot.guard_base();
    while guard < slot.stack_base() {
        if paging::translate(guard).is_some() {
            return false;
        }
        guard += PAGE_SIZE;
    }

    // Every stack page is present, writable and never executable.
    let mut page = slot.stack_base();
    while page < slot.stack_top() {
        match paging::translate(page) {
            Some((_, flags))
                if flags.contains(Flags::WRITABLE) && flags.contains(Flags::NO_EXECUTE) => {}
            _ => return false,
        }
        page += PAGE_SIZE;
    }

    true
}

fn the_double_fault_stack_cannot_be_unmapped() -> bool {
    let Some(layout) = thread::fault_stack_layout() else {
        return false;
    };
    let Some(slot) = layout.slot(0) else {
        return false;
    };
    let inactive_refused = in_reclaimed_space(AddressSpace::new, |inactive| {
        let inactive_root = inactive.root();
        matches!(
            inactive.map_pinned_stack(&slot),
            Err(paging::StackMapError::Mapping(MapError::Inactive(root))) if root == inactive_root
        )
    });
    let mut context = ThreadContext::require();

    inactive_refused
        && matches!(
            mem::unmap_fault_stack_for_test(&mut context),
            Some(Err(MapError::Pinned(address))) if address == slot.stack_base()
        )
        && pinned_guard_refuses_mapping(&slot)
        && the_double_fault_stack_is_guarded()
}

fn pinned_guard_refuses_mapping(slot: &StackSlot) -> bool {
    let free_before = mm::free_frames();
    let guard_before = paging::translate(slot.guard_base());
    let stack_before = paging::translate(slot.stack_base());
    mem::pinned_stack_refuses_map_for_test(&mut context(), slot) == Some(true)
        && paging::translate(slot.guard_base()) == guard_before
        && paging::translate(slot.stack_base()) == stack_before
        && mm::free_frames() == free_before
}

fn lower_stack_leaf_substitution_is_restored(
    slot: &StackSlot,
    probe: impl FnOnce() -> Option<bool>,
) -> bool {
    let free_before = mm::free_frames();
    let lower = slot.stack_base();
    let Some(upper) = slot.stack_top().checked_sub(PAGE_SIZE) else {
        return false;
    };
    let guard_before = paging::translate(slot.guard_base());
    let lower_before = paging::translate(lower);
    let upper_before = paging::translate(upper);

    guard_before.is_none()
        && lower_before.is_some()
        && upper_before.is_some()
        && probe() == Some(true)
        && paging::translate(slot.guard_base()) == guard_before
        && paging::translate(lower) == lower_before
        && paging::translate(upper) == upper_before
        && mm::free_frames() == free_before
}

fn the_boot_stack_rejects_a_lower_leaf_substitution() -> bool {
    let Some(slot) = thread::boot_stack_layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    let pointer = cpu::stack_pointer();
    (slot.stack_base()..slot.stack_top()).contains(&pointer)
        && lower_stack_leaf_substitution_is_restored(&slot, || {
            paging::permanent_stack_rejects_lower_leaf_substitution_for_test(slot.stack_base())
        })
        && (slot.stack_base()..slot.stack_top()).contains(&cpu::stack_pointer())
}

fn the_double_fault_stack_rejects_a_lower_leaf_substitution() -> bool {
    let Some(slot) = thread::fault_stack_layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    let hardware_before = segment::double_fault_stack();
    hardware_before == slot.stack_top()
        && lower_stack_leaf_substitution_is_restored(&slot, || {
            paging::permanent_stack_rejects_lower_leaf_substitution_for_test(slot.stack_base())
        })
        && segment::double_fault_stack() == hardware_before
}

fn a_noncurrent_task_stack_rejects_a_lower_leaf_substitution() -> bool {
    const FIRST_TASK: usize = task::BOOT + 1;

    let Some(slot) = thread::layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    task::current() == task::BOOT
        && lower_stack_leaf_substitution_is_restored(&slot, || {
            task::pinned_stack_rejects_lower_leaf_substitution_for_test(FIRST_TASK)
        })
        && task::current() == task::BOOT
}

fn protected_stacks_refuse_overlapping_maps() -> bool {
    let Some(task_stack) = thread::layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    let Some(fault_stack) = thread::fault_stack_layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    let Some(boot_stack) = thread::boot_stack_layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    let free_before = mm::free_frames();
    let all_refused = [task_stack, fault_stack, boot_stack].iter().all(|slot| {
        let guard_before = paging::translate(slot.guard_base());
        let lower_before = paging::translate(slot.stack_base());
        let upper_before = slot.stack_top().checked_sub(1).and_then(paging::translate);
        let refused = mem::pinned_stack_refuses_map_for_test(&mut context(), slot) == Some(true);
        refused
            && paging::translate(slot.guard_base()) == guard_before
            && paging::translate(slot.stack_base()) == lower_before
            && slot.stack_top().checked_sub(1).and_then(paging::translate) == upper_before
    });
    all_refused && mm::free_frames() == free_before
}

fn a_second_space_holds_the_kernel_and_nothing_else() -> bool {
    in_reclaimed_space(AddressSpace::for_user, |space| {
        let text = boot::sections().text.start;
        let Some(stack_layout) = thread::layout() else {
            return false;
        };
        let Some(slot) = stack_layout.slot(0) else {
            return false;
        };
        let Some(direct_map) = mem::direct_map_base() else {
            return false;
        };
        let shared = [text, slot.stack_base(), direct_map].into_iter().all(|at| {
            match (paging::translate(at), space.lookup(at)) {
                (Some(here), Some(there)) => here == there,
                _ => false,
            }
        });

        let Some((probe, _)) = boundary_probe() else {
            return false;
        };
        let user_is_absent = paging::translate(probe).is_some() && space.lookup(probe).is_none();

        // The shared half is not reachable from ring 3: a top-level entry
        // carrying `USER` would hand every program the kernel.
        let kernel_is_not_user = space
            .lookup(text)
            .is_some_and(|(_, flags)| !flags.contains(Flags::USER));

        shared && user_is_absent && kernel_is_not_user
    })
}

fn a_walk_reports_the_permission_the_cpu_grants() -> bool {
    let Some((user_at, _)) = boundary_probe() else {
        return false;
    };
    let Some((_, user_flags)) = paging::translate(user_at) else {
        return false;
    };
    // The user page really is user-accessible, all the way down.
    let user_is_user = user_flags.contains(Flags::USER);

    // The kernel's own text is not, and neither are its parents.
    let text = boot::sections().text.start;
    let Some((_, text_flags)) = paging::translate(text) else {
        return false;
    };
    let kernel_is_not_user = !text_flags.contains(Flags::USER);

    // And the direct map is writable and never executable, which is the pair
    // W^X depends on being reported truthfully.
    let Some(direct) = mm::direct_map().and_then(|map| map.virtual_address(LARGE_PAGE_SIZE)) else {
        return false;
    };
    let Some((_, direct_flags)) = paging::translate(direct) else {
        return false;
    };
    let direct_is_data =
        direct_flags.contains(Flags::WRITABLE) && direct_flags.contains(Flags::NO_EXECUTE);

    user_is_user && kernel_is_not_user && direct_is_data
}

fn a_refused_mapping_leaves_no_trace() -> bool {
    let Some(base) =
        thread::boot_stack_layout().and_then(|layout| layout.end().checked_add(8 * PAGE_SIZE))
    else {
        return false;
    };
    let blocker = base + PAGE_SIZE;
    let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let free_before = mm::free_frames();

    // Occupy the *second* page of the range, so the mapping below gets one
    // page in before it is refused.
    let Some(taken) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let Some(Ok(blocker_mapping)) = mem::map_owned_for_test(&mut context(), blocker, taken, wanted)
    else {
        return false;
    };

    let free_with_blocker = mm::free_frames();
    let Some(run) = mm::allocate_contiguous(2, 1) else {
        let cleaned = mem::unmap_for_test(&mut context(), blocker_mapping)
            .is_some_and(|result| result.map(drop).is_ok());
        assert!(
            cleaned && paging::translate(blocker).is_none() && mm::free_frames() == free_before,
            "a refused mapping setup did not return its blocker"
        );
        return false;
    };
    let refused = mem::map_owned_for_test(&mut context(), base, run, wanted);

    let rejected = matches!(refused, Some(Err(MapError::Conflict(address))) if address == blocker);
    let no_trace = paging::translate(base).is_none();
    let returned = mm::free_frames() == free_with_blocker;
    let blocker_intact = paging::translate(blocker).is_some();
    let cleaned = mem::unmap_for_test(&mut context(), blocker_mapping)
        .is_some_and(|result| result.map(drop).is_ok())
        && paging::translate(blocker).is_none()
        && mm::free_frames() == free_before;

    rejected && no_trace && returned && blocker_intact && cleaned
}

fn a_partial_mapping_failure_returns_its_tables() -> bool {
    const BASE: u64 = 0x0000_3700_0000_0000;
    let flags = Flags::WRITABLE.union(Flags::NO_EXECUTE);

    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let allocator_with_root = AllocatorSnapshot::capture();
    let Some(run) = mm::allocate_contiguous(2, 1) else {
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a refused mapping allocation did not return its root"
        );
        return false;
    };
    paging::refuse_mapping_after_for_test(1);
    let result = space.map_owned(BASE, run, flags);
    paging::restore_mapping_refusals_for_test();

    let refused = result == Err(MapError::OutOfMemory);
    let absent = space.lookup(BASE).is_none() && space.lookup(BASE + PAGE_SIZE).is_none();
    let transaction_returned = AllocatorSnapshot::capture()
        .accounted_physical_frames()
        .is_some_and(|accounted| {
            allocator_with_root.accounted_physical_frames() == Some(accounted)
        });
    let root_reclaimed = space.reclaim(None).is_ok_and(|tables| tables == 1);
    drop(space);
    let root_returned =
        root_reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before);
    refused && absent && transaction_returned && root_returned
}

fn a_refused_remap_preserves_every_older_entry() -> bool {
    const BASE: u64 = 0x0000_3400_0000_0000;
    let Some(physical) = mm::first_memory_run(PAGE_SIZE * 3) else {
        return false;
    };
    let Some(first) = BootRegion::describe(physical, PAGE_SIZE) else {
        return false;
    };
    let Some(blocker) = BootRegion::describe(physical + 2 * PAGE_SIZE, PAGE_SIZE) else {
        return false;
    };
    let Some(requested) = BootRegion::describe(physical, 2 * PAGE_SIZE) else {
        return false;
    };
    in_reclaimed_space(AddressSpace::for_user, |space| {
        let flags = Flags::NO_EXECUTE;
        if space.map_boot(BASE, first, flags).is_err()
            || space.map_boot(BASE + PAGE_SIZE, blocker, flags).is_err()
        {
            return false;
        }

        let refused = matches!(
            space.map_boot(BASE, requested, flags),
            Err(MapError::Conflict(address)) if address == BASE + PAGE_SIZE
        );
        let expected = flags.union(Flags::PRESENT);
        let first_intact = space.lookup(BASE) == Some((physical, expected));
        let blocker_intact =
            space.lookup(BASE + PAGE_SIZE) == Some((physical + 2 * PAGE_SIZE, expected));
        refused && first_intact && blocker_intact
    })
}

fn a_shared_mapping_keeps_its_frame() -> bool {
    const PROBE: u64 = 0x0000_3500_0000_0000;
    in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some(run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        let physical = run.address();
        let mut shared = mm::USegment::from(run).share().into_bytes();
        if !shared.write_volatile_at(0, &[0xC7]) {
            return false;
        }
        let flags = Flags::NO_EXECUTE;
        if space.map_shared(PROBE, &shared, flags).is_err() {
            return false;
        }
        let held = mm::free_frames();

        drop(shared);
        let withheld = mm::free_frames() == held;
        let expected = flags.union(Flags::PRESENT).union(Flags::USER);
        let mapping_intact = space.lookup(PROBE) == Some((physical, expected));

        withheld && mapping_intact
    })
}

fn shared_bytes_reject_cache_alias(virtual_base: u64, flags: Flags) -> bool {
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let checked = match mm::allocate_contiguous(1, 1) {
        Some(mut run) => {
            if run.zero() {
                let shared = mm::USegment::from(run).share().into_bytes();
                let held = mm::free_frames();
                let refused = space.map_shared(virtual_base, &shared, flags)
                    == Err(MapError::CacheAlias(virtual_base))
                    && space.lookup(virtual_base).is_none();
                drop(shared);
                let owner_released = held.checked_add(1) == Some(mm::free_frames());
                refused && owner_released
            } else {
                false
            }
        }
        None => false,
    };
    let tables_unchanged = space.reclaim(None).is_ok_and(|tables| tables == 1);
    checked && tables_unchanged
}

fn shared_ring_reject_cache_alias(virtual_base: u64, flags: Flags) -> bool {
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let checked = match mm::allocate_contiguous(1, 1) {
        Some(mut run) => {
            if run.zero() {
                match SharedLayout::<1>::new(PAGE_SIZE as usize, [0]) {
                    Ok(layout) => match mm::USegment::from(run).share().into_ring(layout) {
                        Ok(shared) => {
                            let held = mm::free_frames();
                            let refused = space.map_shared_ring(virtual_base, &shared, flags)
                                == Err(MapError::CacheAlias(virtual_base))
                                && space.lookup(virtual_base).is_none();
                            drop(shared);
                            let owner_released = held.checked_add(1) == Some(mm::free_frames());
                            refused && owner_released
                        }
                        Err(_) => false,
                    },
                    Err(_) => false,
                }
            } else {
                false
            }
        }
        None => false,
    };
    let tables_unchanged = space.reclaim(None).is_ok_and(|tables| tables == 1);
    checked && tables_unchanged
}

fn shared_mappings_reject_cache_aliases() -> bool {
    const BYTES: u64 = 0x0000_3510_0000_0000;
    const RING: u64 = 0x0000_3520_0000_0000;
    let mut all_refused = true;
    for cache in [
        Flags::NO_CACHE,
        Flags::WRITE_THROUGH,
        Flags::NO_CACHE.union(Flags::WRITE_THROUGH),
    ] {
        let flags = Flags::NO_EXECUTE.union(cache);
        all_refused &= shared_bytes_reject_cache_alias(BYTES, flags);
        all_refused &= shared_ring_reject_cache_alias(RING, flags);
    }
    all_refused
}

fn ram_mappings_keep_the_direct_map_cache_type() -> bool {
    const OWNED: u64 = 0x0000_3530_0000_0000;
    const USER: u64 = 0x0000_3540_0000_0000;
    const BOOT: u64 = 0x0000_3550_0000_0000;

    let mut refused = true;
    for cache in [
        Flags::NO_CACHE,
        Flags::WRITE_THROUGH,
        Flags::NO_CACHE.union(Flags::WRITE_THROUGH),
    ] {
        let flags = Flags::NO_EXECUTE.union(cache);
        refused &= in_reclaimed_space(AddressSpace::for_user, |space| {
            let Some(owned) = mm::allocate_contiguous(1, 1) else {
                return false;
            };
            let owned_refused =
                space.map_owned(OWNED, owned, flags) == Err(MapError::CacheAlias(OWNED));
            let Some(user) = mm::allocate_contiguous(1, 1) else {
                return false;
            };
            let address = user.address();
            let user_refused = match space.map_user(USER, user.into(), flags) {
                Ok(()) => false,
                Err(error) => {
                    let reason = error.reason();
                    let run = error.into_run();
                    let returned = run.address() == address;
                    drop(run);
                    reason == MapError::CacheAlias(USER) && returned
                }
            };
            let Some(region) = probe_page(0) else {
                return false;
            };
            let boot_refused =
                space.map_boot(BOOT, region, flags) == Err(MapError::CacheAlias(BOOT));
            owned_refused
                && user_refused
                && boot_refused
                && [OWNED, USER, BOOT]
                    .into_iter()
                    .all(|at| space.lookup(at).is_none())
        });
    }

    // Each boot range maps with exactly its own direct-map type.
    let framebuffer = mm::memory_map_for_test().and_then(|map| {
        map.page_runs()
            .find(|run| run.cache() == wost::boot::DirectMapCache::Framebuffer)
            .map(|run| run.base())
    });
    let exact = in_reclaimed_space(AddressSpace::for_user, |space| {
        let (Some(ordinary), Some(base)) = (probe_page(0), framebuffer) else {
            return false;
        };
        let (Some(bare_screen), Some(screen)) = (
            BootRegion::describe(base, PAGE_SIZE),
            BootRegion::describe(base, PAGE_SIZE),
        ) else {
            return false;
        };
        let ordinary_mapped = space.map_boot(BOOT, ordinary, Flags::NO_EXECUTE).is_ok();
        let screen_refused = space.map_boot(OWNED, bare_screen, Flags::NO_EXECUTE)
            == Err(MapError::CacheAlias(OWNED));
        let screen_mapped = space
            .map_boot(USER, screen, Flags::NO_EXECUTE.union(Flags::WRITE_THROUGH))
            .is_ok();
        ordinary_mapped && screen_refused && screen_mapped
    });
    refused && exact
}

fn shared_topology_rejects_hostile_root_entries() -> bool {
    let huge = in_reclaimed_space(AddressSpace::new, |space| {
        space.rejects_huge_shared_topology_for_test()
    });
    let user = in_reclaimed_space(AddressSpace::new, |space| {
        space.rejects_user_shared_topology_for_test()
    });
    let supervisor = in_reclaimed_space(AddressSpace::new, |space| {
        space.reserves_supervisor_shared_topology_for_test()
    });
    huge && user && supervisor
}

fn a_large_space_returns_every_table_frame() -> bool {
    const BASE: u64 = 0x0000_1000_0000_0000;
    const TABLES: u64 = 34;

    let Some(physical) = mm::first_memory_run(PAGE_SIZE) else {
        return false;
    };
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    for index in 0..TABLES {
        let Some(region) = BootRegion::describe(physical, PAGE_SIZE) else {
            let reclaimed = space.reclaim(None).is_ok();
            drop(space);
            assert!(
                reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
                "a refused large-space description did not return its tables"
            );
            return false;
        };
        let Some(at) = index
            .checked_mul(LARGE_PAGE_SIZE)
            .and_then(|offset| BASE.checked_add(offset))
        else {
            let reclaimed = space.reclaim(None).is_ok();
            drop(space);
            assert!(
                reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
                "a refused large-space address did not return its tables"
            );
            return false;
        };
        if space.map_boot(at, region, Flags::NO_EXECUTE).is_err() {
            let reclaimed = space.reclaim(None).is_ok();
            drop(space);
            assert!(
                reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
                "a refused large-space map did not return its tables"
            );
            return false;
        }
    }

    let allocator_while_mapped = AllocatorSnapshot::capture();
    let Ok(returned) = space.reclaim(None) else {
        return false;
    };
    drop(space);
    let table_frames_held = allocator_before
        .accounted_physical_frames()
        .zip(allocator_while_mapped.accounted_physical_frames())
        .and_then(|(before, mapped)| before.checked_sub(mapped))
        == Some(returned);

    returned > 32
        && table_frames_held
        && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

fn an_unmapped_range_returns_its_frames() -> bool {
    /// More than one page, so the walk covers a run rather than a single slot.
    const PAGES: u64 = 4;
    let Some(probe) =
        thread::boot_stack_layout().and_then(|layout| layout.end().checked_add(16 * PAGE_SIZE))
    else {
        return false;
    };

    let free_before = mm::free_frames();
    let Some(run) = mm::allocate_contiguous(PAGES as usize, 1) else {
        return false;
    };
    let physical = run.address();

    let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let Some(Ok(mapping)) = mem::map_owned_for_test(&mut context(), probe, run, wanted) else {
        return false;
    };
    let mapped_here = paging::translate(probe).map(|(frame, _)| frame) == Some(physical);

    let free_while_mapped = mm::free_frames();

    let Some(Ok(token)) = mem::unmap_for_test(&mut context(), mapping) else {
        return false;
    };

    // The entries are gone the moment `unmap` returns...
    let gone = paging::translate(probe).is_none();
    // ...but the frames are not the allocator's yet.
    let withheld = mm::free_frames() == free_while_mapped && token.pages() == PAGES;

    let returned = token.into_kernel().is_some_and(|segment| {
        let same = segment.address() == physical && segment.frames() == PAGES as usize;
        drop(segment);
        same
    });
    let counted = mm::free_frames() == free_before
        && free_before.checked_sub(free_while_mapped) == Some(PAGES as usize);

    mapped_here && gone && withheld && returned && counted
}

fn unmapping_refuses_what_it_does_not_own() -> bool {
    mem::invalid_unmaps_are_refused(&mut context()).unwrap_or(false)
}

fn probe_page(index: u64) -> Option<BootRegion> {
    let base = mm::first_memory_run(PROBE_PAGES * PAGE_SIZE)?;
    BootRegion::describe(base.checked_add(index * PAGE_SIZE)?, PAGE_SIZE)
}

/// How many pages [`probe_page`] hands out, and the run it asks for.
const PROBE_PAGES: u64 = 2;

fn the_map_offers_a_run_to_probe() -> bool {
    probe_page(0).is_some() && probe_page(PROBE_PAGES - 1).is_some()
}

fn a_differing_remap_is_refused() -> bool {
    /// Canonical, and far from anything a fresh space would map.
    const PROBE: u64 = 0xffff_8800_0000_0000;

    in_reclaimed_space(AddressSpace::new, |space| {
        let (Some(here), Some(again_here), Some(elsewhere)) =
            (probe_page(0), probe_page(0), probe_page(1))
        else {
            return false;
        };
        let first = space.map_boot(PROBE, here, Flags::NO_EXECUTE);
        let again = space.map_boot(PROBE, again_here, Flags::NO_EXECUTE);
        let differing = space.map_boot(PROBE, elsewhere, Flags::NO_EXECUTE);
        first.is_ok() && again.is_ok() && differing == Err(MapError::Conflict(PROBE))
    })
}

fn a_boot_mapping_cannot_claim_ownership() -> bool {
    /// Canonical, and far from anything a fresh space maps.
    const PROBE: u64 = 0xffff_8880_0000_0000;

    in_reclaimed_space(AddressSpace::new, |space| {
        let Some(region) = probe_page(0) else {
            return false;
        };
        if space
            .map_boot(PROBE, region, Flags::NO_EXECUTE.union(Flags::OWNED))
            .is_err()
        {
            return false;
        }
        let provenance_removed = space
            .lookup(PROBE)
            .is_some_and(|(_, flags)| !flags.contains(Flags::OWNED));
        let before = mm::free_frames();
        // With the bit stripped, `unmap` sees a mapping nothing owns and
        // refuses, rather than minting a second owner of a boot frame.
        let refused = space.unmap(PROBE, PAGE_SIZE).err() == Some(MapError::NotOwned(PROBE));
        provenance_removed && refused && mm::free_frames() == before
    })
}

fn caller_global_is_removed_from_safe_mappings() -> bool {
    const USER: u64 = 0x0000_0100_0000_0000;
    const SHARED: u64 = 0x0000_0200_0000_0000;
    const OWNED: u64 = 0x0000_0300_0000_0000;
    const BOOT: u64 = 0x0000_0400_0000_0000;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let hostile = Flags::GLOBAL.union(Flags::NO_EXECUTE);

        let Some(mut user) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !user.zero() || space.map_user(USER, user.into(), hostile).is_err() {
            return false;
        }

        let Some(mut shared) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !shared.zero() {
            return false;
        }
        let shared = mm::USegment::from(shared).share().into_bytes();
        if space.map_shared(SHARED, &shared, hostile).is_err() {
            return false;
        }

        let Some(mut owned) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !owned.zero() || space.map_owned(OWNED, owned, hostile).is_err() {
            return false;
        }

        let Some(region) = probe_page(0) else {
            return false;
        };
        if space.map_boot(BOOT, region, hostile).is_err() {
            return false;
        }

        let stripped = [USER, SHARED, OWNED, BOOT].into_iter().all(|at| {
            space
                .lookup(at)
                .is_some_and(|(_, flags)| !flags.contains(Flags::GLOBAL))
        });
        let authorities_hold =
            space.lookup(USER).is_some_and(|(_, flags)| {
                flags.contains(Flags::USER) && flags.contains(Flags::OWNED)
            }) && space.lookup(SHARED).is_some_and(|(_, flags)| {
                flags.contains(Flags::USER) && !flags.contains(Flags::OWNED)
            }) && space.lookup(OWNED).is_some_and(|(_, flags)| {
                !flags.contains(Flags::USER) && flags.contains(Flags::OWNED)
            }) && space.lookup(BOOT).is_some_and(|(_, flags)| {
                !flags.contains(Flags::USER) && !flags.contains(Flags::OWNED)
            });

        let user_removed = space.unmap(USER, PAGE_SIZE).map(drop).is_ok();
        let owned_removed = space.unmap(OWNED, PAGE_SIZE).map(drop).is_ok();
        stripped && authorities_hold && user_removed && owned_removed
    })
}

fn a_boot_mapping_cannot_publish_kernel_memory() -> bool {
    const PROBE: u64 = 0x0000_0000_5000_0000;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some(region) = probe_page(0) else {
            return false;
        };
        space.map_boot(PROBE, region, Flags::USER.union(Flags::NO_EXECUTE))
            == Err(MapError::NotYours(PROBE))
            && space.lookup(PROBE).is_none()
    })
}

fn a_user_space_cannot_reach_the_kernel_half() -> bool {
    // The direct map ensures the hostile walk reaches real entries to blank.
    const KERNEL: u64 = 0xffff_8000_0010_0000;
    const HOLE_EDGE: u64 = x86_paging::USER_MAPPING_END;
    const LAST_USER: u64 = x86_paging::USER_MAPPING_END - PAGE_SIZE;

    in_reclaimed_space(AddressSpace::for_user, |user| {
        let Some(region) = probe_page(0) else {
            return false;
        };
        let unmapped = user.unmap(KERNEL, PAGE_SIZE);
        let mapped = user.map_boot(KERNEL, region, Flags::NO_EXECUTE);
        // The canonical-hole check precedes half ownership, so the straddle
        // must report that distinct refusal.
        let straddles =
            user.unmap(HOLE_EDGE, 2 * PAGE_SIZE).err() == Some(MapError::NonCanonical(HOLE_EDGE));
        // The last mappable user page reaches mapping lookup rather than half
        // refusal.
        let reaches_its_own =
            user.unmap(LAST_USER, PAGE_SIZE).err() != Some(MapError::NotYours(LAST_USER));

        unmapped.err() == Some(MapError::NotYours(KERNEL))
            && mapped == Err(MapError::NotYours(KERNEL))
            && straddles
            && reaches_its_own
    })
}

fn the_last_canonical_page_is_never_user_memory() -> bool {
    // Code ending on this page would trap with a non-canonical return
    // address, which `iretq` refuses in ring 0 (D-348).
    const TOP: u64 = x86_paging::USER_MAPPING_END;
    const BELOW: u64 = TOP - PAGE_SIZE;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some(run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        let address = run.address();
        let refused = match space.map_user(TOP, run.into(), Flags::NONE) {
            Ok(()) => false,
            Err(error) => {
                let reason = error.reason();
                let run = error.into_run();
                let returned = run.address() == address;
                drop(run);
                reason == MapError::NotYours(TOP) && returned
            }
        };
        let Some(below) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        let admitted = space.map_user(BELOW, below.into(), Flags::NONE).is_ok()
            && space.lookup(BELOW).is_some();
        let top_absent = space.lookup(TOP).is_none();
        let released = space
            .unmap(BELOW, PAGE_SIZE)
            .ok()
            .and_then(|unmapped| unmapped.into_user())
            .and_then(|user| user.reclaim())
            .is_some();
        refused && admitted && top_absent && released
    })
}

fn a_user_mapping_cannot_widen_a_supervisor_table() -> bool {
    /// Two addresses one page apart, so they share every table above the leaf.
    const SUPERVISOR: u64 = 0x0000_0000_4000_0000;
    const USER: u64 = SUPERVISOR + PAGE_SIZE;

    in_reclaimed_space(AddressSpace::new, |space| {
        let Some(first) = probe_page(0) else {
            return false;
        };
        if space
            .map_boot(SUPERVISOR, first, Flags::NO_EXECUTE)
            .is_err()
        {
            return false;
        }
        let Some(mut second) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !second.zero() {
            return false;
        }
        space
            .map_user(USER, second.into(), Flags::NO_EXECUTE)
            .err()
            .is_some_and(|error| error.reason() == MapError::Conflict(USER))
    })
}

fn an_undescribed_range_cannot_be_mapped() -> bool {
    /// Far above anything a PC memory map describes, and above `mm::CEILING`.
    const NOWHERE: u64 = 0x0000_7000_0000_0000;

    BootRegion::describe(NOWHERE, PAGE_SIZE).is_none()
        // Unaligned and empty ranges are refused before the map is consulted.
        && BootRegion::describe(PAGE_SIZE + 1, PAGE_SIZE).is_none()
        && BootRegion::describe(PAGE_SIZE, 0).is_none()
        // And a range that runs off the end of physical space cannot overflow
        // its way into looking described.
        && BootRegion::describe(u64::MAX - PAGE_SIZE + 1, PAGE_SIZE).is_none()
}

fn device_mappings_are_physically_exclusive() -> bool {
    const PROBE: u64 = 0xfee0_0000;
    let mut context = context();
    let Some(before) = mem::device_mappings(&mut context) else {
        return false;
    };
    let Some(Ok(first)) = mem::map_device(&mut context, PROBE + 4, 4) else {
        return false;
    };
    let Ok(first) = first.claim_releasable() else {
        return false;
    };
    let overlap = matches!(
        mem::map_device(&mut context, PROBE + 8, 4),
        Some(Err(MapError::DeviceOverlap(found))) if found == PROBE + 8
    );
    let removed = matches!(
        mem::unmap_device_exact(&mut context, first),
        mem::DeviceRetirement::Retired
    );
    overlap && removed && mem::device_mappings(&mut context) == Some(before)
}

fn device_mappings_refuse_allocatable_ram() -> bool {
    in_reclaimed_space(AddressSpace::new, |space| {
        let Some(frame) = mm::allocate() else {
            return false;
        };
        let address = frame.address();
        matches!(
            paging::admit_device_range(address, 4).and_then(|range| space.map_device(range)),
            Err(MapError::NotDeviceMemory(found)) if found == address
        )
    })
}

fn malformed_device_extents_are_refused() -> bool {
    in_reclaimed_space(AddressSpace::new, |space| {
        matches!(
            paging::admit_device_range(0xfee0_0000, 0).and_then(|range| space.map_device(range)),
            Err(MapError::InvalidDeviceRange(0xfee0_0000))
        ) && matches!(
            paging::admit_device_range(4, u64::MAX).and_then(|range| space.map_device(range)),
            Err(MapError::InvalidDeviceRange(4))
        )
    })
}

fn device_window_reuses_arbitrary_order_gaps() -> bool {
    fn exercise(context: &mut ThreadContext, physical: u64) -> Option<bool> {
        let before = mem::device_mappings(context)?;
        let first = match mem::map_device(context, physical, PAGE_SIZE)? {
            Ok(mapping) => mapping,
            Err(MapError::DeviceOverlap(_) | MapError::NotDeviceMemory(_)) => return None,
            Err(_) => return Some(false),
        };
        let first_address = first.address();
        let Ok(first) = first.claim_releasable() else {
            return Some(false);
        };

        let second = match mem::map_device(context, physical + PAGE_SIZE, PAGE_SIZE)? {
            Ok(mapping) => mapping,
            Err(MapError::DeviceOverlap(_) | MapError::NotDeviceMemory(_)) => {
                if matches!(
                    mem::unmap_device_exact(context, first),
                    mem::DeviceRetirement::Retired
                ) {
                    return None;
                }
                return Some(false);
            }
            Err(_) => return Some(false),
        };
        let Ok(second) = second.claim_releasable() else {
            return Some(false);
        };

        paging::refuse_device_unmap_for_test();
        let first_removed = match mem::unmap_device(context, first) {
            mem::DeviceUnmapOutcome::Refused(failure) => {
                let unchanged = mem::device_mappings(context) == Some(before + 2);
                unchanged
                    && matches!(
                        mem::unmap_device_exact(context, failure.region),
                        mem::DeviceRetirement::Retired
                    )
            }
            _ => false,
        };
        let second_removed = matches!(
            mem::unmap_device_exact(context, second),
            mem::DeviceRetirement::Retired
        );
        if !first_removed || !second_removed {
            return Some(false);
        }

        let third = match mem::map_device(context, physical, PAGE_SIZE)? {
            Ok(mapping) => mapping,
            Err(_) => return Some(false),
        };
        let reused = third.address() == first_address;
        let Ok(third) = third.claim_releasable() else {
            return Some(false);
        };
        let restored = matches!(
            mem::unmap_device_exact(context, third),
            mem::DeviceRetirement::Retired
        ) && mem::device_mappings(context) == Some(before);
        Some(reused && restored)
    }

    let Some(map) = mm::memory_map_for_test() else {
        return false;
    };
    let mut context = context();
    let mut attempted = 0usize;
    for region in map.regions().filter(|region| {
        matches!(
            region.kind,
            boot::MemoryKind::Reserved | boot::MemoryKind::Unknown(_)
        )
    }) {
        let Some(mut physical) = region.base.checked_next_multiple_of(PAGE_SIZE) else {
            continue;
        };
        let region_end = region.base + region.len;
        while attempted < 256 {
            let Some(end) = physical.checked_add(PAGE_SIZE * 2) else {
                break;
            };
            if end > region_end {
                break;
            }
            attempted += 1;
            let exclusively_device = map.regions().all(|other| {
                let overlaps = physical < other.base + other.len && other.base < end;
                !overlaps
                    || matches!(
                        other.kind,
                        boot::MemoryKind::Reserved | boot::MemoryKind::Unknown(_)
                    )
            });
            if exclusively_device && let Some(result) = exercise(&mut context, physical) {
                return result;
            }
            let Some(next) = physical.checked_add(PAGE_SIZE) else {
                break;
            };
            physical = next;
        }
    }
    false
}

struct DeviceLiveness {
    shared_topology: bool,
    stale_root: bool,
    first: bool,
    middle: bool,
    last: bool,
    cache_policy: bool,
}

fn map_three_page_device_extent(context: &mut ThreadContext) -> Option<paging::DeviceMapping> {
    const EXTENT: u64 = 3 * PAGE_SIZE;

    let map = mm::memory_map_for_test()?;
    let mut attempted = 0usize;
    for region in map.regions().filter(|region| {
        matches!(
            region.kind,
            boot::MemoryKind::Reserved | boot::MemoryKind::Unknown(_)
        )
    }) {
        let Some(mut physical) = region.base.checked_next_multiple_of(PAGE_SIZE) else {
            continue;
        };
        let Some(region_end) = region.base.checked_add(region.len) else {
            continue;
        };
        while attempted < 256 {
            let Some(end) = physical.checked_add(EXTENT) else {
                break;
            };
            if end > region_end {
                break;
            }
            attempted += 1;
            match mem::map_device(context, physical, EXTENT)? {
                Ok(mapping) => return Some(mapping),
                Err(MapError::DeviceOverlap(_) | MapError::NotDeviceMemory(_)) => {}
                Err(_) => return None,
            }
            let Some(next) = physical.checked_add(PAGE_SIZE) else {
                break;
            };
            physical = next;
        }
    }
    None
}

fn device_liveness_covers_the_current_root_and_extent() -> Option<DeviceLiveness> {
    let mut context = context();
    let before = mem::device_mappings(&mut context)?;
    let mut stale = AddressSpace::for_user().ok()?;
    let Some(mapping) = map_three_page_device_extent(&mut context) else {
        if let Err(reason) = stale.reclaim(None) {
            panic!("device liveness fixture could not reclaim its unused root: {reason}");
        }
        return None;
    };

    let address = mapping.address();
    let live_before = paging::translate(address);
    let shared_upper = live_before.is_some() && stale.lookup(address) == live_before;
    let (mapping, first) = mapping.rejects_physical_substitution_for_test(0);
    let (mapping, middle) = mapping.rejects_physical_substitution_for_test(1);
    let (mapping, last) = mapping.rejects_physical_substitution_for_test(2);
    let (mapping, cache_policy) = mapping.rejects_cache_substitution_for_test(1);

    let (loaded, under_stale) = mapping.claim_releasable_under_for_test(&stale);
    let (stale_root, region) = match under_stale {
        Err(mapping) => {
            let Ok(region) = mapping.claim_releasable() else {
                panic!("a restored device mapping could not be reclaimed by its test");
            };
            (loaded, region)
        }
        Ok(region) => (false, region),
    };
    let unchanged = paging::translate(address) == live_before;
    let removed = matches!(
        mem::unmap_device_exact(&mut context, region),
        mem::DeviceRetirement::Retired
    );
    let reclaimed = stale.reclaim(None).is_ok();
    let cleaned =
        removed && reclaimed && mem::device_mappings(&mut context) == Some(before) && unchanged;

    Some(DeviceLiveness {
        shared_topology: shared_upper && cleaned,
        stale_root: stale_root && cleaned,
        first: first && cleaned,
        middle: middle && cleaned,
        last: last && cleaned,
        cache_policy: cache_policy && cleaned,
    })
}

fn active_device_tables_cannot_be_reclaimed() -> bool {
    mem::active_space_reclaim_is_guarded(&mut context()) == Some(true)
}

fn active_kernel_topology_is_frozen() -> bool {
    mem::active_kernel_topology_is_frozen(&mut context()) == Some(true)
}

fn the_editing_keys_move_the_cursor() -> bool {
    let (Some(backspace), Some(newline)) = (
        ps2_keys::us::char_for(ps2_keys::Key::Backspace, ps2_keys::Shift::new()),
        ps2_keys::us::char_for(ps2_keys::Key::Enter, ps2_keys::Shift::new()),
    ) else {
        return false;
    };

    let Some(((start_x, start_y), (glyph_width, line_height))) = dev::console_cursor() else {
        return false;
    };

    dev::with_console(|console| {
        let _ = console.write_char('X');
    });
    let Some(((typed_x, typed_y), _)) = dev::console_cursor() else {
        return false;
    };
    let advanced = typed_x == start_x + glyph_width && typed_y == start_y;

    dev::with_console(|console| {
        let _ = console.write_char(backspace);
    });
    let Some(((erased_x, erased_y), _)) = dev::console_cursor() else {
        return false;
    };
    let erased = erased_x == start_x && erased_y == start_y;

    // From a column past the margin, so the return to it is observable.
    dev::with_console(|console| {
        let _ = console.write_char('X');
        let _ = console.write_char(newline);
    });
    let Some(((home_x, home_y), _)) = dev::console_cursor() else {
        return false;
    };
    // The margin, not `home_x <= start_x`, which holds for every reset; the
    // margin is where `erase_previous` refuses to go below (D-105).
    let started_a_line =
        home_x == crate::console::MARGIN && home_x <= typed_x && home_y == start_y + line_height;

    advanced && erased && started_a_line
}

fn the_framebuffer_is_write_combining() -> bool {
    cpu::write_combining_is_enabled()
        && dev::framebuffer_address()
            .and_then(paging::translate)
            .is_some_and(|(_, flags)| {
                flags.contains(Flags::WRITE_THROUGH)
                    && !flags.contains(Flags::NO_CACHE)
                    && !flags.is_write_execute()
            })
}

fn default_deny_interrupt_remapping_guards_pci() -> bool {
    iommu::interrupt_boundary_is_deny_all_for_test()
        && iommu::interrupt_boundary_active()
        && !input::is_interrupt_driven()
}

fn the_io_apic_owner_was_returned_after_masked_rollback() -> bool {
    crate::keyboard_routing_rollback_is_exact()
        && dev::io_apic_base().is_none()
        && dev::routing_of(dev::keyboard_gsi()).is_none()
}

fn keyboard_input_is_polled_without_legacy_interrupts() -> bool {
    !input::is_interrupt_driven()
        && input::interrupt_count() == 0
        && input::poller_started_for_test()
        && input::poller_passes() > 0
}

fn the_heap_reuses_a_freed_slot() -> bool {
    // Warm the class, so what is measured below is a reuse and not the refill
    // that any first allocation of a class must pay for.
    drop(core::hint::black_box(Box::new(0u32)));

    let before = mm::heap::stats();
    let boxed = core::hint::black_box(Box::new(0x5A5A_5A5Au32));
    let taken = mm::heap::stats().live == before.live + 1;
    drop(boxed);
    let after = mm::heap::stats();
    let balanced = taken && after.live == before.live && after.slab_frames == before.slab_frames;

    // Accounting cannot see a free-list leak; LIFO reuse must return the
    // same address, with `black_box` ordering both allocations (D-124).
    let first = core::hint::black_box(Box::new(0u32));
    let address = core::hint::black_box(core::ptr::from_ref(&*first) as usize);
    drop(first);
    let second = core::hint::black_box(Box::new(0u32));
    let reused = core::hint::black_box(core::ptr::from_ref(&*second) as usize) == address;
    drop(second);

    balanced && reused
}

fn a_refill_costs_one_frame_and_balances() -> bool {
    /// Enough allocations of one class to span several slab frames.
    const TAKEN: usize = 512;

    // The vector's buffer outlives all three snapshots, so only the slots
    // under test move between them.
    let mut held: Vec<Box<[u8; 64]>> = Vec::with_capacity(TAKEN);
    let before = mm::heap::stats();
    for index in 0..TAKEN {
        held.push(Box::new([index as u8; 64]));
    }
    let after_taking = mm::heap::stats();

    // Each slot still holds what it was given, which is what a slot handed out
    // twice would have overwritten.
    let intact = held
        .iter()
        .enumerate()
        .all(|(index, slot)| slot.iter().all(|byte| *byte == index as u8));

    held.clear();
    let after = mm::heap::stats();

    let refilled = after_taking.slab_frames > before.slab_frames;
    let took_every_one = after_taking.live == before.live + TAKEN;
    let balanced = after.live == before.live && after.slab_frames == after_taking.slab_frames;

    refilled && took_every_one && intact && balanced
}

fn heap_memory_holds_what_it_is_given() -> bool {
    let mut values: Vec<u32> = (0..1024).collect();
    values
        .iter_mut()
        .for_each(|value| *value = value.wrapping_mul(3));

    values.len() == 1024
        && values
            .iter()
            .enumerate()
            .all(|(i, v)| *v == (i as u32).wrapping_mul(3))
}

fn a_large_allocation_takes_and_returns_frames() -> bool {
    const FRAMES: usize = 4;

    let frames_before = mm::free_frames();
    let heap_before = mm::heap::stats();
    let large: Vec<u8> = alloc::vec![0x42; FRAMES * 4096];
    let heap_taken = mm::heap::stats();
    let taken = frames_before.checked_sub(mm::free_frames()) == Some(FRAMES)
        && heap_taken.large_frames == heap_before.large_frames + FRAMES
        && heap_taken.live == heap_before.live + 1;
    let held = large.iter().all(|byte| *byte == 0x42);
    drop(large);

    taken && held && mm::free_frames() == frames_before && mm::heap::stats() == heap_before
}

fn context() -> ThreadContext {
    ThreadContext::require()
}

/// Yields a thread may spend waiting for something before the check gives up
/// — so a switch that never hands control over is a red verdict, not a hang.
const YIELD_BUDGET: usize = 100_000;

/// Turns the alternation check asks for.
const TURNS: usize = 64;

/// Whose turn it is: zero for the first thread, one for the second.
static TURN: AtomicUsize = AtomicUsize::new(0);

/// Turns taken so far, each one strictly after the other thread's.
static TURNS_TAKEN: AtomicUsize = AtomicUsize::new(0);

fn take_turns(mine: usize) {
    let mut budget = YIELD_BUDGET;
    while TURNS_TAKEN.load(Ordering::Acquire) < TURNS && budget > 0 {
        if TURN.load(Ordering::Acquire) == mine {
            TURNS_TAKEN.fetch_add(1, Ordering::AcqRel);
            TURN.store(1 - mine, Ordering::Release);
        } else {
            budget -= 1;
            thread::yield_now(&mut context());
        }
    }
}

fn first_turns() {
    take_turns(0);
}

fn second_turns() {
    take_turns(1);
}

fn yields_until(done: impl Fn() -> bool) -> bool {
    (0..YIELD_BUDGET).any(|_| {
        thread::yield_now(&mut context());
        done()
    })
}

fn threads_alternate_strictly() -> bool {
    TURN.store(0, Ordering::Release);
    TURNS_TAKEN.store(0, Ordering::Release);
    let free = thread::free_slots();

    if thread::spawn("first", first_turns).is_err()
        || thread::spawn("second", second_turns).is_err()
    {
        return false;
    }
    let alternated = yields_until(|| TURNS_TAKEN.load(Ordering::Acquire) >= TURNS);
    let exited = yields_until(|| thread::free_slots() == free);

    alternated && exited && TURNS_TAKEN.load(Ordering::Acquire) == TURNS
}

/// What the register-churning thread concluded: zero while it is still
/// running, one if its values survived every switch, two if they did not.
static CHURN_VERDICT: AtomicUsize = AtomicUsize::new(0);

/// Rounds of churn, each ending in a yield.
const CHURN_ROUNDS: u64 = 256;

fn churn(seed: u64) -> bool {
    let (mut a, mut b, mut c, mut d, mut e) = (
        core::hint::black_box(seed),
        core::hint::black_box(seed.rotate_left(7)),
        core::hint::black_box(seed.rotate_left(14)),
        core::hint::black_box(seed.rotate_left(21)),
        core::hint::black_box(seed.rotate_left(28)),
    );
    for round in 0..CHURN_ROUNDS {
        a = a.wrapping_mul(MULTIPLIER).wrapping_add(round);
        b = b.wrapping_mul(MULTIPLIER).wrapping_add(a);
        c = c.wrapping_mul(MULTIPLIER).wrapping_add(b);
        d = d.wrapping_mul(MULTIPLIER).wrapping_add(c);
        e = e.wrapping_mul(MULTIPLIER).wrapping_add(d);
        thread::yield_now(&mut context());
    }

    let (mut p, mut q, mut r, mut s, mut t) = (
        seed,
        seed.rotate_left(7),
        seed.rotate_left(14),
        seed.rotate_left(21),
        seed.rotate_left(28),
    );
    for round in 0..CHURN_ROUNDS {
        p = p.wrapping_mul(MULTIPLIER).wrapping_add(round);
        q = q.wrapping_mul(MULTIPLIER).wrapping_add(p);
        r = r.wrapping_mul(MULTIPLIER).wrapping_add(q);
        s = s.wrapping_mul(MULTIPLIER).wrapping_add(r);
        t = t.wrapping_mul(MULTIPLIER).wrapping_add(s);
    }
    (a, b, c, d, e) == (p, q, r, s, t)
}

/// Knuth's multiplier, for a sequence with no short cycle.
const MULTIPLIER: u64 = 6_364_136_223_846_793_005;

fn churn_and_report() {
    let survived = churn(0x9E37_79B9_7F4A_7C15);
    CHURN_VERDICT.store(if survived { 1 } else { 2 }, Ordering::Release);
}

fn registers_survive_a_switch() -> bool {
    CHURN_VERDICT.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("churn", churn_and_report).is_err() {
        return false;
    }

    let boot_survived = churn(0xD1B5_4A32_D192_ED03);
    let exited = yields_until(|| thread::free_slots() == free);

    boot_survived && exited && CHURN_VERDICT.load(Ordering::Acquire) == 1
}

fn a_leaf_guard_is_the_last_lock() -> bool {
    match wost::sync::taken_under_a_leaf() {
        None => true,
        Some(at) => {
            match wost::sync::the_leaf_it_was_under() {
                Some(leaf) => error!("{at} took a lock under the leaf guard at {leaf}"),
                None => error!("a lock was taken under a leaf guard at {at}"),
            }
            false
        }
    }
}

fn a_scroll_moves_one_visible_cursor() -> bool {
    const AT_X: usize = crate::console::MARGIN;

    let Some(((_, mut cursor_y), (_, line_height))) = dev::console_cursor() else {
        return false;
    };
    let Some(geometry) = dev::console_geometry() else {
        return false;
    };

    let mut scrolled_from = None;
    for _ in 0..=geometry.height / line_height.max(1) {
        dev::with_console(|console| console.write_line(""));
        let Some(((.., next_y), _)) = dev::console_cursor() else {
            return false;
        };
        if next_y == cursor_y {
            scrolled_from = Some(cursor_y);
            break;
        }
        cursor_y = next_y;
    }

    // The cursor underline moves one pixel above its prior origin. Blank input
    // leaves no other legitimate foreground mark there.
    let Some(probe_y) = scrolled_from.and_then(|y| y.checked_sub(1)) else {
        return false;
    };
    let Some((old_pixel, foreground, _)) = dev::console_probe(AT_X, probe_y) else {
        return false;
    };
    let Some(((cursor_x, cursor_y), (_, cursor_height))) = dev::console_cursor() else {
        return false;
    };
    let Some(cursor_y) = cursor_y.checked_add(cursor_height - 1) else {
        return false;
    };
    let Some((cursor_pixel, cursor_foreground, _)) = dev::console_probe(cursor_x, cursor_y) else {
        return false;
    };
    old_pixel != foreground && cursor_pixel == cursor_foreground
}

fn a_stack_stops_being_installed_when_it_is_unmapped() -> bool {
    let mut context = context();
    let free_before = mm::free_frames();

    // Above every stack the kernel maps, so nothing else is in this window.
    let Some(above) = thread::boot_stack_layout()
        .and_then(|boot| StackLayout::new(boot.end(), 2 * PAGE_SIZE, PAGE_SIZE, 1))
    else {
        return false;
    };
    let Some(slot) = above.slot(0) else {
        return false;
    };
    let Some(Ok((stack, mapping))) = mem::map_reclaimable_stack_for_test(&mut context, above)
    else {
        return false;
    };

    let installed = stack.is_installed();
    let Some((lower_mapping, top_mapping)) = mem::split_mapping_for_test(mapping, PAGE_SIZE) else {
        return false;
    };

    let Some(Ok(lower_taken)) = mem::unmap_for_test(&mut context, lower_mapping) else {
        return false;
    };
    let partial_is_not_the_stack = !stack.is_installed();
    let Some(lower_original) = lower_taken.into_kernel() else {
        return false;
    };
    let Some(Ok(top_taken)) = mem::unmap_for_test(&mut context, top_mapping) else {
        return false;
    };
    let gone = !stack.is_installed();
    let Some(top_original) = top_taken.into_kernel() else {
        return false;
    };

    let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let Some(replacement) = mm::allocate_contiguous(2, 1) else {
        return false;
    };
    let physically_different = replacement.address() != lower_original.address();
    let Some(Ok(replacement_mapping)) =
        mem::map_owned_for_test(&mut context, slot.stack_base(), replacement, wanted)
    else {
        return false;
    };
    let substitution_is_not_the_stack = !stack.is_installed();
    let Some(Ok(replacement_taken)) = mem::unmap_for_test(&mut context, replacement_mapping) else {
        return false;
    };
    drop(replacement_taken.into_kernel());
    drop(lower_original);
    drop(top_original);

    installed
        && gone
        && partial_is_not_the_stack
        && physically_different
        && substitution_is_not_the_stack
        && mm::free_frames() == free_before
}

struct StackLiveness {
    stale_root: bool,
    first: bool,
    middle: bool,
    last: bool,
}

fn stack_liveness_covers_the_current_root_and_extent() -> Option<StackLiveness> {
    let allocator_before = AllocatorSnapshot::capture();
    let layout = thread::boot_stack_layout()
        .and_then(|boot| StackLayout::new(boot.end(), 3 * PAGE_SIZE, PAGE_SIZE, 1))?;
    let slot = layout.slot(0)?;
    let mut stale = AddressSpace::for_user().ok()?;
    let mut context = context();
    let (stack, mapping) = match mem::map_reclaimable_stack_for_test(&mut context, layout) {
        Some(Ok(mapped)) => mapped,
        Some(Err(reason @ paging::StackMapError::Rollback { .. })) => {
            panic!("stack liveness fixture stranded cleanup authority: {reason}");
        }
        Some(Err(paging::StackMapError::Mapping(_))) | None => {
            if let Err(reason) = stale.reclaim(None) {
                panic!("stack liveness fixture could not reclaim its unused root: {reason}");
            }
            return None;
        }
        Some(Err(reason)) => {
            panic!("stack liveness fixture refused with an unknown cleanup state: {reason}");
        }
    };

    let live_before = paging::translate(slot.stack_base());
    let shared_upper = live_before.is_some() && stale.lookup(slot.stack_base()) == live_before;
    let stale_root = stack.is_installed_under_for_test(&stale) == Some(false)
        && stack.is_installed()
        && paging::translate(slot.stack_base()) == live_before;
    let first = stack.rejects_substituted_page_for_test(0) == Some(true);
    let middle = stack.rejects_substituted_page_for_test(1) == Some(true);
    let last = stack.rejects_substituted_page_for_test(2) == Some(true);

    let removed =
        mem::unmap_for_test(&mut context, mapping).is_some_and(|result| result.map(drop).is_ok());
    let reclaimed = stale.reclaim(None).is_ok();
    drop(stale);
    let cleaned =
        removed && reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before);
    Some(StackLiveness {
        stale_root: stale_root && shared_upper && cleaned,
        first: first && cleaned,
        middle: middle && cleaned,
        last: last && cleaned,
    })
}

fn task_init_rejects_a_changed_stack_arena() -> bool {
    let allocator_before = AllocatorSnapshot::capture();
    let Some(layout) = thread::boot_stack_layout()
        .and_then(|boot| boot.end().checked_add(8 * PAGE_SIZE))
        .and_then(|base| StackLayout::new(base, 2 * PAGE_SIZE, PAGE_SIZE, 1))
    else {
        return false;
    };
    let Some(slot) = layout.slot(0) else {
        return false;
    };
    let mut context = context();
    let Some(Ok((arena, stack_mapping))) = mem::map_stack_arena_for_test(&mut context, layout)
    else {
        return false;
    };
    let Some((lower_mapping, upper_mapping)) =
        mem::split_mapping_for_test(stack_mapping, PAGE_SIZE)
    else {
        return false;
    };
    let admitted = task::validate_arena_for_test(&arena) == Ok(());
    let upper_address = slot.stack_base() + PAGE_SIZE;
    let upper_before = paging::translate(upper_address);

    let Some(guard) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let Some(Ok(guard_mapping)) =
        mem::map_owned_for_test(&mut context, slot.guard_base(), guard, wanted)
    else {
        return false;
    };
    let guard_refused = matches!(
        task::validate_arena_for_test(&arena),
        Err(task::TaskError::Unmapped(address)) if address == slot.guard_base()
    );
    let guard_removed = mem::unmap_for_test(&mut context, guard_mapping)
        .is_some_and(|result| result.map(drop).is_ok());
    let restored = task::validate_arena_for_test(&arena) == Ok(());

    let Some(Ok(unmapped)) = mem::unmap_for_test(&mut context, lower_mapping) else {
        return false;
    };
    let Some(original) = unmapped.into_kernel() else {
        return false;
    };
    let Some(replacement) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let physically_different = replacement.address() != original.address();
    let Some(Ok(replacement_mapping)) =
        mem::map_owned_for_test(&mut context, slot.stack_base(), replacement, Flags::NONE)
    else {
        return false;
    };
    let leaf_refused = matches!(
        task::validate_arena_for_test(&arena),
        Err(task::TaskError::Unmapped(address)) if address == slot.stack_base()
    );
    let upper_unchanged =
        upper_before.is_some() && paging::translate(upper_address) == upper_before;
    let replacement_removed = mem::unmap_for_test(&mut context, replacement_mapping)
        .is_some_and(|result| result.map(drop).is_ok());
    let upper_removed = mem::unmap_for_test(&mut context, upper_mapping)
        .is_some_and(|result| result.map(drop).is_ok());
    drop(arena);
    drop(original);

    admitted
        && guard_refused
        && guard_removed
        && restored
        && physically_different
        && leaf_refused
        && upper_unchanged
        && replacement_removed
        && upper_removed
        && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

fn thread_stacks_are_guarded() -> bool {
    let Some(layout) = thread::layout() else {
        return false;
    };
    let exact = (0..layout.slots()).all(|index| {
        let Some(slot) = layout.slot(index) else {
            return false;
        };
        let guard_absent = paging::translate(slot.guard_base()).is_none()
            && paging::translate(slot.stack_base() - 8).is_none();
        let stack_present = [slot.stack_base(), slot.stack_top() - 8]
            .into_iter()
            .all(|address| match paging::translate(address) {
                Some((_, flags)) => {
                    flags.contains(Flags::WRITABLE)
                        && flags.contains(Flags::NO_EXECUTE)
                        && !flags.is_write_execute()
                }
                None => false,
            });
        guard_absent && stack_present
    });
    let pinned = layout
        .slot(0)
        .is_some_and(|slot| pinned_guard_refuses_mapping(&slot));
    let init_validation = task_init_rejects_a_changed_stack_arena();
    exact && pinned && init_validation
}

/// What a thread found the interrupt flag to be on its first instruction:
/// zero before it ran, one for set, two for clear.
static FIRST_RUN_FLAG: AtomicUsize = AtomicUsize::new(0);
static FIRST_RUN_CONTEXT: AtomicUsize = AtomicUsize::new(0);

fn report_the_flag() {
    FIRST_RUN_CONTEXT.store(
        interrupt::context_matches_for_test(task::current()) as usize,
        Ordering::Release,
    );
    FIRST_RUN_FLAG.store(
        if interrupt::are_enabled() { 1 } else { 2 },
        Ordering::Release,
    );
}

fn a_new_thread_is_interruptible() -> bool {
    FIRST_RUN_FLAG.store(0, Ordering::Release);
    FIRST_RUN_CONTEXT.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("flag", report_the_flag).is_err() {
        return false;
    }

    let ran = yields_until(|| FIRST_RUN_FLAG.load(Ordering::Acquire) != 0);
    let exited = yields_until(|| thread::free_slots() == free);

    ran && exited
        && FIRST_RUN_FLAG.load(Ordering::Acquire) == 1
        && FIRST_RUN_CONTEXT.load(Ordering::Acquire) == 1
        && interrupt::context_matches_for_test(task::current())
        && interrupt::exclusion_order_is_enforced_for_test()
}

/// Where [`report_a_local_address`] leaves its probe, or zero until it runs.
static ALIGNMENT_PROBE: AtomicUsize = AtomicUsize::new(0);

fn report_a_local_address() {
    #[repr(align(16))]
    struct Probe {
        _pad: [u8; 16],
    }

    let probe = Probe { _pad: [0; 16] };
    ALIGNMENT_PROBE.store(
        core::hint::black_box(&raw const probe as usize),
        Ordering::Release,
    );
}

fn a_thread_starts_aligned() -> bool {
    ALIGNMENT_PROBE.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("aligned", report_a_local_address).is_err() {
        return false;
    }

    let ran = yields_until(|| ALIGNMENT_PROBE.load(Ordering::Acquire) != 0);
    let exited = yields_until(|| thread::free_slots() == free);

    ran && exited && ALIGNMENT_PROBE.load(Ordering::Acquire).is_multiple_of(16)
}

fn leave_at_once() {}

const REUSED_SLOT_SECRET: xstate::Probe = xstate::Probe {
    x87: 0x1357_2468_1234_5678,
    xmm: 0xA5A5_5A5A_1122_3344,
    ymm: 0xC3C3_3C3C_5566_7788,
};

static REUSED_SLOT_STATE_CLEARED: AtomicUsize = AtomicUsize::new(0);
static REUSED_SLOT_SECRET_WRITTEN: AtomicUsize = AtomicUsize::new(0);

fn leave_extended_state() {
    let written = xstate::write_probe(REUSED_SLOT_SECRET);
    REUSED_SLOT_SECRET_WRITTEN.store(written as usize, Ordering::Release);
}

fn inspect_reused_extended_state() {
    let cleared = xstate::read_probe().is_some_and(|found| {
        found.x87 != REUSED_SLOT_SECRET.x87 && found.xmm == 0 && found.ymm == 0
    });
    REUSED_SLOT_STATE_CLEARED.store(cleared as usize, Ordering::Release);
}

fn an_exited_slot_is_reused() -> bool {
    let free = thread::free_slots();
    if thread::spawn("brief", leave_at_once).is_err() {
        return false;
    }
    let returned = yields_until(|| thread::free_slots() == free);

    let again = thread::spawn("brief again", leave_at_once).is_ok();
    let returned_again = yields_until(|| thread::free_slots() == free);

    let state_cleared = if xstate::is_enabled() {
        REUSED_SLOT_SECRET_WRITTEN.store(0, Ordering::Release);
        REUSED_SLOT_STATE_CLEARED.store(0, Ordering::Release);
        let secret_slot = thread::spawn("state owner", leave_extended_state).ok();
        let secret_returned = yields_until(|| thread::free_slots() == free);
        let clean_slot = thread::spawn("state successor", inspect_reused_extended_state).ok();
        let clean_returned = yields_until(|| thread::free_slots() == free);
        secret_returned
            && clean_returned
            && secret_slot == clean_slot
            && REUSED_SLOT_SECRET_WRITTEN.load(Ordering::Acquire) == 1
            && REUSED_SLOT_STATE_CLEARED.load(Ordering::Acquire) == 1
    } else {
        true
    };

    returned && again && returned_again && state_cleared
}

/// Set to tell every spinner to finish.
static STOP_SPINNING: AtomicUsize = AtomicUsize::new(0);

/// How many loops each spinner has run: one counter per weight class.
static LIGHT_SPINS: AtomicUsize = AtomicUsize::new(0);
static HEAVY_SPINS: AtomicUsize = AtomicUsize::new(0);

fn spin_counting(counter: &AtomicUsize) {
    while STOP_SPINNING.load(Ordering::Acquire) == 0 {
        counter.fetch_add(1, Ordering::Relaxed);
        spin_loop();
    }
}

fn light_spinner() {
    spin_counting(&LIGHT_SPINS);
}

fn heavy_spinner() {
    spin_counting(&HEAVY_SPINS);
}

fn runs_until(ms: u64, done: impl Fn() -> bool) -> bool {
    if !the_clock_is_running() {
        return false;
    }
    let deadline = time::uptime_ms() + ms;
    loop {
        if done() {
            return true;
        }
        if time::uptime_ms() >= deadline {
            return false;
        }
        spin_loop();
    }
}

fn the_clock_is_running() -> bool {
    const ATTEMPTS: u32 = 1_000_000;

    let start = time::now_ns();
    time::is_calibrated() && spins_until(ATTEMPTS, || time::now_ns() > start)
}

fn a_spinner_cannot_starve_another() -> bool {
    /// Generous beside a 3 ms slice, stingy beside a hung harness.
    const PATIENCE_MS: u64 = 500;

    STOP_SPINNING.store(0, Ordering::Release);
    LIGHT_SPINS.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("spinner", light_spinner).is_err() {
        return false;
    }

    let progressed = runs_until(PATIENCE_MS, || LIGHT_SPINS.load(Ordering::Relaxed) > 0);
    STOP_SPINNING.store(1, Ordering::Release);
    let exited = yields_until(|| thread::free_slots() == free);

    progressed && exited
}

fn runtime_divides_by_weight() -> bool {
    /// Long enough for a couple of hundred slices between them.
    const WINDOW_MS: u64 = 600;

    STOP_SPINNING.store(0, Ordering::Release);
    LIGHT_SPINS.store(0, Ordering::Release);
    HEAVY_SPINS.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn_weighted_for_test("light", light_spinner, eevdf::BASE_WEIGHT).is_err()
        || thread::spawn_weighted_for_test("heavy", heavy_spinner, 2 * eevdf::BASE_WEIGHT).is_err()
    {
        return false;
    }

    runs_until(WINDOW_MS, || false);
    STOP_SPINNING.store(1, Ordering::Release);
    let exited = yields_until(|| thread::free_slots() == free);

    let light = LIGHT_SPINS.load(Ordering::Relaxed);
    let heavy = HEAVY_SPINS.load(Ordering::Relaxed);
    // Integer bounds: 1.4 ≤ heavy / light ≤ 2.6, without dividing.
    let ratio_in_bounds = light > 0 && heavy * 10 >= light * 14 && heavy * 10 <= light * 26;

    exited && ratio_in_bounds
}

/// What the blocked reader saw: zero until it has anything to say.
static READER_SAW: AtomicUsize = AtomicUsize::new(0);

/// Whether the reader actually stopped being runnable to get its keystroke:
/// zero until it knows, one for blocked, two for not.
///
/// Recorded across the read because key arrival alone cannot distinguish a
/// blocked reader from a polling one.
static READER_BLOCKED: AtomicUsize = AtomicUsize::new(0);

fn read_one_key() {
    let mut context = context();
    let blocks = thread::blocks();
    let keystroke = input::read_key(&mut context);
    READER_BLOCKED.store(
        if thread::blocks() > blocks { 1 } else { 2 },
        Ordering::Release,
    );
    READER_SAW.store(keystroke.event.key as usize + 1, Ordering::Release);
}

fn a_blocked_reader_waits_while_the_poller_runs() -> bool {
    READER_SAW.store(0, Ordering::Release);
    READER_BLOCKED.store(0, Ordering::Release);

    let blocks = thread::blocks();
    if thread::spawn("reader", read_one_key).is_err() {
        return false;
    }
    // Long enough for the reader to be picked, run, and block; short beside
    // everything between here and the injection.
    let blocked = runs_until(50, || thread::blocks() > blocks);

    let passes = input::poller_passes();
    let poller_progressed = runs_until(100, || input::poller_passes() > passes);

    blocked && poller_progressed && input::has_reader() && READER_SAW.load(Ordering::Acquire) == 0
}

fn the_blocked_reader_woke_with_the_key(passes_before: usize) -> bool {
    let wanted = ps2_keys::Key::A as usize + 1;

    let woke = runs_until(1_000, || READER_SAW.load(Ordering::Acquire) != 0);
    woke && READER_SAW.load(Ordering::Acquire) == wanted
        && READER_BLOCKED.load(Ordering::Acquire) == 1
        && input::dropped_events() == 0
        && input::poller_passes() > passes_before
}

static CLOCKLESS_READER_SAW: AtomicUsize = AtomicUsize::new(0);
static CLOCKLESS_KEY_QUEUED: AtomicUsize = AtomicUsize::new(0);

fn read_clockless_key() {
    let key = input::read_key(&mut context()).event.key;
    CLOCKLESS_READER_SAW.store(key as usize + 1, Ordering::Release);
}

fn advance_beside_clockless_poller() {
    while !input::poller_started_for_test() {
        thread::yield_now(&mut context());
    }
    if input::queue_key_for_test(ps2_keys::Key::A) {
        CLOCKLESS_KEY_QUEUED.store(1, Ordering::Release);
    }
    input::mark_poller_peer_progress_for_test();
}

fn the_polled_fallback_runs_its_rounds_and_sleeps() -> bool {
    const ROUNDS: usize = 4;

    let free = thread::free_slots();
    let before = input::poller_passes();
    let expiries_before = time::expiries();
    if !input::start_bounded_poller(ROUNDS) {
        return false;
    }
    // Its own sleeps are what let this thread run, so waiting is a sleep too.
    thread::sleep_ms(&mut context(), (ROUNDS as u64 + 2) * 10);

    let passes = input::poller_passes() - before;
    let exited = yields_until(|| thread::free_slots() == free);

    // And it slept between rounds, which the count cannot show: each sleep
    // arms a timer, one of which is this thread's own (D-124).
    let slept = time::expiries() - expiries_before >= ROUNDS as u64;

    if passes != ROUNDS || !exited || !slept {
        return false;
    }

    CLOCKLESS_READER_SAW.store(0, Ordering::Release);
    CLOCKLESS_KEY_QUEUED.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("clockless-reader", read_clockless_key).is_err()
        || !yields_until(input::has_reader)
    {
        return false;
    }

    let clockless = time::without_calibration_for_test(|| {
        if thread::spawn("clockless-poller-peer", advance_beside_clockless_poller).is_err()
            || !input::start_bounded_poller(ROUNDS)
        {
            return false;
        }
        let finished = yields_until(|| input::poller_passes() >= before + 2 * ROUNDS);
        let reader_ran = yields_until(|| CLOCKLESS_READER_SAW.load(Ordering::Acquire) != 0);
        let threads_exited = yields_until(|| thread::free_slots() == free);
        finished
            && reader_ran
            && threads_exited
            && CLOCKLESS_KEY_QUEUED.load(Ordering::Acquire) == 1
            && CLOCKLESS_READER_SAW.load(Ordering::Acquire) == ps2_keys::Key::A as usize + 1
            && input::poller_saw_peer_progress_for_test()
    });

    if !clockless || !time::is_calibrated() {
        return false;
    }

    let before = input::poller_passes();
    input::start_poller();
    runs_until(100, || {
        input::poller_started_for_test() && input::poller_passes() > before
    })
}

/// Release every thread used to fill the table before the idle retry.
static RELEASE_IDLE_FILLERS: AtomicUsize = AtomicUsize::new(0);

fn wait_for_idle_retry() {
    while RELEASE_IDLE_FILLERS.load(Ordering::Acquire) == 0 {
        spin_loop();
    }
}

fn a_failed_idle_start_can_be_retried_exactly_once() -> bool {
    let free = thread::free_slots();
    if free == 0 || thread::idle_threads() != 0 {
        return false;
    }

    RELEASE_IDLE_FILLERS.store(0, Ordering::Release);
    for _ in 0..free {
        if thread::spawn("idle-slot-filler", wait_for_idle_retry).is_err() {
            RELEASE_IDLE_FILLERS.store(1, Ordering::Release);
            assert!(
                yields_until(|| thread::free_slots() == free),
                "idle-slot filler cleanup did not return every slot"
            );
            return false;
        }
    }

    thread::start_idle();
    let refused =
        thread::free_slots() == 0 && !thread::idle_is_started() && thread::idle_threads() == 0;

    RELEASE_IDLE_FILLERS.store(1, Ordering::Release);
    let released = yields_until(|| thread::free_slots() == free);
    RELEASE_IDLE_FILLERS.store(0, Ordering::Release);
    if !released {
        return false;
    }

    thread::start_idle();
    thread::start_idle();
    let exact = refused
        && thread::idle_is_started()
        && thread::idle_threads() == 1
        && thread::free_slots() + 1 == free;
    if !thread::idle_is_started() {
        assert!(
            thread::restore_idle_retry_for_test(),
            "the refused idle reservation could not be restored"
        );
        thread::start_idle();
    }
    exact
}

fn a_busy_machine_reaps_a_program_without_another_spawn() -> bool {
    while user::collect().is_some() {}

    let gate_resources = spawn_resources();
    let crossings = trap::last_syscall().count;
    let attempt = queue::without_worker_for_test(user::run);
    let denied = matches!(&attempt, Err(user::SpawnError::NoServiceWorker));
    let cleaned = match attempt {
        Ok(spawned) => cleanup_unexpected_spawn(spawned),
        Err(_) => true,
    };
    let fail_closed = denied
        && cleaned
        && trap::last_syscall().count == crossings
        && spawn_resources().has_same_ownership(&gate_resources);
    if !fail_closed {
        return false;
    }

    let resources = spawn_resources();
    let reclaimed = user::reclaimed();
    let reaps = queue::worker_reaps_for_test();
    let idle_halts = thread::idle_halts();

    STOP_SPINNING.store(0, Ordering::Release);
    LIGHT_SPINS.store(0, Ordering::Release);
    let busy = thread::spawn("reap-pressure", light_spinner).is_ok();
    let Ok(spawned) = user::run_for_bootstrap() else {
        return false;
    };

    // This exact incarnation retired: its rites are recorded and, with no other
    // program spawned to reuse the slot, stay recorded past the reap. The busy
    // worker then reaped it without a spawn: its slot holds no zombie and the
    // reap and reclaim counters advanced (D-355).
    let reaped_this_child = last_rites_for(&spawned, 2_000).is_some()
        && runs_until(2000, || {
            let now = user::reclaimed();
            !user::zombie_owns_slot_for_test(spawned.slot)
                && queue::worker_reaps_for_test() > reaps
                && now.0 > reclaimed.0
                && now.1 > reclaimed.1
                && !user::has_zombies()
        });
    let idle_stayed_out = thread::idle_halts() == idle_halts;

    STOP_SPINNING.store(1, Ordering::Release);
    let settled = yields_until(|| thread::free_slots() == resources.thread_slots);

    busy && reaped_this_child
        && idle_stayed_out
        && LIGHT_SPINS.load(Ordering::Relaxed) > 0
        && settled
        && spawn_resources().has_same_ownership(&resources)
}

fn the_idle_thread_runs_only_when_nothing_else_can() -> bool {
    // Slept rather than spun: while this normal thread is runnable the idle
    // thread must not run, which the second half checks.
    let idled = thread::idle_halts();
    thread::sleep_ms(&mut context(), 20);
    let ran_when_free = thread::idle_halts() > idled;

    STOP_SPINNING.store(0, Ordering::Release);
    LIGHT_SPINS.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("busy", light_spinner).is_err() {
        return false;
    }
    let busy = thread::idle_halts();
    runs_until(100, || false);
    let stayed_off = thread::idle_halts() == busy;
    STOP_SPINNING.store(1, Ordering::Release);
    let exited = yields_until(|| thread::free_slots() == free);

    ran_when_free && stayed_off && exited && LIGHT_SPINS.load(Ordering::Relaxed) > 0
}

fn a_scroll_does_not_block_interrupts() -> bool {
    const EXPECTED_EXPIRIES: u64 = 1;

    let Some((_, (_, line_height))) = dev::console_cursor() else {
        return false;
    };
    let Some(geometry) = dev::console_geometry() else {
        return false;
    };
    let rows = geometry.height / line_height.max(1);

    STOP_SPINNING.store(0, Ordering::Release);
    LIGHT_SPINS.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("scroll-witness", light_spinner).is_err() {
        return false;
    }

    let measured = (|| {
        let mut ready = false;
        for _ in 0..=rows {
            dev::with_console(|console| {
                ready = console.next_line_scrolls();
                if !ready {
                    console.write_line("");
                }
            });
            if ready {
                break;
            }
        }
        if !ready {
            return None;
        }

        let mut result = None;
        dev::with_console(|console| {
            let cursor = console.cursor().1;
            let excluded = interrupt::disable();
            let taken = time::expiries();
            time::arm_at(time::scheduler_now_ns());
            excluded.restore();
            let before = time::now_ns();
            console.write_line("");
            result = Some((
                time::now_ns() - before,
                time::expiries() - taken,
                console.cursor().1 == cursor,
            ));
        });
        result
    })();
    thread::rearm_current_for_test();

    STOP_SPINNING.store(1, Ordering::Release);
    let exited = yields_until(|| thread::free_slots() == free);
    let Some((elapsed, expiries, scrolled)) = measured else {
        LONGEST_SCROLL.store(0, Ordering::Relaxed);
        return false;
    };
    LONGEST_SCROLL.store(elapsed as usize, Ordering::Relaxed);

    exited && scrolled && elapsed > 0 && expiries >= EXPECTED_EXPIRIES
}

static LONGEST_SCROLL: AtomicUsize = AtomicUsize::new(0);

/// Tally of what has been checked so far.
struct Report {
    passed: usize,
    failed: usize,
}

/// Bumped whenever the machine-to-harness verdict grammar changes.
const VERDICT_VERSION: usize = 1;

/// Exact check count; mismatch catches deletion or bypass before the verdict.
/// Changing it requires a retained mutation (D-119).
const EXPECTED_CHECKS: usize = 277;

#[cfg(feature = "selftest")]
fn awaiting(marker: &str, press: &str) {
    info!("{marker}");
    dev::with_console(|console| {
        console.write_line("");
        console.write_line(press);
        console.write_line("");
    });
}

impl Report {
    const fn new() -> Self {
        Self {
            passed: 0,
            failed: 0,
        }
    }

    fn check(&mut self, name: &str, holds: bool) {
        if holds {
            self.passed += 1;
            info!("selftest: {name}");
        } else {
            self.failed += 1;
            error!("selftest: {name} FAILED");
            if time::Revocation::current() != time::Revocation::None {
                let (tsc, pit, expected) = time::last_watchdog_sample();
                error!(
                    "selftest: clock revoked {:?}, watchdog tsc={tsc} pit={pit} expected={expected}",
                    time::Revocation::current()
                );
            }
        }
    }
}

/// Run every check and hand the verdict to the harness.
pub(crate) fn run(serial: Result<(), UartError>) {
    SELFTEST_STARTED_NS.store(time::now_ns(), Ordering::Relaxed);
    let mut report = Report::new();
    let geometry = dev::console_geometry();

    report.check("serial console came up", serial.is_ok());
    report.check("framebuffer console came up", geometry.is_some());

    report.check(
        "framebuffer geometry is sane",
        geometry.is_some_and(geometry_is_sane),
    );
    report.check(
        "colour channels do not overlap",
        geometry.is_some_and(channels_are_disjoint),
    );

    report.check(
        "a claimed port cannot be claimed twice",
        ports_are_exclusive(),
    );
    report.check(
        "a save stays inside its area",
        a_save_stays_inside_its_area(),
    );
    report.check(
        "filling the arena never repeats a slot",
        filling_the_arena_never_repeats_a_slot(),
    );
    report.check(
        "ring three has no fast-syscall door",
        the_fast_syscall_doors_are_shut_by_this_kernel(),
    );
    report.check(
        "a reclaimed space answers nothing",
        a_reclaimed_space_answers_nothing(),
    );
    report.check(
        "a refused pair of claims strands neither",
        a_refused_pair_of_claims_strands_neither(),
    );
    report.check(
        "a claim covers the width it drives",
        a_claim_covers_the_width_it_drives(),
    );
    report.check("a held lock excludes another holder", lock_excludes());
    report.check(
        "forgotten lock ownership is distinct from contention",
        forgotten_lock_ownership_is_distinct_from_contention(),
    );
    report.check("a one-shot latch admits one caller", latch_admits_one());

    report.check(
        "the frame allocator admits only exclusively usable frames",
        mm::free_frames() > 0 && mm::overlapping_allocator_claims_are_arbitrated_for_test(),
    );
    report.check(
        "a frame is returned when its handle drops",
        frame_round_trips(),
    );
    report.check(
        "uncertain dma memory is never recycled",
        uncertain_dma_memory_is_never_recycled(),
    );
    report.check(
        "dma byte access is exact and bounded",
        dma_byte_access_is_exact_and_bounded(),
    );
    report.check(
        "two live frames are different frames",
        frames_are_exclusive(),
    );
    report.check(
        "dirty memory can be zeroed, and seen to be",
        dirty_memory_can_be_zeroed(),
    );
    report.check("allocation returns a zeroed frame", allocation_is_zeroed());
    report.check(
        "a contiguous run is contiguous and aligned",
        a_run_is_contiguous_and_aligned(),
    );

    report.check("the loaded page table is ours", the_loaded_table_is_ours());
    report.check(
        "an admitted canonical page can be invalidated",
        an_admitted_canonical_page_can_be_invalidated(),
    );
    report.check(
        "the kernel image obeys write-xor-execute",
        the_image_obeys_write_xor_execute(),
    );
    report.check(
        "supervisor writes obey read-only pages",
        supervisor_writes_obey_read_only_pages(),
    );
    report.check(
        "the direct map is writable and never executable",
        the_direct_map_is_not_executable(),
    );
    report.check(
        "a mapping added after the switch is live",
        a_mapping_added_after_the_switch_is_live(),
    );
    report.check(
        "a second address space is refused",
        a_second_switch_is_refused(),
    );
    report.check(
        "an unserved call is refused without stopping the machine",
        an_unserved_call_is_refused_without_stopping_the_machine(),
    );
    report.check(
        "a thread crosses into ring three and calls back",
        a_thread_crosses_into_ring_three_and_calls_back(),
    );
    report.check(
        "ring-three entry restores the kernel stack selector",
        trap::kernel_stack_selector_on_last_syscall() == u64::from(segment::KERNEL_DATA.bits()),
    );
    report.check(
        "a program cannot bring smap down with it",
        a_program_cannot_bring_smap_down_with_it(),
    );
    report.check(
        "a faulting program dies alone",
        a_faulting_program_dies_alone(),
    );
    report.check(
        "a noncanonical user return stack retires only its program",
        a_noncanonical_user_return_stack_retires_only_its_program(),
    );
    report.check(
        "unsafe user return flags retire only their program",
        unsafe_user_return_flags_retire_only_their_program(),
    );
    report.check(
        "a program cannot aim at the copy window",
        a_program_cannot_aim_at_the_copy_window(),
    );
    report.check(
        "two spaces map one address to different memory",
        two_spaces_map_one_address_to_different_memory(),
    );
    report.check(
        "every enabled state component survives a switch",
        every_enabled_state_component_survives_a_switch(),
    );
    report.check(
        "a ring announces its shape and keeps its pages",
        a_ring_announces_its_shape_and_keeps_its_pages(),
    );
    report.check(
        "a word that does not fit is refused rather than truncated",
        a_word_that_does_not_fit_is_refused_rather_than_truncated(),
    );
    report.check(
        "a completion carries a file handle",
        a_completion_carries_a_file_handle(),
    );
    report.check(
        "opening a node does not restore universal rights",
        opening_a_node_does_not_restore_universal_rights(),
    );
    report.check(
        "a read is a submission with its own offset",
        a_read_is_a_submission_with_its_own_offset(),
    );
    report.check(
        "file work pressure is bounded at admission",
        file_work_pressure_is_bounded_at_admission(),
    );
    report.check(
        "a delayed read preserves interrupt admission and exact teardown",
        a_delayed_read_preserves_interrupt_admission_and_exact_teardown(),
    );
    report.check(
        "a ring operation cannot reach past its rights or its pages",
        a_ring_operation_cannot_reach_past_its_rights_or_its_pages(),
    );
    report.check(
        "a second backend answers the same three operations",
        a_second_backend_answers_the_same_three_operations(),
    );
    report.check(
        "enumeration callbacks run without backend ownership",
        enumeration_callbacks_run_without_backend_ownership(),
    );
    report.check("a device is not a file", a_device_is_not_a_file());
    report.check(
        "no backend lets a path escape or cross",
        no_backend_lets_a_path_escape_or_cross(),
    );
    report.check(
        "a path resolves from a directory handle",
        a_path_resolves_from_a_directory_handle(),
    );
    report.check(
        "no path can name anything above the handle",
        no_path_can_name_anything_above_the_handle(),
    );
    report.check(
        "a file cannot be walked through",
        a_file_cannot_be_walked_through(),
    );
    report.check(
        "enumeration lists what traversal would find",
        enumeration_lists_what_traversal_would_find(),
    );
    report.check(
        "the volume mounts and says what a fat32 volume says",
        the_volume_mounts_and_says_what_a_fat32_volume_says(),
    );
    report.check(
        "the kernel reads the file the host wrote",
        the_kernel_reads_the_file_the_host_wrote(),
    );
    report.check(
        "a file longer than a cluster is read across its chain",
        a_file_longer_than_a_cluster_is_read_across_its_chain(),
    );
    report.check(
        "a path resolves through a directory",
        a_path_resolves_through_a_directory(),
    );
    report.check(
        "the controller was read rather than assumed",
        the_controller_was_read_rather_than_assumed(),
    );
    report.check(
        "nvme candidates preserve two pages of boot-stack headroom",
        nvme_candidates_preserve_boot_stack_headroom(),
    );
    report.check(
        "a later nvme controller survives an earlier failure",
        crate::a_later_nvme_controller_survives_an_earlier_failure(),
    );
    report.check(
        "a live nvme bar obeys maxphyaddr",
        crate::a_live_nvme_bar_obeyed_maxphyaddr(),
    );
    report.check(
        "a failed bus master admission stops before nvme start",
        a_failed_bus_master_admission_stops_before_nvme_start(),
    );
    report.check(
        "invalid nvme namespace stops before io queues",
        invalid_nvme_namespace_stops_before_io_queues(),
    );
    report.check(
        "invalid nvme reads stop before submission",
        invalid_nvme_reads_stop_before_submission(),
    );
    report.check(
        "a block read returns the block that was asked for",
        a_block_read_returns_the_block_that_was_asked_for(),
    );
    report.check(
        "the queues wrap and the phase tag still works",
        the_queues_wrap_and_the_phase_tag_still_works(),
    );
    report.check(
        "the controller's registers are uncacheable",
        the_controllers_registers_are_uncacheable(),
    );
    report.check(
        "the bus walk descends through a bridge",
        the_bus_walk_descends_through_a_bridge(),
    );
    report.check(
        "a device is claimed once and a refusal strands nothing",
        a_device_is_claimed_once_and_a_refusal_strands_nothing(),
    );
    report.check(
        "a claimed device's io window is not claimable bare",
        a_claimed_devices_io_window_is_not_claimable_bare(),
    );
    report.check(
        "work in flight is ended before the ring is released",
        work_in_flight_is_ended_before_the_ring_is_released(),
    );
    report.check(
        "a refused settlement retains every owner",
        a_refused_settlement_retains_every_owner(),
    );
    report.check(
        "a promise outlives the program that made it",
        a_promise_outlives_the_program_that_made_it(),
    );
    report.check(
        "the time page states the clock the kernel measured",
        the_time_page_states_the_clock_the_kernel_measured(),
    );
    report.check(
        "the time page and the kernel clock tell the same time",
        the_time_page_and_the_kernel_clock_tell_the_same_time(),
    );
    report.check(
        "revoking the clock sends readers to ring Uptime",
        revoking_the_clock_sends_readers_to_uptime(),
    );
    report.check(
        "the clock's update window is never preemptible",
        the_clocks_update_window_is_never_preemptible(),
    );
    report.check(
        "a program sees the time page and cannot write it",
        a_program_sees_the_time_page_and_cannot_write_it(),
    );
    report.check(
        "a completion carries a handle with narrowed rights",
        a_completion_carries_a_handle_with_narrowed_rights(),
    );
    report.check(
        "a completion cannot widen what was offered",
        a_completion_cannot_widen_what_was_offered(),
    );
    report.check(
        "a refused handle operation returns its promise",
        a_refused_handle_operation_returns_its_promise(),
    );
    report.check(
        "a full table refuses a handle operation at submission",
        a_full_table_refuses_a_handle_operation_at_submission(),
    );
    report.check(
        "an operation completes after its deadline and only once",
        an_operation_completes_after_its_deadline_and_only_once(),
    );
    report.check(
        "a cancelled operation and its cancellation complete separately",
        a_cancelled_operation_and_its_cancellation_complete_separately(),
    );
    report.check(
        "every submission gets exactly one completion",
        every_submission_gets_exactly_one_completion(),
    );
    report.check(
        "admission refuses before the completion queue can overflow",
        admission_refuses_before_the_completion_queue_can_overflow(),
    );
    report.check(
        "a full ring admits again only after userspace reaps",
        a_full_ring_admits_again_only_after_userspace_reaps(),
    );
    report.check(
        "a rewritten submission cannot change what was served",
        a_rewritten_submission_cannot_change_what_was_served(),
    );
    report.check(
        "kernel memory cannot be published to ring three",
        kernel_memory_cannot_be_published_to_ring_three(),
    );
    report.check(
        "external user bytes are zeroed before free",
        external_user_bytes_are_zeroed_before_free(),
    );
    report.check(
        "a refused external clear withholds its run",
        a_refused_external_clear_withholds_its_run(),
    );
    report.check(
        "only a space that matches the kernel can be installed",
        only_a_space_that_matches_the_kernel_can_be_installed(),
    );
    report.check(
        "the loader refuses what it may not map",
        the_loader_refuses_what_it_may_not_map(),
    );
    report.check(
        "a second thread crosses onto its own stack",
        a_second_thread_crosses_onto_its_own_stack(),
    );
    report.check(
        "thread local storage follows the thread",
        thread_local_storage_follows_the_thread(),
    );
    report.check(
        "device window reuses arbitrary-order gaps",
        device_window_reuses_arbitrary_order_gaps(),
    );
    let device_liveness = device_liveness_covers_the_current_root_and_extent();
    report.check(
        "late device mappings reach earlier user roots",
        device_liveness
            .as_ref()
            .is_some_and(|probe| probe.shared_topology),
    );
    report.check(
        "a stale shared root cannot authenticate a device mapping",
        device_liveness
            .as_ref()
            .is_some_and(|probe| probe.stale_root),
    );
    report.check(
        "a device mapping checks its first leaf",
        device_liveness.as_ref().is_some_and(|probe| probe.first),
    );
    report.check(
        "a device mapping checks its middle leaf",
        device_liveness.as_ref().is_some_and(|probe| probe.middle),
    );
    report.check(
        "a device mapping checks its last leaf",
        device_liveness.as_ref().is_some_and(|probe| probe.last),
    );
    report.check(
        "a device mapping checks its cache policy",
        device_liveness
            .as_ref()
            .is_some_and(|probe| probe.cache_policy),
    );
    report.check(
        "a bound space is not reclaimed",
        a_bound_space_is_not_reclaimed(),
    );
    report.check("one space has one binding", one_space_has_one_binding());
    report.check(
        "binding ignores only accessed root drift",
        binding_ignores_only_accessed_root_drift(),
    );
    report.check(
        "every refused spawn returns every resource",
        every_refused_spawn_returns_every_resource(),
    );
    report.check(
        "a commit refusal reunites cleanup authority",
        a_commit_refusal_reunites_cleanup_authority(),
    );
    report.check(
        "a maximum image and stack fit the cleanup ledger",
        a_maximum_image_and_stack_fit_the_cleanup_ledger(),
    );
    report.check(
        "cleanup refusals retain reachable tables",
        cleanup_refusals_retain_reachable_tables(),
    );
    report.check(
        "partial stack rollback is retryable",
        partial_stack_rollback_is_retryable(),
    );
    report.check(
        "guard rollback refusal retains the stack",
        guard_rollback_refusal_retains_the_stack(),
    );
    report.check(
        "a failed boot-space build returns every table frame",
        mem::partial_build_failure_is_atomic_for_test(),
    );
    report.check(
        "a retirement refusal is settled before slot reuse",
        a_retirement_refusal_is_settled_before_slot_reuse(),
    );
    report.check(
        "a task release refusal retains the zombie for retry",
        a_task_release_refusal_retains_the_zombie_for_retry(),
    );
    report.check(
        "one zombie has one settlement collector",
        one_zombie_has_one_settlement_collector(),
    );
    report.check(
        "an in-flight collector keeps the worker retry live",
        an_in_flight_collector_keeps_the_worker_retry_live(),
    );
    report.check(
        "an unguarded boot stack closes the user boundary",
        an_unguarded_boot_stack_closes_the_user_boundary(),
    );
    report.check(
        "unadmitted interrupts cannot be enabled by a user transition",
        unadmitted_interrupts_close_the_user_boundary(),
    );
    report.check(
        "shared kernel mappings without an isolation proof are refused",
        shared_kernel_mappings_without_proof_close_the_user_boundary(),
    );
    report.check(
        "no-execute protection closes the user boundary",
        no_execute_protection_closes_the_user_boundary(),
    );
    report.check(
        "missing extended state closes the user boundary",
        missing_extended_state_closes_the_user_boundary(),
    );
    report.check(
        "an uncalibrated clock admits no user or timer",
        an_uncalibrated_clock_admits_no_user_or_timer(),
    );
    report.check(
        "a clock disagreement revokes time without stranding timers",
        a_clock_disagreement_revokes_time_without_stranding_timers(),
    );
    report.check(
        "a user page comes back cleared",
        a_user_page_comes_back_cleared(),
    );
    report.check(
        "a dead program gives its memory back",
        a_dead_program_gives_its_memory_back(),
    );
    report.check(
        "a program drives its own ring",
        a_program_drives_its_own_ring(),
    );
    report.check(
        "a ring enter round trip is measurable",
        a_ring_enter_round_trip_is_measurable(),
    );
    report.check(
        "a program duplicates its bootstrap handle",
        a_program_duplicates_its_bootstrap_handle(),
    );
    report.check(
        "a program waits through its trap frame",
        a_program_waits_through_its_trap_frame(),
    );
    report.check(
        "a syscall wake arms the timer for what it woke",
        a_syscall_wake_arms_the_timer_for_what_it_woke(),
    );
    report.check(
        "a spawned program holds exactly one handle",
        a_spawned_program_holds_exactly_one_handle(),
    );
    report.check(
        "the return to ring three clears buffers",
        the_return_to_ring_three_clears_buffers(),
    );
    report.check(
        "a faulting user copy resumes and reports",
        a_faulting_user_copy_resumes_and_reports(),
    );
    report.check(
        "bytes cross the boundary only where they may",
        bytes_cross_the_boundary_only_where_they_may(),
    );
    report.check(
        "an active unmap exposes only its replacement",
        an_active_unmap_exposes_only_its_replacement(),
    );
    report.check(
        "a shared kernel unmap evicts the live user root",
        a_shared_kernel_unmap_evicts_the_live_user_root(),
    );
    report.check(
        "an inactive lower-root unmap preserves the live observer",
        an_inactive_lower_root_unmap_preserves_the_live_observer(),
    );
    report.check(
        "a waiter rereads its condition before blocking",
        a_waiter_rereads_its_condition_before_blocking(),
    );
    report.check(
        "a meaningless right is refused",
        a_meaningless_right_is_refused(),
    );
    report.check(
        "a handle reveals no foreign occupancy",
        a_handle_reveals_no_foreign_occupancy(),
    );
    report.check(
        "table identities never wrap or repeat",
        table_identities_never_wrap_or_repeat(),
    );
    report.check(
        "a table cannot exceed its quota",
        a_table_cannot_exceed_its_quota(),
    );
    report.check(
        "the handle arena backs a table's whole quota",
        the_handle_arena_backs_a_tables_whole_quota(),
    );
    report.check(
        "an object lives exactly as long as its handles",
        an_object_lives_exactly_as_long_as_its_handles(),
    );
    report.check(
        "a dead table releases its objects",
        a_dead_table_releases_its_objects(),
    );
    report.check(
        "the boot thread stands on a guarded stack",
        the_boot_thread_stands_on_a_guarded_stack(),
    );
    report.check(
        "the double-fault stack is guarded",
        the_double_fault_stack_is_guarded(),
    );
    report.check(
        "the double-fault stack cannot be unmapped",
        the_double_fault_stack_cannot_be_unmapped(),
    );
    report.check(
        "the boot stack rejects a lower-leaf substitution",
        the_boot_stack_rejects_a_lower_leaf_substitution(),
    );
    report.check(
        "a noncurrent task stack rejects a lower-leaf substitution",
        a_noncurrent_task_stack_rejects_a_lower_leaf_substitution(),
    );
    report.check(
        "the double-fault stack rejects a lower-leaf substitution",
        the_double_fault_stack_rejects_a_lower_leaf_substitution(),
    );
    report.check(
        "protected stacks refuse overlapping maps",
        protected_stacks_refuse_overlapping_maps(),
    );
    report.check(
        "a second space holds the kernel and nothing else",
        a_second_space_holds_the_kernel_and_nothing_else(),
    );
    report.check(
        "a walk reports what the live tables grant",
        a_walk_reports_the_permission_the_cpu_grants(),
    );
    report.check(
        "a refused mapping leaves no trace",
        a_refused_mapping_leaves_no_trace(),
    );
    report.check(
        "a partial mapping failure returns its tables",
        a_partial_mapping_failure_returns_its_tables(),
    );
    report.check(
        "a refused remap preserves every older entry",
        a_refused_remap_preserves_every_older_entry(),
    );
    report.check(
        "a shared mapping keeps its frame",
        a_shared_mapping_keeps_its_frame(),
    );
    report.check(
        "shared mappings reject cache aliases",
        shared_mappings_reject_cache_aliases(),
    );
    report.check(
        "ram mappings keep the direct map's cache type",
        ram_mappings_keep_the_direct_map_cache_type(),
    );
    report.check(
        "a large space returns every table frame",
        a_large_space_returns_every_table_frame(),
    );
    report.check(
        "an unmapped range returns its frames",
        an_unmapped_range_returns_its_frames(),
    );
    report.check(
        "unmapping refuses what it does not own",
        unmapping_refuses_what_it_does_not_own(),
    );
    report.check(
        "the map offers a run to probe",
        the_map_offers_a_run_to_probe(),
    );
    report.check(
        "a boot mapping cannot claim ownership",
        a_boot_mapping_cannot_claim_ownership(),
    );
    report.check(
        "caller global is removed from safe mappings",
        caller_global_is_removed_from_safe_mappings(),
    );
    report.check(
        "a boot mapping cannot publish kernel memory",
        a_boot_mapping_cannot_publish_kernel_memory(),
    );
    report.check(
        "a user space cannot reach the kernel half",
        a_user_space_cannot_reach_the_kernel_half(),
    );
    report.check(
        "the last canonical page is never user memory",
        the_last_canonical_page_is_never_user_memory(),
    );
    report.check(
        "a user mapping cannot widen a supervisor table",
        a_user_mapping_cannot_widen_a_supervisor_table(),
    );
    report.check(
        "an undescribed range cannot be mapped",
        an_undescribed_range_cannot_be_mapped(),
    );
    report.check(
        "a differing remap is refused",
        a_differing_remap_is_refused(),
    );
    report.check(
        "device mappings are physically exclusive",
        device_mappings_are_physically_exclusive(),
    );
    report.check(
        "device mappings refuse allocatable ram",
        device_mappings_refuse_allocatable_ram(),
    );
    report.check(
        "malformed device extents are refused",
        malformed_device_extents_are_refused(),
    );
    report.check(
        "active device tables cannot be reclaimed",
        active_device_tables_cannot_be_reclaimed(),
    );
    report.check(
        "active kernel topology refuses a new shared root",
        active_kernel_topology_is_frozen(),
    );
    report.check(
        "shared topology rejects hostile root entries",
        shared_topology_rejects_hostile_root_entries(),
    );
    report.check(
        "firmware reads are confined to acpi storage",
        firmware_reads_are_bounded(),
    );

    report.check(
        "the heap reuses a freed slot",
        the_heap_reuses_a_freed_slot(),
    );
    report.check(
        "a refill costs one frame and balances",
        a_refill_costs_one_frame_and_balances(),
    );
    report.check(
        "heap memory holds what it is given",
        heap_memory_holds_what_it_is_given(),
    );
    report.check(
        "a large allocation takes and returns frames",
        a_large_allocation_takes_and_returns_frames(),
    );
    report.check(
        "the console has a complete shadow to scroll from",
        dev::console_has_complete_back_buffer(),
    );

    report.check("threads are available", thread::is_available());
    report.check(
        "an unmapped stack is no longer installed",
        a_stack_stops_being_installed_when_it_is_unmapped(),
    );
    let stack_liveness = stack_liveness_covers_the_current_root_and_extent();
    report.check(
        "a stale shared root cannot authenticate a stack",
        stack_liveness
            .as_ref()
            .is_some_and(|probe| probe.stale_root),
    );
    report.check(
        "a stack checks its first leaf",
        stack_liveness.as_ref().is_some_and(|probe| probe.first),
    );
    report.check(
        "a stack checks its middle leaf",
        stack_liveness.as_ref().is_some_and(|probe| probe.middle),
    );
    report.check(
        "a stack checks its last leaf",
        stack_liveness.as_ref().is_some_and(|probe| probe.last),
    );
    report.check(
        "thread stacks are guarded below",
        thread_stacks_are_guarded(),
    );
    report.check(
        "two threads alternate strictly through yields",
        threads_alternate_strictly(),
    );
    report.check(
        "callee-saved registers survive a switch",
        registers_survive_a_switch(),
    );
    report.check("a thread starts aligned", a_thread_starts_aligned());
    report.check("an exited slot is reused", an_exited_slot_is_reused());
    report.check(
        "a new thread runs with interrupts accepted",
        a_new_thread_is_interruptible(),
    );
    report.check(
        "a spinning thread cannot starve another",
        a_spinner_cannot_starve_another(),
    );
    report.check(
        "runtime divides in proportion to weight",
        runtime_divides_by_weight(),
    );
    report.check(
        "the policy never needed its fallback",
        thread::policy_fallbacks() == 0,
    );

    report.check(
        "the code selector is the one we installed",
        segment::code_selector() == segment::KERNEL_CODE,
    );
    report.check(
        "the task register holds our segment",
        segment::task_register() == segment::TASK_STATE,
    );
    report.check(
        "the interrupt table the CPU holds is ours",
        interrupt::table_is_loaded(),
    );

    let before = trap::last_breakpoint();
    interrupt::breakpoint();
    let taken = trap::last_breakpoint();

    report.check(
        "a breakpoint is delivered and resumes",
        taken.count == before.count + 1,
    );
    report.check(
        "the saved frame is the frame the CPU pushed",
        frame_is_consistent(taken),
    );
    report.check(
        "interrupt entry pushes the frame the layout describes",
        frame_spans_the_saved_words(taken),
    );

    report.check(
        "every protection the CPU supports is enabled",
        Protections::enabled() == Protections::supported(),
    );

    report.check(
        "the legacy controllers are masked",
        pic::masks() == Some((MASKED, MASKED)),
    );
    report.check("the local apic is software enabled", apic::is_enabled());
    report.check(
        "local apic initialization stayed unpublished and interrupt-excluded",
        apic::initialization_was_atomic_for_test(),
    );
    report.check("this cpu is accepting interrupts", interrupt::are_enabled());
    report.check(
        "a lock excludes interrupts and gives them back",
        a_lock_excludes_interrupts(),
    );

    report.check(
        "the timer entry is one-shot and unmasked",
        timer_entry_is_programmed(),
    );
    report.check(
        "both calibrated rates are believable",
        rates_are_believable(),
    );
    report.check(
        "every refusal the clock can record reads back as itself",
        time::every_refusal_reads_back_as_itself(),
    );

    let sweep = sweep_clock();
    report.check("the clock never runs backwards", sweep.monotonic);
    // The smallest observed step measures resolution; interpolation alone
    // cannot prove two events inside one tick are distinguishable.
    report.check(
        "the clock advances in steps finer than a microsecond",
        sweep.finest_step.is_some_and(|step| step < 1_000),
    );
    report.check(
        "an idle kernel takes no timer interrupts",
        an_idle_kernel_takes_no_timer_interrupts(),
    );
    report.check(
        "the timer fires about once a slice under load",
        the_timer_fires_about_once_a_slice(),
    );
    report.check(
        "a sleeping thread wakes on time, and quietly",
        a_sleeper_wakes_on_time_and_quietly(),
    );
    report.check(
        "a sleeping thread hands the cpu over",
        sleeping_hands_the_cpu_over(),
    );

    // Everything above ran without an idle thread, the only witness for a
    // sleeper halting on its own stack; from here something is always runnable.
    report.check(
        "a failed idle start can be retried exactly once",
        a_failed_idle_start_can_be_retried_exactly_once(),
    );
    thread::start_idle();
    report.check("the idle thread is running", thread::idle_is_started());
    report.check(
        "the idle thread runs only when nothing else can",
        the_idle_thread_runs_only_when_nothing_else_can(),
    );
    report.check(
        "a busy machine reaps a program without another spawn",
        a_busy_machine_reaps_a_program_without_another_spawn(),
    );
    report.check(
        "the polled keyboard fallback is bounded before it stays live",
        the_polled_fallback_runs_its_rounds_and_sleeps(),
    );

    report.check("the keyboard controller came up", input::is_ready());
    report.check(
        "a refused controller configuration is typed and changes nothing",
        input::configuration_refusal_is_visible(),
    );
    report.check(
        "a refused keyboard route restores configuration, masking and mappings",
        crate::keyboard_routing_rollback_is_exact()
            && ioapic::ignored_writes_are_refused_for_test()
            && crate::unproven_external_controllers_veto_interrupts(),
    );
    report.check(
        "an isolated I/O APIC route encodes its typed vector",
        ioapic::typed_route_is_encoded_for_test(),
    );
    report.check(
        "a window that is not an I/O APIC is never taken over",
        ioapic::an_implausible_version_is_refused_for_test(),
    );
    report.check(
        "a failed controller flush stops every following transaction",
        input::flush_refusals_are_visible(),
    );
    report.check(
        "a refused controller command is typed and consumes no reply",
        input::transport_refusals_are_visible(),
    );
    report.check(
        "the controller route and ordered interrupt witness are exact",
        keyboard_config_matches_its_routing()
            && input::watch_order_is_causal()
            && input::queue_admission_is_exact(),
    );
    report.check(
        "an echo to the keyboard comes back",
        input::echo_roundtrips(),
    );
    // The identity first, because it decides what the set query is worth;
    // recorded as answered, since `None` and a wrong number differ (D-176).
    let identity = input::identity();
    let translated = identity == Some(TRANSLATED_MF2);
    if !translated && identity != Some(UNTRANSLATED_MF2) {
        warn!("keyboard: identifies as {identity:?}, which names no known keyboard");
    }
    report.check(
        "the keyboard identifies as one the decoder knows",
        translated || identity == Some(UNTRANSLATED_MF2),
    );

    // Translation rewrites set-query replies, so this stays diagnostic and
    // the decoder is proved from received bytes (D-178, D-179, D-239).
    let set = input::scancode_set();
    if !translated && set != Some(2) {
        warn!("keyboard: reports scancode set {set:?}, not 2");
    }

    // Set up before the marker: the reader must be blocked before the harness
    // presses, or an already-waiting keystroke proves nothing.
    report.check(
        "a blocked keyboard reader waits while the poller runs",
        a_blocked_reader_waits_while_the_poller_runs(),
    );

    // Kept in step with READY_MARKER in `xtask/src/qemu.rs`; nothing may talk
    // to the keyboard between the marker and the wait.
    let first_key = input::arm_key(ps2_keys::Key::A);
    let last_key = input::arm_key(ps2_keys::Key::Enter);
    let polling_before = input::poller_passes();
    awaiting(
        "selftest: awaiting injected keystroke",
        "  PRESS  a  then BACKSPACE  then ENTER",
    );
    report.check(
        "an injected keystroke arrives, and its release follows",
        first_key.is_some_and(input::await_armed_key),
    );

    report.check(
        "the framebuffer is mapped write-combining",
        the_framebuffer_is_write_combining(),
    );
    report.check(
        "default-deny interrupt remapping guards every pci requester",
        default_deny_interrupt_remapping_guards_pci(),
    );
    report.check(
        "the io apic owner returned after masked rollback",
        the_io_apic_owner_was_returned_after_masked_rollback(),
    );
    report.check(
        "keyboard input is polled without legacy interrupts",
        keyboard_input_is_polled_without_legacy_interrupts(),
    );
    report.check(
        "the blocked reader woke with the injected key through polling",
        the_blocked_reader_woke_with_the_key(polling_before),
    );
    // Before the editing check: Enter is the last injected key and only a
    // press echoes, so its release is when the console stops moving.
    report.check(
        "the injected editing keys arrive, ending with enter",
        last_key.is_some_and(input::await_armed_key),
    );
    report.check(
        "backspace erases and enter starts a line",
        the_editing_keys_move_the_cursor(),
    );

    // The set 1 path: QEMU honours the translation bit, producing the stream
    // VMware delivers unasked, with a key the harness chose (D-179).
    report.check(
        "the controller accepts translation being turned on",
        input::set_translation(crate::arch::current::i8042::Translation::Enabled),
    );
    report.check(
        "translation updates the decoder from the controller identity",
        input::decoder_matches_identity(),
    );
    // Seeded against the stream on purpose: the round can succeed only by
    // reading the bytes, as VMware forces (D-239).
    input::assume_wrong_numbering();
    // Kept in step with TRANSLATED_MARKER in `xtask/src/qemu.rs`, under the
    // same rule: nothing talks to the keyboard between marker and wait.
    let translated_key = input::arm_key(ps2_keys::Key::B);
    awaiting(
        "selftest: awaiting translated keystroke",
        "  PRESS  b   for the last round",
    );
    // `B`, not `A`, so the first round's stale event cannot satisfy this wait
    // (D-155, D-165).
    report.check(
        "a translated keystroke decodes to the same key",
        translated_key.is_some_and(input::await_armed_key),
    );
    // Ask only after a byte has arrived so this observes the active decoder
    // instead of re-deriving it from the identity that configured it (D-239).
    report.check(
        "the keyboard's numbering and the decoder's are the same",
        input::decoder_reads_what_arrives(),
    );
    // Turned back off, and the question re-opened rather than answered from
    // the written bit; the next decisive byte settles it (D-178, D-239).
    let restored = input::set_translation(crate::arch::current::i8042::Translation::Disabled);
    report.check(
        "the decoder reads the set that arrives, not the bit that was written",
        restored && input::rereads_the_stream(),
    );

    // Last timed check because it advances the console to the bottom.
    report.check(
        "a console scroll does not block interrupts",
        a_scroll_does_not_block_interrupts(),
    );
    info!(
        "selftest: longest console scroll {} ns",
        LONGEST_SCROLL.load(Ordering::Relaxed)
    );

    report.check(
        "no spurious interrupt has been delivered",
        trap::spurious_count() == 0,
    );

    // Last, because reading the error register clears it.
    report.check(
        "the local apic has recorded no error",
        apic::error_status() == 0,
    );

    // Read this detector last so every preceding check can trip it (D-143).
    report.check(
        "no lock is taken while a leaf guard is held",
        a_leaf_guard_is_the_last_lock(),
    );

    // Last because it moves the console contents other checks inspect.
    report.check(
        "a scroll moves one visible cursor",
        a_scroll_moves_one_visible_cursor(),
    );

    report.check(
        "a full-buffer nvme read is measurable",
        a_full_buffer_nvme_read_is_measurable(),
    );

    report.check(
        "translation is active for the storage function",
        translation_is_active_for_the_storage_function(),
    );
    report.check(
        "every shared vtd requester was fenced across controller fallback",
        every_shared_vtd_requester_was_fenced_across_controller_fallback(),
    );
    report.check(
        "the unit walks the kernel's root table",
        the_unit_walks_the_kernels_root_table(),
    );
    report.check(
        "the dma domain reaches exactly the live queue frames",
        the_dma_domain_reaches_exactly_the_live_queue_frames(),
    );
    // Before the translation toggle below, which flushes the unit's caches
    // and would hide a stale translation this check exists to expose.
    report.check(
        "a device write into an unmapped frame faults and changes nothing",
        a_device_write_into_an_unmapped_frame_faults_and_changes_nothing(),
    );
    report.check(
        "uncertain vtd mapping never recycles its frames",
        an_uncertain_vtd_mapping_never_recycles_its_frames(),
    );
    report.check(
        "uncertain vtd unmapping never recycles its frames",
        an_uncertain_vtd_unmap_never_recycles_its_frames(),
    );
    report.check(
        "a domain proof is refused while translation is off",
        a_domain_proof_is_refused_while_translation_is_off(),
    );
    report.check(
        "a reserved dma region over kernel memory refuses isolation",
        a_reserved_dma_region_over_kernel_memory_refuses_isolation(),
    );

    // Last of all: the timeout proof resets the only storage controller.
    report.check(
        "an nvme timeout quiesces before dma memory returns",
        an_nvme_timeout_quiesces_before_dma_memory_returns(),
    );

    report.check(
        "a pending local interrupt refuses the first sti",
        apic::pending_audit_refuses_queued_self_interrupt_for_test(|| trap::last_ping().count),
    );

    // Last interrupt exercise: a broken EOI cannot starve later timed checks.
    let before = trap::last_ping();
    let duplicate_before = trap::rejected_duplicate_eoi_claims();
    let reclaimed_before = trap::reclaimed_abandoned_eoi();
    let first_sent = apic::send_self_interrupt().is_ok();
    report.check(
        "a self-interrupt is delivered",
        first_sent && ping_arrives(before.count + 1),
    );

    let taken = trap::last_ping();
    let second_sent = apic::send_self_interrupt().is_ok();
    report.check(
        "the interrupt is dismissed, so the next one arrives",
        second_sent && ping_arrives(before.count + 2),
    );
    report.check(
        "one delivery mints one completion authority",
        matches!(
            (
                trap::last_ping().count.checked_sub(before.count),
                trap::rejected_duplicate_eoi_claims().checked_sub(duplicate_before),
            ),
            (Some(deliveries), Some(refusals)) if deliveries == refusals
        ),
    );
    report.check(
        "an abandoned completion can be reclaimed for the same delivery",
        matches!(
            (
                trap::last_ping().count.checked_sub(before.count),
                trap::reclaimed_abandoned_eoi().checked_sub(reclaimed_before),
            ),
            (Some(deliveries), Some(reclaims)) if reclaims == usize::from(deliveries >= 2)
        ),
    );
    report.check(
        "the interrupted code was taking interrupts",
        ping_is_consistent(taken),
    );

    let apic_failures =
        apic::initialization_failures_for_test(vector::LOCAL_APIC, || trap::last_ping().count);
    report.check(
        "every late or ignored-write apic refusal stays unavailable and terminal",
        apic_failures.unavailable_and_terminal()
            & apic_failures.ignored_spurious_write_is_terminal(),
    );
    report.check(
        "apic refusal defers a live interrupt until exclusion unwinds",
        apic_failures.delivery_waited_for_unwind(),
    );

    let activation = paging::activation_publication_for_test();
    report.check(
        "paging activation stays unpublished under interrupt exclusion",
        activation.initializing_is_excluded(),
    );
    report.check(
        "paging activation publishes its root before readiness",
        activation.root_precedes_ready(),
    );

    report_baseline();

    let checked = report.passed + report.failed;
    let passed = report.failed == 0 && checked == EXPECTED_CHECKS && EXPECTED_CHECKS > 0;
    if passed {
        info!(
            "selftest: verdict v{} PASS passed={} failed={} expected={}",
            VERDICT_VERSION, report.passed, report.failed, EXPECTED_CHECKS
        );
    } else {
        error!(
            "selftest: verdict v{} FAIL passed={} failed={} expected={}",
            VERDICT_VERSION, report.passed, report.failed, EXPECTED_CHECKS
        );
    }
    debug_exit::report(passed);
}

fn geometry_is_sane(geometry: FramebufferInfo) -> bool {
    let described = geometry.width > 0
        && geometry.height > 0
        && geometry.bytes_per_pixel > 0
        && geometry.pitch >= geometry.width * geometry.bytes_per_pixel;

    let last = (geometry.height - 1) * geometry.pitch + geometry.width * geometry.bytes_per_pixel;
    described && dev::framebuffer_bytes().is_some_and(|mapped| last <= mapped)
}

fn channels_are_disjoint(geometry: FramebufferInfo) -> bool {
    let pack = |red, green, blue| geometry.format.pack(Rgb { red, green, blue });
    let (red, green, blue) = (
        pack(u8::MAX, 0, 0),
        pack(0, u8::MAX, 0),
        pack(0, 0, u8::MAX),
    );

    let disjoint = red != 0
        && green != 0
        && blue != 0
        && red & green == 0
        && green & blue == 0
        && red & blue == 0;

    // And the three compose as the format says, which disjoint masks alone
    // do not show: a `pack` dropping a channel passes them (D-124).
    let white = pack(u8::MAX, u8::MAX, u8::MAX) == red | green | blue;
    let black = pack(0, 0, 0) == 0;

    disjoint && white && black
}

fn ports_are_exclusive() -> bool {
    port::claim_block::<8>(uart::COM1).is_none() && port::claim::<u8>(uart::COM1).is_none()
}

fn a_save_stays_inside_its_area() -> bool {
    if !xstate::is_enabled() {
        // The bounds check has no instruction to exercise; user launch refuses.
        return xstate::bytes() == 0;
    }
    // The form is whatever the processor supports; what must hold on every
    // processor is that the area was sized for the form in use.
    xstate::save_stays_inside_its_area()
}

fn filling_the_arena_never_repeats_a_slot() -> bool {
    /// Enough to wrap the hint past the end and force a grow or two.
    const HANDLES: usize = 24;

    let table = handle_table_or_false!(HANDLES);
    let mut handles = [0u32; HANDLES];

    for slot in &mut handles {
        let object = object::bootstrap_identity();
        let Ok(handle) = table.insert(object, Rights::UNIVERSAL) else {
            return false;
        };
        *slot = handle;
    }

    // Distinct values. Two claims of one slot would give the same index and
    // the same generation, so the handles themselves would collide.
    let mut distinct = true;
    for (index, handle) in handles.iter().enumerate() {
        for earlier in &handles[..index] {
            distinct &= earlier != handle;
        }
    }

    // And every one still resolves — an overwrite leaves the loser naming a
    // slot that is stale in its table rather than the object it was given.
    let all_live = handles
        .iter()
        .all(|handle| table.kind_of(*handle) == Ok(ObjectType::Channel));

    let counted = table.live() == HANDLES;

    distinct && all_live && counted
}

fn the_fast_syscall_doors_are_shut_by_this_kernel() -> bool {
    cpu::drive_fast_syscall_shutdown_for_test()
}

fn a_reclaimed_space_answers_nothing() -> bool {
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let zero_is_live = space.zero_root_is_live_for_test();

    // Something to look up, so the `None` below is about the reclaim and not
    // about an empty space.
    let Some(mut page) = mm::allocate_contiguous(1, 1) else {
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a missing reclaimed-space page did not return its tables"
        );
        return false;
    };
    if !page.zero() {
        drop(page);
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a refused reclaimed-space zero did not return setup resources"
        );
        return false;
    }
    let at = user::STACK + 0xB0_0000;
    if space
        .map_user(at, page.into(), Flags::WRITABLE.union(Flags::NO_EXECUTE))
        .is_err()
    {
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a refused reclaimed-space map did not return setup resources"
        );
        return false;
    }
    let answered_before = space.lookup(at).is_some();
    let unmapped = space.unmap(at, PAGE_SIZE).map(drop).is_ok();

    // Never bound, so reclaim exercises the cold-space path.
    let reclaimed = space.reclaim(None).is_ok();
    let empty = space.is_reclaimed();

    // Counted, not observed: frame zero reads as zeros, so an unguarded
    // `lookup` returns the same `None` the guard does (D-139).
    let refusals = paging::reclaimed_refusals();

    let silent = space.lookup(at).is_none();
    let unbindable = matches!(space.bind(), Err(ActivateError::Reclaimed));
    let unactivatable = matches!(paging::activate(&mut space), Err(ActivateError::Reclaimed));
    let once_only = matches!(
        space.reclaim(None),
        Err(paging::ReclaimFailure {
            reason: ReclaimError::Reclaimed,
            ..
        })
    );

    let all_four_refused = paging::reclaimed_refusals() == refusals + 4;

    let checked = answered_before
        && zero_is_live
        && unmapped
        && reclaimed
        && empty
        && silent
        && unbindable
        && unactivatable
        && once_only
        && all_four_refused;
    drop(space);
    checked && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

fn a_refused_pair_of_claims_strands_neither() -> bool {
    /// Free, and two below COM1 so it cannot collide with the serial block.
    const FREE: u16 = uart::COM1 - 4;

    // COM1 is the serial driver's, so any pair naming it must be refused.
    let refused = port::claim_pair::<u8, u8>(FREE, uart::COM1).is_none();

    // And the free half is still free: chaining would have taken it on the
    // way to failing.
    let untouched = port::claim::<u8>(FREE).is_some();

    // Which also proves the first line was a real refusal and not a refusal
    // of `FREE` itself — it was claimable a moment later.
    let taken_now = port::claim::<u8>(FREE).is_none();

    refused && untouched && taken_now
}

fn a_claim_covers_the_width_it_drives() -> bool {
    const BELOW_COM1: u16 = uart::COM1 - 2;

    port::claim::<u32>(BELOW_COM1).is_none()
        && port::claim::<u8>(BELOW_COM1).is_some()
        && port::claim::<u8>(BELOW_COM1).is_none()
}

fn lock_excludes() -> bool {
    let lock = SpinLock::new(0u32);
    let held = lock.lock_leaf();
    let refused = lock.try_lock_leaf().is_none();
    drop(held);
    refused && lock.try_lock_leaf().is_some()
}

fn forgotten_lock_ownership_is_distinct_from_contention() -> bool {
    let lock = SpinLock::new(0u32);
    let mut context = context();
    let interrupts_before = interrupt::are_enabled();

    let held = lock.lock(&mut context);
    let ordinary =
        lock.witness_for_test() == SpinLockWitness::Contended && lock.try_lock_leaf().is_none();

    let recovery = held.forget_for_test();
    let forgotten =
        lock.witness_for_test() == SpinLockWitness::Forgotten && lock.try_lock_leaf().is_none();
    drop(recovery);

    let restored = lock.witness_for_test() == SpinLockWitness::Free
        && interrupt::are_enabled() == interrupts_before;
    let reacquired = lock.try_lock(&mut context);
    let available = reacquired.is_some();
    drop(reacquired);

    ordinary
        && forgotten
        && restored
        && available
        && lock.witness_for_test() == SpinLockWitness::Free
        && interrupt::are_enabled() == interrupts_before
}

fn latch_admits_one() -> bool {
    let latch = Once::new();
    !latch.is_claimed() && latch.claim() && !latch.claim() && latch.is_claimed()
}

/// Every input line on both legacy controllers, disconnected.
const MASKED: u8 = 0xFF;

fn a_lock_excludes_interrupts() -> bool {
    let outer = SpinLock::new(0u32);
    let inner = SpinLock::new(0u32);
    let mut context = context();

    let before = interrupt::are_enabled();
    // The sanctioned order is one token-bearing outer lock and one inner leaf.
    // Two leaves would trip D-143's detector and mask a later real violation.
    let outer_guard = outer.lock(&mut context);
    let excluded = !interrupt::are_enabled();

    let inner_guard = inner.lock_leaf();
    drop(inner_guard);
    let still_excluded = !interrupt::are_enabled();

    drop(outer_guard);
    before && excluded && still_excluded && interrupt::are_enabled()
}

fn spins_until(attempts: u32, done: impl Fn() -> bool) -> bool {
    (0..attempts).any(|_| {
        spin_loop();
        done()
    })
}

fn ping_arrives(target: usize) -> bool {
    const ATTEMPTS: u32 = 1_000_000;

    spins_until(ATTEMPTS, || trap::last_ping().count >= target)
}

fn timer_entry_is_programmed() -> bool {
    let entry = apic::timer_entry();
    !entry.is_masked() && entry.is_one_shot() && entry.vector() == vector::TIMER.get()
}

fn rates_are_believable() -> bool {
    (1_000_000..=10_000_000_000).contains(&time::apic_hz())
        && (500_000_000..=10_000_000_000).contains(&time::counter_hz())
}

/// What a continuous read of the clock saw.
struct ClockSweep {
    monotonic: bool,
    /// The smallest step the clock was seen to take, or `None` if it never
    /// took one.
    finest_step: Option<u64>,
}

fn sweep_clock() -> ClockSweep {
    const ATTEMPTS: u32 = 2_000_000;
    const SPAN_MS: u64 = 3;

    let deadline = time::uptime_ms() + SPAN_MS;
    // Seed with a real reading so the first comparison, sampled under a lock,
    // is not vacuously monotonic against zero (D-124).
    let mut previous = time::now_ns();
    let mut sweep = ClockSweep {
        monotonic: true,
        finest_step: None,
    };

    for attempt in 0..ATTEMPTS {
        let sample = if attempt == 0 {
            let lock = SpinLock::new(0u32);
            let held = lock.lock_leaf();
            let sample = time::now_ns();
            drop(held);
            sample
        } else {
            time::now_ns()
        };

        if sample < previous {
            sweep.monotonic = false;
        }
        if attempt > 0 && sample > previous {
            let step = sample - previous;
            sweep.finest_step = Some(sweep.finest_step.map_or(step, |finest| finest.min(step)));
        }
        previous = sample;

        if sample / time::NANOSECONDS_PER_MS >= deadline {
            break;
        }
    }
    sweep
}

fn an_idle_kernel_takes_no_timer_interrupts() -> bool {
    // Fifty 100 Hz periods expose any surviving periodic source.
    const QUIET_MS: u64 = 500;

    let before = time::expiries();
    runs_until(QUIET_MS, || false);
    time::expiries() == before
}

fn the_timer_fires_about_once_a_slice() -> bool {
    const WINDOW_MS: u64 = 300;
    /// `thread::SLICE_NS`, in the unit this window is measured in.
    const SLICE_MS: u64 = 3;

    STOP_SPINNING.store(0, Ordering::Release);
    LIGHT_SPINS.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("competitor", light_spinner).is_err() {
        return false;
    }

    let before = time::expiries();
    runs_until(WINDOW_MS, || false);
    let spent = time::expiries() - before;
    STOP_SPINNING.store(1, Ordering::Release);
    let exited = yields_until(|| thread::free_slots() == free);

    // A factor of four either way: the window is slice-quantised and
    // hypervisor-jittered, and the cases differ by orders of magnitude.
    let expected = WINDOW_MS / SLICE_MS;
    exited && spent >= expected / 4 && spent <= expected * 4
}

fn a_sleeper_wakes_on_time_and_quietly() -> bool {
    /// Long enough that a surviving tick would be caught many times, short
    /// beside the harness's patience.
    const SLEEP_MS: u64 = 200;
    // Five bounded validation windows plus wake slack; a 10 ms periodic source
    // would exceed this budget fourfold.
    const EXPIRY_BUDGET: u64 = 7;
    /// How late the wake may be. Nothing else is runnable, so the thread
    /// resumes the moment its own timer fires — this is slack for the
    /// hypervisor, not for the policy.
    const LATE_MS: u64 = 50;

    let expiries = time::expiries();
    let before = time::uptime_ms();
    thread::sleep_ms(&mut context(), SLEEP_MS);
    let slept = time::uptime_ms() - before;
    let spent = time::expiries() - expiries;

    // Two-sided: armed on the APIC clock and measured on the counter's, so a
    // conversion wrong either way is a wait of the wrong length.
    (SLEEP_MS..=SLEEP_MS + LATE_MS).contains(&slept) && (1..=EXPIRY_BUDGET).contains(&spent)
}

fn sleeping_hands_the_cpu_over() -> bool {
    const SLEEP_MS: u64 = 100;
    /// A thread with the CPU to itself counts far more than this in 100 ms;
    /// one scavenging a spin loop's leftovers counts far fewer.
    const PROGRESS: usize = 10_000;

    STOP_SPINNING.store(0, Ordering::Release);
    LIGHT_SPINS.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("while-asleep", light_spinner).is_err() {
        return false;
    }

    let woke = thread::wakes();
    thread::sleep_ms(&mut context(), SLEEP_MS);
    let progressed = LIGHT_SPINS.load(Ordering::Relaxed) >= PROGRESS;
    let woken = thread::wakes() == woke + 1;
    STOP_SPINNING.store(1, Ordering::Release);
    let exited = yields_until(|| thread::free_slots() == free);

    progressed && woken && exited
}

// Interpret identity in the observed translation mode because a translator
// rewrites replies and scancodes alike (D-178).
const UNTRANSLATED_MF2: (u8, u8) = (0xAB, 0x83);

/// The same keyboard, behind a controller that translates.
const TRANSLATED_MF2: (u8, u8) = (0xAB, 0x41);

fn keyboard_config_matches_its_routing() -> bool {
    const FIRST_IRQ: u8 = 1 << 0;
    const SECOND_IRQ: u8 = 1 << 1;
    const TRANSLATE: u8 = 1 << 6;

    let Some(byte) = input::controller_config() else {
        return false;
    };

    // Translation stays off: a controller quietly converting to set 1 is the
    // classic keyboard that types the wrong characters.
    let raw = byte & TRANSLATE == 0;
    // The second port is never enabled — nothing is on it.
    let second_silent = byte & SECOND_IRQ == 0;
    let first_matches_routing = (byte & FIRST_IRQ != 0) == input::is_interrupt_driven();

    raw && second_silent && first_matches_routing
}

fn ping_is_consistent(ping: Record) -> bool {
    /// `RFLAGS` bit 9.
    const INTERRUPT_FLAG: u64 = 1 << 9;

    ping.vector == u64::from(vector::PING.get())
        && ping.error_code == 0
        && ping.code_selector == u64::from(segment::KERNEL_CODE.bits())
        && ping.flags & INTERRUPT_FLAG != 0
}

fn frame_is_consistent(breakpoint: Record) -> bool {
    /// Bit 1 of the flags register reads as one on every x86 ever made.
    const RESERVED_FLAG: u64 = 1 << 1;

    breakpoint.vector == u64::from(exception::BREAKPOINT)
        && breakpoint.error_code == 0
        && breakpoint.code_selector == u64::from(segment::KERNEL_CODE.bits())
        && breakpoint.stack_selector == u64::from(segment::KERNEL_DATA.bits())
        && breakpoint.flags & RESERVED_FLAG != 0
        && breakpoint.stack_pointer == breakpoint.rax
}

fn frame_spans_the_saved_words(breakpoint: Record) -> bool {
    breakpoint
        .frame_address
        .checked_add(interrupt::SAVED_FRAME_BYTES)
        == Some(breakpoint.rax)
}
