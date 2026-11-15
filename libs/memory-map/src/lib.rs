#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

const PAGE_SIZE: u64 = 4096;

const MAX_REGIONS: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryKind {
    Usable,
    BootloaderReclaimable,
    KernelAndModules,
    Framebuffer,
    AcpiReclaimable,
    AcpiNvs,
    Reserved,
    BadMemory,
    Other(u64),
}

impl MemoryKind {
    // Reclaimable kinds still have users.
    const fn is_allocatable(self) -> bool {
        matches!(self, Self::Usable)
    }

    pub const fn is_firmware_table(self) -> bool {
        matches!(self, Self::AcpiReclaimable | Self::AcpiNvs)
    }

    // Reserved ranges may be device holes.
    const fn is_direct_mapped(self) -> bool {
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
        matches!(self, Self::Reserved | Self::Other(_))
    }
}

impl fmt::Display for MemoryKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Usable => formatter.write_str("usable"),
            Self::BootloaderReclaimable => formatter.write_str("bootloader reclaimable"),
            Self::KernelAndModules => formatter.write_str("kernel and modules"),
            Self::Framebuffer => formatter.write_str("framebuffer"),
            Self::AcpiReclaimable => formatter.write_str("acpi reclaimable"),
            Self::AcpiNvs => formatter.write_str("acpi nvs"),
            Self::Reserved => formatter.write_str("reserved"),
            Self::BadMemory => formatter.write_str("bad memory"),
            Self::Other(value) => write!(formatter, "unknown type {value}"),
        }
    }
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryMapError {
    Empty,
    TooMany { observed: usize, capacity: usize },
    InvalidRegion { base: u64, len: u64 },
    PageEndOverflow { base: u64, len: u64 },
}

impl fmt::Display for MemoryMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty => formatter.write_str("the map is empty"),
            Self::TooMany { observed, capacity } => {
                write!(
                    formatter,
                    "the map has at least {observed} regions but storage holds {capacity}"
                )
            }
            Self::InvalidRegion { base, len } => {
                write!(formatter, "memory region {base:#x} + {len:#x} overflows")
            }
            Self::PageEndOverflow { base, len } => write!(
                formatter,
                "direct-mapped region {base:#x} + {len:#x} cannot cover its final page"
            ),
        }
    }
}

impl core::error::Error for MemoryMapError {}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MemoryMap {
    regions: [MemoryRegion; MAX_REGIONS],
    count: usize,
}

impl MemoryMap {
    pub fn try_from_regions(
        regions: impl IntoIterator<Item = MemoryRegion>,
    ) -> Result<Self, MemoryMapError> {
        let mut stored = [MemoryRegion {
            base: 0,
            len: 0,
            kind: MemoryKind::Reserved,
        }; MAX_REGIONS];
        let mut count = 0usize;
        for region in regions {
            if count == MAX_REGIONS {
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
            stored[count] = region;
            count += 1;
        }
        if count == 0 {
            return Err(MemoryMapError::Empty);
        }
        Ok(Self {
            regions: stored,
            count,
        })
    }

    pub fn regions(&self) -> impl Iterator<Item = MemoryRegion> + '_ {
        self.regions[..self.count].iter().copied()
    }

    pub fn page_runs(&self) -> PageRuns<'_> {
        PageRuns {
            map: self,
            cursor: self.first_boundary(),
            done: false,
        }
    }

    pub fn allocatable_runs(&self) -> AllocatableRuns<'_> {
        AllocatableRuns {
            runs: self.page_runs(),
        }
    }

    pub fn covers(&self, start: u64, end: u64) -> bool {
        self.covered_by(start, end, |_| true)
    }

    fn covers_memory(&self, start: u64, end: u64) -> bool {
        self.covered_by(start, end, MemoryKind::is_direct_mapped)
    }

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

    pub fn allows_device_mapping(&self, start: u64, end: u64) -> bool {
        start < end
            && self.regions().all(|region| {
                let overlaps = region.len != 0 && start < region.end() && region.base < end;
                !overlaps || region.kind.permits_device_mapping()
            })
    }

    pub fn first_memory_run(&self, length: u64) -> Option<u64> {
        if length == 0 {
            return None;
        }
        let mut first = None;
        for region in self.regions() {
            for boundary in [region.base, region.end()] {
                let Some(base) = boundary.max(PAGE_SIZE).checked_next_multiple_of(PAGE_SIZE) else {
                    continue;
                };
                let Some(end) = base.checked_add(length) else {
                    continue;
                };
                if first.is_none_or(|known| base < known) && self.covers_memory(base, end) {
                    first = Some(base);
                }
            }
        }
        first
    }

    // A long admitted span can jump a nested claim, so intersecting regions are arbitrated first.
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
        for _ in 0..=self.count {
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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryMap")
            .field("regions", &self.count)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirectMapCache {
    Normal,
    Framebuffer,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PageRun {
    base: u64,
    bytes: u64,
    cache: DirectMapCache,
    allocatable: bool,
}

impl PageRun {
    pub const fn base(self) -> u64 {
        self.base
    }

    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    pub const fn cache(self) -> DirectMapCache {
        self.cache
    }

    pub const fn is_allocatable(self) -> bool {
        self.allocatable
    }
}

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
                bytes: end - base,
                cache,
                allocatable,
            });
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AllocatableRun {
    base: u64,
    bytes: u64,
}

impl AllocatableRun {
    pub const fn base(self) -> u64 {
        self.base
    }

    pub const fn bytes(self) -> u64 {
        self.bytes
    }
}

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
            bytes: run.bytes(),
        })
    }
}

#[cfg(test)]
mod tests;
