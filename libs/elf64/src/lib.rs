//! Just enough ELF64 to load a program: the header, and the segments.
//!
//! # Examples
//!
//! ```
//! use elf64::{Elf, SegmentKind, HEADER_BYTES, PROGRAM_HEADER_BYTES};
//!
//! // The smallest thing that is an ELF: a header and one loadable segment.
//! let mut image = vec![0u8; HEADER_BYTES + PROGRAM_HEADER_BYTES];
//! image[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
//! image[4] = 2; // ELFCLASS64
//! image[5] = 1; // little-endian
//! image[6] = 1; // current ELF identification version
//! image[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
//! image[18..20].copy_from_slice(&62u16.to_le_bytes()); // EM_X86_64
//! image[20..24].copy_from_slice(&1u32.to_le_bytes()); // current ELF version
//! image[24..32].copy_from_slice(&0x40_1000u64.to_le_bytes()); // e_entry
//! image[32..40].copy_from_slice(&(HEADER_BYTES as u64).to_le_bytes()); // e_phoff
//! image[52..54].copy_from_slice(&(HEADER_BYTES as u16).to_le_bytes()); // e_ehsize
//! image[54..56].copy_from_slice(&(PROGRAM_HEADER_BYTES as u16).to_le_bytes());
//! image[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
//!
//! let at = HEADER_BYTES;
//! image[at..at + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
//! image[at + 4..at + 8].copy_from_slice(&5u32.to_le_bytes()); // R|X
//! image[at + 16..at + 24].copy_from_slice(&0x40_0000u64.to_le_bytes()); // p_vaddr
//!
//! let elf = Elf::parse(&image).expect("a 64-bit little-endian ELF");
//! assert_eq!(elf.entry(), 0x40_1000);
//!
//! let loadable: Vec<_> = elf
//!     .segments()
//!     .filter(|segment| segment.kind() == SegmentKind::Load)
//!     .collect();
//! assert_eq!(loadable.len(), 1);
//! assert!(loadable[0].is_executable() && !loadable[0].is_writable());
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::fmt;

pub const MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

/// `ELFCLASS64`.
pub const CLASS_64: u8 = 2;

/// `ELFDATA2LSB`.
pub const LITTLE_ENDIAN: u8 = 1;

pub const HEADER_BYTES: usize = 64;

pub const PROGRAM_HEADER_BYTES: usize = 56;

/// `EM_X86_64`, the only machine this loader will run.
pub const MACHINE_X86_64: u16 = 62;

const CURRENT_VERSION: u32 = 1;

const EXECUTABLE: u16 = 2;

/// Why an image could not be read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ElfError {
    /// Shorter than the structure being read.
    Truncated { wanted: usize, available: usize },
    /// Does not begin with [`MAGIC`].
    NotAnElf,
    /// Not `ELFCLASS64`.
    NotSixtyFourBit(u8),
    /// Not `ELFDATA2LSB`.
    NotLittleEndian(u8),
    /// The independent identification version is not `EV_CURRENT`.
    WrongIdentificationVersion(u8),
    /// The object is not an executable image.
    WrongType(u16),
    /// The file-header version is not `EV_CURRENT`.
    WrongVersion(u32),
    /// Not built for this machine, though every other rule here may pass.
    WrongMachine(u16),
    /// The file-header size is not [`HEADER_BYTES`].
    WrongHeaderSize(u16),
    /// The program-header entry size is not [`PROGRAM_HEADER_BYTES`], so it is another format.
    WrongProgramHeaderSize(u16),
    /// The program-header table does not fit inside the image.
    ProgramHeadersOutOfRange { offset: u64, count: u16 },
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Truncated { wanted, available } => {
                write!(f, "wanted {wanted} bytes and there are {available}")
            }
            Self::NotAnElf => f.write_str("does not begin with the elf magic"),
            Self::NotSixtyFourBit(class) => write!(f, "elf class {class} is not 64-bit"),
            Self::NotLittleEndian(data) => {
                write!(f, "elf data encoding {data} is not little-endian")
            }
            Self::WrongIdentificationVersion(version) => {
                write!(f, "elf identification version {version} is not current")
            }
            Self::WrongType(kind) => write!(f, "elf object type {kind} is not executable"),
            Self::WrongVersion(version) => {
                write!(f, "elf file version {version} is not current")
            }
            Self::WrongMachine(machine) => {
                write!(f, "elf machine {machine} is not x86-64")
            }
            Self::WrongHeaderSize(size) => {
                write!(f, "elf header is {size} bytes, expected {HEADER_BYTES}")
            }
            Self::WrongProgramHeaderSize(size) => {
                write!(f, "program headers of {size} bytes are not this format's")
            }
            Self::ProgramHeadersOutOfRange { offset, count } => {
                write!(
                    f,
                    "{count} program headers at {offset:#x} run past the image"
                )
            }
        }
    }
}

impl core::error::Error for ElfError {}

