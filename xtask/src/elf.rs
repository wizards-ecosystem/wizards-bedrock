//! Reading the linked kernel back, and checking it is the image we meant.
//!
//! A linker script fails loudly when it is malformed and quietly when it is
//! merely incomplete: an unnamed section becomes an *orphan*, placed by a
//! heuristic rather than by the script, and the result still links. So the
//! properties `kernel/linker.ld` is written to produce are asserted here
//! against the bytes it produced, after every build.
//!
//! Just enough ELF64 to do that — program headers, section headers, and their
//! names. Hand-rolled on the `sha256.rs` precedent rather than taking on a
//! dependency for four `u64`s per record.

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

/// Section type: occupies address space but no file space.
const SHT_NOBITS: u32 = 8;

/// Section flag: this section is part of the loaded image.
const SHF_ALLOC: u64 = 1 << 1;
/// Section flag: bytes may be written after loading.
const SHF_WRITE: u64 = 1;
/// Section flag: bytes may be executed.
const SHF_EXECUTE: u64 = 1 << 2;

const PAGE_BYTES: u64 = 4096;

const SECTION_HEADER_BYTES: usize = 64;

/// Largest linked kernel artifact the host verifier and image builder admit.
pub(crate) const MAX_KERNEL_ELF_BYTES: u64 = 64 * 1024 * 1024;
/// A kernel needs tens of sections; this leaves generous diagnostic headroom.
const MAX_SECTION_COUNT: usize = 4096;
/// Aggregate owned section-name bytes, independent of repeated name offsets.
const MAX_SECTION_NAME_BYTES: usize = 1024 * 1024;

/// Where the higher half begins — the base `kernel/linker.ld` links at, and
/// the range the kernel code model can address.
const HIGHER_HALF: u64 = 0xffff_ffff_8000_0000;

const fn ranges_overlap(left: u64, left_size: u64, right: u64, right_size: u64) -> bool {
    if left_size == 0 || right_size == 0 {
        return false;
    }
    match (left.checked_add(left_size), right.checked_add(right_size)) {
        (Some(left_end), Some(right_end)) => left < right_end && right < left_end,
        _ => true,
    }
}

/// One loadable segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// `p_type`.
    pub kind: u32,
    /// `p_flags`.
    pub flags: u32,
    /// `p_vaddr`.
    pub vaddr: u64,
    /// `p_filesz` — bytes taken from the file.
    pub file_size: u64,
    /// `p_memsz` — bytes occupied once loaded.
    pub memory_size: u64,
}

impl Segment {
    /// Whether this segment is loaded into memory at all.
    pub const fn is_loadable(self) -> bool {
        self.kind == elf64::SegmentKind::Load.bits()
    }

    /// Whether an address of `size` bytes at `vaddr` lies inside this segment.
    pub const fn covers(self, vaddr: u64, size: u64) -> bool {
        match (
            self.vaddr.checked_add(self.memory_size),
            vaddr.checked_add(size),
        ) {
            (Some(segment_end), Some(end)) => vaddr >= self.vaddr && end <= segment_end,
            _ => false,
        }
    }

    /// Whether this segment intersects any byte of another non-empty range.
    pub const fn overlaps(self, vaddr: u64, size: u64) -> bool {
        if self.memory_size == 0 || size == 0 {
            return false;
        }
        match (
            self.vaddr.checked_add(self.memory_size),
            vaddr.checked_add(size),
        ) {
            (Some(segment_end), Some(end)) => self.vaddr < end && vaddr < segment_end,
            _ => true,
        }
    }

    /// Whether this segment is both writable and executable.
    pub const fn is_writable_and_executable(self) -> bool {
        self.flags & elf64::Segment::WRITE != 0 && self.flags & elf64::Segment::EXECUTE != 0
    }
}

/// One section header, with its name resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The name from the section header string table.
    pub name: String,
    /// `sh_type`.
    pub kind: u32,
    /// `sh_flags`.
    pub flags: u64,
    /// `sh_addr`.
    pub addr: u64,
    /// `sh_size`.
    pub size: u64,
}

impl Section {
    /// Whether this section is part of the loaded image.
    pub const fn is_allocated(&self) -> bool {
        self.flags & SHF_ALLOC != 0
    }

    /// Whether this section occupies address space but no file space.
    pub const fn is_nobits(&self) -> bool {
        self.kind == SHT_NOBITS
    }
}

/// A linked ELF64 image, reduced to what the checks below need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The entry point address.
    pub entry: u64,
    /// Every program header, in order.
    pub segments: Vec<Segment>,
    /// Every section header, in order.
    pub sections: Vec<Section>,
}

/// Why an image is not the one the linker script describes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImageError {
    /// The entry point is outside the higher half the kernel is linked at.
    EntryOutsideHigherHalf {
        /// The entry point that was found.
        entry: u64,
    },
    /// An allocated section lies outside every loadable segment — the shape an
    /// orphan section takes when the heuristic places it badly.
    SectionNotLoaded {
        /// Name of the stranded section.
        name: String,
        /// Where it was placed.
        addr: u64,
    },
    /// A segment is writable and executable at once.
    WritableAndExecutable {
        /// Where the segment begins.
        vaddr: u64,
    },
    /// A section that should occupy no file space is being written into the
    /// image, which is what happens when something loadable follows it.
    NobitsInFile {
        /// Name of the section.
        name: String,
        /// Bytes of file space its segment is spending.
        wasted: u64,
    },
    /// A section the boot protocol needs is absent or not loaded.
    MissingSection {
        /// Name of the section.
        name: &'static str,
    },
    /// A dedicated entry-enclave section is not page-exact or has the wrong
    /// memory permissions.
    InvalidEnclaveSection {
        /// Name of the malformed section.
        name: String,
    },
    /// A dedicated entry-enclave segment contains unrelated bytes or has the
    /// wrong memory permissions.
    InvalidEnclaveSegment {
        /// Name of the section whose segment is malformed.
        name: &'static str,
    },
    /// An unrecognised allocated `.kpti.*` section escaped the closed set.
    UnexpectedEnclaveSection {
        /// Name the linker emitted.
        name: String,
    },
    /// A segment claims more file bytes than it will occupy in memory, or
    /// bytes that run off the end of the address space.
    ImpossibleSegment {
        /// Where the segment begins.
        vaddr: u64,
        /// The file size it claims.
        file_size: u64,
        /// The memory size it claims.
        memory_size: u64,
    },
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ImpossibleSegment {
                vaddr,
                file_size,
                memory_size,
            } => write!(
                f,
                "the segment at {vaddr:#018x} claims {file_size:#x} file bytes for {memory_size:#x} memory bytes"
            ),
            Self::EntryOutsideHigherHalf { entry } => write!(
                f,
                "entry point {entry:#018x} is below the higher half at {HIGHER_HALF:#018x}"
            ),
            Self::SectionNotLoaded { name, addr } => write!(
                f,
                "{name} at {addr:#018x} is in no loadable segment — name it in linker.ld \
                 rather than leaving it an orphan"
            ),
            Self::WritableAndExecutable { vaddr } => {
                write!(
                    f,
                    "the segment at {vaddr:#018x} is both writable and executable"
                )
            }
            Self::NobitsInFile { name, wasted } => write!(
                f,
                "{name} occupies no memory of its own but is costing {wasted} bytes of image — \
                 something loadable was placed after it"
            ),
            Self::MissingSection { name } => {
                write!(f, "{name} is missing from the loaded image")
            }
            Self::InvalidEnclaveSection { name } => {
                write!(f, "{name} is not one complete page-exact enclave range")
            }
            Self::InvalidEnclaveSegment { name } => {
                write!(f, "the load segment for {name} is not dedicated and exact")
            }
            Self::UnexpectedEnclaveSection { name } => {
                write!(
                    f,
                    "unrecognised entry-enclave section {name} escaped the closed set"
                )
            }
        }
    }
}

