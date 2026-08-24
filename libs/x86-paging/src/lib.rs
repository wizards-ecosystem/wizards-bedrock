//! x86_64 four-level page table encoding.
//!
//! Entry bits, the index each level takes from a virtual address, and
//! canonical form. `wost` owns the tables; the encoding is checkable on the
//! host, so D-30 puts it here.
//!
//! Transcribed from Intel SDM Vol. 3A §4.5, tables 4-14 through 4-19. The
//! tests restate each bit position from the manual rather than from this file,
//! which is the only way a transcription error fails instead of agreeing with
//! itself.
//!
//! ```
//! use x86_paging::{Entry, Flags, Level, PAGE_SIZE};
//!
//! // A read-only, executable mapping of physical 0x2000 — kernel text.
//! let text = Entry::page(0x2000, Flags::PRESENT).expect("aligned and in range");
//! assert!(text.flags().contains(Flags::PRESENT));
//! assert!(!text.flags().contains(Flags::WRITABLE));
//! assert_eq!(text.address(), 0x2000);
//!
//! // The slot a virtual address takes in the top-level table.
//! assert_eq!(Level::Pml4.index(0xffff_ffff_8000_0000), 511);
//! assert_eq!(PAGE_SIZE, 4096);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

/// Bytes in the smallest page.
pub const PAGE_SIZE: u64 = 4096;

/// Bytes in a large page, mapped by a single page-directory entry.
pub const LARGE_PAGE_SIZE: u64 = 2 << 20;

/// Entries in every table at every level.
pub const ENTRIES: usize = 512;

/// Bits of an entry that hold a physical address.
///
/// SDM: bits 51:12. The architecture reserves 63:52 for flags and the
/// no-execute bit, and the low twelve bits are always zero because every
/// target is page-aligned.
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

/// Physical addresses this encoding can represent.
pub const MAX_PHYSICAL: u64 = ADDRESS_MASK + PAGE_SIZE;

/// Flags of a page table entry.
///
/// A hand-rolled bit set rather than a `bitflags` dependency, on the
/// `sha256.rs` precedent: one type, a handful of operations, and the
/// alternative is a crate in the kernel's trusted path.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags(u64);

impl Flags {
    /// The entry is valid. Without it the CPU ignores every other bit.
    pub const PRESENT: Self = Self(1 << 0);
    /// Writes are permitted. Absent, the mapping is read-only.
    pub const WRITABLE: Self = Self(1 << 1);
    /// Ring 3 may reach it. Absent, the mapping is supervisor-only.
    pub const USER: Self = Self(1 << 2);
    /// Writes go straight to memory rather than being cached back.
    pub const WRITE_THROUGH: Self = Self(1 << 3);
    /// The region is not cached at all — what device memory wants.
    pub const NO_CACHE: Self = Self(1 << 4);
    /// The CPU has read through this entry since it was last cleared.
    pub const ACCESSED: Self = Self(1 << 5);
    /// The CPU has written through this entry since it was last cleared.
    pub const DIRTY: Self = Self(1 << 6);
    /// At a page-directory level, this entry maps a large page directly
    /// instead of pointing at the table below.
    pub const HUGE: Self = Self(1 << 7);
    /// The mapping survives a `CR3` reload. For pages the kernel keeps in
    /// every address space.
    pub const GLOBAL: Self = Self(1 << 8);
    /// Instruction fetches from this mapping fault. Honoured only once
    /// `EFER.NXE` is set, which is why it is enabled before any table is
    /// built.
    pub const NO_EXECUTE: Self = Self(1 << 63);

    /// No flags at all.
    pub const NONE: Self = Self(0);

    /// The bits themselves.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Both sets of flags together.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// This set without the flags in `other`.
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Whether every flag in `other` is present here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any flag in `other` is present here.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether this mapping is both writable and executable.
    ///
    /// The predicate W^X is the absence of. Named here rather than spelled out
    /// at each call site so the security baseline has one definition (D-12).
    pub const fn is_write_execute(self) -> bool {
        self.contains(Self::PRESENT)
            && self.contains(Self::WRITABLE)
            && !self.contains(Self::NO_EXECUTE)
    }
}

