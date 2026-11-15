use super::*;

#[test]
fn a_version_two_root_pointer_names_its_xsdt() {
    let mut bytes = [0u8; Rsdp::V2_LEN];
    bytes[..8].copy_from_slice(b"RSD PTR ");
    bytes[15] = 2;
    bytes[20..24].copy_from_slice(&36u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&0xdead_0000u64.to_le_bytes());
    fix_checksum(&mut bytes[..Rsdp::V1_LEN], 9);
    fix_checksum(&mut bytes, 32);

    let rsdp = Rsdp::parse(&bytes).expect("well formed");
    assert_eq!(rsdp.xsdt(), Some(0xdead_0000));
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