impl std::error::Error for ImageError {}

/// Sections the image cannot boot without.
///
/// `.requests` is where `wost::entry!` puts the Limine request block; without
/// it the bootloader answers nothing and the kernel faults reading a response
/// that was never filled in.
const REQUIRED: &[&str] = &[
    ".text",
    ".requests",
    ".kpti.text",
    ".kpti.rodata",
    ".kpti.tables",
    ".kpti.data",
];

const KPTI_SECTIONS: &[(&str, bool, bool)] = &[
    (".kpti.text", false, true),
    (".kpti.rodata", false, false),
    (".kpti.tables", true, false),
    (".kpti.data", true, false),
];

impl Image {
    /// Check every property `kernel/linker.ld` is written to produce.
    ///
    /// Reports all of them rather than the first: a linker-script change that
    /// breaks one usually breaks several, and finding out one at a time is how
    /// a script gets fixed into a different wrong shape.
    pub fn verify(&self) -> Vec<ImageError> {
        let mut faults = Vec::new();

        if self.entry < HIGHER_HALF {
            faults.push(ImageError::EntryOutsideHigherHalf { entry: self.entry });
        }

        for segment in self.segments.iter().filter(|s| s.is_loadable()) {
            if segment.is_writable_and_executable() {
                faults.push(ImageError::WritableAndExecutable {
                    vaddr: segment.vaddr,
                });
            }
            // Reported, not saturated: `covers` checks only `vaddr +
            // memory_size`, so an oversized `p_filesz` reached an unchecked add (D-105).
            if segment.file_size > segment.memory_size
                || segment.vaddr.checked_add(segment.file_size).is_none()
            {
                faults.push(ImageError::ImpossibleSegment {
                    vaddr: segment.vaddr,
                    file_size: segment.file_size,
                    memory_size: segment.memory_size,
                });
            }
        }

        for section in self.sections.iter().filter(|s| s.is_allocated()) {
            let Some(segment) = self.segment_for(section) else {
                faults.push(ImageError::SectionNotLoaded {
                    name: section.name.clone(),
                    addr: section.addr,
                });
                continue;
            };
            // Any file-backed byte past the section's start puts it in the
            // file; the unchecked add panicked on `p_filesz = u64::MAX` (D-105).
            let file_end = segment.vaddr.saturating_add(segment.file_size);
            if section.is_nobits() && section.addr < file_end {
                faults.push(ImageError::NobitsInFile {
                    name: section.name.clone(),
                    wasted: (file_end - section.addr).min(section.size),
                });
            }
        }

        for name in REQUIRED {
            let present = self
                .sections
                .iter()
                .any(|section| section.name == *name && section.is_allocated());
            if !present {
                faults.push(ImageError::MissingSection { name });
            }
        }

        self.verify_entry_enclave(&mut faults);

        faults
    }

    fn verify_entry_enclave(&self, faults: &mut Vec<ImageError>) {
        for section in self
            .sections
            .iter()
            .filter(|section| section.is_allocated() && section.name.starts_with(".kpti."))
        {
            if !KPTI_SECTIONS
                .iter()
                .any(|(name, _, _)| section.name == *name)
            {
                faults.push(ImageError::UnexpectedEnclaveSection {
                    name: section.name.clone(),
                });
            }
        }

        for &(name, writable, executable) in KPTI_SECTIONS {
            let Some(section) = self
                .sections
                .iter()
                .find(|section| section.name == name && section.is_allocated())
            else {
                continue;
            };
            let page_exact = section.addr.is_multiple_of(PAGE_BYTES)
                && section.size != 0
                && section.size.is_multiple_of(PAGE_BYTES);
            let permissions = (section.flags & SHF_WRITE != 0) == writable
                && (section.flags & SHF_EXECUTE != 0) == executable;
            let exclusive = !self.sections.iter().any(|other| {
                other.is_allocated()
                    && other.name != section.name
                    && ranges_overlap(section.addr, section.size, other.addr, other.size)
            });
            if !page_exact || !permissions || !exclusive {
                faults.push(ImageError::InvalidEnclaveSection {
                    name: section.name.clone(),
                });
            }
        }

        let named = |name: &'static str| {
            self.sections
                .iter()
                .find(|section| section.name == name && section.is_allocated())
        };
        for (name, expected_flags) in [
            (".kpti.text", elf64::Segment::READ | elf64::Segment::EXECUTE),
            (".kpti.rodata", elf64::Segment::READ),
        ] {
            let Some(section) = named(name) else {
                continue;
            };
            let mut intersecting = self.segments.iter().copied().filter(|segment| {
                segment.is_loadable() && segment.overlaps(section.addr, section.size)
            });
            let Some(segment) = intersecting.next() else {
                continue;
            };
            if !segment.covers(section.addr, section.size)
                || segment.vaddr != section.addr
                || segment.memory_size != section.size
                || segment.flags != expected_flags
                || intersecting.next().is_some()
            {
                faults.push(ImageError::InvalidEnclaveSegment { name });
            }
        }