impl core::ops::BitOr for Flags {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.union(other)
    }
}

impl fmt::Debug for Flags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        for (flag, name) in [
            (Self::PRESENT, "present"),
            (Self::WRITABLE, "writable"),
            (Self::USER, "user"),
            (Self::WRITE_THROUGH, "write-through"),
            (Self::NO_CACHE, "no-cache"),
            (Self::ACCESSED, "accessed"),
            (Self::DIRTY, "dirty"),
            (Self::HUGE, "huge"),
            (Self::GLOBAL, "global"),
            (Self::NO_EXECUTE, "no-execute"),
        ] {
            if self.contains(flag) {
                f.write_str(separator)?;
                f.write_str(name)?;
                separator = " ";
            }
        }
        if separator.is_empty() {
            f.write_str("none")?;
        }
        Ok(())
    }
}

/// Why an entry could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntryError {
    /// The physical address is not page-aligned.
    Misaligned(u64),
    /// The physical address has bits above the 52 the encoding carries.
    OutOfRange(u64),
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Misaligned(address) => write!(f, "{address:#x} is not page-aligned"),
            Self::OutOfRange(address) => write!(f, "{address:#x} does not fit 52 bits"),
        }
    }
}

impl core::error::Error for EntryError {}

/// One page table entry.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct Entry(u64);

impl Entry {
    /// An absent entry — every bit zero.
    pub const EMPTY: Self = Self(0);

    /// Rebuild an entry from the word a table holds.
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The word to store in a table.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// An entry mapping a page at `address` with `flags`.
    ///
    /// `PRESENT` is not added for you: an entry whose caller forgot it is a
    /// mapping that silently does not exist, and that is worth a compile-time
    /// habit of naming it.
    ///
    /// # Errors
    ///
    /// [`EntryError::Misaligned`] unless `address` is aligned to the page the
    /// entry maps — in a `HUGE` entry bit 12 is the PAT select, not address,
    /// so a 4 KiB-aligned frame would silently encode both wrong — and
    /// [`EntryError::OutOfRange`] if it needs more than 52 bits.
    pub const fn page(address: u64, flags: Flags) -> Result<Self, EntryError> {
        let alignment = if flags.contains(Flags::HUGE) {
            Level::Pd.span()
        } else {
            PAGE_SIZE
        };
        if !address.is_multiple_of(alignment) {
            return Err(EntryError::Misaligned(address));
        }
        if address & !ADDRESS_MASK != 0 {
            return Err(EntryError::OutOfRange(address));
        }
        Ok(Self(address | flags.bits()))
    }

    /// An entry pointing at the table below.
    ///
    /// Intermediate entries carry `PRESENT | WRITABLE` and never `NO_EXECUTE`:
    /// the effective permission of a mapping is the *intersection* down the
    /// walk, so a restrictive parent would silently cap every leaf beneath it.
    /// W^X is therefore enforced at the leaves, which is where this kernel
    /// checks it.
    ///
    /// # Errors
    ///
    /// As [`page`](Self::page).
    pub const fn table(address: u64) -> Result<Self, EntryError> {
        Self::page(address, Flags::PRESENT.union(Flags::WRITABLE))
    }

    /// The physical address this entry names.
    pub const fn address(self) -> u64 {
        self.0 & ADDRESS_MASK
    }

    /// The flags this entry carries.
    pub const fn flags(self) -> Flags {
        Flags(self.0 & !ADDRESS_MASK)
    }

    /// Whether the entry maps or points at anything at all.
    pub const fn is_present(self) -> bool {
        self.flags().contains(Flags::PRESENT)
    }

