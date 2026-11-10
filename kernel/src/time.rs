//! The clock is the free-running TSC, and the alarm the one-shot APIC timer, armed only if awaited.
//!
//! Both rates are measured against the 8254, or the TSC's taken from a complete invariant CPUID
//! ratio when that measurement refuses; the retained 8254 then validates live APIC windows.

use core::fmt;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use timebase::Timebase;
use wost::arch::current::{apic, cpu, interrupt};
use wost::sync::SpinLock;
use x86_apic::timer::{self, Divide};
use x86_pit::calibrate::WatchdogVerdict;
use x86_pit::{CRYSTAL_HZ, calibrate, elapsed_ticks};

use crate::arch::current::pit::Pit;

#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
use selftest::{
    APIC_HZ, EXPIRIES, LAST_WATCHDOG_EXPECTED, LAST_WATCHDOG_PIT, LAST_WATCHDOG_TSC,
    REFUSE_CALIBRATION,
};
#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    adjacent_readings_for_test, advance_scheduler_for_test, apic_hz,
    every_refusal_reads_back_as_itself, expiries, last_watchdog_sample,
    with_watchdog_disagreement_for_test, without_calibration_for_test,
};

pub(crate) const NANOSECONDS_PER_MS: u64 = 1_000_000;

/// Undivided: calibration and every arming use the divided clock, so this sets only how finely and
/// how far ahead the alarm can be set.
const DIVIDE: Divide = Divide::By1;

/// The soonest the timer will be armed for, so a deadline already passed retriggers at most twenty
/// thousand times a second.
const MINIMUM_LEAD_NS: u64 = 50_000;

/// Twenty milliseconds of crystal ticks: long enough that the bracketing port reads are noise, and
/// well inside the 54.9 ms at which the sixteen-bit counter becomes ambiguous.
const WINDOW_TICKS: u16 = (CRYSTAL_HZ / 50) as u16;

/// The largest believable gap between two counter reads, ten milliseconds. A stall across the
/// 54.9 ms span folds into a rate the believability bounds cannot see, so a larger or backward
/// step discards the attempt.
const MAX_STEP: u16 = (CRYSTAL_HZ / 100) as u16;

/// Windows to attempt before concluding the hardware is lying; two must agree.
const MEASUREMENT_ATTEMPTS: u32 = 5;

/// Counter reads per window before declaring the chip stuck, each a bus transaction, so a dead chip
/// costs seconds across every attempt and never the harness's whole timeout.
const MAX_POLLS: u32 = 100_000;

/// Windows within one part in this measure one clock; a stall past the step guard cannot fake that
/// twice.
const AGREEMENT_DIVISOR: u64 = 64;

/// Longest interval between validation interrupts while a deadline exists, inside the PIT's
/// unambiguous 54.9 ms span. With no deadline nothing is armed.
const WATCHDOG_NS: u64 = 40 * NANOSECONDS_PER_MS;

/// Consecutive conclusive failures required before revocation.
const WATCHDOG_FAILURES: usize = 2;

const _: () = {
    assert!(
        CRYSTAL_HZ / 50 <= u16::MAX as u32,
        "the window must fit the counter"
    );
    assert!(WINDOW_TICKS as u32 + MAX_STEP as u32 <= u16::MAX as u32);
};

/// QEMU models a gigahertz and VMware tens of megahertz; a reading three orders of magnitude
/// outside is a corrupted measurement.
const BELIEVABLE_APIC_HZ: core::ops::RangeInclusive<u64> = 1_000_000..=50_000_000_000;

/// A CPU clock: hundreds of megahertz on the slowest part that boots this, to far above any made.
const BELIEVABLE_TSC_HZ: core::ops::RangeInclusive<u64> = 100_000_000..=100_000_000_000;

/// The counter's reading when the clock started, so uptime begins at zero.
static ORIGIN: AtomicU64 = AtomicU64::new(0);

/// Counter ticks into nanoseconds, packed (`Timebase::to_bits`); zero until calibrated, which
/// converts every reading to zero.
static TSC_TO_NANOSECONDS: AtomicU64 = AtomicU64::new(0);

/// Nanoseconds into APIC countdown ticks, packed; zero until calibrated, and [`arm_at`] arms
/// nothing through it.
static NANOSECONDS_TO_APIC: AtomicU64 = AtomicU64::new(0);

