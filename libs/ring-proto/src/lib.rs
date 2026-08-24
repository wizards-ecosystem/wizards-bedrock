//! The index arithmetic of a submission/completion ring, and the admission
//! control that makes overflow impossible.
//!
//! D-79 settled the shape; this is the half that is arithmetic, so it is here
//! where `cargo test` can reach it (D-41) and where D-84 puts it under
//! property and concurrency testing before the kernel has a ring at all.
//!
//! Three rules from D-79 are structural rather than advisory, and this crate
//! is where they are made unbreakable:
//!
//! - **Overflow is impossible by construction, not by mitigation.** A
//!   submission reserves its completion slot *before* it is accepted, which is
//!   Windows RIO's model. io_uring's `CQ_NODROP` instead took an unbounded
//!   kernel-memory liability for a misbehaving application. A ring that cannot
//!   admit work it has nowhere to report is a ring that cannot drop a
//!   completion.
//! - **The completion queue is a power of two and at least twice the
//!   submission queue.** Windows requires it; the reason is that a submission
//!   may complete more than once in flight terms while the ring is being
//!   drained, and the slack is what keeps admission from stalling on a
//!   consumer that is merely slow.
//! - **Counters are free-running and the index is a mask of them.** Head and
//!   tail count *operations*, not slots, so full and empty are different
//!   states rather than the same `head == tail`. That removes the sacrificial
//!   slot and, more importantly, makes every comparison a wrapping subtraction
//!   whose result is a count — which is the form that stays correct across the
//!   wrap at `u32::MAX`.
//!
//! Nothing here reads shared memory. A ring's real head and tail live in pages
//! userspace can write at any moment, so the kernel copies them out once and
//! validates the copy (D-79, CVE-2025-39816); the copy is what these functions
//! take. Keeping that boundary outside this crate is deliberate — it means no
//! function here can be handed a value it did not already have to trust.
//!
//! # Examples
//!
//! ```
//! use ring_proto::{Capacity, Ring};
//!
//! let mut ring = Ring::new(Capacity::new(4).expect("a power of two"));
//! assert!(ring.is_empty());
//!
//! for _ in 0..4 {
//!     assert!(ring.produce().is_some());
//! }
//! assert!(ring.is_full());
//! assert_eq!(ring.produce(), None, "a full ring admits nothing");
//!
//! // Consuming makes room again, and the slot index wraps.
//! assert_eq!(ring.consume(), Some(0));
//! assert_eq!(ring.produce(), Some(0));
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// The version every ring structure carries.
///
/// IoRing's improvement over io_uring, adopted by D-06: a shared structure
/// with no version field can only be extended by guessing. Bumped when the
/// layout changes in any way a reader could notice.
pub const VERSION: u32 = 1;

/// Why a ring could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapacityError {
    /// Zero, or not a power of two. The mask arithmetic needs one.
    NotAPowerOfTwo(u32),
    /// Large enough that the free-running counters could not distinguish a
    /// full ring from an empty one.
    TooLarge(u32),
}

impl core::fmt::Display for CapacityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::NotAPowerOfTwo(value) => write!(f, "{value} is not a power of two"),
            Self::TooLarge(value) => write!(f, "{value} entries is more than a ring may hold"),
        }
    }
}

impl core::error::Error for CapacityError {}

/// How many entries a ring holds: a power of two, and not too many.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity(u32);

impl Capacity {
    /// The largest a ring may be.
    ///
    /// Half the counter range, so that `tail - head` is always a count and
    /// never an ambiguity: with a larger ring, a producer far enough ahead of
    /// a consumer would wrap into looking behind it.
    pub const MAX: u32 = 1 << 31;

    /// A capacity of `entries`, or why not.
    ///
    /// # Errors
    ///
    /// [`CapacityError::NotAPowerOfTwo`] for zero or a non-power of two, and
    /// [`CapacityError::TooLarge`] above [`MAX`](Self::MAX).
    pub const fn new(entries: u32) -> Result<Self, CapacityError> {
        if entries == 0 || !entries.is_power_of_two() {
            return Err(CapacityError::NotAPowerOfTwo(entries));
        }
        if entries > Self::MAX {
            return Err(CapacityError::TooLarge(entries));
        }
        Ok(Self(entries))
    }

    /// How many entries.
    pub const fn entries(self) -> u32 {
        self.0
    }

    /// The mask that turns a counter into a slot index.
    pub const fn mask(self) -> u32 {
        self.0 - 1
    }
}

