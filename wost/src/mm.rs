//! Physical memory: which frames exist, and who owns each one.

mod direct;
mod dma;
mod frame;
pub mod heap;
#[cfg(feature = "selftest")]
mod selftest;
mod shared;

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use frame_space::{FRAME_SIZE, FrameSpace, words_for};

pub(crate) use dma::{DmaMappingOwner, IommuTable, PublishedIommuTable};
pub use dma::{DmaSegment, TableWalks};
pub use frame::{Frame, Segment, allocate, allocate_contiguous, allocate_zeroed};
#[cfg(feature = "selftest")]
pub use selftest::{
    dma_mapping_owner_drop_retains_for_test, first_memory_run, free_frames,
    freed_external_run_is_zero_for_test, memory_map_for_test,
    overlapping_allocator_claims_are_arbitrated_for_test,
    published_iommu_table_drop_retains_for_test, refused_external_clear_withholds_for_test,
};
pub(crate) use shared::SharedKeepalive;
pub use shared::{Shared, SharedBytes, SharedRing, USegment};

use crate::boot::{DirectMap, MemoryMap, MemoryRegion};
use crate::panic::PanicToken;
use crate::sync::{Once, SpinLock};

/// A physical address.
pub type Paddr = u64;

/// Highest physical address the frame allocator tracks; boot reports memory above it ignored.
///
/// Two zeroed `.bss` bits per frame, so 16 GiB costs one megabyte the image does not carry.
pub const CEILING: u64 = 16 << 30;

type Frames = FrameSpace<{ words_for(CEILING) }>;

static FRAMES: SpinLock<Frames> = SpinLock::new(Frames::new());

/// The direct map's offset, or zero before [`init`]; the higher-half direct map is never at zero.
///
/// Atomic so zeroing a frame never holds the allocator's lock; once written it never changes.
static DIRECT_MAP_OFFSET: AtomicU64 = AtomicU64::new(0);

static INIT: Once = Once::new();

/// What [`init`] found.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Report {
    /// Frames the allocator now manages.
    pub managed: usize,
    /// Frames dropped because they lie above [`CEILING`].
    pub ignored: usize,
}

impl Report {
    const fn managed_bytes(&self) -> u64 {
        self.managed as u64 * FRAME_SIZE
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} MiB usable in {} frames",
            self.managed_bytes() / (1 << 20),
            self.managed
        )?;
        if self.ignored != 0 {
            write!(
                f,
                ", {} frames above the {} GiB ceiling ignored",
                self.ignored,
                CEILING >> 30
            )?;
        }
        Ok(())
    }
}

/// Manage every exclusively allocatable whole-frame run, dropping frames another claim touches.
///
/// # Panics
///
/// On a second call.
pub fn init(map: &MemoryMap, direct: DirectMap) -> Report {
    assert!(INIT.claim(), "physical memory initialized twice");

    DIRECT_MAP_OFFSET.store(direct.offset(), Ordering::Release);
    *MAP.lock_leaf() = Some(*map);

    let mut frames = FRAMES.lock_leaf();
    let ignored = seed_frames(&mut frames, map);
    let managed = frames.managed_count();
    drop(frames);

    Report { managed, ignored }
}

fn seed_frames<const WORDS: usize>(frames: &mut FrameSpace<WORDS>, map: &MemoryMap) -> usize {
    let mut ignored = 0;
    for run in map.allocatable_runs() {
        ignored += frames.insert(run.base(), run.bytes()).ignored;
    }
    ignored
}

/// Allocation-free read-only access to physical firmware tables at handoff, before [`init`].
///
/// It cannot read usable, bootloader, kernel, reserved or device memory.
#[derive(Clone, Copy, Debug)]
pub struct EarlyFirmwareMemory<'map> {
    map: &'map MemoryMap,
    direct: DirectMap,
}

impl<'map> EarlyFirmwareMemory<'map> {
    /// Bind the complete copied memory map to its bootloader direct map.
    pub const fn new(map: &'map MemoryMap, direct: DirectMap) -> Self {
        Self { map, direct }
    }

    /// Whether every byte is described exclusively as ACPI table storage.
    pub fn contains(&self, address: Paddr, len: usize) -> bool {
        firmware_contains(self.map, address, len)
    }

    /// Copy `into.len()` table bytes at `address` with volatile reads.
    #[must_use = "a refused early firmware read copied no bytes"]
    pub fn read(&self, address: Paddr, into: &mut [u8]) -> bool {
        if !self.contains(address, into.len()) {
            return false;
        }
        let Ok(len) = u64::try_from(into.len()) else {
            return false;
        };
        let Some(end) = address.checked_add(len) else {
            return false;
        };
        if self.direct.virtual_address(end).is_none() {
            return false;
        }
        let Some(base) = self.direct.virtual_address(address) else {
            return false;
        };
        read_firmware_bytes(base as *const u8, into);
        true
    }
}

