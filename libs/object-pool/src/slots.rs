#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus, verus_spec, verus_verify};

use crate::{Refusal, SLOTS_PER_PAGE};

#[verus_verify]
pub(crate) const END: u16 = u16::MAX;

#[verus_verify]
pub(crate) const LIVE: u16 = u16::MAX - 1;

#[verus_verify]
pub(crate) const RETIRED: u16 = u16::MAX - 2;

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Issued {
    pub slot: u16,
    pub generation: u32,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Released {
    Held,
    NowZero,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Removed {
    Freed,
    Retired,
}

#[verus_verify]
#[derive(PartialEq, Eq, Debug)]
pub struct Slots<const N: usize> {
    pub(crate) generation: [u32; N],
    pub(crate) strong: [u32; N],
    pub(crate) list: FreeList<N>,
}

// FIFO free list: reuse spreads over every free slot, so each slot retires as late as possible.
// Its fields change together in one call, after which the invariant of `Slots` is checked.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FreeList<const N: usize> {
    pub(crate) next_free: [u16; N],
    pub(crate) head: u16,
    pub(crate) tail: u16,
    pub(crate) free: u16,
    pub(crate) live: u16,
}

#[verus_verify]
impl<const N: usize> Clone for Slots<N> {
    fn clone(&self) -> Self {
        proof! {
            use_type_invariant(self);
        }
        Self {
            generation: self.generation,
            strong: self.strong,
            list: self.list,
        }
    }
}

verus! {

impl<const N: usize> Slots<N> {
    pub fn new(floor: u32) -> (r: Option<Self>)
        ensures
            match r {
                Some(slots) => {
                    &&& 0 < N <= 1024
                    &&& slots.spec_free() == N && slots.spec_live() == 0
                    &&& forall|slot: int|
                        0 <= slot < N ==> !slots.is_live(slot) && !slots.is_retired(slot)
                            && slots.generation_of(slot) == floor && slots.strong_of(slot) == 0
                },
                None => N == 0 || N > 1024,
            },
    {
        proof {
            crate::proof::lemma_page_constants();
        }
        if N == 0 || N > usize::from(SLOTS_PER_PAGE) {
            return None;
        }
        let mut next_free = [END; N];
        let mut slot: u16 = 0;
        while usize::from(slot) < N
            invariant
                0 < N <= 1024,
                slot <= N,
                forall|at: int|
                    0 <= at < slot ==> next_free@[at] == crate::proof::first_link(at, N as int),
                forall|at: int| slot <= at < N ==> next_free@[at] == END,
            decreases N - slot,
        {
            if usize::from(slot) + 1 < N {
                next_free[usize::from(slot)] = slot + 1;
            }
            slot += 1;
        }
        proof {
            crate::proof::lemma_first_list(next_free@);
        }
        Some(
            Self {
                generation: [floor; N],
                strong: [0; N],
                list: FreeList { next_free, head: 0, tail: slot - 1, free: slot, live: 0 },
            },
        )
    }

    // A page that later takes this page's index starts at this floor; `None` once a slot retired.
    pub fn floor_after(&self, floor: u32) -> (r: Option<u32>)
        ensures
            match r {
                Some(bound) => floor <= bound && forall|slot: int|
                    0 <= slot < N ==> crate::proof::below_the_floor(*self, slot, bound),
                None => exists|slot: int| 0 <= slot < N && #[trigger] self.closes_its_index(slot),
            },
    {
        let mut bound = floor;
        let mut at = 0;
        while at < N
            invariant
                at <= N,
                floor <= bound,
                forall|slot: int|
                    0 <= slot < at ==> crate::proof::below_the_floor(*self, slot, bound),
            decreases N - at,
        {
            let above = match self.list.next_free[at] {
                RETIRED => {
                    proof {
                        assert(self.closes_its_index(at as int));
                    }
                    return None;
                },
                LIVE => match self.generation[at].checked_add(1) {
                    Some(above) => {
                        proof {
                            assert(self.is_live(at as int));
                        }
                        above
                    },
                    None => {
                        proof {
                            assert(self.closes_its_index(at as int));
                        }
                        return None;
                    },
                },
                _ => {
                    proof {
                        assert(!self.is_live(at as int) && !self.is_retired(at as int));
                    }
                    self.generation[at]
                },
            };
            let ghost lower = bound;
            if above > bound {
                bound = above;
            }
            proof {
                assert forall|slot: int| 0 <= slot < at implies crate::proof::below_the_floor(
                    *self,
                    slot,
                    bound,
                ) by {
                    assert(crate::proof::below_the_floor(*self, slot, lower));
                }
                assert(crate::proof::below_the_floor(*self, at as int, bound));
            }
            at += 1;
        }
        Some(bound)
    }
}

} // verus!

#[verus_verify]
impl<const N: usize> Slots<N> {
    #[verus_spec(r =>
        ensures
            r == self.spec_free(),
            (r == 0) == (self.insert_refuses()),
    )]
    pub fn free(&self) -> u16 {
        proof! {
            use_type_invariant(self);
        }
        self.list.free
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_live(),
            (r == 0) == (forall|slot: int| 0 <= slot < N ==> !self.is_live(slot)),
    )]
    pub fn live(&self) -> u16 {
        proof! {
            use_type_invariant(self);
            crate::proof::lemma_none_live(*self);
        }
        self.list.live
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(issued) => {
                    &&& !old(self).insert_refuses()
                    &&& (issued.slot as int) < N
                    &&& !old(self).is_live(issued.slot as int)
                    &&& !old(self).is_retired(issued.slot as int)
                    &&& issued.generation == old(self).generation_of(issued.slot as int)
                    &&& final(self).resolves_to(issued.slot as int, issued.generation)
                    &&& final(self).strong_of(issued.slot as int) == 1
                    &&& final(self).spec_free() + 1 == old(self).spec_free()
                    &&& final(self).spec_live() == old(self).spec_live() + 1
                    &&& crate::proof::only_touches(*old(self), *final(self), issued.slot as int)
                },
                Err(refusal) => refusal == Refusal::Full && old(self).insert_refuses()
                    && *final(self) == *old(self),
            },
            crate::proof::slot_never_repeats_a_generation(*old(self), *final(self)),
    )]
    pub fn insert(&mut self) -> Result<Issued, Refusal> {
        proof! {
            use_type_invariant(&*self);
            let list = self.list;
            crate::proof::lemma_head_names_a_free_slot(
                list.links(),
                list.head,
                list.tail,
                list.free,
                list.live,
            );
        }
        let slot = self.list.head;
        let at = usize::from(slot);
        if at >= N {
            return Err(Refusal::Full);
        }
        self.list.take(at);
        self.strong[at] = 1;
        Ok(Issued {
            slot,
            generation: self.generation[at],
        })
    }

    #[verus_spec(r =>
        ensures
            r == self.resolves_to(slot as int, generation),
    )]
    pub fn resolves(&self, slot: u16, generation: u32) -> bool {
        self.live_at(slot, generation).is_some()
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(()) => old(self).resolves_to(slot as int, generation)
                    && 0 < old(self).strong_of(slot as int) < u32::MAX
                    && final(self).strong_of(slot as int) == old(self).strong_of(slot as int) + 1,
                Err(Refusal::Stale) => !old(self).resolves_to(slot as int, generation),
                Err(Refusal::Condemned) => old(self).strong_of(slot as int) == 0,
                Err(Refusal::Saturated) => old(self).strong_of(slot as int) == u32::MAX,
                Err(_) => false,
            },
            r is Err ==> *final(self) == *old(self),
            crate::proof::counts_only(*old(self), *final(self), slot as int),
    )]
    pub fn acquire(&mut self, slot: u16, generation: u32) -> Result<(), Refusal> {
        proof! {
            use_type_invariant(&*self);
        }
        let Some(at) = self.live_at(slot, generation) else {
            return Err(Refusal::Stale);
        };
        if self.strong[at] == 0 {
            return Err(Refusal::Condemned);
        }
        if self.strong[at] == u32::MAX {
            return Err(Refusal::Saturated);
        }
        self.strong[at] += 1;
        Ok(())
    }

    #[verus_spec(r =>
        ensures
            crate::proof::now_zero_once_per_fall(*old(self), *final(self), slot as int, r),
            match r {
                Err(Refusal::Stale) => !old(self).resolves_to(slot as int, generation),
                Err(Refusal::Condemned) => true,
                Err(_) => false,
                Ok(_) => old(self).resolves_to(slot as int, generation),
            },
            r is Err ==> *final(self) == *old(self),
            crate::proof::counts_only(*old(self), *final(self), slot as int),
    )]
    pub fn release(&mut self, slot: u16, generation: u32) -> Result<Released, Refusal> {
        proof! {
            use_type_invariant(&*self);
        }
        let Some(at) = self.live_at(slot, generation) else {
            return Err(Refusal::Stale);
        };
        if self.strong[at] == 0 {
            return Err(Refusal::Condemned);
        }
        self.strong[at] -= 1;
        if self.strong[at] == 0 {
            Ok(Released::NowZero)
        } else {
            Ok(Released::Held)
        }
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(removed) => {
                    &&& old(self).resolves_to(slot as int, generation)
                    &&& old(self).strong_of(slot as int) == 0
                    &&& crate::proof::removed_index_never_resolves(
                        *final(self),
                        slot as int,
                        generation,
                    )
                    &&& final(self).spec_live() + 1 == old(self).spec_live()
                    &&& match removed {
                        Removed::Freed => generation < u32::MAX && !final(self).is_live(slot as int)
                            && !final(self).is_retired(slot as int)
                            && final(self).spec_free() == old(self).spec_free() + 1,
                        Removed::Retired => generation == u32::MAX
                            && final(self).is_retired(slot as int)
                            && final(self).spec_free() == old(self).spec_free(),
                    }
                    &&& crate::proof::only_touches(*old(self), *final(self), slot as int)
                },
                Err(Refusal::Stale) => !old(self).resolves_to(slot as int, generation),
                Err(Refusal::Held) => old(self).strong_of(slot as int) != 0,
                Err(_) => false,
            },
            r is Err ==> *final(self) == *old(self),
            crate::proof::slot_never_repeats_a_generation(*old(self), *final(self)),
    )]
    pub fn remove(&mut self, slot: u16, generation: u32) -> Result<Removed, Refusal> {
        proof! {
            use_type_invariant(&*self);
        }
        let Some(at) = self.live_at(slot, generation) else {
            return Err(Refusal::Stale);
        };
        if self.strong[at] != 0 {
            return Err(Refusal::Held);
        }
        if self.generation[at] == u32::MAX {
            self.list.retire(at);
            return Ok(Removed::Retired);
        }
        self.generation[at] += 1;
        let tail = usize::from(self.list.tail);
        self.list.put(slot, at, tail);
        Ok(Removed::Freed)
    }

    #[verus_spec(r =>
        ensures
            match r {
                Some(at) => at == slot as usize && self.resolves_to(slot as int, generation),
                None => !self.resolves_to(slot as int, generation),
            },
    )]
    fn live_at(&self, slot: u16, generation: u32) -> Option<usize> {
        let at = usize::from(slot);
        if at < N && self.list.next_free[at] == LIVE && self.generation[at] == generation {
            Some(at)
        } else {
            None
        }
    }
}

