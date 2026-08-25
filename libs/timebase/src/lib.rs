//! Turning a counter's ticks into nanoseconds, and nanoseconds back into
//! ticks.
//!
//! A counter measured at some arbitrary rate, converted to nanoseconds — and
//! nanoseconds back into ticks of a second counter at a third rate. Both are
//! one rational multiplication, precomputed once as the rounded fixed-point
//! ratio `value × multiplier >> shift`, because dividing would put a
//! sixty-four-bit division on the path every clock read takes.
//!
//! The multiplier is held to thirty-two bits, which keeps the product inside
//! a `u128` and lets a whole conversion pack into one `u64` — how a kernel
//! publishes one to a reader inside an interrupt handler, where no lock can
//! go. The shift is then the largest that still fits, so a conversion carries
//! at least thirty-one significant bits: under a nanosecond per second.
//!
//! # Examples
//!
//! ```
//! use timebase::Timebase;
//!
//! // A counter measured at two and a half gigahertz.
//! let elapsed = Timebase::nanoseconds(2_500_000_000).unwrap();
//! assert_eq!(elapsed.convert(2_500_000_000), 1_000_000_000);
//!
//! // And the timer that has to be armed in its own ticks.
//! let countdown = Timebase::ticks(1_000_000_000).unwrap();
//! assert_eq!(countdown.convert(3_000_000), 3_000_000);
//!
//! // A conversion survives being published as one word.
//! assert_eq!(Timebase::from_bits(elapsed.to_bits()), elapsed);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Nanoseconds in a second, which is the unit on both sides of every
/// conversion here.
pub const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

/// The largest multiplier a conversion may carry.
///
/// Thirty-two bits, so that `u64 × multiplier` cannot overflow a `u128` and
/// so that multiplier and shift together fit one `u64`.
const MAX_MULTIPLIER: u64 = u32::MAX as u64;

/// The largest shift, and the reason [`Timebase::from_bits`] masks: a shift
/// of sixty-four or more is undefined, and this type accepts bits it did not
/// write.
const MAX_SHIFT: u32 = 63;

/// Bits of a packed conversion that hold the shift.
const SHIFT_BITS: u32 = 32;

/// One direction of one clock's conversion: multiply, then shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timebase {
    multiplier: u64,
    shift: u32,
}

impl Timebase {
    /// The conversion of a clock that was never measured: everything it is
    /// asked converts to zero.
    ///
    /// Deliberately what a zeroed word decodes to, so a kernel publishing
    /// conversions into `static`s reads "no clock yet" without a second flag.
    pub const UNCALIBRATED: Self = Self {
        multiplier: 0,
        shift: 0,
    };

