use super::*;

#[test]
fn a_modern_apic_reports_an_integrated_version() {
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

/// An unmapped register page reads back as zero, which must not pass for a working APIC.
#[test]
fn nothing_at_all_is_not_an_integrated_apic() {
    assert!(!Version::from_bits(0).is_integrated());
    assert_eq!(Version::from_bits(0).number(), 0);
}

#[test]
fn the_version_ignores_everything_above_its_byte() {
    assert_eq!(Version::from_bits(0xFFFF_FF14).number(), 0x14);
}

/// Common values admit any threshold in 0x10..=0x14, so both sides anchor the boundary.
#[test]
fn the_integrated_threshold_is_sixteen() {
    assert_eq!(FIRST_INTEGRATED, 0x10);
    assert!(
        Version::from_bits(0x10).is_integrated(),
        "0x10 is integrated"
    );
    assert!(!Version::from_bits(0x0F).is_integrated(), "0x0F is not");
    for version in [0x11, 0x14, 0x15] {
        assert!(Version::from_bits(version).is_integrated(), "{version:#x}");
    }
}
