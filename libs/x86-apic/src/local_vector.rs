//! Local vector table entries.
//!
//! One 32-bit register for each interrupt source the APIC owns itself — its
//! timer, its two local pins, its error line — all sharing a layout. An entry
//! says which vector the source delivers on, or that it delivers nothing.

use crate::FIRST_VECTOR;

/// Bits 8–10: delivery mode. Zero is a plain vector delivered to this
/// processor, which is the only mode this kernel programs. The others exist so
/// firmware can wire a pin to the non-maskable interrupt or to the legacy
/// controller, and undoing that is exactly what masking is for.
const DELIVERY_FIXED: u32 = 0;

/// Bit 16: the source is disconnected while this is set.
const MASK: u32 = 1 << 16;

/// Bits 0–7: the vector.
const VECTOR: u32 = 0xFF;

/// One local vector table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry(u32);

impl Entry {
    /// An entry that delivers nothing.
    ///
    /// This is also the value the architecture resets every entry to, so
    /// writing it puts a pin back the way the processor started rather than
    /// merely quiet.
    pub const MASKED: Self = Self(MASK);

    /// Deliver `vector` to this processor whenever the source fires.
    ///
    /// # Panics
    ///
    /// Panics if `vector` is below [`FIRST_VECTOR`]. The APIC would refuse it
    /// too, by recording an illegal-vector error and delivering nothing, which
    /// is a great deal harder to notice than a panic during boot.
    pub const fn fixed(vector: u8) -> Self {
        assert!(
            vector >= FIRST_VECTOR,
            "the local APIC will not deliver a vector below 16"
        );
        Self(vector as u32 | DELIVERY_FIXED)
    }

    /// Interpret what the register returned.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The value to write.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether the source is disconnected.
    pub const fn is_masked(self) -> bool {
        self.0 & MASK != 0
    }

    /// The vector this entry delivers on.
    pub const fn vector(self) -> u8 {
        (self.0 & VECTOR) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SDM Vol. 3A §11.5.1: every local vector table entry resets to 0x00010000.
    #[test]
    fn a_masked_entry_is_the_value_the_processor_resets_to() {
        assert_eq!(Entry::MASKED.bits(), 0x0001_0000);
        assert!(Entry::MASKED.is_masked());
    }

    #[test]
    fn a_fixed_entry_delivers_its_vector_and_is_not_masked() {
        let entry = Entry::fixed(0xFE);

        assert_eq!(entry.vector(), 0xFE);
        assert!(!entry.is_masked());
        assert_eq!(entry.bits(), 0xFE);
    }

    /// Firmware conventionally leaves LINT0 delivering the legacy controller's
    /// output as an external interrupt — delivery mode 7, unmasked. Masking it
    /// has to clear the mode as well as set the mask bit, or the pin stays
    /// wired to a controller the kernel has stopped listening to.
    #[test]
    fn masking_a_pin_firmware_wired_to_the_legacy_controller_clears_its_mode() {
        let firmware = Entry::from_bits(0x0000_0700);

        assert!(!firmware.is_masked());
        assert_eq!(Entry::MASKED.bits() & 0x0000_0700, 0);
    }

    #[test]
    fn an_entry_read_back_round_trips() {
        let written = Entry::fixed(0x40);

        assert_eq!(Entry::from_bits(written.bits()), written);
    }

    #[test]
    #[should_panic(expected = "below 16")]
    fn an_entry_refuses_a_vector_the_apic_would_reject() {
        let _ = Entry::fixed(FIRST_VECTOR - 1);
    }
}
