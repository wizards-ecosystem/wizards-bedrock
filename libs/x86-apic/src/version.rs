//! The local APIC version register.

use register_bits::u16_halves;

/// Lowest version an integrated APIC reports; below it is the discrete 82489DX.
const FIRST_INTEGRATED: u8 = 0x10;

/// The contents of [`crate::register::VERSION`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Version(u32);

impl Version {
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The version number, in bits 7:0.
    pub const fn number(self) -> u8 {
        u16_halves(self.0).0 as u8
    }

    /// Highest local-vector-table entry index, in bits 23:16, so five means six entries.
    pub const fn maximum_lvt_entry(self) -> u8 {
        u16_halves(self.0).1 as u8
    }

    /// Whether this APIC is integrated; a zero read, from a misdirected page, is not.
    pub const fn is_integrated(self) -> bool {
        self.number() >= FIRST_INTEGRATED
    }
}

#[cfg(test)]
mod tests;
