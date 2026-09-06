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

/// A region of device memory, accessed with volatile operations.
#[derive(Debug)]
pub struct MmioRegion {
    base: *mut u8,
    len: usize,
}

// SAFETY: `MmioRegion` is a base pointer plus a length, and the raw pointer
// keeps it `!Sync`, so accesses cannot race across CPUs. `new` requires the
// same live mapping in every page-table root that can run.
unsafe impl Send for MmioRegion {}

impl MmioRegion {
    /// Wrap `len` bytes of device memory at `base`.
    ///
    /// # Safety
    ///
    /// `base` must name the same live, 4-byte-aligned mapping of at least `len`
    /// bytes in every page-table root that can run for this region's lifetime,
    /// and no other code may access that memory while this region exists.
    pub(crate) const unsafe fn new(base: *mut u8, len: usize) -> Self {
        Self { base, len }
    }

    /// The address the region begins at.
    ///
    /// For a caller that needs to describe the same memory another way — the
    /// framebuffer, which is remapped with a different memory type once the
    /// kernel owns its page tables.
    #[cfg(feature = "selftest")]
    pub(crate) const fn base_address(&self) -> *mut u8 {
        self.base
    }

    /// Write one 32-bit word at `offset`.
    pub fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), MmioError> {
        self.write_values(offset, core::iter::once(value))
    }

    /// Read one 32-bit word at `offset`.
    pub fn read_u32(&self, offset: usize) -> Result<u32, MmioError> {
        let offset = self.check_words(offset, 1)?.offset();
        let word = self.base.wrapping_add(offset).cast::<u32>();
        // SAFETY: `check_words` proved the word is contained and aligned in the
        // live mapping promised by `new`.
        Ok(unsafe { word.read_volatile() })
    }

    /// Write `count` copies of a 32-bit word starting at `offset`.
    pub fn fill_u32(&mut self, offset: usize, count: usize, value: u32) -> Result<(), MmioError> {
        self.write_values(offset, core::iter::repeat_n(value, count))
    }

    /// Write a slice of 32-bit words starting at `offset`.
    pub fn write_u32_slice(&mut self, offset: usize, values: &[u32]) -> Result<(), MmioError> {
        self.write_values(offset, values.iter().copied())
    }

    fn write_values(
        &mut self,
        offset: usize,
        values: impl ExactSizeIterator<Item = u32>,
    ) -> Result<(), MmioError> {
        let range = self.check_words(offset, values.len())?;
        for (cursor, value) in range.offsets().zip(values) {
            let word = self.base.wrapping_add(cursor).cast::<u32>();
            // SAFETY: `range` proves each word is aligned and contained by
            // this unique device mapping.
            unsafe { word.write_volatile(value) };
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
}

/// Bytes in the only access width this region supports.
const WORD: usize = size_of::<u32>();
