use super::*;

fn watchdog_spec(
    apic_remaining: u32,
    tsc_ticks: u64,
    pit_ticks: u16,
    expected_pit_ticks: u16,
    expected_tsc_hz: u64,
) -> WatchdogVerdict {
    if apic_remaining != 0 {
        return WatchdogVerdict::Inconclusive;
    }
    if expected_pit_ticks == 0 {
        return WatchdogVerdict::Inconclusive;
    }
    if expected_tsc_hz == 0 {
        return WatchdogVerdict::Inconclusive;
    }

    let expected = u128::from(expected_pit_ticks);
    let tolerance = (expected / 64).max(1);
    let pit = u128::from(pit_ticks);
    let latest = expected + 1_193_182_u128 / 100;
    let latest_tsc = u128::from(expected_tsc_hz) * latest / 1_193_182;
    if u128::from(tsc_ticks) > latest_tsc {
        return WatchdogVerdict::Inconclusive;
    }
    if pit > expected + tolerance {
        return WatchdogVerdict::Inconclusive;
    }
    if pit + tolerance < expected {
        return WatchdogVerdict::Disagreed;
    }

    let numerator = u128::from(tsc_ticks) * 1_193_182 + pit / 2;
    let observed: u128 = numerator.checked_div(pit).unwrap_or_default();
    let expected_hz = u128::from(expected_tsc_hz);
    let difference = observed.max(expected_hz) - observed.min(expected_hz);
    if difference <= expected_hz / 64 {
        WatchdogVerdict::Stable
    } else {
        WatchdogVerdict::Disagreed
    }
}

#[test]
fn an_exact_ratio_calibrates_exactly() {
    assert_eq!(measured_hz(200_000, 20_000), 10 * CRYSTAL_HZ as u64);
}

/// 1_193_182 / 4 is 298_295.5, which nearest rounds up.
#[test]
fn rounding_is_to_nearest_not_floor() {
    assert_eq!(measured_hz(1, 4), 298_296);
}

/// No addend wraps, since `u64::MAX mod CRYSTAL_HZ` is 958_759 and the largest is `u16::MAX / 2`.
#[test]
fn the_product_at_the_top_of_the_space_cannot_overflow() {
    let largest_counted = u64::MAX / CRYSTAL_HZ as u64;

    assert!(measured_hz(largest_counted, u16::MAX) > 0);
    assert_eq!(measured_hz(largest_counted + 1, u16::MAX), 0);

    let remainder = u64::MAX % CRYSTAL_HZ as u64;
    assert!(
        remainder >= u64::from(u16::MAX / 2),
        "the rounding addend no longer fits: {remainder} left"
    );
}

#[test]
fn an_empty_window_yields_nothing() {
    assert_eq!(measured_hz(500, 0), 0);
}

#[test]
fn a_counter_that_never_moved_measures_no_rate() {
    assert_eq!(measured_hz(0, 20_000), 0);
}

/// QEMU's gigahertz APIC clock and host TSC, and VMware's 66 MHz bus, over a 20 ms window.
#[test]
fn qemu_and_vmware_shaped_numbers_land_in_sane_bounds() {
    let window = 23_863;

    let apic = measured_hz(20_000_000, window);
    assert!((999_000_000..=1_001_000_000).contains(&apic), "qemu {apic}");

    let tsc = measured_hz(64_000_000, window);
    assert!(
        (3_190_000_000..=3_210_000_000).contains(&tsc),
        "qemu tsc {tsc}"
    );

    let apic = measured_hz(1_320_000, window);
    assert!((65_000_000..=67_000_000).contains(&apic), "vmware {apic}");
}

/// A 10 MHz bus and a 5 GHz counter both measure from one window without saturating.
#[test]
fn the_plausible_clock_range_stays_representable() {
    let window = 23_863;

    let slow = measured_hz(200_000, window);
    assert!((9_900_000..=10_100_000).contains(&slow), "slow {slow}");

    let fast = measured_hz(100_000_000, window);
    assert!(
        (4_990_000_000..=5_010_000_000).contains(&fast),
        "fast {fast}"
    );
}

#[test]
fn the_watchdog_distinguishes_disagreement_from_delay() {
    let expected_pit_ticks = (CRYSTAL_HZ / 50) as u16;
    let tsc_hz = 3_200_000_000;
    let tsc_ticks = tsc_hz / 50;

    assert_eq!(
        watchdog_verdict(0, tsc_ticks, expected_pit_ticks, expected_pit_ticks, tsc_hz,),
        WatchdogVerdict::Stable
    );
    assert_eq!(
        watchdog_verdict(0, 0, expected_pit_ticks, expected_pit_ticks, tsc_hz),
        WatchdogVerdict::Disagreed,
        "a stopped TSC must be revoked"
    );
    assert_eq!(
        watchdog_verdict(
            0,
            tsc_ticks / 2,
            expected_pit_ticks / 2,
            expected_pit_ticks,
            tsc_hz,
        ),
        WatchdogVerdict::Disagreed,
        "an early APIC expiry must be revoked"
    );
    assert_eq!(
        watchdog_verdict(
            0,
            tsc_ticks * 2,
            expected_pit_ticks + expected_pit_ticks / 16,
            expected_pit_ticks,
            tsc_hz,
        ),
        WatchdogVerdict::Inconclusive,
        "a late handler must not accuse the clocks"
    );
    assert_eq!(
        watchdog_verdict(1, 0, 1, expected_pit_ticks, tsc_hz),
        WatchdogVerdict::Inconclusive,
        "a pending older interrupt does not belong to the newly armed timer"
    );
}

#[test]
fn watchdog_boundaries_match_the_independent_specification() {
    for apic_remaining in [0, 1] {
        for expected in [0, 1, 63, 64, 65, 127, 128, 129, 1_000, 23_863, u16::MAX] {
            for expected_hz in [0, 1, 63, 64, 65, 1_193_182, 3_200_000_000] {
                let tolerance = (u64::from(expected) / 64).max(1);
                let mut pit_samples = vec![0, 1, expected, u16::MAX];
                for pit in [
                    u64::from(expected).saturating_sub(tolerance + 1),
                    u64::from(expected).saturating_sub(tolerance),
                    u64::from(expected).saturating_add(tolerance),
                    u64::from(expected).saturating_add(tolerance + 1),
                ] {
                    if let Ok(pit) = u16::try_from(pit) {
                        pit_samples.push(pit);
                    }
                }
                pit_samples.sort_unstable();
                pit_samples.dedup();

                let latest = u128::from(expected) + 1_193_182_u128 / 100;
                let latest_tsc = u128::from(expected_hz) * latest / 1_193_182;
                for pit in pit_samples {
                    let ideal_tsc =
                        (u128::from(expected_hz) * u128::from(pit) + 1_193_182 / 2) / 1_193_182;
                    let mut tsc_samples = vec![0, 1, ideal_tsc, latest_tsc];
                    tsc_samples.push(latest_tsc.saturating_sub(1));
                    tsc_samples.push(latest_tsc + 1);
                    tsc_samples.sort_unstable();
                    tsc_samples.dedup();

                    for tsc in tsc_samples {
                        let tsc = u64::try_from(tsc).unwrap();
                        assert_eq!(
                            watchdog_verdict(apic_remaining, tsc, pit, expected, expected_hz,),
                            watchdog_spec(apic_remaining, tsc, pit, expected, expected_hz,),
                            "apic={apic_remaining} tsc={tsc} pit={pit} expected={expected} hz={expected_hz}"
                        );
                    }
                }
            }
        }
    }
}