        if let (Some(tables), Some(data)) = (named(".kpti.tables"), named(".kpti.data")) {
            let covering = |section: &Section| {
                self.segments
                    .iter()
                    .copied()
                    .filter(|segment| {
                        segment.is_loadable() && segment.overlaps(section.addr, section.size)
                    })
                    .collect::<Vec<_>>()
            };
            let table_segments = covering(tables);
            let data_segments = covering(data);
            let segment = table_segments.first().copied();
            let same = table_segments.len() == 1
                && data_segments.len() == 1
                && segment == data_segments.first().copied();
            let exact = segment.is_some_and(|segment| {
                tables.addr.checked_add(tables.size) == Some(data.addr)
                    && data.addr.checked_add(data.size)
                        == segment.vaddr.checked_add(segment.memory_size)
                    && segment.vaddr == tables.addr
                    && segment.flags == (elf64::Segment::READ | elf64::Segment::WRITE)
            });
            if !same || !exact {
                faults.push(ImageError::InvalidEnclaveSegment {
                    name: ".kpti.tables/.kpti.data",
                });
            }
        }
    }

    /// The loadable segment holding `section`, if any.
    fn segment_for(&self, section: &Section) -> Option<Segment> {
        self.segments
            .iter()
            .copied()
            .filter(|segment| segment.is_loadable())
            .find(|segment| segment.covers(section.addr, section.size))
    }
}

/// Parse the ELF64 headers of `bytes`.
///
/// The file header and the program headers come from `libs/elf64` — the crate
/// the kernel loads programs with — so the verifier and the loader agree about
/// the format by construction rather than by two transcriptions being kept in
/// step. D-105 recorded that duplication as owed and D-124 found what it costs:
/// a repair to one copy left the other two validations short (D-158).
///
/// The section-entry and string-table parser stays here. `elf64` exposes the
/// four file-header fields so this verifier does not privately transcribe the
/// header, but a loader never needs the host-only entry walk (D-213).
pub fn parse(bytes: &[u8]) -> Result<Image> {
    ensure!(
        u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= MAX_KERNEL_ELF_BYTES,
        "kernel ELF exceeds the {}-byte limit",
        MAX_KERNEL_ELF_BYTES
    );
    let image = elf64::Elf::parse(bytes).map_err(|error| anyhow::anyhow!("{error}"))?;
    let segments = image
        .segments()
        .map(|segment| Segment {
            kind: segment.kind().bits(),
            flags: segment.flags(),
            vaddr: segment.address(),
            file_size: segment.file_size(),
            memory_size: segment.memory_size(),
        })
        .collect();
    let sections = parse_sections(bytes, image.section_table())?;
    Ok(Image {
        entry: image.entry(),
        segments,
        sections,
    })
}

/// Where entry `index` of a header table starts, refusing rather than wrapping.
///
/// Every operand is a header field, so every one is the file's to choose. The
/// unchecked form panicked the build on `e_shoff = u64::MAX` — the verifier
/// stopping the build for the wrong reason, which is precisely what D-40 said
/// it must not do (D-105).
fn entry_at(table: usize, index: usize, stride: usize) -> Result<usize> {
    index
        .checked_mul(stride)
        .and_then(|skip| table.checked_add(skip))
        .context("the header table runs off the end of the address space")
}

/// A field `offset` bytes into a header entry, refusing rather than wrapping.
fn field(at: usize, offset: usize) -> Result<usize> {
    at.checked_add(offset)
        .context("a header field runs off the end of the address space")
}

fn parse_sections(bytes: &[u8], header: elf64::SectionTable) -> Result<Vec<Section>> {
    let table = usize::try_from(header.offset()).context("section table offset is too large")?;
    let stride = usize::from(header.entry_size());
    let count = usize::from(header.count());
    let names_index = usize::from(header.names_index());
    if count == 0 {
        return Ok(Vec::new());
    }
    ensure!(
        count <= MAX_SECTION_COUNT,
        "kernel ELF has {count} sections; at most {MAX_SECTION_COUNT} are admitted"
    );
    if stride != SECTION_HEADER_BYTES {
        bail!("section headers of {stride} bytes are not ELF64 section headers");
    }
    if names_index >= count {
        bail!("the section name table index is outside the section table");
    }
    let names_header = entry_at(table, names_index, stride)?;
    let names_at = usize::try_from(read_u64(bytes, field(names_header, 0x18)?)?)
        .context("section name table offset is too large")?;
    let names_len = usize::try_from(read_u64(bytes, field(names_header, 0x20)?)?)
        .context("section name table length is too large")?;
    let names_end = names_at
        .checked_add(names_len)
        .context("section name table extent overflowed")?;
    let names = bytes
        .get(names_at..names_end)
        .context("section name table runs past the end of the file")?;

    let mut total_name_bytes = 0usize;
    let mut sections = Vec::with_capacity(count);
    for index in 0..count {
        let at = entry_at(table, index, stride)?;
        let name = read_name(names, read_u32(bytes, at)? as usize)?;
        total_name_bytes = checked_name_budget(total_name_bytes, name.len())?;
        sections.push(Section {
            name: name.to_owned(),
            // Check each field offset directly instead of relying on an
            // earlier initializer to reject a large `at` (D-105, D-124).
            kind: read_u32(bytes, field(at, 0x04)?)?,
            flags: read_u64(bytes, field(at, 0x08)?)?,
            addr: read_u64(bytes, field(at, 0x10)?)?,
            size: read_u64(bytes, field(at, 0x20)?)?,
        });
    }
    Ok(sections)
}

fn checked_name_budget(total: usize, additional: usize) -> Result<usize> {
    let total = total
        .checked_add(additional)
        .context("aggregate section-name length overflowed")?;
    ensure!(
        total <= MAX_SECTION_NAME_BYTES,
        "kernel ELF section names exceed the {MAX_SECTION_NAME_BYTES}-byte aggregate limit"
    );
    Ok(total)
}

/// The NUL-terminated name at `at`.
fn read_name(bytes: &[u8], at: usize) -> Result<&str> {
    let rest = bytes
        .get(at..)
        .with_context(|| format!("section name at {at:#x} is past the end of the file"))?;
    let end = rest
        .iter()
        .position(|&byte| byte == 0)
        .with_context(|| format!("unterminated section name at {at:#x}"))?;
    core::str::from_utf8(&rest[..end]).context("a section name is not valid UTF-8")
}

