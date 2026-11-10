use core::mem::size_of;
use core::sync::atomic::{AtomicU64, Ordering};

use frame_space::{FRAME_SIZE, FrameSpace};

use super::direct::DirectSpan;
use super::dma::{DmaMappingOwner, DmaSegment, IommuTable, PublishedIommuTable, TableWalks};
use super::frame::{Frame, Segment, allocate_contiguous};
use super::shared::USegment;
#[cfg(doc)]
use super::shared::{SharedBytes, SharedRing};
use super::{
    CEILING, FRAMES, MAP, Paddr, direct_map_pointer, direct_pointer, memory_map_snapshot,
    seed_frames,
};
use crate::boot::{MemoryKind, MemoryMap, MemoryRegion};

#[cfg(feature = "selftest")]
/// Check that a reserved overlap is never seeded into the frame allocator.
pub fn overlapping_allocator_claims_are_arbitrated_for_test() -> bool {
    let Ok(map) = MemoryMap::try_from_regions([
        MemoryRegion {
            base: FRAME_SIZE,
            len: FRAME_SIZE * 4,
            kind: MemoryKind::Usable,
        },
        MemoryRegion {
            base: FRAME_SIZE * 2,
            len: FRAME_SIZE,
            kind: MemoryKind::Reserved,
        },
    ]) else {
        return false;
    };
    let mut frames = FrameSpace::<1>::new();
    let ignored = seed_frames(&mut frames, &map);
    let first = frames.allocate();
    let second = frames.allocate();
    let third = frames.allocate();
    ignored == 0
        && frames.managed_count() == 3
        && [first, second, third] == [Some(1), Some(3), Some(4)]
        && frames.allocate().is_none()
        && !frames.is_managed(2)
}

/// Frames available to allocate.
#[cfg(feature = "selftest")]
pub fn free_frames() -> usize {
    FRAMES.lock_leaf().free_count()
}

/// Where the boot map covers `len` bytes with memory, or `None` before it is up.
///
/// See [`crate::boot::MemoryMap::first_memory_run`] for why a caller asks
/// rather than names an address (D-177).
#[cfg(feature = "selftest")]
pub fn first_memory_run(len: u64) -> Option<Paddr> {
    MAP.lock_leaf()
        .as_ref()
        .and_then(|map| map.first_memory_run(len))
}

impl Frame {
    /// Whether every byte of the frame is zero, or `None` if the direct map
    /// cannot reach it.
    ///
    /// This is the machine witness for the D-12 zero-on-allocation boundary.
    #[cfg(feature = "selftest")]
    pub fn is_zero(&self) -> Option<bool> {
        const WORDS: usize = FRAME_SIZE as usize / size_of::<u64>();

        let base = direct_map_pointer(self.address())?.cast::<u64>();
        let mut zero = true;
        for index in 0..WORDS {
            let word = base.wrapping_add(index);
            // SAFETY: the frame `self` owns is allocatable memory and so is
            // direct-mapped whichever map is in force, and `index` stays below
            // the number of words in one frame. The direct map preserves the
            // frame's `u64` alignment.
            zero &= unsafe { word.read_volatile() } == 0;
        }
        Some(zero)
    }
}

impl Segment {
    /// Read `into.len()` bytes from `offset` within the run.
    ///
    /// [`SharedBytes`] uses volatile copies and [`SharedRing`] reserves its
    /// publication words for the architecture boundary. [`USegment`] exposes
    /// no reader (D-107, D-221).
    ///
    /// `false` if the read would run past the run, or if the direct map cannot
    /// reach it, having written nothing into `into`.
    #[cfg(feature = "selftest")]
    #[must_use = "a refused segment read copied no bytes"]
    pub fn read_at(&self, offset: usize, into: &mut [u8]) -> bool {
        let Some(end) = offset.checked_add(into.len()) else {
            return false;
        };
        if end as u64 > self.len() {
            return false;
        }
        let Some(base) = direct_pointer(self.address(), self.len()) else {
            return false;
        };
        let source = base.wrapping_add(offset);
        // SAFETY: the checked read lies in this owned allocator run, which is
        // direct-mapped, and the caller's output cannot overlap it.
        unsafe { core::ptr::copy_nonoverlapping(source, into.as_mut_ptr(), into.len()) };
        true
    }
}

