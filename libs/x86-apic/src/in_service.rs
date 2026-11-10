//! Selection of the interrupt the local APIC currently owns.

use crate::lifecycle::Interrupts;
use crate::vector::InterruptVector;

/// Whether the frame behind an EOI claim remains the active trap frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameLiveness {
    /// The claimed frame is still the innermost active trap.
    Live,
    Stale,
}

/// Whether live ISR state still names the claimed dismissal target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DismissalOwnership {
    /// The claimed vector remains the highest-priority in-service vector.
    Owned,
    Unowned,
}

/// The highest-priority vector in the eight ISR words, which an EOI write dismisses.
///
/// A lower set bit therefore cannot authorize the EOI while a higher-priority delivery is nested.
pub fn highest(registers: &[u32; 8]) -> Option<u8> {
    for (bank, bits) in registers.iter().enumerate().rev() {
        if *bits != 0 {
            let bit = 31 - bits.leading_zeros() as usize;
            return u8::try_from(bank * 32 + bit).ok();
        }
    }
    None
}

/// Whether `vector` is the one an EOI write would dismiss.
pub fn owns(registers: &[u32; 8], vector: InterruptVector) -> bool {
    highest(registers) == Some(vector.get())
}

/// Whether a live trap still owns the sole EOI write it claimed.
pub const fn may_complete(
    interrupts: Interrupts,
    frame_liveness: FrameLiveness,
    frame_vector: u64,
    token_vector: InterruptVector,
    dismissal: DismissalOwnership,
) -> bool {
    matches!(interrupts, Interrupts::Excluded)
        && matches!(frame_liveness, FrameLiveness::Live)
        && frame_vector == token_vector.get() as u64
        && matches!(dismissal, DismissalOwnership::Owned)
}

#[cfg(test)]
mod tests;
