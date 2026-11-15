use core::fmt;

use crate::{AcpiError, Sdt, u32_at};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MadtEntry {
    IoApic {
        id: u8,
        address: u32,
        gsi_base: u32,
    },
    InterruptSourceOverride {
        bus: u8,
        source: u8,
        gsi: u32,
        flags: u16,
    },
    Other,
}

#[derive(Debug)]
pub(crate) struct MadtEntries<'table> {
    body: &'table [u8],
    offset: usize,
}

impl<'table> MadtEntries<'table> {
    const HEADER: usize = Sdt::LEN + 8;

    pub(crate) fn new(table: &'table [u8]) -> Result<Self, AcpiError> {
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
            let entry_length = usize::from(prefix[1]);
            let minimum = match kind {
                0 => 8,
                1 => 12,
                2 => 10,
                _ => 2,
            };
            if entry_length < minimum || entry_length > available {
                return Err(AcpiError::MalformedEntry {
                    offset,
                    length: entry_length,
                    available,
                });
            }
            offset += entry_length;
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
            _ => MadtEntry::Other,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IoApic {
    pub(crate) id: u8,
    pub(crate) address: u32,
    pub(crate) gsi_base: u32,
}

impl IoApic {
    pub const fn id(self) -> u8 {
        self.id
    }

    pub const fn address(self) -> u32 {
        self.address
    }

    pub const fn gsi_base(self) -> u32 {
        self.gsi_base
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InterruptError {
    Table(AcpiError),
    TooManyIoApics { limit: usize },
    NonIsaOverride { bus: u8, source: u8 },
    InvalidOverrideSource(u8),
    DuplicateOverride(u8),
    ReservedOverrideFlags { source: u8, flags: u16 },
}

impl fmt::Display for InterruptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Table(reason) => write!(formatter, "madt is unusable: {reason}"),
            Self::TooManyIoApics { limit } => {
                write!(formatter, "madt describes more than {limit} i/o apics")
            }
            Self::NonIsaOverride { bus, source } => {
                write!(
                    formatter,
                    "madt override for irq {source} names non-ISA bus {bus}"
                )
            }
            Self::InvalidOverrideSource(source) => {
                write!(formatter, "madt override names non-ISA irq {source}")
            }
            Self::DuplicateOverride(source) => {
                write!(formatter, "madt repeats the override for irq {source}")
            }
            Self::ReservedOverrideFlags { source, flags } => write!(
                formatter,
                "madt override for irq {source} has reserved flags {flags:#x}"
            ),
        }
    }
}

impl core::error::Error for InterruptError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MadtInterrupts<const IO_APICS: usize> {
    io_apics: [Option<IoApic>; IO_APICS],
    io_apic_count: usize,
    overrides: [Option<(u32, u16)>; 16],
}

impl<const IO_APICS: usize> MadtInterrupts<IO_APICS> {
    pub fn parse(table: &[u8]) -> Result<Self, InterruptError> {
        let entries = MadtEntries::new(table).map_err(InterruptError::Table)?;
        let mut model = Self {
            io_apics: [None; IO_APICS],
            io_apic_count: 0,
            overrides: [None; 16],
        };

        for entry in entries {
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
                    let Some(slot) = model.overrides.get_mut(usize::from(source)) else {
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
                MadtEntry::Other => {}
            }
        }
        Ok(model)
    }

    pub fn io_apics(&self) -> impl Iterator<Item = IoApic> + '_ {
        self.io_apics[..self.io_apic_count]
            .iter()
            .copied()
            .flatten()
    }

    pub const fn io_apic_count(&self) -> usize {
        self.io_apic_count
    }

    pub fn gsi_for(&self, irq: u8) -> u32 {
        self.override_for(irq)
            .map_or(u32::from(irq), |(gsi, _)| gsi)
    }

    pub fn flags_for(&self, irq: u8) -> u16 {
        self.override_for(irq).map_or(0, |(_, flags)| flags)
    }

    pub fn override_for(&self, irq: u8) -> Option<(u32, u16)> {
        self.overrides.get(usize::from(irq)).copied().flatten()
    }
}
