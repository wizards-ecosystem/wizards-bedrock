use crate::{Seq, Timebase};

/// The published page and the kernel clock share one conversion, so the two must remain one clock.
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

/// A base in the wrong domain still looks plausible; only the difference of two readings shows it.
#[test]
fn a_published_clock_advances_with_its_counter() {
    let clock = Timebase::nanoseconds(1_000_000_000).expect("a 1 GHz counter");

    let first = clock.resume(1_000_000, 0, 0);
    let later = clock.resume(1_000_000, 0, 250_000);

    assert_eq!(first, 1_000_000, "no ticks is the base itself");
    assert_eq!(later - first, 250_000, "a 1 GHz tick is a nanosecond");
}

/// The carried fraction exists for this; the rate makes a step of one millisecond leave a fraction.
#[test]
fn a_thousand_updates_do_not_drift_from_one_conversion() {
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

/// The kernel advances the base with one and a program reads between updates with the other.
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

/// A wrap here would be time going backwards, which every reader treats as impossible.
#[test]
fn a_reading_too_large_for_the_word_saturates() {
    let clock = Timebase::nanoseconds(1_000_000_000).expect("a 1 GHz counter");

    assert_eq!(clock.resume(u64::MAX, 0, u64::MAX), u64::MAX);
    assert_eq!(clock.resume(0, 0, u64::MAX), u64::MAX);
    assert_eq!(clock.advance(u64::MAX, 0, u64::MAX), (u64::MAX, 0));
}

/// A reader testing only equality accepts a read taken entirely inside one update.
#[test]
fn a_reader_keeps_what_it_read_only_when_nothing_moved() {
    let quiet = Seq::QUIET;
    assert!(Seq::is_stable(quiet));
    assert!(Seq::agrees(quiet, quiet));

    let writing = Seq::begin(quiet);
    assert!(!Seq::is_stable(writing));
    assert!(!Seq::agrees(writing, writing));
    assert!(!Seq::agrees(quiet, writing));

    let written = Seq::end(writing);
    assert!(Seq::is_stable(written));
    assert_ne!(written, quiet, "a write must be visible as a change");
    assert!(!Seq::agrees(quiet, written));
    assert!(Seq::agrees(written, written));
}

/// The seam is thirty-five years away at a thousand updates a second, so it is checked here.
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
