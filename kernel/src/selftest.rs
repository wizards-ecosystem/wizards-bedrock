//! Boot-time self-checks.
//!
//! The kernel cannot host `cargo test`: it is a `#![no_std] #![no_main]` binary,
//! and the in-QEMU test harnesses need nightly. So the checks that need a real
//! machine run here instead, on a kernel built with the `selftest` feature, and
//! report their verdict through [`crate::arch::current::debug_exit`].
//!
//! Everything here must be checkable *on the machine*. Anything that is pure
//! arithmetic belongs in a `libs/` crate with ordinary unit tests instead.
//!
//! Checks run against a fully booted kernel, so anything the boot path already
//! asserts is not repeated here — a rejected base revision panics long before
//! this, which the harness sees as a missing verdict.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt::Write;
use core::hint::spin_loop;
use core::sync::atomic::{AtomicUsize, Ordering};

use handle_space::Rights;
use wost::arch::current::cpu::{self, Protections};
use wost::arch::current::paging::{self, ActivateError, AddressSpace, BootRegion, MapError};
use wost::arch::current::usercopy::{self, UserCopyError};
use wost::arch::current::{apic, interrupt, pic, port, segment};
use wost::boot::{self, FramebufferInfo, Rgb};
use wost::mm;
use wost::sync::{Once, SpinLock};
use x86_paging::{Flags, LARGE_PAGE_SIZE, PAGE_SIZE};
use x86_tables::exception;

use crate::arch::current::debug_exit;
use crate::arch::current::trap::{self, Record};
use crate::arch::current::uart::{self, UartError};
use crate::arch::current::vector;
use crate::handle::{HandleError, HandleTable};
use crate::object::{self, ObjectRef, ObjectType};
use crate::thread::ThreadContext;
use crate::user;
use crate::{dev, error, info, input, mem, thread, time};

/// A frame's handle releases it, and the allocator notices.
///
/// Counted rather than compared by address: nothing promises the next
/// allocation reuses the frame just freed, but the free count must come back
/// to exactly where it started.
fn frame_round_trips() -> bool {
    let before = mm::free_frames();
    let frame = mm::allocate();
    let taken = frame.is_some() && mm::free_frames() == before - 1;
    drop(frame);
    taken && mm::free_frames() == before
}

/// Two frames held at once are never the same frame.
fn frames_are_exclusive() -> bool {
    let (Some(first), Some(second)) = (mm::allocate(), mm::allocate()) else {
        return false;
    };
    first.address() != second.address()
}

/// Dirty memory can be zeroed, and the kernel can tell the difference.
///
/// Dirtied first, because memory on a fresh boot is usually already zero and
/// a check that only reads one back cannot fail. Covers `zero`; reuse of a
/// known-dirty frame is not forced, since the allocator hint moves past a
/// released frame.
fn dirty_memory_can_be_zeroed() -> bool {
    let Some(mut frame) = mm::allocate() else {
        return false;
    };

    let dirtied = frame.fill(0xAA) && frame.is_zero() == Some(false);
    let cleaned = frame.zero() && frame.is_zero() == Some(true);
    dirtied && cleaned
}

/// The allocating form hands out a frame that reads as zero.
fn allocation_is_zeroed() -> bool {
    mm::allocate_zeroed().and_then(|frame| frame.is_zero()) == Some(true)
}

/// A run of frames is consecutive, aligned, and released whole.
fn a_run_is_contiguous_and_aligned() -> bool {
    const FRAMES: usize = 8;

    let before = mm::free_frames();
    let Some(run) = mm::allocate_contiguous(FRAMES, FRAMES) else {
        return false;
    };
    let aligned = run.address().is_multiple_of(FRAMES as u64 * 4096);
    let sized = run.frames() == FRAMES && mm::free_frames() == before - FRAMES;
    drop(run);

    aligned && sized && mm::free_frames() == before
}

/// The kernel is walking the tables it built, not the bootloader's.
///
/// Both halves matter. `is_active` says the switch was attempted and the
/// checks passed; `CR3` says it took effect. A kernel that recorded success
/// and kept the old tables would satisfy only the first.
fn the_loaded_table_is_ours() -> bool {
    paging::is_active() && cpu::page_table_base() == paging::active_root()
}

/// Every address in the kernel image obeys W^X, read back from the tables the
/// CPU is walking.
///
/// Walked rather than assumed: the mapping code could be right and the entries
/// still wrong, and the entries are what the CPU obeys. Each section is probed
/// at its first page, its last, and one in the middle, because a run of large
/// pages can be correct at the edges and wrong between them.
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

/// The direct map is writable and never executable.
///
/// The largest writable mapping in the kernel, and the one an attacker would
/// most like to run code from.
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

/// Firmware reads are confined to immutable ACPI storage.
///
/// Three refusals and one acceptance: a check that only saw refusals would
/// pass against a function that returned `None` for everything.
fn firmware_reads_are_bounded() -> bool {
    const WORD: usize = 8;

    // Chosen under the map's lock, used outside it: the reader takes it again.
    let addresses = mm::with_memory_map(|map| {
        let pick = |wanted: fn(&wost::boot::MemoryRegion) -> bool| {
            map.regions()
                .find(|region| wanted(region) && region.len >= WORD as u64)
                .map(|region| region.base)
        };
        // Inside the address space, so only the map can refuse it — an address
        // at the top would be caught by an overflow guard instead.
        let undescribed = map
            .regions()
            .map(|region| region.base.saturating_add(region.len))
            .max()
            .map(|top| top.next_multiple_of(4096));
        (
            pick(|region| region.kind.is_allocatable()),
            pick(|region| !region.kind.is_firmware_table() && !region.kind.is_allocatable()),
            pick(|region| region.kind.is_firmware_table()),
            undescribed,
        )
    });

    let Some((Some(usable), Some(unreachable), Some(readable), Some(undescribed))) = addresses
    else {
        return false;
    };

    let Some(firmware) = mm::firmware_memory() else {
        return false;
    };
    let mut buffer = [0; WORD];
    !firmware.read(usable, &mut buffer)
        && !firmware.read(undescribed, &mut buffer)
        && !firmware.read(unreachable, &mut buffer)
        && firmware.read(readable, &mut buffer)
}

/// A second switch is refused.
///
/// One `CR3` owner, still: the tables the CPU walks are the ones this kernel
/// built, and no second set can displace them. What this *no longer* claims is
/// that it is why `map` is safe — that was true while `activate` consumed the
/// space, and D-86 replaced it with the property `set` actually enforces.
fn a_second_switch_is_refused() -> bool {
    match AddressSpace::new() {
        Ok(mut space) => paging::activate(&mut space) == Err(ActivateError::AlreadyActive),
        Err(_) => false,
    }
}

/// A mapping added after the switch lands in the tables the CPU is walking.
///
/// The capability D-86 opened, and the one Phase 5 is blocked on: until now
/// `activate` consumed the address space, so stacks, the device window and the
/// heap's backing all had to be decided while the tables were still being
/// built (D-45, D-52). Mapping into the *running* space is what lets a stack
/// arrive with the thread that needs it, and a memory object with the process.
///
/// `translate` is evidence rather than a read-back: it walks the physical
/// tables through the direct map, so a `map` that wrote nothing, or wrote into
/// a table `CR3` does not point at, fails here. What it cannot show is the
/// TLB, and the architecture is what covers that — a translation that was not
/// present was not cached, so no invalidation is owed for a new mapping.
/// Touching the page to prove it would need a raw write, which this crate may
/// not contain (D-13); the safe type for reaching a mapping arrives with
/// memory objects in slice 7, and this check grows a second half there.
fn a_mapping_added_after_the_switch_is_live() -> bool {
    /// A page well clear of the direct map, the stack arena and the device
    /// window, so a failure cannot land on anything the kernel is using.
    const PROBE: u64 = 0xffff_fd00_0000_0000;

    let unmapped_before = paging::translate(PROBE).is_none();

    let Some(run) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let physical = run.address();

    // Ownership of the frame moves into the space (D-87). Nothing unmaps yet,
    // so it stays there for the life of the kernel — deliberately, and the
    // debt the next slice pays.
    let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let mapped =
        mem::with_active_space(&mut context(), |space| space.map_owned(PROBE, run, wanted));
    if !matches!(mapped, Some(Ok(()))) {
        return false;
    }

    // `OWNED` is in the expectation because `map_owned` writes it: the entry
    // is where provenance lives, so unmap can tell the allocator's frames from
    // the firmware's (D-88).
    let expected = wanted.union(Flags::PRESENT).union(Flags::OWNED);
    let translated = paging::translate(PROBE) == Some((physical, expected));
    let on_the_live_root = paging::active_root() == cpu::page_table_base();

    unmapped_before && translated && on_the_live_root
}

/// The event whose already-signalled state the re-check must find.
static REREAD_EVENT: SpinLock<Option<ObjectRef>> = SpinLock::new(None);

/// 0 before the thread runs, 1 once it has come back through `wait_unless`.
static REREAD_RETURNED: AtomicUsize = AtomicUsize::new(0);

/// Wait on an event that is *already* signalled, and record coming back.
fn wait_on_a_signalled_event() {
    let Some(object) = REREAD_EVENT.lock_leaf().clone() else {
        return;
    };
    let Some(event) = object.as_event() else {
        return;
    };
    // Straight at `wait_unless`, not through `Event::wait` — that one tests
    // the state before it calls in, so it would never reach the re-check this
    // is about, and a broken re-check would sail past unnoticed.
    event
        .waiters_for_test()
        .wait_unless(&mut context(), || event.is_signalled());
    REREAD_RETURNED.store(1, Ordering::Release);
}

