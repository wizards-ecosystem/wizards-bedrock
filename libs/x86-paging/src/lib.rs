//! x86_64 four-level page table encoding, from Intel SDM Vol. 3A §4.5.
//!
//! ```
//! use x86_paging::{Entry, Flags, Level, MappingRange, PAGE_SIZE};
//!
//! // A read-only, executable mapping of physical 0x2000, as for kernel text.
//! let text = Entry::page(Level::Pt, 0x2000, Flags::PRESENT)
//!     .expect("aligned and in range");
//! assert!(text.flags().contains(Flags::PRESENT));
//! assert!(!text.flags().contains(Flags::WRITABLE));
//! assert_eq!(text.address(), 0x2000);
//!
//! assert_eq!(Level::Pml4.index(0xffff_ffff_8000_0000), 511);
//! assert_eq!(PAGE_SIZE, 4096);
//!
//! let range = MappingRange::new(0xffff_8000_0000_0000, 0, PAGE_SIZE).unwrap();
//! assert_eq!(range.len(), PAGE_SIZE);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod activation;
pub mod device;
pub mod kpti;

use core::fmt;

/// Bytes in the smallest page.
pub const PAGE_SIZE: u64 = 4096;

/// Bytes in a large page, mapped by a single page-directory entry.
pub const LARGE_PAGE_SIZE: u64 = 2 << 20;

/// Entries in every table at every level.
pub const ENTRIES: usize = 512;

/// Bits 51:12 of an entry, which hold a page-aligned physical address.
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

/// Physical addresses this encoding can represent.
pub const MAX_PHYSICAL: u64 = ADDRESS_MASK + PAGE_SIZE;

/// The first address that is not user memory; the canonical hole begins here.
pub const USER_END: u64 = 0x0000_8000_0000_0000;

/// The first address a user mapping may not cover, which keeps the last canonical page unmapped.
/// No trap can then save a RIP of [`USER_END`], whose return would #GP in ring 0.
pub const USER_MAPPING_END: u64 = USER_END - PAGE_SIZE;

/// Whether `len` bytes at `at` lie wholly below [`USER_MAPPING_END`].
///
/// ```
/// use x86_paging::{PAGE_SIZE, USER_MAPPING_END, user_mapping_fits};
///
/// assert!(user_mapping_fits(USER_MAPPING_END - PAGE_SIZE, PAGE_SIZE));
/// assert!(!user_mapping_fits(USER_MAPPING_END, PAGE_SIZE));
/// assert!(!user_mapping_fits(u64::MAX, 2));
/// ```
pub const fn user_mapping_fits(at: u64, len: u64) -> bool {
    match at.checked_add(len) {
        Some(end) => end <= USER_MAPPING_END,
        None => false,
    }
}

/// Flags of a page table entry, hand-rolled to keep a `bitflags` crate out of the trusted path.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags(u64);

impl Flags {
    /// The entry is valid; without it the CPU ignores every other bit.
    pub const PRESENT: Self = Self(1 << 0);
    pub const WRITABLE: Self = Self(1 << 1);
    /// Ring 3 may reach it.
    pub const USER: Self = Self(1 << 2);
    pub const WRITE_THROUGH: Self = Self(1 << 3);
    /// The region is not cached, as device memory needs.
    pub const NO_CACHE: Self = Self(1 << 4);
    /// The CPU has read through this entry since it was last cleared.
    pub const ACCESSED: Self = Self(1 << 5);
    /// The CPU has written through this entry since it was last cleared.
    pub const DIRTY: Self = Self(1 << 6);
    /// At a page-directory or page-directory-pointer level, the entry maps a large page.
    pub const HUGE: Self = Self(1 << 7);
    /// The mapping survives a `CR3` reload.
    pub const GLOBAL: Self = Self(1 << 8);
    /// Instruction fetches fault once `EFER.NXE` is set, so NXE is enabled before any table.
    pub const NO_EXECUTE: Self = Self(1 << 63);

    /// Software bit 9 (an SDM `AVL` bit): the mapping owns its frame, so unmapping returns it.
    /// Set it only on allocator frames: unmap frees them, and the entry is their only provenance.
    pub const OWNED: Self = Self(1 << 9);

