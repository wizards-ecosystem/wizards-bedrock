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

/// An entry of the Multiple APIC Description Table.
///
/// Only the kinds this kernel acts on are named. Everything else is
/// [`Other`](Self::Other) with its type byte, so an unrecognised controller is
/// reportable rather than invisible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MadtEntry {
    /// A processor's local APIC.
    LocalApic {
        /// The processor this belongs to.
        processor: u8,
        /// The controller's own identifier.
        id: u8,
        /// Whether the processor is usable.
        enabled: bool,
    },
    /// An I/O APIC: the controller that routes device interrupts.
    IoApic {
        /// The controller's identifier.
        id: u8,
        /// Physical address of its registers.
        address: u32,
        /// The first global system interrupt it handles.
        gsi_base: u32,
    },
    /// A legacy IRQ that does not arrive on the interrupt line sharing its
    /// number.
    ///
    /// The reason IRQ routing cannot assume `irq == gsi`: firmware commonly
    /// remaps the timer, and may remap anything.
    InterruptSourceOverride {
        /// The legacy bus, always 0 (ISA) in practice.
        bus: u8,
        /// The legacy IRQ number.
        source: u8,
        /// The global system interrupt it actually arrives on.
        gsi: u32,
        /// Polarity and trigger mode, in the MPS INTI encoding.
        flags: u16,
    },
    /// A kind this kernel does not act on, with its type byte.
    Other {
        /// The entry type.
        kind: u8,
    },
}

/// The entries of a validated MADT.
#[derive(Debug)]
pub struct MadtEntries<'a> {
    body: &'a [u8],
    offset: usize,
}

impl<'a> MadtEntries<'a> {
    /// Bytes of MADT-specific header before the entries begin: the local APIC
    /// address and the flags word.
    const HEADER: usize = Sdt::LEN + 8;

    /// Validate a MADT and walk its entries.
    ///
    /// Validation covers the MADT-specific header and every variable-length
    /// entry before iteration starts. That makes a corrupt tail invalidate the
    /// table as a whole instead of leaving callers with a trusted prefix.
    ///
    /// # Errors
    ///
    /// If the table is not a valid MADT or any entry is truncated, has a
    /// non-advancing length, or is shorter than the fields of its known kind.
    pub fn new(table: &'a [u8]) -> Result<Self, AcpiError> {
        let header = Sdt::parse(table)?;
        if header.signature() != Sdt::MADT {
            return Err(AcpiError::WrongTable {
                expected: Sdt::MADT,
                found: header.signature(),
            });
        }
        let length = header.length() as usize;
        if length < Self::HEADER {
            return Err(AcpiError::BadLength(header.length()));
        }
        let body = &table[..length];
        let mut offset = Self::HEADER;
        while offset < body.len() {
            let available = body.len() - offset;
            let Some(prefix) = body.get(offset..offset.saturating_add(2)) else {
                return Err(AcpiError::MalformedEntry {
                    offset,
                    length: 0,
                    available,
                });
            };
            let kind = prefix[0];
            let entry_len = usize::from(prefix[1]);
            let minimum = match kind {
                0 => 8,
                1 => 12,
                2 => 10,
                _ => 2,
            };
            if entry_len < minimum || entry_len > available {
                return Err(AcpiError::MalformedEntry {
                    offset,
                    length: entry_len,
                    available,
                });
            }
            offset += entry_len;
        }
        Ok(Self {
            body,
            offset: Self::HEADER,
        })
    }
}

impl Iterator for MadtEntries<'_> {
    type Item = MadtEntry;

    fn next(&mut self) -> Option<MadtEntry> {
        let kind = *self.body.get(self.offset)?;
        let length = usize::from(*self.body.get(self.offset + 1)?);
        let entry = self.body.get(self.offset..self.offset + length)?;
        self.offset += length;

        Some(match kind {
            0 => MadtEntry::LocalApic {
                processor: entry[2],
                id: entry[3],
                enabled: u32_at(entry, 4).unwrap_or(0) & 1 != 0,
            },
            1 => MadtEntry::IoApic {
                id: entry[2],
                address: u32_at(entry, 4).unwrap_or(0),
                gsi_base: u32_at(entry, 8).unwrap_or(0),
            },
            2 => MadtEntry::InterruptSourceOverride {
                bus: entry[2],
                source: entry[3],
                gsi: u32_at(entry, 4).unwrap_or(0),
                flags: u16::from_le_bytes([entry[8], entry[9]]),
            },
            kind => MadtEntry::Other { kind },
        })
    }
}

/// One I/O APIC description retained from a MADT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoApic {
    id: u8,
    address: u32,
    gsi_base: u32,
}

impl IoApic {
    /// The controller identifier firmware assigned.
    pub const fn id(self) -> u8 {
        self.id
    }

    /// Physical address of the controller's register window.
    pub const fn address(self) -> u32 {
        self.address
    }

    /// The first global system interrupt the controller reports.
    pub const fn gsi_base(self) -> u32 {
        self.gsi_base
    }
}

/// Why a MADT could not become a complete bounded interrupt inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InterruptError {
    /// The MADT itself is not structurally valid.
    Table(AcpiError),
    /// Firmware described more I/O APICs than the caller can retain.
    TooManyIoApics {
        /// Number of descriptions the caller permitted.
        limit: usize,
    },
    /// An interrupt source override did not describe the ISA bus.
    NonIsaOverride {
        /// Firmware bus identifier.
        bus: u8,
        /// Source IRQ named by the entry.
        source: u8,
    },
    /// An interrupt source override names an IRQ outside the ISA domain.
    InvalidOverrideSource(u8),
    /// More than one override names the same source IRQ.
    DuplicateOverride(u8),
    /// The MPS INTI polarity or trigger field has a reserved encoding.
    ReservedOverrideFlags {
        /// Source IRQ named by the entry.
        source: u8,
        /// Raw MPS INTI flags.
        flags: u16,
    },
}

impl fmt::Display for InterruptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Table(error) => write!(f, "madt is unusable: {error}"),
            Self::TooManyIoApics { limit } => {
                write!(f, "madt describes more than {limit} i/o apics")
            }
            Self::NonIsaOverride { bus, source } => {
                write!(f, "madt override for irq {source} names non-ISA bus {bus}")
            }
            Self::InvalidOverrideSource(source) => {
                write!(f, "madt override names non-ISA irq {source}")
            }
            Self::DuplicateOverride(source) => {
                write!(f, "madt repeats the override for irq {source}")
            }
            Self::ReservedOverrideFlags { source, flags } => write!(
                f,
                "madt override for irq {source} has reserved flags {flags:#x}"
            ),
        }
    }
}

impl core::error::Error for InterruptError {}

/// A complete, fixed-capacity view of MADT interrupt routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MadtInterrupts<const IO_APICS: usize> {
    io_apics: [Option<IoApic>; IO_APICS],
    io_apic_count: usize,
    overrides: [Option<(u32, u16)>; 16],
}

impl<const IO_APICS: usize> MadtInterrupts<IO_APICS> {
    /// Parse every I/O APIC and legacy interrupt override in `table`.
    ///
    /// # Errors
    ///
    /// [`InterruptError::Table`] for a malformed MADT, a typed override error
    /// for contradictory routing, or [`InterruptError::TooManyIoApics`]
    /// rather than returning a truncated inventory.
    pub fn parse(table: &[u8]) -> Result<Self, InterruptError> {
        let mut entries = MadtEntries::new(table).map_err(InterruptError::Table)?;
        let mut model = Self {
            io_apics: [None; IO_APICS],
            io_apic_count: 0,
            overrides: [None; 16],
        };

        let mut next = core::iter::Iterator::next(&mut entries);
        while let Some(entry) = next {
            match entry {
                MadtEntry::IoApic {
                    id,
                    address,
                    gsi_base,
                } => {
                    let Some(slot) = model.io_apics.get_mut(model.io_apic_count) else {
                        return Err(InterruptError::TooManyIoApics { limit: IO_APICS });
                    };
                    *slot = Some(IoApic {
                        id,
                        address,
                        gsi_base,
                    });
                    model.io_apic_count += 1;
                }
                MadtEntry::InterruptSourceOverride {
                    bus,
                    source,
                    gsi,
                    flags,
                } => {
                    if bus != 0 {
                        return Err(InterruptError::NonIsaOverride { bus, source });
                    }
                    let Some(slot) = model.overrides.get_mut(source as usize) else {
                        return Err(InterruptError::InvalidOverrideSource(source));
                    };
                    let polarity = flags & 0b11;
                    let trigger = (flags >> 2) & 0b11;
                    if flags & !0xf != 0 || polarity == 0b10 || trigger == 0b10 {
                        return Err(InterruptError::ReservedOverrideFlags { source, flags });
                    }
                    if slot.is_some() {
                        return Err(InterruptError::DuplicateOverride(source));
                    }
                    *slot = Some((gsi, flags));
                }
                _ => {}
            }
            next = core::iter::Iterator::next(&mut entries);
        }
        Ok(model)
    }

