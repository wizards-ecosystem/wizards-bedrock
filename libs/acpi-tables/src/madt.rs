use core::fmt;

use crate::{AcpiError, Sdt, u16_at, u32_at};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MadtEntry {
    LocalApic {
        uid: u8,
        apic_id: u8,
        flags: u32,
    },
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
    LocalApicNmi {
        uid: u8,
        flags: u16,
        lint: u8,
    },
    LocalX2Apic {
        apic_id: u32,
        flags: u32,
        uid: u32,
    },
    LocalX2ApicNmi {
        flags: u16,
        uid: u32,
        lint: u8,
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
                1 | 0xa => 12,
                2 => 10,
                4 => 6,
                9 => 16,
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
            0 => MadtEntry::LocalApic {
                uid: entry[2],
                apic_id: entry[3],
                flags: u32_at(entry, 4).unwrap_or(0),
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
            4 => MadtEntry::LocalApicNmi {
                uid: entry[2],
                flags: u16_at(entry, 3).unwrap_or(0),
                lint: entry[5],
            },
            9 => MadtEntry::LocalX2Apic {
                apic_id: u32_at(entry, 4).unwrap_or(0),
                flags: u32_at(entry, 8).unwrap_or(0),
                uid: u32_at(entry, 12).unwrap_or(0),
            },
            0xa => MadtEntry::LocalX2ApicNmi {
                flags: u16_at(entry, 2).unwrap_or(0),
                uid: u32_at(entry, 4).unwrap_or(0),
                lint: entry[8],
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
                    if !inti_flags_are_valid(flags) {
                        return Err(InterruptError::ReservedOverrideFlags { source, flags });
                    }
                    if slot.is_some() {
                        return Err(InterruptError::DuplicateOverride(source));
                    }
                    *slot = Some((gsi, flags));
                }
                _ => {}
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

    pub fn override_for(&self, irq: u8) -> Option<(u32, u16)> {
        self.overrides.get(usize::from(irq)).copied().flatten()
    }
}

const fn inti_flags_are_valid(flags: u16) -> bool {
    let polarity = flags & 0b11;
    let trigger = (flags >> 2) & 0b11;
    flags & !0xf == 0 && polarity != 0b10 && trigger != 0b10
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessorState {
    Enabled,
    OnlineCapable,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LocalNmi {
    lint: u8,
    flags: u16,
}

impl LocalNmi {
    pub const fn lint(self) -> u8 {
        self.lint
    }

    pub const fn flags(self) -> u16 {
        self.flags
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Processor {
    apic_id: u32,
    uid: u32,
    state: ProcessorState,
    nmi: Option<LocalNmi>,
}

impl Processor {
    pub const fn apic_id(self) -> u32 {
        self.apic_id
    }

    pub const fn uid(self) -> u32 {
        self.uid
    }

    pub const fn state(self) -> ProcessorState {
        self.state
    }

    pub const fn nmi(self) -> Option<LocalNmi> {
        self.nmi
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessorError {
    Table(AcpiError),
    TooManyProcessors { limit: usize },
    ReservedProcessorFlags { apic_id: u32, flags: u32 },
    BroadcastApicId(u32),
    DuplicateApicId(u32),
    DuplicateUid(u32),
    InvalidNmiLine { lint: u8 },
    ReservedNmiFlags(u16),
    DuplicateNmi { apic_id: u32 },
}

impl fmt::Display for ProcessorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Table(reason) => write!(formatter, "madt is unusable: {reason}"),
            Self::TooManyProcessors { limit } => {
                write!(
                    formatter,
                    "madt describes more than {limit} usable processors"
                )
            }
            Self::ReservedProcessorFlags { apic_id, flags } => write!(
                formatter,
                "madt processor {apic_id} has reserved flags {flags:#x}"
            ),
            Self::BroadcastApicId(apic_id) => {
                write!(
                    formatter,
                    "madt names the broadcast id {apic_id:#x} as a processor"
                )
            }
            Self::DuplicateApicId(apic_id) => {
                write!(formatter, "madt repeats processor apic id {apic_id}")
            }
            Self::DuplicateUid(uid) => write!(formatter, "madt repeats processor uid {uid}"),
            Self::InvalidNmiLine { lint } => {
                write!(formatter, "madt nmi names local interrupt line {lint}")
            }
            Self::ReservedNmiFlags(flags) => {
                write!(formatter, "madt nmi has reserved flags {flags:#x}")
            }
            Self::DuplicateNmi { apic_id } => {
                write!(formatter, "madt gives processor {apic_id} a second nmi")
            }
        }
    }
}

impl core::error::Error for ProcessorError {}

const ENABLED: u32 = 1;
const ONLINE_CAPABLE: u32 = 2;
const XAPIC_BROADCAST: u32 = 0xff;
const X2APIC_BROADCAST: u32 = u32::MAX;
const NMI_ALL_XAPIC: u32 = 0xff;
const NMI_ALL_X2APIC: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MadtProcessors<const PROCESSORS: usize> {
    processors: [Option<Processor>; PROCESSORS],
    count: usize,
}

impl<const PROCESSORS: usize> MadtProcessors<PROCESSORS> {
    pub fn parse(table: &[u8]) -> Result<Self, ProcessorError> {
        let mut model = Self {
            processors: [None; PROCESSORS],
            count: 0,
        };
        for entry in MadtEntries::new(table).map_err(ProcessorError::Table)? {
            match entry {
                MadtEntry::LocalApic {
                    uid,
                    apic_id,
                    flags,
                } => model.admit(u32::from(apic_id), u32::from(uid), flags, XAPIC_BROADCAST)?,
                MadtEntry::LocalX2Apic {
                    apic_id,
                    flags,
                    uid,
                } => model.admit(apic_id, uid, flags, X2APIC_BROADCAST)?,
                _ => {}
            }
        }
        for entry in MadtEntries::new(table).map_err(ProcessorError::Table)? {
            match entry {
                MadtEntry::LocalApicNmi { uid, flags, lint } => {
                    let uid = u32::from(uid);
                    let target = if uid == NMI_ALL_XAPIC {
                        None
                    } else {
                        Some(uid)
                    };
                    model.route_nmi(target, flags, lint)?;
                }
                MadtEntry::LocalX2ApicNmi { flags, uid, lint } => {
                    let target = if uid == NMI_ALL_X2APIC {
                        None
                    } else {
                        Some(uid)
                    };
                    model.route_nmi(target, flags, lint)?;
                }
                _ => {}
            }
        }
        Ok(model)
    }

    fn admit(
        &mut self,
        apic_id: u32,
        uid: u32,
        flags: u32,
        broadcast: u32,
    ) -> Result<(), ProcessorError> {
        let state = match flags {
            0 => return Ok(()),
            ENABLED => ProcessorState::Enabled,
            ONLINE_CAPABLE => ProcessorState::OnlineCapable,
            _ => return Err(ProcessorError::ReservedProcessorFlags { apic_id, flags }),
        };
        if apic_id == broadcast {
            return Err(ProcessorError::BroadcastApicId(apic_id));
        }
        let mut index = 0;
        while index < self.count {
            if let Some(kept) = self.processors[index] {
                if kept.apic_id == apic_id {
                    return Err(ProcessorError::DuplicateApicId(apic_id));
                }
                if kept.uid == uid {
                    return Err(ProcessorError::DuplicateUid(uid));
                }
            }
            index += 1;
        }
        let Some(slot) = self.processors.get_mut(self.count) else {
            return Err(ProcessorError::TooManyProcessors { limit: PROCESSORS });
        };
        *slot = Some(Processor {
            apic_id,
            uid,
            state,
            nmi: None,
        });
        self.count += 1;
        Ok(())
    }

    // A `None` target is every processor.
    fn route_nmi(
        &mut self,
        target: Option<u32>,
        flags: u16,
        lint: u8,
    ) -> Result<(), ProcessorError> {
        if lint > 1 {
            return Err(ProcessorError::InvalidNmiLine { lint });
        }
        if !inti_flags_are_valid(flags) {
            return Err(ProcessorError::ReservedNmiFlags(flags));
        }
        let mut index = 0;
        while index < self.count {
            if let Some(processor) = &mut self.processors[index]
                && (target.is_none() || target == Some(processor.uid))
            {
                if processor.nmi.is_some() {
                    return Err(ProcessorError::DuplicateNmi {
                        apic_id: processor.apic_id,
                    });
                }
                processor.nmi = Some(LocalNmi { lint, flags });
            }
            index += 1;
        }
        Ok(())
    }

    pub const fn count(&self) -> usize {
        self.count
    }

    pub const fn processor(&self, index: usize) -> Option<Processor> {
        if index < self.count {
            self.processors[index]
        } else {
            None
        }
    }
}
