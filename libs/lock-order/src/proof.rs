use vstd::prelude::verus;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
use crate::Pair;
use crate::{HeldRanks, Key};

verus! {

impl HeldRanks {
    pub open(crate) spec fn spec_bits(self) -> u64 {
        self.0
    }
}

impl Key {
    pub open(crate) spec fn spec_node(self) -> u32 {
        self.node
    }

    pub open(crate) spec fn spec_index(self) -> u32 {
        self.index
    }
}

pub open spec fn holds_rank(word: u64, rank: u8) -> bool {
    rank < 64 && word & (1u64 << rank) != 0
}

// The word after taking `rank`: refused for rank zero, past the leaf, or below a held rank.
pub open spec fn taken(word: u64, rank: u8) -> Option<u64> {
    if rank == 0 || rank > 63 || word & (u64::MAX << rank) != 0 {
        None
    } else {
        Some(word | (1u64 << rank))
    }
}

pub open spec fn released(word: u64, rank: u8) -> Option<u64> {
    if rank == 0 || rank > 63 || word & (1u64 << rank) == 0 {
        None
    } else {
        Some(word & !(1u64 << rank))
    }
}

// Lock order: a processor takes a lock only above every rank it holds, so an equal rank is
// refused too, and the lock it took is then held.
pub proof fn lemma_takes_only_above_every_held_rank(word: u64, rank: u8)
    requires
        taken(word, rank) is Some,
    ensures
        forall|held: u8| holds_rank(word, held) ==> held < rank,
        holds_rank(taken(word, rank)->Some_0, rank),
{
    assert forall|held: u8| holds_rank(word, held) implies held < rank by {
        assert(word & (u64::MAX << rank) == 0 && rank <= held && held < 64 ==> word & (1u64
            << held) == 0) by (bit_vector);
    }
    assert(rank < 64 ==> (word | (1u64 << rank)) & (1u64 << rank) != 0) by (bit_vector);
}

// Lock order: a release restores exactly the word from before the acquire.
pub proof fn lemma_release_restores_the_word(word: u64, rank: u8)
    requires
        taken(word, rank) is Some,
    ensures
        released(taken(word, rank)->Some_0, rank) == Some(word),
{
    assert(rank < 64 && word & (u64::MAX << rank) == 0 ==> (word | (1u64 << rank)) & (1u64
        << rank) != 0 && ((word | (1u64 << rank)) & !(1u64 << rank)) == word) by (bit_vector);
}

pub open spec fn key_below(first: Key, second: Key) -> bool {
    first.spec_node() < second.spec_node() || (first.spec_node() == second.spec_node()
        && first.spec_index() < second.spec_index())
}

// Lock order: two locks of one rank are taken only in ascending order of their objects, and only
// one object compares equal to itself.
pub open spec fn ascending_by_object(first: Key, second: Key) -> Option<Pair> {
    if key_below(first, second) {
        Some(Pair::AsGiven)
    } else if key_below(second, first) {
        Some(Pair::Swapped)
    } else {
        None
    }
}

pub proof fn lemma_only_one_object_is_unordered(first: Key, second: Key)
    requires
        ascending_by_object(first, second) is None,
    ensures
        first == second,
{
}

} // verus!
