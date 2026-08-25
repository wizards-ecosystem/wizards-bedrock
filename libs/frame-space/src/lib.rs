//! Allocation bookkeeping for physical page frames.
//!
//! Arithmetic over two bitmaps, with no hardware behind it; `wost::mm` wraps
//! one in a lock and hands out the frame handles (D-43, on `port_space`'s
//! precedent).
//!
//! Two bitmaps, because one could not tell a double free from a frame that was
//! never ours — and [`free`](FrameSpace::free) is what a `Drop` impl calls,
//! the one place a bug is silent.
//!
//! Capacity is a const parameter so the kernel names its ceiling while tests
//! use spaces small enough for a stack. Both start zeroed, so a `static` one
//! lands in `.bss`.
//!
//! The search is first-fit from a rotating hint. Swapping in a buddy changes
//! this file and nothing else.
//!
//! ```
//! use frame_space::{FrameSpace, FRAME_SIZE};
//!
//! // A space covering the first 64 frames of physical memory.
//! let mut space: FrameSpace<1> = FrameSpace::new();
//! space.insert(FRAME_SIZE, FRAME_SIZE * 7);
//!
//! let run = space.allocate_contiguous(2, 2).expect("frames 2 and 3 are free");
//! assert_eq!(run, 2);
//! assert!(space.free_contiguous(run, 2).is_ok());
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

/// Bytes in one page frame.
pub const FRAME_SIZE: u64 = 4096;

/// Frames one word of a bitmap accounts for.
const FRAMES_PER_WORD: usize = u64::BITS as usize;

/// Bitmap words needed to cover `bytes` of physical address space.
///
/// The const argument a [`FrameSpace`] takes: `FrameSpace<{ words_for(16 << 30) }>`
/// covers 16 GiB and occupies 2 MiB of `.bss`, two bits per frame.
pub const fn words_for(bytes: u64) -> usize {
    (bytes.div_ceil(FRAME_SIZE) as usize).div_ceil(FRAMES_PER_WORD)
}

/// The bit standing for `frame` within its word.
const fn bit(frame: usize) -> u64 {
    1 << (frame % FRAMES_PER_WORD)
}

/// Why a frame could not be released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FreeError {
    /// The frame is not one this space manages — it was never usable RAM, or
    /// it lies above the space's ceiling.
    NotManaged(usize),
    /// The frame is managed but already free: a double release.
    AlreadyFree(usize),
}

impl fmt::Display for FreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotManaged(frame) => write!(f, "frame {frame} is not managed memory"),
            Self::AlreadyFree(frame) => write!(f, "frame {frame} is already free"),
        }
    }
}

impl core::error::Error for FreeError {}

/// What one [`insert`](FrameSpace::insert) added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Inserted {
    /// Frames that became managed and free.
    pub added: usize,
    /// Frames dropped because they lie above the space's ceiling.
    pub ignored: usize,
}

/// Which physical frames exist, and which are free.
///
/// `WORDS` sets the ceiling: the space covers `WORDS * 64` frames starting at
/// physical zero. Use [`words_for`] to name it in bytes.
pub struct FrameSpace<const WORDS: usize> {
    /// Set: this frame is usable RAM under our control.
    managed: [u64; WORDS],
    /// Set: this managed frame is not currently allocated. Always a subset of
    /// `managed`.
    free: [u64; WORDS],
    /// Frames currently free — maintained rather than recomputed.
    free_count: usize,
    /// Frames currently managed.
    managed_count: usize,
    /// Where the next first-fit search starts, so repeated single-frame
    /// allocations do not rescan the same full prefix every time.
    hint: usize,
}

impl<const WORDS: usize> FrameSpace<WORDS> {
    /// Frames this space has a bit for.
    pub const FRAME_COUNT: usize = WORDS * FRAMES_PER_WORD;

    /// The first physical address this space does not cover.
    pub const CEILING: u64 = Self::FRAME_COUNT as u64 * FRAME_SIZE;

    /// A space managing nothing.
    pub const fn new() -> Self {
        Self {
            managed: [0; WORDS],
            free: [0; WORDS],
            free_count: 0,
            managed_count: 0,
            hint: 0,
        }
    }

    /// Frames available to allocate right now.
    pub const fn free_count(&self) -> usize {
        self.free_count
    }

    /// Frames under management, allocated or not.
    pub const fn managed_count(&self) -> usize {
        self.managed_count
    }

