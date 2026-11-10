//! Local vector table entries.
//!
//! One 32-bit register for each interrupt source the APIC owns itself — its
//! timer, its two local pins, its error line — all sharing a layout. An entry
//! says which vector the source delivers on, or that it delivers nothing.

use crate::vector::InterruptVector;

/// Bits 8–10: delivery mode. Zero is a plain vector delivered to this
/// processor, which is the only mode this kernel programs. The others exist so
/// firmware can wire a pin to the non-maskable interrupt or to the legacy
/// controller, and undoing that is exactly what masking is for.
const DELIVERY_FIXED: u32 = 0;

/// Bit 16: the source is disconnected while this is set.
const MASK: u32 = 1 << 16;

/// Bits 17–18: the timer's mode, meaningful only in the timer's entry. The
/// third value, TSC-deadline, is not implemented here: it arms the timer from
/// a comparison against the timestamp counter rather than from a countdown,
/// which is a different interface, not another mode of this one.
const TIMER_MODE: u32 = 0b11 << 17;

/// The mode that counts down once and stops. Zero, which is also what the
/// register resets to.
const TIMER_ONE_SHOT: u32 = 0b00 << 17;

/// The mode that reloads the initial count at zero and keeps counting.
const TIMER_PERIODIC: u32 = 0b01 << 17;

/// Bits 0–7: the vector.
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

    /// The admitted vector.
    pub const fn get(self) -> InterruptVector {
        self.0
    }
}

/// One local vector table entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry(u32);

impl Entry {
    /// An entry that delivers nothing.
    ///
    /// This is also the value the architecture resets every entry to, so
    /// writing it puts a pin back the way the processor started rather than
    /// merely quiet.
    pub const MASKED: Self = Self(MASK);

    /// Deliver `vector` to this processor whenever the source fires.
    ///
    pub const fn fixed(vector: InterruptVector) -> Self {
        Self(vector.get() as u32 | DELIVERY_FIXED)
    }

    /// Deliver `vector` once when the timer's countdown reaches zero, and
    /// then stop.
    ///
    /// The mode a tickless kernel programs: each expiry is an event the
    /// scheduler asked for, and the timer does nothing again until it is
    /// armed for the next one.
    ///
    pub const fn one_shot(vector: InterruptVector) -> Self {
        Self(Self::fixed(vector).bits() | TIMER_ONE_SHOT)
    }

    /// Deliver `vector` every time the timer's countdown reaches zero.
    ///
    /// The reload is the hardware's: at zero the timer restarts from its
    /// initial count with no software involved, which is what makes the tick
    /// periodic rather than re-armed.
    ///
    pub const fn periodic(vector: InterruptVector) -> Self {
        Self(Self::fixed(vector).bits() | TIMER_PERIODIC)
    }

    /// Interpret what the register returned.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The value to write.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether the source is disconnected.
    pub const fn is_masked(self) -> bool {
        self.0 & MASK != 0
    }

    /// The vector this entry delivers on.
    pub const fn vector(self) -> u8 {
        (self.0 & VECTOR) as u8
    }

    /// Whether the timer reloads and continues at zero rather than stopping.
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
