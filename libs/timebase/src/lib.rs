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
            // The half-denominator rounds to nearest rather than towards
            // zero, halving the worst-case error of every conversion.
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

    /// Rebuild a conversion from published multiplier and shift fields.
    ///
    /// A zero multiplier remains a valid uncalibrated conversion. Other
    /// fields must fit the representation used by [`to_bits`](Self::to_bits).
    pub const fn from_parts(multiplier: u64, shift: u32) -> Option<Self> {
        if multiplier > MAX_MULTIPLIER {
            return None;
        }
        if shift > MAX_SHIFT {
            return None;
        }
        Some(Self { multiplier, shift })
    }

    /// The multiplier this conversion applies.
    ///
    /// For a clock that is *published* rather than called: a reader with no
    /// lock cannot ask a `Timebase` anything, so the two numbers have to be
    /// written into a page it can see.
    pub const fn multiplier(self) -> u64 {
        self.multiplier
    }

    /// The shift this conversion applies.
    pub const fn shift(self) -> u32 {
        self.shift
    }

    /// Read a published clock: whole nanoseconds, a carried fraction, and
    /// `ticks` of the counter since it was published.
    ///
    /// **One multiply-add and one shift, no division**, which is what D-152
    /// asks of the reader — a program with no lock and no syscall has to be
    /// able to do this in a few instructions.
    ///
    /// The base is split rather than stored wholly pre-shifted, and that is
    /// not a detail. `base_ns << shift` overflows a `u64` after about eight
    /// seconds of uptime at a 1 GHz counter's shift, so the obvious form is
    /// wrong on any machine that has been on for a minute. Linux splits it the
    /// same way — whole seconds beside a shifted remainder — for the same
    /// reason. What is carried here is a *fraction of a nanosecond*, always
    /// below `1 << shift`, so the sum below cannot overflow for any tick count
    /// a real update interval produces.
    ///
    /// # Examples
    ///
    /// ```
    /// use timebase::Timebase;
    ///
    /// let clock = Timebase::nanoseconds(1_000_000_000).expect("a 1 GHz counter");
    ///
    /// // A thousand ticks of a 1 GHz counter is a thousand nanoseconds.
    /// assert_eq!(clock.resume(5_000, 0, 1_000), 6_000);
    /// ```
    pub const fn resume(self, base_ns: u64, base_fraction: u64, ticks: u64) -> u64 {
        let advanced = base_fraction as u128 + (ticks as u128 * self.multiplier as u128);
        let whole = advanced >> self.shift;
        if whole > u64::MAX as u128 {
            return u64::MAX;
        }
        base_ns.saturating_add(whole as u64)
    }

    /// Advance a published clock by `ticks`, keeping the fraction.
    ///
    /// What the kernel does at each update, and the reason the fraction exists
    /// at all: throwing it away would lose up to a nanosecond *per update*, so
    /// a clock updated a thousand times a second would fall a microsecond
    /// behind every second. Carrying it costs one mask.
    ///
    /// The reading this produces is the one [`resume`](Self::resume) would
    /// have given for the same arguments, which is what lets a program's own
    /// read and the kernel's agree rather than merely resemble each other.
    pub const fn advance(self, base_ns: u64, base_fraction: u64, ticks: u64) -> (u64, u64) {
        let advanced = base_fraction as u128 + (ticks as u128 * self.multiplier as u128);
        let whole = advanced >> self.shift;
        if whole > u64::MAX as u128 {
            return (u64::MAX, 0);
        }
        let carried = advanced - (whole << self.shift);
        (base_ns.saturating_add(whole as u64), carried as u64)
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
    #[test]
    fn a_monotonic_poll_limit_expires_by_time_not_iteration_rate() {
        let mut limit = PollLimit::after_ms(Some(1_000_000_000), 500, 1);
        for _ in 0..10_000 {
            assert!(limit.permit(Some(1_499_999_999)));
        }
        assert!(!limit.permit(Some(1_500_000_000)));
        assert!(!limit.permit(None), "a vanished clock fails closed");
    }

    #[test]
    fn a_human_wait_uses_the_exact_forty_five_second_nanosecond_boundary() {
        const START: u64 = 7_000_000_000;
        const WAIT_MS: u64 = 45_000;
        let mut limit = PollLimit::after_ms(Some(START), WAIT_MS, 1);

        assert!(limit.permit(Some(START + 44_999_999_999)));
        assert!(!limit.permit(Some(START + 45_000_000_000)));
    }

    #[test]
    fn a_clockless_poll_limit_permits_its_exact_fallback_count() {
        let mut limit = PollLimit::after_ms(None, u64::MAX, 3);
        assert!(limit.permit(None));
        assert!(limit.permit(Some(u64::MAX)));
        assert!(limit.permit(None));
        assert!(!limit.permit(None));
    }

    #[test]
    fn a_deadline_saturates_instead_of_wrapping_into_the_past() {
        let mut limit = PollLimit::after_ms(Some(u64::MAX - 1), u64::MAX, 0);
        assert!(limit.permit(Some(u64::MAX - 1)));
        assert!(!limit.permit(Some(u64::MAX)));
    }
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
        // Real frequencies with awkward ratios, one chosen so nearest and
        // ceiling differ.
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

    #[test]
    fn raw_parts_refuse_a_multiplier_that_cannot_be_published() {
        assert!(Timebase::from_parts(MAX_MULTIPLIER, MAX_SHIFT).is_some());
        assert!(Timebase::from_parts(MAX_MULTIPLIER + 1, MAX_SHIFT).is_none());
        assert!(Timebase::from_parts(u64::MAX, 0).is_none());
    }

    #[test]
    fn raw_parts_refuse_a_shift_that_cannot_be_evaluated() {
        assert!(Timebase::from_parts(MAX_MULTIPLIER, MAX_SHIFT).is_some());
        assert!(Timebase::from_parts(MAX_MULTIPLIER, MAX_SHIFT + 1).is_none());
        assert!(Timebase::from_parts(0, u32::MAX).is_none());
    }

    #[test]
    fn raw_boundary_fields_round_trip_through_one_word() {
        for shift in 0..=MAX_SHIFT {
            for multiplier in [0, 1, MAX_MULTIPLIER / 2, MAX_MULTIPLIER] {
                let conversion = Timebase::from_parts(multiplier, shift)
                    .expect("the fields fit the published representation");
                assert_eq!(Timebase::from_bits(conversion.to_bits()), conversion);
            }
        }
    }

    #[test]
    fn raw_field_accessors_preserve_nontrivial_values() {
        let conversion = Timebase::from_parts(0x1234_5678, 37).expect("valid fields");
        assert_eq!(conversion.multiplier(), 0x1234_5678);
        assert_eq!(conversion.shift(), 37);
    }

    #[test]
    fn published_arithmetic_distinguishes_the_u64_boundary_from_overflow() {
        let exact = Timebase::from_parts(2, 1).expect("valid fields");
        assert_eq!(exact.resume(0, 1, u64::MAX), u64::MAX);
        assert_eq!(exact.advance(0, 1, u64::MAX), (u64::MAX, 1));

        let overflow = Timebase::from_parts(2, 0).expect("valid fields");
        assert_eq!(overflow.resume(0, 0, u64::MAX), u64::MAX);
        assert_eq!(overflow.advance(0, 0, u64::MAX), (u64::MAX, 0));
    }

    /// Arbitrary stored bits must decode without producing an invalid shift.
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

    /// A real-clock conversion must spend every representable precision bit.
    #[test]
    fn the_search_takes_the_largest_shift_that_fits() {
        for hz in PLAUSIBLE_HZ {
            for conversion in [
                Timebase::nanoseconds(*hz).unwrap(),
                Timebase::ticks(*hz).unwrap(),
            ] {
                // Only the final assertion tests the search; these two record
                // representation bounds.
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

/// The sequence counter of a clock published to readers that cannot lock.
///
/// This type supplies only wrapping counter arithmetic. Odd marks an active
/// write; readers accept only matching even observations. The publisher and
/// reader must supply both memory-ordering barriers; `kernel::timepage` owns
/// them for the current caller (D-152). Correctness also requires that a full
/// `u32` counter cycle cannot occur inside one read.
///
/// # Examples
///
/// ```
/// use timebase::Seq;
///
/// let quiet = 4;
/// assert!(Seq::is_stable(quiet));
///
/// let writing = Seq::begin(quiet);
/// assert!(!Seq::is_stable(writing));
/// assert!(!Seq::agrees(quiet, writing));
///
/// let written = Seq::end(writing);
/// assert!(Seq::is_stable(written));
/// // A reader that started before the write does not accept what it read.
/// assert!(!Seq::agrees(quiet, written));
/// ```
#[derive(Debug)]
pub struct Seq;

impl Seq {
    /// What a page holds before anything has ever been published.
    pub const QUIET: u32 = 0;

    /// Whether no write is in progress.
    pub const fn is_stable(seq: u32) -> bool {
        seq.is_multiple_of(2)
    }

    /// The value a writer stores before it begins.
    pub const fn begin(seq: u32) -> u32 {
        seq.wrapping_add(1)
    }

    /// The value a writer stores when it is done.
    pub const fn end(seq: u32) -> u32 {
        seq.wrapping_add(1)
    }

    /// Whether a read that saw `before` and then `after` may keep what it read.
    ///
    /// Both conditions, and they are different: **equal** rules out a write
    /// that started and finished inside the read, and **even** rules out one
    /// that was already in progress when it started. Checking only equality
    /// accepts a read taken entirely inside an update, which is the mistake
    /// that makes a seqlock look like it works.
    pub const fn agrees(before: u32, after: u32) -> bool {
        before == after && Self::is_stable(before)
    }
}

#[cfg(test)]
mod published_tests {
    use super::{Seq, Timebase};

    /// A published reading is the reading the conversion would have given.
    ///
    /// The two must not drift, because the published page and the kernel clock
    /// use the same conversion. Ring Uptime is the revocation fallback, so its
    /// answer and the page's answer must remain the same clock (D-166, D-181).
    ///
    /// Exactly equal when the fraction starts at zero: both are
    /// `(ticks * mult) >> shift` and the base is added outside the shift.
    #[test]
    fn a_published_reading_matches_the_conversion_it_came_from() {
        for hz in [1_000_000_000, 2_400_000_000, 3_192_000_000, 14_318_180, 100] {
            let clock = Timebase::nanoseconds(hz).expect("a real rate");
            for base in [0u64, 1, 999, 1_000_000, 86_400_000_000_000] {
                for ticks in [0u64, 1, 3, 1_000, 1_000_000, 1_000_000_007] {
                    assert_eq!(
                        clock.resume(base, 0, ticks),
                        base + clock.convert(ticks),
                        "{hz} Hz, base {base}, {ticks} ticks"
                    );
                }
            }
        }
    }

    /// A published clock advances by what the counter advanced.
    ///
    /// The property a reader depends on and the one a wrong shift breaks
    /// silently: a base in the wrong domain still produces plausible numbers,
    /// and only the *difference* between two readings shows it.
    #[test]
    fn a_published_clock_advances_with_its_counter() {
        let clock = Timebase::nanoseconds(1_000_000_000).expect("a 1 GHz counter");

        let first = clock.resume(1_000_000, 0, 0);
        let later = clock.resume(1_000_000, 0, 250_000);

        assert_eq!(first, 1_000_000, "no ticks is the base itself");
        assert_eq!(later - first, 250_000, "a 1 GHz tick is a nanosecond");
    }

    /// Updating a thousand times does not drift from converting once.
    ///
    /// This is what the carried fraction is *for*, and the check that fails
    /// without it. A counter whose rate is not a whole number of nanoseconds
    /// loses part of one at every update; dropping the fraction costs up to a
    /// nanosecond a time, so a clock updated at a kilohertz falls a microsecond
    /// behind every second and nothing else in this file would notice.
    #[test]
    fn a_thousand_updates_do_not_drift_from_one_conversion() {
        // Chosen so the two answers can differ: a step of exactly one
        // millisecond has no fraction to lose and left this green (D-155).
        let clock = Timebase::nanoseconds(3_192_000_000).expect("a real rate");
        const STEP: u64 = 1_000_000;
        const UPDATES: u64 = 1_000;

        let (mut base, mut fraction) = (0u64, 0u64);
        for _ in 0..UPDATES {
            (base, fraction) = clock.advance(base, fraction, STEP);
        }

        let at_once = clock.convert(STEP * UPDATES);
        let apart = base.abs_diff(at_once);
        assert!(
            apart <= 1,
            "{UPDATES} updates read {base}, one conversion reads {at_once}: {apart} ns apart"
        );
    }

    /// An update leaves the clock where a read of the same moment would.
    ///
    /// The two are the same arithmetic and must stay so: the kernel advances
    /// the base with one and a program reads between updates with the other.
    #[test]
    fn advancing_lands_where_reading_the_same_moment_does() {
        let clock = Timebase::nanoseconds(2_400_000_000).expect("a real rate");
        for fraction in [0u64, 1, 12_345] {
            for ticks in [0u64, 7, 2_400_000, 999_999_999] {
                let (advanced, carried) = clock.advance(1_000, fraction, ticks);
                assert_eq!(advanced, clock.resume(1_000, fraction, ticks));
                assert!(
                    carried < 1 << clock.shift(),
                    "the carry is a fraction of a nanosecond, not a nanosecond"
                );
            }
        }
    }

    /// A reading too large for the word saturates rather than wrapping.
    ///
    /// A wrap here is time going *backwards*, which every reader treats as
    /// impossible.
    #[test]
    fn a_reading_too_large_for_the_word_saturates() {
        let clock = Timebase::nanoseconds(1_000_000_000).expect("a 1 GHz counter");

        assert_eq!(clock.resume(u64::MAX, 0, u64::MAX), u64::MAX);
        assert_eq!(clock.resume(0, 0, u64::MAX), u64::MAX);
        assert_eq!(clock.advance(u64::MAX, 0, u64::MAX), (u64::MAX, 0));
    }

    /// The sequence counter says "stable" only when it is even and unchanged.
    ///
    /// Both halves are checked separately, because a reader that tests only
    /// equality accepts a read taken entirely inside one update — the mistake
    /// that leaves a seqlock looking correct.
    #[test]
    fn a_reader_keeps_what_it_read_only_when_nothing_moved() {
        let quiet = Seq::QUIET;
        assert!(Seq::is_stable(quiet));
        assert!(Seq::agrees(quiet, quiet));

        let writing = Seq::begin(quiet);
        assert!(!Seq::is_stable(writing));
        // Caught mid-write, twice over: unchanged but odd is still a refusal.
        assert!(!Seq::agrees(writing, writing));
        assert!(!Seq::agrees(quiet, writing));

        let written = Seq::end(writing);
        assert!(Seq::is_stable(written));
        assert_ne!(written, quiet, "a write must be visible as a change");
        assert!(!Seq::agrees(quiet, written));
        assert!(Seq::agrees(written, written));
    }

    /// The counter wraps without ever reading as stable at the seam.
    ///
    /// Thirty-five years of updates at a thousand a second, so this is not a
    /// reachable state — which is why it is checked here, where reaching it
    /// costs nothing.
    #[test]
    fn the_sequence_counter_wraps_without_going_odd_at_the_seam() {
        let last_even = u32::MAX - 1;
        assert!(Seq::is_stable(last_even));

        let writing = Seq::begin(last_even);
        assert_eq!(writing, u32::MAX);
        assert!(!Seq::is_stable(writing));

        let wrapped = Seq::end(writing);
        assert_eq!(wrapped, 0);
        assert!(Seq::is_stable(wrapped));
        assert!(!Seq::agrees(last_even, wrapped));
    }
}

/// A hardware poll bounded either by monotonic time or by boot-time attempts.
///
/// The mode is chosen once. After calibration, code generation and processor
/// speed cannot change the real duration; before calibration, a caller still
/// has a finite, explicit fallback rather than an accidental endless wait
/// (D-200).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollLimit {
    /// Stop at this monotonic nanosecond.
    Deadline(u64),
    /// Permit exactly this many more hardware reads.
    Attempts(u32),
}

impl PollLimit {
    /// Choose a monotonic deadline when `now_ns` exists, or `fallback_attempts`
    /// while the boot clock is unavailable.
    pub const fn after_ms(now_ns: Option<u64>, timeout_ms: u64, fallback_attempts: u32) -> Self {
        match now_ns {
            Some(now) => Self::Deadline(now.saturating_add(timeout_ms.saturating_mul(1_000_000))),
            None => Self::Attempts(fallback_attempts),
        }
    }

    /// Whether one more hardware read is permitted at `now_ns`.
    ///
    /// A monotonic limit does not spend attempts: any number of fast polls
    /// sees the same real deadline. An attempt limit ignores a clock which
    /// appears later, so boot behaviour cannot change mode halfway through a
    /// transaction.
    pub const fn permit(&mut self, now_ns: Option<u64>) -> bool {
        match self {
            Self::Deadline(deadline) => match now_ns {
                Some(now) => now < *deadline,
                None => false,
            },
            Self::Attempts(remaining) if *remaining > 0 => {
                *remaining -= 1;
                true
            }
            Self::Attempts(_) => false,
        }
    }
}
