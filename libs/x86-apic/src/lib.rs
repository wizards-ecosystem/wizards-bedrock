//! Local and I/O APIC register encoding and range policy.
//!
//! Which register is which, which bit software-enables the controller, how a
//! local vector table entry packs a vector and a mask, what a self-directed
//! interrupt command looks like: arithmetic over bits with no hardware behind
//! it, so it lives here and is unit-tested on the host. `wost` encodes with
//! this crate and does nothing but write the result — see
//! `docs/006-cpu-fundamentals.md`.
//!
//! Bit positions and offsets follow Intel SDM Vol. 3A ch. 11 (advanced
//! programmable interrupt controller), table 11-1 in particular.
//!
//! Registers are identified by their xAPIC offset because that is how the
//! manual's table names them, and converted to an x2APIC model-specific
//! register number by [`Register::msr`]. Only the second is reachable in this
//! kernel: the first is a page of memory-mapped words that the bootloader does
//! not map and the kernel cannot map for itself until it owns page tables.
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

/// Bytes of memory-mapped register space one local APIC occupies — a page.
pub const REGISTER_BYTES: usize = 4096;

/// Model-specific register the x2APIC block starts at.
const FIRST_MSR: u32 = 0x800;

/// Boundary every local APIC register is aligned to.
const REGISTER_STRIDE: usize = 16;

/// One local APIC register, named by where it sits in the memory-mapped page.
///
/// The two modes number the same registers differently, and this is the one
/// place that knows it: the offset the manual's table gives, and the
/// model-specific register [`msr`](Self::msr) derives from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Register(usize);

impl Register {
    /// The register `offset` bytes into the register page.
    ///
    /// # Panics
    ///
    /// Panics if `offset` leaves the page or is not 16-byte aligned. Every
    /// register in the architecture's table is both, so this fires only on a
    /// typo — at compile time, since every caller is a constant.
    pub const fn new(offset: usize) -> Self {
        // Subtraction: `offset + 4` wraps near `usize::MAX` and would name the
        // wrong fault (D-105); `REGISTER_BYTES` exceeds four, so this cannot underflow.
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

    /// Byte offset from the base of the memory-mapped register page.
    pub const fn offset(self) -> usize {
        self.0
    }

    /// The model-specific register x2APIC mode reaches this one through.
    ///
    /// The block is laid out so that each sixteen-byte slot in the page becomes
    /// one consecutive register number from the base of the block — the same table,
    /// divided by its own stride.
    pub const fn msr(self) -> u32 {
        FIRST_MSR + (self.0 / REGISTER_STRIDE) as u32
    }
}

#[cfg(test)]
mod tests;