/// The measured counter rate used by the watchdog and time page.
static TSC_HZ: AtomicU64 = AtomicU64::new(0);

/// Nanoseconds at which the timing subsystem was revoked.
static REVOKED_AT_NS: AtomicU64 = AtomicU64::new(0);

/// Private scheduling epoch after the public clock is revoked.
static SCHEDULER_NOW_NS: AtomicU64 = AtomicU64::new(0);

/// Why the live timing subsystem was revoked; zero means it remains active.
static REVOCATION: AtomicUsize = AtomicUsize::new(0);

/// Consecutive conclusive watchdog disagreements.
static WATCHDOG_DISAGREEMENTS: AtomicUsize = AtomicUsize::new(0);

/// The claimed reference clock and the validation window currently armed.
static HARDWARE: SpinLock<ClockHardware> = SpinLock::new(ClockHardware::EMPTY);

#[derive(Debug)]
struct ClockHardware {
    pit: Option<Pit>,
    armed: Option<WatchdogArm>,
}

impl ClockHardware {
    const EMPTY: Self = Self {
        pit: None,
        armed: None,
    };
}

#[derive(Clone, Copy, Debug)]
struct WatchdogArm {
    tsc_start: u64,
    pit_start: u16,
    expected_pit_ticks: u16,
}

/// A post-boot reason the clock and alarm are no longer trusted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Revocation {
    /// The timing subsystem remains active.
    None = 0,
    /// A live APIC/TSC window disagreed with the PIT crystal.
    WatchdogDisagreed = 1,
}

impl Revocation {
    /// The current post-boot clock state.
    pub(crate) fn current() -> Self {
        match REVOCATION.load(Ordering::Acquire) {
            1 => Self::WatchdogDisagreed,
            _ => Self::None,
        }
    }
}

/// Why the clock could not be started.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TimeError {
    /// The interval timer's ports are already claimed.
    PitUnavailable,
    /// The local APIC timer would not enter its required state.
    LocalTimer(apic::TimerError),
    /// Neither timing source produced an agreeing pair of believable windows.
    Implausible,
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PitUnavailable => {
                core::fmt::Write::write_str(f, "the interval timer's ports are already claimed")
            }
            Self::LocalTimer(reason) => write!(f, "the local timer refused setup: {reason}"),
            Self::Implausible => {
                // The failed guard and values make a calibration refusal actionable.
                let (tsc, apic) = last_measurement();
                write!(
                    f,
                    "no consistent believable calibration: {} (last window: tsc {tsc} hz, apic {apic} hz)",
                    Refusal::last()
                )
            }
        }
    }
}

impl core::error::Error for TimeError {}

/// What one window measured of both clocks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Calibration {
    /// The timestamp counter's rate.
    tsc_hz: u64,
    /// The APIC timer's divided clock's rate.
    apic_hz: u64,
    /// Whether the processor advertises the counter as invariant.
    invariant: bool,
    /// Which independent evidence supplied the rate.
    source: CalibrationSource,
    /// Exact TSC rate reported by CPUID leaf 0x15, when complete.
    cpuid_tsc_hz: Option<u64>,
    /// Nominal core rate from leaf 0x16, for diagnostics only.
    cpuid_base_hz: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CalibrationSource {
    Pit,
    Cpuid,
}

impl Calibration {
    /// Whether two windows measured the same pair of clocks, each within [`AGREEMENT_DIVISOR`].
    ///
    /// That is far looser than honest jitter and far tighter than a folded window's inflation. Both
    /// must agree, because a window that fooled one counter fooled both.
    fn agrees_with(self, other: Self) -> bool {
        close(self.tsc_hz, other.tsc_hz) && close(self.apic_hz, other.apic_hz)
    }

    /// Whether the processor promised that the counter this clock is built on runs at a constant
    /// rate.
    ///
    /// Advisory: QEMU declines the bit with a steady counter, and the evidence is the two agreeing
    /// windows, so a machine without it gets only a warning.
    pub(crate) const fn counter_is_invariant(self) -> bool {
        self.invariant
    }

