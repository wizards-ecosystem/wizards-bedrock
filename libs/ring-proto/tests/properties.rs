//! Property tests over the ring's index arithmetic.
//!
//! The unit tests in the crate pin the values a reader can check by hand; these
//! ask whether the invariants hold over sequences nobody would think to write.
//! D-84 adopted both tiers for this crate specifically, because the queue ABI
//! is the interface this kernel will live with longest and the one place where
//! writing code before checking the design would waste the most work.
//!
//! What is *not* here: anything about shared memory. A ring's real head and
//! tail live in pages userspace can write at any moment, and the crate takes
//! copies rather than reading them — so a property test that reasoned about
//! concurrent writers would be testing a boundary this crate does not have.
//! The interleavings are `interleavings.rs`'s, under shuttle.

use proptest::prelude::*;
use ring_proto::{Capacity, Queues, Ring};

/// One thing a test can do to a ring.
#[derive(Debug, Clone, Copy)]
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
    /// Occupancy stays within capacity, and both boundary predicates agree.
    ///
    /// The arithmetic is wrapping subtraction of free-running counters, so the
    /// interesting inputs are the ones that cross `u32::MAX` — which no
    /// hand-written sequence reaches and which `adopt` can start at directly.
    #[test]
    fn occupancy_and_boundary_predicates_remain_consistent(
        entries in capacity(),
        head in any::<u32>(),
        offset in 0u32..256,
        sequence in steps(),
    ) {
        // Start anywhere, including across the wrap.
        let tail = head.wrapping_add(offset.min(entries.entries()));
        let Some(mut ring) = Ring::adopt(entries, head, tail) else {
            // `adopt` refuses a tail more than a capacity ahead, which the
            // clamp above should have prevented.
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

    /// A produce that returns a slot is one the next consume returns, in order.
    ///
    /// The property that makes the ring a queue rather than a bag, and the one
    /// a mask error breaks silently: two counters far apart can mask to the
    /// same index and still compare correctly.
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

    /// No sequence of submissions and completions can overflow the completion
    /// queue.
    ///
    /// D-79's structural claim, and the reason admission reserves before it
    /// accepts (Windows RIO's model rather than io_uring's `CQ_NODROP`). If a
    /// submission could be admitted with nowhere to report it, some sequence
    /// here would find it.
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

    /// Every operation admitted can be completed, and completing one frees the
    /// room the next submission needs.
    ///
    /// Exactly one terminal completion per operation is D-79's other structural
    /// rule. This is the arithmetic half of it: the reservation a submission
    /// takes is the reservation its completion returns, so a run of
    /// submit-then-complete never runs out of room however long it is.
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
