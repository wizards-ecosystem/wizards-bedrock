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

mod published;
