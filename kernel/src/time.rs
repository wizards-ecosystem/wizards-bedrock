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
use selftest::EXPIRIES;
#[cfg(feature = "selftest")]
pub(crate) use selftest::{adjacent_readings_for_test, expiries};

pub(crate) const NANOSECONDS_PER_MS: u64 = 1_000_000;

// Calibration and arming both use the divided clock, so this sets only alarm resolution.
const DIVIDE: Divide = Divide::By1;

// A deadline already passed retriggers at most twenty thousand times a second.
const MINIMUM_LEAD_NS: u64 = 50_000;

// Twenty milliseconds: port reads are noise, and well inside the counter's 54.9 ms span.
const WINDOW_TICKS: u16 = (CRYSTAL_HZ / 50) as u16;

// Ten ms: a stall across the 54.9 ms span folds into a rate no bound can see, so it discards.
const MAX_STEP: u16 = (CRYSTAL_HZ / 100) as u16;

// Two of these must agree before the hardware is believed.
const MEASUREMENT_ATTEMPTS: u32 = 5;

// Each read is a bus transaction: a dead chip costs seconds, never the harness's timeout.
const MAX_POLLS: u32 = 100_000;

// A stall past the step guard cannot fake agreement within one part in this twice.
const AGREEMENT_DIVISOR: u64 = 64;

// Inside the PIT's unambiguous 54.9 ms span.
const WATCHDOG_NS: u64 = 40 * NANOSECONDS_PER_MS;

const WATCHDOG_FAILURES: usize = 2;

const _: () = {
    assert!(
        CRYSTAL_HZ / 50 <= u16::MAX as u32,
        "the window must fit the counter"
    );
    assert!(WINDOW_TICKS as u32 + MAX_STEP as u32 <= u16::MAX as u32);
};

// QEMU models a gigahertz and VMware tens of megahertz; far outside is a corrupted reading.
const BELIEVABLE_APIC_HZ: core::ops::RangeInclusive<u64> = 1_000_000..=50_000_000_000;

const BELIEVABLE_TSC_HZ: core::ops::RangeInclusive<u64> = 100_000_000..=100_000_000_000;

static ORIGIN: AtomicU64 = AtomicU64::new(0);

static TSC_TO_NANOSECONDS: AtomicU64 = AtomicU64::new(0);

static NANOSECONDS_TO_APIC: AtomicU64 = AtomicU64::new(0);

static TSC_HZ: AtomicU64 = AtomicU64::new(0);

static REVOKED_AT_NS: AtomicU64 = AtomicU64::new(0);

static SCHEDULER_NOW_NS: AtomicU64 = AtomicU64::new(0);

static REVOCATION: AtomicUsize = AtomicUsize::new(0);

static WATCHDOG_DISAGREEMENTS: AtomicUsize = AtomicUsize::new(0);

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Revocation {
    None = 0,
    WatchdogDisagreed = 1,
}

impl Revocation {
    pub(crate) fn current() -> Self {
        match REVOCATION.load(Ordering::Acquire) {
            1 => Self::WatchdogDisagreed,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TimeError {
    PitUnavailable,
    LocalTimer(apic::TimerError),
    Implausible,
}

impl fmt::Display for TimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PitUnavailable => {
                formatter.write_str("the interval timer's ports are already claimed")
            }
            Self::LocalTimer(reason) => {
                write!(formatter, "the local timer refused setup: {reason}")
            }
            Self::Implausible => {
                // The failed guard and values make a calibration refusal actionable.
                let (tsc, apic) = last_measurement();
                write!(
                    formatter,
                    "no consistent believable calibration: {} (last window: tsc {tsc} hz, apic {apic} hz)",
                    Refusal::last()
                )
            }
        }
    }
}

impl core::error::Error for TimeError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Calibration {
    tsc_hz: u64,
    apic_hz: u64,
    invariant: bool,
    source: CalibrationSource,
    cpuid_tsc_hz: Option<u64>,
    cpuid_base_hz: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CalibrationSource {
    Pit,
    Cpuid,
}

impl Calibration {
    // Both clocks must agree: a window that fooled one counter fooled both.
    fn agrees_with(self, other: Self) -> bool {
        close(self.tsc_hz, other.tsc_hz) && close(self.apic_hz, other.apic_hz)
    }

