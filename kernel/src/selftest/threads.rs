use core::hint::spin_loop;
use core::sync::atomic::{AtomicUsize, Ordering};

use stack_layout::StackLayout;
use wost::arch::current::paging::{self, AddressSpace};
use wost::arch::current::{interrupt, task, xstate};
use wost::mm;
use x86_paging::{Flags, PAGE_SIZE};

use super::address_spaces::pinned_guard_refuses_mapping;
use super::clocks::spins_until;
use super::processes::{
    AllocatorSnapshot, cleanup_unexpected_spawn, last_rites_for, spawn_resources,
};
use super::runs_until;
use crate::arch::current::trap;
use crate::queue;
use crate::thread::ThreadContext;
use crate::user;
use crate::wait::WaitQueue;
use crate::{error, input, mem, thread, time};

static REREAD_QUEUE: WaitQueue = WaitQueue::new();
static REREAD_READY: AtomicUsize = AtomicUsize::new(0);

/// 0 before the thread runs, 1 once it has come back through `wait_unless`.
static REREAD_RETURNED: AtomicUsize = AtomicUsize::new(0);

fn wait_on_a_ready_condition() {
    REREAD_QUEUE.wait_unless(&mut context(), || REREAD_READY.load(Ordering::Acquire) != 0);
    REREAD_RETURNED.store(1, Ordering::Release);
}

pub(super) fn a_waiter_rereads_its_condition_before_blocking() -> bool {
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

pub(super) fn every_enabled_state_component_survives_a_switch() -> bool {
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

pub(super) fn thread_local_storage_follows_the_thread() -> bool {
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

pub(super) fn context() -> ThreadContext {
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

pub(super) fn yields_until(done: impl Fn() -> bool) -> bool {
    (0..YIELD_BUDGET).any(|_| {
        thread::yield_now(&mut context());
        done()
    })
}

pub(super) fn threads_alternate_strictly() -> bool {
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

pub(super) fn registers_survive_a_switch() -> bool {
    CHURN_VERDICT.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("churn", churn_and_report).is_err() {
        return false;
    }

    let boot_survived = churn(0xD1B5_4A32_D192_ED03);
    let exited = yields_until(|| thread::free_slots() == free);

    boot_survived && exited && CHURN_VERDICT.load(Ordering::Acquire) == 1
}

pub(super) fn a_leaf_guard_is_the_last_lock() -> bool {
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

pub(super) fn a_stack_stops_being_installed_when_it_is_unmapped() -> bool {
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

pub(super) struct StackLiveness {
    pub(super) stale_root: bool,
    pub(super) first: bool,
    pub(super) middle: bool,
    pub(super) last: bool,
}

pub(super) fn stack_liveness_covers_the_current_root_and_extent() -> Option<StackLiveness> {
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

pub(super) fn thread_stacks_are_guarded() -> bool {
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

pub(super) fn a_new_thread_is_interruptible() -> bool {
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

pub(super) fn a_thread_starts_aligned() -> bool {
    ALIGNMENT_PROBE.store(0, Ordering::Release);
    let free = thread::free_slots();
    if thread::spawn("aligned", report_a_local_address).is_err() {
        return false;
    }

    let ran = yields_until(|| ALIGNMENT_PROBE.load(Ordering::Acquire) != 0);
    let exited = yields_until(|| thread::free_slots() == free);

    ran && exited && ALIGNMENT_PROBE.load(Ordering::Acquire).is_multiple_of(16)
}

pub(super) fn leave_at_once() {}

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

pub(super) fn an_exited_slot_is_reused() -> bool {
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
pub(super) static STOP_SPINNING: AtomicUsize = AtomicUsize::new(0);

/// How many loops each spinner has run: one counter per weight class.
pub(super) static LIGHT_SPINS: AtomicUsize = AtomicUsize::new(0);
static HEAVY_SPINS: AtomicUsize = AtomicUsize::new(0);

fn spin_counting(counter: &AtomicUsize) {
    while STOP_SPINNING.load(Ordering::Acquire) == 0 {
        counter.fetch_add(1, Ordering::Relaxed);
        spin_loop();
    }
}

pub(super) fn light_spinner() {
    spin_counting(&LIGHT_SPINS);
}

fn heavy_spinner() {
    spin_counting(&HEAVY_SPINS);
}

pub(super) fn the_clock_is_running() -> bool {
    const ATTEMPTS: u32 = 1_000_000;

    let start = time::now_ns();
    time::is_calibrated() && spins_until(ATTEMPTS, || time::now_ns() > start)
}

pub(super) fn a_spinner_cannot_starve_another() -> bool {
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

pub(super) fn runtime_divides_by_weight() -> bool {
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

pub(super) fn a_blocked_reader_waits_while_the_poller_runs() -> bool {
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

pub(super) fn the_blocked_reader_woke_with_the_key(passes_before: usize) -> bool {
    let wanted = ps2_keys::Key::A as usize + 1;

    let woke_through_polling = runs_until(1_000, || {
        READER_SAW.load(Ordering::Acquire) != 0 && input::poller_passes() > passes_before
    });
    woke_through_polling
        && READER_SAW.load(Ordering::Acquire) == wanted
        && READER_BLOCKED.load(Ordering::Acquire) == 1
        && input::dropped_events() == 0
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

pub(super) fn the_polled_fallback_runs_its_rounds_and_sleeps() -> bool {
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

pub(super) fn a_failed_idle_start_can_be_retried_exactly_once() -> bool {
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

pub(super) fn a_busy_machine_reaps_a_program_without_another_spawn() -> bool {
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

pub(super) fn the_idle_thread_runs_only_when_nothing_else_can() -> bool {
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
