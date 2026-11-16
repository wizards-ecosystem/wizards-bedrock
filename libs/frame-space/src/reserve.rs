use vstd::prelude::verus;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
use crate::proof;
use crate::{ColourCounts, ColourMask, Colours, FrameSpace, FreeError, MAX_COLOURS};

verus! {

// Per colour, every reservation's unused share summed, never above that colour's free frames.
#[derive(Debug)]
pub struct Pool {
    pub(crate) unused: ColourCounts,
}

impl Pool {
    pub const fn new() -> (r: Self)
        ensures
            forall|colour: int| #[trigger] r.spec_unused(colour) == 0,
    {
        Self { unused: ColourCounts::ZERO }
    }

    pub const fn unused(&self, colour: usize) -> (r: usize)
        ensures
            r == self.spec_unused(colour as int),
    {
        self.unused.get(colour)
    }

    // Leaves the reservation empty, so it can release nothing twice.
    #[must_use = "a refused dissolve released nothing"]
    pub fn dissolve(&mut self, reservation: &mut Reservation) -> (r: bool)
        ensures
            !r ==> *final(self) == *old(self) && *final(reservation) == *old(reservation),
            r ==> forall|colour: int| #[trigger] final(reservation).spec_reserved(colour) == 0,
            r ==> final(reservation).spec_mask() == old(reservation).spec_mask(),
            r ==> forall|colour: int|
                #[trigger] final(self).spec_unused(colour) == if old(self).spec_unused(colour)
                    >= old(reservation).spec_unused(colour) {
                    old(self).spec_unused(colour) - old(reservation).spec_unused(colour)
                } else {
                    0
                },
    {
        proof {
            use_type_invariant(&*reservation);
        }
        if !reservation.used.is_zero() {
            return false;
        }
        let mut colour = 0;
        while colour < MAX_COLOURS
            invariant
                colour <= 64,
                forall|other: int| #[trigger] reservation.used.spec_get(other) == 0,
                *reservation == *old(reservation),
                forall|other: int|
                    #[trigger] self.spec_unused(other) == if 0 <= other < colour {
                        if old(self).spec_unused(other) >= reservation.spec_reserved(other) {
                            (old(self).spec_unused(other) - reservation.spec_reserved(
                                other,
                            )) as usize
                        } else {
                            0
                        }
                    } else {
                        old(self).spec_unused(other)
                    },
            decreases 64 - colour,
        {
            let ghost before = *self;
            let reserved = reservation.reserved.get(colour);
            self.unused.decrease(colour, reserved);
            proof {
                assert(self.spec_unused(colour as int) == if before.spec_unused(colour as int)
                    >= reserved {
                    (before.spec_unused(colour as int) - reserved) as usize
                } else {
                    0
                });
                assert forall|other: int| other != colour implies #[trigger] self.spec_unused(other)
                    == before.spec_unused(other) by {}
            }
            colour += 1;
        }
        reservation.reserved = ColourCounts::ZERO;
        true
    }

    fn unreserved<const WORDS: usize>(
        &self,
        space: &FrameSpace<WORDS>,
        colours: Colours,
        mask: ColourMask,
    ) -> (r: Option<ColourMask>)
        ensures
            r matches Some(chosen) ==> forall|colour: int| #[trigger]
                chosen.spec_contains(colour) ==> mask.spec_contains(colour) && 0 <= colour
                    < colours.spec_count() && space.free_of_colour(colours.spec_count(), colour)
                    > self.spec_unused(colour),
            r.is_none() ==> forall|colour: int|
                0 <= colour < colours.spec_count() && mask.spec_contains(colour)
                    ==> #[trigger] space.free_of_colour(colours.spec_count(), colour)
                    <= self.spec_unused(colour),
    {
        let mut bits: u64 = 0;
        let mut colour = 0;
        let count = colours.count();
        proof {
            proof::lemma_zero_word_all();
        }
        while colour < count
            invariant
                count == colours.spec_count(),
                0 < count <= 64,
                colour <= count,
                forall|other: u64|
                    other < 64 ==> #[trigger] proof::bit_set(bits, other) == (other < colour
                        && mask.spec_contains(other as int) && space.free_of_colour(
                        count,
                        other as int,
                    ) > self.spec_unused(other as int)),
            decreases count - colour,
        {
            let ghost before = bits;
            let free = space.free_in(colours, colour);
            if mask.contains(colour) && free > self.unused.get(colour) {
                proof {
                    assert forall|other: u64| other < 64 implies #[trigger] proof::bit_set(
                        before | (1u64 << colour as u64),
                        other,
                    ) == (proof::bit_set(before, other) || other == colour) by {
                        proof::lemma_set_bit(before, colour as u64, other);
                    }
                }
                bits |= 1 << (colour as u64);
            }
            colour += 1;
        }
        proof {
            if bits == 0 {
                assert forall|colour: int|
                    0 <= colour < count && mask.spec_contains(colour) implies #[trigger] space.free_of_colour(
                        count,
                        colour,
                    ) <= self.spec_unused(colour) by {
                    proof::lemma_zero_word(colour as u64);
                }
            }
        }
        ColourMask::new(colours, bits)
    }
}

