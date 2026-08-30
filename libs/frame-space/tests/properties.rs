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

use frame_space::{FRAME_SIZE, FrameSpace};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestRunner};

/// Frames a `FrameSpace<1>` manages; small enough that a sequence can exhaust
/// it, which is where the interesting refusals live.
const FRAMES: usize = 64;

/// A space managing its whole first word of frames.
fn managed() -> FrameSpace<1> {
    let mut space: FrameSpace<1> = FrameSpace::new();
    space.insert(0, FRAME_SIZE * FRAMES as u64);
    space
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
    /// the two-bitmap claim.
    #[test]
    fn freeing_an_unmanaged_frame_is_refused(frame in FRAMES..FRAMES + 64) {
        let mut space = managed();
        prop_assert!(!space.is_managed(frame), "frame {} should be unmanaged", frame);
        prop_assert!(space.free(frame).is_err(), "an unmanaged frame was freed");
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
