//! The interrupt command register.
//!
//! Writing it sends an interrupt to a processor. The only destination this
//! kernel names is itself — the mechanism it uses to prove its own interrupt
//! path end to end — and a command addressed by shorthand carries no
//! destination, so the upper half of the register stays zero.

use crate::FIRST_VECTOR;

/// Bits 8–10: delivery mode. Zero delivers the vector as written, which is
/// everything short of the startup and reset commands that bring up another
/// processor.
const DELIVERY_FIXED: u32 = 0;

/// Bit 14: asserted rather than de-asserted. The de-assert form exists only for
/// the level-triggered INIT of very old multiprocessor firmware, so every
/// command this kernel builds sets it.
const ASSERT: u32 = 1 << 14;

/// Bits 18–19: destination shorthand. One means the processor that sent it.
const SHORTHAND_SELF: u32 = 1 << 18;

/// Bits 0–7: the vector.
const VECTOR: u32 = 0xFF;

/// The contents of [`crate::register::COMMAND`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command(u32);

impl Command {
    /// Deliver `vector` to the processor that sends it.
    ///
    /// # Panics
    ///
    /// Panics if `vector` is below [`FIRST_VECTOR`], which the APIC would
    /// record as an illegal-vector error rather than deliver.
    pub const fn self_interrupt(vector: u8) -> Self {
        assert!(
            vector >= FIRST_VECTOR,
            "the local APIC will not deliver a vector below 16"
        );
        Self(vector as u32 | DELIVERY_FIXED | ASSERT | SHORTHAND_SELF)
    }

    /// Interpret what the register returned.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The value to write.
    ///
    /// The register is 64 bits wide and its upper half is the destination,
    /// which a shorthand makes irrelevant — so a caller widens this and writes
    /// zeros above it.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The vector this command delivers.
    pub const fn vector(self) -> u8 {
        (self.0 & VECTOR) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_self_interrupt_names_its_vector_and_no_destination() {
        let command = Command::self_interrupt(0x40);

        assert_eq!(command.vector(), 0x40);
        assert_eq!(command.bits(), 0x0004_4040);
        assert_eq!(command.bits() & 0x0000_0700, 0, "delivery mode is fixed");
        assert_ne!(command.bits() & ASSERT, 0, "level is asserted");
        assert_eq!(
            command.bits() & 0x000C_0000,
            SHORTHAND_SELF,
            "shorthand is self"
        );
    }

    /// Bit 12 is the delivery-status bit of the memory-mapped encoding, which
    /// x2APIC removed — a write there is synchronous. Building a command that
    /// sets it would be writing a reserved bit, which faults.
    #[test]
    fn a_command_leaves_the_removed_delivery_status_bit_clear() {
        assert_eq!(Command::self_interrupt(0x40).bits() & 1 << 12, 0);
    }

    #[test]
    #[should_panic(expected = "below 16")]
    fn a_command_refuses_a_vector_the_apic_would_reject() {
        let _ = Command::self_interrupt(FIRST_VECTOR - 1);
    }
}
