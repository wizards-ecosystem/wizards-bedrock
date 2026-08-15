//! The local APIC version register.

/// Lowest version number an APIC integrated into the processor reports.
///
/// Below it is the discrete 82489DX of the 486 era, which predates most of what
/// the rest of this crate encodes.
const FIRST_INTEGRATED: u8 = 0x10;

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
mod tests {
    use super::*;

    #[test]
    fn a_modern_apic_reports_an_integrated_version() {
        // Six LVT entries and version 0x14, which is what current parts report.
        let version = Version::from_bits(0x0005_0014);

        assert_eq!(version.number(), 0x14);
        assert!(version.is_integrated());
    }

    #[test]
    fn the_discrete_controller_is_not_integrated() {
        assert!(!Version::from_bits(0x0003_000F).is_integrated());
    }

    /// An unmapped register page reads back as zero, and a version of zero must
    /// not pass for a working APIC.
    #[test]
    fn nothing_at_all_is_not_an_integrated_apic() {
        assert!(!Version::from_bits(0).is_integrated());
        assert_eq!(Version::from_bits(0).number(), 0);
    }

    #[test]
    fn the_version_ignores_everything_above_its_byte() {
        assert_eq!(Version::from_bits(0xFFFF_FF14).number(), 0x14);
    }
}
