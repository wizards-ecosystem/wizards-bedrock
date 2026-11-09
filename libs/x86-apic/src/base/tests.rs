use super::*;

#[test]
fn x2apic_keeps_every_bit_that_is_already_set() {
    let promoted = Base::from_bits(TYPICAL).x2apic();
    assert_eq!(promoted.bits() & TYPICAL, TYPICAL, "no bit is cleared");
    let again = Base::from_bits(TYPICAL | X2APIC).x2apic();
    assert_eq!(again.bits(), TYPICAL | X2APIC, "the door is idempotent");
}

/// What a bootstrap processor with an enabled xAPIC at the architectural
/// default address reports.
const TYPICAL: u64 = 0xFEE0_0900;

/// The three flag bits sit where the manual puts them.
///
/// `TYPICAL = 0xFEE0_0900` sets both `BOOTSTRAP` and `ENABLE`, so swapping
/// the two constants left all fifty-seven tests green — and
/// `Base::enabled()` would then set the **read-only** bootstrap bit and
/// never enable the APIC (D-142).
///
/// SDM Vol. 3A §11.4.4, `IA32_APIC_BASE`: bit 8 BSP, bit 10 x2APIC enable,
/// bit 11 APIC global enable. Written as literals, against which the
/// constants are checked one at a time.
#[test]
fn the_base_flags_sit_where_the_manual_puts_them() {
    assert_eq!(BOOTSTRAP, 1 << 8, "BSP is bit 8");
    assert_eq!(X2APIC, 1 << 10, "x2APIC enable is bit 10");
    assert_eq!(ENABLE, 1 << 11, "APIC global enable is bit 11");
    // Pairwise distinct, so no constant can stand in for another.
    assert_ne!(BOOTSTRAP, X2APIC);
    assert_ne!(X2APIC, ENABLE);
    assert_ne!(BOOTSTRAP, ENABLE);
}

#[test]
fn the_default_base_decodes_as_an_enabled_bootstrap_xapic() {
    let base = Base::from_bits(TYPICAL);

    assert!(base.is_bootstrap());
    assert!(base.is_enabled());
    assert!(!base.is_x2apic());
}

#[test]
fn bootstrap_status_depends_only_on_bit_eight() {
    assert!(!Base::from_bits(0).is_bootstrap());
    assert!(!Base::from_bits(ENABLE | X2APIC).is_bootstrap());
    assert!(Base::from_bits(BOOTSTRAP).is_bootstrap());
}

#[test]
fn enabling_sets_one_bit_and_leaves_the_rest() {
    let disabled = Base::from_bits(TYPICAL & !ENABLE);
    let enabled = disabled.enabled();

    assert!(!disabled.is_enabled());
    assert!(enabled.is_enabled());
    assert_eq!(enabled.is_bootstrap(), disabled.is_bootstrap());
    assert_eq!(enabled.bits(), TYPICAL);
}

#[test]
fn enabling_an_enabled_apic_changes_nothing() {
    let base = Base::from_bits(TYPICAL);

    assert_eq!(base.enabled(), base);
}

#[test]
fn x2apic_mode_is_visible() {
    assert!(Base::from_bits(TYPICAL | X2APIC).is_x2apic());
}

#[test]
fn switching_to_x2apic_keeps_the_apic_enabled() {
    let xapic = Base::from_bits(TYPICAL);
    let extended = xapic.x2apic();

    assert!(extended.is_x2apic());
    assert!(extended.is_enabled());
    assert_eq!(extended.bits(), TYPICAL | X2APIC);
}

#[test]
fn asking_an_x2apic_for_x2apic_is_idempotent() {
    let extended = Base::from_bits(TYPICAL | X2APIC);
    assert_eq!(extended.x2apic(), extended);
}

/// A disabled APIC asked for x2APIC must come back with both bits, not just
/// the extended one — the pair 0b10 is the encoding the processor faults on.
#[test]
fn switching_a_disabled_apic_sets_the_enable_bit_too() {
    let disabled = Base::from_bits(TYPICAL & !ENABLE);

    assert_eq!(
        disabled.x2apic().bits() & (ENABLE | X2APIC),
        ENABLE | X2APIC
    );
}

/// The base MSR's fields sit where the manual puts them.
///
/// Literal anchors keep an implementation constant from serving as its own
/// oracle (D-105).
#[test]
fn the_base_msr_fields_are_where_the_manual_puts_them() {
    // SDM Vol. 3A §11.4.4 / table 11-5: bit 8 BSP, bit 10 x2APIC enable,
    // bit 11 global enable; MSR 0x1B is IA32_APIC_BASE.
    assert_eq!(MSR, 0x1B);
    assert_eq!(X2APIC, 0x0000_0400);

    // A word with only bit 10 set is x2APIC and nothing else.
    let only_mode = Base::from_bits(0x0000_0400);
    assert!(only_mode.is_x2apic());
    assert!(!only_mode.is_enabled());

    // And bit 9 is not the mode bit.
    assert!(!Base::from_bits(0x0000_0200).is_x2apic());
}