    /// Whether the entry maps a large page rather than naming a table.
    ///
    /// Meaningful only at [`Level::Pd`] and [`Level::Pdpt`]; the bit means
    /// something else entirely at the other two.
    pub const fn is_huge(self) -> bool {
        self.flags().contains(Flags::HUGE)
    }
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.is_present() {
            return f.write_str("Entry(absent)");
        }
        write!(f, "Entry({:#x} {:?})", self.address(), self.flags())
    }
}

/// A level of the four-level page table walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// The page table: entries map 4 KiB pages.
    Pt = 1,
    /// The page directory: entries map 2 MiB pages or name a [`Pt`](Self::Pt).
    Pd = 2,
    /// The page-directory-pointer table: entries map 1 GiB pages or name a
    /// [`Pd`](Self::Pd).
    Pdpt = 3,
    /// The top-level table, whose physical address goes in `CR3`.
    Pml4 = 4,
}

impl Level {
    /// Every level, top down — the order a walk visits them.
    pub const WALK: [Self; 4] = [Self::Pml4, Self::Pdpt, Self::Pd, Self::Pt];

    /// The level below this one, or `None` at [`Pt`](Self::Pt).
    pub const fn below(self) -> Option<Self> {
        match self {
            Self::Pml4 => Some(Self::Pdpt),
            Self::Pdpt => Some(Self::Pd),
            Self::Pd => Some(Self::Pt),
            Self::Pt => None,
        }
    }

    /// The lowest bit of the virtual address this level's index occupies.
    ///
    /// Nine bits per level above the twelve-bit page offset: 12, 21, 30, 39.
    pub const fn shift(self) -> u32 {
        12 + 9 * (self as u32 - 1)
    }

    /// Bytes one entry at this level covers.
    pub const fn span(self) -> u64 {
        1 << self.shift()
    }

    /// The slot `address` selects in a table at this level.
    pub const fn index(self, address: u64) -> usize {
        (address >> self.shift()) as usize % ENTRIES
    }

    /// Whether this level's entries may map a page directly.
    ///
    /// [`Pt`](Self::Pt) always does. [`Pd`](Self::Pd) and
    /// [`Pdpt`](Self::Pdpt) do with [`Flags::HUGE`]. [`Pml4`](Self::Pml4)
    /// never does on any shipping x86_64.
    pub const fn can_map_page(self) -> bool {
        !matches!(self, Self::Pml4)
    }
}

/// Whether `address` is canonical — bits 63:48 all copies of bit 47.
///
/// The CPU faults on a non-canonical address rather than truncating it, so
/// this is worth checking before a walk rather than after a general-protection
/// fault.
pub const fn is_canonical(address: u64) -> bool {
    let sign_extended = ((address as i64) << 16 >> 16) as u64;
    sign_extended == address
}