impl Default for Pool {
    fn default() -> (r: Self)
        ensures
            forall|colour: int| #[trigger] r.spec_unused(colour) == 0,
    {
        Self::new()
    }
}

// `used` never exceeds `reserved`, and both are zero outside `mask`.
#[derive(Debug)]
pub struct Reservation {
    pub(crate) mask: ColourMask,
    pub(crate) reserved: ColourCounts,
    pub(crate) used: ColourCounts,
}

impl Reservation {
    // Admits no colour, so nothing can be reserved into it or charged to it.
    pub exec const EMPTY: Self
        ensures
            forall|colour: int| #[trigger] Self::EMPTY.spec_reserved(colour) == 0,
            forall|colour: int| #[trigger] Self::EMPTY.spec_used(colour) == 0,
            forall|colour: int| !#[trigger] Self::EMPTY.spec_mask().spec_contains(colour),
    {
        proof {
            proof::lemma_zero_word_all();
        }
        Self::new(ColourMask::NONE)
    }

    pub const fn new(mask: ColourMask) -> (r: Self)
        ensures
            r.spec_mask() == mask,
            forall|colour: int| #[trigger] r.spec_reserved(colour) == 0,
            forall|colour: int| #[trigger] r.spec_used(colour) == 0,
    {
        Self { mask, reserved: ColourCounts::ZERO, used: ColourCounts::ZERO }
    }

    pub const fn mask(&self) -> (r: ColourMask)
        ensures
            r == self.spec_mask(),
    {
        self.mask
    }

    pub const fn reserved(&self, colour: usize) -> (r: usize)
        ensures
            r == self.spec_reserved(colour as int),
    {
        self.reserved.get(colour)
    }

    pub const fn used(&self, colour: usize) -> (r: usize)
        ensures
            r == self.spec_used(colour as int),
    {
        self.used.get(colour)
    }

    pub fn unused_total(&self) -> usize {
        let mut total: usize = 0;
        let mut colour = 0;
        while colour < MAX_COLOURS
            invariant
                colour <= 64,
            decreases 64 - colour,
        {
            total = total.saturating_add(self.unused(colour));
            colour += 1;
        }
        total
    }

    const fn unused(&self, colour: usize) -> (r: usize)
        ensures
            r == self.spec_unused(colour as int),
    {
        proof {
            use_type_invariant(self);
        }
        self.reserved.get(colour) - self.used.get(colour)
    }

