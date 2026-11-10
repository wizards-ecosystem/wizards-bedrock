//! Local and I/O APIC register encoding and range policy, after Intel SDM Vol. 3A ch. 11.
//!
//! # Examples
//!
//! ```
//! use x86_apic::local_vector::Entry;
//! use x86_apic::register;
//! use x86_apic::vector::InterruptVector;
//!
//! // The timer's entry lives at xAPIC offset 0x320, which is MSR 0x832.
//! assert_eq!(register::LVT_TIMER.msr(), 0x832);
//!
//! // A one-shot entry is its vector and nothing else; masking is one bit.
//! let vector = InterruptVector::new(0x41).expect("kernel-owned vector");
//! assert_eq!(Entry::one_shot(vector).bits(), 0x41);
//! assert_eq!(Entry::MASKED.bits(), 0x0001_0000);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

pub mod base;
pub mod command;
pub mod in_service;
pub mod io;
pub mod lifecycle;
pub mod local_vector;
pub mod pending;
pub mod register;
pub mod spurious;
pub mod timer;
pub mod vector;
pub mod version;

/// Bytes of memory-mapped register space one local APIC occupies: a page.
pub const REGISTER_BYTES: usize = 4096;

const FIRST_MSR: u32 = 0x800;

const REGISTER_STRIDE: usize = 16;

/// One local APIC register, named by its xAPIC offset in table 11-1 and reached by its x2APIC MSR.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Register(usize);

impl Register {
    /// The register `offset` bytes into the page; panics unless it is inside and aligned.
    pub const fn new(offset: usize) -> Self {
        assert!(
            offset <= REGISTER_BYTES - size_of::<u32>(),
            "register lies outside the local APIC page"
        );
        assert!(
            offset.is_multiple_of(REGISTER_STRIDE),
            "local APIC registers are 16-byte aligned"
        );
        Self(offset)
    }

    pub const fn offset(self) -> usize {
        self.0
    }

    /// The x2APIC MSR: each sixteen-byte slot of the page is one consecutive register number.
    pub const fn msr(self) -> u32 {
        FIRST_MSR + (self.0 / REGISTER_STRIDE) as u32
    }
}

#[cfg(test)]
mod tests;
