use core::sync::atomic::{AtomicU64, Ordering};

use timebase::Timebase;
use wost::arch::current::{cpu, interrupt};
use wost::sync::Injection;
use x86_pit::calibrate;

use super::{
    LAST_REFUSAL, REVOCATION, Refusal, Revocation, TSC_HZ, TSC_TO_NANOSECONDS,
    WATCHDOG_DISAGREEMENTS, WATCHDOG_FAILURES, WINDOW_TICKS, act_on_watchdog, advance_scheduler,
    conversion, nanoseconds_at, origin_ticks,
};

/// The measured APIC rate observed by the self-test.
#[cfg(feature = "selftest")]
pub(super) static APIC_HZ: AtomicU64 = AtomicU64::new(0);

/// Last live watchdog operands observed by the self-test.
#[cfg(feature = "selftest")]
pub(super) static LAST_WATCHDOG_TSC: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "selftest")]
pub(super) static LAST_WATCHDOG_PIT: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "selftest")]
pub(super) static LAST_WATCHDOG_EXPECTED: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "selftest")]
pub(super) static EXPIRIES: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "selftest")]
impl Refusal {
    /// Every refusal, so a check can sweep them without a list of its own.
    pub(crate) const ALL: [Self; 7] = [
        Self::None,
        Self::CrystalStepped,
        Self::NeverClosed,
        Self::CounterWentBack,
        Self::Overshot,
        Self::NotBelievable,
        Self::Disagreed,
    ];
}

/// Every refusal reads back as the one that was recorded.
///
/// Nothing else notices a discriminant and its `match` arm drifting apart to name the wrong guard.
#[cfg(feature = "selftest")]
pub(crate) fn every_refusal_reads_back_as_itself() -> bool {
    let held = LAST_REFUSAL.load(Ordering::Relaxed);
    let faithful = Refusal::ALL.iter().all(|&refusal| {
        LAST_REFUSAL.store(refusal as usize, Ordering::Relaxed);
        Refusal::last() == refusal
    });
    LAST_REFUSAL.store(held, Ordering::Relaxed);
    faithful
}

/// Adjacent counter readings converted through the public clock's timebase.
#[cfg(feature = "selftest")]
pub(crate) fn adjacent_readings_for_test() -> (u64, u64) {
    let elapsed = Timebase::from_bits(TSC_TO_NANOSECONDS.load(Ordering::Relaxed));
    let first = cpu::timestamp();
    let second = cpu::timestamp();
    (
        nanoseconds_at(elapsed, first),
        nanoseconds_at(elapsed, second),
    )
}

#[cfg(feature = "selftest")]
pub(super) static REFUSE_CALIBRATION: Injection = Injection::new();

/// Run a check through the production uncalibrated-clock predicate.
#[cfg(feature = "selftest")]
pub(crate) fn without_calibration_for_test<T>(check: impl FnOnce() -> T) -> T {
    let disabled = interrupt::disable();
    let result = REFUSE_CALIBRATION.with(true, check);
    disabled.restore();
    result
}

/// Run a check while a stopped-TSC watchdog sample revokes the live clock.
#[cfg(feature = "selftest")]
pub(crate) fn with_watchdog_disagreement_for_test<T>(check: impl FnOnce() -> T) -> T {
    let disabled = interrupt::disable();
    let expected = WINDOW_TICKS;
    for _ in 0..WATCHDOG_FAILURES {
        act_on_watchdog(calibrate::watchdog_verdict(
            0,
            0,
            expected,
            expected,
            TSC_HZ.load(Ordering::Relaxed),
        ));
    }
    let result = check();
    WATCHDOG_DISAGREEMENTS.store(0, Ordering::Release);
    REVOCATION.store(Revocation::None as usize, Ordering::Release);
    assert_eq!(
        crate::timepage::publish(crate::timepage::MODE_COUNTER, origin_ticks(), conversion(),),
        crate::timepage::Publication::Published,
        "the restored time page refused the live clock"
    );
    disabled.restore();
    result
}

/// Advance the private fallback epoch through its production conversion.
#[cfg(feature = "selftest")]
pub(crate) fn advance_scheduler_for_test() {
    advance_scheduler(WINDOW_TICKS);
}

/// Operands behind the most recent live watchdog decision.
#[cfg(feature = "selftest")]
pub(crate) fn last_watchdog_sample() -> (u64, u64, u64) {
    (
        LAST_WATCHDOG_TSC.load(Ordering::Relaxed),
        LAST_WATCHDOG_PIT.load(Ordering::Relaxed),
        LAST_WATCHDOG_EXPECTED.load(Ordering::Relaxed),
    )
}

/// Timer interrupts taken since the clock started.
#[cfg(feature = "selftest")]
pub(crate) fn expiries() -> u64 {
    EXPIRIES.load(Ordering::Relaxed)
}

/// The measured rate of the APIC timer's divided clock; zero if calibration declined.
#[cfg(feature = "selftest")]
pub(crate) fn apic_hz() -> u64 {
    APIC_HZ.load(Ordering::Relaxed)
}
