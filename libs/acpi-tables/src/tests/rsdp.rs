use super::*;

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
