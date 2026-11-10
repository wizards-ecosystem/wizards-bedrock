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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, PartialEq, Eq, Debug)]
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
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    /// The entry point address.
    pub entry: u64,
    /// Every program header, in order.
    pub segments: Vec<Segment>,
    /// Every section header, in order.
    pub sections: Vec<Section>,
}

/// Why an image is not the one the linker script describes.
#[derive(Clone, PartialEq, Eq, Debug)]
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

        for segment in self.segments.iter().filter(|segment| segment.is_loadable()) {
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
mod tests;