    /// Rate of the APIC timer's source before the programmed divisor.
    pub(crate) const fn apic_source_hz(self) -> u64 {
        self.apic_hz * DIVIDE.factor() as u64
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
            "tickless: counter at {} hz{}, divided apic timer at {} hz",
            self.tsc_hz,
            if self.invariant { " (invariant)" } else { "" },
            self.apic_hz,
        )?;
        match self.source {
            CalibrationSource::Pit => core::fmt::Write::write_str(f, ", measured against pit")?,
            CalibrationSource::Cpuid => core::fmt::Write::write_str(f, ", tsc from cpuid")?,
        }
        if let Some(cpuid_hz) = self.cpuid_tsc_hz {
            write!(f, ", cpuid tsc {cpuid_hz} hz")?;
        }
        if let Some(base_hz) = self.cpuid_base_hz {
            write!(f, ", nominal core {base_hz} hz")?;
        }
        Ok(())
    }
}

/// Select, validate and start the kernel's clock and alarm, leaving the timer one-shot and stopped.
///
/// Must run with interrupts still masked, which makes the measurement unpreemptible. Two
/// consecutive windows must agree: a stall of exactly the wrong length can slip every guard in
/// [`measure`], but a second window cannot be slipped to the same wrong answer. When they refuse,
/// an invariant TSC with a complete CPUID crystal ratio is the rate and the APIC is measured
/// against it. The PIT stays claimed as the independent live watchdog.
///
/// # Errors
///
/// [`TimeError::PitUnavailable`] if the reference is already claimed,
/// [`TimeError::Implausible`] if neither timing source produces two agreeing,
/// believable windows, or [`TimeError::LocalTimer`] if final timer setup fails.
pub(crate) fn init() -> Result<Calibration, TimeError> {
    assert!(
        !interrupt::are_enabled(),
        "calibration must run before interrupts are enabled"
    );
    let mut pit = Pit::claim().map_err(|_| TimeError::PitUnavailable)?;
    let frequency = cpu::frequency_hints();
    let invariant = cpu::has_invariant_tsc();
    apic::set_timer_divide(DIVIDE);

    let mut candidate: Option<Calibration> = None;
    let mut outcome = None;
    let mut measured_windows = 0_u32;
    for _ in 0..MEASUREMENT_ATTEMPTS {
        let Some(current) = measure(&mut pit) else {
            candidate = None;
            continue;
        };
        measured_windows += 1;
        match candidate {
            Some(previous) if previous.agrees_with(current) => {
                outcome = Some(current);
                break;
            }
            _ => candidate = Some(current),
        }
    }
    let mut calibration = if let Some(calibration) = outcome {
        calibration
    } else if let Some(tsc_hz) = frequency.tsc_hz().filter(|_| invariant) {
        let Some(calibration) = calibrate_apic_from_tsc(tsc_hz) else {
            pit.park();
            return Err(TimeError::Implausible);
        };
        calibration
    } else {
        // A believable window need not have an adjacent usable peer.
        if measured_windows > 0 {
            LAST_REFUSAL.store(Refusal::Disagreed as usize, Ordering::Relaxed);
        }
        pit.park();
        return Err(TimeError::Implausible);
    };
    calibration.cpuid_tsc_hz = frequency.tsc_hz();
    calibration.cpuid_base_hz = frequency.base_hz();
    let (Some(elapsed), Some(countdown)) = (
        Timebase::nanoseconds(calibration.tsc_hz),
        Timebase::ticks(calibration.apic_hz),
    ) else {
        pit.park();
        return Err(TimeError::Implausible);
    };

    // Stopped before unmasking, or the last window's countdown would interrupt on reaching zero.
    let disarmed = match apic::disarm_timer() {
        Ok(disarmed) => disarmed,
        Err(reason) => {
            pit.park();
            return Err(TimeError::LocalTimer(reason));
        }
    };
    if let Err(reason) = apic::set_timer_one_shot(disarmed) {
        pit.park();
        return Err(TimeError::LocalTimer(reason));
    }

    TSC_HZ.store(calibration.tsc_hz, Ordering::Relaxed);
    #[cfg(feature = "selftest")]
    APIC_HZ.store(calibration.apic_hz, Ordering::Relaxed);
    NANOSECONDS_TO_APIC.store(countdown.to_bits(), Ordering::Relaxed);
    // Origin before the conversion that makes it readable, or a reader could report the machine's
    // age as the kernel's.
    ORIGIN.store(cpu::timestamp(), Ordering::Relaxed);
    TSC_TO_NANOSECONDS.store(elapsed.to_bits(), Ordering::Relaxed);
    *HARDWARE.lock_leaf() = ClockHardware {
        pit: Some(pit),
        armed: None,
    };
    Ok(calibration)
}

