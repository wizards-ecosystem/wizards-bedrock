#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]
mod attestation;
mod dmar;
mod madt;
mod requesters;

pub use attestation::{DmaCoverage, DmaCoverageError, DmaHandoff, DmaHandoffError};
pub use dmar::{Bdf, BusRange};
pub use madt::{InterruptError, IoApic, MadtInterrupts};
pub use requesters::{
    Dmar, DmarError, GoverningUnit, RequesterInventory, RequesterInventoryError, ReservedRegions,
};

use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AcpiError {
    TooShort {
        needed: usize,
        available: usize,
    },
    BadSignature([u8; 8]),
    BadChecksum,
    BadLength(u32),
    TooLong {
        declared: usize,
        limit: usize,
    },
    WrongTable {
        expected: [u8; 4],
        found: [u8; 4],
    },
    PartialEntry {
        width: usize,
        trailing: usize,
    },
    TooManyTables {
        listed: usize,
        limit: usize,
    },
    MalformedEntry {
        offset: usize,
        length: usize,
        available: usize,
    },
    UnsupportedRevision(u8),
}

impl fmt::Display for AcpiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooShort { needed, available } => {
                write!(
                    formatter,
                    "table needs {needed} bytes and {available} are readable"
                )
            }
            Self::BadSignature(_) => formatter.write_str("table signature is not the expected one"),
            Self::BadChecksum => formatter.write_str("table checksum does not sum to zero"),
            Self::BadLength(length) => write!(formatter, "table length {length} is impossible"),
            Self::TooLong { declared, limit } => {
                write!(
                    formatter,
                    "table declares {declared} bytes, above the {limit}-byte limit"
                )
            }
            Self::WrongTable { expected, found } => {
                write!(
                    formatter,
                    "table signature {found:?} does not match {expected:?}"
                )
            }
            Self::PartialEntry { width, trailing } => write!(
                formatter,
                "table has {trailing} trailing bytes after {width}-byte entries"
            ),
            Self::TooManyTables { listed, limit } => {
                write!(
                    formatter,
                    "root table lists {listed} pointers, above the {limit}-pointer limit"
                )
            }
            Self::MalformedEntry {
                offset,
                length,
                available,
            } => write!(
                formatter,
                "entry at byte {offset} declares {length} bytes with {available} available"
            ),
            Self::UnsupportedRevision(revision) => {
                write!(
                    formatter,
                    "table revision {revision} is not read by this kernel"
                )
            }
        }
    }
}

impl core::error::Error for AcpiError {}

fn checksum_is_valid(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let field = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes(field.try_into().ok()?))
}

fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    let field = bytes.get(offset..offset + 8)?;
    Some(u64::from_le_bytes(field.try_into().ok()?))
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    let field = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes(field.try_into().ok()?))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rsdp {
    revision: u8,
    rsdt: u32,
    xsdt: u64,
}

impl Rsdp {
    const SIGNATURE: [u8; 8] = *b"RSD PTR ";

    pub const V1_LEN: usize = 20;

    pub const V2_LEN: usize = 36;

    pub const EXTENDED_REVISION: u8 = 2;

