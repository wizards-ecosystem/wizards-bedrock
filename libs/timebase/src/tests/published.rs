use crate::{Seq, Timebase};

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
