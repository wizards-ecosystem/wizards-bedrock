use super::*;

/// Every `p_type` survives a decode and an encode unchanged.
///
/// Exhaustive over the named range and its edges rather than a sample:
/// the whole risk in a decode table is one arm returning a neighbour's
/// value, and a sample that skips the neighbour cannot see it.
#[test]
fn every_segment_type_round_trips_through_the_decode_table() {
    for bits in (0..24).chain([u32::MAX, u32::MAX - 1, 0x6474_E551, 0x7000_0000]) {
        assert_eq!(
            SegmentKind::from_bits(bits).bits(),
            bits,
            "p_type {bits:#x} did not survive the table"
        );
    }

    // And no two named kinds encode alike, which the loop above cannot
    // see because it never holds two answers at once.
    let named = [
        SegmentKind::Null,
        SegmentKind::Load,
        SegmentKind::Dynamic,
        SegmentKind::Interpreter,
        SegmentKind::Note,
        SegmentKind::ProgramHeaders,
        SegmentKind::ThreadLocal,
    ];
    for (index, kind) in named.iter().enumerate() {
        for earlier in &named[..index] {
            assert_ne!(
                kind.bits(),
                earlier.bits(),
                "{kind:?} and {earlier:?} encode to the same p_type"
            );
        }
    }
}

/// A well-formed image with the given program headers.
fn image(segments: &[(u32, u32, u64, u64, u64, u64)]) -> Vec<u8> {
    let mut bytes = vec![0u8; HEADER_BYTES + segments.len() * PROGRAM_HEADER_BYTES];
    bytes[0..4].copy_from_slice(&MAGIC);
    bytes[4] = CLASS_64;
    bytes[5] = LITTLE_ENDIAN;
    bytes[6] = CURRENT_VERSION as u8;
    bytes[16..18].copy_from_slice(&EXECUTABLE.to_le_bytes());
    bytes[18..20].copy_from_slice(&MACHINE_X86_64.to_le_bytes());
    bytes[20..24].copy_from_slice(&CURRENT_VERSION.to_le_bytes());
    bytes[24..32].copy_from_slice(&0x40_1000u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&(HEADER_BYTES as u64).to_le_bytes());
    bytes[52..54].copy_from_slice(&(HEADER_BYTES as u16).to_le_bytes());
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

    let segment = core::iter::Iterator::next(&mut elf.segments()).expect("one segment");
    assert_eq!(segment.kind(), SegmentKind::Load, "p_type is at offset 0");
    assert_eq!(segment.address(), 0x40_0000, "p_vaddr is at offset 16");
    assert_eq!(segment.file_size(), 0x100, "p_filesz is at offset 32");
    assert_eq!(segment.memory_size(), 0x200, "p_memsz is at offset 40");
    assert_eq!(segment.alignment(), 0x1000, "p_align is at offset 48");
}

