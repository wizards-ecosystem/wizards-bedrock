use super::*;

#[test]
fn a_range_is_non_empty_and_stays_inside_the_gsi_namespace() {
    assert_eq!(GsiRange::new(7, 0), None);
    assert_eq!(GsiRange::new(u32::MAX, 2), None);

    let one = GsiRange::new(u32::MAX, 1).expect("the final GSI alone");
    assert_eq!(one.first(), u32::MAX);
    assert_eq!(one.last, u32::MAX);
    assert!(one.contains(u32::MAX));
}

#[test]
fn a_gsi_selects_its_later_controller_across_a_gap() {
    let ranges = [
        GsiRange::new(0, 8).expect("0 through 7"),
        GsiRange::new(24, 16).expect("24 through 39"),
    ];

    assert_eq!(owner_of(&ranges, 1), Ok(Some(0)));
    assert_eq!(owner_of(&ranges, 23), Ok(None));
    assert_eq!(owner_of(&ranges, 24), Ok(Some(1)));
    assert_eq!(owner_of(&ranges, 39), Ok(Some(1)));
    assert_eq!(owner_of(&ranges, 40), Ok(None));
}

#[test]
fn any_overlap_refuses_the_complete_inventory() {
    let ranges = [
        GsiRange::new(0, 24).expect("0 through 23"),
        GsiRange::new(40, 8).expect("40 through 47"),
        GsiRange::new(20, 8).expect("20 through 27"),
    ];

    assert_eq!(
        owner_of(&ranges, 44),
        Err(OverlappingRanges {
            first: 0,
            second: 2,
        })
    );
}

#[test]
fn an_overlap_error_names_both_controller_indices() {
    assert_eq!(
        OverlappingRanges {
            first: 3,
            second: 7,
        }
        .to_string(),
        "i/o apic ranges 3 and 7 overlap"
    );
}

#[test]
fn a_version_register_yields_an_entry_count_only_when_it_is_plausible() {
    assert_eq!(entries_from_version(0x0017_0011), Some(24));
    assert_eq!(entries_from_version(0x0017_0020), Some(24));
    assert_eq!(entries_from_version(0x0000_0011), Some(1));
    assert_eq!(entries_from_version(0xFF17_8011), Some(24));
    assert_eq!(entries_from_version(0x00EF_0011), Some(240));

    assert_eq!(entries_from_version(0x0017_0010), None);
    assert_eq!(entries_from_version(0x0017_0021), None);
    assert_eq!(entries_from_version(0x0017_0000), None);
    assert_eq!(entries_from_version(0x00F0_0011), None);
    assert_eq!(entries_from_version(0x00FF_0011), None);
    assert_eq!(entries_from_version(0xFFFF_FFFF), None);
}
