//! The linked kernel read back and checked against what `kernel/linker.ld` is written to produce.

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

const SHT_NOBITS: u32 = 8;

const SHF_ALLOC: u64 = 1 << 1;
const SHF_WRITE: u64 = 1;
const SHF_EXECUTE: u64 = 1 << 2;

const PAGE_BYTES: u64 = 4096;

const SECTION_HEADER_BYTES: usize = 64;

/// Largest linked kernel artifact the host verifier and image builder admit.
pub(crate) const MAX_KERNEL_ELF_BYTES: u64 = 64 * 1024 * 1024;
/// A kernel needs tens of sections; this leaves generous diagnostic headroom.
const MAX_SECTION_COUNT: usize = 4096;
/// Aggregate owned section-name bytes, independent of repeated name offsets.
const MAX_SECTION_NAME_BYTES: usize = 1024 * 1024;

/// The base `kernel/linker.ld` links at, and the range the kernel code model can address.
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Segment {
    /// `p_type`.
    pub kind: u32,
    /// `p_flags`.
    pub flags: u32,
    /// `p_vaddr`.
    pub vaddr: u64,
    /// `p_filesz`.
    pub file_size: u64,
    /// `p_memsz`.
    pub memory_size: u64,
}

impl Segment {
    pub const fn is_loadable(self) -> bool {
        self.kind == elf64::SegmentKind::Load.bits()
    }

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

