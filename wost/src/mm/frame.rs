use frame_space::{Access, ColourMask, FRAME_SIZE, Owner};

use super::{FRAMES, PhysicalAddress, colours, direct_map_pointer, direct_pointer, table};
use crate::arch::current::paging::{DirectAccess, Flush, set_direct_access};

pub(super) const fn frame_address(index: usize) -> PhysicalAddress {
    match PhysicalAddress::new(index as u64 * FRAME_SIZE) {
        Some(address) => address,
        None => panic!("a managed frame lies beyond the physical address limit"),
    }
}

// Kernel and table frames come back here; arena and sealed frames have their own release.
fn release_record(frame: usize) {
    let previous = table::release(frame);
    assert!(
        matches!(previous.owner(), Owner::Kernel | Owner::Table)
            && previous.access() == Access::Writable,
        "frame {frame} was freed as a kernel frame while it is {previous:?}"
    );
}

#[derive(PartialEq, Eq, Debug)]
pub struct Frame {
    index: usize,
}

impl Frame {
    /// # Safety
    ///
    /// `address` must be an [`allocate`] or [`allocate_zeroed`] frame whose handle was forgotten.
    /// No other `Frame` may name it: dropping the result frees it, so a second handle double frees.
    /// A handle over a frame nothing allocated corrupts the allocator's records.
    pub(crate) const unsafe fn from_raw(address: PhysicalAddress) -> Self {
        Self {
            index: (address.get() / FRAME_SIZE) as usize,
        }
    }

    pub const fn address(&self) -> PhysicalAddress {
        frame_address(self.index)
    }

    #[must_use = "a refused frame fill changed no bytes"]
    pub fn fill(&mut self, byte: u8) -> bool {
        let Some(base) = direct_map_pointer(self.address()) else {
            return false;
        };
        // SAFETY: `&mut self` and one allocator handle per frame make this the only access; a
        // kernel frame is never sealed, so its 4 KiB direct-map leaf stays writable.
        unsafe { core::ptr::write_bytes(base, byte, FRAME_SIZE as usize) };
        true
    }

    #[must_use = "a refused frame clear changed no bytes"]
    pub fn zero(&mut self) -> bool {
        self.fill(0)
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        let mut frames = FRAMES.lock_leaf();
        release_record(self.index);
        let released = frames.space.free(self.index);
        assert!(
            released.is_ok(),
            "frame ownership ledger refused a live handle: {released:?}"
        );
    }
}

#[derive(PartialEq, Eq, Debug)]
pub struct Segment {
    first: usize,
    count: usize,
}

impl Segment {
    pub const fn address(&self) -> PhysicalAddress {
        frame_address(self.first)
    }

    pub const fn frames(&self) -> usize {
        self.count
    }

    #[expect(
        clippy::len_without_is_empty,
        reason = "allocation refuses a zero-frame segment"
    )]
    pub const fn len(&self) -> u64 {
        self.count as u64 * FRAME_SIZE
    }

    #[must_use = "a refused segment fill changed no bytes"]
    pub fn fill(&mut self, byte: u8) -> bool {
        let Some(base) = direct_pointer(self.address(), self.len()) else {
            return false;
        };
        // SAFETY: the live `Segment` proves every frame reserved to it, a sealed run is held only
        // by `SealedSegment`, so these direct-map leaves are writable, and the arithmetic fits.
        unsafe { core::ptr::write_bytes(base, byte, self.len() as usize) };
        true
    }

    #[must_use = "a refused segment clear changed no bytes"]
    pub fn zero(&mut self) -> bool {
        self.fill(0)
    }

    #[must_use = "a refused segment write changed no bytes"]
    pub fn write_at(&mut self, offset: usize, bytes: &[u8]) -> bool {
        let Some(end) = offset.checked_add(bytes.len()) else {
            return false;
        };
        if end as u64 > self.len() {
            return false;
        }
        let Some(base) = direct_pointer(self.address(), self.len()) else {
            return false;
        };
        let destination = base.wrapping_add(offset);
        // SAFETY: the checked write lies in this uniquely owned allocator run, whose direct-map
        // leaves are writable while a `Segment` holds it, and the caller's bytes cannot overlap it.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
        true
    }

    /// # Safety
    ///
    /// Each of the `count` frames at `address` must be allocated, owned by the caller through a
    /// handle it forgot, and named by no other `Frame` or `Segment`: the drop frees every one.
    pub(crate) const unsafe fn from_raw(address: PhysicalAddress, count: usize) -> Self {
        Self {
            first: (address.get() / FRAME_SIZE) as usize,
            count,
        }
    }

    const fn indices(&self) -> core::ops::Range<usize> {
        self.first..self.first + self.count
    }
}

