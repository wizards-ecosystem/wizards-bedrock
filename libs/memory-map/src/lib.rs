//! A complete bounded physical memory map.
//!
//! The map is built before an allocator exists, so its storage is fixed. A
//! fixed bound is safe only when overflow refuses the whole handoff rather
//! than returning a useful-looking prefix.
//!
//! # Examples
//!
//! ```
//! use memory_map::{MemoryKind, MemoryMap, MemoryRegion};
//!
//! let regions = [MemoryRegion {
//!     base: 0x1000,
//!     len: 0x4000,
//!     kind: MemoryKind::Usable,
//! }];
//! let map = MemoryMap::try_from_regions(regions.into_iter()).expect("complete map");
//! assert_eq!(map.allocatable_bytes(), 0x4000);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

/// Bytes in one page on the target architecture.
const PAGE_SIZE: u64 = 4096;

/// Regions an allocation-free [`MemoryMap`] can hold.
pub const MAX_REGIONS: usize = 128;

/// What a region of the physical address space is for.
///
/// Named after what the kernel may do with a region rather than after any one
/// bootloader's constants. [`MemoryKind::Unknown`] preserves a foreign value
/// so it can be reported and treated as reserved rather than silently folded
/// into a known kind (D-42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MemoryKind {
    /// Free RAM, available to the frame allocator.
    Usable,
    /// The bootloader's own structures, reclaimable once nothing refers to them.
    BootloaderReclaimable,
    /// The kernel image and modules loaded with it.
    KernelAndModules,
    /// A framebuffer.
    Framebuffer,
    /// ACPI tables that may eventually be reclaimed.
    AcpiReclaimable,
    /// ACPI non-volatile storage, which must survive.
    AcpiNvs,
    /// Reserved by firmware and not available to the kernel.
    Reserved,
    /// Known-bad memory.
    BadMemory,
    /// A type the kernel does not recognise, carrying its raw value.
    Unknown(u64),
}

impl MemoryKind {
    /// Whether the frame allocator may hand this region out.
    ///
    /// Reclaimable kinds are excluded because their live structures still
    /// have users when the allocator is built.
    pub const fn is_allocatable(self) -> bool {
        matches!(self, Self::Usable)
    }

    /// Whether immutable firmware-table reads may reach this region.
    pub const fn is_firmware_table(self) -> bool {
        matches!(self, Self::AcpiReclaimable | Self::AcpiNvs)
    }

    /// Whether the kernel's direct map should cover this region.
    ///
    /// Reserved ranges may be device holes and must not gain the direct map's
    /// cache type. Bootloader, image, framebuffer and firmware-table memory
    /// stay included because the kernel is still using them when it changes
    /// page tables (D-45).
    pub const fn is_direct_mapped(self) -> bool {
        matches!(
            self,
            Self::Usable
                | Self::BootloaderReclaimable
                | Self::KernelAndModules
                | Self::Framebuffer
                | Self::AcpiReclaimable
                | Self::AcpiNvs
        )
    }
}

impl fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Usable => f.write_str("usable"),
            Self::BootloaderReclaimable => f.write_str("bootloader reclaimable"),
            Self::KernelAndModules => f.write_str("kernel and modules"),
            Self::Framebuffer => f.write_str("framebuffer"),
            Self::AcpiReclaimable => f.write_str("acpi reclaimable"),
            Self::AcpiNvs => f.write_str("acpi nvs"),
            Self::Reserved => f.write_str("reserved"),
            Self::BadMemory => f.write_str("bad memory"),
            Self::Unknown(value) => write!(f, "unknown type {value}"),
        }
    }
}

/// One region of the physical address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegion {
    /// First physical byte.
    pub base: u64,
    /// Length in bytes.
    pub len: u64,
    /// What the region is for.
    pub kind: MemoryKind,
}

/// Why no complete bounded map could be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MemoryMapError {
    /// The producer described no regions.
    Empty,
    /// The producer described more regions than fixed early storage can hold.
    TooMany {
        /// Regions in the complete input.
        reported: usize,
        /// Regions the allocation-free representation can hold.
        capacity: usize,
    },
}

impl fmt::Display for MemoryMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty => f.write_str("the map is empty"),
            Self::TooMany { reported, capacity } => {
                write!(
                    f,
                    "the map has {reported} regions but storage holds {capacity}"
                )
            }
        }
    }
}

impl core::error::Error for MemoryMapError {}

/// A complete physical memory map copied into fixed early storage.
///
/// Copying releases the bootloader-owned source structures for later reclaim.
/// The fixed representation exists before heap initialization, and its
/// constructor never returns a partial prefix.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MemoryMap {
    regions: [MemoryRegion; MAX_REGIONS],
    len: usize,
}