#[verus_verify]
impl<const N: usize> FreeList<N> {
    #[verus_spec(
        requires
            self.holds(),
            at == self.head,
            at < N,
        ensures
            final(self).holds(),
            final(self).links() == old(self).links().update(old(self).head as int, LIVE),
            final(self).free + 1 == old(self).free,
            final(self).live == old(self).live + 1,
        no_unwind
    )]
    fn take(&mut self, at: usize) {
        proof! {
            let (head, tail, free, live) = (self.head, self.tail, self.free, self.live);
            crate::proof::lemma_take_head(self.links(), head, tail, free, live);
        }
        self.head = self.next_free[at];
        if self.head == END {
            self.tail = END;
        }
        self.next_free[at] = LIVE;
        self.free -= 1;
        self.live += 1;
    }

    #[verus_spec(
        requires
            self.holds(),
            at == slot,
            at < N,
            self.links()[slot as int] == LIVE,
            tail == self.tail,
        ensures
            final(self).holds(),
            final(self).links()[slot as int] == END,
            forall|other: int|
                0 <= other < N && other != slot ==> crate::proof::same_state(
                    old(self).links()[other],
                    final(self).links()[other],
                ),
            final(self).free == old(self).free + 1,
            final(self).live + 1 == old(self).live,
        no_unwind
    )]
    fn put(&mut self, slot: u16, at: usize, tail: usize) {
        proof! {
            let (head, tail, free, live) = (self.head, self.tail, self.free, self.live);
            crate::proof::lemma_put(self.links(), head, tail, free, live, slot);
        }
        self.next_free[at] = END;
        if tail < N {
            self.next_free[tail] = slot;
        } else {
            self.head = slot;
        }
        self.tail = slot;
        self.free += 1;
        self.live -= 1;
    }

    #[verus_spec(
        requires
            self.holds(),
            at < N,
            self.links()[at as int] == LIVE,
        ensures
            final(self).holds(),
            final(self).links() == old(self).links().update(at as int, RETIRED),
            final(self).free == old(self).free,
            final(self).live + 1 == old(self).live,
        no_unwind
    )]
    fn retire(&mut self, at: usize) {
        proof! {
            let (head, tail, free, live) = (self.head, self.tail, self.free, self.live);
            crate::proof::lemma_retire(self.links(), head, tail, free, live, at as u16);
        }
        self.live -= 1;
        self.next_free[at] = RETIRED;
    }
}

verus! {

impl<const N: usize> Slots<N> {
    // A teardown that must empty the page finds what is still live by this.
    pub fn first_live(&self) -> (r: Option<Issued>)
        ensures
            r matches Some(issued) ==> self.resolves_to(issued.slot as int, issued.generation),
    {
        let mut slot: u16 = 0;
        while usize::from(slot) < N && slot < u16::MAX
            decreases u16::MAX - slot,
        {
            let generation = self.generation[usize::from(slot)];
            if self.resolves(slot, generation) {
                return Some(Issued { slot, generation });
            }
            slot += 1;
        }
        None
    }
}

} // verus!
