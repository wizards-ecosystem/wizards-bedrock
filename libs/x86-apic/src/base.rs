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
    #[must_use]
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
    #[must_use]
    pub const fn x2apic(self) -> Self {
        Self(self.0 | ENABLE | X2APIC)
    }
}

#[cfg(test)]
mod tests;
