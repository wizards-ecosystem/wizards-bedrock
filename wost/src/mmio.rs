use core::fmt;

use mmio_range::{AccessError, AccessRange};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum MmioError {
    OutOfBounds {
        offset: usize,
        len: usize,
        region_len: usize,
    },
    Misaligned {
        offset: usize,
        align: usize,
    },
    Overflow {
        offset: usize,
        count: usize,
    },
}

impl fmt::Display for MmioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::OutOfBounds {
                offset,
                len,
                region_len,
            } => write!(
                formatter,
                "access of {len} bytes at offset {offset} leaves a {region_len}-byte region"
            ),
            Self::Misaligned { offset, align } => {
                write!(formatter, "offset {offset} is not {align}-byte aligned")
            }
            Self::Overflow { offset, count } => {
                write!(
                    formatter,
                    "{count} words at offset {offset} cannot be represented"
                )
            }
        }
    }
}

impl core::error::Error for MmioError {}

#[derive(Debug)]
pub struct MmioRegion {
    base: *mut u8,
    len: usize,
}

// SAFETY: a base pointer and a length, kept `!Sync` by the raw pointer so accesses cannot race
// across CPUs; `new` requires the same live mapping in every page-table root that can run.
unsafe impl Send for MmioRegion {}

impl MmioRegion {
    /// # Safety
    ///
    /// `base` must name the same live, 4-byte-aligned mapping of at least `len` bytes in every
    /// page-table root that can run for this region's lifetime, and no other code may access that
    /// memory while this region exists.
    pub(crate) const unsafe fn new(base: *mut u8, len: usize) -> Self {
        Self { base, len }
    }

    pub fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), MmioError> {
        self.write_values(offset, core::iter::once(value))
    }

    pub fn read_u32(&self, offset: usize) -> Result<u32, MmioError> {
        let offset = self.range_of_words(offset, 1)?.offset();
        let word = self.base.wrapping_add(offset).cast::<u32>();
        // SAFETY: `range_of_words` proved the word is contained and aligned in the live mapping
        // promised by `new`.
        Ok(unsafe { word.read_volatile() })
    }

    pub(crate) fn fill_u32(
        &mut self,
        offset: usize,
        count: usize,
        value: u32,
    ) -> Result<(), MmioError> {
        self.write_values(offset, core::iter::repeat_n(value, count))
    }

    pub(crate) fn write_u32_slice(
        &mut self,
        offset: usize,
        values: &[u32],
    ) -> Result<(), MmioError> {
        self.write_values(offset, values.iter().copied())
    }

    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the write goes through a raw pointer into the device mapping this borrow owns"
    )]
    fn write_values(
        &mut self,
        offset: usize,
        values: impl ExactSizeIterator<Item = u32>,
    ) -> Result<(), MmioError> {
        let range = self.range_of_words(offset, values.len())?;
        for (cursor, value) in range.offsets().zip(values) {
            let word = self.base.wrapping_add(cursor).cast::<u32>();
            // SAFETY: `range` proves each word aligned and contained by this unique device mapping.
            unsafe { word.write_volatile(value) };
        }
        Ok(())
    }

    fn range_of_words(&self, offset: usize, count: usize) -> Result<AccessRange, MmioError> {
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
            _ => MmioError::Overflow { offset, count },
        })
    }
}

const WORD: usize = size_of::<u32>();
