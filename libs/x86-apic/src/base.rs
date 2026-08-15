//! The `IA32_APIC_BASE` model-specific register.
//!
//! One register answers four questions: where the register page is, whether the
//! APIC is enabled at all, whether it is in x2APIC mode — where these offsets
//! mean nothing — and whether this is the processor the firmware started.

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

/// Bits 12–51: the physical page the register block is mapped at.
const ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;

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

    /// Physical address of the register page.
    ///
    /// Comes back page-aligned with the flag bits masked off, so it can be
    /// handed to a mapping without stripping anything first.
    pub const fn address(self) -> u64 {
        self.0 & ADDRESS
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
    /// Everything else is preserved, including the address: the architecture
    /// permits relocating the register page and nothing here has any business
    /// doing so.
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

    /// What a bootstrap processor with an enabled xAPIC at the architectural
    /// default address reports.
    const TYPICAL: u64 = 0xFEE0_0900;

    #[test]
    fn the_default_base_decodes_as_an_enabled_bootstrap_xapic() {
        let base = Base::from_bits(TYPICAL);

        assert_eq!(base.address(), 0xFEE0_0000);
        assert!(base.is_bootstrap());
        assert!(base.is_enabled());
        assert!(!base.is_x2apic());
    }

    /// The flag bits live below bit 12 and the address starts at it, so an
    /// address that came back carrying them would be off by a page or less —
    /// the kind of mistake that maps the wrong thing rather than failing.
    #[test]
    fn the_address_carries_none_of_the_flags() {
        let base = Base::from_bits(TYPICAL);

        assert_eq!(base.address() % 4096, 0);
        assert_eq!(base.address() & BOOTSTRAP, 0);
        assert_eq!(base.address() & ENABLE, 0);
        assert_eq!(base.address() & X2APIC, 0);
    }

    #[test]
    fn the_address_spans_bits_twelve_to_fifty_one() {
        let base = Base::from_bits(u64::MAX);

        assert_eq!(base.address(), 0x000F_FFFF_FFFF_F000);
    }

    #[test]
    fn enabling_sets_one_bit_and_leaves_the_rest() {
        let disabled = Base::from_bits(TYPICAL & !ENABLE);
        let enabled = disabled.enabled();

        assert!(!disabled.is_enabled());
        assert!(enabled.is_enabled());
        assert_eq!(enabled.address(), disabled.address());
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
        assert_eq!(extended.address(), xapic.address());
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
}