    pub const NONE: Self = Self(0);

    /// The permissions a walk narrows rather than reads off the leaf.
    pub const NARROWED: Self = Self::USER.union(Self::WRITABLE);

    /// What a walk starts with, before any entry has narrowed it.
    pub const EVERYTHING_ALLOWED: Self = Self::NARROWED;

    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Only the flags in both.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Both sets of flags together.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The walk permission after `entry`: `USER` and `WRITABLE` narrow, `NO_EXECUTE` accumulates.
    ///
    /// # Examples
    ///
    /// ```
    /// use x86_paging::Flags;
    ///
    /// // A supervisor table above a user page grants no user access.
    /// let supervisor = Flags::PRESENT.union(Flags::WRITABLE);
    /// let leaf = Flags::PRESENT.union(Flags::WRITABLE).union(Flags::USER);
    /// let walked = Flags::EVERYTHING_ALLOWED.through(supervisor).through(leaf);
    /// assert!(!walked.effective_for(leaf).contains(Flags::USER));
    /// ```
    ///
    /// ```
    /// use x86_paging::Flags;
    ///
    /// // A `NO_EXECUTE` anywhere on the path forbids execution below it.
    /// let no_execute = Flags::PRESENT.union(Flags::WRITABLE).union(Flags::NO_EXECUTE);
    /// let leaf = Flags::PRESENT.union(Flags::WRITABLE);
    /// let walked = Flags::EVERYTHING_ALLOWED.through(no_execute).through(leaf);
    /// assert!(walked.effective_for(leaf).contains(Flags::NO_EXECUTE));
    /// ```
    #[must_use]
    pub const fn through(self, entry: Self) -> Self {
        let narrowed = self.intersection(entry).intersection(Self::NARROWED);
        let forbidden = self
            .intersection(Self::NO_EXECUTE)
            .union(entry.intersection(Self::NO_EXECUTE));
        narrowed.union(forbidden)
    }

    /// Fold a walk onto the leaf, which takes `USER`, `WRITABLE` and `NO_EXECUTE` from the path.
    #[must_use]
    pub const fn effective_for(self, leaf: Self) -> Self {
        let kept = leaf.difference(Self::NARROWED).difference(Self::NO_EXECUTE);
        kept.union(self.intersection(Self::NARROWED))
            .union(self.intersection(Self::NO_EXECUTE))
    }

    /// This set without the flags in `other`.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Whether every flag in `other` is present here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether this mapping is both writable and executable, the one definition W^X forbids.
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
            (Self::OWNED, "owned"),
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

/// Whether a walk found the expected 4 KiB leaf, ignoring the hardware-set accessed and dirty bits.
pub const fn page_identity_matches(
    mapped: u64,
    flags: Flags,
    span: u64,
    expected: u64,
    expected_flags: Flags,
) -> bool {
    let volatile = Flags::ACCESSED.union(Flags::DIRTY);
    mapped == expected
        && span == PAGE_SIZE
        && flags.difference(volatile).bits() == expected_flags.difference(volatile).bits()
}

/// Why an entry could not be encoded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum EntryError {
    /// The physical address is not page-aligned.
    Misaligned(u64),
    /// The physical address has bits above the 52 the encoding carries.
    OutOfRange(u64),
    /// This table level cannot contain a leaf mapping.
    CannotMapPage(Level),
    /// [`Flags::HUGE`] is derived from the leaf level rather than supplied.
    HugeFlagIsDerived,
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Misaligned(address) => write!(f, "{address:#x} is not page-aligned"),
            Self::OutOfRange(address) => write!(f, "{address:#x} does not fit 52 bits"),
            Self::CannotMapPage(level) => write!(f, "{level:?} cannot map a page"),
            Self::HugeFlagIsDerived => {
                f.write_str("the huge-page flag is derived from the leaf level")
            }
        }
    }
}

impl core::error::Error for EntryError {}

/// One page table entry.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct Entry(u64);

