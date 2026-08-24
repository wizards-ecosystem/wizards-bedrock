//! Just enough ELF64 to load a program: the header, and the segments.
//!
//! Two callers want this and they want different amounts of it. `xtask`
//! verifies the *linked kernel* after every build (D-40) and cares about
//! sections, names and a string table — host things, with an allocator. The
//! kernel loads a *user program* and cares about program headers and an entry
//! point, in `no_std`, with no allocator and no ability to panic on hostile
//! input.
//!
//! What they share is the format itself, and this crate is that. Keeping it in
//! one place is not tidiness: two transcriptions of the same table is how
//! `x86-tables` and `trap-cause` came to disagree about vector 28, and that
//! was found by a test written to compare them rather than by either being
//! wrong on its own.
//!
//! **Nothing here allocates, indexes unchecked, or panics.** The bytes may
//! come from an initrd a program supplied, so every read is bounds-checked
//! against the slice and every field that could be a length is checked against
//! what remains.
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
//! image[24..32].copy_from_slice(&0x40_1000u64.to_le_bytes()); // e_entry
//! image[32..40].copy_from_slice(&(HEADER_BYTES as u64).to_le_bytes()); // e_phoff
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

/// Bytes of the identification array at the start of every ELF.
pub const IDENT_BYTES: usize = 16;

/// What every ELF begins with.
pub const MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

/// `ELFCLASS64`.
pub const CLASS_64: u8 = 2;

/// `ELFDATA2LSB`.
pub const LITTLE_ENDIAN: u8 = 1;

/// Bytes of an ELF64 file header.
pub const HEADER_BYTES: usize = 64;

/// Bytes of an ELF64 program header.
pub const PROGRAM_HEADER_BYTES: usize = 56;

/// Why an image could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ElfError {
    /// Shorter than the structure being read.
    Truncated {
        /// Bytes wanted.
        wanted: usize,
        /// Bytes there.
        available: usize,
    },
    /// Does not begin with [`MAGIC`].
    NotAnElf,
    /// Not `ELFCLASS64`.
    NotSixtyFourBit(u8),
    /// Not `ELFDATA2LSB`.
    NotLittleEndian(u8),
    /// The program-header entry size is not [`PROGRAM_HEADER_BYTES`].
    ///
    /// Refused rather than honoured: a different size means a different
    /// format, and walking it with this one's field offsets would read
    /// whatever happened to be there.
    WrongProgramHeaderSize(u16),
    /// The program-header table does not fit inside the image.
    ProgramHeadersOutOfRange {
        /// Where the table claims to start.
        offset: u64,
        /// How many entries it claims.
        count: u16,
    },
}

impl core::fmt::Display for ElfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Truncated { wanted, available } => {
                write!(f, "wanted {wanted} bytes and there are {available}")
            }
            Self::NotAnElf => f.write_str("does not begin with the elf magic"),
            Self::NotSixtyFourBit(class) => write!(f, "elf class {class} is not 64-bit"),
            Self::NotLittleEndian(data) => {
                write!(f, "elf data encoding {data} is not little-endian")
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SegmentKind {
    /// `PT_NULL` — ignore it.
    Null,
    /// `PT_LOAD` — the only kind a loader places in memory.
    Load,
    /// `PT_DYNAMIC`.
    Dynamic,
    /// `PT_INTERP` — the program wants a dynamic linker, which this kernel
    /// does not have.
    Interpreter,
    /// `PT_NOTE`.
    Note,
    /// `PT_PHDR`.
    ProgramHeaders,
    /// `PT_TLS`.
    ThreadLocal,
    /// Anything else, by value. `PT_GNU_STACK` and friends land here.
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
}

/// One program header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// What it describes.
    pub const fn kind(self) -> SegmentKind {
        self.kind
    }

    /// Where in the file its contents start.
    pub const fn offset(self) -> u64 {
        self.offset
    }

    /// Where in memory it goes.
    pub const fn address(self) -> u64 {
        self.address
    }

    /// Bytes to copy from the file.
    pub const fn file_size(self) -> u64 {
        self.file_size
    }

    /// Bytes to occupy in memory. Anything past `file_size` is zero — which is
    /// how `.bss` costs no file space (D-40).
    pub const fn memory_size(self) -> u64 {
        self.memory_size
    }

    /// What alignment it asks for.
    pub const fn alignment(self) -> u64 {
        self.alignment
    }

    /// Whether it may be read.
    pub const fn is_readable(self) -> bool {
        self.flags & Self::READ != 0
    }

    /// Whether it may be written.
    pub const fn is_writable(self) -> bool {
        self.flags & Self::WRITE != 0
    }

    /// Whether it may be executed.
    pub const fn is_executable(self) -> bool {
        self.flags & Self::EXECUTE != 0
    }

    /// Whether it asks to be both writable and executable.
    ///
    /// The one combination this kernel never maps (D-12, D-32). A loader is
    /// expected to refuse the image rather than drop a permission, because an
    /// image that wants W+X is not an image that works without it.
    pub const fn is_write_execute(self) -> bool {
        self.is_writable() && self.is_executable()
    }

    /// Whether its file contents fit inside an image of `bytes` bytes.
    ///
    /// Separate from parsing because a header is well formed whatever it
    /// points at: this is the question a *loader* must ask before copying, and
    /// it is checked here so no caller has to remember the overflow.
    pub const fn contents_fit(self, bytes: usize) -> bool {
        match self.offset.checked_add(self.file_size) {
            Some(end) => end <= bytes as u64,
            None => false,
        }
    }

    /// Whether it claims more file bytes than memory to put them in.
    ///
    /// Malformed, and worth its own question: a loader that copied
    /// `file_size` into a `memory_size` region would write past the mapping.
    pub const fn is_oversized(self) -> bool {
        self.file_size > self.memory_size
    }
}

