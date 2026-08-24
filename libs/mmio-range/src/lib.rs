//! Checked address arithmetic for repeated MMIO accesses.
//!
//! Hardware access belongs in `wost`; deciding whether a repeated access fits
//! is pure arithmetic and is host-tested here.
//!
//! ```
//! let span = mmio_range::AccessRange::repeated(8, 3, 4, 32).unwrap();
//! assert_eq!(span.len(), 12);
//! assert_eq!(span.end(), 20);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

/// A checked byte range inside a bounded region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessRange {
    offset: usize,
    len: usize,
    end: usize,
}

/// Why a repeated access does not name a valid range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessError {
    /// An access width must be a non-zero power of two.
    InvalidWidth(usize),
    /// The first access is not naturally aligned.
    Misaligned {
        /// Requested byte offset.
        offset: usize,
        /// Required alignment.
        align: usize,
    },
    /// `count * width` does not fit in `usize`.
    LengthOverflow {
        /// Number of repeated accesses.
        count: usize,
        /// Bytes in each access.
        width: usize,
    },
    /// Adding the repeated length to its offset wraps.
    EndOverflow {
        /// Requested byte offset.
        offset: usize,
        /// Checked byte length.
        len: usize,
    },
    /// The range extends beyond the containing region.
    OutOfBounds {
        /// Requested byte offset.
        offset: usize,
        /// Checked byte length.
        len: usize,
        /// Bytes in the containing region.
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
    /// Check `count` naturally aligned accesses of `width` bytes.
    ///
    /// # Errors
    ///
    /// Refuses invalid widths, misalignment, arithmetic overflow, and any
    /// range whose exclusive end is past `region_len`.
    pub const fn repeated(
        offset: usize,
        count: usize,
        width: usize,
        region_len: usize,
    ) -> Result<Self, AccessError> {
        if width == 0 || !width.is_power_of_two() {
            return Err(AccessError::InvalidWidth(width));
        }
        if !offset.is_multiple_of(width) {
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
        Ok(Self { offset, len, end })
    }

    /// First byte in the access range.
    pub const fn offset(self) -> usize {
        self.offset
    }

    /// Bytes covered by all accesses.
    pub const fn len(self) -> usize {
        self.len
    }

    /// Whether no bytes are accessed.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Exclusive end byte.
    pub const fn end(self) -> usize {
        self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exactly_bounded_span_is_accepted() {
        assert_eq!(
            AccessRange::repeated(4, 3, 4, 16),
            Ok(AccessRange {
                offset: 4,
                len: 12,
                end: 16,
            })
        );
    }

    #[test]
    fn invalid_widths_are_refused() {
        assert_eq!(
            AccessRange::repeated(0, 1, 0, 4),
            Err(AccessError::InvalidWidth(0))
        );
        assert_eq!(
            AccessRange::repeated(0, 1, 3, 4),
            Err(AccessError::InvalidWidth(3))
        );
    }

    #[test]
    fn a_misaligned_start_is_refused() {
        assert_eq!(
            AccessRange::repeated(2, 1, 4, 8),
            Err(AccessError::Misaligned {
                offset: 2,
                align: 4,
            })
        );
    }

    #[test]
    fn a_repeated_length_that_wraps_is_refused() {
        assert_eq!(
            AccessRange::repeated(0, usize::MAX, 4, usize::MAX),
            Err(AccessError::LengthOverflow {
                count: usize::MAX,
                width: 4,
            })
        );
    }

    #[test]
    fn an_end_that_wraps_is_refused() {
        let offset = usize::MAX - 3;
        assert_eq!(
            AccessRange::repeated(offset, 2, 4, usize::MAX),
            Err(AccessError::EndOverflow { offset, len: 8 })
        );
    }

    #[test]
    fn a_span_past_the_region_is_refused() {
        assert_eq!(
            AccessRange::repeated(4, 2, 4, 8),
            Err(AccessError::OutOfBounds {
                offset: 4,
                len: 8,
                region_len: 8,
            })
        );
    }

    #[test]
    fn an_empty_span_is_valid_only_at_or_before_the_end() {
        assert!(AccessRange::repeated(8, 0, 4, 8).is_ok());
        assert!(matches!(
            AccessRange::repeated(12, 0, 4, 8),
            Err(AccessError::OutOfBounds { .. })
        ));
    }
}
