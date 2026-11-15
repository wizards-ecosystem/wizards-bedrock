#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::fmt;

pub const MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

pub const CLASS_64: u8 = 2;

pub const LITTLE_ENDIAN: u8 = 1;

pub const HEADER_BYTES: usize = 64;

pub const PROGRAM_HEADER_BYTES: usize = 56;

pub const MACHINE_X86_64: u16 = 62;

const CURRENT_VERSION: u32 = 1;

const EXECUTABLE: u16 = 2;

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
            Self::WrongType(kind) => write!(formatter, "elf object type {kind} is not executable"),
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
        if kind != EXECUTABLE {
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
        let bytes = self.bytes;
        let base = self.program_offset as usize;
        (0..self.program_count as usize).filter_map(move |index| {
            let at = base + index * PROGRAM_HEADER_BYTES;
            let header = take(bytes, at, PROGRAM_HEADER_BYTES).ok()?;
            Some(Segment {
                kind: SegmentKind::from_bits(read_u32(header, 0).ok()?),
                flags: read_u32(header, 4).ok()?,
                offset: read_u64(header, 8).ok()?,
                address: read_u64(header, 16).ok()?,
                file_size: read_u64(header, 32).ok()?,
                memory_size: read_u64(header, 40).ok()?,
                alignment: read_u64(header, 48).ok()?,
            })
        })
    }
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