    // Advisory: QEMU declines the bit with a steady counter, so its absence only warns.
    pub(crate) const fn counter_is_invariant(self) -> bool {
        self.invariant
    }

    pub(crate) const fn apic_source_hz(self) -> u64 {
        self.apic_hz * DIVIDE.factor() as u64
    }
}

fn close(one: u64, other: u64) -> bool {
    one.abs_diff(other) <= one / AGREEMENT_DIVISOR
}

impl fmt::Display for Calibration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "tickless: counter at {} hz{}, divided apic timer at {} hz",
            self.tsc_hz,
            if self.invariant { " (invariant)" } else { "" },
            self.apic_hz,
        )?;
        match self.source {
            CalibrationSource::Pit => formatter.write_str(", measured against pit")?,
            CalibrationSource::Cpuid => formatter.write_str(", tsc from cpuid")?,
        }
        if let Some(cpuid_hz) = self.cpuid_tsc_hz {
            write!(formatter, ", cpuid tsc {cpuid_hz} hz")?;
        }
        if let Some(base_hz) = self.cpuid_base_hz {
            write!(formatter, ", nominal core {base_hz} hz")?;
        }
        Ok(())
    }
}

// Runs before `sti`, so no window is preempted; the timer is left one-shot and stopped.
pub(crate) fn init() -> Result<Calibration, TimeError> {
    assert!(
        !interrupt::are_enabled(),
        "calibration must run before interrupts are enabled"
    );
    let mut pit = Pit::claim().map_err(|_| TimeError::PitUnavailable)?;
    let frequency = cpu::frequency_hints();
    let invariant = cpu::has_invariant_tsc();
    apic::set_timer_divide(DIVIDE);

    let rated = frequency
        .tsc_hz()
        .filter(|_| invariant)
        .and_then(calibrate_apic_from_tsc);
    let Some(mut calibration) = rated.or_else(|| calibrate_against_pit(&mut pit)) else {
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
    NANOSECONDS_TO_APIC.store(countdown.to_bits(), Ordering::Relaxed);
    // Origin before the conversion that makes it readable; `Relaxed` orders them on one CPU only.
    ORIGIN.store(cpu::timestamp(), Ordering::Relaxed);
    TSC_TO_NANOSECONDS.store(elapsed.to_bits(), Ordering::Relaxed);
    // Only a PIT that measured the clock can watch it; one that refused would revoke it.
    let reference = match calibration.source {
        CalibrationSource::Pit => Some(pit),
        CalibrationSource::Cpuid => {
            pit.park();
            None
        }
    };
    *HARDWARE.lock_leaf() = ClockHardware {
        pit: reference,
        armed: None,
    };
    Ok(calibration)
}

// A stall of exactly the wrong length can slip every guard in one window, never in two agreeing.
fn calibrate_against_pit(pit: &mut Pit) -> Option<Calibration> {
    let mut candidate: Option<Calibration> = None;
    let mut measured_windows = 0_u32;
    for _ in 0..MEASUREMENT_ATTEMPTS {
        let Some(current) = measure(pit) else {
            candidate = None;
            continue;
        };
        measured_windows += 1;
        match candidate {
            Some(previous) if previous.agrees_with(current) => return Some(current),
            _ => candidate = Some(current),
        }
    }
    if measured_windows > 0 {
        LAST_REFUSAL.store(Refusal::Disagreed as usize, Ordering::Relaxed);
    }
    None
}

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
    for _ in 0..MAX_TSC_POLLS {
        if cpu::timestamp().checked_sub(tsc_start)? >= target {
            break;
        }
        core::hint::spin_loop();
    }
    let apic_end = apic::timer_current_count();
    let tsc_counted = cpu::timestamp().checked_sub(tsc_start)?;
    if !(target..=target.saturating_add(target / 2)).contains(&tsc_counted) {
        LAST_REFUSAL.store(Refusal::Overshot as usize, Ordering::Relaxed);
        return None;
    }
    let apic_counted = u64::from(apic_start.checked_sub(apic_end)?);
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

fn measure(pit: &mut Pit) -> Option<Calibration> {
    match measured(pit) {
        Ok(calibration) => Some(calibration),
        Err(why) => {
            LAST_REFUSAL.store(why as usize, Ordering::Relaxed);
            None
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Refusal {
    None = 0,
    CrystalStepped = 1,
    NeverClosed = 2,
    CounterWentBack = 3,
    Overshot = 4,
    NotBelievable = 5,
    Disagreed = 6,
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("nothing was measured"),
            Self::CrystalStepped => formatter.write_str("the crystal stepped backwards or too far"),
            Self::NeverClosed => {
                formatter.write_str("the window never closed; the crystal may not be counting")
            }
            Self::CounterWentBack => {
                formatter.write_str("a counter read backwards across the window")
            }
            Self::Overshot => formatter.write_str("the window overshot its bound"),
            Self::NotBelievable => formatter.write_str("a measured rate is outside belief"),
            Self::Disagreed => {
                formatter.write_str("no agreeing pair of believable windows was admitted")
            }
        }
    }
}

impl core::error::Error for Refusal {}

impl Refusal {
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

static LAST_REFUSAL: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

static LAST_APIC_HZ: AtomicU64 = AtomicU64::new(0);
static LAST_TSC_HZ: AtomicU64 = AtomicU64::new(0);

fn last_measurement() -> (u64, u64) {
    (
        LAST_TSC_HZ.load(Ordering::Relaxed),
        LAST_APIC_HZ.load(Ordering::Relaxed),
    )
}

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

    // The same order as the opening reads, so each clock's window spans the same read costs.
    let pit_end = pit.count();
    let apic_end = apic::timer_current_count();
    let tsc_end = cpu::timestamp();

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
    // The closing reads sit outside the step guard, so a range bounds a fold since loop exit.
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

// Lock-free, so a trap handler may call it; a revoked clock freezes at its last trusted reading.
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

// Scheduler progress only: after revocation it advances per expiry while `now_ns` freezes.
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

// The time page reader computes `convert(rdtsc - origin)` exactly as `now_ns` does.
pub(crate) fn origin_ticks() -> u64 {
    ORIGIN.load(Ordering::Relaxed)
}

pub(crate) fn conversion() -> Timebase {
    Timebase::from_bits(TSC_TO_NANOSECONDS.load(Ordering::Relaxed))
}

pub(crate) fn uptime_ms() -> u64 {
    now_ns() / NANOSECONDS_PER_MS
}

// Anything that means to wait must ask first: with no clock a wait is a hang.
pub(crate) fn is_calibrated() -> bool {
    Timebase::from_bits(TSC_TO_NANOSECONDS.load(Ordering::Relaxed)).is_calibrated()
        && Revocation::current() == Revocation::None
}

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
    hardware.armed = hardware.pit.as_mut().map(|pit| {
        pit.start_measurement();
        WatchdogArm {
            pit_start: pit.count(),
            tsc_start: cpu::timestamp(),
            expected_pit_ticks,
        }
    });
    apic::arm_timer(timer::initial_count(countdown.convert(lead)));
}

pub(crate) fn disarm() {
    HARDWARE.lock_leaf().armed = None;
    stop_apic_timer();
}

fn stop_apic_timer() {
    // No conversion means no countdown was admitted; losing an admitted timer is fatal.
    match apic::disarm_timer() {
        Ok(_) => {}
        Err(apic::TimerError::Unavailable)
            if !Timebase::from_bits(NANOSECONDS_TO_APIC.load(Ordering::Relaxed))
                .is_calibrated() => {}
        Err(reason) => panic!("the local timer would not stop: {reason}"),
    }
}

// Trap context: nothing here may log.
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
            Some(tsc_ticks) => calibrate::watchdog_verdict(
                apic_remaining,
                tsc_ticks,
                pit_ticks,
                arm.expected_pit_ticks,
                TSC_HZ.load(Ordering::Relaxed),
            ),
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

pub(crate) fn counter_hz() -> u64 {
    TSC_HZ.load(Ordering::Relaxed)
}
