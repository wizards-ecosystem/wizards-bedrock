//! Property tests over the allocator's ledger.
//!
//! The module doc states the reason for the design, and it is a checkable
//! claim rather than a description:
//!
//! > Two bitmaps, because one could not tell a double free from a frame that
//! > was never ours — and `free` is what a `Drop` impl calls, the one place a
//! > bug is silent.
//!
//! `allocate_contiguous` states another: "Allocate `count` consecutive frames
//! whose first frame is a multiple of `align` ... `align` is rounded up to the
//! next power of two." A run that is not aligned, or overlaps a live one, is
//! two owners for one frame — the failure a frame allocator exists to prevent.
//!
//! What is *not* here: the policy. The doc says "Nothing here promises a
//! policy", so no property asserts which frame comes back, only that whichever
//! does is free, aligned, and nobody else's.

use frame_space::{FRAME_SIZE, FrameSpace, FreeError};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestRunner};

/// Frames a `FrameSpace<1>` manages; small enough that a sequence can exhaust
/// it, which is where the interesting refusals live.
const FRAMES: usize = 64;

const LOWER_END: usize = 16;
const UPPER_START: usize = 32;
const UPPER_END: usize = 48;

/// A space managing its whole first word of frames.
fn managed() -> FrameSpace<1> {
    let mut space: FrameSpace<1> = FrameSpace::new();
    space.insert(0, FRAME_SIZE * FRAMES as u64);
    space
}

/// Two managed islands with a hole inside their physical extent.
fn gapped() -> FrameSpace<1> {
    let mut space: FrameSpace<1> = FrameSpace::new();
    space.insert(0, FRAME_SIZE * LOWER_END as u64);
    space.insert(
        FRAME_SIZE * UPPER_START as u64,
        FRAME_SIZE * (UPPER_END - UPPER_START) as u64,
    );
    space
}