    // Moves reservation to a child whose mask lies within this one; a refusal changes neither.
    #[must_use = "a refused transfer moved nothing"]
    pub fn transfer(&mut self, child: &mut Self, amounts: &ColourCounts) -> (r: bool)
        ensures
            !r ==> *final(self) == *old(self) && *final(child) == *old(child),
            r ==> forall|colour: int|
                #[trigger] final(self).spec_unused(colour) + final(child).spec_unused(colour)
                    == old(self).spec_unused(colour) + old(child).spec_unused(colour),
            final(self).spec_mask() == old(self).spec_mask(),
            final(child).spec_mask() == old(child).spec_mask(),
    {
        proof {
            use_type_invariant(&*self);
            use_type_invariant(&*child);
        }
        if !child.mask.is_within(self.mask) {
            return false;
        }
        let mut colour = 0;
        while colour < MAX_COLOURS
            invariant
                colour <= 64,
                *self == *old(self),
                *child == *old(child),
                forall|other: int|
                    0 <= other < colour ==> #[trigger] self.admits(*child, *amounts, other),
            decreases 64 - colour,
        {
            let amount = amounts.get(colour);
            let room = child.reserved.get(colour).checked_add(amount).is_some();
            if !(amount == 0 || (child.mask.contains(colour) && amount <= self.unused(colour)
                && room)) {
                return false;
            }
            colour += 1;
        }
        let ghost parent_before = *self;
        let ghost child_before = *child;
        colour = 0;
        while colour < MAX_COLOURS
            invariant
                colour <= 64,
                forall|other: int|
                    0 <= other < 64 ==> #[trigger] parent_before.admits(
                        child_before,
                        *amounts,
                        other,
                    ),
                self.mask == parent_before.mask,
                self.used == parent_before.used,
                child.mask == child_before.mask,
                child.used == child_before.used,
                forall|other: int|
                    #[trigger] self.spec_reserved(other) == if 0 <= other < colour {
                        (parent_before.spec_reserved(other) - amounts.spec_get(other)) as usize
                    } else {
                        parent_before.spec_reserved(other)
                    },
                forall|other: int|
                    #[trigger] child.spec_reserved(other) == if 0 <= other < colour {
                        (child_before.spec_reserved(other) + amounts.spec_get(other)) as usize
                    } else {
                        child_before.spec_reserved(other)
                    },
            decreases 64 - colour,
        {
            let amount = amounts.get(colour);
            let ghost (parent_now, child_now) = (*self, *child);
            proof {
                use_type_invariant(&*self);
                use_type_invariant(&*child);
                parent_before.lemma_admitted(child_before, *amounts, colour as int);
                assert(self.spec_reserved(colour as int) == parent_before.spec_reserved(
                    colour as int,
                ));
                assert(child.spec_reserved(colour as int) == child_before.spec_reserved(
                    colour as int,
                ));
            }
            self.reserved.decrease(colour, amount);
            child.reserved.increase(colour, amount);
            proof {
                assert(self.spec_reserved(colour as int) == (parent_before.spec_reserved(
                    colour as int,
                ) - amount) as usize);
                assert(child.spec_reserved(colour as int) == (child_before.spec_reserved(
                    colour as int,
                ) + amount) as usize);
                assert forall|other: int| other != colour implies #[trigger] self.spec_reserved(
                    other,
                ) == parent_now.spec_reserved(other) by {}
                assert forall|other: int| other != colour implies #[trigger] child.spec_reserved(
                    other,
                ) == child_now.spec_reserved(other) by {}
            }
            colour += 1;
        }
        proof {
            assert forall|other: int|
                #[trigger] self.spec_unused(other) + child.spec_unused(other)
                    == parent_before.spec_unused(other) + child_before.spec_unused(other) by {
                if 0 <= other < 64 {
                    assert(self.spec_reserved(other) == (parent_before.spec_reserved(other)
                        - amounts.spec_get(other)) as usize);
                    assert(child.spec_reserved(other) == (child_before.spec_reserved(other)
                        + amounts.spec_get(other)) as usize);
                    parent_before.lemma_admitted(child_before, *amounts, other);
                }
            }
        }
        true
    }

    // Leaves this reservation empty, so it can return nothing twice.
    #[must_use = "a refused return moved nothing"]
    pub fn return_to(&mut self, parent: &mut Self) -> (r: bool)
        ensures
            !r ==> *final(self) == *old(self) && *final(parent) == *old(parent),
            r ==> forall|colour: int| #[trigger] final(self).spec_reserved(colour) == 0,
            r ==> forall|colour: int|
                #[trigger] final(self).spec_unused(colour) + final(parent).spec_unused(colour)
                    == old(self).spec_unused(colour) + old(parent).spec_unused(colour),
            final(self).spec_mask() == old(self).spec_mask(),
            final(parent).spec_mask() == old(parent).spec_mask(),
    {
        proof {
            use_type_invariant(&*self);
            use_type_invariant(&*parent);
        }
        if !self.used.is_zero() || !self.mask.is_within(parent.mask) {
            return false;
        }
        // Checked first, so a parent that cannot hold the whole return refuses all of it.
        let mut colour = 0;
        while colour < MAX_COLOURS
            invariant
                colour <= 64,
                *self == *old(self),
                *parent == *old(parent),
                forall|other: int|
                    0 <= other < colour ==> #[trigger] parent.spec_reserved(other)
                        + self.spec_reserved(other) <= usize::MAX,
            decreases 64 - colour,
        {
            if parent.reserved.get(colour).checked_add(self.reserved.get(colour)).is_none() {
                return false;
            }
            colour += 1;
        }
        let ghost parent_before = *parent;
        colour = 0;
        while colour < MAX_COLOURS
            invariant
                colour <= 64,
                forall|other: int|
                    0 <= other < 64 ==> #[trigger] parent_before.spec_reserved(other)
                        + self.spec_reserved(other) <= usize::MAX,
                forall|other: int| #[trigger] self.used.spec_get(other) == 0,
                forall|other: int| #[trigger]
                    self.mask.spec_contains(other) ==> parent.mask.spec_contains(other),
                parent.mask == parent_before.mask,
                parent.used == parent_before.used,
                *self == *old(self),
                forall|other: int|
                    #[trigger] parent.spec_reserved(other) == if 0 <= other < colour {
                        (parent_before.spec_reserved(other) + self.spec_reserved(other)) as usize
                    } else {
                        parent_before.spec_reserved(other)
                    },
            decreases 64 - colour,
        {
            let amount = self.reserved.get(colour);
            let ghost parent_now = *parent;
            proof {
                use_type_invariant(&*self);
                use_type_invariant(&*parent);
                assert(parent.spec_reserved(colour as int) == parent_before.spec_reserved(
                    colour as int,
                ));
            }
            parent.reserved.increase(colour, amount);
            proof {
                assert forall|other: int| other != colour implies #[trigger] parent.spec_reserved(
                    other,
                ) == parent_now.spec_reserved(other) by {}
            }
            colour += 1;
        }
        self.reserved = ColourCounts::ZERO;
        proof {
            assert forall|other: int|
                #[trigger] self.spec_unused(other) + parent.spec_unused(other) == old(
                    self,
                ).spec_unused(other) + old(parent).spec_unused(other) by {
                if 0 <= other < 64 {
                    assert(parent.spec_reserved(other) == (parent_before.spec_reserved(other)
                        + old(self).spec_reserved(other)) as usize);
                    assert(old(self).used.spec_get(other) == 0);
                }
            }
        }
        true
    }

    fn chargeable(&self, colours: Colours) -> (r: Option<ColourMask>)
        ensures
            r matches Some(chosen) ==> forall|colour: int|
                #[trigger] chosen.spec_contains(colour) ==> 0 <= colour < colours.spec_count()
                    && self.spec_unused(colour) > 0,
            r matches Some(chosen) ==> forall|colour: int|
                0 <= colour < colours.spec_count() && self.spec_unused(colour) > 0
                    ==> #[trigger] chosen.spec_contains(colour),
            r.is_none() ==> forall|colour: int|
                0 <= colour < colours.spec_count() ==> #[trigger] self.spec_unused(colour) == 0,
    {
        proof {
            use_type_invariant(self);
        }
        let mut bits: u64 = 0;
        let mut colour = 0;
        let count = colours.count();
        proof {
            proof::lemma_zero_word_all();
        }
        while colour < count
            invariant
                count == colours.spec_count(),
                0 < count <= 64,
                colour <= count,
                forall|other: u64|
                    other < 64 ==> #[trigger] proof::bit_set(bits, other) == (other < colour
                        && self.spec_unused(other as int) > 0),
            decreases count - colour,
        {
            let ghost before = bits;
            if self.unused(colour) > 0 {
                proof {
                    assert forall|other: u64| other < 64 implies #[trigger] proof::bit_set(
                        before | (1u64 << colour as u64),
                        other,
                    ) == (proof::bit_set(before, other) || other == colour) by {
                        proof::lemma_set_bit(before, colour as u64, other);
                    }
                }
                bits |= 1 << (colour as u64);
            }
            colour += 1;
        }
        proof {
            if bits == 0 {
                assert forall|colour: int| 0 <= colour < count implies #[trigger] self.spec_unused(
                    colour,
                ) == 0 by {
                    proof::lemma_zero_word(colour as u64);
                }
            }
        }
        ColourMask::new(colours, bits)
    }
}

