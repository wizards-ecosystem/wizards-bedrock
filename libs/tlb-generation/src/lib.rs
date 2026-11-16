#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod proof;

#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus, verus_spec, verus_verify};

#[verus_verify]
pub const MAX_CPUS: u32 = 64;

#[verus_verify]
pub const SLOTS: usize = 6;

#[verus_verify]
const STRIPPED_PCID: u16 = 0x800;

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cpu(u8);

#[verus_verify]
impl Cpu {
    #[verus_spec(r =>
        ensures
            r matches Some(cpu) ==> cpu.spec_index() == index,
            r.is_some() == (index < 64),
    )]
    pub const fn new(index: u32) -> Option<Self> {
        if index < MAX_CPUS {
            Some(Self(index as u8))
        } else {
            None
        }
    }

    #[verus_spec(r =>
        ensures
            r as int == self.spec_index(),
    )]
    pub const fn index(self) -> u8 {
        self.0
    }

    #[verus_spec(r =>
        requires
            self.spec_index() < 64,
        ensures
            r == 1u64 << self.0 as u64,
    )]
    const fn bit(self) -> u64 {
        1 << (self.0 as u64)
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CpuSet(u64);

#[verus_verify]
impl CpuSet {
    #[verus_spec(r =>
        ensures
            forall|cpu: int| !#[trigger] r.spec_contains(cpu),
    )]
    pub const fn empty() -> Self {
        proof! {
            proof::lemma_no_bits(0);
        }
        Self(0)
    }

    #[verus_spec(r =>
        ensures
            r.spec_bits() == bits,
    )]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_bits(),
    )]
    pub const fn bits(self) -> u64 {
        self.0
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_contains(cpu.spec_index()),
    )]
    pub const fn contains(self, cpu: Cpu) -> bool {
        proof! {
            use_type_invariant(cpu);
            proof::lemma_test_bit(self.0, cpu.0 as u64);
        }
        self.0 & cpu.bit() != 0
    }

    #[verus_spec(r =>
        ensures
            forall|other: int| #[trigger] r.spec_contains(other) == (self.spec_contains(other)
                || other == cpu.spec_index()),
    )]
    #[must_use]
    pub const fn with(self, cpu: Cpu) -> Self {
        proof! {
            use_type_invariant(cpu);
            proof::lemma_with_bit(self.0, cpu.0 as u64);
        }
        Self(self.0 | cpu.bit())
    }

    #[verus_spec(r =>
        ensures
            forall|other: int| #[trigger] r.spec_contains(other) == (self.spec_contains(other)
                && other != cpu.spec_index()),
    )]
    #[must_use]
    pub const fn without(self, cpu: Cpu) -> Self {
        proof! {
            use_type_invariant(cpu);
            proof::lemma_without_bit(self.0, cpu.0 as u64);
        }
        Self(self.0 & !cpu.bit())
    }

    #[verus_spec(r =>
        ensures
            r == forall|cpu: int| !#[trigger] self.spec_contains(cpu),
    )]
    pub const fn is_empty(self) -> bool {
        proof! {
            if self.0 == 0 {
                proof::lemma_no_bits(self.0);
            } else {
                let bit = proof::lemma_some_bit(self.0);
                assert(self.spec_contains(bit));
            }
        }
        self.0 == 0
    }
}

