use super::*;

#[test]
fn a_32_bit_read_ignores_the_reserved_high_half() {
    assert_eq!(low_u32(0x0000_0000_8000_0001), 0x8000_0001);
    assert_eq!(low_u32(0xFFFF_FFFF_0000_0000), 0);
    assert_eq!(low_u32(0xDEAD_BEEF_0000_0001), 1);
}