/// Exercise the actual DMA owner and destructor without publishing a leaf.
#[cfg(feature = "selftest")]
pub fn dma_mapping_owner_drop_retains_for_test() -> bool {
    let compact = size_of::<Option<DmaMappingOwner>>() == size_of::<Paddr>();
    let before = free_frames();
    let (Some(mapped), Some(revoked)) = (allocate_contiguous(1, 1), allocate_contiguous(2, 1))
    else {
        return false;
    };
    let owner = DmaMappingOwner::new(mapped.address());
    let foreign = DmaMappingOwner::new(revoked.address());
    let mut mapped = DmaSegment::from(mapped);
    let mut revoked = DmaSegment::from(revoked);
    let mapped_ok = mapped.begin_mapping(owner);
    let refused = !mapped.begin_mapping(owner)
        && !mapped.begin_mapping(foreign)
        && !mapped.release_mapping(foreign)
        && mapped.is_mapped_by(owner);
    let released = !revoked.release_mapping(owner)
        && revoked.begin_mapping(owner)
        && revoked.release_mapping(owner)
        && !revoked.is_mapped_by(owner)
        && !revoked.release_mapping(owner);
    drop(revoked);
    let reclaimed = free_frames() == before - 1;
    drop(mapped);
    compact && mapped_ok && refused && released && reclaimed && free_frames() == before - 1
}

/// Compare ordinary pre-publication cleanup with actual published-owner Drop.
#[cfg(feature = "selftest")]
pub fn published_iommu_table_drop_retains_for_test() -> bool {
    let before = free_frames();
    let Some(unpublished) = IommuTable::new(1, TableWalks::Coherent) else {
        return false;
    };
    let allocated = free_frames() == before - 1;
    drop(unpublished);
    if !allocated || free_frames() != before {
        return false;
    }
    let mut retained = 0;
    for frames in [1, 2, 3, 4, 5] {
        let Some(table) = IommuTable::new(frames, TableWalks::Coherent) else {
            return false;
        };
        let address = table.address();
        let mut published = PublishedIommuTable::new(table);
        let exact = published.address() == address
            && published.write_u64(0, address)
            && published.read_u64(0) == Some(address);
        retained += frames;
        drop(published);
        if !exact || free_frames() != before - retained {
            return false;
        }
    }
    true
}

/// A copy of the boot memory map for machine checks.
#[cfg(feature = "selftest")]
pub fn memory_map_for_test() -> Option<MemoryMap> {
    memory_map_snapshot()
}

#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_INACTIVE: u64 = u64::MAX;
#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_ARMED: u64 = 1;
#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_REFUSED: u64 = 2;
#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_WITHHELD: u64 = 3;
#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_RELEASED: u64 = 4;
#[cfg(feature = "selftest")]
const EXTERNAL_ZERO_PHASE_MASK: u64 = FRAME_SIZE - 1;

#[cfg(feature = "selftest")]
static EXTERNAL_ZERO_REFUSAL: AtomicU64 = AtomicU64::new(EXTERNAL_ZERO_INACTIVE);

#[cfg(feature = "selftest")]
fn external_zero_state(address: Paddr, phase: u64) -> Option<u64> {
    if address >= CEILING
        || !address.is_multiple_of(FRAME_SIZE)
        || phase == 0
        || phase > EXTERNAL_ZERO_PHASE_MASK
    {
        return None;
    }
    Some(address | phase)
}

