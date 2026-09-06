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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
mod tests {
    use super::*;

    fn vector(raw: u8) -> InterruptVector {
        InterruptVector::new(raw).expect("test vector is assignable")
    }

    #[test]
    fn enabling_packs_the_vector_below_the_enable_bit() {
        let spurious = Spurious::enabled(vector(0xFF));

        assert_eq!(spurious.bits(), 0x1FF);
        assert!(spurious.is_enabled());
        assert_eq!(spurious.bits() & 0xFF, 0xFF);
    }

    #[test]
    fn a_register_read_back_round_trips() {
        let written = Spurious::enabled(vector(0x3F));
        let read_back = Spurious::from_bits(written.bits());

        assert_eq!(read_back, written);
        assert_eq!(read_back.bits() & 0xFF, 0x3F);
        assert!(read_back.is_enabled());
    }

    /// What the register holds before anything writes it.
    #[test]
    fn a_reset_apic_is_not_software_enabled() {
        assert!(!Spurious::from_bits(0x0000_00FF).is_enabled());
    }

    #[test]
    #[should_panic(expected = "low four bits")]
    fn a_vector_the_hardware_would_alter_is_refused() {
        let _ = Spurious::enabled(vector(0x30));
    }

    /// The hardwired nibble is four bits, not three.
    ///
    /// Values ending in 0xF or 0x0 cannot distinguish 0x0F from 0x07. The
    /// literal anchor prevents accepting 0x27 when hardware delivers 0x2F
    /// (D-124).
    #[test]
    fn the_hardwired_low_bits_are_the_whole_nibble() {
        assert_eq!(HARDWIRED_LOW_BITS, 0b1111);
    }

    #[test]
    #[should_panic(expected = "spurious")]
    fn a_vector_whose_low_nibble_is_seven_is_refused() {
        // 0b0111 passes a three-bit mask and fails a four-bit one.
        let _ = Spurious::enabled(vector(0x37));
    }
}
