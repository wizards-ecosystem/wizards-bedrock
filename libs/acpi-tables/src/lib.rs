//! Parsing for the ACPI tables this kernel reads.
//!
//! The RSDP points at the XSDT, the XSDT lists tables by signature, the
//! MADT describes the interrupt controllers, and the DMAR describes the
//! VT-d IOMMU hardware. Nothing more, until something reads it.
//!
//! Works on `&[u8]` and validates before it trusts. Firmware is the least
//! trustworthy input the kernel has, so a malformed table must produce an
//! error — never a panic, never a read past the buffer.
//!
//! ```
//! use acpi_tables::{Rsdp, Sdt};
//!
//! // A minimal well-formed RSDP, checksummed.
//! let mut bytes = [0u8; 36];
//! bytes[..8].copy_from_slice(b"RSD PTR ");
//! bytes[15] = 2; // revision 2 — the XSDT is present
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AcpiError {
    /// The buffer is shorter than the structure it should hold.
    TooShort {
        /// Bytes needed.
        needed: usize,
        /// Bytes available.
        available: usize,
    },
    /// The leading signature is not the expected one.
    BadSignature([u8; 8]),
    /// The bytes do not sum to zero, so the table is corrupt.
    BadChecksum,
    /// The table's own length field is impossible.
    BadLength(u32),
    /// The table is structurally valid but exceeds the reader's allocation
    /// budget.
    TooLong {
        /// Bytes the table declares.
        declared: usize,
        /// Largest table the caller will accept.
        limit: usize,
    },
    /// A system-description table is not the kind a typed view requires.
    WrongTable {
        /// Signature the view accepts.
        expected: [u8; 4],
        /// Signature the table carries.
        found: [u8; 4],
    },
    /// A fixed-width table body ends with a partial entry.
    PartialEntry {
        /// Bytes in each complete entry.
        width: usize,
        /// Bytes left after the last complete entry.
        trailing: usize,
    },
    /// A root table names more tables than boot discovery will inspect.
    TooManyTables {
        /// Pointers listed by the root table.
        listed: usize,
        /// Maximum accepted pointer count.
        limit: usize,
    },
    /// A MADT entry has a length that cannot describe the bytes it names.
    MalformedEntry {
        /// Byte offset of the entry in the table.
        offset: usize,
        /// Length in the entry header, if that header was complete.
        length: usize,
        /// Bytes available from `offset` to the table's end.
        available: usize,
    },
    /// The revision is one this kernel does not read.
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
///
/// For building tables in tests and doctests; firmware does this for us.
///
/// # Panics
///
/// Panics if `index` is outside `bytes`.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rsdp {
    revision: u8,
    rsdt: u32,
    xsdt: u64,
}

impl Rsdp {
    /// The signature every RSDP begins with.
    pub const SIGNATURE: [u8; 8] = *b"RSD PTR ";

    /// Bytes in the original, revision-0 structure.
    pub const V1_LEN: usize = 20;

    /// Bytes in the extended structure, which adds the 64-bit XSDT pointer.
    pub const V2_LEN: usize = 36;

    /// Read the extent a root pointer declares without allocating it.
    ///
    /// Revision zero has the fixed twenty-byte form. Revision two carries a
    /// length field so compatible extensions remain inside the checksum.
    /// Other revision values have no defined layout and are refused.
    ///
    /// # Errors
    ///
    /// [`AcpiError::TooShort`] if the bytes do not reach the needed field,
    /// [`AcpiError::BadLength`] for an undersized extended pointer,
    /// [`AcpiError::TooLong`] above `limit`, and
    /// [`AcpiError::UnsupportedRevision`] for an unknown layout.
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

