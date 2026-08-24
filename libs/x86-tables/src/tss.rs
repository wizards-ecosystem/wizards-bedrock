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
    /// Byte offset of interrupt-stack entry `index` inside the segment.
    ///
    /// Exists so a caller can rewrite one entry rather than the whole
    /// structure. That distinction is not cosmetic: the entry is eight bytes
    /// and naturally aligned, so a targeted store is atomic against a double
    /// fault arriving mid-write, while a whole-segment `write_volatile` is a
    /// memcpy that can be observed torn (D-94).
    ///
    /// # Panics
    ///
    /// Outside 1..=[`INTERRUPT_STACKS`], as [`set_interrupt_stack`] does — the
    /// table is indexed from one because zero means "no stack switch".
    ///
    /// [`set_interrupt_stack`]: Self::set_interrupt_stack
    pub const fn interrupt_stack_offset(index: usize) -> usize {
        assert!(
            index.wrapping_sub(1) < INTERRUPT_STACKS,
            "the interrupt stack table is indexed from 1 and has seven entries"
        );
        // reserved_start, then the privilege stacks, then a reserved word.
        size_of::<u32>()
            + PRIVILEGE_STACKS * size_of::<u64>()
            + size_of::<u64>()
            + (index - 1) * size_of::<u64>()
    }

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

    /// The interrupt-stack entries sit where the architecture puts them.
    ///
    /// Intel SDM Vol. 3A figure 8-11: `IST1` begins at offset 0x24 and each
    /// entry is eight bytes. The transcribed number is checked against the
    /// **compiler's** view of the layout via `offset_of!`, so a reordered or
    /// re-`repr`'d field disagrees with the manual instead of agreeing with
    /// whatever the struct happens to be.
    ///
    /// An earlier version wrote the value into a zeroed array at the offset it
    /// was testing and read it back from the same place — an identity that
    /// would have passed under any layout at all (D-95).
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

    /// The offset a targeted store uses is four-mod-eight.
    ///
    /// Not a curiosity: it is why `wost` pads its task-state segment. An
    /// eight-byte store to `IST1` in a segment on an eight-byte boundary would
    /// be misaligned, and the padding is what makes it sound (D-95). If the
    /// architecture's offset ever stopped being four-mod-eight the padding
    /// would be wrong, so the fact is pinned here.
    #[test]
    fn the_interrupt_stack_offset_is_four_modulo_eight() {
        assert_eq!(TaskState::interrupt_stack_offset(1) % 8, 4);
    }
}
