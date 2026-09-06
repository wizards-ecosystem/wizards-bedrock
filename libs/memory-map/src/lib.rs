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
//! assert!(map.covers_memory(0x1000, 0x5000));
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegion {
    /// First physical byte.
    pub base: u64,
    /// Length in bytes.
    pub len: u64,
    /// What the region is for.
    pub kind: MemoryKind,
}

impl MemoryRegion {
    const fn end(self) -> u64 {
        self.base + self.len
    }
}

/// Why no complete bounded map could be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MemoryMapError {
    /// The producer described no regions.
    Empty,
    /// The producer described more regions than fixed early storage can hold.
    TooMany {
        /// Regions observed before bounded construction stopped.
        observed: usize,
        /// Regions the allocation-free representation can hold.
        capacity: usize,
    },
    /// A region's exclusive end is outside the physical address space.
    InvalidRegion {
        /// First physical byte.
        base: u64,
        /// Requested length.
        len: u64,
    },
    /// A direct-mapped region cannot be rounded out to a whole final page.
    PageEndOverflow {
        /// First physical byte.
        base: u64,
        /// Requested length.
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

/// Cache policy for one admitted direct-map run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectMapCache {
    /// Ordinary cacheable memory.
    Normal,
    /// A framebuffer page, mapped write-through until write-combining exists.
    Framebuffer,
}

/// One maximal page-normalized run with one direct-map and allocation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRun {
    base: u64,
    len: u64,
    cache: DirectMapCache,
    allocatable: bool,
}

impl PageRun {
    /// First physical byte in the run.
    pub const fn base(self) -> u64 {
        self.base
    }

    /// Bytes in the run, always a nonzero whole number of pages.
    pub const fn bytes(self) -> u64 {
        self.len
    }

    /// Cache policy for the direct mapping.
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
pub struct PageRuns<'a> {
    map: &'a MemoryMap,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocatableRun {
    base: u64,
    len: u64,
}

impl AllocatableRun {
    /// First physical byte in the run.
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
pub struct AllocatableRuns<'a> {
    runs: PageRuns<'a>,
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

impl MemoryMap {
    /// Copy a complete region stream or refuse it whole.
    ///
    /// Regions may overlap or arrive unsorted. Every covering claim participates
    /// in arbitration, so a non-admitted kind refuses an overlapping range
    /// regardless of order (D-241).
    ///
    /// # Errors
    ///
    /// [`MemoryMapError::Empty`] when `regions` is empty, or
    /// [`MemoryMapError::TooMany`] without exposing the copied prefix when it
    /// exceeds [`MAX_REGIONS`], [`MemoryMapError::InvalidRegion`] when a
    /// region's exclusive end cannot be represented, or
    /// [`MemoryMapError::PageEndOverflow`] when an admitted direct-map region
    /// cannot be rounded outward to a complete page.
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

    /// Yield normalized page-policy runs shared by mapping and allocation.
    ///
    /// Runs are page-aligned, disjoint and maximal. A page is omitted unless
    /// every claim admits direct mapping; framebuffer cache policy wins.
    /// Allocation additionally requires complete, exclusively allocatable
    /// coverage.
    pub fn page_runs(&self) -> PageRuns<'_> {
        PageRuns {
            map: self,
            cursor: self.first_boundary(),
            done: false,
        }
    }

    /// Yield whole frames covered only by allocatable claims.
    ///
    /// Partial frames and frames touched by any other claim are excluded. The
    /// boundary sweep prevents a huge range from causing a page-by-page delay.
    pub fn allocatable_runs(&self) -> AllocatableRuns<'_> {
        AllocatableRuns {
            runs: self.page_runs(),
        }
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

    /// Whether a physical extent is disjoint from RAM and firmware storage.
    pub fn allows_device_mapping(&self, start: u64, end: u64) -> bool {
        start < end
            && self.regions().all(|region| {
                let overlaps = start < region.end() && region.base < end;
                !overlaps || region.kind.permits_device_mapping()
            })
    }

