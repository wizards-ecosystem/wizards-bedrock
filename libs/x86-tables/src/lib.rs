//! x86_64 descriptor table encoding, after Intel SDM Vol. 3A chapters 3, 6 and 7.
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

/// A descriptor table reference: requested privilege in bits 1:0, the unused LDT bit in bit 2.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Selector(u16);

impl Selector {
    /// GDT entry `index` at ring `privilege`; panics unless they fit thirteen bits and two.
    pub const fn new(index: u16, privilege: u16) -> Self {
        assert!(index < 0x2000, "descriptor index does not fit a selector");
        assert!(privilege < 4, "privilege level is two bits");
        Self(index << 3 | privilege)
    }

    /// Reinterpret the raw contents of a segment register.
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

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
