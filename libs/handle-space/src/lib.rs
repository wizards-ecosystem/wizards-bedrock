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
