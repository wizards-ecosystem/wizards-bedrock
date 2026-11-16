#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::fmt;

const MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

const CLASS_64: u8 = 2;

const LITTLE_ENDIAN: u8 = 1;

pub const HEADER_BYTES: usize = 64;

pub const PROGRAM_HEADER_BYTES: usize = 56;

const MACHINE_X86_64: u16 = 62;

const CURRENT_VERSION: u32 = 1;

const EXECUTABLE: u16 = 2;

const SHARED: u16 = 3;

const PAGE: u64 = 4096;

const MAX_LOADS: usize = 8;

const DYNAMIC_ENTRY_BYTES: usize = 16;

const RELA_ENTRY_BYTES: usize = 24;

const R_X86_64_RELATIVE: u64 = 8;

const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_PLTRELSZ: u64 = 2;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const DT_REL: u64 = 17;
const DT_RELR: u64 = 36;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ElfError {
    Truncated { wanted: usize, available: usize },
    NotAnElf,
    NotSixtyFourBit(u8),
    NotLittleEndian(u8),
    WrongIdentificationVersion(u8),
    WrongType(u16),
    WrongVersion(u32),
    WrongMachine(u16),
    WrongHeaderSize(u16),
    WrongProgramHeaderSize(u16),
    ProgramHeadersOutOfRange { offset: u64, count: u16 },
    HasInterpreter,
    TooManyLoads,
    NoLoadSegments,
    SegmentMalformed { index: u16 },
    WritableAndExecutable { index: u16 },
    MissingDynamic,
    DynamicMalformed,
    UnsupportedDynamicTag(u64),
    WrongRelocationEntrySize(u64),
    RelocationTableOutOfRange,
    UnsupportedRelocation(u64),
    RelocationOutsideWritable(u64),
    EntryOutsideCode(u64),
}

impl fmt::Display for ElfError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Truncated { wanted, available } => {
                write!(formatter, "wanted {wanted} bytes and there are {available}")
            }
            Self::NotAnElf => formatter.write_str("does not begin with the elf magic"),
            Self::NotSixtyFourBit(class) => write!(formatter, "elf class {class} is not 64-bit"),
            Self::NotLittleEndian(data) => {
                write!(formatter, "elf data encoding {data} is not little-endian")
            }
            Self::WrongIdentificationVersion(version) => {
                write!(
                    formatter,
                    "elf identification version {version} is not current"
                )
            }
            Self::WrongType(kind) => {
                write!(formatter, "elf object type {kind} is not accepted here")
            }
            Self::WrongVersion(version) => {
                write!(formatter, "elf file version {version} is not current")
            }
            Self::WrongMachine(machine) => {
                write!(formatter, "elf machine {machine} is not x86-64")
            }
            Self::WrongHeaderSize(size) => {
                write!(
                    formatter,
                    "elf header is {size} bytes, expected {HEADER_BYTES}"
                )
            }
            Self::WrongProgramHeaderSize(size) => {
                write!(
                    formatter,
                    "program headers of {size} bytes are not this format's"
                )
            }
            Self::ProgramHeadersOutOfRange { offset, count } => {
                write!(
                    formatter,
                    "{count} program headers at {offset:#x} run past the image"
                )
            }
            Self::HasInterpreter => formatter.write_str("the image asks for an interpreter"),
            Self::TooManyLoads => write!(formatter, "more than {MAX_LOADS} loadable segments"),
            Self::NoLoadSegments => formatter.write_str("the image has no loadable segment"),
            Self::SegmentMalformed { index } => {
                write!(formatter, "loadable segment {index} is not well formed")
            }
            Self::WritableAndExecutable { index } => {
                write!(
                    formatter,
                    "loadable segment {index} is writable and executable"
                )
            }
            Self::MissingDynamic => formatter.write_str("the image has no dynamic segment"),
            Self::DynamicMalformed => formatter.write_str("the dynamic segment is not well formed"),
            Self::UnsupportedDynamicTag(tag) => {
                write!(formatter, "dynamic tag {tag} is not supported")
            }
            Self::WrongRelocationEntrySize(size) => {
                write!(
                    formatter,
                    "relocation entries of {size} bytes are not this format's"
                )
            }
            Self::RelocationTableOutOfRange => {
                formatter.write_str("the relocation table is not inside a loadable segment")
            }
            Self::UnsupportedRelocation(info) => {
                write!(
                    formatter,
                    "relocation {info:#x} is not a relative relocation"
                )
            }
            Self::RelocationOutsideWritable(offset) => {
                write!(
                    formatter,
                    "relocation at {offset:#x} is not inside a writable segment"
                )
            }
            Self::EntryOutsideCode(entry) => {
                write!(
                    formatter,
                    "entry {entry:#x} is not inside an executable segment"
                )
            }
        }
    }
}

