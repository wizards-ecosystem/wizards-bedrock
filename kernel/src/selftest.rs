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
use alloc::vec::Vec;
use core::fmt::Write;
use core::hint::spin_loop;
use core::sync::atomic::{AtomicUsize, Ordering};

use wost::arch::current::cpu::{self, Protections};
use wost::arch::current::paging::{self, ActivateError, AddressSpace, MapError};
use wost::arch::current::{apic, interrupt, pic, port, segment};
use wost::boot::{self, FramebufferInfo, Rgb};
use wost::mm;
use wost::sync::{Once, SpinLock};
use x86_paging::Flags;
use x86_tables::exception;

use crate::arch::current::debug_exit;
use crate::arch::current::trap::{self, Record};
use crate::arch::current::uart::{self, UartError};
use crate::arch::current::vector;
use crate::thread::ThreadContext;
use crate::{dev, error, info, input, thread, time};

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
            .all(|address| match paging::translate(address) {
                None => false,
                Some((_, flags)) => {
                    !flags.is_write_execute()
                        && flags.contains(Flags::WRITABLE) == writable
                        && !flags.contains(Flags::NO_EXECUTE) == executable
                }
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

/// Physical reads are refused for anything the direct map may not reach.
///
/// Three refusals and one acceptance: a check that only saw refusals would
/// pass against a function that returned `None` for everything.
fn physical_reads_are_bounded() -> bool {
    const WORD: usize = 8;

    // Chosen under the map's lock, used outside it: `read_physical` takes it.
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
            pick(|region| !region.kind.is_direct_mapped()),
            pick(|region| region.kind.is_direct_mapped() && !region.kind.is_allocatable()),
            undescribed,
        )
    });

    let Some((Some(usable), Some(unreachable), Some(readable), Some(undescribed))) = addresses
    else {
        return false;
    };

    let mut buffer = [0; WORD];
    !mm::read_physical(usable, &mut buffer)
        && !mm::read_physical(undescribed, &mut buffer)
        && !mm::read_physical(unreachable, &mut buffer)
        && mm::read_physical(readable, &mut buffer)
}

/// A second switch is refused.
///
/// The property that makes `AddressSpace::map` safe: nothing can add a mapping
/// to a table the CPU is already walking, because nothing can install a second
/// table to map into.
fn a_second_switch_is_refused() -> bool {
    match AddressSpace::new() {
        Ok(space) => paging::activate(space) == Err(ActivateError::AlreadyActive),
        Err(_) => false,
    }
}

/// A mapping cannot be silently re-pointed; an identical rewrite is the one
/// rewrite allowed, because the direct map's outward rounding produces it.
fn a_differing_remap_is_refused() -> bool {
    /// Canonical, and far from anything a fresh space would map.
    const PROBE: u64 = 0xffff_8800_0000_0000;

    let Ok(mut space) = AddressSpace::new() else {
        return false;
    };
    let first = space.map(PROBE, 4096, 4096, Flags::NO_EXECUTE);
    let again = space.map(PROBE, 4096, 4096, Flags::NO_EXECUTE);
    let differing = space.map(PROBE, 8192, 4096, Flags::NO_EXECUTE);
    first.is_ok() && again.is_ok() && differing == Err(MapError::Conflict(PROBE))
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
            thread::yield_now(&context());
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
        thread::yield_now(&context());
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
        thread::yield_now(&context());
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
        let guard_absent = paging::translate(slot.guard_base).is_none()
            && paging::translate(slot.stack_base - 8).is_none();
        let stack_present = [slot.stack_base, slot.stack_top() - 8]
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
    let context = context();
    let blocks = thread::blocks();
    let keystroke = input::read_key(&context);
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
    thread::sleep_ms(&context(), (ROUNDS as u64 + 2) * 10);

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
    thread::sleep_ms(&context(), 20);
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

    if let Some(geometry) = geometry {
        report.check("framebuffer geometry is sane", geometry_is_sane(geometry));
        report.check(
            "colour channels do not overlap",
            channels_are_disjoint(geometry),
        );
    }

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
        "a second address space is refused",
        a_second_switch_is_refused(),
    );
    report.check(
        "a differing remap is refused",
        a_differing_remap_is_refused(),
    );
    report.check(
        "a stale device claim is refused",
        a_stale_device_claim_is_refused(),
    );
    report.check(
        "a physical read is refused for memory the direct map may not reach",
        physical_reads_are_bounded(),
    );

    report.check(
        "the heap reuses a freed slot",
        the_heap_reuses_a_freed_slot(),
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
    let stack_pointer = cpu::stack_pointer();
    interrupt::breakpoint();
    let taken = trap::last_breakpoint();

    report.check(
        "a breakpoint is delivered and resumes",
        taken.count == before.count + 1,
    );
    report.check(
        "the saved frame is the frame the CPU pushed",
        frame_is_consistent(taken, stack_pointer),
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

    let passed = report.failed == 0;
    if passed {
        info!("selftest: {} checks passed", report.passed);
    } else {
        error!(
            "selftest: {} passed, {} FAILED",
            report.passed, report.failed
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
    let held = lock.lock();
    let refused = lock.try_lock().is_none();
    drop(held);
    refused && lock.try_lock().is_some()
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
    let outer_guard = outer.lock();
    let excluded = !interrupt::are_enabled();

    let inner_guard = inner.lock();
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
            let held = lock.lock();
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
    thread::sleep_ms(&context(), SLEEP_MS);
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
    thread::sleep_ms(&context(), SLEEP_MS);
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
/// `stack_pointer` is the caller's, taken immediately before the breakpoint: the
/// saved one belongs to a frame or two deeper, so it is checked for proximity
/// rather than equality.
fn frame_is_consistent(breakpoint: Record, stack_pointer: u64) -> bool {
    /// Bit 1 of the flags register reads as one on every x86 ever made.
    const RESERVED_FLAG: u64 = 1 << 1;

    /// How far the saved stack pointer may be from the one measured beside it.
    const STACK_SLACK: u64 = 4096;

    breakpoint.vector == u64::from(exception::BREAKPOINT)
        && breakpoint.error_code == 0
        && breakpoint.code_selector == u64::from(segment::KERNEL_CODE.bits())
        && breakpoint.flags & RESERVED_FLAG != 0
        && breakpoint.stack_pointer.abs_diff(stack_pointer) < STACK_SLACK
}
