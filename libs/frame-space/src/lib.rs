//! Allocation bookkeeping for physical page frames.
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
use core::num::NonZeroU64;

/// Bytes in one page frame.
pub const FRAME_SIZE: u64 = 4096;

const FRAMES_PER_WORD: usize = u64::BITS as usize;

/// An aligned frame's nonzero identity, biased by one so frame zero and `Option` both fit a word.
pub const fn frame_identity(address: u64) -> Option<NonZeroU64> {
    if !address.is_multiple_of(FRAME_SIZE) {
        return None;
    }
    NonZeroU64::new(address / FRAME_SIZE + 1)
}

/// Bitmap words covering `bytes`; `words_for(16 << 30)` costs 1 MiB of `.bss`, two bits per frame.
pub const fn words_for(bytes: u64) -> usize {
    (bytes.div_ceil(FRAME_SIZE) as usize).div_ceil(FRAMES_PER_WORD)
}

const fn bit(frame: usize) -> u64 {
    1 << (frame % FRAMES_PER_WORD)
}

/// Why a frame could not be released.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum FreeError {
    /// The frame was never usable RAM, or lies above the space's ceiling.
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
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Inserted {
    /// Frames that became managed and free.
    pub added: usize,
    /// Frames dropped because they lie above the space's ceiling.
    pub ignored: usize,
}

/// Which of `WORDS * 64` frames exist and are free; two bitmaps separate double and foreign frees.
pub struct FrameSpace<const WORDS: usize> {
    managed: [u64; WORDS],
    /// Always a subset of `managed`.
    free: [u64; WORDS],
    free_count: usize,
    managed_count: usize,
    /// Where the next first-fit search starts, so single-frame allocations skip the full prefix.
    hint: usize,
}

impl<const WORDS: usize> FrameSpace<WORDS> {
    /// Frames this space has a bit for.
    pub const FRAME_COUNT: usize = WORDS * FRAMES_PER_WORD;

    /// The first physical address this space does not cover.
    pub const CEILING: u64 = Self::FRAME_COUNT as u64 * FRAME_SIZE;

    pub const fn new() -> Self {
        Self {
            managed: [0; WORDS],
            free: [0; WORDS],
            free_count: 0,
            managed_count: 0,
            hint: 0,
        }
    }

    pub const fn free_count(&self) -> usize {
        self.free_count
    }

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

    /// Manage the whole frames inside `base..base + len` as free, leaving managed frames alone.
    pub fn insert(&mut self, base: u64, len: u64) -> Inserted {
        let mut inserted = Inserted::default();
        let Some(end) = base.checked_add(len) else {
            return inserted;
        };

        let first = base.div_ceil(FRAME_SIZE);
        let last = end / FRAME_SIZE;
        // Counted arithmetically so a corrupt region length cannot become a huge loop.
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

    pub fn allocate(&mut self) -> Option<usize> {
        self.allocate_contiguous(1, 1)
    }

    /// First-fit `count` frames from a rotating hint, starting at a multiple of `align` rounded up.
    pub fn allocate_contiguous(&mut self, count: usize, align: usize) -> Option<usize> {
        if count == 0 || count > Self::FRAME_COUNT {
            return None;
        }
        let align = align.max(1).checked_next_power_of_two()?;

        // The wrap pass reaches `count - 1` past the hint, so a run straddling it is found.
        let wrap_end = self
            .hint
            .saturating_add(count.saturating_sub(1))
            .min(Self::FRAME_COUNT);
        let found = self
            .search(self.hint, Self::FRAME_COUNT, count, align)
            .or_else(|| self.search(0, wrap_end, count, align))?;
        let end = found.checked_add(count)?;

        for frame in found..end {
            self.take(frame);
        }
        self.hint = end;
        Some(found)
    }

    /// Release `count` frames all or nothing; a run past the space reports `FRAME_COUNT`.
    pub fn free_contiguous(&mut self, first: usize, count: usize) -> Result<(), FreeError> {
        if count == 0 {
            return Ok(());
        }
        if first >= Self::FRAME_COUNT {
            return Err(FreeError::NotManaged(first));
        }
        let end = match first.checked_add(count) {
            Some(end) if end <= Self::FRAME_COUNT => end,
            _ => return Err(FreeError::NotManaged(Self::FRAME_COUNT)),
        };

        for frame in first..end {
            self.check_releasable(frame)?;
        }
        for frame in first..end {
            self.release(frame);
        }
        Ok(())
    }

    pub fn free(&mut self, frame: usize) -> Result<(), FreeError> {
        self.check_releasable(frame)?;
        self.release(frame);
        Ok(())
    }

    const fn check_releasable(&self, frame: usize) -> Result<(), FreeError> {
        if !self.is_managed(frame) {
            return Err(FreeError::NotManaged(frame));
        }
        if self.is_free(frame) {
            return Err(FreeError::AlreadyFree(frame));
        }
        Ok(())
    }

    fn release(&mut self, frame: usize) {
        if !self.is_free(frame) {
            self.free[frame / FRAMES_PER_WORD] |= bit(frame);
            self.free_count += 1;
        }
    }

    /// Mark a frame not free, returning whether it had been.
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
        for _ in 0..=Self::FRAME_COUNT {
            let end = start.checked_add(count)?;
            if end > to {
                return None;
            }
            match (start..end).find(|&frame| !self.is_free(frame)) {
                None => return Some(start),
                // No run can straddle the blocker, so resume past it.
                Some(blocker) => {
                    let next = align_up(blocker.checked_add(1)?, align)?;
                    if next <= blocker {
                        return None;
                    }
                    start = next;
                }
            }
        }
        None
    }
}

impl<const WORDS: usize> Default for FrameSpace<WORDS> {
    fn default() -> Self {
        Self::new()
    }
}

/// Counts only, since the bitmaps run to megabytes and a panic handler may print one.
impl<const WORDS: usize> fmt::Debug for FrameSpace<WORDS> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameSpace")
            .field("managed", &self.managed_count)
            .field("free", &self.free_count)
            .finish()
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

#[cfg(test)]
mod tests;
