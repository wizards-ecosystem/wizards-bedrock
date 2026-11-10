//! x86_64 descriptor table encoding.
//!
//! Which bit of a segment descriptor means present, where a 64-bit gate splits
//! its handler address, what a task-state segment looks like in memory: all of
//! it is arithmetic over bits with no hardware behind it, so it lives here and
//! is unit-tested on the host. `wost` builds descriptors from this crate and
//! does nothing to them but load them — see `docs/006-cpu-fundamentals.md`.
//!
//! Bit positions follow Intel SDM Vol. 3A ch. 3 (segment descriptors), ch. 6
//! (gate descriptors) and ch. 7 (the 64-bit TSS).
//!
//! # Examples
//!
//! ```
//! use x86_tables::Selector;
//! use x86_tables::gdt::Descriptor;
//!
//! // Entry one at ring zero: the index sits above the three low bits.
//! assert_eq!(Selector::new(1, 0).bits(), 0x08);
//!
//! // The two values every x86_64 kernel arrives at, already accessed so the
//! // processor never writes the table.
//! assert_eq!(Descriptor::KERNEL_CODE.bits(), 0x0020_9b00_0000_0000);
//! assert_eq!(Descriptor::KERNEL_DATA.bits(), 0x0000_9300_0000_0000);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod exception;
pub mod gdt;
pub mod idt;
pub mod return_state;
pub mod tss;

/// A reference to one descriptor table entry, as loaded into a segment register.
///
/// The low two bits are the requested privilege level and the third selects the
/// local descriptor table, which this kernel never uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Selector(u16);

impl Selector {
    /// A selector for global descriptor table entry `index`, requesting ring
    /// `privilege`.
    ///
    /// # Panics
    ///
    /// Panics if `index` does not fit in thirteen bits or `privilege` exceeds 3.
    pub const fn new(index: u16, privilege: u16) -> Self {
        assert!(index < 0x2000, "descriptor index does not fit a selector");
        assert!(privilege < 4, "privilege level is two bits");
        Self(index << 3 | privilege)
    }

    /// Reinterpret the raw contents of a segment register.
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// The value a segment register holds.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Which descriptor table entry this names.
    pub const fn index(self) -> u16 {
        self.0 >> 3
    }
}

#[cfg(test)]
mod tests;
