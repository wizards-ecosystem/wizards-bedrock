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
    start_bus: u8,
    path: &'a [u8],
}

impl<'a> DeviceScope<'a> {
    /// Bytes in a scope entry with no path hops: the type, length, flags,
    /// reserved byte, enumeration id, and start bus.
    const MIN_LEN: usize = 6;

    /// Path hops a scope may take; a longer path is refused, not walked.
    const MAX_HOPS: usize = 8;

    /// The hardware this scope names.
    pub const fn kind(self) -> DeviceScopeKind {
        self.kind
    }

    /// The first bus number in the hierarchical path to the device.
    pub const fn start_bus(self) -> u8 {
        self.start_bus
    }

    /// The (device, function) hops from `start_bus` down to the target.
    pub const fn path(self) -> PciPath<'a> {
        PciPath { bytes: self.path }
    }

    /// What this scope names in `segment`, walked through the live topology.
    ///
    /// Each hop before the last must be a bridge, and the next hop lies on
    /// its secondary bus. `None` for a kind that names no PCI function, an
    /// empty or overlong path, or a hop the topology does not vouch for.
    fn walk(self, segment: u16, bridge: &impl Fn(Bdf) -> Option<BusRange>) -> Option<Named> {
        let hierarchy = match self.kind() {
            DeviceScopeKind::PciEndpoint => false,
            DeviceScopeKind::PciSubHierarchy => true,
            // No PCI function: I/O APIC and HPET scopes serve interrupt remapping (PW-710).
            DeviceScopeKind::IoApic
            | DeviceScopeKind::Hpet
            | DeviceScopeKind::AcpiNamespaceDevice
            | DeviceScopeKind::Other { .. } => return None,
        };
        if self.path.len() > 2 * Self::MAX_HOPS {
            return None;
        }
        let mut hops = self.path();
        let (mut device, mut function) = hops.next()?;
        let mut bus = self.start_bus();
        let mut bridge_path = [None; Self::MAX_HOPS];
        let mut bridge_count = 0usize;
        for (next_device, next_function) in hops {
            let at = Bdf::new(segment, bus, device, function);
            bridge_path[bridge_count] = Some(at);
            bridge_count += 1;
            let buses = behind(at, bridge)?;
            bus = buses.secondary;
            (device, function) = (next_device, next_function);
        }
        let at = Bdf::new(segment, bus, device, function);
        if hierarchy {
            bridge_path[bridge_count] = Some(at);
            bridge_count += 1;
            Some(Named::Hierarchy(Hierarchy {
                root: at,
                buses: behind(at, bridge)?,
                bridge_path,
                bridge_count,
            }))
        } else {
            Some(Named::Function(at))
        }
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

/// The buses a PCI-to-PCI bridge forwards to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusRange {
    /// The bus directly behind the bridge.
    pub secondary: u8,
    /// The highest-numbered bus behind the bridge.
    pub subordinate: u8,
}

/// The buses behind the bridge `bridge` reports at `at`, if its numbering
/// leads forward: a secondary bus past its own and no later than its
/// subordinate, so a walk never returns to a bus it left.
fn behind(at: Bdf, bridge: &impl Fn(Bdf) -> Option<BusRange>) -> Option<BusRange> {
    let buses = bridge(at)?;
    (at.bus() < buses.secondary && buses.secondary <= buses.subordinate).then_some(buses)
}

/// What a PCI device scope names once its path is walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Named {
    /// Exactly one function.
    Function(Bdf),
    /// A bridge, and every function on the buses behind it.
    Hierarchy(Hierarchy),
}

/// One resolved hierarchy and the exact live bridge path that reached it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hierarchy {
    root: Bdf,
    buses: BusRange,
    bridge_path: [Option<Bdf>; DeviceScope::MAX_HOPS],
    bridge_count: usize,
}

impl Named {
    /// Whether this names `bdf`, as itself or behind a named bridge.
    fn covers(self, bdf: Bdf) -> bool {
        match self {
            Self::Function(at) => at == bdf,
            Self::Hierarchy(hierarchy) => {
                hierarchy.root == bdf
                    || (hierarchy.buses.secondary..=hierarchy.buses.subordinate)
                        .contains(&bdf.bus())
            }
        }
    }

    /// Whether this hierarchy strictly contains another through the live
    /// bridge topology, rather than merely overlapping its numeric bus range.
    fn strictly_contains(self, inner: Self) -> bool {
        let (Self::Hierarchy(outer), Self::Hierarchy(inner)) = (self, inner) else {
            return false;
        };
        outer.root != inner.root
            && inner.bridge_path[..inner.bridge_count].contains(&Some(outer.root))
            && outer.buses.secondary <= inner.buses.secondary
            && inner.buses.subordinate <= outer.buses.subordinate
    }
}

/// How a remapping unit's scope reaches one function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// An endpoint scope names the function itself.
    Endpoint,
    /// A sub-hierarchy scope names this bridge, and the function is the
    /// bridge or behind it.
    Bridge(Bdf),
    /// No scope names the function, and the unit sets `INCLUDE_PCI_ALL`.
    CatchAll,
}

/// The remapping unit that governs one function, and the scope that chose it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoverningUnit {
    /// Physical address of the unit's register set.
    pub register_base: u64,
    /// How the unit's scope reaches the function.
    pub coverage: Coverage,
}

/// Every live PCI requester proved to belong to one remapping unit.
///
/// The fixed storage is deliberate: callers must choose and enforce their
/// ownership bound before a unit-wide hardware transition can begin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequesterInventory<const N: usize> {
    unit: GoverningUnit,
    requesters: [Option<Bdf>; N],
    count: usize,
}

impl<const N: usize> RequesterInventory<N> {
    /// The exact DRHD whose requester inventory was accepted.
    pub const fn unit(&self) -> GoverningUnit {
        self.unit
    }

    /// Number of distinct live requesters in the unit.
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Whether the inventory contains no requester.
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The distinct requester BDFs in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = Bdf> + '_ {
        self.requesters[..self.count].iter().flatten().copied()
    }
}

