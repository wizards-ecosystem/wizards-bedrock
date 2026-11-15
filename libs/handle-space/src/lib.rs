#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

// Always set, so zero is never a handle.
const RESERVED_BITS: u32 = 2;

pub const RESERVED: u32 = 0b11;

const INDEX_BITS: u32 = 18;

const GENERATION_BITS: u32 = 32 - INDEX_BITS - RESERVED_BITS;

pub const SLOTS: u32 = 1 << INDEX_BITS;

const GENERATIONS: u32 = 1 << GENERATION_BITS;

pub const fn next_table_identity(identity: u64) -> Option<u64> {
    identity.checked_add(1)
}

// Usually makes the same slot look different in another table; never an authority.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mixer(u32);

// Odd, so multiplying by it is a bijection over `u32`.
const MULTIPLIER: u32 = 0x9E37_79B9;

// Added after the multiply, so table zero is not a fixed point.
const ADDEND: u32 = 0x7F4A_7C15;

impl Mixer {
    pub const fn new(seed: u32) -> Self {
        Self(seed & !RESERVED)
    }

    pub const fn for_table(id: u32) -> Self {
        let mixed = Self::new(id.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND));
        if mixed.is_identity() {
            Self::new(MULTIPLIER)
        } else {
            mixed
        }
    }

    const fn is_identity(self) -> bool {
        self.0 == 0
    }

    pub const fn bits(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slot {
    index: u32,
    generation: u32,
}

impl Slot {
    pub const fn new(index: u32, generation: u32) -> Option<Self> {
        if index >= SLOTS || generation >= GENERATIONS {
            return None;
        }
        Some(Self { index, generation })
    }

    pub const fn index(self) -> u32 {
        self.index
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }

    pub const fn encode(self, mixer: Mixer) -> u32 {
        let base = (self.generation << INDEX_BITS) | self.index;
        ((base << RESERVED_BITS) | RESERVED) ^ mixer.bits()
    }

    // `Some` proves only that the encoding is well formed.
    pub const fn decode(value: u32, mixer: Mixer) -> Option<Self> {
        let base = value ^ mixer.bits();
        if base & RESERVED != RESERVED {
            return None;
        }
        let base = base >> RESERVED_BITS;
        Self::new(base & (SLOTS - 1), base >> INDEX_BITS)
    }
}

// Wraps rather than saturating, which would make the last generation reusable.
pub const fn next_generation(generation: u32) -> u32 {
    generation.wrapping_add(1) % GENERATIONS
}

#[derive(Debug)]
pub struct Quota;

impl Quota {
    pub const EMPTY: u64 = 0;

    pub const fn live(bits: u64) -> u32 {
        bits as u32
    }

    pub const fn reserved(bits: u64) -> u32 {
        (bits >> 32) as u32
    }

    pub const fn charged(bits: u64) -> u64 {
        Self::live(bits) as u64 + Self::reserved(bits) as u64
    }

    pub const fn reserve(bits: u64, limit: usize) -> Option<u64> {
        if Self::reserved(bits) == u32::MAX || Self::charged(bits) >= limit as u64 {
            return None;
        }
        Some(bits + (1 << 32))
    }

    pub const fn commit(bits: u64) -> Option<u64> {
        if Self::reserved(bits) == 0 || Self::live(bits) == u32::MAX {
            return None;
        }
        Some(bits - (1 << 32) + 1)
    }

    pub const fn release(bits: u64) -> Option<u64> {
        if Self::reserved(bits) == 0 {
            return None;
        }
        Some(bits - (1 << 32))
    }

    pub const fn take(bits: u64, limit: usize) -> Option<u64> {
        if Self::live(bits) == u32::MAX || Self::charged(bits) >= limit as u64 {
            return None;
        }
        Some(bits + 1)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rights(u32);

impl Rights {
    pub const NONE: Self = Self(0);

    pub const DUPLICATE: Self = Self(1 << 0);

    pub const TRANSFER: Self = Self(1 << 1);

    pub const WAIT: Self = Self(1 << 2);

    pub const INSPECT: Self = Self(1 << 3);

    pub const UNIVERSAL: Self = Self(0b1111);

    pub const READ: Self = Self(1 << 8);

    pub const WRITE: Self = Self(1 << 9);

    // Never held with `WRITE` in a mapping.
    pub const EXECUTE: Self = Self(1 << 10);

    pub const MAP: Self = Self(1 << 11);

    pub const TRAVERSE: Self = Self(1 << 12);

    pub const ENUMERATE: Self = Self(1 << 13);

    pub const MODIFY: Self = Self(1 << 14);

    // An undefined bit refuses the whole word: masking would grant quietly.
    pub const fn from_bits(bits: u32) -> Option<Self> {
        if bits & !Self::ALL.0 != 0 {
            return None;
        }
        Some(Self(bits))
    }

    pub const ALL: Self = Self(
        Self::DUPLICATE.0
            | Self::TRANSFER.0
            | Self::WAIT.0
            | Self::INSPECT.0
            | Self::READ.0
            | Self::WRITE.0
            | Self::EXECUTE.0
            | Self::MAP.0
            | Self::TRAVERSE.0
            | Self::ENUMERATE.0
            | Self::MODIFY.0,
    );

    pub const fn bits(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn are_meaningful_for(self, per_type: Self) -> bool {
        self.difference(Self::UNIVERSAL).difference(per_type).0 == 0
    }

    pub const fn attenuate(self, wanted: Self) -> Option<Self> {
        if !self.contains(wanted) {
            return None;
        }
        Some(wanted)
    }
}

#[cfg(test)]
mod tests;