    /// Find a page-aligned, nonzero memory run of at least `len` bytes.
    ///
    /// Callers use this rather than assuming a conventional low page exists,
    /// which keeps machine checks meaningful across firmware layouts (D-177).
    ///
    /// The run it names is one [`covers_memory`](Self::covers_memory) agrees
    /// is memory. Filtering to direct-mapped regions was not enough on its own:
    /// a reserved region overlapping the one chosen made the two answers
    /// contradict, and a caller acting on this one would map a device hole
    /// (D-241).
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

    fn covered_by(&self, start: u64, end: u64, allowed: impl Fn(MemoryKind) -> bool) -> bool {
        // A long admitted span can jump over a nested claim, so arbitrate every
        // region intersecting the requested range before walking its union.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct MemoryKindCase {
        kind: MemoryKind,
        name: &'static str,
        allocatable: bool,
        firmware_table: bool,
        direct_mapped: bool,
        device_mapping: bool,
    }

    const fn kind_case(
        kind: MemoryKind,
        name: &'static str,
        allocatable: bool,
        firmware_table: bool,
        direct_mapped: bool,
        device_mapping: bool,
    ) -> MemoryKindCase {
        MemoryKindCase {
            kind,
            name,
            allocatable,
            firmware_table,
            direct_mapped,
            device_mapping,
        }
    }

    fn memory_kind_cases() -> [MemoryKindCase; 9] {
        [
            kind_case(MemoryKind::Usable, "usable", true, false, true, false),
            kind_case(
                MemoryKind::BootloaderReclaimable,
                "bootloader reclaimable",
                false,
                false,
                true,
                false,
            ),
            kind_case(
                MemoryKind::KernelAndModules,
                "kernel and modules",
                false,
                false,
                true,
                false,
            ),
            kind_case(
                MemoryKind::Framebuffer,
                "framebuffer",
                false,
                false,
                true,
                false,
            ),
            kind_case(
                MemoryKind::AcpiReclaimable,
                "acpi reclaimable",
                false,
                true,
                true,
                false,
            ),
            kind_case(MemoryKind::AcpiNvs, "acpi nvs", false, true, true, false),
            kind_case(MemoryKind::Reserved, "reserved", false, false, false, true),
            kind_case(
                MemoryKind::BadMemory,
                "bad memory",
                false,
                false,
                false,
                false,
            ),
            kind_case(
                MemoryKind::Unknown(41),
                "unknown type 41",
                false,
                false,
                false,
                true,
            ),
        ]
    }

    fn regions(count: usize) -> impl ExactSizeIterator<Item = MemoryRegion> {
        (0..count).map(|index| MemoryRegion {
            base: (index as u64 + 1) * PAGE_SIZE,
            len: PAGE_SIZE,
            kind: MemoryKind::Reserved,
        })
    }

    #[test]
    fn every_memory_kind_has_its_exact_policy_and_name() {
        for case in memory_kind_cases() {
            assert_eq!(case.kind.to_string(), case.name);
            assert_eq!(
                case.kind.is_allocatable(),
                case.allocatable,
                "{:?}",
                case.kind
            );
            assert_eq!(
                case.kind.is_firmware_table(),
                case.firmware_table,
                "{:?}",
                case.kind
            );
            assert_eq!(
                case.kind.is_direct_mapped(),
                case.direct_mapped,
                "{:?}",
                case.kind
            );
            assert_eq!(
                case.kind.permits_device_mapping(),
                case.device_mapping,
                "{:?}",
                case.kind
            );
        }
    }