/// Why one DRHD cannot become a complete fixed requester inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequesterInventoryError {
    /// No unambiguous unit governs the selected function.
    Ungoverned,
    /// The selected function is governed only by catch-all coverage.
    SelectedUsesCatchAll,
    /// The unit can govern functions not named by an endpoint scope.
    IncludesAllPci,
    /// A PCI scope could not be resolved through the supplied topology.
    UnresolvedScope,
    /// A declared endpoint resolves to another unit or ambiguously.
    ForeignEndpoint,
    /// A non-interrupt scope can name a requester the PCI inventory cannot fence.
    UnsupportedScope,
    /// A present requester within a hierarchy has ambiguous governance.
    AmbiguousRequester,
    /// The selected function was not among the unit's resolved requesters.
    MissingSelected,
    /// The unit has more distinct live requesters than the caller can own.
    TooManyRequesters {
        /// Fixed requester capacity supplied by the caller.
        maximum: usize,
    },
    /// One unit declares more PCI requester scopes than a boot may probe.
    TooManyRequesterScopes {
        /// Fixed per-unit scope budget.
        maximum: usize,
    },
}

impl fmt::Display for RequesterInventoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ungoverned => core::fmt::Write::write_str(
                f,
                "no unambiguous remapping unit governs the selected function",
            ),
            Self::SelectedUsesCatchAll => core::fmt::Write::write_str(
                f,
                "the selected function is governed only by catch-all coverage",
            ),
            Self::IncludesAllPci => core::fmt::Write::write_str(
                f,
                "the remapping unit includes unlisted PCI requesters",
            ),
            Self::UnresolvedScope => core::fmt::Write::write_str(
                f,
                "a remapping-unit PCI scope does not resolve through the topology",
            ),
            Self::ForeignEndpoint => core::fmt::Write::write_str(
                f,
                "a declared endpoint is not governed by the same remapping unit",
            ),
            Self::UnsupportedScope => core::fmt::Write::write_str(
                f,
                "the remapping unit contains an unsupported requester scope",
            ),
            Self::AmbiguousRequester => core::fmt::Write::write_str(
                f,
                "a present hierarchy requester has ambiguous remapping-unit governance",
            ),
            Self::MissingSelected => core::fmt::Write::write_str(
                f,
                "the selected function is absent from the remapping-unit requester inventory",
            ),
            Self::TooManyRequesters { maximum } => {
                write!(
                    f,
                    "the remapping unit has more than {maximum} live requesters"
                )
            }
            Self::TooManyRequesterScopes { maximum } => write!(
                f,
                "the remapping unit has more than {maximum} PCI requester scopes"
            ),
        }
    }
}

