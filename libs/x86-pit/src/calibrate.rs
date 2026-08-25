//! Turning one measurement window into a clock's rate.
//!
//! A window is a pair: cycles some other clock counted while the 8254 counted
//! `pit_ticks` of its crystal. One window can be shared by every clock read
//! across it — the kernel measures the APIC timer and the timestamp counter
//! in the same one — so this is a single function, applied once per clock.
//!
//! Rounding is to nearest, from the raw numbers, exactly once. Deriving
//! anything from an already-rounded rate would compound two floors.

use crate::CRYSTAL_HZ;

/// The measured clock's rate in hertz, from one window against the crystal.
///
/// Zero when the window is empty or the product overflows — both are readings
/// no real measurement produces, and zero fails any plausibility bound the
/// caller applies.
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

/// The product plus the half-denominator that makes the division round to
/// nearest — `None` if either step overflows, so the caller stays total on
/// inputs no real measurement produces.
const fn rounded_numerator(counted: u64, denominator: u64) -> Option<u64> {
    match counted.checked_mul(CRYSTAL_HZ as u64) {
        Some(product) => product.checked_add(denominator / 2),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A count that is an exact multiple of the window makes the ratio exact:
    /// ten cycles per crystal tick is ten times the crystal.
    #[test]
    fn an_exact_ratio_calibrates_exactly() {
        assert_eq!(measured_hz(200_000, 20_000), 10 * CRYSTAL_HZ as u64);
    }

    /// 1_193_182 / 4 is 298_295.5: nearest rounds up where floor would lose
    /// half a hertz per crystal tick.
    #[test]
    fn rounding_is_to_nearest_not_floor() {
        assert_eq!(measured_hz(1, 4), 298_296);
    }

    /// Inputs at the top of the space must answer, not overflow.
    ///
    /// The *product*'s `checked_mul` is what these two lines exercise. The
    /// rounding addend's `checked_add` is unreachable and the doc used to
    /// claim it was covered: `u64::MAX mod CRYSTAL_HZ` is 958_759 and the
    /// largest addend is `u16::MAX / 2`, so the sum never wraps — mutating it
    /// to `wrapping_add` survived all eighteen tests (D-105). Stated rather
    /// than tested, because the input that would reach it does not exist.
    #[test]
    fn the_product_at_the_top_of_the_space_cannot_overflow() {
        let largest_counted = u64::MAX / CRYSTAL_HZ as u64;

        assert!(measured_hz(largest_counted, u16::MAX) > 0);
        assert_eq!(measured_hz(largest_counted + 1, u16::MAX), 0);

        // The headroom the addend relies on, so a change to `CRYSTAL_HZ` that
        // removed it is visible here rather than nowhere.
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

    /// The numbers the two real machines produce, for both clocks a window is
    /// now shared by. QEMU models the APIC clock at a gigahertz and passes
    /// the host's timestamp counter through; VMware has traditionally
    /// advertised a sixty-six megahertz bus. All measured over a genuine
    /// 20 ms window.
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

    /// The plausible extremes both fit: a 10 MHz bus and a 5 GHz counter are
    /// measured from the same window arithmetic without either saturating.
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
}