/// Calibrate the APIC against an invariant TSC whose crystal ratio CPUID gave.
fn calibrate_apic_from_tsc(tsc_hz: u64) -> Option<Calibration> {
    let mut candidate: Option<Calibration> = None;
    for _ in 0..MEASUREMENT_ATTEMPTS {
        let Some(current) = measure_apic_from_tsc(tsc_hz) else {
            candidate = None;
            continue;
        };
        if candidate.is_some_and(|previous| previous.agrees_with(current)) {
            return Some(current);
        }
        candidate = Some(current);
    }
    LAST_REFUSAL.store(Refusal::Disagreed as usize, Ordering::Relaxed);
    None
}

/// One bounded APIC window against the CPUID-rated TSC.
fn measure_apic_from_tsc(tsc_hz: u64) -> Option<Calibration> {
    const WINDOW_DIVISOR: u64 = 500;
    const MAX_TSC_POLLS: u32 = 20_000_000;

    let target = tsc_hz / WINDOW_DIVISOR;
    if target == 0 {
        return None;
    }
    apic::arm_timer(u32::MAX);
    let apic_start = apic::timer_current_count();
    let tsc_start = cpu::timestamp();
    let mut tsc_counted = 0;
    for _ in 0..MAX_TSC_POLLS {
        tsc_counted = cpu::timestamp().checked_sub(tsc_start)?;
        if tsc_counted >= target {
            break;
        }
        core::hint::spin_loop();
    }
    if !(target..=target.saturating_add(target / 2)).contains(&tsc_counted) {
        LAST_REFUSAL.store(Refusal::Overshot as usize, Ordering::Relaxed);
        return None;
    }
    let apic_counted = u64::from(apic_start.checked_sub(apic::timer_current_count())?);
    let apic_hz = Timebase::new(tsc_hz, tsc_counted)?.convert(apic_counted);
    LAST_TSC_HZ.store(tsc_hz, Ordering::Relaxed);
    LAST_APIC_HZ.store(apic_hz, Ordering::Relaxed);
    if !BELIEVABLE_APIC_HZ.contains(&apic_hz) || !BELIEVABLE_TSC_HZ.contains(&tsc_hz) {
        LAST_REFUSAL.store(Refusal::NotBelievable as usize, Ordering::Relaxed);
        return None;
    }
    Some(Calibration {
        tsc_hz,
        apic_hz,
        invariant: true,
        source: CalibrationSource::Cpuid,
        cpuid_tsc_hz: Some(tsc_hz),
        cpuid_base_hz: None,
    })
}

/// One bracketed window measuring both clocks, or `None`, with the [`Refusal`] recorded.
///
/// The counter reads sit inside the crystal reads, so port skew understates each rate by noise.
/// The caller decides how patient to be.
fn measure(pit: &mut Pit) -> Option<Calibration> {
    match measured(pit) {
        Ok(calibration) => Some(calibration),
        Err(why) => {
            LAST_REFUSAL.store(why as usize, Ordering::Relaxed);
            None
        }
    }
}

/// Why a window was not trustworthy, recorded for machines no debugger can reach.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Refusal {
    /// Nothing has been measured yet.
    None = 0,
    /// The crystal's count went backwards, or jumped further than one poll should.
    CrystalStepped = 1,
    /// The window did not close inside the poll bound, exactly as a crystal not counting would.
    NeverClosed = 2,
    /// A counter read backwards across the window.
    CounterWentBack = 3,
    /// The window overshot its bound between the loop and the closing reads.
    Overshot = 4,
    /// A rate outside what any real part reports.
    NotBelievable = 5,
    /// No agreeing pair of believable windows was admitted.
    Disagreed = 6,
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::None => core::fmt::Write::write_str(f, "nothing was measured"),
            Self::CrystalStepped => {
                core::fmt::Write::write_str(f, "the crystal stepped backwards or too far")
            }
            Self::NeverClosed => core::fmt::Write::write_str(
                f,
                "the window never closed; the crystal may not be counting",
            ),
            Self::CounterWentBack => {
                core::fmt::Write::write_str(f, "a counter read backwards across the window")
            }
            Self::Overshot => core::fmt::Write::write_str(f, "the window overshot its bound"),
            Self::NotBelievable => {
                core::fmt::Write::write_str(f, "a measured rate is outside belief")
            }
            Self::Disagreed => core::fmt::Write::write_str(
                f,
                "no agreeing pair of believable windows was admitted",
            ),
        }
    }
}

