use frame_space::FRAME_SIZE;

use super::direct::DirectSpan;
use super::frame::{Frame, Segment};
use super::{FRAMES, PhysicalAddress, direct_map_pointer, direct_pointer};

#[cfg(feature = "selftest")]
pub fn free_frames() -> usize {
    FRAMES.lock_leaf().free_count()
}

impl Frame {
    #[cfg(feature = "selftest")]
    pub fn is_zero(&self) -> Option<bool> {
        const WORDS: usize = FRAME_SIZE as usize / size_of::<u64>();

        let base = direct_map_pointer(self.address())?.cast::<u64>();
        let mut zero = true;
        for index in 0..WORDS {
            let word = base.wrapping_add(index);
            // SAFETY: allocatable, the owned frame is direct-mapped under either map; `index` stays
            // below one frame's words, and the direct map keeps the frame's `u64` alignment.
            zero &= unsafe { word.read_volatile() } == 0;
        }
        Some(zero)
    }
}

impl Segment {
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

#[cfg(feature = "selftest")]
pub fn freed_external_run_is_zero_for_test(address: PhysicalAddress, count: usize) -> bool {
    if count == 0 || !address.get().is_multiple_of(FRAME_SIZE) {
        return false;
    }
    let Some(len) = (count as u64).checked_mul(FRAME_SIZE) else {
        return false;
    };
    let first = (address.get() / FRAME_SIZE) as usize;
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
