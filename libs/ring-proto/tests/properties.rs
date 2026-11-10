//! Property tests over the ring's index arithmetic, on copied counters only.

use proptest::prelude::*;
use ring_proto::{Capacity, Queues, Ring};

/// One thing a test can do to a ring.
#[derive(Clone, Copy, Debug)]
enum Step {
    Produce,
    Consume,
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec(
        prop_oneof![Just(Step::Produce), Just(Step::Consume)],
        0..256,
    )
}

/// A power of two a `Capacity` accepts, kept small so a sequence can fill one.
fn capacity() -> impl Strategy<Value = Capacity> {
    (0u32..8).prop_map(|shift| Capacity::new(1 << shift).expect("a power of two"))
}

proptest! {
    /// The interesting inputs cross `u32::MAX`, where `adopt` can start directly.
    #[test]
    fn occupancy_and_boundary_predicates_remain_consistent(
        entries in capacity(),
        head in any::<u32>(),
        offset in 0u32..256,
        sequence in steps(),
    ) {
        let tail = head.wrapping_add(offset.min(entries.entries()));
        let Some(mut ring) = Ring::adopt(entries, head, tail) else {
            prop_assert!(offset > entries.entries());
            return Ok(());
        };

        for step in sequence {
            match step {
                Step::Produce => { ring.produce(); }
                Step::Consume => { ring.consume(); }
            }
            let occupied = ring.occupied();
            let capacity = ring.capacity().entries();
            prop_assert!(occupied <= capacity);
            prop_assert_eq!(ring.is_empty(), occupied == 0);
            prop_assert_eq!(ring.is_full(), occupied == capacity);
        }
    }

    /// A mask error breaks this silently, since counters far apart can mask to one index.
    #[test]
    fn what_is_produced_is_consumed_in_the_same_order(
        entries in capacity(),
        head in any::<u32>(),
        sequence in steps(),
    ) {
        let Some(mut ring) = Ring::adopt(entries, head, head) else {
            return Ok(());
        };
        let mut produced = std::collections::VecDeque::new();

        for step in sequence {
            match step {
                Step::Produce => {
                    if let Some(slot) = ring.produce() {
                        prop_assert!(slot < entries.entries() as usize);
                        produced.push_back(slot);
                    } else {
                        prop_assert!(ring.is_full());
                    }
                }
                Step::Consume => match ring.consume() {
                    Some(slot) => prop_assert_eq!(Some(slot), produced.pop_front()),
                    None => prop_assert!(ring.is_empty()),
                },
            }
        }
    }

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

    /// A run of submit then complete never runs out of room, however long it is.
    #[test]
    fn a_reservation_is_returned_by_the_completion_it_was_taken_for(
        shift in 0u32..6,
        rounds in 1usize..200,
    ) {
        let submission = Capacity::new(1 << shift).expect("a power of two");
        let completion = Capacity::new(2 << shift).expect("a power of two");
        let mut queues = Queues::new(submission, completion).expect("a valid pair");

        for _ in 0..rounds {
            prop_assert!(queues.submit().is_some(), "a drained ring refused work");
            prop_assert!(queues.take().is_some());
            prop_assert!(queues.complete().is_some());
            prop_assert!(queues.reap().is_some());
            prop_assert_eq!(queues.outstanding(), 0);
        }
    }
}