impl core::error::Error for Refusal {}

impl Refusal {
    /// What the last refusal was.
    pub(crate) fn last() -> Self {
        match LAST_REFUSAL.load(Ordering::Relaxed) {
            1 => Self::CrystalStepped,
            2 => Self::NeverClosed,
            3 => Self::CounterWentBack,
            4 => Self::Overshot,
            5 => Self::NotBelievable,
            6 => Self::Disagreed,
            _ => Self::None,
        }
    }
}

/// The last reason a window was refused, for the warning that follows.
static LAST_REFUSAL: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// The numbers the last refused window saw, for a machine that will not say.
static LAST_APIC_HZ: AtomicU64 = AtomicU64::new(0);
/// See [`LAST_APIC_HZ`].
static LAST_TSC_HZ: AtomicU64 = AtomicU64::new(0);

/// What the last refused window measured, whether or not it was believed.
pub(crate) fn last_measurement() -> (u64, u64) {
    (
        LAST_TSC_HZ.load(Ordering::Relaxed),
        LAST_APIC_HZ.load(Ordering::Relaxed),
    )
}

/// One window, saying why it was refused.
fn measured(pit: &mut Pit) -> Result<Calibration, Refusal> {
    // Free-running from the top, still masked: 32 bits at even 5 GHz is 0.86 s, forty windows.
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
            return Err(Refusal::CrystalStepped);
        }
        previous = elapsed;
        if elapsed >= WINDOW_TICKS {
            break;
        }
        polls += 1;
        if polls >= MAX_POLLS {
            return Err(Refusal::NeverClosed);
        }
        core::hint::spin_loop();
    }

    let tsc_end = cpu::timestamp();
    let apic_end = apic::timer_current_count();
    let pit_end = pit.count();

    // The APIC counts down and the timestamp counter up, the only difference between the two.
    let apic_counted = u64::from(
        apic_start
            .checked_sub(apic_end)
            .ok_or(Refusal::CounterWentBack)?,
    );
    let tsc_counted = tsc_end
        .checked_sub(tsc_start)
        .ok_or(Refusal::CounterWentBack)?;
    let pit_ticks = elapsed_ticks(pit_start, pit_end);
    // The closing reads sit outside the step guard, so a range bounds them: a fold between loop
    // exit and here lands outside it.
    if !(WINDOW_TICKS..=WINDOW_TICKS + MAX_STEP).contains(&pit_ticks) {
        return Err(Refusal::Overshot);
    }

    let apic_hz = calibrate::measured_hz(apic_counted, pit_ticks);
    let tsc_hz = calibrate::measured_hz(tsc_counted, pit_ticks);
    // Recorded whether or not believed: which number was wrong is the whole diagnosis.
    LAST_APIC_HZ.store(apic_hz, Ordering::Relaxed);
    LAST_TSC_HZ.store(tsc_hz, Ordering::Relaxed);
    if !BELIEVABLE_APIC_HZ.contains(&apic_hz) || !BELIEVABLE_TSC_HZ.contains(&tsc_hz) {
        return Err(Refusal::NotBelievable);
    }
    Ok(Calibration {
        tsc_hz,
        apic_hz,
        invariant: cpu::has_invariant_tsc(),
        source: CalibrationSource::Pit,
        cpuid_tsc_hz: None,
        cpuid_base_hz: None,
    })
}

