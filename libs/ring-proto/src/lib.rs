//! Submission and completion ring arithmetic, over copied values, never shared memory.
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

use core::fmt;

/// The version every ring structure carries, bumped whenever a reader-visible layout changes.
pub const VERSION: u32 = 1;

/// Bytes in one shared-memory publication word.
pub const PUBLICATION_WORD_BYTES: usize = size_of::<u32>();

/// Why a shared-ring layout cannot partition one allocation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

impl fmt::Display for SharedLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty => f.write_str("a shared-ring layout cannot be empty"),
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

/// One shared ring allocation, split into its aligned publication words and payload.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum CapacityError {
    /// Zero, or not a power of two, which the mask arithmetic needs.
    NotAPowerOfTwo(u32),
    /// Too large for the free-running counters to tell a full ring from an empty one.
    TooLarge(u32),
}

impl fmt::Display for CapacityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotAPowerOfTwo(value) => write!(f, "{value} is not a power of two"),
            Self::TooLarge(value) => write!(f, "{value} entries is more than a ring may hold"),
        }
    }
}

impl core::error::Error for CapacityError {}

/// How many entries a ring holds: a power of two, and not too many.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capacity(u32);

impl Capacity {
    /// Half the counter range, so that `tail - head` is always a count.
    pub const MAX: u32 = 1 << 31;

    /// A capacity of `entries`, or why not.
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

    /// A ring from counters read elsewhere, or `None` if they hold more than `capacity`.
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

    /// How many entries are waiting, correct across the wrap at `u32::MAX`.
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

    /// Where the next entry goes, advancing the tail, or `None` if full.
    pub const fn produce(&mut self) -> Option<usize> {
        if self.is_full() {
            return None;
        }
        let slot = (self.tail & self.capacity.mask()) as usize;
        self.tail = self.tail.wrapping_add(1);
        Some(slot)
    }

    /// Where the oldest entry is, advancing the head, or `None` if empty.
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

impl fmt::Display for PairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

/// A submission queue and the completion queue it reports into, with every completion reserved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Queues {
    submission: Ring,
    completion: Ring,
    reserved: u32,
}

impl Queues {
    /// A pair, if the completion queue is at least twice the submission queue.
    pub const fn new(submission: Capacity, completion: Capacity) -> Result<Self, PairError> {
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

    /// Admit one operation and reserve its completion, or `None` when either queue has no room.
    pub const fn submit(&mut self) -> Option<usize> {
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

    /// Take the oldest submission for processing; its reservation stays held.
    pub const fn take(&mut self) -> Option<usize> {
        self.submission.consume()
    }

    /// Post one operation's terminal completion, cancelled ones included, spending its reservation.
    pub const fn complete(&mut self) -> Option<usize> {
        if self.reserved == 0 {
            return None;
        }
        match self.completion.produce() {
            Some(slot) => {
                self.reserved -= 1;
                Some(slot)
            }
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

    /// Whether outstanding and posted completions together still fit the completion queue.
    pub const fn is_consistent(&self) -> bool {
        (self.reserved as u64) + (self.completion.occupied() as u64)
            <= self.completion.capacity().entries() as u64
    }
}

#[cfg(test)]
mod tests;
