//! Page-rounded physical device extents.

use core::fmt;

use x86_cpuid::physical::PhysicalAddressWidth;

use crate::PAGE_SIZE;

const REGISTER_ALIGNMENT: u64 = size_of::<u32>() as u64;

/// A register range whose complete mapped pages fit the running CPU's width.
///
/// # Examples
///
/// ```
/// use x86_cpuid::physical::{ADDRESS_SIZE_LEAF, PhysicalAddressWidth};
/// use x86_paging::device::DeviceRange;
///
/// let width = PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF, 40).unwrap();
/// let range = DeviceRange::new(0xfee0_0004, 4, width).unwrap();
/// assert_eq!(range.page_start(), 0xfee0_0000);
/// assert_eq!(range.page_end(), 0xfee0_1000);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceRange {
    physical: u64,
    page_start: u64,
    page_end: u64,
}

impl DeviceRange {
    /// Validate one register range and every page its mapping would reach.
    ///
    /// # Errors
    ///
    /// Refuses an empty or non-word-aligned range, arithmetic overflow, or an
    /// extent whose rounded pages reach beyond `MAXPHYADDR`.
    pub const fn new(
        physical: u64,
        len: u64,
        width: PhysicalAddressWidth,
    ) -> Result<Self, DeviceRangeError> {
        if len == 0 {
            return Err(DeviceRangeError::Empty);
        }
        if !physical.is_multiple_of(REGISTER_ALIGNMENT) {
            return Err(DeviceRangeError::Unaligned(physical));
        }
        let Some(end) = physical.checked_add(len) else {
            return Err(DeviceRangeError::Overflow);
        };
        let page_start = physical - physical % PAGE_SIZE;
        let remainder = end % PAGE_SIZE;
        let page_end = if remainder == 0 {
            end
        } else {
            let Some(rounded) = end.checked_add(PAGE_SIZE - remainder) else {
                return Err(DeviceRangeError::Overflow);
            };
            rounded
        };
        if !width.contains(page_start, page_end) {
            return Err(DeviceRangeError::OutsideWidth {
                bits: width.bits(),
                page_start,
                page_end,
            });
        }
        Ok(Self {
            physical,
            page_start,
            page_end,
        })
    }

    /// The first requested register byte.
    pub const fn physical(self) -> u64 {
        self.physical
    }

    /// The first physical page the mapping reaches.
    pub const fn page_start(self) -> u64 {
        self.page_start
    }

    /// One byte past the last physical page the mapping reaches.
    pub const fn page_end(self) -> u64 {
        self.page_end
    }
}

/// Why a physical register range cannot be represented safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceRangeError {
    /// No register byte was requested.
    Empty,
    /// The first register is not word aligned.
    Unaligned(u64),
    /// The requested or rounded exclusive end overflowed.
    Overflow,
    /// At least one reached page needs an unimplemented physical-address bit.
    OutsideWidth {
        /// Reported `MAXPHYADDR`.
        bits: u8,
        /// First reached page.
        page_start: u64,
        /// Exclusive end of the reached pages.
        page_end: u64,
    },
}

impl fmt::Display for DeviceRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty => core::fmt::Write::write_str(formatter, "the device range is empty"),
            Self::Unaligned(at) => write!(formatter, "device address {at:#x} is not word aligned"),
            Self::Overflow => core::fmt::Write::write_str(formatter, "the device range overflows"),
            Self::OutsideWidth {
                bits,
                page_start,
                page_end,
            } => write!(
                formatter,
                "device pages {page_start:#x}..{page_end:#x} do not fit MAXPHYADDR {bits}"
            ),
        }
    }
}

impl core::error::Error for DeviceRangeError {}

#[cfg(test)]
mod tests {
    use x86_cpuid::physical::ADDRESS_SIZE_LEAF;

    use super::*;

    fn width(bits: u32) -> PhysicalAddressWidth {
        PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF, bits).unwrap()
    }

    #[test]
    fn each_runtime_width_accepts_its_last_page_and_refuses_the_next() {
        for bits in [36, 40, 46, 52] {
            let limit = 1_u64 << bits;
            let accepted = DeviceRange::new(limit - PAGE_SIZE, PAGE_SIZE, width(bits)).unwrap();
            assert_eq!(accepted.page_end(), limit);
            assert!(matches!(
                DeviceRange::new(limit - 4, 8, width(bits)),
                Err(DeviceRangeError::OutsideWidth { .. })
            ));
        }
    }

    #[test]
    fn a_short_register_range_expands_to_the_complete_page() {
        let physical = PAGE_SIZE + 4;
        let len = 4;
        let range = DeviceRange::new(physical, len, width(36)).unwrap();
        assert_eq!(range.page_start(), PAGE_SIZE);
        assert_eq!(range.page_end(), 2 * PAGE_SIZE);
        let requested_end = physical + len;
        let outside_request = (range.page_start()..range.page_end())
            .filter(|byte| *byte < physical || *byte >= requested_end)
            .count();
        assert_eq!(outside_request, (PAGE_SIZE - len) as usize);
    }

    #[test]
    fn empty_unaligned_wrapping_and_rounding_overflow_are_refused() {
        assert_eq!(
            DeviceRange::new(0, 0, width(52)),
            Err(DeviceRangeError::Empty)
        );
        assert_eq!(
            DeviceRange::new(1, 4, width(52)),
            Err(DeviceRangeError::Unaligned(1))
        );
        assert_eq!(
            DeviceRange::new(u64::MAX - 3, 8, width(52)),
            Err(DeviceRangeError::Overflow)
        );
        assert_eq!(
            DeviceRange::new(u64::MAX - 7, 4, width(52)),
            Err(DeviceRangeError::Overflow)
        );
    }
}
