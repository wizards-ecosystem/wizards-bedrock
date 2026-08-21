//! The timer's divide encoding and down-counter arithmetic.
//!
//! Two pieces, both hardware-free. The divide-configuration register scatters
//! a four-bit value across bits 0, 1 and 3 (Intel SDM Vol. 3A §11.5.4), which
//! is exactly the kind of layout that earns a transcribed table and a test.
//! And turning a tick count plus the down-counter's position into elapsed
//! milliseconds is arithmetic the kernel should not be trusted to improvise.

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

/// Milliseconds since the timer started: whole ticks, plus the fraction of the
/// current tick the down-counter has already covered.
///
/// The fraction never exceeds `ms_per_tick`, and reaches it only at the
/// instant the counter reads zero — the boundary the tick count is about to
/// claim, so the value is continuous across the increment rather than jumping.
/// A zero `initial_count` means the timer never calibrated, and the answer is
/// whole ticks alone, which for a timer that never started is zero.
pub const fn elapsed_milliseconds(
    ticks: u64,
    ms_per_tick: u32,
    initial_count: u32,
    current_count: u32,
) -> u64 {
    let whole = ticks.saturating_mul(ms_per_tick as u64);
    if initial_count == 0 {
        return whole;
    }
    // Both factors fit in 32 bits, so the product fits in 64 with room over.
    let counted = initial_count.saturating_sub(current_count) as u64;
    let fraction = counted * ms_per_tick as u64 / initial_count as u64;
    whole.saturating_add(fraction)
}

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
    fn interpolation_starts_a_tick_at_zero() {
        assert_eq!(elapsed_milliseconds(7, 10, 100_000, 100_000), 70);
    }

    /// Sweeping the counter from full to empty must never claim more than the
    /// tick in progress, and may claim all of it only at the instant the
    /// counter reads zero — where the value meets, exactly, what the counted
    /// boundary will read after the increment.
    #[test]
    fn interpolation_never_passes_the_next_tick() {
        for initial in [3u32, 7, 100, 100_000, 10_000_000] {
            for current in (0..=initial).rev() {
                let elapsed = elapsed_milliseconds(5, 10, initial, current);
                if current == 0 {
                    assert_eq!(elapsed, elapsed_milliseconds(6, 10, initial, initial));
                } else {
                    assert!(elapsed < 60, "initial {initial} current {current}");
                }
            }
        }
    }

    #[test]
    fn interpolation_is_monotonic_within_a_tick_and_across_the_boundary() {
        let initial = 10_000;
        let mut previous = 0;
        for current in (0..=initial).rev() {
            let elapsed = elapsed_milliseconds(3, 10, initial, current);
            assert!(elapsed >= previous, "current {current}");
            previous = elapsed;
        }
        assert!(elapsed_milliseconds(4, 10, initial, initial) >= previous);
    }

    /// The path a machine whose timer declined takes: no initial count means
    /// whole ticks only, and no ticks means zero.
    #[test]
    fn a_zero_initial_count_counts_whole_ticks_only() {
        assert_eq!(elapsed_milliseconds(12, 10, 0, 0), 120);
        assert_eq!(elapsed_milliseconds(0, 10, 0, 0), 0);
    }

    /// A read that races the hardware's reload can catch the counter above the
    /// initial count it is falling from; the fraction saturates to the start
    /// of the tick rather than wrapping.
    #[test]
    fn a_current_count_above_initial_saturates_to_the_tick_start() {
        assert_eq!(elapsed_milliseconds(4, 10, 100, 250), 40);
    }

    #[test]
    fn the_arithmetic_cannot_overflow_at_the_extremes() {
        assert_eq!(elapsed_milliseconds(u64::MAX, 1000, u32::MAX, 0), u64::MAX);
        assert_eq!(
            elapsed_milliseconds(0, u32::MAX, u32::MAX, 0),
            u64::from(u32::MAX)
        );
    }
}