    /// Validate and read an RSDP.
    ///
    /// Both checksums are checked: revision 0 covers the first 20 bytes, and
    /// revision 2 adds a second covering all 36. A table that fails either is
    /// refused — firmware that miscomputes a checksum is firmware whose
    /// pointers are not worth following.
    ///
    /// # Errors
    ///
    /// [`AcpiError::TooShort`], [`AcpiError::BadSignature`],
    /// [`AcpiError::BadChecksum`], [`AcpiError::BadLength`] or
    /// [`AcpiError::UnsupportedRevision`].
    pub fn parse(bytes: &[u8]) -> Result<Self, AcpiError> {
        let head = bytes.get(..Self::V1_LEN).ok_or(AcpiError::TooShort {
            needed: Self::V1_LEN,
            available: bytes.len(),
        })?;

        // Before the revision: the signature is what says these bytes are an
        // RSDP at all, and an unknown revision is a poor name for random memory.
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

    /// The 64-bit XSDT address, if this RSDP has one.
    ///
    /// Preferred over the RSDT wherever present: the RSDT's entries are 32-bit
    /// and cannot name a table above 4 GiB.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sdt {
    signature: [u8; 4],
    length: u32,
}

impl Sdt {
    /// Bytes in the header.
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

    /// The length a header claims, without validating anything else.
    ///
    /// The one field that has to be read before a table can be validated at
    /// all: the checksum covers exactly `length` bytes, so a reader must learn
    /// `length` from an unvalidated header in order to fetch the bytes
    /// [`parse`](Self::parse) will then check. `limit` is the reader's explicit
    /// allocation budget, applied before any allocation follows this result.
    ///
    /// # Errors
    ///
    /// [`AcpiError::TooShort`] if the header is incomplete,
    /// [`AcpiError::BadLength`] if the declared length is smaller than a
    /// header, and [`AcpiError::TooLong`] above `limit`.
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

    /// Validate and read a table header.
    ///
    /// `bytes` must span the *whole* table, not merely its header: the
    /// checksum covers every byte the length field claims, and a header alone
    /// cannot be validated. That is deliberate — it makes it impossible to
    /// accept a table on the strength of a header whose body was never read.
    ///
    /// # Errors
    ///
    /// [`AcpiError::TooShort`] if the buffer is smaller than the length field
    /// claims, [`AcpiError::BadLength`] if that field is smaller than a
    /// header, and [`AcpiError::BadChecksum`] if the bytes do not sum to zero.
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

    /// The four-character signature naming the table.
    pub const fn signature(self) -> [u8; 4] {
        self.signature
    }

    /// Bytes in the whole table, header included.
    pub const fn length(self) -> u32 {
        self.length
    }
}

/// The most table pointers a root table may list.
///
/// A valid root can otherwise choose unbounded pre-interrupt reads and
/// allocations. Real machines list only a few dozen tables, so 256 preserves
/// ample headroom while bounding firmware-controlled work.
const MAX_TABLES: usize = 256;

/// The table addresses an XSDT or RSDT lists.
///
/// Construction refuses a root above the fixed boot-discovery bound.
#[derive(Debug)]
pub struct SdtPointers<'a> {
    body: &'a [u8],
    width: usize,
    offset: usize,
}

impl<'a> SdtPointers<'a> {
    fn new(table: &'a [u8], expected: [u8; 4], width: usize) -> Result<Self, AcpiError> {
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
    ///
    /// # Errors
    ///
    /// If the table fails validation, is not an XSDT, exceeds the discovery
    /// bound, or ends in a partial eight-byte pointer.
    pub fn xsdt(table: &'a [u8]) -> Result<Self, AcpiError> {
        Self::new(table, Sdt::XSDT, 8)
    }

    /// The entries of a validated RSDT.
    ///
    /// # Errors
    ///
    /// If the table fails validation, is not an RSDT, exceeds the discovery
    /// bound, or ends in a partial four-byte pointer.
    pub fn rsdt(table: &'a [u8]) -> Result<Self, AcpiError> {
        Self::new(table, Sdt::RSDT, 4)
    }
}

impl Iterator for SdtPointers<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        let address = match self.width {
            8 => u64_at(self.body, self.offset)?,
            _ => u32_at(self.body, self.offset)? as u64,
        };
        self.offset += self.width;
        Some(address)
    }
}

#[cfg(test)]
mod tests;
