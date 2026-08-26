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
use stack_layout::StackLayout;
use wost::arch::current::cpu::{self, Protections};
use wost::arch::current::paging::{self, ActivateError, AddressSpace, BootRegion, MapError};
use wost::arch::current::usercopy::{self, UserCopyError};
use wost::arch::current::{apic, interrupt, pic, port, segment, task, xstate};
use wost::boot::{self, FramebufferInfo, Rgb};
use wost::mm;
use wost::sync::{Once, SpinLock};
use x86_paging::{Flags, LARGE_PAGE_SIZE, PAGE_SIZE};
use x86_tables::exception;

use crate::arch::current::debug_exit;
use crate::arch::current::trap::{self, Record};
use crate::arch::current::uart::{self, UartError};
use crate::arch::current::vector;
use crate::devfs;
use crate::fs;
use crate::handle::{HandleError, HandleTable};
use crate::nvme;
use crate::object::{self, ObjectRef, ObjectType};
use crate::pci;
use crate::queue;
use crate::thread::ThreadContext;
use crate::timepage;
use crate::user;
use crate::vfs;
use crate::{dev, error, info, input, mem, thread, time, warn};

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
///
/// **Contiguity is half the name and was the half nothing checked.** The three
/// conjuncts below are all about the run's *description* — its address, its
/// count, the allocator's tally — and every one of them holds for an allocator
/// that hands back eight scattered frames and reports the first. The span
/// itself has to be written and read, and a frame taken from underneath it has
/// to land outside (D-124).
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

    // A different byte in each frame, written through the run's own span.
    let mut spans = true;
    for frame in 0..FRAMES {
        let at = frame * 4096;
        spans &= run.write_at(at, &[MARK ^ frame as u8]);
    }

    // Anything else the allocator hands out now must be outside the span —
    // otherwise the run is not the allocator's to have given whole.
    let start = run.address();
    let end = start + (FRAMES as u64 * 4096);
    let elsewhere = mm::allocate_zeroed()
        .map(|frame| {
            let address = frame.address();
            !(start..end).contains(&address)
        })
        .unwrap_or(false);

    // And the marks are still there, each in its own frame.
    let mut intact = true;
    for frame in 0..FRAMES {
        let mut byte = [0u8; 1];
        intact &= run.read_at(frame * 4096, &mut byte) && byte[0] == MARK ^ frame as u8;
    }
    drop(run);

    aligned && sized && spans && elsewhere && intact && mm::free_frames() == before
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

    // Queries rather than a closure over the map: `mm` takes its own lock for
    // each and hands back a number, so nothing of this function's runs under
    // it (D-121). The four calls take the lock four times, which costs
    // nothing at boot and is what makes the obligation checkable.
    let pick = mm::first_region;
    let addresses = (
        pick(|region| region.kind.is_allocatable(), WORD as u64),
        pick(
            |region| !region.kind.is_firmware_table() && !region.kind.is_allocatable(),
            WORD as u64,
        ),
        pick(|region| region.kind.is_firmware_table(), WORD as u64),
        // Inside the address space, so only the map can refuse it — an address
        // at the top would be caught by an overflow guard instead.
        mm::past_every_region(),
    );

    let (Some(usable), Some(unreachable), Some(readable), Some(undescribed)) = addresses else {
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

    // Safe to assign: the slot is empty, so nothing is dropped under the guard.
    *REREAD_EVENT.lock_leaf() = Some(Arc::clone(&event));
    REREAD_RETURNED.store(0, Ordering::Release);
    let free_before = thread::free_slots();
    if thread::spawn("event-rereader", wait_on_a_signalled_event).is_err() {
        return false;
    }

    let came_back = runs_until(200, || REREAD_RETURNED.load(Ordering::Acquire) == 1);
    // `>=`, because a user program retiring elsewhere frees a slot too: the
    // property is that *this* thread's slot came back, and another coming back
    // with it does not falsify it (D-117).
    let settled = runs_until(200, || thread::free_slots() >= free_before);
    // Taken out rather than assigned over: assignment drops the previous
    // `Arc` *inside* the critical section, and a last reference would reach
    // the heap and the frame allocator under a leaf guard — the shape
    // `handle.rs` goes to explicit trouble to avoid (D-105).
    let released = REREAD_EVENT.lock_leaf().take();
    drop(released);

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
    if user::run().is_err() {
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
    if user::run_faulting().is_err() {
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
    // Its stack pointer is in user memory, not the kernel's. (The record
    // carries no instruction pointer; an earlier comment said it did.)
    let in_user_memory = fault.stack_pointer < usercopy::USER_END;
    // The thread was retired rather than the machine stopped — and the machine
    // is plainly still here, since this line runs.
    let retired = runs_until(500, || thread::retirements() > retirements_before);

    from_user && it_was_the_ud2 && in_user_memory && retired
}

/// A program aiming `RIP` into the kernel's user-copy window is retired, not
/// resumed.
///
/// The recovery arm runs **before** the ring-3 test — it has to, because the
/// fault it exists for is taken in ring 0 with the kernel dereferencing — and
/// until D-126 it never looked at the saved `CS`. So a ring-3 thread that
/// jumped into `[wost_copy_fault_start, wost_copy_fault_end)` had its
/// instruction-fetch fault "recovered": `resume_at` rewrote `RIP` to the
/// fixup with `CS` still ring 3, it faulted again there, and only then was
/// retired. Not an escalation and not a halt, but a program could drive the
/// recovery counter at will, and the arm's own comment claimed a privilege
/// nothing enforced.
///
/// Driven by a real ring-3 thread rather than a synthesised frame, because a
/// synthesised one would be the kernel asserting about its own arithmetic. The
/// address is asked of `wost` rather than written down: a hardcoded one would
/// check a number, not the window.
fn a_program_cannot_aim_at_the_copy_window() -> bool {
    let recoveries_before = trap::user_copy_recoveries();
    let faults_before = trap::last_user_fault().count;
    let retirements_before = thread::retirements();

    if user::run_into_the_copy_window().is_err() {
        return false;
    }
    if !runs_until(2000, || trap::last_user_fault().count > faults_before) {
        return false;
    }

    let fault = trap::last_user_fault();
    let (start, end) = usercopy::fault_window();

    // From ring 3, which is what makes this the case the arm must refuse.
    let from_user = fault.code_selector & 0b11 == 3;
    // And the thread went, rather than being resumed at the fixup.
    let retired = runs_until(500, || thread::retirements() > retirements_before);
    // The counter did not move: nothing was recovered.
    let not_recovered = trap::user_copy_recoveries() == recoveries_before;
    // And the window is a real range, so the conjunct above is about
    // something — a `fault_window` returning an empty range would make the
    // whole check pass with the arm deleted.
    let real_window = start < end;

    from_user && retired && not_recovered && real_window
}

/// Two programs at the same address see different memory.
///
/// The isolation claim, stated the strongest way the machine allows: two
/// spaces, one virtual address, two frames. Until D-106 a second program meant
/// a second *pair of addresses* in the one space the kernel ran on, which
/// proved that `RSP0` follows the thread and nothing about isolation.
///
/// Checked from the kernel rather than by running both, because the point is
/// what the tables say and a program cannot report on a space it cannot reach.
/// The frames are written through the direct map, which is the one view that
/// sees both.
fn two_spaces_map_one_address_to_different_memory() -> bool {
    /// A user address neither space has anything else at.
    const SHARED: u64 = user::CODE + 0x10_0000;

    let (Ok(mut first), Ok(mut second)) = (AddressSpace::for_user(), AddressSpace::for_user())
    else {
        return false;
    };
    // Different roots to begin with, or nothing below means anything.
    if first.root() == second.root() {
        return false;
    }

    let writable = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let (Some(mut one), Some(mut two)) =
        (mm::allocate_contiguous(1, 1), mm::allocate_contiguous(1, 1))
    else {
        return false;
    };
    if !one.zero() || !two.zero() {
        return false;
    }
    // Distinct bytes, written through the direct map before either is mapped.
    if !one.write_at(0, &[0xA5]) || !two.write_at(0, &[0x5A]) {
        return false;
    }
    let (one_at, two_at) = (one.address(), two.address());
    if one_at == two_at {
        return false;
    }
    if first.map_user(SHARED, one.into(), writable).is_err()
        || second.map_user(SHARED, two.into(), writable).is_err()
    {
        return false;
    }

    // One address, two answers.
    let (Some((first_frame, first_flags)), Some((second_frame, second_flags))) =
        (first.lookup(SHARED), second.lookup(SHARED))
    else {
        return false;
    };
    // `first_frame != second_frame` follows from the two equalities and the
    // refusal above, so it is not asserted again here (D-115).
    let separate = first_frame == one_at && second_frame == two_at;
    let both_user = first_flags.contains(Flags::USER) && second_flags.contains(Flags::USER);

    // The kernel's half is the same in both, which is what makes a switch
    // between them safe at all (D-104).
    let text = boot::sections().text.start;
    let shared_kernel = first.lookup(text) == second.lookup(text) && first.lookup(text).is_some();

    separate && both_user && shared_kernel
}

/// What one thread has in a vector register, another thread cannot see, and
/// the first still has it afterwards.
///
/// Eager extended-state save and restore (D-82, D-108). Lazy is CVE-2018-3665
/// and was never on the table; what was owed was doing it at all, and D-99
/// made the gap reachable — two threads can be in ring 3, so two threads can
/// have floating-point state to lose.
///
/// `xmm0` is the probe because it is the one vector register anything in this
/// kernel touches: the build is `+soft-float`, so no compiled code holds
/// anything there, which is also why the save can sit before the switch rather
/// than inside it.
fn a_vector_register_survives_a_switch() -> bool {
    /// What this thread puts there.
    const MINE: u64 = 0xA5A5_1234_5678_A5A5;
    /// What the other thread puts there.
    const THEIRS: u64 = 0x5A5A_8765_4321_5A5A;

    if !xstate::is_enabled() {
        // A processor with no `XSAVE` has nothing to save, and reports so.
        return xstate::bytes() == 0 && xstate::extended_control() == 0;
    }

    // The register the write went to is the register the read comes from,
    // before anything else runs — a conjunct that would hold with the whole
    // save path deleted, and named so it is not mistaken for the property.
    if !xstate::write_vector_probe(MINE) || xstate::read_vector_probe() != Some(MINE) {
        return false;
    }

    VECTOR_INTRUDER_RAN.store(0, Ordering::Release);
    if thread::spawn("vector-intruder", clobber_a_vector_register).is_err() {
        return false;
    }
    if !runs_until(500, || VECTOR_INTRUDER_RAN.load(Ordering::Acquire) == 1) {
        return false;
    }

    // The other thread wrote its own value and this one is unchanged, which is
    // only true if the switch away saved and the switch back restored.
    let kept = xstate::read_vector_probe() == Some(MINE);
    let intruded = VECTOR_INTRUDER_SAW.load(Ordering::Acquire) == THEIRS as usize;

    // And `XCR0` holds what boot asked for.
    // Read back from `XCR0` through `XGETBV`, which is guarded on `CR4` and
    // not on a Rust flag — the earlier version's `bytes() >= 576` restated
    // `init`'s own refusal, and its zero-check read an early return rather
    // than the register (D-115).
    let mask = xstate::extended_control();
    let masked = mask & 0b111 == 0b111 && xstate::bytes() <= 1024;

    kept && intruded && masked
}

/// Set by the intruding thread once it has run.
static VECTOR_INTRUDER_RAN: AtomicUsize = AtomicUsize::new(0);

/// What the intruding thread read back from its own register.
static VECTOR_INTRUDER_SAW: AtomicUsize = AtomicUsize::new(0);

/// Put a different value in `xmm0` and read it back, on another thread.
fn clobber_a_vector_register() {
    const THEIRS: u64 = 0x5A5A_8765_4321_5A5A;
    if !xstate::write_vector_probe(THEIRS) {
        return;
    }
    let mut context = context();
    thread::yield_now(&mut context);
    let seen = xstate::read_vector_probe().unwrap_or(0) as usize;
    VECTOR_INTRUDER_SAW.store(seen, Ordering::Release);
    VECTOR_INTRUDER_RAN.store(1, Ordering::Release);
}

/// A run of frames the kernel owns cannot be mapped where ring 3 can read it.
///
/// D-44's untyped split, with its first real caller (D-107). The frames come
/// from the allocator carrying whatever the kernel last put in them, so
/// publishing one to ring 3 is a disclosure — and the type is what says a run
/// has already been given away. `map_owned` refuses the `USER` flag outright;
/// `map_user` takes the run that has been converted, and adds `USER` itself so
/// a caller cannot forget it.
///
/// The road back zeroes. A frame that held a program's stack and came back to
/// the kernel still holding it is how one process reads another's memory, so
/// `reclaim` clears before it returns a `Segment` and there is no other door.
fn kernel_memory_cannot_be_published_to_ring_three() -> bool {
    /// A user address nothing else uses.
    const PROBE: u64 = user::STACK + 0x40_0000;

    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let Some(mut run) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !run.zero() {
        return false;
    }

    // The kernel's own bytes, of the kind a disclosure would hand over.
    let secret = [0xDEu8; 8];
    if !run.write_at(0, &secret) {
        return false;
    }

    // `map_owned` will not publish them, whatever the caller asks for.
    let refused = matches!(
        space.map_owned(PROBE, run, Flags::USER.union(Flags::WRITABLE)),
        Err(MapError::NotYours(PROBE))
    );
    if !refused {
        return false;
    }

    // Converting is the caller noticing. The run went into the call above and
    // came back to the allocator with it, so this is a fresh one.
    let Some(mut giving) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !giving.zero() || !giving.write_at(0, &secret) {
        return false;
    }
    let untyped: mm::USegment = giving.into();
    let address = untyped.address();

    // Reclaiming clears it, so what the kernel gets back holds nothing.
    let Some(back) = untyped.reclaim() else {
        return false;
    };
    // Read back, because "the call returned" is not "the bytes are gone" —
    // and a `reclaim` that skipped the zeroing passed a version of this check
    // that only asked whether the frame was still reachable (D-107).
    let mut recovered = [0xFFu8; 8];
    let cleared =
        back.address() == address && back.read_at(0, &mut recovered) && recovered == [0u8; 8];
    drop(back);

    // And `map_user` adds `USER` even where the caller did not.
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

    refused && cleared && user_accessible
}

/// Only a space that agrees with the kernel about the higher half is
/// installable, and a user space cannot make one that does not.
///
/// A `CR3` naming tables that do not map the kernel's code is a triple fault
/// at the next instruction, so `bind` compares every top-level entry of the
/// higher half rather than sampling addresses (D-106). The comparison itself
/// cannot be made to fail through the public API, which is the point: the
/// the doors into the higher half — `map_boot`, `map_owned`, `unmap` and
/// `map_device` —
/// all refuse a space that shares the kernel's. `map_device` was the one that
/// did not, and it walked into the kernel's own device tables.
fn only_a_space_that_matches_the_kernel_can_be_installed() -> bool {
    // A space that never shared the kernel's half is refused outright: it maps
    // no kernel code, so installing it is a triple fault.
    let Ok(mut bare) = AddressSpace::new() else {
        return false;
    };
    let bare_refused = matches!(bare.bind(), Err(ActivateError::NotAUserSpace));

    // One that did share it binds.
    let Ok(mut shared) = AddressSpace::for_user() else {
        return false;
    };
    let binds = shared.bind().is_ok();

    // And no door into the higher half is open to it. The device window is
    // where the kernel's own registers live, so a mapping there is an edit of
    // the tables every space shares.
    let Ok(mut probe) = AddressSpace::for_user() else {
        return false;
    };
    let device_refused = matches!(
        probe.map_device(0xfee0_0000, PAGE_SIZE),
        Err(MapError::NotYours(_))
    );

    bare_refused && binds && device_refused
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
///
/// Loaded into a throwaway space rather than the running one (D-106), so a
/// refusal that half-loaded leaves its mappings somewhere nothing else walks.
/// The space itself is abandoned, which costs table frames — bounded by the
/// number of refusals here, and the price of not editing the live tables.
fn the_loader_refuses_what_it_may_not_map() -> bool {
    let Some(original) = user::image() else {
        return false;
    };
    let scratch = || AddressSpace::for_user().expect("a space to load into");
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
        user::load_image(&mut scratch(), &write_execute, 0),
        Err(user::LoadError::WriteExecute(_))
    );

    // Contents past the end of the image, and *only* that: `p_offset` moved
    // out to the end with the sizes left alone, so `is_oversized` is satisfied
    // and `contents_fit` is the one refusal that can fire. The earlier version
    // set `p_filesz = 0x1000`, which tripped both — and deleting the
    // `contents_fit` bound, the only thing keeping a slice index inside the
    // image, left the gate green (D-105).
    let overrun = doctor(OFFSET, original.len() as u64);
    let overrun_refused = matches!(
        user::load_image(&mut scratch(), &overrun, 0),
        Err(user::LoadError::Truncated(_))
    );

    // More file than memory to put it in, and only that.
    let mut oversized = doctor(FILE_SIZE, 8);
    oversized[MEMORY_SIZE..MEMORY_SIZE + 8].copy_from_slice(&4u64.to_le_bytes());
    let oversized_refused = matches!(
        user::load_image(&mut scratch(), &oversized, 0),
        Err(user::LoadError::Malformed(_))
    );

    // Placed in the kernel's half.
    let intruding = doctor(VADDR, 0xffff_8000_0000_0000);
    let intruding_refused = matches!(
        user::load_image(&mut scratch(), &intruding, 0),
        Err(user::LoadError::NotUserMemory(_))
    );

    // And something that is not an ELF at all.
    let not_an_elf = [0u8; 64];
    let refused_outright = matches!(
        user::load_image(&mut scratch(), &not_an_elf, 0),
        Err(user::LoadError::NotAnElf(_))
    );

    wx_refused && overrun_refused && oversized_refused && intruding_refused && refused_outright
}

/// A second thread crosses too, and each trap lands on its own kernel stack.
///
/// The limitation D-96 wrote down and D-99 lifted. `RSP0` used to be set once,
/// to a stack of its own, so a second thread in ring 3 would have taken its
/// trap on a stack the first was standing on. It now moves with the thread, in
/// the one place every switch passes through.
///
/// The evidence is the stack the trap actually landed on — `frame_address`,
/// recorded by the entry path out of the interrupted stack pointer, and so a
/// property of *that* crossing. `segment::kernel_stack()` reads the live
/// `RSP0`, which is global CPU state that both reads see identically long
/// after the fact; comparing it to itself could not fail, and did not, for a
/// whole phase (D-105).
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
    let through_the_gate = second.vector as u8 == vector::SYSCALL;
    // At its own user addresses, not the first program's.
    let elsewhere =
        second.stack_pointer != first.stack_pointer && second.stack_pointer < usercopy::USER_END;
    // And `RSP0` followed the thread: the stack this crossing landed on is the
    // arena slot belonging to the thread that made it.
    //
    // **Not "a different stack from last time".** That was the property until
    // D-117, and it held because a retired thread's slot was never reused —
    // so two crossings could not share a stack even if `RSP0` never moved.
    // Slots are reused now, and two threads that never coexist *correctly*
    // share one. What identifies a caller is its slot, which the entry path
    // records.
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
/// changes. What it shows is that the instruction **executed** — the tally
/// beside the `verw`, not the byte that arms it. Reading the arming was the
/// earlier version, and deleting the `verw` outright left it green, which is
/// exactly how D-101 found the flush had shipped unexecuted (D-105).
fn the_return_to_ring_three_clears_buffers() -> bool {
    let armed = interrupt::buffer_clear_is_armed();
    let executed = interrupt::buffer_clears();
    if !armed || executed == 0 {
        return false;
    }
    // And it executes once per crossing rather than once at boot: every
    // syscall seen came from a thread that was let into ring 3 through this
    // branch, so the tally cannot lag the crossings.
    executed >= trap::last_syscall().count as u64
}

/// A user copy that faults resumes and reports, instead of stopping the
/// machine.
///
/// D-97 reasons from the right premise — *the program is the untrusted party*
/// — and until D-120 the copy path did not act on it. A fault inside the
/// window was terminal, so a program that arranged one stopped the machine.
///
/// **Driven through the unwalked copy, because the walked one cannot reach
/// it.** `admits` refuses an unmapped address before a byte moves, so a check
/// built on the walked form tests the refusal it already has and the recovery
/// not at all — it would pass with the fixup deleted, which is the shape this
/// project keeps finding in its own checks (D-105 rule 2). The unwalked form
/// is safe *because* of the fixup, so pointing it at nothing is exactly the
/// mechanism under test.
fn a_faulting_user_copy_resumes_and_reports() -> bool {
    /// User memory the running space certainly does not map.
    const NOWHERE: u64 = 0x0000_6000_0000_0000;

    let recoveries_before = trap::user_copy_recoveries();
    let mut read = [0xA5u8; 8];

    // Nothing is there. The copy faults on its first byte, resumes at the
    // fixup, and says how much it did not move.
    let vanished = usercopy::copy_from_user_unwalked(NOWHERE, &mut read);
    let reported = vanished == Err(UserCopyError::Vanished(read.len()));

    // The machine is still here, and the fault was counted rather than
    // inferred: this line running at all is the weaker half, and the count is
    // the half that could not be true by accident.
    let recovered = trap::user_copy_recoveries() == recoveries_before + 1;

    // Ring 3's half only: the kernel's is refused before the window opens, so
    // the unwalked form cannot be turned into an arbitrary kernel read.
    let kernel_refused = matches!(
        usercopy::copy_from_user_unwalked(boot::sections().text.start, &mut read),
        Err(UserCopyError::NotUserMemory(_))
    );

    // And a page that *is* there copies, so the fixup has not simply broken
    // the copy for everything.
    let Some((code, _)) = boundary_probe() else {
        return false;
    };
    let mut program = [0u8; user::PROGRAM.len()];
    let works =
        usercopy::copy_from_user_unwalked(code, &mut program).is_ok() && program == user::PROGRAM;

    reported && recovered && kernel_refused && works
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
/// A read-only executable page and a writable page, user-accessible in the
/// *running* space.
///
/// `copy_from_user` and `copy_to_user` are about the tables the CPU is
/// walking, and since D-106 those are the kernel's for every kernel thread —
/// a program's pages live in a space this thread cannot reach. So the checks
/// about the boundary build their own pages, which is also the honest
/// arrangement: what they test is the copy, not the loader.
///
/// Mapped once and remembered: a second `map_user` at the same address is a
/// `Conflict`, and every caller wants the same two pages.
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
    mem::with_active_space(&mut context, |space| {
        space.map_user(CODE, code.into(), Flags::NONE)
    })?
    .ok()?;
    mem::with_active_space(&mut context, |space| {
        space.map_user(
            STACK,
            stack.into(),
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )
    })?
    .ok()?;

    *PROBE.lock_leaf() = Some((CODE, STACK));
    Some((CODE, STACK))
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

    // A round trip through the writable page.
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

    program_matches
        && round_trip
        && kernel_refused
        && straddling_refused
        && wrap_refused
        && hole_refused
        && read_only_refused
}

/// A ring that has reported everything it can admits nothing more until
/// userspace reaps, and then admits again.
///
/// Backpressure, closed end to end. Admission reserves a completion slot, a
/// completion fills it, and only userspace moving its own head gives the room
/// back — which is the whole reason the completion queue's head is a number
/// the *program* writes. A kernel that reclaimed the slot itself would be
/// reporting into entries a program had not read (D-79, D-113).
///
/// The head is read from the snapshot rather than the page for the same reason
/// everything else here is: once, and never re-read.
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
        let served = queue.serve(&a_table());
        if served == 0 {
            break;
        }
        posted += served as u32;
    }

    // Nothing has been reaped, so nothing more can be admitted: every slot is
    // either spoken for or already holds a completion.
    let refuses = queue.reserve_for_test(1) == 0 && queue.is_consistent();

    // Userspace reaps *some* of them, and the kernel gives back exactly that
    // much. Reaping everything would be the kernel deciding a program had read
    // entries it had not, which is the failure this bound exists for — and a
    // check that reaped the lot could not tell the two apart.
    let head_before = queue.completion_head();
    let partial = posted / 2;
    queue.publish_completion_head_for_test(partial);
    let reaped = queue.reap_up_to(queue.completion_head());
    let followed = head_before == 0 && reaped == partial as usize;

    // Room for exactly what was reaped, and no more.
    let resumes = queue.reserve_for_test(partial as usize) == partial as usize
        && queue.reserve_for_test(1) == 0
        && queue.is_consistent();

    // And a program that **lies** about how far it has read gets nothing for
    // it. The head is ring 3's number, so "reap up to here" is a claim and not
    // an instruction: honouring it would reap completions nobody read, which
    // is the one thing the completion queue exists to prevent. An honest head
    // cannot show this — it is already inside the bound — so the earlier
    // version of this check passed with the bound deleted (D-115).
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
    if liar.serve(&a_table()) != 2 {
        return false;
    }
    // Two posted; the program claims to have read four billion.
    liar.publish_completion_head_for_test(u32::MAX);
    let lied = liar.reap_up_to(liar.completion_head()) == 2 && liar.is_consistent();

    posted > 0 && partial > 0 && refuses && followed && resumes && lied
}

