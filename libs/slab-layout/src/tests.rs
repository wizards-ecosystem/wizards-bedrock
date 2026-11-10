use super::*;

fn size_class(size: usize, align: usize) -> Option<SizeClass> {
    SizeClass::for_size_align(size, align).expect("valid test alignment")
}

fn placement(size: usize, align: usize) -> Placement {
    place(size, align).expect("valid test alignment")
}

/// Every index is checked because `wost/src/mm/heap.rs` indexes its free-list array with it.
#[test]
fn a_size_class_reports_its_own_index_and_bytes() {
    for index in 0..CLASSES {
        let class = SizeClass::from_index(index).expect("a class that exists");
        assert_eq!(class.index(), index, "class {index} reports another index");
        assert_eq!(
            class.bytes(),
            MIN_CLASS << index,
            "class {index} holds the wrong slot size"
        );
    }
    assert!(SizeClass::from_index(CLASSES).is_none());
}

#[test]
fn classes_are_powers_of_two_from_sixteen_to_two_thousand_and_forty_eight() {
    let sizes: Vec<_> = (0..CLASSES)
        .map(|index| SizeClass::from_index(index).expect("in range").bytes())
        .collect();
    assert_eq!(sizes, [16, 32, 64, 128, 256, 512, 1024, 2048]);
    assert_eq!(sizes[0], MIN_CLASS);
    assert_eq!(sizes[CLASSES - 1], MAX_CLASS);
    assert_eq!(SizeClass::from_index(CLASSES), None);
}

#[test]
fn a_frame_divides_exactly_into_slots_of_every_class() {
    for index in 0..CLASSES {
        let class = SizeClass::from_index(index).expect("in range");
        assert_eq!(
            class.slots_per_frame() * class.bytes(),
            FRAME_SIZE,
            "class {} leaves a remainder in a frame",
            class.bytes()
        );
    }
}

#[test]
fn a_request_takes_the_smallest_class_that_holds_it() {
    assert_eq!(size_class(1, 1).map(SizeClass::bytes), Some(16));
    assert_eq!(size_class(16, 1).map(SizeClass::bytes), Some(16));
    assert_eq!(size_class(17, 1).map(SizeClass::bytes), Some(32));
    assert_eq!(size_class(2048, 1).map(SizeClass::bytes), Some(2048));
    assert_eq!(size_class(2049, 1), None);
}

#[test]
fn every_slab_size_takes_the_exact_smallest_class() {
    let classes = [16, 32, 64, 128, 256, 512, 1024, 2048];
    for size in 0..=MAX_CLASS {
        let expected = classes
            .into_iter()
            .find(|bytes| *bytes >= size.max(1))
            .expect("the largest class covers this size");
        assert_eq!(
            size_class(size, 1).map(SizeClass::bytes),
            Some(expected),
            "size {size}"
        );
    }
}

#[test]
fn placement_errors_name_the_invalid_alignment() {
    assert_eq!(
        PlacementError::InvalidAlignment(24).to_string(),
        "alignment 24 is not a non-zero power of two"
    );
}

#[test]
fn alignment_is_met_by_taking_a_larger_class_not_by_padding() {
    // A 16-byte slot sits at a multiple of 16, which need not be a multiple of 64.
    let class = size_class(8, 64).expect("small enough");
    assert_eq!(class.bytes(), 64);
    assert_eq!(size_class(8, 2048).map(SizeClass::bytes), Some(2048));
    assert_eq!(
        size_class(8, 4096),
        None,
        "an alignment past the largest class needs whole frames"
    );
}

#[test]
fn every_class_divides_the_frame_and_its_slots_fill_it() {
    for index in 0..CLASSES {
        let class = SizeClass::from_index(index).expect("in range");
        assert!(
            FRAME_SIZE.is_multiple_of(class.bytes()),
            "{} does not divide a frame",
            class.bytes()
        );
        assert_eq!(
            class.slots_per_frame() * class.bytes(),
            FRAME_SIZE,
            "class {} leaves the frame short or overruns it",
            class.bytes()
        );
        let last = class.slots_per_frame() - 1;
        assert_eq!(class.offset_of(0), 0);
        assert_eq!(class.offset_of(last), FRAME_SIZE - class.bytes());
    }
}