    pub fn parse_length(bytes: &[u8], limit: usize) -> Result<usize, AcpiError> {
        let head = bytes.get(..Self::V1_LEN).ok_or(AcpiError::TooShort {
            needed: Self::V1_LEN,
            available: bytes.len(),
        })?;
        let length = match head[15] {
            0 => Self::V1_LEN,
            Self::EXTENDED_REVISION.. => {
                let prefix = bytes.get(..24).ok_or(AcpiError::TooShort {
                    needed: 24,
                    available: bytes.len(),
                })?;
                let declared = u32_at(prefix, 20).ok_or(AcpiError::BadChecksum)?;
                if declared < Self::V2_LEN as u32 {
                    return Err(AcpiError::BadLength(declared));
                }
                declared as usize
            }
            revision => return Err(AcpiError::UnsupportedRevision(revision)),
        };
        if length > limit {
            return Err(AcpiError::TooLong {
                declared: length,
                limit,
            });
        }
        Ok(length)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, AcpiError> {
        let head = bytes.get(..Self::V1_LEN).ok_or(AcpiError::TooShort {
            needed: Self::V1_LEN,
            available: bytes.len(),
        })?;

        let signature: [u8; 8] = head[..8].try_into().map_err(|_| AcpiError::BadChecksum)?;
        if signature != Self::SIGNATURE {
            return Err(AcpiError::BadSignature(signature));
        }
        let length = Self::parse_length(bytes, usize::MAX)?;
        if !checksum_is_valid(head) {
            return Err(AcpiError::BadChecksum);
        }

        let revision = head[15];
        let rsdt = u32_at(head, 16).ok_or(AcpiError::BadChecksum)?;
        if revision == 0 {
            return Ok(Self {
                revision,
                rsdt,
                xsdt: 0,
            });
        }

        let full = bytes.get(..length).ok_or(AcpiError::TooShort {
            needed: length,
            available: bytes.len(),
        })?;
        if !checksum_is_valid(full) {
            return Err(AcpiError::BadChecksum);
        }

        Ok(Self {
            revision,
            rsdt,
            xsdt: u64_at(full, 24).ok_or(AcpiError::BadChecksum)?,
        })
    }

    // Preferred: RSDT entries cannot name a table above 4 GiB.
    pub const fn xsdt(self) -> Option<u64> {
        if self.revision >= Self::EXTENDED_REVISION && self.xsdt != 0 {
            Some(self.xsdt)
        } else {
            None
        }
    }

    pub const fn rsdt(self) -> Option<u64> {
        if self.rsdt != 0 {
            Some(self.rsdt as u64)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sdt {
    signature: [u8; 4],
    length: u32,
}

impl Sdt {
    pub const LEN: usize = 36;

    pub const XSDT: [u8; 4] = *b"XSDT";

    pub const RSDT: [u8; 4] = *b"RSDT";

    pub const MADT: [u8; 4] = *b"APIC";

    pub const DMAR: [u8; 4] = *b"DMAR";

    pub const DMA_COVERAGE: [u8; 4] = *b"WZDM";

    pub const DMA_HANDOFF: [u8; 4] = *b"WZDH";

    pub fn parse_length(bytes: &[u8], limit: usize) -> Result<usize, AcpiError> {
        let head = bytes.get(..Self::LEN).ok_or(AcpiError::TooShort {
            needed: Self::LEN,
            available: bytes.len(),
        })?;
        let declared = u32_at(head, 4).ok_or(AcpiError::BadChecksum)?;
        if declared < Self::LEN as u32 {
            return Err(AcpiError::BadLength(declared));
        }
        let length = declared as usize;
        if length > limit {
            return Err(AcpiError::TooLong {
                declared: length,
                limit,
            });
        }
        Ok(length)
    }

    // `bytes` must span the whole table its checksum covers.
    pub fn parse(bytes: &[u8]) -> Result<Self, AcpiError> {
        let head = bytes.get(..Self::LEN).ok_or(AcpiError::TooShort {
            needed: Self::LEN,
            available: bytes.len(),
        })?;

        let length = u32_at(head, 4).ok_or(AcpiError::BadChecksum)?;
        if (length as usize) < Self::LEN {
            return Err(AcpiError::BadLength(length));
        }
        let whole = bytes.get(..length as usize).ok_or(AcpiError::TooShort {
            needed: length as usize,
            available: bytes.len(),
        })?;
        if !checksum_is_valid(whole) {
            return Err(AcpiError::BadChecksum);
        }

        Ok(Self {
            signature: head[..4].try_into().map_err(|_| AcpiError::BadChecksum)?,
            length,
        })
    }

    pub const fn signature(self) -> [u8; 4] {
        self.signature
    }

    pub const fn length(self) -> u32 {
        self.length
    }
}

// Real machines list a few dozen tables; this bounds firmware-controlled pre-interrupt work.
const MAX_TABLES: usize = 256;

#[derive(Debug)]
pub struct SdtPointers<'table> {
    body: &'table [u8],
    width: usize,
    offset: usize,
}

impl<'table> SdtPointers<'table> {
    fn new(table: &'table [u8], expected: [u8; 4], width: usize) -> Result<Self, AcpiError> {
        let header = Sdt::parse(table)?;
        if header.signature() != expected {
            return Err(AcpiError::WrongTable {
                expected,
                found: header.signature(),
            });
        }
        let length = header.length() as usize;
        let trailing = (length - Sdt::LEN) % width;
        if trailing != 0 {
            return Err(AcpiError::PartialEntry { width, trailing });
        }
        let listed = (length - Sdt::LEN) / width;
        if listed > MAX_TABLES {
            return Err(AcpiError::TooManyTables {
                listed,
                limit: MAX_TABLES,
            });
        }
        Ok(Self {
            body: &table[..length],
            width,
            offset: Sdt::LEN,
        })
    }

    pub fn xsdt(table: &'table [u8]) -> Result<Self, AcpiError> {
        Self::new(table, Sdt::XSDT, 8)
    }

    pub fn rsdt(table: &'table [u8]) -> Result<Self, AcpiError> {
        Self::new(table, Sdt::RSDT, 4)
    }
}

impl Iterator for SdtPointers<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        let address = match self.width {
            8 => u64_at(self.body, self.offset)?,
            _ => u64::from(u32_at(self.body, self.offset)?),
        };
        self.offset += self.width;
        Some(address)
    }
}

#[cfg(test)]
mod tests;
