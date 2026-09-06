//! Property tests over the placement the module doc promises.
//!
//! The module doc names its own proof obligation: "the proof (as host tests)
//! that no two slots touch". Nine unit tests carry it for the layouts a reader
//! can check by hand; this asks it of every layout the constructor accepts.
//!
//! `ranges_overlap` states three separate rules in its own doc, and each is a
//! property here rather than a case (D-242).
//!
//! What is *not* here: whether the guard is large enough. The doc says plainly
//! that "a stack frame larger than the guard can step clean over it without
//! faulting", and D-52 accepts that for a kernel built without `alloca`. No
//! arithmetic can check a property of the code generator.

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestRunner};
use stack_layout::{PAGE_SIZE, StackLayout, ranges_overlap};

/// Page-multiple lengths, the granularity the crate says guards come in.
fn pages(max: u64) -> impl Strategy<Value = u64> {
    (1u64..=max).prop_map(|count| count * PAGE_SIZE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Contact {
    Crosses,
    Adjacent,
    OnePast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Below,
    Above,
}

/// Ranges deliberately placed at both ends of a reference range.
fn touching_pair() -> impl Strategy<Value = (u64, u64, u64, u64, Contact, Side)> {
    (
        64u64..1024,
        1u64..32,
        1u64..32,
        prop_oneof![
            Just(Contact::Crosses),
            Just(Contact::Adjacent),
            Just(Contact::OnePast),
        ],
        prop_oneof![Just(Side::Below), Just(Side::Above)],
    )
        .prop_map(|(base, len, other_len, contact, side)| {
            let end = base + len;
            let other_base = match (side, contact) {
                (Side::Above, Contact::Crosses) => end - 1,
                (Side::Above, Contact::Adjacent) => end,
                (Side::Above, Contact::OnePast) => end + 1,
                (Side::Below, Contact::Crosses) => base + 1 - other_len,
                (Side::Below, Contact::Adjacent) => base - other_len,
                (Side::Below, Contact::OnePast) => base - 1 - other_len,
            };
            (base, len, other_base, other_len, contact, side)
        })
}

/// A window, a stack size, a guard size and a slot count.
fn layout() -> impl Strategy<Value = (u64, u64, u64, usize)> {
    (pages(64), pages(8), pages(4), 1usize..12)
}

proptest! {
    /// "Empty ranges touch nothing."
    #[test]
    fn an_empty_range_touches_nothing(a in any::<u64>(), b in any::<u64>(), len in any::<u64>()) {
        prop_assert!(!ranges_overlap(a, 0, b, len), "an empty left range overlapped");
        prop_assert!(!ranges_overlap(a, len, b, 0), "an empty right range overlapped");
    }

    /// "Whether two half-open byte ranges overlap."
    ///
    /// The three placements differ by one byte at an exclusive end: crossing
    /// shares one byte, while adjacency and one-past share none. Both argument
    /// orders must have the same answer.
    #[test]
    fn half_open_ranges_distinguish_crossing_adjacent_and_one_past(
        (a, alen, b, blen, contact, side) in touching_pair(),
    ) {
        let expected = contact == Contact::Crosses;
        prop_assert_eq!(
            ranges_overlap(a, alen, b, blen),
            expected,
            "{:?} range on the {:?} side had the wrong overlap answer",
            contact,
            side
        );
        prop_assert_eq!(
            ranges_overlap(b, blen, a, alen),
            expected,
            "{:?} range on the {:?} side changed when arguments were swapped",
            contact,
            side
        );
    }

    /// "An overflowing non-empty range is invalid and treated as overlapping so
    /// a caller protecting memory fails closed."
    #[test]
    fn an_overflowing_range_fails_closed(
        b in any::<u64>(),
        blen in 1u64..u64::MAX,
        over in 1u64..u64::MAX,
    ) {
        let base = u64::MAX - over + 1;
        let len = over.saturating_add(1);
        prop_assume!(base.checked_add(len).is_none());
        prop_assert!(
            ranges_overlap(base, len, b, blen),
            "an overflowing range did not fail closed"
        );
    }

    /// No two slots touch — the crate's stated proof obligation.
    ///
    /// A guard shared between two slots is a stack that faults into its
    /// neighbour's memory instead of into nothing, which is the whole failure
    /// the design exists to prevent.
    #[test]
    fn no_two_slots_touch((base, stack, guard, slots) in layout()) {
        let Some(layout) = StackLayout::new(base, stack, guard, slots) else {
            return Ok(());
        };
        let placed: Vec<_> = (0..layout.slots())
            .filter_map(|index| layout.slot(index))
            .collect();
        prop_assert_eq!(placed.len(), layout.slots(), "a slot in range was absent");

        for (i, left) in placed.iter().enumerate() {
            for (j, right) in placed.iter().enumerate() {
                if i == j {
                    continue;
                }
                prop_assert!(
                    !ranges_overlap(
                        left.stack_base(),
                        left.stack_len(),
                        right.stack_base(),
                        right.stack_len()
                    ),
                    "slots {} and {} share stack bytes",
                    i,
                    j
                );
                prop_assert!(
                    !ranges_overlap(
                        left.stack_base(),
                        left.stack_len(),
                        right.guard_base(),
                        guard
                    ),
                    "slot {}'s stack occupies slot {}'s guard",
                    i,
                    j
                );
            }
        }
    }

    /// "The guard occupies the low addresses, the stack the high ones, so a
    /// stack growing down runs into its own guard and never into a neighbour."
    #[test]
    fn a_stack_growing_down_meets_its_own_guard((base, stack, guard, slots) in layout()) {
        let Some(layout) = StackLayout::new(base, stack, guard, slots) else {
            return Ok(());
        };
        for index in 0..layout.slots() {
            let Some(slot) = layout.slot(index) else {
                continue;
            };
            prop_assert_eq!(
                slot.guard_base().checked_add(guard),
                Some(slot.stack_base()),
                "the guard does not sit immediately below the stack in slot {}",
                index
            );
            prop_assert_eq!(
                slot.stack_base().checked_add(slot.stack_len()),
                Some(slot.stack_top()),
                "the stack top is not its base plus its length in slot {}",
                index
            );
        }
    }
}

#[test]
fn the_half_open_property_reaches_every_boundary_class() {
    const SAMPLES: usize = 4096;
    let mut runner = TestRunner::new(Config {
        cases: SAMPLES as u32,
        ..Config::default()
    });
    let mut crossing = 0usize;
    let mut adjacent = 0usize;
    let mut one_past = 0usize;
    let mut below = 0usize;
    let mut above = 0usize;

    for _ in 0..SAMPLES {
        let (_, _, _, _, contact, side) = touching_pair()
            .new_tree(&mut runner)
            .expect("a boundary pair")
            .current();
        match contact {
            Contact::Crosses => crossing += 1,
            Contact::Adjacent => adjacent += 1,
            Contact::OnePast => one_past += 1,
        }
        match side {
            Side::Below => below += 1,
            Side::Above => above += 1,
        }
    }

    println!(
        "stack-layout boundaries: {crossing} crossing, {adjacent} adjacent, \
         {one_past} one-past; {below} below and {above} above"
    );
    for (name, reached) in [
        ("crossing", crossing),
        ("adjacent", adjacent),
        ("one-past", one_past),
    ] {
        assert!(
            reached > SAMPLES / 5,
            "only {reached} of {SAMPLES} pairs reached {name}"
        );
    }
    assert!(below > SAMPLES / 3, "only {below} pairs reached below");
    assert!(above > SAMPLES / 3, "only {above} pairs reached above");
}

/// How often the constructor accepts, and how often it yields more than one
/// slot to compare — D-242's obligation on a generated-input check.
///
/// `no_two_slots_touch` has nothing to say about a one-slot layout, so the
/// multi-slot rate is the number that decides whether it asserts anything.
#[test]
fn the_layout_properties_reach_multi_slot_windows() {
    const SAMPLES: usize = 4096;
    let mut runner = TestRunner::new(Config {
        cases: SAMPLES as u32,
        ..Config::default()
    });
    let mut accepted = 0usize;
    let mut multi_slot = 0usize;

    for _ in 0..SAMPLES {
        let (base, stack, guard, slots) =
            layout().new_tree(&mut runner).expect("a layout").current();
        if let Some(layout) = StackLayout::new(base, stack, guard, slots) {
            accepted += 1;
            if layout.slots() > 1 {
                multi_slot += 1;
            }
        }
    }

    println!("stack-layout: {accepted}/{SAMPLES} accepted, {multi_slot} with more than one slot");
    assert!(
        multi_slot > SAMPLES / 4,
        "only {multi_slot} of {SAMPLES} layouts had two slots to compare; \
         no_two_slots_touch is near-vacuous"
    );
}
