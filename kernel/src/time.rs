//! The kernel's clock: a calibrated tick, and the uptime it accumulates.
//!
//! Nothing reports the APIC timer's rate, so it is measured — against the
//! 8254, the one clock in the machine whose rate is already known and whose
//! registers are reachable without page tables. One twenty-millisecond
//! window, bracketed reads, and the arithmetic in `x86_pit::calibrate` turn
//! that into a reload count; the timer then ticks at [`TICK_HZ`] and the
//! handler does nothing but count. Milliseconds come from interpolating the
//! hardware's countdown between ticks. See D-36.

use core::fmt;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering, compiler_fence};

use wost::arch::current::{apic, interrupt};
use x86_apic::timer::{self, Divide};
use x86_pit::{CRYSTAL_HZ, calibrate, elapsed_ticks};

use crate::arch::current::pit::Pit;
use crate::arch::current::vector;

/// Ticks per second. Deliberately boring: cheap enough to leave running,
/// decimal enough to read, and disposable when the tickless scheduling of
/// D-15 arrives.
const TICK_HZ: u32 = 100;

/// Milliseconds each tick contributes.
pub(crate) const MS_PER_TICK: u32 = 1000 / TICK_HZ;

/// The bus clock, undivided. The divisor never enters the correctness path —
/// calibration measures the divided clock and the reload count is derived
/// from the same clock — so this choice only sets the interpolation grain,
/// and the finest available costs nothing.
const DIVIDE: Divide = Divide::By1;

/// Measurement window, in crystal ticks: twenty milliseconds. Long enough
/// that the port reads bracketing it are noise, short enough to stay well
/// inside the 54.9 ms at which the sixteen-bit counter becomes ambiguous.
const WINDOW_TICKS: u16 = (CRYSTAL_HZ / 50) as u16;

/// The largest believable gap between two counter observations: ten
/// milliseconds. An honest gap is a handful of ticks and a hypervisor
/// hiccup a few thousand; only a stall jumps further, and a stall that
/// crosses the counter's 54.9 ms span folds the window modulo 65536 into a
/// rate inflated by a factor the believability bounds cannot see. A step
/// past this — or backwards, which is the fold showing directly — discards
/// the attempt instead of guessing.
const MAX_STEP: u16 = (CRYSTAL_HZ / 100) as u16;

/// Windows to attempt before concluding the hardware is lying. Two must
/// agree, so this allows a few discarded ones around them.
const MEASUREMENT_ATTEMPTS: u32 = 5;

/// Counter reads to allow one window before declaring the chip stuck. A
/// healthy window needs a few thousand; each read is a bus transaction, so
/// the cap has to stay small enough that a dead chip costs seconds across
/// every attempt, not the harness's whole timeout.
const MAX_POLLS: u32 = 100_000;

/// One part in sixty-four: two windows this close are measuring the same
/// clock; a stall that survived the step guard cannot fake agreement twice.
const AGREEMENT_DIVISOR: u64 = 64;

const _: () = {
    assert!(1000 % TICK_HZ == 0, "a tick must be whole milliseconds");
    assert!(
        CRYSTAL_HZ / 50 <= u16::MAX as u32,
        "the window must fit the counter"
    );
    assert!(WINDOW_TICKS as u32 + MAX_STEP as u32 <= u16::MAX as u32);
};

/// Believable bounds for the divided clock: QEMU models a gigahertz, VMware
/// tens of megahertz. A reading outside three orders of magnitude either way
/// is a corrupted measurement, not a strange machine.
const BELIEVABLE_HZ: core::ops::RangeInclusive<u64> = 1_000_000..=50_000_000_000;

/// Ticks taken since the timer started.
static TICKS: AtomicU64 = AtomicU64::new(0);

/// The reload count the timer runs on. Zero until calibration succeeds, which
/// [`timer::elapsed_milliseconds`] reads as "whole ticks only".
static INITIAL_COUNT: AtomicU32 = AtomicU32::new(0);

/// The divided clock's measured rate. Zero until calibration succeeds.
static DIVIDED_HZ: AtomicU64 = AtomicU64::new(0);

