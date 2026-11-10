//! The interrupt command register, which this kernel only ever addresses to itself.

use crate::local_vector::DeliverableVector;

/// Bits 10:8: delivery mode, fixed.
const DELIVERY_FIXED: u32 = 0;

/// Bit 14: assert, which every command this kernel builds sets.
const ASSERT: u32 = 1 << 14;

/// Bits 19:18: destination shorthand, one for the sending processor.
const SHORTHAND_SELF: u32 = 1 << 18;

/// The contents of [`crate::register::COMMAND`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Command(u32);

impl Command {
    /// Build a self-directed command from a deliverable source.
    pub const fn self_interrupt(vector: DeliverableVector) -> Self {
        Self(vector.get().get() as u32 | DELIVERY_FIXED | ASSERT | SHORTHAND_SELF)
    }

    /// The low half; the caller widens it, zeroing the destination a shorthand ignores.
    pub const fn bits(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests;