#[test]
fn slot_offsets_fill_the_frame_without_overlapping() {
    let class = size_class(256, 1).expect("a class");
    let last = class.slots_per_frame() - 1;
    assert_eq!(class.offset_of(0), 0);
    assert_eq!(class.offset_of(last), FRAME_SIZE - class.bytes());
    assert_eq!(
        class.offset_of(last) + class.bytes(),
        FRAME_SIZE,
        "the last slot ends exactly at the frame's end"
    );
}

#[test]
#[should_panic(expected = "slot outside the slab")]
fn a_slot_past_the_end_of_a_slab_panics() {
    let class = size_class(16, 1).expect("a class");
    let _ = class.offset_of(class.slots_per_frame());
}

#[test]
fn a_small_request_is_placed_in_a_slot() {
    assert_eq!(
        placement(100, 8),
        Placement::Slot(size_class(100, 8).expect("a class"))
    );
}

#[test]
fn a_large_request_is_placed_in_whole_frames() {
    assert_eq!(placement(4096, 1), Placement::Frames { count: 1, align: 1 });
    assert_eq!(placement(4097, 1), Placement::Frames { count: 2, align: 1 });
    assert_eq!(
        placement(2049, 1),
        Placement::Frames { count: 1, align: 1 },
        "one byte past the largest class already needs a frame"
    );
    assert_eq!(
        placement(8192, 8192),
        Placement::Frames { count: 2, align: 2 },
        "alignment is counted in frames, like the run itself"
    );
}

#[test]
fn a_zero_sized_request_still_gets_a_slot() {
    // `GlobalAlloc` must return a unique non-null pointer even for zero bytes.
    assert_eq!(
        placement(0, 1),
        Placement::Slot(SizeClass::from_index(0).expect("the smallest class"))
    );
}

/// The naive arithmetic gives zero frames here, and a null `GlobalAlloc` means out of memory.
#[test]
fn a_zero_sized_request_too_aligned_for_a_class_still_gets_a_frame() {
    assert_eq!(
        placement(0, MAX_CLASS * 2),
        Placement::Frames { count: 1, align: 1 }
    );
    assert_eq!(
        placement(0, FRAME_SIZE * 4),
        Placement::Frames { count: 1, align: 4 }
    );
}

#[test]
fn invalid_alignments_are_refused_before_placement_arithmetic() {
    for align in [0, 3, usize::MAX] {
        assert_eq!(
            SizeClass::for_size_align(1, align),
            Err(PlacementError::InvalidAlignment(align))
        );
        assert_eq!(
            place(1, align),
            Err(PlacementError::InvalidAlignment(align))
        );
    }
}

/// Counts and offsets are worked out by hand, so a broken formula disagrees with them.
#[test]
fn no_two_slots_of_any_class_overlap() {
    // (class size, slots in a 4096-byte frame, offset of the last slot).
    const EXPECTED: [(usize, usize, usize); CLASSES] = [
        (16, 256, 4080),
        (32, 128, 4064),
        (64, 64, 4032),
        (128, 32, 3968),
        (256, 16, 3840),
        (512, 8, 3584),
        (1024, 4, 3072),
        (2048, 2, 2048),
    ];

    for (index, (bytes, slots, last)) in EXPECTED.into_iter().enumerate() {
        let Some(class) = SizeClass::from_index(index) else {
            panic!("class {index} is within CLASSES but does not exist");
        };
        assert_eq!(class.bytes(), bytes, "class {index} is the wrong size");
        assert_eq!(class.slots_per_frame(), slots, "class {bytes} miscounts");
        assert_eq!(class.offset_of(0), 0, "class {bytes} starts late");
        assert_eq!(class.offset_of(slots - 1), last, "class {bytes} ends wrong");

        let offsets: Vec<usize> = (0..slots).map(|slot| class.offset_of(slot)).collect();
        let mut sorted = offsets.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), slots, "class {bytes} repeats an offset");
        assert_eq!(offsets, sorted, "class {bytes} hands them out unordered");
        assert!(
            sorted.last().is_some_and(|end| end + bytes <= FRAME_SIZE),
            "class {bytes} runs past the frame"
        );
    }
}