#[test]
fn diagnostic_messages_keep_their_reason_and_values() {
    let cases = [
        (
            ElfError::Truncated {
                wanted: 64,
                available: 17,
            },
            "wanted 64 bytes and there are 17",
        ),
        (ElfError::NotAnElf, "does not begin with the elf magic"),
        (ElfError::NotSixtyFourBit(1), "elf class 1 is not 64-bit"),
        (
            ElfError::NotLittleEndian(2),
            "elf data encoding 2 is not little-endian",
        ),
        (
            ElfError::WrongIdentificationVersion(0),
            "elf identification version 0 is not current",
        ),
        (
            ElfError::WrongType(4),
            "elf object type 4 is not executable",
        ),
        (
            ElfError::WrongVersion(0),
            "elf file version 0 is not current",
        ),
        (ElfError::WrongMachine(183), "elf machine 183 is not x86-64"),
        (
            ElfError::WrongHeaderSize(52),
            "elf header is 52 bytes, expected 64",
        ),
        (
            ElfError::WrongProgramHeaderSize(32),
            "program headers of 32 bytes are not this format's",
        ),
        (
            ElfError::ProgramHeadersOutOfRange {
                offset: 0x1234,
                count: 7,
            },
            "7 program headers at 0x1234 run past the image",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn segment_metadata_and_permission_boundaries_are_exact() {
    let flags = Segment::WRITE | 0x8000_0000;
    let offset = 0x1122_3344_5566_7788;
    let bytes = image(&[
        (1, flags, offset, 0x1000, 17, 17),
        (1, Segment::READ, 3, 0x2000, 1, 2),
    ]);
    let elf = Elf::parse(&bytes).expect("well formed");
    assert_eq!(elf.segment_count(), 2);

    let segments: Vec<_> = elf.segments().collect();
    assert_eq!(segments[0].flags(), flags);
    assert_eq!(segments[0].offset(), offset);
    assert!(!segments[0].is_readable(), "write is not read permission");
    assert!(segments[1].is_readable());
    assert!(
        !segments[0].is_oversized(),
        "equal file and memory sizes fit exactly"
    );
}

#[test]
fn the_section_table_fields_are_where_the_file_header_puts_them() {
    let mut bytes = image(&[]);
    bytes[40..48].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    bytes[58..60].copy_from_slice(&0x1234u16.to_le_bytes());
    bytes[60..62].copy_from_slice(&0x2345u16.to_le_bytes());
    bytes[62..64].copy_from_slice(&0x3456u16.to_le_bytes());

    let table = Elf::parse(&bytes).expect("well formed").section_table();
    assert_eq!(table.offset(), 0x1122_3344_5566_7788);
    assert_eq!(table.entry_size(), 0x1234);
    assert_eq!(table.count(), 0x2345);
    assert_eq!(table.names_index(), 0x3456);
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

#[test]
fn every_required_header_identity_field_is_validated() {
    let mut identification = image(&[]);
    identification[6] = 0;
    assert_eq!(
        Elf::parse(&identification).err(),
        Some(ElfError::WrongIdentificationVersion(0))
    );

    let mut object_type = image(&[]);
    object_type[16..18].copy_from_slice(&4u16.to_le_bytes());
    assert_eq!(Elf::parse(&object_type).err(), Some(ElfError::WrongType(4)));

    let mut version = image(&[]);
    version[20..24].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(Elf::parse(&version).err(), Some(ElfError::WrongVersion(0)));

    let mut header_size = image(&[]);
    header_size[52..54].copy_from_slice(&52u16.to_le_bytes());
    assert_eq!(
        Elf::parse(&header_size).err(),
        Some(ElfError::WrongHeaderSize(52))
    );
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

#[test]
fn segment_alignment_is_power_of_two_and_congruent() {
    let mut bytes = image(&[(1, Segment::READ, 0, 0x4000, 0, 1)]);
    let field = HEADER_BYTES + 48;
    for alignment in [0u64, 1, 2, 0x1000] {
        bytes[field..field + 8].copy_from_slice(&alignment.to_le_bytes());
        let segment =
            core::iter::Iterator::next(&mut Elf::parse(&bytes).expect("well formed").segments())
                .expect("one segment");
        assert!(segment.has_valid_alignment(), "alignment {alignment}");
    }

    bytes[field..field + 8].copy_from_slice(&3u64.to_le_bytes());
    let non_power =
        core::iter::Iterator::next(&mut Elf::parse(&bytes).expect("well formed").segments())
            .expect("one segment");
    assert!(!non_power.has_valid_alignment());

    bytes[field..field + 8].copy_from_slice(&0x1000u64.to_le_bytes());
    let offset = HEADER_BYTES + 8;
    bytes[offset..offset + 8].copy_from_slice(&1u64.to_le_bytes());
    let incongruent =
        core::iter::Iterator::next(&mut Elf::parse(&bytes).expect("well formed").segments())
            .expect("one segment");
    assert!(!incongruent.has_valid_alignment());

    let address = HEADER_BYTES + 16;
    bytes[address..address + 8].copy_from_slice(&0x4001u64.to_le_bytes());
    let congruent =
        core::iter::Iterator::next(&mut Elf::parse(&bytes).expect("well formed").segments())
            .expect("one segment");
    assert!(congruent.has_valid_alignment());
}

/// An unknown segment type keeps its number rather than becoming nothing.
#[test]
fn an_unknown_segment_type_keeps_its_number() {
    let bytes = image(&[(0x6474_e551, 6, 0, 0, 0, 0)]);
    let segment =
        core::iter::Iterator::next(&mut Elf::parse(&bytes).expect("well formed").segments())
            .expect("one segment");
    assert_eq!(segment.kind(), SegmentKind::Other(0x6474_e551));
}

#[test]
fn an_elf_for_another_architecture_is_refused() {
    // Well-formed by every other rule and full of instructions this CPU
    // would fault on; nothing read `EM_X86_64` back before this (D-105).
    let mut aarch64 = image(&[]);
    aarch64[18..20].copy_from_slice(&183u16.to_le_bytes());
    assert_eq!(
        Elf::parse(&aarch64).err(),
        Some(ElfError::WrongMachine(183))
    );

    let mut i386 = image(&[]);
    i386[18..20].copy_from_slice(&3u16.to_le_bytes());
    assert_eq!(Elf::parse(&i386).err(), Some(ElfError::WrongMachine(3)));

    // And the fixture itself says x86-64.
    assert!(Elf::parse(&image(&[])).is_ok());
}

/// The two structure sizes, against the format rather than each other.
///
/// The fixture builds its headers *from* these constants, so every test
/// using it stays self-consistent when they move together. Literal format
/// values keep this host check independent; `xtask` separately validates
/// every linked kernel immediately after building it (D-105).
#[test]
fn the_structure_sizes_are_the_format_s() {
    // ELF64 header: 16 ident + 2 + 2 + 4 + 8 entry + 8 phoff + 8 shoff
    // + 4 flags + 2 + 2 + 2 + 2 + 2 + 2 = 64.
    assert_eq!(HEADER_BYTES, 64);
    // ELF64 program header: 4 type + 4 flags + 8 offset + 8 vaddr
    // + 8 paddr + 8 filesz + 8 memsz + 8 align = 56.
    assert_eq!(PROGRAM_HEADER_BYTES, 56);
    assert_eq!(MACHINE_X86_64, 62);
}
