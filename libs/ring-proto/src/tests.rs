use super::*;

fn capacity(entries: u32) -> Capacity {
    Capacity::new(entries).expect("a usable capacity")
}

/// Pin the externally visible version independently of its users.
#[test]
fn the_wire_version_is_one() {
    assert_eq!(VERSION, 1, "the ring layout has not changed");
}

#[test]
fn publication_words_exhaust_every_small_offset() {
    for bytes in 1..=16 {
        for at in 0..=20 {
            let admitted = SharedLayout::new(bytes, [at]);
            let expected = at % align_of::<u32>() == 0
                && at
                    .checked_add(PUBLICATION_WORD_BYTES)
                    .is_some_and(|end| end <= bytes);
            assert_eq!(admitted.is_ok(), expected, "bytes {bytes}, word {at}");
        }
    }
}

#[test]
fn payload_exhausts_touching_overlapping_and_outside_ranges() {
    const BYTES: usize = 16;
    let layout = SharedLayout::new(BYTES, [4, 12]).expect("two disjoint words");

    for at in 0..=BYTES + 2 {
        for len in 0..=BYTES + 2 {
            let expected = at.checked_add(len).is_some_and(|end| {
                end <= BYTES
                    && ![(4, 8), (12, 16)]
                        .into_iter()
                        .any(|(word, word_end)| at < word_end && word < end)
            });
            assert_eq!(
                layout.contains_payload(at, len),
                expected,
                "payload {at}..+{len}"
            );
        }
    }

    assert!(layout.contains_payload(0, 4), "left touching range");
    assert!(layout.contains_payload(8, 4), "range between words");
    assert!(!layout.contains_payload(3, 2), "one-byte overlap");
    assert!(!layout.contains_payload(15, 1), "last-byte overlap");
}

#[test]
fn a_layout_reports_its_bytes_and_every_error_names_its_word() {
    let layout = SharedLayout::new(16, [4, 12]).expect("two disjoint words");
    assert_eq!(layout.bytes(), 16);

    for (error, expected) in [
        (SharedLayoutError::Empty, "empty"),
        (SharedLayoutError::PublicationRangeOverflow(7), "7"),
        (SharedLayoutError::PublicationOutOfBounds(16), "16"),
        (SharedLayoutError::PublicationMisaligned(3), "3"),
        (
            SharedLayoutError::PublicationOverlap {
                first: 4,
                second: 6,
            },
            "6",
        ),
    ] {
        let text = error.to_string();
        assert!(text.contains(expected), "{error:?} rendered as {text:?}");
    }
}

#[test]
fn claiming_exactly_the_posted_head_reaps_everything() {
    let mut queues = Queues::new(capacity(4), capacity(8)).expect("a usable pair");
    queues.completion.head = 10;
    queues.completion.tail = 13;

    assert_eq!(
        queues.reap_up_to(13),
        3,
        "the posted head itself is reapable"
    );
    assert_eq!(queues.completion.head(), 13);
    assert_eq!(queues.reap_up_to(13), 0, "nothing remains");
}

#[test]
fn malformed_publication_partitions_are_refused_exactly() {
    assert_eq!(SharedLayout::<0>::new(0, []), Err(SharedLayoutError::Empty));
    assert_eq!(
        SharedLayout::new(16, [usize::MAX]),
        Err(SharedLayoutError::PublicationRangeOverflow(usize::MAX))
    );
    assert_eq!(
        SharedLayout::new(16, [16]),
        Err(SharedLayoutError::PublicationOutOfBounds(16))
    );
    assert_eq!(
        SharedLayout::new(16, [2]),
        Err(SharedLayoutError::PublicationMisaligned(2))
    );
    assert_eq!(
        SharedLayout::new(16, [4, 4]),
        Err(SharedLayoutError::PublicationOverlap {
            first: 4,
            second: 4,
        })
    );
    let layout = SharedLayout::new(16, [4]).expect("one word");
    assert!(!layout.contains_payload(usize::MAX, 2));
    assert!(layout.contains_publication_word(4));
    assert!(!layout.contains_publication_word(8));
}

#[test]
fn the_consistency_oracle_refuses_an_over_reserved_ring() {
    let queues = Queues::new(capacity(4), capacity(8)).expect("a usable pair");
    assert!(queues.is_consistent(), "a fresh pair is consistent");

    let mut impossible = queues;
    impossible.reserved = 9;
    assert!(
        !impossible.is_consistent(),
        "nine reservations cannot fit in eight completion slots"
    );

    let mut busy = Queues::new(capacity(4), capacity(8)).expect("a usable pair");
    let mut admitted = 0;
    while busy.submit().is_some() {
        admitted += 1;
        assert!(
            busy.is_consistent(),
            "reserving the {admitted}th slot broke the invariant"
        );
    }
    assert!(admitted > 0, "nothing was admitted; the check is vacuous");
    assert!(busy.submit().is_none(), "a full ring admits nothing more");
    assert!(busy.is_consistent(), "and refusing kept the invariant");
}

