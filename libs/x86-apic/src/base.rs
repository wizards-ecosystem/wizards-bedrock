//! The `IA32_APIC_BASE` model-specific register.
//!
//! One register records whether the APIC is enabled, whether it uses x2APIC,
//! and whether this is the processor the firmware started.

/// Register number `rdmsr` and `wrmsr` name.
pub const MSR: u32 = 0x1B;

/// Bit 8: this is the processor the firmware started from reset.
const BOOTSTRAP: u64 = 1 << 8;

/// Bit 10: the APIC is reached through model-specific registers rather than
/// memory, and every offset in [`crate::register`] is meaningless.
const X2APIC: u64 = 1 << 10;

/// Bit 11: the APIC exists as far as the rest of the machine is concerned.
/// Distinct from the software-enable bit in the spurious-interrupt register:
/// this one is the hardware switch, and on some processors clearing it is
/// permanent until reset.
const ENABLE: u64 = 1 << 11;

/// The contents of [`MSR`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Base(u64);

impl Base {
    /// Interpret what `rdmsr` returned.
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The value to hand `wrmsr`.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Whether this is the processor the firmware started.
    pub const fn is_bootstrap(self) -> bool {
        self.0 & BOOTSTRAP != 0
    }

    /// Whether the APIC is enabled in hardware.
    pub const fn is_enabled(self) -> bool {
        self.0 & ENABLE != 0
    }

    /// Whether the APIC is in x2APIC mode.
    pub const fn is_x2apic(self) -> bool {
        self.0 & X2APIC != 0
    }

    /// The same register with the hardware enable bit set.
    ///
    /// Every other register bit is preserved.
    pub const fn enabled(self) -> Self {
        Self(self.0 | ENABLE)
    }

    /// The same register in x2APIC mode.
    ///
    /// Sets the hardware enable bit as well, because the architecture's state
    /// machine has no edge from disabled straight to x2APIC — writing that
    /// combination in one step raises a general protection fault. A caller
    /// still has to make the two writes in order; this only ensures the second
    /// one is not also the first.
    pub const fn x2apic(self) -> Self {
        Self(self.0 | ENABLE | X2APIC)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x2apic_keeps_an_enable_bit_that_is_already_set() {
        let promoted = Base::from_bits(TYPICAL).x2apic();
        assert_eq!(promoted.bits() & TYPICAL, TYPICAL, "no bit is cleared");
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
}