// A generation never repeats, so a space that would pass the last one is refused.
#[verus_spec(r =>
    ensures
        r matches Some(next) ==> next == generation + 1,
        r.is_none() == (generation == u64::MAX),
)]
#[must_use]
pub const fn advance(generation: u64) -> Option<u64> {
    generation.checked_add(1)
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Extent {
    Range,
    Whole,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Skip,
    Range,
    Whole,
}

// Returns the flush to perform and the generation this CPU has then seen.
#[verus_spec(r =>
    ensures
        proof::catches_up(seen, request, latest, extent, r.0, r.1),
)]
#[must_use]
pub const fn catch_up(seen: u64, request: u64, latest: u64, extent: Extent) -> (Action, u64) {
    let newest = if request > latest { request } else { latest };
    if seen >= newest || request <= seen {
        return (Action::Skip, seen);
    }
    if matches!(extent, Extent::Range) && request == seen + 1 && request == newest {
        return (Action::Range, request);
    }
    (Action::Whole, newest)
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Slot {
    space: u64,
    seen: u64,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Load {
    slot: usize,
    flushes: bool,
    evicted: Option<u64>,
}

#[verus_verify]
impl Load {
    #[verus_spec(r =>
        ensures
            r == self.spec_slot(),
    )]
    pub const fn slot(self) -> usize {
        self.slot
    }

    #[verus_spec(r =>
        ensures
            r == self.full_pcid_spec(),
            r as int == self.spec_slot() + 1,
    )]
    pub const fn full_pcid(self) -> u16 {
        proof! {
            use_type_invariant(self);
        }
        self.slot as u16 + 1
    }

    #[verus_spec(r =>
        ensures
            r == self.full_pcid_spec() | 0x800,
    )]
    pub const fn stripped_pcid(self) -> u16 {
        self.full_pcid() | STRIPPED_PCID
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_flushes(),
    )]
    pub const fn flushes(self) -> bool {
        self.flushes
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_evicted(),
    )]
    pub const fn evicted(self) -> Option<u64> {
        self.evicted
    }
}

verus! {

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slots {
    entries: [Option<Slot>; SLOTS],
    victim: usize,
}

impl Slots {
    pub const fn new() -> (r: Self)
        ensures
            forall|space: u64| r.seen_of(space).is_none(),
    {
        Self { entries: [None; SLOTS], victim: 0 }
    }

    // A space already in a slot reloads without a flush only if that slot saw `latest`.
    pub fn load(&mut self, space: u64, latest: u64) -> (r: Load)
        ensures
            r.spec_slot() < SLOTS,
            proof::stale_only_with_flush(old(self).seen_of(space), latest, r.spec_flushes()),
            final(self).seen_of(space) == Some(
                match old(self).seen_of(space) {
                    Some(seen) => if seen >= latest {
                        seen
                    } else {
                        latest
                    },
                    None => latest,
                },
            ),
            final(self).slot_of(space) == Some(r.spec_slot() as int),
            forall|other: u64|
                other != space && r.spec_evicted() != Some(other) ==> final(self).seen_of(other)
                    == #[trigger] old(self).seen_of(other),
            r.spec_evicted() matches Some(gone) ==> gone != space && final(self).seen_of(
                gone,
            ).is_none(),
    {
        proof {
            use_type_invariant(&*self);
        }
        let mut index = 0;
        while index < SLOTS
            invariant
                index <= SLOTS,
                *self == *old(self),
                forall|other: int| 0 <= other < index ==> !#[trigger] self.holds(other, space),
            decreases SLOTS - index,
        {
            proof {
                use_type_invariant(&*self);
            }
            if let Some(seen) = seen_in(self.entries[index], space) {
                let flushes = seen < latest;
                let ghost before = *self;
                proof {
                    self.lemma_slot_of(index as int, space);
                    assert(old(self).seen_of(space) == Some(seen));
                    self.lemma_replace_keeps_distinct(index as int, space);
                }
                if flushes {
                    self.entries[index] = Some(Slot { space, seen: latest });
                }
                proof {
                    if flushes {
                        self.lemma_replaced(before, index as int, space);
                    }
                }
                return Load { slot: index, flushes, evicted: None };
            }
            index += 1;
        }
        proof {
            self.lemma_absent(space);
        }
        index = 0;
        while index < SLOTS
            invariant
                index <= SLOTS,
                *self == *old(self),
                old(self).seen_of(space).is_none(),
                forall|other: int| 0 <= other < SLOTS ==> !#[trigger] self.holds(other, space),
                forall|other: int| 0 <= other < index ==> #[trigger] self.entries@[other].is_some(),
            decreases SLOTS - index,
        {
            proof {
                use_type_invariant(&*self);
            }
            if self.entries[index].is_none() {
                let ghost before = *self;
                proof {
                    self.lemma_replace_keeps_distinct(index as int, space);
                }
                self.entries[index] = Some(Slot { space, seen: latest });
                proof {
                    self.lemma_replaced(before, index as int, space);
                }
                let r = Load { slot: index, flushes: true, evicted: None };
                return r;
            }
            index += 1;
        }
        proof {
            use_type_invariant(&*self);
        }
        let index = self.victim;
        let mut evicted = None;
        if let Some(slot) = self.entries[index] {
            evicted = Some(slot.space);
        }
        let ghost before = *self;
        proof {
            assert(!self.holds(index as int, space));
            self.lemma_replace_keeps_distinct(index as int, space);
        }
        self.entries[index] = Some(Slot { space, seen: latest });
        proof {
            self.lemma_replaced(before, index as int, space);
            before.lemma_evicted(*self, index as int, space);
            use_type_invariant(&*self);
        }
        let ghost placed = *self;
        self.victim = if index + 1 == SLOTS {
            0
        } else {
            index + 1
        };
        proof {
            assert(self.entries@ == placed.entries@);
            assert forall|other: u64|
                other != space && evicted != Some(other) implies self.seen_of(other)
                == #[trigger] old(self).seen_of(other) by {
                assert(!before.holds(index as int, other));
                assert(placed.seen_of(other) == before.seen_of(other));
            }
        }
        Load { slot: index, flushes: true, evicted }
    }

    // Only for the space loaded on this CPU; a space not loaded flushes when it is next loaded.
    pub fn catch_up(&mut self, space: u64, request: u64, latest: u64, extent: Extent) -> (r: Action)
        ensures
            old(self).seen_of(space) matches Some(seen) ==> final(self).seen_of(space) matches Some(
                now,
            ) && proof::catches_up(seen, request, latest, extent, r, now),
            old(self).seen_of(space).is_none() ==> r == Action::Skip && *final(self) == *old(
                self,
            ),
            forall|other: u64|
                other != space ==> final(self).seen_of(other) == #[trigger] old(self).seen_of(
                    other,
                ),
    {
        proof {
            use_type_invariant(&*self);
        }
        let mut index = 0;
        while index < SLOTS
            invariant
                index <= SLOTS,
                *self == *old(self),
                forall|other: int| 0 <= other < index ==> !#[trigger] self.holds(other, space),
            decreases SLOTS - index,
        {
            proof {
                use_type_invariant(&*self);
            }
            if let Some(had) = seen_in(self.entries[index], space) {
                let (action, seen) = catch_up(had, request, latest, extent);
                let ghost before = *self;
                proof {
                    self.lemma_slot_of(index as int, space);
                    self.lemma_replace_keeps_distinct(index as int, space);
                }
                self.entries[index] = Some(Slot { space, seen });
                proof {
                    self.lemma_replaced(before, index as int, space);
                }
                return action;
            }
            index += 1;
        }
        proof {
            self.lemma_absent(space);
        }
        Action::Skip
    }
}

