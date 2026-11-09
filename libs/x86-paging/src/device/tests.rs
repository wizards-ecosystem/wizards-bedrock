use x86_cpuid::physical::ADDRESS_SIZE_LEAF;

use super::*;

fn width(bits: u32) -> PhysicalAddressWidth {
    PhysicalAddressWidth::from_leaf(ADDRESS_SIZE_LEAF, bits).unwrap()
}

#[test]
fn a_checked_range_reports_its_first_register_byte() {
    let range = DeviceRange::new(0xFEE0_0010, 8, width(40)).unwrap();
    assert_eq!(range.physical(), 0xFEE0_0010);
    assert_eq!(range.page_start(), 0xFEE0_0000);
    assert_eq!(range.page_end(), 0xFEE0_1000);
}

#[test]
fn every_range_error_names_its_position() {
    for (error, expected) in [
        (DeviceRangeError::Empty, "empty"),
        (DeviceRangeError::Unaligned(0xFEE0_0001), "0xfee00001"),
        (DeviceRangeError::Overflow, "overflow"),
        (
            DeviceRangeError::OutsideWidth {
                bits: 36,
                page_start: 0x10_0000_0000,
                page_end: 0x10_0000_1000,
            },
            "36",
        ),
    ] {
        let text = error.to_string();
        assert!(text.contains(expected), "{error:?} rendered as {text:?}");
    }
}

#[test]
fn each_runtime_width_accepts_its_last_page_and_refuses_the_next() {
    for bits in [36, 40, 46, 52] {
        let limit = 1_u64 << bits;
        let accepted = DeviceRange::new(limit - PAGE_SIZE, PAGE_SIZE, width(bits)).unwrap();
        assert_eq!(accepted.page_end(), limit);
        assert!(matches!(
            DeviceRange::new(limit - 4, 8, width(bits)),
            Err(DeviceRangeError::OutsideWidth { .. })
        ));
    }
}

#[test]
fn a_short_register_range_expands_to_the_complete_page() {
    let physical = PAGE_SIZE + 4;
    let len = 4;
    let range = DeviceRange::new(physical, len, width(36)).unwrap();
    assert_eq!(range.page_start(), PAGE_SIZE);
    assert_eq!(range.page_end(), 2 * PAGE_SIZE);
    let requested_end = physical + len;
    let outside_request = (range.page_start()..range.page_end())
        .filter(|byte| *byte < physical || *byte >= requested_end)
        .count();
    assert_eq!(outside_request, (PAGE_SIZE - len) as usize);
}

#[test]
fn empty_unaligned_wrapping_and_rounding_overflow_are_refused() {
    assert_eq!(
        DeviceRange::new(0, 0, width(52)),
        Err(DeviceRangeError::Empty)
    );
    assert_eq!(
        DeviceRange::new(1, 4, width(52)),
        Err(DeviceRangeError::Unaligned(1))
    );
    assert_eq!(
        DeviceRange::new(u64::MAX - 3, 8, width(52)),
        Err(DeviceRangeError::Overflow)
    );
    assert_eq!(
        DeviceRange::new(u64::MAX - 7, 4, width(52)),
        Err(DeviceRangeError::Overflow)
    );
}
