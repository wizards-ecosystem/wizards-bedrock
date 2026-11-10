//! The `IA32_APIC_BASE` model-specific register.

pub const MSR: u32 = 0x1B;

/// Bit 8: this is the processor the firmware started from reset.
const BOOTSTRAP: u64 = 1 << 8;

/// Bit 10: the APIC is reached through MSRs, so no offset in [`crate::register`] means anything.
const X2APIC: u64 = 1 << 10;

/// Bit 11: the hardware switch, whose clearing is permanent until reset on some processors.
const ENABLE: u64 = 1 << 11;

/// The contents of [`MSR`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Base(u64);

impl Base {
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

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

    pub const fn is_x2apic(self) -> bool {
        self.0 & X2APIC != 0
    }

    /// The same register with the hardware enable bit set.
    #[must_use]
    pub const fn enabled(self) -> Self {
        Self(self.0 | ENABLE)
    }

    /// The same register in x2APIC mode, enabled too, since disabled to x2APIC in one write faults.
    #[must_use]
    pub const fn x2apic(self) -> Self {
        Self(self.0 | ENABLE | X2APIC)
    }
}

#[cfg(test)]
mod tests;