    /// Every I/O APIC description, in firmware order.
    pub fn io_apics(&self) -> impl Iterator<Item = IoApic> + '_ {
        self.io_apics[..self.io_apic_count]
            .iter()
            .copied()
            .flatten()
    }

    /// Number of I/O APIC descriptions retained.
    pub const fn io_apic_count(&self) -> usize {
        self.io_apic_count
    }

    /// The global system interrupt `irq` arrives on.
    pub fn gsi_for(&self, irq: u8) -> u32 {
        self.override_for(irq)
            .map_or(u32::from(irq), |(gsi, _)| gsi)
    }

    /// The MPS INTI flags for `irq`, or zero when firmware gave none.
    pub fn flags_for(&self, irq: u8) -> u16 {
        self.override_for(irq).map_or(0, |(_, flags)| flags)
    }

    /// Firmware's explicit override for `irq`, if it supplied one.
    pub fn override_for(&self, irq: u8) -> Option<(u32, u16)> {
        self.overrides.get(irq as usize).copied().flatten()
    }
}

/// A remapping structure of the DMA Remapping Reporting table.
///
/// Only the kinds a VT-d driver acts on are named; everything else is
/// [`Other`](Self::Other) with its type word, the same shape as
/// [`MadtEntry::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DmarStructure<'a> {
    /// A DMA remapping hardware unit: one IOMMU and the segment or devices
    /// it covers.
    Drhd {
        /// Whether this unit covers every device in `segment` that no other
        /// unit scopes explicitly.
        include_pci_all: bool,
        /// The PCI segment group this unit serves.
        segment: u16,
        /// Physical address of the unit's register set.
        register_base: u64,
        /// Raw device-scope bytes; walk with [`DeviceScopes`].
        scopes: &'a [u8],
    },
    /// A reserved memory region a device DMAs into directly, which
    /// remapping must identity-map rather than translate.
    Rmrr {
        /// The PCI segment group the scoped device is in.
        segment: u16,
        /// First byte of the reserved region.
        base: u64,
        /// Last byte of the reserved region, inclusive.
        limit: u64,
        /// Raw device-scope bytes; walk with [`DeviceScopes`].
        scopes: &'a [u8],
    },
    /// A root-port ATS capability report.
    Atsr {
        /// The PCI segment group the scoped devices are in.
        segment: u16,
        /// Raw device-scope bytes; walk with [`DeviceScopes`].
        scopes: &'a [u8],
    },
    /// A remapping hardware static affinity, linking a unit to a NUMA
    /// proximity domain from the SRAT.
    Rhsa {
        /// Physical address of the DRHD this affinity describes.
        register_base: u64,
        /// The proximity domain.
        proximity_domain: u32,
    },
    /// A kind this crate does not act on, with its type word.
    Other {
        /// The structure type.
        kind: u16,
    },
}

/// The remapping structures of a validated DMAR.
#[derive(Debug)]
pub struct DmarEntries<'a> {
    body: &'a [u8],
    offset: usize,
}

impl<'a> DmarEntries<'a> {
    /// Bytes of DMAR-specific header before the remapping structures begin:
    /// the host address width, flags, and ten reserved bytes.
    const HEADER: usize = Sdt::LEN + 12;

    /// Validate a DMAR and walk its remapping structures.
    ///
    /// Validation covers every variable-length structure — and any device
    /// scope nested inside a DRHD, RMRR, or ATSR — before iteration starts,
    /// so a corrupt tail invalidates the table as a whole instead of
    /// leaving callers with a trusted prefix.
    ///
    /// # Errors
    ///
    /// If the table is not a valid DMAR, or a structure or a nested device
    /// scope is truncated, has a non-advancing length, or runs past its
    /// container.
    pub fn new(table: &'a [u8]) -> Result<Self, AcpiError> {
        let header = Sdt::parse(table)?;
        if header.signature() != Sdt::DMAR {
            return Err(AcpiError::WrongTable {
                expected: Sdt::DMAR,
                found: header.signature(),
            });
        }
        let length = header.length() as usize;
        if length < Self::HEADER {
            return Err(AcpiError::BadLength(header.length()));
        }
        let body = &table[..length];
        let mut offset = Self::HEADER;
        while offset < body.len() {
            let available = body.len() - offset;
            let Some(prefix) = body.get(offset..offset.saturating_add(4)) else {
                return Err(AcpiError::MalformedEntry {
                    offset,
                    length: 0,
                    available,
                });
            };
            let kind = u16::from_le_bytes([prefix[0], prefix[1]]);
            let entry_len = usize::from(u16::from_le_bytes([prefix[2], prefix[3]]));
            // Each kind's minimum is the sum of its own fixed fields, so it
            // also rejects a non-advancing (zero) length.
            let (minimum, scope_start) = match kind {
                0 => (16, Some(16)), // DRHD
                1 => (24, Some(24)), // RMRR
                2 => (8, Some(8)),   // ATSR
                3 => (20, None),     // RHSA, no device scope
                _ => (4, None),      // unknown: only the header need be there
            };
            if entry_len < minimum || entry_len > available {
                return Err(AcpiError::MalformedEntry {
                    offset,
                    length: entry_len,
                    available,
                });
            }
            if let Some(scope_start) = scope_start {
                let structure = &body[offset..offset + entry_len];
                let scope_base = offset.saturating_add(scope_start);
                validate_device_scopes(scope_base, &structure[scope_start..])?;
            }
            offset += entry_len;
        }
        Ok(Self {
            body,
            offset: Self::HEADER,
        })
    }
}

/// Bounds-check every device scope nested in a DRHD, RMRR, or ATSR
/// structure.
///
/// `base` is the scope list's absolute offset in the table, so a refusal
/// names the byte a firmware author would need to fix.
fn validate_device_scopes(base: usize, scopes: &[u8]) -> Result<(), AcpiError> {
    let mut offset = 0;
    while offset < scopes.len() {
        let available = scopes.len() - offset;
        let Some(prefix) = scopes.get(offset..offset.saturating_add(2)) else {
            return Err(AcpiError::MalformedEntry {
                offset: base.saturating_add(offset),
                length: 0,
                available,
            });
        };
        let entry_len = usize::from(prefix[1]);
        if entry_len < DeviceScope::MIN_LEN || entry_len > available {
            return Err(AcpiError::MalformedEntry {
                offset: base.saturating_add(offset),
                length: entry_len,
                available,
            });
        }
        offset += entry_len;
    }
    Ok(())
}

impl<'a> Iterator for DmarEntries<'a> {
    type Item = DmarStructure<'a>;

    fn next(&mut self) -> Option<DmarStructure<'a>> {
        let prefix = self.body.get(self.offset..self.offset + 4)?;
        let kind = u16::from_le_bytes([prefix[0], prefix[1]]);
        let length = usize::from(u16::from_le_bytes([prefix[2], prefix[3]]));
        let entry = self.body.get(self.offset..self.offset + length)?;
        self.offset += length;

        Some(match kind {
            0 => DmarStructure::Drhd {
                include_pci_all: entry[4] & 1 != 0,
                segment: u16_at(entry, 6).unwrap_or(0),
                register_base: u64_at(entry, 8).unwrap_or(0),
                scopes: &entry[16..],
            },
            1 => DmarStructure::Rmrr {
                segment: u16_at(entry, 6).unwrap_or(0),
                base: u64_at(entry, 8).unwrap_or(0),
                limit: u64_at(entry, 16).unwrap_or(0),
                scopes: &entry[24..],
            },
            2 => DmarStructure::Atsr {
                segment: u16_at(entry, 6).unwrap_or(0),
                scopes: &entry[8..],
            },
            3 => DmarStructure::Rhsa {
                register_base: u64_at(entry, 8).unwrap_or(0),
                proximity_domain: u32_at(entry, 16).unwrap_or(0),
            },
            kind => DmarStructure::Other { kind },
        })
    }
}

