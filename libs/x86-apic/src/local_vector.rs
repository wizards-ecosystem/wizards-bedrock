use crate::vector::InterruptVector;

const DELIVERY_FIXED: u32 = 0;

const MASK: u32 = 1 << 16;

const TIMER_ONE_SHOT: u32 = 0b00 << 17;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeliverableVector(InterruptVector);

impl DeliverableVector {
    pub const fn configured(vector: InterruptVector, spurious: InterruptVector) -> Option<Self> {
        if vector.get() == spurious.get() {
            return None;
        }
        Some(Self(vector))
    }

    pub const fn get(self) -> InterruptVector {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry(u32);

impl Entry {
    pub const MASKED: Self = Self(MASK);

    pub const fn fixed(vector: InterruptVector) -> Self {
        Self(vector.get() as u32 | DELIVERY_FIXED)
    }

    pub(crate) const fn one_shot(vector: InterruptVector) -> Self {
        Self(Self::fixed(vector).bits() | TIMER_ONE_SHOT)
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn is_masked(self) -> bool {
        self.0 & MASK != 0
    }
}
