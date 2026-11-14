//! Property tests over the allocator's ledger; none asserts which frame comes back.

use frame_space::{FRAME_SIZE, FrameSpace};
use proptest::prelude::*;

/// Frames a `FrameSpace<1>` manages, few enough that a sequence can exhaust them.
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

    /// Checked after every step, so a wrong release count cannot be cancelled by a later one.
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

}