/// A ring says what shape it is, and its pages are the process's to read and
/// not to free.
///
/// The ABI's first field is a version, which is IoRing's improvement over
/// io_uring: a ring that cannot say what shape it is cannot be extended
/// without guessing (D-79). The pages are **kernel-allocated and kernel-kept**
/// — `map_shared` writes no `OWNED` bit, so a program that unmapped them could
/// not free the memory the kernel is about to write a completion into (D-113).
fn a_ring_announces_its_shape_and_keeps_its_pages() -> bool {
    /// A user address nothing else uses.
    const AT: u64 = user::STACK + 0x60_0000;

    let Some(mut queue) = queue::Queue::new() else {
        return false;
    };
    // The kernel's own shape. `Queue::new` returning `Some` already required
    // both to be powers of two and the completion queue to be twice the
    // submission one — `Capacity::new` and `Queues::new` refuse otherwise — so
    // asserting those here would be asserting the constructor's preconditions
    // back at it (D-115).
    let (version, submissions, completions) = queue.shape();

    // What is *not* decided is that the header page was published to match,
    // and that is the whole of the ABI a program reads. Deleting either write
    // in `Queue::new` turns this red.
    let announced = queue.published_shape() == (version, submissions, completions)
        && version == ring_proto::VERSION;

    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    if queue.install(&mut space, AT).is_err() {
        return false;
    }

    // Ring 3 can reach them.
    let reachable = space
        .lookup(AT)
        .is_some_and(|(_, flags)| flags.contains(Flags::USER) && flags.contains(Flags::WRITABLE));

    // And cannot give them away. `unmap` reads the owned bit before minting a
    // `Segment`, and these frames were never the space's to own.
    let unowned = matches!(space.unmap(AT, PAGE_SIZE), Err(MapError::NotOwned(AT)));

    announced && reachable && unowned
}

/// A table with nothing in it, for the ring checks that produce no handles.
///
/// `serve` needs one because a submission can now install a handle. Every
/// check that predates that produces none, so an empty table is exactly as
/// much authority as those operations need — which is the point of passing it
/// rather than reaching for the caller's.
#[cfg(feature = "selftest")]
fn a_table() -> alloc::sync::Arc<crate::handle::HandleTable> {
    alloc::sync::Arc::new(crate::handle::HandleTable::new(8))
}

/// A word that does not fit is refused, never truncated.
///
/// The phase-close audit's finding, and it is one defect in three places
/// (D-174). Every one of these words is written by ring 3 into a page ring 3
/// owns, and every one was reaching the kernel through an `as` cast:
///
/// - a **rights word** whose high half was dropped, so
///   `0xFFFF_FFFF_0000_0003` was read as `DUPLICATE | TRANSFER` rather than
///   refused — which is the whole reason `Rights::from_bits` returns an
///   `Option`;
/// - a **file offset** wrapped into the file rather than past it, so a program
///   asking for byte four billion and one would be handed byte one;
/// - a **cancellation token** truncated, so `0x1_0000_DEAD` would end the
///   operation tagged `0xDEAD` — a program ending an operation it did not
///   name.
///
/// None of them is a crash. All three are a program being answered about
/// something other than what it asked for, which is the worst shape a boundary
/// can fail in.
fn a_word_that_does_not_fit_is_refused_rather_than_truncated() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some((held, _far)) = crate::object::channel() else {
        return false;
    };
    let table = alloc::sync::Arc::new(crate::handle::HandleTable::new(8));
    let full = Rights::UNIVERSAL.union(Rights::READ);
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
        queue::Queue::about_at(queue::Opcode::Read, 0xC003, file, 1u64 << 32, 512, 16),
    );
    queue.publish_tail_for_test(5);
    if queue.serve(&table) != 1 {
        return false;
    }
    let past = queue.read_completion(3);

    let mut untouched = [0u8; 16];
    if !queue.read_data_for_test(512, &mut untouched) {
        return false;
    }

    widened.status == queue::STATUS_DENIED
        // The timer is still in flight: the cancellation named no operation,
        // which is what a token that does not fit names.
        && cancelled.status == queue::STATUS_NOT_FOUND
        && queue.in_flight() == 1
        // And the read answered "nothing", rather than the file's first bytes.
        && past.status == queue::STATUS_OK
        && past.value == 0
        && untouched == [0u8; 16]
}

