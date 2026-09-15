//! Submission/completion ring arithmetic and overflow-safe admission.
//!
//! Each accepted submission reserves a completion. The completion queue is a
//! power of two and at least twice the submission queue, while free-running
//! counters distinguish full from empty across wrap (D-79). This crate accepts
//! copied values, never references to user-writable shared memory (D-221).
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
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::mem::{align_of, size_of};

/// The version every ring structure carries.
///
/// Bumped whenever a reader-visible layout changes.
pub const VERSION: u32 = 1;

/// Bytes in one shared-memory publication word.
pub const PUBLICATION_WORD_BYTES: usize = size_of::<u32>();

/// Why a shared-ring layout cannot partition one allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SharedLayoutError {
    /// An allocation with no bytes cannot hold a ring.
    Empty,
    /// The publication word's end could not be represented.
    PublicationRangeOverflow(usize),
    /// The publication word leaves the shared allocation.
    PublicationOutOfBounds(usize),
    /// The publication word is not naturally aligned.
    PublicationMisaligned(usize),
    /// Two publication words name at least one common byte.
    PublicationOverlap {
        /// The earlier word in the supplied layout.
        first: usize,
        /// The overlapping word.
        second: usize,
    },
}

impl core::fmt::Display for SharedLayoutError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Empty => core::fmt::Write::write_str(f, "a shared-ring layout cannot be empty"),
            Self::PublicationRangeOverflow(at) => {
                write!(f, "the publication word at {at} overflows")
            }
            Self::PublicationOutOfBounds(at) => {
                write!(f, "the publication word at {at} leaves the allocation")
            }
            Self::PublicationMisaligned(at) => {
                write!(f, "the publication word at {at} is misaligned")
            }
            Self::PublicationOverlap { first, second } => {
                write!(f, "publication words at {first} and {second} overlap")
            }
        }
    }
}

impl core::error::Error for SharedLayoutError {}

/// A complete payload/publication partition of one shared ring allocation.
///
/// The listed naturally aligned words are the only offsets usable for
/// synchronized publication. Every other byte is payload; a payload range
/// intersecting any listed word is refused whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedLayout<const WORDS: usize> {
    bytes: usize,
    publication_words: [usize; WORDS],
}

impl<const WORDS: usize> SharedLayout<WORDS> {
    /// Validate the allocation length and every publication word.
    pub const fn new(
        bytes: usize,
        publication_words: [usize; WORDS],
    ) -> Result<Self, SharedLayoutError> {
        if bytes == 0 {
            return Err(SharedLayoutError::Empty);
        }

        let mut index = 0;
        while index < WORDS {
            let at = publication_words[index];
            let Some(end) = at.checked_add(PUBLICATION_WORD_BYTES) else {
                return Err(SharedLayoutError::PublicationRangeOverflow(at));
            };
            if end > bytes {
                return Err(SharedLayoutError::PublicationOutOfBounds(at));
            }
            if !at.is_multiple_of(align_of::<u32>()) {
                return Err(SharedLayoutError::PublicationMisaligned(at));
            }

            let mut earlier = 0;
            while earlier < index {
                let first = publication_words[earlier];
                let first_end = first + PUBLICATION_WORD_BYTES;
                if at < first_end && first < end {
                    return Err(SharedLayoutError::PublicationOverlap { first, second: at });
                }
                earlier += 1;
            }
            index += 1;
        }

        Ok(Self {
            bytes,
            publication_words,
        })
    }

    /// Bytes in the partitioned allocation.
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    /// Whether `at` is one exact admitted publication word.
    pub const fn contains_publication_word(self, at: usize) -> bool {
        let mut index = 0;
        while index < WORDS {
            if self.publication_words[index] == at {
                return true;
            }
            index += 1;
        }
        false
    }

    /// Whether every byte of `[at, at + len)` belongs to payload.
    pub const fn contains_payload(self, at: usize, len: usize) -> bool {
        let Some(end) = at.checked_add(len) else {
            return false;
        };
        if end > self.bytes {
            return false;
        }

        let mut index = 0;
        while index < WORDS {
            let word = self.publication_words[index];
            let word_end = word + PUBLICATION_WORD_BYTES;
            if at < word_end && word < end {
                return false;
            }
            index += 1;
        }
        true
    }
}

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
    /// [`CapacityError::NotAPowerOfTwo`] for zero or a non-power of two.
    ///
    /// [`CapacityError::TooLarge`] guards a wider future index type; no `u32`
    /// power of two exceeds [`MAX`](Self::MAX).
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
    /// This is the validation half of read-once-then-validate: occupancy greater
    /// than capacity is refused before indexing (D-79).
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
    /// The completion queue is smaller than twice the submission queue.
    CompletionTooSmall {
        /// Entries the submission queue was asked for.
        submission: u32,
        /// Entries the completion queue was asked for.
        completion: u32,
    },
}

impl core::fmt::Display for PairError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
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
/// Admission reserves report capacity, making completion overflow
/// unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Queues {
    submission: Ring,
    completion: Ring,
    reserved: u32,
}

impl Queues {
    /// A pair, if the completion queue is at least twice the submission queue.
    ///
    /// # Errors
    ///
    /// [`PairError::CompletionTooSmall`] when the completion ring cannot hold
    /// reservations for twice the submission ring.
    pub const fn new(submission: Capacity, completion: Capacity) -> Result<Self, PairError> {
        // Widening preserves the exact comparison when twice `Capacity::MAX`
        // no longer fits in `u32`; division on the completion side would round.
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
        // Posted and reserved completions both own slots; ignoring either can
        // admit work with nowhere to report its result.
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
            // Unreachable while the reservation invariant holds; a return
            // tells the caller that broke it instead of killing the machine.
            None => None,
        }
    }

    /// Hand the oldest completion to whoever is reading them.
    pub const fn reap(&mut self) -> Option<usize> {
        self.completion.consume()
    }

    /// Reap only a forward head within the completions actually posted.
    pub const fn reap_up_to(&mut self, claimed_head: u32) -> u32 {
        let asked = claimed_head.wrapping_sub(self.completion.head());
        if asked > self.completion.occupied() {
            return 0;
        }
        let mut reaped = 0;
        while reaped < asked {
            if self.completion.consume().is_none() {
                return reaped;
            }
            reaped += 1;
        }
        reaped
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
    /// Outstanding and posted completions together must fit the completion queue.
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

    /// Pin the externally visible version independently of its users (D-148).
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

        // Independent arithmetic: the head starts at
        // `u32::MAX - 3`, which is `0xFFFF_FFFC`, and `0xFFFF_FFFC & 7` is 4.
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
        // Widening must make twice the largest capacity a refusal, not wrap it.
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

    #[test]
    fn the_reservation_invariant_holds_under_interleaving() {
        let mut queues = Queues::new(capacity(2), capacity(4)).expect("a valid pair");
        let mut admitted = 0u32;
        let mut reported = 0u32;

        // A fixed walk makes failures reproducible without another dependency.
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

    #[test]
    fn every_power_of_two_a_u32_holds_is_accepted() {
        // Checked rather than reasoned: nothing a `u32` holds survives the
        // power-of-two refusal and then exceeds `MAX` (D-105).
        for shift in 0..u32::BITS {
            let entries = 1u32 << shift;
            assert!(Capacity::new(entries).is_ok(), "{entries} was refused");
        }
        assert_eq!(Capacity::MAX, 1 << 31);
    }
}
