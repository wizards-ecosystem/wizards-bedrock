use core::fmt;

use x86_paging::{EntryError, MappingRangeError};

use super::Released;
use crate::mm::{PhysicalAddress, USegment};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum MapError {
    OutOfMemory,
    Unreachable(PhysicalAddress),
    Conflict(u64),
    NonCanonical(u64),
    Misaligned(u64),
    UnalignedDevice(u64),
    Encoding(EntryError),
    InvalidRange(MappingRangeError),
    InvalidDeviceRange(u64),
    PhysicalWidthUnavailable,
    OutsidePhysicalWidth { bits: u8, physical: u64 },
    NotDeviceMemory(u64),
    DeviceOverlap(u64),
    TooManyDevices,
    Undescribed(u64),
    NotMapped(u64),
    NotOwned(u64),
    Unsplittable(u64),
    Discontiguous(u64),
    NotYours(u64),
    Pinned(u64),
    CacheAlias(u64),
    WriteExecute(u64),
    Inactive(PhysicalAddress),
    KernelTopologyFrozen(u64),
    Activated(u64),
    AllocatableAlias(u64),
    InvalidEntryEnclave(u64),
    EntryEnclaveUnavailable,
}

impl From<MappingRangeError> for MapError {
    fn from(reason: MappingRangeError) -> Self {
        Self::InvalidRange(reason)
    }
}

impl fmt::Display for MapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::OutOfMemory => formatter.write_str("no free frame for a page table"),
            Self::Unreachable(address) => {
                write!(
                    formatter,
                    "page table at {address:#x} is outside the direct map"
                )
            }
            Self::Conflict(address) => write!(formatter, "{address:#x} is already mapped"),
            Self::NonCanonical(address) => {
                write!(formatter, "{address:#x} is not a canonical address")
            }
            Self::Misaligned(address) => write!(formatter, "{address:#x} is not page-aligned"),
            Self::UnalignedDevice(address) => {
                write!(
                    formatter,
                    "device registers at {address:#x} are not four-byte aligned"
                )
            }
            Self::Encoding(reason) => write!(formatter, "{reason}"),
            Self::InvalidRange(reason) => write!(formatter, "{reason}"),
            Self::InvalidDeviceRange(address) => {
                write!(
                    formatter,
                    "device range at {address:#x} is empty or overflows"
                )
            }
            Self::PhysicalWidthUnavailable => {
                formatter.write_str("the processor did not report a usable MAXPHYADDR")
            }
            Self::OutsidePhysicalWidth { bits, physical } => write!(
                formatter,
                "device range at {physical:#x} needs bits outside MAXPHYADDR {bits}"
            ),
            Self::NotDeviceMemory(address) => {
                write!(
                    formatter,
                    "device range at {address:#x} overlaps ordinary memory"
                )
            }
            Self::DeviceOverlap(address) => {
                write!(formatter, "device range at {address:#x} is already owned")
            }
            Self::TooManyDevices => formatter.write_str("device ownership table is full"),
            Self::Undescribed(address) => {
                write!(formatter, "{address:#x} is not described by the memory map")
            }
            Self::NotMapped(address) => write!(formatter, "nothing is mapped at {address:#x}"),
            Self::NotOwned(address) => {
                write!(
                    formatter,
                    "the mapping at {address:#x} does not own its frame"
                )
            }
            Self::Unsplittable(address) => {
                write!(
                    formatter,
                    "{address:#x} is inside a large page, which is not split"
                )
            }
            Self::Discontiguous(address) => {
                write!(formatter, "the frames behind {address:#x} are not one run")
            }
            Self::NotYours(address) => {
                write!(formatter, "{address:#x} is not in the half this space owns")
            }
            Self::Pinned(address) => {
                write!(
                    formatter,
                    "the hardware-pinned stack at {address:#x} cannot be unmapped"
                )
            }
            Self::CacheAlias(address) => write!(
                formatter,
                "shared mapping at {address:#x} conflicts with the direct-map cache policy"
            ),
            Self::WriteExecute(address) => {
                write!(
                    formatter,
                    "mapping at {address:#x} is writable and executable"
                )
            }
            Self::Inactive(root) => {
                write!(formatter, "the address space at {root:#x} is not active")
            }
            Self::KernelTopologyFrozen(address) => write!(
                formatter,
                "the active kernel cannot add a top-level mapping at {address:#x}"
            ),
            Self::Activated(address) => write!(
                formatter,
                "the kernel's tables are active, so no boot region maps at {address:#x}"
            ),
            Self::AllocatableAlias(address) => write!(
                formatter,
                "allocatable memory maps only at its direct-map address, not {address:#x}"
            ),
            Self::InvalidEntryEnclave(address) => {
                write!(formatter, "the entry enclave is invalid at {address:#x}")
            }
            Self::EntryEnclaveUnavailable => {
                formatter.write_str("the entry enclave is unavailable")
            }
        }
    }
}

impl core::error::Error for MapError {}