impl Entry {
    /// Rebuild an entry from the word a table holds.
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The word to store in a table.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// An all-zero entry that maps nothing; writing it takes a mapping away.
    pub const ABSENT: Self = Self(0);

    /// A leaf at `level` mapping `address`; `PRESENT` is never added and `HUGE` follows the level.
    pub const fn page(level: Level, address: u64, flags: Flags) -> Result<Self, EntryError> {
        if !level.can_map_page() {
            return Err(EntryError::CannotMapPage(level));
        }
        if flags.contains(Flags::HUGE) {
            return Err(EntryError::HugeFlagIsDerived);
        }
        let flags = match level {
            Level::Pd | Level::Pdpt => flags.union(Flags::HUGE),
            Level::Pt => flags,
            Level::Pml4 => return Err(EntryError::CannotMapPage(level)),
        };
        Self::encode(address, level.span(), flags)
    }

    /// A `PRESENT | WRITABLE`, never `NO_EXECUTE` table entry; W^X is decided at the leaves.
    /// It carries [`Flags::USER`] only for user privilege, so user leaves below stay usable.
    pub const fn table(address: u64, privilege: TablePrivilege) -> Result<Self, EntryError> {
        let mut flags = Flags::PRESENT.union(Flags::WRITABLE);
        if matches!(privilege, TablePrivilege::User) {
            flags = flags.union(Flags::USER);
        }
        Self::encode(address, PAGE_SIZE, flags)
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

    /// Whether the entry maps a large page; meaningful only at [`Level::Pd`] and [`Level::Pdpt`].
    pub const fn is_huge(self) -> bool {
        self.flags().contains(Flags::HUGE)
    }

    /// Whether `found` names the same table with the same policy, apart from hardware `ACCESSED`.
    pub const fn table_identity_matches(self, found: Self) -> bool {
        self.address() == found.address()
            && self.flags().difference(Flags::ACCESSED).bits()
                == found.flags().difference(Flags::ACCESSED).bits()
    }

    const fn encode(address: u64, alignment: u64, flags: Flags) -> Result<Self, EntryError> {
        if !address.is_multiple_of(alignment) {
            return Err(EntryError::Misaligned(address));
        }
        if address & !ADDRESS_MASK != 0 {
            return Err(EntryError::OutOfRange(address));
        }
        Ok(Self(address | flags.bits()))
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

/// Whether an intermediate table participates in supervisor or user mappings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TablePrivilege {
    /// Every leaf below this table is supervisor-only.
    Supervisor,
    /// Leaves below this table may grant ring-3 access with [`Flags::USER`].
    User,
}

/// A level of the four-level page table walk.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    /// The page table: entries map 4 KiB pages.
    Pt = 1,
    /// The page directory: entries map 2 MiB pages or name a [`Pt`](Self::Pt).
    Pd = 2,
    /// The page-directory-pointer table: entries map 1 GiB pages or name a [`Pd`](Self::Pd).
    Pdpt = 3,
    /// The top-level table, whose physical address goes in `CR3`.
    Pml4 = 4,
}

impl Level {
    /// Every level in the order a walk visits them.
    pub const WALK: [Self; 4] = [Self::Pml4, Self::Pdpt, Self::Pd, Self::Pt];

    /// The lowest virtual-address bit this level's index occupies: 12, 21, 30 or 39.
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

    /// Whether entries may map a page directly, which a [`Pml4`](Self::Pml4) entry never does.
    pub const fn can_map_page(self) -> bool {
        !matches!(self, Self::Pml4)
    }
}

/// Whether bits 63:48 of `address` all copy bit 47; the CPU faults on any other address.
pub const fn is_canonical(address: u64) -> bool {
    let sign_extended = ((address as i64) << 16 >> 16) as u64;
    sign_extended == address
}

/// A virtual address whose upper bits are the sign extension of bit 47.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CanonicalAddress(u64);

impl CanonicalAddress {
    pub const ZERO: Self = Self(0);