/// The largest uptime ever reported — the monotonic clamp.
static HIGH_WATER: AtomicU64 = AtomicU64::new(0);

/// Why the timer could not be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimeError {
    /// The interval timer's ports are already claimed.
    PitUnavailable,
    /// Every measurement produced numbers outside [`BELIEVABLE_HZ`].
    Implausible,
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PitUnavailable => f.write_str("the interval timer's ports are already claimed"),
            Self::Implausible => f.write_str("no believable rate from any measurement"),
        }
    }
}

impl core::error::Error for TimeError {}

/// What calibration measured and chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Calibration {
    /// The divided clock's measured rate.
    divided_hz: u64,
    /// The reload count that makes it expire [`TICK_HZ`] times a second.
    initial_count: u32,
}

impl Calibration {
    /// Whether two windows measured the same clock: within one part in
    /// sixty-four — far looser than honest window jitter, far tighter than
    /// the factor-of-two-and-up a folded window inflates by.
    fn agrees_with(self, other: Self) -> bool {
        self.divided_hz.abs_diff(other.divided_hz) <= self.divided_hz / AGREEMENT_DIVISOR
    }
}

impl fmt::Display for Calibration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{TICK_HZ} hz tick: apic timer at {} hz, initial count {}",
            self.divided_hz * u64::from(DIVIDE.factor()),
            self.initial_count
        )
    }
}

/// Calibrate the APIC timer against the interval timer and start the tick.
///
/// Runs with the interrupt flag still clear — asserted, because it is what
/// makes the measurement unpreemptible for free — and the first tick this
/// arms stays pending in the APIC until the flag is set. Two consecutive
/// windows must agree before their answer is believed: every guard inside
/// [`measure`] can be slipped by a stall of exactly the wrong length, but a
/// second independent window cannot be slipped to the same wrong answer.
/// The interval timer is parked afterwards — stopped, its ports claimed for
/// good — because these measurements are all this kernel will ever ask of
/// it.
pub(crate) fn init() -> Result<Calibration, TimeError> {
    assert!(
        !interrupt::are_enabled(),
        "calibration must run before interrupts are enabled"
    );
    let mut pit = Pit::claim().map_err(|_| TimeError::PitUnavailable)?;
    apic::set_timer_divide(DIVIDE);

    let mut candidate: Option<Calibration> = None;
    let mut outcome = None;
    for _ in 0..MEASUREMENT_ATTEMPTS {
        let Some(current) = measure(&mut pit) else {
            candidate = None;
            continue;
        };
        match candidate {
            Some(previous) if previous.agrees_with(current) => {
                outcome = Some(current);
                break;
            }
            _ => candidate = Some(current),
        }
    }
    pit.park();

    let Some(calibration) = outcome else {
        return Err(TimeError::Implausible);
    };

    INITIAL_COUNT.store(calibration.initial_count, Ordering::Relaxed);
    DIVIDED_HZ.store(calibration.divided_hz, Ordering::Relaxed);
    apic::start_periodic_timer(vector::TIMER, calibration.initial_count);
    Ok(calibration)
}

