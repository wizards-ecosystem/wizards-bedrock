//! Counter ticks to nanoseconds and back, as one rounded multiply and shift with no division.
//!
//! # Examples
//!
//! ```
//! use timebase::Timebase;
//!
//! // A counter measured at two and a half gigahertz.
//! let elapsed = Timebase::nanoseconds(2_500_000_000).unwrap();
//! assert_eq!(elapsed.convert(2_500_000_000), 1_000_000_000);
//!
//! // And the timer that has to be armed in its own ticks.
//! let countdown = Timebase::ticks(1_000_000_000).unwrap();
//! assert_eq!(countdown.convert(3_000_000), 3_000_000);
//!
//! // A conversion survives being published as one word.
//! assert_eq!(Timebase::from_bits(elapsed.to_bits()), elapsed);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Nanoseconds in a second, the unit on both sides of every conversion here.
pub const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

/// Thirty-two bits, so that the product fits a `u128` and a conversion packs into one `u64`.
const MAX_MULTIPLIER: u64 = u32::MAX as u64;

/// The largest defined shift, which [`Timebase::from_bits`] masks to since it accepts any bits.
const MAX_SHIFT: u32 = 63;

/// Bits of a packed conversion that hold the shift.
const SHIFT_BITS: u32 = 32;

/// One direction of one clock's conversion: multiply, then shift.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Timebase {
    multiplier: u64,
    shift: u32,
}

impl Timebase {
    /// A clock never measured, converting everything to zero; a zeroed word decodes to it.
    pub const UNCALIBRATED: Self = Self {
        multiplier: 0,
        shift: 0,
    };

    /// Multiply by `numerator / denominator`, rounded to nearest; `None` if 32 bits cannot hold it.
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

    /// Rebuild a conversion from published fields that fit [`to_bits`](Self::to_bits).
    pub const fn from_parts(multiplier: u64, shift: u32) -> Option<Self> {
        if multiplier > MAX_MULTIPLIER {
            return None;
        }
        if shift > MAX_SHIFT {
            return None;
        }
        Some(Self { multiplier, shift })
    }

    /// The multiplier this conversion applies, for writing into a page a lockless reader sees.
    pub const fn multiplier(self) -> u64 {
        self.multiplier
    }

    /// The shift this conversion applies.
    pub const fn shift(self) -> u32 {
        self.shift
    }

    /// Read a published clock; the base is split since `base_ns << shift` overflows within seconds.
    ///
    /// # Examples
    ///
    /// ```
    /// use timebase::Timebase;
    ///
    /// let clock = Timebase::nanoseconds(1_000_000_000).expect("a 1 GHz counter");
    ///
    /// // A thousand ticks of a 1 GHz counter is a thousand nanoseconds.
    /// assert_eq!(clock.resume(5_000, 0, 1_000), 6_000);
    /// ```
    pub const fn resume(self, base_ns: u64, base_fraction: u64, ticks: u64) -> u64 {
        let advanced = base_fraction as u128 + (ticks as u128 * self.multiplier as u128);
        let whole = advanced >> self.shift;
        if whole > u64::MAX as u128 {
            return u64::MAX;
        }
        base_ns.saturating_add(whole as u64)
    }

    /// Advance by `ticks` to what [`resume`](Self::resume) reads, carrying the fraction.
    pub const fn advance(self, base_ns: u64, base_fraction: u64, ticks: u64) -> (u64, u64) {
        let advanced = base_fraction as u128 + (ticks as u128 * self.multiplier as u128);
        let whole = advanced >> self.shift;
        if whole > u64::MAX as u128 {
            return (u64::MAX, 0);
        }
        let carried = advanced - (whole << self.shift);
        (base_ns.saturating_add(whole as u64), carried as u64)
    }

    /// Ticks of a counter running at `hz`, into nanoseconds.
    pub const fn nanoseconds(hz: u64) -> Option<Self> {
        Self::new(NANOSECONDS_PER_SECOND, hz)
    }

    /// Nanoseconds, into ticks of a counter running at `hz`.
    pub const fn ticks(hz: u64) -> Option<Self> {
        Self::new(hz, NANOSECONDS_PER_SECOND)
    }

    /// Whether this conversion came from a measurement.
    pub const fn is_calibrated(self) -> bool {
        self.multiplier != 0
    }

    /// Apply the conversion, saturating because a clock must never read as wrapped.
    pub const fn convert(self, value: u64) -> u64 {
        let scaled = (value as u128 * self.multiplier as u128) >> self.shift;
        if scaled > u64::MAX as u128 {
            u64::MAX
        } else {
            scaled as u64
        }
    }

    /// The conversion as one word, for publishing where a lock cannot go.
    pub const fn to_bits(self) -> u64 {
        (self.shift as u64) << SHIFT_BITS | self.multiplier
    }

    /// Read back what [`to_bits`](Self::to_bits) wrote, total on every input.
    pub const fn from_bits(bits: u64) -> Self {
        Self {
            multiplier: bits & MAX_MULTIPLIER,
            shift: ((bits >> SHIFT_BITS) as u32) & MAX_SHIFT,
        }
    }
}

/// The sequence counter of a lockless clock; the caller supplies both memory-ordering barriers.
/// `kernel::timepage` supplies them, and a read must not span a full `u32` counter cycle.
///
/// # Examples
///
/// ```
/// use timebase::Seq;
///
/// let quiet = 4;
/// assert!(Seq::is_stable(quiet));
///
/// let writing = Seq::begin(quiet);
/// assert!(!Seq::is_stable(writing));
/// assert!(!Seq::agrees(quiet, writing));
///
/// let written = Seq::end(writing);
/// assert!(Seq::is_stable(written));
/// // A reader that started before the write does not accept what it read.
/// assert!(!Seq::agrees(quiet, written));
/// ```
#[derive(Debug)]
pub struct Seq;

impl Seq {
    /// What a page holds before anything has ever been published.
    pub const QUIET: u32 = 0;

    /// Whether no write is in progress.
    pub const fn is_stable(seq: u32) -> bool {
        seq.is_multiple_of(2)
    }

    /// The value a writer stores before it begins.
    pub const fn begin(seq: u32) -> u32 {
        seq.wrapping_add(1)
    }

    /// The value a writer stores when it is done.
    pub const fn end(seq: u32) -> u32 {
        seq.wrapping_add(1)
    }

    /// Whether a read may be kept: equal rules out a whole write inside it, even one already begun.
    pub const fn agrees(before: u32, after: u32) -> bool {
        before == after && Self::is_stable(before)
    }
}

/// A hardware poll bounded by monotonic time, or by attempts before the clock is calibrated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PollLimit {
    /// Stop at this monotonic nanosecond.
    Deadline(u64),
    /// Permit exactly this many more hardware reads.
    Attempts(u32),
}

impl PollLimit {
    /// A deadline `timeout_ms` after `now_ns`, or `fallback_attempts` while there is no clock.
    pub const fn after_ms(now_ns: Option<u64>, timeout_ms: u64, fallback_attempts: u32) -> Self {
        match now_ns {
            Some(now) => Self::Deadline(now.saturating_add(timeout_ms.saturating_mul(1_000_000))),
            None => Self::Attempts(fallback_attempts),
        }
    }

    /// Whether one more read is permitted at `now_ns`; the mode never changes mid-poll.
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