#[derive(PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum UserMapError {
    Mapping { reason: MapError, run: USegment },
}

impl UserMapError {
    pub const fn reason(&self) -> MapError {
        match self {
            Self::Mapping { reason, .. } => *reason,
        }
    }
}

impl fmt::Display for UserMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.reason(), formatter)
    }
}

impl core::error::Error for UserMapError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ActivateError {
    AlreadyActive,
    Reclaimed,
    CodeUnmapped(u64),
    CodeNotExecutable(u64),
    StackUnmapped(u64),
    StackReadOnly(u64),
    StackUnsafe(u64),
    MappingChanged(u64),
    ImageChanged(u64),
    DirectMapGap(u64),
    KernelHalfDiffers(usize),
    UserHalfDiffers(usize),
    NotAUserSpace,
    EntryEnclaveIncomplete,
    SharedTopology(MapError),
    NoExecuteDisabled,
    UserKernelEntry(usize),
    HandoffUnmapped(u64),
}

impl fmt::Display for ActivateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::AlreadyActive => formatter.write_str("an address space is already installed"),
            Self::Reclaimed => formatter.write_str("the space has been reclaimed"),
            Self::CodeUnmapped(address) => {
                write!(
                    formatter,
                    "the switch's own code at {address:#x} is unmapped"
                )
            }
            Self::CodeNotExecutable(address) => {
                write!(
                    formatter,
                    "the switch's own code at {address:#x} is not executable"
                )
            }
            Self::StackUnmapped(address) => {
                write!(formatter, "the stack at {address:#x} is unmapped")
            }
            Self::StackReadOnly(address) => {
                write!(formatter, "the stack at {address:#x} is read-only")
            }
            Self::StackUnsafe(address) => {
                write!(
                    formatter,
                    "the stack at {address:#x} has unsafe permissions"
                )
            }
            Self::MappingChanged(address) => {
                write!(
                    formatter,
                    "the mapping at {address:#x} changes physical memory"
                )
            }
            Self::ImageChanged(address) => {
                write!(
                    formatter,
                    "the kernel image mapping at {address:#x} is wrong"
                )
            }
            Self::DirectMapGap(address) => {
                write!(
                    formatter,
                    "the direct map at {address:#x} is missing or wrong"
                )
            }
            Self::KernelHalfDiffers(index) => write!(
                formatter,
                "top-level slot {index} of the higher half is not the kernel's"
            ),
            Self::UserHalfDiffers(index) => {
                write!(
                    formatter,
                    "top-level slot {index} does not share one user subtree"
                )
            }
            Self::NotAUserSpace => {
                formatter.write_str("this space does not share the kernel's higher half")
            }
            Self::EntryEnclaveIncomplete => {
                formatter.write_str("the paired address space has no validated entry enclave")
            }
            Self::SharedTopology(reason) => {
                write!(
                    formatter,
                    "shared kernel topology could not be reserved: {reason}"
                )
            }
            Self::NoExecuteDisabled => formatter
                .write_str("EFER.NXE is clear, so the tables' no-execute bits are reserved"),
            Self::UserKernelEntry(index) => {
                write!(
                    formatter,
                    "top-level slot {index} of the higher half is user-accessible"
                )
            }
            Self::HandoffUnmapped(address) => write!(
                formatter,
                "the framebuffer or firmware tables at {address:#x} are not direct-mapped"
            ),
        }
    }
}

impl core::error::Error for ActivateError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ReclaimError {
    Reclaimed,
    StillBound(PhysicalAddress),
    WrongRelease {
        expected: PhysicalAddress,
        found: PhysicalAddress,
    },
    UnexpectedRelease(PhysicalAddress),
    Active(PhysicalAddress),
    Mapped(usize),
    Devices(usize),
}

impl fmt::Display for ReclaimError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Reclaimed => formatter.write_str("the address space was already reclaimed"),
            Self::StillBound(root) => {
                write!(formatter, "a task still names the space at {root:#x}")
            }
            Self::WrongRelease { expected, found } => write!(
                formatter,
                "release evidence names {found:#x}, not the space at {expected:#x}"
            ),
            Self::UnexpectedRelease(found) => write!(
                formatter,
                "release evidence for {found:#x} was supplied to an unbound space"
            ),
            Self::Active(root) => write!(formatter, "the CPU is walking the space at {root:#x}"),
            Self::Mapped(pages) => write!(formatter, "{pages} owned page(s) remain mapped"),
            Self::Devices(mappings) => {
                write!(formatter, "{mappings} device mapping(s) remain live")
            }
        }
    }
}

impl core::error::Error for ReclaimError {}

#[derive(Debug)]
#[must_use = "a refused address-space reclaim may return release authority"]
pub struct ReclaimFailure {
    pub reason: ReclaimError,
    pub released: Option<Released>,
}

impl fmt::Display for ReclaimFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.reason, formatter)
    }
}

impl core::error::Error for ReclaimFailure {}
