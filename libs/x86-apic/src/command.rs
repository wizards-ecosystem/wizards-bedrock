//! The interrupt command register.
//!
//! Writing it sends an interrupt to a processor. The only destination this
//! kernel names is itself — the mechanism it uses to prove its own interrupt
//! path end to end — and a command addressed by shorthand carries no
//! destination, so the upper half of the register stays zero.

use crate::local_vector::DeliverableVector;

/// Bits 8–10: delivery mode. Zero delivers the vector as written, which is
/// everything short of the startup and reset commands that bring up another
/// processor.
const DELIVERY_FIXED: u32 = 0;

/// Bit 14: asserted rather than de-asserted. The de-assert form exists only for
/// the level-triggered INIT of very old multiprocessor firmware, so every
/// command this kernel builds sets it.
const ASSERT: u32 = 1 << 14;

/// Bits 18–19: destination shorthand. One means the processor that sent it.
const SHORTHAND_SELF: u32 = 1 << 18;

/// The contents of [`crate::register::COMMAND`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Command(u32);

impl Command {
    /// Build a self-directed command from a deliverable source.
    pub const fn self_interrupt(vector: DeliverableVector) -> Self {
        Self(vector.get().get() as u32 | DELIVERY_FIXED | ASSERT | SHORTHAND_SELF)
    }

    /// The value to write.
    ///
    /// The register is 64 bits wide and its upper half is the destination,
    /// which a shorthand makes irrelevant — so a caller widens this and writes
    /// zeros above it.
    pub const fn bits(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests;
