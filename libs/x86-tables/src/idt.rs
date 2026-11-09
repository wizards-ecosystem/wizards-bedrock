//! Interrupt descriptor table gates.
//!
//! A 64-bit gate is sixteen bytes that name a handler, the code selector to
//! enter it with, and optionally one of the seven interrupt stacks in the
//! task-state segment.

use crate::Selector;

/// Gates in an interrupt descriptor table. The architecture fixes this.
pub const TABLE_LEN: usize = 256;

/// Size of a full table in bytes.
pub const TABLE_BYTES: usize = TABLE_LEN * size_of::<Gate>();

/// Value the table's limit field takes: one less than its size.
pub const TABLE_LIMIT: u16 = (TABLE_BYTES - 1) as u16;

/// Present, ring 0, 64-bit interrupt gate.
///
/// An interrupt gate clears the interrupt flag on entry; the trap gate that
/// does not is deliberately unavailable, because a handler that wants to run
/// with interrupts on should say so itself.
const INTERRUPT_GATE: u8 = 0x8E;

/// Highest interrupt stack table index a gate can select.
const MAX_INTERRUPT_STACK: u8 = 7;

/// One interrupt descriptor table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    /// A gate with the present bit clear: taking this vector raises a general
    /// protection fault rather than jumping somewhere arbitrary.
    pub const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        interrupt_stack: 0,
        attributes: 0,
        offset_middle: 0,
        offset_high: 0,
        reserved: 0,
    };

    /// An interrupt gate entering `handler` through `selector`.
    ///
    /// `stack` selects an entry in the task-state segment's interrupt stack
    /// table, counted from 1; zero means the handler runs on whichever stack
    /// was in use when the interrupt arrived.
    ///
    /// # Panics
    ///
    /// Panics if `stack` exceeds 7.
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

    /// An interrupt gate ring 3 may invoke with `int`.
    ///
    /// The same gate in every respect but its descriptor privilege level.
    /// A gate at DPL 0 raises a general-protection fault when user code names
    /// it, which is what every vector in this table should do — a device's
    /// line is not a service. Exactly one gate is opened this way, and it is
    /// the kernel's entry (D-96).
    ///
    /// An interrupt gate clears IF before kernel stack discipline exists.
    ///
    /// # Panics
    ///
    /// As [`interrupt`](Self::interrupt).
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
