//! The kernel's clock: a counter that never stops, and a timer armed only for
//! what someone is waiting for.
//!
//! Two clocks, one window. The timestamp counter free-runs and is read with
//! one instruction, so it is the clock: [`now_ns`] is a subtraction and a
//! multiply. The APIC timer counts down and interrupts, so it is the alarm:
//! one-shot, armed for the next moment the scheduler has a reason to look and
//! stopped when there is none.
//!
//! Neither rate is reported, so both are measured against the 8254 in the
//! same twenty-millisecond window. D-36 for the measurement, D-57 for why the
//! tick it once produced is gone.

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use timebase::Timebase;
use wost::arch::current::{apic, cpu, interrupt};
use x86_apic::timer::{self, Divide};
use x86_pit::{CRYSTAL_HZ, calibrate, elapsed_ticks};

use crate::arch::current::pit::Pit;
use crate::arch::current::vector;

/// Nanoseconds in a millisecond.
pub(crate) const NANOSECONDS_PER_MS: u64 = 1_000_000;

/// The bus clock, undivided. The divisor never enters the correctness path —
/// calibration measures the divided clock and every arming is derived from
/// the same clock — so this choice only sets how finely and how far ahead the
/// alarm can be set, and the finest costs nothing.
const DIVIDE: Divide = Divide::By1;

/// The soonest the timer will be armed for.
///
/// A deadline already passed would otherwise arm one tick of a gigahertz
/// clock and take the interrupt again immediately. The floor bounds that at
/// twenty thousand a second.
const MINIMUM_LEAD_NS: u64 = 50_000;

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
    assert!(
        CRYSTAL_HZ / 50 <= u16::MAX as u32,
        "the window must fit the counter"
    );
    assert!(WINDOW_TICKS as u32 + MAX_STEP as u32 <= u16::MAX as u32);
};

/// Believable bounds for the APIC's divided clock: QEMU models a gigahertz,
/// VMware tens of megahertz. A reading outside three orders of magnitude
/// either way is a corrupted measurement, not a strange machine.
const BELIEVABLE_APIC_HZ: core::ops::RangeInclusive<u64> = 1_000_000..=50_000_000_000;

/// Believable bounds for the timestamp counter, which is a CPU clock rather
/// than a bus one: hundreds of megahertz at the bottom of what can boot this,
/// a hundred gigahertz above anything that will exist for a while.
const BELIEVABLE_TSC_HZ: core::ops::RangeInclusive<u64> = 100_000_000..=100_000_000_000;

/// The counter's reading when the clock started, subtracted from every one
/// since so uptime begins at zero rather than at whenever the machine did.
static ORIGIN: AtomicU64 = AtomicU64::new(0);

/// Counter ticks into nanoseconds, packed (`Timebase::to_bits`). Zero until
/// calibration succeeds, which reads back as the conversion that answers zero
/// — so an uncalibrated clock reports no time at all rather than nonsense.
static TSC_TO_NANOSECONDS: AtomicU64 = AtomicU64::new(0);

/// Nanoseconds into APIC countdown ticks, packed the same way. Zero until
/// calibration succeeds, and [`arm_at`] declines to arm anything through it.
static NANOSECONDS_TO_APIC: AtomicU64 = AtomicU64::new(0);

/// Both measured rates, kept for reporting only.
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
static APIC_HZ: AtomicU64 = AtomicU64::new(0);

/// Timer interrupts taken since the clock started.
///
/// The evidence for D-57: this used to climb a hundred times a second
/// whatever the machine was doing, and now counts events somebody asked for.
static EXPIRIES: AtomicU64 = AtomicU64::new(0);

/// Why the clock could not be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimeError {
    /// The interval timer's ports are already claimed.
    PitUnavailable,
    /// Every measurement produced numbers outside the believable range.
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

/// What one window measured of both clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Calibration {
    /// The timestamp counter's rate.
    tsc_hz: u64,
    /// The APIC timer's divided clock's rate.
    apic_hz: u64,
    /// Whether the processor advertises the counter as invariant.
    invariant: bool,
}