/// The kind of hardware a device-scope entry names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceScopeKind {
    /// A PCI endpoint device.
    PciEndpoint,
    /// A PCI-to-PCI bridge, and everything behind it.
    PciSubHierarchy,
    /// An I/O APIC, named by its ACPI enumeration id rather than a path.
    IoApic,
    /// An HPET, named by its ACPI enumeration id rather than a path.
    Hpet,
    /// An ACPI namespace device.
    AcpiNamespaceDevice,
    /// A kind this crate does not act on, with its type byte.
    Other {
        /// The scope type.
        kind: u8,
    },
}

impl DeviceScopeKind {
    fn from_raw(kind: u8) -> Self {
        match kind {
            1 => Self::PciEndpoint,
            2 => Self::PciSubHierarchy,
            3 => Self::IoApic,
            4 => Self::Hpet,
            5 => Self::AcpiNamespaceDevice,
            kind => Self::Other { kind },
        }
    }
}

/// One device-scope entry nested in a DRHD, RMRR, or ATSR structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceScope<'a> {
    kind: DeviceScopeKind,
    enumeration_id: u8,
    start_bus: u8,
    path: &'a [u8],
}

impl<'a> DeviceScope<'a> {
    /// Bytes in a scope entry with no path hops: the type, length, flags,
    /// reserved byte, enumeration id, and start bus.
    const MIN_LEN: usize = 6;

    /// The hardware this scope names.
    pub const fn kind(self) -> DeviceScopeKind {
        self.kind
    }

    /// The ACPI enumeration id: an I/O APIC or HPET identifier, meaningless
    /// for a PCI-addressed scope.
    pub const fn enumeration_id(self) -> u8 {
        self.enumeration_id
    }

    /// The first bus number in the hierarchical path to the device.
    pub const fn start_bus(self) -> u8 {
        self.start_bus
    }

    /// The (device, function) hops from `start_bus` down to the target.
    pub const fn path(self) -> PciPath<'a> {
        PciPath { bytes: self.path }
    }

    /// The target of a single-hop PCI scope, as `(bus, device, function)`.
    ///
    /// `None` for a path of any other length: a bridge's secondary bus
    /// number, needed to resolve a multi-hop path, is read from live PCI
    /// configuration space, which is outside a byte-level table parser.
    pub fn pci_target(self) -> Option<(u8, u8, u8)> {
        let mut hops = self.path();
        let (device, function) = hops.next()?;
        if hops.next().is_some() {
            return None;
        }
        Some((self.start_bus, device, function))
    }
}

/// The (device, function) hops of a [`DeviceScope`]'s PCI path.
#[derive(Debug)]
pub struct PciPath<'a> {
    bytes: &'a [u8],
}

impl Iterator for PciPath<'_> {
    type Item = (u8, u8);

    fn next(&mut self) -> Option<(u8, u8)> {
        let pair = self.bytes.get(..2)?;
        self.bytes = &self.bytes[2..];
        Some((pair[0], pair[1]))
    }
}

/// The device-scope entries nested in a DRHD, RMRR, or ATSR
/// [`DmarStructure`].
#[derive(Debug)]
pub struct DeviceScopes<'a> {
    bytes: &'a [u8],
}

impl<'a> DeviceScopes<'a> {
    /// Walk the raw device-scope bytes a [`DmarStructure`] carries.
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

impl<'a> Iterator for DeviceScopes<'a> {
    type Item = DeviceScope<'a>;

    fn next(&mut self) -> Option<DeviceScope<'a>> {
        let kind = *self.bytes.first()?;
        let length = usize::from(*self.bytes.get(1)?);
        // Defensive rather than trusting: unlike `DmarEntries`, this
        // iterator is reachable without going through a validating
        // constructor, so a malformed slice must stop cleanly, not index
        // out of bounds.
        if length < DeviceScope::MIN_LEN {
            return None;
        }
        let entry = self.bytes.get(..length)?;
        self.bytes = &self.bytes[length..];

        Some(DeviceScope {
            kind: DeviceScopeKind::from_raw(kind),
            enumeration_id: entry[4],
            start_bus: entry[5],
            path: &entry[6..],
        })
    }
}

/// A PCI function, addressed the way a DMAR device scope names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bdf {
    segment: u16,
    bus: u8,
    device: u8,
    function: u8,
}

impl Bdf {
    /// Name a PCI function by its segment, bus, device, and function
    /// numbers.
    pub const fn new(segment: u16, bus: u8, device: u8, function: u8) -> Self {
        Self {
            segment,
            bus,
            device,
            function,
        }
    }

    /// The PCI segment group.
    pub const fn segment(self) -> u16 {
        self.segment
    }

    /// The bus number.
    pub const fn bus(self) -> u8 {
        self.bus
    }

    /// The device number.
    pub const fn device(self) -> u8 {
        self.device
    }

    /// The function number.
    pub const fn function(self) -> u8 {
        self.function
    }
}

/// Why a DMAR could not become a validated VT-d inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DmarError {
    /// The DMAR itself is not structurally valid.
    Table(AcpiError),
    /// An RMRR's limit precedes its base.
    ReservedRegionInverted {
        /// First byte of the declared region.
        base: u64,
        /// Last byte of the declared region.
        limit: u64,
    },
    /// Two DRHDs advertise the same register base.
    DuplicateUnit {
        /// The register base both units claim.
        register_base: u64,
    },
}

impl fmt::Display for DmarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Table(error) => write!(f, "dmar is unusable: {error}"),
            Self::ReservedRegionInverted { base, limit } => write!(
                f,
                "dmar reserved region [{base:#x}, {limit:#x}] has its limit before its base"
            ),
            Self::DuplicateUnit { register_base } => write!(
                f,
                "dmar has two drhd units at register base {register_base:#x}"
            ),
        }
    }
}

impl core::error::Error for DmarError {}

/// A validated DMAR: every remapping structure and nested device scope
/// checked once, so the queries below are pure arithmetic over trusted
/// bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dmar<'a> {
    body: &'a [u8],
}

impl<'a> Dmar<'a> {
    /// Validate `table` as a DMAR, and additionally enforce the two rules
    /// bytes alone cannot express: an RMRR's limit must not precede its
    /// base, and no two DRHDs may claim the same register base.
    ///
    /// # Errors
    ///
    /// [`DmarError::Table`] if the table is not a structurally valid DMAR,
    /// [`DmarError::ReservedRegionInverted`] for a backwards RMRR range, or
    /// [`DmarError::DuplicateUnit`] if two DRHDs share a register base.
    pub fn new(table: &'a [u8]) -> Result<Self, DmarError> {
        let entries = DmarEntries::new(table).map_err(DmarError::Table)?;
        let dmar = Self { body: entries.body };

        // O(n^2), not a growable set: real firmware lists a handful of
        // units, and there is no allocator here to collect them into one.
        for (index, structure) in dmar.entries().enumerate() {
            match structure {
                DmarStructure::Drhd { register_base, .. } => {
                    let duplicate = dmar.entries().skip(index + 1).any(|later| {
                        matches!(
                            later,
                            DmarStructure::Drhd { register_base: other, .. }
                                if other == register_base
                        )
                    });
                    if duplicate {
                        return Err(DmarError::DuplicateUnit { register_base });
                    }
                }
                DmarStructure::Rmrr { base, limit, .. } if base > limit => {
                    return Err(DmarError::ReservedRegionInverted { base, limit });
                }
                _ => {}
            }
        }

        Ok(dmar)
    }

    // Skips `DmarEntries::new`'s validation: `Dmar::new` already ran it
    // once, and repeating it on every query would defeat the point of
    // validating once.
    fn entries(&self) -> DmarEntries<'a> {
        DmarEntries {
            body: self.body,
            offset: DmarEntries::HEADER,
        }
    }

    /// The register base of the DRHD unit that governs `bdf`.
    ///
    /// An explicit PCI-endpoint scope on some unit outranks an
    /// `INCLUDE_PCI_ALL` unit in the same segment: firmware uses
    /// `INCLUDE_PCI_ALL` to mean "everything I did not call out by name".
    pub fn unit_for(&self, bdf: Bdf) -> Option<u64> {
        let mut catch_all = None;
        for structure in self.entries() {
            let DmarStructure::Drhd {
                include_pci_all,
                segment,
                register_base,
                scopes,
            } = structure
            else {
                continue;
            };
            if segment != bdf.segment() {
                continue;
            }
            let target = (bdf.bus(), bdf.device(), bdf.function());
            let scoped = DeviceScopes::new(scopes).any(|scope| {
                scope.kind() == DeviceScopeKind::PciEndpoint && scope.pci_target() == Some(target)
            });
            if scoped {
                return Some(register_base);
            }
            if include_pci_all && catch_all.is_none() {
                catch_all = Some(register_base);
            }
        }
        catch_all
    }

    /// Every reserved memory region a device at `bdf` must have
    /// identity-mapped rather than translated.
    pub fn reserved_regions_for(&self, bdf: Bdf) -> ReservedRegions<'a> {
        ReservedRegions {
            entries: self.entries(),
            bdf,
        }
    }
}

