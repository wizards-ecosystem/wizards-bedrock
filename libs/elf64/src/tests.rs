use super::*;

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
fn a_table_that_runs_past_the_image_is_refused() {
    let mut bytes = image(&[(1, 5, 0, 0x40_0000, 0, 0)]);

    bytes[56..58].copy_from_slice(&64u16.to_le_bytes());
    assert!(matches!(
        Elf::parse(&bytes),
        Err(ElfError::ProgramHeadersOutOfRange { .. })
    ));

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