    #[test]
    fn map_errors_name_the_reported_and_supported_region_counts() {
        assert_eq!(MemoryMapError::Empty.to_string(), "the map is empty");
        assert_eq!(
            MemoryMapError::TooMany {
                observed: 129,
                capacity: 128,
            }
            .to_string(),
            "the map has at least 129 regions but storage holds 128"
        );
        assert_eq!(
            MemoryMapError::InvalidRegion {
                base: u64::MAX,
                len: 1,
            }
            .to_string(),
            "memory region 0xffffffffffffffff + 0x1 overflows"
        );
        assert_eq!(
            MemoryMapError::PageEndOverflow {
                base: u64::MAX - 1,
                len: 1,
            }
            .to_string(),
            "direct-mapped region 0xfffffffffffffffe + 0x1 cannot cover its final page"
        );
    }

    #[test]
    fn debug_reports_only_the_complete_region_count() {
        let map = MemoryMap::try_from_regions([
            MemoryRegion {
                base: PAGE_SIZE,
                len: PAGE_SIZE * 2,
                kind: MemoryKind::Usable,
            },
            MemoryRegion {
                base: PAGE_SIZE * 3,
                len: PAGE_SIZE,
                kind: MemoryKind::Reserved,
            },
        ])
        .expect("map");
        assert_eq!(format!("{map:?}"), "MemoryMap { regions: 2 }");
    }

    /// Two regions at one base answer the same whichever order they arrive in.
    ///
    /// Transcribed from `covers`'s "Input order is not assumed" contract.
    /// Overlap order must change neither the verdict nor the walk distance
    /// (D-241).
    #[test]
    fn overlapping_regions_answer_the_same_in_either_order() {
        let usable = MemoryRegion {
            base: 0,
            len: PAGE_SIZE * 3,
            kind: MemoryKind::Usable,
        };
        let reserved = MemoryRegion {
            base: 0,
            len: PAGE_SIZE * 2,
            kind: MemoryKind::Reserved,
        };
        let forward = MemoryMap::try_from_regions([reserved, usable]).expect("map");
        let reversed = MemoryMap::try_from_regions([usable, reserved]).expect("map");

        for (start, end) in [(0, 1), (0, PAGE_SIZE), (PAGE_SIZE, PAGE_SIZE * 3)] {
            assert_eq!(
                forward.covers(start, end),
                reversed.covers(start, end),
                "covers({start:#x}, {end:#x}) depends on region order"
            );
            assert_eq!(
                forward.covers_memory(start, end),
                reversed.covers_memory(start, end),
                "covers_memory({start:#x}, {end:#x}) depends on region order"
            );
        }
        // A reserved claim wins where it overlaps: the point is to keep a
        // cacheable alias off device registers, so a contradiction refuses.
        assert!(!forward.covers_memory(0, 1));
        assert!(forward.covers(0, 1));
    }

    #[test]
    fn allocatable_runs_remove_a_nested_reserved_frame_in_either_order() {
        let usable = MemoryRegion {
            base: PAGE_SIZE,
            len: PAGE_SIZE * 4,
            kind: MemoryKind::Usable,
        };
        let reserved = MemoryRegion {
            base: PAGE_SIZE * 2,
            len: PAGE_SIZE,
            kind: MemoryKind::Reserved,
        };
        let expected = [
            AllocatableRun {
                base: PAGE_SIZE,
                len: PAGE_SIZE,
            },
            AllocatableRun {
                base: PAGE_SIZE * 3,
                len: PAGE_SIZE * 2,
            },
        ];

        for regions in [[usable, reserved], [reserved, usable]] {
            let map = MemoryMap::try_from_regions(regions).expect("overlapping map");
            assert_eq!(map.allocatable_runs().collect::<Vec<_>>(), expected);
        }
    }

