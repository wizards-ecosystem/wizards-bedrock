//! Memory-mapped I/O regions.
//!
//! An [`MmioRegion`] is constructed once from a mapping the caller vouches for,
//! then hands out bounds- and alignment-checked volatile accesses in safe code.
//! Every access returns a [`Result`]: a driver bug becomes an error value, not
//! a stray write into whatever follows the device.

use core::fmt;
use mmio_range::{AccessError, AccessRange};

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
    /// Repeated-access arithmetic overflowed before a range could be formed.
    Overflow {
        /// Byte offset the access started at.
        offset: usize,
        /// Number of words requested.
        count: usize,
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
            Self::Overflow { offset, count } => {
                write!(f, "{count} words at offset {offset} cannot be represented")
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
        self.check_words(offset, 1)?;
        // SAFETY: `check_words` confirmed the word is in bounds and 4-byte aligned.
        unsafe { self.write_word_unchecked(offset, value) };
        Ok(())
    }

    /// Read one 32-bit word at `offset`.
    pub fn read_u32(&self, offset: usize) -> Result<u32, MmioError> {
        self.check_words(offset, 1)?;
        // SAFETY: `check_words` confirmed the word is inside the region, so the
        // result stays within the mapping `new`'s contract promised.
        let word = unsafe { self.base.add(offset) }.cast::<u32>();
        // SAFETY: `check_words` confirmed `offset` is 4-byte aligned, so `word` is
        // aligned, and the mapping is valid for reads.
        Ok(unsafe { word.read_volatile() })
    }

    /// Write `count` copies of a 32-bit word starting at `offset`.
    pub fn fill_u32(&mut self, offset: usize, count: usize, value: u32) -> Result<(), MmioError> {
        let span = self.check_words(offset, count)?;
        let mut cursor = span.offset();
        for _ in 0..count {
            // SAFETY: `check_words` confirmed the whole span is in bounds and the
            // start is 4-byte aligned, so every word within it is too.
            unsafe { self.write_word_unchecked(cursor, value) };
            cursor += WORD;
        }
        Ok(())
    }

    /// Confirm repeated word accesses are in bounds and aligned.
    fn check_words(&self, offset: usize, count: usize) -> Result<AccessRange, MmioError> {
        AccessRange::repeated(offset, count, WORD, self.len).map_err(|error| match error {
            AccessError::Misaligned { offset, align } => MmioError::Misaligned { offset, align },
            AccessError::OutOfBounds {
                offset,
                len,
                region_len,
            } => MmioError::OutOfBounds {
                offset,
                len,
                region_len,
            },
            // Overflow and a bad width alike: the request never named a range.
            _ => MmioError::Overflow { offset, count },
        })
    }

    /// # Safety
    ///
    /// `offset` must be 4-byte aligned and `offset + 4` must be within the
    /// region — that is, [`Self::check_words`] must have accepted it.
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
