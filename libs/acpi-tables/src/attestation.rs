use core::fmt;

use crate::{AcpiError, Sdt, u16_at, u64_at};

/// A platform attestation that one exact VT-d unit covers a complete segment.
///
/// This vendor table is useful when a hypervisor's standards-defined DMAR
/// describes only its boot-time device scopes even though its one emulated
/// unit intercepts the whole segment. It does not replace the DMAR: consumers
/// must corroborate this segment and register base against one RMRR-free DRHD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaCoverage {
    segment: u16,
    register_base: u64,
}

/// Why a Wizard DMA coverage attestation is not authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DmaCoverageError {
    /// The enclosing ACPI table is malformed.
    Table(AcpiError),
    /// The table is valid ACPI but has another signature.
    WrongTable {
        /// Signature this view accepts.
        expected: [u8; 4],
        /// Signature the table carries.
        found: [u8; 4],
    },
    /// The fixed-format table has another length.
    WrongLength(u32),
    /// The table revision has no defined coverage semantics.
    UnsupportedRevision(u8),
    /// Reserved format bits are not zero.
    NonzeroReserved(u16),
}

impl fmt::Display for DmaCoverageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Table(reason) => write!(f, "DMA coverage table is invalid: {reason}"),
            Self::WrongTable { expected, found } => {
                write!(f, "table signature {found:?} does not match {expected:?}")
            }
            Self::WrongLength(length) => {
                write!(f, "DMA coverage table length {length} is not 48 bytes")
            }
            Self::UnsupportedRevision(revision) => {
                write!(f, "DMA coverage revision {revision} is unsupported")
            }
            Self::NonzeroReserved(bits) => {
                write!(f, "DMA coverage reserved bits are {bits:#x}")
            }
        }
    }
}

impl core::error::Error for DmaCoverageError {}

impl DmaCoverage {
    /// Bytes in the complete fixed-format table.
    pub const LEN: usize = Sdt::LEN + 12;

    /// Validate a whole-segment coverage attestation.
    ///
    /// # Errors
    ///
    /// If the ACPI envelope, signature, fixed length, revision, or reserved
    /// field is invalid.
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

    /// PCI segment whose complete requester-ID space the unit intercepts.
    pub const fn segment(self) -> u16 {
        self.segment
    }

    /// Physical register base of the corroborating sole DMAR unit.
    pub const fn register_base(self) -> u64 {
        self.register_base
    }
}

/// A platform attestation that PCI DMA is safe at kernel handoff.
///
/// The table is deliberately separate from [`DmaCoverage`]. Coverage names a
/// remapping unit; this contract says the platform kept one PCI segment
/// quiescent before the kernel can establish any unit of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaHandoff {
    segment: u16,
}

/// Why a Wizard DMA handoff attestation is not authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DmaHandoffError {
    /// The enclosing ACPI table is malformed.
    Table(AcpiError),
    /// The table is valid ACPI but has another signature.
    WrongTable {
        /// Signature this view accepts.
        expected: [u8; 4],
        /// Signature the table carries.
        found: [u8; 4],
    },
    /// The fixed-format table has another length.
    WrongLength(u32),
    /// The table revision has no defined handoff semantics.
    UnsupportedRevision(u8),
    /// Required handoff properties are missing or unknown ones are asserted.
    UnsafeFlags(u16),
    /// Reserved format bytes are not zero.
    NonzeroReserved(u64),
}

impl fmt::Display for DmaHandoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Table(reason) => write!(f, "DMA handoff table is invalid: {reason}"),
            Self::WrongTable { expected, found } => {
                write!(f, "table signature {found:?} does not match {expected:?}")
            }
            Self::WrongLength(length) => {
                write!(f, "DMA handoff table length {length} is not 48 bytes")
            }
            Self::UnsupportedRevision(revision) => {
                write!(f, "DMA handoff revision {revision} is unsupported")
            }
            Self::UnsafeFlags(flags) => {
                write!(
                    f,
                    "DMA handoff flags {flags:#x} are not the complete contract"
                )
            }
            Self::NonzeroReserved(bits) => {
                write!(f, "DMA handoff reserved bits are {bits:#x}")
            }
        }
    }
}

impl core::error::Error for DmaHandoffError {}

impl DmaHandoff {
    /// Bytes in the complete fixed-format table.
    pub const LEN: usize = Sdt::LEN + 12;

    /// Every platform property required for a safe early requester fence.
    pub const REQUIRED_FLAGS: u16 = handoff::COMPLETE_LEGACY_CONFIGURATION
        | handoff::QUIESCENT_THROUGH_FENCE
        | handoff::STATIC_REQUESTERS;

    /// Validate a complete PCI DMA handoff attestation.
    ///
    /// # Errors
    ///
    /// If the ACPI envelope, signature, fixed length, revision, flags or
    /// reserved field is invalid.
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

    /// PCI segment protected by the complete handoff contract.
    pub const fn segment(self) -> u16 {
        self.segment
    }
}

/// DMA-safe handoff properties encoded in [`DmaHandoff`].
mod handoff {
    /// CF8/CFC reaches every DMA-capable requester in the named segment.
    pub(super) const COMPLETE_LEGACY_CONFIGURATION: u16 = 1 << 0;
    /// No requester can issue DMA and no older request is in flight through the fence.
    pub(super) const QUIESCENT_THROUGH_FENCE: u16 = 1 << 1;
    /// Requester presence, identity and command state cannot change externally.
    pub(super) const STATIC_REQUESTERS: u16 = 1 << 2;
}