/// Nanoseconds since the clock started, zero if it never did.
///
/// One counter read and one multiply, taking no lock, so a trap handler may call it.
/// A revoked clock freezes at its last trusted reading: it never fabricates progress and never
/// moves backwards while callers unwind their waits.
pub(crate) fn now_ns() -> u64 {
    if Revocation::current() != Revocation::None {
        return REVOKED_AT_NS.load(Ordering::Acquire);
    }
    let current = raw_now_ns();
    if Revocation::current() == Revocation::None {
        current
    } else {
        REVOKED_AT_NS.load(Ordering::Acquire)
    }
}

fn raw_now_ns() -> u64 {
    let elapsed = Timebase::from_bits(TSC_TO_NANOSECONDS.load(Ordering::Relaxed));
    nanoseconds_at(elapsed, cpu::timestamp())
}

fn nanoseconds_at(elapsed: Timebase, timestamp: u64) -> u64 {
    elapsed.convert(timestamp.wrapping_sub(ORIGIN.load(Ordering::Relaxed)))
}

/// Nanoseconds used only for scheduler progress: the public clock until revocation, then advanced
/// by each APIC expiry from the PIT while [`now_ns`] stays frozen.
///
/// No caller can then mistake fallback scheduling for trusted elapsed time.
pub(crate) fn scheduler_now_ns() -> u64 {
    if Revocation::current() != Revocation::None {
        return SCHEDULER_NOW_NS.load(Ordering::Acquire);
    }
    let current = raw_now_ns();
    if Revocation::current() == Revocation::None {
        current
    } else {
        SCHEDULER_NOW_NS.load(Ordering::Acquire)
    }
}

/// The counter reading the clock's zero sits at, for the time page.
///
/// Its reader computes `convert(rdtsc - origin)`, exactly what [`now_ns`] computes, so the two
/// cannot drift.
pub(crate) fn origin_ticks() -> u64 {
    ORIGIN.load(Ordering::Relaxed)
}

/// How counter ticks become nanoseconds.
pub(crate) fn conversion() -> Timebase {
    Timebase::from_bits(TSC_TO_NANOSECONDS.load(Ordering::Relaxed))
}

/// Milliseconds since the clock started, for anything that reports to a person.
pub(crate) fn uptime_ms() -> u64 {
    now_ns() / NANOSECONDS_PER_MS
}

/// Whether the clock was measured, and therefore whether it runs at all.
///
/// Anything that means to wait must ask first: with no clock there is no deadline and no timer, so
/// a wait would be a hang.
pub(crate) fn is_calibrated() -> bool {
    let calibrated = Timebase::from_bits(TSC_TO_NANOSECONDS.load(Ordering::Relaxed))
        .is_calibrated()
        && Revocation::current() == Revocation::None;
    #[cfg(feature = "selftest")]
    let calibrated = calibrated && !REFUSE_CALIBRATION.armed();
    calibrated
}

/// Ask for a timer interrupt at `deadline_ns`, or as soon as the hardware can manage if it passed.
///
/// Does nothing before the alarm is calibrated. After revocation it still gives bounded scheduler
/// progress against the retained PIT, but admits no new timing promise.
///
/// # Panics
///
/// If an admitted local timer refuses to stop.
pub(crate) fn arm_at(deadline_ns: u64) {
    let countdown = Timebase::from_bits(NANOSECONDS_TO_APIC.load(Ordering::Relaxed));
    if !countdown.is_calibrated() {
        disarm();
        return;
    }
    let lead = deadline_ns
        .saturating_sub(scheduler_now_ns())
        .clamp(MINIMUM_LEAD_NS, WATCHDOG_NS);
    let Some(pit_countdown) = Timebase::ticks(CRYSTAL_HZ.into()) else {
        disarm();
        return;
    };
    let Some(expected_pit_ticks) = u16::try_from(pit_countdown.convert(lead)).ok() else {
        disarm();
        return;
    };
    let mut hardware = HARDWARE.lock_leaf();
    let Some(pit) = hardware.pit.as_mut() else {
        stop_apic_timer();
        return;
    };
    pit.start_measurement();
    let pit_start = pit.count();
    let arm = WatchdogArm {
        tsc_start: cpu::timestamp(),
        pit_start,
        expected_pit_ticks,
    };
    hardware.armed = Some(arm);
    apic::arm_timer(timer::initial_count(countdown.convert(lead)));
}

/// Stop the timer. Nothing is waiting, so nothing needs waking.
///
/// # Panics
///
/// If an admitted local timer refuses to stop.
pub(crate) fn disarm() {
    HARDWARE.lock_leaf().armed = None;
    stop_apic_timer();
}