/// Sign-extend a 48-bit address into canonical form.
pub const fn canonicalise(address: u64) -> u64 {
    ((address as i64) << 16 >> 16) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bit positions, restated from Intel SDM Vol. 3A table 4-19 rather than
    /// from the constants above. A transcription error shows up here as a
    /// failure; copying the constants would only prove they equal themselves.
    #[test]
    fn flag_bits_match_the_manual() {
        assert_eq!(Flags::PRESENT.bits(), 1);
        assert_eq!(Flags::WRITABLE.bits(), 2);
        assert_eq!(Flags::USER.bits(), 4);
        assert_eq!(Flags::WRITE_THROUGH.bits(), 8);
        assert_eq!(Flags::NO_CACHE.bits(), 16);
        assert_eq!(Flags::ACCESSED.bits(), 32);
        assert_eq!(Flags::DIRTY.bits(), 64);
        assert_eq!(Flags::HUGE.bits(), 128);
        assert_eq!(Flags::GLOBAL.bits(), 256);
        assert_eq!(Flags::NO_EXECUTE.bits(), 0x8000_0000_0000_0000);
    }

    #[test]
    fn the_address_field_is_bits_51_through_12() {
        assert_eq!(ADDRESS_MASK.trailing_zeros(), 12);
        assert_eq!(ADDRESS_MASK.count_ones(), 40, "51 - 12 + 1 bits");
        assert_eq!(ADDRESS_MASK.leading_zeros(), 12, "bits 63:52 are flags");
    }

    #[test]
    fn an_entry_round_trips_its_address_and_flags() {
        let flags = Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE;
        let entry = Entry::page(0x1234_5000, flags).expect("aligned");

        assert_eq!(entry.address(), 0x1234_5000);
        assert_eq!(entry.flags(), flags);
        assert!(entry.is_present());
        assert_eq!(Entry::from_bits(entry.bits()), entry);
    }

    #[test]
    fn the_address_and_the_flags_never_overlap() {
        // The highest representable address alongside every flag: neither may
        // corrupt the other.
        let flags = Flags::PRESENT | Flags::NO_EXECUTE | Flags::GLOBAL;
        let top = MAX_PHYSICAL - PAGE_SIZE;
        let entry = Entry::page(top, flags).expect("in range");

        assert_eq!(entry.address(), top);
        assert_eq!(entry.flags(), flags);
    }

    #[test]
    fn a_misaligned_or_oversized_address_is_refused() {
        assert_eq!(
            Entry::page(0x1001, Flags::PRESENT),
            Err(EntryError::Misaligned(0x1001))
        );
        assert_eq!(
            Entry::page(MAX_PHYSICAL, Flags::PRESENT),
            Err(EntryError::OutOfRange(MAX_PHYSICAL))
        );
    }

    #[test]
    fn an_empty_entry_is_absent() {
        assert!(!Entry::EMPTY.is_present());
        assert_eq!(Entry::EMPTY.address(), 0);
        assert_eq!(Entry::EMPTY.flags(), Flags::NONE);
    }

    #[test]
    fn an_intermediate_entry_is_permissive() {
        let entry = Entry::table(0x3000).expect("aligned");

        assert!(entry.is_present());
        assert!(entry.flags().contains(Flags::WRITABLE));
        assert!(
            !entry.flags().contains(Flags::NO_EXECUTE),
            "permissions intersect down the walk, so a parent must not restrict"
        );
        assert!(!entry.flags().contains(Flags::USER), "kernel tables only");
    }

    #[test]
    fn write_execute_is_exactly_writable_executable_and_present() {
        assert!((Flags::PRESENT | Flags::WRITABLE).is_write_execute());
        assert!(
            !(Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE).is_write_execute(),
            "no-execute is what makes a writable page safe"
        );
        assert!(
            !Flags::PRESENT.is_write_execute(),
            "read-only and executable is the other half of W^X"
        );
        assert!(
            !(Flags::WRITABLE).is_write_execute(),
            "an absent entry maps nothing, so it violates nothing"
        );
    }

    #[test]
    fn level_shifts_are_nine_bits_apart_above_the_page_offset() {
        assert_eq!(Level::Pt.shift(), 12);
        assert_eq!(Level::Pd.shift(), 21);
        assert_eq!(Level::Pdpt.shift(), 30);
        assert_eq!(Level::Pml4.shift(), 39);
    }

    #[test]
    fn a_level_spans_what_its_shift_says() {
        assert_eq!(Level::Pt.span(), PAGE_SIZE);
        assert_eq!(Level::Pd.span(), LARGE_PAGE_SIZE);
        assert_eq!(Level::Pdpt.span(), 1 << 30);
        assert_eq!(Level::Pml4.span(), 512 << 30);
    }

    #[test]
    fn indices_decompose_a_known_address() {
        // The kernel's own base. Worked by hand from the bit layout:
        // 0xffff_ffff_8000_0000 >> 39 = ...1ff, and the rest follow.
        let base = 0xffff_ffff_8000_0000;
        assert_eq!(Level::Pml4.index(base), 511);
        assert_eq!(Level::Pdpt.index(base), 510);
        assert_eq!(Level::Pd.index(base), 0);
        assert_eq!(Level::Pt.index(base), 0);
    }

    #[test]
    fn every_index_stays_inside_a_table() {
        for level in Level::WALK {
            assert_eq!(level.index(u64::MAX), ENTRIES - 1);
            assert_eq!(level.index(0), 0);
            // One past this level's span wraps to the next slot, never out.
            assert_eq!(level.index(level.span()), 1);
            assert_eq!(level.index(level.span() * ENTRIES as u64), 0);
        }
    }

    #[test]
    fn the_walk_descends_one_level_at_a_time() {
        assert_eq!(Level::WALK[0], Level::Pml4);
        assert_eq!(Level::Pml4.below(), Some(Level::Pdpt));
        assert_eq!(Level::Pdpt.below(), Some(Level::Pd));
        assert_eq!(Level::Pd.below(), Some(Level::Pt));
        assert_eq!(Level::Pt.below(), None);

        for pair in Level::WALK.windows(2) {
            assert_eq!(pair[0].below(), Some(pair[1]));
            assert!(pair[0] > pair[1], "the walk goes downward");
        }
    }

    #[test]
    fn only_the_top_level_cannot_map_a_page() {
        assert!(!Level::Pml4.can_map_page());
        assert!(Level::Pdpt.can_map_page());
        assert!(Level::Pd.can_map_page());
        assert!(Level::Pt.can_map_page());
    }

    #[test]
    fn canonical_addresses_are_sign_extended_from_bit_47() {
        assert!(is_canonical(0));
        assert!(is_canonical(0x0000_7fff_ffff_ffff), "top of the low half");
        assert!(is_canonical(0xffff_8000_0000_0000), "base of the high half");
        assert!(is_canonical(0xffff_ffff_8000_0000), "the kernel's own base");

        assert!(!is_canonical(0x0000_8000_0000_0000), "the hole below");
        assert!(!is_canonical(0xffff_7fff_ffff_ffff), "the hole above");
    }

    #[test]
    fn canonicalise_is_the_identity_on_canonical_addresses() {
        for address in [
            0,
            0x0000_7fff_ffff_ffff,
            0xffff_8000_0000_0000,
            0xffff_ffff_8000_0000,
            u64::MAX,
        ] {
            assert_eq!(canonicalise(address), address);
            assert!(is_canonical(canonicalise(address)));
        }
        // A 48-bit address with bit 47 set sign-extends into the high half.
        assert_eq!(canonicalise(0x0000_8000_0000_0000), 0xffff_8000_0000_0000);
    }

    #[test]
    fn a_huge_entry_is_distinguishable_from_a_table_pointer() {
        let huge = Entry::page(0x20_0000, Flags::PRESENT | Flags::HUGE).expect("aligned");
        let table = Entry::table(0x20_0000).expect("aligned");

        assert!(huge.is_huge());
        assert!(!table.is_huge());
        assert_eq!(
            huge.address(),
            table.address(),
            "same target, different kind"
        );
    }

    #[test]
    fn a_huge_entry_refuses_a_merely_page_aligned_address() {
        // Bit 12 of a huge entry selects the PAT, so 4 KiB alignment would
        // encode a different frame and a different memory type.
        assert_eq!(
            Entry::page(0x1000, Flags::PRESENT | Flags::HUGE),
            Err(EntryError::Misaligned(0x1000))
        );
        assert!(Entry::page(0x1000, Flags::PRESENT).is_ok());
    }

    #[test]
    fn debug_says_what_the_entry_is() {
        extern crate std;
        use std::format;

        assert_eq!(format!("{:?}", Entry::EMPTY), "Entry(absent)");
        let entry = Entry::page(0x1000, Flags::PRESENT | Flags::NO_EXECUTE).expect("aligned");
        assert_eq!(format!("{entry:?}"), "Entry(0x1000 present no-execute)");
        assert_eq!(format!("{:?}", Flags::NONE), "none");
    }
}