/// One ring's head and tail, as counts of operations rather than slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ring {
    capacity: Capacity,
    head: u32,
    tail: u32,
}

impl Ring {
    /// An empty ring of `capacity`.
    pub const fn new(capacity: Capacity) -> Self {
        Self {
            capacity,
            head: 0,
            tail: 0,
        }
    }

    /// A ring whose counters came from somewhere else, if they are consistent.
    ///
    /// The validation half of read-once-then-validate (D-79). The head and
    /// tail of a real ring live in memory userspace can write, so a kernel
    /// that has copied them out must still establish that they mean something
    /// before using them: `tail - head` greater than the capacity is not a
    /// large ring, it is a lie, and treating it as a count is how a shared-ring
    /// kernel writes outside its own buffer.
    ///
    /// # Errors
    ///
    /// `None` when the counters describe more outstanding entries than the
    /// ring can hold.
    pub const fn adopt(capacity: Capacity, head: u32, tail: u32) -> Option<Self> {
        if tail.wrapping_sub(head) > capacity.entries() {
            return None;
        }
        Some(Self {
            capacity,
            head,
            tail,
        })
    }

    /// How many entries are waiting.
    ///
    /// A wrapping subtraction, which is what keeps this a count rather than a
    /// comparison: it is correct across the wrap at `u32::MAX`, where `tail`
    /// is numerically below `head` and the ring is not empty at all.
    pub const fn occupied(&self) -> u32 {
        self.tail.wrapping_sub(self.head)
    }

    /// How many more entries would fit.
    pub const fn vacant(&self) -> u32 {
        self.capacity.entries() - self.occupied()
    }

    /// Whether it holds nothing.
    pub const fn is_empty(&self) -> bool {
        self.occupied() == 0
    }

    /// Whether it holds all it can.
    pub const fn is_full(&self) -> bool {
        self.occupied() == self.capacity.entries()
    }

    /// How many entries it holds when full.
    pub const fn capacity(&self) -> Capacity {
        self.capacity
    }

    /// Where the next entry goes, advancing the tail — or `None` if full.
    pub const fn produce(&mut self) -> Option<usize> {
        if self.is_full() {
            return None;
        }
        let slot = (self.tail & self.capacity.mask()) as usize;
        self.tail = self.tail.wrapping_add(1);
        Some(slot)
    }

    /// Where the oldest entry is, advancing the head — or `None` if empty.
    pub const fn consume(&mut self) -> Option<usize> {
        if self.is_empty() {
            return None;
        }
        let slot = (self.head & self.capacity.mask()) as usize;
        self.head = self.head.wrapping_add(1);
        Some(slot)
    }

    /// The head counter, for handing back to shared memory.
    pub const fn head(&self) -> u32 {
        self.head
    }

    /// The tail counter, for handing back to shared memory.
    pub const fn tail(&self) -> u32 {
        self.tail
    }
}

/// Why a pair of rings could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairError {
    /// One of the two capacities was not usable.
    Capacity(CapacityError),
    /// The completion queue is smaller than twice the submission queue.
    CompletionTooSmall {
        /// Entries the submission queue was asked for.
        submission: u32,
        /// Entries the completion queue was asked for.
        completion: u32,
    },
}

impl From<CapacityError> for PairError {
    fn from(error: CapacityError) -> Self {
        Self::Capacity(error)
    }
}

impl core::fmt::Display for PairError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Capacity(error) => write!(f, "{error}"),
            Self::CompletionTooSmall {
                submission,
                completion,
            } => write!(
                f,
                "a completion queue of {completion} is less than twice a submission queue of {submission}"
            ),
        }
    }
}

impl core::error::Error for PairError {}

/// A submission queue and the completion queue it reports into.
///
/// The whole point of the pair being one type: an operation is admitted only
/// when there is somewhere to report it, so **a completion can never be
/// dropped for want of room**. That is Windows RIO's reservation model rather
/// than io_uring's overflow list, and the difference is that overflow here is
/// unrepresentable rather than handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Queues {
    submission: Ring,
    completion: Ring,
    /// Operations admitted and not yet reported. Each holds a completion slot
    /// that nothing else may take.
    reserved: u32,
}

