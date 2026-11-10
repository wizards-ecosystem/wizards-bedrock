//! Parsing for the ACPI tables this kernel reads.
//!
//! Firmware bytes are hostile: a malformed table is an error, never a panic or out-of-bounds read.
//!
//! ```
//! use acpi_tables::{Rsdp, Sdt};
//!
//! let mut bytes = [0u8; 36];
//! bytes[..8].copy_from_slice(b"RSD PTR ");
//! bytes[15] = 2;
//! bytes[20..24].copy_from_slice(&36u32.to_le_bytes());
//! bytes[24..32].copy_from_slice(&0xdead_0000u64.to_le_bytes());
//! fn fix_checksum(bytes: &mut [u8], index: usize) {
//!     bytes[index] = 0;
//!     let sum = bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
//!     bytes[index] = sum.wrapping_neg();
//! }
//! fix_checksum(&mut bytes[..20], 9);
//! fix_checksum(&mut bytes, 32);
//!
//! let rsdp = Rsdp::parse(&bytes).expect("well formed");
//! assert_eq!(rsdp.xsdt(), Some(0xdead_0000));
//! assert_eq!(Sdt::MADT, *b"APIC");
//! ```

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
pub use dmar::{
    Bdf, BusRange, DeviceScope, DeviceScopeKind, DeviceScopes, DmarEntries, DmarStructure, PciPath,
};
pub use madt::{InterruptError, IoApic, MadtEntries, MadtEntry, MadtInterrupts};
pub use requesters::{
    Coverage, Dmar, DmarError, GoverningUnit, RequesterInventory, RequesterInventoryError,
    ReservedRegions,
};

use core::fmt;

/// Why a table could not be read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum AcpiError {
    TooShort {
        needed: usize,
        available: usize,
    },
    BadSignature([u8; 8]),
    BadChecksum,
    BadLength(u32),
    /// The table is structurally valid but exceeds the reader's allocation budget.
    TooLong {
        declared: usize,
        limit: usize,
    },
    /// A system-description table is not the kind a typed view requires.
    WrongTable {
        expected: [u8; 4],
        found: [u8; 4],
    },
    /// A fixed-width table body ends with a partial entry.
    PartialEntry {
        width: usize,
        trailing: usize,
    },
    /// A root table names more tables than boot discovery will inspect.
    TooManyTables {
        listed: usize,
        limit: usize,
    },
    /// An entry has a length that cannot describe the bytes it names.
    MalformedEntry {
        offset: usize,
        /// Length in the entry header, if that header was complete.
        length: usize,
        available: usize,
    },
    UnsupportedRevision(u8),
}

impl fmt::Display for AcpiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooShort { needed, available } => {
                write!(f, "table needs {needed} bytes and {available} are readable")
            }
            Self::BadSignature(_) => {
                core::fmt::Write::write_str(f, "table signature is not the expected one")
            }
            Self::BadChecksum => {
                core::fmt::Write::write_str(f, "table checksum does not sum to zero")
            }
            Self::BadLength(length) => write!(f, "table length {length} is impossible"),
            Self::TooLong { declared, limit } => {
                write!(
                    f,
                    "table declares {declared} bytes, above the {limit}-byte limit"
                )
            }
            Self::WrongTable { expected, found } => {
                write!(f, "table signature {found:?} does not match {expected:?}")
            }
            Self::PartialEntry { width, trailing } => write!(
                f,
                "table has {trailing} trailing bytes after {width}-byte entries"
            ),
            Self::TooManyTables { listed, limit } => {
                write!(
                    f,
                    "root table lists {listed} pointers, above the {limit}-pointer limit"
                )
            }
            Self::MalformedEntry {
                offset,
                length,
                available,
            } => write!(
                f,
                "entry at byte {offset} declares {length} bytes with {available} available"
            ),
            Self::UnsupportedRevision(revision) => {
                write!(f, "table revision {revision} is not read by this kernel")
            }
        }
    }
}

impl core::error::Error for AcpiError {}

/// Whether `bytes` sums to zero in eight-bit arithmetic, as ACPI requires.
fn checksum_is_valid(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

/// Set `bytes[index]` so the whole slice sums to zero.
#[cfg(test)]
fn fix_checksum(bytes: &mut [u8], index: usize) {
    bytes[index] = 0;
    let sum = bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    bytes[index] = sum.wrapping_neg();
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

/// The Root System Description Pointer: where the table tree starts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rsdp {
    revision: u8,
    rsdt: u32,
    xsdt: u64,
}

impl Rsdp {
    pub const SIGNATURE: [u8; 8] = *b"RSD PTR ";

    /// Bytes in the original, revision-0 structure.
    pub const V1_LEN: usize = 20;

    /// Bytes in the extended structure, which adds the 64-bit XSDT pointer.
    pub const V2_LEN: usize = 36;

    /// Read the extent a root pointer declares, refusing a revision with no defined layout.
    pub fn parse_length(bytes: &[u8], limit: usize) -> Result<usize, AcpiError> {
        let head = bytes.get(..Self::V1_LEN).ok_or(AcpiError::TooShort {
            needed: Self::V1_LEN,
            available: bytes.len(),
        })?;
        let length = match head[15] {
            0 => Self::V1_LEN,
            2 => {
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

    /// Validate and read an RSDP, checking its signature before trusting its revision.
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

    /// The 64-bit XSDT address, preferred because RSDT entries cannot name a table above 4 GiB.
    pub const fn xsdt(self) -> Option<u64> {
        if self.revision >= 2 && self.xsdt != 0 {
            Some(self.xsdt)
        } else {
            None
        }
    }

    /// The 32-bit RSDT address, for firmware too old to have an XSDT.
    pub const fn rsdt(self) -> Option<u64> {
        if self.rsdt != 0 {
            Some(self.rsdt as u64)
        } else {
            None
        }
    }
}

/// The header every System Description Table begins with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sdt {
    signature: [u8; 4],
    length: u32,
}

impl Sdt {
    pub const LEN: usize = 36;

    /// Signature of the Extended System Description Table.
    pub const XSDT: [u8; 4] = *b"XSDT";

    /// Signature of the Root System Description Table.
    pub const RSDT: [u8; 4] = *b"RSDT";

    /// Signature of the Multiple APIC Description Table.
    pub const MADT: [u8; 4] = *b"APIC";

    /// Signature of the DMA Remapping Reporting table.
    pub const DMAR: [u8; 4] = *b"DMAR";

    /// Signature of Wizard's explicit whole-segment DMA coverage attestation.
    pub const DMA_COVERAGE: [u8; 4] = *b"WZDM";

    /// Signature of Wizard's DMA-safe firmware handoff attestation.
    pub const DMA_HANDOFF: [u8; 4] = *b"WZDH";

    /// The unvalidated length a header claims, bounded by the reader's allocation `limit`.
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

    /// Validate and read a table header; `bytes` must span the whole table its checksum covers.
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

    /// Bytes in the whole table, header included.
    pub const fn length(self) -> u32 {
        self.length
    }
}

/// Real machines list a few dozen tables; this bounds firmware-controlled pre-interrupt work.
const MAX_TABLES: usize = 256;

/// The table addresses an XSDT or RSDT lists.
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

    /// The entries of a validated XSDT.
    pub fn xsdt(table: &'table [u8]) -> Result<Self, AcpiError> {
        Self::new(table, Sdt::XSDT, 8)
    }

    /// The entries of a validated RSDT.
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
