use crate::lifecycle::Interrupts;
use crate::pending::highest;
use crate::vector::InterruptVector;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameLiveness {
    Live,
    Stale,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DismissalOwnership {
    Owned,
    Unowned,
}

// An EOI dismisses the highest in-service vector, so only that one is owned.
pub fn owns(registers: &[u32; 8], vector: InterruptVector) -> bool {
    highest(registers) == Some(vector.get())
}

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