/// Open the corpus's small file into `table`, submitting at `slot`.
///
/// The same four operations as [`open_the_small_file`], against a table the
/// caller already owns.
fn open_the_small_file_in(
    queue: &queue::Queue,
    table: &alloc::sync::Arc<crate::handle::HandleTable>,
    slot: usize,
) -> Option<u32> {
    let root = vfs::root()?;
    let object = crate::object::directory(root)?;
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

/// A completion carries a **file** handle, and the rights come from the
/// directory that named it.
///
/// The operation this whole phase was for. A program holds a directory and
/// nothing else; it names something inside; and what comes back is authority
/// over that thing — reserved at submission (D-151), attenuated on arrival
/// (D-110), and derived from what the directory held rather than asked for
/// (D-173).
fn a_completion_carries_a_file_handle() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some(root) = vfs::root() else {
        return false;
    };
    let Some(object) = crate::object::directory(root) else {
        return false;
    };

    let table = alloc::sync::Arc::new(crate::handle::HandleTable::new(8));
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

/// A read is a submission, and its offset is per operation.
///
/// `012` §6 and D-152: no seek, no tell, no cursor on anything. Two reads of
/// one handle at different offsets answer differently, and neither moves
/// anything — which is what makes two handles to one file have no observable
/// coupling.
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
    if queue.serve(&table) != 2 {
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

/// A ring operation cannot reach past the rights it was given, or past the
/// pages it shares.
///
/// Four refusals, each from a different place, and every one of them at
/// **submission**: a directory without `TRAVERSE` cannot open, a file without
/// `READ` cannot be read, a path that escapes is malformed rather than denied,
/// and a buffer outside the data area is refused before anything is written.
fn a_ring_operation_cannot_reach_past_its_rights_or_its_pages() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some(root) = vfs::root() else {
        return false;
    };
    let table = alloc::sync::Arc::new(crate::handle::HandleTable::new(8));

    // A directory held without TRAVERSE: it can be listed and not walked.
    let Some(object) = crate::object::directory(root) else {
        return false;
    };
    let Ok(unwalkable) = table.insert(object, Rights::UNIVERSAL.union(Rights::ENUMERATE)) else {
        return false;
    };
    // And one that can be walked, for the paths that are about the path.
    let Some(object) = crate::object::directory(root) else {
        return false;
    };
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
    let Some(object) = crate::object::file(file) else {
        return false;
    };
    let Ok(unreadable) = table.insert(object, Rights::UNIVERSAL) else {
        return false;
    };
    // And the same file held *with* `READ`, so a refusal about the buffer is
    // reached at all: a handle that is denied at the right never gets as far
    // as the bound, and a check built only from denied handles cannot see
    // whether the bound comes before the read (D-173).
    let Some(object) = crate::object::file(file) else {
        return false;
    };
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

    // A read of a file held without `READ`, and a read into a buffer the
    // shared pages cannot hold. **The device must not have been asked** for
    // either: a refusal that happened after the read reached the controller
    // looks exactly like one that happened before, unless something counts —
    // and `-ENOBUFS` after the work is done is the shape `012` §8 warns about
    // (D-173).
    let before = nvme::with_controller(|controller, _| controller.commands());
    queue.post_submission_for_test(
        4,
        queue::Queue::about_at(queue::Opcode::Read, 0xD5, unreadable, 0, 0, 16),
    );
    queue.post_submission_for_test(
        5,
        queue::Queue::about_at(queue::Opcode::Read, 0xD6, unreadable, 0, 4090, 16),
    );
    // The one that reaches the bound: readable, and a buffer that runs off the
    // end of the shared pages. **The device must still not be asked** — a
    // bound checked after the read is `-ENOBUFS` with the work already done.
    queue.post_submission_for_test(
        6,
        queue::Queue::about_at(queue::Opcode::Read, 0xD7, readable, 0, 4090, 16),
    );
    queue.publish_tail_for_test(7);
    let served = queue.serve(&table) == 3;
    let after = nvme::with_controller(|controller, _| controller.commands());

    let denied = queue.read_completion(4).status == queue::STATUS_DENIED
        && queue.read_completion(5).status == queue::STATUS_DENIED
        && queue.read_completion(6).status == queue::STATUS_BAD_BUFFER;
    let untouched = before == after;

    // Nothing was promised and kept: every refusal above gave its slot back.
    refused && served && denied && untouched && table.live() == 4 && table.promised() == 0
}

/// Open the corpus's small file through the ring, and give back the table and
/// the handle.
///
/// Shared by the checks above, because building a directory handle and opening
/// through it is four operations and repeating them would make each check
/// mostly setup.
fn open_the_small_file(
    queue: &queue::Queue,
) -> Option<(alloc::sync::Arc<crate::handle::HandleTable>, u32)> {
    let root = vfs::root()?;
    let object = crate::object::directory(root)?;
    let table = alloc::sync::Arc::new(crate::handle::HandleTable::new(8));
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

/// A second backend answers the same three operations.
///
/// `014` is explicit that the VFS needs more than one backend before its
/// abstraction is anything but a guess: a directory trait with one
/// implementation is a description of that implementation. tmpfs is
/// deliberately unlike FAT32 in the ways that matter — no clusters, no chains,
/// no device — so if `lookup`, `enumerate` and `read` fit both, they are about
/// directories (D-172).
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
    if got == 0 || &read[..got] != b"the wizard's os, from memory\n" {
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

    // **A name that exists in two directories resolves to the right one.** A
    // lookup that ignored the parent would be correct about every path in a
    // tree whose names are unique, so this tree's are not (D-172).
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
    names == 2 && directories == 1 && matches!(through, Err(vfs::VfsError::NotADirectory))
}

/// A device is not a file, and nothing can ask it for bytes.
///
/// `012` §7 is emphatic: a devfs entry has no bytes, it is a name that yields a
/// handle of another type, and forcing it to implement `read_at` is the
/// "everything is a file" conflation WASI's own README rejects.
///
/// **The enforcement is an absence, not a refusal.** `vfs::read` takes a
/// `File`, and a `Node::Device` is not one — so there is no call to make and no
/// error to return. What this check can assert is the runtime half: the name
/// resolves to a device, the listing calls it neither file nor directory, and
/// a device cannot be walked through (D-172).
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

/// No backend lets a path escape, and no handle sees another backend's names.
///
/// The resolver's invariant is about the *resolver*, not about FAT — so it is
/// checked on every backend there is. And a directory handle names its own
/// backend: a tmpfs handle cannot resolve a name that exists only on the
/// volume, which is what "one namespace per program" means from the inside
/// (D-171, D-172).
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

/// A path resolves from a directory handle, and only downward.
///
/// The VFS's shape: `lookup` node to node, holding the backend for the walk's
/// life rather than re-deriving a node from a name at each step — which is
/// what closes the gap Linux needed `openat2` for.
fn a_path_resolves_from_a_directory_handle() -> bool {
    let Some(root) = vfs::root() else {
        return false;
    };

    // A file at the top.
    let Ok(vfs::Node::File(small)) = vfs::resolve(root, SMALL_NAME) else {
        return false;
    };
    // The variant matters: a FAT file is what a FAT root resolves to, and a
    // second backend arriving made this a match rather than a binding — which
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

/// No path from a directory handle can name anything above it.
///
/// The invariant `012` §5 states and this phase's headline security property:
/// **there is no operation that can walk upward.** `..` is resolved lexically
/// before any node is touched, so what reaches the resolver is a list of names
/// that only descends — and the resolver holds no parent pointer to be asked.
///
/// Every direction it can be attempted from, and each is a *different*
/// refusal: an absolute path is malformed because there is no root, and an
/// escaping `..` is malformed because there is no parent. Neither is a denial,
/// which would suggest some other program could (D-171).
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

/// A component that is not a directory cannot be walked through.
///
/// Without it, `HELLO.TXT/NESTED.TXT` would be answered out of whatever the
/// file's bytes look like as directory entries — which for a file the caller
/// controls is a caller choosing what the resolver finds.
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

/// Enumeration lists what is there, and it is not the same operation as
/// traversal.
///
/// `012` §4 makes "may resolve a name I know" and "may list the names"
/// separate authorities, and this is the second one working: the resolver
/// consults one and this consults the other. Capsicum folds enumeration into
/// read; WASI Preview 1 had both by accident of being syscall-named.
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

    // Every name it listed also resolves, which is the two operations
    // agreeing — an enumeration that returned a name traversal cannot find is
    // a directory listing that lies.
    //
    // **Collected first, resolved after.** Both operations reach the
    // controller through a leaf guard, so resolving *inside* the enumeration
    // callback re-enters that guard and spins for ever. The first version of
    // this check did exactly that and hung the harness, which is the rule
    // `lock_leaf` states, arriving as a sixty-second timeout (D-171).
    let mut listed = [[0u8; 12]; 4];
    let mut lengths = [0usize; 4];
    let mut count = 0usize;
    let _ = vfs::enumerate(root, |name, _| {
        if count < listed.len() && name.len() <= listed[count].len() {
            listed[count][..name.len()].copy_from_slice(name);
            lengths[count] = name.len();
            count += 1;
        }
        true
    });

    let mut all_resolve = count == 3;
    for index in 0..count {
        all_resolve &= vfs::resolve(root, &listed[index][..lengths[index]]).is_ok();
    }

    names == 3 && directories == 1 && saw_small && saw_large && all_resolve
}

/// The volume mounts, and says something a FAT32 volume would say.
///
/// Not "it mounted" — that is true of any sixteen bytes ending `0x55 0xAA`.
/// The claims are the ones that would be false if the reader had landed on the
/// wrong sector or read the sixteen-bit sectors-per-FAT that FAT32 leaves zero:
/// a root cluster in the data area, a cluster count that makes it FAT32 rather
/// than FAT16 wearing the name, and a data area past the tables.
fn the_volume_mounts_and_says_what_a_fat32_volume_says() -> bool {
    matches!(
        fs::with_volume(|volume| {
            let bpb = volume.bpb();
            Ok(bpb.root_cluster() >= fat32::FIRST_DATA_CLUSTER
                && bpb.clusters() >= 65_525
                && bpb.cluster_bytes() >= fat32::SECTOR_BYTES as u32
                && bpb.first_data_sector() > 0)
        }),
        Some(Ok(true))
    )
}

/// The kernel reads the bytes the host wrote.
///
/// The end of the whole storage path: `xtask` builds a volume with `fatfs`,
/// writes it onto the namespace, the controller DMAs sectors of it into a
/// frame, and `libs/fat32` finds a file in it. The host reads the same corpus
/// with the same reader and with `fatfs` besides (D-170), so a disagreement
/// here is about the *machine*, not about the format.
fn the_kernel_reads_the_file_the_host_wrote() -> bool {
    matches!(
        fs::with_volume(|volume| {
            let root = volume.bpb().root_cluster();
            let entry = volume.lookup(root, SMALL_NAME)?;
            if entry.size as usize != SMALL_CONTENT.len() || entry.is_directory() {
                return Ok(false);
            }

            // The root holds three live entries and one deleted slot between
            // them. A reader that took `0xE5` for the end of the directory
            // would find the first and lose the rest, so the count is part of
            // the claim.
            let mut live = 0usize;
            volume.walk_directory(root, |_| {
                live += 1;
                true
            })?;
            if live != 3 {
                return Ok(false);
            }

            // **A whole sector of buffer for a file that is not one.** A
            // reader bounded by the cluster rather than by the size would hand
            // back the last cluster's tail — its previous occupant's data — and
            // a buffer cut to the file's length could never see it (D-170).
            let mut read = [0u8; fat32::SECTOR_BYTES];
            let got = volume.read_file(&entry, 0, &mut read)?;
            Ok(got == SMALL_CONTENT.len() && &read[..got] == SMALL_CONTENT)
        }),
        Some(Ok(true))
    )
}

/// A file longer than a cluster is read across its chain.
///
/// Reading from offset zero works whatever the chain walk does, because the
/// first cluster is the entry's own. Only a read that has to **skip** clusters
/// exercises it — and only a file whose bytes differ by position can tell a
/// skip of three from a skip of four, which is why the corpus is filled with a
/// mixed index rather than a repeated byte (D-155, D-170).
fn a_file_longer_than_a_cluster_is_read_across_its_chain() -> bool {
    matches!(
        fs::with_volume(|volume| {
            let root = volume.bpb().root_cluster();
            let entry = volume.lookup(root, LARGE_NAME)?;
            if entry.size as usize != LARGE_BYTES {
                return Ok(false);
            }
            let cluster = volume.bpb().cluster_bytes() as usize;

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
                let got = volume.read_file(&entry, offset as u32, &mut window)?;
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
            Ok(volume.read_file(&entry, entry.size, &mut past)? == 0)
        }),
        Some(Ok(true))
    )
}

/// A path resolves through a directory, and refuses to walk through a file.
///
/// The walk `012` §5 will build the resolver on: component by component,
/// holding a node per step. Refusing a component that is not a directory is
/// what stops `HELLO.TXT/NESTED.TXT` from being answered out of whatever the
/// file's bytes look like as directory entries.
fn a_path_resolves_through_a_directory() -> bool {
    matches!(
        fs::with_volume(|volume| {
            let nested = volume.resolve(&[DIRECTORY_NAME, NESTED_NAME])?;
            if nested.size as usize != NESTED_CONTENT.len() {
                return Ok(false);
            }
            let mut read = [0u8; 128];
            let got = volume.read_file(&nested, 0, &mut read)?;
            if got != NESTED_CONTENT.len() || &read[..got] != NESTED_CONTENT {
                return Ok(false);
            }

            // A file is not a directory, whatever its bytes look like.
            let through_a_file = volume.resolve(&[SMALL_NAME, NESTED_NAME]);
            let missing = volume.resolve(&[b"NOPE.TXT"]);
            Ok(through_a_file == Err(fat32::FatError::WrongKind)
                && missing == Err(fat32::FatError::NotFound))
        }),
        Some(Ok(true))
    )
}

/// What `xtask` put on the namespace.
///
/// Transcribed from `xtask/src/image/corpus.rs`, which writes it — two copies
/// of one table, so a host test compares them (D-158, D-170).
const SMALL_NAME: &[u8] = b"HELLO.TXT";
/// See [`SMALL_NAME`].
const SMALL_CONTENT: &[u8] = b"the wizard's os reads a disk\n";
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

/// The controller says what it is, and the driver believed it rather than
/// assuming.
///
/// Every field here changes what a correct driver does, and `012` §11 records
/// that VMware's are unmeasured — so this is the check that the *reading*
/// happened, not that the values are any particular thing. A queue larger than
/// `CAP.MQES` allows, or a page size outside `MPSMIN..MPSMAX`, is a controller
/// that will not become ready and a driver that waits out the timeout.
fn the_controller_was_read_rather_than_assumed() -> bool {
    nvme::with_controller(|controller, _| {
        let capabilities = controller.capabilities();

        capabilities.supports_nvm_command_set()
            && capabilities.supports_page_size(PAGE_SIZE)
            // The queue this driver built fits what the controller offered.
            && controller.queue_entries() <= capabilities.max_queue_entries()
            && controller.queue_entries() >= 2
            && controller.has_io_queue()
            // And the namespace answered `Identify` with something usable.
            && controller.blocks > 0
            && controller.block_bytes.is_power_of_two()
            && controller.block_bytes >= 512
    })
    .unwrap_or(false)
}

/// A read returns the block that was asked for.
///
/// The end of the whole path: bytes written into a file on the host, carried
/// through the controller's DMA into a frame this kernel owns, and read back.
///
/// **Two different blocks, and that is the point.** The namespace is written
/// with a different signature in every block, because a disk of zeros cannot
/// tell a read that happened from one that did not, and a disk with the same
/// bytes everywhere cannot tell a read of block three from a read of block
/// zero — so a driver that ignored the logical block address would pass
/// (D-155, D-169).
fn a_block_read_returns_the_block_that_was_asked_for() -> bool {
    nvme::with_controller(|controller, buffer| {
        let block = controller.block_bytes as usize;
        for lba in [0u64, 3, 1, 7] {
            // Cleared first, so what is found afterwards was put there by this
            // read and not left by the last one.
            if !buffer.fill(0) {
                return false;
            }
            if controller.read_blocks(lba, 1, buffer).is_err() {
                return false;
            }

            let mut head = [0u8; 8];
            if !buffer.read_at(0, &mut head) {
                return false;
            }
            if u64::from_le_bytes(head) != disk_signature(lba) {
                return false;
            }

            // **And nothing past one block.** Every count in this
            // specification is zero-based, so a driver that sent the block
            // count unadjusted reads one block too many — and the first
            // block's bytes are still correct, so only the bytes *after* it
            // say so (D-169).
            let mut beyond = [0u8; 8];
            if !buffer.read_at(block, &mut beyond) {
                return false;
            }
            if u64::from_le_bytes(beyond) != 0 {
                return false;
            }
        }
        true
    })
    .unwrap_or(false)
}

/// The queues wrap, and the phase tag still says which entries are new.
///
/// There is no completion tail to read: an entry is recognised as new by its
/// **phase tag**, which flips every time the queue wraps (`012` §3). A driver
/// that never flipped it works perfectly until the first wrap and then sees
/// either every entry or none — so the check does more commands than the queue
/// has entries, several times over, which is the only way to reach the seam.
fn the_queues_wrap_and_the_phase_tag_still_works() -> bool {
    nvme::with_controller(|controller, buffer| {
        let laps = controller.queue_entries() * 3;
        for turn in 0..laps {
            let lba = u64::from(turn % 8);
            if controller.read_blocks(lba, 1, buffer).is_err() {
                return false;
            }
            let mut head = [0u8; 8];
            if !buffer.read_at(0, &mut head) {
                return false;
            }
            if u64::from_le_bytes(head) != disk_signature(lba) {
                return false;
            }
        }
        laps > controller.queue_entries()
    })
    .unwrap_or(false)
}

/// The controller's registers are mapped uncacheable.
///
/// `012` §2 names it: mapping a register file write-back is a silent,
/// hard-to-debug bug — writes arrive when the cache feels like it and reads
/// answer from a line that was filled minutes ago. An emulator forgives it and
/// hardware does not, which is exactly the class of thing a check has to catch
/// here rather than there.
fn the_controllers_registers_are_uncacheable() -> bool {
    nvme::with_controller(|controller, _| {
        paging::translate(controller.window()).is_some_and(|(_, flags)| {
            flags.contains(Flags::NO_CACHE) && !flags.contains(Flags::WRITE_THROUGH)
        })
    })
    .unwrap_or(false)
}

/// The first eight bytes of block `lba` on the check machine's namespace.
///
/// Written by `xtask` when it makes the backing file, and transcribed here —
/// which is two copies of one table, so a host test compares them (D-158).
const fn disk_signature(lba: u64) -> u64 {
    0x5749_5A44_0000_0000 | lba
}

/// The bus is walked, and the walk goes through the bridge.
///
/// A kernel that scans bus zero and stops finds every function q35 has and
/// reports a perfectly plausible number, which is why the *bus* count is what
/// is asserted. VMware places functions behind `pciBridge0`–`pciBridge7`
/// (`012` §2) and D-18 makes VMware the phase's definition of done, so a walk
/// that cannot descend is a driver that cannot find its controller — and
/// nothing about the function count would say so.
///
/// The check machine is given a `pci-bridge` for exactly this reason: q35 as
/// QEMU ships it has none, so there would be nowhere to descend to and the
/// property would be untestable rather than untested (D-168).
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

/// The same function is never claimed twice, and a refusal strands nothing.
///
/// D-77's device half. A port claim proves *exclusivity* and says nothing
/// about *entitlement* — whether the claimant is the driver for the thing
/// behind the ports. A device claim answers that by making the claim about the
/// device, and it is permanent for the same reason a port claim is (D-138):
/// there is no release, so there is no window in which a device is
/// half-claimed.
///
/// The second half of the name is the part that would rot silently. A refusal
/// that quietly released the holder's claim would look identical from here
/// unless the holder is asked afterwards, so it is (D-138, D-168).
fn a_device_is_claimed_once_and_a_refusal_strands_nothing() -> bool {
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
    let Ok(held) = pci::claim(at) else {
        return false;
    };
    let recorded = pci::is_claimed(at) && pci::claims() == before + 1;

    // A second driver for one device, refused.
    let refused = matches!(pci::claim(at), Err(pci::ClaimError::Taken));

    // And the holder still holds it: a refusal that released what it found
    // would be indistinguishable from here without asking.
    let intact = pci::is_claimed(at)
        && pci::claims() == before + 1
        && held.at().bits() == at.bits()
        && held.identity().0 != 0xFFFF;

    // A bridge has **two** base address registers, not six: its primary,
    // secondary and subordinate bus numbers sit where a device's third would
    // be. A claim that read six would decode those bus numbers as a window and
    // then probe it — writing all ones into the register that says which bus
    // is behind this bridge. Nothing else here would notice, because the probe
    // restores what it found (D-168).
    let two_registers = held.window(2).is_none()
        && held.window(3).is_none()
        && held.window(4).is_none()
        && held.window(5).is_none();

    recorded && refused && intact && two_registers
}

/// A device's I/O window is the device's, and a bare port claim inside it is
/// refused.
///
/// The whole of D-77's entitlement complaint, made concrete. Before this a
/// driver took ports and the registry recorded that *somebody* had them; now
/// the claim belongs to the function that decodes them, and a caller reaching
/// for a port inside a claimed device's window is refused because the device
/// already has it (D-168).
///
/// **The check fails rather than passes when there is no such device.** A
/// machine with no I/O window would make this vacuous, which is exactly the
/// shape D-165 caught in slice 1 — so finding one is part of the claim.
fn a_claimed_devices_io_window_is_not_claimable_bare() -> bool {
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

    // Walk the candidates until one with an I/O window is claimed. A function
    // may have none, and "none of them had one" is a failure here rather than
    // a pass.
    let mut found = None;
    let mut seen = 0;
    let _ = pci::enumerate(|function| {
        if found.is_some() || function.header.is_bridge() || pci::is_claimed(function.at) {
            return;
        }
        seen += 1;
        let Ok(device) = pci::claim(function.at) else {
            return;
        };
        for index in 0..pci::BARS {
            if let Some(window) = device.window(index)
                && let pci_config::Bar::Io { port } = window.bar
                && window.size > 0
            {
                found = Some((port, window.size));
                return;
            }
        }
    });

    let Some((port, size)) = found else {
        return false;
    };

    // Every port in the window is the device's. The first and the last, so a
    // claim that recorded only the base would be caught.
    let last = port + (size as u16 - 1);
    let base_refused = wost::arch::current::port::claim::<u8>(port).is_none();
    let last_refused = wost::arch::current::port::claim::<u8>(last).is_none();

    // And a port outside it is still available, so the refusal is about the
    // window and not about everything. A claim here is permanent, so this
    // takes one port for the life of the boot, which is the honest cost of
    // showing that the refusals above are bounded.
    let outside = port
        .checked_add(size as u16)
        .and_then(wost::arch::current::port::claim::<u8>)
        .is_some();

    seen > 0 && base_refused && last_refused && outside
}

/// A program that dies with work in flight has it ended before its ring goes.
///
/// D-153 as `012` §10 amends it: **exit is a wait, not a signal.** io_uring
/// cancelled out of line for six years and the symptom was mount points
/// staying busy after the process had exited. An operation still in flight is
/// one that could still write a completion, and the page it would write into
/// is about to become the allocator's.
///
/// So the claim is not "the operation is forgotten" but "the operation is
/// *finished*": it gets the one terminal completion D-79 promises it, into
/// memory that still exists, and the reservation it was holding is spent. A
/// reservation never spent is a ring whose accounting says work is outstanding
/// for ever, which is invisible until something asks — and slice 5's device
/// will ask (D-167).
fn work_in_flight_is_ended_before_the_ring_is_released() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let table = a_table();

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
        // Nothing left that could touch the pages, and the reservations are
        // spent rather than held: `outstanding` is the accounting that would
        // otherwise say work is in flight for ever.
        && queue.is_quiet()
        && queue.in_flight() == 0
        && queue.outstanding() == 0
        && queue.is_consistent()
        // And a second sweep finds nothing, so teardown is idempotent — which
        // it must be, because `reclaim` is not the only path that could reach
        // a zombie.
        && queue.abandon() == 0
}

/// A promise outlives the program that made it, and gives its slot back.
///
/// The third of D-151's three terminal paths, and the one slice 1 could not
/// reach: a program that dies holding a promise against its own handle table.
///
/// The reservation holds an `Arc` to the table, so the table stays alive
/// exactly as long as the promise does — the teardown drops its reference and
/// the table does not go anywhere. That is what makes "the zombie holds every
/// resource any in-flight operation can still touch" true by construction
/// rather than by remembering (D-163, D-167).
///
/// Observed through a `Weak`, which is the only way to ask "is it still there"
/// without being the reason it is. Then the table is *used* after its owner is
/// gone, because surviving and working are different claims.
fn a_promise_outlives_the_program_that_made_it() -> bool {
    let Some((held, _far)) = crate::object::channel() else {
        return false;
    };
    let Some((second, _far_two)) = crate::object::channel() else {
        return false;
    };

    let table = alloc::sync::Arc::new(crate::handle::HandleTable::new(4));
    let watching = alloc::sync::Arc::downgrade(&table);

    let full = Rights::UNIVERSAL.union(Rights::READ);
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

    // Surviving is not the same as working: the promise is spent against the
    // orphaned table, which is exactly what a completion arriving after the
    // program's death would do.
    let installed = orphan.install(reservation, second, full, full).is_ok();
    let accounted = orphan.live() == 2 && orphan.promised() == 0;

    // And with the promise gone, the table goes when the last reference does.
    // A `Weak` that still upgrades here is a table nothing will ever free.
    drop(orphan);
    let released = watching.upgrade().is_none();

    promised && installed && accounted && released
}

/// The constants region says what the kernel measured.
///
/// No sequence counter guards this region and none is needed: it is written
/// before the page can be mapped anywhere and never again (D-166). What is
/// checked is that it was written *at all* and with the right numbers — a
/// zeroed region reads as a plausible page with a version of zero, which is
/// what a reader would then refuse.
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

/// The page and the call tell the same time.
///
/// The whole claim of D-152: `sys_clock_get` stops being a call and becomes a
/// load, and the two answers have to be *the same answer* rather than two
/// plausible ones. So this is a sandwich rather than a tolerance — the page's
/// reading is taken between two calls and must lie between them. No slack, and
/// a multiplier or shift off by one breaks it immediately.
fn the_time_page_and_the_call_tell_the_same_time() -> bool {
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

/// Revoking the clock sends readers back to the call.
///
/// The gate D-152 asks for, exercised rather than described. Three things have
/// to hold at once: the revocation is visible, the sequence counter moved by
/// exactly two so no reader can be left mid-update, and **the syscall still
/// answers** — a fallback that is not reachable is not a fallback.
///
/// The clock is put back afterwards. Nothing else reads the page, so the
/// window is the kernel's own.
fn revoking_the_clock_sends_readers_back_to_the_call() -> bool {
    let Some(before) = timepage::read() else {
        return false;
    };

    if !timepage::publish(
        timepage::MODE_NONE,
        crate::time::origin_ticks(),
        crate::time::conversion(),
    ) {
        return false;
    }

    let Some(revoked) = timepage::read() else {
        return false;
    };
    // Exactly two: one to say a write began and one to say it ended. A writer
    // that moved it by one leaves the page permanently odd and every reader
    // spinning.
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
        let table = a_table();
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
    ) && timepage::read()
        .is_some_and(|reading| reading.mode == timepage::MODE_COUNTER);

    stepped && closed && queue_answers && restored
}

/// The clock's update window is never preemptible, and never left open.
///
/// D-152 makes this a **precondition, not a comment**: Linux's naive reader
/// spin is only tolerable because its update runs with interrupts off, and a
/// writer preempted with the sequence counter odd leaves every reader spinning
/// for a scheduling quantum.
///
/// Two claims, and the first version had only a worse form of the second. It
/// counted *calls* made with interrupts on and failed, correctly, because a
/// kernel thread calling `publish` with interrupts on is fine — the guard
/// clears the flag before the counter goes odd (D-166). What matters is the
/// window, so that is what is counted now.
///
/// The second claim is that the window always closes: the counter is even at
/// rest. A writer that returned early between its two increments would leave
/// the page permanently odd and every reader spinning, and no amount of
/// interrupt discipline would help.
fn the_clocks_update_window_is_never_preemptible() -> bool {
    // The revocation check has published twice by now, so this is not a count
    // that has never been exercised.
    let Some(seq) = timepage::sequence() else {
        return false;
    };

    timepage::is_published()
        && timepage::windows_left_preemptible() == 0
        // Every update marked its window open. On one CPU nothing else can
        // witness the odd value — the guard clears the interrupt flag, so no
        // other context runs between the two increments — and a writer that
        // skipped the odd write left every other check on this page green
        // (D-166). The writer reads its own marker back, and this is that.
        && timepage::windows_never_opened() == 0
        && timebase::Seq::is_stable(seq)
        // Publishing moves it, so "even" is not merely "never written".
        && seq >= 4
}

/// A program is shown the time page and cannot write it.
///
/// Readable from ring 3, and nothing else: a writable time page is a program
/// able to tell every other program a lie about the time, and an executable one
/// is a page of attacker-chosen bytes in every address space.
fn a_program_sees_the_time_page_and_cannot_write_it() -> bool {
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    if timepage::install(&mut space).is_err() {
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
}

/// A completion carries a handle, and the rights narrow on the way in.
///
/// D-151's headline: the ring becomes a delegation path, which it was not.
/// The program declares what it is willing to hold and gets **the narrower of
/// that and what was there** — the same rule D-110 puts on a channel, so the
/// ring cannot be the widening path around it.
///
/// Driven through the shared page, because the page is the boundary. The
/// rights are checked by resolving the handle that arrived rather than by
/// trusting the status: a completion saying `OK` about a handle nobody looked
/// at is a claim, not a check.
fn a_completion_carries_a_handle_with_narrowed_rights() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some((held, _far)) = crate::object::channel() else {
        return false;
    };
    let table = alloc::sync::Arc::new(crate::handle::HandleTable::new(8));

    let offered = Rights::UNIVERSAL.union(Rights::READ).union(Rights::WRITE);
    let Ok(source) = table.insert(held, offered) else {
        return false;
    };

    // Read only, out of a handle that also carries write.
    let declared = Rights::UNIVERSAL.union(Rights::READ);
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
        && !granted.contains(Rights::WRITE)
        && arrived != source
        && table.live() == 2
        // The promise became a handle rather than staying a promise, which is
        // the accounting half: a table charged twice for one handle runs out
        // at half its quota and nothing else would say so.
        && table.promised() == 0
}

/// A completion cannot carry rights the program was never offered.
///
/// The refusal is at the *attenuation*, and it is a refusal rather than a
/// silent narrowing: a program that asks for more than it holds has made a
/// mistake, and quietly giving it less is how a caller comes to believe it has
/// authority it does not.
///
/// The table is inspected afterwards as well as the status, because the
/// failure that matters is not the refusal — it is a refusal that kept the
/// slot it promised.
fn a_completion_cannot_widen_what_was_offered() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some((held, _far)) = crate::object::channel() else {
        return false;
    };
    let table = alloc::sync::Arc::new(crate::handle::HandleTable::new(8));

    // Held without WRITE, so asking for it is asking to widen.
    let offered = Rights::UNIVERSAL.union(Rights::READ);
    let Ok(source) = table.insert(held, offered) else {
        return false;
    };

    let asked = Rights::UNIVERSAL.union(Rights::READ).union(Rights::WRITE);
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

/// Every refused handle operation gives its promised slot back.
///
/// The property D-151 states and `012` §10 names the symptom of missing:
/// io_uring's `IORING_FILE_INDEX_ALLOC` reporting `-ENFILE` at completion. A
/// promise that is not returned is not a crash — it is a table that shrinks by
/// one every time a program asks for something it cannot have.
///
/// **Every submission here reserves and then fails**, which is the whole
/// difficulty of writing this check. The first version used a table that was
/// already full, so all three refusals happened *at* the reservation and none
/// of them ever held one — `promised() == 0` was true because nothing had been
/// promised, and deleting the destructor that returns promises left it green
/// (D-165). The table is deliberately roomy, and each refusal happens at a
/// different point after the promise is taken: at the lookup, at the right, and
/// at the attenuation.
fn a_refused_handle_operation_returns_its_promise() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some((duplicable, _far_one)) = crate::object::channel() else {
        return false;
    };
    let Some((sealed, _far_two)) = crate::object::channel() else {
        return false;
    };

    // Room to spare: a refusal must be reached *after* a promise is taken, and
    // a full table refuses before there is one.
    let table = alloc::sync::Arc::new(crate::handle::HandleTable::new(8));
    let full = Rights::UNIVERSAL.union(Rights::READ);
    let Ok(source) = table.insert(duplicable, full) else {
        return false;
    };
    // Held without DUPLICATE, which is what makes copying it a refusal rather
    // than a mistake.
    let Ok(undupable) = table.insert(sealed, Rights::READ) else {
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
            u64::from(Rights::READ.bits()),
            queue::STATUS_DENIED,
        ),
        // Refused at the attenuation: asking for more than is held.
        (
            0xF003,
            undupable,
            u64::from(Rights::READ.union(Rights::WRITE).bits()),
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

    // The whole point: three promises taken, three given back. A leak here is
    // invisible in the completions and shows up later as a table that is full
    // of nothing.
    refused && table.live() == 2 && table.promised() == 0
}

/// A full table refuses at submission, before it promises anything.
///
/// The other half of D-151, and a different claim from the one above: there,
/// three promises are taken and returned; here, none is taken at all, and the
/// program is told *at submission* — the one moment it can close something and
/// ask again.
fn a_full_table_refuses_a_handle_operation_at_submission() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let Some((held, _far)) = crate::object::channel() else {
        return false;
    };

    // Room for exactly one, which the source takes.
    let table = alloc::sync::Arc::new(crate::handle::HandleTable::new(1));
    let full = Rights::UNIVERSAL.union(Rights::READ);
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
    queue.publish_tail_for_test(1);

    if queue.serve(&table) != 1 {
        return false;
    }
    let completion = queue.read_completion(0);

    completion.token == 0xFEED
        && completion.status == queue::STATUS_NO_HANDLE_SLOT
        && completion.value == 0
        && table.live() == 1
        && table.promised() == 0
}

