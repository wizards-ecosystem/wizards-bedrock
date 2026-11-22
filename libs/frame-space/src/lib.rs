#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod colour;
// The crate's only unverified code: Verus cannot check formatting.
#[cfg_attr(verus_only, verifier::external)]
mod fmt;
pub mod proof;
mod record;
mod reserve;

use core::num::NonZeroU64;

#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus, verus_spec, verus_verify};

pub use colour::{ColourCounts, ColourMask, Colours, MAX_COLOURS};
pub use record::{Access, ArenaIndex, FrameKind, LINKED_FRAMES, Link, Owner, Record};
pub use reserve::{Pool, Reservation};

#[verus_verify]
pub const FRAME_SIZE: u64 = 4096;

#[verus_verify]
const FRAMES_PER_WORD: usize = 64;

// Biased by one so frame zero and `Option` both fit a word.
#[verus_spec(r =>
    ensures
        r.is_some() == (address % 4096 == 0),
)]
pub const fn frame_identity(address: u64) -> Option<NonZeroU64> {
    if !address.is_multiple_of(FRAME_SIZE) {
        return None;
    }
    NonZeroU64::new(address / FRAME_SIZE + 1)
}

#[verus_spec(r =>
    ensures
        (bytes as int + 4095) / 4096 <= usize::MAX ==> r as int == ((bytes as int + 4095) / 4096
            + 63) / 64,
)]
pub const fn words_for(bytes: u64) -> usize {
    let partial = if bytes.is_multiple_of(FRAME_SIZE) {
        0
    } else {
        1
    };
    let frames = (bytes / FRAME_SIZE + partial) as usize;
    let spare = if frames.is_multiple_of(FRAMES_PER_WORD) {
        0
    } else {
        1
    };
    frames / FRAMES_PER_WORD + spare
}

