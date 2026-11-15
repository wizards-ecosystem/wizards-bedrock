use super::*;

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
    assert_eq!(
        watchdog_verdict(0, 0, 100, 60_000, tsc_hz),
        WatchdogVerdict::Inconclusive,
        "a window whose late bound passes the 16-bit span can have folded"
    );
}
