//! The 64-bit task-state segment.
//!
//! Hardware task switching is gone in long mode; what survives is a table of
//! stack pointers. `RSP0` is the stack the CPU switches to when an interrupt
//! arrives from ring 3, and the seven interrupt stack table entries are stacks a
//! gate can demand unconditionally — which is the only way to handle a fault
//! whose cause is that the current stack is unusable.

use core::fmt;

/// Privilege levels with a stack pointer in the task-state segment.
const PRIVILEGE_STACKS: usize = 3;

/// Entries in the interrupt stack table, counted from 1 by the gate that
/// selects one.
pub const INTERRUPT_STACKS: usize = 7;

/// The layout the CPU reads. Every reserved field must stay zero.
///
/// Packed to four bytes because the architecture places the first stack pointer
/// at offset 4, which natural alignment would not do.
#[derive(Clone, Copy)]
#[repr(C, packed(4))]
pub struct TaskState {
    reserved_start: u32,
    privilege_stacks: [u64; PRIVILEGE_STACKS],
    reserved_middle: u64,
    interrupt_stacks: [u64; INTERRUPT_STACKS],
    reserved_end: u64,
    reserved_tail: u16,
    iomap_base: u16,
}

impl TaskState {
    /// An empty task-state segment with no I/O permission bitmap.
    ///
    /// `iomap_base` is set to the size of the segment, which puts the bitmap
    /// outside the limit — how x86 spells "ring 3 may touch no port at all".
    pub const fn new() -> Self {
        Self {
            reserved_start: 0,
            privilege_stacks: [0; PRIVILEGE_STACKS],
            reserved_middle: 0,
            interrupt_stacks: [0; INTERRUPT_STACKS],
            reserved_end: 0,
            reserved_tail: 0,
            iomap_base: size_of::<Self>() as u16,
        }
    }

    /// Point interrupt stack table entry `index` at `top`.
    ///
    /// The value is the address the stack grows *down* from, so it is one past
    /// the end of the memory reserved for it.
    ///
    /// # Panics
    ///
    /// Panics unless `index` is between 1 and 7.
    pub const fn set_interrupt_stack(&mut self, index: usize, top: u64) {
        assert!(
            index.wrapping_sub(1) < INTERRUPT_STACKS,
            "the interrupt stack table is indexed from 1 and has seven entries"
        );
        self.interrupt_stacks[index - 1] = top;
    }

    /// The stack interrupt stack table entry `index` names, or zero if unset.
    ///
    /// # Panics
    ///
    /// Panics unless `index` is between 1 and 7.
    pub const fn interrupt_stack(&self, index: usize) -> u64 {
        assert!(
            index.wrapping_sub(1) < INTERRUPT_STACKS,
            "the interrupt stack table is indexed from 1 and has seven entries"
        );
        self.interrupt_stacks[index - 1]
    }

    /// Offset at which the I/O permission bitmap would begin.
    pub const fn iomap_base(&self) -> u16 {
        self.iomap_base
    }
}

impl Default for TaskState {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for TaskState {
    /// Hand-written because fields of a packed struct cannot be borrowed, which
    /// is what a derived implementation does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let interrupt_stacks = self.interrupt_stacks;
        let iomap_base = self.iomap_base;
        f.debug_struct("TaskState")
            .field("interrupt_stacks", &interrupt_stacks)
            .field("iomap_base", &iomap_base)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use core::mem::offset_of;

    use super::*;

    /// Every offset here is dictated by the architecture; nothing but this test
    /// stands between a mistyped reserved field and a stack pointer the CPU
    /// reads from the wrong place.
    #[test]
    fn the_layout_matches_the_architecture() {
        assert_eq!(size_of::<TaskState>(), 104);
        assert_eq!(offset_of!(TaskState, privilege_stacks), 4);
        assert_eq!(offset_of!(TaskState, interrupt_stacks), 36);
        assert_eq!(offset_of!(TaskState, iomap_base), 102);
    }

    #[test]
    fn an_empty_segment_puts_its_bitmap_past_its_own_limit() {
        assert_eq!(TaskState::new().iomap_base(), 104);
    }

    #[test]
    fn interrupt_stacks_are_indexed_from_one() {
        let mut task_state = TaskState::new();
        task_state.set_interrupt_stack(1, 0xDEAD_0000);
        task_state.set_interrupt_stack(7, 0xBEEF_0000);

        assert_eq!(task_state.interrupt_stack(1), 0xDEAD_0000);
        assert_eq!(task_state.interrupt_stack(7), 0xBEEF_0000);
        assert_eq!(task_state.interrupt_stack(2), 0);

        let stacks = task_state.interrupt_stacks;
        assert_eq!(stacks[0], 0xDEAD_0000);
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
}