/// A parsed ELF64 image, borrowing the bytes it came from.
#[derive(Debug, Clone, Copy)]
pub struct Elf<'a> {
    bytes: &'a [u8],
    entry: u64,
    program_offset: u64,
    program_count: u16,
}

impl<'a> Elf<'a> {
    /// Read the file header, or say why it is not one.
    ///
    /// Validates the identification, the class, the endianness, the
    /// program-header entry size, and that the program-header table lies
    /// inside the image. It does **not** validate the segments: a caller that
    /// only wants the entry point should not pay for a walk, and a loader has
    /// to look at each one anyway.
    ///
    /// # Errors
    ///
    /// See [`ElfError`].
    pub fn parse(bytes: &'a [u8]) -> Result<Self, ElfError> {
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

        let entry = read_u64(header, 24)?;
        let program_offset = read_u64(header, 32)?;
        let entry_size = read_u16(header, 54)?;
        let program_count = read_u16(header, 56)?;

        if program_count != 0 && entry_size as usize != PROGRAM_HEADER_BYTES {
            return Err(ElfError::WrongProgramHeaderSize(entry_size));
        }

        // The table must lie inside the image, checked without overflowing:
        // a `p_phoff` near `u64::MAX` with a plausible count is exactly what a
        // hostile image looks like.
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
        })
    }

    /// Where execution begins.
    pub const fn entry(&self) -> u64 {
        self.entry
    }

    /// How many program headers there are.
    pub const fn segment_count(&self) -> u16 {
        self.program_count
    }

    /// The bytes this was parsed from.
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Every program header, in order.
    ///
    /// Infallible by then: `parse` already established that the whole table is
    /// inside the image, so the walk cannot run off the end and needs no
    /// per-entry error.
    pub fn segments(&self) -> impl Iterator<Item = Segment> + use<'a> {
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
mod tests {
    use super::*;

    /// A well-formed image with the given program headers.
    fn image(segments: &[(u32, u32, u64, u64, u64, u64)]) -> Vec<u8> {
        let mut bytes = vec![0u8; HEADER_BYTES + segments.len() * PROGRAM_HEADER_BYTES];
        bytes[0..4].copy_from_slice(&MAGIC);
        bytes[4] = CLASS_64;
        bytes[5] = LITTLE_ENDIAN;
        bytes[24..32].copy_from_slice(&0x40_1000u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&(HEADER_BYTES as u64).to_le_bytes());
        bytes[54..56].copy_from_slice(&(PROGRAM_HEADER_BYTES as u16).to_le_bytes());
        bytes[56..58].copy_from_slice(&(segments.len() as u16).to_le_bytes());

        for (index, (kind, flags, offset, address, file, memory)) in
            segments.iter().copied().enumerate()
        {
            let at = HEADER_BYTES + index * PROGRAM_HEADER_BYTES;
            bytes[at..at + 4].copy_from_slice(&kind.to_le_bytes());
            bytes[at + 4..at + 8].copy_from_slice(&flags.to_le_bytes());
            bytes[at + 8..at + 16].copy_from_slice(&offset.to_le_bytes());
            bytes[at + 16..at + 24].copy_from_slice(&address.to_le_bytes());
            bytes[at + 32..at + 40].copy_from_slice(&file.to_le_bytes());
            bytes[at + 40..at + 48].copy_from_slice(&memory.to_le_bytes());
            bytes[at + 48..at + 56].copy_from_slice(&0x1000u64.to_le_bytes());
        }
        bytes
    }

