use frame_space::FRAME_SIZE;

use super::{FRAMES, PhysicalAddress, direct_map_pointer, direct_pointer};

const fn frame_address(index: usize) -> PhysicalAddress {
    match PhysicalAddress::new(index as u64 * FRAME_SIZE) {
        Some(address) => address,
        None => panic!("a managed frame lies beyond the physical address limit"),
    }
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
        // SAFETY: `&mut self` and one allocator handle per frame make this the only access; every
        // managed frame is direct-mapped, by the bootloader and then by ours (`direct_map_gap`).
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
        let released = FRAMES.lock_leaf().free(self.index);
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
        // SAFETY: the live `Segment` proves every frame reserved to it, managed frames are direct-
        // mapped by the bootloader and then by ours (`direct_map_gap`), and the arithmetic fits.
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
        // SAFETY: the checked write lies in this uniquely owned allocator run,
        // which is direct-mapped, and the caller's bytes cannot overlap it.
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
}

impl Drop for Segment {
    fn drop(&mut self) {
        let released = FRAMES.lock_leaf().free_contiguous(self.first, self.count);
        assert!(
            released.is_ok(),
            "segment ownership ledger refused a live handle: {released:?}"
        );
    }
}

pub fn allocate() -> Option<Frame> {
    let index = FRAMES.lock_leaf().allocate()?;
    Some(Frame { index })
}

// `align` is in frames and rounds up to a power of two.
pub fn allocate_contiguous(count: usize, align: usize) -> Option<Segment> {
    let first = FRAMES.lock_leaf().allocate_contiguous(count, align)?;
    Some(Segment { first, count })
}

pub fn allocate_zeroed() -> Option<Frame> {
    let mut frame = allocate()?;
    frame.zero().then_some(frame)
}
