//! Local vector table entries: one shared layout for every source the APIC owns itself.

use crate::vector::InterruptVector;

/// Bits 10:8: delivery mode, fixed, the only one this kernel programs.
const DELIVERY_FIXED: u32 = 0;

/// Bit 16: the source is disconnected while this is set.
const MASK: u32 = 1 << 16;

/// Bits 18:17: the timer's mode; TSC-deadline, the third value, is not implemented here.
const TIMER_MODE: u32 = 0b11 << 17;

/// Count down once and stop, which is also the reset value.
const TIMER_ONE_SHOT: u32 = 0b00 << 17;

/// Reload the initial count at zero and keep counting.
const TIMER_PERIODIC: u32 = 0b01 << 17;

/// Bits 7:0: the vector.
const VECTOR: u32 = 0xFF;

/// A vector proven deliverable under one configured spurious source.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeliverableVector(InterruptVector);

impl DeliverableVector {
    /// Admit a vector only when both sources are architectural and distinct.
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

/// One local vector table entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry(u32);

impl Entry {
    /// An entry that delivers nothing, which is also the architectural reset value.
    pub const MASKED: Self = Self(MASK);

    /// Deliver `vector` to this processor whenever the source fires.
    pub const fn fixed(vector: InterruptVector) -> Self {
        Self(vector.get() as u32 | DELIVERY_FIXED)
    }

    /// Deliver `vector` once when the timer's countdown reaches zero, and then stop.
    pub const fn one_shot(vector: InterruptVector) -> Self {
        Self(Self::fixed(vector).bits() | TIMER_ONE_SHOT)
    }

    /// Deliver `vector` every time the timer's countdown reaches zero.
    pub const fn periodic(vector: InterruptVector) -> Self {
        Self(Self::fixed(vector).bits() | TIMER_PERIODIC)
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether the source is disconnected.
    pub const fn is_masked(self) -> bool {
        self.0 & MASK != 0
    }

    pub const fn vector(self) -> u8 {
        (self.0 & VECTOR) as u8
    }

    /// Whether the timer reloads and continues at zero.
    pub const fn is_periodic(self) -> bool {
        self.0 & TIMER_MODE == TIMER_PERIODIC
    }

    /// Whether the timer counts down once and stops.
    pub const fn is_one_shot(self) -> bool {
        self.0 & TIMER_MODE == TIMER_ONE_SHOT
    }
}

#[cfg(test)]
mod tests;
