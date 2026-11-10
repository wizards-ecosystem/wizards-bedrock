//! A complete bounded physical memory map.
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
//! assert!(map.covers_memory(0x1000, 0x5000));
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

const PAGE_SIZE: u64 = 4096;

/// Regions an allocation-free [`MemoryMap`] can hold.
pub const MAX_REGIONS: usize = 128;

/// What a region of the physical address space is for; an unrecognised kind keeps its raw value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum MemoryKind {
    Usable,
    BootloaderReclaimable,
    KernelAndModules,
    Framebuffer,
    AcpiReclaimable,
    /// ACPI non-volatile storage, which must survive.
    AcpiNvs,
    Reserved,
    BadMemory,
    Unknown(u64),
}

impl MemoryKind {
    /// Whether the frame allocator may hand this region out; reclaimable kinds still have users.
    pub const fn is_allocatable(self) -> bool {
        matches!(self, Self::Usable)
    }

    /// Whether immutable firmware-table reads may reach this region.
    pub const fn is_firmware_table(self) -> bool {
        matches!(self, Self::AcpiReclaimable | Self::AcpiNvs)
    }

    /// Whether the direct map covers this region; reserved ranges may be device holes.
    /// Boot, image, framebuffer and table memory stay mapped, in use across the CR3 switch.
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

    const fn permits_device_mapping(self) -> bool {
        matches!(self, Self::Reserved | Self::Unknown(_))
    }
}

impl fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Usable => core::fmt::Write::write_str(f, "usable"),
            Self::BootloaderReclaimable => core::fmt::Write::write_str(f, "bootloader reclaimable"),
            Self::KernelAndModules => core::fmt::Write::write_str(f, "kernel and modules"),
            Self::Framebuffer => core::fmt::Write::write_str(f, "framebuffer"),
            Self::AcpiReclaimable => core::fmt::Write::write_str(f, "acpi reclaimable"),
            Self::AcpiNvs => core::fmt::Write::write_str(f, "acpi nvs"),
            Self::Reserved => core::fmt::Write::write_str(f, "reserved"),
            Self::BadMemory => core::fmt::Write::write_str(f, "bad memory"),
            Self::Unknown(value) => write!(f, "unknown type {value}"),
        }
    }
}

/// One region of the physical address space.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MemoryRegion {
    pub base: u64,
    pub len: u64,
    pub kind: MemoryKind,
}

impl MemoryRegion {
    const fn end(self) -> u64 {
        self.base + self.len
    }
}

/// Why no complete bounded map could be constructed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum MemoryMapError {
    Empty,
    /// The producer described more regions than fixed early storage can hold.
    TooMany {
        observed: usize,
        capacity: usize,
    },
    /// A region's exclusive end is outside the physical address space.
    InvalidRegion {
        base: u64,
        len: u64,
    },
    /// A direct-mapped region cannot be rounded out to a whole final page.
    PageEndOverflow {
        base: u64,
        len: u64,
    },
}

impl fmt::Display for MemoryMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty => core::fmt::Write::write_str(f, "the map is empty"),
            Self::TooMany { observed, capacity } => {
                write!(
                    f,
                    "the map has at least {observed} regions but storage holds {capacity}"
                )
            }
            Self::InvalidRegion { base, len } => {
                write!(f, "memory region {base:#x} + {len:#x} overflows")
            }
            Self::PageEndOverflow { base, len } => write!(
                f,
                "direct-mapped region {base:#x} + {len:#x} cannot cover its final page"
            ),
        }
    }
}

impl core::error::Error for MemoryMapError {}

/// A complete physical memory map copied into fixed early storage before any heap exists.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MemoryMap {
    regions: [MemoryRegion; MAX_REGIONS],
    len: usize,
}

impl MemoryMap {
    /// Copy a complete region stream or refuse it whole.
    pub fn try_from_regions(
        regions: impl IntoIterator<Item = MemoryRegion>,
    ) -> Result<Self, MemoryMapError> {
        let mut stored = [MemoryRegion {
            base: 0,
            len: 0,
            kind: MemoryKind::Reserved,
        }; MAX_REGIONS];
        let mut len = 0usize;
        for region in regions {
            if len == MAX_REGIONS {
                return Err(MemoryMapError::TooMany {
                    observed: MAX_REGIONS + 1,
                    capacity: MAX_REGIONS,
                });
            }
            let Some(end) = region.base.checked_add(region.len) else {
                return Err(MemoryMapError::InvalidRegion {
                    base: region.base,
                    len: region.len,
                });
            };
            if region.len != 0
                && region.kind.is_direct_mapped()
                && end.checked_next_multiple_of(PAGE_SIZE).is_none()
            {
                return Err(MemoryMapError::PageEndOverflow {
                    base: region.base,
                    len: region.len,
                });
            }
            stored[len] = region;
            len += 1;
        }
        if len == 0 {
            return Err(MemoryMapError::Empty);
        }
        Ok(Self {
            regions: stored,
            len,
        })
    }

