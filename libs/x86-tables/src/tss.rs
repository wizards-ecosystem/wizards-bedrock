use core::fmt;

const PRIVILEGE_STACKS: usize = 3;

const INTERRUPT_STACKS: usize = 7;

// Packed to four because `RSP0` sits at offset 4.
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
    // For a targeted store: a whole-segment `write_volatile` is a memcpy a trap can see torn.
    pub const fn privilege_stack_offset(level: usize) -> usize {
        assert!(level < PRIVILEGE_STACKS, "there are three privilege stacks");
        size_of::<u32>() + level * size_of::<u64>()
    }

    // For a targeted store: a whole-segment `write_volatile` is a memcpy a fault can see torn.
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

    // `iomap_base` is the segment's size, so ring 3 may touch no port.
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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let interrupt_stacks = self.interrupt_stacks;
        let iomap_base = self.iomap_base;
        formatter
            .debug_struct("TaskState")
            .field("interrupt_stacks", &interrupt_stacks)
            .field("iomap_base", &iomap_base)
            .finish_non_exhaustive()
    }
}
