#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;
use core::num::NonZeroU64;

pub const FRAME_SIZE: u64 = 4096;

const FRAMES_PER_WORD: usize = u64::BITS as usize;

// Biased by one so frame zero and `Option` both fit a word.
pub const fn frame_identity(address: u64) -> Option<NonZeroU64> {
    if !address.is_multiple_of(FRAME_SIZE) {
        return None;
    }
    NonZeroU64::new(address / FRAME_SIZE + 1)
}

pub const fn words_for(bytes: u64) -> usize {
    (bytes.div_ceil(FRAME_SIZE) as usize).div_ceil(FRAMES_PER_WORD)
}

const fn bit(frame: usize) -> u64 {
    1 << (frame % FRAMES_PER_WORD)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FreeError {
    NotManaged(usize),
    AlreadyFree(usize),
}

impl fmt::Display for FreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotManaged(frame) => write!(formatter, "frame {frame} is not managed memory"),
            Self::AlreadyFree(frame) => write!(formatter, "frame {frame} is already free"),
        }
    }
}

impl core::error::Error for FreeError {}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Inserted {
    pub ignored: usize,
}

pub struct FrameSpace<const WORDS: usize> {
    managed: [u64; WORDS],
    // Always a subset of `managed`.
    free: [u64; WORDS],
    free_count: usize,
    managed_count: usize,
    hint: usize,
}

impl<const WORDS: usize> FrameSpace<WORDS> {
    const FRAME_COUNT: usize = WORDS * FRAMES_PER_WORD;

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

    pub const fn is_free(&self, frame: usize) -> bool {
        frame < Self::FRAME_COUNT && self.free[frame / FRAMES_PER_WORD] & bit(frame) != 0
    }

    pub const fn is_managed(&self, frame: usize) -> bool {
        frame < Self::FRAME_COUNT && self.managed[frame / FRAMES_PER_WORD] & bit(frame) != 0
    }

    pub fn insert(&mut self, base: u64, length: u64) -> Inserted {
        let mut inserted = Inserted::default();
        let Some(end) = base.checked_add(length) else {
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
        }
        inserted
    }

    pub fn allocate(&mut self) -> Option<usize> {
        self.allocate_contiguous(1, 1)
    }

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

    fn take(&mut self, frame: usize) {
        if self.is_free(frame) {
            self.free[frame / FRAMES_PER_WORD] &= !bit(frame);
            self.free_count -= 1;
        }
    }

    fn search(&self, from: usize, to: usize, count: usize, align: usize) -> Option<usize> {
        let mut start = align_up(from, align)?;
        for _ in 0..=Self::FRAME_COUNT {
            let end = start.checked_add(count)?;
            if end > to {
                return None;
            }
            match (start..end).find(|&frame| !self.is_free(frame)) {
                None => return Some(start),
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

// Counts only: the bitmaps run to megabytes and a panic handler may print one.
impl<const WORDS: usize> fmt::Debug for FrameSpace<WORDS> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FrameSpace")
            .field("managed", &self.managed_count)
            .field("free", &self.free_count)
            .finish()
    }
}

const fn align_up(value: usize, align: usize) -> Option<usize> {
    let mask = align - 1;
    match value.checked_add(mask) {
        Some(sum) => Some(sum & !mask),
        None => None,
    }
}

#[cfg(test)]
mod tests;
