//! Checked address arithmetic for repeated MMIO accesses, host-tested here while `wost` performs them.
//!
//! ```
//! let span = mmio_range::AccessRange::repeated(8, 3, 4, 32).unwrap();
//! assert_eq!(span.offset(), 8);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

/// One non-empty half-open address range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AddressRange {
    base: u64,
    end: u64,
}

impl AddressRange {
    /// Build `base..base + len` when its end is representable and non-empty.
    pub const fn new(base: u64, len: u64) -> Option<Self> {
        if len == 0 {
            return None;
        }
        let Some(end) = base.checked_add(len) else {
            return None;
        };
        Some(Self { base, end })
    }

    /// Smallest aligned range covering `base..base + len`.
    pub const fn covering(base: u64, len: u64, align: u64) -> Option<Self> {
        if len == 0 || align == 0 || !align.is_power_of_two() {
            return None;
        }
        let Some(end) = base.checked_add(len) else {
            return None;
        };
        let Some(end) = end.checked_next_multiple_of(align) else {
            return None;
        };
        Some(Self {
            base: base & !(align - 1),
            end,
        })
    }

    /// First byte in the range.
    pub const fn base(self) -> u64 {
        self.base
    }

    /// One past the final byte.
    pub const fn end(self) -> u64 {
        self.end
    }
}

/// Find the lowest aligned gap, projecting each occupied record so a ledger needs no second array of ranges.
pub fn first_fit_by<T>(
    window: AddressRange,
    len: u64,
    align: u64,
    occupied: &[T],
    range_of: impl Fn(&T) -> Option<AddressRange>,
) -> Option<u64> {
    if len == 0 || align == 0 || !align.is_power_of_two() {
        return None;
    }
    let mut cursor = window.base.checked_next_multiple_of(align)?;

    for _ in 0..=occupied.len() {
        let candidate_end = cursor.checked_add(len)?;
        if candidate_end > window.end {
            return None;
        }

        let mut blocked_until = cursor;
        for item in occupied {
            let Some(range) = range_of(item) else {
                continue;
            };
            if cursor < range.end && range.base < candidate_end {
                blocked_until = blocked_until.max(range.end);
            }
        }
        if blocked_until == cursor {
            return Some(cursor);
        }
        cursor = blocked_until.checked_next_multiple_of(align)?;
    }
    None
}

/// A checked byte range inside a bounded region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AccessRange {
    offset: usize,
    end: usize,
    width: usize,
}

/// The exact offsets authorized by an [`AccessRange`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AccessOffsets {
    next: usize,
    remaining: usize,
    width: usize,
}

impl Iterator for AccessOffsets {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let offset = self.next;
        self.remaining -= 1;
        self.next += self.width;
        Some(offset)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for AccessOffsets {}
impl core::iter::FusedIterator for AccessOffsets {}

/// Why a repeated access does not name a valid range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum AccessError {
    /// An access width must be a non-zero power of two.
    InvalidWidth(usize),
    /// The first access is not naturally aligned.
    Misaligned { offset: usize, align: usize },
    /// `count * width` does not fit in `usize`.
    LengthOverflow { count: usize, width: usize },
    /// Adding the repeated length to its offset wraps.
    EndOverflow { offset: usize, len: usize },
    /// The range extends beyond the containing region.
    OutOfBounds {
        offset: usize,
        len: usize,
        region_len: usize,
    },
}

impl fmt::Display for AccessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidWidth(width) => write!(formatter, "access width {width} is invalid"),
            Self::Misaligned { offset, align } => {
                write!(formatter, "offset {offset} is not {align}-byte aligned")
            }
            Self::LengthOverflow { count, width } => {
                write!(formatter, "{count} accesses of {width} bytes overflow")
            }
            Self::EndOverflow { offset, len } => {
                write!(formatter, "access of {len} bytes at {offset} wraps")
            }
            Self::OutOfBounds {
                offset,
                len,
                region_len,
            } => write!(
                formatter,
                "access of {len} bytes at {offset} leaves a {region_len}-byte region"
            ),
        }
    }
}

impl core::error::Error for AccessError {}

impl AccessRange {
    /// Check `count` naturally aligned `width`-byte accesses; a zero count accesses nothing, so needs no alignment.
    pub const fn repeated(
        offset: usize,
        count: usize,
        width: usize,
        region_len: usize,
    ) -> Result<Self, AccessError> {
        if width == 0 || !width.is_power_of_two() {
            return Err(AccessError::InvalidWidth(width));
        }
        if count != 0 && !offset.is_multiple_of(width) {
            return Err(AccessError::Misaligned {
                offset,
                align: width,
            });
        }
        let Some(len) = count.checked_mul(width) else {
            return Err(AccessError::LengthOverflow { count, width });
        };
        let Some(end) = offset.checked_add(len) else {
            return Err(AccessError::EndOverflow { offset, len });
        };
        if end > region_len {
            return Err(AccessError::OutOfBounds {
                offset,
                len,
                region_len,
            });
        }
        Ok(Self { offset, end, width })
    }

    /// First byte in the access range.
    pub const fn offset(self) -> usize {
        self.offset
    }

    /// Each access start in order, with no unchecked arithmetic left to the caller.
    pub const fn offsets(self) -> AccessOffsets {
        AccessOffsets {
            next: self.offset,
            remaining: (self.end - self.offset) / self.width,
            width: self.width,
        }
    }
}

#[cfg(test)]
mod tests;