/// Read-only access to physical firmware tables after [`init`], with the early reader's authority.
#[derive(Clone, Copy, Debug)]
pub struct FirmwareMemory(());

impl FirmwareMemory {
    /// Whether every byte is described exclusively as ACPI table storage.
    pub fn contains(&self, address: Paddr, len: usize) -> bool {
        let guard = MAP.lock_leaf();
        let Some(map) = guard.as_ref() else {
            return false;
        };
        firmware_contains(map, address, len)
    }

    /// Copy `into.len()` table bytes at `address` with volatile reads.
    #[must_use = "a refused firmware read copied no bytes"]
    pub fn read(&self, address: Paddr, into: &mut [u8]) -> bool {
        if !self.contains(address, into.len()) {
            return false;
        }
        let Some(base) = direct_pointer(address, into.len() as u64) else {
            return false;
        };
        read_firmware_bytes(base, into);
        true
    }
}

fn firmware_contains(map: &MemoryMap, address: Paddr, len: usize) -> bool {
    let Ok(len) = u64::try_from(len) else {
        return false;
    };
    if len == 0 {
        return false;
    }
    let Some(end) = address.checked_add(len) else {
        return false;
    };
    let overlaps = |region: &MemoryRegion| address < region.base + region.len && region.base < end;
    map.covers(address, end)
        && map
            .regions()
            .filter(overlaps)
            .all(|region| region.kind.is_firmware_table())
}

fn read_firmware_bytes(base: *const u8, into: &mut [u8]) {
    for (index, slot) in into.iter_mut().enumerate() {
        let byte = base.wrapping_add(index);
        // SAFETY: the only callers first proved every byte immutable ACPI
        // storage and proved the direct-map address range cannot overflow.
        *slot = unsafe { byte.read_volatile() };
    }
}

/// Acquire the narrow reader after the memory map and direct map exist.
pub fn firmware_memory() -> Option<FirmwareMemory> {
    (MAP.lock_leaf().is_some() && DIRECT_MAP_OFFSET.load(Ordering::Acquire) != 0)
        .then_some(FirmwareMemory(()))
}

/// Whether `start..end` is disjoint from every RAM or firmware allocation.
pub(crate) fn device_range_is_eligible(start: Paddr, end: Paddr) -> bool {
    let guard = MAP.lock_leaf();
    let Some(map) = guard.as_ref() else {
        return false;
    };
    map.allows_device_mapping(start, end)
}

/// The memory map [`init`] was given, for ownership and firmware checks.
static MAP: SpinLock<Option<MemoryMap>> = SpinLock::new(None);

/// The one cache type the direct map gives `start..end`, if it has one.
pub(crate) fn map_direct_cache(start: Paddr, end: Paddr) -> Option<crate::boot::DirectMapCache> {
    MAP.lock_leaf()
        .as_ref()
        .and_then(|map| map.direct_map_cache(start, end))
}

/// The direct map [`init`] recorded, so a physical-address owner can name a byte for readback.
pub fn direct_map() -> Option<DirectMap> {
    match DIRECT_MAP_OFFSET.load(Ordering::Acquire) {
        0 => None,
        offset => Some(DirectMap::new(offset)),
    }
}

/// Reclaim the frame allocator's locks on the panic path, before reporting reads their state.
pub(crate) fn force_unlock(panic: &PanicToken) {
    FRAMES.force_unlock(panic);
    MAP.force_unlock(panic);
}

/// Copy the boot map so policy queries run after its leaf guard is released.
pub(crate) fn memory_map_snapshot() -> Option<MemoryMap> {
    *MAP.lock_leaf()
}

fn direct_map_pointer(physical: Paddr) -> Option<*mut u8> {
    direct_pointer(physical, FRAME_SIZE)
}

/// Where the direct map puts `len` bytes at `physical`, by arithmetic alone.
///
/// The caller must establish that the map reaches those bytes, as a [`Frame`] or [`Segment`] does.
pub(crate) fn direct_pointer(physical: Paddr, len: u64) -> Option<*mut u8> {
    let offset = DIRECT_MAP_OFFSET.load(Ordering::Acquire);
    if offset == 0 {
        return None;
    }
    let end = physical.checked_add(len)?;
    let _ = offset.checked_add(end)?;
    offset
        .checked_add(physical)
        .map(|address| address as *mut u8)
}
