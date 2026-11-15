#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::fmt;

// Bumped whenever a reader-visible layout changes.
pub const VERSION: u32 = 1;

const PUBLICATION_WORD_BYTES: usize = size_of::<u32>();

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SharedLayoutError {
    Empty,
    PublicationRangeOverflow(usize),
    PublicationOutOfBounds(usize),
    PublicationMisaligned(usize),
    PublicationOverlap { first: usize, second: usize },
}

impl fmt::Display for SharedLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty => formatter.write_str("a shared-ring layout cannot be empty"),
            Self::PublicationRangeOverflow(at) => {
                write!(formatter, "the publication word at {at} overflows")
            }
            Self::PublicationOutOfBounds(at) => {
                write!(
                    formatter,
                    "the publication word at {at} leaves the allocation"
                )
            }
            Self::PublicationMisaligned(at) => {
                write!(formatter, "the publication word at {at} is misaligned")
            }
            Self::PublicationOverlap { first, second } => {
                write!(
                    formatter,
                    "publication words at {first} and {second} overlap"
                )
            }
        }
    }
}

impl core::error::Error for SharedLayoutError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SharedLayout<const WORDS: usize> {
    bytes: usize,
    publication_words: [usize; WORDS],
}

impl<const WORDS: usize> SharedLayout<WORDS> {
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

    pub const fn bytes(self) -> usize {
        self.bytes
    }

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

    pub const fn contains_payload(self, at: usize, length: usize) -> bool {
        let Some(end) = at.checked_add(length) else {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CapacityError {
    NotAPowerOfTwo(u32),
    TooLarge(u32),
}

impl fmt::Display for CapacityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotAPowerOfTwo(value) => write!(formatter, "{value} is not a power of two"),
            Self::TooLarge(value) => {
                write!(formatter, "{value} entries is more than a ring may hold")
            }
        }
    }
}

impl core::error::Error for CapacityError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capacity(u32);

impl Capacity {
    // Half the counter range, so `tail - head` is always a count.
    pub const MAX: u32 = 1 << 31;

    pub const fn new(entries: u32) -> Result<Self, CapacityError> {
        if entries == 0 || !entries.is_power_of_two() {
            return Err(CapacityError::NotAPowerOfTwo(entries));
        }
        if entries > Self::MAX {
            return Err(CapacityError::TooLarge(entries));
        }
        Ok(Self(entries))
    }

    pub const fn entries(self) -> u32 {
        self.0
    }

    pub const fn mask(self) -> u32 {
        self.0 - 1
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ring {
    capacity: Capacity,
    head: u32,
    tail: u32,
}

impl Ring {
    pub const fn new(capacity: Capacity) -> Self {
        Self {
            capacity,
            head: 0,
            tail: 0,
        }
    }

    pub const fn occupied(&self) -> u32 {
        self.tail.wrapping_sub(self.head)
    }

    pub const fn is_empty(&self) -> bool {
        self.occupied() == 0
    }

    pub const fn is_full(&self) -> bool {
        self.occupied() == self.capacity.entries()
    }

    pub const fn capacity(&self) -> Capacity {
        self.capacity
    }

    const fn produce(&mut self) -> Option<usize> {
        if self.is_full() {
            return None;
        }
        let slot = (self.tail & self.capacity.mask()) as usize;
        self.tail = self.tail.wrapping_add(1);
        Some(slot)
    }

    const fn consume(&mut self) -> Option<usize> {
        if self.is_empty() {
            return None;
        }
        let slot = (self.head & self.capacity.mask()) as usize;
        self.head = self.head.wrapping_add(1);
        Some(slot)
    }

    pub const fn head(&self) -> u32 {
        self.head
    }

    pub const fn tail(&self) -> u32 {
        self.tail
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PairError {
    CompletionTooSmall { submission: u32, completion: u32 },
}

impl fmt::Display for PairError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::CompletionTooSmall {
                submission,
                completion,
            } => write!(
                formatter,
                "a completion queue of {completion} is less than twice a submission queue of {submission}"
            ),
        }
    }
}

impl core::error::Error for PairError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Queues {
    submission: Ring,
    completion: Ring,
    reserved: u32,
}

impl Queues {
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

    pub const fn take(&mut self) -> Option<usize> {
        self.submission.consume()
    }

    // Every operation, cancelled ones included, must post exactly one.
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

    pub const fn outstanding(&self) -> u32 {
        self.reserved
    }

    pub const fn submissions(&self) -> &Ring {
        &self.submission
    }

    pub const fn completions(&self) -> &Ring {
        &self.completion
    }

    pub const fn is_consistent(&self) -> bool {
        (self.reserved as u64) + (self.completion.occupied() as u64)
            <= self.completion.capacity().entries() as u64
    }
}
