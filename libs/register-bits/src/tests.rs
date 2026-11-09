use super::*;

#[test]
fn a_32_bit_read_ignores_the_reserved_high_half() {
    assert_eq!(low_u32(0x0000_0000_8000_0001), 0x8000_0001);
    assert_eq!(low_u32(0xFFFF_FFFF_0000_0000), 0);
    assert_eq!(low_u32(0xDEAD_BEEF_0000_0001), 1);
}

#[test]
fn a_register_value_splits_into_its_halves() {
    assert_eq!(u32_halves(0), (0, 0));
    assert_eq!(u32_halves(0xFFFF_FFFF_0000_0000), (0, u32::MAX));
    assert_eq!(
        u32_halves(0x1234_5678_9ABC_DEF0),
        (0x9ABC_DEF0, 0x1234_5678)
    );
    assert_eq!(u16_halves(0xFFFF_0000), (0, u16::MAX));
    assert_eq!(u16_halves(0x1234_8086), (0x8086, 0x1234));
    assert_eq!(usize_from_u64(0xFFFF_FFFF_FFFF_FFFF), usize::MAX);
}