#[verus_spec(r =>
    ensures
        r == 1u64 << (frame % 64) as u64,
)]
const fn bit(frame: usize) -> u64 {
    1 << ((frame % FRAMES_PER_WORD) as u64)
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FreeError {
    NotManaged(usize),
    AlreadyFree(usize),
    NotCharged(usize),
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Inserted {
    pub ignored: usize,
}

verus! {

pub struct FrameSpace<const WORDS: usize> {
    managed: [u64; WORDS],
    // Always a subset of `managed`.
    free: [u64; WORDS],
    // Free frames by index modulo 64; a colour's free count sums the positions of that colour.
    free_by_bit: [usize; 64],
    managed_count: usize,
    // One past the highest managed frame, so a search never walks the empty top of the domain.
    limit: usize,
    hint: usize,
}

// The type invariant is checked after each call that borrows a field, so one step changes every
// field the invariant ties together.
fn mark_managed<const WORDS: usize>(
    managed: &mut [u64; WORDS],
    count: &mut usize,
    limit: &mut usize,
    frame: usize,
)
    requires
        frame / 64 < WORDS,
        *old(count) < usize::MAX,
        frame < usize::MAX,
    ensures
        final(managed)@ == proof::with_bit(old(managed)@, frame as int),
        *final(count) == *old(count) + 1,
        *final(limit) == if *old(limit) < frame + 1 {
            (frame + 1) as usize
        } else {
            *old(limit)
        },
    no_unwind
{
    let index = frame / FRAMES_PER_WORD;
    managed[index] |= 1u64 << ((frame % FRAMES_PER_WORD) as u64);
    *count += 1;
    if *limit < frame + 1 {
        *limit = frame + 1;
    }
}

fn mark_free<const WORDS: usize>(free: &mut [u64; WORDS], by_bit: &mut [usize; 64], frame: usize)
    requires
        frame / 64 < WORDS,
        old(by_bit)@[frame as int % 64] < usize::MAX,
    ensures
        final(free)@ == proof::with_bit(old(free)@, frame as int),
        final(by_bit)@ == old(by_bit)@.update(
            frame as int % 64,
            (old(by_bit)@[frame as int % 64] + 1) as usize,
        ),
    no_unwind
{
    let index = frame / FRAMES_PER_WORD;
    let position = frame % FRAMES_PER_WORD;
    free[index] |= 1u64 << (position as u64);
    by_bit[position] += 1;
}

fn mark_taken<const WORDS: usize>(free: &mut [u64; WORDS], by_bit: &mut [usize; 64], frame: usize)
    requires
        frame / 64 < WORDS,
        old(by_bit)@[frame as int % 64] >= 1,
    ensures
        final(free)@ == proof::without_bit(old(free)@, frame as int),
        final(by_bit)@ == old(by_bit)@.update(
            frame as int % 64,
            (old(by_bit)@[frame as int % 64] - 1) as usize,
        ),
    no_unwind
{
    let index = frame / FRAMES_PER_WORD;
    let position = frame % FRAMES_PER_WORD;
    free[index] &= !(1u64 << (position as u64));
    by_bit[position] -= 1;
}

impl<const WORDS: usize> FrameSpace<WORDS> {
    // Words past this bound would number frames beyond `usize`, so they hold none.
    const fn frame_count() -> (r: usize)
        ensures
            r == proof::frame_count(WORDS),
    {
        let words = if WORDS < usize::MAX / FRAMES_PER_WORD {
            WORDS
        } else {
            usize::MAX / FRAMES_PER_WORD
        };
        words * FRAMES_PER_WORD
    }

    pub const fn new() -> (r: Self)
        ensures
            forall|frame: int| !#[trigger] r.is_managed_frame(frame),
            r.managed_total() == 0,
    {
        let managed = [0u64; WORDS];
        let free = [0u64; WORDS];
        let free_by_bit = [0usize; 64];
        proof {
            proof::lemma_parts_empty(WORDS, managed@, free@, free_by_bit@);
        }
        Self { managed, free, free_by_bit, managed_count: 0, limit: 0, hint: 0 }
    }

    pub fn free_count(&self) -> (r: usize)
        ensures
            r == self.free_total(),
            r <= self.managed_total(),
    {
        proof {
            use_type_invariant(self);
            self.lemma_free_bounded();
        }
        let mut total: usize = 0;
        let mut position = 0;
        while position < FRAMES_PER_WORD
            invariant
                position <= 64,
                self.ownership(),
                total == proof::strided(self.free_by_bit@, 1, 0, position as nat),
            decreases 64 - position,
        {
            proof {
                proof::lemma_strided_prefix(self.free_by_bit@, 1, 0, (position + 1) as nat, 64);
                self.lemma_free_bounded();
            }
            total += self.free_by_bit[position];
            position += 1;
        }
        total
    }

    pub fn free_in(&self, colours: Colours, colour: usize) -> (r: usize)
        ensures
            r == self.free_of_colour(colours.spec_count(), colour as int),
    {
        proof {
            use_type_invariant(self);
        }
        let stride = colours.count();
        if colour >= stride {
            return 0;
        }
        let mut total: usize = 0;
        let mut position = colour;
        proof {
            vstd::arithmetic::div_mod::lemma_small_mod(colour as nat, stride as nat);
            proof::lemma_strided_below(self.free_by_bit@, stride as nat, colour as nat, colour as nat);
        }
        while position < FRAMES_PER_WORD
            invariant
                self.ownership(),
                0 < stride <= 64,
                colour < stride,
                position % stride == colour,
                position < 64 + stride,
                total == proof::strided(
                    self.free_by_bit@,
                    stride as nat,
                    colour as nat,
                    if position < 64 {
                        position as nat
                    } else {
                        64
                    },
                ),
            decreases 64 + stride - position,
        {
            proof {
                self.lemma_free_bounded();
                proof::lemma_strided_step(
                    self.free_by_bit@,
                    stride as nat,
                    colour as nat,
                    position as nat,
                    1,
                );
                proof::lemma_strided_prefix(
                    self.free_by_bit@,
                    stride as nat,
                    colour as nat,
                    (position + 1) as nat,
                    64,
                );
                proof::lemma_strided_bound(self.free_by_bit@, stride as nat, colour as nat, 64);
                proof::lemma_strided_step(
                    self.free_by_bit@,
                    stride as nat,
                    colour as nat,
                    position as nat,
                    if position + stride <= 64 {
                        stride as nat
                    } else {
                        (64 - position) as nat
                    },
                );
            }
            total += self.free_by_bit[position];
            proof {
                vstd::arithmetic::div_mod::lemma_mod_add_multiples_vanish(position as int, stride as int);
            }
            position += stride;
        }
        total
    }

    pub const fn managed_count(&self) -> (r: usize)
        ensures
            r == self.managed_total(),
    {
        self.managed_count
    }

    pub const fn limit(&self) -> (r: usize)
        ensures
            forall|frame: int| #[trigger] self.is_managed_frame(frame) ==> frame < r,
    {
        proof {
            use_type_invariant(self);
        }
        self.limit
    }

    pub const fn is_free(&self, frame: usize) -> (r: bool)
        ensures
            r == self.is_free_frame(frame as int),
    {
        if frame / FRAMES_PER_WORD < WORDS {
            proof {
                proof::lemma_test_bit(self.free@[frame as int / 64], (frame % 64) as u64);
            }
            self.free[frame / FRAMES_PER_WORD] & bit(frame) != 0
        } else {
            false
        }
    }

    const fn is_managed(&self, frame: usize) -> (r: bool)
        ensures
            r == self.is_managed_frame(frame as int),
    {
        if frame / FRAMES_PER_WORD < WORDS {
            proof {
                proof::lemma_test_bit(self.managed@[frame as int / 64], (frame % 64) as u64);
            }
            self.managed[frame / FRAMES_PER_WORD] & bit(frame) != 0
        } else {
            false
        }
    }

    pub fn insert(&mut self, base: u64, length: u64) -> (inserted: Inserted)
        ensures
            forall|frame: int|
                #[trigger] final(self).is_managed_frame(frame) == (old(self).is_managed_frame(frame)
                    || proof::inserted(WORDS, base, length, frame)),
            forall|frame: int|
                #[trigger] final(self).is_free_frame(frame) == (old(self).is_free_frame(frame) || (
                proof::inserted(WORDS, base, length, frame) && !old(self).is_managed_frame(
                    frame,
                ))),
    {
        let mut inserted = Inserted { ignored: 0 };
        let Some(end) = base.checked_add(length) else {
            return inserted;
        };

        let first = base / FRAME_SIZE + if base.is_multiple_of(FRAME_SIZE) {
            0
        } else {
            1
        };
        let last = end / FRAME_SIZE;
        // Counted arithmetically so a corrupt region length cannot become a huge loop.
        let domain = Self::frame_count() as u64;
        let clamp = if last < domain {
            last
        } else {
            domain
        };
        let floor = if clamp > first {
            clamp
        } else {
            first
        };
        inserted.ignored = last.saturating_sub(floor) as usize;
        let mut frame = first;
        while frame < clamp
            invariant
                first <= frame,
                frame == first || frame <= clamp,
                clamp <= proof::frame_count(WORDS),
                forall|other: int|
                    #[trigger] proof::inserted(WORDS, base, length, other) == (first <= other
                        < clamp),
                forall|other: int|
                    #[trigger] self.is_managed_frame(other) == (old(self).is_managed_frame(other)
                        || (first <= other < frame)),
                forall|other: int|
                    #[trigger] self.is_free_frame(other) == (old(self).is_free_frame(other) || (
                    first <= other < frame && !old(self).is_managed_frame(other))),
            decreases clamp - frame,
        {
            let index = frame as usize;
            if !self.is_managed(index) {
                self.manage(index);
                self.release(index);
            }
            frame += 1;
        }
        inserted
    }

    fn manage(&mut self, frame: usize)
        requires
            (frame as int) < proof::frame_count(WORDS),
            !old(self).is_managed_frame(frame as int),
        ensures
            forall|other: int|
                #[trigger] final(self).is_managed_frame(other) == (old(self).is_managed_frame(other)
                    || other == frame),
            forall|other: int| #[trigger]
                final(self).is_free_frame(other) == old(self).is_free_frame(other),
    {
        proof {
            use_type_invariant(&*self);
            proof::lemma_parts_manage(
                WORDS,
                self.managed@,
                self.free@,
                self.free_by_bit@,
                self.managed_count,
                self.limit,
                frame as int,
            );
        }
        mark_managed(&mut self.managed, &mut self.managed_count, &mut self.limit, frame);
    }

    pub fn allocate(&mut self) -> (r: Option<usize>)
        ensures
            final(self).allocated_run(*old(self), r, 1),
    {
        self.allocate_contiguous(1, 1)
    }

    pub fn allocate_contiguous(&mut self, count: usize, align: usize) -> (r: Option<usize>)
        ensures
            final(self).allocated_run(*old(self), r, count),
            r matches Some(first) ==> proof::is_power_of_two(align) ==> first % align == 0,
    {
        proof {
            use_type_invariant(&*self);
        }
        if count == 0 || count > self.limit {
            return None;
        }
        proof {
            if proof::is_power_of_two(align) {
                proof::lemma_power_positive(align);
            }
        }
        let align = next_power_of_two(if align > 1 {
            align
        } else {
            1
        })?;
        // The wrap pass reaches `count - 1` past the hint, so a run straddling it is found.
        let reach = self.hint.saturating_add(count - 1);
        let wrap_end = if reach < self.limit {
            reach
        } else {
            self.limit
        };
        let found = match self.search(self.hint, self.limit, count, align) {
            Some(found) => found,
            None => self.search(0, wrap_end, count, align)?,
        };
        let end = found + count;
        let ghost before = *self;
        let mut frame = found;
        while frame < end
            invariant
                found <= frame <= end,
                end <= before.limit,
                end == found + count,
                self.limit == before.limit,
                self.hint == before.hint,
                self.managed_count == before.managed_count,
                forall|other: int| #[trigger]
                    self.is_managed_frame(other) == before.is_managed_frame(other),
                forall|other: int|
                    #[trigger] self.is_free_frame(other) == (before.is_free_frame(other) && !(found
                        <= other < frame)),
                forall|other: int| found <= other < end ==> #[trigger] before.is_free_frame(other),
            decreases end - frame,
        {
            self.take(frame);
            frame += 1;
        }
        proof {
            use_type_invariant(&*self);
        }
        let ghost taken = *self;
        self.hint = end;
        proof {
            assert forall|other: int| #[trigger] self.is_free_frame(other) == taken.is_free_frame(
                other,
            ) && self.is_managed_frame(other) == taken.is_managed_frame(other) by {}
            assert forall|other: int| #[trigger] self.is_managed_frame(other)
                == before.is_managed_frame(other) by {
                assert(taken.is_managed_frame(other) == before.is_managed_frame(other));
            }
            assert forall|other: int| #[trigger] self.is_free_frame(other) == (
            before.is_free_frame(other) && !(found <= other < end)) by {
                assert(taken.is_free_frame(other) == (before.is_free_frame(other) && !(found
                    <= other < end)));
            }
        }
        Some(found)
    }

    fn allocate_coloured(&mut self, colours: Colours, mask: ColourMask) -> (r: Option<usize>)
        ensures
            final(self).allocated_run(*old(self), r, 1),
            final(self).hint == old(self).hint,
            final(self).managed_count == old(self).managed_count,
            r matches Some(frame) ==> old(self).free_by_bit@[frame as int % 64] >= 1
                && final(self).free_by_bit@ == old(self).free_by_bit@.update(
                frame as int % 64,
                (old(self).free_by_bit@[frame as int % 64] - 1) as usize,
            ),
            r matches Some(frame) ==> mask.spec_contains(colours.spec_of(frame as int)),
            r.is_none() ==> forall|frame: int|
                #[trigger] old(self).is_free_frame(frame) ==> !mask.spec_contains(
                    colours.spec_of(frame),
                ),
    {
        proof {
            use_type_invariant(&*self);
            use_type_invariant(colours);
        }
        let pattern = mask.word_pattern(colours);
        let low = if self.hint < self.limit {
            self.hint
        } else {
            self.limit
        };
        let found = match self.next_free_in(self.hint, self.limit, pattern) {
            Some(found) => found,
            None => match self.next_free_in(0, low, pattern) {
                Some(found) => found,
                None => {
                    proof {
                        assert forall|frame: int| #[trigger] self.is_free_frame(frame)
                            implies !mask.spec_contains(colours.spec_of(frame)) by {
                            self.lemma_free_is_managed(frame);
                            colours.lemma_pattern(mask, pattern, frame);
                        }
                    }
                    return None;
                },
            },
        };
        proof {
            colours.lemma_pattern(mask, pattern, found as int);
        }
        self.take(found);
        Some(found)
    }

    pub fn free_contiguous(&mut self, first: usize, count: usize) -> (r: Result<(), FreeError>)
        ensures
            r.is_err() ==> *final(self) == *old(self),
            r.is_ok() ==> forall|frame: int|
                first <= frame < first + count ==> #[trigger] old(self).is_managed_frame(frame)
                    && !old(self).is_free_frame(frame),
            r.is_ok() ==> final(self).released_run(*old(self), first as int, count as int),
    {
        if count == 0 {
            return Ok(());
        }
        let frames = Self::frame_count();
        if first >= frames {
            return Err(FreeError::NotManaged(first));
        }
        let end = match first.checked_add(count) {
            Some(end) if end <= frames => end,
            _ => return Err(FreeError::NotManaged(frames)),
        };

        let mut frame = first;
        while frame < end
            invariant
                first <= frame <= end,
                *self == *old(self),
                forall|other: int|
                    first <= other < frame ==> #[trigger] self.is_managed_frame(other),
                forall|other: int| first <= other < frame ==> !#[trigger] self.is_free_frame(other),
            decreases end - frame,
        {
            self.check_releasable(frame)?;
            frame += 1;
        }
        let ghost before = *self;
        frame = first;
        while frame < end
            invariant
                first <= frame <= end,
                forall|other: int| first <= other < end ==> #[trigger] before.is_managed_frame(other),
                forall|other: int| first <= other < end ==> !#[trigger] before.is_free_frame(other),
                forall|other: int| #[trigger]
                    self.is_managed_frame(other) == before.is_managed_frame(other),
                forall|other: int|
                    #[trigger] self.is_free_frame(other) == (before.is_free_frame(other) || (first
                        <= other < frame)),
                self.limit == before.limit,
            decreases end - frame,
        {
            self.release(frame);
            frame += 1;
        }
        Ok(())
    }

    pub fn free(&mut self, frame: usize) -> (r: Result<(), FreeError>)
        ensures
            r.is_err() ==> *final(self) == *old(self),
            r.is_ok() ==> old(self).is_managed_frame(frame as int) && !old(self).is_free_frame(
                frame as int,
            ),
            r.is_ok() ==> final(self).released_run(*old(self), frame as int, 1),
    {
        self.check_releasable(frame)?;
        self.release(frame);
        Ok(())
    }

    const fn check_releasable(&self, frame: usize) -> (r: Result<(), FreeError>)
        ensures
            r.is_ok() == (self.is_managed_frame(frame as int) && !self.is_free_frame(frame as int)),
    {
        if !self.is_managed(frame) {
            return Err(FreeError::NotManaged(frame));
        }
        if self.is_free(frame) {
            return Err(FreeError::AlreadyFree(frame));
        }
        Ok(())
    }

    fn release(&mut self, frame: usize)
        requires
            old(self).is_managed_frame(frame as int),
        ensures
            forall|other: int|
                #[trigger] final(self).is_free_frame(other) == (old(self).is_free_frame(other)
                    || other == frame),
            forall|other: int| #[trigger]
                final(self).is_managed_frame(other) == old(self).is_managed_frame(other),
            final(self).limit == old(self).limit,
            final(self).hint == if frame < old(self).hint {
                frame
            } else {
                old(self).hint
            },
            final(self).managed_count == old(self).managed_count,
            final(self).free_by_bit@ == if old(self).is_free_frame(frame as int) {
                old(self).free_by_bit@
            } else {
                old(self).free_by_bit@.update(
                    frame as int % 64,
                    (old(self).free_by_bit@[frame as int % 64] + 1) as usize,
                )
            },
    {
        if !self.is_free(frame) {
            proof {
                use_type_invariant(&*self);
                proof::lemma_parts_free(
                    WORDS,
                    self.managed@,
                    self.free@,
                    self.free_by_bit@,
                    self.managed_count,
                    self.limit,
                    frame as int,
                );
            }
            mark_free(&mut self.free, &mut self.free_by_bit, frame);
        }
        proof {
            use_type_invariant(&*self);
        }
        if frame < self.hint {
            self.hint = frame;
        }
    }

    fn take(&mut self, frame: usize)
        ensures
            forall|other: int|
                #[trigger] final(self).is_free_frame(other) == (old(self).is_free_frame(other)
                    && other != frame),
            forall|other: int| #[trigger]
                final(self).is_managed_frame(other) == old(self).is_managed_frame(other),
            final(self).limit == old(self).limit,
            final(self).hint == old(self).hint,
            final(self).managed_count == old(self).managed_count,
            final(self).free_by_bit@ == if old(self).is_free_frame(frame as int) {
                old(self).free_by_bit@.update(
                    frame as int % 64,
                    (old(self).free_by_bit@[frame as int % 64] - 1) as usize,
                )
            } else {
                old(self).free_by_bit@
            },
            old(self).is_free_frame(frame as int) ==> old(self).free_by_bit@[frame as int % 64] >= 1,
    {
        if self.is_free(frame) {
            proof {
                use_type_invariant(&*self);
                proof::lemma_parts_take(
                    WORDS,
                    self.managed@,
                    self.free@,
                    self.free_by_bit@,
                    self.managed_count,
                    self.limit,
                    frame as int,
                );
            }
            mark_taken(&mut self.free, &mut self.free_by_bit, frame);
        }
    }

    // Each step moves `start` past a taken frame, so the loop ends within `to` steps.
    fn search(&self, from: usize, to: usize, count: usize, align: usize) -> (r: Option<usize>)
        requires
            to <= proof::frame_count(WORDS),
            0 < count,
            proof::is_power_of_two(align),
        ensures
            r matches Some(start) ==> from <= start && start + count <= to && start % align == 0
                && forall|frame: int|
                start <= frame < start + count ==> #[trigger] self.is_free_frame(frame),
    {
        let free = self.next_free(from, to)?;
        let mut start = align_up(free, align)?;
        loop
            invariant
                to <= proof::frame_count(WORDS),
                0 < count,
                from <= start,
                start % align == 0,
                proof::is_power_of_two(align),
            decreases usize::MAX - start,
        {
            let end = start.checked_add(count)?;
            if end > to {
                return None;
            }
            let Some(blocker) = self.first_taken(start, end) else {
                return Some(start);
            };
            let next = self.next_free(blocker + 1, to)?;
            start = align_up(next, align)?;
        }
    }

    fn next_free(&self, from: usize, to: usize) -> (r: Option<usize>)
        requires
            to <= proof::frame_count(WORDS),
        ensures
            r matches Some(frame) ==> from <= frame < to && self.is_free_frame(frame as int),
    {
        proof {
            proof::lemma_full_pattern();
        }
        self.next_free_in(from, to, u64::MAX)
    }

    fn next_free_in(&self, from: usize, to: usize, pattern: u64) -> (r: Option<usize>)
        requires
            to <= proof::frame_count(WORDS),
        ensures
            r matches Some(frame) ==> from <= frame < to && self.is_free_frame(frame as int)
                && proof::bit_set(pattern, (frame % 64) as u64),
            r.is_none() ==> forall|frame: int|
                from <= frame < to ==> !(#[trigger] self.is_free_frame(frame) && proof::bit_set(
                    pattern,
                    (frame % 64) as u64,
                )),
    {
        if from >= to {
            return None;
        }
        let mut word = from / FRAMES_PER_WORD;
        proof {
            proof::lemma_domain_index(WORDS, to as int, from as int);
        }
        let mut free = self.free[word] & pattern & (u64::MAX << ((from % FRAMES_PER_WORD) as u64));
        proof {
            self.lemma_masked_first(pattern, from as int);
        }
        loop
            invariant
                to <= proof::frame_count(WORDS),
                from < to,
                word < WORDS,
                from / 64 <= word,
                word * 64 < to,
                forall|frame: int|
                    from <= frame < word * 64 ==> !(#[trigger] self.is_free_frame(frame)
                        && proof::bit_set(pattern, (frame % 64) as u64)),
                forall|bit: u64|
                    bit < 64 ==> #[trigger] proof::bit_set(free, bit) == (proof::bit_set(
                        self.free@[word as int],
                        bit,
                    ) && proof::bit_set(pattern, bit) && word * 64 + bit >= from),
            decreases WORDS - word,
        {
            if free != 0 {
                let zeros = free.trailing_zeros();
                proof {
                    proof::lemma_lowest_bit(free);
                    self.lemma_first_in_word(pattern, free, word as int, zeros as int, from as int);
                }
                let frame = word * FRAMES_PER_WORD + zeros as usize;
                if frame < to {
                    return Some(frame);
                }
                return None;
            }
            proof {
                self.lemma_empty_word(pattern, free, word as int, from as int);
            }
            word += 1;
            if word * FRAMES_PER_WORD >= to {
                return None;
            }
            proof {
                proof::lemma_domain_index(WORDS, to as int, (word * 64) as int);
                self.lemma_masked_next(pattern, word as int);
            }
            free = self.free[word] & pattern;
        }
    }

    fn first_taken(&self, start: usize, end: usize) -> (r: Option<usize>)
        requires
            end <= proof::frame_count(WORDS),
        ensures
            r matches Some(blocker) ==> start <= blocker < end && !self.is_free_frame(
                blocker as int,
            ),
            r.is_none() ==> forall|frame: int|
                start <= frame < end ==> #[trigger] self.is_free_frame(frame),
    {
        let mut frame = start;
        while frame < end
            invariant
                start <= frame,
                end <= proof::frame_count(WORDS),
                forall|other: int| start <= other < frame ==> #[trigger] self.is_free_frame(other),
            decreases end + 64 - frame,
        {
            let offset = frame % FRAMES_PER_WORD;
            proof {
                proof::lemma_domain_index(WORDS, end as int, frame as int);
            }
            let taken = (!self.free[frame / FRAMES_PER_WORD]) >> (offset as u64);
            if taken != 0 {
                let zeros = taken.trailing_zeros();
                proof {
                    proof::lemma_lowest_bit(taken);
                    self.lemma_first_taken(taken, frame as int, zeros as int);
                }
                let blocker = frame + zeros as usize;
                if blocker < end {
                    return Some(blocker);
                }
                return None;
            }
            proof {
                self.lemma_rest_free(frame as int);
            }
            frame += FRAMES_PER_WORD - offset;
        }
        None
    }
}

