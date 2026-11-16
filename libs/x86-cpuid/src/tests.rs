use super::*;

const IMMUNE: u64 = capability::MDS_NO
    | capability::TAA_NO
    | capability::SBDR_SSDP_NO
    | capability::FBSDP_NO
    | capability::PSDP_NO
    | capability::RFDS_NO;

fn posture(ebx: u32, edx: u32, capabilities: u64) -> Speculation {
    Speculation::read(
        CpuidLeaf::new(0, ebx, 0, edx),
        CpuidLeaf::new(0, 0, 0, 0),
        |_| capabilities,
    )
}

#[test]
fn the_buffer_clear_is_skipped_only_when_every_attack_it_stops_is_ruled_out() {
    assert!(!posture(RTM_BIT, ARCH_CAPABILITIES_BIT, IMMUNE).needs_buffer_clear());
    assert!(
        !posture(0, ARCH_CAPABILITIES_BIT, IMMUNE & !capability::TAA_NO).needs_buffer_clear(),
        "without TSX there is no TAA to clear"
    );
    assert!(
        posture(RTM_BIT, 0, IMMUNE).needs_buffer_clear(),
        "an absent capability word claims nothing"
    );
    for missing in [
        capability::MDS_NO,
        capability::TAA_NO,
        capability::SBDR_SSDP_NO,
        capability::FBSDP_NO,
        capability::PSDP_NO,
        capability::RFDS_NO,
    ] {
        assert!(
            posture(RTM_BIT, ARCH_CAPABILITIES_BIT, IMMUNE & !missing).needs_buffer_clear(),
            "{missing:#x} alone must keep the clear"
        );
    }
}

#[test]
fn the_colour_count_never_trusts_the_sliced_last_level_cache() {
    let cache = |kind: u32, level: u32, sets: u32| {
        cache::Cache::from_leaf(kind | level << 5, 64 - 1, sets - 1).expect("a described cache")
    };
    let comet_lake = [
        cache(1, 1, 64),
        cache(2, 1, 64),
        cache(3, 2, 1024),
        cache(3, 3, 8192),
    ];
    assert_eq!(
        cache::colour_count(comet_lake),
        16,
        "the L2 bounds the colours"
    );
    assert_eq!(
        cache::colour_count([cache(1, 1, 64), cache(3, 3, 8192)]),
        1,
        "without an L2 nothing bounds the slice hash"
    );
}
