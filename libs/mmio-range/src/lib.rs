#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AddressRange {
    base: u64,
    end: u64,
}

impl AddressRange {
    pub const fn new(base: u64, length: u64) -> Option<Self> {
        if length == 0 {
            return None;
        }
        let Some(end) = base.checked_add(length) else {
            return None;
        };
        Some(Self { base, end })
    }

    pub const fn base(self) -> u64 {
        self.base
    }

    pub const fn end(self) -> u64 {
        self.end
    }
}

pub fn first_fit_by<T>(
    window: AddressRange,
    length: u64,
    align: u64,
    occupied: &[T],
    range_of: impl Fn(&T) -> Option<AddressRange>,
) -> Option<u64> {
    if length == 0 || align == 0 || !align.is_power_of_two() {
        return None;
    }
    let mut cursor = window.base.checked_next_multiple_of(align)?;

    for _ in 0..=occupied.len() {
        let candidate_end = cursor.checked_add(length)?;
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AccessRange {
    offset: usize,
    end: usize,
    width: usize,
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccessError {
    InvalidWidth(usize),
    Misaligned {
        offset: usize,
        align: usize,
    },
    LengthOverflow {
        count: usize,
        width: usize,
    },
    EndOverflow {
        offset: usize,
        len: usize,
    },
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

    pub const fn offset(self) -> usize {
        self.offset
    }

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