/// What a program header describes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum SegmentKind {
    /// `PT_NULL`.
    Null,
    /// `PT_LOAD`, the only kind a loader places in memory.
    Load,
    /// `PT_DYNAMIC`.
    Dynamic,
    /// `PT_INTERP`, a request for a dynamic linker this kernel does not have.
    Interpreter,
    /// `PT_NOTE`.
    Note,
    /// `PT_PHDR`.
    ProgramHeaders,
    /// `PT_TLS`.
    ThreadLocal,
    /// Anything else by value, such as `PT_GNU_STACK`.
    Other(u32),
}

impl SegmentKind {
    /// Decode the `p_type` field.
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

    /// The `p_type` field, the inverse of [`from_bits`](Self::from_bits) so the table is checkable.
    pub const fn bits(self) -> u32 {
        match self {
            Self::Null => 0,
            Self::Load => 1,
            Self::Dynamic => 2,
            Self::Interpreter => 3,
            Self::Note => 4,
            Self::ProgramHeaders => 6,
            Self::ThreadLocal => 7,
            Self::Other(other) => other,
        }
    }
}

/// One program header.
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
    /// `PF_X`.
    pub const EXECUTE: u32 = 1;
    /// `PF_W`.
    pub const WRITE: u32 = 1 << 1;
    /// `PF_R`.
    pub const READ: u32 = 1 << 2;

    pub const fn kind(self) -> SegmentKind {
        self.kind
    }

    /// The raw `p_flags` field, for a verifier that reports the value it objected to.
    pub const fn flags(self) -> u32 {
        self.flags
    }

    /// Where in the file its contents start.
    pub const fn offset(self) -> u64 {
        self.offset
    }

    pub const fn address(self) -> u64 {
        self.address
    }

    pub const fn file_size(self) -> u64 {
        self.file_size
    }

    /// Bytes to occupy in memory, zero past `file_size`.
    pub const fn memory_size(self) -> u64 {
        self.memory_size
    }

    pub const fn alignment(self) -> u64 {
        self.alignment
    }

    /// Whether `p_align` is legal and the file and memory offsets agree.
    pub const fn has_valid_alignment(self) -> bool {
        self.alignment <= 1
            || self.alignment.is_power_of_two()
                && self.address % self.alignment == self.offset % self.alignment
    }

    pub const fn is_readable(self) -> bool {
        self.flags & Self::READ != 0
    }

    pub const fn is_writable(self) -> bool {
        self.flags & Self::WRITE != 0
    }

    pub const fn is_executable(self) -> bool {
        self.flags & Self::EXECUTE != 0
    }

    /// Whether it asks for W+X, which a loader refuses rather than dropping a permission.
    pub const fn is_write_execute(self) -> bool {
        self.is_writable() && self.is_executable()
    }

    /// Whether its file contents fit inside an image of `bytes` bytes, without overflow.
    pub const fn contents_fit(self, bytes: usize) -> bool {
        match self.offset.checked_add(self.file_size) {
            Some(end) => end <= bytes as u64,
            None => false,
        }
    }

    /// Whether it claims more file bytes than memory to put them in.
    pub const fn is_oversized(self) -> bool {
        self.file_size > self.memory_size
    }
}

/// The file header's unvalidated description of the section table, for the host verifier.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SectionTable {
    offset: u64,
    entry_size: u16,
    count: u16,
    names_index: u16,
}

impl SectionTable {
    /// File offset of the first section header.
    pub const fn offset(self) -> u64 {
        self.offset
    }

    pub const fn entry_size(self) -> u16 {
        self.entry_size
    }

    pub const fn count(self) -> u16 {
        self.count
    }

    /// Index of the section-name string table.
    pub const fn names_index(self) -> u16 {
        self.names_index
    }
}

/// A parsed ELF64 image, borrowing the bytes it came from.
#[derive(Clone, Copy, Debug)]
pub struct Elf<'bytes> {
    bytes: &'bytes [u8],
    entry: u64,
    program_offset: u64,
    program_count: u16,
    sections: SectionTable,
}

impl<'bytes> Elf<'bytes> {
    /// Validate the file header and program-header table bounds, leaving segments to the loader.
    pub fn parse(bytes: &'bytes [u8]) -> Result<Self, ElfError> {
        let header = take(bytes, 0, HEADER_BYTES)?;
        if header.get(..4) != Some(&MAGIC) {
            return Err(ElfError::NotAnElf);
        }
        match header.get(4) {
            Some(&CLASS_64) => {}
            Some(&class) => return Err(ElfError::NotSixtyFourBit(class)),
            None => return Err(ElfError::NotAnElf),
        }
        match header.get(5) {
            Some(&LITTLE_ENDIAN) => {}
            Some(&data) => return Err(ElfError::NotLittleEndian(data)),
            None => return Err(ElfError::NotAnElf),
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

        let span = (program_count as u64) * (PROGRAM_HEADER_BYTES as u64);
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

    pub const fn segment_count(&self) -> u16 {
        self.program_count
    }

    pub const fn section_table(&self) -> SectionTable {
        self.sections
    }

    /// Every program header, in order, inside the table `parse` already bounded.
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

/// `len` bytes at `at`, or why they are not there.
fn take(bytes: &[u8], at: usize, len: usize) -> Result<&[u8], ElfError> {
    bytes
        .get(at..)
        .and_then(|rest| rest.get(..len))
        .ok_or(ElfError::Truncated {
            wanted: at.saturating_add(len),
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
