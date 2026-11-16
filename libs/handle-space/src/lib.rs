#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod proof;
#[cfg(test)]
mod tests;

#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus_spec, verus_verify};

// Always set, so zero is never a handle.
#[verus_verify]
const RESERVED_BITS: u32 = 2;

#[verus_verify]
pub const RESERVED: u64 = 0b11;

#[verus_verify]
const INDEX_BITS: u32 = 18;

#[verus_verify]
const GENERATION_BITS: u32 = 64 - INDEX_BITS - RESERVED_BITS;

#[verus_verify]
pub const SLOTS: u32 = 1 << 18;

#[verus_verify]
const GENERATIONS: u64 = 1 << GENERATION_BITS;

// XORed into every value a table issues; derived from the world key once per process.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mixer(u64);

#[verus_verify]
impl Mixer {
    #[verus_spec(r =>
        ensures
            r.spec_bits() & RESERVED == 0,
    )]
    pub const fn from_derived(bytes: [u8; 8]) -> Self {
        let bits = little_endian(bytes);
        proof! {
            proof::lemma_reserved_bits_clear(bits);
        }
        Self(bits & !RESERVED)
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_bits(),
            r & RESERVED == 0,
    )]
    pub const fn bits(self) -> u64 {
        proof! {
            use_type_invariant(self);
        }
        self.0
    }
}

// `u64::from_le_bytes` has no specification, so the crate assembles the bytes itself.
#[verus_verify]
const fn little_endian(bytes: [u8; 8]) -> u64 {
    bytes[0] as u64
        | (bytes[1] as u64) << 8
        | (bytes[2] as u64) << 16
        | (bytes[3] as u64) << 24
        | (bytes[4] as u64) << 32
        | (bytes[5] as u64) << 40
        | (bytes[6] as u64) << 48
        | (bytes[7] as u64) << 56
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slot {
    index: u32,
    generation: u64,
}

#[verus_verify]
impl Slot {
    #[verus_spec(r =>
        ensures
            match r {
                Some(slot) => {
                    &&& slot.spec_index() == index && slot.spec_generation() == generation
                    &&& index < SLOTS && generation < 0x1000_0000_0000
                },
                None => index >= SLOTS || generation >= 0x1000_0000_0000,
            },
    )]
    pub const fn new(index: u32, generation: u64) -> Option<Self> {
        proof! {
            proof::lemma_widths();
        }
        if index >= SLOTS || generation >= GENERATIONS {
            return None;
        }
        Some(Self { index, generation })
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_index(),
            r < SLOTS,
    )]
    pub const fn index(self) -> u32 {
        proof! {
            use_type_invariant(self);
            proof::lemma_widths();
        }
        self.index
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_generation(),
            r < 0x1000_0000_0000,
    )]
    pub const fn generation(self) -> u64 {
        proof! {
            use_type_invariant(self);
        }
        self.generation
    }

    #[verus_spec(r =>
        ensures
            r == proof::handle_value(self.spec_index(), self.spec_generation(), mixer.spec_bits()),
            r != 0,
            proof::decoded(r, mixer.spec_bits()) == Some(
                (self.spec_index(), self.spec_generation()),
            ),
    )]
    pub const fn encode(self, mixer: Mixer) -> u64 {
        proof! {
            use_type_invariant(self);
            use_type_invariant(mixer);
            proof::lemma_value_decodes_to_its_slot(self.index, self.generation, mixer.0);
        }
        let base = (self.generation << INDEX_BITS) | self.index as u64;
        ((base << RESERVED_BITS) | RESERVED) ^ mixer.bits()
    }

    // `Some` proves only that the encoding is well formed.
    #[verus_spec(r =>
        ensures
            match r {
                Some(slot) => {
                    &&& proof::decoded(value, mixer.spec_bits()) == Some(
                        (slot.spec_index(), slot.spec_generation()),
                    )
                    &&& proof::handle_value(
                        slot.spec_index(),
                        slot.spec_generation(),
                        mixer.spec_bits(),
                    ) == value
                },
                None => proof::decoded(value, mixer.spec_bits()) is None,
            },
    )]
    pub const fn decode(value: u64, mixer: Mixer) -> Option<Self> {
        let base = value ^ mixer.bits();
        if base & RESERVED != RESERVED {
            return None;
        }
        let base = base >> RESERVED_BITS;
        proof! {
            proof::lemma_widths();
            proof::lemma_slot_from_value(value, mixer.0);
        }
        Self::new((base & (SLOTS as u64 - 1)) as u32, base >> INDEX_BITS)
    }
}

// A slot whose generation would reach the limit retires instead of repeating one.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Entry {
    Free(u64),
    Live(u64),
    Retired,
}