/// A waiter re-reads its condition after registering, and does not block if it
/// has become true.
///
/// The half of the lost-wakeup fix that one CPU can observe (D-95). The
/// condition is evaluated under the same queue lock a waker must take, so a
/// signal cannot land between the read and the registration — they are one
/// step. The earlier shape read the condition *outside* that lock, and a signal
/// in the gap woke an empty queue while the waiter went on to sleep for ever.
///
/// Run on a spawned thread so that failure is a timeout here rather than a
/// hang: without the re-check the waiter blocks on an event nobody will signal
/// again, and a check that blocks *itself* can only stop the machine. Phase 4's
/// lesson about breaks that crash, applied to where the check runs.
///
/// What this cannot show is the race itself. On one CPU, with the flag clear
/// and the lock held, there is no interleaving left to drive — and the
/// predicate must not touch the queue either, since signalling from inside it
/// deadlocks against the lock it already holds, which is how the first version
/// of this check hung the machine instead of failing it. The race is D-70 rule
/// 6's, and slice 9's model is what will check it.
fn a_waiter_rereads_its_condition_before_blocking() -> bool {
    let event = object::event();
    let Some(inner) = event.as_event() else {
        return false;
    };
    inner.signal();

    *REREAD_EVENT.lock_leaf() = Some(Arc::clone(&event));
    REREAD_RETURNED.store(0, Ordering::Release);
    let free_before = thread::free_slots();
    if thread::spawn("event-rereader", wait_on_a_signalled_event).is_err() {
        return false;
    }

    let came_back = runs_until(200, || REREAD_RETURNED.load(Ordering::Acquire) == 1);
    let settled = runs_until(200, || thread::free_slots() == free_before);
    *REREAD_EVENT.lock_leaf() = None;

    // And nothing is left registered, so a later wake cannot name a thread
    // that is no longer waiting here.
    came_back && settled && inner.waiters_for_test().is_idle()
}

/// A thread crosses into ring 3, calls back through the one gate that admits
/// it, and is retired from the far side.
///
/// The boundary this whole phase is for, proved end to end rather than in
/// parts: the ring-3 descriptors, the privilege stack, the `iretq` frame, the
/// user-callable gate, and the return path all have to agree, and any one of
/// them being wrong is a fault in a privilege level that cannot report it.
///
/// The evidence is the saved code selector. `CS` in the trap frame is the one
/// the *CPU* pushed when it took the interrupt, so a ring-3 value there cannot
/// be produced by a kernel thread saying `int 0x80` — the number in it is the
/// hardware's account of where the caller was (D-96).
fn a_thread_crosses_into_ring_three_and_calls_back() -> bool {
    let before = trap::last_syscall();
    let retirements_before = thread::retirements();
    if thread::spawn("user", user::run).is_err() {
        return false;
    }

    // Generous: the thread maps two pages and a guarded stack before it can
    // cross, and a refusal on any of them logs rather than hanging.
    if !runs_until(2000, || trap::last_syscall().count > before.count) {
        return false;
    }
    let call = trap::last_syscall();

    // Ring 3, by the selector the CPU pushed.
    let from_user = call.code_selector & 0b11 == 3;
    // And through the gate meant for it.
    let through_the_gate = call.vector as u8 == vector::SYSCALL;
    // Which is the only one ring 3 may reach.
    let only_gate = interrupt::gate_privilege(vector::SYSCALL) == 3
        && interrupt::gate_privilege(vector::TIMER) == 0
        && interrupt::gate_privilege(vector::KEYBOARD) == 0
        && interrupt::gate_privilege(exception::PAGE_FAULT) == 0;
    // Its stack was the user one: ring 3 in the selector, and a low-half
    // address, which is the half the kernel never maps.
    let on_a_user_stack =
        call.stack_selector & 0b11 == 3 && call.stack_pointer < usercopy::USER_END;
    // And the thread is gone, counted rather than inferred. The first version
    // of this waited for a slot to come back — which a retired thread never
    // returns — and fell back to a comparison that is true of the checking
    // thread whatever happened, so it could not fail at all (D-101).
    let retired = thread::retirements() > retirements_before;

    from_user && through_the_gate && only_gate && on_a_user_stack && retired
}

/// A ring-3 program cannot bring SMAP down with it.
///
/// The hole a third adversarial pass found (D-101), and the reason the program
/// in the image is hostile: `EFLAGS.AC` is writable at CPL 3, and interrupt
/// delivery clears `TF`, `NT`, `RF` and `VM` but **not** `AC`. So a program
/// that sets it hands the kernel a trap with SMAP already off — for the whole
/// handler, and for every thread switched to afterwards, since a context
/// switch restores no flags.
///
/// Both halves are asserted, because either alone would pass for the wrong
/// reason. The saved flags must show the program really did set `AC` — a
/// program that failed to would make the second half true trivially. And the
/// handler must have been running with it clear.
fn a_program_cannot_bring_smap_down_with_it() -> bool {
    /// `EFLAGS.AC`.
    const AC: u64 = 1 << 18;

    let call = trap::last_syscall();
    if call.count == 0 {
        return false;
    }
    // The program set it, so this check is about something that happened.
    let the_program_tried = call.flags & AC != 0;
    // And the entry path took it away again.
    let the_kernel_undid_it = trap::smap_survived_the_last_syscall();

    the_program_tried && the_kernel_undid_it && interrupt::access_clear_is_armed()
}

/// A program that faults dies, and the machine does not.
///
/// D-31 made a fault terminal when a fault was necessarily a kernel bug. Since
/// ring 3 exists it can be a program bug, and D-97 already reasons from the
/// right premise for the copy path — *the program is the untrusted party*.
/// D-102 applies it to the trap path: a fault from CPL 3 retires the caller.
///
/// The program executes `ud2` on purpose, chosen by the argument word the
/// kernel wrote onto its stack. That the fault is deliberate is what makes
/// this a check rather than a hope: an accidental fault would be indeterminate
/// about *which* fault, and `#UD` is the one nothing else in this kernel
/// raises.
///
/// Three things are asserted, and the first is the one that would otherwise be
/// assumed: the fault must have come from ring 3, by the selector the CPU
/// pushed. A kernel `ud2` retired instead of reported would be a kernel bug
/// swallowed, which is worse than the panic this replaces.
fn a_faulting_program_dies_alone() -> bool {
    let before = trap::last_user_fault();
    let retirements_before = thread::retirements();
    if thread::spawn("user-fault", user::run_faulting).is_err() {
        return false;
    }

    if !runs_until(2000, || trap::last_user_fault().count > before.count) {
        return false;
    }
    let fault = trap::last_user_fault();

    // From ring 3, by the CPU's own account.
    let from_user = fault.code_selector & 0b11 == 3;
    // The fault the program chose, and no other.
    let it_was_the_ud2 = fault.vector == u64::from(exception::INVALID_OPCODE);
    // Its instruction pointer is in user memory, not the kernel's.
    let in_user_memory = fault.stack_pointer < usercopy::USER_END;
    // The thread was retired rather than the machine stopped — and the machine
    // is plainly still here, since this line runs.
    let retired = runs_until(500, || thread::retirements() > retirements_before);

    from_user && it_was_the_ud2 && in_user_memory && retired
}

/// The loader refuses an image that asks for what it may not map.
///
/// The half of a loader that matters. An image is bytes a program supplied,
/// and every one of these is a way of asking the kernel to map something it
/// should not: a segment both writable and executable, one whose contents run
/// past the file, one claiming more memory than the file holds, and one placed
/// in the half of the address space the kernel lives in.
///
/// **W^X is not relaxed for an image that asks nicely** (D-100). A loader that
/// dropped the write bit to make a program load would be loading something
/// that faults later and blames the kernel.
///
/// Each is a copy of the real initrd with one field changed, so what is being
/// refused differs from what loads by exactly the thing under test.
fn the_loader_refuses_what_it_may_not_map() -> bool {
    let Some(original) = user::image() else {
        return false;
    };
    /// Offset of the first program header's flags, from the ELF64 layout.
    const FLAGS: usize = elf64::HEADER_BYTES + 4;
    /// Its `p_offset`, `p_filesz` and `p_vaddr`.
    const OFFSET: usize = elf64::HEADER_BYTES + 8;
    const VADDR: usize = elf64::HEADER_BYTES + 16;
    const FILE_SIZE: usize = elf64::HEADER_BYTES + 32;
    const MEMORY_SIZE: usize = elf64::HEADER_BYTES + 40;

    let doctor = |at: usize, value: u64| -> Vec<u8> {
        let mut copy = original.to_vec();
        copy[at..at + 8].copy_from_slice(&value.to_le_bytes());
        copy
    };

    // Writable and executable at once.
    let mut write_execute = original.to_vec();
    write_execute[FLAGS..FLAGS + 4].copy_from_slice(&0b111u32.to_le_bytes());
    let wx_refused = matches!(
        user::load_image(&mut context(), &write_execute, 0),
        Err(user::LoadError::WriteExecute(_))
    );

    // Contents past the end of the image.
    let overrun = doctor(FILE_SIZE, 0x1000);
    let overrun_refused = matches!(
        user::load_image(&mut context(), &overrun, 0),
        Err(user::LoadError::Malformed(_))
    );

    // More file than memory to put it in.
    let mut oversized = doctor(FILE_SIZE, 8);
    oversized[MEMORY_SIZE..MEMORY_SIZE + 8].copy_from_slice(&4u64.to_le_bytes());
    let oversized_refused = matches!(
        user::load_image(&mut context(), &oversized, 0),
        Err(user::LoadError::Malformed(_))
    );

    // Placed in the kernel's half.
    let intruding = doctor(VADDR, 0xffff_8000_0000_0000);
    let intruding_refused = matches!(
        user::load_image(&mut context(), &intruding, 0),
        Err(user::LoadError::NotUserMemory(_))
    );

    // And something that is not an ELF at all.
    let not_an_elf = [0u8; 64];
    let refused_outright = matches!(
        user::load_image(&mut context(), &not_an_elf, 0),
        Err(user::LoadError::NotAnElf(_))
    );

    let _ = OFFSET;
    wx_refused && overrun_refused && oversized_refused && intruding_refused && refused_outright
}