impl core::error::Error for RequesterInventoryError {}

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
    /// More units were supplied than the allocation-free semantic pass admits.
    TooManyUnits {
        /// Maximum number of DRHD units accepted in one table.
        maximum: usize,
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
            Self::TooManyUnits { maximum } => {
                write!(f, "dmar has more than {maximum} drhd units")
            }
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

fn push_requester<const N: usize>(
    requesters: &mut [Option<Bdf>; N],
    count: &mut usize,
    requester: Bdf,
) -> Result<(), RequesterInventoryError> {
    if requesters[..*count].contains(&Some(requester)) {
        return Ok(());
    }
    let Some(slot) = requesters.get_mut(*count) else {
        return Err(RequesterInventoryError::TooManyRequesters { maximum: N });
    };
    *slot = Some(requester);
    *count += 1;
    Ok(())
}

fn add_live_requester<const N: usize>(
    dmar: &Dmar<'_>,
    bridge: &impl Fn(Bdf) -> Option<BusRange>,
    register_base: u64,
    requester: Bdf,
    requesters: &mut [Option<Bdf>; N],
    count: &mut usize,
) -> Result<(), RequesterInventoryError> {
    match dmar.unit_for(requester, bridge) {
        Some(governing) if governing.register_base == register_base => {
            push_requester(requesters, count, requester)
        }
        // A more-specific scope may validly remove one requester from a broad
        // hierarchy and assign it to another unit.
        Some(_) => Ok(()),
        None => Err(RequesterInventoryError::AmbiguousRequester),
    }
}

/// A platform needs only a handful of units. Bounding this allocation-free
/// inventory also bounds the duplicate scan below on hostile firmware.
const MAX_DRHD_UNITS: usize = 64;

/// A requester inventory may perform a full bounded PCI scan for each
/// hierarchy. Keep hostile duplicate scopes from amplifying that proof into an
/// impractical number of configuration cycles.
const MAX_REQUESTER_SCOPES: usize = 16;

impl<'a> Dmar<'a> {
    /// Validate `table` as a DMAR, bound its unit inventory, and additionally
    /// enforce the two rules bytes alone cannot express: an RMRR's limit must
    /// not precede its base, and no two DRHDs may claim the same register base.
    ///
    /// # Errors
    ///
    /// [`DmarError::Table`] if the table is not a structurally valid DMAR,
    /// [`DmarError::ReservedRegionInverted`] for a backwards RMRR range, or
    /// [`DmarError::DuplicateUnit`] if two DRHDs share a register base, or
    /// [`DmarError::TooManyUnits`] if the semantic pass would exceed its bound.
    pub fn new(table: &'a [u8]) -> Result<Self, DmarError> {
        let entries = DmarEntries::new(table).map_err(DmarError::Table)?;
        let dmar = Self { body: entries.body };

        let units = dmar
            .entries()
            .filter(|structure| matches!(structure, DmarStructure::Drhd { .. }))
            .take(MAX_DRHD_UNITS + 1)
            .count();
        if units > MAX_DRHD_UNITS {
            return Err(DmarError::TooManyUnits {
                maximum: MAX_DRHD_UNITS,
            });
        }

        // O(n^2), not a growable set; the explicit unit ceiling above keeps
        // this allocation-free duplicate pass finite on hostile firmware.
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

    /// How many remapping units the table describes.
    pub fn unit_count(&self) -> usize {
        self.entries()
            .filter(|structure| matches!(structure, DmarStructure::Drhd { .. }))
            .count()
    }

    /// The DRHD unit that governs `bdf`, and the scope that decided it.
    ///
    /// Scopes resolve through the live topology: `bridge` reports the buses
    /// behind a function, or `None` when the function is absent or is not a
    /// bridge. The most specific scope wins: an endpoint scope naming `bdf`
    /// outranks a bridge scope enclosing it, an inner bridge outranks the
    /// bridges above it, and an `INCLUDE_PCI_ALL` unit governs only what no
    /// scope in its segment names. Equal-specificity claims from distinct
    /// units are ambiguous and fail closed as `None`.
    pub fn unit_for(
        &self,
        bdf: Bdf,
        bridge: impl Fn(Bdf) -> Option<BusRange>,
    ) -> Option<GoverningUnit> {
        let mut endpoint: Option<GoverningUnit> = None;
        let mut endpoint_ambiguous = false;
        let mut enclosing: Option<(Named, GoverningUnit)> = None;
        let mut enclosing_ambiguous = false;
        let mut catch_all: Option<GoverningUnit> = None;
        let mut catch_all_ambiguous = false;
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
            for scope in DeviceScopes::new(scopes) {
                match scope.walk(segment, &bridge) {
                    Some(named @ Named::Function(_)) if named.covers(bdf) => {
                        let candidate = GoverningUnit {
                            register_base,
                            coverage: Coverage::Endpoint,
                        };
                        if endpoint.is_some_and(|unit| unit.register_base != register_base) {
                            endpoint_ambiguous = true;
                        } else {
                            endpoint.get_or_insert(candidate);
                        }
                    }
                    Some(named @ Named::Hierarchy(hierarchy)) if named.covers(bdf) => {
                        let candidate = GoverningUnit {
                            register_base,
                            coverage: Coverage::Bridge(hierarchy.root),
                        };
                        match enclosing {
                            Some((current, unit)) if unit.register_base == register_base => {
                                if current.strictly_contains(named) {
                                    enclosing = Some((named, candidate));
                                }
                            }
                            Some((current, _)) if current.strictly_contains(named) => {
                                enclosing = Some((named, candidate));
                            }
                            Some((current, _)) if named.strictly_contains(current) => {}
                            Some(_) => {
                                enclosing_ambiguous = true;
                            }
                            None => enclosing = Some((named, candidate)),
                        }
                    }
                    _ => {}
                }
            }
            if include_pci_all {
                let candidate = GoverningUnit {
                    register_base,
                    coverage: Coverage::CatchAll,
                };
                if catch_all.is_some_and(|unit| unit.register_base != register_base) {
                    catch_all_ambiguous = true;
                } else {
                    catch_all.get_or_insert(candidate);
                }
            }
        }
        if endpoint.is_some() {
            return (!endpoint_ambiguous).then_some(endpoint).flatten();
        }
        if enclosing.is_some() {
            return (!enclosing_ambiguous)
                .then_some(enclosing.map(|(_, unit)| unit))
                .flatten();
        }
        (!catch_all_ambiguous).then_some(catch_all).flatten()
    }

    /// Inventory every live PCI requester governed by the selected function's
    /// exact DRHD.
    ///
    /// Endpoint paths are resolved exactly. Sub-hierarchy scopes are expanded
    /// across every bus, device and function number in their bounded range;
    /// `present` must report whether that exact requester currently exists.
    /// Each result is checked through [`Self::unit_for`] again so a scope
    /// shadowed by a provably more-specific unit cannot enter the inventory.
    /// Overlapping sibling hierarchies are ambiguous, and a fixed scope budget
    /// bounds hostile duplicate scans. Catch-all coverage remains unsupported
    /// because it has no fixed scope boundary.
    pub fn requester_inventory<const N: usize>(
        &self,
        selected: Bdf,
        bridge: impl Fn(Bdf) -> Option<BusRange>,
        present: impl Fn(Bdf) -> bool,
    ) -> Result<RequesterInventory<N>, RequesterInventoryError> {
        let unit = self
            .unit_for(selected, &bridge)
            .ok_or(RequesterInventoryError::Ungoverned)?;
        if unit.coverage == Coverage::CatchAll {
            return Err(RequesterInventoryError::SelectedUsesCatchAll);
        }

        let mut requesters = [None; N];
        let mut count = 0usize;
        let mut requester_scopes = 0usize;
        let mut found_unit = false;
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
            if register_base != unit.register_base {
                continue;
            }
            found_unit = true;
            if include_pci_all {
                return Err(RequesterInventoryError::IncludesAllPci);
            }
            for scope in DeviceScopes::new(scopes) {
                if matches!(
                    scope.kind(),
                    DeviceScopeKind::PciEndpoint | DeviceScopeKind::PciSubHierarchy
                ) {
                    requester_scopes += 1;
                    if requester_scopes > MAX_REQUESTER_SCOPES {
                        return Err(RequesterInventoryError::TooManyRequesterScopes {
                            maximum: MAX_REQUESTER_SCOPES,
                        });
                    }
                }
                match scope.kind() {
                    DeviceScopeKind::PciEndpoint => {
                        let Some(Named::Function(endpoint)) = scope.walk(segment, &bridge) else {
                            return Err(RequesterInventoryError::UnresolvedScope);
                        };
                        if self
                            .unit_for(endpoint, &bridge)
                            .is_none_or(|governing| governing.register_base != register_base)
                        {
                            return Err(RequesterInventoryError::ForeignEndpoint);
                        }
                        push_requester(&mut requesters, &mut count, endpoint)?;
                    }
                    DeviceScopeKind::PciSubHierarchy => {
                        let Some(Named::Hierarchy(hierarchy)) = scope.walk(segment, &bridge) else {
                            return Err(RequesterInventoryError::UnresolvedScope);
                        };
                        add_live_requester(
                            self,
                            &bridge,
                            register_base,
                            hierarchy.root,
                            &mut requesters,
                            &mut count,
                        )?;
                        for bus in hierarchy.buses.secondary..=hierarchy.buses.subordinate {
                            for device in 0..32 {
                                for function in 0..8 {
                                    let requester = Bdf::new(segment, bus, device, function);
                                    if present(requester) {
                                        add_live_requester(
                                            self,
                                            &bridge,
                                            register_base,
                                            requester,
                                            &mut requesters,
                                            &mut count,
                                        )?;
                                    }
                                }
                            }
                        }
                    }
                    DeviceScopeKind::IoApic | DeviceScopeKind::Hpet => {}
                    DeviceScopeKind::AcpiNamespaceDevice | DeviceScopeKind::Other { .. } => {
                        return Err(RequesterInventoryError::UnsupportedScope);
                    }
                }
            }
        }
        debug_assert!(found_unit, "a governing unit must name one DRHD");
        if !requesters[..count].contains(&Some(selected)) {
            return Err(RequesterInventoryError::MissingSelected);
        }
        Ok(RequesterInventory {
            unit,
            requesters,
            count,
        })
    }

    /// Every reserved memory region a device at `bdf` must have
    /// identity-mapped rather than translated.
    ///
    /// An RMRR's scopes resolve through `bridge` as [`Self::unit_for`]'s do,
    /// and its region applies when any of them names `bdf`, as itself or
    /// behind a named bridge.
    pub fn reserved_regions_for<F: Fn(Bdf) -> Option<BusRange>>(
        &self,
        bdf: Bdf,
        bridge: F,
    ) -> ReservedRegions<'a, F> {
        ReservedRegions {
            entries: self.entries(),
            bdf,
            bridge,
        }
    }
}