impl Drop for Segment {
    fn drop(&mut self) {
        let mut frames = FRAMES.lock_leaf();
        for frame in self.indices() {
            release_record(frame);
        }
        let released = frames.space.free_contiguous(self.first, self.count);
        assert!(
            released.is_ok(),
            "segment ownership ledger refused a live handle: {released:?}"
        );
    }
}

pub(crate) fn is_sealed(frame: PhysicalAddress) -> bool {
    table::read((frame.get() / FRAME_SIZE) as usize).access() == Access::Sealed
}

// Seals each frame's record, then drops its direct-map leaf to read-only.
pub(crate) fn seal_run(segment: &Segment, flush: &mut Flush) {
    let _frames = FRAMES.lock_leaf();
    for frame in segment.indices() {
        table::seal(frame);
        flush.merge(set_direct_access(
            frame_address(frame),
            DirectAccess::ReadOnly,
        ));
    }
}

// The record goes free before the leaf turns writable, so no sealed frame is ever writable.
pub(crate) fn release_sealed_run(segment: Segment, flush: &mut Flush) {
    let segment = core::mem::ManuallyDrop::new(segment);
    {
        let _frames = FRAMES.lock_leaf();
        for frame in segment.indices() {
            let previous = table::release(frame);
            assert!(
                previous.access() == Access::Sealed,
                "frame {frame} was released as sealed while it is {previous:?}"
            );
            flush.merge(set_direct_access(
                frame_address(frame),
                DirectAccess::Writable,
            ));
        }
    }
    let Some(base) = direct_pointer(segment.address(), segment.len()) else {
        panic!("a sealed run lost its direct-map address");
    };
    // SAFETY: the run is still taken in the bitmap and no handle but this one names it, and its
    // leaves were made writable above.
    unsafe { core::ptr::write_bytes(base, 0, segment.len() as usize) };
    let released = FRAMES
        .lock_leaf()
        .space
        .free_contiguous(segment.first, segment.count);
    assert!(
        released.is_ok(),
        "a sealed run's ledger refused its release: {released:?}"
    );
}

/// # Safety
///
/// The caller must hold the only handle to frame `index`, and its direct-map leaf must be writable.
pub(super) unsafe fn zero_frame(index: usize) {
    let Some(base) = direct_map_pointer(frame_address(index)) else {
        panic!("managed frame {index} has no direct-map address");
    };
    // SAFETY: the caller's sole handle makes this the only access, and its leaf is writable.
    unsafe { core::ptr::write_bytes(base, 0, FRAME_SIZE as usize) };
}

fn allocate_owned(owner: Owner) -> Option<Frame> {
    let colours = colours();
    let mut frames = FRAMES.lock_leaf();
    let frames = &mut *frames;
    let index =
        frames
            .space
            .allocate_unreserved(colours, &frames.pool, ColourMask::full(colours))?;
    table::claim(index, owner);
    Some(Frame { index })
}

pub fn allocate() -> Option<Frame> {
    allocate_owned(Owner::Kernel)
}

// `align` is in frames and rounds up to a power of two.
pub fn allocate_contiguous(count: usize, align: usize) -> Option<Segment> {
    let colours = colours();
    let mut frames = FRAMES.lock_leaf();
    let frames = &mut *frames;
    let first = frames
        .space
        .allocate_contiguous_unreserved(colours, &frames.pool, count, align)?;
    for frame in first..first + count {
        table::claim(frame, Owner::Kernel);
    }
    Some(Segment { first, count })
}

pub fn allocate_zeroed() -> Option<Frame> {
    let mut frame = allocate()?;
    frame.zero().then_some(frame)
}

pub(crate) fn allocate_table() -> Option<Frame> {
    let mut frame = allocate_owned(Owner::Table)?;
    frame.zero().then_some(frame)
}