impl<const WORDS: usize> Default for FrameSpace<WORDS> {
    fn default() -> Self {
        Self::new()
    }
}

const fn align_up(value: usize, align: usize) -> (r: Option<usize>)
    requires
        proof::is_power_of_two(align),
    ensures
        r matches Some(aligned) ==> value <= aligned && aligned % align == 0,
{
    proof {
        proof::lemma_power_positive(align);
    }
    let mask = align - 1;
    match value.checked_add(mask) {
        Some(sum) => {
            proof {
                proof::lemma_align_mask(value, align, sum);
            }
            Some(sum & !mask)
        },
        None => None,
    }
}

const fn next_power_of_two(value: usize) -> (r: Option<usize>)
    ensures
        r matches Some(power) ==> value <= power && proof::is_power_of_two(power),
        proof::is_power_of_two(value) ==> r == Some(value),
{
    let mut shift = 0;
    while shift < usize::BITS
        invariant
            shift <= usize::BITS,
            forall|smaller: u32| smaller < shift ==> value > 1usize << smaller,
        decreases usize::BITS - shift,
    {
        if value <= 1usize << shift {
            proof {
                if proof::is_power_of_two(value) {
                    let exact = choose|exact: u32| exact < usize::BITS && value == 1usize << exact;
                    if exact > shift {
                        proof::lemma_shift_grows(shift, exact);
                    }
                    assert(!(exact < shift));
                }
            }
            return Some(1usize << shift);
        }
        shift += 1;
    }
    proof {
        if proof::is_power_of_two(value) {
            let exact = choose|exact: u32| exact < usize::BITS && value == 1usize << exact;
            assert(value > 1usize << exact);
        }
    }
    None
}

} // verus!

#[cfg(test)]
mod tests;