#[verus_verify]
impl Entry {
    #[verus_spec(r =>
        ensures
            match r {
                Some((next, issued)) => self == Entry::Free(issued) && next == Entry::Live(issued),
                None => !(self is Free),
            },
    )]
    pub const fn fill(self) -> Option<(Self, u64)> {
        match self {
            Self::Free(generation) => Some((Self::Live(generation), generation)),
            Self::Live(_) | Self::Retired => None,
        }
    }

    #[verus_spec(r =>
        ensures
            match r {
                Some(next) => self == Entry::Live(generation)
                    && proof::retires_rather_than_repeats(generation, next),
                None => self != Entry::Live(generation),
            },
    )]
    pub const fn empty(self, generation: u64) -> Option<Self> {
        proof! {
            proof::lemma_widths();
        }
        match self {
            Self::Live(live) if live == generation => match generation.checked_add(1) {
                Some(next) if next < GENERATIONS => Some(Self::Free(next)),
                _ => Some(Self::Retired),
            },
            Self::Free(_) | Self::Live(_) | Self::Retired => None,
        }
    }

    // A seal names the object again in the same slot under the next generation, so the old value
    // stops resolving; the last generation cannot renew.
    #[verus_spec(r =>
        ensures
            match r {
                Some((next, issued)) => self == Entry::Live(generation)
                    && issued == generation + 1
                    && issued < GENERATIONS
                    && next == Entry::Live(issued)
                    && next != Entry::Live(generation),
                None => self != Entry::Live(generation) || generation + 1 >= GENERATIONS,
            },
    )]
    pub const fn renew(self, generation: u64) -> Option<(Self, u64)> {
        proof! {
            proof::lemma_widths();
        }
        match self {
            Self::Live(live) if live == generation && generation < GENERATIONS - 1 => {
                Some((Self::Live(generation + 1), generation + 1))
            }
            Self::Free(_) | Self::Live(_) | Self::Retired => None,
        }
    }

    #[verus_spec(r =>
        ensures
            r == (self == Entry::Live(generation)),
    )]
    pub const fn is_live(self, generation: u64) -> bool {
        matches!(self, Self::Live(live) if live == generation)
    }
}

#[verus_verify]
#[derive(Debug)]
pub struct Quota;

#[verus_verify]
impl Quota {
    pub const EMPTY: u64 = 0;

    #[verus_spec(r =>
        ensures
            r == proof::quota_live(bits),
    )]
    pub const fn live(bits: u64) -> u32 {
        proof! {
            proof::lemma_quota_halves(bits);
        }
        bits as u32
    }

    #[verus_spec(r =>
        ensures
            r == proof::quota_reserved(bits),
    )]
    pub const fn reserved(bits: u64) -> u32 {
        proof! {
            proof::lemma_quota_halves(bits);
        }
        (bits >> 32) as u32
    }

    #[verus_spec(r =>
        ensures
            r == proof::quota_live(bits) + proof::quota_reserved(bits),
    )]
    pub const fn charged(bits: u64) -> u64 {
        Self::live(bits) as u64 + Self::reserved(bits) as u64
    }

    #[verus_spec(r =>
        ensures
            match r {
                Some(next) => proof::charged_below(bits, limit)
                    && proof::quota_reserved(next) == proof::quota_reserved(bits) + 1
                    && proof::quota_live(next) == proof::quota_live(bits),
                None => proof::quota_reserved(bits) == u32::MAX
                    || !proof::charged_below(bits, limit),
            },
    )]
    pub const fn reserve(bits: u64, limit: usize) -> Option<u64> {
        proof! {
            proof::lemma_quota_halves(bits);
        }
        if Self::reserved(bits) == u32::MAX || Self::charged(bits) >= limit as u64 {
            return None;
        }
        Some(bits + (1 << 32))
    }

    #[verus_spec(r =>
        ensures
            match r {
                Some(next) => proof::quota_reserved(next) + 1 == proof::quota_reserved(bits)
                    && proof::quota_live(next) == proof::quota_live(bits) + 1,
                None => proof::quota_reserved(bits) == 0 || proof::quota_live(bits) == u32::MAX,
            },
    )]
    pub const fn commit(bits: u64) -> Option<u64> {
        proof! {
            proof::lemma_quota_halves(bits);
        }
        if Self::reserved(bits) == 0 || Self::live(bits) == u32::MAX {
            return None;
        }
        Some(bits - (1 << 32) + 1)
    }

    #[verus_spec(r =>
        ensures
            match r {
                Some(next) => proof::quota_reserved(next) + 1 == proof::quota_reserved(bits)
                    && proof::quota_live(next) == proof::quota_live(bits),
                None => proof::quota_reserved(bits) == 0,
            },
    )]
    pub const fn release(bits: u64) -> Option<u64> {
        proof! {
            proof::lemma_quota_halves(bits);
        }
        if Self::reserved(bits) == 0 {
            return None;
        }
        Some(bits - (1 << 32))
    }

    #[verus_spec(r =>
        ensures
            match r {
                Some(next) => proof::charged_below(bits, limit)
                    && proof::quota_reserved(next) == proof::quota_reserved(bits)
                    && proof::quota_live(next) == proof::quota_live(bits) + 1,
                None => proof::quota_live(bits) == u32::MAX || !proof::charged_below(bits, limit),
            },
    )]
    pub const fn take(bits: u64, limit: usize) -> Option<u64> {
        proof! {
            proof::lemma_quota_halves(bits);
        }
        if Self::live(bits) == u32::MAX || Self::charged(bits) >= limit as u64 {
            return None;
        }
        Some(bits + 1)
    }

    #[verus_spec(r =>
        ensures
            match r {
                Some(next) => proof::quota_reserved(next) == proof::quota_reserved(bits)
                    && proof::quota_live(next) + 1 == proof::quota_live(bits),
                None => proof::quota_live(bits) == 0,
            },
    )]
    pub const fn drop_live(bits: u64) -> Option<u64> {
        proof! {
            proof::lemma_quota_halves(bits);
        }
        if Self::live(bits) == 0 {
            return None;
        }
        Some(bits - 1)
    }
}