impl Calibration {
    /// Whether two windows measured the same pair of clocks: each within one
    /// part in sixty-four — far looser than honest window jitter, far tighter
    /// than the factor-of-two-and-up a folded window inflates by. Both must
    /// agree, because a window that fooled one counter fooled both.
    fn agrees_with(self, other: Self) -> bool {
        close(self.tsc_hz, other.tsc_hz) && close(self.apic_hz, other.apic_hz)
    }

    /// Whether the processor promised that the counter this clock is built on
    /// runs at a constant rate.
    ///
    /// Not a veto: QEMU declines the bit while providing a steady counter, and
    /// the evidence this kernel actually has is its two agreeing windows. A
    /// machine without it gets a warning naming what is unpromised.
    pub(crate) const fn counter_is_invariant(self) -> bool {
        self.invariant
    }
}

/// Whether two measurements of one clock are within [`AGREEMENT_DIVISOR`].
fn close(one: u64, other: u64) -> bool {
    one.abs_diff(other) <= one / AGREEMENT_DIVISOR
}

impl fmt::Display for Calibration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "tickless: counter at {} hz{}, apic timer at {} hz",
            self.tsc_hz,
            if self.invariant { " (invariant)" } else { "" },
            self.apic_hz * u64::from(DIVIDE.factor()),
        )
    }
}

/// Measure both clocks against the interval timer and start the kernel's.
///
/// Runs with the interrupt flag still clear — asserted, because it is what
/// makes the measurement unpreemptible for free. Two consecutive windows must
/// agree before their answer is believed: every guard inside [`measure`] can
/// be slipped by a stall of exactly the wrong length, but a second
/// independent window cannot be slipped to the same wrong answer. The
/// interval timer is parked afterwards — stopped, its ports claimed for good
/// — because these measurements are all this kernel will ever ask of it.
///
/// Nothing is armed at the end. The timer is put in one-shot mode and left
/// stopped; the first thing to want an interrupt from it is the scheduler,
/// and until something does there is nothing to interrupt for.
///
/// # Errors
///
/// [`TimeError::PitUnavailable`] if the reference is already claimed, and
/// [`TimeError::Implausible`] if no two windows agreed on believable numbers.
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
    // Both conversions exist for any rate inside the believable ranges; the
    // arithmetic refuses only ratios no pair of clocks produces.
    let (Some(elapsed), Some(countdown)) = (
        Timebase::nanoseconds(calibration.tsc_hz),
        Timebase::ticks(calibration.apic_hz),
    ) else {
        return Err(TimeError::Implausible);
    };

    // Stopped before unmasked, in that order: the free-running countdown the
    // last window left behind would otherwise deliver an interrupt when it
    // happened to reach zero.
    apic::disarm_timer();
    apic::set_timer_one_shot(vector::TIMER);

    TSC_HZ.store(calibration.tsc_hz, Ordering::Relaxed);
    APIC_HZ.store(calibration.apic_hz, Ordering::Relaxed);
    NANOSECONDS_TO_APIC.store(countdown.to_bits(), Ordering::Relaxed);
    // Origin before the conversion that makes it readable, so no reader can
    // see a live conversion against an origin of zero and report the age of
    // the machine as the age of the kernel.
    ORIGIN.store(cpu::timestamp(), Ordering::Relaxed);
    TSC_TO_NANOSECONDS.store(elapsed.to_bits(), Ordering::Relaxed);
    Ok(calibration)
}

