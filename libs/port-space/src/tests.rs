use super::*;

/// Widths an x86 port access can have, so every span length a real kernel asks for.
const WIDTHS: [usize; 3] = [1, 2, 4];

/// A claim is permanent, so a refused request keeping its first span strands it for the boot.
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
fn a_granted_multi_span_reservation_takes_every_span() {
    let spans = [(0x60, 1), (0x64, 1), (0x3F8, 8)];

    let mut space = PortSpace::new();
    assert!(space.reserve_all(&spans));

    for (port, len) in spans {
        assert!(!space.is_available(port, len), "{port:#x} was not recorded");
    }
    assert!(space.is_available(0x70, 1));
    assert!(!space.reserve_all(&spans));
}

/// Without the pairwise check both halves pass `span`, and the caller owns one range twice.
#[test]
fn a_request_that_overlaps_itself_is_refused() {
    let mut space = PortSpace::new();
    assert!(!space.reserve_all(&[(0x60, 4), (0x62, 2)]));
    assert!(space.is_available(0x60, 4), "nothing was recorded");

    assert!(space.reserve_all(&[(0x60, 2), (0x62, 2)]));

    let mut reversed = PortSpace::new();
    assert!(reversed.reserve_all(&[(0x62, 2), (0x60, 2)]));
}

#[test]
fn an_empty_or_oversized_multi_span_request_is_refused() {
    let mut space = PortSpace::new();
    assert!(!space.reserve_all(&[]));

    let too_many: [(usize, usize); MAX_SPANS + 1] = core::array::from_fn(|index| (index * 2, 1));
    assert!(!space.reserve_all(&too_many), "past the ceiling is refused");
    assert!(space.is_available(0, 1), "and nothing was recorded");
}

#[test]
fn a_fresh_space_reserves_nothing() {
    let space = PortSpace::new();

    for port in [0, 1, 0x3F8, PORT_COUNT - 1] {
        assert!(!space.is_reserved(port), "port {port:#x}");
    }
    assert_eq!(space, PortSpace::default());
}

#[test]
fn reserving_marks_exactly_its_span() {
    let mut space = PortSpace::new();
    assert!(space.reserve(0x3F8, 8));

    assert!(!space.is_reserved(0x3F7), "the port below is untouched");
    for port in 0x3F8..0x400 {
        assert!(space.is_reserved(port), "port {port:#x}");
    }
    assert!(!space.is_reserved(0x400), "the port above is untouched");
}

/// The defect this crate exists to rule out: a claim marking one port of a four-port access.
#[test]
fn a_wide_reservation_covers_every_port_it_drives() {
    for width in WIDTHS {
        let mut space = PortSpace::new();
        assert!(space.reserve(0xF4, width), "width {width}");

        for offset in 0..width {
            assert!(space.is_reserved(0xF4 + offset), "width {width} +{offset}");
        }
        assert!(!space.is_reserved(0xF4 + width), "width {width} ran over");
    }
}

#[test]
fn a_second_reservation_of_the_same_port_is_refused() {
    let mut space = PortSpace::new();
    assert!(space.reserve(0x60, 1));
    assert!(!space.reserve(0x60, 1));
}

/// Swept over every relative position, so no way one span can meet another is left out.
#[test]
fn overlap_is_refused_from_either_side_and_adjacency_is_not() {
    let held = 0x40..0x44;
    for start in 0x3C..0x48 {
        for len in WIDTHS {
            let mut space = PortSpace::new();
            assert!(space.reserve(held.start, held.len()));

            let overlaps = (start..start + len).any(|port| held.contains(&port));
            assert_eq!(
                space.reserve(start, len),
                !overlaps,
                "{len} ports at {start:#x} against {held:#x?}"
            );
        }
    }
}

/// A refused reservation must leave the table untouched.
/// Otherwise a refused driver owns half of a neighbour's registers.
#[test]
fn a_refused_reservation_records_nothing() {
    let mut space = PortSpace::new();
    assert!(space.reserve(0x64, 1));

    let before = space;
    assert!(!space.reserve(0x62, 4), "the span straddles 0x64");
    assert_eq!(space, before);
    assert!(!space.is_reserved(0x62));
    assert!(!space.is_reserved(0x63));
    assert!(!space.is_reserved(0x65));
}