#[test]
fn completion_heads_move_only_forward_inside_the_posted_window() {
    let mut queues = Queues::new(capacity(4), capacity(8)).expect("a usable pair");
    queues.completion.head = 10;
    queues.completion.tail = 12;

    assert_eq!(queues.reap_up_to(9), 0, "a head just behind is stale");
    assert_eq!(queues.completion.head(), 10);
    assert_eq!(queues.reap_up_to(100), 0, "a far head is not a large reap");
    assert_eq!(queues.completion.head(), 10);
    assert_eq!(queues.reap_up_to(11), 1);
    assert_eq!(queues.completion.head(), 11);

    queues.completion.head = u32::MAX;
    queues.completion.tail = 1;
    assert_eq!(queues.reap_up_to(0), 1, "one forward step crosses wrap");
    assert_eq!(queues.completion.head(), 0);
    assert_eq!(queues.completion.occupied(), 1);
}

#[test]
fn a_capacity_is_a_power_of_two_that_leaves_room_to_count() {
    for good in [1, 2, 4, 8, 1024, Capacity::MAX] {
        assert!(Capacity::new(good).is_ok(), "{good} was refused");
    }
    for bad in [0, 3, 5, 6, 7, 100, u32::MAX] {
        assert_eq!(
            Capacity::new(bad),
            Err(CapacityError::NotAPowerOfTwo(bad)),
            "{bad} was accepted"
        );
    }
    assert_eq!(
        Capacity::new(Capacity::MAX),
        Ok(Capacity(Capacity::MAX)),
        "the largest usable ring was refused"
    );
    assert_eq!(Capacity::MAX.checked_mul(2), None, "MAX must be 2^31");
}

#[test]
fn capacity_errors_name_the_refused_value_and_rule() {
    let power = CapacityError::NotAPowerOfTwo(3).to_string();
    assert!(
        power.contains('3') && power.contains("power of two"),
        "{power}"
    );

    let size = CapacityError::TooLarge(17).to_string();
    assert!(size.contains("17") && size.contains("more than"), "{size}");
}

#[test]
fn pair_errors_name_both_capacities_and_the_rule() {
    let error = PairError::CompletionTooSmall {
        submission: 4,
        completion: 7,
    }
    .to_string();
    assert!(
        error.contains('4') && error.contains('7') && error.contains("twice"),
        "{error}"
    );
}

/// Counting operations rather than slots tells full from empty without sacrificing an entry.
#[test]
fn full_and_empty_are_not_the_same_state() {
    let mut ring = Ring::new(capacity(4));
    assert!(ring.is_empty() && !ring.is_full());

    for _ in 0..4 {
        assert!(ring.produce().is_some());
    }
    assert!(ring.is_full() && !ring.is_empty());
    assert_eq!(
        ring.head() & ring.capacity().mask(),
        ring.tail() & ring.capacity().mask(),
        "the slot indices agree, and the counters must not"
    );
    assert_eq!(ring.produce(), None);
}

#[test]
fn slots_are_handed_out_in_order_and_repeat() {
    let mut ring = Ring::new(capacity(4));
    let produced: Vec<usize> = (0..4).filter_map(|_| ring.produce()).collect();
    assert_eq!(produced, [0, 1, 2, 3]);

    let consumed: Vec<usize> = (0..4).filter_map(|_| ring.consume()).collect();
    assert_eq!(consumed, [0, 1, 2, 3]);
    assert!(ring.is_empty());

    assert_eq!(ring.produce(), Some(0), "the next lap did not start over");
}

#[test]
fn counter_accessors_report_counts_not_masked_slots() {
    let mut ring = Ring::adopt(capacity(8), 41, 44).expect("three occupied entries");
    assert_eq!(ring.head(), 41);
    assert_eq!(ring.tail(), 44);
    assert_eq!(ring.produce(), Some(4));
    assert_eq!(ring.tail(), 45);
}