impl Queues {
    /// A pair, if the completion queue is at least twice the submission queue.
    ///
    /// # Errors
    ///
    /// [`PairError::Capacity`] for a bad capacity, and
    /// [`PairError::CompletionTooSmall`] otherwise.
    pub const fn new(submission: Capacity, completion: Capacity) -> Result<Self, PairError> {
        // Widened, because doubling `Capacity::MAX` does not fit in a `u32` —
        // the comment that used to be here said it could not overflow, and the
        // test for the largest submission queue is what said otherwise. A
        // division on the completion side would have rounded instead.
        if (completion.entries() as u64) < (submission.entries() as u64) * 2 {
            return Err(PairError::CompletionTooSmall {
                submission: submission.entries(),
                completion: completion.entries(),
            });
        }
        Ok(Self {
            submission: Ring::new(submission),
            completion: Ring::new(completion),
            reserved: 0,
        })
    }

    /// Admit one operation, reserving the completion slot it will need.
    ///
    /// `None` when the submission queue is full **or** when every completion
    /// slot is already spoken for. The second is the case that matters: it is
    /// where a ring that would otherwise have to drop a completion instead
    /// declines the work.
    pub const fn submit(&mut self) -> Option<usize> {
        // Both halves, and the second is the one an interleaving test found
        // missing: a completion already posted and not yet reaped occupies a
        // slot just as firmly as a reservation does. Counting only
        // reservations admitted work into a queue that was already full, and
        // the dropped completion this model exists to make impossible would
        // have happened anyway.
        if self.reserved + self.completion.occupied() >= self.completion.capacity().entries() {
            return None;
        }
        match self.submission.produce() {
            Some(slot) => {
                self.reserved += 1;
                Some(slot)
            }
            None => None,
        }
    }

    /// Take the oldest submission for processing.
    ///
    /// Its reservation stays held: the operation is in flight, and the slot it
    /// will report into is still its own.
    pub const fn take(&mut self) -> Option<usize> {
        self.submission.consume()
    }

    /// Report one operation's terminal completion, spending its reservation.
    ///
    /// `None` only when nothing is outstanding, which is a caller bug rather
    /// than a full queue — the reservation taken at [`submit`](Self::submit)
    /// is what guarantees the room. **Every operation posts exactly one of
    /// these, always**, cancelled ones included (D-79).
    pub const fn complete(&mut self) -> Option<usize> {
        if self.reserved == 0 {
            return None;
        }
        match self.completion.produce() {
            Some(slot) => {
                self.reserved -= 1;
                Some(slot)
            }
            // Unreachable while the reservation invariant holds, and returning
            // rather than panicking so a caller that broke it is told instead
            // of killing the machine.
            None => None,
        }
    }

    /// Hand the oldest completion to whoever is reading them.
    pub const fn reap(&mut self) -> Option<usize> {
        self.completion.consume()
    }

    /// Operations admitted and not yet reported.
    pub const fn outstanding(&self) -> u32 {
        self.reserved
    }

    /// The submission ring.
    pub const fn submissions(&self) -> &Ring {
        &self.submission
    }

    /// The completion ring.
    pub const fn completions(&self) -> &Ring {
        &self.completion
    }

    /// Whether the reservation invariant still holds.
    ///
    /// Every outstanding operation owns a completion slot, and the slots
    /// already filled are owned by operations that have reported. So the two
    /// together can never exceed the completion queue — if they do, a
    /// completion is going to be dropped, which this design says cannot
    /// happen.
    pub const fn is_consistent(&self) -> bool {
        (self.reserved as u64) + (self.completion.occupied() as u64)
            <= self.completion.capacity().entries() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capacity(entries: u32) -> Capacity {
        Capacity::new(entries).expect("a usable capacity")
    }

    /// Only powers of two, and not more than half the counter range.
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
        // A power of two above half the range still cannot be counted.
        assert_eq!(
            Capacity::new(Capacity::MAX),
            Ok(Capacity(Capacity::MAX)),
            "the largest usable ring was refused"
        );
        assert_eq!(Capacity::MAX.checked_mul(2), None, "MAX must be 2^31");
    }

    /// **Full and empty are different states**, which is the whole reason the
    /// counters count operations rather than slots.
    ///
    /// A ring that stored slot indices would have `head == tail` for both, and
    /// would have to sacrifice an entry to tell them apart.
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

    /// Every slot is handed out once per lap, in order, and the lap repeats.
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