    pub const fn new(address: u64) -> Option<Self> {
        if is_canonical(address) {
            Some(Self(address))
        } else {
            None
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A mapping range whose alignment, canonical form and encodability are checked up front.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MappingRange {
    virtual_base: u64,
    physical_base: u64,
    len: u64,
}

impl MappingRange {
    /// Preflight a virtual-to-physical range before any page table is allocated or written.
    pub const fn new(
        virtual_base: u64,
        physical_base: u64,
        len: u64,
    ) -> Result<Self, MappingRangeError> {
        if len == 0 {
            return Err(MappingRangeError::ZeroLength);
        }
        if !virtual_base.is_multiple_of(PAGE_SIZE) {
            return Err(MappingRangeError::MisalignedVirtualBase(virtual_base));
        }
        if !physical_base.is_multiple_of(PAGE_SIZE) {
            return Err(MappingRangeError::MisalignedPhysicalBase(physical_base));
        }
        if !len.is_multiple_of(PAGE_SIZE) {
            return Err(MappingRangeError::MisalignedLength(len));
        }

        let Some(last_virtual) = virtual_base.checked_add(len - 1) else {
            return Err(MappingRangeError::VirtualEndOverflow);
        };
        let Some(last_physical) = physical_base.checked_add(len - 1) else {
            return Err(MappingRangeError::PhysicalEndOverflow);
        };
        if !is_canonical(virtual_base) {
            return Err(MappingRangeError::NonCanonicalVirtual(virtual_base));
        }
        if !is_canonical(last_virtual) {
            return Err(MappingRangeError::NonCanonicalVirtual(last_virtual));
        }
        if (virtual_base ^ last_virtual) & (1 << 47) != 0 {
            return Err(MappingRangeError::CrossesCanonicalHole);
        }
        if physical_base >= MAX_PHYSICAL {
            return Err(MappingRangeError::PhysicalOutOfRange(physical_base));
        }
        if last_physical >= MAX_PHYSICAL {
            return Err(MappingRangeError::PhysicalOutOfRange(last_physical));
        }

        Ok(Self {
            virtual_base,
            physical_base,
            len,
        })
    }

    pub const fn virtual_base(self) -> u64 {
        self.virtual_base
    }

    pub const fn physical_base(self) -> u64 {
        self.physical_base
    }

    #[expect(
        clippy::len_without_is_empty,
        reason = "construction refuses zero-length mapping ranges"
    )]
    pub const fn len(self) -> u64 {
        self.len
    }
}

/// Why a complete mapping range cannot be represented safely.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum MappingRangeError {
    ZeroLength,
    MisalignedVirtualBase(u64),
    MisalignedPhysicalBase(u64),
    /// The length is not a whole number of pages.
    MisalignedLength(u64),
    /// Computing the last virtual byte would wrap `u64`.
    VirtualEndOverflow,
    /// Computing the last physical byte would wrap `u64`.
    PhysicalEndOverflow,
    /// A virtual endpoint is not canonical.
    NonCanonicalVirtual(u64),
    /// Canonical endpoints lie on opposite sides of the noncanonical hole.
    CrossesCanonicalHole,
    /// A physical address needs bits outside the entry's address field.
    PhysicalOutOfRange(u64),
}

impl fmt::Display for MappingRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ZeroLength => f.write_str("mapping length is zero"),
            Self::MisalignedVirtualBase(address) => {
                write!(f, "virtual base {address:#x} is not page-aligned")
            }
            Self::MisalignedPhysicalBase(address) => {
                write!(f, "physical base {address:#x} is not page-aligned")
            }
            Self::MisalignedLength(len) => {
                write!(f, "mapping length {len} is not page-aligned")
            }
            Self::VirtualEndOverflow => f.write_str("virtual mapping end overflows"),
            Self::PhysicalEndOverflow => f.write_str("physical mapping end overflows"),
            Self::NonCanonicalVirtual(address) => {
                write!(f, "virtual address {address:#x} is not canonical")
            }
            Self::CrossesCanonicalHole => {
                f.write_str("virtual mapping crosses the noncanonical hole")
            }
            Self::PhysicalOutOfRange(address) => {
                write!(f, "physical address {address:#x} does not fit 52 bits")
            }
        }
    }
}

impl core::error::Error for MappingRangeError {}

#[cfg(test)]
mod tests;
