use core::fmt;

use crate::local_vector::{DeliverableVector, Entry};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum OneShotError {
    Running,
}

impl fmt::Display for OneShotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the local APIC timer is still running")
    }
}

impl core::error::Error for OneShotError {}

pub const fn one_shot_entry(
    current_count: u32,
    vector: DeliverableVector,
) -> Result<Entry, OneShotError> {
    if current_count != 0 {
        return Err(OneShotError::Running);
    }
    Ok(Entry::one_shot(vector.get()))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Divide {
    By1,
    By2,
    By4,
    By8,
    By16,
    By32,
    By64,
    By128,
}

impl Divide {
    // The value is spread across register bits 0, 1 and 3.
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

// Zero would stop the timer; after a clamped `u32::MAX` expires the caller re-arms the rest.
pub const fn initial_count(ticks: u64) -> u32 {
    if ticks == 0 {
        return 1;
    }
    if ticks > u32::MAX as u64 {
        return u32::MAX;
    }
    ticks as u32
}
