//! The timer's divide encoding and the count it is armed with.
//!
//! Two pieces, both hardware-free. The divide-configuration register scatters
//! a four-bit value across bits 0, 1 and 3 (Intel SDM Vol. 3A §11.5.4), which
//! is exactly the kind of layout that earns a transcribed table and a test.
//! And a one-shot timer is armed by writing a count, which the architecture
//! bounds at both ends: zero stops the hardware rather than firing it soon,
//! and thirty-two bits is as far ahead as it can be asked to look.

/// How the bus clock is divided before it drives the timer's countdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divide {
    /// The bus clock, undivided.
    By1,
    /// Every second edge — also what the register resets to.
    By2,
    /// Every fourth edge.
    By4,
    /// Every eighth edge.
    By8,
    /// Every sixteenth edge.
    By16,
    /// Every thirty-second edge.
    By32,
    /// Every sixty-fourth edge.
    By64,
    /// Every hundred-twenty-eighth edge.
    By128,
}

impl Divide {
    /// The value to write, spread across register bits 0, 1 and 3.
    pub const fn bits(self) -> u32 {
        match self {
            Self::By1 => 0b1011,
            Self::By2 => 0b0000,
            Self::By4 => 0b0001,
            Self::By8 => 0b0010,
            Self::By16 => 0b0011,
            Self::By32 => 0b1000,
            Self::By64 => 0b1001,
            Self::By128 => 0b1010,
        }
    }

    /// The divisor this encodes.
    pub const fn factor(self) -> u32 {
        match self {
            Self::By1 => 1,
            Self::By2 => 2,
            Self::By4 => 4,
            Self::By8 => 8,
            Self::By16 => 16,
            Self::By32 => 32,
            Self::By64 => 64,
            Self::By128 => 128,
        }
    }
}

/// The count to arm a one-shot timer with, for a wait of `ticks` of the
/// divided clock.
///
/// Clamped at both ends, because both ends mean something the caller did not
/// ask for. Zero is the architecture's way of spelling "stopped", so a wait
/// of no time at all becomes the soonest the hardware can be asked for — one
/// tick — rather than a timer that never fires. And a wait further ahead than
/// the counter's thirty-two bits becomes the furthest it can express, which
/// is a caller that must re-arm when it expires rather than one that has been
/// silently given a wrapped-around count.
pub const fn initial_count(ticks: u64) -> u32 {
    if ticks == 0 {
        return 1;
    }
    if ticks > u32::MAX as u64 {
        return u32::MAX;
    }
    ticks as u32
}

/// The longest wait one arming can express, in ticks of the divided clock.
pub const MAX_TICKS: u64 = u32::MAX as u64;

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[Divide] = &[
        Divide::By1,
        Divide::By2,
        Divide::By4,
        Divide::By8,
        Divide::By16,
        Divide::By32,
        Divide::By64,
        Divide::By128,
    ];

    /// SDM Vol. 3A §11.5.4's divide-value table, transcribed rather than
    /// derived: bits 1:0 carry the low half of the value and bit 3 the high
    /// bit, with divide-by-one the all-set case.
    #[test]
    fn every_divide_encoding_matches_the_manual() {
        for (divide, bits, factor) in [
            (Divide::By2, 0b0000, 2),
            (Divide::By4, 0b0001, 4),
            (Divide::By8, 0b0010, 8),
            (Divide::By16, 0b0011, 16),
            (Divide::By32, 0b1000, 32),
            (Divide::By64, 0b1001, 64),
            (Divide::By128, 0b1010, 128),
            (Divide::By1, 0b1011, 1),
        ] {
            assert_eq!(divide.bits(), bits, "{divide:?} bits");
            assert_eq!(divide.factor(), factor, "{divide:?} factor");
        }
    }

    /// Bit 2 is reserved, and the architecture faults on reserved bits in this
    /// register.
    #[test]
    fn the_reserved_bit_stays_clear() {
        for divide in ALL {
            assert_eq!(divide.bits() & (1 << 2), 0, "{divide:?}");
        }
    }

    #[test]
    fn no_two_divides_share_an_encoding() {
        for (index, divide) in ALL.iter().enumerate() {
            for other in &ALL[index + 1..] {
                assert_ne!(divide.bits(), other.bits(), "{divide:?} and {other:?}");
            }
        }
    }

    #[test]
    fn a_wait_inside_the_counter_is_the_count() {
        assert_eq!(initial_count(1), 1);
        assert_eq!(initial_count(3_000_000), 3_000_000);
        assert_eq!(initial_count(MAX_TICKS), u32::MAX);
    }

    /// The clamp that matters: a scheduler asking for a deadline that has
    /// already passed must get an interrupt, and a zero count would instead
    /// stop the timer for good.
    #[test]
    fn a_wait_of_no_time_still_arms_the_timer() {
        assert_eq!(initial_count(0), 1);
    }

    /// The other clamp: a sleep longer than the counter can express is armed
    /// as far ahead as it goes, and the caller re-arms for the remainder.
    #[test]
    fn a_wait_past_the_counter_is_the_counter() {
        assert_eq!(initial_count(MAX_TICKS + 1), u32::MAX);
        assert_eq!(initial_count(u64::MAX), u32::MAX);
    }
}
