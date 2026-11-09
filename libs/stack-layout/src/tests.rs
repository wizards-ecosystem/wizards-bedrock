use super::*;

fn layout() -> StackLayout {
    StackLayout::new(0xffff_fe00_0000_0000, 16 * 1024, 4096, 16).expect("a valid layout")
}

/// The addresses the layout actually produces, transcribed.
///
/// Absolute addresses are independent of `stride()`, unlike a relational
/// `base + stride·i <= base + stride·i` tautology that also admits
/// overlapping slots (D-105).
#[test]
fn slots_land_where_the_arithmetic_says() {
    let layout = layout();
    // Stride is 4096 guard + 16384 stack = 20480 = 0x5000.
    assert_eq!(layout.stride(), 0x5000);
    assert_eq!(layout.base(), 0xffff_fe00_0000_0000);

    let first = layout.slot(0).expect("in range");
    assert_eq!(first.guard_base(), 0xffff_fe00_0000_0000);
    assert_eq!(first.stack_base(), 0xffff_fe00_0000_1000);
    assert_eq!(first.stack_top(), 0xffff_fe00_0000_5000);

    let second = layout.slot(1).expect("in range");
    assert_eq!(second.guard_base(), 0xffff_fe00_0000_5000);
    assert_eq!(second.stack_base(), 0xffff_fe00_0000_6000);
    assert_eq!(second.stack_top(), 0xffff_fe00_0000_a000);

    let last = layout.slot(15).expect("in range");
    assert_eq!(last.guard_base(), 0xffff_fe00_0004_b000);
    assert_eq!(last.stack_top(), 0xffff_fe00_0005_0000);
}

/// No stack overlaps a neighbour's, and every gap between them is a guard.
///
/// Stated against the *stacks* rather than against `stride`, so a stride
/// that swallowed the guard would be visible here: a guard page is a page
/// nothing maps, and it is only a guard if it is not another slot's stack.
#[test]
fn no_two_stacks_touch_and_a_guard_lies_between_them() {
    let layout = layout();
    for index in 1..layout.slots() {
        let below = layout.slot(index - 1).expect("in range");
        let slot = layout.slot(index).expect("in range");
        assert_eq!(
            slot.stack_base() - below.stack_top(),
            4096,
            "slot {index} is not one guard page above its neighbour"
        );
        assert!(
            below.stack_top() <= slot.guard_base(),
            "slot {index} begins before its neighbour ends"
        );
    }
}

#[test]
fn every_stack_has_its_guard_immediately_below() {
    let layout = layout();
    for index in 0..layout.slots() {
        let slot = layout.slot(index).expect("in range");
        assert_eq!(slot.guard_base() + 4096, slot.stack_base());
        assert_eq!(slot.stack_top() - slot.stack_base(), 16 * 1024);
        assert_eq!(slot.stack_len(), 16 * 1024);
    }
}

#[test]
fn a_live_pointer_splits_stack_capacity_exactly() {
    let slot = layout().slot(0).expect("in range");
    let at_top = slot.usage_at(slot.stack_top()).expect("top is valid");
    assert_eq!(at_top.capacity(), 16 * 1024);
    assert_eq!(at_top.used(), 0);
    assert_eq!(at_top.headroom(), 16 * 1024);

    let interior = slot
        .usage_at(slot.stack_top() - 0x1234)
        .expect("interior is valid");
    assert_eq!(interior.used(), 0x1234);
    assert_eq!(interior.headroom(), 16 * 1024 - 0x1234);

    let at_base = slot.usage_at(slot.stack_base()).expect("base is valid");
    assert_eq!(at_base.used(), 16 * 1024);
    assert_eq!(at_base.headroom(), 0);
}

#[test]
fn a_pointer_outside_the_stack_has_no_usage() {
    let slot = layout().slot(0).expect("in range");
    assert_eq!(slot.usage_at(slot.stack_base() - 1), None);
    assert_eq!(slot.usage_at(slot.stack_top() + 1), None);
}

/// `end()` and `last.stack_top()` are the same expression written twice,
/// so the equality between them is `X == X`; the window's length against a
/// transcribed number is what says the layout is the size it claims
/// (D-105).
#[test]
fn the_window_ends_where_the_last_slot_does() {
    let layout = layout();
    let last = layout.slot(layout.slots() - 1).expect("in range");
    assert_eq!(layout.end(), 0xffff_fe00_0005_0000);
    assert_eq!(layout.end() - layout.base(), 16 * 0x5000);
    assert_eq!(layout.end(), last.stack_top());
    assert_eq!(layout.slot(layout.slots()), None);
}

#[test]
fn misalignment_is_refused() {
    assert_eq!(StackLayout::new(0x1000 + 8, 0x4000, 0x1000, 4), None);
    assert_eq!(StackLayout::new(0x1000, 0x4000 + 8, 0x1000, 4), None);
    assert_eq!(StackLayout::new(0x1000, 0x4000, 0x1000 + 8, 4), None);
}

#[test]
fn nothing_may_be_zero() {
    assert_eq!(StackLayout::new(0x1000, 0, 0x1000, 4), None);
    assert_eq!(StackLayout::new(0x1000, 0x4000, 0, 4), None);
    assert_eq!(StackLayout::new(0x1000, 0x4000, 0x1000, 0), None);
}

#[test]
fn a_window_that_would_wrap_is_refused() {
    let top = !(PAGE_SIZE - 1);
    assert_eq!(StackLayout::new(top, 0x4000, 0x1000, 1), None);
    assert_eq!(StackLayout::new(0, !(PAGE_SIZE - 1), 0x1000, 2), None);
}

/// A page is four kilobytes, and a guard smaller than one is refused.
///
/// Anchor `PAGE_SIZE` and include a 2048-byte rejection because positive
/// values aligned to 4096 cannot distinguish a halved modulus. `wost` maps
/// in 4096-byte units (D-124).
#[test]
fn a_guard_smaller_than_a_page_is_refused() {
    assert_eq!(PAGE_SIZE, 4096);
    assert!(
        StackLayout::new(0xffff_fe00_0000_0000, 4096, 2048, 2).is_none(),
        "a half-page guard was accepted"
    );
    assert!(
        StackLayout::new(0xffff_fe00_0000_0000, 2048, 4096, 2).is_none(),
        "a half-page stack was accepted"
    );
}