/// A second thread crosses too, and each trap lands on its own kernel stack.
///
/// The limitation D-96 wrote down and D-99 lifted. `RSP0` used to be set once,
/// to a stack of its own, so a second thread in ring 3 would have taken its
/// trap on a stack the first was standing on. It now moves with the thread, in
/// the one place every switch passes through.
///
/// The evidence is the stack the trap landed on. Each crossing records the
/// kernel stack `RSP0` named at the time, and the two must differ and must
/// each be the arena slot of the thread that called — a single shared landing
/// would give one address twice.
///
/// The two programs share an address space, so this is not isolation and is
/// not claimed to be: what it shows is that the *kernel* side of two
/// simultaneous crossings is separate. Isolation needs a second set of tables,
/// which is what the rest of this slice is for.
fn a_second_thread_crosses_onto_its_own_stack() -> bool {
    let first = trap::last_syscall();
    if first.count == 0 {
        return false;
    }
    let first_landing = segment::kernel_stack();

    if thread::spawn("user-two", user::run_second).is_err() {
        return false;
    }
    if !runs_until(2000, || trap::last_syscall().count > first.count) {
        return false;
    }
    let second = trap::last_syscall();

    // Ring 3 again, through the same one gate.
    let from_user = second.code_selector & 0b11 == 3;
    let through_the_gate = second.vector as u8 == vector::SYSCALL;
    // At its own user addresses, not the first program's.
    let elsewhere =
        second.stack_pointer != first.stack_pointer && second.stack_pointer < usercopy::USER_END;
    // And `RSP0` moved: the landing the second thread trapped onto is not the
    // one the first used.
    let own_landing = segment::kernel_stack() != first_landing;

    from_user && through_the_gate && elsewhere && own_landing
}

/// The return to ring 3 clears the buffers, and the crossing happened with it
/// armed.
///
/// D-81 asked for `VERW` on the way out because it is one instruction on a
/// path that is painful to modify once mature. D-98 arms it unless the
/// processor reports `MDS_NO`, rather than gating on `MD_CLEAR` — `verw`
/// exists on every x86-64, and that bit says only whether it also flushes, so
/// a machine whose capability word cannot be read gets the instruction rather
/// than the assumption.
///
/// That policy is what makes this checkable at all. QEMU reports no capability
/// word (D-91), so gating on `MD_CLEAR` would have left the instruction
/// unreachable here — the same hole eIBRS is still in.
///
/// What the check cannot show is that the flush *flushes*: nothing observable
/// changes. What it shows is that the branch is armed and that a ring-3 return
/// was taken through it, which is the part that could crash and the part a
/// later edit could break.
fn the_return_to_ring_three_clears_buffers() -> bool {
    let armed = interrupt::buffer_clear_is_armed();
    // The crossing already happened, and it happened after boot armed this —
    // so the path with the flush in it is a path this machine has run.
    let crossed = trap::last_syscall().count > 0;
    armed && crossed
}

/// Bytes cross the boundary in both directions, and every way they must not
/// is refused.
///
/// The user-copy ABI (D-81, D-97). It runs after the ring-3 check, so the
/// program's pages are mapped and this reads the very instructions the CPU
/// executed — which is stronger than a round trip through memory the kernel
/// also wrote, because it is the *user* mapping being reached through.
///
/// The refusals are the half that matters. A user pointer naming kernel
/// memory, a range that runs off the end of user space, a range that wraps,
/// an unmapped page, and a write to a page that is user-readable but not
/// user-writable — the last being the code page, which is executable and
/// read-only precisely so that W^X holds for user memory too.
fn bytes_cross_the_boundary_only_where_they_may() -> bool {
    let mut read = [0u8; user::PROGRAM.len()];
    if usercopy::copy_from_user(user::CODE, &mut read).is_err() {
        return false;
    }
    let program_matches = read == user::PROGRAM;

    // A round trip through the user stack, which is writable.
    let written = [0xA5u8; 8];
    let mut echoed = [0u8; 8];
    let round_trip = usercopy::copy_to_user(user::STACK, &written).is_ok()
        && usercopy::copy_from_user(user::STACK, &mut echoed).is_ok()
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
        usercopy::copy_to_user(user::CODE, &written),
        Err(UserCopyError::ReadOnly(_))
    );

    program_matches
        && round_trip
        && kernel_refused
        && straddling_refused
        && wrap_refused
        && hole_refused
        && read_only_refused
}

/// A handle names its object, and a closed one names nothing.
///
/// The lifetime property the whole model rests on (D-78). A handle kept across
/// a close must not name whatever takes the slot next, and the generation is
/// what turns that use-after-free into a decode failure rather than a silent
/// reference to someone else's object.
fn a_closed_handle_names_nothing() -> bool {
    let mut table = HandleTable::new(8);
    let Ok(handle) = table.insert(object::event(), Rights::UNIVERSAL) else {
        return false;
    };

    let resolves = table.kind_of(handle) == Ok(ObjectType::Event);
    let counted = table.live() == 1;

    if table.close(handle).is_err() {
        return false;
    }
    let stale = matches!(table.resolve(handle), Err(HandleError::Stale));
    let closed_twice = table.close(handle) == Err(HandleError::Stale);
    let emptied = table.live() == 0;

    // And the slot's next tenant does not answer to the old handle, which is
    // the case the generation exists for rather than the one above.
    let Ok(next) = table.insert(object::event(), Rights::UNIVERSAL) else {
        return false;
    };
    let not_reused = next != handle && matches!(table.resolve(handle), Err(HandleError::Stale));

    resolves && counted && stale && closed_twice && emptied && not_reused
}

/// A right that means nothing for an object type is refused for it.
///
/// D-78's one rule — *a right must correspond to a distinction the kernel
/// actually makes about that object, never to a syscall name* — enforced
/// rather than stated. It was neither, until an adversarial pass noticed that
/// the first object shipped with `READ`, `WRITE`, `EXECUTE` and `MAP` granted
/// on an event, and that the check named for rights used that as its fixture
/// (D-95).
///
/// An event has no pages. `READ` on one is a category error, not a narrower
/// permission, and granting it would make the rights word mean less every time
/// an object type was added.
fn a_meaningless_right_is_refused() -> bool {
    let mut table = HandleTable::new(4);

    let refused = table.insert(object::event(), Rights::READ) == Err(HandleError::Meaningless);
    let also_refused = table.insert(object::event(), Rights::ALL) == Err(HandleError::Meaningless);

    // The universal four mean the same for everything, so they are always
    // available — that is what makes them universal.
    let allowed = table.insert(object::event(), Rights::UNIVERSAL).is_ok();
    let nothing_is_allowed = table.insert(object::event(), Rights::NONE).is_ok();

    refused && also_refused && allowed && nothing_is_allowed && table.live() == 2
}

/// A handle from one table is not a handle in another.
///
/// The single ownership comparison that makes a *global* arena safe to share
/// between tables (D-78). A well-formed handle belonging to someone else is
/// refused as theirs, and a value from another table usually does not decode
/// at all, because the mixers differ.
fn a_handle_does_not_cross_tables() -> bool {
    let mut mine = HandleTable::new(4);
    let theirs = HandleTable::new(4);
    let Ok(handle) = mine.insert(object::event(), Rights::UNIVERSAL) else {
        return false;
    };

    // Forged deliberately, because the accident does not reach the comparison.
    // Two mixers differ, so `mine`'s handle decodes in `theirs` to an index
    // that is almost never in the arena — and the bounds check refuses it
    // before the owner is ever looked at. An earlier version accepted any of
    // three errors and so passed with the ownership comparison deleted (D-95).
    let Some(slot) = mine.slot_of(handle) else {
        return false;
    };
    let forged = theirs.handle_for(slot);
    let refused = theirs.resolve(forged).err() == Some(HandleError::NotYours);

    // The accidental case is refused too, however it happens to land.
    let stray = matches!(
        theirs.resolve(handle),
        Err(HandleError::NotYours) | Err(HandleError::Malformed) | Err(HandleError::Stale)
    );
    let still_mine = mine.resolve(handle).is_ok();
    refused && stray && still_mine
}

/// Rights are checked, and duplication only ever narrows them.
fn rights_are_checked_and_only_narrow() -> bool {
    let mut table = HandleTable::new(8);
    // `WAIT` rather than `READ`: an event has no pages, so a memory right on
    // one is a category error rather than a narrower permission (D-95). This
    // fixture used to grant `READ` on an event and check it was honoured,
    // which baked the violation of D-78's rule into the check named for it.
    let readable = Rights::INSPECT.union(Rights::DUPLICATE).union(Rights::WAIT);
    let Ok(handle) = table.insert(object::event(), readable) else {
        return false;
    };

    let allowed = table.resolve_with(handle, Rights::WAIT).is_ok();
    let denied = matches!(
        table.resolve_with(handle, Rights::TRANSFER),
        Err(HandleError::Denied)
    );

    let Ok(narrowed) = table.duplicate(handle, Rights::WAIT) else {
        return false;
    };
    let narrowed_cannot_duplicate =
        table.duplicate(narrowed, Rights::WAIT) == Err(HandleError::Denied);

    let refused_widening =
        table.duplicate(handle, readable.union(Rights::TRANSFER)) == Err(HandleError::Denied);

    allowed && denied && narrowed_cannot_duplicate && refused_widening
}

/// A table cannot hold more than its quota, and closing makes room again.
///
/// D-44 put the kernel-memory bound at the handle layer rather than at the
/// frame allocator, on the grounds that a quota is policy and stays revisable
/// while making callers donate memory would have been an ABI decision and
/// permanent.
fn a_table_cannot_exceed_its_quota() -> bool {
    const QUOTA: usize = 3;
    let mut table = HandleTable::new(QUOTA);
    let mut held = Vec::new();
    for _ in 0..QUOTA {
        match table.insert(object::event(), Rights::UNIVERSAL) {
            Ok(handle) => held.push(handle),
            Err(_) => return false,
        }
    }

    let refused =
        table.insert(object::event(), Rights::UNIVERSAL) == Err(HandleError::QuotaExhausted);
    let full = table.live() == QUOTA;

    let Some(first) = held.first().copied() else {
        return false;
    };
    if table.close(first).is_err() {
        return false;
    }
    let room_again = table.insert(object::event(), Rights::UNIVERSAL).is_ok();

    refused && full && room_again
}

/// An object outlives every handle to it, and no longer.
///
/// `Arc` is the refcount (D-78 wants `inc_not_zero` from day one), so this is
/// really a check that the table holds a reference rather than a pointer: two
/// handles keep one object alive, and it dies when the second goes.
fn an_object_lives_exactly_as_long_as_its_handles() -> bool {
    let mut table = HandleTable::new(8);
    let object = object::event();
    let watcher = Arc::downgrade(&object);
    let Ok(first) = table.insert(object, Rights::UNIVERSAL) else {
        return false;
    };
    let Ok(second) = table.duplicate(first, Rights::UNIVERSAL) else {
        return false;
    };

    let alive = watcher.upgrade().is_some();
    if table.close(first).is_err() {
        return false;
    }
    let still_alive = watcher.upgrade().is_some();
    if table.close(second).is_err() {
        return false;
    }
    let gone = watcher.upgrade().is_none();

    alive && still_alive && gone
}

