use super::*;
use crate::test_support::function_after;

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

/// The shape `kernel/linker.ld` produces: exact enclave segments, three protection classes,
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
    let entry = &[
        include_str!("../../../wost/src/arch/x86_64/interrupt.rs"),
        include_str!("../../../wost/src/arch/x86_64/interrupt/selftest.rs"),
    ]
    .concat();
    let kpti = &[
        include_str!("../../../wost/src/arch/x86_64/kpti.rs"),
        include_str!("../../../wost/src/arch/x86_64/kpti/selftest.rs"),
    ]
    .concat();
    let cpu = &[
        include_str!("../../../wost/src/arch/x86_64/cpu.rs"),
        include_str!("../../../wost/src/arch/x86_64/cpu/selftest.rs"),
    ]
    .concat();
    let task = include_str!("../../../wost/src/arch/x86_64/task.rs");
    let linker = include_str!("../../../kernel/linker.ld");

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
    assert!(rust_call < last_secret && last_secret < stripped_load && stripped_load < user_iret);
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

    let finish_function =
        function_after(kpti, "pub(super) fn finish_route_update").expect("route finisher");
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
    let entry = include_str!("../../../wost/src/arch/x86_64/interrupt.rs");
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
    let entry = include_str!("../../../wost/src/arch/x86_64/interrupt.rs");
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

/// The motivating defect: an orphan `.got` after `.bss` forces all of `.bss` into the file.
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

/// File bytes reach part-way into `.bss`, so `memsz > filesz` holds and a segment check passes.
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

/// The bound is the section's whole span, not just where it starts.
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

/// Limine enforces W^X from these flags.
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
    assert!(
        messages
            .iter()
            .any(|message| message.contains("higher half"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("writable and executable"))
    );
    assert!(messages.iter().any(|message| message.contains(".requests")));
}

/// Genuine ELF64 bytes, so the parser is tested against bytes rather than the struct it produces.
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
    // Full headers let refusal reach class and endianness; shorter fixtures stop at truncation.
    let mut thirty_two = encode(&good_image());
    thirty_two[4] = 1;
    let message = parse(&thirty_two).expect_err("32-bit").to_string();
    assert!(message.contains("64-bit"), "said {message:?}");

    let mut big_endian = encode(&good_image());
    big_endian[5] = 2;
    let message = parse(&big_endian).expect_err("big-endian").to_string();
    assert!(message.contains("little-endian"), "said {message:?}");
}

#[test]
fn a_truncated_file_is_refused_rather_than_panicking() {
    let whole = encode(&good_image());

    for length in [0x40, 0x60, whole.len() / 2, whole.len() - 1] {
        assert!(parse(&whole[..length]).is_err(), "truncated to {length}");
    }
}

/// Truncation never yields a large offset, so only these inputs reach the header arithmetic.
/// `xtask` builds with overflow checks on, and `verify_file` runs after every kernel build.
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

/// The verifier's machine and header-shape refusals must stay aligned with the loader's.
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

    // Literal anchors catch a crate-wide typo that would move both readers together.
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

    // Every header names one long string, so a parser that drops the running total
    // would clone more than the admitted aggregate.
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
    let (path, file) = crate::test_support::reserve_temp_file("bedrock-elf-limit");
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

/// `covers` checks `vaddr + memory_size`, so a huge `p_filesz` reaches the addition in `verify`.
#[test]
fn a_segment_that_claims_the_whole_range_is_refused_rather_than_panicking() {
    let mut image = good_image();
    image.segments[0].file_size = u64::MAX;
    let faults = image.verify();
    assert!(!faults.is_empty(), "a file size of u64::MAX passed");
}
