//! Property tests that no two slots of any accepted layout touch, and over the slot answers.

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestRunner};
use stack_layout::{PAGE_SIZE, Slot, StackLayout, ranges_overlap};

/// Bytes in the word a paint is aligned to.
const WORD: u64 = 8;

/// Page-multiple lengths, the granularity the crate says guards come in.
fn pages(max: u64) -> impl Strategy<Value = u64> {
    (1u64..=max).prop_map(|count| count * PAGE_SIZE)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Contact {
    Crosses,
    Adjacent,
    OnePast,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Placement {
    InGuard,
    Base,
    Interior,
    Top,
    AboveTop,
}

type Placed = ((u64, u64, u64, usize), prop::sample::Index, Placement, u64);

/// A layout, one of its slots, and a pointer placed against that slot.
fn placed() -> impl Strategy<Value = Placed> {
    (
        layout(),
        any::<prop::sample::Index>(),
        prop_oneof![
            Just(Placement::InGuard),
            Just(Placement::Base),
            Just(Placement::Interior),
            Just(Placement::Top),
            Just(Placement::AboveTop),
        ],
        1u64..8 * PAGE_SIZE,
    )
}

/// The slot a placement names and the pointer it puts there.
fn slot_and_pointer(
    ((base, stack, guard, slots), index, placement, offset): Placed,
) -> Option<(Slot, u64)> {
    let layout = StackLayout::new(base, stack, guard, slots)?;
    let slot = layout.slot(index.index(layout.slots()))?;
    let pointer = match placement {
        Placement::InGuard => slot.stack_base() - 1 - (offset - 1) % guard,
        Placement::Base => slot.stack_base(),
        Placement::Interior => slot.stack_base() + 1 + offset % (slot.stack_len() - 1),
        Placement::Top => slot.stack_top(),
        Placement::AboveTop => slot.stack_top() + offset,
    };
    Some((slot, pointer))
}

proptest! {
    #[test]
    fn an_empty_range_touches_nothing(a in any::<u64>(), b in any::<u64>(), len in any::<u64>()) {
        prop_assert!(!ranges_overlap(a, 0, b, len), "an empty left range overlapped");
        prop_assert!(!ranges_overlap(a, len, b, 0), "an empty right range overlapped");
    }

    /// Crossing shares one byte at an exclusive end, while adjacency and one-past share none.
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

    /// A shared guard would let a stack fault into its neighbour's memory.
    #[test]
    fn no_two_slots_touch((base, stack, guard, slots) in layout()) {
        let Some(layout) = StackLayout::new(base, stack, guard, slots) else {
            return Ok(());
        };
        let placed: Vec<_> = (0..layout.slots())
            .filter_map(|index| layout.slot(index))
            .collect();
        prop_assert_eq!(placed.len(), layout.slots(), "a slot in range was absent");

        for (left_index, left) in placed.iter().enumerate() {
            for (right_index, right) in placed.iter().enumerate() {
                if left_index == right_index {
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
                    left_index,
                    right_index
                );
                prop_assert!(
                    !ranges_overlap(
                        left.stack_base(),
                        left.stack_len(),
                        right.guard_base(),
                        guard
                    ),
                    "slot {}'s stack occupies slot {}'s guard",
                    left_index,
                    right_index
                );
            }
        }
    }

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

    #[test]
    fn usage_splits_a_slot_at_every_pointer_inside_it_and_refuses_the_rest(
        placement in placed(),
    ) {
        let Some((slot, pointer)) = slot_and_pointer(placement) else {
            return Ok(());
        };
        let place = placement.2;
        let usage = slot.usage_at(pointer);
        if matches!(place, Placement::InGuard | Placement::AboveTop) {
            prop_assert_eq!(usage, None, "a pointer {:?} had a usage", place);
            return Ok(());
        }
        let Some(usage) = usage else {
            return Err(TestCaseError::fail(format!("a pointer at the {place:?} was refused")));
        };
        prop_assert_eq!(usage.capacity(), slot.stack_len(), "capacity is not the slot's length");
        prop_assert_eq!(
            usage.used().checked_add(usage.headroom()),
            Some(usage.capacity()),
            "used and headroom do not split the capacity"
        );
        match place {
            Placement::Top => prop_assert_eq!(usage.used(), 0, "the top used the stack"),
            Placement::Base => prop_assert_eq!(usage.headroom(), 0, "the base left headroom"),
            _ => prop_assert_eq!(
                slot.usage_at(pointer - 1).map(|deeper| deeper.used()),
                Some(usage.used() + 1),
                "one byte deeper did not use exactly one byte more"
            ),
        }
    }

    #[test]
    fn a_span_below_a_pointer_starts_at_the_base_and_stops_a_clearance_short(
        placement in placed(),
        clearance in 0u64..3 * PAGE_SIZE,
    ) {
        let Some((slot, pointer)) = slot_and_pointer(placement) else {
            return Ok(());
        };
        let room = slot.usage_at(pointer).is_some()
            && slot.stack_base() + WORD + clearance <= pointer;
        let Some(span) = slot.span_below(pointer, clearance) else {
            prop_assert!(!room, "a pointer with a word below its clearance had no span");
            return Ok(());
        };
        prop_assert!(room, "a pointer {:?} without a word of room had a span", placement.2);
        prop_assert_eq!(span.start, slot.stack_base(), "the span does not start at the base");
        prop_assert!(span.start < span.end, "the span is empty");
        prop_assert_eq!(span.end % WORD, 0, "the span does not end on a word");
        prop_assert!(span.end + clearance <= pointer, "the span reaches into the clearance");
        prop_assert!(
            span.end + WORD + clearance > pointer,
            "the span stops more than a word short of the clearance"
        );
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

/// The usage and span properties must reach each placement, a span and a refusal.
#[test]
fn the_usage_and_span_properties_reach_every_placement_and_both_answers() {
    const SAMPLES: usize = 4096;
    let mut runner = TestRunner::new(Config {
        cases: SAMPLES as u32,
        ..Config::default()
    });
    let mut places = [0usize; 5];
    let mut spans = 0usize;
    let mut refusals = 0usize;

    for _ in 0..SAMPLES {
        let (placement, clearance) = (placed(), 0u64..3 * PAGE_SIZE)
            .new_tree(&mut runner)
            .expect("a placed pointer")
            .current();
        places[placement.2 as usize] += 1;
        let (slot, pointer) = slot_and_pointer(placement).expect("an accepted layout");
        match slot.span_below(pointer, clearance) {
            Some(_) => spans += 1,
            None => refusals += 1,
        }
    }

    println!("stack-layout placements: {places:?}; {spans} spans and {refusals} refusals");
    for (place, reached) in places.into_iter().enumerate() {
        assert!(
            reached > SAMPLES / 8,
            "only {reached} of {SAMPLES} pointers reached placement {place}"
        );
    }
    assert!(
        spans > SAMPLES / 8,
        "only {spans} of {SAMPLES} pointers had a span"
    );
    assert!(
        refusals > SAMPLES / 5,
        "only {refusals} of {SAMPLES} pointers were refused"
    );
}

/// `no_two_slots_touch` says nothing about one slot, so the multi-slot rate decides its worth.
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