const fn seen_in(entry: Option<Slot>, space: u64) -> (r: Option<u64>)
    ensures
        r == match entry {
            Some(slot) => if slot.space == space {
                Some(slot.seen)
            } else {
                None
            },
            None => None,
        },
{
    match entry {
        Some(slot) if slot.space == space => Some(slot.seen),
        _ => None,
    }
}

impl Default for Slots {
    fn default() -> (r: Self)
        ensures
            forall|space: u64| r.seen_of(space).is_none(),
    {
        Self::new()
    }
}

} // verus!

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shootdown {
    generation: u64,
    remaining: CpuSet,
}

#[verus_verify]
impl Shootdown {
    #[verus_spec(r =>
        ensures
            r.spec_generation() == generation,
            forall|cpu: int| #[trigger] r.spec_remaining().spec_contains(cpu) == (
                targets.spec_contains(cpu) && cpu != requester.spec_index()),
    )]
    pub const fn new(targets: CpuSet, requester: Cpu, generation: u64) -> Self {
        Self {
            generation,
            remaining: targets.without(requester),
        }
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_generation(),
    )]
    pub const fn generation(self) -> u64 {
        self.generation
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_remaining(),
    )]
    pub const fn remaining(self) -> CpuSet {
        self.remaining
    }

    #[verus_spec(r =>
        ensures
            proof::acknowledged_once(self, cpu, r),
    )]
    #[must_use]
    pub const fn acknowledge(self, cpu: Cpu) -> Option<Self> {
        if !self.remaining.contains(cpu) {
            return None;
        }
        Some(Self {
            generation: self.generation,
            remaining: self.remaining.without(cpu),
        })
    }

    #[verus_spec(r =>
        ensures
            proof::complete_only_when_acknowledged(self, r),
    )]
    pub const fn is_complete(self) -> bool {
        self.remaining.is_empty()
    }
}