    /// The conversion that multiplies by `numerator / denominator`.
    ///
    /// `None` if either side is zero, or if the ratio is so large or so small
    /// that no shift represents it in thirty-two bits — a ratio above four
    /// billion, or below one part in about eighteen quintillion. Neither is a
    /// pair of real clocks.
    pub const fn new(numerator: u64, denominator: u64) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            return None;
        }
        let denominator = denominator as u128;
        let mut shift = MAX_SHIFT;
        loop {
            // The half-denominator is what makes the division round to
            // nearest rather than towards zero, which halves the worst-case
            // error of every conversion made through it.
            let multiplier = (((numerator as u128) << shift) + denominator / 2) / denominator;
            if multiplier > 0 && multiplier <= MAX_MULTIPLIER as u128 {
                return Some(Self {
                    multiplier: multiplier as u64,
                    shift,
                });
            }
            if shift == 0 {
                return None;
            }
            shift -= 1;
        }
    }

    /// Ticks of a counter running at `hz`, into nanoseconds.
    pub const fn nanoseconds(hz: u64) -> Option<Self> {
        Self::new(NANOSECONDS_PER_SECOND, hz)
    }

    /// Nanoseconds, into ticks of a counter running at `hz`.
    pub const fn ticks(hz: u64) -> Option<Self> {
        Self::new(hz, NANOSECONDS_PER_SECOND)
    }

    /// Whether this conversion came from a measurement.
    pub const fn is_calibrated(self) -> bool {
        self.multiplier != 0
    }

    /// Apply the conversion, saturating rather than wrapping.
    ///
    /// Saturating because this is a clock: a reading past the end of the range
    /// should read as far ahead as the type allows, never as though it had
    /// wrapped to the beginning.
    pub const fn convert(self, value: u64) -> u64 {
        // Both factors are bounded — any `u64` and at most thirty-two bits —
        // so the product is at most ninety-six bits and cannot overflow.
        let scaled = (value as u128 * self.multiplier as u128) >> self.shift;
        if scaled > u64::MAX as u128 {
            u64::MAX
        } else {
            scaled as u64
        }
    }

    /// The conversion as one word, for publishing where a lock cannot go.
    pub const fn to_bits(self) -> u64 {
        (self.shift as u64) << SHIFT_BITS | self.multiplier
    }

    /// Read back what [`to_bits`](Self::to_bits) wrote.
    ///
    /// Total on every input: the shift is masked, so a word of nonsense
    /// converts to nonsense rather than to undefined behaviour.
    pub const fn from_bits(bits: u64) -> Self {
        Self {
            multiplier: bits & MAX_MULTIPLIER,
            shift: ((bits >> SHIFT_BITS) as u32) & MAX_SHIFT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rates worth believing from a real machine: the 8254's crystal at the
    /// bottom, an APIC bus clock, the gigahertz QEMU models, two host CPUs,
    /// and a deliberately absurd one above them all.
    const PLAUSIBLE_HZ: &[u64] = &[
        1_193_182,
        66_000_000,
        100_000_000,
        1_000_000_000,
        2_400_000_000,
        3_300_000_000,
        5_000_000_000,
        50_000_000_000,
    ];

    /// The multiplier is the **nearest** integer to the exact ratio.
    ///
    /// The implementation gets that with `+ denominator / 2`, and every test
    /// here passed with it changed to `+ denominator - 1` — round *up* — which
    /// doubles the worst-case error of every conversion and biases it in one
    /// direction. A clock whose error is one-sided drifts; one whose error is
    /// symmetric does not (D-155).
    ///
    /// Checked against the two candidates rather than against the trick: the
    /// claim is "nearest", so the test computes both neighbours and their
    /// distances and requires the closer one.
    #[test]
    fn the_multiplier_is_the_nearest_integer_not_the_ceiling() {
        // Real frequencies, including ones whose ratios are awkward: a 100 MHz
        // APIC, an odd TSC, and one deliberately chosen so the exact ratio
        // falls below the halfway point and nearest differs from ceiling.
        for hz in [
            100_000_000u64,
            1_000_000_000,
            2_500_000_000,
            3_300_000_000,
            1_193_182,
            999_999_937,
        ] {
            let conversion = Timebase::nanoseconds(hz).expect("a usable conversion");
            let bits = conversion.to_bits();
            let multiplier = u128::from(bits & u64::from(u32::MAX));
            let shift = (bits >> SHIFT_BITS) as u32;

            let exact = u128::from(NANOSECONDS_PER_SECOND) << shift;
            let denominator = u128::from(hz);
            let floor = exact / denominator;
            let ceiling = floor + 1;

            let below = exact - floor * denominator;
            let above = ceiling * denominator - exact;
            let nearest = if above < below { ceiling } else { floor };

            assert_eq!(
                multiplier, nearest,
                "at {hz} Hz the multiplier is not the nearest integer to the ratio"
            );
        }
    }

    #[test]
    fn a_gigahertz_counter_counts_nanoseconds_exactly() {
        let elapsed = Timebase::nanoseconds(1_000_000_000).unwrap();
        let countdown = Timebase::ticks(1_000_000_000).unwrap();

        for ticks in [0, 1, 1_000, 1_000_000_000, u32::MAX as u64] {
            assert_eq!(elapsed.convert(ticks), ticks);
            assert_eq!(countdown.convert(ticks), ticks);
        }
    }

    /// The property everything else rests on: a second's worth of a clock's
    /// ticks is a second. The bound is four nanoseconds — the rounding of a
    /// thirty-one-bit multiplier over a billion, plus the final shift's floor
    /// — and it is tight enough that losing a bit of the multiplier breaks it.
    #[test]
    fn a_second_of_every_plausible_clock_converts_to_a_second() {
        for hz in PLAUSIBLE_HZ {
            let elapsed = Timebase::nanoseconds(*hz).unwrap();
            let measured = elapsed.convert(*hz);
            assert!(
                measured.abs_diff(NANOSECONDS_PER_SECOND) <= 4,
                "{hz} hz measured a second as {measured} ns"
            );

            let countdown = Timebase::ticks(*hz).unwrap();
            let counted = countdown.convert(NANOSECONDS_PER_SECOND);
            assert!(
                counted.abs_diff(*hz) <= hz / 100_000_000 + 1,
                "{hz} hz counted a second as {counted} ticks"
            );
        }
    }

    /// A conversion out and back must land where it started, give or take the
    /// grain of the clock it went through — one tick's worth of nanoseconds,
    /// plus the part per billion each of the two multipliers may round away.
    #[test]
    fn a_round_trip_loses_less_than_one_tick() {
        for hz in PLAUSIBLE_HZ {
            let to_ticks = Timebase::ticks(*hz).unwrap();
            let to_nanoseconds = Timebase::nanoseconds(*hz).unwrap();

            for nanoseconds in [1_000, 1_000_000, 3_000_000, 1_000_000_000, 60_000_000_000] {
                let grain = NANOSECONDS_PER_SECOND / hz + nanoseconds / 1_000_000_000 + 2;
                let there_and_back = to_nanoseconds.convert(to_ticks.convert(nanoseconds));
                assert!(
                    there_and_back.abs_diff(nanoseconds) <= grain,
                    "{hz} hz turned {nanoseconds} ns into {there_and_back} ns"
                );
            }
        }
    }

    /// The counter is read for as long as the machine is up, so the error has
    /// to stay proportional rather than accumulate: a year of ticks converts
    /// to a year within a part per million.
    #[test]
    fn a_year_of_ticks_stays_within_a_part_per_million() {
        const SECONDS_IN_A_YEAR: u64 = 365 * 24 * 60 * 60;

        for hz in PLAUSIBLE_HZ {
            let elapsed = Timebase::nanoseconds(*hz).unwrap();
            let expected = SECONDS_IN_A_YEAR * NANOSECONDS_PER_SECOND;
            let measured = elapsed.convert(hz * SECONDS_IN_A_YEAR);
            assert!(
                measured.abs_diff(expected) <= expected / 1_000_000,
                "{hz} hz drifted to {measured} ns over a year"
            );
        }
    }

    /// Time may not run backwards because the arithmetic under it did.
    #[test]
    fn conversion_never_decreases() {
        let elapsed = Timebase::nanoseconds(3_300_000_000).unwrap();
        let mut previous = 0;
        for ticks in (0..100_000).map(|step| step * 7919) {
            let converted = elapsed.convert(ticks);
            assert!(converted >= previous, "{ticks} ticks went backwards");
            previous = converted;
        }
    }

    #[test]
    fn an_uncalibrated_conversion_answers_zero() {
        assert!(!Timebase::UNCALIBRATED.is_calibrated());
        assert_eq!(Timebase::UNCALIBRATED.convert(u64::MAX), 0);
        assert_eq!(Timebase::from_bits(0), Timebase::UNCALIBRATED);
        assert!(
            Timebase::nanoseconds(1_000_000_000)
                .unwrap()
                .is_calibrated()
        );
    }

    #[test]
    fn a_clock_of_no_rate_has_no_conversion() {
        assert!(Timebase::nanoseconds(0).is_none());
        assert!(Timebase::ticks(0).is_none());
        assert!(Timebase::new(0, 1).is_none());
        assert!(Timebase::new(1, 0).is_none());
    }

    /// A ratio no pair of clocks produces, refused rather than truncated: a
    /// clock over four billion times faster than the unit it converts into
    /// has no thirty-two-bit multiplier at any shift.
    #[test]
    fn a_ratio_too_large_for_the_multiplier_is_refused() {
        assert!(Timebase::new(u64::MAX, 1).is_none());
        assert!(Timebase::new(MAX_MULTIPLIER, 1).is_some());
        assert!(Timebase::ticks(u64::MAX).is_none());
    }

    #[test]
    fn a_conversion_saturates_instead_of_wrapping() {
        // One hertz: every tick is a full second of nanoseconds, so the top
        // of the range is reached long before the top of the input.
        let elapsed = Timebase::nanoseconds(1).unwrap();

        assert_eq!(elapsed.convert(u64::MAX), u64::MAX);
        assert_eq!(elapsed.convert(1), NANOSECONDS_PER_SECOND);
    }

    #[test]
    fn a_conversion_survives_being_packed_into_a_word() {
        for hz in PLAUSIBLE_HZ {
            for conversion in [Timebase::nanoseconds(*hz), Timebase::ticks(*hz)] {
                let conversion = conversion.unwrap();
                assert_eq!(Timebase::from_bits(conversion.to_bits()), conversion);
            }
        }
    }

    /// `from_bits` is fed whatever a `static` held, which on a kernel that
    /// has not calibrated yet is zero and in a bug is anything at all. It has
    /// to answer rather than shift by sixty-four, which is undefined — so the
    /// worst word there is decodes to the largest shift and multiplier the
    /// type allows, and converts by them.
    #[test]
    fn unpacking_nonsense_still_answers() {
        let nonsense = Timebase::from_bits(u64::MAX);

        assert!(nonsense.is_calibrated());
        assert_eq!(nonsense, Timebase::from_bits(0x3F_FFFF_FFFF));
        assert_eq!(nonsense.convert(0), 0);
        // The transcribed answer, not the expression `convert` evaluates:
        // `(2^64 - 1) * (2^32 - 1) >> 63` is 8_589_934_589 (D-105).
        assert_eq!(nonsense.convert(u64::MAX), 8_589_934_589);
    }

    /// The multiplier is the whole precision budget, and the search is what
    /// spends it: every conversion of a real clock must land in the top half
    /// of the thirty-two bits available, or a shift was left on the table.
    #[test]
    fn the_search_takes_the_largest_shift_that_fits() {
        for hz in PLAUSIBLE_HZ {
            for conversion in [
                Timebase::nanoseconds(*hz).unwrap(),
                Timebase::ticks(*hz).unwrap(),
            ] {
                // The two bounds below are `new`'s own loop conditions
                // restated — `multiplier <= MAX_MULTIPLIER` is its exit test
                // and `shift` only ever decrements from `MAX_SHIFT`. Kept as
                // documentation of the range; the assertion after them is the
                // one that can fail (D-105).
                assert!(conversion.multiplier <= MAX_MULTIPLIER, "{conversion:?}");
                assert!(conversion.shift <= MAX_SHIFT, "{conversion:?}");
                assert!(
                    conversion.shift == MAX_SHIFT || conversion.multiplier > MAX_MULTIPLIER / 2,
                    "{hz} hz left a shift unspent: {conversion:?}"
                );
            }
        }
    }
}
