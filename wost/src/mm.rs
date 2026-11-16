mod direct;
mod dma;
mod firmware;
mod frame;
pub mod heap;
#[cfg(feature = "selftest")]
mod selftest;
mod shared;

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use frame_space::{FRAME_SIZE, FrameSpace, words_for};

pub use address::PhysicalAddress;

pub(crate) use dma::{DmaMappingOwner, IommuTable, PublishedIommuTable};
pub use dma::{DmaSegment, TableWalks};
pub use firmware::{EarlyFirmwareMemory, FirmwareMemory, firmware_memory};
pub use frame::{Frame, Segment, allocate, allocate_contiguous, allocate_zeroed};
#[cfg(feature = "selftest")]
pub use selftest::{free_frames, is_freed_run_zero};
pub(crate) use shared::SharedKeepalive;
pub use shared::{Shared, SharedBytes, SharedRing, USegment};

use crate::boot::{BootMemory, DirectMap, MemoryKind, MemoryMap};
use crate::panic::PanicToken;
use crate::sync::SpinLock;

// Two zeroed `.bss` bits per frame, so 16 GiB costs one megabyte the image does not carry.
pub(crate) const CEILING: u64 = 16 << 30;

type Frames = FrameSpace<{ words_for(CEILING) }>;

static FRAMES: SpinLock<Frames> = SpinLock::new(Frames::new());

// Zero until `init`; the validated higher-half offset is never zero and never changes.
static DIRECT_MAP_OFFSET: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Report {
    pub managed: usize,
    pub ignored: usize,
}

impl fmt::Display for Report {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} MiB usable in {} frames",
            self.managed as u64 * FRAME_SIZE / (1 << 20),
            self.managed
        )?;
        if self.ignored != 0 {
            write!(
                formatter,
                ", {} frames above the {} GiB ceiling ignored",
                self.ignored,
                CEILING >> 30
            )?;
        }
        Ok(())
    }
}

pub fn init(memory: BootMemory) -> Report {
    let map = memory.map();
    DIRECT_MAP_OFFSET.store(memory.direct().offset(), Ordering::Release);
    *MAP.lock_leaf() = Some(*map);

    let mut frames = FRAMES.lock_leaf();
    let ignored = seed_frames(&mut frames, map);
    let managed = frames.managed_count();
    drop(frames);

    Report { managed, ignored }
}

fn seed_frames(frames: &mut Frames, map: &MemoryMap) -> usize {
    let mut ignored = 0;
    for run in map.allocatable_runs() {
        ignored += frames.insert(run.base(), run.bytes()).ignored;
    }
    ignored
}

pub(crate) fn device_range_is_eligible(start: u64, end: u64) -> bool {
    let guard = MAP.lock_leaf();
    let Some(map) = guard.as_ref() else {
        return false;
    };
    map.allows_device_mapping(start, end)
}

static MAP: SpinLock<Option<MemoryMap>> = SpinLock::new(None);

pub(crate) fn map_direct_cache(start: u64, end: u64) -> Option<crate::boot::DirectMapCache> {
    MAP.lock_leaf()
        .as_ref()
        .and_then(|map| map.direct_map_cache(start, end))
}

pub(crate) fn is_kernel_image(start: u64, len: u64) -> bool {
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    MAP.lock_leaf().as_ref().is_some_and(|map| {
        map.regions()
            .filter(|region| {
                region.len != 0
                    && region.base < end
                    && start < region.base.saturating_add(region.len)
            })
            .all(|region| region.kind == MemoryKind::KernelAndModules)
    })
}

pub fn direct_map() -> Option<DirectMap> {
    match DIRECT_MAP_OFFSET.load(Ordering::Acquire) {
        0 => None,
        offset => Some(DirectMap::new(offset)),
    }
}

pub(crate) fn force_unlock(panic: &PanicToken) {
    FRAMES.force_unlock(panic);
    MAP.force_unlock(panic);
}

pub(crate) fn memory_map_snapshot() -> Option<MemoryMap> {
    *MAP.lock_leaf()
}

fn direct_map_pointer(physical: PhysicalAddress) -> Option<*mut u8> {
    direct_pointer(physical, FRAME_SIZE)
}

// The caller must establish that the map reaches those bytes, as a `Frame` or `Segment` does.
pub(crate) fn direct_pointer(physical: PhysicalAddress, len: u64) -> Option<*mut u8> {
    let offset = DIRECT_MAP_OFFSET.load(Ordering::Acquire);
    if offset == 0 {
        return None;
    }
    let end = physical.get().checked_add(len)?;
    let _ = offset.checked_add(end)?;
    offset
        .checked_add(physical.get())
        .map(|address| address as *mut u8)
}
