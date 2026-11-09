use super::*;

#[test]
fn a_decoded_width_reports_its_bits() {
    for bits in [32_u8, 36, 40, 46, 52] {
        let width = PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF, u32::from(bits)).unwrap();
        assert_eq!(width.bits(), bits);
    }
}

#[test]
fn every_reported_byte_is_either_supported_or_refused() {
    for bits in u8::MIN..=u8::MAX {
        let decoded = PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF, u32::from(bits));
        assert_eq!(decoded.is_some(), (32..=52).contains(&bits), "{bits}");
    }
}

#[test]
fn an_absent_leaf_never_borrows_an_unrelated_answer() {
    assert_eq!(
        PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF - 1, 52),
        None
    );
}

#[test]
fn each_supported_limit_is_exclusive() {
    for bits in [36, 40, 46, 52] {
        let width = PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF, bits).unwrap();
        let limit = 1_u64 << bits;
        assert!(width.contains(limit - 1, limit));
        assert!(!width.contains(limit - 1, limit + 1));
        assert!(!width.contains(limit, limit));
    }
}