/// The `(base, limit)` ranges [`Dmar::reserved_regions_for`] finds for one
/// device, inclusive of both ends.
#[derive(Debug)]
pub struct ReservedRegions<'a> {
    entries: DmarEntries<'a>,
    bdf: Bdf,
}

impl Iterator for ReservedRegions<'_> {
    type Item = (u64, u64);

    fn next(&mut self) -> Option<(u64, u64)> {
        for structure in self.entries.by_ref() {
            let DmarStructure::Rmrr {
                segment,
                base,
                limit,
                scopes,
            } = structure
            else {
                continue;
            };
            if segment != self.bdf.segment() {
                continue;
            }
            let target = (self.bdf.bus(), self.bdf.device(), self.bdf.function());
            let scoped = DeviceScopes::new(scopes).any(|scope| {
                scope.kind() == DeviceScopeKind::PciEndpoint && scope.pci_target() == Some(target)
            });
            if scoped {
                return Some((base, limit));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rsdp(revision: u8) -> Vec<u8> {
        let mut bytes = vec![0u8; Rsdp::V2_LEN];
        bytes[..8].copy_from_slice(&Rsdp::SIGNATURE);
        bytes[15] = revision;
        bytes[16..20].copy_from_slice(&0x1234u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&(Rsdp::V2_LEN as u32).to_le_bytes());
        bytes[24..32].copy_from_slice(&0xdead_beefu64.to_le_bytes());
        fix_checksum(&mut bytes[..Rsdp::V1_LEN], 9);
        fix_checksum(&mut bytes, 32);
        bytes
    }

    fn table(signature: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; Sdt::LEN];
        bytes[..4].copy_from_slice(signature);
        bytes.extend_from_slice(body);
        let length = bytes.len() as u32;
        bytes[4..8].copy_from_slice(&length.to_le_bytes());
        fix_checksum(&mut bytes, 9);
        bytes
    }

    #[test]
    fn errors_name_the_refused_table_property() {
        let cases = [
            (
                AcpiError::TooShort {
                    needed: 36,
                    available: 20,
                },
                "table needs 36 bytes and 20 are readable",
            ),
            (
                AcpiError::BadSignature(*b"NOT ACPI"),
                "table signature is not the expected one",
            ),
            (
                AcpiError::BadChecksum,
                "table checksum does not sum to zero",
            ),
            (AcpiError::BadLength(12), "table length 12 is impossible"),
            (
                AcpiError::TooLong {
                    declared: 4097,
                    limit: 4096,
                },
                "table declares 4097 bytes, above the 4096-byte limit",
            ),
            (
                AcpiError::WrongTable {
                    expected: *b"XSDT",
                    found: *b"APIC",
                },
                "table signature [65, 80, 73, 67] does not match [88, 83, 68, 84]",
            ),
            (
                AcpiError::PartialEntry {
                    width: 8,
                    trailing: 3,
                },
                "table has 3 trailing bytes after 8-byte entries",
            ),
            (
                AcpiError::MalformedEntry {
                    offset: 44,
                    length: 12,
                    available: 4,
                },
                "entry at byte 44 declares 12 bytes with 4 available",
            ),
            (
                AcpiError::UnsupportedRevision(7),
                "table revision 7 is not read by this kernel",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn interrupt_errors_name_the_table_or_inventory_bound() {
        assert_eq!(
            InterruptError::Table(AcpiError::BadChecksum).to_string(),
            "madt is unusable: table checksum does not sum to zero"
        );
        assert_eq!(
            InterruptError::TooManyIoApics { limit: 16 }.to_string(),
            "madt describes more than 16 i/o apics"
        );
    }

    #[test]
    fn a_madt_entry_shorter_than_its_fields_is_refused_not_read() {
        // Independent minima from ACPI 6.5 table 5-22 keep the bounds from
        // merely restating the parser (D-142).
        for (kind, minimum) in [(0u8, 8usize), (1, 12), (2, 10), (0x80, 2)] {
            let short = minimum - 1;
            let mut madt = madt_with_entry(kind, short);
            assert!(
                MadtEntries::new(&madt).is_err(),
                "kind {kind} accepted {short} bytes, one short of {minimum}"
            );

            madt = madt_with_entry(kind, minimum);
            let entries = MadtEntries::new(&madt).expect("a long-enough entry");
            assert_eq!(entries.count(), 1);
        }
    }

    fn madt_with_entry(kind: u8, len: usize) -> Vec<u8> {
        const MADT_HEADER: usize = Sdt::LEN + 8;

        let mut table = vec![0u8; MADT_HEADER + len];
        table[..4].copy_from_slice(&Sdt::MADT);
        let length = u32::try_from(table.len()).expect("fits");
        table[4..8].copy_from_slice(&length.to_le_bytes());
        table[8] = 1;
        if let Some(slot) = table.get_mut(MADT_HEADER) {
            *slot = kind;
        }
        if let Some(slot) = table.get_mut(MADT_HEADER + 1) {
            *slot = u8::try_from(len).expect("a byte");
        }
        let checksum = table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        table[9] = table[9].wrapping_sub(checksum);
        table
    }

    #[test]
    fn a_root_table_above_the_cap_cannot_hide_a_tail_madt() {
        const LISTED: usize = MAX_TABLES + 1;

        let mut table = vec![0u8; Sdt::LEN + LISTED * 8];
        table[..4].copy_from_slice(&Sdt::XSDT);
        let length = u32::try_from(table.len()).expect("fits");
        table[4..8].copy_from_slice(&length.to_le_bytes());
        table[8] = 1;
        for index in 0..LISTED {
            let at = Sdt::LEN + index * 8;
            let address = if index == MAX_TABLES {
                0xfeed_cafe
            } else {
                0x1000u64 + index as u64
            };
            table[at..at + 8].copy_from_slice(&address.to_le_bytes());
        }
        let checksum = table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        table[9] = table[9].wrapping_sub(checksum);

        assert!(matches!(
            SdtPointers::xsdt(&table),
            Err(AcpiError::TooManyTables {
                listed: LISTED,
                limit: MAX_TABLES
            })
        ));
    }

    #[test]
    fn a_root_table_at_the_cap_is_walked_whole() {
        const LISTED: usize = MAX_TABLES;

        let mut table = vec![0u8; Sdt::LEN + LISTED * 4];
        table[..4].copy_from_slice(&Sdt::RSDT);
        let length = u32::try_from(table.len()).expect("fits");
        table[4..8].copy_from_slice(&length.to_le_bytes());
        table[8] = 1;
        for index in 0..LISTED {
            let at = Sdt::LEN + index * 4;
            table[at..at + 4].copy_from_slice(&(0x2000u32 + index as u32).to_le_bytes());
        }
        let checksum = table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        table[9] = table[9].wrapping_sub(checksum);

        let walked: Vec<_> = SdtPointers::rsdt(&table)
            .expect("the exact pointer budget")
            .collect();
        assert_eq!(walked.len(), LISTED);
        assert_eq!(walked[0], 0x2000);
        assert_eq!(walked[LISTED - 1], 0x2000 + LISTED as u64 - 1);
    }

    #[test]
    fn a_root_table_inside_the_cap_is_walked_whole() {
        const LISTED: usize = 3;

        let mut table = vec![0u8; Sdt::LEN + LISTED * 8];
        table[..4].copy_from_slice(&Sdt::XSDT);
        let length = u32::try_from(table.len()).expect("fits");
        table[4..8].copy_from_slice(&length.to_le_bytes());
        table[8] = 1;
        for index in 0..LISTED {
            let at = Sdt::LEN + index * 8;
            table[at..at + 8].copy_from_slice(&(0x2000u64 + index as u64).to_le_bytes());
        }
        let checksum = table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        table[9] = table[9].wrapping_sub(checksum);

        assert_eq!(
            SdtPointers::xsdt(&table)
                .expect("a valid xsdt")
                .collect::<Vec<_>>(),
            [0x2000, 0x2001, 0x2002]
        );
    }

    #[test]
    fn a_well_formed_rsdp_yields_its_pointers() {
        let bytes = rsdp(2);
        let parsed = Rsdp::parse(&bytes).expect("well formed");

        assert_eq!(parsed.xsdt(), Some(0xdead_beef));
        assert_eq!(parsed.rsdt(), Some(0x1234));
    }

    #[test]
    fn root_pointer_lengths_accept_their_exact_budget() {
        let bytes = rsdp(2);
        assert_eq!(Rsdp::parse_length(&bytes, Rsdp::V2_LEN), Ok(Rsdp::V2_LEN));
        assert!(matches!(
            Rsdp::parse_length(&bytes, Rsdp::V2_LEN - 1),
            Err(AcpiError::TooLong { .. })
        ));
    }

    #[test]
    fn an_xsdt_requires_both_a_supporting_revision_and_a_nonzero_address() {
        assert_eq!(
            (Rsdp {
                revision: 2,
                rsdt: 0,
                xsdt: 0,
            })
            .xsdt(),
            None
        );
        assert_eq!(
            (Rsdp {
                revision: 0,
                rsdt: 0,
                xsdt: 0x1_0000_0000,
            })
            .xsdt(),
            None
        );
    }

    #[test]
    fn a_revision_zero_rsdp_has_no_xsdt() {
        let bytes = rsdp(0);
        let parsed = Rsdp::parse(&bytes).expect("well formed");

        assert_eq!(parsed.xsdt(), None, "the field exists but must not be read");
        assert_eq!(parsed.rsdt(), Some(0x1234));
    }

    #[test]
    fn a_bad_rsdp_signature_is_refused() {
        let mut bytes = rsdp(2);
        bytes[0] = b'X';
        assert!(matches!(
            Rsdp::parse(&bytes),
            Err(AcpiError::BadSignature(_))
        ));
    }

    #[test]
    fn either_rsdp_checksum_failing_is_refused() {
        let mut first = rsdp(2);
        first[10] = first[10].wrapping_add(1);
        assert_eq!(Rsdp::parse(&first), Err(AcpiError::BadChecksum));

        // The extended checksum covers bytes the original one does not, so
        // corrupting only those must still be caught.
        let mut second = rsdp(2);
        second[30] = second[30].wrapping_add(1);
        assert_eq!(Rsdp::parse(&second), Err(AcpiError::BadChecksum));
    }

    #[test]
    fn a_truncated_rsdp_is_refused_rather_than_read() {
        assert!(matches!(
            Rsdp::parse(&rsdp(2)[..10]),
            Err(AcpiError::TooShort { .. })
        ));
        assert!(matches!(
            Rsdp::parse(&rsdp(2)[..Rsdp::V1_LEN]),
            Err(AcpiError::TooShort { .. })
        ));
    }

    #[test]
    fn bytes_that_are_not_an_rsdp_are_named_by_their_signature() {
        // An undefined revision must not out-rank the field that says whether
        // these bytes describe an RSDP at all.
        let mut bytes = rsdp(0);
        bytes[0] = b'X';
        bytes[15] = 7;

        assert!(matches!(
            Rsdp::parse(&bytes),
            Err(AcpiError::BadSignature(_))
        ));
    }

    #[test]
    fn undefined_rsdp_revisions_are_refused() {
        assert_eq!(
            Rsdp::parse(&rsdp(1)),
            Err(AcpiError::UnsupportedRevision(1))
        );
        assert_eq!(
            Rsdp::parse(&rsdp(3)),
            Err(AcpiError::UnsupportedRevision(3))
        );
    }

    #[test]
    fn the_extended_rsdp_checksum_covers_its_declared_length() {
        let mut bytes = rsdp(2);
        bytes.resize(40, 0);
        bytes[20..24].copy_from_slice(&40u32.to_le_bytes());
        fix_checksum(&mut bytes[..Rsdp::V1_LEN], 9);
        fix_checksum(&mut bytes, 32);
        assert!(Rsdp::parse(&bytes).is_ok());

        bytes[39] = 1;
        assert_eq!(Rsdp::parse(&bytes), Err(AcpiError::BadChecksum));
    }

    #[test]
    fn an_extended_rsdp_cannot_be_shorter_than_its_fixed_fields() {
        let mut bytes = rsdp(2);
        bytes[20..24].copy_from_slice(&24u32.to_le_bytes());

        assert_eq!(Rsdp::parse(&bytes), Err(AcpiError::BadLength(24)));
    }

    #[test]
    fn an_rsdp_length_cannot_exceed_the_readers_budget() {
        let mut bytes = rsdp(2);
        bytes[20..24].copy_from_slice(&u32::MAX.to_le_bytes());

        assert_eq!(
            Rsdp::parse_length(&bytes, 4096),
            Err(AcpiError::TooLong {
                declared: u32::MAX as usize,
                limit: 4096,
            })
        );
    }

    #[test]
    fn a_table_header_is_validated_against_its_whole_body() {
        let bytes = table(&Sdt::MADT, &[1, 2, 3, 4]);
        let header = Sdt::parse(&bytes).expect("well formed");

        assert_eq!(header.signature(), Sdt::MADT);
        assert_eq!(header.length() as usize, Sdt::LEN + 4);
    }

    #[test]
    fn a_length_can_be_read_before_the_table_is_validated() {
        let bytes = table(&Sdt::MADT, &[1, 2, 3, 4]);
        assert_eq!(Sdt::parse_length(&bytes, usize::MAX), Ok(Sdt::LEN + 4));
        assert_eq!(
            Sdt::parse_length(&bytes[..Sdt::LEN], usize::MAX),
            Ok(Sdt::LEN + 4)
        );

        assert!(matches!(
            Sdt::parse_length(&bytes[..Sdt::LEN - 1], usize::MAX),
            Err(AcpiError::TooShort { .. })
        ));
        let mut short = bytes;
        short[4..8].copy_from_slice(&8u32.to_le_bytes());
        assert_eq!(
            Sdt::parse_length(&short, usize::MAX),
            Err(AcpiError::BadLength(8))
        );
    }

    #[test]
    fn a_header_only_table_and_its_exact_budget_are_valid_lengths() {
        let bytes = table(&Sdt::XSDT, &[]);
        assert_eq!(Sdt::parse_length(&bytes, Sdt::LEN), Ok(Sdt::LEN));
        assert_eq!(
            Sdt::parse(&bytes).map(|header| header.length()),
            Ok(Sdt::LEN as u32)
        );
        assert!(matches!(
            Sdt::parse_length(&bytes, Sdt::LEN - 1),
            Err(AcpiError::TooLong { .. })
        ));
    }

    #[test]
    fn an_unvalidated_length_cannot_exceed_the_readers_budget() {
        let mut header = [0u8; Sdt::LEN];
        header[4..8].copy_from_slice(&u32::MAX.to_le_bytes());

        assert_eq!(
            Sdt::parse_length(&header, 1 << 20),
            Err(AcpiError::TooLong {
                declared: u32::MAX as usize,
                limit: 1 << 20,
            })
        );
    }

    #[test]
    fn a_table_whose_body_is_missing_is_refused() {
        let bytes = table(&Sdt::MADT, &[1, 2, 3, 4]);
        assert!(matches!(
            Sdt::parse(&bytes[..Sdt::LEN + 2]),
            Err(AcpiError::TooShort { .. })
        ));
    }

    #[test]
    fn a_table_shorter_than_its_own_header_is_refused() {
        let mut bytes = table(&Sdt::MADT, &[]);
        bytes[4..8].copy_from_slice(&8u32.to_le_bytes());
        assert_eq!(Sdt::parse(&bytes), Err(AcpiError::BadLength(8)));
    }

    #[test]
    fn a_corrupt_table_body_fails_the_checksum() {
        let mut bytes = table(&Sdt::MADT, &[1, 2, 3, 4]);
        let last = bytes.len() - 1;
        bytes[last] = bytes[last].wrapping_add(1);
        assert_eq!(Sdt::parse(&bytes), Err(AcpiError::BadChecksum));
    }

    #[test]
    fn an_xsdt_lists_its_tables() {
        let mut body = Vec::new();
        for address in [0x1000u64, 0x2000, 0x3000] {
            body.extend_from_slice(&address.to_le_bytes());
        }
        let bytes = table(&Sdt::XSDT, &body);
        let pointers: Vec<_> = SdtPointers::xsdt(&bytes).expect("validated xsdt").collect();
        assert_eq!(pointers, [0x1000, 0x2000, 0x3000]);
    }

    #[test]
    fn an_xsdt_pointer_keeps_its_high_thirty_two_bits() {
        let address = 0x1234_5678_9abc_def0u64;
        let bytes = table(&Sdt::XSDT, &address.to_le_bytes());
        assert_eq!(
            SdtPointers::xsdt(&bytes)
                .expect("validated xsdt")
                .collect::<Vec<_>>(),
            [address]
        );
    }

    #[test]
    fn an_rsdt_lists_its_tables_in_thirty_two_bits() {
        let mut body = Vec::new();
        for address in [0x1000u32, 0x2000] {
            body.extend_from_slice(&address.to_le_bytes());
        }
        let bytes = table(&Sdt::RSDT, &body);
        let pointers: Vec<_> = SdtPointers::rsdt(&bytes).expect("validated rsdt").collect();
        assert_eq!(pointers, [0x1000, 0x2000]);
    }

    #[test]
    fn a_typed_pointer_view_refuses_the_wrong_table() {
        let bytes = table(&Sdt::MADT, &[0; 8]);

        assert!(matches!(
            SdtPointers::xsdt(&bytes),
            Err(AcpiError::WrongTable {
                expected: Sdt::XSDT,
                found: Sdt::MADT,
            })
        ));
    }

    #[test]
    fn a_pointer_table_with_a_partial_tail_is_invalid() {
        let bytes = table(&Sdt::XSDT, &[0; 9]);

        assert_eq!(
            SdtPointers::xsdt(&bytes).map(|pointers| pointers.count()),
            Err(AcpiError::PartialEntry {
                width: 8,
                trailing: 1,
            })
        );
    }

    #[test]
    fn a_pointer_list_stops_at_the_declared_length() {
        let mut bytes = table(&Sdt::XSDT, &0x1000u64.to_le_bytes());
        bytes.extend_from_slice(&0xbadu64.to_le_bytes());

        let pointers: Vec<_> = SdtPointers::xsdt(&bytes).expect("validated xsdt").collect();
        assert_eq!(
            pointers,
            [0x1000],
            "the appended entry is outside the table"
        );
    }

    #[test]
    fn a_madt_yields_the_entries_this_kernel_acts_on() {
        let mut body = Vec::new();
        body.extend_from_slice(&0xfee0_0000u32.to_le_bytes()); // local APIC
        body.extend_from_slice(&1u32.to_le_bytes()); // flags
        body.extend_from_slice(&[0, 8, 3, 7, 1, 0, 0, 0]); // local APIC entry
        body.extend_from_slice(&[1, 12, 2, 0]); // I/O APIC entry
        body.extend_from_slice(&0xfec0_0000u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&[2, 10, 0, 1]); // source override, IRQ 1
        body.extend_from_slice(&9u32.to_le_bytes());
        body.extend_from_slice(&0x000fu16.to_le_bytes());
        body.extend_from_slice(&[4, 6, 0, 0, 0, 0]); // an NMI entry, not ours

        let bytes = table(&Sdt::MADT, &body);
        let entries: Vec<_> = MadtEntries::new(&bytes).expect("validated madt").collect();
        assert_eq!(
            entries,
            [
                MadtEntry::LocalApic {
                    processor: 3,
                    id: 7,
                    enabled: true
                },
                MadtEntry::IoApic {
                    id: 2,
                    address: 0xfec0_0000,
                    gsi_base: 0
                },
                MadtEntry::InterruptSourceOverride {
                    bus: 0,
                    source: 1,
                    gsi: 9,
                    flags: 0x000f
                },
                MadtEntry::Other { kind: 4 },
            ]
        );
    }

    #[test]
    fn a_local_apic_without_the_enabled_bit_is_disabled() {
        let bytes = madt_with_entry(0, 8);
        assert_eq!(
            core::iter::Iterator::next(&mut MadtEntries::new(&bytes).expect("valid madt")),
            Some(MadtEntry::LocalApic {
                processor: 0,
                id: 0,
                enabled: false,
            })
        );
    }

    #[test]
    fn io_apic_accessors_return_each_firmware_field() {
        let controller = IoApic {
            id: 7,
            address: 0xfec1_0000,
            gsi_base: 24,
        };
        assert_eq!(controller.id(), 7);
        assert_eq!(controller.address(), 0xfec1_0000);
        assert_eq!(controller.gsi_base(), 24);
    }

    fn madt_with_overrides(overrides: &[(u8, u8, u32, u16)]) -> Vec<u8> {
        let mut body = vec![0u8; 8];
        for (bus, source, gsi, flags) in overrides {
            body.extend_from_slice(&[2, 10, *bus, *source]);
            body.extend_from_slice(&gsi.to_le_bytes());
            body.extend_from_slice(&flags.to_le_bytes());
        }
        table(&Sdt::MADT, &body)
    }

    #[test]
    fn interrupt_overrides_are_unique_isa_sources_with_defined_flags() {
        for entries in [[(0, 1, 9, 0), (0, 1, 10, 0)], [(0, 1, 10, 0), (0, 1, 9, 0)]] {
            assert_eq!(
                MadtInterrupts::<1>::parse(&madt_with_overrides(&entries)),
                Err(InterruptError::DuplicateOverride(1))
            );
        }

        assert_eq!(
            MadtInterrupts::<1>::parse(&madt_with_overrides(&[(1, 1, 9, 0)])),
            Err(InterruptError::NonIsaOverride { bus: 1, source: 1 })
        );
        assert_eq!(
            MadtInterrupts::<1>::parse(&madt_with_overrides(&[(0, 16, 9, 0)])),
            Err(InterruptError::InvalidOverrideSource(16))
        );
        for flags in [0b0010, 0b1000, 0x0010] {
            assert_eq!(
                MadtInterrupts::<1>::parse(&madt_with_overrides(&[(0, 1, 9, flags)])),
                Err(InterruptError::ReservedOverrideFlags { source: 1, flags })
            );
        }
    }

    #[test]
    fn a_bounded_interrupt_inventory_keeps_every_controller() {
        let mut body = vec![0u8; 8];
        body.extend_from_slice(&[1, 12, 7, 0]);
        body.extend_from_slice(&0xfec0_0000u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&[2, 10, 0, 1]);
        body.extend_from_slice(&25u32.to_le_bytes());
        body.extend_from_slice(&0x000fu16.to_le_bytes());
        body.extend_from_slice(&[1, 12, 9, 0]);
        body.extend_from_slice(&0xfec1_0000u32.to_le_bytes());
        body.extend_from_slice(&24u32.to_le_bytes());
        let bytes = table(&Sdt::MADT, &body);

        let inventory = MadtInterrupts::<2>::parse(&bytes).expect("two controllers fit");
        assert_eq!(inventory.io_apic_count(), 2);
        assert_eq!(
            inventory.io_apics().collect::<Vec<_>>(),
            [
                IoApic {
                    id: 7,
                    address: 0xfec0_0000,
                    gsi_base: 0,
                },
                IoApic {
                    id: 9,
                    address: 0xfec1_0000,
                    gsi_base: 24,
                },
            ]
        );
        assert_eq!(inventory.gsi_for(1), 25);
        assert_eq!(inventory.flags_for(1), 0x000f);
        assert_eq!(inventory.gsi_for(2), 2);
        assert_eq!(inventory.flags_for(2), 0);
    }

    #[test]
    fn an_interrupt_inventory_refuses_overflow_instead_of_truncating() {
        let mut body = vec![0u8; 8];
        for (id, address, base) in [
            (1, 0xfec0_0000u32, 0u32),
            (2, 0xfec1_0000, 24),
            (3, 0xfec2_0000, 48),
        ] {
            body.extend_from_slice(&[1, 12, id, 0]);
            body.extend_from_slice(&address.to_le_bytes());
            body.extend_from_slice(&base.to_le_bytes());
        }
        let bytes = table(&Sdt::MADT, &body);

        assert_eq!(
            MadtInterrupts::<2>::parse(&bytes).map(|inventory| inventory.io_apic_count()),
            Err(InterruptError::TooManyIoApics { limit: 2 })
        );
    }

    #[test]
    fn a_madt_entry_claiming_zero_length_invalidates_the_table() {
        // The corrupt input that would otherwise loop forever: an entry whose
        // length never advances the cursor.
        let mut body = vec![0u8; 8];
        body.extend_from_slice(&[1, 0, 0, 0]);
        let bytes = table(&Sdt::MADT, &body);
        assert!(matches!(
            MadtEntries::new(&bytes),
            Err(AcpiError::MalformedEntry { length: 0, .. })
        ));
    }

    #[test]
    fn a_madt_without_its_fixed_header_is_invalid() {
        let bytes = table(&Sdt::MADT, &[]);

        assert_eq!(
            MadtEntries::new(&bytes).map(|entries| entries.count()),
            Err(AcpiError::BadLength(Sdt::LEN as u32))
        );
    }

    #[test]
    fn a_corrupt_tail_invalidates_valid_madt_entries_before_it() {
        let mut body = vec![0u8; 8];
        body.extend_from_slice(&[1, 12, 2, 0]);
        body.extend_from_slice(&0xfec0_0000u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&[0, 0]);
        let bytes = table(&Sdt::MADT, &body);

        assert!(matches!(
            MadtEntries::new(&bytes),
            Err(AcpiError::MalformedEntry { length: 0, .. })
        ));
    }

    #[test]
    fn a_madt_entry_running_past_the_table_invalidates_it() {
        let mut body = vec![0u8; 8];
        body.extend_from_slice(&[1, 40, 0, 0]);
        let bytes = table(&Sdt::MADT, &body);
        assert!(matches!(
            MadtEntries::new(&bytes),
            Err(AcpiError::MalformedEntry {
                length: 40,
                available: 4,
                ..
            })
        ));
    }

    #[test]
    fn a_truncated_entry_of_a_known_kind_invalidates_the_table() {
        let mut body = vec![0u8; 8];
        body.extend_from_slice(&[1, 10, 2, 0, 0, 0, 0, 0, 0, 0]);
        let bytes = table(&Sdt::MADT, &body);
        assert!(matches!(
            MadtEntries::new(&bytes),
            Err(AcpiError::MalformedEntry { length: 10, .. })
        ));
    }

    #[test]
    fn the_system_description_header_is_thirty_six_bytes() {
        // Independent field widths from ACPI 6.5 table 5.4 prevent a
        // self-referential fixture from accepting a changed header size.
        assert_eq!(Sdt::LEN, 36);
        assert_eq!(4 + 4 + 1 + 1 + 6 + 8 + 4 + 4 + 4, Sdt::LEN);
    }

    fn dmar_body(structures: &[u8]) -> Vec<u8> {
        let mut body = vec![0u8; 12]; // host address width, flags, 10 reserved
        body.extend_from_slice(structures);
        body
    }

    fn drhd_bytes(
        include_pci_all: bool,
        segment: u16,
        register_base: u64,
        scopes: &[u8],
    ) -> Vec<u8> {
        let mut bytes = vec![0u8; 16];
        bytes[0..2].copy_from_slice(&0u16.to_le_bytes());
        bytes[4] = u8::from(include_pci_all);
        bytes[6..8].copy_from_slice(&segment.to_le_bytes());
        bytes[8..16].copy_from_slice(&register_base.to_le_bytes());
        bytes.extend_from_slice(scopes);
        let len = u16::try_from(bytes.len()).expect("fits");
        bytes[2..4].copy_from_slice(&len.to_le_bytes());
        bytes
    }

    fn rmrr_bytes(segment: u16, base: u64, limit: u64, scopes: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 24];
        bytes[0..2].copy_from_slice(&1u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&segment.to_le_bytes());
        bytes[8..16].copy_from_slice(&base.to_le_bytes());
        bytes[16..24].copy_from_slice(&limit.to_le_bytes());
        bytes.extend_from_slice(scopes);
        let len = u16::try_from(bytes.len()).expect("fits");
        bytes[2..4].copy_from_slice(&len.to_le_bytes());
        bytes
    }

    fn atsr_bytes(segment: u16, scopes: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 8];
        bytes[0..2].copy_from_slice(&2u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&segment.to_le_bytes());
        bytes.extend_from_slice(scopes);
        let len = u16::try_from(bytes.len()).expect("fits");
        bytes[2..4].copy_from_slice(&len.to_le_bytes());
        bytes
    }

    fn rhsa_bytes(register_base: u64, proximity_domain: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; 20];
        bytes[0..2].copy_from_slice(&3u16.to_le_bytes());
        bytes[2..4].copy_from_slice(&20u16.to_le_bytes());
        bytes[8..16].copy_from_slice(&register_base.to_le_bytes());
        bytes[16..20].copy_from_slice(&proximity_domain.to_le_bytes());
        bytes
    }

    fn pci_endpoint_scope(start_bus: u8, device: u8, function: u8) -> Vec<u8> {
        vec![1, 8, 0, 0, 0, start_bus, device, function]
    }

    #[test]
    fn a_dmar_yields_its_structures_and_resolves_each_bdf_to_its_unit_and_regions() {
        let scoped_target = pci_endpoint_scope(2, 3, 0);
        let rmrr_scope = pci_endpoint_scope(2, 3, 0);
        let mut structures = Vec::new();
        structures.extend_from_slice(&drhd_bytes(true, 0, 0xfed9_0000, &[]));
        structures.extend_from_slice(&drhd_bytes(false, 0, 0xfed9_1000, &scoped_target));
        structures.extend_from_slice(&rmrr_bytes(0, 0x1000, 0x1fff, &rmrr_scope));
        structures.extend_from_slice(&atsr_bytes(0, &[]));
        structures.extend_from_slice(&rhsa_bytes(0xfed9_1000, 1));
        let bytes = table(&Sdt::DMAR, &dmar_body(&structures));

        let parsed: Vec<_> = DmarEntries::new(&bytes).expect("validated dmar").collect();
        assert_eq!(
            parsed,
            [
                DmarStructure::Drhd {
                    include_pci_all: true,
                    segment: 0,
                    register_base: 0xfed9_0000,
                    scopes: &[],
                },
                DmarStructure::Drhd {
                    include_pci_all: false,
                    segment: 0,
                    register_base: 0xfed9_1000,
                    scopes: &scoped_target,
                },
                DmarStructure::Rmrr {
                    segment: 0,
                    base: 0x1000,
                    limit: 0x1fff,
                    scopes: &rmrr_scope,
                },
                DmarStructure::Atsr {
                    segment: 0,
                    scopes: &[],
                },
                DmarStructure::Rhsa {
                    register_base: 0xfed9_1000,
                    proximity_domain: 1,
                },
            ]
        );

        let dmar = Dmar::new(&bytes).expect("no duplicate units, no inverted regions");
        let scoped_bdf = Bdf::new(0, 2, 3, 0);
        let unscoped_bdf = Bdf::new(0, 9, 0, 0);
        assert_eq!(dmar.unit_for(scoped_bdf), Some(0xfed9_1000));
        assert_eq!(dmar.unit_for(unscoped_bdf), Some(0xfed9_0000));
        assert_eq!(
            dmar.reserved_regions_for(scoped_bdf).collect::<Vec<_>>(),
            [(0x1000, 0x1fff)]
        );
        assert_eq!(
            dmar.reserved_regions_for(unscoped_bdf).collect::<Vec<_>>(),
            []
        );
    }

    #[test]
    fn a_dmar_view_refuses_a_table_that_is_not_a_dmar() {
        let bytes = table(&Sdt::MADT, &[0; 12]);
        assert!(matches!(
            DmarEntries::new(&bytes),
            Err(AcpiError::WrongTable {
                expected: Sdt::DMAR,
                found: Sdt::MADT,
            })
        ));
    }

    #[test]
    fn a_dmar_without_its_fixed_preamble_is_invalid() {
        let bytes = table(&Sdt::DMAR, &[]);
        assert_eq!(
            DmarEntries::new(&bytes).map(|entries| entries.count()),
            Err(AcpiError::BadLength(Sdt::LEN as u32))
        );
    }

    #[test]
    fn a_truncated_dmar_structure_header_is_refused_not_read() {
        let mut body = dmar_body(&[]);
        body.extend_from_slice(&[0, 0, 16]); // an incomplete type+length prefix
        let bytes = table(&Sdt::DMAR, &body);
        assert!(matches!(
            DmarEntries::new(&bytes),
            Err(AcpiError::MalformedEntry { length: 0, .. })
        ));
    }

    #[test]
    fn a_dmar_structure_claiming_zero_length_invalidates_the_table() {
        // The corrupt input that would otherwise loop forever: a length
        // that never advances the cursor.
        let mut body = dmar_body(&[]);
        body.extend_from_slice(&[0, 0, 0, 0]); // kind 0 (DRHD), length 0
        let bytes = table(&Sdt::DMAR, &body);
        assert!(matches!(
            DmarEntries::new(&bytes),
            Err(AcpiError::MalformedEntry { length: 0, .. })
        ));
    }

    #[test]
    fn a_dmar_structure_running_past_the_table_invalidates_it() {
        let mut body = dmar_body(&[]);
        body.extend_from_slice(&[0, 0, 40, 0]); // kind 0, length 40, 4 bytes actually present
        let bytes = table(&Sdt::DMAR, &body);
        assert!(matches!(
            DmarEntries::new(&bytes),
            Err(AcpiError::MalformedEntry {
                length: 40,
                available: 4,
                ..
            })
        ));
    }

    #[test]
    fn a_device_scope_running_past_its_structure_invalidates_the_table() {
        let short_scope = [1u8, 8, 0, 0, 0, 0]; // claims 8 bytes; only 6 are present
        let drhd = drhd_bytes(false, 0, 0x1000, &short_scope);
        let bytes = table(&Sdt::DMAR, &dmar_body(&drhd));
        assert!(matches!(
            DmarEntries::new(&bytes),
            Err(AcpiError::MalformedEntry {
                length: 8,
                available: 6,
                ..
            })
        ));
    }

    #[test]
    fn an_unknown_dmar_structure_is_preserved_as_other_and_does_not_stop_the_walk() {
        let mut structures = vec![9, 0, 8, 0, 1, 2, 3, 4]; // kind 9, length 8
        structures.extend_from_slice(&drhd_bytes(true, 0, 0x2000, &[]));
        let bytes = table(&Sdt::DMAR, &dmar_body(&structures));

        let parsed: Vec<_> = DmarEntries::new(&bytes)
            .expect("unknown kinds still validate")
            .collect();
        assert_eq!(
            parsed,
            [
                DmarStructure::Other { kind: 9 },
                DmarStructure::Drhd {
                    include_pci_all: true,
                    segment: 0,
                    register_base: 0x2000,
                    scopes: &[],
                },
            ]
        );
    }

    #[test]
    fn a_dmar_structure_shorter_than_its_fields_is_refused_not_read() {
        // Independent field widths from the Intel VT-d specification's
        // DRHD, RMRR, ATSR, and RHSA structure layouts keep the bounds from
        // merely restating the parser (D-142).
        for (kind, minimum) in [(0u16, 16usize), (1, 24), (2, 8), (3, 20), (0x80, 4)] {
            let short_len = minimum - 1;
            let mut short = vec![0u8; short_len];
            if short_len >= 4 {
                short[0..2].copy_from_slice(&kind.to_le_bytes());
                let len = u16::try_from(short_len).expect("fits");
                short[2..4].copy_from_slice(&len.to_le_bytes());
            }
            let bytes = table(&Sdt::DMAR, &dmar_body(&short));
            assert!(
                DmarEntries::new(&bytes).is_err(),
                "kind {kind} accepted {short_len} bytes, one short of {minimum}"
            );

            let mut exact = vec![0u8; minimum];
            exact[0..2].copy_from_slice(&kind.to_le_bytes());
            let len = u16::try_from(minimum).expect("fits");
            exact[2..4].copy_from_slice(&len.to_le_bytes());
            let bytes = table(&Sdt::DMAR, &dmar_body(&exact));
            assert_eq!(
                DmarEntries::new(&bytes).map(|entries| entries.count()),
                Ok(1),
                "kind {kind} refused exactly {minimum} bytes"
            );
        }
    }

    #[test]
    fn the_first_remapping_structure_begins_after_the_dmar_preamble() {
        // Independent field widths from the Intel VT-d specification's DMAR
        // table header (host address width, flags, ten reserved bytes)
        // prevent a self-referential fixture from accepting a changed
        // preamble size.
        assert_eq!(DmarEntries::HEADER, 48);
        assert_eq!(Sdt::LEN + 1 + 1 + 10, DmarEntries::HEADER);
    }

    #[test]
    fn dmar_errors_name_the_table_or_domain_rule_broken() {
        assert_eq!(
            DmarError::Table(AcpiError::BadChecksum).to_string(),
            "dmar is unusable: table checksum does not sum to zero"
        );
        assert_eq!(
            DmarError::ReservedRegionInverted {
                base: 0x2000,
                limit: 0x1000,
            }
            .to_string(),
            "dmar reserved region [0x2000, 0x1000] has its limit before its base"
        );
        assert_eq!(
            DmarError::DuplicateUnit {
                register_base: 0xfed9_0000
            }
            .to_string(),
            "dmar has two drhd units at register base 0xfed90000"
        );
    }

    #[test]
    fn an_inverted_reserved_region_is_refused_by_the_validated_view() {
        let rmrr = rmrr_bytes(0, 0x2000, 0x1000, &[]);
        let bytes = table(&Sdt::DMAR, &dmar_body(&rmrr));

        // The raw walk does not know the domain rule and still yields the
        // structure; only the validated `Dmar` view refuses it.
        assert!(DmarEntries::new(&bytes).is_ok());
        assert_eq!(
            Dmar::new(&bytes),
            Err(DmarError::ReservedRegionInverted {
                base: 0x2000,
                limit: 0x1000,
            })
        );
    }

    #[test]
    fn a_one_byte_reserved_region_has_base_equal_to_limit_and_is_accepted() {
        let rmrr = rmrr_bytes(0, 0x3000, 0x3000, &[]);
        let bytes = table(&Sdt::DMAR, &dmar_body(&rmrr));
        assert!(Dmar::new(&bytes).is_ok());
    }

    #[test]
    fn two_drhds_at_the_same_register_base_are_refused_by_the_validated_view() {
        let mut structures = drhd_bytes(true, 0, 0xfed9_0000, &[]);
        structures.extend_from_slice(&drhd_bytes(false, 1, 0xfed9_0000, &[]));
        let bytes = table(&Sdt::DMAR, &dmar_body(&structures));

        assert!(DmarEntries::new(&bytes).is_ok());
        assert_eq!(
            Dmar::new(&bytes),
            Err(DmarError::DuplicateUnit {
                register_base: 0xfed9_0000
            })
        );
    }

    #[test]
    fn a_multi_hop_device_scope_path_is_exposed_but_not_resolved_to_a_bdf() {
        let scope_bytes = [1u8, 10, 0, 0, 0, 2, 3, 0, 4, 0]; // kind 1, length 10, two hops
        let drhd = drhd_bytes(false, 0, 0x4000, &scope_bytes);
        let bytes = table(&Sdt::DMAR, &dmar_body(&drhd));

        let structure = DmarEntries::new(&bytes)
            .expect("validated dmar")
            .next()
            .expect("one drhd");
        let DmarStructure::Drhd { scopes, .. } = structure else {
            unreachable!("only structure in the table is the drhd")
        };
        let scope = DeviceScopes::new(scopes).next().expect("one device scope");
        assert_eq!(scope.start_bus(), 2);
        assert_eq!(scope.path().collect::<Vec<_>>(), [(3, 0), (4, 0)]);
        assert_eq!(scope.pci_target(), None);
    }

    #[test]
    fn an_unknown_device_scope_kind_is_preserved_as_other() {
        let scope_bytes = [0x7fu8, 6, 0, 0, 0, 0]; // kind 0x7f, length 6, no path
        let drhd = drhd_bytes(false, 0, 0x5000, &scope_bytes);
        let bytes = table(&Sdt::DMAR, &dmar_body(&drhd));

        let structure = DmarEntries::new(&bytes)
            .expect("validated dmar")
            .next()
            .expect("one drhd");
        let DmarStructure::Drhd { scopes, .. } = structure else {
            unreachable!("only structure in the table is the drhd")
        };
        let scope = DeviceScopes::new(scopes).next().expect("one device scope");
        assert_eq!(scope.kind(), DeviceScopeKind::Other { kind: 0x7f });
    }
}
