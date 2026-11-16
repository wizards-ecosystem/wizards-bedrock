#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

// Thirty-two bits, so the product fits a `u128` and a conversion packs into one `u64`.
const MAX_MULTIPLIER: u64 = u32::MAX as u64;

const MAX_SHIFT: u32 = 63;

const SHIFT_BITS: u32 = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Timebase {
    multiplier: u64,
    shift: u32,
}

impl Timebase {
    pub const fn new(numerator: u64, denominator: u64) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            return None;
        }
        let denominator = denominator as u128;
        let mut shift = MAX_SHIFT;
        loop {
            let multiplier = (((numerator as u128) << shift) + denominator / 2) / denominator;
            if multiplier > 0 && multiplier <= MAX_MULTIPLIER as u128 {
                return Some(Self {
                    multiplier: multiplier as u64,
                    shift,
                });
            }
            if shift == 0 {
                return None;
            }
            shift -= 1;
        }
    }

    pub const fn nanoseconds(hz: u64) -> Option<Self> {
        Self::new(NANOSECONDS_PER_SECOND, hz)
    }

    pub const fn ticks(hz: u64) -> Option<Self> {
        Self::new(hz, NANOSECONDS_PER_SECOND)
    }

    pub const fn is_calibrated(self) -> bool {
        self.multiplier != 0
    }

    pub const fn convert(self, value: u64) -> u64 {
        let scaled = (value as u128 * self.multiplier as u128) >> self.shift;
        if scaled > u64::MAX as u128 {
            u64::MAX
        } else {
            scaled as u64
        }
    }

    pub const fn to_bits(self) -> u64 {
        (self.shift as u64) << SHIFT_BITS | self.multiplier
    }

    pub const fn from_bits(bits: u64) -> Self {
        Self {
            multiplier: bits & MAX_MULTIPLIER,
            shift: ((bits >> SHIFT_BITS) as u32) & MAX_SHIFT,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PollLimit {
    Deadline(u64),
    Attempts(u32),
}

impl PollLimit {
    pub const fn after_ms(now_ns: Option<u64>, timeout_ms: u64, fallback_attempts: u32) -> Self {
        match now_ns {
            Some(now) => Self::Deadline(now.saturating_add(timeout_ms.saturating_mul(1_000_000))),
            None => Self::Attempts(fallback_attempts),
        }
    }

    pub const fn permit(&mut self, now_ns: Option<u64>) -> bool {
        match self {
            Self::Deadline(deadline) => match now_ns {
                Some(now) => now < *deadline,
                None => false,
            },
            Self::Attempts(remaining) if *remaining > 0 => {
                *remaining -= 1;
                true
            }
            Self::Attempts(_) => false,
        }
    }
}

#[cfg(test)]
mod tests;