    /// The header's fields land where the format says.
    ///
    /// Offsets transcribed from the ELF64 specification rather than taken from
    /// the parser, so a field read from the wrong place fails here.
    #[test]
    fn the_header_fields_are_where_the_format_puts_them() {
        let bytes = image(&[(1, 5, 0, 0x40_0000, 0x100, 0x200)]);
        let elf = Elf::parse(&bytes).expect("well formed");
        assert_eq!(elf.entry(), 0x40_1000, "e_entry is at offset 24");
        assert_eq!(elf.segment_count(), 1, "e_phnum is at offset 56");

        let segment = elf.segments().next().expect("one segment");
        assert_eq!(segment.kind(), SegmentKind::Load, "p_type is at offset 0");
        assert_eq!(segment.address(), 0x40_0000, "p_vaddr is at offset 16");
        assert_eq!(segment.file_size(), 0x100, "p_filesz is at offset 32");
        assert_eq!(segment.memory_size(), 0x200, "p_memsz is at offset 40");
        assert_eq!(segment.alignment(), 0x1000, "p_align is at offset 48");
    }

    /// Anything that is not this format is refused, each for its own reason.
    #[test]
    fn what_is_not_an_elf64_is_refused_by_name() {
        assert_eq!(
            Elf::parse(&[]).err(),
            Some(ElfError::Truncated {
                wanted: HEADER_BYTES,
                available: 0
            })
        );

        let short = vec![0u8; HEADER_BYTES - 1];
        assert!(matches!(
            Elf::parse(&short),
            Err(ElfError::Truncated { .. })
        ));

        let mut wrong = image(&[]);
        wrong[1] = b'X';
        assert_eq!(Elf::parse(&wrong).err(), Some(ElfError::NotAnElf));

        let mut thirty_two = image(&[]);
        thirty_two[4] = 1;
        assert_eq!(
            Elf::parse(&thirty_two).err(),
            Some(ElfError::NotSixtyFourBit(1))
        );

        let mut big = image(&[]);
        big[5] = 2;
        assert_eq!(Elf::parse(&big).err(), Some(ElfError::NotLittleEndian(2)));
    }

    /// **A program-header table running past the image is refused, and the
    /// check cannot be overflowed past.**
    ///
    /// The shape of a hostile image: an offset near the top of the range with
    /// a plausible count, so `offset + count * size` wraps to something small
    /// and looks like it fits. An initrd is where these bytes come from.
    #[test]
    fn a_table_that_runs_past_the_image_is_refused() {
        let mut bytes = image(&[(1, 5, 0, 0x40_0000, 0, 0)]);

        // Honest overrun: more entries than there is table.
        bytes[56..58].copy_from_slice(&64u16.to_le_bytes());
        assert!(matches!(
            Elf::parse(&bytes),
            Err(ElfError::ProgramHeadersOutOfRange { .. })
        ));

        // Wrapping overrun: the arithmetic must not come back round.
        bytes[56..58].copy_from_slice(&1u16.to_le_bytes());
        bytes[32..40].copy_from_slice(&(u64::MAX - 8).to_le_bytes());
        assert!(
            matches!(
                Elf::parse(&bytes),
                Err(ElfError::ProgramHeadersOutOfRange { .. })
            ),
            "an offset near the top of the range wrapped into looking valid"
        );
    }