#[cfg(feature = "selftest")]
pub(super) fn refuse_external_zero_for_test(address: Paddr) -> bool {
    let (Some(armed), Some(refused)) = (
        external_zero_state(address, EXTERNAL_ZERO_ARMED),
        external_zero_state(address, EXTERNAL_ZERO_REFUSED),
    ) else {
        return false;
    };
    EXTERNAL_ZERO_REFUSAL
        .compare_exchange(armed, refused, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

#[cfg(feature = "selftest")]
pub(super) fn observe_external_run_withheld_for_test(address: Paddr) {
    let (Some(refused), Some(withheld)) = (
        external_zero_state(address, EXTERNAL_ZERO_REFUSED),
        external_zero_state(address, EXTERNAL_ZERO_WITHHELD),
    ) else {
        return;
    };
    let _ = EXTERNAL_ZERO_REFUSAL.compare_exchange(
        refused,
        withheld,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

#[cfg(feature = "selftest")]
pub(super) fn observe_external_run_release_for_test(address: Paddr) {
    let Some(released) = external_zero_state(address, EXTERNAL_ZERO_RELEASED) else {
        return;
    };
    loop {
        let current = EXTERNAL_ZERO_REFUSAL.load(Ordering::Acquire);
        if current == EXTERNAL_ZERO_INACTIVE
            || current & !EXTERNAL_ZERO_PHASE_MASK != address
            || current & EXTERNAL_ZERO_PHASE_MASK == EXTERNAL_ZERO_RELEASED
        {
            return;
        }
        if EXTERNAL_ZERO_REFUSAL
            .compare_exchange(current, released, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return;
        }
    }
}

#[cfg(feature = "selftest")]
#[derive(Debug)]
struct ExternalZeroRefusal {
    address: Paddr,
    count: usize,
    active: bool,
}

#[cfg(feature = "selftest")]
impl ExternalZeroRefusal {
    fn arm(segment: &Segment) -> Option<Self> {
        let address = segment.address();
        let armed = external_zero_state(address, EXTERNAL_ZERO_ARMED)?;
        EXTERNAL_ZERO_REFUSAL
            .compare_exchange(
                EXTERNAL_ZERO_INACTIVE,
                armed,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()?;
        Some(Self {
            address,
            count: segment.frames(),
            active: true,
        })
    }

    fn state(&self, phase: u64) -> u64 {
        self.address | phase
    }

    fn is_withheld_with(&self, expected: &[u8]) -> bool {
        if EXTERNAL_ZERO_REFUSAL.load(Ordering::Acquire) != self.state(EXTERNAL_ZERO_WITHHELD) {
            return false;
        }
        let first = (self.address / FRAME_SIZE) as usize;
        let Some(end) = first.checked_add(self.count) else {
            return false;
        };
        let held = {
            let frames = FRAMES.lock_leaf();
            (first..end).all(|frame| frames.is_managed(frame) && !frames.is_free(frame))
        };
        if !held {
            return false;
        }
        let Some(len) = (self.count as u64).checked_mul(FRAME_SIZE) else {
            return false;
        };
        let Ok(expected_len) = u64::try_from(expected.len()) else {
            return false;
        };
        if expected_len > len {
            return false;
        }
        // SAFETY: the refusal state and allocator ledger above prove this exact
        // run is withheld and cannot be reused while the observation runs.
        let Some(span) = (unsafe { DirectSpan::from_proven_run(self.address, len) }) else {
            return false;
        };
        span.starts_with(expected)
    }

    fn recover(&mut self) -> bool {
        if !self.active {
            return false;
        }
        let current = EXTERNAL_ZERO_REFUSAL.load(Ordering::Acquire);
        if current == self.state(EXTERNAL_ZERO_RELEASED) {
            if EXTERNAL_ZERO_REFUSAL
                .compare_exchange(
                    current,
                    EXTERNAL_ZERO_INACTIVE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                self.active = false;
            }
            return false;
        }
        if current != self.state(EXTERNAL_ZERO_WITHHELD) {
            return false;
        }
        if EXTERNAL_ZERO_REFUSAL
            .compare_exchange(
                current,
                EXTERNAL_ZERO_INACTIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }

        // SAFETY: `WITHHELD` is published only after the sole `Segment` handle
        // is forgotten. `Segment::drop` publishes `RELEASED` instead, so this
        // state proves no allocator owner can name these exact frames.
        let mut segment = unsafe { Segment::from_raw(self.address, self.count) };
        if !segment.zero() {
            core::mem::forget(segment);
            EXTERNAL_ZERO_REFUSAL.store(current, Ordering::Release);
            return false;
        }
        drop(segment);
        self.active = false;
        true
    }
}

#[cfg(feature = "selftest")]
impl Drop for ExternalZeroRefusal {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let armed = self.state(EXTERNAL_ZERO_ARMED);
        if EXTERNAL_ZERO_REFUSAL
            .compare_exchange(
                armed,
                EXTERNAL_ZERO_INACTIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.active = false;
            return;
        }
        assert!(
            self.recover(),
            "a refused external clear could not recover its withheld run"
        );
    }
}

/// Force one external run's clear to refuse, then recover it after observation.
#[cfg(feature = "selftest")]
#[must_use = "the refusal proof reports whether the dirty run stayed withheld"]
pub fn refused_external_clear_withholds_for_test(segment: Segment, expected_prefix: &[u8]) -> bool {
    let address = segment.address();
    let count = segment.frames();
    let Some(mut refusal) = ExternalZeroRefusal::arm(&segment) else {
        return false;
    };
    drop(<USegment as From<Segment>>::from(segment));
    let withheld = refusal.is_withheld_with(expected_prefix);
    let recovered = refusal.recover();
    withheld && recovered && freed_external_run_is_zero_for_test(address, count)
}

/// Whether an exact released run is free and contains only zero bytes.
#[cfg(feature = "selftest")]
pub fn freed_external_run_is_zero_for_test(address: Paddr, count: usize) -> bool {
    if count == 0 || !address.is_multiple_of(FRAME_SIZE) {
        return false;
    }
    let Some(len) = (count as u64).checked_mul(FRAME_SIZE) else {
        return false;
    };
    let first = (address / FRAME_SIZE) as usize;
    let Some(end) = first.checked_add(count) else {
        return false;
    };
    let frames = FRAMES.lock_leaf();
    if !(first..end).all(|frame| frames.is_free(frame)) {
        return false;
    }
    // SAFETY: the allocator lock keeps this verified free run from being
    // allocated until the scan completes, and managed frames are direct-mapped.
    let Some(span) = (unsafe { DirectSpan::from_proven_run(address, len) }) else {
        return false;
    };
    span.all_bytes_equal(0)
}