#[test]
fn a_span_running_past_the_end_of_the_space_is_refused() {
    let mut space = PortSpace::new();

    assert!(!space.reserve(PORT_COUNT - 3, 4));
    assert!(!space.reserve(PORT_COUNT, 1));
    assert!(!space.reserve(usize::MAX, 1), "the addition must not wrap");
    assert_eq!(space, PortSpace::new());
}

#[test]
fn a_span_ending_exactly_at_the_end_of_the_space_is_allowed() {
    let mut space = PortSpace::new();

    assert!(space.reserve(PORT_COUNT - 4, 4));
    assert!(space.is_reserved(PORT_COUNT - 1));
}

#[test]
fn an_empty_span_reserves_nothing_and_says_so() {
    let mut space = PortSpace::new();

    assert!(!space.reserve(0x80, 0));
    assert!(!space.is_available(0x80, 0));
    assert!(!space.is_reserved(0x80));
}

#[test]
fn every_port_outside_the_space_reads_as_reserved() {
    let space = PortSpace::new();

    assert!(space.is_reserved(PORT_COUNT));
    assert!(space.is_reserved(usize::MAX));
}

/// Both sides call `span`, so agreement alone holds for any `span`; a `Range` oracle checks it.
#[test]
fn availability_agrees_with_what_reserving_would_do() {
    let mut space = PortSpace::new();
    assert!(space.reserve(0x20, 2));
    let taken = 0x20usize..0x22;

    for start in 0x1C..0x28 {
        for len in WIDTHS {
            let available = space.is_available(start, len);
            let mut copy = space;
            assert_eq!(copy.reserve(start, len), available, "{len} at {start:#x}");

            let end = start + len;
            let overlaps = (start..end).any(|port| taken.contains(&port));
            assert_eq!(available, !overlaps, "{len} at {start:#x}");
        }
    }
}

/// Word boundaries are where a bitmap goes wrong.
#[test]
fn a_span_crossing_a_word_boundary_marks_both_words() {
    let mut space = PortSpace::new();
    let boundary = PORTS_PER_WORD;

    assert!(space.reserve(boundary - 2, 4));
    assert!(space.is_reserved(boundary - 1), "last bit of the low word");
    assert!(space.is_reserved(boundary), "first bit of the high word");
    assert!(!space.is_reserved(boundary + 2));
}

#[test]
fn a_multi_span_release_is_exact_and_all_or_none() {
    let spans = [(0x100usize, 4usize), (0x120, 8), (0x1ff, 2)];
    let mut space = PortSpace::new();
    assert!(space.reserve_all(&spans));

    for refused in [
        vec![(0x100, 4), (0x140, 1)],
        vec![(0x100, 4), (0x102, 2)],
        vec![(0x100, 0)],
        vec![(PORT_COUNT, 1)],
    ] {
        let before = space;
        assert!(!space.release_all(&refused));
        assert_eq!(space, before);
    }

    assert!(space.release_all(&spans));
    for (base, len) in spans {
        assert!(space.is_available(base, len));
    }
    let empty = space;
    assert!(!space.release_all(&spans), "nothing is held a second time");
    assert_eq!(space, empty);
}

#[test]
fn every_port_in_the_space_can_be_reserved_exactly_once() {
    let mut space = PortSpace::new();

    for port in 0..PORT_COUNT {
        assert!(space.reserve(port, 1), "port {port:#x}");
    }
    for port in 0..PORT_COUNT {
        assert!(space.is_reserved(port), "port {port:#x}");
        assert!(!space.reserve(port, 1), "port {port:#x} reserved twice");
    }
}

/// Literal witnesses, so `PORT_COUNT` does not serve as its own oracle.
#[test]
fn the_io_space_is_sixteen_bits() {
    assert_eq!(PORT_COUNT, 65_536);

    let mut space = PortSpace::new();
    assert!(space.reserve(0xFFFF, 1), "the last port is reservable");
    assert!(!space.is_available(0xFFFF, 2), "and nothing runs past it");
}
