#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests;

// Always set, so zero is never a handle.
const RESERVED_BITS: u32 = 2;

pub const RESERVED: u64 = 0b11;

const INDEX_BITS: u32 = 18;

const GENERATION_BITS: u32 = 64 - INDEX_BITS - RESERVED_BITS;

pub const SLOTS: u32 = 1 << INDEX_BITS;

const GENERATIONS: u64 = 1 << GENERATION_BITS;

// XORed into every value a table issues; derived from the world key once per process.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mixer(u64);

impl Mixer {
    pub const fn from_derived(bytes: [u8; 8]) -> Self {
        Self(u64::from_le_bytes(bytes) & !RESERVED)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slot {
    index: u32,
    generation: u64,
}

impl Slot {
    pub const fn new(index: u32, generation: u64) -> Option<Self> {
        if index >= SLOTS || generation >= GENERATIONS {
            return None;
        }
        Some(Self { index, generation })
    }

    pub const fn index(self) -> u32 {
        self.index
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }

    pub const fn encode(self, mixer: Mixer) -> u64 {
        let base = (self.generation << INDEX_BITS) | self.index as u64;
        ((base << RESERVED_BITS) | RESERVED) ^ mixer.bits()
    }

    // `Some` proves only that the encoding is well formed.
    pub const fn decode(value: u64, mixer: Mixer) -> Option<Self> {
        let base = value ^ mixer.bits();
        if base & RESERVED != RESERVED {
            return None;
        }
        let base = base >> RESERVED_BITS;
        Self::new((base & (SLOTS as u64 - 1)) as u32, base >> INDEX_BITS)
    }
}

// A slot whose generation would reach the limit retires instead of repeating one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Entry {
    Free(u64),
    Live(u64),
    Retired,
}

impl Entry {
    pub const fn fill(self) -> Option<(Self, u64)> {
        match self {
            Self::Free(generation) => Some((Self::Live(generation), generation)),
            Self::Live(_) | Self::Retired => None,
        }
    }

    pub const fn empty(self, generation: u64) -> Option<Self> {
        match self {
            Self::Live(live) if live == generation => match generation.checked_add(1) {
                Some(next) if next < GENERATIONS => Some(Self::Free(next)),
                _ => Some(Self::Retired),
            },
            Self::Free(_) | Self::Live(_) | Self::Retired => None,
        }
    }

    pub const fn is_live(self, generation: u64) -> bool {
        matches!(self, Self::Live(live) if live == generation)
    }
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

    pub const fn drop_live(bits: u64) -> Option<u64> {
        if Self::live(bits) == 0 {
            return None;
        }
        Some(bits - 1)
    }
}
