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
    pub const UNCALIBRATED: Self = Self {
        multiplier: 0,
        shift: 0,
    };

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

    pub const fn from_parts(multiplier: u64, shift: u32) -> Option<Self> {
        if multiplier > MAX_MULTIPLIER {
            return None;
        }
        if shift > MAX_SHIFT {
            return None;
        }
        Some(Self { multiplier, shift })
    }

    pub const fn multiplier(self) -> u64 {
        self.multiplier
    }

    pub const fn shift(self) -> u32 {
        self.shift
    }

    // The base is split because `base_ns << shift` overflows within seconds.
    pub const fn resume(self, base_ns: u64, base_fraction: u64, ticks: u64) -> u64 {
        let advanced = base_fraction as u128 + (ticks as u128 * self.multiplier as u128);
        let whole = advanced >> self.shift;
        if whole > u64::MAX as u128 {
            return u64::MAX;
        }
        base_ns.saturating_add(whole as u64)
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

// The caller supplies both memory-ordering barriers; a read must not span a full `u32` cycle.
#[derive(Debug)]
pub struct Seq;

impl Seq {
    pub const fn is_stable(seq: u32) -> bool {
        seq.is_multiple_of(2)
    }

    pub const fn begin(seq: u32) -> u32 {
        seq.wrapping_add(1)
    }

    pub const fn end(seq: u32) -> u32 {
        seq.wrapping_add(1)
    }

    pub const fn agrees(before: u32, after: u32) -> bool {
        before == after && Self::is_stable(before)
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