    #[test]
    fn allocatable_runs_exclude_partial_edges_and_every_poisoned_frame() {
        let regions = [
            MemoryRegion {
                base: PAGE_SIZE + 1,
                len: PAGE_SIZE * 8 - 2,
                kind: MemoryKind::Usable,
            },
            MemoryRegion {
                base: PAGE_SIZE * 3 + 1,
                len: PAGE_SIZE - 2,
                kind: MemoryKind::Reserved,
            },
            MemoryRegion {
                base: PAGE_SIZE * 5,
                len: PAGE_SIZE,
                kind: MemoryKind::BadMemory,
            },
            MemoryRegion {
                base: PAGE_SIZE * 6 + 1,
                len: 1,
                kind: MemoryKind::Unknown(7),
            },
        ];
        let expected = [
            AllocatableRun {
                base: PAGE_SIZE * 2,
                len: PAGE_SIZE,
            },
            AllocatableRun {
                base: PAGE_SIZE * 4,
                len: PAGE_SIZE,
            },
            AllocatableRun {
                base: PAGE_SIZE * 7,
                len: PAGE_SIZE,
            },
        ];

        let forward = MemoryMap::try_from_regions(regions).expect("map");
        let mut reversed = regions;
        reversed.reverse();
        let reverse = MemoryMap::try_from_regions(reversed).expect("reversed map");
        assert_eq!(forward.allocatable_runs().collect::<Vec<_>>(), expected);
        assert_eq!(reverse.allocatable_runs().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn page_runs_are_disjoint_order_independent_and_count_each_page_once() {
        let usable = MemoryRegion {
            base: PAGE_SIZE,
            len: PAGE_SIZE * 4,
            kind: MemoryKind::Usable,
        };
        let framebuffer = MemoryRegion {
            base: PAGE_SIZE * 2,
            len: PAGE_SIZE * 2,
            kind: MemoryKind::Framebuffer,
        };
        let reserved = MemoryRegion {
            base: PAGE_SIZE * 3,
            len: PAGE_SIZE,
            kind: MemoryKind::Reserved,
        };
        let expected = [
            PageRun {
                base: PAGE_SIZE,
                len: PAGE_SIZE,
                cache: DirectMapCache::Normal,
                allocatable: true,
            },
            PageRun {
                base: PAGE_SIZE * 2,
                len: PAGE_SIZE,
                cache: DirectMapCache::Framebuffer,
                allocatable: false,
            },
            PageRun {
                base: PAGE_SIZE * 4,
                len: PAGE_SIZE,
                cache: DirectMapCache::Normal,
                allocatable: true,
            },
        ];

        for regions in [
            [usable, usable, framebuffer, reserved],
            [reserved, framebuffer, usable, usable],
        ] {
            let map = MemoryMap::try_from_regions(regions).expect("overlapping map");
            let runs = map.page_runs().collect::<Vec<_>>();
            assert_eq!(runs, expected);
            assert_eq!(
                runs.iter().map(|run| run.bytes()).sum::<u64>(),
                PAGE_SIZE * 3
            );
        }
    }

    #[test]
    fn page_runs_require_complete_coverage_but_adjacent_claims_may_supply_it() {
        let partial = MemoryMap::try_from_regions([MemoryRegion {
            base: PAGE_SIZE + 1,
            len: 1,
            kind: MemoryKind::Usable,
        }])
        .expect("partial claim");
        assert_eq!(core::iter::Iterator::next(&mut partial.page_runs()), None);

        let hole = MemoryMap::try_from_regions([
            MemoryRegion {
                base: PAGE_SIZE,
                len: PAGE_SIZE / 2,
                kind: MemoryKind::Usable,
            },
            MemoryRegion {
                base: PAGE_SIZE * 2 - PAGE_SIZE / 4,
                len: PAGE_SIZE / 4,
                kind: MemoryKind::AcpiNvs,
            },
        ])
        .expect("claims with a hole");
        assert_eq!(core::iter::Iterator::next(&mut hole.page_runs()), None);

        let adjacent = MemoryMap::try_from_regions([
            MemoryRegion {
                base: PAGE_SIZE,
                len: PAGE_SIZE / 2,
                kind: MemoryKind::Usable,
            },
            MemoryRegion {
                base: PAGE_SIZE + PAGE_SIZE / 2,
                len: PAGE_SIZE / 2,
                kind: MemoryKind::Framebuffer,
            },
        ])
        .expect("adjacent claims");
        assert_eq!(
            adjacent.page_runs().collect::<Vec<_>>(),
            [PageRun {
                base: PAGE_SIZE,
                len: PAGE_SIZE,
                cache: DirectMapCache::Framebuffer,
                allocatable: false,
            }]
        );
    }

    #[test]
    fn a_disallowed_region_at_the_exclusive_end_does_not_overlap() {
        let start = PAGE_SIZE;
        let end = PAGE_SIZE * 3;
        let map = MemoryMap::try_from_regions([
            MemoryRegion {
                base: start,
                len: end - start,
                kind: MemoryKind::Usable,
            },
            MemoryRegion {
                base: end,
                len: PAGE_SIZE,
                kind: MemoryKind::Reserved,
            },
        ])
        .expect("adjacent regions");

        assert!(map.covers_memory(start, end));
        assert!(!map.covers_memory(start, end + 1));
    }

    #[test]
    fn device_eligibility_covers_the_whole_mapped_page_not_only_register_bytes() {
        let page = PAGE_SIZE;
        let map = MemoryMap::try_from_regions([
            MemoryRegion {
                base: page,
                len: 4,
                kind: MemoryKind::Usable,
            },
            MemoryRegion {
                base: page + 4,
                len: page - 4,
                kind: MemoryKind::Reserved,
            },
        ])
        .expect("one split page");

        assert!(map.allows_device_mapping(page + 4, page + 8));
        assert!(!map.allows_device_mapping(page, page * 2));
        assert!(map.allows_device_mapping(page * 2, page * 3));
        assert!(!map.allows_device_mapping(page * 3, page * 3));
    }

    #[test]
    fn every_forbidden_kind_and_byte_in_a_page_refuses_device_mapping() {
        let page = PAGE_SIZE;
        for case in memory_kind_cases()
            .into_iter()
            .filter(|case| !case.device_mapping)
        {
            for offset in 0..PAGE_SIZE {
                let map = MemoryMap::try_from_regions([MemoryRegion {
                    base: page + offset,
                    len: 1,
                    kind: case.kind,
                }])
                .expect("one forbidden byte");
                assert!(
                    !map.allows_device_mapping(page, page * 2),
                    "{:?} byte {offset:#x} was admitted",
                    case.kind
                );
            }
        }

        for case in memory_kind_cases()
            .into_iter()
            .filter(|case| case.device_mapping)
        {
            let map = MemoryMap::try_from_regions([MemoryRegion {
                base: page,
                len: page,
                kind: case.kind,
            }])
            .expect("one device-hole page");
            assert!(map.allows_device_mapping(page, page * 2), "{:?}", case.kind);
        }
    }

    #[test]
    fn a_disallowed_region_at_a_device_extents_exclusive_end_is_disjoint() {
        let map = MemoryMap::try_from_regions([MemoryRegion {
            base: PAGE_SIZE * 2,
            len: PAGE_SIZE,
            kind: MemoryKind::Usable,
        }])
        .expect("map");

        assert!(map.allows_device_mapping(PAGE_SIZE, PAGE_SIZE * 2));
        assert!(!map.allows_device_mapping(PAGE_SIZE, PAGE_SIZE * 2 + 1));
    }

    /// A run this names is one `covers_memory` agrees is memory.
    ///
    /// The two contracts are about the same address and disagreed: the filter
    /// picked from a direct-mapped region while the walk landed in a reserved
    /// one overlapping it, and D-177 says callers act on this answer (D-241).
    #[test]
    fn a_named_memory_run_is_memory_by_the_other_contract() {
        let usable = MemoryRegion {
            base: 0,
            len: PAGE_SIZE * 3,
            kind: MemoryKind::Usable,
        };
        let reserved = MemoryRegion {
            base: 0,
            len: PAGE_SIZE * 2,
            kind: MemoryKind::Reserved,
        };
        let map = MemoryMap::try_from_regions([reserved, usable]).expect("map");

        for len in [1, PAGE_SIZE, PAGE_SIZE * 2] {
            if let Some(base) = map.first_memory_run(len) {
                let end = base.checked_add(len).expect("a run inside the space");
                assert!(
                    map.covers_memory(base, end),
                    "first_memory_run({len:#x}) named {base:#x}, which covers_memory denies"
                );
            }
        }
    }

    #[test]
    fn the_first_run_jumps_past_a_reserved_prefix() {
        let map = MemoryMap::try_from_regions([
            MemoryRegion {
                base: PAGE_SIZE,
                len: PAGE_SIZE * 3,
                kind: MemoryKind::Usable,
            },
            MemoryRegion {
                base: PAGE_SIZE,
                len: PAGE_SIZE,
                kind: MemoryKind::Reserved,
            },
        ])
        .expect("overlapping map");

        assert_eq!(map.first_memory_run(PAGE_SIZE), Some(PAGE_SIZE * 2));
    }

    #[test]
    fn a_zero_length_run_has_no_first_byte() {
        let map = MemoryMap::try_from_regions([MemoryRegion {
            base: PAGE_SIZE,
            len: PAGE_SIZE,
            kind: MemoryKind::Usable,
        }])
        .expect("map");

        assert_eq!(map.first_memory_run(0), None);
    }

    #[test]
    fn an_overflowing_region_refuses_the_whole_map() {
        let valid = MemoryRegion {
            base: PAGE_SIZE,
            len: PAGE_SIZE,
            kind: MemoryKind::Usable,
        };
        let overflowing = MemoryRegion {
            base: u64::MAX,
            len: 1,
            kind: MemoryKind::Reserved,
        };

        assert_eq!(
            MemoryMap::try_from_regions([valid, overflowing]),
            Err(MemoryMapError::InvalidRegion {
                base: u64::MAX,
                len: 1,
            })
        );
        assert!(
            MemoryMap::try_from_regions([MemoryRegion {
                base: u64::MAX,
                len: 0,
                kind: MemoryKind::Reserved,
            }])
            .is_ok()
        );
        assert_eq!(
            MemoryMap::try_from_regions([MemoryRegion {
                base: u64::MAX - 1,
                len: 1,
                kind: MemoryKind::Usable,
            }]),
            Err(MemoryMapError::PageEndOverflow {
                base: u64::MAX - 1,
                len: 1,
            })
        );
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
                observed: MAX_REGIONS + 1,
                capacity: MAX_REGIONS,
            })
        );
    }

    #[test]
    fn an_overfull_producer_is_not_polled_past_the_first_refused_region() {
        struct BoundedProducer {
            next: usize,
        }

        impl Iterator for BoundedProducer {
            type Item = MemoryRegion;

            fn next(&mut self) -> Option<Self::Item> {
                assert!(
                    self.next <= MAX_REGIONS,
                    "constructor polled past its bound"
                );
                let index = self.next;
                self.next += 1;
                Some(MemoryRegion {
                    base: index as u64 * PAGE_SIZE,
                    len: PAGE_SIZE,
                    kind: MemoryKind::Reserved,
                })
            }
        }

        assert_eq!(
            MemoryMap::try_from_regions(BoundedProducer { next: 0 }),
            Err(MemoryMapError::TooMany {
                observed: MAX_REGIONS + 1,
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
        assert_eq!(map.first_memory_run(PAGE_SIZE * 2), Some(tail));
        assert!(map.covers(tail, tail + PAGE_SIZE * 4));
        assert!(map.covers_memory(tail, tail + PAGE_SIZE * 3));
        assert!(!map.covers_memory(tail, tail + PAGE_SIZE * 4));
    }
}