    /// Every region in the complete input, in its original order.
    pub fn regions(&self) -> impl Iterator<Item = MemoryRegion> + '_ {
        self.regions[..self.len].iter().copied()
    }

    /// Maximal page-aligned runs every claim admits to the direct map; framebuffer policy wins.
    pub fn page_runs(&self) -> PageRuns<'_> {
        PageRuns {
            map: self,
            cursor: self.first_boundary(),
            done: false,
        }
    }

    /// Whole frames covered only by allocatable claims, found by a boundary sweep.
    pub fn allocatable_runs(&self) -> AllocatableRuns<'_> {
        AllocatableRuns {
            runs: self.page_runs(),
        }
    }

    /// Whether every byte of `start..end` is described; input order is not assumed.
    pub fn covers(&self, start: u64, end: u64) -> bool {
        self.covered_by(start, end, |_| true)
    }

    /// Whether every byte of `start..end` lies inside described memory, which a device hole is not.
    /// The distinction keeps any cacheable alias away from device registers.
    pub fn covers_memory(&self, start: u64, end: u64) -> bool {
        self.covered_by(start, end, MemoryKind::is_direct_mapped)
    }

    /// The one cache type the direct map gives every byte of `start..end`, if there is one.
    /// A mixed range gets `None`, because no single mapping type agrees for every page.
    pub fn direct_map_cache(&self, start: u64, end: u64) -> Option<DirectMapCache> {
        if !self.covers_memory(start, end) {
            return None;
        }
        let touches_framebuffer = self.regions().any(|region| {
            region.len != 0
                && region.kind == MemoryKind::Framebuffer
                && region.base < end
                && start < region.end()
        });
        if !touches_framebuffer {
            return Some(DirectMapCache::Normal);
        }
        self.covered_by(start, end, |kind| kind == MemoryKind::Framebuffer)
            .then_some(DirectMapCache::Framebuffer)
    }

    /// Whether a physical extent is disjoint from RAM and firmware storage.
    pub fn allows_device_mapping(&self, start: u64, end: u64) -> bool {
        start < end
            && self.regions().all(|region| {
                let overlaps = region.len != 0 && start < region.end() && region.base < end;
                !overlaps || region.kind.permits_device_mapping()
            })
    }

    /// Whether reserved bytes `base..=limit` share no page with RAM or firmware storage.
    pub fn allows_reserved_region(&self, base: u64, limit: u64) -> bool {
        let start = base - base % PAGE_SIZE;
        let end = limit
            .checked_add(1)
            .and_then(|end| end.checked_next_multiple_of(PAGE_SIZE));
        base <= limit && end.is_some_and(|end| self.allows_device_mapping(start, end))
    }

    /// Find a page-aligned, nonzero memory run of at least `len` bytes.
    /// [`Self::covers_memory`] agrees the run is memory, so a caller never maps a device hole.
    pub fn first_memory_run(&self, len: u64) -> Option<u64> {
        if len == 0 {
            return None;
        }
        let mut first = None;
        for region in self.regions() {
            for boundary in [region.base, region.end()] {
                let Some(base) = boundary.max(PAGE_SIZE).checked_next_multiple_of(PAGE_SIZE) else {
                    continue;
                };
                let Some(end) = base.checked_add(len) else {
                    continue;
                };
                if first.is_none_or(|known| base < known) && self.covers_memory(base, end) {
                    first = Some(base);
                }
            }
        }
        first
    }

    // A long admitted span can jump over a nested claim, so intersecting regions are arbitrated first.
    fn covered_by(&self, start: u64, end: u64, allowed: impl Fn(MemoryKind) -> bool) -> bool {
        if start > end {
            return false;
        }
        if start < end
            && self.regions().any(|region| {
                region.len != 0
                    && region.base < end
                    && start < region.end()
                    && !allowed(region.kind)
            })
        {
            return false;
        }

        let mut cursor = start;
        for _ in 0..=self.len {
            if cursor >= end {
                return true;
            }
            let Some(reach) = self
                .regions()
                .filter(|region| region.base <= cursor && cursor < region.end())
                .map(MemoryRegion::end)
                .max()
            else {
                return false;
            };
            if reach <= cursor {
                return false;
            }
            cursor = reach;
        }
        false
    }

    fn page_policy(&self, start: u64, end: u64) -> Option<(DirectMapCache, bool)> {
        if !self.covered_by(start, end, MemoryKind::is_direct_mapped) {
            return None;
        }
        let mut framebuffer = false;
        for region in self.regions() {
            if region.len == 0 || start >= region.end() || region.base >= end {
                continue;
            }
            framebuffer |= region.kind == MemoryKind::Framebuffer;
        }
        let cache = if framebuffer {
            DirectMapCache::Framebuffer
        } else {
            DirectMapCache::Normal
        };
        Some((
            cache,
            self.covered_by(start, end, MemoryKind::is_allocatable),
        ))
    }

    fn first_boundary(&self) -> Option<u64> {
        self.boundary(None)
    }

    fn boundary_after(&self, after: u64) -> Option<u64> {
        self.boundary(Some(after))
    }

    fn boundary(&self, after: Option<u64>) -> Option<u64> {
        let mut found = None;
        for region in self.regions() {
            let end = region.end();
            let candidates = [
                Some(region.base - region.base % PAGE_SIZE),
                region.base.checked_next_multiple_of(PAGE_SIZE),
                Some(end - end % PAGE_SIZE),
                end.checked_next_multiple_of(PAGE_SIZE),
            ];
            for candidate in candidates.into_iter().flatten() {
                if after.is_some_and(|known| candidate <= known)
                    || found.is_some_and(|known| candidate >= known)
                {
                    continue;
                }
                found = Some(candidate);
            }
        }
        found
    }
}

