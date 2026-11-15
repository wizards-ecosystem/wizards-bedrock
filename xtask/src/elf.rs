use anyhow::{Context, Result, bail, ensure};

const SHT_NOBITS: u32 = 8;
const SHF_WRITE: u64 = 1;
const SHF_ALLOC: u64 = 1 << 1;
const SHF_EXECUTE: u64 = 1 << 2;
const PAGE_BYTES: u64 = 4096;
const SECTION_HEADER_BYTES: usize = 64;
const HIGHER_HALF: u64 = 0xffff_ffff_8000_0000;

// `.requests` holds the Limine request block; without it no response is filled in.
const REQUIRED: &[&str] = &[
    ".text",
    ".requests",
    ".kpti.text",
    ".kpti.rodata",
    ".kpti.tables",
    ".kpti.data",
];

const ENCLAVE_SECTIONS: &[(&str, Access)] = &[
    (".kpti.text", Access::Execute),
    (".kpti.rodata", Access::Read),
    (".kpti.tables", Access::Write),
    (".kpti.data", Access::Write),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Access {
    Read,
    Write,
    Execute,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Segment {
    flags: u32,
    vaddr: u64,
    file_size: u64,
    memory_size: u64,
}

impl Segment {
    fn covers(self, addr: u64, size: u64) -> bool {
        match (
            self.vaddr.checked_add(self.memory_size),
            addr.checked_add(size),
        ) {
            (Some(segment_end), Some(end)) => addr >= self.vaddr && end <= segment_end,
            _ => false,
        }
    }

    fn overlaps(self, addr: u64, size: u64) -> bool {
        ranges_overlap(self.vaddr, self.memory_size, addr, size)
    }

    fn has(self, flag: u32) -> bool {
        self.flags & flag != 0
    }
}

fn ranges_overlap(left: u64, left_size: u64, right: u64, right_size: u64) -> bool {
    if left_size == 0 || right_size == 0 {
        return false;
    }
    match (left.checked_add(left_size), right.checked_add(right_size)) {
        (Some(left_end), Some(right_end)) => left < right_end && right < left_end,
        _ => true,
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct Section {
    name: String,
    kind: u32,
    flags: u64,
    addr: u64,
    size: u64,
}

impl Section {
    fn access(&self) -> Access {
        if self.flags & SHF_EXECUTE != 0 {
            Access::Execute
        } else if self.flags & SHF_WRITE != 0 {
            Access::Write
        } else {
            Access::Read
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct Image {
    entry: u64,
    segments: Vec<Segment>,
    sections: Vec<Section>,
}

pub(crate) fn verify(bytes: &[u8]) -> Result<()> {
    let faults = parse(bytes)?.faults();
    if faults.is_empty() {
        return Ok(());
    }
    let report = faults
        .iter()
        .map(|fault| format!("\n  {fault}"))
        .collect::<String>();
    bail!("the linked kernel breaks its layout contract:{report}")
}

impl Image {
    fn faults(&self) -> Vec<String> {
        let mut faults = Vec::new();
        self.segment_faults(&mut faults);
        let entry_executable = self
            .segments
            .iter()
            .any(|segment| segment.has(elf64::Segment::EXECUTE) && segment.covers(self.entry, 1));
        if !entry_executable {
            faults.push(format!(
                "entry point {:#018x} lies in no executable load segment",
                self.entry
            ));
        }
        for section in self.allocated() {
            let Some(segment) = self
                .segments
                .iter()
                .find(|segment| segment.covers(section.addr, section.size))
            else {
                continue;
            };
            let file_end = segment.vaddr.saturating_add(segment.file_size);
            if section.kind == SHT_NOBITS && section.addr < file_end {
                faults.push(format!(
                    "{} occupies no memory of its own but costs {} bytes of image: something \
                     loadable was placed after it",
                    section.name,
                    (file_end - section.addr).min(section.size)
                ));
            }
        }
        for name in REQUIRED {
            if self.section(name).is_none() {
                faults.push(format!("{name} is missing from the loaded image"));
            }
        }
        self.enclave_faults(&mut faults);
        faults
    }

    fn segment_faults(&self, faults: &mut Vec<String>) {
        for (index, segment) in self.segments.iter().enumerate() {
            let vaddr = segment.vaddr;
            if segment.has(elf64::Segment::WRITE) && segment.has(elf64::Segment::EXECUTE) {
                faults.push(format!(
                    "the segment at {vaddr:#018x} is writable and executable"
                ));
            }
            if segment.file_size > segment.memory_size
                || vaddr.checked_add(segment.memory_size).is_none()
            {
                faults.push(format!(
                    "the segment at {vaddr:#018x} claims {:#x} file bytes for {:#x} memory bytes",
                    segment.file_size, segment.memory_size
                ));
            }
            if vaddr < HIGHER_HALF {
                faults.push(format!(
                    "the segment at {vaddr:#018x} is below the higher half at {HIGHER_HALF:#018x}"
                ));
            }
            if !vaddr.is_multiple_of(PAGE_BYTES) {
                faults.push(format!("the segment at {vaddr:#018x} is not page-aligned"));
            }
            for other in &self.segments[index + 1..] {
                if segment.overlaps(other.vaddr, other.memory_size) {
                    faults.push(format!(
                        "the segments at {vaddr:#018x} and {:#018x} overlap",
                        other.vaddr
                    ));
                }
            }
        }
    }

    fn enclave_faults(&self, faults: &mut Vec<String>) {
        for section in self.allocated() {
            let known = ENCLAVE_SECTIONS
                .iter()
                .any(|(name, _)| section.name == *name);
            if section.name.starts_with(".kpti.") && !known {
                faults.push(format!(
                    "unrecognised entry-enclave section {}",
                    section.name
                ));
            }
        }
        for &(name, access) in ENCLAVE_SECTIONS {
            let Some(section) = self.section(name) else {
                continue;
            };
            let page_exact = section.addr.is_multiple_of(PAGE_BYTES)
                && section.size != 0
                && section.size.is_multiple_of(PAGE_BYTES);
            let exclusive = !self.allocated().any(|other| {
                other.name != section.name
                    && ranges_overlap(section.addr, section.size, other.addr, other.size)
            });
            if !page_exact || section.access() != access || !exclusive {
                faults.push(format!(
                    "{name} is not one complete page-exact enclave range"
                ));
            }
        }
        for (name, flags) in [
            (".kpti.text", elf64::Segment::READ | elf64::Segment::EXECUTE),
            (".kpti.rodata", elf64::Segment::READ),
        ] {
            let Some(section) = self.section(name) else {
                continue;
            };
            let exact = match self.segments_overlapping(section).as_slice() {
                [segment] => {
                    segment.vaddr == section.addr
                        && segment.memory_size == section.size
                        && segment.flags == flags
                }
                _ => false,
            };
            if !exact {
                faults.push(format!(
                    "the load segment for {name} is not dedicated and exact"
                ));
            }
        }
        let (Some(tables), Some(data)) = (self.section(".kpti.tables"), self.section(".kpti.data"))
        else {
            return;
        };
        let table_segments = self.segments_overlapping(tables);
        let exact = table_segments == self.segments_overlapping(data)
            && match table_segments.as_slice() {
                [segment] => {
                    segment.vaddr == tables.addr
                        && tables.addr.checked_add(tables.size) == Some(data.addr)
                        && data.addr.checked_add(data.size)
                            == segment.vaddr.checked_add(segment.memory_size)
                        && segment.flags == (elf64::Segment::READ | elf64::Segment::WRITE)
                }
                _ => false,
            };
        if !exact {
            faults.push(
                "the load segment for .kpti.tables and .kpti.data is not dedicated and exact"
                    .to_string(),
            );
        }
    }

    fn allocated(&self) -> impl Iterator<Item = &Section> {
        self.sections
            .iter()
            .filter(|section| section.flags & SHF_ALLOC != 0)
    }

    fn section(&self, name: &str) -> Option<&Section> {
        self.allocated().find(|section| section.name == name)
    }

    fn segments_overlapping(&self, section: &Section) -> Vec<Segment> {
        self.segments
            .iter()
            .copied()
            .filter(|segment| segment.overlaps(section.addr, section.size))
            .collect()
    }
}

fn parse(bytes: &[u8]) -> Result<Image> {
    let elf = elf64::Elf::parse(bytes).map_err(|error| anyhow::anyhow!("{error}"))?;
    let segments = elf
        .segments()
        .filter(|segment| segment.kind() == elf64::SegmentKind::Load)
        .map(|segment| Segment {
            flags: segment.flags(),
            vaddr: segment.address(),
            file_size: segment.file_size(),
            memory_size: segment.memory_size(),
        })
        .collect();
    Ok(Image {
        entry: elf.entry(),
        segments,
        sections: parse_sections(bytes, elf.section_table())?,
    })
}

fn parse_sections(bytes: &[u8], table: elf64::SectionTable) -> Result<Vec<Section>> {
    let count = usize::from(table.count());
    if count == 0 {
        return Ok(Vec::new());
    }
    ensure!(
        usize::from(table.entry_size()) == SECTION_HEADER_BYTES,
        "section headers are not ELF64 section headers"
    );
    let offset = usize::try_from(table.offset()).context("section table offset is too large")?;
    let header = |index: usize| {
        index
            .checked_mul(SECTION_HEADER_BYTES)
            .and_then(|skip| skip.checked_add(offset))
            .and_then(|start| bytes.get(start..)?.get(..SECTION_HEADER_BYTES))
            .context("a section header runs past the end of the file")
    };
    let names = header(usize::from(table.names_index()))?;
    let names_at = usize::try_from(u64_at(names, 0x18)?).context("name table is too far")?;
    let names_len = usize::try_from(u64_at(names, 0x20)?).context("name table is too long")?;
    let names = bytes
        .get(names_at..)
        .and_then(|rest| rest.get(..names_len))
        .context("the section name table runs past the end of the file")?;
    (0..count)
        .map(|index| {
            let header = header(index)?;
            let name_at = usize::try_from(u32_at(header, 0)?).context("name is too far")?;
            let name = names
                .get(name_at..)
                .and_then(|rest| rest.split(|&byte| byte == 0).next())
                .context("a section name is past the end of the name table")?;
            Ok(Section {
                name: String::from_utf8_lossy(name).into_owned(),
                kind: u32_at(header, 0x04)?,
                flags: u64_at(header, 0x08)?,
                addr: u64_at(header, 0x10)?,
                size: u64_at(header, 0x20)?,
            })
        })
        .collect()
}

fn u32_at(header: &[u8], at: usize) -> Result<u32> {
    header
        .get(at..at + 4)
        .and_then(|field| field.try_into().ok())
        .map(u32::from_le_bytes)
        .context("a section header field is truncated")
}

fn u64_at(header: &[u8], at: usize) -> Result<u64> {
    header
        .get(at..at + 8)
        .and_then(|field| field.try_into().ok())
        .map(u64::from_le_bytes)
        .context("a section header field is truncated")
}