/// Read `N` bytes at `at`, or say where the file ran out.
fn read_bytes<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N]> {
    let slice = bytes
        .get(at..)
        .and_then(|rest| rest.get(..N))
        .with_context(|| format!("{N} bytes at {at:#x} are past the end of the file"))?;
    let mut out = [0u8; N];
    out.copy_from_slice(slice);
    Ok(out)
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32> {
    read_bytes(bytes, at).map(u32::from_le_bytes)
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64> {
    read_bytes(bytes, at).map(u64::from_le_bytes)
}

/// Read one linked kernel without allowing the file to grow past its bound
/// between metadata inspection and capture.
pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let length = file
        .metadata()
        .with_context(|| format!("measuring {}", path.display()))?
        .len();
    ensure!(
        length <= MAX_KERNEL_ELF_BYTES,
        "{} is {length} bytes; kernel ELF limit is {MAX_KERNEL_ELF_BYTES}",
        path.display()
    );
    let mut bytes = Vec::with_capacity(usize::try_from(length).context("kernel ELF is too large")?);
    file.take(MAX_KERNEL_ELF_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    ensure!(
        u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= MAX_KERNEL_ELF_BYTES,
        "{} grew beyond the {MAX_KERNEL_ELF_BYTES}-byte kernel ELF limit while being read",
        path.display()
    );
    Ok(bytes)
}

/// Read the ELF at `path` and require every property of a bootable image.
pub fn verify_file(path: &Path) -> Result<()> {
    let bytes = read_file(path)?;
    let image = parse(&bytes).with_context(|| format!("parsing {}", path.display()))?;

    let faults = image.verify();
    if faults.is_empty() {
        return Ok(());
    }
    let report = faults
        .iter()
        .map(|fault| format!("\n  {fault}"))
        .collect::<String>();
    bail!("{} is not a bootable image:{report}", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.text`-shaped section: allocated, with contents.
    const SHT_PROGBITS: u32 = 1;

    /// `SHF_ALLOC | SHF_EXECINSTR`.
    const TEXT_FLAGS: u64 = SHF_ALLOC | 1 << 2;

    fn segment(vaddr: u64, size: u64, flags: u32) -> Segment {
        Segment {
            kind: elf64::SegmentKind::Load.bits(),
            flags,
            vaddr,
            file_size: size,
            memory_size: size,
        }
    }

    fn section(name: &str, addr: u64, size: u64, kind: u32, flags: u64) -> Section {
        Section {
            name: name.to_string(),
            kind,
            flags,
            addr,
            size,
        }
    }

    /// The shape `kernel/linker.ld` is written to produce: exact enclave
    /// segments followed by the three ordinary protection classes, with
    /// `.bss` last and costing no file space.
    fn good_image() -> Image {
        let data = Segment {
            kind: elf64::SegmentKind::Load.bits(),
            flags: elf64::Segment::WRITE | (1 << 2),
            vaddr: HIGHER_HALF + 0x1f000,
            file_size: 0x100,
            memory_size: 0x8000,
        };
        Image {
            entry: HIGHER_HALF + 0x5bb0,
            segments: vec![
                segment(
                    HIGHER_HALF,
                    0x2000,
                    elf64::Segment::READ | elf64::Segment::EXECUTE,
                ),
                segment(
                    HIGHER_HALF + 0x2000,
                    0x6000,
                    elf64::Segment::READ | elf64::Segment::EXECUTE,
                ),
                segment(HIGHER_HALF + 0x8000, 0x1000, elf64::Segment::READ),
                segment(HIGHER_HALF + 0x9000, 0x9000, elf64::Segment::READ),
                segment(
                    HIGHER_HALF + 0x12000,
                    0xd000,
                    elf64::Segment::READ | elf64::Segment::WRITE,
                ),
                data,
            ],
            sections: vec![
                section(
                    ".text",
                    HIGHER_HALF + 0x2000,
                    0x5800,
                    SHT_PROGBITS,
                    TEXT_FLAGS,
                ),
                section(
                    ".rodata",
                    HIGHER_HALF + 0x9000,
                    0x8000,
                    SHT_PROGBITS,
                    SHF_ALLOC,
                ),
                section(
                    ".requests",
                    HIGHER_HALF + 0x1f000,
                    0x60,
                    SHT_PROGBITS,
                    SHF_ALLOC | 1,
                ),
                section(
                    ".bss",
                    HIGHER_HALF + 0x1f100,
                    0x7f00,
                    SHT_NOBITS,
                    SHF_ALLOC | 1,
                ),
                section(".symtab", 0, 0x800, 2, 0),
                section(".kpti.text", HIGHER_HALF, 0x2000, SHT_PROGBITS, TEXT_FLAGS),
                section(
                    ".kpti.rodata",
                    HIGHER_HALF + 0x8000,
                    0x1000,
                    SHT_PROGBITS,
                    SHF_ALLOC,
                ),
                section(
                    ".kpti.tables",
                    HIGHER_HALF + 0x12000,
                    0x2000,
                    SHT_PROGBITS,
                    SHF_ALLOC | SHF_WRITE,
                ),
                section(
                    ".kpti.data",
                    HIGHER_HALF + 0x14000,
                    0xb000,
                    SHT_PROGBITS,
                    SHF_ALLOC | SHF_WRITE,
                ),
            ],
        }
    }

    #[test]
    fn the_shape_the_linker_script_produces_passes() {
        assert_eq!(good_image().verify(), []);
    }

    #[test]
    fn every_entry_enclave_section_is_required_page_exact_and_permission_exact() {
        for (name, _, _) in KPTI_SECTIONS {
            let mut missing = good_image();
            missing.sections.retain(|section| section.name != *name);
            assert!(
                missing
                    .verify()
                    .contains(&ImageError::MissingSection { name }),
                "missing {name} was accepted"
            );
        }

        let mut misaligned = good_image();
        let text = misaligned
            .sections
            .iter_mut()
            .find(|section| section.name == ".kpti.text")
            .expect("entry text");
        text.addr += 1;
        assert!(
            misaligned
                .verify()
                .contains(&ImageError::InvalidEnclaveSection {
                    name: ".kpti.text".to_string(),
                })
        );

        let mut writable_text = good_image();
        let text = writable_text
            .sections
            .iter_mut()
            .find(|section| section.name == ".kpti.text")
            .expect("entry text");
        text.flags |= SHF_WRITE;
        assert!(
            writable_text
                .verify()
                .contains(&ImageError::InvalidEnclaveSection {
                    name: ".kpti.text".to_string(),
                })
        );
    }

    #[test]
    fn an_entry_segment_cannot_absorb_an_unlisted_page_or_section() {
        let mut enlarged = good_image();
        enlarged.segments[0].memory_size += PAGE_BYTES;
        assert!(
            enlarged
                .verify()
                .contains(&ImageError::InvalidEnclaveSegment { name: ".kpti.text" })
        );

        let mut unknown = good_image();
        unknown.sections.push(section(
            ".kpti.secret",
            HIGHER_HALF + 0x1f080,
            8,
            SHT_PROGBITS,
            SHF_ALLOC | SHF_WRITE,
        ));
        assert!(
            unknown
                .verify()
                .contains(&ImageError::UnexpectedEnclaveSection {
                    name: ".kpti.secret".to_string(),
                })
        );

        let mut overlapping = good_image();
        overlapping.segments.push(segment(
            HIGHER_HALF,
            0x2000,
            elf64::Segment::READ | elf64::Segment::EXECUTE,
        ));
        assert!(
            overlapping
                .verify()
                .contains(&ImageError::InvalidEnclaveSegment { name: ".kpti.text" })
        );

        let mut partially_overlapping = good_image();
        partially_overlapping.segments.push(segment(
            HIGHER_HALF + PAGE_BYTES,
            PAGE_BYTES,
            elf64::Segment::READ | elf64::Segment::EXECUTE,
        ));
        assert!(
            partially_overlapping
                .verify()
                .contains(&ImageError::InvalidEnclaveSegment { name: ".kpti.text" })
        );

        let mut section_overlap = good_image();
        section_overlap.sections.push(section(
            ".ordinary.secret",
            HIGHER_HALF + PAGE_BYTES,
            8,
            SHT_PROGBITS,
            SHF_ALLOC,
        ));
        assert!(
            section_overlap
                .verify()
                .contains(&ImageError::InvalidEnclaveSection {
                    name: ".kpti.text".to_string(),
                })
        );
    }

    #[test]
    fn kpti_entry_transition_switches_roots_in_the_exact_enclave_order() {
        let entry = include_str!("../../wost/src/arch/x86_64/interrupt.rs");
        let kpti = include_str!("../../wost/src/arch/x86_64/kpti.rs");
        let cpu = include_str!("../../wost/src/arch/x86_64/cpu.rs");
        let task = include_str!("../../wost/src/arch/x86_64/task.rs");
        let linker = include_str!("../../kernel/linker.ld");

        let section = entry
            .find(".pushsection .kpti.text")
            .expect("entry section");
        let stubs = entry[section..]
            .find("wost_interrupt_stubs")
            .expect("all-vector table");
        let dispatch = entry[section..]
            .find("call {dispatch}")
            .expect("common Rust call");
        let end = entry[section..]
            .find(".popsection")
            .expect("entry section end");
        assert!(stubs < dispatch && dispatch < end);
        assert!(kpti.contains("const ENTRY_ROOT_SWITCH_IMPLEMENTED: bool = true;"));
        for offset in [
            "offset_of!(RoutingPage, state) == 0",
            "offset_of!(RoutingPage, full_root) == 8",
            "offset_of!(RoutingPage, stripped_root) == 16",
            "offset_of!(RoutingPage, task_stack_top) == 24",
            "offset_of!(RoutingPage, transition_stack_top) == 32",
        ] {
            assert!(kpti.contains(offset), "route ABI omitted {offset}");
        }
        let entry_load = entry
            .find("wost_kpti_entry_load_full_root:")
            .expect("full-root entry label");
        let rust_call = entry
            .find("wost_kpti_entry_call_rust:")
            .expect("Rust dispatch label");
        let last_secret = entry
            .find("wost_kpti_exit_last_secret_access:")
            .expect("last-secret exit label");
        let stripped_load = entry
            .find("wost_kpti_exit_load_stripped_root:")
            .expect("stripped-root exit label");
        let user_iret = entry.find("wost_kpti_exit_iret:").expect("user iret label");
        assert!(entry_load < rust_call);
        assert!(
            rust_call < last_secret && last_secret < stripped_load && stripped_load < user_iret
        );
        let entry_to_rust = &entry[entry_load..rust_call];
        assert!(entry_to_rust.contains("mov cr3, r11"));
        let after_stripped = &entry[stripped_load..user_iret];
        assert!(after_stripped.contains("mov cr3, r10"));
        assert!(!after_stripped.contains("call {dispatch}"));
        assert!(!after_stripped.contains("BUFFER_CLEARS"));
        assert_eq!(entry.matches("mov cr3, r10").count(), 1);
        assert_eq!(entry.matches("mov cr3, r11").count(), 2);
        assert!(!kpti.contains("mov cr3"));
        assert_eq!(cpu.matches("mov cr3").count(), 1);

        let nested_window = entry
            .find("A kernel-origin frame already inside either enclave stack")
            .expect("nested transition proof");
        assert!(nested_window < rust_call);
        let nested_proof = &entry[nested_window..rust_call];
        for required in [
            "{route_transition_stack}",
            "{route_interrupt_stack}",
            "jae wost_kpti_stop",
        ] {
            assert!(
                nested_proof.contains(required),
                "nested proof omitted {required}"
            );
        }
        assert_eq!(
            entry
                .matches("cmp qword ptr [rip + WOST_KPTI_ROUTE + {route_state}], 2")
                .count(),
            2
        );

        let begin = task
            .find("kpti::begin_route_update();")
            .expect("route begin");
        let publish = task[begin..]
            .find("kpti::publish_route")
            .map(|offset| begin + offset)
            .expect("route fields");
        let stacks = task[publish..]
            .find("kpti::install_transition_rsp0()")
            .map(|offset| publish + offset)
            .expect("entry stacks");
        let activate = task[stacks..]
            .find("cpu::set_page_table_base(wanted)")
            .map(|offset| stacks + offset)
            .expect("full-root activation");
        let finish = task[activate..]
            .find("kpti::finish_route_update(roots)")
            .map(|offset| activate + offset)
            .expect("route publication");
        assert!(begin < publish && publish < stacks && stacks < activate && activate < finish);

        let finish_function = kpti
            .split_once("pub(super) fn finish_route_update")
            .expect("route finisher")
            .1
            .split_once("/// Whether the enclave")
            .expect("route finisher end")
            .0;
        let active_root = finish_function
            .find("cpu::page_table_base() != roots.full_root()")
            .expect("active full-root check");
        let published = finish_function
            .find(".store(ROUTE_PUBLISHED, Ordering::Release)")
            .expect("published store");
        assert!(active_root < published);
        assert!(kpti.contains("assert!(route_roots_are_valid(0, PAGE_BYTES));"));

        for wildcard in [
            ".kpti.text.*",
            ".kpti.rodata.*",
            ".kpti.tables.*",
            ".kpti.state.*",
        ] {
            assert!(
                !linker.contains(wildcard),
                "linker admits wildcard {wildcard}"
            );
        }
        for admitted in [
            ".kpti.text",
            ".kpti.rodata.geometry",
            ".kpti.rodata.verw_selector",
            ".kpti.tables.idt",
            ".kpti.tables.gdt",
            ".kpti.tables.tss",
            ".kpti.state.route",
            ".kpti.state.transition_stack",
            ".kpti.state.ist_stack",
        ] {
            assert!(linker.contains(admitted), "linker omitted {admitted}");
        }

        for marker in [
            "__kpti_text_start",
            "__kpti_text_end",
            "__kpti_tables_start",
            "__kpti_tables_end",
            "__kpti_transition_guard_start",
            "__kpti_transition_stack_start",
            "__kpti_ist_guard_start",
            "__kpti_ist_stack_start",
        ] {
            assert!(linker.contains(marker), "linker omitted {marker}");
        }
    }

    #[test]
    fn kpti_entry_loads_full_root_before_rust() {
        let entry = include_str!("../../wost/src/arch/x86_64/interrupt.rs");
        let load = entry
            .find("wost_kpti_entry_load_full_root:")
            .expect("full-root label");
        let rust = entry
            .find("wost_kpti_entry_call_rust:")
            .expect("Rust label");
        let transition = &entry[load..rust];
        let root = transition
            .find("mov r11, qword ptr [rip + WOST_KPTI_ROUTE + {route_full}]")
            .expect("full-root read");
        let cr3 = transition.find("mov cr3, r11").expect("full-root load");
        let ordinary_stack = transition
            .find("{route_task_stack}")
            .expect("ordinary stack destination");
        let dispatch = entry[rust..]
            .find("call {dispatch}")
            .expect("Rust dispatch");
        assert!(root < cr3 && cr3 < ordinary_stack);
        assert!(dispatch > 0);
    }

    #[test]
    fn kpti_exit_loads_stripped_root_after_scrub_and_last_secret_access() {
        let entry = include_str!("../../wost/src/arch/x86_64/interrupt.rs");
        let return_tail = entry
            .find("wost_interrupt_return:")
            .expect("user-return tail");
        let scrub = entry[return_tail..]
            .find("mov rdi, qword ptr [rip + WOST_KPTI_ROUTE + {route_transition_stack}]")
            .map(|offset| return_tail + offset)
            .expect("transition scrub");
        let last_secret = entry
            .find("wost_kpti_exit_last_secret_access:")
            .expect("last secret boundary");
        let stripped = entry
            .find("wost_kpti_exit_load_stripped_root:")
            .expect("stripped-root boundary");
        let user_iret = entry
            .find("wost_kpti_exit_iret:")
            .expect("user return boundary");
        assert!(scrub < last_secret && last_secret < stripped && stripped < user_iret);

        let tail = &entry[stripped..user_iret];
        let instructions = tail
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix('"')
                    .and_then(|line| line.strip_suffix("\","))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            instructions,
            [
                ".if {record_transition_evidence}",
                "lock inc qword ptr [rip + WOST_KPTI_ROUTE + {route_prepared_returns}]",
                ".endif",
                "mov cr3, r10",
                "pop r15",
                "pop r14",
                "pop r13",
                "pop r12",
                "pop r11",
                "pop r10",
                "pop r9",
                "pop r8",
                "pop rbp",
                "pop rdi",
                "pop rsi",
                "pop rdx",
                "pop rcx",
                "pop rbx",
                "pop rax",
                "cmp byte ptr [rip + {clear}], 0",
                "je 10f",
                "verw word ptr [rip + {verw_selector}]",
                "10:",
                ".globl wost_kpti_exit_iret",
            ]
        );
    }

    /// The defect that motivated this module: `.got` placed after `.bss` by
    /// orphan heuristics, which forces the whole of `.bss` into the file.
    #[test]
    fn an_orphan_after_bss_is_caught_by_the_file_space_it_costs() {
        let mut image = good_image();
        let data = image.segments.last_mut().expect("three segments");
        data.file_size = data.memory_size;
        image.sections.push(section(
            ".got",
            HIGHER_HALF + 0x28000,
            8,
            SHT_PROGBITS,
            SHF_ALLOC | 1,
        ));

        assert_eq!(
            image.verify(),
            [
                ImageError::NobitsInFile {
                    name: ".bss".to_string(),
                    wasted: 0x7f00,
                },
                ImageError::SectionNotLoaded {
                    name: ".got".to_string(),
                    addr: HIGHER_HALF + 0x28000,
                },
            ]
        );
    }

    /// The subtler shape of the same defect: file bytes reach only part-way
    /// into `.bss`, so `memsz > filesz` still holds and the whole-segment
    /// comparison would look fine.
    #[test]
    fn a_partially_file_backed_bss_is_caught() {
        let mut image = good_image();
        let data = image.segments.last_mut().expect("three segments");
        data.file_size = 0x200;

        assert_eq!(
            image.verify(),
            [ImageError::NobitsInFile {
                name: ".bss".to_string(),
                wasted: 0x100,
            }]
        );
    }

    #[test]
    fn a_section_stranded_outside_every_segment_is_caught() {
        let mut image = good_image();
        image
            .sections
            .push(section(".orphan", 0xdead_0000, 16, SHT_PROGBITS, SHF_ALLOC));

        assert_eq!(
            image.verify(),
            [ImageError::SectionNotLoaded {
                name: ".orphan".to_string(),
                addr: 0xdead_0000,
            }]
        );
    }

    /// A section running off the end of its segment is stranded too — the
    /// bound has to be the whole span, not just where it starts.
    #[test]
    fn a_section_overrunning_its_segment_is_not_covered_by_it() {
        let mut image = good_image();
        image.sections[0].size = 0x9000;

        assert_eq!(
            image.verify(),
            [
                ImageError::SectionNotLoaded {
                    name: ".text".to_string(),
                    addr: HIGHER_HALF + 0x2000,
                },
                ImageError::InvalidEnclaveSection {
                    name: ".kpti.rodata".to_string(),
                },
            ]
        );
    }

    /// W^X, which Limine enforces from these flags: nothing may be both.
    #[test]
    fn a_writable_executable_segment_is_caught() {
        let mut image = good_image();
        image.segments[1].flags |= elf64::Segment::WRITE;

        assert_eq!(
            image.verify(),
            [ImageError::WritableAndExecutable {
                vaddr: HIGHER_HALF + 0x2000,
            }]
        );
    }

    #[test]
    fn a_lower_half_entry_point_is_caught() {
        let mut image = good_image();
        image.entry = 0x1000;

        assert!(
            image
                .verify()
                .contains(&ImageError::EntryOutsideHigherHalf { entry: 0x1000 })
        );
    }

    #[test]
    fn a_missing_request_block_is_caught() {
        let mut image = good_image();
        image.sections.retain(|section| section.name != ".requests");

        assert_eq!(
            image.verify(),
            [ImageError::MissingSection { name: ".requests" }]
        );
    }

    /// A `.requests` section that exists but is not loaded is as useless as an
    /// absent one, and looks different in the section table.
    #[test]
    fn an_unallocated_request_block_does_not_count_as_present() {
        let mut image = good_image();
        for section in &mut image.sections {
            if section.name == ".requests" {
                section.flags = 0;
            }
        }

        assert!(
            image
                .verify()
                .contains(&ImageError::MissingSection { name: ".requests" })
        );
    }

    /// Non-allocated sections — symbols, strings, debug info — are not part of
    /// the loaded image and must not be judged as if they were.
    #[test]
    fn debug_sections_are_not_expected_to_be_loaded() {
        let mut image = good_image();
        image
            .sections
            .push(section(".debug_info", 0, 0x4000, SHT_PROGBITS, 0));

        assert_eq!(image.verify(), []);
    }

    #[test]
    fn every_fault_says_something_specific() {
        let mut image = good_image();
        image.entry = 0;
        image.segments[1].flags |= elf64::Segment::WRITE;
        image.sections.retain(|section| section.name != ".requests");

        let messages: Vec<String> = image.verify().iter().map(ToString::to_string).collect();
        assert_eq!(messages.len(), 3, "{messages:#?}");
        assert!(messages.iter().any(|m| m.contains("higher half")));
        assert!(
            messages
                .iter()
                .any(|m| m.contains("writable and executable"))
        );
        assert!(messages.iter().any(|m| m.contains(".requests")));
    }

    // --- the reader ---

    /// Assemble a minimal but genuine ELF64 file from `segments` and
    /// `sections`, so the parser is tested against bytes rather than against
    /// the struct it is supposed to produce.
    fn encode(image: &Image) -> Vec<u8> {
        const HEADER: usize = elf64::HEADER_BYTES;
        const PH_STRIDE: usize = elf64::PROGRAM_HEADER_BYTES;
        const SH_STRIDE: usize = SECTION_HEADER_BYTES;

        let mut names = vec![0u8];
        let mut name_offsets = Vec::new();
        for section in &image.sections {
            name_offsets.push(names.len() as u32);
            names.extend_from_slice(section.name.as_bytes());
            names.push(0);
        }
        // The name table is itself a section, appended last.
        let name_section = image.sections.len();
        name_offsets.push(names.len() as u32);
        names.extend_from_slice(b".shstrtab\0");

        let ph_table = HEADER;
        let sh_table = ph_table + image.segments.len() * PH_STRIDE;
        let names_at = sh_table + (image.sections.len() + 1) * SH_STRIDE;

        let mut out = vec![0u8; names_at + names.len()];
        out[..4].copy_from_slice(&elf64::MAGIC);
        out[4] = elf64::CLASS_64;
        out[5] = elf64::LITTLE_ENDIAN;
        out[6] = 1;
        out[0x10..0x12].copy_from_slice(&2u16.to_le_bytes());
        out[0x12..0x14].copy_from_slice(&elf64::MACHINE_X86_64.to_le_bytes());
        out[0x14..0x18].copy_from_slice(&1u32.to_le_bytes());
        out[0x18..0x20].copy_from_slice(&image.entry.to_le_bytes());
        out[0x20..0x28].copy_from_slice(&(ph_table as u64).to_le_bytes());
        out[0x28..0x30].copy_from_slice(&(sh_table as u64).to_le_bytes());
        out[0x34..0x36].copy_from_slice(&(HEADER as u16).to_le_bytes());
        out[0x36..0x38].copy_from_slice(&(PH_STRIDE as u16).to_le_bytes());
        out[0x38..0x3A].copy_from_slice(&(image.segments.len() as u16).to_le_bytes());
        out[0x3A..0x3C].copy_from_slice(&(SH_STRIDE as u16).to_le_bytes());
        out[0x3C..0x3E].copy_from_slice(&((image.sections.len() + 1) as u16).to_le_bytes());
        out[0x3E..0x40].copy_from_slice(&(name_section as u16).to_le_bytes());

        for (index, segment) in image.segments.iter().enumerate() {
            let at = ph_table + index * PH_STRIDE;
            out[at..at + 4].copy_from_slice(&segment.kind.to_le_bytes());
            out[at + 4..at + 8].copy_from_slice(&segment.flags.to_le_bytes());
            out[at + 0x10..at + 0x18].copy_from_slice(&segment.vaddr.to_le_bytes());
            out[at + 0x20..at + 0x28].copy_from_slice(&segment.file_size.to_le_bytes());
            out[at + 0x28..at + 0x30].copy_from_slice(&segment.memory_size.to_le_bytes());
        }

        let mut write_section =
            |index: usize, name: u32, kind: u32, flags: u64, addr: u64, size: u64| {
                let at = sh_table + index * SH_STRIDE;
                out[at..at + 4].copy_from_slice(&name.to_le_bytes());
                out[at + 4..at + 8].copy_from_slice(&kind.to_le_bytes());
                out[at + 8..at + 0x10].copy_from_slice(&flags.to_le_bytes());
                out[at + 0x10..at + 0x18].copy_from_slice(&addr.to_le_bytes());
                out[at + 0x20..at + 0x28].copy_from_slice(&size.to_le_bytes());
            };
        for (index, section) in image.sections.iter().enumerate() {
            write_section(
                index,
                name_offsets[index],
                section.kind,
                section.flags,
                section.addr,
                section.size,
            );
        }
        write_section(
            name_section,
            name_offsets[name_section],
            3,
            0,
            0,
            names.len() as u64,
        );
        // `sh_offset` of the name table, which the reader follows to the names.
        let at = sh_table + name_section * SH_STRIDE + 0x18;
        out[at..at + 8].copy_from_slice(&(names_at as u64).to_le_bytes());

        out[names_at..].copy_from_slice(&names);
        out
    }

    #[test]
    fn a_parsed_image_is_the_one_that_was_encoded() {
        let original = good_image();
        let parsed = parse(&encode(&original)).expect("parse");

        assert_eq!(parsed.entry, original.entry);
        assert_eq!(parsed.segments, original.segments);
        // The encoder appends its own name table, which the original has not.
        assert_eq!(
            &parsed.sections[..original.sections.len()],
            &original.sections[..]
        );
        assert_eq!(parsed.verify(), []);
    }

    #[test]
    fn a_file_that_is_not_an_elf_is_refused() {
        assert!(parse(b"").is_err());
        assert!(parse(b"MZ\x90\x00 a PE executable").is_err());
        // Full headers make rejection reach class/endianness; shorter fixtures
        // stop at truncation (D-105).
        let mut thirty_two = encode(&good_image());
        thirty_two[4] = 1;
        let message = parse(&thirty_two).expect_err("32-bit").to_string();
        assert!(message.contains("64-bit"), "said {message:?}");

        let mut big_endian = encode(&good_image());
        big_endian[5] = 2;
        let message = parse(&big_endian).expect_err("big-endian").to_string();
        assert!(message.contains("little-endian"), "said {message:?}");
    }

    /// A header that promises more tables than the file holds must be an
    /// error, not an index out of bounds.
    #[test]
    fn a_truncated_file_is_refused_rather_than_panicking() {
        let whole = encode(&good_image());

        for length in [0x40, 0x60, whole.len() / 2, whole.len() - 1] {
            assert!(parse(&whole[..length]).is_err(), "truncated to {length}");
        }
    }

    /// A header field near the top of the range is refused, not added to.
    ///
    /// Truncation shrinks the buffer and never produces a large offset, so the
    /// sweep above cannot reach the arithmetic. These inputs panicked the
    /// build — `xtask` builds in the dev profile, so the overflow checks are
    /// on, and `verify_file` runs after every kernel build (D-105).
    #[test]
    fn a_header_field_near_the_top_of_the_range_is_refused() {
        let mut section_table = encode(&good_image());
        // `e_shoff`, the section header table's file offset.
        section_table[0x28..0x30].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(parse(&section_table).is_err(), "e_shoff = u64::MAX");

        let mut program_table = encode(&good_image());
        // `e_phoff`, the program header table's file offset.
        program_table[0x20..0x28].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(parse(&program_table).is_err(), "e_phoff = u64::MAX");

        let mut names = encode(&good_image());
        // `e_shstrndx`, chosen so the name table's own entry is far out.
        names[0x3E..0x40].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(parse(&names).is_err(), "e_shstrndx = u16::MAX");
    }

    /// Keep the verifier's machine and header-shape refusals aligned with the
    /// loader because `verify_file` judges every kernel image (D-105, D-124).
    #[test]
    fn an_image_for_another_machine_or_of_another_shape_is_refused() {
        let mut aarch64 = encode(&good_image());
        // `e_machine`, at offset 18.
        aarch64[0x12..0x14].copy_from_slice(&183u16.to_le_bytes());
        let message = parse(&aarch64).expect_err("aarch64").to_string();
        assert!(message.contains("x86-64"), "said {message:?}");

        let mut wrong_stride = encode(&good_image());
        // `e_phentsize`, at offset 54.
        wrong_stride[0x36..0x38].copy_from_slice(&64u16.to_le_bytes());
        let message = parse(&wrong_stride).expect_err("stride").to_string();
        assert!(message.contains("program headers"), "said {message:?}");

        // Shared constants prevent reader drift; literal anchors still catch a
        // crate-wide typo that would move both readers together (D-158).
        assert_eq!(elf64::PROGRAM_HEADER_BYTES, 56);
        assert_eq!(elf64::MACHINE_X86_64, 62);
    }

    #[test]
    fn a_section_header_stride_is_exactly_elf64s() {
        for stride in [0u16, 1, 63, 65, u16::MAX] {
            let mut image = encode(&good_image());
            image[0x3A..0x3C].copy_from_slice(&stride.to_le_bytes());
            let message = parse(&image)
                .expect_err("non-ELF64 section stride")
                .to_string();
            assert!(
                message.contains("section headers") && message.contains(&stride.to_string()),
                "stride {stride} said {message:?}"
            );
        }
        assert!(parse(&encode(&good_image())).is_ok());
    }

    #[test]
    fn section_count_and_owned_name_bytes_are_bounded_before_amplification() {
        let mut too_many = encode(&good_image());
        too_many[0x3C..0x3E]
            .copy_from_slice(&(u16::try_from(MAX_SECTION_COUNT + 1).unwrap()).to_le_bytes());
        let message = parse(&too_many).expect_err("too many sections").to_string();
        assert!(message.contains("at most 4096"), "said {message:?}");

        assert_eq!(
            checked_name_budget(MAX_SECTION_NAME_BYTES - 1, 1).unwrap(),
            MAX_SECTION_NAME_BYTES
        );
        let message = checked_name_budget(MAX_SECTION_NAME_BYTES, 1)
            .expect_err("name amplification")
            .to_string();
        assert!(message.contains("aggregate limit"), "said {message:?}");

        // Exercise the parser seam, not only the budget helper: every header
        // names the same long string, so a parser that forgets to carry the
        // running total would clone more than the admitted aggregate.
        let repeated_name = "n".repeat(257);
        let mut repeated_image = good_image();
        repeated_image.sections = (0..MAX_SECTION_COUNT - 1)
            .map(|_| section(&repeated_name, 0, 0, SHT_PROGBITS, 0))
            .collect();
        let mut repeated = encode(&repeated_image);
        let mut table = [0u8; 8];
        table.copy_from_slice(&repeated[0x28..0x30]);
        let table = usize::try_from(u64::from_le_bytes(table)).unwrap();
        for index in 0..MAX_SECTION_COUNT - 1 {
            let at = table + index * SECTION_HEADER_BYTES;
            repeated[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        }
        let message = parse(&repeated)
            .expect_err("repeated parsed name amplification")
            .to_string();
        assert!(message.contains("aggregate limit"), "said {message:?}");

        let mut names_escape = encode(&good_image());
        let mut offset = [0u8; 8];
        offset.copy_from_slice(&names_escape[0x28..0x30]);
        let table = usize::try_from(u64::from_le_bytes(offset)).unwrap();
        let mut index = [0u8; 2];
        index.copy_from_slice(&names_escape[0x3E..0x40]);
        let names_header = table + usize::from(u16::from_le_bytes(index)) * SECTION_HEADER_BYTES;
        names_escape[names_header + 0x20..names_header + 0x28].copy_from_slice(&1u64.to_le_bytes());
        let message = parse(&names_escape)
            .expect_err("section name outside the declared string table")
            .to_string();
        assert!(message.contains("section name"), "said {message:?}");
    }

    #[test]
    fn a_kernel_file_over_the_shared_ceiling_is_refused_before_allocation() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "bedrock-elf-limit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .expect("exclusive sparse ELF fixture");
        file.set_len(MAX_KERNEL_ELF_BYTES + 1)
            .expect("sparse oversized ELF fixture");
        drop(file);
        let error = read_file(&path).expect_err("oversized kernel ELF");
        std::fs::remove_file(path).expect("remove sparse ELF fixture");
        let message = error.to_string();
        assert!(
            message.contains(&format!("is {} bytes", MAX_KERNEL_ELF_BYTES + 1))
                && message.contains("kernel ELF limit"),
            "said {message:?}"
        );
    }

    /// A segment whose file size runs off the end does not overflow the
    /// nobits comparison.
    ///
    /// `covers` checks `vaddr + memory_size`, so a huge `p_filesz` walks past
    /// it and reaches the addition in `verify` (D-105).
    #[test]
    fn a_segment_that_claims_the_whole_range_is_refused_rather_than_panicking() {
        let mut image = good_image();
        image.segments[0].file_size = u64::MAX;
        let faults = image.verify();
        assert!(!faults.is_empty(), "a file size of u64::MAX passed");
    }
}
