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
use std::path::Path;

use anyhow::{Context, Result, bail};

/// Section type: occupies address space but no file space.
const SHT_NOBITS: u32 = 8;

/// Section flag: this section is part of the loaded image.
const SHF_ALLOC: u64 = 1 << 1;

/// Where the higher half begins — the base `kernel/linker.ld` links at, and
/// the range the kernel code model can address.
const HIGHER_HALF: u64 = 0xffff_ffff_8000_0000;

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
        }
    }
}

impl std::error::Error for ImageError {}

/// Sections the image cannot boot without.
///
/// `.requests` is where `wost::entry!` puts the Limine request block; without
/// it the bootloader answers nothing and the kernel faults reading a response
/// that was never filled in.
const REQUIRED: &[&str] = &[".text", ".requests"];

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
            // Reported rather than saturated over: `covers` checks only
            // `vaddr + memory_size`, so a `p_filesz` past the end of the range
            // walked past every fault and reached an unchecked addition
            // downstream (D-105).
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
            // File-backed bytes reaching past the section's start put it in
            // the file, whether or not they cover the whole segment.
            // Both operands are the file's; the unchecked add panicked the
            // build on `p_filesz = u64::MAX` (D-105).
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

        faults
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
/// The section table stays here. `elf64` is a loader's crate and a loader
/// never reads one; nothing else in the tree wants it.
pub fn parse(bytes: &[u8]) -> Result<Image> {
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
    let sections = parse_sections(bytes)?;
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

fn parse_sections(bytes: &[u8]) -> Result<Vec<Section>> {
    let table = read_u64(bytes, 0x28)? as usize;
    let stride = read_u16(bytes, 0x3A)? as usize;
    let count = read_u16(bytes, 0x3C)? as usize;
    let names_index = read_u16(bytes, 0x3E)? as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    if names_index >= count {
        bail!("the section name table index is outside the section table");
    }
    let names = read_u64(bytes, field(entry_at(table, names_index, stride)?, 0x18)?)? as usize;

    (0..count)
        .map(|index| {
            let at = entry_at(table, index, stride)?;
            Ok(Section {
                name: read_name(bytes, field(names, read_u32(bytes, at)? as usize)?)?,
                // Through `field` like the segment walk, rather than raw
                // addition. It was unreachable — the `name` field initialises
                // first and bails on a large `at` — but that made the safety
                // argument rest on Rust's struct-field initialisation order,
                // which is not what D-105's fix was resting on (D-124).
                kind: read_u32(bytes, field(at, 0x04)?)?,
                flags: read_u64(bytes, field(at, 0x08)?)?,
                addr: read_u64(bytes, field(at, 0x10)?)?,
                size: read_u64(bytes, field(at, 0x20)?)?,
            })
        })
        .collect()
}

/// The NUL-terminated name at `at`.
fn read_name(bytes: &[u8], at: usize) -> Result<String> {
    let rest = bytes
        .get(at..)
        .with_context(|| format!("section name at {at:#x} is past the end of the file"))?;
    let end = rest
        .iter()
        .position(|&byte| byte == 0)
        .with_context(|| format!("unterminated section name at {at:#x}"))?;
    String::from_utf8(rest[..end].to_vec()).context("a section name is not valid UTF-8")
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

fn read_u16(bytes: &[u8], at: usize) -> Result<u16> {
    read_bytes(bytes, at).map(u16::from_le_bytes)
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32> {
    read_bytes(bytes, at).map(u32::from_le_bytes)
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64> {
    read_bytes(bytes, at).map(u64::from_le_bytes)
}

/// Read the ELF at `path` and require every property of a bootable image.
pub fn verify_file(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
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

    /// The shape `kernel/linker.ld` is written to produce: three segments,
    /// read-execute then read-only then read-write, with `.bss` last and
    /// costing no file space.
    fn good_image() -> Image {
        let data = Segment {
            kind: elf64::SegmentKind::Load.bits(),
            flags: elf64::Segment::WRITE | (1 << 2),
            vaddr: HIGHER_HALF + 0x12000,
            file_size: 0x100,
            memory_size: 0x8000,
        };
        Image {
            entry: HIGHER_HALF + 0x5bb0,
            segments: vec![
                segment(HIGHER_HALF, 0x8000, elf64::Segment::EXECUTE | (1 << 2)),
                segment(HIGHER_HALF + 0x8000, 0xa000, 1 << 2),
                data,
            ],
            sections: vec![
                section(".text", HIGHER_HALF, 0x7800, SHT_PROGBITS, TEXT_FLAGS),
                section(
                    ".rodata",
                    HIGHER_HALF + 0x8000,
                    0x9000,
                    SHT_PROGBITS,
                    SHF_ALLOC,
                ),
                section(
                    ".requests",
                    HIGHER_HALF + 0x12000,
                    0x60,
                    SHT_PROGBITS,
                    SHF_ALLOC | 1,
                ),
                section(
                    ".bss",
                    HIGHER_HALF + 0x12100,
                    0x7f00,
                    SHT_NOBITS,
                    SHF_ALLOC | 1,
                ),
                section(".symtab", 0, 0x800, 2, 0),
            ],
        }
    }

    #[test]
    fn the_shape_the_linker_script_produces_passes() {
        assert_eq!(good_image().verify(), []);
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
            HIGHER_HALF + 0x1a000,
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
                    addr: HIGHER_HALF + 0x1a000,
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
            [ImageError::SectionNotLoaded {
                name: ".text".to_string(),
                addr: HIGHER_HALF,
            }]
        );
    }

    /// W^X, which Limine enforces from these flags: nothing may be both.
    #[test]
    fn a_writable_executable_segment_is_caught() {
        let mut image = good_image();
        image.segments[0].flags |= elf64::Segment::WRITE;

        assert_eq!(
            image.verify(),
            [ImageError::WritableAndExecutable { vaddr: HIGHER_HALF }]
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
        image.segments[0].flags |= elf64::Segment::WRITE;
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
        const HEADER: usize = 0x40;
        const PH_STRIDE: usize = 0x38;
        const SH_STRIDE: usize = 0x40;

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
        out[0x12..0x14].copy_from_slice(&elf64::MACHINE_X86_64.to_le_bytes());
        out[0x18..0x20].copy_from_slice(&image.entry.to_le_bytes());
        out[0x20..0x28].copy_from_slice(&(ph_table as u64).to_le_bytes());
        out[0x28..0x30].copy_from_slice(&(sh_table as u64).to_le_bytes());
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
        // Full-length identifiers, because `elf64` takes the whole 64-byte
        // file header before it looks at the class or the endianness, and a
        // short slice is refused for being short instead. The six- and
        // eight-byte versions of these two lines said "32-bit" and
        // "big-endian" while testing neither, and deleting both checks left
        // them green (D-105).
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

    /// The two refusals `libs/elf64` has, which this reader had fallen behind
    /// on.
    ///
    /// The duplication D-105 recorded as owed is not merely still owed: a
    /// repair to one copy left the other two validations short, which is the
    /// duplication being a defect rather than a cost. `verify_file` runs after
    /// every kernel build, so an aarch64 ELF passing it would have shipped
    /// (D-124).
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

        // Both refusals are `elf64`'s now rather than transcriptions of it,
        // which is what makes them impossible to fall behind on (D-158). The
        // numbers are still asserted, because a crate-wide typo would move
        // both readers together and neither would notice.
        assert_eq!(elf64::PROGRAM_HEADER_BYTES, 56);
        assert_eq!(elf64::MACHINE_X86_64, 62);
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
