use core::hint::spin_loop;
use core::sync::atomic::{AtomicUsize, Ordering};

use wost::arch::current::{task, xstate};

use super::clocks::spins_until;
use super::runs_until;
use crate::thread::ThreadContext;
use crate::{thread, time};

pub(super) fn context() -> ThreadContext {
    ThreadContext::require()
}

// A switch that never hands control over is a red verdict, not a hang.
const YIELD_BUDGET: usize = 100_000;

pub(super) fn yields_until(done: impl Fn() -> bool) -> bool {
    (0..YIELD_BUDGET).any(|_| {
        thread::yield_now(&mut context());
        done()
    })
}

pub(super) fn every_enabled_state_component_survives_a_switch() -> bool {
    const MINE: xstate::Probe = xstate::Probe {
        x87: 0x1234_5678_7654_3210,
        xmm: 0xA5A5_1234_5678_A5A5,
        ymm: 0xC3C3_0F0F_F0F0_3C3C,
    };
    if !xstate::is_enabled() {
        return xstate::bytes() == 0 && xstate::extended_control() == 0;
    }

    // Reading pops x87, so the probe is installed again for the switch.
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

    xstate::read_probe() == Some(MINE)
        && EXTENDED_INTRUDER_SAW_OWN.load(Ordering::Acquire) == 1
        && xstate::save_stays_inside_its_area()
}

static EXTENDED_INTRUDER_RAN: AtomicUsize = AtomicUsize::new(0);
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
    thread::yield_now(&mut context());
    let saw_own = xstate::read_probe() == Some(THEIRS);
    EXTENDED_INTRUDER_SAW_OWN.store(usize::from(saw_own), Ordering::Release);
    EXTENDED_INTRUDER_RAN.store(1, Ordering::Release);
}

pub(super) fn thread_local_storage_follows_the_thread() -> bool {
    const MINE: u64 = 0x0000_1234_5678_0000;

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
        let intruded = BASE_INTRUDER_SAW.load(Ordering::Acquire) == THEIR_BASE as usize;
        kernel_refused && hole_refused && kept && intruded
    })();
    let restored = task::set_thread_base(restore) && task::read_thread_base() == restore;
    checked && restored
}

const THEIR_BASE: u64 = 0x0000_7654_3210_0000;

static BASE_INTRUDER_RAN: AtomicUsize = AtomicUsize::new(0);
static BASE_INTRUDER_SAW: AtomicUsize = AtomicUsize::new(0);

fn set_another_thread_base() {
    if task::set_thread_base(THEIR_BASE) {
        thread::yield_now(&mut context());
        BASE_INTRUDER_SAW.store(task::read_thread_base() as usize, Ordering::Release);
    }
    BASE_INTRUDER_RAN.store(1, Ordering::Release);
}

// Zero while the churning thread runs, one if its values survived every switch, two if not.
static CHURN_VERDICT: AtomicUsize = AtomicUsize::new(0);

const CHURN_ROUNDS: u64 = 256;

// Knuth's multiplier, for a sequence with no short cycle.
const MULTIPLIER: u64 = 6_364_136_223_846_793_005;

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

const REUSED_SLOT_SECRET: xstate::Probe = xstate::Probe {
    x87: 0x1357_2468_1234_5678,
    xmm: 0xA5A5_5A5A_1122_3344,
    ymm: 0xC3C3_3C3C_5566_7788,
};

static REUSED_SLOT_STATE_CLEARED: AtomicUsize = AtomicUsize::new(0);
static REUSED_SLOT_SECRET_WRITTEN: AtomicUsize = AtomicUsize::new(0);

fn leave_extended_state() {
    let written = xstate::write_probe(REUSED_SLOT_SECRET);
    REUSED_SLOT_SECRET_WRITTEN.store(usize::from(written), Ordering::Release);
}

fn inspect_reused_extended_state() {
    let cleared = xstate::read_probe().is_some_and(|found| {
        found.x87 != REUSED_SLOT_SECRET.x87 && found.xmm == 0 && found.ymm == 0
    });
    REUSED_SLOT_STATE_CLEARED.store(usize::from(cleared), Ordering::Release);
}

pub(super) fn an_exited_slot_is_reused() -> bool {
    if !xstate::is_enabled() {
        return true;
    }
    let free = thread::free_slots();
    REUSED_SLOT_SECRET_WRITTEN.store(0, Ordering::Release);
    REUSED_SLOT_STATE_CLEARED.store(0, Ordering::Release);
    let secret_slot = thread::spawn("state owner", leave_extended_state).ok();
    let secret_returned = yields_until(|| thread::free_slots() == free);
    let clean_slot = thread::spawn("state successor", inspect_reused_extended_state).ok();
    let clean_returned = yields_until(|| thread::free_slots() == free);

    secret_slot.is_some()
        && secret_slot == clean_slot
        && secret_returned
        && clean_returned
        && REUSED_SLOT_SECRET_WRITTEN.load(Ordering::Acquire) == 1
        && REUSED_SLOT_STATE_CLEARED.load(Ordering::Acquire) == 1
}

pub(super) static STOP_SPINNING: AtomicUsize = AtomicUsize::new(0);
pub(super) static LIGHT_SPINS: AtomicUsize = AtomicUsize::new(0);

pub(super) fn light_spinner() {
    while STOP_SPINNING.load(Ordering::Acquire) == 0 {
        LIGHT_SPINS.fetch_add(1, Ordering::Relaxed);
        spin_loop();
    }
}

pub(super) fn the_clock_is_running() -> bool {
    let start = time::now_ns();
    time::is_calibrated() && spins_until(1_000_000, || time::now_ns() > start)
}

pub(super) fn a_spinner_cannot_starve_another() -> bool {
    // Generous beside a 3 ms slice, stingy beside a hung harness.
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
