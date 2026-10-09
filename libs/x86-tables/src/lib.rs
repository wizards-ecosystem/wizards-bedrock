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
//! // The two values every x86_64 kernel arrives at.
//! assert_eq!(Descriptor::KERNEL_CODE.bits(), 0x0020_9a00_0000_0000);
//! assert_eq!(Descriptor::KERNEL_DATA.bits(), 0x0000_9200_0000_0000);
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
mod tests {
    use super::*;

    #[test]
    fn selector_packs_index_above_privilege() {
        assert_eq!(Selector::new(1, 0).bits(), 0x08);
        assert_eq!(Selector::new(2, 0).bits(), 0x10);
        assert_eq!(Selector::new(3, 0).bits(), 0x18);
        assert_eq!(Selector::new(4, 3).bits(), 0x23);
    }

    #[test]
    fn selector_round_trips_through_its_bits() {
        let selector = Selector::new(5, 3);
        let read_back = Selector::from_bits(selector.bits());
        assert_eq!(read_back.index(), 5);
        assert_eq!(read_back.bits() & 0b11, 3);
        assert_eq!(read_back, selector);
    }

    #[test]
    #[should_panic(expected = "privilege level is two bits")]
    fn selector_rejects_an_impossible_privilege() {
        let _ = Selector::new(1, 4);
    }

    #[test]
    #[should_panic(expected = "descriptor index does not fit a selector")]
    fn an_index_past_thirteen_bits_is_refused() {
        // `index << 3` would drop the top bits and name a different
        // descriptor; nothing tested this assert (D-105).
        let _ = Selector::new(0x2000, 0);
    }

    #[test]
    fn the_largest_index_a_selector_holds_is_accepted() {
        // Index in 15:3, table indicator clear in bit 2, RPL in 1:0.
        assert_eq!(Selector::new(0x1FFF, 3).bits(), 0xFFFB);
    }
}