impl<const WORDS: usize> FrameSpace<WORDS> {
    #[must_use = "a refused reservation reserved nothing"]
    pub fn reserve(
        &self,
        colours: Colours,
        pool: &mut Pool,
        into: &mut Reservation,
        amounts: &ColourCounts,
    ) -> (r: bool)
        ensures
            !r ==> *final(pool) == *old(pool) && *final(into) == *old(into),
            final(into).spec_mask() == old(into).spec_mask(),
            self.colour_reservations_fit(colours, *old(pool)) ==> self.colour_reservations_fit(
                colours,
                *final(pool),
            ),
            r ==> forall|colour: int|
                #[trigger] final(pool).spec_unused(colour) - final(into).spec_unused(colour) == old(
                    pool,
                ).spec_unused(colour) - old(into).spec_unused(colour),
    {
        proof {
            use_type_invariant(self);
            use_type_invariant(&*into);
            self.lemma_colour_bounded(colours);
            self.lemma_free_bounded();
        }
        let mut colour = 0;
        while colour < MAX_COLOURS
            invariant
                colour <= 64,
                *pool == *old(pool),
                *into == *old(into),
                forall|other: int|
                    0 <= other < colour ==> #[trigger] self.reserve_admits(
                        colours,
                        *pool,
                        *into,
                        *amounts,
                        other,
                    ),
            decreases 64 - colour,
        {
            let amount = amounts.get(colour);
            let slack = self.free_in(colours, colour).saturating_sub(pool.unused.get(colour));
            let room = into.reserved.get(colour).checked_add(amount).is_some();
            if !(amount == 0 || (into.mask.contains(colour) && amount <= slack && room)) {
                return false;
            }
            colour += 1;
        }
        let ghost pool_before = *pool;
        let ghost into_before = *into;
        colour = 0;
        while colour < MAX_COLOURS
            invariant
                colour <= 64,
                self.ownership(),
                forall|other: int| #[trigger]
                    self.free_of_colour(colours.spec_count(), other) <= self.managed_count,
                self.managed_count < usize::MAX,
                forall|other: int|
                    0 <= other < 64 ==> #[trigger] self.reserve_admits(
                        colours,
                        pool_before,
                        into_before,
                        *amounts,
                        other,
                    ),
                into.mask == into_before.mask,
                into.used == into_before.used,
                forall|other: int|
                    #[trigger] pool.spec_unused(other) == if 0 <= other < colour {
                        (pool_before.spec_unused(other) + amounts.spec_get(other)) as usize
                    } else {
                        pool_before.spec_unused(other)
                    },
                forall|other: int|
                    #[trigger] into.spec_reserved(other) == if 0 <= other < colour {
                        (into_before.spec_reserved(other) + amounts.spec_get(other)) as usize
                    } else {
                        into_before.spec_reserved(other)
                    },
            decreases 64 - colour,
        {
            let amount = amounts.get(colour);
            let ghost (pool_now, into_now) = (*pool, *into);
            proof {
                use_type_invariant(&*into);
                self.lemma_reserve_admitted(
                    colours,
                    pool_before,
                    into_before,
                    *amounts,
                    colour as int,
                );
                assert(pool.spec_unused(colour as int) == pool_before.spec_unused(colour as int));
                assert(into.spec_reserved(colour as int) == into_before.spec_reserved(
                    colour as int,
                ));
            }
            pool.unused.increase(colour, amount);
            into.reserved.increase(colour, amount);
            proof {
                assert(pool.spec_unused(colour as int) == (pool_before.spec_unused(colour as int)
                    + amount) as usize);
                assert(into.spec_reserved(colour as int) == (into_before.spec_reserved(
                    colour as int,
                ) + amount) as usize);
                assert forall|other: int| other != colour implies #[trigger] pool.spec_unused(other)
                    == pool_now.spec_unused(other) by {}
                assert forall|other: int| other != colour implies #[trigger] into.spec_reserved(
                    other,
                ) == into_now.spec_reserved(other) by {}
            }
            colour += 1;
        }
        proof {
            assert forall|other: int|
                0 <= other < colours.spec_count() && self.colour_reservations_fit(
                    colours,
                    pool_before,
                ) implies #[trigger] pool.spec_unused(other) <= self.free_of_colour(
                colours.spec_count(),
                other,
            ) by {
                assert(self.reserve_admits(colours, pool_before, into_before, *amounts, other));
            }
            assert forall|other: int|
                #[trigger] pool.spec_unused(other) - into.spec_unused(other) == pool_before.spec_unused(
                    other,
                ) - into_before.spec_unused(other) by {
                if 0 <= other < 64 {
                    assert(self.reserve_admits(colours, pool_before, into_before, *amounts, other));
                    assert(into.spec_reserved(other) == (into_before.spec_reserved(other)
                        + amounts.spec_get(other)) as usize);
                    assert(pool.spec_unused(other) == (pool_before.spec_unused(other)
                        + amounts.spec_get(other)) as usize);
                }
            }
        }
        true
    }

    pub fn allocate_contiguous_unreserved(
        &mut self,
        colours: Colours,
        pool: &Pool,
        count: usize,
        align: usize,
    ) -> (r: Option<usize>)
        ensures
            final(self).allocated_run(*old(self), r, count),
            r.is_some() ==> final(self).colour_reservations_fit(colours, *pool),
            r matches Some(first) ==> proof::is_power_of_two(align) ==> first % align == 0,
    {
        proof {
            use_type_invariant(&*self);
        }
        let hint = self.hint;
        let ghost before = *self;
        let first = self.allocate_contiguous(count, align)?;
        let ghost taken = *self;
        let mut colour = 0;
        let mut intact = true;
        while colour < colours.count()
            invariant
                *self == taken,
                taken.allocated_run(before, Some(first), count),
                colour <= colours.spec_count(),
                intact ==> forall|other: int|
                    0 <= other < colour ==> #[trigger] pool.spec_unused(other)
                        <= self.free_of_colour(colours.spec_count(), other),
            decreases colours.spec_count() - colour,
        {
            if self.free_in(colours, colour) < pool.unused.get(colour) {
                intact = false;
            }
            colour += 1;
        }
        if intact {
            return Some(first);
        }
        let mut frame = first;
        while frame < first + count
            invariant
                first <= frame <= first + count,
                first + count <= before.limit,
                forall|other: int|
                    #[trigger] self.is_free_frame(other) == (before.is_free_frame(other) && !(frame
                        <= other < first + count)),
                forall|other: int| #[trigger]
                    self.is_managed_frame(other) == before.is_managed_frame(other),
                forall|other: int|
                    first <= other < first + count ==> #[trigger] before.is_free_frame(other),
                self.limit == before.limit,
                self.managed_count == before.managed_count,
                before.ownership(),
            decreases first + count - frame,
        {
            proof {
                use_type_invariant(&*self);
                before.lemma_free_is_managed(frame as int);
            }
            self.release(frame);
            frame += 1;
        }
        // A refused allocation leaves even the search hint as it found it.
        proof {
            use_type_invariant(&*self);
        }
        let ghost rolled = *self;
        self.hint = hint;
        proof {
            use_type_invariant(&*self);
            assert forall|other: int| #[trigger] self.is_free_frame(other) == before.is_free_frame(
                other,
            ) by {
                assert(rolled.is_free_frame(other) == before.is_free_frame(other));
            }
            assert forall|other: int| #[trigger] self.is_managed_frame(other)
                == before.is_managed_frame(other) by {
                assert(rolled.is_managed_frame(other) == before.is_managed_frame(other));
            }
            self.lemma_restored(before);
        }
        None
    }

    pub fn allocate_unreserved(
        &mut self,
        colours: Colours,
        pool: &Pool,
        mask: ColourMask,
    ) -> (r: Option<usize>)
        ensures
            final(self).allocated_run(*old(self), r, 1),
            r matches Some(frame) ==> mask.spec_contains(colours.spec_of(frame as int)),
            old(self).colour_reservations_fit(colours, *pool) ==> final(self).colour_reservations_fit(
                colours,
                *pool,
            ),
    {
        proof {
            use_type_invariant(colours);
        }
        let chosen = pool.unreserved(self, colours, mask)?;
        let ghost before = *self;
        let found = self.allocate_coloured(colours, chosen);
        proof {
            if let Some(frame) = found {
                self.lemma_one_taken(before, colours, frame as int);
            }
        }
        found
    }

    pub fn allocate_reserved(
        &mut self,
        colours: Colours,
        pool: &mut Pool,
        into: &mut Reservation,
    ) -> (r: Option<usize>)
        ensures
            final(self).allocated_run(*old(self), r, 1),
            r.is_none() ==> *final(pool) == *old(pool) && *final(into) == *old(into),
            r matches Some(frame) ==> old(into).spec_mask().spec_contains(
                colours.spec_of(frame as int),
            ),
            final(into).spec_mask() == old(into).spec_mask(),
            old(self).colour_reservations_fit(colours, *old(pool)) ==> final(self).colour_reservations_fit(
                colours,
                *final(pool),
            ),
            r matches Some(frame) ==> forall|colour: int|
                #[trigger] final(pool).spec_unused(colour) - final(into).spec_unused(colour) == old(
                    pool,
                ).spec_unused(colour) - old(into).spec_unused(colour) || colour == colours.spec_of(
                    frame as int,
                ) && old(pool).spec_unused(colour) == 0,
            old(self).colour_reservations_fit(colours, *old(pool)) && old(into).within_pool(*old(pool))
                && old(into).has_unused(colours) ==> r.is_some(),
    {
        proof {
            use_type_invariant(&*self);
            use_type_invariant(&*into);
            use_type_invariant(colours);
        }
        let mask = into.chargeable(colours)?;
        let ghost before = *self;
        let Some(frame) = self.allocate_coloured(colours, mask) else {
            proof {
                if before.colour_reservations_fit(colours, *pool) && into.within_pool(*pool)
                    && into.has_unused(colours) {
                    before.lemma_reserved_frame_exists(colours, *pool, *into, mask);
                }
            }
            return None;
        };
        let colour = colours.of(frame);
        let ghost (pool_before, into_before) = (*pool, *into);
        proof {
            self.lemma_one_taken(before, colours, frame as int);
            assert(into.spec_unused(colour as int) > 0);
            vstd::arithmetic::div_mod::lemma_mod_bound(frame as int, colours.spec_count() as int);
        }
        into.used.increase(colour, 1);
        pool.unused.decrease(colour, 1);
        proof {
            assert(into.used.spec_get(colour as int) == into_before.used.spec_get(colour as int)
                + 1);
            assert forall|other: int| other != colour implies #[trigger] into.spec_unused(other)
                == into_before.spec_unused(other) && #[trigger] pool.spec_unused(other)
                == pool_before.spec_unused(other) by {}
            assert(pool.spec_unused(colour as int) == if pool_before.spec_unused(colour as int)
                >= 1 {
                (pool_before.spec_unused(colour as int) - 1) as usize
            } else {
                0
            });
            if before.colour_reservations_fit(colours, pool_before) {
                assert forall|other: int| 0 <= other < colours.spec_count() implies #[trigger]
                    pool.spec_unused(other) <= self.free_of_colour(colours.spec_count(), other) by {
                    assert(pool_before.spec_unused(other) <= before.free_of_colour(
                        colours.spec_count(),
                        other,
                    ));
                }
            }
        }
        Some(frame)
    }

    pub fn free_reserved(
        &mut self,
        frame: usize,
        colours: Colours,
        pool: &mut Pool,
        from: &mut Reservation,
    ) -> (r: Result<(), FreeError>)
        ensures
            r.is_err() ==> *final(self) == *old(self) && *final(pool) == *old(pool) && *final(from)
                == *old(from),
            r.is_ok() ==> final(self).released_run(*old(self), frame as int, 1),
            final(from).spec_mask() == old(from).spec_mask(),
            old(self).colour_reservations_fit(colours, *old(pool)) ==> final(self).colour_reservations_fit(
                colours,
                *final(pool),
            ),
            r.is_ok() && old(self).colour_reservations_fit(colours, *old(pool)) ==> forall|colour: int|
                #[trigger] final(pool).spec_unused(colour) - final(from).spec_unused(colour) == old(
                    pool,
                ).spec_unused(colour) - old(from).spec_unused(colour),
    {
        proof {
            use_type_invariant(&*self);
            use_type_invariant(&*from);
            use_type_invariant(colours);
        }
        self.check_releasable(frame)?;
        let colour = colours.of(frame);
        if from.used.get(colour) == 0 {
            return Err(FreeError::NotCharged(frame));
        }
        let ghost before = *self;
        let ghost fit = before.colour_reservations_fit(colours, *pool);
        proof {
            if fit {
                before.lemma_unused_room(colours, *pool, frame as int);
            }
        }
        let ghost (pool_before, from_before) = (*pool, *from);
        self.release(frame);
        proof {
            before.lemma_free_bounded();
            self.lemma_one_released(before, colours, frame as int);
            vstd::arithmetic::div_mod::lemma_mod_bound(frame as int, colours.spec_count() as int);
        }
        from.used.decrease(colour, 1);
        pool.unused.increase(colour, 1);
        proof {
            assert(from.used.spec_get(colour as int) == from_before.used.spec_get(colour as int)
                - 1);
            assert forall|other: int| other != colour implies #[trigger] from.spec_unused(other)
                == from_before.spec_unused(other) && #[trigger] pool.spec_unused(other)
                == pool_before.spec_unused(other) by {}
            if fit {
                assert(pool.spec_unused(colour as int) == pool_before.spec_unused(colour as int)
                    + 1);
                assert forall|other: int| 0 <= other < colours.spec_count() implies #[trigger]
                    pool.spec_unused(other) <= self.free_of_colour(colours.spec_count(), other) by {
                    assert(pool_before.spec_unused(other) <= before.free_of_colour(
                        colours.spec_count(),
                        other,
                    ));
                }
            }
        }
        Ok(())
    }
}

} // verus!

