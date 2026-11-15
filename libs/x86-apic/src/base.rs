pub const MSR: u32 = 0x1B;

const BOOTSTRAP: u64 = 1 << 8;

const X2APIC: u64 = 1 << 10;

// Clearing it is permanent until reset on some processors.
const ENABLE: u64 = 1 << 11;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Base(u64);

impl Base {
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn is_bootstrap(self) -> bool {
        self.0 & BOOTSTRAP != 0
    }

    pub const fn is_enabled(self) -> bool {
        self.0 & ENABLE != 0
    }

    pub const fn is_x2apic(self) -> bool {
        self.0 & X2APIC != 0
    }

    #[must_use]
    pub const fn enabled(self) -> Self {
        Self(self.0 | ENABLE)
    }

    // From disabled, write `enabled` first: this value in one write from disabled faults.
    #[must_use]
    pub const fn x2apic(self) -> Self {
        Self(self.0 | ENABLE | X2APIC)
    }
}