    /// Whether `frame` is managed and currently free.
    pub const fn is_free(&self, frame: usize) -> bool {
        frame < Self::FRAME_COUNT && self.free[frame / FRAMES_PER_WORD] & bit(frame) != 0
    }

    /// Whether `frame` is usable RAM under our control, allocated or not.
    pub const fn is_managed(&self, frame: usize) -> bool {
        frame < Self::FRAME_COUNT && self.managed[frame / FRAMES_PER_WORD] & bit(frame) != 0
    }

    /// Take the physical byte range `base..base + len` under management, free.
    ///
    /// The range is narrowed to whole frames *inside* it: a region that starts
    /// mid-frame does not donate that frame, because the rest of it belongs to
    /// something else. Frames already managed are left alone, so overlapping
    /// regions in a bootloader's map cannot double-count.
    pub fn insert(&mut self, base: u64, len: u64) -> Inserted {
        let mut inserted = Inserted::default();
        let Some(end) = base.checked_add(len) else {
            return inserted;
        };

        let first = base.div_ceil(FRAME_SIZE);
        let last = end / FRAME_SIZE;
        // Counted arithmetically: a corrupt region length must not become a
        // quadrillion-iteration loop.
        let clamp = last.min(Self::FRAME_COUNT as u64);
        inserted.ignored = last.saturating_sub(clamp.max(first)) as usize;
        for frame in first..clamp {
            let frame = frame as usize;
            if self.is_managed(frame) {
                continue;
            }
            self.managed[frame / FRAMES_PER_WORD] |= bit(frame);
            self.managed_count += 1;
            self.release(frame);
            inserted.added += 1;
        }
        inserted
    }

    /// Allocate one frame.
    pub fn allocate(&mut self) -> Option<usize> {
        self.allocate_contiguous(1, 1)
    }

    /// Allocate `count` consecutive frames whose first frame is a multiple of
    /// `align`, both counted in frames.
    ///
    /// `align` is rounded up to the next power of two, so a caller may pass the
    /// natural alignment it wants without checking. A 2 MiB page needs
    /// `allocate_contiguous(512, 512)`.
    ///
    /// First-fit from a rotating hint, wrapping once. Nothing here promises a
    /// policy — see the module docs.
    pub fn allocate_contiguous(&mut self, count: usize, align: usize) -> Option<usize> {
        if count == 0 || count > Self::FRAME_COUNT {
            return None;
        }
        let align = align.max(1).checked_next_power_of_two()?;

        // The wrap pass reaches `count - 1` past the hint, so a free run
        // straddling it is still found.
        let wrap_end = self.hint.saturating_add(count - 1).min(Self::FRAME_COUNT);
        let found = self
            .search(self.hint, Self::FRAME_COUNT, count, align)
            .or_else(|| self.search(0, wrap_end, count, align))?;
        let end = found.checked_add(count)?;

        for frame in found..end {
            self.take(frame);
        }
        self.hint = end % Self::FRAME_COUNT;
        Some(found)
    }

    /// Hand `count` frames starting at `first` back.
    ///
    /// All or nothing: if any frame in the run is not ours or is already free,
    /// nothing is released and the first offending frame is reported. A partial
    /// release would leave a caller's `Drop` believing it had given everything
    /// back.
    ///
    /// A run that overflows or runs off the end reports the first frame past
    /// the space rather than a frame inside it — the earlier version named
    /// `FRAME_COUNT`, which is a ceiling and not an offending frame (D-105).
    pub fn free_contiguous(&mut self, first: usize, count: usize) -> Result<(), FreeError> {
        if count == 0 {
            return Ok(());
        }
        if first >= Self::FRAME_COUNT {
            return Err(FreeError::NotManaged(first));
        }
        let Some(end) = first.checked_add(count) else {
            return Err(FreeError::NotManaged(Self::FRAME_COUNT));
        };
        if end > Self::FRAME_COUNT {
            // The first frame of the run that is not ours, which is where the
            // space ends when the run starts inside it.
            return Err(FreeError::NotManaged(Self::FRAME_COUNT));
        }

        for frame in first..end {
            self.check_releasable(frame)?;
        }
        for frame in first..end {
            self.release(frame);
        }
        Ok(())
    }

    /// Hand one frame back.
    pub fn free(&mut self, frame: usize) -> Result<(), FreeError> {
        self.check_releasable(frame)?;
        self.release(frame);
        Ok(())
    }