/// The `(base, limit)` ranges [`Dmar::reserved_regions_for`] finds for one
/// device, inclusive of both ends.
#[derive(Debug)]
pub struct ReservedRegions<'a, F> {
    entries: DmarEntries<'a>,
    bdf: Bdf,
    bridge: F,
}

impl<F: Fn(Bdf) -> Option<BusRange>> Iterator for ReservedRegions<'_, F> {
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
            let named = DeviceScopes::new(scopes).any(|scope| {
                scope
                    .walk(segment, &self.bridge)
                    .is_some_and(|named| named.covers(self.bdf))
            });
            if named {
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
        assert_eq!(dmar.unit_count(), 2);
        let scoped_bdf = Bdf::new(0, 2, 3, 0);
        let unscoped_bdf = Bdf::new(0, 9, 0, 0);
        assert_eq!(
            dmar.unit_for(scoped_bdf, no_bridges),
            Some(GoverningUnit {
                register_base: 0xfed9_1000,
                coverage: Coverage::Endpoint,
            })
        );
        assert_eq!(
            dmar.unit_for(unscoped_bdf, no_bridges),
            Some(GoverningUnit {
                register_base: 0xfed9_0000,
                coverage: Coverage::CatchAll,
            })
        );
        assert_eq!(
            dmar.reserved_regions_for(scoped_bdf, no_bridges)
                .collect::<Vec<_>>(),
            [(0x1000, 0x1fff)]
        );
        assert_eq!(
            dmar.reserved_regions_for(unscoped_bdf, no_bridges)
                .collect::<Vec<_>>(),
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
    fn a_device_scope_exposes_its_kind_start_bus_and_every_hop() {
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
        assert_eq!(scope.kind(), DeviceScopeKind::PciEndpoint);
        assert_eq!(scope.start_bus(), 2);
        assert_eq!(scope.path().collect::<Vec<_>>(), [(3, 0), (4, 0)]);
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

    // Device-scope type bytes from the VT-d specification's DMAR chapter.
    const ENDPOINT: u8 = 1;
    const SUB_HIERARCHY: u8 = 2;
    const IO_APIC: u8 = 3;
    const HPET: u8 = 4;
    const NAMESPACE_DEVICE: u8 = 5;

    const SCOPED: u64 = 0xfed9_1000;
    const INNER: u64 = 0xfed9_2000;
    const EXACT: u64 = 0xfed9_3000;
    const CATCH_ALL: u64 = 0xfed9_0000;

    fn scope(kind: u8, start_bus: u8, path: &[(u8, u8)]) -> Vec<u8> {
        let mut bytes = vec![kind, 0, 0, 0, 0, start_bus];
        for &(device, function) in path {
            bytes.extend_from_slice(&[device, function]);
        }
        bytes[1] = u8::try_from(bytes.len()).expect("fits");
        bytes
    }

    fn buses(secondary: u8, subordinate: u8) -> BusRange {
        BusRange {
            secondary,
            subordinate,
        }
    }

    /// A topology in which only the listed functions are bridges.
    fn topology(bridges: &[(Bdf, BusRange)]) -> impl Fn(Bdf) -> Option<BusRange> + '_ {
        move |at| {
            bridges
                .iter()
                .find(|(bridge, _)| *bridge == at)
                .map(|(_, behind)| *behind)
        }
    }

    fn no_bridges(_: Bdf) -> Option<BusRange> {
        None
    }

    fn no_requesters(_: Bdf) -> bool {
        false
    }

    fn present(requesters: &[Bdf]) -> impl Fn(Bdf) -> bool + '_ {
        move |at| requesters.contains(&at)
    }

    fn dmar_of(structures: &[Vec<u8>]) -> Vec<u8> {
        table(&Sdt::DMAR, &dmar_body(&structures.concat()))
    }

    const fn catch_all() -> GoverningUnit {
        GoverningUnit {
            register_base: CATCH_ALL,
            coverage: Coverage::CatchAll,
        }
    }

    #[test]
    fn an_endpoint_inventory_owns_each_distinct_endpoint_on_the_unit() {
        let first = Bdf::new(0, 0, 4, 0);
        let second = Bdf::new(0, 0, 5, 0);
        let first_scope = scope(ENDPOINT, 0, &[(4, 0)]);
        let second_scope = scope(ENDPOINT, 0, &[(5, 0)]);
        let scopes = [
            first_scope.as_slice(),
            second_scope.as_slice(),
            first_scope.as_slice(),
            scope(IO_APIC, 0, &[(0x1f, 0)]).as_slice(),
        ]
        .concat();
        let bytes = dmar_of(&[drhd_bytes(false, 0, SCOPED, &scopes)]);
        let dmar = Dmar::new(&bytes).expect("valid dmar");

        let inventory = dmar
            .requester_inventory::<2>(second, no_bridges, no_requesters)
            .expect("bounded requester inventory");
        assert_eq!(inventory.unit().register_base, SCOPED);
        assert_eq!(inventory.len(), 2);
        assert!(!inventory.is_empty());
        assert_eq!(inventory.iter().collect::<Vec<_>>(), [first, second]);
    }

    #[test]
    fn requester_inventory_expands_hierarchies_and_refuses_catch_all_and_overflow() {
        let selected = Bdf::new(0, 0, 4, 0);
        let endpoint = scope(ENDPOINT, 0, &[(4, 0)]);

        let include_all = dmar_of(&[drhd_bytes(true, 0, SCOPED, &endpoint)]);
        assert_eq!(
            Dmar::new(&include_all)
                .expect("valid dmar")
                .requester_inventory::<2>(selected, no_bridges, no_requesters),
            Err(RequesterInventoryError::IncludesAllPci)
        );

        let bridge = Bdf::new(0, 0, 0x1c, 0);
        let bridges = [(bridge, buses(0x20, 0x20))];
        let downstream = Bdf::new(0, 0x20, 3, 1);
        let hierarchy = scope(SUB_HIERARCHY, 0, &[(0x1c, 0)]);
        let shared = dmar_of(&[drhd_bytes(
            false,
            0,
            SCOPED,
            &[endpoint.as_slice(), hierarchy.as_slice()].concat(),
        )]);
        let inventory = Dmar::new(&shared)
            .expect("valid dmar")
            .requester_inventory::<3>(selected, topology(&bridges), present(&[downstream]))
            .expect("bounded hierarchy inventory");
        assert_eq!(
            inventory.iter().collect::<Vec<_>>(),
            [selected, bridge, downstream]
        );

        let crowded = dmar_of(&[drhd_bytes(
            false,
            0,
            SCOPED,
            &[
                endpoint.as_slice(),
                scope(ENDPOINT, 0, &[(5, 0)]).as_slice(),
            ]
            .concat(),
        )]);
        assert_eq!(
            Dmar::new(&crowded)
                .expect("valid dmar")
                .requester_inventory::<1>(selected, no_bridges, no_requesters),
            Err(RequesterInventoryError::TooManyRequesters { maximum: 1 })
        );
    }

    #[test]
    fn requester_inventory_refuses_unresolved_foreign_and_catch_all_selection() {
        let selected = Bdf::new(0, 0, 4, 0);
        let endpoint = scope(ENDPOINT, 0, &[(4, 0)]);
        let unresolved = scope(ENDPOINT, 0, &[(0x1c, 0), (5, 0)]);
        let bytes = dmar_of(&[drhd_bytes(
            false,
            0,
            SCOPED,
            &[endpoint.as_slice(), unresolved.as_slice()].concat(),
        )]);
        assert_eq!(
            Dmar::new(&bytes)
                .expect("valid dmar")
                .requester_inventory::<2>(selected, no_bridges, no_requesters),
            Err(RequesterInventoryError::UnresolvedScope)
        );

        let sibling = scope(ENDPOINT, 0, &[(5, 0)]);
        let ambiguous = dmar_of(&[
            drhd_bytes(
                false,
                0,
                SCOPED,
                &[endpoint.as_slice(), sibling.as_slice()].concat(),
            ),
            drhd_bytes(false, 0, EXACT, &sibling),
        ]);
        assert_eq!(
            Dmar::new(&ambiguous)
                .expect("valid dmar")
                .requester_inventory::<2>(selected, no_bridges, no_requesters),
            Err(RequesterInventoryError::ForeignEndpoint)
        );

        let catch_all = dmar_of(&[drhd_bytes(true, 0, CATCH_ALL, &[])]);
        assert_eq!(
            Dmar::new(&catch_all)
                .expect("valid dmar")
                .requester_inventory::<2>(selected, no_bridges, no_requesters),
            Err(RequesterInventoryError::SelectedUsesCatchAll)
        );
        assert_eq!(
            Dmar::new(&catch_all)
                .expect("valid dmar")
                .requester_inventory::<2>(Bdf::new(1, 0, 4, 0), no_bridges, no_requesters,),
            Err(RequesterInventoryError::Ungoverned)
        );
    }

    #[test]
    fn a_selected_requester_under_a_hierarchy_is_inventoried_with_its_bridge() {
        let bridge = Bdf::new(0, 0, 0x1c, 0);
        let selected = Bdf::new(0, 0x20, 3, 0);
        let bridges = [(bridge, buses(0x20, 0x20))];
        let bytes = dmar_of(&[drhd_bytes(
            false,
            0,
            SCOPED,
            &scope(SUB_HIERARCHY, 0, &[(0x1c, 0)]),
        )]);

        let inventory = Dmar::new(&bytes)
            .expect("valid dmar")
            .requester_inventory::<2>(selected, topology(&bridges), present(&[selected]))
            .expect("complete hierarchy inventory");
        assert_eq!(inventory.unit().coverage, Coverage::Bridge(bridge));
        assert_eq!(inventory.iter().collect::<Vec<_>>(), [bridge, selected]);
    }

    #[test]
    fn overlapping_sibling_hierarchies_are_ambiguous_not_numerically_nested() {
        let first = Bdf::new(0, 0, 0x1c, 0);
        let second = Bdf::new(0, 0, 0x1d, 0);
        let target = Bdf::new(0, 0x30, 3, 0);
        let bridges = [(first, buses(0x20, 0x3f)), (second, buses(0x30, 0x4f))];
        let bytes = dmar_of(&[
            drhd_bytes(false, 0, SCOPED, &scope(SUB_HIERARCHY, 0, &[(0x1c, 0)])),
            drhd_bytes(false, 0, INNER, &scope(SUB_HIERARCHY, 0, &[(0x1d, 0)])),
        ]);
        let dmar = Dmar::new(&bytes).expect("valid dmar");

        assert_eq!(dmar.unit_for(target, topology(&bridges)), None);
        assert_eq!(
            dmar.requester_inventory::<4>(first, topology(&bridges), present(&[target])),
            Err(RequesterInventoryError::AmbiguousRequester)
        );
    }

    #[test]
    fn requester_scope_work_is_bounded_before_duplicate_hierarchy_scans() {
        let selected = Bdf::new(0, 0, 4, 0);
        let bridge = Bdf::new(0, 0, 0x1c, 0);
        let endpoint = scope(ENDPOINT, 0, &[(4, 0)]);
        let hierarchy = scope(SUB_HIERARCHY, 0, &[(0x1c, 0)]);
        let mut scopes = endpoint;
        for _ in 0..MAX_REQUESTER_SCOPES {
            scopes.extend_from_slice(&hierarchy);
        }
        let bytes = dmar_of(&[drhd_bytes(false, 0, SCOPED, &scopes)]);
        let bridges = [(bridge, buses(0x20, 0x20))];

        assert_eq!(
            Dmar::new(&bytes)
                .expect("valid dmar")
                .requester_inventory::<4>(selected, topology(&bridges), no_requesters),
            Err(RequesterInventoryError::TooManyRequesterScopes {
                maximum: MAX_REQUESTER_SCOPES,
            })
        );
    }

    #[test]
    fn requester_inventory_refuses_non_pci_and_unknown_requester_scopes() {
        let selected = Bdf::new(0, 0, 4, 0);
        let endpoint = scope(ENDPOINT, 0, &[(4, 0)]);
        for kind in [NAMESPACE_DEVICE, 0x7f] {
            let scopes = [endpoint.as_slice(), scope(kind, 0, &[(0x1f, 0)]).as_slice()].concat();
            let bytes = dmar_of(&[drhd_bytes(false, 0, SCOPED, &scopes)]);
            assert_eq!(
                Dmar::new(&bytes)
                    .expect("valid dmar")
                    .requester_inventory::<2>(selected, no_bridges, no_requesters),
                Err(RequesterInventoryError::UnsupportedScope)
            );
        }
    }

    #[test]
    fn an_endpoint_scope_resolves_through_each_bridge_on_its_path() {
        // Every bus number differs, so a walk that stops early, descends to a
        // subordinate or asks another segment misses the target.
        let root_port = Bdf::new(3, 0x10, 0x1c, 4);
        let switch_port = Bdf::new(3, 0x20, 0x03, 2);
        let bridges = [
            (root_port, buses(0x20, 0x2f)),
            (switch_port, buses(0x24, 0x26)),
        ];
        let topology = topology(&bridges);
        let endpoint = scope(ENDPOINT, 0x10, &[(0x1c, 4), (0x03, 2), (0x05, 1)]);
        let bytes = dmar_of(&[
            drhd_bytes(false, 3, SCOPED, &endpoint),
            drhd_bytes(true, 3, CATCH_ALL, &[]),
            rmrr_bytes(3, 0x7000, 0x7fff, &endpoint),
        ]);
        let dmar = Dmar::new(&bytes).expect("valid dmar");

        let target = Bdf::new(3, 0x24, 0x05, 1);
        assert_eq!(
            dmar.unit_for(target, &topology),
            Some(GoverningUnit {
                register_base: SCOPED,
                coverage: Coverage::Endpoint,
            })
        );
        assert_eq!(
            dmar.reserved_regions_for(target, &topology)
                .collect::<Vec<_>>(),
            [(0x7000, 0x7fff)]
        );
        for elsewhere in [
            Bdf::new(3, 0x10, 0x05, 1),
            Bdf::new(3, 0x20, 0x05, 1),
            Bdf::new(3, 0x26, 0x05, 1),
            Bdf::new(3, 0x24, 0x05, 0),
        ] {
            assert_eq!(
                dmar.unit_for(elsewhere, &topology),
                Some(catch_all()),
                "{elsewhere:?}"
            );
            assert_eq!(
                dmar.reserved_regions_for(elsewhere, &topology).count(),
                0,
                "{elsewhere:?}"
            );
        }
    }

    #[test]
    fn a_bridge_scope_covers_its_bridge_and_exactly_the_buses_behind_it() {
        let bridge = Bdf::new(0, 0x00, 0x1c, 0);
        let bridges = [(bridge, buses(0x20, 0x2f))];
        let topology = topology(&bridges);
        let hierarchy = scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)]);
        let bytes = dmar_of(&[
            drhd_bytes(false, 0, SCOPED, &hierarchy),
            drhd_bytes(true, 0, CATCH_ALL, &[]),
            rmrr_bytes(0, 0x7000, 0x7fff, &hierarchy),
        ]);
        let dmar = Dmar::new(&bytes).expect("valid dmar");

        let behind_bridge = GoverningUnit {
            register_base: SCOPED,
            coverage: Coverage::Bridge(bridge),
        };
        let functions = [
            (bridge, Some(behind_bridge)),
            (Bdf::new(0, 0x1f, 0x05, 1), Some(catch_all())),
            (Bdf::new(0, 0x20, 0x05, 1), Some(behind_bridge)),
            (Bdf::new(0, 0x2f, 0x05, 1), Some(behind_bridge)),
            (Bdf::new(0, 0x30, 0x05, 1), Some(catch_all())),
        ];
        for (function, governed) in functions {
            assert_eq!(dmar.unit_for(function, &topology), governed, "{function:?}");
            let reserved = dmar.reserved_regions_for(function, &topology).count();
            assert_eq!(reserved, usize::from(governed == Some(behind_bridge)));
        }
    }

    #[test]
    fn a_path_through_a_function_that_is_not_a_bridge_names_nothing() {
        // Nothing here is a bridge, so neither scope may name a function.
        let through = scope(ENDPOINT, 0x00, &[(0x1c, 0), (0x05, 1)]);
        let hierarchy = scope(SUB_HIERARCHY, 0x00, &[(0x1d, 0)]);
        let bytes = dmar_of(&[
            drhd_bytes(false, 0, SCOPED, &[through, hierarchy].concat()),
            drhd_bytes(true, 0, CATCH_ALL, &[]),
        ]);
        let dmar = Dmar::new(&bytes).expect("valid dmar");

        for function in [
            Bdf::new(0, 0x00, 0x05, 1),
            Bdf::new(0, 0x00, 0x1c, 0),
            Bdf::new(0, 0x00, 0x1d, 0),
        ] {
            assert_eq!(
                dmar.unit_for(function, no_bridges),
                Some(catch_all()),
                "{function:?}"
            );
        }
    }

    #[test]
    fn a_bridge_that_does_not_lead_past_its_own_bus_names_nothing() {
        // A secondary bus at or before the bridge's own would let a walk
        // return to a bus it left.
        let bridges = [
            (Bdf::new(0, 0x04, 0x1c, 0), buses(0x04, 0x08)),
            (Bdf::new(0, 0x30, 0x1c, 0), buses(0x20, 0x2f)),
        ];
        let topology = topology(&bridges);
        let looping = scope(ENDPOINT, 0x04, &[(0x1c, 0), (0x05, 1)]);
        let backward = scope(ENDPOINT, 0x30, &[(0x1c, 0), (0x06, 1)]);
        let bytes = dmar_of(&[
            drhd_bytes(false, 0, SCOPED, &[looping, backward].concat()),
            drhd_bytes(true, 0, CATCH_ALL, &[]),
        ]);
        let dmar = Dmar::new(&bytes).expect("valid dmar");

        for function in [Bdf::new(0, 0x04, 0x05, 1), Bdf::new(0, 0x20, 0x06, 1)] {
            assert_eq!(
                dmar.unit_for(function, &topology),
                Some(catch_all()),
                "{function:?}"
            );
        }
    }

    #[test]
    fn a_bridge_whose_secondary_bus_exceeds_its_subordinate_names_nothing() {
        let bridge = Bdf::new(0, 0x00, 0x1c, 0);
        let bridges = [(bridge, buses(0x30, 0x20))];
        let topology = topology(&bridges);
        let hierarchy = scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)]);
        let through = scope(ENDPOINT, 0x00, &[(0x1c, 0), (0x05, 1)]);
        let bytes = dmar_of(&[
            drhd_bytes(false, 0, SCOPED, &[hierarchy, through].concat()),
            drhd_bytes(true, 0, CATCH_ALL, &[]),
        ]);
        let dmar = Dmar::new(&bytes).expect("valid dmar");

        for function in [
            bridge,
            Bdf::new(0, 0x30, 0x05, 1),
            Bdf::new(0, 0x25, 0x05, 1),
        ] {
            assert_eq!(
                dmar.unit_for(function, &topology),
                Some(catch_all()),
                "{function:?}"
            );
        }
    }

    #[test]
    fn a_path_longer_than_the_hop_bound_names_nothing() {
        // A chain of bridges, each leading to the next bus, deep enough for
        // one hop past the bound.
        let bridges = (0..=u8::try_from(DeviceScope::MAX_HOPS).expect("fits"))
            .map(|bus| (Bdf::new(0, bus, 0x01, 0), buses(bus + 1, 0xff)))
            .collect::<Vec<_>>();
        let topology = topology(&bridges);
        let governing = |hops: usize| {
            let mut path = vec![(0x01, 0); hops - 1];
            path.push((0x05, 1));
            let bytes = dmar_of(&[
                drhd_bytes(false, 0, SCOPED, &scope(ENDPOINT, 0x00, &path)),
                drhd_bytes(true, 0, CATCH_ALL, &[]),
            ]);
            let dmar = Dmar::new(&bytes).expect("valid dmar");
            let target = Bdf::new(0, u8::try_from(hops - 1).expect("fits"), 0x05, 1);
            dmar.unit_for(target, &topology)
                .map(|unit| unit.register_base)
        };

        assert_eq!(governing(DeviceScope::MAX_HOPS), Some(SCOPED));
        assert_eq!(governing(DeviceScope::MAX_HOPS + 1), Some(CATCH_ALL));
    }

    #[test]
    fn scopes_that_name_no_pci_function_never_govern_one() {
        // Each carries the path an endpoint scope for the target would.
        let target = Bdf::new(0, 0x00, 0x1f, 0);
        let foreign = [IO_APIC, HPET, NAMESPACE_DEVICE, 0x7f]
            .map(|kind| scope(kind, 0x00, &[(0x1f, 0)]))
            .concat();
        let bytes = dmar_of(&[
            drhd_bytes(false, 0, SCOPED, &foreign),
            drhd_bytes(true, 0, CATCH_ALL, &[]),
            rmrr_bytes(0, 0x7000, 0x7fff, &foreign),
        ]);
        let dmar = Dmar::new(&bytes).expect("valid dmar");

        assert_eq!(dmar.unit_for(target, no_bridges), Some(catch_all()));
        assert_eq!(dmar.reserved_regions_for(target, no_bridges).count(), 0);
    }

    #[test]
    fn the_most_specific_scope_governs_across_units() {
        // An outer bridge, an inner bridge behind it, and an endpoint behind
        // both, each on its own unit and listed outermost first.
        let outer = Bdf::new(0, 0x00, 0x1c, 0);
        let inner = Bdf::new(0, 0x20, 0x00, 0);
        let bridges = [(outer, buses(0x20, 0x2f)), (inner, buses(0x21, 0x22))];
        let topology = topology(&bridges);
        let bytes = dmar_of(&[
            drhd_bytes(false, 0, SCOPED, &scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)])),
            drhd_bytes(
                false,
                0,
                INNER,
                &scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0), (0x00, 0)]),
            ),
            drhd_bytes(
                false,
                0,
                EXACT,
                &scope(ENDPOINT, 0x00, &[(0x1c, 0), (0x00, 0), (0x00, 0)]),
            ),
            drhd_bytes(true, 0, CATCH_ALL, &[]),
        ]);
        let dmar = Dmar::new(&bytes).expect("valid dmar");

        let governed = [
            (Bdf::new(0, 0x21, 0x00, 0), EXACT, Coverage::Endpoint),
            (Bdf::new(0, 0x22, 0x03, 0), INNER, Coverage::Bridge(inner)),
            (Bdf::new(0, 0x25, 0x00, 0), SCOPED, Coverage::Bridge(outer)),
            (Bdf::new(0, 0x30, 0x00, 0), CATCH_ALL, Coverage::CatchAll),
        ];
        for (function, register_base, coverage) in governed {
            assert_eq!(
                dmar.unit_for(function, &topology),
                Some(GoverningUnit {
                    register_base,
                    coverage,
                }),
                "{function:?}"
            );
        }
    }

    #[test]
    fn the_catch_all_unit_governs_only_functions_no_scope_names() {
        // Listed first, against the specification's ordering, so table
        // order cannot be what keeps it from claiming a named function.
        let bridge = Bdf::new(0, 0x00, 0x1c, 0);
        let bridges = [(bridge, buses(0x20, 0x20))];
        let topology = topology(&bridges);
        let bytes = dmar_of(&[
            drhd_bytes(true, 0, CATCH_ALL, &[]),
            drhd_bytes(false, 0, EXACT, &pci_endpoint_scope(0x00, 0x05, 0)),
            drhd_bytes(false, 0, SCOPED, &scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)])),
            drhd_bytes(false, 2, INNER, &pci_endpoint_scope(0x00, 0x07, 0)),
        ]);
        let dmar = Dmar::new(&bytes).expect("valid dmar");

        let governed = [
            (
                Bdf::new(0, 0x00, 0x05, 0),
                Some((EXACT, Coverage::Endpoint)),
            ),
            (
                Bdf::new(0, 0x20, 0x00, 0),
                Some((SCOPED, Coverage::Bridge(bridge))),
            ),
            (
                Bdf::new(0, 0x00, 0x06, 0),
                Some((CATCH_ALL, Coverage::CatchAll)),
            ),
            (Bdf::new(2, 0x00, 0x08, 0), None),
            (Bdf::new(1, 0x00, 0x05, 0), None),
        ];
        for (function, expected) in governed {
            let expected = expected.map(|(register_base, coverage)| GoverningUnit {
                register_base,
                coverage,
            });
            assert_eq!(dmar.unit_for(function, &topology), expected, "{function:?}");
        }
    }

    #[test]
    fn excessive_and_ambiguous_remapping_authority_fails_closed() {
        let units = (0..=MAX_DRHD_UNITS)
            .map(|index| drhd_bytes(false, 0, 0x1000 + index as u64 * 0x1000, &[]))
            .collect::<Vec<_>>();
        assert_eq!(
            Dmar::new(&dmar_of(&units)),
            Err(DmarError::TooManyUnits {
                maximum: MAX_DRHD_UNITS
            })
        );

        let target = Bdf::new(0, 0x00, 0x05, 0);
        let endpoint = pci_endpoint_scope(0x00, 0x05, 0);
        let bytes = dmar_of(&[
            drhd_bytes(false, 0, EXACT, &endpoint),
            drhd_bytes(false, 0, SCOPED, &endpoint),
            drhd_bytes(true, 0, CATCH_ALL, &[]),
        ]);
        let dmar = Dmar::new(&bytes).expect("structurally valid ambiguous endpoints");
        assert_eq!(dmar.unit_for(target, no_bridges), None);

        let elsewhere = Bdf::new(0, 0x00, 0x06, 0);
        let bytes = dmar_of(&[
            drhd_bytes(true, 0, CATCH_ALL, &[]),
            drhd_bytes(true, 0, INNER, &[]),
        ]);
        let dmar = Dmar::new(&bytes).expect("structurally valid ambiguous catch-all units");
        assert_eq!(dmar.unit_for(elsewhere, no_bridges), None);

        let bridge = Bdf::new(0, 0x00, 0x1c, 0);
        let bridges = [(bridge, buses(0x20, 0x2f))];
        let topology = topology(&bridges);
        let hierarchy = scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)]);
        let bytes = dmar_of(&[
            drhd_bytes(false, 0, SCOPED, &hierarchy),
            drhd_bytes(false, 0, INNER, &hierarchy),
        ]);
        let dmar = Dmar::new(&bytes).expect("structurally valid ambiguous bridges");
        assert_eq!(dmar.unit_for(Bdf::new(0, 0x20, 0x01, 0), &topology), None);
    }

    #[test]
    fn a_reserved_region_follows_scopes_resolved_through_the_topology() {
        let bridge = Bdf::new(0, 0x00, 0x1c, 0);
        let bridges = [(bridge, buses(0x20, 0x22))];
        let topology = topology(&bridges);
        let bytes = dmar_of(&[
            drhd_bytes(true, 0, CATCH_ALL, &[]),
            rmrr_bytes(
                0,
                0x7000,
                0x7fff,
                &scope(ENDPOINT, 0x00, &[(0x1c, 0), (0x00, 0)]),
            ),
            rmrr_bytes(0, 0x9000, 0x9fff, &scope(SUB_HIERARCHY, 0x00, &[(0x1c, 0)])),
        ]);
        let dmar = Dmar::new(&bytes).expect("valid dmar");

        let regions = |function| {
            dmar.reserved_regions_for(function, &topology)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            regions(Bdf::new(0, 0x20, 0x00, 0)),
            [(0x7000, 0x7fff), (0x9000, 0x9fff)]
        );
        assert_eq!(regions(Bdf::new(0, 0x22, 0x01, 0)), [(0x9000, 0x9fff)]);
        assert_eq!(regions(Bdf::new(0, 0x23, 0x00, 0)), []);
    }
}