/// The head starts at `u32::MAX - 3`, which is `0xFFFF_FFFC`, and `0xFFFF_FFFC & 7` is 4.
#[test]
fn the_counters_survive_the_wrap() {
    let capacity = capacity(8);
    let mut ring = Ring::adopt(capacity, u32::MAX - 3, u32::MAX - 3).expect("consistent");

    for _ in 0..8 {
        assert!(ring.produce().is_some());
    }
    assert!(ring.is_full(), "the ring lost count across the wrap");
    assert!(
        ring.tail() < ring.head(),
        "this test is only meaningful once the tail has wrapped below the head"
    );
    assert_eq!(ring.occupied(), 8);

    let slots: Vec<usize> = (0..8).filter_map(|_| ring.consume()).collect();
    assert_eq!(slots, [4, 5, 6, 7, 0, 1, 2, 3]);
    assert!(ring.is_empty());
}

#[test]
fn impossible_counters_are_refused() {
    let capacity = capacity(4);
    assert!(Ring::adopt(capacity, 0, 4).is_some());
    assert_eq!(Ring::adopt(capacity, 0, 5), None);
    assert_eq!(Ring::adopt(capacity, 0, u32::MAX), None);
    assert!(Ring::adopt(capacity, u32::MAX, 2).is_some());
    assert_eq!(Ring::adopt(capacity, u32::MAX, 10), None);
}

/// Widening must make twice the largest capacity a refusal, not wrap it.
#[test]
fn a_completion_queue_is_at_least_twice_the_submission_queue() {
    assert!(Queues::new(capacity(4), capacity(8)).is_ok());
    assert!(Queues::new(capacity(4), capacity(16)).is_ok());
    assert_eq!(
        Queues::new(capacity(4), capacity(4)),
        Err(PairError::CompletionTooSmall {
            submission: 4,
            completion: 4
        })
    );
    assert!(Queues::new(capacity(Capacity::MAX), capacity(Capacity::MAX)).is_err());
}

#[test]
fn admission_stops_before_a_completion_could_be_dropped() {
    let mut queues = Queues::new(capacity(4), capacity(8)).expect("a valid pair");

    for _ in 0..4 {
        assert!(queues.submit().is_some());
        assert!(queues.is_consistent());
    }
    assert_eq!(queues.submit(), None, "admitted past the submission queue");
    assert_eq!(queues.outstanding(), 4);

    for _ in 0..4 {
        assert!(queues.take().is_some());
    }
    assert_eq!(
        queues.outstanding(),
        4,
        "taking work released a reservation"
    );

    for _ in 0..4 {
        assert!(queues.submit().is_some());
        assert!(queues.is_consistent());
    }
    assert_eq!(queues.outstanding(), 8);
    for _ in 0..4 {
        assert!(queues.take().is_some());
    }
    assert_eq!(
        queues.submit(),
        None,
        "admitted a ninth operation with eight completion slots reserved"
    );

    for _ in 0..8 {
        assert!(queues.complete().is_some(), "a completion was dropped");
        assert!(queues.is_consistent());
    }
    assert_eq!(queues.outstanding(), 0);
    assert_eq!(queues.complete(), None, "reported more than was admitted");
}

/// A fixed walk makes failures reproducible without another dependency.
#[test]
fn the_reservation_invariant_holds_under_interleaving() {
    let mut queues = Queues::new(capacity(2), capacity(4)).expect("a valid pair");
    let mut admitted = 0u32;
    let mut reported = 0u32;

    let mut state = 0x2545_F491u32;
    for _ in 0..10_000 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        match state >> 30 {
            0 => {
                if queues.submit().is_some() {
                    admitted += 1;
                }
            }
            1 => {
                let _ = queues.take();
            }
            2 => {
                if queues.complete().is_some() {
                    reported += 1;
                }
            }
            _ => {
                let _ = queues.reap();
            }
        }
        assert!(
            queues.is_consistent(),
            "the reservation invariant broke after {admitted} admitted and {reported} reported"
        );
        assert_eq!(
            queues.outstanding(),
            admitted - reported,
            "outstanding drifted from admitted minus reported"
        );
        assert!(queues.submissions().occupied() <= 2);
        assert!(queues.completions().occupied() <= 4);
    }
    assert!(admitted > 100, "the walk never exercised admission");
    assert!(reported > 100, "the walk never exercised completion");
}

#[test]
fn a_completion_with_no_reservation_is_refused() {
    let mut queues = Queues::new(capacity(2), capacity(4)).expect("a valid pair");
    assert_eq!(queues.complete(), None);
    assert!(queues.completions().is_empty());
    assert!(queues.is_consistent());
}

/// Checked rather than reasoned: no `u32` passes the power-of-two refusal and then exceeds `MAX`.
#[test]
fn every_power_of_two_a_u32_holds_is_accepted() {
    for shift in 0..u32::BITS {
        let entries = 1u32 << shift;
        assert!(Capacity::new(entries).is_ok(), "{entries} was refused");
    }
    assert_eq!(Capacity::MAX, 1 << 31);
}
