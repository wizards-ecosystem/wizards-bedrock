//! Physical memory: which frames exist, and who owns each one.
//!
//! A [`Frame`] owns one allocator entry until drop; `frame_space::FrameSpace`
//! supplies the host-tested search. Frames are reached through the direct map
//! whose offset [`init`] records.
//!
//! [`USegment`] marks bytes as externally modifiable. Such bytes cannot become
//! kernel-owned again until [`USegment::reclaim`] clears them (D-44, D-107).

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

/// Highest physical address the frame allocator tracks.
///
/// Two bits of `.bss` per frame, so 16 GiB costs one megabyte — which the
/// image does not carry, because both bitmaps start zeroed. Memory above this
/// is reported as ignored at boot rather than silently lost.
pub const CEILING: u64 = 16 << 30;

/// The bookkeeping, sized for [`CEILING`].
type Frames = FrameSpace<{ words_for(CEILING) }>;

static FRAMES: SpinLock<Frames> = SpinLock::new(Frames::new());

/// The direct map's offset, or zero before [`init`] has run.
///
/// An atomic rather than a field of the locked structure: zeroing a frame must
/// not be done while holding the allocator's lock, and the offset never
/// changes once written. Zero is an unambiguous "not yet" — the higher-half
/// direct map is never at zero.
static DIRECT_MAP_OFFSET: AtomicU64 = AtomicU64::new(0);

static INIT: Once = Once::new();

/// What [`init`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Frames the allocator now manages.
    pub managed: usize,
    /// Frames dropped because they lie above [`CEILING`].
    pub ignored: usize,
}

impl Report {
    /// Bytes under management.
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

/// Take every exclusively allocatable whole-frame run under management.
///
/// The map arbitrates overlaps first, excluding every frame touched by a
/// kernel, boot, firmware, or other non-allocatable claim.
///
/// # Panics
///
/// Panics on a duplicate initialization attempt.
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

/// Allocation-free read-only access to physical firmware tables at handoff.
///
/// This deliberately cannot read usable, bootloader, kernel, reserved, or
/// device memory. It borrows the copied boot map directly so the platform
/// contract can be checked before the allocator records global memory state.
#[derive(Debug, Clone, Copy)]
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

    /// Copy table bytes at `address` into `buffer` with volatile reads.
    #[must_use = "a refused early firmware read copied no bytes"]
    pub fn read(&self, address: Paddr, buffer: &mut [u8]) -> bool {
        if !self.contains(address, buffer.len()) {
            return false;
        }
        let Ok(len) = u64::try_from(buffer.len()) else {
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
        read_firmware_bytes(base as *const u8, buffer);
        true
    }
}

/// Read-only access to physical firmware tables after memory initialization.
///
/// This carries the same narrow authority as [`EarlyFirmwareMemory`] without
/// borrowing the boot handoff after the global map has accepted it.
#[derive(Debug, Clone, Copy)]
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

    /// Copy table bytes at `address` into `buffer` with volatile reads.
    #[must_use = "a refused firmware read copied no bytes"]
    pub fn read(&self, address: Paddr, buffer: &mut [u8]) -> bool {
        if !self.contains(address, buffer.len()) {
            return false;
        }
        let Some(base) = direct_pointer(address, buffer.len() as u64) else {
            return false;
        };
        read_firmware_bytes(base, buffer);
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

fn read_firmware_bytes(base: *const u8, buffer: &mut [u8]) {
    for (index, slot) in buffer.iter_mut().enumerate() {
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

/// The direct map [`init`] recorded, or `None` before it has run.
///
/// Lets a physical-address owner name the same byte for permission readback.
pub fn direct_map() -> Option<DirectMap> {
    match DIRECT_MAP_OFFSET.load(Ordering::Acquire) {
        0 => None,
        offset => Some(DirectMap::new(offset)),
    }
}

/// Reclaim the frame allocator's lock on the terminal panic path.
///
/// A panic can interrupt either lock's holder, so reporting must recover them
/// before it reads allocator state.
pub(crate) fn force_unlock(panic: &PanicToken) {
    FRAMES.force_unlock(panic);
    MAP.force_unlock(panic);
}

/// Copy the boot map so policy queries run after its leaf guard is released.
pub(crate) fn memory_map_snapshot() -> Option<MemoryMap> {
    *MAP.lock_leaf()
}

/// Where the direct map makes one frame at `physical` accessible.
fn direct_map_pointer(physical: Paddr) -> Option<*mut u8> {
    direct_pointer(physical, FRAME_SIZE)
}

/// Where the direct map makes `len` bytes at `physical` accessible.
///
/// Arithmetic only. Whether the map actually *reaches* those bytes is the
/// caller's to establish: [`FirmwareMemory`] checks the memory map, and a
/// [`Frame`] or [`Segment`] carries it by construction.
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