fn stop_apic_timer() {
    // No conversion means no countdown was admitted; losing the timer after publishing one is a
    // fatal hardware-authority contradiction.
    match apic::disarm_timer() {
        Ok(_) => {}
        Err(apic::TimerError::Unavailable)
            if !Timebase::from_bits(NANOSECONDS_TO_APIC.load(Ordering::Relaxed))
                .is_calibrated() => {}
        Err(reason) => panic!("the local timer would not stop: {reason}"),
    }
}

/// One expiry and nothing else, in trap context where nothing may log; the scheduler acts next.
pub(crate) fn on_expiry() {
    #[cfg(feature = "selftest")]
    EXPIRIES.fetch_add(1, Ordering::Relaxed);
    let apic_remaining = apic::timer_current_count();
    let tsc_end = cpu::timestamp();
    let (verdict, pit_ticks) = {
        let mut hardware = HARDWARE.lock_leaf();
        let Some(arm) = hardware.armed.take() else {
            return;
        };
        let Some(pit) = hardware.pit.as_mut() else {
            return;
        };
        let pit_ticks = elapsed_ticks(arm.pit_start, pit.count());
        let verdict = match tsc_end.checked_sub(arm.tsc_start) {
            Some(tsc_ticks) => {
                #[cfg(feature = "selftest")]
                {
                    LAST_WATCHDOG_TSC.store(tsc_ticks, Ordering::Relaxed);
                    LAST_WATCHDOG_PIT.store(u64::from(pit_ticks), Ordering::Relaxed);
                    LAST_WATCHDOG_EXPECTED
                        .store(u64::from(arm.expected_pit_ticks), Ordering::Relaxed);
                }
                calibrate::watchdog_verdict(
                    apic_remaining,
                    tsc_ticks,
                    pit_ticks,
                    arm.expected_pit_ticks,
                    TSC_HZ.load(Ordering::Relaxed),
                )
            }
            None => WatchdogVerdict::Disagreed,
        };
        (verdict, pit_ticks)
    };
    if Revocation::current() != Revocation::None {
        advance_scheduler(pit_ticks);
        return;
    }
    act_on_watchdog(verdict);
}

fn advance_scheduler(pit_ticks: u16) {
    let Some(conversion) = Timebase::nanoseconds(CRYSTAL_HZ.into()) else {
        return;
    };
    let elapsed = conversion.convert(u64::from(pit_ticks.max(1)));
    assert!(
        SCHEDULER_NOW_NS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |now| {
                Some(now.saturating_add(elapsed))
            })
            .is_ok(),
        "the scheduler clock refused an unconditional advance"
    );
}

fn act_on_watchdog(verdict: WatchdogVerdict) {
    match verdict {
        WatchdogVerdict::Stable | WatchdogVerdict::Inconclusive => {
            WATCHDOG_DISAGREEMENTS.store(0, Ordering::Release);
        }
        WatchdogVerdict::Disagreed => {
            let failures = WATCHDOG_DISAGREEMENTS.fetch_add(1, Ordering::AcqRel) + 1;
            if failures >= WATCHDOG_FAILURES {
                revoke(Revocation::WatchdogDisagreed);
            }
        }
        _ => WATCHDOG_DISAGREEMENTS.store(0, Ordering::Release),
    }
}

fn revoke(reason: Revocation) -> bool {
    if reason == Revocation::None || Revocation::current() != Revocation::None {
        return false;
    }
    let last_trusted = raw_now_ns();
    REVOKED_AT_NS.store(last_trusted, Ordering::Release);
    SCHEDULER_NOW_NS.store(last_trusted, Ordering::Release);
    if REVOCATION
        .compare_exchange(
            Revocation::None as usize,
            reason as usize,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return false;
    }
    disarm();
    match crate::timepage::publish(crate::timepage::MODE_NONE, origin_ticks(), conversion()) {
        crate::timepage::Publication::Published | crate::timepage::Publication::Deferred => {}
    }
    true
}

/// The measured rate of the timestamp counter; zero if calibration declined.
pub(crate) fn counter_hz() -> u64 {
    TSC_HZ.load(Ordering::Relaxed)
}
