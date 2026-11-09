use super::*;

#[test]
fn address_ranges_refuse_empty_and_wrapping_bounds() {
    assert_eq!(AddressRange::new(0x1000, 0), None);
    assert_eq!(AddressRange::new(u64::MAX, 1), None);
    assert_eq!(
        AddressRange::new(u64::MAX, 0),
        None,
        "empty-at-the-end is still empty"
    );
    assert_eq!(
        AddressRange::new(u64::MAX - 1, 1),
        Some(AddressRange {
            base: u64::MAX - 1,
            end: u64::MAX,
        })
    );
}

#[test]
fn a_covering_range_refuses_rounding_past_the_address_space() {
    assert_eq!(
        AddressRange::covering(0x1234, 1, 0x1000),
        Some(AddressRange {
            base: 0x1000,
            end: 0x2000,
        })
    );
    assert_eq!(
        AddressRange::covering(u64::MAX - 0x0fff, 0x0fff, 0x1000),
        None
    );
    assert_eq!(AddressRange::covering(0x1000, 0, 0x1000), None);
    assert_eq!(AddressRange::covering(0x1000, 1, 3), None);
}

#[test]
fn first_fit_refuses_invalid_request_geometry() {
    let window = AddressRange::new(0x1000, 0x1000).expect("window");
    assert_eq!(first_fit_by(window, 0, 0x1000, &[], |range| *range), None);
    assert_eq!(first_fit_by(window, 0x1000, 0, &[], |range| *range), None);
    assert_eq!(first_fit_by(window, 0x1000, 3, &[], |range| *range), None);
}

#[test]
fn first_fit_accounts_for_ranges_crossing_the_window_edges() {
    let window = AddressRange::new(4, 4).expect("window");
    let crosses_end = AddressRange::new(5, 7).expect("occupied");
    let after_end = AddressRange::new(8, 4).expect("occupied");

    assert_eq!(
        first_fit_by(window, 2, 1, &[Some(crosses_end)], |range| *range),
        None
    );
    assert_eq!(
        first_fit_by(window, 2, 1, &[Some(after_end)], |range| *range),
        Some(4)
    );
}

#[test]
fn first_fit_reuses_an_interior_gap_in_any_active_order() {
    let window = AddressRange::new(0x1000, 0x4000).expect("window");
    let first = AddressRange::new(0x1000, 0x1000).expect("first");
    let second = AddressRange::new(0x2000, 0x1000).expect("second");

    assert_eq!(
        first_fit_by(window, 0x1000, 0x1000, &[Some(second)], |range| *range),
        Some(0x1000)
    );
    assert_eq!(
        first_fit_by(
            window,
            0x1000,
            0x1000,
            &[Some(second), Some(first)],
            |range| *range,
        ),
        Some(0x3000)
    );
    assert_eq!(
        first_fit_by(
            window,
            0x1000,
            0x1000,
            &[Some(first), Some(second)],
            |range| *range,
        ),
        Some(0x3000)
    );
}

#[test]
fn an_access_range_reports_its_own_offset() {
    const REGION: usize = 0x1000;
    const OFFSET: usize = 0x40;
    const COUNT: usize = 4;
    const WIDTH: usize = 4;

    let range = AccessRange::repeated(OFFSET, COUNT, WIDTH, REGION).expect("inside the region");

    assert_eq!(range.offset(), OFFSET, "offset() gave something else");
}

#[test]
fn an_exactly_bounded_span_is_accepted() {
    assert_eq!(
        AccessRange::repeated(4, 3, 4, 16),
        Ok(AccessRange {
            offset: 4,
            end: 16,
            width: 4,
        })
    );
}

#[test]
fn invalid_widths_are_refused() {
    assert_eq!(
        AccessRange::repeated(0, 1, 0, 4),
        Err(AccessError::InvalidWidth(0))
    );
    assert_eq!(
        AccessRange::repeated(0, 1, 3, 4),
        Err(AccessError::InvalidWidth(3))
    );
}

#[test]
fn a_misaligned_start_is_refused() {
    assert_eq!(
        AccessRange::repeated(2, 1, 4, 8),
        Err(AccessError::Misaligned {
            offset: 2,
            align: 4,
        })
    );
}

#[test]
fn a_repeated_length_that_wraps_is_refused() {
    assert_eq!(
        AccessRange::repeated(0, usize::MAX, 4, usize::MAX),
        Err(AccessError::LengthOverflow {
            count: usize::MAX,
            width: 4,
        })
    );
}

#[test]
fn an_end_that_wraps_is_refused() {
    let offset = usize::MAX - 3;
    assert_eq!(
        AccessRange::repeated(offset, 2, 4, usize::MAX),
        Err(AccessError::EndOverflow { offset, len: 8 })
    );
}

#[test]
fn a_span_past_the_region_is_refused() {
    assert_eq!(
        AccessRange::repeated(4, 2, 4, 8),
        Err(AccessError::OutOfBounds {
            offset: 4,
            len: 8,
            region_len: 8,
        })
    );
}

#[test]
fn a_span_one_byte_past_the_region_is_refused() {
    assert!(AccessRange::repeated(0, 1, 8, 8).is_ok());
    assert!(AccessRange::repeated(4, 1, 4, 8).is_ok());
    assert_eq!(
        AccessRange::repeated(8, 1, 1, 8),
        Err(AccessError::OutOfBounds {
            offset: 8,
            len: 1,
            region_len: 8,
        })
    );
    assert_eq!(
        AccessRange::repeated(1, 1, 1, 1),
        Err(AccessError::OutOfBounds {
            offset: 1,
            len: 1,
            region_len: 1,
        })
    );
}

#[test]
fn an_empty_span_is_valid_only_at_or_before_the_end() {
    assert!(AccessRange::repeated(8, 0, 4, 8).is_ok());
    assert!(matches!(
        AccessRange::repeated(12, 0, 4, 8),
        Err(AccessError::OutOfBounds { .. })
    ));
}

#[test]
fn errors_name_the_refused_access() {
    let cases = [
        (AccessError::InvalidWidth(3), "access width 3 is invalid"),
        (
            AccessError::Misaligned {
                offset: 6,
                align: 4,
            },
            "offset 6 is not 4-byte aligned",
        ),
        (
            AccessError::LengthOverflow { count: 9, width: 8 },
            "9 accesses of 8 bytes overflow",
        ),
        (
            AccessError::EndOverflow { offset: 15, len: 2 },
            "access of 2 bytes at 15 wraps",
        ),
        (
            AccessError::OutOfBounds {
                offset: 8,
                len: 4,
                region_len: 10,
            },
            "access of 4 bytes at 8 leaves a 10-byte region",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}
