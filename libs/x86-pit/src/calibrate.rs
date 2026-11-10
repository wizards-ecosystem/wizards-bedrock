//! A clock's rate from one PIT window, rounded to nearest once from the raw numbers.

use crate::CRYSTAL_HZ;

/// What an independent PIT window says about an armed TSC/APIC pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum WatchdogVerdict {
    /// The APIC expiry and TSC progress agree with the PIT crystal.
    Stable,
    /// Interrupt latency or a window fold prevents a safe conclusion.
    Inconclusive,
    /// The expiry arrived early or the TSC rate disagreed with the crystal.
    Disagreed,
}

/// Judge one APIC expiry against a PIT window started with it and kept under one span.
///
/// A late expiry is inconclusive, since latency and a slow APIC look alike; an early one disagrees.
///
/// TSC progress is judged only while its own count proves the PIT window cannot have folded.
pub const fn watchdog_verdict(
    apic_remaining: u32,
    tsc_ticks: u64,
    pit_ticks: u16,
    expected_pit_ticks: u16,
    expected_tsc_hz: u64,
) -> WatchdogVerdict {
    const AGREEMENT_DIVISOR: u64 = 64;
    const LATE_TICKS: u64 = CRYSTAL_HZ as u64 / 100;

    if apic_remaining != 0 || expected_pit_ticks == 0 || expected_tsc_hz == 0 {
        return WatchdogVerdict::Inconclusive;
    }

    let expected = expected_pit_ticks as u64;
    let tolerance = expected / AGREEMENT_DIVISOR;
    let tolerance = if tolerance == 0 { 1 } else { tolerance };
    let pit_ticks = pit_ticks as u64;
    let latest_unambiguous = expected + LATE_TICKS;
    let maximum_tsc_ticks =
        expected_tsc_hz as u128 * latest_unambiguous as u128 / CRYSTAL_HZ as u128;
    if tsc_ticks as u128 > maximum_tsc_ticks || pit_ticks > expected + tolerance {
        return WatchdogVerdict::Inconclusive;
    }
    if pit_ticks < expected.saturating_sub(tolerance) {
        return WatchdogVerdict::Disagreed;
    }

    let observed_hz = measured_hz(tsc_ticks, pit_ticks as u16);
    if observed_hz.abs_diff(expected_tsc_hz) <= expected_tsc_hz / AGREEMENT_DIVISOR {
        WatchdogVerdict::Stable
    } else {
        WatchdogVerdict::Disagreed
    }
}

/// The measured clock's rate in hertz, zero for an empty window or an overflowing product.
///
/// Zero fails any plausibility bound, which the caller must apply.
pub const fn measured_hz(counted: u64, pit_ticks: u16) -> u64 {
    if pit_ticks == 0 {
        return 0;
    }
    let denominator = pit_ticks as u64;
    match rounded_numerator(counted, denominator) {
        Some(numerator) => numerator / denominator,
        None => 0,
    }
}

/// The product plus the half-denominator that rounds the division, `None` on overflow.
const fn rounded_numerator(counted: u64, denominator: u64) -> Option<u64> {
    match counted.checked_mul(CRYSTAL_HZ as u64) {
        Some(product) => product.checked_add(denominator / 2),
        None => None,
    }
}

#[cfg(test)]
mod tests;