/// One bracketed window, measuring both clocks.
///
/// The two counter reads sit inside the crystal reads, so the skew of the
/// port transactions lands symmetrically: each rate is understated by two
/// reads in twenty milliseconds, which is noise. `None` means the window is
/// not trustworthy — a stuck counter, a step past [`MAX_STEP`] or backwards
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
    let tsc_start = cpu::timestamp();

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

    let tsc_end = cpu::timestamp();
    let apic_end = apic::timer_current_count();
    let pit_end = pit.count();

    // The APIC counts down and the timestamp counter counts up, which is the
    // only difference between the two measurements.
    let apic_counted = u64::from(apic_start.checked_sub(apic_end)?);
    let tsc_counted = tsc_end.checked_sub(tsc_start)?;
    let pit_ticks = elapsed_ticks(pit_start, pit_end);
    // The closing reads sit outside the step guard, so they get the same
    // bound by range: a fold between loop exit and here lands outside it.
    if !(WINDOW_TICKS..=WINDOW_TICKS + MAX_STEP).contains(&pit_ticks) {
        return None;
    }

    let apic_hz = calibrate::measured_hz(apic_counted, pit_ticks);
    let tsc_hz = calibrate::measured_hz(tsc_counted, pit_ticks);
    if !BELIEVABLE_APIC_HZ.contains(&apic_hz) || !BELIEVABLE_TSC_HZ.contains(&tsc_hz) {
        return None;
    }
    Some(Calibration {
        tsc_hz,
        apic_hz,
        invariant: cpu::has_invariant_tsc(),
    })
}

/// Nanoseconds since the clock started. Zero for as long as it never does.
///
/// One counter read and one multiply, taking no lock — which is what makes it
/// callable from a trap handler, where every scheduling decision is made.
/// Nothing clamps, so monotonicity is the counter's claim and the check that
/// watches for it is watching the hardware (D-57).
pub(crate) fn now_ns() -> u64 {
    let elapsed = Timebase::from_bits(TSC_TO_NANOSECONDS.load(Ordering::Relaxed));
    elapsed.convert(cpu::timestamp().wrapping_sub(ORIGIN.load(Ordering::Relaxed)))
}

/// Milliseconds since the clock started, for anything that reports to a
/// person.
pub(crate) fn uptime_ms() -> u64 {
    now_ns() / NANOSECONDS_PER_MS
}

/// Whether the clock was measured, and therefore whether it runs at all.
///
/// What anything that means to wait must ask first: with no clock there is no
/// deadline and no timer, so a wait would be a hang.
pub(crate) fn is_calibrated() -> bool {
    Timebase::from_bits(TSC_TO_NANOSECONDS.load(Ordering::Relaxed)).is_calibrated()
}

/// Ask for a timer interrupt at `deadline`, or as soon as the hardware can
/// manage if it has already passed.
///
/// Does nothing before the clock is calibrated, which is also the state a
/// machine whose timer declined stays in for good.
pub(crate) fn arm_at(deadline_ns: u64) {
    let countdown = Timebase::from_bits(NANOSECONDS_TO_APIC.load(Ordering::Relaxed));
    if !countdown.is_calibrated() {
        return;
    }
    let lead = deadline_ns.saturating_sub(now_ns()).max(MINIMUM_LEAD_NS);
    apic::arm_timer(timer::initial_count(countdown.convert(lead)));
}

/// Stop the timer. Nothing is waiting, so nothing needs waking.
pub(crate) fn disarm() {
    apic::disarm_timer();
}

/// One expiry — and nothing else, because this runs in trap context, where
/// D-31 and D-34 still bind: no locks, no logging. What to do about it is the
/// scheduler's, and it is called next.
pub(crate) fn on_expiry() {
    EXPIRIES.fetch_add(1, Ordering::Relaxed);
}

/// Timer interrupts taken since the clock started.
#[cfg(feature = "selftest")]
pub(crate) fn expiries() -> u64 {
    EXPIRIES.load(Ordering::Relaxed)
}

/// The measured rate of the timestamp counter; zero if calibration declined.
#[cfg(feature = "selftest")]
pub(crate) fn counter_hz() -> u64 {
    TSC_HZ.load(Ordering::Relaxed)
}

/// The measured rate of the APIC timer's divided clock; zero if calibration
/// declined.
#[cfg(feature = "selftest")]
pub(crate) fn apic_hz() -> u64 {
    APIC_HZ.load(Ordering::Relaxed)
}
