//! Interrupt descriptor table gates: a handler, its code selector and an optional IST stack.

use crate::Selector;

/// Gates in an interrupt descriptor table, fixed by the architecture.
pub const TABLE_LEN: usize = 256;

pub const TABLE_BYTES: usize = TABLE_LEN * size_of::<Gate>();

/// Value the table's limit field takes: one less than its size.
pub const TABLE_LIMIT: u16 = (TABLE_BYTES - 1) as u16;

/// Present, ring 0, 64-bit interrupt gate; a handler wanting interrupts on enables them itself.
const INTERRUPT_GATE: u8 = 0x8E;

/// Highest interrupt stack table index a gate can select.
const MAX_INTERRUPT_STACK: u8 = 7;

/// One interrupt descriptor table entry.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[repr(C)]
pub struct Gate {
    offset_low: u16,
    selector: u16,
    interrupt_stack: u8,
    attributes: u8,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

impl Gate {
    /// A gate with the present bit clear, so taking its vector raises a general protection fault.
    pub const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        interrupt_stack: 0,
        attributes: 0,
        offset_middle: 0,
        offset_high: 0,
        reserved: 0,
    };

    /// An interrupt gate entering `handler` through `selector` on IST entry `stack`, 0 for none.
    pub const fn interrupt(handler: u64, selector: Selector, stack: u8) -> Self {
        assert!(
            stack <= MAX_INTERRUPT_STACK,
            "the interrupt stack table has seven entries"
        );

        Self {
            offset_low: handler as u16,
            selector: selector.bits(),
            interrupt_stack: stack,
            attributes: INTERRUPT_GATE,
            offset_middle: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }

    /// The same interrupt gate at DPL 3, opened only for the kernel's entry.
    pub const fn callable_from_user(handler: u64, selector: Selector, stack: u8) -> Self {
        let mut gate = Self::interrupt(handler, selector, stack);
        gate.attributes |= 0b11 << 5;
        gate
    }

    /// The privilege level that may reach this gate with `int`.
    pub const fn privilege(self) -> u8 {
        (self.attributes >> 5) & 0b11
    }
}

#[cfg(test)]
mod tests;
