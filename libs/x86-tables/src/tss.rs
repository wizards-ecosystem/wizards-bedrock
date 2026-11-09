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
    /// The stack the CPU switches to when an interrupt raises privilege to
    /// ring `level`.
    ///
    /// # Panics
    ///
    /// Above ring 2. There is no `RSP3`: the CPU never switches *to* ring 3.
    pub const fn set_privilege_stack(&mut self, level: usize, top: u64) {
        assert!(level < PRIVILEGE_STACKS, "there are three privilege stacks");
        self.privilege_stacks[level] = top;
    }

    /// Byte offset of privilege-stack entry `level` inside the segment.
    ///
    /// Exists for the same reason the interrupt-stack one does: a targeted
    /// eight-byte store rather than a rewrite of the whole structure, so a
    /// trap arriving mid-update reads one stack or the other (D-95).
    ///
    /// # Panics
    ///
    /// As [`set_privilege_stack`](Self::set_privilege_stack).
    pub const fn privilege_stack_offset(level: usize) -> usize {
        assert!(level < PRIVILEGE_STACKS, "there are three privilege stacks");
        size_of::<u32>() + level * size_of::<u64>()
    }

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
mod tests;
