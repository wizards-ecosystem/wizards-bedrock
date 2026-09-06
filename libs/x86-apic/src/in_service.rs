//! Selection of the interrupt the local APIC currently owns.

use crate::lifecycle::Interrupts;
use crate::vector::InterruptVector;

/// Whether the frame behind an EOI claim remains the active trap frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameLiveness {
    /// The claimed frame is still the innermost active trap.
    Live,
    /// The claimed frame is no longer active.
    Stale,
}

/// Whether live ISR state still names the claimed dismissal target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DismissalOwnership {
    /// The claimed vector remains the highest-priority in-service vector.
    Owned,
    /// No in-service dismissal belongs to the claim.
    Unowned,
}

/// Return the highest-priority vector present in the eight ISR words.
///
/// The APIC dismisses that vector when software writes EOI, so a lower set bit
/// cannot authorize the write while a higher-priority delivery is nested.
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
mod tests {
    use super::*;

    fn vector(raw: u8) -> InterruptVector {
        InterruptVector::new(raw).expect("test vector is assignable")
    }

    #[test]
    fn every_one_hot_is_its_exact_vector() {
        for vector in u8::MIN..=u8::MAX {
            let mut registers = [0u32; 8];
            registers[usize::from(vector) / 32] = 1 << (vector % 32);
            assert_eq!(highest(&registers), Some(vector));
        }
    }

    #[test]
    fn the_highest_set_vector_is_the_dismissal_target() {
        let mut registers = [0u32; 8];
        for vector in [16u8, 0x40, 0x41, 0x80, 0xfe] {
            registers[usize::from(vector) / 32] |= 1 << (vector % 32);
        }
        assert_eq!(highest(&registers), Some(0xfe));
        assert_eq!(highest(&[0; 8]), None);
    }

    #[test]
    fn absent_and_wrong_sources_plan_no_fake_eoi_write() {
        let mut registers = [0u32; 8];
        registers[2] = 1 << 1;

        for (state, requested) in [([0; 8], 0x41), (registers, 0x40)] {
            let mut writes = 0;
            if owns(&state, vector(requested)) {
                writes += 1;
            }
            assert_eq!(writes, 0, "vector {requested:#x}");
        }
        assert!(owns(&registers, vector(0x41)));
    }

    #[test]
    fn only_named_live_ownership_observations_authorize_completion() {
        for interrupts in [Interrupts::Excluded, Interrupts::Enabled] {
            for frame_liveness in [FrameLiveness::Live, FrameLiveness::Stale] {
                for frame_vector in [0x40, 0x41] {
                    for token_vector in [vector(0x40), vector(0x41)] {
                        for dismissal in [DismissalOwnership::Owned, DismissalOwnership::Unowned] {
                            let expected = interrupts == Interrupts::Excluded
                                && frame_liveness == FrameLiveness::Live
                                && frame_vector == u64::from(token_vector.get())
                                && dismissal == DismissalOwnership::Owned;
                            assert_eq!(
                                may_complete(
                                    interrupts,
                                    frame_liveness,
                                    frame_vector,
                                    token_vector,
                                    dismissal,
                                ),
                                expected,
                                "{interrupts:?} {frame_liveness:?} {frame_vector:#x} \
                                 {token_vector:?} {dismissal:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
