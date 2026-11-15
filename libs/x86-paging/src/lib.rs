#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod activation;
pub mod device;
pub mod kpti;

use core::fmt;

pub const PAGE_SIZE: u64 = 4096;

pub const LARGE_PAGE_SIZE: u64 = 2 << 20;

pub const ENTRIES: usize = 512;

const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

pub const MAX_PHYSICAL: u64 = ADDRESS_MASK + PAGE_SIZE;

pub const USER_END: u64 = 0x0000_8000_0000_0000;

// The last canonical page stays unmapped: a trap saving a RIP of `USER_END` would #GP on return.
pub const USER_MAPPING_END: u64 = USER_END - PAGE_SIZE;

pub const fn user_mapping_fits(at: u64, length: u64) -> bool {
    match at.checked_add(length) {
        Some(end) => end <= USER_MAPPING_END,
        None => false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags(u64);

impl Flags {
    pub const PRESENT: Self = Self(1 << 0);
    pub const WRITABLE: Self = Self(1 << 1);
    pub const USER: Self = Self(1 << 2);
    pub const WRITE_THROUGH: Self = Self(1 << 3);
    pub const NO_CACHE: Self = Self(1 << 4);
    pub const ACCESSED: Self = Self(1 << 5);
    const DIRTY: Self = Self(1 << 6);
    pub const HUGE: Self = Self(1 << 7);
    pub const GLOBAL: Self = Self(1 << 8);
    // Fetches fault only once `EFER.NXE` is set, so NXE is enabled before any table.
    pub const NO_EXECUTE: Self = Self(1 << 63);

    // Set only on allocator frames: unmap frees them, and the entry is their only provenance.
    pub const OWNED: Self = Self(1 << 9);

    pub const NONE: Self = Self(0);

    const NARROWED: Self = Self::USER.union(Self::WRITABLE);

    pub const EVERYTHING_ALLOWED: Self = Self::NARROWED;

    pub const fn bits(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn through(self, entry: Self) -> Self {
        let narrowed = self.intersection(entry).intersection(Self::NARROWED);
        let forbidden = self
            .intersection(Self::NO_EXECUTE)
            .union(entry.intersection(Self::NO_EXECUTE));
        narrowed.union(forbidden)
    }

    #[must_use]
    pub const fn effective_for(self, leaf: Self) -> Self {
        let kept = leaf.difference(Self::NARROWED).difference(Self::NO_EXECUTE);
        kept.union(self.intersection(Self::NARROWED))
            .union(self.intersection(Self::NO_EXECUTE))
    }

    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
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
                formatter.write_str(separator)?;
                formatter.write_str(name)?;
                separator = " ";
            }
        }
        if separator.is_empty() {
            formatter.write_str("none")?;
        }
        Ok(())
    }
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryError {
    Misaligned(u64),
    OutOfRange(u64),
    CannotMapPage(Level),
    HugeFlagIsDerived,
}

impl fmt::Display for EntryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Misaligned(address) => write!(formatter, "{address:#x} is not page-aligned"),
            Self::OutOfRange(address) => write!(formatter, "{address:#x} does not fit 52 bits"),
            Self::CannotMapPage(level) => write!(formatter, "{level:?} cannot map a page"),
            Self::HugeFlagIsDerived => {
                formatter.write_str("the huge-page flag is derived from the leaf level")
            }
        }
    }
}

impl core::error::Error for EntryError {}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct Entry(u64);

impl Entry {
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const ABSENT: Self = Self(0);

    // `PRESENT` is never added; `HUGE` follows the level.
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

    // Never `NO_EXECUTE`: W^X is decided at the leaves.
    pub const fn table(address: u64, privilege: TablePrivilege) -> Result<Self, EntryError> {
        let mut flags = Flags::PRESENT.union(Flags::WRITABLE);
        if matches!(privilege, TablePrivilege::User) {
            flags = flags.union(Flags::USER);
        }
        Self::encode(address, PAGE_SIZE, flags)
    }