/// A table's death closes everything it held.
///
/// Which is the whole of a process's teardown once there are processes: its
/// authority is its handles, so releasing them releases everything it could
/// reach.
fn a_dead_table_releases_its_objects() -> bool {
    let object = object::event();
    let watcher = Arc::downgrade(&object);
    {
        let mut table = HandleTable::new(4);
        if table.insert(object, Rights::UNIVERSAL).is_err() {
            return false;
        }
        if watcher.upgrade().is_none() {
            return false;
        }
    }
    watcher.upgrade().is_none()
}

/// The object a handle names is shared with the thread that waits on it.
static WAITED_EVENT: SpinLock<Option<ObjectRef>> = SpinLock::new(None);

/// 0 before the waiter runs, 1 once it is about to block, 2 once it is woken.
static EVENT_WAITER: AtomicUsize = AtomicUsize::new(0);

/// Block on the shared event, and record having come back.
fn wait_on_the_event() {
    let Some(object) = WAITED_EVENT.lock_leaf().clone() else {
        return;
    };
    let Some(event) = object.as_event() else {
        return;
    };
    EVENT_WAITER.store(1, Ordering::Release);
    event.wait(&mut context());
    EVENT_WAITER.store(2, Ordering::Release);
}

/// A thread waiting on an event is off the run queue, and a signal wakes it.
///
/// The first object doing what it exists for, reached the way everything will
/// be reached: through a handle carrying the right to wait on it. `WAIT` is a
/// right rather than a syscall name because being allowed to observe an
/// object's signals is a distinction the kernel genuinely makes about it
/// (D-78's rule).
///
/// The signal is sticky, which is what makes the waiter's check-then-block
/// safe: a signal landing between the two sets a state the loop re-reads,
/// rather than delivering a wakeup that can be missed.
fn a_signal_wakes_a_thread_waiting_on_a_handle() -> bool {
    let mut table = HandleTable::new(4);
    let Ok(handle) = table.insert(object::event(), Rights::WAIT.union(Rights::INSPECT)) else {
        return false;
    };
    let Ok(object) = table.resolve_with(handle, Rights::WAIT) else {
        return false;
    };
    let Some(event) = object.as_event() else {
        return false;
    };
    if event.is_signalled() {
        return false;
    }

    *WAITED_EVENT.lock_leaf() = Some(Arc::clone(&object));
    EVENT_WAITER.store(0, Ordering::Release);
    let free_before = thread::free_slots();
    let blocks_before = thread::blocks();
    if thread::spawn("event-waiter", wait_on_the_event).is_err() {
        return false;
    }

    // Wait for the *block*, not for the flag the waiter sets on its way in.
    // Signalling between those two would leave nothing on the queue to wake,
    // and this check would pass on a thread that never slept — which is the
    // shape D-49 caught lying, and which the first version of this check had.
    if !runs_until(200, || thread::blocks() > blocks_before) {
        return false;
    }
    let blocked = EVENT_WAITER.load(Ordering::Acquire) == 1;

    let woke_count = event.signal();
    let came_back = runs_until(200, || EVENT_WAITER.load(Ordering::Acquire) == 2);

    // A waiter arriving after the signal does not block at all, which is what
    // sticky means and what a condition variable would get wrong.
    let late_blocks = thread::blocks();
    event.wait(&mut context());
    let late_did_not_block = thread::blocks() == late_blocks;

    // Leave nothing running: a thread still exiting perturbs whatever check
    // comes next, and the alternation check is sensitive to exactly that.
    let settled = runs_until(200, || thread::free_slots() == free_before);
    *WAITED_EVENT.lock_leaf() = None;

    blocked && woke_count >= 1 && came_back && late_did_not_block && settled
}

/// The boot thread stands on a guarded stack, and not the bootloader's.
///
/// The last of the three stacks `002`'s security baseline named. It could not
/// be fixed where it was: the bootloader's stack lives inside the direct map,
/// which is two-megabyte on purpose (D-45), so guarding it would mean
/// splitting a large page — which D-88 refuses. The boot thread therefore
/// leaves it instead (D-92).
///
/// This check runs *on* that stack, which is what makes it more than
/// structural: the stack pointer under it must be inside the mapped region,
/// so a switch that silently did not happen fails here rather than passing on
/// the strength of the tables alone.
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
    true
}

/// The double-fault handler stands on a guarded stack.
///
/// One of the two stacks `002`'s security baseline recorded as having nothing
/// below them. It used to be a `.bss` array, so an overrun ran into whatever
/// the linker put next — silently, on the one path that exists because the
/// kernel is already in trouble.
///
/// Structural, like every guard check here: the fault itself is terminal, so
/// what is asserted is what the tables say (D-52). The interrupt-stack entry
/// must point at the top of the mapped region, every page of it must be
/// writable and never executable, and the page below must be absent.
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

    // It used to end by comparing the stack top against the layout base, which
    // is true of every layout ever built — the failure path was unreachable,
    // which is D-77's own corrected mistake reappearing. The property that
    // matters is the one checked above: the interrupt-stack entry points at
    // *this* region, which the `.bss` array it replaced never satisfies.
    true
}

/// A walk reports the permission the CPU will grant, not the leaf's opinion.
///
/// The architecture takes `USER` and `WRITABLE` as the conjunction down the
/// whole walk and `NO_EXECUTE` as the disjunction, so a leaf that says `USER`
/// under a parent that does not is *not* user memory. `translate` used to
/// return the leaf's flags alone (D-103), and four callers — `enter_user`
/// twice and `admits` twice — read them as effective. It held only because
/// `Entry::table` always sets `PRESENT | WRITABLE` and `set` upgrades parents
/// to `USER` when a user leaf needs one, so the one narrowable bit was always
/// repaired on the way down.
///
/// Checked here rather than in `libs/x86-paging` because it is a property of
/// the *walk*, not of an entry: it needs a table hierarchy with a parent that
/// disagrees with its child, which only a real address space has.
fn a_walk_reports_the_permission_the_cpu_grants() -> bool {
    // Two addresses in the same page table, so they share every parent above
    // it. If narrowing were absent, they would still agree — so the check
    // below turns the parent itself.
    let user_at = user::CODE;
    let Some((_, user_flags)) = paging::translate(user_at) else {
        return false;
    };
    // The program's page really is user-accessible, all the way down.
    let user_is_user = user_flags.contains(Flags::USER);

    // The kernel's own text is not, and its parents are not either — so a walk
    // that reported the leaf alone and a walk that narrows must agree here,
    // which is what makes the next part the interesting one.
    let text = boot::sections().text.start;
    let Some((_, text_flags)) = paging::translate(text) else {
        return false;
    };
    let kernel_is_not_user = !text_flags.contains(Flags::USER);

    // And the direct map, which is writable and never executable, reports both
    // — a walk that lost the disjunction would drop the `NO_EXECUTE` a parent
    // contributes.
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

/// A refused mapping leaves no trace, and gives the frames back.
///
/// The defect an adversarial pass found in D-87 (fixed by D-94): `map_range`
/// wrote entries as it walked, so a range that failed part-way left the
/// earlier pages mapped — while `map_owned`, believing nothing had been
/// mapped, dropped the run and handed those very frames back to the allocator.
/// A live writable mapping onto reallocated memory, reachable from safe code.
///
/// Checked from both ends, because either alone would have passed before the
/// fix: the address must be unmapped afterwards, *and* the frames must be back.
fn a_refused_mapping_leaves_no_trace() -> bool {
    /// Two pages clear of every other probe here, in a page table that this
    /// check populates itself rather than inheriting from whatever ran first.
    const BASE: u64 = 0xffff_fd00_0002_0000;
    let blocker = BASE + PAGE_SIZE;
    let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);

    // Occupy the *second* page of the range, so the mapping below gets one
    // page in before it is refused.
    let Some(taken) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let placed = mem::with_active_space(&mut context(), |space| {
        space.map_owned(blocker, taken, wanted)
    });
    if !matches!(placed, Some(Ok(()))) {
        return false;
    }

    // From here the page table exists, so nothing below allocates one and the
    // frame count means what it looks like.
    let free_before = mm::free_frames();
    let Some(run) = mm::allocate_contiguous(2, 1) else {
        return false;
    };
    let refused =
        mem::with_active_space(&mut context(), |space| space.map_owned(BASE, run, wanted));

    let rejected = matches!(refused, Some(Err(MapError::Conflict(address))) if address == blocker);
    let no_trace = paging::translate(BASE).is_none();
    let returned = mm::free_frames() == free_before;
    let blocker_intact = paging::translate(blocker).is_some();

    rejected && no_trace && returned && blocker_intact
}

/// An unmapped range gives its frames back, and not before the invalidation.
///
/// The debt D-86 named and left open: while nothing unmapped, a frame put into
/// the live space could never be returned, because freeing it would hand the
/// allocator a page a live mapping still reached. `unmap` closes it, and the
/// shape that makes it sound is that the frames live *inside* the token until
/// its invalidation has run (D-88) — so this counts free frames before and
/// after, and the count only comes back once the token is spent.
fn an_unmapped_range_returns_its_frames() -> bool {
    /// Clear of the direct map, the stack arena and the device window.
    const PROBE: u64 = 0xffff_fd00_0001_0000;
    /// More than one page, so the walk covers a run rather than a single slot.
    const PAGES: u64 = 4;

    let Some(run) = mm::allocate_contiguous(PAGES as usize, 1) else {
        return false;
    };
    let physical = run.address();

    let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let mapped =
        mem::with_active_space(&mut context(), |space| space.map_owned(PROBE, run, wanted));
    if !matches!(mapped, Some(Ok(()))) {
        return false;
    }
    let mapped_here = paging::translate(PROBE).map(|(frame, _)| frame) == Some(physical);

    // Counted *after* the mapping, so the page-table frames it may have taken
    // are already spent and this measures only the run. Taking it earlier made
    // the check depend on whether a previous one had built the same table.
    let before = mm::free_frames();

    let Some(Ok(token)) = mem::with_active_space(&mut context(), |space| {
        space.unmap(PROBE, PAGES * PAGE_SIZE)
    }) else {
        return false;
    };

    // The entries are gone the moment `unmap` returns...
    let gone = paging::translate(PROBE).is_none();
    // ...but the frames are not the allocator's yet.
    let withheld = mm::free_frames() == before && token.pages() == PAGES;

    let returned = token.flush().is_some_and(|segment| {
        let same = segment.address() == physical && segment.frames() == PAGES as usize;
        drop(segment);
        same
    });
    let counted = mm::free_frames() == before + PAGES as usize;
    let _ = physical;

    mapped_here && gone && withheld && returned && counted
}

