use frame_space::FRAME_SIZE;

#[cfg(doc)]
use super::init;
#[cfg(feature = "selftest")]
use super::selftest::{observe_external_run_release_for_test, refuse_external_zero_for_test};
use super::{FRAMES, Paddr, direct_map_pointer, direct_pointer};

/// One physical frame, owned while this value lives; only allocation makes one, so one owner.
#[derive(PartialEq, Eq, Debug)]
pub struct Frame {
    index: usize,
}

impl Frame {
    /// Rebuild the handle for a frame this crate allocated and forgot.
    ///
    /// # Safety
    ///
    /// `address` must be an [`allocate`] or [`allocate_zeroed`] frame whose handle was forgotten.
    /// No other `Frame` may name it: dropping the result frees it, so a second handle double frees.
    /// A handle over a frame nothing allocated corrupts the allocator's records.
    pub(crate) const unsafe fn from_raw(address: Paddr) -> Self {
        Self {
            index: (address / FRAME_SIZE) as usize,
        }
    }

    /// Where the frame begins in physical memory.
    pub const fn address(&self) -> Paddr {
        self.index as u64 * FRAME_SIZE
    }

    /// Fill every byte of the frame with `byte`, or `false` before [`init`] or past the boot map.
    ///
    /// A reportable refusal, since a silent no-op would hand out a table of the last owner's bytes.
    #[must_use = "a refused frame fill changed no bytes"]
    pub fn fill(&mut self, byte: u8) -> bool {
        let Some(base) = direct_map_pointer(self.address()) else {
            return false;
        };
        // SAFETY: `&mut self` and one allocator handle per frame make this the only access, and
        // allocatable memory is direct-mapped, so the whole frame is reachable through `base`.
        unsafe { core::ptr::write_bytes(base, byte, FRAME_SIZE as usize) };
        true
    }

    /// Fill the frame with zeros.
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

/// A run of consecutive frames with one handle, owned while it lives and released whole on drop.
#[derive(PartialEq, Eq, Debug)]
pub struct Segment {
    first: usize,
    count: usize,
}

impl Segment {
    /// Where the run begins in physical memory.
    pub const fn address(&self) -> Paddr {
        self.first as u64 * FRAME_SIZE
    }

    /// Frames in the run.
    pub const fn frames(&self) -> usize {
        self.count
    }

    /// Bytes in the run.
    #[expect(
        clippy::len_without_is_empty,
        reason = "allocation refuses a zero-frame segment"
    )]
    pub const fn len(&self) -> u64 {
        self.count as u64 * FRAME_SIZE
    }

    /// Fill every byte of the run with `byte`, or `false` having written nothing if unreachable.
    #[must_use = "a refused segment fill changed no bytes"]
    pub fn fill(&mut self, byte: u8) -> bool {
        let Some(base) = direct_pointer(self.address(), self.len()) else {
            return false;
        };
        // SAFETY: the live `Segment` proves every frame admitted and reserved, managed frames are
        // direct-mapped, and `direct_pointer` separately proved the address arithmetic.
        unsafe { core::ptr::write_bytes(base, byte, self.len() as usize) };
        true
    }

    /// Fill the run with zeros.
    #[must_use = "a refused segment clear changed no bytes"]
    pub fn zero(&mut self) -> bool {
        #[cfg(feature = "selftest")]
        if refuse_external_zero_for_test(self.address()) {
            return false;
        }
        self.fill(0)
    }

    /// Write `bytes` at `offset`, the safe loading path before pages turn read-only and executable.
    ///
    /// `false` if the write would run past the run, or if the direct map cannot reach it.
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
}

impl Segment {
    /// Rebuild the handle for a run this crate allocated.
    ///
    /// # Safety
    ///
    /// `count` frames at `address` must be an [`allocate_contiguous`] run, its handle forgotten.
    /// No other `Segment` may name them; dropping each frees them, so two handles double free.
    pub(crate) const unsafe fn from_raw(address: Paddr, count: usize) -> Self {
        Self {
            first: (address / FRAME_SIZE) as usize,
            count,
        }
    }
}

impl Drop for Segment {
    fn drop(&mut self) {
        #[cfg(feature = "selftest")]
        observe_external_run_release_for_test(self.address());
        let released = FRAMES.lock_leaf().free_contiguous(self.first, self.count);
        assert!(
            released.is_ok(),
            "segment ownership ledger refused a live handle: {released:?}"
        );
    }
}

/// Allocate one frame, with unspecified contents.
pub fn allocate() -> Option<Frame> {
    let index = FRAMES.lock_leaf().allocate()?;
    Some(Frame { index })
}

/// Allocate `count` consecutive frames, the first aligned to `align` frames.
///
/// `align` rounds up to a power of two; a 2 MiB page wants `allocate_contiguous(512, 512)`.
pub fn allocate_contiguous(count: usize, align: usize) -> Option<Segment> {
    let first = FRAMES.lock_leaf().allocate_contiguous(count, align)?;
    Some(Segment { first, count })
}

/// Allocate one zeroed frame, returning an unreachable one to the allocator to hide stale bytes.
pub fn allocate_zeroed() -> Option<Frame> {
    let mut frame = allocate()?;
    frame.zero().then_some(frame)
}