impl MemoryMap {
    /// Copy a complete region stream or refuse it whole.
    ///
    /// # Errors
    ///
    /// [`MemoryMapError::Empty`] when `regions` is empty, or
    /// [`MemoryMapError::TooMany`] without exposing the copied prefix when it
    /// exceeds [`MAX_REGIONS`].
    pub fn try_from_regions(
        regions: impl IntoIterator<Item = MemoryRegion>,
    ) -> Result<Self, MemoryMapError> {
        let mut stored = [MemoryRegion {
            base: 0,
            len: 0,
            kind: MemoryKind::Reserved,
        }; MAX_REGIONS];
        let mut reported = 0usize;
        let mut overflow = false;
        for region in regions {
            if let Some(slot) = stored.get_mut(reported) {
                *slot = region;
            } else {
                overflow = true;
            }
            reported = reported.saturating_add(1);
        }
        if reported == 0 {
            return Err(MemoryMapError::Empty);
        }
        if overflow {
            return Err(MemoryMapError::TooMany {
                reported,
                capacity: MAX_REGIONS,
            });
        }
        Ok(Self {
            regions: stored,
            len: reported,
        })
    }

    /// Every region in the complete input, in its original order.
    pub fn regions(&self) -> impl Iterator<Item = MemoryRegion> + '_ {
        self.regions[..self.len].iter().copied()
    }

    /// Whether every byte of `start..end` lies inside a described region.
    ///
    /// Adjacent regions jointly cover a range. Input order is not assumed, so
    /// each bounded step searches the complete map.
    pub fn covers(&self, start: u64, end: u64) -> bool {
        self.covered_by(start, end, |_| true)
    }

    /// Whether every byte of `start..end` lies inside described memory.
    ///
    /// A reserved device hole is described but is not memory; this distinction
    /// prevents a cacheable alias from being granted to device registers.
    pub fn covers_memory(&self, start: u64, end: u64) -> bool {
        self.covered_by(start, end, MemoryKind::is_direct_mapped)
    }

    /// Find a page-aligned, nonzero memory run of at least `len` bytes.
    ///
    /// Callers use this rather than assuming a conventional low page exists,
    /// which keeps machine checks meaningful across firmware layouts (D-177).
    pub fn first_memory_run(&self, len: u64) -> Option<u64> {
        self.regions()
            .filter(|region| region.kind.is_direct_mapped())
            .find_map(|region| {
                let end = region.base.checked_add(region.len)?;
                let base = region
                    .base
                    .max(PAGE_SIZE)
                    .checked_next_multiple_of(PAGE_SIZE)?;
                (end.checked_sub(base)? >= len).then_some(base)
            })
    }

    fn covered_by(&self, start: u64, end: u64, allowed: impl Fn(MemoryKind) -> bool) -> bool {
        let mut cursor = start;
        while cursor < end {
            let Some(region) = self.regions().find(|region| {
                region.base <= cursor && cursor < region.base.saturating_add(region.len)
            }) else {
                return false;
            };
            if !allowed(region.kind) {
                return false;
            }
            cursor = region.base.saturating_add(region.len);
        }
        true
    }

    /// Total bytes the frame allocator may hand out.
    pub fn allocatable_bytes(&self) -> u64 {
        self.regions()
            .filter(|region| region.kind.is_allocatable())
            .map(|region| region.len)
            .sum()
    }
}

impl fmt::Debug for MemoryMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryMap")
            .field("regions", &self.len)
            .field("allocatable", &self.allocatable_bytes())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions(count: usize) -> impl ExactSizeIterator<Item = MemoryRegion> {
        (0..count).map(|index| MemoryRegion {
            base: (index as u64 + 1) * PAGE_SIZE,
            len: PAGE_SIZE,
            kind: MemoryKind::Reserved,
        })
    }

    #[test]
    fn the_region_limit_is_all_or_nothing() {
        assert_eq!(
            MemoryMap::try_from_regions(core::iter::empty()),
            Err(MemoryMapError::Empty)
        );
        for count in [MAX_REGIONS - 1, MAX_REGIONS] {
            let map = MemoryMap::try_from_regions(regions(count)).expect("map within bound");
            assert_eq!(map.regions().count(), count);
            assert!(map.regions().eq(regions(count)));
        }

        assert_eq!(
            MemoryMap::try_from_regions(regions(MAX_REGIONS + 1)),
            Err(MemoryMapError::TooMany {
                reported: MAX_REGIONS + 1,
                capacity: MAX_REGIONS,
            })
        );
    }

    #[test]
    fn every_accepted_region_informs_memory_policy() {
        let tail = 0x4000_0000;
        let mut input = regions(MAX_REGIONS).collect::<Vec<_>>();
        input[MAX_REGIONS - 3] = MemoryRegion {
            base: tail,
            len: PAGE_SIZE * 2,
            kind: MemoryKind::Usable,
        };
        input[MAX_REGIONS - 2] = MemoryRegion {
            base: tail + PAGE_SIZE * 2,
            len: PAGE_SIZE,
            kind: MemoryKind::AcpiNvs,
        };
        input[MAX_REGIONS - 1] = MemoryRegion {
            base: tail + PAGE_SIZE * 3,
            len: PAGE_SIZE,
            kind: MemoryKind::Reserved,
        };

        let map = MemoryMap::try_from_regions(input).expect("map at bound");
        assert_eq!(map.allocatable_bytes(), PAGE_SIZE * 2);
        assert_eq!(map.first_memory_run(PAGE_SIZE * 2), Some(tail));
        assert!(map.covers(tail, tail + PAGE_SIZE * 4));
        assert!(map.covers_memory(tail, tail + PAGE_SIZE * 3));
        assert!(!map.covers_memory(tail, tail + PAGE_SIZE * 4));
    }
}