verus! {

impl Pool {
    pub open(crate) spec fn spec_unused(self, colour: int) -> usize {
        self.unused.spec_get(colour)
    }
}

impl Reservation {
    #[verifier::type_invariant]
    pub(crate) open spec fn within_reserved(self) -> bool {
        &&& forall|colour: int| #[trigger]
            self.used.spec_get(colour) <= self.reserved.spec_get(colour)
        &&& forall|colour: int|
            !self.mask.spec_contains(colour) ==> #[trigger] self.reserved.spec_get(colour) == 0
    }

    pub open(crate) spec fn spec_mask(self) -> ColourMask {
        self.mask
    }

    pub open(crate) spec fn spec_reserved(self, colour: int) -> usize {
        self.reserved.spec_get(colour)
    }

    pub open(crate) spec fn spec_used(self, colour: int) -> usize {
        self.used.spec_get(colour)
    }

    pub open(crate) spec fn spec_unused(self, colour: int) -> int {
        self.reserved.spec_get(colour) - self.used.spec_get(colour)
    }

    // The pool's count for a colour covers this reservation's unused share of it.
    pub open(crate) spec fn within_pool(self, pool: Pool) -> bool {
        forall|colour: int| #[trigger] self.spec_unused(colour) <= pool.spec_unused(colour)
    }

    pub open(crate) spec fn has_unused(self, colours: Colours) -> bool {
        exists|colour: int|
            0 <= colour < colours.spec_count() && #[trigger] self.spec_unused(colour) > 0
    }

    pub(crate) open spec fn admits(self, child: Self, amounts: ColourCounts, colour: int) -> bool {
        let amount = amounts.spec_get(colour);
        amount == 0 || (child.mask.spec_contains(colour) && amount <= (if self.reserved.spec_get(
            colour,
        ) >= self.used.spec_get(colour) {
            self.reserved.spec_get(colour) - self.used.spec_get(colour)
        } else {
            0
        }) && child.reserved.spec_get(colour) + amount <= usize::MAX)
    }

    pub(crate) proof fn lemma_admitted(self, child: Self, amounts: ColourCounts, colour: int)
        requires
            self.admits(child, amounts, colour),
        ensures
            amounts.spec_get(colour) <= self.reserved.spec_get(colour),
            amounts.spec_get(colour) == 0 || amounts.spec_get(colour) + self.used.spec_get(colour)
                <= self.reserved.spec_get(colour),
            amounts.spec_get(colour) == 0 || child.mask.spec_contains(colour),
            child.reserved.spec_get(colour) + amounts.spec_get(colour) <= usize::MAX,
    {
    }

}

impl<const WORDS: usize> FrameSpace<WORDS> {
    // For each colour, the unused reservations never exceed that colour's free frames.
    pub open(crate) spec fn colour_reservations_fit(self, colours: Colours, pool: Pool) -> bool {
        forall|colour: int|
            0 <= colour < colours.spec_count() ==> #[trigger] pool.spec_unused(colour)
                <= self.free_of_colour(colours.spec_count(), colour)
    }

    pub(crate) open spec fn reserve_admits(
        self,
        colours: Colours,
        pool: Pool,
        into: Reservation,
        amounts: ColourCounts,
        colour: int,
    ) -> bool {
        let amount = amounts.spec_get(colour);
        amount == 0 || (into.mask.spec_contains(colour) && into.reserved.spec_get(colour) + amount
            <= usize::MAX && amount + pool.spec_unused(colour) <= self.free_of_colour(
            colours.spec_count(),
            colour,
        ))
    }

    pub(crate) proof fn lemma_reserve_admitted(
        self,
        colours: Colours,
        pool: Pool,
        into: Reservation,
        amounts: ColourCounts,
        colour: int,
    )
        requires
            self.reserve_admits(colours, pool, into, amounts, colour),
        ensures
            into.reserved.spec_get(colour) + amounts.spec_get(colour) <= usize::MAX,
            amounts.spec_get(colour) == 0 || into.mask.spec_contains(colour),
            amounts.spec_get(colour) + pool.spec_unused(colour) <= self.free_of_colour(
                colours.spec_count(),
                colour,
            ) || amounts.spec_get(colour) == 0,
    {
    }

    pub(crate) proof fn lemma_colour_bounded(self, colours: Colours)
        requires
            self.ownership(),
        ensures
            forall|colour: int| #[trigger]
                self.free_of_colour(colours.spec_count(), colour) <= self.managed_count,
    {
        self.lemma_free_bounded();
        assert forall|colour: int| #[trigger]
            self.free_of_colour(colours.spec_count(), colour) <= self.managed_count by {
            if 0 <= colour < colours.spec_count() {
                proof::lemma_strided_bound(
                    self.free_by_bit@,
                    colours.spec_count() as nat,
                    colour as nat,
                    64,
                );
            }
        }
    }

    pub(crate) proof fn lemma_reserved_frame_exists(
        self,
        colours: Colours,
        pool: Pool,
        into: Reservation,
        mask: ColourMask,
    )
        requires
            self.ownership(),
            into.within_reserved(),
            colours.valid(),
            self.colour_reservations_fit(colours, pool),
            into.within_pool(pool),
            into.has_unused(colours),
            forall|colour: int|
                0 <= colour < colours.spec_count() && into.spec_unused(colour) > 0
                    ==> #[trigger] mask.spec_contains(colour),
        ensures
            exists|frame: int| #[trigger]
                self.is_free_frame(frame) && mask.spec_contains(colours.spec_of(frame)),
    {
        let colour = choose|colour: int|
            0 <= colour < colours.spec_count() && #[trigger] into.spec_unused(colour) > 0;
        assert(pool.spec_unused(colour) <= self.free_of_colour(colours.spec_count(), colour));
        let frame = self.lemma_free_frame_of_colour(colours, colour);
        assert(self.is_free_frame(frame) && mask.spec_contains(colours.spec_of(frame)));
    }

    pub(crate) proof fn lemma_unused_room(self, colours: Colours, pool: Pool, frame: int)
        requires
            self.ownership(),
            colours.valid(),
            self.colour_reservations_fit(colours, pool),
            0 <= frame,
            self.is_managed_frame(frame),
            !self.is_free_frame(frame),
        ensures
            pool.spec_unused(colours.spec_of(frame)) + 1 <= usize::MAX,
    {
        self.lemma_colour_bounded(colours);
        self.lemma_managed_bounded();
        vstd::arithmetic::div_mod::lemma_mod_bound(frame, colours.spec_count() as int);
    }

}

} // verus!
