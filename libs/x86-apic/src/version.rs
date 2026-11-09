//! The local APIC version register.

/// Lowest version number an APIC integrated into the processor reports.
///
/// Below it is the discrete 82489DX of the 486 era, which predates most of what
/// the rest of this crate encodes.
const FIRST_INTEGRATED: u8 = 0x10;

/// First bit of the maximum local-vector-table entry number.
const MAXIMUM_LVT_ENTRY_SHIFT: u32 = 16;

/// The contents of [`crate::register::VERSION`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version(u32);

impl Version {
    /// Interpret what the register returned.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The version number, in bits 0–7.
    pub const fn number(self) -> u8 {
        self.0 as u8
    }

    /// Highest local-vector-table entry number this APIC implements.
    ///
    /// The architectural value is an index rather than a count, so five means
    /// the controller implements entries zero through five.
    pub const fn maximum_lvt_entry(self) -> u8 {
        (self.0 >> MAXIMUM_LVT_ENTRY_SHIFT) as u8
    }

    /// Whether this APIC is part of the processor rather than a separate chip.
    ///
    /// A reading of zero is the other useful answer: it means nothing answered
    /// the read, which is what an unmapped or misdirected register page looks
    /// like from here.
    pub const fn is_integrated(self) -> bool {
        self.number() >= FIRST_INTEGRATED
    }
}

#[cfg(test)]
mod tests;
