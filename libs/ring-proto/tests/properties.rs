//! Property tests over the ring's index arithmetic, on copied counters only.

use proptest::prelude::*;
use ring_proto::{Capacity, Queues};

proptest! {

    /// Admission reserves before accepting, as Windows RIO does, unlike io_uring's `CQ_NODROP`.
    #[test]
    fn admission_never_lets_the_completion_queue_overflow(
        shift in 0u32..6,
        sequence in prop::collection::vec(0u8..4, 0..512),
    ) {
        let submission = Capacity::new(1 << shift).expect("a power of two");
        let completion = Capacity::new(2 << shift).expect("a power of two");
        let mut queues = Queues::new(submission, completion).expect("a valid pair");

        let mut admitted: u64 = 0;
        let mut reported: u64 = 0;
        for action in sequence {
            match action {
                0 => if queues.submit().is_some() { admitted += 1 },
                1 => { queues.take(); }
                2 => if queues.complete().is_some() { reported += 1 },
                _ => { queues.reap(); }
            }
            prop_assert!(
                queues.is_consistent(),
                "reserved {} with {} occupied in a completion queue of {}",
                queues.outstanding(),
                queues.completions().occupied(),
                completion.entries()
            );
            prop_assert!(!(queues.completions().is_full() && queues.submit().is_some()));
        }
        prop_assert!(reported <= admitted, "more completions than submissions");
    }

}