impl core::error::Error for ElfError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SegmentKind {
    Null,
    Load,
    Dynamic,
    Interpreter,
    Note,
    ProgramHeaders,
    ThreadLocal,
    Other(u32),
}

impl SegmentKind {
    pub const fn from_bits(bits: u32) -> Self {
        match bits {
            0 => Self::Null,
            1 => Self::Load,
            2 => Self::Dynamic,
            3 => Self::Interpreter,
            4 => Self::Note,
            6 => Self::ProgramHeaders,
            7 => Self::ThreadLocal,
            other => Self::Other(other),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Segment {
    kind: SegmentKind,
    flags: u32,
    offset: u64,
    address: u64,
    file_size: u64,
    memory_size: u64,
    alignment: u64,
}

impl Segment {
    pub const EXECUTE: u32 = 1;
    pub const WRITE: u32 = 1 << 1;
    pub const READ: u32 = 1 << 2;

    pub const fn kind(self) -> SegmentKind {
        self.kind
    }

    pub const fn flags(self) -> u32 {
        self.flags
    }

    pub const fn offset(self) -> u64 {
        self.offset
    }

    pub const fn address(self) -> u64 {
        self.address
    }

    pub const fn file_size(self) -> u64 {
        self.file_size
    }

    pub const fn memory_size(self) -> u64 {
        self.memory_size
    }

    pub const fn alignment(self) -> u64 {
        self.alignment
    }

    pub const fn has_valid_alignment(self) -> bool {
        self.alignment <= 1
            || self.alignment.is_power_of_two()
                && self.address % self.alignment == self.offset % self.alignment
    }

    pub const fn is_writable(self) -> bool {
        self.flags & Self::WRITE != 0
    }

    pub const fn is_executable(self) -> bool {
        self.flags & Self::EXECUTE != 0
    }

    pub const fn is_write_execute(self) -> bool {
        self.is_writable() && self.is_executable()
    }

    // A loader must ask this before copying: parsing never checks where a header points.
    pub const fn contents_fit(self, bytes: usize) -> bool {
        match self.offset.checked_add(self.file_size) {
            Some(end) => end <= bytes as u64,
            None => false,
        }
    }

    // A loader must refuse it: copying `file_size` bytes would write past the mapping.
    pub const fn is_oversized(self) -> bool {
        self.file_size > self.memory_size
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SectionTable {
    offset: u64,
    entry_size: u16,
    count: u16,
    names_index: u16,
}

impl SectionTable {
    pub const fn offset(self) -> u64 {
        self.offset
    }

    pub const fn entry_size(self) -> u16 {
        self.entry_size
    }

    pub const fn count(self) -> u16 {
        self.count
    }

    pub const fn names_index(self) -> u16 {
        self.names_index
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Elf<'bytes> {
    bytes: &'bytes [u8],
    entry: u64,
    program_offset: u64,
    program_count: u16,
    sections: SectionTable,
}

impl<'bytes> Elf<'bytes> {
    pub fn parse(bytes: &'bytes [u8]) -> Result<Self, ElfError> {
        Self::parse_kind(bytes, EXECUTABLE)
    }

    fn parse_kind(bytes: &'bytes [u8], wanted: u16) -> Result<Self, ElfError> {
        let header = take(bytes, 0, HEADER_BYTES)?;
        if header.get(..4) != Some(&MAGIC) {
            return Err(ElfError::NotAnElf);
        }
        let class = header[4];
        if class != CLASS_64 {
            return Err(ElfError::NotSixtyFourBit(class));
        }
        let data = header[5];
        if data != LITTLE_ENDIAN {
            return Err(ElfError::NotLittleEndian(data));
        }

        let identification_version = header[6];
        if u32::from(identification_version) != CURRENT_VERSION {
            return Err(ElfError::WrongIdentificationVersion(identification_version));
        }

        let kind = read_u16(header, 16)?;
        if kind != wanted {
            return Err(ElfError::WrongType(kind));
        }

        let entry = read_u64(header, 24)?;
        let machine = read_u16(header, 18)?;
        if machine != MACHINE_X86_64 {
            return Err(ElfError::WrongMachine(machine));
        }
        let version = read_u32(header, 20)?;
        if version != CURRENT_VERSION {
            return Err(ElfError::WrongVersion(version));
        }
        let header_size = read_u16(header, 52)?;
        if header_size as usize != HEADER_BYTES {
            return Err(ElfError::WrongHeaderSize(header_size));
        }

        let program_offset = read_u64(header, 32)?;
        let entry_size = read_u16(header, 54)?;
        let program_count = read_u16(header, 56)?;
        let sections = SectionTable {
            offset: read_u64(header, 40)?,
            entry_size: read_u16(header, 58)?,
            count: read_u16(header, 60)?,
            names_index: read_u16(header, 62)?,
        };

        if program_count != 0 && entry_size as usize != PROGRAM_HEADER_BYTES {
            return Err(ElfError::WrongProgramHeaderSize(entry_size));
        }

        let span = u64::from(program_count) * (PROGRAM_HEADER_BYTES as u64);
        let fits = program_offset
            .checked_add(span)
            .is_some_and(|end| end <= bytes.len() as u64);
        if !fits {
            return Err(ElfError::ProgramHeadersOutOfRange {
                offset: program_offset,
                count: program_count,
            });
        }

        Ok(Self {
            bytes,
            entry,
            program_offset,
            program_count,
            sections,
        })
    }

    pub const fn entry(&self) -> u64 {
        self.entry
    }

    pub const fn section_table(&self) -> SectionTable {
        self.sections
    }

    pub fn segments(&self) -> impl Iterator<Item = Segment> + use<'bytes> {
        let elf = *self;
        (0..elf.program_count).filter_map(move |index| elf.program_header(index).ok())
    }

    fn program_header(&self, index: u16) -> Result<Segment, ElfError> {
        let out_of_range = ElfError::ProgramHeadersOutOfRange {
            offset: self.program_offset,
            count: self.program_count,
        };
        let Ok(base) = usize::try_from(self.program_offset) else {
            return Err(out_of_range);
        };
        let Some(at) = base.checked_add(usize::from(index) * PROGRAM_HEADER_BYTES) else {
            return Err(out_of_range);
        };
        let header = take(self.bytes, at, PROGRAM_HEADER_BYTES)?;
        Ok(Segment {
            kind: SegmentKind::from_bits(read_u32(header, 0)?),
            flags: read_u32(header, 4)?,
            offset: read_u64(header, 8)?,
            address: read_u64(header, 16)?,
            file_size: read_u64(header, 32)?,
            memory_size: read_u64(header, 40)?,
            alignment: read_u64(header, 48)?,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Relocation {
    offset: u64,
    addend: i64,
}

impl Relocation {
    pub const fn offset(self) -> u64 {
        self.offset
    }

    pub const fn addend(self) -> i64 {
        self.addend
    }
}

const NO_SEGMENT: Segment = Segment {
    kind: SegmentKind::Null,
    flags: 0,
    offset: 0,
    address: 0,
    file_size: 0,
    memory_size: 0,
    alignment: 0,
};

// Every address and offset is relative to the base the loader picks; `span` is page rounded.
#[derive(Clone, Copy, Debug)]
pub struct Pie<'bytes> {
    entry: u64,
    span: u64,
    alignment: u64,
    loads: [Segment; MAX_LOADS],
    load_count: usize,
    relocations: &'bytes [u8],
}

impl<'bytes> Pie<'bytes> {
    pub fn parse(bytes: &'bytes [u8]) -> Result<Self, ElfError> {
        let elf = Elf::parse_kind(bytes, SHARED)?;
        let mut loads = [NO_SEGMENT; MAX_LOADS];
        let mut load_count = 0;
        let mut dynamic = None;
        let mut span = 0;
        let mut alignment = PAGE;
        let mut index = 0;
        while index < elf.program_count {
            let segment = elf.program_header(index)?;
            match segment.kind() {
                SegmentKind::Load => {
                    if load_count == MAX_LOADS {
                        return Err(ElfError::TooManyLoads);
                    }
                    span = check_load(segment, index, bytes.len(), span)?;
                    alignment = alignment.max(segment.alignment());
                    loads[load_count] = segment;
                    load_count += 1;
                }
                SegmentKind::Dynamic => {
                    if dynamic.is_some() {
                        return Err(ElfError::DynamicMalformed);
                    }
                    dynamic = Some(segment);
                }
                SegmentKind::Interpreter => return Err(ElfError::HasInterpreter),
                _ => {}
            }
            index += 1;
        }
        if load_count == 0 {
            return Err(ElfError::NoLoadSegments);
        }
        let Some(dynamic) = dynamic else {
            return Err(ElfError::MissingDynamic);
        };
        if !entry_in_code(&loads, load_count, elf.entry) {
            return Err(ElfError::EntryOutsideCode(elf.entry));
        }
        let relocations = locate_relocations(bytes, dynamic, &loads, load_count)?;
        check_relocations(relocations, &loads, load_count)?;
        Ok(Self {
            entry: elf.entry,
            span,
            alignment,
            loads,
            load_count,
            relocations,
        })
    }

    pub const fn entry(&self) -> u64 {
        self.entry
    }

    pub const fn span(&self) -> u64 {
        self.span
    }

    pub const fn alignment(&self) -> u64 {
        self.alignment
    }

    pub const fn segment_count(&self) -> usize {
        self.load_count
    }

    pub fn segment(&self, index: usize) -> Option<Segment> {
        if index >= self.load_count {
            return None;
        }
        self.loads.get(index).copied()
    }

    pub const fn relocation_count(&self) -> usize {
        self.relocations.len() / RELA_ENTRY_BYTES
    }

    pub fn relocation(&self, index: usize) -> Option<Relocation> {
        let (offset, _, addend) = read_relocation(self.relocations, index)?;
        Some(Relocation { offset, addend })
    }
}

fn check_load(segment: Segment, index: u16, file_len: usize, floor: u64) -> Result<u64, ElfError> {
    if segment.is_write_execute() {
        return Err(ElfError::WritableAndExecutable { index });
    }
    let malformed = ElfError::SegmentMalformed { index };
    if !segment.contents_fit(file_len)
        || segment.is_oversized()
        || !segment.has_valid_alignment()
        || segment.address() % PAGE != segment.offset() % PAGE
        || segment.address() < floor
    {
        return Err(malformed);
    }
    let Some(end) = segment.address().checked_add(segment.memory_size()) else {
        return Err(malformed);
    };
    let Some(padded) = end.checked_add(PAGE - 1) else {
        return Err(malformed);
    };
    Ok(padded & !(PAGE - 1))
}

fn entry_in_code(loads: &[Segment; MAX_LOADS], load_count: usize, entry: u64) -> bool {
    let mut index = 0;
    while index < load_count {
        let segment = loads[index];
        if segment.is_executable()
            && entry >= segment.address()
            && entry - segment.address() < segment.file_size()
        {
            return true;
        }
        index += 1;
    }
    false
}

fn set_once(slot: &mut Option<u64>, value: u64) -> Result<(), ElfError> {
    if slot.is_some() {
        return Err(ElfError::DynamicMalformed);
    }
    *slot = Some(value);
    Ok(())
}

fn file_range(bytes: &[u8], offset: u64, size: u64) -> Option<&[u8]> {
    let end = usize::try_from(offset.checked_add(size)?).ok()?;
    bytes.get(usize::try_from(offset).ok()?..end)
}

fn locate_relocations<'bytes>(
    bytes: &'bytes [u8],
    dynamic: Segment,
    loads: &[Segment; MAX_LOADS],
    load_count: usize,
) -> Result<&'bytes [u8], ElfError> {
    let Some(table) = file_range(bytes, dynamic.offset(), dynamic.file_size()) else {
        return Err(ElfError::DynamicMalformed);
    };
    let mut rela = None;
    let mut rela_size = None;
    let mut rela_entry = None;
    let mut ended = false;
    let mut index = 0;
    while index < table.len() / DYNAMIC_ENTRY_BYTES {
        let at = index * DYNAMIC_ENTRY_BYTES;
        let tag = read_u64(table, at)?;
        let value = read_u64(table, at + 8)?;
        match tag {
            DT_NULL => {
                ended = true;
                break;
            }
            DT_NEEDED | DT_REL | DT_RELR => return Err(ElfError::UnsupportedDynamicTag(tag)),
            DT_PLTRELSZ if value != 0 => return Err(ElfError::UnsupportedDynamicTag(tag)),
            DT_RELA => set_once(&mut rela, value)?,
            DT_RELASZ => set_once(&mut rela_size, value)?,
            DT_RELAENT => set_once(&mut rela_entry, value)?,
            _ => {}
        }
        index += 1;
    }
    if !ended || table.len() % DYNAMIC_ENTRY_BYTES != 0 {
        return Err(ElfError::DynamicMalformed);
    }

    let size = rela_size.unwrap_or(0);
    let Some(address) = rela else {
        if size != 0 {
            return Err(ElfError::RelocationTableOutOfRange);
        }
        return Ok(&[]);
    };
    let entry_size = rela_entry.unwrap_or(0);
    if entry_size != RELA_ENTRY_BYTES as u64 {
        return Err(ElfError::WrongRelocationEntrySize(entry_size));
    }
    if size % entry_size != 0 {
        return Err(ElfError::RelocationTableOutOfRange);
    }
    if size == 0 {
        return Ok(&[]);
    }
    let mut index = 0;
    while index < load_count {
        let segment = loads[index];
        if address >= segment.address() {
            let start = address - segment.address();
            if start <= segment.file_size()
                && size <= segment.file_size() - start
                && let Some(offset) = segment.offset().checked_add(start)
                && let Some(found) = file_range(bytes, offset, size)
            {
                return Ok(found);
            }
        }
        index += 1;
    }
    Err(ElfError::RelocationTableOutOfRange)
}

fn read_relocation(table: &[u8], index: usize) -> Option<(u64, u64, i64)> {
    let entry = take(
        table,
        index.checked_mul(RELA_ENTRY_BYTES)?,
        RELA_ENTRY_BYTES,
    )
    .ok()?;
    let offset = read_u64(entry, 0).ok()?;
    let info = read_u64(entry, 8).ok()?;
    let addend = i64::from_le_bytes(read_u64(entry, 16).ok()?.to_le_bytes());
    Some((offset, info, addend))
}

fn check_relocations(
    table: &[u8],
    loads: &[Segment; MAX_LOADS],
    load_count: usize,
) -> Result<(), ElfError> {
    let mut index = 0;
    while index < table.len() / RELA_ENTRY_BYTES {
        let Some((offset, info, _)) = read_relocation(table, index) else {
            return Err(ElfError::RelocationTableOutOfRange);
        };
        if info != R_X86_64_RELATIVE {
            return Err(ElfError::UnsupportedRelocation(info));
        }
        if !in_writable(loads, load_count, offset) {
            return Err(ElfError::RelocationOutsideWritable(offset));
        }
        index += 1;
    }
    Ok(())
}

fn in_writable(loads: &[Segment; MAX_LOADS], load_count: usize, offset: u64) -> bool {
    let mut index = 0;
    while index < load_count {
        let segment = loads[index];
        if segment.is_writable() && offset >= segment.address() {
            let start = offset - segment.address();
            if let Some(end) = start.checked_add(8)
                && end <= segment.memory_size()
            {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn take(bytes: &[u8], at: usize, length: usize) -> Result<&[u8], ElfError> {
    bytes
        .get(at..)
        .and_then(|rest| rest.get(..length))
        .ok_or(ElfError::Truncated {
            wanted: at.saturating_add(length),
            available: bytes.len(),
        })
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, ElfError> {
    let field = take(bytes, at, 2)?;
    Ok(u16::from_le_bytes([field[0], field[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, ElfError> {
    let field = take(bytes, at, 4)?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64, ElfError> {
    let field = take(bytes, at, 8)?;
    let mut value = [0u8; 8];
    value.copy_from_slice(field);
    Ok(u64::from_le_bytes(value))
}

#[cfg(test)]
mod tests;
