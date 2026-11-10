use super::*;

fn rsdp(revision: u8) -> Vec<u8> {
    let mut bytes = vec![0u8; Rsdp::V2_LEN];
    bytes[..8].copy_from_slice(&Rsdp::SIGNATURE);
    bytes[15] = revision;
    bytes[16..20].copy_from_slice(&0x1234u32.to_le_bytes());
    bytes[20..24].copy_from_slice(&(Rsdp::V2_LEN as u32).to_le_bytes());
    bytes[24..32].copy_from_slice(&0xdead_beefu64.to_le_bytes());
    fix_checksum(&mut bytes[..Rsdp::V1_LEN], 9);
    fix_checksum(&mut bytes, 32);
    bytes
}

#[test]
fn errors_name_the_refused_table_property() {
    let cases = [
        (
            AcpiError::TooShort {
                needed: 36,
                available: 20,
            },
            "table needs 36 bytes and 20 are readable",
        ),
        (
            AcpiError::BadSignature(*b"NOT ACPI"),
            "table signature is not the expected one",
        ),
        (
            AcpiError::BadChecksum,
            "table checksum does not sum to zero",
        ),
        (AcpiError::BadLength(12), "table length 12 is impossible"),
        (
            AcpiError::TooLong {
                declared: 4097,
                limit: 4096,
            },
            "table declares 4097 bytes, above the 4096-byte limit",
        ),
        (
            AcpiError::WrongTable {
                expected: *b"XSDT",
                found: *b"APIC",
            },
            "table signature [65, 80, 73, 67] does not match [88, 83, 68, 84]",
        ),
        (
            AcpiError::PartialEntry {
                width: 8,
                trailing: 3,
            },
            "table has 3 trailing bytes after 8-byte entries",
        ),
        (
            AcpiError::MalformedEntry {
                offset: 44,
                length: 12,
                available: 4,
            },
            "entry at byte 44 declares 12 bytes with 4 available",
        ),
        (
            AcpiError::UnsupportedRevision(7),
            "table revision 7 is not read by this kernel",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn a_root_table_above_the_cap_cannot_hide_a_tail_madt() {
    const LISTED: usize = MAX_TABLES + 1;

    let mut table = vec![0u8; Sdt::LEN + LISTED * 8];
    table[..4].copy_from_slice(&Sdt::XSDT);
    let length = u32::try_from(table.len()).expect("fits");
    table[4..8].copy_from_slice(&length.to_le_bytes());
    table[8] = 1;
    for index in 0..LISTED {
        let at = Sdt::LEN + index * 8;
        let address = if index == MAX_TABLES {
            0xfeed_cafe
        } else {
            0x1000u64 + index as u64
        };
        table[at..at + 8].copy_from_slice(&address.to_le_bytes());
    }
    let checksum = table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    table[9] = table[9].wrapping_sub(checksum);

    assert!(matches!(
        SdtPointers::xsdt(&table),
        Err(AcpiError::TooManyTables {
            listed: LISTED,
            limit: MAX_TABLES
        })
    ));
}

#[test]
fn a_root_table_at_the_cap_is_walked_whole() {
    const LISTED: usize = MAX_TABLES;

    let mut table = vec![0u8; Sdt::LEN + LISTED * 4];
    table[..4].copy_from_slice(&Sdt::RSDT);
    let length = u32::try_from(table.len()).expect("fits");
    table[4..8].copy_from_slice(&length.to_le_bytes());
    table[8] = 1;
    for index in 0..LISTED {
        let at = Sdt::LEN + index * 4;
        table[at..at + 4].copy_from_slice(&(0x2000u32 + index as u32).to_le_bytes());
    }
    let checksum = table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    table[9] = table[9].wrapping_sub(checksum);

    let walked: Vec<_> = SdtPointers::rsdt(&table)
        .expect("the exact pointer budget")
        .collect();
    assert_eq!(walked.len(), LISTED);
    assert_eq!(walked[0], 0x2000);
    assert_eq!(walked[LISTED - 1], 0x2000 + LISTED as u64 - 1);
}

#[test]
fn a_root_table_inside_the_cap_is_walked_whole() {
    const LISTED: usize = 3;

    let mut table = vec![0u8; Sdt::LEN + LISTED * 8];
    table[..4].copy_from_slice(&Sdt::XSDT);
    let length = u32::try_from(table.len()).expect("fits");
    table[4..8].copy_from_slice(&length.to_le_bytes());
    table[8] = 1;
    for index in 0..LISTED {
        let at = Sdt::LEN + index * 8;
        table[at..at + 8].copy_from_slice(&(0x2000u64 + index as u64).to_le_bytes());
    }
    let checksum = table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    table[9] = table[9].wrapping_sub(checksum);

    assert_eq!(
        SdtPointers::xsdt(&table)
            .expect("a valid xsdt")
            .collect::<Vec<_>>(),
        [0x2000, 0x2001, 0x2002]
    );
}

#[test]
fn a_well_formed_rsdp_yields_its_pointers() {
    let bytes = rsdp(2);
    let parsed = Rsdp::parse(&bytes).expect("well formed");

    assert_eq!(parsed.xsdt(), Some(0xdead_beef));
    assert_eq!(parsed.rsdt(), Some(0x1234));
}

#[test]
fn root_pointer_lengths_accept_their_exact_budget() {
    let bytes = rsdp(2);
    assert_eq!(Rsdp::parse_length(&bytes, Rsdp::V2_LEN), Ok(Rsdp::V2_LEN));
    assert!(matches!(
        Rsdp::parse_length(&bytes, Rsdp::V2_LEN - 1),
        Err(AcpiError::TooLong { .. })
    ));
}

#[test]
fn an_xsdt_requires_both_a_supporting_revision_and_a_nonzero_address() {
    assert_eq!(
        (Rsdp {
            revision: 2,
            rsdt: 0,
            xsdt: 0,
        })
        .xsdt(),
        None
    );
    assert_eq!(
        (Rsdp {
            revision: 0,
            rsdt: 0,
            xsdt: 0x1_0000_0000,
        })
        .xsdt(),
        None
    );
}

#[test]
fn a_revision_zero_rsdp_has_no_xsdt() {
    let bytes = rsdp(0);
    let parsed = Rsdp::parse(&bytes).expect("well formed");

    assert_eq!(parsed.xsdt(), None, "the field exists but must not be read");
    assert_eq!(parsed.rsdt(), Some(0x1234));
}

#[test]
fn a_bad_rsdp_signature_is_refused() {
    let mut bytes = rsdp(2);
    bytes[0] = b'X';
    assert!(matches!(
        Rsdp::parse(&bytes),
        Err(AcpiError::BadSignature(_))
    ));
}

#[test]
fn either_rsdp_checksum_failing_is_refused() {
    let mut first = rsdp(2);
    first[10] = first[10].wrapping_add(1);
    assert_eq!(Rsdp::parse(&first), Err(AcpiError::BadChecksum));

    let mut second = rsdp(2);
    second[30] = second[30].wrapping_add(1);
    assert_eq!(Rsdp::parse(&second), Err(AcpiError::BadChecksum));
}

#[test]
fn a_truncated_rsdp_is_refused_rather_than_read() {
    assert!(matches!(
        Rsdp::parse(&rsdp(2)[..10]),
        Err(AcpiError::TooShort { .. })
    ));
    assert!(matches!(
        Rsdp::parse(&rsdp(2)[..Rsdp::V1_LEN]),
        Err(AcpiError::TooShort { .. })
    ));
}

#[test]
fn bytes_that_are_not_an_rsdp_are_named_by_their_signature() {
    let mut bytes = rsdp(0);
    bytes[0] = b'X';
    bytes[15] = 7;

    assert!(matches!(
        Rsdp::parse(&bytes),
        Err(AcpiError::BadSignature(_))
    ));
}

#[test]
fn undefined_rsdp_revisions_are_refused() {
    assert_eq!(
        Rsdp::parse(&rsdp(1)),
        Err(AcpiError::UnsupportedRevision(1))
    );
    assert_eq!(
        Rsdp::parse(&rsdp(3)),
        Err(AcpiError::UnsupportedRevision(3))
    );
}

#[test]
fn the_extended_rsdp_checksum_covers_its_declared_length() {
    let mut bytes = rsdp(2);
    bytes.resize(40, 0);
    bytes[20..24].copy_from_slice(&40u32.to_le_bytes());
    fix_checksum(&mut bytes[..Rsdp::V1_LEN], 9);
    fix_checksum(&mut bytes, 32);
    assert!(Rsdp::parse(&bytes).is_ok());

    bytes[39] = 1;
    assert_eq!(Rsdp::parse(&bytes), Err(AcpiError::BadChecksum));
}

#[test]
fn an_extended_rsdp_cannot_be_shorter_than_its_fixed_fields() {
    let mut bytes = rsdp(2);
    bytes[20..24].copy_from_slice(&24u32.to_le_bytes());

    assert_eq!(Rsdp::parse(&bytes), Err(AcpiError::BadLength(24)));
}

#[test]
fn an_rsdp_length_cannot_exceed_the_readers_budget() {
    let mut bytes = rsdp(2);
    bytes[20..24].copy_from_slice(&u32::MAX.to_le_bytes());

    assert_eq!(
        Rsdp::parse_length(&bytes, 4096),
        Err(AcpiError::TooLong {
            declared: u32::MAX as usize,
            limit: 4096,
        })
    );
}

#[test]
fn a_table_header_is_validated_against_its_whole_body() {
    let bytes = table(&Sdt::MADT, &[1, 2, 3, 4]);
    let header = Sdt::parse(&bytes).expect("well formed");

    assert_eq!(header.signature(), Sdt::MADT);
    assert_eq!(header.length() as usize, Sdt::LEN + 4);
}

#[test]
fn a_length_can_be_read_before_the_table_is_validated() {
    let bytes = table(&Sdt::MADT, &[1, 2, 3, 4]);
    assert_eq!(Sdt::parse_length(&bytes, usize::MAX), Ok(Sdt::LEN + 4));
    assert_eq!(
        Sdt::parse_length(&bytes[..Sdt::LEN], usize::MAX),
        Ok(Sdt::LEN + 4)
    );

    assert!(matches!(
        Sdt::parse_length(&bytes[..Sdt::LEN - 1], usize::MAX),
        Err(AcpiError::TooShort { .. })
    ));
    let mut short = bytes;
    short[4..8].copy_from_slice(&8u32.to_le_bytes());
    assert_eq!(
        Sdt::parse_length(&short, usize::MAX),
        Err(AcpiError::BadLength(8))
    );
}

#[test]
fn a_header_only_table_and_its_exact_budget_are_valid_lengths() {
    let bytes = table(&Sdt::XSDT, &[]);
    assert_eq!(Sdt::parse_length(&bytes, Sdt::LEN), Ok(Sdt::LEN));
    assert_eq!(
        Sdt::parse(&bytes).map(|header| header.length()),
        Ok(Sdt::LEN as u32)
    );
    assert!(matches!(
        Sdt::parse_length(&bytes, Sdt::LEN - 1),
        Err(AcpiError::TooLong { .. })
    ));
}

#[test]
fn an_unvalidated_length_cannot_exceed_the_readers_budget() {
    let mut header = [0u8; Sdt::LEN];
    header[4..8].copy_from_slice(&u32::MAX.to_le_bytes());

    assert_eq!(
        Sdt::parse_length(&header, 1 << 20),
        Err(AcpiError::TooLong {
            declared: u32::MAX as usize,
            limit: 1 << 20,
        })
    );
}

#[test]
fn a_table_whose_body_is_missing_is_refused() {
    let bytes = table(&Sdt::MADT, &[1, 2, 3, 4]);
    assert!(matches!(
        Sdt::parse(&bytes[..Sdt::LEN + 2]),
        Err(AcpiError::TooShort { .. })
    ));
}

#[test]
fn a_table_shorter_than_its_own_header_is_refused() {
    let mut bytes = table(&Sdt::MADT, &[]);
    bytes[4..8].copy_from_slice(&8u32.to_le_bytes());
    assert_eq!(Sdt::parse(&bytes), Err(AcpiError::BadLength(8)));
}

#[test]
fn a_corrupt_table_body_fails_the_checksum() {
    let mut bytes = table(&Sdt::MADT, &[1, 2, 3, 4]);
    let last = bytes.len() - 1;
    bytes[last] = bytes[last].wrapping_add(1);
    assert_eq!(Sdt::parse(&bytes), Err(AcpiError::BadChecksum));
}

#[test]
fn an_xsdt_lists_its_tables() {
    let mut body = Vec::new();
    for address in [0x1000u64, 0x2000, 0x3000] {
        body.extend_from_slice(&address.to_le_bytes());
    }
    let bytes = table(&Sdt::XSDT, &body);
    let pointers: Vec<_> = SdtPointers::xsdt(&bytes).expect("validated xsdt").collect();
    assert_eq!(pointers, [0x1000, 0x2000, 0x3000]);
}

#[test]
fn an_xsdt_pointer_keeps_its_high_thirty_two_bits() {
    let address = 0x1234_5678_9abc_def0u64;
    let bytes = table(&Sdt::XSDT, &address.to_le_bytes());
    assert_eq!(
        SdtPointers::xsdt(&bytes)
            .expect("validated xsdt")
            .collect::<Vec<_>>(),
        [address]
    );
}

#[test]
fn an_rsdt_lists_its_tables_in_thirty_two_bits() {
    let mut body = Vec::new();
    for address in [0x1000u32, 0x2000] {
        body.extend_from_slice(&address.to_le_bytes());
    }
    let bytes = table(&Sdt::RSDT, &body);
    let pointers: Vec<_> = SdtPointers::rsdt(&bytes).expect("validated rsdt").collect();
    assert_eq!(pointers, [0x1000, 0x2000]);
}

#[test]
fn a_typed_pointer_view_refuses_the_wrong_table() {
    let bytes = table(&Sdt::MADT, &[0; 8]);

    assert!(matches!(
        SdtPointers::xsdt(&bytes),
        Err(AcpiError::WrongTable {
            expected: Sdt::XSDT,
            found: Sdt::MADT,
        })
    ));
}

#[test]
fn a_pointer_table_with_a_partial_tail_is_invalid() {
    let bytes = table(&Sdt::XSDT, &[0; 9]);

    assert_eq!(
        SdtPointers::xsdt(&bytes).map(|pointers| pointers.count()),
        Err(AcpiError::PartialEntry {
            width: 8,
            trailing: 1,
        })
    );
}

#[test]
fn a_pointer_list_stops_at_the_declared_length() {
    let mut bytes = table(&Sdt::XSDT, &0x1000u64.to_le_bytes());
    bytes.extend_from_slice(&0xbadu64.to_le_bytes());

    let pointers: Vec<_> = SdtPointers::xsdt(&bytes).expect("validated xsdt").collect();
    assert_eq!(
        pointers,
        [0x1000],
        "the appended entry is outside the table"
    );
}

// Field widths from ACPI 6.5 table 5.4, independent of the parser.
#[test]
fn the_system_description_header_is_thirty_six_bytes() {
    assert_eq!(Sdt::LEN, 36);
    assert_eq!(4 + 4 + 1 + 1 + 6 + 8 + 4 + 4 + 4, Sdt::LEN);
}
