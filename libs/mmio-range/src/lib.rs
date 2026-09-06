//! Checked address arithmetic for repeated MMIO accesses.
//!
//! Hardware access belongs in `wost`; deciding whether a repeated access fits
//! is pure arithmetic and is host-tested here.
//!
//! ```
//! let span = mmio_range::AccessRange::repeated(8, 3, 4, 32).unwrap();
//! assert_eq!(span.offset(), 8);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

/// One non-empty half-open address range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        let end = match base.checked_add(len) {
            Some(end) => end,
            None => return None,
        };
        let end = match end.checked_next_multiple_of(align) {
            Some(end) => end,
            None => return None,
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

/// Find the lowest aligned gap after projecting each occupied record.
///
/// This keeps a bounded ownership ledger in its native representation rather
/// than requiring a second array of ranges beside it.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessRange {
    offset: usize,
    end: usize,
    width: usize,
}

/// The exact offsets authorized by an [`AccessRange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// range whose exclusive end is past `region_len`. A zero count is an
    /// empty no-op, so its offset need not be aligned.
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
mod tests {
    use super::*;

    #[test]
    fn address_ranges_refuse_empty_and_wrapping_bounds() {
        assert_eq!(AddressRange::new(0x1000, 0), None);
        assert_eq!(AddressRange::new(u64::MAX, 1), None);
        assert_eq!(
            AddressRange::new(u64::MAX, 0),
            None,
            "empty-at-the-end is still empty"
        );
        assert_eq!(
            AddressRange::new(u64::MAX - 1, 1),
            Some(AddressRange {
                base: u64::MAX - 1,
                end: u64::MAX,
            })
        );
    }

    #[test]
    fn a_covering_range_refuses_rounding_past_the_address_space() {
        assert_eq!(
            AddressRange::covering(0x1234, 1, 0x1000),
            Some(AddressRange {
                base: 0x1000,
                end: 0x2000,
            })
        );
        assert_eq!(
            AddressRange::covering(u64::MAX - 0x0fff, 0x0fff, 0x1000),
            None
        );
        assert_eq!(AddressRange::covering(0x1000, 0, 0x1000), None);
        assert_eq!(AddressRange::covering(0x1000, 1, 3), None);
    }

    #[test]
    fn first_fit_refuses_invalid_request_geometry() {
        let window = AddressRange::new(0x1000, 0x1000).expect("window");
        assert_eq!(first_fit_by(window, 0, 0x1000, &[], |range| *range), None);
        assert_eq!(first_fit_by(window, 0x1000, 0, &[], |range| *range), None);
        assert_eq!(first_fit_by(window, 0x1000, 3, &[], |range| *range), None);
    }

    #[test]
    fn first_fit_accounts_for_ranges_crossing_the_window_edges() {
        let window = AddressRange::new(4, 4).expect("window");
        let crosses_end = AddressRange::new(5, 7).expect("occupied");
        let after_end = AddressRange::new(8, 4).expect("occupied");

        assert_eq!(
            first_fit_by(window, 2, 1, &[Some(crosses_end)], |range| *range),
            None
        );
        assert_eq!(
            first_fit_by(window, 2, 1, &[Some(after_end)], |range| *range),
            Some(4)
        );
    }

    #[test]
    fn first_fit_reuses_an_interior_gap_in_any_active_order() {
        let window = AddressRange::new(0x1000, 0x4000).expect("window");
        let first = AddressRange::new(0x1000, 0x1000).expect("first");
        let second = AddressRange::new(0x2000, 0x1000).expect("second");

        assert_eq!(
            first_fit_by(window, 0x1000, 0x1000, &[Some(second)], |range| *range),
            Some(0x1000)
        );
        assert_eq!(
            first_fit_by(
                window,
                0x1000,
                0x1000,
                &[Some(second), Some(first)],
                |range| *range,
            ),
            Some(0x3000)
        );
        assert_eq!(
            first_fit_by(
                window,
                0x1000,
                0x1000,
                &[Some(first), Some(second)],
                |range| *range,
            ),
            Some(0x3000)
        );
    }

    #[test]
    fn an_access_range_reports_its_own_offset() {
        const REGION: usize = 0x1000;
        const OFFSET: usize = 0x40;
        const COUNT: usize = 4;
        const WIDTH: usize = 4;

        let range = AccessRange::repeated(OFFSET, COUNT, WIDTH, REGION).expect("inside the region");

        assert_eq!(range.offset(), OFFSET, "offset() gave something else");
    }

    #[test]
    fn an_exactly_bounded_span_is_accepted() {
        assert_eq!(
            AccessRange::repeated(4, 3, 4, 16),
            Ok(AccessRange {
                offset: 4,
                end: 16,
                width: 4,
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
    fn a_span_one_byte_past_the_region_is_refused() {
        assert!(AccessRange::repeated(0, 1, 8, 8).is_ok());
        assert!(AccessRange::repeated(4, 1, 4, 8).is_ok());
        assert_eq!(
            AccessRange::repeated(8, 1, 1, 8),
            Err(AccessError::OutOfBounds {
                offset: 8,
                len: 1,
                region_len: 8,
            })
        );
        assert_eq!(
            AccessRange::repeated(1, 1, 1, 1),
            Err(AccessError::OutOfBounds {
                offset: 1,
                len: 1,
                region_len: 1,
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

    #[test]
    fn errors_name_the_refused_access() {
        let cases = [
            (AccessError::InvalidWidth(3), "access width 3 is invalid"),
            (
                AccessError::Misaligned {
                    offset: 6,
                    align: 4,
                },
                "offset 6 is not 4-byte aligned",
            ),
            (
                AccessError::LengthOverflow { count: 9, width: 8 },
                "9 accesses of 8 bytes overflow",
            ),
            (
                AccessError::EndOverflow { offset: 15, len: 2 },
                "access of 2 bytes at 15 wraps",
            ),
            (
                AccessError::OutOfBounds {
                    offset: 8,
                    len: 4,
                    region_len: 10,
                },
                "access of 4 bytes at 8 leaves a 10-byte region",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }
}
