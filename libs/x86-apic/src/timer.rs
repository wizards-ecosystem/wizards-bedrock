//! The timer's divide encoding and the count it is armed with.
//!
//! Two pieces, both hardware-free. The divide-configuration register scatters
//! a four-bit value across bits 0, 1 and 3 (Intel SDM Vol. 3A §11.5.4), which
//! is exactly the kind of layout that earns a transcribed table and a test.
//! And a one-shot timer is armed by writing a count, which the architecture
//! bounds at both ends: zero stops the hardware rather than firing it soon,
//! and thirty-two bits is as far ahead as it can be asked to look.

use crate::local_vector::{DeliverableVector, Entry};

/// Why a one-shot timer entry cannot be published yet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum OneShotError {
    /// The current countdown has not stopped.
    Running,
}

impl core::fmt::Display for OneShotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Write::write_str(f, "the local APIC timer is still running")
    }
}

impl core::error::Error for OneShotError {}

/// Plan the sole register write that unmasks a one-shot timer.
pub const fn one_shot_entry(
    current_count: u32,
    vector: DeliverableVector,
) -> Result<Entry, OneShotError> {
    if current_count != 0 {
        return Err(OneShotError::Running);
    }
    Ok(Entry::one_shot(vector.get()))
}

/// How the bus clock is divided before it drives the timer's countdown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

#[cfg(test)]
mod tests;
