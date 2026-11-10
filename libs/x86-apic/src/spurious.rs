//! The spurious-interrupt vector register, whose enable bit turns the APIC on once all else is set.

use crate::vector::InterruptVector;

/// Bit 8: the APIC delivers nothing at all while this is clear.
const ENABLE: u32 = 1 << 8;

/// Vectors whose low four bits the earliest APICs hardwire to one.
const HARDWIRED_LOW_BITS: u8 = 0x0F;

/// The contents of [`crate::register::SPURIOUS_INTERRUPT_VECTOR`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Spurious(u32);

impl Spurious {
    /// An enabled APIC delivering `vector`; panics unless its low nibble is set, as P6 wires it.
    pub const fn enabled(vector: InterruptVector) -> Self {
        let vector = vector.get();
        assert!(
            vector & HARDWIRED_LOW_BITS == HARDWIRED_LOW_BITS,
            "a spurious vector must have its low four bits set"
        );
        Self(vector as u32 | ENABLE)
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether the APIC is software-enabled.
    pub const fn is_enabled(self) -> bool {
        self.0 & ENABLE != 0
    }
}

#[cfg(test)]
mod tests;
