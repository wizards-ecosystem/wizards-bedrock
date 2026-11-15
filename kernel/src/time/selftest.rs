use core::sync::atomic::{AtomicU64, Ordering};

use timebase::Timebase;
use wost::arch::current::{cpu, interrupt};
use wost::sync::Injection;

use super::{Refusal, TSC_TO_NANOSECONDS, nanoseconds_at};

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
impl Refusal {}

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
