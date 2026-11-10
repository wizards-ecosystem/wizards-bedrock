//! The spurious-interrupt vector register.
//!
//! Two unrelated things share one register: the vector the APIC delivers when
//! it has committed to delivering something and then found it had nothing to
//! say, and the bit that software-enables the controller. Writing the second is
//! what turns the APIC on, so it is written once everything else is configured.

use crate::vector::InterruptVector;

/// Bit 8: the APIC delivers nothing at all while this is clear.
const ENABLE: u32 = 1 << 8;

/// Vectors whose low four bits the earliest APICs hardwire to one.
const HARDWIRED_LOW_BITS: u8 = 0x0F;

/// The contents of [`crate::register::SPURIOUS_INTERRUPT_VECTOR`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Spurious(u32);

impl Spurious {
    /// An enabled APIC that delivers `vector` when it has nothing to say.
    ///
    /// # Panics
    ///
    /// Panics unless the low four bits are all set. P6 and Pentium APICs
    /// hardwire those bits, so any other
    /// vector would silently become a different one there — and a spurious
    /// interrupt arriving on a vector nobody is expecting is the hardest kind
    /// of fault to read.
    pub const fn enabled(vector: InterruptVector) -> Self {
        let vector = vector.get();
        assert!(
            vector & HARDWIRED_LOW_BITS == HARDWIRED_LOW_BITS,
            "a spurious vector must have its low four bits set"
        );
        Self(vector as u32 | ENABLE)
    }

    /// Interpret what the register returned.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The value to write.
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