    /// A header size this parser does not speak is refused rather than walked
    /// with the wrong field offsets.
    #[test]
    fn an_unfamiliar_program_header_size_is_refused() {
        let mut bytes = image(&[(1, 5, 0, 0x40_0000, 0, 0)]);
        bytes[54..56].copy_from_slice(&32u16.to_le_bytes());
        assert_eq!(
            Elf::parse(&bytes).err(),
            Some(ElfError::WrongProgramHeaderSize(32))
        );

        // With no headers the size means nothing, and a linker may leave it
        // zero.
        let mut empty = image(&[]);
        empty[54..56].copy_from_slice(&0u16.to_le_bytes());
        assert!(Elf::parse(&empty).is_ok());
    }

    /// Permissions decode independently, and write-execute is recognised.
    #[test]
    fn permissions_decode_and_write_execute_is_visible() {
        let bytes = image(&[
            (1, Segment::READ, 0, 0x1000, 0, 0),
            (1, Segment::READ | Segment::WRITE, 0, 0x2000, 0, 0),
            (1, Segment::READ | Segment::EXECUTE, 0, 0x3000, 0, 0),
            (1, Segment::WRITE | Segment::EXECUTE, 0, 0x4000, 0, 0),
        ]);
        let segments: Vec<Segment> = Elf::parse(&bytes)
            .expect("well formed")
            .segments()
            .collect();
        assert_eq!(segments.len(), 4);

        assert!(segments[0].is_readable() && !segments[0].is_writable());
        assert!(segments[1].is_writable() && !segments[1].is_executable());
        assert!(segments[2].is_executable() && !segments[2].is_writable());
        assert!(
            segments[3].is_write_execute(),
            "a writable executable segment was not recognised"
        );
        assert!(
            !segments[0].is_write_execute() && !segments[1].is_write_execute(),
            "write-execute reported for a segment that is only one of them"
        );
    }

    /// A loader can ask whether the contents fit without remembering the
    /// overflow itself.
    #[test]
    fn a_loader_can_ask_whether_the_contents_fit() {
        let bytes = image(&[
            (1, 5, 0, 0x1000, 16, 32),
            (1, 5, 0, 0x2000, u64::MAX, u64::MAX),
            (1, 5, u64::MAX - 4, 0x3000, 16, 16),
            (1, 5, 0, 0x4000, 64, 16),
        ]);
        let segments: Vec<Segment> = Elf::parse(&bytes)
            .expect("well formed")
            .segments()
            .collect();

        assert!(segments[0].contents_fit(bytes.len()));
        assert!(!segments[1].contents_fit(bytes.len()), "a huge size fitted");
        assert!(
            !segments[2].contents_fit(bytes.len()),
            "an offset near the top of the range wrapped into fitting"
        );
        assert!(
            segments[3].is_oversized(),
            "more file than memory went unseen"
        );
        assert!(!segments[0].is_oversized());
    }

    /// An unknown segment type keeps its number rather than becoming nothing.
    #[test]
    fn an_unknown_segment_type_keeps_its_number() {
        let bytes = image(&[(0x6474_e551, 6, 0, 0, 0, 0)]);
        let segment = Elf::parse(&bytes)
            .expect("well formed")
            .segments()
            .next()
            .expect("one segment");
        assert_eq!(segment.kind(), SegmentKind::Other(0x6474_e551));
    }

    /// The real linked kernel parses, and looks like a kernel.
    ///
    /// The one input here nobody wrote for the test. Read only when a build
    /// has produced it, so a fresh checkout does not fail on its absence.
    #[test]
    fn the_linked_kernel_parses() {
        let candidates = [
            "../../target/x86_64-unknown-none/release/wizard-kernel",
            "../../target/x86_64-unknown-none/debug/wizard-kernel",
        ];
        let Some(bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) else {
            return;
        };

        let elf = Elf::parse(&bytes).expect("the linked kernel is an elf64");
        assert!(
            elf.entry() >= 0xffff_ffff_8000_0000,
            "the entry point is not in the higher half"
        );

        let loadable: Vec<Segment> = elf
            .segments()
            .filter(|segment| segment.kind() == SegmentKind::Load)
            .collect();
        assert!(!loadable.is_empty(), "no loadable segments");
        for segment in loadable {
            assert!(
                !segment.is_write_execute(),
                "the kernel has a writable executable segment, which D-40 forbids"
            );
            assert!(segment.contents_fit(bytes.len()));
            assert!(!segment.is_oversized());
        }
    }
}