/// One bracketed window.
///
/// The APIC reads sit inside the crystal reads, so the skew of the four port
/// transactions lands symmetrically: the rate is understated by two reads in
/// twenty milliseconds, which is noise. `None` means the window is not
/// trustworthy — a stuck counter, a step past [`MAX_STEP`] or backwards
/// (a stall folding through the counter's span), a window that overshot its
/// bound, or a rate outside belief — and the caller decides how patient to
/// be.
fn measure(pit: &mut Pit) -> Option<Calibration> {
    // Free-running from the top under a still-masked entry: 32 bits at even
    // 5 GHz is 0.86 s to zero, forty times the window.
    apic::arm_timer(u32::MAX);
    pit.start_measurement();

    let pit_start = pit.count();
    let apic_start = apic::timer_current_count();

    let mut previous = 0;
    let mut polls = 0;
    loop {
        let elapsed = elapsed_ticks(pit_start, pit.count());
        if elapsed < previous || elapsed - previous > MAX_STEP {
            return None;
        }
        previous = elapsed;
        if elapsed >= WINDOW_TICKS {
            break;
        }
        polls += 1;
        if polls >= MAX_POLLS {
            return None;
        }
        core::hint::spin_loop();
    }

    let apic_end = apic::timer_current_count();
    let pit_end = pit.count();

    let counted = u64::from(apic_start.checked_sub(apic_end)?);
    let pit_ticks = elapsed_ticks(pit_start, pit_end);
    // The closing reads sit outside the step guard, so they get the same
    // bound by range: a fold between loop exit and here lands outside it.
    if !(WINDOW_TICKS..=WINDOW_TICKS + MAX_STEP).contains(&pit_ticks) {
        return None;
    }

    let divided_hz = calibrate::divided_hz(counted, pit_ticks);
    if !BELIEVABLE_HZ.contains(&divided_hz) {
        return None;
    }
    let initial_count = calibrate::periodic_initial_count(counted, pit_ticks, TICK_HZ)?;
    Some(Calibration {
        divided_hz,
        initial_count,
    })
}

/// One more tick — and nothing else, because this runs in trap context, where
/// D-31 and D-34 still bind: no locks, no logging.
pub(crate) fn on_tick() {
    // Relaxed, because the counter publishes no companion fields — unlike the
    // trap recorders, whose release ordering exists to publish theirs.
    TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Milliseconds since the tick started. Zero for as long as it never does.
///
/// Whole ticks come from the counter the handler increments; the fraction
/// comes from where the hardware countdown stands. Two defences keep the
/// combination honest. The countdown read is sandwiched between two reads of
/// the tick count, and retried if they disagree — a tick landed in between.
/// And the result is clamped monotonic against every earlier reading, because
/// code holding the interrupt flag clear can watch the countdown reload while
/// the tick it announces stays pending, and a reader there would otherwise
/// see time step back.
pub(crate) fn uptime_ms() -> u64 {
    /// Two collisions in a row mean the loads were a whole tick apart; whole
    /// ticks are then the honest answer.
    const READ_RETRIES: u32 = 3;

    let initial_count = INITIAL_COUNT.load(Ordering::Relaxed);
    // An uncalibrated clock has no countdown worth reading: whole ticks —
    // zero, since the timer never started — without touching the hardware.
    if initial_count == 0 {
        return clamp_monotonic(
            TICKS
                .load(Ordering::Relaxed)
                .saturating_mul(u64::from(MS_PER_TICK)),
        );
    }
    for _ in 0..READ_RETRIES {
        let before = TICKS.load(Ordering::Relaxed);
        // The fences keep the tick loads from crossing the countdown read;
        // the read itself is non-`pure` assembly, which the compiler must not
        // hoist, merge, or discard. Even under the most hostile reading — the
        // `nomem` register read floating past a fence — the error is bounded
        // to one tick, and the clamp below keeps it monotonic.
        compiler_fence(Ordering::SeqCst);
        let current = apic::timer_current_count();
        compiler_fence(Ordering::SeqCst);
        if TICKS.load(Ordering::Relaxed) == before {
            return clamp_monotonic(timer::elapsed_milliseconds(
                before,
                MS_PER_TICK,
                initial_count,
                current,
            ));
        }
    }
    clamp_monotonic(
        TICKS
            .load(Ordering::Relaxed)
            .saturating_mul(u64::from(MS_PER_TICK)),
    )
}

/// No reading may ever be smaller than one already given out.
fn clamp_monotonic(raw: u64) -> u64 {
    HIGH_WATER.fetch_max(raw, Ordering::Relaxed).max(raw)
}

/// Ticks taken since the timer started.
#[cfg(feature = "selftest")]
pub(crate) fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// The calibrated rate of the divided clock; zero if calibration declined.
#[cfg(feature = "selftest")]
pub(crate) fn divided_hz() -> u64 {
    DIVIDED_HZ.load(Ordering::Relaxed)
}
