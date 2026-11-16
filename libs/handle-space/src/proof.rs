use vstd::prelude::verus;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
use crate::Entry;
use crate::{Mixer, Slot};

verus! {

impl Mixer {
    #[verifier::type_invariant]
    pub(crate) open spec fn leaves_the_reserved_bits(self) -> bool {
        self.0 & 3 == 0
    }

    pub open(crate) spec fn spec_bits(self) -> u64 {
        self.0
    }
}

impl Slot {
    #[verifier::type_invariant]
    pub(crate) open spec fn fits_its_fields(self) -> bool {
        self.index < 0x4_0000 && self.generation < 0x1000_0000_0000
    }

    pub open(crate) spec fn spec_index(self) -> u32 {
        self.index
    }

    pub open(crate) spec fn spec_generation(self) -> u64 {
        self.generation
    }
}

// Handles, Table: a handle value is a function of the mixer, the slot and the generation.
pub open spec fn handle_value(index: u32, generation: u64, mixer: u64) -> u64 {
    ((((generation << 18u64) | index as u64) << 2u64) | 3) ^ mixer
}

// What a value names under a mixer, when its reserved bits and generation are well formed.
pub open spec fn decoded(value: u64, mixer: u64) -> Option<(u32, u64)> {
    let base = value ^ mixer;
    if base & 3 != 3 || (base >> 2u64) >> 18u64 >= 0x1000_0000_0000 {
        None
    } else {
        Some((((base >> 2u64) & 0x3_ffff) as u32, (base >> 2u64) >> 18u64))
    }
}

// Handles, Table: a handle value decodes only under the mixer that issued it. Under that mixer it
// names its own slot, and two mixers give one slot two different values.
pub proof fn lemma_decodes_only_under_its_mixer(
    index: u32,
    generation: u64,
    mine: u64,
    theirs: u64,
)
    requires
        handle_value(index, generation, mine) == handle_value(index, generation, theirs),
    ensures
        mine == theirs,
{
    let low = index as u64;
    assert((((((generation << 18u64) | low) << 2u64) | 3) ^ mine) == (((((generation << 18u64)
        | low) << 2u64) | 3) ^ theirs) ==> mine == theirs) by (bit_vector);
}

// Handles, Table: a slot retires rather than repeat a generation.
pub open spec fn retires_rather_than_repeats(generation: u64, next: Entry) -> bool {
    if generation + 1 < 0x1000_0000_0000 {
        next == Entry::Free((generation + 1) as u64)
    } else {
        next == Entry::Retired
    }
}

pub open spec fn quota_live(bits: u64) -> u32 {
    (bits % 0x1_0000_0000) as u32
}

pub open spec fn quota_reserved(bits: u64) -> u32 {
    (bits / 0x1_0000_0000) as u32
}

pub open spec fn charged_below(bits: u64, limit: usize) -> bool {
    quota_live(bits) + quota_reserved(bits) < limit
}

pub(crate) proof fn lemma_widths()
    ensures
        crate::SLOTS == 0x4_0000,
        crate::GENERATIONS == 0x1000_0000_0000,
{
    assert(1u32 << 18u32 == 0x4_0000u32) by (bit_vector);
    assert(1u64 << 44u32 == 0x1000_0000_0000u64) by (bit_vector);
}

pub(crate) proof fn lemma_reserved_bits_clear(bits: u64)
    ensures
        (bits & !3u64) & 3 == 0,
{
    assert((bits & !3u64) & 3u64 == 0) by (bit_vector);
}

pub(crate) proof fn lemma_value_decodes_to_its_slot(index: u32, generation: u64, mixer: u64)
    requires
        index < 0x4_0000,
        generation < 0x1000_0000_0000,
        mixer & 3 == 0,
    ensures
        decoded(handle_value(index, generation, mixer), mixer) == Some((index, generation)),
        handle_value(index, generation, mixer) != 0,
{
    let low = index as u64;
    assert((((((generation << 18u64) | low) << 2u64) | 3) ^ mixer) ^ mixer == (((generation
        << 18u64) | low) << 2u64) | 3) by (bit_vector);
    assert(((((generation << 18u64) | low) << 2u64) | 3) & 3 == 3 && (((((generation << 18u64)
        | low) << 2u64) | 3) >> 2u64) >> 18u64 == generation && (((((generation << 18u64) | low)
        << 2u64) | 3) >> 2u64) & 0x3_ffff == low) by (bit_vector)
        requires
            low < 0x4_0000u64,
            generation < 0x1000_0000_0000u64,
    ;
    assert(((((generation << 18u64) | low) << 2u64) | 3) ^ mixer != 0) by (bit_vector)
        requires
            mixer & 3 == 0,
    ;
}

pub(crate) proof fn lemma_slot_from_value(value: u64, mixer: u64)
    ensures
        ((value ^ mixer) >> 2u64) & 0x3_ffff < 0x4_0000,
        (value ^ mixer) & 3 == 3 && ((value ^ mixer) >> 2u64) >> 18u64 < 0x1000_0000_0000
            ==> handle_value(
            (((value ^ mixer) >> 2u64) & 0x3_ffff) as u32,
            ((value ^ mixer) >> 2u64) >> 18u64,
            mixer,
        ) == value,
{
    let base = value ^ mixer;
    let low = (base >> 2u64) & 0x3_ffff;
    assert(((value ^ mixer) >> 2u64) & 0x3_ffff < 0x4_0000u64) by (bit_vector);
    if base & 3 == 3 && (base >> 2u64) >> 18u64 < 0x1000_0000_0000 {
        assert(low as u32 as u64 == low);
        assert(((((((value ^ mixer) >> 2u64) >> 18u64) << 18u64) | (((value ^ mixer) >> 2u64)
            & 0x3_ffff)) << 2u64 | 3) ^ mixer == value) by (bit_vector)
            requires
                (value ^ mixer) & 3 == 3,
                ((value ^ mixer) >> 2u64) >> 18u64 < 0x1000_0000_0000u64,
        ;
    }
}

pub(crate) proof fn lemma_quota_halves(bits: u64)
    ensures
        bits >> 32u64 == bits / 0x1_0000_0000,
        bits as u32 == (bits % 0x1_0000_0000) as u32,
        1u64 << 32u64 == 0x1_0000_0000,
{
    assert(bits as u32 == (bits % 0x1_0000_0000u64) as u32) by (bit_vector);
    assert(bits >> 32u64 == bits / 0x1_0000_0000u64) by (bit_vector);
    assert(1u64 << 32u64 == 0x1_0000_0000u64) by (bit_vector);
}

} // verus!
