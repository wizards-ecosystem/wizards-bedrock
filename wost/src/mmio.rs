//! Memory-mapped I/O regions.
//!
//! An [`MmioRegion`] is constructed once from a mapping the caller vouches for,
//! then hands out bounds- and alignment-checked volatile accesses in safe code.
//! Every access returns a [`Result`]: a driver bug becomes an error value, not
//! a stray write into whatever follows the device.

use core::fmt;

/// Why an [`MmioRegion`] access was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MmioError {
    /// The access would extend past the end of the region.
    OutOfBounds {
        /// Byte offset the access started at.
        offset: usize,
        /// Length of the access in bytes.
        len: usize,
        /// Size of the region in bytes.
        region_len: usize,
    },
    /// The offset is not aligned for an access of this width.
    Misaligned {
        /// Byte offset the access started at.
        offset: usize,
        /// Alignment the access required.
        align: usize,
    },
}

impl fmt::Display for MmioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::OutOfBounds {
                offset,
                len,
                region_len,
            } => write!(
                f,
                "access of {len} bytes at offset {offset} leaves a {region_len}-byte region"
            ),
            Self::Misaligned { offset, align } => {
                write!(f, "offset {offset} is not {align}-byte aligned")
            }
        }
    }
}

impl core::error::Error for MmioError {}

/// A region of device memory, accessed with volatile writes.
#[derive(Debug)]
pub struct MmioRegion {
    base: *mut u8,
    len: usize,
}

// SAFETY: `MmioRegion` is a base pointer plus a length, and the raw pointer
// keeps it `!Sync`, so accesses cannot race across CPUs. Moving the pair is
// sound provided the mapping is global, which `new`'s contract requires.
unsafe impl Send for MmioRegion {}

impl MmioRegion {
    /// Wrap `len` bytes of device memory at `base`.
    ///
    /// # Safety
    ///
    /// `base` must be a globally valid, 4-byte-aligned mapping of at least
    /// `len` bytes that stays live for as long as this region, and no other
    /// code may access that memory while this region exists.
    pub const unsafe fn new(base: *mut u8, len: usize) -> Self {
        Self { base, len }
    }

    /// The address the region begins at.
    ///
    /// For a caller that needs to describe the same memory another way — the
    /// framebuffer, which is remapped with a different memory type once the
    /// kernel owns its page tables.
    pub const fn base_address(&self) -> *mut u8 {
        self.base
    }

    /// Write one 32-bit word at `offset`.
    pub fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), MmioError> {
        self.check(offset, WORD)?;
        // SAFETY: `check` confirmed the word is in bounds and 4-byte aligned.
        unsafe { self.write_word_unchecked(offset, value) };
        Ok(())
    }

    /// Read one 32-bit word at `offset`.
    pub fn read_u32(&self, offset: usize) -> Result<u32, MmioError> {
        self.check(offset, WORD)?;
        // SAFETY: `check` confirmed the word is inside the region, so the
        // result stays within the mapping `new`'s contract promised.
        let word = unsafe { self.base.add(offset) }.cast::<u32>();
        // SAFETY: `check` confirmed `offset` is 4-byte aligned, so `word` is
        // aligned, and the mapping is valid for reads.
        Ok(unsafe { word.read_volatile() })
    }

    /// Write `count` copies of a 32-bit word starting at `offset`.
    pub fn fill_u32(&mut self, offset: usize, count: usize, value: u32) -> Result<(), MmioError> {
        self.check(offset, count.saturating_mul(WORD))?;
        for index in 0..count {
            // SAFETY: `check` confirmed the whole span is in bounds and the
            // start is 4-byte aligned, so every word within it is too.
            unsafe { self.write_word_unchecked(offset + index * WORD, value) };
        }
        Ok(())
    }

    /// Confirm that a `len`-byte access at `offset` is in bounds and aligned.
    fn check(&self, offset: usize, len: usize) -> Result<(), MmioError> {
        if !offset.is_multiple_of(WORD) {
            return Err(MmioError::Misaligned {
                offset,
                align: WORD,
            });
        }
        let out_of_bounds = MmioError::OutOfBounds {
            offset,
            len,
            region_len: self.len,
        };
        match offset.checked_add(len) {
            Some(end) if end <= self.len => Ok(()),
            _ => Err(out_of_bounds),
        }
    }

    /// # Safety
    ///
    /// `offset` must be 4-byte aligned and `offset + 4` must be within the
    /// region — that is, [`Self::check`] must have accepted it.
    unsafe fn write_word_unchecked(&mut self, offset: usize, value: u32) {
        // SAFETY: the caller guarantees `offset` is inside the region, so the
        // result stays within the mapping `new`'s contract promised.
        let word = unsafe { self.base.add(offset) }.cast::<u32>();
        // SAFETY: the caller guarantees `offset` is 4-byte aligned, so `word` is
        // aligned, and the mapping is valid for writes.
        unsafe { word.write_volatile(value) };
    }
}

/// Bytes in the only access width this region supports.
const WORD: usize = size_of::<u32>();
