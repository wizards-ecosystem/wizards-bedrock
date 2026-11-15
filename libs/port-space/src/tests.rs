use super::*;

#[test]
fn a_refused_multi_span_reservation_takes_nothing() {
    const SPANS: [(usize, usize); MAX_SPANS] = [
        (0x100, 4),
        (0x110, 3),
        (0x120, 2),
        (0x130, 5),
        (0x140, 1),
        (0x150, 6),
        (0x160, 7),
        (0x170, 8),
    ];

    for (refused, &(base, len)) in SPANS.iter().enumerate() {
        let mut space = PortSpace::new();
        assert!(space.reserve(base + len / 2, 1));
        assert!(
            !space.reserve_all(&SPANS),
            "the request must refuse conflict {refused}"
        );

        for (index, &(base, len)) in SPANS.iter().enumerate() {
            if index != refused {
                assert!(
                    space.is_available(base, len),
                    "conflict {refused} stranded span {index}"
                );
            }
        }
    }
}

#[test]
fn a_request_that_overlaps_itself_is_refused() {
    let mut space = PortSpace::new();
    assert!(!space.reserve_all(&[(0x60, 4), (0x62, 2)]));
    assert!(space.is_available(0x60, 4), "nothing was recorded");

    assert!(space.reserve_all(&[(0x60, 2), (0x62, 2)]));

    let mut reversed = PortSpace::new();
    assert!(reversed.reserve_all(&[(0x62, 2), (0x60, 2)]));
}