    /// **The wrap at `u32::MAX` is where a comparison would be wrong and a
    /// wrapping subtraction is right.**
    ///
    /// Driven right through the boundary: at the moment the tail wraps past
    /// zero it is numerically *below* the head, and a ring that asked
    /// `tail > head` would call a busy ring empty.
    #[test]
    fn the_counters_survive_the_wrap() {
        let capacity = capacity(8);
        let mut ring = Ring::adopt(capacity, u32::MAX - 3, u32::MAX - 3).expect("consistent");

        // Fill it, which takes the tail through zero.
        for _ in 0..8 {
            assert!(ring.produce().is_some());
        }
        assert!(ring.is_full(), "the ring lost count across the wrap");
        assert!(
            ring.tail() < ring.head(),
            "this test is only meaningful once the tail has wrapped below the head"
        );
        assert_eq!(ring.occupied(), 8);

        // And drains cleanly, in the same slot order as any other lap.
        // Worked out by hand rather than from the code: the head starts at
        // `u32::MAX - 3`, which is `0xFFFF_FFFC`, and `0xFFFF_FFFC & 7` is 4.
        let slots: Vec<usize> = (0..8).filter_map(|_| ring.consume()).collect();
        assert_eq!(slots, [4, 5, 6, 7, 0, 1, 2, 3]);
        assert!(ring.is_empty());
    }

    /// Counters that describe more entries than the ring holds are refused.
    ///
    /// The validation half of read-once-then-validate: a real head and tail
    /// live where userspace can write them, so "more outstanding than the ring
    /// can hold" is a lie to reject rather than a number to trust.
    #[test]
    fn impossible_counters_are_refused() {
        let capacity = capacity(4);
        assert!(Ring::adopt(capacity, 0, 4).is_some());
        assert_eq!(Ring::adopt(capacity, 0, 5), None);
        assert_eq!(Ring::adopt(capacity, 0, u32::MAX), None);
        // A wrapped-but-legal pair is still accepted.
        assert!(Ring::adopt(capacity, u32::MAX, 2).is_some());
        // A wrapped-and-impossible one is not.
        assert_eq!(Ring::adopt(capacity, u32::MAX, 10), None);
    }

    /// The completion queue must be at least twice the submission queue.
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
        // And the largest submission queue cannot be paired at all, because
        // twice it does not exist. Better a refusal than an overflow that
        // makes an undersized pair look acceptable.
        assert!(Queues::new(capacity(Capacity::MAX), capacity(Capacity::MAX)).is_err());
    }

    /// **A completion can never be dropped**, which is the reservation model's
    /// whole claim.
    ///
    /// Driven to the limit: admit until refused, complete everything, and at
    /// no point may the reservation invariant break. io_uring's answer to this
    /// situation was an overflow list and an unbounded kernel-memory
    /// liability; here the work is declined instead.
    #[test]
    fn admission_stops_before_a_completion_could_be_dropped() {
        let mut queues = Queues::new(capacity(4), capacity(8)).expect("a valid pair");

        // Fill the submission queue.
        for _ in 0..4 {
            assert!(queues.submit().is_some());
            assert!(queues.is_consistent());
        }
        assert_eq!(queues.submit(), None, "admitted past the submission queue");
        assert_eq!(queues.outstanding(), 4);

        // Drain them into flight; the reservations are still held.
        for _ in 0..4 {
            assert!(queues.take().is_some());
        }
        assert_eq!(
            queues.outstanding(),
            4,
            "taking work released a reservation"
        );

        // Four more can be admitted, and that exhausts the completion queue's
        // eight slots — not the submission queue's four.
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

        // Every one of them reports, and none is refused for want of room.
        for _ in 0..8 {
            assert!(queues.complete().is_some(), "a completion was dropped");
            assert!(queues.is_consistent());
        }
        assert_eq!(queues.outstanding(), 0);
        assert_eq!(queues.complete(), None, "reported more than was admitted");
    }

    /// The invariant holds under any interleaving of the four operations.
    ///
    /// A small deterministic sweep rather than a proof — D-84 puts the real
    /// one in TLA+ and under `shuttle`. What this catches is an ordering the
    /// hand-written test above never tries.
    #[test]
    fn the_reservation_invariant_holds_under_interleaving() {
        let mut queues = Queues::new(capacity(2), capacity(4)).expect("a valid pair");
        let mut admitted = 0u32;
        let mut reported = 0u32;

        // A fixed pseudo-random walk: cheap, reproducible, and no dependency.
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

    /// Completing without an outstanding operation is refused, not counted.
    ///
    /// The reservation is what proves there is room, so a completion with no
    /// reservation behind it is a caller bug — and turning it into a returned
    /// `None` rather than a slot is what stops it becoming a dropped
    /// completion for something else.
    #[test]
    fn a_completion_with_no_reservation_is_refused() {
        let mut queues = Queues::new(capacity(2), capacity(4)).expect("a valid pair");
        assert_eq!(queues.complete(), None);
        assert!(queues.completions().is_empty());
        assert!(queues.is_consistent());
    }
}
