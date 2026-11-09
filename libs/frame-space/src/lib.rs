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
use core::num::NonZeroU64;

/// Bytes in one page frame.
pub const FRAME_SIZE: u64 = 4096;

/// An aligned frame's distinct nonzero identity, including physical frame zero.
///
/// Unaligned addresses are refused. Even the highest `u64` frame number leaves
/// room for the nonzero bias, so an optional identity still occupies one word.
pub const fn frame_identity(address: u64) -> Option<NonZeroU64> {
    if !address.is_multiple_of(FRAME_SIZE) {
        return None;
    }
    NonZeroU64::new(address / FRAME_SIZE + 1)
}

/// Frames one word of a bitmap accounts for.
const FRAMES_PER_WORD: usize = u64::BITS as usize;

/// Bitmap words needed to cover `bytes` of physical address space.
///
/// The const argument a [`FrameSpace`] takes: `FrameSpace<{ words_for(16 << 30) }>`
/// covers 16 GiB and occupies 1 MiB of `.bss`, two bits per frame — two
/// arrays of 65,536 `u64`, which is what `wost/src/mm/mod.rs` calls "one
/// megabyte" and this line called two.
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

    /// Hand `count` frames starting at `first` back.
    ///
    /// All or nothing: if any frame in the run is not ours or is already free,
    /// nothing is released and the first offending frame is reported. A partial
    /// release would leave a caller's `Drop` believing it had given everything
    /// back.
    ///
    /// A run that overflows or runs off the end reports the first frame past
    /// the space because `FRAME_COUNT` is a ceiling, not an offending frame
    /// (D-105).
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
        for _ in 0..=Self::FRAME_COUNT {
            let end = start.checked_add(count)?;
            if end > to {
                return None;
            }
            match (start..end).find(|&frame| !self.is_free(frame)) {
                // Every frame in the window is free.
                None => return Some(start),
                // Resume past the blocker: no run can straddle it, so the
                // windows in between cannot succeed either.
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
mod tests;
