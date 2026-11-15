use core::fmt;

use crate::{AcpiError, Sdt, u16_at, u64_at};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DmaCoverage {
    segment: u16,
    register_base: u64,
}

impl DmaCoverage {
    pub const LEN: usize = Sdt::LEN + 12;

    pub fn new(table: &[u8]) -> Result<Self, DmaCoverageError> {
        let header = Sdt::parse(table).map_err(DmaCoverageError::Table)?;
        if header.signature() != Sdt::DMA_COVERAGE {
            return Err(DmaCoverageError::WrongTable {
                expected: Sdt::DMA_COVERAGE,
                found: header.signature(),
            });
        }
        if header.length() as usize != Self::LEN {
            return Err(DmaCoverageError::WrongLength(header.length()));
        }
        if table[8] != 1 {
            return Err(DmaCoverageError::UnsupportedRevision(table[8]));
        }
        let reserved =
            u16_at(table, Sdt::LEN + 2).ok_or(DmaCoverageError::WrongLength(header.length()))?;
        if reserved != 0 {
            return Err(DmaCoverageError::NonzeroReserved(reserved));
        }
        let segment =
            u16_at(table, Sdt::LEN).ok_or(DmaCoverageError::WrongLength(header.length()))?;
        let register_base =
            u64_at(table, Sdt::LEN + 4).ok_or(DmaCoverageError::WrongLength(header.length()))?;
        Ok(Self {
            segment,
            register_base,
        })
    }

    pub const fn segment(self) -> u16 {
        self.segment
    }

    pub const fn register_base(self) -> u64 {
        self.register_base
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DmaCoverageError {
    Table(AcpiError),
    WrongTable { expected: [u8; 4], found: [u8; 4] },
    WrongLength(u32),
    UnsupportedRevision(u8),
    NonzeroReserved(u16),
}

impl fmt::Display for DmaCoverageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Table(reason) => write!(formatter, "DMA coverage table is invalid: {reason}"),
            Self::WrongTable { expected, found } => {
                write!(
                    formatter,
                    "table signature {found:?} does not match {expected:?}"
                )
            }
            Self::WrongLength(length) => {
                write!(
                    formatter,
                    "DMA coverage table length {length} is not 48 bytes"
                )
            }
            Self::UnsupportedRevision(revision) => {
                write!(formatter, "DMA coverage revision {revision} is unsupported")
            }
            Self::NonzeroReserved(bits) => {
                write!(formatter, "DMA coverage reserved bits are {bits:#x}")
            }
        }
    }
}

impl core::error::Error for DmaCoverageError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DmaHandoff {
    segment: u16,
}

impl DmaHandoff {
    pub const LEN: usize = Sdt::LEN + 12;

    pub const REQUIRED_FLAGS: u16 = handoff::COMPLETE_LEGACY_CONFIGURATION
        | handoff::QUIESCENT_THROUGH_FENCE
        | handoff::STATIC_REQUESTERS;

    pub fn new(table: &[u8]) -> Result<Self, DmaHandoffError> {
        let header = Sdt::parse(table).map_err(DmaHandoffError::Table)?;
        if header.signature() != Sdt::DMA_HANDOFF {
            return Err(DmaHandoffError::WrongTable {
                expected: Sdt::DMA_HANDOFF,
                found: header.signature(),
            });
        }
        if header.length() as usize != Self::LEN {
            return Err(DmaHandoffError::WrongLength(header.length()));
        }
        if table[8] != 1 {
            return Err(DmaHandoffError::UnsupportedRevision(table[8]));
        }
        let segment =
            u16_at(table, Sdt::LEN).ok_or(DmaHandoffError::WrongLength(header.length()))?;
        let flags =
            u16_at(table, Sdt::LEN + 2).ok_or(DmaHandoffError::WrongLength(header.length()))?;
        if flags != Self::REQUIRED_FLAGS {
            return Err(DmaHandoffError::UnsafeFlags(flags));
        }
        let reserved =
            u64_at(table, Sdt::LEN + 4).ok_or(DmaHandoffError::WrongLength(header.length()))?;
        if reserved != 0 {
            return Err(DmaHandoffError::NonzeroReserved(reserved));
        }
        Ok(Self { segment })
    }

    pub const fn segment(self) -> u16 {
        self.segment
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DmaHandoffError {
    Table(AcpiError),
    WrongTable { expected: [u8; 4], found: [u8; 4] },
    WrongLength(u32),
    UnsupportedRevision(u8),
    UnsafeFlags(u16),
    NonzeroReserved(u64),
}

impl fmt::Display for DmaHandoffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Table(reason) => write!(formatter, "DMA handoff table is invalid: {reason}"),
            Self::WrongTable { expected, found } => {
                write!(
                    formatter,
                    "table signature {found:?} does not match {expected:?}"
                )
            }
            Self::WrongLength(length) => {
                write!(
                    formatter,
                    "DMA handoff table length {length} is not 48 bytes"
                )
            }
            Self::UnsupportedRevision(revision) => {
                write!(formatter, "DMA handoff revision {revision} is unsupported")
            }
            Self::UnsafeFlags(flags) => {
                write!(
                    formatter,
                    "DMA handoff flags {flags:#x} are not the complete contract"
                )
            }
            Self::NonzeroReserved(bits) => {
                write!(formatter, "DMA handoff reserved bits are {bits:#x}")
            }
        }
    }
}

impl core::error::Error for DmaHandoffError {}

mod handoff {
    pub(super) const COMPLETE_LEGACY_CONFIGURATION: u16 = 1 << 0;
    pub(super) const QUIESCENT_THROUGH_FENCE: u16 = 1 << 1;
    pub(super) const STATIC_REQUESTERS: u16 = 1 << 2;
}
