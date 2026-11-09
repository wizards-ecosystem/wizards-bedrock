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
//! use x86_paging::{Entry, Flags, Level, MappingRange, PAGE_SIZE};
//!
//! // A read-only, executable mapping of physical 0x2000 — kernel text.
//! let text = Entry::page(Level::Pt, 0x2000, Flags::PRESENT)
//!     .expect("aligned and in range");
//! assert!(text.flags().contains(Flags::PRESENT));
//! assert!(!text.flags().contains(Flags::WRITABLE));
//! assert_eq!(text.address(), 0x2000);
//!
//! // The slot a virtual address takes in the top-level table.
//! assert_eq!(Level::Pml4.index(0xffff_ffff_8000_0000), 511);
//! assert_eq!(PAGE_SIZE, 4096);
//!
//! let range = MappingRange::new(0xffff_8000_0000_0000, 0, PAGE_SIZE).unwrap();
//! assert_eq!(range.len(), PAGE_SIZE);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

pub mod activation;
pub mod device;
pub mod kpti;

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

/// The first address that is not user memory; the canonical hole begins here.
pub const USER_END: u64 = 0x0000_8000_0000_0000;

/// The first address a user mapping may not cover.
///
/// The last canonical page stays unmapped, so no user instruction can end at
/// [`USER_END`]. A trap taken after such an instruction would save a
/// non-canonical return address, and returning to it faults in the kernel
/// (D-348).
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
    /// At a page-directory or page-directory-pointer level, this entry maps a
    /// large page directly instead of pointing at the table below.
    pub const HUGE: Self = Self(1 << 7);
    /// The mapping survives a `CR3` reload. For pages the kernel keeps in
    /// every address space.
    pub const GLOBAL: Self = Self(1 << 8);
    /// Instruction fetches from this mapping fault. Honoured only once
    /// `EFER.NXE` is set, which is why it is enabled before any table is
    /// built.
    pub const NO_EXECUTE: Self = Self(1 << 63);

    /// This mapping owns the frame beneath it, so unmapping returns that
    /// frame to the allocator (D-88).
    ///
    /// Bit 9 is one of the three the architecture leaves to software in a page
    /// entry (Intel SDM Vol. 3A ch. 4, the `AVL` bits); the CPU ignores it. It
    /// is provenance written where the mapping is, which is the only place
    /// that survives every path an unmap can be reached by — a frame the
    /// allocator handed out must come back, and one the firmware placed must
    /// not, and nothing outside the entry knows which is which.
    pub const OWNED: Self = Self(1 << 9);

    /// No flags at all.
    pub const NONE: Self = Self(0);

    /// The bits themselves.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Only the flags in both.
    ///
    /// What a page walk needs to compute an *effective* permission: the
    /// architecture grants `USER` and `WRITABLE` only where every level of the
    /// walk grants them, so the answer is a conjunction rather than the leaf's
    /// own opinion (D-103).
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Both sets of flags together.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The permission a walk has left after passing through `self`.
    ///
    /// The architecture computes a page's effective permission from the whole
    /// path, not from its leaf: `USER` and `WRITABLE` are the conjunction down
    /// the walk, and `NO_EXECUTE` the disjunction. A caller that reads the
    /// leaf alone states a stronger permission than the CPU will grant.
    ///
    /// `self` is what the walk has accumulated so far — start it at
    /// [`Flags::EVERYTHING_ALLOWED`] — and `entry` is the level just read.
    ///
    /// # Examples
    ///
    /// A supervisor table above a user page grants no user access, however
    /// the leaf is written:
    ///
    /// ```
    /// use x86_paging::Flags;
    ///
    /// let supervisor = Flags::PRESENT.union(Flags::WRITABLE);
    /// let leaf = Flags::PRESENT.union(Flags::WRITABLE).union(Flags::USER);
    /// let walked = Flags::EVERYTHING_ALLOWED.through(supervisor).through(leaf);
    /// assert!(!walked.effective_for(leaf).contains(Flags::USER));
    /// ```
    ///
    /// And a `NO_EXECUTE` anywhere on the path forbids execution below it:
    ///
    /// ```
    /// use x86_paging::Flags;
    ///
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

    /// Fold an accumulated walk back onto the leaf's own flags.
    ///
    /// The leaf keeps everything the walk does not narrow — its address bits'
    /// companions, `PRESENT`, `HUGE`, `OWNED` — and gives up `USER` and
    /// `WRITABLE` to the path, and gains `NO_EXECUTE` from it.
    #[must_use]
    pub const fn effective_for(self, leaf: Self) -> Self {
        let kept = leaf.difference(Self::NARROWED).difference(Self::NO_EXECUTE);
        kept.union(self.intersection(Self::NARROWED))
            .union(self.intersection(Self::NO_EXECUTE))
    }

    /// The permissions a walk narrows rather than reads off the leaf.
    pub const NARROWED: Self = Self::USER.union(Self::WRITABLE);

    /// What a walk starts with, before any entry has narrowed it.
    pub const EVERYTHING_ALLOWED: Self = Self::NARROWED;

    /// This set without the flags in `other`.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Whether every flag in `other` is present here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
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