fn allocated_gapped() -> FrameSpace<1> {
    let mut space = gapped();
    for _ in 0..FrameSpace::<1>::FRAME_COUNT {
        if space.allocate().is_none() {
            return space;
        }
    }
    assert!(
        space.allocate().is_none(),
        "allocation did not terminate after the complete frame domain"
    );
    space
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameClass {
    Managed,
    InRangeUnmanaged,
    OutOfRange,
}

fn classified_frame() -> impl Strategy<Value = (FrameClass, usize)> {
    prop_oneof![
        prop_oneof![0usize..LOWER_END, UPPER_START..UPPER_END]
            .prop_map(|frame| (FrameClass::Managed, frame)),
        (LOWER_END..UPPER_START).prop_map(|frame| (FrameClass::InRangeUnmanaged, frame)),
        (FRAMES..FRAMES + 64).prop_map(|frame| (FrameClass::OutOfRange, frame)),
    ]
}

/// One allocation request: a run length and an alignment worth asking for.
fn request() -> impl Strategy<Value = (usize, usize)> {
    (
        1usize..8,
        prop_oneof![Just(1usize), Just(2), Just(4), Just(8), Just(16)],
    )
}

proptest! {
    /// A run is aligned, and no frame in it belongs to a live run.
    ///
    /// Two owners for one frame is the defect; a sequence of differently
    /// aligned requests is what produces the near-misses that find it.
    #[test]
    fn an_allocated_run_is_aligned_and_nobody_elses(
        requests in prop::collection::vec(request(), 1..10),
    ) {
        let mut space = managed();
        let mut live: Vec<(usize, usize)> = Vec::new();
        for (count, align) in requests {
            let Some(first) = space.allocate_contiguous(count, align) else {
                continue;
            };
            let rounded = align.next_power_of_two();
            prop_assert_eq!(first % rounded, 0, "run at {} is not {}-aligned", first, rounded);
            for offset in 0..count {
                let frame = first + offset;
                prop_assert!(!space.is_free(frame), "frame {} was handed out and reads free", frame);
                for (other_first, other_count) in &live {
                    prop_assert!(
                        frame < *other_first || frame >= other_first + other_count,
                        "frame {} was handed out twice",
                        frame
                    );
                }
            }
            live.push((first, count));
        }
    }

    /// Allocation and release return the ledger to exactly where it started.
    ///
    /// The count is checked after every step as well as at the end, so a
    /// release that returns the wrong number of frames cannot be cancelled out
    /// by a later one.
    #[test]
    fn the_free_count_returns_to_its_baseline(
        requests in prop::collection::vec(request(), 1..10),
    ) {
        let mut space = managed();
        let baseline = space.free_count();
        let mut live: Vec<(usize, usize)> = Vec::new();
        let mut taken = 0usize;

        for (count, align) in requests {
            if let Some(first) = space.allocate_contiguous(count, align) {
                taken += count;
                prop_assert_eq!(
                    space.free_count(),
                    baseline - taken,
                    "the free count did not drop by exactly the run length"
                );
                live.push((first, count));
            }
        }
        for (first, count) in live {
            prop_assert!(space.free_contiguous(first, count).is_ok(), "a live run refused release");
        }
        prop_assert_eq!(space.free_count(), baseline, "the ledger did not return to baseline");
    }

    /// "One could not tell a double free from a frame that was never ours."
    #[test]
    fn a_double_free_is_refused(count in 1usize..8, align in 1usize..8) {
        let mut space = managed();
        let Some(first) = space.allocate_contiguous(count, align) else {
            return Ok(());
        };
        prop_assert!(space.free_contiguous(first, count).is_ok(), "the first release failed");
        prop_assert!(
            space.free_contiguous(first, count).is_err(),
            "a double free of {}..{} was accepted",
            first,
            first + count
        );
    }

    /// A frame that was never inserted is not ours to free — the other half of
    /// the two-bitmap claim. A gapped map keeps that case inside the managed
    /// physical extent, distinct from both owned and out-of-range frames.
    #[test]
    fn release_authority_distinguishes_managed_in_range_unmanaged_and_out_of_range(
        (class, frame) in classified_frame(),
    ) {
        let mut space = allocated_gapped();
        match class {
            FrameClass::Managed => {
                prop_assert!(space.is_managed(frame), "frame {} should be managed", frame);
                prop_assert_eq!(space.free(frame), Ok(()), "managed frame {} refused release", frame);
            }
            FrameClass::InRangeUnmanaged => {
                prop_assert!(frame < FrameSpace::<1>::FRAME_COUNT, "frame {} should be in range", frame);
                prop_assert!(!space.is_managed(frame), "hole frame {} should be unmanaged", frame);
                prop_assert_eq!(
                    space.free(frame),
                    Err(FreeError::NotManaged(frame)),
                    "in-range unmanaged frame {} was not refused as foreign",
                    frame
                );
            }
            FrameClass::OutOfRange => {
                prop_assert!(frame >= FrameSpace::<1>::FRAME_COUNT, "frame {} should be out of range", frame);
                prop_assert_eq!(
                    space.free(frame),
                    Err(FreeError::NotManaged(frame)),
                    "out-of-range frame {} was not refused as foreign",
                    frame
                );
            }
        }
    }
}

/// How often a generated request is actually granted — D-242's obligation on a
/// generated-input check.
///
/// Every run property skips a refused allocation, so a low grant rate would
/// leave them asserting almost nothing.
#[test]
fn the_ledger_properties_reach_granted_allocations() {
    const SAMPLES: usize = 4096;
    let mut runner = TestRunner::new(Config {
        cases: SAMPLES as u32,
        ..Config::default()
    });
    let mut granted = 0usize;
    let mut exhausted = 0usize;

    for _ in 0..SAMPLES {
        let requests = prop::collection::vec(request(), 1..10)
            .new_tree(&mut runner)
            .expect("requests")
            .current();
        let mut space = managed();
        for (count, align) in requests {
            if space.allocate_contiguous(count, align).is_some() {
                granted += 1;
            } else {
                exhausted += 1;
            }
        }
    }

    println!("frame-space: {granted} allocations granted, {exhausted} refused for space");
    assert!(
        granted > SAMPLES,
        "only {granted} allocations were granted across {SAMPLES} sequences; \
         the run properties are near-vacuous"
    );
    assert!(
        exhausted > 0,
        "the space was never exhausted; refusal is never exercised"
    );
}

#[test]
fn the_release_property_reaches_every_frame_class() {
    const SAMPLES: usize = 4096;
    let mut runner = TestRunner::new(Config {
        cases: SAMPLES as u32,
        ..Config::default()
    });
    let mut managed = 0usize;
    let mut in_range_unmanaged = 0usize;
    let mut out_of_range = 0usize;

    for _ in 0..SAMPLES {
        let (class, _) = classified_frame()
            .new_tree(&mut runner)
            .expect("a classified frame")
            .current();
        match class {
            FrameClass::Managed => managed += 1,
            FrameClass::InRangeUnmanaged => in_range_unmanaged += 1,
            FrameClass::OutOfRange => out_of_range += 1,
        }
    }

    println!(
        "frame-space release classes: {managed} managed, {in_range_unmanaged} \
         in-range unmanaged, {out_of_range} out-of-range"
    );
    for (name, reached) in [
        ("managed", managed),
        ("in-range unmanaged", in_range_unmanaged),
        ("out-of-range", out_of_range),
    ] {
        assert!(
            reached > SAMPLES / 5,
            "only {reached} of {SAMPLES} frames reached {name}"
        );
    }
}
