use core::fmt;

use crate::{AcpiError, Sdt, u32_at};

/// An entry of the Multiple APIC Description Table.
///
/// Only the kinds this kernel acts on are named. Everything else is
/// [`Other`](Self::Other) with its type byte, so an unrecognised controller is
/// reportable rather than invisible.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IoApic {
    pub(crate) id: u8,
    pub(crate) address: u32,
    pub(crate) gsi_base: u32,
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
