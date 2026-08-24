//! Parsing for the ACPI tables this kernel reads.
//!
//! The RSDP points at the XSDT, the XSDT lists tables by signature, and the
//! MADT describes the interrupt controllers. Nothing more, until something
//! reads it.
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
//! acpi_tables::fix_checksum(&mut bytes[..20], 9);
//! acpi_tables::fix_checksum(&mut bytes, 32);
//!
//! let rsdp = Rsdp::parse(&bytes).expect("well formed");
//! assert_eq!(rsdp.xsdt(), Some(0xdead_0000));
//! assert_eq!(Sdt::MADT, *b"APIC");
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

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
            Self::BadSignature(_) => f.write_str("table signature is not the expected one"),
            Self::BadChecksum => f.write_str("table checksum does not sum to zero"),
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
pub fn checksum_is_valid(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

/// Set `bytes[index]` so the whole slice sums to zero.
///
/// For building tables in tests and doctests; firmware does this for us.
///
/// # Panics
///
/// Panics if `index` is outside `bytes`.
pub fn fix_checksum(bytes: &mut [u8], index: usize) {
    bytes[index] = 0;
    let sum = bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    bytes[index] = sum.wrapping_neg();
}

/// Read a little-endian `u32` at `offset`, or `None` past the end.
fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let field = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes(field.try_into().ok()?))
}

/// Read a little-endian `u64` at `offset`, or `None` past the end.
fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    let field = bytes.get(offset..offset + 8)?;
    Some(u64::from_le_bytes(field.try_into().ok()?))
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
    /// [`AcpiError::TooShort`], [`AcpiError::BadSignature`] or
    /// [`AcpiError::BadChecksum`].
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

    /// The ACPI revision this pointer declares.
    pub const fn revision(self) -> u8 {
        self.revision
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
    revision: u8,
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
            revision: head[8],
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

    /// The table's revision.
    pub const fn revision(self) -> u8 {
        self.revision
    }
}

/// The table addresses an XSDT or RSDT lists.
///
/// Yielded rather than collected so no allocation is needed, and bounded by
/// the header's own length so a corrupt table cannot produce an endless
/// sequence.
#[derive(Debug)]
pub struct SdtPointers<'a> {
    body: &'a [u8],
    width: usize,
    offset: usize,
}

impl<'a> SdtPointers<'a> {
    /// Validate the table and bind its entry width to its signature.
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
    /// If the table fails validation, is not an XSDT, or ends in a partial
    /// eight-byte pointer.
    pub fn xsdt(table: &'a [u8]) -> Result<Self, AcpiError> {
        Self::new(table, Sdt::XSDT, 8)
    }

    /// The entries of a validated RSDT.
    ///
    /// # Errors
    ///
    /// If the table fails validation, is not an RSDT, or ends in a partial
    /// four-byte pointer.
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

    /// The physical address of the local APIC, as the MADT reports it.
    pub fn local_apic_address(&self) -> u32 {
        u32_at(self.body, Sdt::LEN).unwrap_or(0)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-formed RSDP at the given revision.
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

    /// A table with `signature`, `body` after the header, and a valid checksum.
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
    fn a_well_formed_rsdp_yields_its_pointers() {
        let bytes = rsdp(2);
        let parsed = Rsdp::parse(&bytes).expect("well formed");

        assert_eq!(parsed.revision(), 2);
        assert_eq!(parsed.xsdt(), Some(0xdead_beef));
        assert_eq!(parsed.rsdt(), Some(0x1234));
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
        // Long enough for revision 0, too short for the extended structure.
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
        // Only the header is needed, which is the whole point.
        assert_eq!(
            Sdt::parse_length(&bytes[..Sdt::LEN], usize::MAX),
            Ok(Sdt::LEN + 4)
        );

        assert!(matches!(
            Sdt::parse_length(&bytes[..Sdt::LEN - 1], usize::MAX),
            Err(AcpiError::TooShort { .. })
        ));
        let mut short = bytes.clone();
        short[4..8].copy_from_slice(&8u32.to_le_bytes());
        assert_eq!(
            Sdt::parse_length(&short, usize::MAX),
            Err(AcpiError::BadLength(8))
        );
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
        // The header claims more bytes than the buffer holds.
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
        // Trailing bytes beyond the header's length must not be read as an
        // entry, even though the buffer holds them.
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
        let entries = MadtEntries::new(&bytes).expect("validated madt");

        assert_eq!(
            entries.local_apic_address(),
            0xfee0_0000,
            "the MADT's own header carries the local APIC address"
        );
        let entries: Vec<_> = entries.collect();
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
        // Claims 40 bytes, of which only four are present.
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
        // An I/O APIC entry two bytes short: its fields must not be read.
        let mut body = vec![0u8; 8];
        body.extend_from_slice(&[1, 10, 2, 0, 0, 0, 0, 0, 0, 0]);
        let bytes = table(&Sdt::MADT, &body);
        assert!(matches!(
            MadtEntries::new(&bytes),
            Err(AcpiError::MalformedEntry { length: 10, .. })
        ));
    }
}
