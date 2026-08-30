//! Bounded exhaustion over the whole size-class domain.
//!
//! This tier needs no dependency at all: the domain is finite and small, so
//! walking it is both cheaper than sampling it and a strictly stronger claim
//! (D-242). Each test names its bound, so a bound that shrinks is a visible
//! edit rather than a silent one.
//!
//! Every claim is transcribed from the module doc:
//!
//! > Classes are powers of two from 16 to 2048 bytes. A frame is 4096-aligned
//! > and a class's slots sit at multiples of the class size, so every slot is
//! > aligned to its own class — which is why `for_size_align` meets an
//! > alignment by choosing a larger class rather than by padding.
//!
//! What is *not* here: anything about the free lists or the frames themselves.
//! Those are `wost::mm::heap`'s, and this crate is deliberately only their
//! arithmetic (D-47).

use slab_layout::{CLASSES, FRAME_SIZE, MAX_CLASS, MIN_CLASS, SizeClass};

/// Every power-of-two alignment a request could name, up to a frame.
const ALIGNMENTS: usize = 13;

fn alignments() -> impl Iterator<Item = usize> {
    (0..ALIGNMENTS).map(|shift| 1usize << shift)
}

/// Every class index round trips and lands inside the documented range.
#[test]
fn every_class_index_round_trips_and_is_a_power_of_two_in_range() {
    for index in 0..CLASSES {
        let class = SizeClass::from_index(index).expect("an index below CLASSES");
        assert_eq!(class.index(), index, "index did not round trip");
        let bytes = class.bytes();
        assert!(
            bytes.is_power_of_two(),
            "class {index} is {bytes}, not a power of two"
        );
        assert!(
            (MIN_CLASS..=MAX_CLASS).contains(&bytes),
            "class {index} is {bytes}, outside {MIN_CLASS}..={MAX_CLASS}"
        );
    }
    assert!(
        SizeClass::from_index(CLASSES).is_none(),
        "an index at CLASSES was accepted"
    );
}

/// Every slot of every class is aligned to its own class and inside the frame.
///
/// The doc's reason for the design: a slot that is not class-aligned is one an
/// allocator would hand out for a request whose alignment it cannot meet.
#[test]
fn every_slot_of_every_class_is_class_aligned_and_tiles_the_frame() {
    for index in 0..CLASSES {
        let class = SizeClass::from_index(index).expect("an index below CLASSES");
        let bytes = class.bytes();
        let slots = class.slots_per_frame();
        assert!(slots > 0, "class {bytes} has no slots");

        let mut previous = None;
        for slot in 0..slots {
            let offset = class.offset_of(slot);
            assert_eq!(
                offset % bytes,
                0,
                "slot {slot} of class {bytes} is not class-aligned"
            );
            assert!(
                offset + bytes <= FRAME_SIZE,
                "slot {slot} of class {bytes} ends past the frame"
            );
            if let Some(last) = previous {
                assert_eq!(
                    offset,
                    last + bytes,
                    "slots of class {bytes} are not contiguous"
                );
            }
            previous = Some(offset);
        }
        assert_eq!(
            slots * bytes,
            FRAME_SIZE,
            "class {bytes} does not tile the frame"
        );
    }
}

/// Every request size to a frame, against every alignment to a frame, is met
/// by a class at least that large and aligned to it.
///
/// 53,261 pairs — the complete domain, not a sample of it. The doc says an
/// alignment is met "by choosing a larger class rather than by padding", so a
/// class returned for a request must satisfy both halves at once.
#[test]
fn every_size_to_4096_against_every_alignment_to_4096_is_met_by_a_larger_class() {
    let mut satisfied = 0usize;
    let mut too_big = 0usize;

    for size in 0..=FRAME_SIZE {
        for align in alignments() {
            match SizeClass::for_size_align(size, align) {
                Ok(Some(class)) => {
                    let bytes = class.bytes();
                    assert!(
                        bytes >= size,
                        "class {bytes} is smaller than request {size}"
                    );
                    assert_eq!(
                        bytes % align,
                        0,
                        "class {bytes} does not meet alignment {align} for size {size}"
                    );
                    satisfied += 1;
                }
                Ok(None) => too_big += 1,
                Err(error) => panic!("power-of-two alignment {align} was refused: {error:?}"),
            }
        }
    }

    assert_eq!(
        satisfied + too_big,
        (FRAME_SIZE + 1) * ALIGNMENTS,
        "the enumeration did not cover the stated domain"
    );
    assert!(
        satisfied > 0 && too_big > 0,
        "one arm of the domain was never reached"
    );
}

/// No alignment that is not a power of two is ever accepted, to 1024.
#[test]
fn no_alignment_that_is_not_a_power_of_two_is_accepted_below_1024() {
    let mut refused = 0usize;
    for align in 0..=1024usize {
        if align.is_power_of_two() {
            continue;
        }
        assert!(
            SizeClass::for_size_align(64, align).is_err(),
            "alignment {align} is not a power of two and was not refused"
        );
        refused += 1;
    }
    assert!(refused > 900, "only {refused} non-powers-of-two were tried");
}
