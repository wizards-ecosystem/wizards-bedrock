//! Local APIC register encoding.
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

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod base;
pub mod command;
pub mod local_vector;
pub mod register;
pub mod spurious;
pub mod timer;
pub mod version;

/// Bytes of memory-mapped register space one local APIC occupies — a page.
pub const REGISTER_BYTES: usize = 4096;

/// Model-specific register the x2APIC block starts at.
const FIRST_MSR: u32 = 0x800;

/// Lowest vector the local APIC will deliver.
///
/// The architecture reserves 0–15, and an APIC told to deliver one records an
/// illegal-vector error instead of doing it. Both [`local_vector::Entry`] and
/// [`command::Command`] refuse them rather than leaving the mistake to be found
/// in the error-status register.
pub const FIRST_VECTOR: u8 = 16;

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
        assert!(
            offset + size_of::<u32>() <= REGISTER_BYTES,
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
mod tests {
    use super::*;

    #[test]
    fn a_register_remembers_its_offset() {
        assert_eq!(Register::new(0x0B0).offset(), 0x0B0);
        assert_eq!(Register::new(0).offset(), 0);
    }

    #[test]
    #[should_panic(expected = "16-byte aligned")]
    fn a_register_rejects_an_unaligned_offset() {
        let _ = Register::new(0x0B4);
    }

    #[test]
    #[should_panic(expected = "outside the local APIC page")]
    fn a_register_rejects_an_offset_past_the_page() {
        let _ = Register::new(REGISTER_BYTES);
    }

    /// The last aligned slot in the page still has to hold four whole bytes.
    #[test]
    fn the_last_register_in_the_page_is_accepted() {
        assert_eq!(
            Register::new(REGISTER_BYTES - REGISTER_STRIDE).offset(),
            0xFF0
        );
    }

    /// Spot values from SDM Vol. 3A table 11-1's "MSR address" column. Getting
    /// this arithmetic wrong reads a neighbouring register, which answers
    /// plausibly and means something else entirely.
    #[test]
    fn a_register_maps_to_the_msr_the_manual_gives() {
        assert_eq!(Register::new(0x020).msr(), 0x802, "id");
        assert_eq!(Register::new(0x0B0).msr(), 0x80B, "end of interrupt");
        assert_eq!(Register::new(0x0F0).msr(), 0x80F, "spurious");
        assert_eq!(Register::new(0x300).msr(), 0x830, "interrupt command");
        assert_eq!(Register::new(0x370).msr(), 0x837, "lvt error");
    }

    #[test]
    fn consecutive_slots_become_consecutive_registers() {
        let first = Register::new(0);
        let second = Register::new(REGISTER_STRIDE);

        assert_eq!(first.msr(), FIRST_MSR);
        assert_eq!(second.msr(), first.msr() + 1);
    }

    /// The whole page maps into the block the architecture reserves for it,
    /// 0x800 through 0x8FF, so no offset can ever name a register belonging to
    /// something else.
    #[test]
    fn no_register_escapes_the_reserved_msr_block() {
        for offset in (0..REGISTER_BYTES).step_by(REGISTER_STRIDE) {
            let msr = Register::new(offset).msr();
            assert!((0x800..=0x8FF).contains(&msr), "offset {offset:#x}");
        }
    }
}
