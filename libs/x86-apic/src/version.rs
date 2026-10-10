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
mod tests {
    use super::*;

    #[test]
    fn a_modern_apic_reports_an_integrated_version() {
        // Six LVT entries and version 0x14, which is what current parts report.
        let version = Version::from_bits(0x0005_0014);

        assert_eq!(version.number(), 0x14);
        assert_eq!(version.maximum_lvt_entry(), 5);
        assert!(version.is_integrated());
    }

    #[test]
    fn the_lvt_limit_is_independent_of_the_version_number() {
        assert_eq!(Version::from_bits(0x00A6_00F1).maximum_lvt_entry(), 0xA6);
        assert_eq!(Version::from_bits(0x0005_FF14).maximum_lvt_entry(), 5);
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

    /// The boundary, both sides.
    ///
    /// Common values 0x14, 0x0F and 0 admit any threshold in 0x10..=0x14, so
    /// both sides explicitly anchor the architectural boundary (D-124).
    #[test]
    fn the_integrated_threshold_is_sixteen() {
        assert_eq!(FIRST_INTEGRATED, 0x10);
        assert!(
            Version::from_bits(0x10).is_integrated(),
            "0x10 is integrated"
        );
        assert!(!Version::from_bits(0x0F).is_integrated(), "0x0F is not");
        // The versions real parts report.
        for version in [0x11, 0x14, 0x15] {
            assert!(Version::from_bits(version).is_integrated(), "{version:#x}");
        }
    }
}