/// Whether one walk found the exact expected four-kilobyte leaf.
///
/// Accessed and dirty are omitted from identity because hardware may set them
/// without changing the mapping's physical target or policy.
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
                core::fmt::Write::write_str(f, separator)?;
                core::fmt::Write::write_str(f, name)?;
                separator = " ";
            }
        }
        if separator.is_empty() {
            core::fmt::Write::write_str(f, "none")?;
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
                core::fmt::Write::write_str(f, "the huge-page flag is derived from the leaf level")
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

    /// An entry that maps nothing.
    ///
    /// What an unmapped slot holds: all zero, so `PRESENT` is clear and every
    /// other bit with it. Writing this is how a mapping is taken away (D-88).
    pub const ABSENT: Self = Self(0);

    /// A leaf entry at `level` mapping a page at `address` with `flags`.
    ///
    /// `PRESENT` is not added for you: an entry whose caller forgot it is a
    /// mapping that silently does not exist, and that is worth a compile-time
    /// habit of naming it.
    ///
    /// # Errors
    ///
    /// [`EntryError::CannotMapPage`] for [`Level::Pml4`],
    /// [`EntryError::HugeFlagIsDerived`] if `flags` includes [`Flags::HUGE`],
    /// [`EntryError::Misaligned`] unless `address` is aligned to the span of
    /// `level`, and [`EntryError::OutOfRange`] if it needs more than 52 bits.
    /// The huge-page bit is added for [`Level::Pd`] and [`Level::Pdpt`] and
    /// omitted for [`Level::Pt`].
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

    /// An entry pointing at the table below.
    ///
    /// Intermediate entries carry `PRESENT | WRITABLE` and never `NO_EXECUTE`.
    /// A [`TablePrivilege::User`] entry also carries [`Flags::USER`]; otherwise
    /// a parent would silently make every user leaf beneath it supervisor-only.
    /// The effective permission of a mapping is the intersection down the
    /// walk, so W^X remains enforced at the leaves.
    ///
    /// # Errors
    ///
    /// [`EntryError::Misaligned`] unless `address` is page-aligned, and
    /// [`EntryError::OutOfRange`] if it needs more than 52 bits.
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

    /// Whether the entry maps a large page rather than naming a table.
    ///
    /// Meaningful only at [`Level::Pd`] and [`Level::Pdpt`]; the bit means
    /// something else entirely at the other two.
    pub const fn is_huge(self) -> bool {
        self.flags().contains(Flags::HUGE)
    }

    /// Whether another intermediate entry names the same table with the same
    /// software-selected policy.
    ///
    /// Hardware may set [`Flags::ACCESSED`] during a page walk. Every other
    /// bit, including the physical address, remains part of the identity.
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
            return core::fmt::Write::write_str(f, "Entry(absent)");
        }
        write!(f, "Entry({:#x} {:?})", self.address(), self.flags())
    }
}

/// Whether an intermediate table participates in supervisor or user mappings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TablePrivilege {
    /// Every leaf below this table is supervisor-only.
    Supervisor,
    /// Leaves below this table may grant ring-3 access with [`Flags::USER`].
    User,
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

/// A virtual address whose upper bits are the sign extension of bit 47.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalAddress(u64);

impl CanonicalAddress {
    /// The canonical null address.
    pub const ZERO: Self = Self(0);

    /// Admit a value the processor accepts as a virtual address.
    pub const fn new(address: u64) -> Option<Self> {
        if is_canonical(address) {
            Some(Self(address))
        } else {
            None
        }
    }

    /// Return the admitted address.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A mapping range whose complete arithmetic has been checked up front.
///
/// Construct this before allocating or writing page tables. Every address the
/// range can produce is page-aligned, canonical on the virtual side, and
/// encodable in an x86_64 entry on the physical side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappingRange {
    virtual_base: u64,
    physical_base: u64,
    len: u64,
}

/// Why a complete mapping range cannot be represented safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MappingRangeError {
    /// An empty request does not describe a mapping.
    ZeroLength,
    /// The virtual base is not page-aligned.
    MisalignedVirtualBase(u64),
    /// The physical base is not page-aligned.
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
            Self::ZeroLength => core::fmt::Write::write_str(f, "mapping length is zero"),
            Self::MisalignedVirtualBase(address) => {
                write!(f, "virtual base {address:#x} is not page-aligned")
            }
            Self::MisalignedPhysicalBase(address) => {
                write!(f, "physical base {address:#x} is not page-aligned")
            }
            Self::MisalignedLength(len) => {
                write!(f, "mapping length {len} is not page-aligned")
            }
            Self::VirtualEndOverflow => {
                core::fmt::Write::write_str(f, "virtual mapping end overflows")
            }
            Self::PhysicalEndOverflow => {
                core::fmt::Write::write_str(f, "physical mapping end overflows")
            }
            Self::NonCanonicalVirtual(address) => {
                write!(f, "virtual address {address:#x} is not canonical")
            }
            Self::CrossesCanonicalHole => {
                core::fmt::Write::write_str(f, "virtual mapping crosses the noncanonical hole")
            }
            Self::PhysicalOutOfRange(address) => {
                write!(f, "physical address {address:#x} does not fit 52 bits")
            }
        }
    }
}

impl core::error::Error for MappingRangeError {}

impl MappingRange {
    /// Preflight a virtual-to-physical range before mutating page tables.
    ///
    /// # Errors
    ///
    /// Returns a [`MappingRangeError`] unless `len` is non-zero and all three
    /// values describe whole pages, both inclusive ends can be calculated,
    /// the virtual range stays within one canonical half, and the physical
    /// range fits the entry encoding.
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

    /// First virtual address in the range.
    pub const fn virtual_base(self) -> u64 {
        self.virtual_base
    }

    /// First physical address in the range.
    pub const fn physical_base(self) -> u64 {
        self.physical_base
    }

    /// Bytes in the range.
    #[expect(
        clippy::len_without_is_empty,
        reason = "construction refuses zero-length mapping ranges"
    )]
    pub const fn len(self) -> u64 {
        self.len
    }
}

#[cfg(test)]
mod tests;