    pub const fn address(self) -> u64 {
        self.0 & ADDRESS_MASK
    }

    pub const fn flags(self) -> Flags {
        Flags(self.0 & !ADDRESS_MASK)
    }

    pub const fn is_present(self) -> bool {
        self.flags().contains(Flags::PRESENT)
    }

    // Meaningful only at `Level::Pd` and `Level::Pdpt`.
    pub const fn is_huge(self) -> bool {
        self.flags().contains(Flags::HUGE)
    }

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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.is_present() {
            return formatter.write_str("Entry(absent)");
        }
        write!(formatter, "Entry({:#x} {:?})", self.address(), self.flags())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TablePrivilege {
    Supervisor,
    User,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Pt = 1,
    Pd = 2,
    Pdpt = 3,
    Pml4 = 4,
}

impl Level {
    pub const WALK: [Self; 4] = [Self::Pml4, Self::Pdpt, Self::Pd, Self::Pt];

    pub const fn shift(self) -> u32 {
        12 + 9 * (self as u32 - 1)
    }

    pub const fn span(self) -> u64 {
        1 << self.shift()
    }

    pub const fn index(self, address: u64) -> usize {
        (address >> self.shift()) as usize % ENTRIES
    }

    const fn can_map_page(self) -> bool {
        !matches!(self, Self::Pml4)
    }
}

pub const fn is_canonical(address: u64) -> bool {
    let sign_extended = ((address as i64) << 16 >> 16) as u64;
    sign_extended == address
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MappingRange {
    virtual_base: u64,
    physical_base: u64,
    len: u64,
}

impl MappingRange {
    pub const fn new(
        virtual_base: u64,
        physical_base: u64,
        length: u64,
    ) -> Result<Self, MappingRangeError> {
        if length == 0 {
            return Err(MappingRangeError::ZeroLength);
        }
        if !virtual_base.is_multiple_of(PAGE_SIZE) {
            return Err(MappingRangeError::MisalignedVirtualBase(virtual_base));
        }
        if !physical_base.is_multiple_of(PAGE_SIZE) {
            return Err(MappingRangeError::MisalignedPhysicalBase(physical_base));
        }
        if !length.is_multiple_of(PAGE_SIZE) {
            return Err(MappingRangeError::MisalignedLength(length));
        }

        let Some(last_virtual) = virtual_base.checked_add(length - 1) else {
            return Err(MappingRangeError::VirtualEndOverflow);
        };
        let Some(last_physical) = physical_base.checked_add(length - 1) else {
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
            len: length,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MappingRangeError {
    ZeroLength,
    MisalignedVirtualBase(u64),
    MisalignedPhysicalBase(u64),
    MisalignedLength(u64),
    VirtualEndOverflow,
    PhysicalEndOverflow,
    NonCanonicalVirtual(u64),
    CrossesCanonicalHole,
    PhysicalOutOfRange(u64),
}

impl fmt::Display for MappingRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ZeroLength => formatter.write_str("mapping length is zero"),
            Self::MisalignedVirtualBase(address) => {
                write!(formatter, "virtual base {address:#x} is not page-aligned")
            }
            Self::MisalignedPhysicalBase(address) => {
                write!(formatter, "physical base {address:#x} is not page-aligned")
            }
            Self::MisalignedLength(length) => {
                write!(formatter, "mapping length {length} is not page-aligned")
            }
            Self::VirtualEndOverflow => formatter.write_str("virtual mapping end overflows"),
            Self::PhysicalEndOverflow => formatter.write_str("physical mapping end overflows"),
            Self::NonCanonicalVirtual(address) => {
                write!(formatter, "virtual address {address:#x} is not canonical")
            }
            Self::CrossesCanonicalHole => {
                formatter.write_str("virtual mapping crosses the noncanonical hole")
            }
            Self::PhysicalOutOfRange(address) => {
                write!(
                    formatter,
                    "physical address {address:#x} does not fit 52 bits"
                )
            }
        }
    }
}

impl core::error::Error for MappingRangeError {}

#[cfg(test)]
mod tests;
