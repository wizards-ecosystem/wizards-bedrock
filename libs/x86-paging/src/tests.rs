use super::*;

#[test]
fn a_mapping_with_canonical_endpoints_that_crosses_the_hole_is_refused() {
    assert_eq!(
        MappingRange::new(0, 0, 0xffff_8000_0000_1000),
        Err(MappingRangeError::CrossesCanonicalHole)
    );
}

#[test]
fn a_narrower_parent_beats_a_wider_leaf() {
    let supervisor = Flags::PRESENT.union(Flags::WRITABLE);
    let read_only = Flags::PRESENT.union(Flags::USER);
    let leaf = Flags::PRESENT.union(Flags::WRITABLE).union(Flags::USER);

    let under_supervisor = Flags::EVERYTHING_ALLOWED.through(supervisor).through(leaf);
    assert!(!under_supervisor.effective_for(leaf).contains(Flags::USER));
    assert!(
        under_supervisor
            .effective_for(leaf)
            .contains(Flags::WRITABLE)
    );

    let under_read_only = Flags::EVERYTHING_ALLOWED.through(read_only).through(leaf);
    assert!(
        !under_read_only
            .effective_for(leaf)
            .contains(Flags::WRITABLE)
    );
    assert!(under_read_only.effective_for(leaf).contains(Flags::USER));
}
