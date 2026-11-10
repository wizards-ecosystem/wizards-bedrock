//! The 64-bit task-state segment: `RSP0` for entries from ring 3 and seven IST stacks.

use core::fmt;

/// Privilege levels with a stack pointer in the task-state segment.
const PRIVILEGE_STACKS: usize = 3;

/// Entries in the interrupt stack table, counted from 1 by the gate that selects one.
pub const INTERRUPT_STACKS: usize = 7;

/// The layout the CPU reads, packed to four since `RSP0` sits at offset 4; reserved fields stay 0.
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
    /// The stack for an interrupt raising privilege to ring `level`; panics above ring 2.
    pub const fn set_privilege_stack(&mut self, level: usize, top: u64) {
        assert!(level < PRIVILEGE_STACKS, "there are three privilege stacks");
        self.privilege_stacks[level] = top;
    }

    /// Byte offset of privilege stack `level`, for a targeted store a trap cannot see torn.
    ///
    /// A whole-segment `write_volatile` is a memcpy a trap can see torn.
    pub const fn privilege_stack_offset(level: usize) -> usize {
        assert!(level < PRIVILEGE_STACKS, "there are three privilege stacks");
        size_of::<u32>() + level * size_of::<u64>()
    }

    /// Byte offset of IST entry `index`, from 1, for a store a double fault cannot see torn.
    ///
    /// A whole-segment `write_volatile` is a memcpy a double fault can see torn.
    pub const fn interrupt_stack_offset(index: usize) -> usize {
        assert!(
            index.wrapping_sub(1) < INTERRUPT_STACKS,
            "the interrupt stack table is indexed from 1 and has seven entries"
        );
        size_of::<u32>()
            + PRIVILEGE_STACKS * size_of::<u64>()
            + size_of::<u64>()
            + (index - 1) * size_of::<u64>()
    }

    /// An empty segment whose `iomap_base` is its size, so ring 3 may touch no port.
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

    /// Point IST entry `index`, 1 to 7, at `top`, one past the end of its stack.
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

// By hand, because a derived Debug borrows fields a packed struct cannot lend.
impl fmt::Debug for TaskState {
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
