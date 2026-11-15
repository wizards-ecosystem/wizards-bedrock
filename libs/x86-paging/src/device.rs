use core::fmt;

use x86_cpuid::physical::PhysicalAddressWidth;

use crate::PAGE_SIZE;

const REGISTER_ALIGNMENT: u64 = size_of::<u32>() as u64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceRange {
    physical: u64,
    page_start: u64,
    page_end: u64,
}

impl DeviceRange {
    pub const fn new(
        physical: u64,
        length: u64,
        width: PhysicalAddressWidth,
    ) -> Result<Self, DeviceRangeError> {
        if length == 0 {
            return Err(DeviceRangeError::Empty);
        }
        if !physical.is_multiple_of(REGISTER_ALIGNMENT) {
            return Err(DeviceRangeError::Unaligned(physical));
        }
        let Some(end) = physical.checked_add(length) else {
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

    pub const fn physical(self) -> u64 {
        self.physical
    }

    pub const fn page_start(self) -> u64 {
        self.page_start
    }

    pub const fn page_end(self) -> u64 {
        self.page_end
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum DeviceRangeError {
    Empty,
    Unaligned(u64),
    Overflow,
    OutsideWidth {
        bits: u8,
        page_start: u64,
        page_end: u64,
    },
}

impl fmt::Display for DeviceRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty => formatter.write_str("the device range is empty"),
            Self::Unaligned(at) => write!(formatter, "device address {at:#x} is not word aligned"),
            Self::Overflow => formatter.write_str("the device range overflows"),
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