    /// Whether releasing `frame` would be legitimate.
    const fn check_releasable(&self, frame: usize) -> Result<(), FreeError> {
        if !self.is_managed(frame) {
            return Err(FreeError::NotManaged(frame));
        }
        if self.is_free(frame) {
            return Err(FreeError::AlreadyFree(frame));
        }
        Ok(())
    }

    /// Mark a managed frame free, maintaining the count.
    fn release(&mut self, frame: usize) {
        if !self.is_free(frame) {
            self.free[frame / FRAMES_PER_WORD] |= bit(frame);
            self.free_count += 1;
        }
    }

    /// Mark a frame not free, maintaining the count. True if it had been.
    fn take(&mut self, frame: usize) -> bool {
        if self.is_free(frame) {
            self.free[frame / FRAMES_PER_WORD] &= !bit(frame);
            self.free_count -= 1;
            return true;
        }
        false
    }

    /// The first aligned run of `count` free frames in `from..to`.
    fn search(&self, from: usize, to: usize, count: usize, align: usize) -> Option<usize> {
        let mut start = align_up(from, align)?;
        loop {
            let end = start.checked_add(count)?;
            if end > to {
                return None;
            }
            match (start..end).find(|&frame| !self.is_free(frame)) {
                // Every frame in the window is free.
                None => return Some(start),
                // Resume past the blocker: no run can straddle it, so the
                // windows in between cannot succeed either.
                Some(blocker) => start = align_up(blocker.checked_add(1)?, align)?,
            }
        }
    }
}

/// Round `value` up to non-zero power-of-two `align`, without wrapping.
const fn align_up(value: usize, align: usize) -> Option<usize> {
    let mask = align - 1;
    match value.checked_add(mask) {
        Some(sum) => Some(sum & !mask),
        None => None,
    }
}

impl<const WORDS: usize> Default for FrameSpace<WORDS> {
    fn default() -> Self {
        Self::new()
    }
}

