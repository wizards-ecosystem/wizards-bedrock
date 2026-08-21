//! Turning one shared measurement window into a rate and a reload count.
//!
//! Both functions work from the same raw pair — cycles another clock counted
//! while the 8254 counted `pit_ticks` of its crystal — and round to nearest
//! exactly once, from the raw numbers. Deriving the reload count from an
//! already-rounded rate would compound two floors; deriving both from the
//! window keeps the tick honest to what was actually measured.

use crate::CRYSTAL_HZ;

/// The measured clock's rate in hertz, from one window against the crystal.
///
/// Zero when the window is empty or the product overflows — both are readings
/// no real measurement produces, and zero fails any plausibility bound the
/// caller applies.
pub const fn divided_hz(counted: u64, pit_ticks: u16) -> u64 {
    if pit_ticks == 0 {
        return 0;
    }
    let denominator = pit_ticks as u64;
    match counted.checked_mul(CRYSTAL_HZ as u64) {
        Some(numerator) => (numerator + denominator / 2) / denominator,
        None => 0,
    }
}

/// The reload count that makes the measured clock expire `tick_hz` times a
/// second.
///
/// `None` when the window is empty, the rate asked for is zero, the count
/// would not fit the timer's thirty-two bits, or it would be zero — a zero
/// reload count stops the hardware rather than running it fast.
pub const fn periodic_initial_count(counted: u64, pit_ticks: u16, tick_hz: u32) -> Option<u32> {
    if pit_ticks == 0 || tick_hz == 0 {
        return None;
    }
    let numerator = match counted.checked_mul(CRYSTAL_HZ as u64) {
        Some(product) => product,
        None => return None,
    };
    let denominator = pit_ticks as u64 * tick_hz as u64;
    let count = (numerator + denominator / 2) / denominator;
    if count == 0 || count > u32::MAX as u64 {
        return None;
    }
    Some(count as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A count that is an exact multiple of the window makes the ratio exact:
    /// ten cycles per crystal tick is ten times the crystal.
    #[test]
    fn an_exact_ratio_calibrates_exactly() {
        assert_eq!(divided_hz(200_000, 20_000), 10 * CRYSTAL_HZ as u64);
        assert_eq!(periodic_initial_count(200_000, 20_000, 100), Some(119_318));
    }

    /// 1_193_182 / 4 is 298_295.5: nearest rounds up where floor loses half a
    /// hertz per crystal tick.
    #[test]
    fn rounding_is_to_nearest_not_floor() {
        assert_eq!(divided_hz(1, 4), 298_296);
    }

    #[test]
    fn an_empty_window_yields_nothing() {
        assert_eq!(divided_hz(500, 0), 0);
        assert_eq!(periodic_initial_count(500, 0, 100), None);
    }

    #[test]
    fn a_zero_rate_yields_nothing() {
        assert_eq!(periodic_initial_count(500, 100, 0), None);
    }

    #[test]
    fn a_counter_that_never_moved_yields_nothing() {
        assert_eq!(periodic_initial_count(0, 20_000, 100), None);
    }

    /// A clock fast enough to overflow the reload count is refused, not
    /// truncated: one crystal tick of window and one expiry a second pushes
    /// the count past thirty-two bits.
    #[test]
    fn a_count_past_thirty_two_bits_yields_nothing() {
        assert_eq!(periodic_initial_count(u32::MAX as u64, 1, 1), None);
    }

    /// The numbers the two real machines produce: QEMU models the APIC clock
    /// at a gigahertz, VMware has traditionally advertised sixty-six
    /// megahertz. Both measured over a genuine 20 ms window.
    #[test]
    fn qemu_and_vmware_shaped_numbers_land_in_sane_bounds() {
        let window = 23_863;

        let hz = divided_hz(20_000_000, window);
        assert!((999_000_000..=1_001_000_000).contains(&hz), "qemu {hz}");
        let count = periodic_initial_count(20_000_000, window, 100).unwrap();
        assert!((9_990_000..=10_010_000).contains(&count), "qemu {count}");

        let hz = divided_hz(1_320_000, window);
        assert!((65_000_000..=67_000_000).contains(&hz), "vmware {hz}");
        let count = periodic_initial_count(1_320_000, window, 100).unwrap();
        assert!((650_000..=670_000).contains(&count), "vmware {count}");
    }

    /// The plausible extremes both fit: a 10 MHz bus still yields a count with
    /// hundred-nanosecond interpolation grain, and a 5 GHz one stays far
    /// inside sixty-four bits.
    #[test]
    fn the_plausible_bus_range_stays_representable() {
        let slow = periodic_initial_count(200_000, 23_863, 100).unwrap();
        assert!((99_000..101_000).contains(&slow), "slow {slow}");

        let fast = periodic_initial_count(100_000_000, 23_863, 100).unwrap();
        assert!((49_900_000..=50_100_000).contains(&fast), "fast {fast}");
    }
}
