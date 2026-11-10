use super::*;

fn layout() -> StackLayout {
    StackLayout::new(0xffff_fe00_0000_0000, 16 * 1024, 4096, 16).expect("a valid layout")
}

/// Transcribed absolute addresses, independent of `stride()`, so overlapping slots fail.
#[test]
fn slots_land_where_the_arithmetic_says() {
    let layout = layout();
    // 4096 guard plus 16384 stack.
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

/// Stated against the stacks, so a stride that swallowed a guard into a neighbour's stack fails.
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

/// The transcribed window length is what checks the size; `end()` equals `stack_top()` by form.
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
fn a_misaligned_base_stack_or_guard_is_refused() {
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

/// A 2048-byte case, since lengths aligned to 4096 cannot tell a halved modulus apart.
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