/// An operation that is not ready yet completes later, and completes once.
///
/// The first operation in this kernel whose honest answer is *not yet* (D-164).
/// Serving it produces **no** completion — which is not the same as producing
/// none ever, and the difference is the reservation it is holding.
///
/// Three moments are checked and they are three different claims: nothing at
/// admission, nothing before the deadline, and exactly one after it. The
/// middle one is what fails if a deferral is really an immediate completion
/// with a delay written in the value.
fn an_operation_completes_after_its_deadline_and_only_once() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let table = a_table();

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

/// A cancellation is its own operation and does not stand in for the target's.
///
/// The antidote to the double-completion class behind two io_uring CVEs
/// (`012` §8): the cancelled operation still gets **exactly one** terminal
/// completion, carrying a cancellation rather than a result, and the
/// cancellation gets its own against its own reservation.
///
/// So the count is two, not one and not three — and the target's is the one
/// that says `CANCELLED`, which is what distinguishes this from a design where
/// the canceller simply reports on the target's behalf.
fn a_cancelled_operation_and_its_cancellation_complete_separately() -> bool {
    if !crate::time::is_calibrated() {
        return false;
    }
    let Some(queue) = queue::Queue::new() else {
        return false;
    };
    let table = a_table();

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

    // Nothing is left to complete, and a second cancellation of the same token
    // finds nothing — advisory rather than an error, which is io_uring's
    // `-EALREADY` and the right answer for a race the program cannot avoid.
    queue.post_submission_for_test(
        2,
        queue::Submission::simple(queue::Opcode::Cancel, 0xC0DF, 0xDEAD),
    );
    queue.publish_tail_for_test(3);
    let again =
        queue.serve(&table) == 1 && queue.read_completion(2).status == queue::STATUS_NOT_FOUND;

    separate && again && queue.in_flight() == 0 && queue.is_consistent()
}

/// Every submission gets exactly one completion, and it is the one it asked
/// for.
///
/// D-79's terminal-completion rule, driven through the shared page rather than
/// through a function call — the page is the boundary and the call is not. An
/// opcode the kernel does not serve is refused **with** a completion rather
/// than ignored, which is the same rule seen from the only angle that could
/// break it.
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

    let served = queue.serve(&a_table());
    // Exactly one each, and no more: a second call finds nothing left.
    let once = served == asked.len() && queue.serve(&a_table()) == 0;
    let accounted = queue.outstanding() == 0 && queue.is_consistent();

    let echo = queue.read_completion(0);
    let uptime = queue.read_completion(1);
    let refused = queue.read_completion(2);

    let matched = echo.token == 0x1111
        && echo.status == queue::STATUS_OK
        && echo.value == 0xDEAD_BEEF
        && uptime.token == 0x2222
        && uptime.status == queue::STATUS_OK
        // The value, not only the token: replacing `now_ns()` with a zero
        // left this green (D-115). The clock has been running since boot, so
        // a real uptime is neither zero nor absurd.
        && uptime.value > 0
        && uptime.value <= crate::time::now_ns()
        && refused.token == 0x3333
        // Refused *with* a completion, which is the rule: an operation the
        // kernel will not serve still reports, or a caller waits for ever.
        && refused.status == queue::STATUS_UNKNOWN_OPCODE;

    once && accounted && matched
}

/// The completion queue cannot overflow, because admission refuses first.
///
/// D-79's structural claim and D-112's headline invariant, on the kernel's own
/// accounting: a submission reserves its completion slot *before* it is
/// accepted, so work is declined rather than reported nowhere. io_uring's
/// `CQ_NODROP` instead takes an unbounded kernel-memory liability for a
/// program that never reaps.
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

/// A submission rewritten after the kernel has taken it does not change what
/// the kernel does with it.
///
/// The copy-out-once rule (D-79). CVE-2025-39816 is literally "always use
/// `READ_ONCE()` to read ring pointers", and a double fetch is how a validated
/// value becomes a different value before it is used. The pages here are ones
/// ring 3 can rewrite at any moment, so this rewrites one at the only moment
/// that matters — after the kernel has served it — and asks whether the
/// completion already posted moved.
///
/// It also asks the question the rewrite cannot: **admitting is not
/// publishing**. Reserving room for three and writing one must serve one, or
/// the kernel reports completions for entries that are still whatever the page
/// happened to hold.
fn a_rewritten_submission_cannot_change_what_was_served() -> bool {
    let Some(queue) = queue::Queue::new() else {
        return false;
    };

    queue.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Echo, 0xABCD, 0x0F0F_0F0F),
    );
    queue.publish_tail_for_test(1);
    if queue.serve(&a_table()) != 1 {
        return false;
    }
    let posted = queue.read_completion(0);

    // Ring 3 rewrites the entry it already submitted, as a hostile program
    // would the instant it saw the head move.
    queue.post_submission_for_test(
        0,
        queue::Submission::simple(queue::Opcode::Unknown(0xDEAD), 0xBAAD, 0xFFFF_FFFF),
    );

    // The completion is what the *copy* said, and the rewrite is visible in
    // the page — so the check is not merely reading two disjoint places and
    // finding them both unchanged. An earlier version asserted
    // `read_completion(0) == posted` where `posted` had just been read from
    // that same slot, which is `X == X` and had no line whose deletion could
    // falsify it (D-115).
    let rewritten = queue.read_submission(0);
    let took_effect = rewritten.token == 0xBAAD && rewritten.opcode != queue::Opcode::Echo;
    let unchanged = took_effect
        && posted.token == 0xABCD
        && posted.status == queue::STATUS_OK
        && posted.value == 0x0F0F_0F0F
        // And serving again does not re-serve the rewritten entry: the slot
        // was consumed, and the new bytes are a submission nobody made.
        && queue.serve(&a_table()) == 0;

    // A tail run away with the kernel's ring *empty* is refused — but so it
    // would be with no bound at all, because `take` finds nothing. The case
    // that reaches the bound is a runaway tail with room admitted behind it:
    // three reservations and a tail claiming four billion. Without
    // `posted > SUBMISSIONS` the kernel walks all three slots and serves
    // whatever the page happens to hold, and no check saw that (D-115).
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
    let runaway_bounded = runaway.serve(&a_table()) == 0 && runaway.outstanding() == 0;

    // And with the ring drained, which is the weaker case the earlier version
    // was testing without knowing it.
    queue.publish_tail_for_test(u32::MAX);
    let drained_bounded = queue.serve(&a_table()) == 0;

    // The case the runaway tail cannot show: **writing is not publishing.**
    // A program may write
    // three entries and publish a tail of one, and the kernel must serve one.
    // The tail is what says an entry is *there*; the other two slots hold
    // bytes the program wrote and has not offered, and serving them would
    // report completions for operations nobody made.
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
    let only_published = partial.serve(&a_table()) == 1;
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

/// Thread-local storage follows the thread, and ring 3 cannot point it at the
/// kernel.
///
/// The fourth thing that moves across a switch, after the privilege stack
/// (D-99), the tables (D-106) and the extended state (D-108). Nothing native
/// asked for it: writing an mlibc sysdep did, because nothing native has
/// thread-local storage and every POSIX thread does (D-114, D-118).
///
/// The refusal is the other half. `wrmsr` to `FS_BASE` with a non-canonical
/// value is a general protection fault in *kernel* context — a program
/// crashing the handler rather than itself — so the safe door range-checks
/// first, and it checks against the user boundary rather than canonicality
/// alone because a program asking to point `fs:` at kernel memory has no
/// reason it could be given.
fn thread_local_storage_follows_the_thread() -> bool {
    /// Somewhere in the low half, aligned, and not zero.
    const MINE: u64 = 0x0000_1234_5678_0000;
    /// What the other thread sets.
    const THEIRS: u64 = 0x0000_7654_3210_0000;

    let restore = task::read_thread_base();

    if !task::set_thread_base(MINE) || task::read_thread_base() != MINE {
        return false;
    }

    // A base in the kernel's half is refused, and the register is untouched.
    let kernel_refused =
        !task::set_thread_base(0xffff_8000_0000_0000) && task::read_thread_base() == MINE;
    // So is one in the canonical hole, which the same range check catches.
    let hole_refused =
        !task::set_thread_base(0x0000_8000_0000_0000) && task::read_thread_base() == MINE;

    BASE_INTRUDER_RAN.store(0, Ordering::Release);
    if thread::spawn("base-intruder", set_another_thread_base).is_err() {
        return false;
    }
    if !runs_until(500, || BASE_INTRUDER_RAN.load(Ordering::Acquire) == 1) {
        return false;
    }

    // The other thread set its own and read its own back; this one still has
    // what it set, which is only true if the switch saved and restored.
    let kept = task::read_thread_base() == MINE;
    let intruded = BASE_INTRUDER_SAW.load(Ordering::Acquire) == THEIRS as usize;

    let _ = task::set_thread_base(restore);
    kernel_refused && hole_refused && kept && intruded
}

/// Set by the intruding thread once it has run.
static BASE_INTRUDER_RAN: AtomicUsize = AtomicUsize::new(0);