/// Counts only. The bitmaps run to megabytes at kernel sizes, and the context
/// most likely to print one is a panic handler (`005-code-conventions.md`).
impl<const WORDS: usize> fmt::Debug for FrameSpace<WORDS> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameSpace")
            .field("managed", &self.managed_count)
            .field("free", &self.free_count)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sixty-four frames: enough to exercise every path, small enough that a
    /// test can hold several on the stack.
    type Small = FrameSpace<1>;

    /// Two words, to cover the case where a run or a scan crosses the boundary
    /// between them — the arithmetic most likely to be wrong.
    type TwoWord = FrameSpace<2>;

    #[test]
    fn words_for_converts_bytes_to_bitmap_words() {
        assert_eq!(words_for(FRAME_SIZE * 64), 1);
        assert_eq!(words_for(16 << 30), 65_536);
        // A partial word still needs covering.
        assert_eq!(words_for(FRAME_SIZE * 65), 2);
        assert_eq!(words_for(FRAME_SIZE * 64 + 1), 2);
        assert_eq!(Small::FRAME_COUNT, 64);
        assert_eq!(Small::CEILING, FRAME_SIZE * 64);
    }

    #[test]
    fn a_new_space_manages_nothing() {
        let space = Small::new();
        assert_eq!(space.free_count(), 0);
        assert_eq!(space.managed_count(), 0);
        assert!(!space.is_managed(0));
        assert!(!space.is_free(0));
    }

    #[test]
    fn insert_takes_only_whole_frames_inside_the_range() {
        let mut space = Small::new();
        // Starts one byte into frame 1 and ends one byte into frame 4, so only
        // frames 2 and 3 lie wholly inside.
        let inserted = space.insert(FRAME_SIZE + 1, FRAME_SIZE * 3);
        assert_eq!(inserted.added, 2);
        assert!(!space.is_managed(1));
        assert!(space.is_managed(2));
        assert!(space.is_managed(3));
        assert!(!space.is_managed(4));
    }

    #[test]
    fn insert_is_idempotent_across_overlapping_regions() {
        let mut space = Small::new();
        assert_eq!(space.insert(0, FRAME_SIZE * 4).added, 4);
        assert_eq!(space.insert(0, FRAME_SIZE * 4).added, 0);
        assert_eq!(space.managed_count(), 4);
        assert_eq!(space.free_count(), 4);
    }

    #[test]
    fn insert_reports_frames_above_the_ceiling() {
        let mut space = Small::new();
        let inserted = space.insert(Small::CEILING - FRAME_SIZE, FRAME_SIZE * 4);
        assert_eq!(inserted.added, 1, "only the last frame under the ceiling");
        assert_eq!(inserted.ignored, 3);
        assert_eq!(space.managed_count(), 1);
    }

    #[test]
    fn insert_survives_a_range_that_would_overflow() {
        let mut space = Small::new();
        assert_eq!(space.insert(u64::MAX - 1, 8), Inserted::default());
    }

    #[test]
    fn allocation_hands_out_managed_frames_and_free_returns_them() {
        let mut space = Small::new();
        space.insert(0, FRAME_SIZE * 4);

        let first = space.allocate().expect("four frames are free");
        assert!(space.is_managed(first));
        assert!(!space.is_free(first));
        assert_eq!(space.free_count(), 3);

        assert_eq!(space.free(first), Ok(()));
        assert!(space.is_free(first));
        assert_eq!(space.free_count(), 4);
    }

    #[test]
    fn allocation_stops_when_the_space_is_empty() {
        let mut space = Small::new();
        space.insert(0, FRAME_SIZE * 2);
        assert!(space.allocate().is_some());
        assert!(space.allocate().is_some());
        assert_eq!(space.allocate(), None);
        assert_eq!(space.free_count(), 0);
    }

    #[test]
    fn a_double_free_and_a_foreign_frame_are_both_refused() {
        let mut space = Small::new();
        space.insert(0, FRAME_SIZE * 2);
        let frame = space.allocate().expect("a frame is free");

        assert_eq!(space.free(frame), Ok(()));
        assert_eq!(space.free(frame), Err(FreeError::AlreadyFree(frame)));
        assert_eq!(space.free(40), Err(FreeError::NotManaged(40)));
        assert_eq!(
            space.free(Small::FRAME_COUNT + 1),
            Err(FreeError::NotManaged(Small::FRAME_COUNT + 1)),
            "a frame past the ceiling is not ours either"
        );
        assert_eq!(space.free_count(), 2, "a refusal changes nothing");
    }

    #[test]
    fn a_contiguous_run_skips_past_a_hole() {
        let mut space = Small::new();
        space.insert(0, FRAME_SIZE * 2);
        space.insert(FRAME_SIZE * 3, FRAME_SIZE * 13);

        let run = space
            .allocate_contiguous(4, 1)
            .expect("frames 3..7 are a clear run");
        assert_eq!(run, 3, "the run before the hole is too short");
        for frame in run..run + 4 {
            assert!(!space.is_free(frame));
        }
    }

    #[test]
    fn a_contiguous_run_honours_alignment() {
        let mut space = TwoWord::new();
        space.insert(FRAME_SIZE, FRAME_SIZE * 63);

        let run = space
            .allocate_contiguous(8, 8)
            .expect("frame 8 starts an aligned run");
        assert_eq!(run, 8);
        assert!(run.is_multiple_of(8));
    }

    #[test]
    fn a_run_may_cross_the_boundary_between_two_bitmap_words() {
        let mut space = TwoWord::new();
        // The only managed run starts at 60 and spans the 64-frame word
        // boundary.
        space.insert(FRAME_SIZE * 60, TwoWord::CEILING - FRAME_SIZE * 60);

        let run = space.allocate_contiguous(8, 1).expect("60..68 is free");
        assert_eq!(run, 60);
        assert!(!space.is_free(63) && !space.is_free(64), "both words moved");
        assert_eq!(space.free_contiguous(run, 8), Ok(()));
        assert!(space.is_free(63) && space.is_free(64));
    }

    #[test]
    fn alignment_is_rounded_up_to_a_power_of_two() {
        let mut space = Small::new();
        space.insert(0, FRAME_SIZE * 64);
        let run = space
            .allocate_contiguous(1, 3)
            .expect("a frame aligned to four exists");
        assert!(run.is_multiple_of(4), "3 rounds up to 4, not down to 2");
    }

    #[test]
    fn a_run_longer_than_the_space_is_refused() {
        let mut space = Small::new();
        space.insert(0, FRAME_SIZE * 4);
        assert_eq!(space.allocate_contiguous(5, 1), None);
        assert_eq!(space.free_count(), 4, "a refused run takes nothing");
        assert_eq!(
            space.allocate_contiguous(0, 1),
            None,
            "zero frames is not a run"
        );
    }

    #[test]
    fn an_impossible_run_count_is_refused_without_wrapping() {
        let mut space = Small::new();
        space.insert(0, Small::CEILING);
        assert_eq!(space.allocate(), Some(0), "leave a non-zero search hint");

        assert_eq!(space.allocate_contiguous(usize::MAX, 1), None);
        assert_eq!(space.free_count(), Small::FRAME_COUNT - 1);
    }

    #[test]
    fn an_alignment_that_cannot_round_to_a_power_of_two_is_refused() {
        let mut space = Small::new();
        space.insert(0, Small::CEILING);
        assert_eq!(space.allocate_contiguous(1, usize::MAX), None);
        assert_eq!(space.free_count(), Small::FRAME_COUNT);
    }

    #[test]
    fn a_release_range_that_wraps_is_refused() {
        let mut space = Small::new();
        space.insert(FRAME_SIZE, FRAME_SIZE);
        let frame = space.allocate().expect("one frame is free");
        assert_eq!(frame, 1);

        assert_eq!(
            space.free_contiguous(frame, usize::MAX),
            Err(FreeError::NotManaged(Small::FRAME_COUNT))
        );
        assert!(!space.is_free(frame), "a refused release changes nothing");
    }

    #[test]
    fn freeing_a_run_is_all_or_nothing() {
        let mut space = Small::new();
        space.insert(0, FRAME_SIZE * 8);
        let run = space.allocate_contiguous(4, 1).expect("eight are free");

        // Frame `run + 4` was never allocated, so the whole release is refused.
        assert_eq!(
            space.free_contiguous(run, 5),
            Err(FreeError::AlreadyFree(run + 4))
        );
        assert!(
            !space.is_free(run),
            "a refused release leaves the run allocated"
        );

        assert_eq!(space.free_contiguous(run, 4), Ok(()));
        assert_eq!(space.free_count(), 8);
    }

    #[test]
    fn the_hint_wraps_so_freed_frames_are_found_again() {
        let mut space = Small::new();
        space.insert(0, FRAME_SIZE * 4);
        let frames: Vec<_> = (0..4).map(|_| space.allocate().expect("free")).collect();
        assert_eq!(space.allocate(), None);

        // Release the lowest frame; the hint is now past it and must wrap.
        space.free(frames[0]).expect("allocated");
        assert_eq!(space.allocate(), Some(frames[0]));
    }

    #[test]
    fn a_free_run_straddling_the_hint_is_found() {
        let mut space = Small::new();
        space.insert(0, Small::CEILING);
        // Drain everything so the hint wraps to zero, then leave the hint in
        // the middle of a free run.
        while space.allocate().is_some() {}
        for frame in 30..34 {
            space.free(frame).expect("allocated");
        }
        assert_eq!(space.allocate_contiguous(2, 1), Some(30));
        space.free(30).expect("allocated");
        space.free(31).expect("allocated");

        // Frames 30..34 are free; the hint sits at 32, inside the run.
        assert_eq!(space.allocate_contiguous(4, 1), Some(30));
    }

    #[test]
    fn insert_of_a_vast_range_returns_promptly() {
        let mut space = Small::new();
        let inserted = space.insert(0, u64::MAX - 1);
        assert_eq!(inserted.added, Small::FRAME_COUNT);
        assert_eq!(
            inserted.ignored,
            ((u64::MAX - 1) / FRAME_SIZE) as usize - Small::FRAME_COUNT
        );
    }

    #[test]
    fn a_sweep_of_the_whole_space_conserves_every_frame() {
        let mut space = TwoWord::new();
        let inserted = space.insert(0, TwoWord::CEILING);
        assert_eq!(inserted.added, TwoWord::FRAME_COUNT);
        assert_eq!(inserted.ignored, 0);
        assert_eq!(space.free_count(), TwoWord::FRAME_COUNT);

        // Drain it entirely, then give it all back, and require the counts to
        // land exactly where they started.
        let mut taken = 0;
        while space.allocate().is_some() {
            taken += 1;
        }
        assert_eq!(taken, TwoWord::FRAME_COUNT);
        assert_eq!(space.free_count(), 0);

        for frame in 0..TwoWord::FRAME_COUNT {
            assert_eq!(space.free(frame), Ok(()));
        }
        assert_eq!(space.free_count(), TwoWord::FRAME_COUNT);
        assert_eq!(space.managed_count(), TwoWord::FRAME_COUNT);
    }
}