/// Unmapping refuses everything it cannot prove it owns.
///
/// Four ways to ask for something that is not a run of owned four-kilobyte
/// pages, and four refusals. The direct map is the interesting one: it is
/// mapped in two-megabyte pages on purpose (D-45), splitting it is not offered,
/// and the measurement in `010` §8 says keeping it that way is right.
fn unmapping_refuses_what_it_does_not_own() -> bool {
    /// Never mapped by anything.
    const HOLE: u64 = 0xffff_fd00_0004_0000;

    // The kernel's own text, at its linked address: mapped in four-kilobyte
    // pages, and never the allocator's to hand back.
    let text = boot::sections().text.start;

    // And a direct-map address, which is mapped in *two-megabyte* pages on
    // purpose (D-45). Each is a distinct refusal with a distinct reason, and
    // an earlier version of this check accepted either from one address —
    // which could not tell which mechanism had fired.
    // Two megabytes up, not one: the direct map rounds small low regions out
    // to four-kilobyte pages, so an address below the first aligned stretch is
    // *not* in a large page and would test the wrong refusal. Probed rather
    // than assumed.
    let Some(direct) = mm::direct_map().and_then(|map| map.virtual_address(LARGE_PAGE_SIZE)) else {
        return false;
    };

    mem::with_active_space(&mut context(), |space| {
        matches!(space.unmap(HOLE, PAGE_SIZE), Err(MapError::NotMapped(_)))
            && matches!(space.unmap(HOLE + 1, PAGE_SIZE), Err(MapError::Misaligned(_)))
            && matches!(space.unmap(HOLE, 0), Err(MapError::Misaligned(_)))
            // A low-half alias of a higher-half address walks the same tables
            // once the top bits are masked, and the invalidation that followed
            // would name an address `invlpg` ignores (D-94).
            && matches!(
                space.unmap(HOLE & 0x0000_ffff_ffff_ffff, PAGE_SIZE),
                Err(MapError::NonCanonical(_))
            )
            && matches!(space.unmap(text, PAGE_SIZE), Err(MapError::NotOwned(_)))
            && matches!(space.unmap(direct, PAGE_SIZE), Err(MapError::Unsplittable(_)))
    })
    .unwrap_or(false)
}

/// A mapping cannot be silently re-pointed; an identical rewrite is the one
/// rewrite allowed, because the direct map's outward rounding produces it.
fn a_differing_remap_is_refused() -> bool {
    /// Canonical, and far from anything a fresh space would map.
    const PROBE: u64 = 0xffff_8800_0000_0000;

    let Ok(mut space) = AddressSpace::new() else {
        return false;
    };
    let (Some(here), Some(again_here), Some(elsewhere)) = (
        BootRegion::describe(PAGE_SIZE, PAGE_SIZE),
        BootRegion::describe(PAGE_SIZE, PAGE_SIZE),
        BootRegion::describe(2 * PAGE_SIZE, PAGE_SIZE),
    ) else {
        return false;
    };
    let first = space.map_boot(PROBE, here, Flags::NO_EXECUTE);
    let again = space.map_boot(PROBE, again_here, Flags::NO_EXECUTE);
    let differing = space.map_boot(PROBE, elsewhere, Flags::NO_EXECUTE);
    first.is_ok() && again.is_ok() && differing == Err(MapError::Conflict(PROBE))
}

/// A physical range the boot handoff never described cannot be mapped at all.
///
/// D-76 left `map` able to name any physical address, and D-77 recorded it as
/// an authority debt for "the phase that introduces the needed owner types".
/// This is that debt paid on the memory side (D-87): the public entries take
/// either frames the caller owns or a [`BootRegion`], and a `BootRegion`
/// exists only where the bootloader's map says there is memory.
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

/// Two virtual windows cannot confer ownership of the same physical device.
fn device_mappings_are_physically_exclusive() -> bool {
    const PROBE: u64 = 0xfee0_0000;
    let Ok(mut space) = AddressSpace::new() else {
        return false;
    };
    if space.map_device(PROBE, 4096).is_err() {
        return false;
    }
    matches!(
        space.map_device(PROBE, 4096),
        Err(MapError::DeviceOverlap(PROBE))
    )
}

/// Device authority never aliases a frame the allocator can hand out.
fn device_mappings_refuse_allocatable_ram() -> bool {
    let Some(frame) = mm::allocate() else {
        return false;
    };
    let address = frame.address();
    let Ok(mut space) = AddressSpace::new() else {
        return false;
    };
    matches!(
        space.map_device(address, 4),
        Err(MapError::NotDeviceMemory(found)) if found == address
    )
}

/// Empty and wrapping MMIO extents are rejected before a page is installed.
fn malformed_device_extents_are_refused() -> bool {
    let Ok(mut space) = AddressSpace::new() else {
        return false;
    };
    matches!(
        space.map_device(0xfee0_0000, 0),
        Err(MapError::InvalidDeviceRange(0xfee0_0000))
    ) && matches!(
        space.map_device(4, u64::MAX),
        Err(MapError::InvalidDeviceRange(4))
    )
}

/// A device claim minted by a space that was never installed is refused —
/// window addresses collide across spaces, so honouring it would cover
/// whatever the installed space put there (D-49).
fn a_stale_device_claim_is_refused() -> bool {
    let mapping = {
        let Ok(mut space) = AddressSpace::new() else {
            return false;
        };
        match space.map_device(0xfec0_0000, 4096) {
            Ok(mapping) => mapping,
            Err(_) => return false,
        }
    };
    mapping.claim().is_none()
}

/// Backspace erases and Enter starts a line, as a person pressing them expects.
///
/// Drives the console directly rather than waiting on the injected keystrokes:
/// every check between here and the injection logs a line, and every line moves
/// the cursor, so watching for motion would race this function's own output.
/// The layout half is asserted from `char_for` and tested on the host in `libs/ps2-keys`.
///
/// Runs only once the last injected key has been seen released: the harness
/// injects three, and one still arriving between two cursor readings is a red
/// check with nothing wrong.
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
    let started_a_line = home_x <= start_x && home_y == start_y + line_height;

    advanced && erased && started_a_line
}

/// The framebuffer is mapped write-combining, by the kernel's own tables.
///
/// Both halves again: the page attribute table must actually hold the
/// write-combining entry — a hypervisor may decline the write — and the
/// mapping must select it. Either alone means writes still cross the bus one
/// at a time (D-50).
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

/// The I/O APIC is routing the keyboard's line to the vector we asked for.
///
/// Read back out of the controller rather than remembered: the redirection
/// entry the hardware holds is what decides where an interrupt goes, and a
/// write that was silently dropped looks exactly like one that took.
fn the_keyboard_line_is_routed() -> bool {
    // The line the kernel routed, not the one a legacy IRQ number suggests:
    // firmware may map any IRQ to any global system interrupt.
    let gsi = dev::keyboard_gsi();

    dev::with_io_apic(|io_apic| {
        matches!(io_apic.is_masked(gsi), Ok(false))
            && matches!(io_apic.vector_of(gsi), Ok(vector::KEYBOARD))
    })
    .unwrap_or(false)
}

/// The controller refuses an interrupt it does not route.
///
/// Below its base the line arithmetic wraps, and in release it wrapped
/// silently into an unrelated register.
fn the_io_apic_refuses_a_line_it_does_not_own() -> bool {
    dev::with_io_apic(|io_apic| {
        let mut refused = 0;
        for gsi in [u32::MAX, dev::keyboard_gsi().wrapping_sub(1)] {
            if !io_apic.handles(gsi) && io_apic.vector_of(gsi).is_err() {
                refused += 1;
            }
        }
        // Only the first always exists: a base of zero has nothing below it.
        refused > 0
    })
    .unwrap_or(false)
}

/// Keystrokes reached the kernel as interrupts, not by polling.
///
/// Retires D-37. A zero means the line was routed on paper and delivered
/// nothing — what the old polled `await_key` hid.
fn keystrokes_arrive_as_interrupts() -> bool {
    input::is_interrupt_driven() && input::interrupt_count() > 0
}

/// A freed slot is reused rather than costing the heap another frame.
///
/// Counted, not compared by address: nothing obliges the compiler to keep two
/// allocator calls in source order when the values between them are dead, so
/// the address form of this check measures the optimiser (D-47).
fn the_heap_reuses_a_freed_slot() -> bool {
    // Warm the class, so what is measured below is a reuse and not the refill
    // that any first allocation of a class must pay for.
    drop(core::hint::black_box(Box::new(0u32)));

    let before = mm::heap::stats();
    let boxed = core::hint::black_box(Box::new(0x5A5A_5A5Au32));
    let taken = mm::heap::stats().live == before.live + 1;
    drop(boxed);
    let after = mm::heap::stats();

    taken && after.live == before.live && after.slab_frames == before.slab_frames
}