    pub const fn is_writable_and_executable(self) -> bool {
        self.flags & elf64::Segment::WRITE != 0 && self.flags & elf64::Segment::EXECUTE != 0
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Section {
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
    pub const fn is_allocated(&self) -> bool {
        self.flags & SHF_ALLOC != 0
    }

    pub const fn is_nobits(&self) -> bool {
        self.kind == SHT_NOBITS
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    pub entry: u64,
    /// Every program header, in order.
    pub segments: Vec<Segment>,
    /// Every section header, in order.
    pub sections: Vec<Section>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ImageError {
    EntryOutsideHigherHalf {
        entry: u64,
    },
    /// An allocated section no loadable segment wholly covers, as an orphan placed badly by the
    /// heuristic is.
    SectionNotLoaded {
        name: String,
        addr: u64,
    },
    WritableAndExecutable {
        vaddr: u64,
    },
    /// A no-file-space section with file bytes in it, as one followed by something loadable is.
    NobitsInFile {
        name: String,
        wasted: u64,
    },
    /// A section the boot protocol needs is absent or not loaded.
    MissingSection {
        name: &'static str,
    },
    /// An entry-enclave section that is not page-exact or has the wrong permissions.
    InvalidEnclaveSection {
        name: String,
    },
    /// An entry-enclave segment holding unrelated bytes or with the wrong permissions.
    InvalidEnclaveSegment {
        name: &'static str,
    },
    /// An allocated `.kpti.*` section outside the closed set.
    UnexpectedEnclaveSection {
        name: String,
    },
    /// More file bytes than memory bytes, or bytes past the end of the address space.
    ImpossibleSegment {
        vaddr: u64,
        file_size: u64,
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

/// Sections the image cannot boot without; `.requests` holds `wost::entry!`'s Limine request
/// block, and without it no response is filled in.
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
    /// Every fault, not the first: a linker-script change that breaks one usually breaks several.
    pub fn verify(&self) -> Vec<ImageError> {
        let mut faults = Vec::new();

        if self.entry < HIGHER_HALF {
            faults.push(ImageError::EntryOutsideHigherHalf { entry: self.entry });
        }

        for segment in self.segments.iter().filter(|segment| segment.is_loadable()) {
            if segment.is_writable_and_executable() {
                faults.push(ImageError::WritableAndExecutable {
                    vaddr: segment.vaddr,
                });
            }
            // Reported, not saturated: `covers` checks only `vaddr + memory_size`, not `p_filesz`.
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

        for section in self
            .sections
            .iter()
            .filter(|section| section.is_allocated())
        {
            let Some(segment) = self.segment_for(section) else {
                faults.push(ImageError::SectionNotLoaded {
                    name: section.name.clone(),
                    addr: section.addr,
                });
                continue;
            };
            // Any file-backed byte past the section's start puts it in the file.
            // Saturating, since `p_filesz = u64::MAX` must not panic the verifier.
            let file_end = segment.vaddr.saturating_add(segment.file_size);
            if section.is_nobits() && section.addr < file_end {
                faults.push(ImageError::NobitsInFile {
                    name: section.name.clone(),
                    wasted: (file_end - section.addr).min(section.size),
                });
            }
        }

        for name in REQUIRED {
            if self.allocated_section(name).is_none() {
                faults.push(ImageError::MissingSection { name });
            }
        }

        self.verify_entry_enclave(&mut faults);

        faults
    }

    fn verify_entry_enclave(&self, faults: &mut Vec<ImageError>) {
        self.verify_enclave_closed_set(faults);
        self.verify_enclave_sections(faults);
        self.verify_enclave_code_segments(faults);
        self.verify_enclave_data_segment(faults);
    }

    fn verify_enclave_closed_set(&self, faults: &mut Vec<ImageError>) {
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
    }

    fn verify_enclave_sections(&self, faults: &mut Vec<ImageError>) {
        for &(name, writable, executable) in KPTI_SECTIONS {
            let Some(section) = self.allocated_section(name) else {
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
    }

    fn verify_enclave_code_segments(&self, faults: &mut Vec<ImageError>) {
        for (name, expected_flags) in [
            (".kpti.text", elf64::Segment::READ | elf64::Segment::EXECUTE),
            (".kpti.rodata", elf64::Segment::READ),
        ] {
            let Some(section) = self.allocated_section(name) else {
                continue;
            };
            let mut intersecting = self.loaded_segments_overlapping(section);
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
    }

    fn verify_enclave_data_segment(&self, faults: &mut Vec<ImageError>) {
        let (Some(tables), Some(data)) = (
            self.allocated_section(".kpti.tables"),
            self.allocated_section(".kpti.data"),
        ) else {
            return;
        };
        let table_segments = self.loaded_segments_overlapping(tables).collect::<Vec<_>>();
        let data_segments = self.loaded_segments_overlapping(data).collect::<Vec<_>>();
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

    fn allocated_section(&self, name: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|section| section.name == name && section.is_allocated())
    }

    fn loaded_segments_overlapping(&self, section: &Section) -> impl Iterator<Item = Segment> {
        let (addr, size) = (section.addr, section.size);
        self.segments
            .iter()
            .copied()
            .filter(move |segment| segment.is_loadable() && segment.overlaps(addr, size))
    }

    fn segment_for(&self, section: &Section) -> Option<Segment> {
        self.segments
            .iter()
            .copied()
            .filter(|segment| segment.is_loadable())
            .find(|segment| segment.covers(section.addr, section.size))
    }
}

/// The bytes are hostile: a malformed file is an error, never a panic or out-of-bounds read.
/// File and program headers come from `libs/elf64`, the crate the kernel loads programs with,
/// so the verifier and the loader agree.
/// The section-table walk stays here because a loader never needs it.
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

/// Every operand is a header field the file chooses, so overflow is refused rather than wrapped.
fn entry_at(table: usize, index: usize, stride: usize) -> Result<usize> {
    index
        .checked_mul(stride)
        .and_then(|skip| table.checked_add(skip))
        .context("the header table runs off the end of the address space")
}

/// A field `offset` bytes into a header entry, refused rather than wrapped.
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
            // Each field offset is checked itself, never left to an earlier read to refuse.
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

fn read_bytes<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N]> {
    let slice = bytes
        .get(at..)
        .and_then(|rest| rest.get(..N))
        .with_context(|| format!("{N} bytes at {at:#x} are past the end of the file"))?;
    let mut value = [0u8; N];
    value.copy_from_slice(slice);
    Ok(value)
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32> {
    read_bytes(bytes, at).map(u32::from_le_bytes)
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64> {
    read_bytes(bytes, at).map(u64::from_le_bytes)
}

/// The kernel bytes, bounded even if the file grows between measurement and read.
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
mod tests;
