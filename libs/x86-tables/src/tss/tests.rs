use core::mem::offset_of;

use super::*;

/// Nothing else stands between a mistyped reserved field and a misplaced stack pointer.
#[test]
fn the_layout_matches_the_architecture() {
    assert_eq!(size_of::<TaskState>(), 104);
    assert_eq!(offset_of!(TaskState, privilege_stacks), 4);
    assert_eq!(offset_of!(TaskState, interrupt_stacks), 36);
    assert_eq!(offset_of!(TaskState, iomap_base), 102);
}

#[test]
fn an_empty_segment_puts_its_bitmap_past_its_own_limit() {
    let iomap_base = TaskState::new().iomap_base;
    assert_eq!(iomap_base, 104);
}

#[test]
fn debug_reports_the_mutable_state() {
    let mut task_state = TaskState::new();
    task_state.set_interrupt_stack(2, 0x1234);

    assert_eq!(
        format!("{task_state:?}"),
        "TaskState { interrupt_stacks: [0, 4660, 0, 0, 0, 0, 0], iomap_base: 104, .. }"
    );
}

#[test]
fn interrupt_stacks_are_indexed_from_one() {
    let mut task_state = TaskState::new();
    task_state.set_interrupt_stack(1, 0xDEAD_0000);
    task_state.set_interrupt_stack(7, 0xBEEF_0000);

    let stacks = task_state.interrupt_stacks;
    assert_eq!(stacks, [0xDEAD_0000, 0, 0, 0, 0, 0, 0xBEEF_0000]);
}

#[test]
#[should_panic(expected = "indexed from 1")]
fn interrupt_stack_zero_does_not_exist() {
    TaskState::new().set_interrupt_stack(0, 0x1000);
}

#[test]
#[should_panic(expected = "indexed from 1")]
fn there_is_no_eighth_interrupt_stack() {
    TaskState::new().set_interrupt_stack(8, 0x1000);
}

#[test]
fn interrupt_stacks_sit_where_the_manual_says() {
    assert_eq!(TaskState::interrupt_stack_offset(1), 0x24);
    assert_eq!(TaskState::interrupt_stack_offset(7), 0x24 + 6 * 8);

    assert_eq!(
        core::mem::offset_of!(TaskState, interrupt_stacks),
        0x24,
        "the compiler places the interrupt stacks somewhere else"
    );
    for index in 1..=INTERRUPT_STACKS {
        assert_eq!(
            TaskState::interrupt_stack_offset(index),
            core::mem::offset_of!(TaskState, interrupt_stacks) + (index - 1) * 8,
            "entry {index} is not where the offset says"
        );
    }
}

#[test]
fn the_interrupt_stack_offset_is_four_modulo_eight() {
    assert_eq!(TaskState::interrupt_stack_offset(1) % 8, 4);
}

#[test]
fn the_privilege_stacks_sit_where_the_manual_says() {
    assert_eq!(TaskState::privilege_stack_offset(0), 4);
    assert_eq!(TaskState::privilege_stack_offset(1), 12);
    assert_eq!(TaskState::privilege_stack_offset(2), 20);
    assert_eq!(TaskState::privilege_stack_offset(0) % 8, 4);

    assert_eq!(
        core::mem::offset_of!(TaskState, privilege_stacks),
        4,
        "the compiler places the privilege stacks somewhere else"
    );
    for level in 0..PRIVILEGE_STACKS {
        assert_eq!(
            TaskState::privilege_stack_offset(level),
            core::mem::offset_of!(TaskState, privilege_stacks) + level * 8
        );
    }
}

/// A privilege stack round-trips, and the three are independent.
#[test]
fn privilege_stacks_do_not_share_a_slot() {
    let mut task_state = TaskState::new();
    task_state.set_privilege_stack(0, 0x1111_0000);
    task_state.set_privilege_stack(2, 0x3333_0000);
    let privilege_stacks = task_state.privilege_stacks;
    assert_eq!(privilege_stacks, [0x1111_0000, 0, 0x3333_0000]);

    let interrupt_stacks = task_state.interrupt_stacks;
    assert_eq!(interrupt_stacks[0], 0);
}

/// There is no stack for ring 3, because the CPU never switches to it.
#[test]
#[should_panic(expected = "three privilege stacks")]
fn ring_three_has_no_privilege_stack() {
    let _ = TaskState::privilege_stack_offset(3);
}