/// A refill happens with the heap's own lock dropped, and the accounting
/// balances across the window that opens.
///
/// D-85 moved the frame allocation out from under `HEAP`'s guard, which
/// deletes D-47's lock order and creates a window: between observing an empty
/// free list and re-acquiring, this thread can be preempted and another can
/// refill the same class. The cost is meant to be at most one spare slab
/// frame — never a lost frame, and never an unbalanced `live`.
///
/// The *other* half of the claim, that no slot is handed out twice, is checked
/// as arithmetic in `libs/slab-layout` instead. Breaking it here corrupts the
/// allocator every later check runs on, so the break is a general-protection
/// fault rather than a red verdict, and a break that crashes proves less than
/// one that fails (D-28, and Phase 4 learned it twice).
///
/// Preemption is on by this point and the count is chosen to force several
/// refills, so the window is crossed rather than assumed.
fn a_refill_costs_one_frame_and_balances() -> bool {
    /// Enough allocations of one class to span several slab frames.
    const TAKEN: usize = 512;

    // The vector's own buffer is allocated before the first snapshot and
    // released after the last, so all three see the same one allocation and
    // what moves between them is only the slots under test.
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

/// A heap allocation is writable, and holds what was written.
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

/// An allocation too large for any slot takes whole frames, and returns them.
fn a_large_allocation_takes_and_returns_frames() -> bool {
    const FRAMES: usize = 4;

    let before = mm::free_frames();
    let large: Vec<u8> = alloc::vec![0x42; FRAMES * 4096];
    let taken = mm::free_frames() <= before - FRAMES;
    let held = large.iter().all(|byte| *byte == 0x42);
    drop(large);

    taken && held && mm::free_frames() == before
}

/// The token the blocking calls below need, minted where they are made:
/// passing one down through every helper would be plumbing with nothing to
/// say.
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

/// Take every turn that is `mine`, yielding whenever it is not.
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

/// Yield until `done`, or until the budget runs out.
fn yields_until(done: impl Fn() -> bool) -> bool {
    (0..YIELD_BUDGET).any(|_| {
        thread::yield_now(&mut context());
        done()
    })
}

/// Two threads take turns in strict alternation, through yields alone.
///
/// Counted rather than inferred: a turn can only be taken when the other
/// thread has just handed it over, so the count is the number of handovers
/// that really happened. A yield that does not switch leaves it at zero.
/// Waits for both threads to exit before returning, so the slots are free
/// for whatever checks next.
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

/// Carry five live values across a yield, many times, and report whether
/// they came back.
///
/// Five scalars rather than an array because an array lives in memory, where
/// a switch cannot lose it. Scalars live across a call are what the compiler
/// puts in callee-saved registers — the ones the switch must preserve — and
/// the value it puts there is exactly what a dropped push would corrupt.
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

/// Callee-saved state survives a switch in both directions.
///
/// Two threads churn registers with different seeds, yielding to each other
/// between rounds, so each one's values cross the switch while the other is
/// overwriting every register it can. Both must come back intact.
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

/// Every thread stack is guarded below, read back from the live tables.
///
/// Structural, because the fault itself is terminal: the guard must be absent
/// from the tables, the stack present, writable and never executable — at its
/// first byte and its last, since a mapping can be right at one end and wrong
/// at the other.
fn thread_stacks_are_guarded() -> bool {
    let Some(layout) = thread::layout() else {
        return false;
    };
    (0..layout.slots()).all(|index| {
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
    })
}

/// What a thread found the interrupt flag to be on its first instruction:
/// zero before it ran, one for set, two for clear.
static FIRST_RUN_FLAG: AtomicUsize = AtomicUsize::new(0);

fn report_the_flag() {
    FIRST_RUN_FLAG.store(
        if interrupt::are_enabled() { 1 } else { 2 },
        Ordering::Release,
    );
}

/// A thread is accepting interrupts on its first instruction.
///
/// Not a formality. A thread's first run is the one moment its flag comes
/// from somewhere other than its own stack, and the obvious place to take it
/// from — the context doing the switch — is always wrong: a scheduler clears
/// the flag around every decision, so the switch never sees anything but a
/// clear one. A thread that starts uninterruptible can never be preempted and
/// never be woken; it can only yield. This kernel spawned such threads from
/// slice 1 until a sleeping check hung waiting for one of them.
///
/// Spawned from a yield, because that is the path that used to get it wrong;
/// the preemption path always handed over a set flag and would pass either
/// way.
fn a_new_thread_is_interruptible() -> bool {
    FIRST_RUN_FLAG.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("flag", report_the_flag).is_err() {
        return false;
    }

    let ran = yields_until(|| FIRST_RUN_FLAG.load(Ordering::Acquire) != 0);
    let exited = yields_until(|| thread::free_slots() == free);

    ran && exited && FIRST_RUN_FLAG.load(Ordering::Acquire) == 1
}

/// Where [`report_a_local_address`] leaves its probe, or zero until it runs.
static ALIGNMENT_PROBE: AtomicUsize = AtomicUsize::new(0);

/// Report the address of a sixteen-byte-aligned local. `black_box` because
/// the compiler believes the ABI and would otherwise answer at compile time.
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

/// A thread's stack is aligned as the ABI promises: the fabricated first
/// switch is the one place a stack pointer is invented rather than saved.
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

/// A slot whose thread has exited is free again, and can be spawned on.
///
/// Counted: the number of free slots must come back to where it began after
/// a thread runs to its end, and a second spawn must succeed on what the
/// first gave back.
fn an_exited_slot_is_reused() -> bool {
    let free = thread::free_slots();
    if thread::spawn("brief", leave_at_once).is_err() {
        return false;
    }
    let taken = thread::free_slots() == free - 1;
    let returned = yields_until(|| thread::free_slots() == free);

    let again = thread::spawn("brief again", leave_at_once).is_ok();
    let returned_again = yields_until(|| thread::free_slots() == free);

    taken && returned && again && returned_again
}

/// Set to tell every spinner to finish.
static STOP_SPINNING: AtomicUsize = AtomicUsize::new(0);

/// How many loops each spinner has run: one counter per weight class.
static LIGHT_SPINS: AtomicUsize = AtomicUsize::new(0);
static HEAVY_SPINS: AtomicUsize = AtomicUsize::new(0);

/// Loop without ever yielding, counting, until told to stop.
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

/// Run without yielding until `done` or `ms` have passed.
///
/// This thread never gives the CPU up on purpose, so anything that makes
/// progress meanwhile did so because the timer took it away.
///
/// Guarded by [`the_clock_is_running`], because a deadline read from a clock
/// that never advances is a hang — and the harness cannot tell a hang from a
/// crash, which is the difference between a red verdict and no verdict at
/// all. The first boot of the tickless kernel spent its sixty seconds here,
/// on a machine whose calibration had declined.
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

/// Whether the clock moves at all, within a bounded number of reads.
///
/// The precondition every timed wait here takes first, and the only one that
/// can be established without a working clock to time it against.
fn the_clock_is_running() -> bool {
    const ATTEMPTS: u32 = 1_000_000;

    let start = time::now_ns();
    time::is_calibrated() && spins_until(ATTEMPTS, || time::now_ns() > start)
}

/// A thread that never yields cannot keep the CPU from one that is waiting.
///
/// Preemption's whole claim. The boot thread spins without yielding and
/// watches for a spawned spinner — which also never yields — to make
/// progress. Either thread making progress while the other spins is the
/// timer switching them, and it has to happen in both directions for the
/// boot thread to be here to notice.
fn a_spinner_cannot_starve_another() -> bool {
    /// Generous beside a 30 ms slice, stingy beside a hung harness.
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

/// CPU time divides between spinners in proportion to their weights.
///
/// Two spinners, one at twice the other's weight, counting as fast as they
/// can while the boot thread spins alongside them. The ratio of their counts
/// is the ratio of their CPU time, quantised by the slice and jittered by the
/// hypervisor — so the bounds are coarse, and deliberately so.
fn runtime_divides_by_weight() -> bool {
    /// Long enough for a couple of hundred slices between them.
    const WINDOW_MS: u64 = 600;

    STOP_SPINNING.store(0, Ordering::Release);
    LIGHT_SPINS.store(0, Ordering::Release);
    HEAVY_SPINS.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn_weighted("light", light_spinner, eevdf::BASE_WEIGHT).is_err()
        || thread::spawn_weighted("heavy", heavy_spinner, 2 * eevdf::BASE_WEIGHT).is_err()
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
/// Recorded by the reader across its own call: a key that arrives is a key
/// that arrives whether the thread waited or spun. An earlier version of this
/// check lacked it, and a deliberately polling `read_key` passed.
static READER_BLOCKED: AtomicUsize = AtomicUsize::new(0);

/// Block on the keyboard, and record the first key that arrives.
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

/// A thread blocked on the keyboard is woken by the keystroke's interrupt.
///
/// D-49's lesson decides the shape: an outcome cannot tell a working wake
/// from a working poll. So three things at once — the reader is blocked,
/// counted by the state transition; the machine is quiet, because nothing is
/// armed for a thread with no deadline; and what it comes back with is the
/// key the harness pressed.
///
/// Set up here and collected after the injection, because the harness presses
/// on a marker this check is upstream of.
fn a_blocked_reader_is_ready_to_wake() -> bool {
    READER_SAW.store(0, Ordering::Release);
    READER_BLOCKED.store(0, Ordering::Release);

    let blocks = thread::blocks();
    if thread::spawn("reader", read_one_key).is_err() {
        return false;
    }
    // Long enough for the reader to be picked, run, and block; short beside
    // everything between here and the injection.
    let blocked = runs_until(50, || thread::blocks() > blocks);

    let quiet = time::expiries();
    runs_until(100, || false);
    let stayed_quiet = time::expiries() == quiet;

    blocked && stayed_quiet && input::has_reader() && READER_SAW.load(Ordering::Acquire) == 0
}

/// The reader woke, with the key the harness pressed and without polling for
/// it.
fn the_blocked_reader_woke_with_the_key() -> bool {
    let wanted = ps2_keys::Key::A as usize + 1;

    let woke = runs_until(1_000, || READER_SAW.load(Ordering::Acquire) != 0);
    woke && READER_SAW.load(Ordering::Acquire) == wanted
        && READER_BLOCKED.load(Ordering::Acquire) == 1
        && input::dropped_events() == 0
}

/// The polled keyboard fallback drains, sleeps and stops.
///
/// D-37's path had never run: both test machines have an I/O APIC. Started
/// here for a bounded number of rounds, on the code the shipped fallback runs.
/// It must make progress *and* leave, so the exit is half the claim.
fn the_polled_fallback_drains_and_sleeps() -> bool {
    const ROUNDS: usize = 4;

    let free = thread::free_slots();
    let before = input::poller_passes();
    if !input::start_bounded_poller(ROUNDS) {
        return false;
    }
    // Its own sleeps are what let this thread run, so waiting is a sleep too.
    thread::sleep_ms(&mut context(), (ROUNDS as u64 + 2) * 10);

    let passes = input::poller_passes() - before;
    let exited = yields_until(|| thread::free_slots() == free);

    passes == ROUNDS && exited
}

/// The idle thread runs when nothing else can, and never when something can.
///
/// Both halves, because either alone is satisfied by a bug: an idle thread
/// that never runs passes the second, one the policy schedules like any other
/// passes the first.
fn the_idle_thread_runs_only_when_nothing_else_can() -> bool {
    // Slept rather than spun: this thread is a normal one, so as long as it
    // is runnable the idle thread must not run — which is the very thing the
    // second half checks.
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

/// The console's longest operation does not stop the machine taking
/// interrupts.
///
/// D-34 left interrupt latency bounded by the longest critical section and
/// the bound unnamed. Named, it is a console scroll — tens of milliseconds,
/// logged beside this check — which is why D-63 stops interrupts waiting
/// behind it rather than stating how long they would.
///
/// So the property is not how long the scroll takes but that interrupts keep
/// arriving while it runs. A spinner makes the timer arm at slice intervals;
/// before D-63 the answer was none.
fn a_scroll_does_not_block_interrupts() -> bool {
    /// Scrolls to time, so one unlucky sample cannot decide it.
    const SAMPLES: usize = 8;
    /// A scroll spans this many slices at the very least, and each one owes
    /// an expiry while a second thread is waiting for the CPU.
    const EXPECTED_EXPIRIES: u64 = 2;

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

    // Filled to the bottom once, not once per sample: from here every line
    // scrolls, and refilling would have cost fifty scrolls a sample — which
    // it did, until a run spent most of the harness's minute doing it.
    for _ in 0..rows {
        dev::with_console(|console| console.write_line(""));
    }

    let mut worst = 0;
    let mut expiries = 0;
    for _ in 0..SAMPLES {
        dev::with_console(|console| {
            // Both readings inside the lock: sampled outside, a pending
            // expiry lands the instant the guard drops and the count moves
            // either way.
            let taken = time::expiries();
            let before = time::now_ns();
            console.write_line("");
            worst = worst.max(time::now_ns() - before);
            expiries += time::expiries() - taken;
        });
    }

    STOP_SPINNING.store(1, Ordering::Release);
    let exited = yields_until(|| thread::free_slots() == free);
    LONGEST_SCROLL.store(worst as usize, Ordering::Relaxed);

    exited && worst > 0 && expiries >= EXPECTED_EXPIRIES
}

/// What [`a_scroll_does_not_block_interrupts`] measured, so the verdict can
/// report the number as well as the judgement — it is the one D-63 is about.
static LONGEST_SCROLL: AtomicUsize = AtomicUsize::new(0);

/// Tally of what has been checked so far.
struct Report {
    passed: usize,
    failed: usize,
}

/// Bumped whenever the machine-to-harness verdict grammar changes.
const VERDICT_VERSION: usize = 1;

/// Every check in [`run`], including checks whose prerequisite is absent.
///
/// Keeping this independent from the tally makes deleting, skipping, or
/// accidentally bypassing a check turn the final verdict red.
const EXPECTED_CHECKS: usize = 104;

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
        }
    }
}

/// Run every check and hand the verdict to the harness.
pub(crate) fn run(serial: Result<(), UartError>) {
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
        "a claim covers the width it drives",
        a_claim_covers_the_width_it_drives(),
    );
    report.check("a held lock excludes another holder", lock_excludes());
    report.check("a one-shot latch admits one caller", latch_admits_one());

    report.check(
        "the frame allocator has usable memory",
        mm::free_frames() > 0,
    );
    report.check(
        "a frame is returned when its handle drops",
        frame_round_trips(),
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
        "the kernel image obeys write-xor-execute",
        the_image_obeys_write_xor_execute(),
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
        "a thread crosses into ring three and calls back",
        a_thread_crosses_into_ring_three_and_calls_back(),
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
        "the loader refuses what it may not map",
        the_loader_refuses_what_it_may_not_map(),
    );
    report.check(
        "a second thread crosses onto its own stack",
        a_second_thread_crosses_onto_its_own_stack(),
    );
    report.check(
        "the return to ring three clears buffers",
        the_return_to_ring_three_clears_buffers(),
    );
    report.check(
        "bytes cross the boundary only where they may",
        bytes_cross_the_boundary_only_where_they_may(),
    );
    report.check(
        "a signal wakes a thread waiting on a handle",
        a_signal_wakes_a_thread_waiting_on_a_handle(),
    );
    report.check(
        "a waiter rereads its condition before blocking",
        a_waiter_rereads_its_condition_before_blocking(),
    );
    report.check(
        "a closed handle names nothing",
        a_closed_handle_names_nothing(),
    );
    report.check(
        "a meaningless right is refused",
        a_meaningless_right_is_refused(),
    );
    report.check(
        "a handle does not cross tables",
        a_handle_does_not_cross_tables(),
    );
    report.check(
        "rights are checked and only narrow",
        rights_are_checked_and_only_narrow(),
    );
    report.check(
        "a table cannot exceed its quota",
        a_table_cannot_exceed_its_quota(),
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
        "a walk reports the permission the cpu grants",
        a_walk_reports_the_permission_the_cpu_grants(),
    );
    report.check(
        "a refused mapping leaves no trace",
        a_refused_mapping_leaves_no_trace(),
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
        "a stale device claim is refused",
        a_stale_device_claim_is_refused(),
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
        "the console scrolls without reading the device",
        dev::console_has_back_buffer(),
    );

    report.check("threads are available", thread::is_available());
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
    report.check("this cpu is accepting interrupts", interrupt::are_enabled());
    report.check(
        "a lock excludes interrupts and gives them back",
        a_lock_excludes_interrupts(),
    );

    // Two in a row, because one only proves the interrupt arrived. The APIC
    // will not deliver a second of the same priority class until the first is
    // dismissed, so the second is what proves the handler's end-of-interrupt.
    let before = trap::last_ping();
    apic::send_self_interrupt(vector::PING);
    report.check(
        "a self-interrupt is delivered",
        ping_arrives(before.count + 1),
    );

    let taken = trap::last_ping();
    apic::send_self_interrupt(vector::PING);
    report.check(
        "the interrupt is dismissed, so the next one arrives",
        ping_arrives(before.count + 2),
    );
    report.check(
        "the interrupted code was taking interrupts",
        ping_is_consistent(taken),
    );

    report.check(
        "the timer entry is one-shot and unmasked",
        timer_entry_is_programmed(),
    );
    report.check(
        "both calibrated rates are believable",
        rates_are_believable(),
    );

    let sweep = sweep_clock();
    report.check("the clock never runs backwards", sweep.monotonic);
    // A thousand times finer than the tick it replaced, and the smallest
    // step is the honest way to say so: the old clock could report a value
    // between two ticks, but never resolve two events inside one.
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

    // Everything above ran on a kernel with no idle thread, which is the
    // state the sleep checks needed: with nothing else runnable, a sleeper
    // halts on its own stack, and that branch has no other witness. From here
    // there is always something runnable, which is what the rest assumes.
    thread::start_idle();
    report.check("the idle thread is running", thread::idle_is_started());
    report.check(
        "the idle thread runs only when nothing else can",
        the_idle_thread_runs_only_when_nothing_else_can(),
    );
    report.check(
        "the polled keyboard fallback drains, sleeps and stops",
        the_polled_fallback_drains_and_sleeps(),
    );

    report.check("the keyboard controller came up", input::is_ready());
    report.check(
        "the controller raises its line exactly when routed, and never translates",
        keyboard_config_matches_its_routing(),
    );
    report.check(
        "an echo to the keyboard comes back",
        input::echo_roundtrips(),
    );
    report.check(
        "the keyboard is speaking scancode set two",
        input::scancode_set() == Some(2),
    );

    // Set up before the marker, collected after it: the reader has to be
    // blocked before the harness presses anything, or what wakes it is a
    // keystroke that was already waiting, which proves nothing.
    report.check(
        "a thread blocked on the keyboard is off the run queue, and quiet",
        a_blocked_reader_is_ready_to_wake(),
    );

    // The harness watches serial for this exact line and presses a key when
    // it appears — kept in step with READY_MARKER in `xtask/src/qemu.rs`.
    // Nothing may talk to the keyboard between the marker and the wait, or
    // the injected bytes could interleave with a command's replies.
    info!("selftest: awaiting injected keystroke");
    report.check(
        "an injected keystroke arrives, and its release follows",
        input::await_key(ps2_keys::Key::A),
    );

    report.check(
        "the framebuffer is mapped write-combining",
        the_framebuffer_is_write_combining(),
    );
    report.check(
        "the io apic routes the keyboard line to our vector",
        the_keyboard_line_is_routed(),
    );
    report.check(
        "the io apic refuses a line it does not own",
        the_io_apic_refuses_a_line_it_does_not_own(),
    );
    report.check(
        "keystrokes arrive as interrupts",
        keystrokes_arrive_as_interrupts(),
    );
    report.check(
        "the blocked reader woke with the injected key, without polling",
        the_blocked_reader_woke_with_the_key(),
    );

    // Before the editing check and not merely beside it: this is what makes
    // that one's cursor readings its own. Enter is the last key the harness
    // injects, and only a press echoes a character, so its release is the
    // moment the console stops moving underneath the measurement.
    report.check(
        "the injected editing keys arrive, ending with enter",
        input::await_key(ps2_keys::Key::Enter),
    );
    report.check(
        "backspace erases and enter starts a line",
        the_editing_keys_move_the_cursor(),
    );

    // Last of the timed checks, because it scrolls the console to the bottom
    // eight times and every check above wants its own output legible.
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

/// A row must be wide enough to hold the pixels the bootloader says are in it.
fn geometry_is_sane(geometry: FramebufferInfo) -> bool {
    geometry.width > 0
        && geometry.height > 0
        && geometry.bytes_per_pixel > 0
        && geometry.pitch >= geometry.width * geometry.bytes_per_pixel
}

/// The bootloader's masks must place red, green and blue in disjoint bits.
///
/// Overlapping masks would not fail loudly — text would simply render in the
/// wrong colour — so this is worth asserting against the format we were handed
/// rather than the one we expect.
fn channels_are_disjoint(geometry: FramebufferInfo) -> bool {
    let pack = |red, green, blue| geometry.format.pack(Rgb { red, green, blue });
    let (red, green, blue) = (
        pack(u8::MAX, 0, 0),
        pack(0, u8::MAX, 0),
        pack(0, 0, u8::MAX),
    );

    red != 0 && green != 0 && blue != 0 && red & green == 0 && green & blue == 0 && red & blue == 0
}

/// COM1 is already owned by the serial driver, so a second claim must be
/// refused — the whole block, and any single port inside it.
fn ports_are_exclusive() -> bool {
    port::claim_block::<8>(uart::COM1).is_none() && port::claim::<u8>(uart::COM1).is_none()
}

/// A claim covers the ports its access *reaches*, not just the one it names.
///
/// The two ports below COM1 are free, so a 32-bit claim starting there can
/// only be refused because its last two ports are the serial driver's. The
/// byte claim that follows proves both halves of that reading: the port really
/// was free, so the refusal was about the width — and the refused claim left
/// nothing behind on its way out. The third line closes the loop.
///
/// This spends port `0x3F6` for the life of the boot, which is why it belongs
/// to a self-test kernel and not to one that ships.
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

fn latch_admits_one() -> bool {
    let latch = Once::new();
    !latch.is_claimed() && latch.claim() && !latch.claim() && latch.is_claimed()
}

/// Every input line on both legacy controllers, disconnected.
const MASKED: u8 = 0xFF;

/// Holding a lock must exclude interrupts, releasing it must give them back,
/// and a nested lock must not give them back early — the last being the one
/// that a naive "enable on drop" gets wrong, and the one that would show up as
/// an interrupt arriving in the middle of a critical section months later.
///
/// Only meaningful with interrupts already on, which by this point they are.
fn a_lock_excludes_interrupts() -> bool {
    let outer = SpinLock::new(0u32);
    let inner = SpinLock::new(0u32);

    let before = interrupt::are_enabled();
    let outer_guard = outer.lock_leaf();
    let excluded = !interrupt::are_enabled();

    let inner_guard = inner.lock_leaf();
    drop(inner_guard);
    let still_excluded = !interrupt::are_enabled();

    drop(outer_guard);
    before && excluded && still_excluded && interrupt::are_enabled()
}

/// Spin until `done`, giving up after `attempts` rather than hanging the
/// harness on something that is never going to happen.
fn spins_until(attempts: u32, done: impl Fn() -> bool) -> bool {
    (0..attempts).any(|_| {
        spin_loop();
        done()
    })
}

/// Spin until the ping count reaches `target`.
///
/// A million reads, because it is measuring hardware that has already been
/// told to do something: a delivery this does not see is not slow, it is
/// absent.
fn ping_arrives(target: usize) -> bool {
    const ATTEMPTS: u32 = 1_000_000;

    spins_until(ATTEMPTS, || trap::last_ping().count >= target)
}

/// The hardware's own record of the timer: unmasked, one-shot, our vector.
///
/// One-shot is the half that is new. A periodic entry here would mean the
/// tick D-57 retired had come back, and every check below that counts
/// interrupts would be counting something the scheduler never asked for.
fn timer_entry_is_programmed() -> bool {
    let entry = apic::timer_entry();
    !entry.is_masked() && entry.is_one_shot() && entry.vector() == vector::TIMER
}

/// Both measured rates, against bounds restated here rather than shared with
/// the code under test. Calibration having declined reads as zero for either,
/// which fails.
fn rates_are_believable() -> bool {
    (1_000_000..=50_000_000_000).contains(&time::apic_hz())
        && (100_000_000..=100_000_000_000).contains(&time::counter_hz())
}

/// What a continuous read of the clock saw.
struct ClockSweep {
    monotonic: bool,
    /// The smallest step the clock was seen to take, or `None` if it never
    /// took one.
    finest_step: Option<u64>,
}

/// Read the clock as fast as it can be read, for a few milliseconds.
///
/// Monotonicity is no longer a formality: the clamp that made it true by
/// construction went with the tick (D-57). The first reading is taken while
/// holding a lock, where the retired clock could read backwards.
///
/// The smallest step is the grain, measured rather than inferred. Adjacent
/// readings that are equal say only that the reads outpaced the counter.
fn sweep_clock() -> ClockSweep {
    const ATTEMPTS: u32 = 2_000_000;
    const SPAN_MS: u64 = 3;

    let deadline = time::uptime_ms() + SPAN_MS;
    let mut previous = 0;
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

/// Timer expiries taken while nothing at all is waiting for one.
///
/// D-57's claim, and a number that would have been fifty before it. Nothing
/// is runnable but the boot thread and nothing sleeps, so there is nothing to
/// be interrupted *for* — none, not merely fewer.
fn an_idle_kernel_takes_no_timer_interrupts() -> bool {
    /// Five times the old tick's period, so a tick that survived would show
    /// up five times over.
    const QUIET_MS: u64 = 500;

    let before = time::expiries();
    runs_until(QUIET_MS, || false);
    time::expiries() == before
}

/// While threads compete for the CPU, the timer fires about once a slice.
///
/// The half the check above cannot make: a machine that takes no interrupts
/// when idle might have a timer that never works. Bounded both ways — too few
/// and nothing is preempted on the slice's schedule, too many and something
/// is arming for less than it was asked.
///
/// Also what proves the handler's end-of-interrupt, which the retired tick's
/// checks used to: the APIC delivers no second timer interrupt until the
/// first is dismissed.
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

    // A factor of four either way: the window is quantised by the slice and
    // jittered by a hypervisor, and the numbers this is separating — one an
    // interrupt per slice, the other none at all or thousands — are orders
    // of magnitude apart.
    let expected = WINDOW_MS / SLICE_MS;
    exited && spent >= expected / 4 && spent <= expected * 4
}

/// A thread that sleeps with nothing else to run comes back no earlier than
/// it asked, and the machine stays quiet while it waits.
///
/// The degenerate branch: nobody to hand the CPU to, so the thread halts on
/// its own stack until its timer fires. Neither half is evidence alone — a
/// sleeper that returned on time might have spun, a quiet machine might have
/// had nothing sleeping — and together they say the wait was the timer's.
fn a_sleeper_wakes_on_time_and_quietly() -> bool {
    /// Long enough that a surviving tick would be caught many times, short
    /// beside the harness's patience.
    const SLEEP_MS: u64 = 200;
    /// The wake itself, and room for the odd extra expiry around it.
    const EXPIRY_BUDGET: u64 = 4;
    /// How late the wake may be. Nothing else is runnable, so the thread
    /// resumes the moment its own timer fires — this is slack for the
    /// hypervisor, not for the policy.
    const LATE_MS: u64 = 50;

    let expiries = time::expiries();
    let before = time::uptime_ms();
    thread::sleep_ms(&mut context(), SLEEP_MS);
    let slept = time::uptime_ms() - before;
    let spent = time::expiries() - expiries;

    // Two-sided, and that is the point: the sleep was armed on the APIC
    // timer's clock and measured on the counter's, so a conversion wrong in
    // either direction shows up as a wait of the wrong length.
    (SLEEP_MS..=SLEEP_MS + LATE_MS).contains(&slept) && (1..=EXPIRY_BUDGET).contains(&spent)
}

/// A sleeping thread gives the CPU to a runnable one, and is put back on the
/// queue when its deadline passes.
///
/// The other branch: something else is runnable, so the sleeper really leaves
/// the CPU and really is woken. What the spinner counted meanwhile is the
/// proof the CPU went somewhere.
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

/// The configuration byte, against bits restated from the 8042 datasheet
/// rather than shared with the driver: both interrupt enables and the set 1
/// translation shim must be off, read back live from the hardware — the
/// self-test's own reset of this byte is the quirk the driver rewrites it
/// after, and this is the check that would catch the rewrite going missing.
fn keyboard_config_matches_its_routing() -> bool {
    const FIRST_IRQ: u8 = 1 << 0;
    const SECOND_IRQ: u8 = 1 << 1;
    const TRANSLATE: u8 = 1 << 6;

    let Some(byte) = input::controller_config() else {
        return false;
    };

    // Translation stays off in both worlds: the decoder reads set 2 raw, and
    // a controller quietly converting to set 1 is the classic way to get a
    // keyboard that types the wrong characters.
    let raw = byte & TRANSLATE == 0;
    // The second port is never enabled — nothing is on it.
    let second_silent = byte & SECOND_IRQ == 0;
    // The first port raises its line exactly when something routed it. Phase 2
    // required this to be off because nothing could route it (D-37); slice 5
    // turns it on, and the two must agree either way.
    let first_matches_routing = (byte & FIRST_IRQ != 0) == input::is_interrupt_driven();

    raw && second_silent && first_matches_routing
}

/// The frame a delivered interrupt saved, checked the way `frame_is_consistent`
/// checks a breakpoint's — except that this one can also assert something a
/// trap cannot: the interrupted code had the interrupt flag set, which is the
/// only direct evidence that this arrived through the flag rather than as a
/// software `int`.
fn ping_is_consistent(ping: Record) -> bool {
    /// `RFLAGS` bit 9.
    const INTERRUPT_FLAG: u64 = 1 << 9;

    ping.vector == u64::from(vector::PING)
        && ping.error_code == 0
        && ping.code_selector == u64::from(segment::KERNEL_CODE.bits())
        && ping.flags & INTERRUPT_FLAG != 0
}

/// The interrupt frame is the one place a struct definition and an assembly
/// listing have to agree, and the compiler cannot check it. If the pushes and
/// the fields drift apart, these values are what turn to nonsense.
///
/// `breakpoint` copies its interrupted `rsp` into `rax`, so equality distinguishes
/// the processor's saved stack pointer from whatever happened to be adjacent.
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

/// The saved frame ends exactly where the interrupted stack pointer was.
///
/// The compile-time assertions fix the *offsets* within `TrapFrame`; nothing
/// but a live trap can say the entry path pushes that many words. `rax` holds
/// the stack pointer as it was before the CPU pushed anything, so the frame
/// base plus the words the layout claims must land back on it.
fn frame_spans_the_saved_words(breakpoint: Record) -> bool {
    breakpoint
        .frame_address
        .checked_add(interrupt::SAVED_FRAME_BYTES)
        == Some(breakpoint.rax)
}