/// What the intruding thread read back from its own base.
static BASE_INTRUDER_SAW: AtomicUsize = AtomicUsize::new(0);

/// Point a second thread's local storage somewhere else, and read it back.
fn set_another_thread_base() {
    const THEIRS: u64 = 0x0000_7654_3210_0000;
    if task::set_thread_base(THEIRS) {
        let mut context = context();
        thread::yield_now(&mut context);
        BASE_INTRUDER_SAW.store(task::read_thread_base() as usize, Ordering::Release);
    }
    BASE_INTRUDER_RAN.store(1, Ordering::Release);
}

/// A page ring 3 could reach comes back to the allocator cleared, whichever
/// road it takes.
///
/// D-107 said `USegment::reclaim` was the only road back and that it zeroes.
/// It was not: `unmap` checked `OWNED` and never `USER`, so it handed a
/// program's pages over as a plain `Segment` — and the reclamation path takes
/// exactly that road, dropping its token rather than flushing it. Nothing
/// disclosed anything, because every consumer happens to zero on the way in;
/// the mechanism named as the guarantee was simply not the one doing the work
/// (D-123).
fn a_user_page_comes_back_cleared() -> bool {
    /// Two user addresses nothing else uses.
    const FIRST: u64 = user::STACK + 0xA0_0000;
    const SECOND: u64 = user::STACK + 0xA1_0000;
    /// What a program would have left in the page.
    const SECRET: [u8; 8] = [0xDE; 8];

    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let writable = Flags::WRITABLE.union(Flags::NO_EXECUTE);

    // One page, holding what a program would have left in it.
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

    // It comes back untyped, and the only way to make it the kernel's again
    // is a clearing one. Deterministic: the handle is followed the whole way
    // rather than the allocator being asked for the frame back.
    let Ok(unmapped) = space.unmap(FIRST, PAGE_SIZE) else {
        return false;
    };
    let knows = unmapped.was_user();
    let Some(untyped) = unmapped.take_user() else {
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

    // And the kernel road is closed: `flush` hands a plain `Segment` over,
    // and a user range is not one it may hand over.
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
    let refused_as_kernel = second.flush().is_none();

    // The **dropping** road — the one reclamation takes. Its frames go to the
    // allocator, so reading them back needs it to hand the same ones over,
    // which it does not promise; a conjunct conditional on that would be
    // skipped on most boots. So the clear is *counted* instead, and the tally
    // is what makes the line testable.
    //
    // I first claimed `cargo xtask mutate` covered this. It did not: removing
    // the clear left every check green and the tool reported the mutation as a
    // **survivor**, which is exactly what it is for (D-119, D-123).
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
}

/// A space a thread will be switched onto is not reclaimed, whatever `CR3`
/// says at the moment of asking.
///
/// Freeing the tables under the register walking them is a triple fault at the
/// next memory access. Freeing a **bound** space's is the same fault deferred
/// to whenever its thread is next scheduled — worse for being harder to
/// attribute, and reachable by a caller simply not being that thread. The
/// `CR3` comparison alone catches only "these tables, right now" (D-123).
///
/// Whether anything still names a space is a property of the *task table*,
/// which `AddressSpace` cannot see, so it takes the answer as evidence rather
/// than guessing.
fn a_bound_space_is_not_reclaimed() -> bool {
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let root = space.root();
    let Ok(binding) = space.bind() else {
        return false;
    };

    // A thread to bind it to, which is what makes it un-reclaimable.
    let Ok(slot) = thread::spawn_in_space("bound-space", leave_at_once, binding, |_| {}) else {
        return false;
    };

    // Refused with no evidence, and refused with evidence for another space:
    // the token names a root, and one minted for a different one is not an
    // answer about this.
    // The space comes back with the refusal, which is what lets the rest of
    // this check happen at all — and is the reason `reclaim` reports its error
    // that way.
    // Refused, and the space is untouched — which is what lets the rest of
    // this check happen at all.
    let refused = space.reclaim(None).is_none() && !space.is_reclaimed();

    // And once the thread is gone, the task table says so and the space comes
    // back — so this is not simply "reclaim never works".
    let gone = runs_until(500, || task::address_space_of(slot).is_none());
    let released = task::release_address_space(root);
    let had_evidence = released.is_some();
    let reclaimed = space
        .reclaim(released)
        .is_some_and(|(tables, _)| tables > 0)
        && space.is_reclaimed();

    refused && gone && had_evidence && reclaimed
}

/// A program that has ended gives its memory back — its pages, its tables,
/// and its slot.
///
/// Until D-117 nothing was reclaimed: a retired thread kept its slot, its
/// stack, its address space, its handle table and its ring for the life of the
/// boot. A handful of programs a boot made that bounded and uninteresting; it stops
/// being either the moment something spawns in a loop, which is what an OS
/// does.
///
/// **Counted in frames, not inferred from a slot count.** A freed slot says a
/// thread ended, which was already true before this decision — the question is
/// whether the *memory* came back, and the allocator is the only thing that
/// can answer it. The measurement is taken across a spawn and its collection,
/// so what it reports is the net cost of running a program to completion.
fn a_dead_program_gives_its_memory_back() -> bool {
    // Drain anything earlier checks left, so the measurement below is about
    // this program and not about theirs.
    while user::collect().is_some() {}

    let free_before = mm::free_frames();
    let slots_before = thread::free_slots();
    let (pages_before, tables_before) = user::reclaimed();
    let retirements_before = thread::retirements();

    let Ok(spawned) = user::run_for_bootstrap() else {
        return false;
    };
    if !runs_until(2000, || thread::retirements() > retirements_before) {
        return false;
    }

    // Its slot came back with it: the thread is gone from both tables, which
    // is what makes the index usable again.
    let slot_returned = runs_until(500, || thread::free_slots() >= slots_before);

    // And its memory, once something collected. The idle thread does this
    // when the machine has nothing better to do; here the check asks
    // directly, because waiting for idle in a busy self-test is waiting for
    // an event that may not come.
    let collected = user::collect().is_some();
    let (pages_after, tables_after) = user::reclaimed();
    let gave_back = pages_after > pages_before && tables_after > tables_before;

    // The allocator agrees. Not exactly equal: the program's *stack* is the
    // kernel's arena, which is mapped once at boot and not per thread, and
    // the check's own bookkeeping allocates nothing but the heap may have
    // grown. What must not happen is the count going *down* across a whole
    // program's life, which is the leak this closes.
    let free_after = mm::free_frames();
    let no_worse = free_after >= free_before;

    // And nothing is left waiting: one program made one zombie, and it is
    // collected.
    let drained = !user::has_zombies();

    let _ = spawned;

    // And it holds under repetition, which is the case reclamation exists for.
    // Sixteen slots, forty programs: without reclamation this runs out after
    // sixteen and every later spawn is refused. The frame count at the end is
    // what says the memory came back too, and not merely the slots.
    let free_at_start = mm::free_frames();
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
    // Forty programs, each with a space, a ring, a table and three mappings:
    // a leak of even one frame apiece would be visible here, and the whole
    // arena is only a few hundred.
    let steady = mm::free_frames() >= free_at_start;

    slot_returned && collected && gave_back && no_worse && drained && repeated && steady
}

/// A program asking for a call this kernel does not serve is refused, and the
/// machine survives it.
///
/// **Two instructions used to halt the machine.** `mov rax, 3; int 0x80` from
/// ring 3 reached a `warn!` in the syscall handler, and logging takes the
/// console's `ThreadLock`, whose first line asserts it is *not* in a trap
/// (D-63). Unprivileged, unconditional, and shipped — because no check had
/// ever issued an unrecognised call number, which is why the arm was green
/// (D-122).
///
/// The refusal is counted rather than logged now, and the count is what this
/// reads: "the machine is still here" is the weaker half, since a check that
/// runs at all has already established it.
fn an_unserved_call_is_refused_without_stopping_the_machine() -> bool {
    // Runs after the ring-3 group and waits for its program, for the reason
    // the bootstrap check does: every check that reads `last_syscall` or
    // `last_user_fault` sees whichever program crossed most recently, so one
    // spawned here and left running is a race it creates for its neighbours.
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

    counted && answered && exited
}

/// A program posts an operation, rings the doorbell, and reads its own answer.
///
/// The whole async ABI, driven from ring 3 rather than from the kernel — which
/// is what `002` pillar 2 is about and what nothing could do until D-116. The
/// earlier checks exercise the kernel's side through the shared page, which is
/// the right boundary to test the *kernel* at; this is the other half, and it
/// is the one that says the ABI is usable.
///
/// **Nothing here is transcribed.** The submission read back is the one the
/// program wrote, and the completion is compared against *it* — so a check
/// that agreed with a constant it also chose is not possible. The kernel never
/// posts to a program's ring, so a matching pair can only have come from a
/// program that ran.
///
/// The program dies on any mismatch: it compares the echoed value and the
/// status itself, and every failure path is a `ud2`. So a wrong answer shows
/// up here as a fault rather than as a quiet pass — the check and the program
/// disagree in the same direction.
fn a_program_drives_its_own_ring() -> bool {
    let faults_before = trap::last_user_fault().count;
    let retirements_before = thread::retirements();

    let Ok(spawned) = user::run_for_bootstrap() else {
        return false;
    };

    // It has to load, cross, submit, be served, check the answer, and exit.
    if !runs_until(2000, || thread::retirements() > retirements_before) {
        return false;
    }

    // Read from the *exit record*, not from the program: reclamation has
    // already given its ring's pages back by now, and reading a process the
    // kernel has collected is reading a corpse (D-117). The record is taken
    // at the moment of death, while there is still something to read.
    let Some(rites) = user::last_rites_of(spawned.slot) else {
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

    // It left by calling exit, not by faulting: a program that disagreed with
    // its own answer executes `ud2`, and that would be a fault this boot did
    // not have before.
    let exited = trap::last_user_fault().count == faults_before;

    one_served && asked && answered && exited
}

/// A freshly spawned program holds exactly one handle, and it is a channel.
///
/// `002`'s pillar 1, converted from a sentence into a check (D-111): *a
/// process receives exactly one handle at spawn — its bootstrap channel — and
/// every later handle arrived through a channel.* The table is the process's
/// whole power, so counting it is the strongest form the claim has, and the
/// count is the half that a second grant would break silently.
///
/// The parent's evidence is the other end. It is the only way to reach the
/// child, which is what "no ambient authority" looks like from this side: a
/// spawner that dropped its end could never speak to what it spawned.
fn a_spawned_program_holds_exactly_one_handle() -> bool {
    let retirements_before = thread::retirements();
    // Its own program rather than a shared launcher: every ring-3 check
    // counts crossings, and an extra one from here would be attributed to
    // whichever check happened to look next.
    let Ok(spawned) = user::run_for_bootstrap() else {
        return false;
    };

    // Its table, while it is alive, and again from the record once it is not:
    // the count is the claim, and a program that gained a handle *during* its
    // life would break it just as surely as one born with two (D-111, D-117).
    let Some((entries, kind)) = user::bootstrap_of(spawned.slot) else {
        return false;
    };
    let exactly_one = entries == 1;
    let a_channel = kind == object::ObjectType::Channel;

    // And a ring, at the address every program's is, readable by it. The
    // handle is the process's authority; the ring is its data path, and both
    // are in place before its first instruction (D-113).
    // The kernel's record, not the header page: the program is live in ring 3
    // by now and that page is its to write, so `2 * submissions` on a number
    // it chose overflows and panics the debug kernel (D-115).
    let has_a_ring =
        user::ring_shape_of(spawned.slot).is_some_and(|(version, submissions, completions)| {
            version == ring_proto::VERSION
                && submissions > 0
                && completions >= submissions.saturating_mul(2)
        });
    let mapped = task::address_space_of(spawned.slot).is_some_and(|root| {
        paging::translate_in_root(root, user::RING)
            .is_some_and(|(_, flags)| flags.contains(Flags::USER))
    });

    // The parent's end is live and is the far side of the child's: a message
    // put in here is one the child could read, which is what makes it a
    // bootstrap rather than a decoration.
    let Some(ours) = spawned.channel.as_channel() else {
        return false;
    };
    let paired = !ours.is_orphaned();
    let usable =
        object::Message::new(b"hello, child").is_ok_and(|message| ours.send(message).is_ok());

    // And it leaves with one, having never been given a second. Waited for
    // rather than left running: a program retiring during a later check moves
    // the free-slot count under it, which is a race this check would be
    // creating for its neighbours (D-117).
    let gone = runs_until(2000, || thread::retirements() > retirements_before);
    let died_with_one = user::last_rites_of(spawned.slot).is_some_and(|rites| {
        rites.handles == 1 && rites.bootstrap == Some(object::ObjectType::Channel)
    });

    exactly_one && a_channel && has_a_ring && mapped && paired && usable && gone && died_with_one
}

/// Bytes sent down one end of a channel come out of the other, once.
///
/// The IPC primitive (`002`), and the smallest true thing about it: a message
/// crosses, it crosses in order, and it is gone from the sender's side. The
/// ends are separate objects that name each other weakly, so a send appends to
/// the *peer's* inbox and a receive takes from its own — which is what makes
/// the pipe bidirectional without either end reading its own writes (D-110).
fn a_message_crosses_a_channel_and_is_gone() -> bool {
    let Some((near, far)) = object::channel() else {
        return false;
    };
    let (Some(here), Some(there)) = (near.as_channel(), far.as_channel()) else {
        return false;
    };

    // Nothing arrives at the end that sent it.
    let Ok(first) = object::Message::new(b"one") else {
        return false;
    };
    if here.send(first).is_err() {
        return false;
    }
    let sender_sees_nothing = !here.has_message() && there.has_message();

    // In order, and each exactly once.
    let Ok(second) = object::Message::new(b"two") else {
        return false;
    };
    if here.send(second).is_err() {
        return false;
    }
    let (Some(one), Some(two)) = (there.receive(), there.receive()) else {
        return false;
    };
    let in_order = one.bytes() == b"one" && two.bytes() == b"two";
    let drained = there.receive().is_none();

    // And the other direction works, on the same pair.
    let Ok(back) = object::Message::new(b"back") else {
        return false;
    };
    let returns = there.send(back).is_ok()
        && here
            .receive()
            .is_some_and(|message| message.bytes() == b"back");

    // The queue is bounded, and a full one refuses rather than dropping.
    let mut refused = false;
    for _ in 0..=object::Channel::DEPTH {
        let Ok(message) = object::Message::new(b"fill") else {
            return false;
        };
        if here.send(message) == Err(object::ChannelError::Full) {
            refused = true;
            break;
        }
    }

    sender_sees_nothing && in_order && drained && returns && refused
}

/// A channel end with nothing on the other side refuses, rather than leaking.
///
/// Two ends holding each other strongly is a cycle `Arc` will not collect, so
/// the peer link is weak — and the price of that is that an orphaned end must
/// say so. A send into nothing that reported success would be a message a
/// caller believes was delivered (D-110).
fn an_orphaned_channel_end_refuses() -> bool {
    let Some((near, far)) = object::channel() else {
        return false;
    };
    let Some(here) = near.as_channel() else {
        return false;
    };
    // Something already sent stays readable: closing the far end does not
    // reach into this end's inbox.
    let Ok(message) = object::Message::new(b"before") else {
        return false;
    };
    if far
        .as_channel()
        .is_none_or(|there| there.send(message).is_err())
    {
        return false;
    }

    let before = here.is_orphaned();
    drop(far);

    let orphaned = !before && here.is_orphaned();
    let still_readable = here
        .receive()
        .is_some_and(|message| message.bytes() == b"before");
    let refuses = object::Message::new(b"after")
        .is_ok_and(|message| here.send(message) == Err(object::ChannelError::PeerClosed));

    orphaned && still_readable && refuses
}

/// A handle sent through a channel arrives with the rights both ends declared,
/// and no more — and is gone from the sender.
///
/// Attenuate-on-send (D-110). The rights are declared per field with no
/// default at either end, which is Fuchsia RFC-0028's finding applied: a
/// default has unclear behaviour that is inconsistent across object types.
/// Both ends declare, so neither can be surprised by the other, and the
/// narrower of the two is what is granted.
///
/// The move is the other half. A handle that survived a send would make
/// delegation into duplication, and `002`'s pillar 1 rests on it not.
fn a_handle_crosses_with_the_rights_both_ends_declared() -> bool {
    let sender = HandleTable::new(8);
    let receiver = HandleTable::new(8);

    let object = object::event();
    let Ok(handle) = sender.insert(object, Rights::UNIVERSAL) else {
        return false;
    };

    // The sender narrows to two of the four it holds.
    let offered = Rights::WAIT.union(Rights::INSPECT);
    let Ok((crossing, granted)) = sender.send(handle, offered) else {
        return false;
    };
    let sender_narrowed = granted == offered;

    // Gone from the sender: a move, not a copy.
    let moved = sender.resolve(handle).is_err();

    // The receiver narrows again, and gets the narrower of the two.
    let Ok(arrived) = receiver.accept(crossing, granted, Rights::WAIT) else {
        return false;
    };
    let Ok((_, held)) = receiver.resolve(arrived) else {
        return false;
    };
    let receiver_narrowed = held == Rights::WAIT;

    // Neither end can widen. The receiver asking for more than was offered is
    // refused rather than clamped, because a clamp is a default by another
    // name.
    let widening = HandleTable::new(8);
    let second = object::event();
    let Ok(full) = widening.insert(second, Rights::UNIVERSAL) else {
        return false;
    };
    let Ok((narrow, narrow_rights)) = widening.send(full, Rights::WAIT) else {
        return false;
    };
    let receiver_cannot_widen = matches!(
        receiver.accept(narrow, narrow_rights, Rights::UNIVERSAL),
        Err(HandleError::Denied)
    );

    // Nor can a sender declare more than it holds.
    let third = object::event();
    let Ok(partial) = widening.insert(third, Rights::TRANSFER.union(Rights::WAIT)) else {
        return false;
    };
    let sender_cannot_widen = matches!(
        widening.send(partial, Rights::UNIVERSAL),
        Err(HandleError::Denied)
    );

    sender_narrowed && moved && receiver_narrowed && receiver_cannot_widen && sender_cannot_widen
}

/// A handle whose `TRANSFER` right was stripped cannot be sent, and the
/// refusal happens at the sender.
///
/// One bit, and it is what makes least privilege expressible: a holder can be
/// given authority it may use and may not pass on. Refused at the sender
/// because a receiver policing this would have to know what the sender held,
/// which it cannot — the message carries an object and a set of rights, not a
/// history (D-110).
fn a_handle_without_transfer_is_refused_at_the_sender() -> bool {
    let sender = HandleTable::new(8);
    let object = object::event();

    // Everything except the right to pass it on.
    let held = Rights::UNIVERSAL.difference(Rights::TRANSFER);
    let Ok(handle) = sender.insert(object, held) else {
        return false;
    };
    let refused = matches!(sender.send(handle, held), Err(HandleError::Denied));

    // And the refusal left it where it was: a failed send is not a close.
    let still_there = sender.resolve(handle).is_ok();

    // Stripping `TRANSFER` on the way through is how a holder is given a
    // non-delegatable handle in the first place.
    let receiver = HandleTable::new(8);
    let second = object::event();
    let Ok(sendable) = sender.insert(second, Rights::UNIVERSAL) else {
        return false;
    };
    let Ok((crossing, granted)) = sender.send(sendable, Rights::WAIT.union(Rights::INSPECT)) else {
        return false;
    };
    let Ok(arrived) = receiver.accept(crossing, granted, granted) else {
        return false;
    };
    let cannot_pass_on = matches!(receiver.send(arrived, granted), Err(HandleError::Denied));

    refused && still_there && cannot_pass_on
}

/// A handle sent in a message is the same object at the far end, and the
/// receiver can use it.
///
/// The claim that makes a channel a delegation mechanism rather than a byte
/// pipe: what arrives names the thing the sender named, and the rights that
/// came with it are the rights it works under.
fn a_handle_sent_in_a_message_names_the_same_object() -> bool {
    let Some((near, far)) = object::channel() else {
        return false;
    };
    let (Some(here), Some(there)) = (near.as_channel(), far.as_channel()) else {
        return false;
    };

    let sender = HandleTable::new(8);
    let receiver = HandleTable::new(8);

    // An event the receiver will be told about by handle, and which this
    // thread keeps a reference to so it can signal it afterwards.
    let shared = object::event();
    let watching = Arc::clone(&shared);
    let Ok(handle) = sender.insert(shared, Rights::UNIVERSAL) else {
        return false;
    };

    let Ok((crossing, granted)) = sender.send(handle, Rights::WAIT.union(Rights::INSPECT)) else {
        return false;
    };
    let Ok(mut message) = object::Message::new(b"take this") else {
        return false;
    };
    if message.attach(crossing, granted).is_err() || here.send(message).is_err() {
        return false;
    }

    let Some(mut arrived) = there.receive() else {
        return false;
    };
    let carried = arrived.bytes() == b"take this";
    let handles = arrived.take_handles();
    if handles.len() != 1 {
        return false;
    }
    let (object, offered) = handles.into_iter().next().expect("one handle");
    let Ok(theirs) = receiver.accept(object, offered, offered) else {
        return false;
    };

    // The same object: signalling through this thread's reference is visible
    // through the receiver's handle.
    let Ok((named, held)) = receiver.resolve(theirs) else {
        return false;
    };
    let unsignalled = named.as_event().is_some_and(|event| !event.is_signalled());
    watching.as_event().expect("an event").signal();
    let same_object = named
        .as_event()
        .is_some_and(super::object::Event::is_signalled);

    carried && held == offered && unsignalled && same_object
}

/// A thread blocked on an empty channel wakes with the message, and a message
/// that lands while it is registering is not lost.
///
/// The wait window, on the second object to have one. `Event` learned this the
/// hard way (D-95): the state can only be re-read once a block *returns*, and
/// it returns only on a wake, so a check-then-register loses anything that
/// lands in between. `receive_blocking` registers first and re-reads under the
/// same lock, and this drives it from a spawned thread rather than from here —
/// a predicate that sends while holding the queue's own lock is how the first
/// version of the event check hung the machine.
fn a_blocked_receiver_wakes_with_its_message() -> bool {
    let Some((near, far)) = object::channel() else {
        return false;
    };
    let Some(here) = near.as_channel() else {
        return false;
    };

    *BLOCKED_CHANNEL.lock_leaf() = Some(Arc::clone(&far));
    BLOCKED_RECEIVER.store(0, Ordering::Release);
    let free_before = thread::free_slots();
    if thread::spawn("channel-receiver", block_on_a_channel).is_err() {
        return false;
    }

    // It is off the run queue and quiet, not spinning: the slot is taken and
    // nothing has been received.
    let blocked = yields_until(|| {
        thread::free_slots() < free_before && BLOCKED_RECEIVER.load(Ordering::Acquire) == 0
    });

    let Ok(message) = object::Message::new(b"wake up") else {
        return false;
    };
    if here.send(message).is_err() {
        return false;
    }

    let woke = runs_until(500, || BLOCKED_RECEIVER.load(Ordering::Acquire) == 1);
    let settled = runs_until(500, || thread::free_slots() >= free_before);

    // And the window itself: a waiter arriving when the message is *already*
    // there registers, re-reads under the same lock, and comes straight back.
    // `receive_blocking` cannot reach that state — it takes the message first
    // — so the wait is driven directly, which is what makes a plain `wait`
    // here a thread that never returns rather than a check that passes.
    let Some((window_near, window_far)) = object::channel() else {
        return false;
    };
    let Ok(already) = object::Message::new(b"already here") else {
        return false;
    };
    if window_near
        .as_channel()
        .is_none_or(|end| end.send(already).is_err())
    {
        return false;
    }
    *WINDOW_CHANNEL.lock_leaf() = Some(Arc::clone(&window_far));
    WINDOW_RETURNED.store(0, Ordering::Release);
    if thread::spawn("channel-window", wait_on_a_channel_that_has_one).is_err() {
        return false;
    }
    let rereads = runs_until(500, || WINDOW_RETURNED.load(Ordering::Acquire) == 1);
    let window_settled = runs_until(500, || thread::free_slots() >= free_before);
    drop(WINDOW_CHANNEL.lock_leaf().take());

    // Nothing is left registered, so a later wake cannot name a thread that is
    // no longer waiting here.
    let idle = far
        .as_channel()
        .is_some_and(|there| there.waiters_for_test().is_idle());
    drop(BLOCKED_CHANNEL.lock_leaf().take());

    blocked && woke && settled && idle && rereads && window_settled
}

/// The end the blocked receiver waits on.
static BLOCKED_CHANNEL: SpinLock<Option<object::ObjectRef>> = SpinLock::new(None);

/// One once the receiver has come back with the right message.
static BLOCKED_RECEIVER: AtomicUsize = AtomicUsize::new(0);

/// Block on the channel until something arrives, then report what it was.
fn block_on_a_channel() {
    let Some(end) = BLOCKED_CHANNEL.lock_leaf().clone() else {
        return;
    };
    let Some(channel) = end.as_channel() else {
        return;
    };
    let mut context = context();
    if let Some(message) = channel.receive_blocking(&mut context)
        && message.bytes() == b"wake up"
    {
        BLOCKED_RECEIVER.store(1, Ordering::Release);
    }
}

/// Wait on a channel that **already** has a message, and come back.
///
/// The registration window, driven the way the event's is (D-95, D-110):
/// `receive_blocking` takes the message before it ever waits, so it cannot
/// reach the wait with the condition already true — and that is the one state
/// where `wait_unless` and a plain `wait` differ. This calls the wait
/// directly, so a `wait` that registered without re-reading blocks on a
/// channel nobody will send to again and this thread never returns.
///
/// Run on a spawned thread so that failure is a timeout in the observer rather
/// than a hang in the check.
fn wait_on_a_channel_that_has_one() {
    let Some(end) = WINDOW_CHANNEL.lock_leaf().clone() else {
        return;
    };
    let Some(channel) = end.as_channel() else {
        return;
    };
    let mut context = context();
    channel.wait_for_message(&mut context);
    WINDOW_RETURNED.store(1, Ordering::Release);
}

/// The end the window check waits on.
static WINDOW_CHANNEL: SpinLock<Option<object::ObjectRef>> = SpinLock::new(None);

/// One once the window waiter has come back.
static WINDOW_RETURNED: AtomicUsize = AtomicUsize::new(0);

/// A receiver that is **already blocked** learns that its peer has died.
///
/// `receive_blocking` returns `None` when the far end is gone, and the check
/// beside this one drove that by polling `is_orphaned` without ever blocking —
/// which is the one arrangement that cannot fail. A thread that has blocked
/// re-reads its condition only when something wakes it, and until D-127
/// nothing did: `Channel` had no `Drop`, so an end could die with a waiter on
/// the other side and never say so. That waiter slept for the rest of the
/// boot, holding its slot.
///
/// It is not a hypothetical arrangement. `user::reclaim` drops a dead
/// program's handle table precisely so that "the parent's end learns its peer
/// is gone", and that sentence was false for a parent that was waiting.
///
/// Nothing is ever sent here, so the wake under test is the only thing that
/// can end the wait — which is what makes this red rather than slow when the
/// wake is absent (D-127).
fn a_blocked_receiver_learns_its_peer_is_gone() -> bool {
    let Some((near, far)) = object::channel() else {
        return false;
    };

    *ORPHANED_CHANNEL.lock_leaf() = Some(Arc::clone(&far));
    ORPHANED_RECEIVER.store(0, Ordering::Release);
    let free_before = thread::free_slots();
    if thread::spawn("channel-orphan", block_until_orphaned).is_err() {
        return false;
    }

    // Asleep rather than spinning: its slot is taken and it has reported
    // nothing, which is the state the wake has to reach.
    let blocked = yields_until(|| {
        thread::free_slots() < free_before && ORPHANED_RECEIVER.load(Ordering::Acquire) == 0
    });

    // The last strong reference to the other end: `far` holds only a `Weak`
    // back. This is the death the waiter has to be told about.
    drop(near);

    let learned = runs_until(500, || ORPHANED_RECEIVER.load(Ordering::Acquire) == 1);
    let settled = runs_until(500, || thread::free_slots() >= free_before);
    // And nothing is left registered, so a later wake cannot name a thread
    // that is no longer waiting here.
    let idle = far
        .as_channel()
        .is_some_and(|end| end.waiters_for_test().is_idle());
    drop(ORPHANED_CHANNEL.lock_leaf().take());

    blocked && learned && settled && idle
}

/// The end the orphaned receiver blocks on.
static ORPHANED_CHANNEL: SpinLock<Option<object::ObjectRef>> = SpinLock::new(None);

/// One once that receiver has come back with nothing, which is the answer.
static ORPHANED_RECEIVER: AtomicUsize = AtomicUsize::new(0);

/// Block until a message arrives or the far end dies, and report only the
/// second: nothing is ever sent to this end, so a message would be a bug in
/// the check rather than a pass.
fn block_until_orphaned() {
    let Some(end) = ORPHANED_CHANNEL.lock_leaf().clone() else {
        return;
    };
    let Some(channel) = end.as_channel() else {
        return;
    };
    let mut context = context();
    if channel.receive_blocking(&mut context).is_none() {
        ORPHANED_RECEIVER.store(1, Ordering::Release);
    }
}

/// A handle names its object, and a closed one names nothing.
///
/// The lifetime property the whole model rests on (D-78). A handle kept across
/// a close must not name whatever takes the slot next, and the generation is
/// what turns that use-after-free into a decode failure rather than a silent
/// reference to someone else's object.
fn a_closed_handle_names_nothing() -> bool {
    let table = HandleTable::new(8);
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
    let table = HandleTable::new(4);

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
    let mine = HandleTable::new(4);
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

    // The accidental case is refused too. `theirs` holds nothing, so the only
    // refusals reachable are the bounds check and the generation; naming both
    // is the whole set, which is the `matches!(Err(_) | Err(_) | Err(_))` shape
    // D-95 deleted from this very function. Named singly instead (D-105).
    let stray = matches!(
        theirs.resolve(handle),
        Err(HandleError::Malformed | HandleError::Stale)
    );
    // And *not* by reaching the ownership comparison: `theirs` holds no
    // entries, so a `NotYours` would mean it read an owner out of an empty
    // slot. Naming all three refusals — which an earlier version did — says
    // only "an error", the shape D-95 deleted from this function (D-105).
    let not_by_ownership = theirs.resolve(handle).err() != Some(HandleError::NotYours);
    let still_mine = mine.resolve(handle).is_ok();
    refused && stray && not_by_ownership && still_mine
}

/// Rights are checked, and duplication only ever narrows them.
fn rights_are_checked_and_only_narrow() -> bool {
    let table = HandleTable::new(8);
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
    let table = HandleTable::new(QUOTA);
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

/// The arena grows twice over, and every handle survives both moves.
///
/// Sixteen live handles is the number no boot had ever reached, and it is
/// where `reserve_a_slot` stops being exercised only in its easy case. The
/// first growth hands back a `Vec` with no allocation behind it; the second
/// hands back a real one, and freeing that is an acquisition of `HEAP` —
/// taken, until now, with `ARENA`'s leaf guard still live. The order D-85
/// deleted, rebuilt inside the one function whose doc says it exists to
/// prevent it.
///
/// So the assertions below are half of this check. The other half is
/// [`a_leaf_guard_is_the_last_lock`], which sees the violation only on a boot
/// that walks the path — and no boot did. D-126 records that limit; this is
/// the boot that removes it for this one path (D-128).
fn the_handle_arena_grows_more_than_once() -> bool {
    /// Two reallocations past an empty arena, whose capacity runs 16, 32, 64.
    const HELD: usize = 40;

    let table = HandleTable::new(HELD);
    let mut handles = Vec::new();
    for _ in 0..HELD {
        match table.insert(object::event(), Rights::UNIVERSAL) {
            Ok(handle) => handles.push(handle),
            Err(_) => return false,
        }
    }

    // Every one still names its own object across both moves. A handle is an
    // index into the arena, so relocating the arena is exactly the operation
    // that could invalidate one.
    let all_resolve = handles.iter().all(|handle| table.kind_of(*handle).is_ok());
    let live = table.live() == HELD;
    // And no two name one slot, which a move that lost entries would produce.
    let distinct = handles
        .iter()
        .all(|handle| handles.iter().filter(|other| *other == handle).count() == 1);

    all_resolve && live && distinct
}

/// An object outlives every handle to it, and no longer.
///
/// `Arc` is the refcount (D-78 wants `inc_not_zero` from day one), so this is
/// really a check that the table holds a reference rather than a pointer: two
/// handles keep one object alive, and it dies when the second goes.
fn an_object_lives_exactly_as_long_as_its_handles() -> bool {
    let table = HandleTable::new(8);
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
        let table = HandleTable::new(4);
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
    let table = HandleTable::new(4);
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
    // `>=`, because a user program retiring elsewhere frees a slot too: the
    // property is that *this* thread's slot came back, and another coming back
    // with it does not falsify it (D-117).
    let settled = runs_until(200, || thread::free_slots() >= free_before);
    // As above: the previous value leaves the guard rather than being dropped
    // inside it.
    drop(WAITED_EVENT.lock_leaf().take());

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

/// A second address space has the kernel and nothing of anyone else.
///
/// The shape a process needs (D-104): the higher half shared so a trap from
/// ring 3 has somewhere to land, and the lower half empty, which is the
/// isolation. Two programs' spaces will share the kernel and nothing else.
///
/// The sharing is checked by *physical identity*, not by both walks
/// succeeding: two spaces that each mapped the kernel separately would both
/// resolve and would not be sharing. The same physical frame at the same
/// virtual address is what sharing means.
///
/// And the emptiness is checked at an address the live space *does* have a
/// mapping for — the boundary probe's page, which the kernel's own space
/// carries. A fresh space that lacked it would be indistinguishable from one
/// that lacked everything, and this way the difference between the halves is
/// what the check sees. (It used to be the running program's code page; since
/// D-106 no program is in the running space.)
fn a_second_space_holds_the_kernel_and_nothing_else() -> bool {
    let Ok(space) = AddressSpace::for_user() else {
        return false;
    };

    // Everything the kernel needs on the way in, at the same frames.
    let text = boot::sections().text.start;
    let Some(stack_layout) = thread::layout() else {
        return false;
    };
    let Some(slot) = stack_layout.slot(0) else {
        return false;
    };
    let shared = [text, slot.stack_base(), mem::direct_map_base()]
        .into_iter()
        .all(|at| match (paging::translate(at), space.lookup(at)) {
            (Some(here), Some(there)) => here == there,
            _ => false,
        });

    // And nothing of the low half, though the live space has this page.
    let Some((probe, _)) = boundary_probe() else {
        return false;
    };
    let user_is_absent = paging::translate(probe).is_some() && space.lookup(probe).is_none();

    // The half it shares is not reachable from ring 3, whatever the leaves
    // say: a shared top-level entry that carried `USER` would hand every
    // program the kernel.
    let kernel_is_not_user = space
        .lookup(text)
        .is_some_and(|(_, flags)| !flags.contains(Flags::USER));

    shared && user_is_absent && kernel_is_not_user
}

/// A walk reports what the live tables grant, at the three addresses that
/// matter.
///
/// **This is not the check for the narrowing.** The architecture takes `USER`
/// and `WRITABLE` as the conjunction down the whole walk and `NO_EXECUTE` as
/// the disjunction (D-103), and no address space this kernel can build has a
/// parent narrower than its child: `Entry::table` always writes
/// `PRESENT | WRITABLE` and never `NO_EXECUTE`, and `set` now *refuses* a user
/// leaf under a supervisor parent rather than widening one (D-105). So the
/// disagreement the fold exists for is unconstructible here, and an earlier
/// version of this doc claimed to construct it while asserting three equalities
/// that hold with the fold deleted.
///
/// The fold moved to `Flags::through` / `Flags::effective_for` in
/// `libs/x86-paging`, where `cargo test` can hand it a hierarchy the kernel
/// will not build. What is left here is the machine's half: the three
/// addresses report what the tables were asked for, end to end.
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
/// Six ways to ask for something that is not a run of owned four-kilobyte
/// pages, and six refusals. The direct map is the interesting one: it is
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
/// One page of described memory, the `index`-th of a run the map offers.
///
/// The four checks below each named `0x1000` and got away with it on QEMU. On
/// VMware that page is not memory, so `describe` answered `None`, every one of
/// them returned `false` from its setup, and four checks that never ran looked
/// exactly like four properties that failed (D-177).
fn probe_page(index: u64) -> Option<BootRegion> {
    let base = mm::first_memory_run(PROBE_PAGES * PAGE_SIZE)?;
    BootRegion::describe(base.checked_add(index * PAGE_SIZE)?, PAGE_SIZE)
}

/// How many pages [`probe_page`] hands out, and the run it asks for.
const PROBE_PAGES: u64 = 2;

/// The map offers the run the probes below are cut from.
///
/// Named separately so that a machine which cannot supply one says so once,
/// rather than reddening every check that wanted a page from it (D-177).
fn the_map_offers_a_run_to_probe() -> bool {
    probe_page(0).is_some() && probe_page(PROBE_PAGES - 1).is_some()
}

fn a_differing_remap_is_refused() -> bool {
    /// Canonical, and far from anything a fresh space would map.
    const PROBE: u64 = 0xffff_8800_0000_0000;

    let Ok(mut space) = AddressSpace::new() else {
        return false;
    };
    let (Some(here), Some(again_here), Some(elsewhere)) =
        (probe_page(0), probe_page(0), probe_page(1))
    else {
        return false;
    };
    let first = space.map_boot(PROBE, here, Flags::NO_EXECUTE);
    let again = space.map_boot(PROBE, again_here, Flags::NO_EXECUTE);
    let differing = space.map_boot(PROBE, elsewhere, Flags::NO_EXECUTE);
    first.is_ok() && again.is_ok() && differing == Err(MapError::Conflict(PROBE))
}

/// A described frame cannot be laundered into the allocator by claiming it.
///
/// `Flags::OWNED` is public in `libs/x86-paging` and nameable from here, and
/// `unmap` consults exactly that bit before minting a `Segment` and handing
/// the frames to the allocator. If `map_boot` passed caller flags through,
/// this safe sequence would produce a second owner of a frame the kernel is
/// standing on — a use-after-free with no `unsafe` anywhere (D-105).
fn a_boot_mapping_cannot_claim_ownership() -> bool {
    /// Canonical, and far from anything a fresh space maps.
    const PROBE: u64 = 0xffff_8880_0000_0000;

    let Ok(mut space) = AddressSpace::new() else {
        return false;
    };
    let Some(region) = probe_page(0) else {
        return false;
    };
    if space
        .map_boot(PROBE, region, Flags::NO_EXECUTE.union(Flags::OWNED))
        .is_err()
    {
        return false;
    }
    let before = mm::free_frames();
    // The bit was stripped, so `unmap` sees a mapping nothing owns and refuses
    // it — rather than minting a `Segment` over the boot handoff's frame and
    // handing it to the allocator, which would be a second owner of a frame
    // the kernel is standing on, from safe code.
    let refused = space.unmap(PROBE, PAGE_SIZE).err() == Some(MapError::NotOwned(PROBE));
    refused && mm::free_frames() == before
}

/// A space that shares the kernel's higher half cannot write to it.
///
/// `for_user` copies the kernel's own PML4 entries rather than duplicating the
/// tables below them, so every higher-half mapping operation on such a space
/// walks into the live kernel's tables. Unmapping a kernel address through one
/// would blank a running thread's stack and free its frames (D-105).
fn a_user_space_cannot_reach_the_kernel_half() -> bool {
    /// Inside the direct map, so the walk would find real entries to blank.
    const KERNEL: u64 = 0xffff_8000_0010_0000;
    /// The last page below the user/supervisor boundary.
    const LAST_USER: u64 = 0x0000_7fff_ffff_f000;

    let Ok(mut user) = AddressSpace::for_user() else {
        return false;
    };
    let Some(region) = probe_page(0) else {
        return false;
    };
    let unmapped = user.unmap(KERNEL, PAGE_SIZE);
    let mapped = user.map_boot(KERNEL, region, Flags::NO_EXECUTE);
    // The refusal is about the half, not about mapping: the last user page is
    // still reachable, and a range that straddles the boundary is not.
    // A range that runs off the top of the user half is refused too, though
    // not by `may_reach`: the boundary is also where the canonical hole
    // begins, so a straddle is non-canonical and stopped one check earlier.
    // Named rather than folded into the others, because "refused" and
    // "refused for the reason this check is about" are different claims.
    let straddles =
        user.unmap(LAST_USER, 2 * PAGE_SIZE).err() == Some(MapError::NonCanonical(LAST_USER));
    // And the refusal is about the *half*, not about everything: the last page
    // below the boundary is refused for having nothing mapped in it, which is
    // a different answer.
    let reaches_its_own =
        user.unmap(LAST_USER, PAGE_SIZE).err() != Some(MapError::NotYours(LAST_USER));

    unmapped.err() == Some(MapError::NotYours(KERNEL))
        && mapped == Err(MapError::NotYours(KERNEL))
        && straddles
        && reaches_its_own
}

/// A user mapping cannot widen an interior table the kernel is under.
///
/// An entry above the leaf covers everything below it, so upgrading a
/// supervisor table to `USER` to make room for one user page would hand ring 3
/// every mapping already under it. The refusal is what makes `set`'s
/// "a present entry accepts only an identical rewrite" true (D-105).
fn a_user_mapping_cannot_widen_a_supervisor_table() -> bool {
    /// Two addresses one page apart, so they share every table above the leaf.
    const SUPERVISOR: u64 = 0x0000_0000_4000_0000;
    const USER: u64 = SUPERVISOR + PAGE_SIZE;

    let Ok(mut space) = AddressSpace::new() else {
        return false;
    };
    let (Some(first), Some(second)) = (probe_page(0), probe_page(1)) else {
        return false;
    };
    if space
        .map_boot(SUPERVISOR, first, Flags::NO_EXECUTE)
        .is_err()
    {
        return false;
    }
    space.map_boot(USER, second, Flags::NO_EXECUTE.union(Flags::USER))
        == Err(MapError::Conflict(USER))
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
    // The margin, not `home_x <= start_x`: that comparison holds for every
    // reset, because `cursor_x` never goes below the margin and `start_x` is
    // read after one. The margin is where a line starts, and the one place
    // `erase_previous` refuses to go below (D-105).
    let started_a_line =
        home_x == crate::console::MARGIN && home_x <= typed_x && home_y == start_y + line_height;

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

    dev::routing_of(gsi)
        .is_some_and(|routing| !routing.masked && routing.vector == vector::KEYBOARD)
}

/// The controller refuses an interrupt it does not route.
///
/// Below its base the line arithmetic wraps, and in release it wrapped
/// silently into an unrelated register.
fn the_io_apic_refuses_a_line_it_does_not_own() -> bool {
    let Some(base) = dev::io_apic_base() else {
        return false;
    };
    // Past the end, which every controller has.
    let past_the_end = dev::routing_of(u32::MAX).is_none();
    // And below the base, which only a controller whose base is above zero
    // has. The old version asked about `keyboard_gsi() - 1` and accepted
    // "one of the two refused", which on this machine — base zero, keyboard on
    // line one — meant the wrap-below arithmetic it names was never reached
    // and the check could not have noticed if it broke (D-105, D-121).
    let below_the_base = base == 0 || dev::routing_of(base - 1).is_none();
    // And the line it *does* own is routed, so this is not the answer "no" to
    // every question.
    let owns_its_own = dev::routing_of(dev::keyboard_gsi()).is_some();

    past_the_end && below_the_base && owns_its_own
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
    let balanced = taken && after.live == before.live && after.slab_frames == before.slab_frames;

    // **And a slot is actually reused**, which nothing above shows: deleting
    // the line that puts a freed slot back on the free list — so every free
    // leaks — left all of this green. `live` is decremented either way, and
    // `slab_frames` cannot move because the class was warmed.
    //
    // The first version of this counted frames, and could not fail either: a
    // slab frame carves hundreds of slots, so leaking a few dozen never
    // reaches a refill. The address is the direct signal — the free list is
    // LIFO, so a freed slot is the *same* slot the next allocation of that
    // class gets, and a leak makes it a different one (D-124).
    //
    // `black_box` on each is load-bearing and was found the hard way: without
    // it LLVM elided the pair outright — the second address equalled the first
    // because neither allocation happened — and the check went red with the
    // heap correct. It escapes the pointer, so both calls must occur and in
    // this order.
    let first = core::hint::black_box(Box::new(0u32));
    let address = core::hint::black_box(core::ptr::from_ref(&*first) as usize);
    drop(first);
    let second = core::hint::black_box(Box::new(0u32));
    let reused = core::hint::black_box(core::ptr::from_ref(&*second) as usize) == address;
    drop(second);

    balanced && reused
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

/// Nothing acquired a second lock while a leaf guard was live.
///
/// `lock_leaf`'s whole obligation is one sentence — **nothing is called while
/// this guard is live** — and a second acquisition is a call. The asymmetry
/// matters and the first version of this check got it backwards: a leaf taken
/// *under* a token-bearing lock is the design, and is what makes D-47's
/// `HEAP` → `FRAMES` order sound and what lets `Segment::drop` reach the
/// allocator from wherever it dies. It is the other direction that is the
/// violation.
///
/// Nothing checked it, and `pic::silence` was in breach: it held
/// `CONTROLLERS`' leaf guard and claimed ports under it, which takes
/// `CLAIMED`'s — establishing a `CONTROLLERS → CLAIMED` order nothing else in
/// the kernel knew about, which is the kind that closes a cycle later (D-121,
/// D-124).
///
/// Reports *where*, because a boolean here would send the next reader looking
/// through every `lock_leaf` in the tree — which is how this one was found,
/// and it took an instrumented boot.
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

/// A scroll does not carry the cursor up the screen with it.
///
/// The cursor is an underline drawn on the framebuffer like any other mark,
/// and `set_cursor`'s doc says every write takes it down first "which is what
/// keeps it from being scrolled up the screen as a stray mark". `write_line`
/// did not: it put the cursor back and *then* called `newline`, so the scroll
/// that follows copied the underline up. Every log line goes through
/// `write_line`, so it happened on every scroll (D-137).
///
/// Driven by writing real lines rather than by calling `newline` directly:
/// the defect is in how the two are ordered, so a check that ordered them
/// itself would be testing its own arrangement.
fn a_scroll_leaves_no_cursor_behind() -> bool {
    /// Where to look: the left margin, which is where a cursor sits after a
    /// line break, on a row a scroll will have moved something into.
    const AT_X: usize = crate::console::MARGIN;

    let Some((_, _, line_height)) = dev::console_probe(AT_X, 0) else {
        // No console came up; nothing to say about its scrolling.
        return true;
    };
    let Some(geometry) = dev::console_geometry() else {
        return true;
    };

    // The row a full screen of lines will have scrolled a cursor up into.
    // Two cells above the bottom, so it is a row the scroll wrote and not the
    // one the live cursor is on.
    let rows = (geometry.height - crate::console::MARGIN) / line_height;
    let probe_y = crate::console::MARGIN + (rows.saturating_sub(3)) * line_height + line_height - 1;

    // A screenful and a half, so the row above has certainly been scrolled
    // through. Blank text, so the only foreground pixel that could land on
    // that row is a cursor: a glyph would put marks there legitimately.
    for _ in 0..rows * 2 {
        dev::with_console(|console| console.write_line(""));
    }

    let Some((pixel, foreground, _)) = dev::console_probe(AT_X, probe_y) else {
        return false;
    };

    // The underline is drawn in the foreground colour. Anything else on a row
    // of blank lines means a cursor rode the scroll up.
    pixel != foreground
}

/// A stack whose mapping is taken away stops being one the CPU can stand on.
///
/// `MappedStack` is evidence dated at construction, and until D-125 that was
/// all `is_installed` looked at: the root still matched after an unmap, so the
/// value happily outlived the mapping it stood for — and `set_double_fault_
/// stack` takes it as proof enough to point the CPU's most privileged handler
/// somewhere. This is the state that could not be reached before, made
/// reachable on purpose (D-94's open item).
fn a_stack_stops_being_installed_when_it_is_unmapped() -> bool {
    let mut context = context();

    // Above every stack the kernel maps, so nothing else is in this window.
    let Some(above) = thread::boot_stack_layout()
        .and_then(|boot| StackLayout::new(boot.end(), PAGE_SIZE, PAGE_SIZE, 1))
    else {
        return false;
    };
    let Some(slot) = above.slot(0) else {
        return false;
    };
    let Some(Ok(stack)) = mem::with_active_space(&mut context, |space| space.map_stack(&slot))
    else {
        return false;
    };

    let installed = stack.is_installed();

    // Taken away underneath it. The value is untouched and still names the
    // running root, which is exactly what made the old answer wrong.
    let unmapped = mem::with_active_space(&mut context, |space| {
        space.unmap(slot.stack_base(), slot.stack_len())
    });
    let Some(Ok(taken)) = unmapped else {
        return false;
    };
    let gone = !stack.is_installed();
    drop(taken.flush());

    installed && gone
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
    // `spawn` re-arms the timer and restores the interrupt flag before it
    // returns, so an expiry landing here can run `leave_at_once` to completion
    // and give the slot back before this line reads it — red with nothing
    // wrong. The property is that a slot was taken *and* came back, which
    // either ordering satisfies (D-105).
    let taken = thread::free_slots() < free || thread::free_slots() == free;
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

/// A wake from a handler arms the timer for what it woke.
///
/// `thread.rs` states the rule without qualification — every path that changes
/// who is runnable ends by arming the timer — and `unblock` exempted trap
/// context, because the trap exit reschedules (D-54). That holds for the timer
/// and keyboard arms, which both end in `thread::reschedule`. It does not hold
/// for the syscall arm, which serves the ring and returns to ring 3: a
/// `ring_enter` that wakes a waiter left it runnable with nothing armed for it.
/// The premise was in `thread.rs` and the arms are in `trap.rs`, and nothing
/// compared them (D-133).
///
/// **The assertion is about the arming, not the running.** A woken thread runs
/// eventually whatever happens here — this checking thread's own yields and
/// sleeps arm timers, and any of them will pick it up — so a check that waited
/// for it to run would pass with the exemption in place. That is the shape
/// D-124 kept finding, and it is why this reads a counter.
///
/// The blocked reader is the machine's only wake from a handler, so the check
/// above establishing that it happened is what makes a zero here mean "one did
/// and armed nothing" rather than "none ever happened".
fn a_wake_from_a_handler_armed_the_timer() -> bool {
    READER_SAW.load(Ordering::Acquire) != 0 && thread::handler_arms() > 0
}

/// The polled keyboard fallback drains, sleeps and stops.
///
/// D-37's path had never run: both test machines have an I/O APIC. Started
/// here for a bounded number of rounds, on the code the shipped fallback runs.
/// It must make progress *and* leave, so the exit is half the claim.
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

    // **And it slept between them**, which the count alone does not show: a
    // poller that spun would run its four rounds and exit just as fast, and
    // every conjunct above would hold. Each sleep arms a timer, so the
    // expiries are the poller's own — one of them is this thread's sleep, so
    // the bound is the rounds rather than the rounds plus one (D-124).
    let slept = time::expiries() - expiries_before >= ROUNDS as u64;

    passes == ROUNDS && exited && slept
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
///
/// **Changing this number is the moment a mutation is owed** (`AGENTS.md`,
/// D-119). Five checks landed in one day without one, three of them here, by
/// sessions that were citing D-119 while writing them. The rule was not
/// forgotten so much as never *reached*: adding a check happens inside a fix,
/// and `xtask/src/mutate.rs` is a different file nothing on that path makes you
/// open. This line and its twin in `xtask/src/qemu.rs` are the two a hand has
/// to touch either way, so the reminder lives on them rather than in a resolve
/// to be more careful.
const EXPECTED_CHECKS: usize = 187;

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
        "a closed channel still yields what it was sent",
        a_closed_channel_still_yields_what_it_was_sent(),
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
        "a stale root authorises no walk",
        a_stale_root_authorises_no_walk(),
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
        "a program cannot aim at the copy window",
        a_program_cannot_aim_at_the_copy_window(),
    );
    report.check(
        "two spaces map one address to different memory",
        two_spaces_map_one_address_to_different_memory(),
    );
    report.check(
        "a vector register survives a switch",
        a_vector_register_survives_a_switch(),
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
        "a read is a submission with its own offset",
        a_read_is_a_submission_with_its_own_offset(),
    );
    report.check(
        "a ring operation cannot reach past its rights or its pages",
        a_ring_operation_cannot_reach_past_its_rights_or_its_pages(),
    );
    report.check(
        "a second backend answers the same three operations",
        a_second_backend_answers_the_same_three_operations(),
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
        "a promise outlives the program that made it",
        a_promise_outlives_the_program_that_made_it(),
    );
    report.check(
        "the time page states the clock the kernel measured",
        the_time_page_states_the_clock_the_kernel_measured(),
    );
    report.check(
        "the time page and the call tell the same time",
        the_time_page_and_the_call_tell_the_same_time(),
    );
    report.check(
        "revoking the clock sends readers back to the call",
        revoking_the_clock_sends_readers_back_to_the_call(),
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
        "a message crosses a channel and is gone",
        a_message_crosses_a_channel_and_is_gone(),
    );
    report.check(
        "an orphaned channel end refuses",
        an_orphaned_channel_end_refuses(),
    );
    report.check(
        "a handle crosses with the rights both ends declared",
        a_handle_crosses_with_the_rights_both_ends_declared(),
    );
    report.check(
        "a handle without transfer is refused at the sender",
        a_handle_without_transfer_is_refused_at_the_sender(),
    );
    report.check(
        "a handle sent in a message names the same object",
        a_handle_sent_in_a_message_names_the_same_object(),
    );
    report.check(
        "a blocked receiver wakes with its message",
        a_blocked_receiver_wakes_with_its_message(),
    );
    report.check(
        "a blocked receiver learns its peer is gone",
        a_blocked_receiver_learns_its_peer_is_gone(),
    );
    report.check(
        "kernel memory cannot be published to ring three",
        kernel_memory_cannot_be_published_to_ring_three(),
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
        "a bound space is not reclaimed",
        a_bound_space_is_not_reclaimed(),
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
        "an unserved call is refused without stopping the machine",
        an_unserved_call_is_refused_without_stopping_the_machine(),
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
        "the handle arena grows more than once",
        the_handle_arena_grows_more_than_once(),
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
        "a user space cannot reach the kernel half",
        a_user_space_cannot_reach_the_kernel_half(),
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
        "the console has a shadow to scroll from",
        dev::console_has_back_buffer(),
    );

    report.check("threads are available", thread::is_available());
    report.check(
        "an unmapped stack is no longer installed",
        a_stack_stops_being_installed_when_it_is_unmapped(),
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
    report.check(
        "every refusal the clock can record reads back as itself",
        time::every_refusal_reads_back_as_itself(),
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
        "the polled keyboard fallback runs its rounds, sleeps and stops",
        the_polled_fallback_runs_its_rounds_and_sleeps(),
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
    // The identity first, because it decides what the set query is worth. What
    // it answered rather than merely that it disagreed: `None` is a controller
    // that never acknowledged, and a number is one that answers differently
    // (D-176).
    let identity = input::identity();
    let translated = identity == Some(TRANSLATED_MF2);
    if !translated && identity != Some(UNTRANSLATED_MF2) {
        warn!("keyboard: identifies as {identity:?}, which names no known keyboard");
    }
    report.check(
        "the keyboard identifies as one the decoder knows",
        translated || identity == Some(UNTRANSLATED_MF2),
    );

    // **The set query is only readable when nothing rewrites it.** Behind a
    // translating controller this reply is translated like every other, so
    // asserting it would be asserting a fact about the controller's rewriting
    // rather than about the keyboard — VMware answers `0x41` here. What is a
    // claim on every machine is that the numbering arriving and the numbering
    // the decoder reads are the same one (D-178, D-179).
    let set = input::scancode_set();
    if !translated && set != Some(2) {
        warn!("keyboard: reports scancode set {set:?}, not 2");
    }
    report.check(
        "the keyboard's numbering and the decoder's are the same",
        input::decoder_reads_what_arrives() && (translated || set == Some(2)),
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
    report.check(
        "a wake from a handler arms the timer for what it woke",
        a_wake_from_a_handler_armed_the_timer(),
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

    // **The set 1 path, on a controller made to behave like VMware's.** QEMU
    // honours the translation bit, so turning it on produces the exact stream
    // that machine delivers unasked — and here the injected key's identity is
    // the harness's choice rather than a guess, which is the only way to check
    // a translated keystroke without a human at the machine (D-179).
    report.check(
        "the controller accepts translation being turned on",
        input::set_translation(true),
    );
    report.check(
        "a translating controller rewrites the identity too",
        input::identity() == Some(TRANSLATED_MF2),
    );
    // Kept in step with TRANSLATED_MARKER in `xtask/src/qemu.rs`, and subject
    // to the same rule as the first: nothing may talk to the keyboard between
    // the marker and the wait.
    info!("selftest: awaiting translated keystroke");
    // `B`, not `A`: the first round pressed `A`, and a wait that a stale event
    // can satisfy is a wait that never reads the table it is about. This check
    // was green with set 1's code for `A` deliberately wrong until the key
    // changed (D-155, D-165).
    report.check(
        "a translated keystroke decodes to the same key",
        input::await_key(ps2_keys::Key::B),
    );
    // Turned back off, and the decoder re-derived from what the machine
    // actually does rather than from what was written — which on a machine
    // that translates regardless leaves the translated decoder in place, and
    // is the whole of D-178's rule as a check.
    let restored = input::set_translation(false);
    report.check(
        "the decoder reads the set that arrives, not the bit that was written",
        restored && input::decoder_reads_what_arrives(),
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

    // **Last, and deliberately**, for a different reason from the scroll check
    // below: this one *reads* a value every other check can change. It sat at
    // position eighty of a hundred and forty-three, so a violation in any of
    // the sixty checks after it — `input.rs` alone has twelve `lock_leaf`
    // sites — happened after the only read and was never reported. A detector
    // is worth what it is read at the end of (D-143).
    report.check(
        "no lock is taken while a leaf guard is held",
        a_leaf_guard_is_the_last_lock(),
    );

    // **Last, and deliberately.** This one writes a screenful of blank lines
    // to make a region whose only possible foreground mark is a cursor — which
    // scrolls the console and moves what every earlier console check is
    // looking at. Placed here it disturbs nothing; placed where it was, it
    // turned the editing check red (D-137).
    report.check(
        "a scroll leaves no cursor behind",
        a_scroll_leaves_no_cursor_behind(),
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

/// A row must be wide enough to hold the pixels the bootloader says are in it,
/// and the last of them must land inside the mapping the kernel made.
///
/// The four conjuncts about the handoff are a precondition and pass on every
/// machine this has run on. The fifth is about *this kernel's* arithmetic: the
/// byte offset of the bottom-right pixel, computed the way `write_pixel`
/// computes it, against the size the framebuffer was mapped with. A pitch or a
/// depth misread by the glue puts that offset outside the region, and nothing
/// above would say so (D-124).
fn geometry_is_sane(geometry: FramebufferInfo) -> bool {
    let described = geometry.width > 0
        && geometry.height > 0
        && geometry.bytes_per_pixel > 0
        && geometry.pitch >= geometry.width * geometry.bytes_per_pixel;

    let last = (geometry.height - 1) * geometry.pitch + geometry.width * geometry.bytes_per_pixel;
    described && dev::framebuffer_bytes().is_some_and(|mapped| last <= mapped)
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

    let disjoint = red != 0
        && green != 0
        && blue != 0
        && red & green == 0
        && green & blue == 0
        && red & blue == 0;

    // And the three compose the way the format says they do, which disjoint
    // masks alone do not show: a `pack` that dropped a channel, or shifted one
    // into the pad bits, satisfies every conjunct above (D-124).
    let white = pack(u8::MAX, u8::MAX, u8::MAX) == red | green | blue;
    let black = pack(0, 0, 0) == 0;

    disjoint && white && black
}

/// COM1 is already owned by the serial driver, so a second claim must be
/// refused — the whole block, and any single port inside it.
fn ports_are_exclusive() -> bool {
    port::claim_block::<8>(uart::COM1).is_none() && port::claim::<u8>(uart::COM1).is_none()
}

/// A closed channel still yields what it was sent.
///
/// `receive_blocking`'s doc says `None` means "the far end closed with nothing
/// left to read". It tested emptiness and orphanhood in two separate lock
/// acquisitions with interrupts on between them, so a peer that sent *and
/// then* closed in that window got `None` with its message sitting in the
/// queue — a lost message, reported as a clean close (D-145).
///
/// **What this pins is the ordering, not the window.** Draining before
/// refusing goes red here — test orphanhood first and the message is lost.
/// The window itself is not reproduced and cannot be single-threaded: it needs
/// a send to land *between* two lock acquisitions, so the fixed line is
/// reachable only from a second thread. Removing that line alone leaves this
/// green, because the take at the top of the loop finds the message. Stated
/// rather than implied (D-145).
fn a_closed_channel_still_yields_what_it_was_sent() -> bool {
    let Some((near, far)) = object::channel() else {
        return false;
    };
    let Some(channel) = near.as_channel() else {
        return false;
    };

    // Sent, then the peer closed. Both orderings of the fix's concern end in
    // this state: orphaned, with a message.
    let Some(peer) = far.as_channel() else {
        return false;
    };
    let Ok(message) = object::Message::new(b"last words") else {
        return false;
    };
    if peer.send(message).is_err() {
        return false;
    }
    drop(far);

    let orphaned = channel.is_orphaned();
    let mut context = context();
    let yielded = channel.receive_blocking(&mut context).is_some();
    // And now it really is empty, so the second answer is the honest `None`.
    let then_closed = channel.receive_blocking(&mut context).is_none();

    orphaned && yielded && then_closed
}

/// A save writes only inside the area the kernel sized for it.
///
/// `bytes()` is computed for whichever form `init` chose, and `save`
/// dispatches on a separate word. Nothing read the second one — `main.rs`
/// prints ", compacted" from a `CPUID` bool, not from the dispatch — so
/// storing the uncompacted form with a compacted size would have gone on
/// announcing the right thing while writing **past the end of every save area
/// on the machine**, each of which lives inside a thread's slot (D-144).
///
/// Checked by consequence: a sentinel past the area must survive the save.
///
/// **Not by asserting the form**, and the first version of this did. QEMU's
/// `-cpu max` reports no `XSAVEC`, so the gate runs the *uncompacted* path;
/// `AGENTS.md` names the compacted one as something only the VMware boot
/// reaches. A check requiring "compacted" is red on one machine and green on
/// the other, which is a check about the hardware. The sentinel is meaningful
/// on both — and since D-135 put the self-test into the VMware image, this is
/// the first thing that will exercise the compacted path with a check
/// watching.
fn a_save_stays_inside_its_area() -> bool {
    if !xstate::is_enabled() {
        // Nothing to save, and nothing to overflow.
        return xstate::bytes() == 0;
    }
    // The form is whatever the processor supports; what must hold on every
    // processor is that the area was sized for the form in use.
    xstate::save_stays_inside_its_area()
}

/// Filling the arena never hands the same slot out twice.
///
/// `Arena::take` used to find a free slot, advance the hint and return the
/// index **without marking it**; `insert` then dropped the guard and
/// re-acquired it to write the entry. Two callers interleaving in that window
/// both got the same index, and the second write overwrote a live entry,
/// dropping an `ObjectRef` under the leaf guard — which can reach
/// `Channel::drop` and then `THREADS` (D-141).
///
/// **This check does not reproduce that interleaving**, and nothing here can:
/// it needs two threads inside a window between two lock acquisitions, and a
/// check that spawned racers would be flaky rather than a check. What it does
/// pin is the deterministic half — every handle a table hands out names a
/// distinct live slot, and one inserted earlier still resolves after later
/// ones — which is the invariant an overwrite breaks when it does happen.
fn filling_the_arena_never_repeats_a_slot() -> bool {
    /// Enough to wrap the hint past the end and force a grow or two.
    const HANDLES: usize = 24;

    let table = HandleTable::new(HANDLES);
    let mut handles = [0u32; HANDLES];

    for slot in &mut handles {
        let Ok(handle) = table.insert(object::event(), Rights::UNIVERSAL) else {
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
    // slot whose entry belongs to someone else, which `resolve` reports as
    // `NotYours` or `Stale` rather than as the object it was given.
    let all_live = handles
        .iter()
        .all(|handle| table.kind_of(*handle) == Ok(ObjectType::Event));

    let counted = table.live() == HANDLES;

    distinct && all_live && counted
}

/// A root outlives its thread as a value and not as authority.
///
/// `Root` is unforgeable — only `task::address_space_of` mints one — and that
/// was taken to settle it. It does not: a caller can hold one across the
/// thread's exit, and `collect` then reclaims the space and returns the root
/// frame to the allocator, which may hand it to a *program*. The walk would
/// read ring-3-authored bytes as top-level page-table entries, which is the
/// arbitrary physical read the type exists to prevent, reached by staleness
/// rather than forgery (D-140).
///
/// Counted rather than observed, for D-139's reason: a stale root usually
/// walks frames that read as nothing, so the answer is `None` either way.
fn a_stale_root_authorises_no_walk() -> bool {
    let Ok(spawned) = user::run_for_bootstrap() else {
        return false;
    };
    let Some(root) = task::address_space_of(spawned.slot) else {
        return false;
    };

    // Alive: the walk is authorised, and the kernel half is mapped in every
    // space, so this is a translation that really resolves.
    let live = root.is_live();
    let answered = paging::translate_in_root(root, mem::direct_map_base()).is_some();

    // Let it run to completion and be collected.
    if !runs_until(2000, || task::address_space_of(spawned.slot).is_none()) {
        return false;
    }
    user::collect();

    let refusals = paging::stale_root_refusals();
    let dead = !root.is_live();
    let refused = paging::translate_in_root(root, mem::direct_map_base()).is_none();
    let counted = paging::stale_root_refusals() == refusals + 1;

    live && answered && dead && refused && counted
}

/// `syscall` and `sysenter` are shut, **by this kernel**.
///
/// Observing that they are shut proves nothing: Limine leaves `EFER.SCE` clear
/// and `IA32_SYSENTER_CS` is zero at reset, so the register reads shut on both
/// test machines whether or not `enable_protections` ever wrote anything. The
/// first version of this check asserted the state and stayed green with the
/// write deleted (D-139).
///
/// So it is driven: open a door, confirm the reader sees it open, shut it
/// through the kernel's own path, and confirm it closed. The middle step is
/// what proves the reader is not simply answering `true`.
///
/// **What this does and does not establish.** The *shutting* is checked —
/// break `shut_the_fast_syscall_doors` and the last conjunct goes red. Whether
/// `enable_protections` **calls** it at boot is **not** falsifiable on either
/// test machine, because Limine leaves `SCE` clear and `SYSENTER_CS` is zero
/// at reset: `shut_at_boot` reads true with the call deleted. That conjunct is
/// a statement about the machine the kernel booted on, not about the kernel,
/// and it is kept because it is the thing a *different* firmware would break.
/// Said plainly rather than left for the next audit to find (D-139).
///
/// Safe to open for those few instructions: no program can be entered in
/// between — this runs on a kernel thread, and `enter_user` refuses while
/// either door is open, so a failure here stops programs rather than exposing
/// one.
fn the_fast_syscall_doors_are_shut_by_this_kernel() -> bool {
    let shut_at_boot = cpu::fast_syscall_doors_are_shut();

    cpu::open_fast_syscall_doors_for_test();
    let reader_sees_it = !cpu::fast_syscall_doors_are_shut();

    cpu::shut_fast_syscall_doors();
    let shut_again = cpu::fast_syscall_doors_are_shut();

    shut_at_boot && reader_sees_it && shut_again
}

/// A reclaimed address space answers nothing and binds to nothing.
///
/// `is_reclaimed`'s own doc says "a reclaimed space answers nothing and maps
/// nothing", and until D-139 only the *mapping* doors checked it. `reclaim`
/// empties a space by setting `self.root = 0`, so `lookup` walked **physical
/// frame zero** as a top-level table — any word there with bit 0 set sends the
/// next level to an arbitrary address. `bind` read the same frame and then
/// installed it.
///
/// And `reclaim` was not idempotent: a second call on a never-bound space
/// reached `Frame::from_raw(0)` and handed frame zero to the allocator, which
/// is plausibly the kernel's own top-level table.
fn a_reclaimed_space_answers_nothing() -> bool {
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };

    // Something to look up, so the `None` below is about the reclaim and not
    // about an empty space.
    let Some(mut page) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !page.zero() {
        return false;
    }
    let at = user::STACK + 0xB0_0000;
    if space
        .map_user(at, page.into(), Flags::WRITABLE.union(Flags::NO_EXECUTE))
        .is_err()
    {
        return false;
    }
    let answered_before = space.lookup(at).is_some();

    // Never bound, so this is the path the second call used to reach
    // `Frame::from_raw(0)` on.
    let reclaimed = space.reclaim(None).is_some();
    let empty = space.is_reclaimed();

    // **Counted, not observed.** Frame zero reads as zeros on both test
    // machines, so an unguarded `lookup` walks it, finds nothing present, and
    // returns the same `None` the guard returns — a check on the *answer* is
    // green with the guard deleted. The tally lives inside each guard, so it
    // cannot move without them (D-139).
    let refusals = paging::reclaimed_refusals();

    let silent = space.lookup(at).is_none();
    let unbindable = matches!(space.bind(), Err(ActivateError::Reclaimed));
    let once_only = space.reclaim(None).is_none();

    let all_three_refused = paging::reclaimed_refusals() == refusals + 3;

    answered_before && reclaimed && empty && silent && unbindable && once_only && all_three_refused
}

/// A refused pair of claims strands neither.
///
/// A claim in this registry is permanent — nothing releases, because a driver
/// owns its registers for the life of the kernel. So a caller that takes one
/// port and then fails on the second has foreclosed the first for the boot,
/// and whatever needed both can never be built. `i8042::Controller::new`
/// chained `DATA` and `CONTROL` exactly that way, so a refused `CONTROL` would
/// have retired the keyboard (D-138).
///
/// Driven against a port the serial driver already owns, so the refusal is
/// real rather than arranged.
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
    let mut context = context();

    let before = interrupt::are_enabled();
    // The **token-bearing** form for the outer one, and the leaf for the
    // inner. That is the sanctioned direction — a leaf under a `lock` is what
    // makes D-47's `HEAP` → `FRAMES` order sound — and it is what this check
    // needs anyway: the property is that dropping an inner guard does not
    // re-enable interrupts, not that two leaves may nest.
    //
    // Both were leaves before, which is the one direction `lock_leaf` forbids,
    // and the leaf-nesting detector reported it the moment its read moved to
    // the end of the run. A deliberate violation in a check is still a
    // violation, and it would have masked every later real one — the detector
    // keeps the first (D-143).
    let outer_guard = outer.lock(&mut context);
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
///
/// **These bounds catch a rate that is absent or absurd, not one that is
/// merely wrong.** They were once wide enough that a counter rate off by a
/// factor of ten sat inside them. Narrowed to what a machine this kernel will
/// run on can actually report — but the check that catches a plausible wrong
/// rate is `a_sleeper_wakes_on_time_and_quietly`, which arms on one clock and
/// measures on the other, and this one does not pretend to (D-124).
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
    // Seeded with a real reading, not zero. With zero the first comparison is
    // vacuous — no clock reads below it — so the sample taken under a lock,
    // which is the one the retired clock could read backwards on and the whole
    // reason attempt zero is special, was the one sample never checked (D-124).
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

/// Timer expiries taken while nothing at all is waiting for one.
///
/// D-57's claim, and a number that would have been fifty before it. Nothing
/// is runnable but the boot thread and nothing sleeps, so there is nothing to
/// be interrupted *for* — none, not merely fewer.
fn an_idle_kernel_takes_no_timer_interrupts() -> bool {
    /// Fifty times the old tick's period, so a tick that survived would show
    /// up fifty times over. The comment said five; the tick was 100 Hz, and
    /// the line four above already says "a number that would have been fifty".
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
/// An MF2 keyboard's identity, as it reads when nothing rewrites it.
///
/// Behind a translating controller the same keyboard answers `(0xAB, 0x41)`,
/// because translation rewrites this reply exactly as it rewrites the
/// scancodes the decoder is about to read (D-178).
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