impl fmt::Debug for MemoryMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryMap")
            .field("regions", &self.len)
            .finish()
    }
}

/// Cache policy for one admitted direct-map run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirectMapCache {
    Normal,
    /// A framebuffer page, mapped write-through until write-combining exists.
    Framebuffer,
}

/// One maximal page-normalized run with one direct-map and allocation policy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PageRun {
    base: u64,
    len: u64,
    cache: DirectMapCache,
    allocatable: bool,
}

impl PageRun {
    pub const fn base(self) -> u64 {
        self.base
    }

    /// Bytes in the run, always a nonzero whole number of pages.
    pub const fn bytes(self) -> u64 {
        self.len
    }

    pub const fn cache(self) -> DirectMapCache {
        self.cache
    }

    /// Whether every frame in the run is exclusively allocatable.
    pub const fn is_allocatable(self) -> bool {
        self.allocatable
    }
}

/// Maximal admitted direct-map runs in ascending address order.
#[derive(Debug)]
pub struct PageRuns<'map> {
    map: &'map MemoryMap,
    cursor: Option<u64>,
    done: bool,
}

impl Iterator for PageRuns<'_> {
    type Item = PageRun;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let mut left = self.cursor?;
        loop {
            let Some(right) = self.map.boundary_after(left) else {
                self.done = true;
                return None;
            };
            let Some((cache, allocatable)) = self.map.page_policy(left, right) else {
                left = right;
                self.cursor = Some(left);
                continue;
            };

            let base = left;
            let mut end = right;
            loop {
                let Some(next) = self.map.boundary_after(end) else {
                    self.done = true;
                    break;
                };
                if self.map.page_policy(end, next) != Some((cache, allocatable)) {
                    self.cursor = Some(end);
                    break;
                }
                end = next;
            }
            return Some(PageRun {
                base,
                len: end - base,
                cache,
                allocatable,
            });
        }
    }
}

/// One maximal run of whole frames described exclusively as allocatable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AllocatableRun {
    base: u64,
    len: u64,
}

impl AllocatableRun {
    pub const fn base(self) -> u64 {
        self.base
    }

    /// Bytes in the run, always a nonzero whole number of frames.
    pub const fn bytes(self) -> u64 {
        self.len
    }
}

/// Maximal exclusively allocatable frame runs in ascending address order.
#[derive(Debug)]
pub struct AllocatableRuns<'map> {
    runs: PageRuns<'map>,
}

impl Iterator for AllocatableRuns<'_> {
    type Item = AllocatableRun;

    fn next(&mut self) -> Option<Self::Item> {
        let run = self.runs.find(|run| run.is_allocatable())?;
        Some(AllocatableRun {
            base: run.base(),
            len: run.bytes(),
        })
    }
}

#[cfg(test)]
mod tests;
