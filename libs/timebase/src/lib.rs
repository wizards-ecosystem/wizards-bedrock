//! Turning a counter's ticks into nanoseconds, and nanoseconds back into
//! ticks.
//!
//! A counter measured at some arbitrary rate, converted to nanoseconds — and
//! nanoseconds back into ticks of a second counter at a third rate. Both are
//! one rational multiplication, precomputed once as the rounded fixed-point
//! ratio `value × multiplier >> shift`, because dividing would put a
//! sixty-four-bit division on the path every clock read takes.
//!
//! The multiplier is held to thirty-two bits, which keeps the product inside
//! a `u128` and lets a whole conversion pack into one `u64` — how a kernel
//! publishes one to a reader inside an interrupt handler, where no lock can
//! go. The shift is then the largest that still fits, so a conversion carries
//! at least thirty-one significant bits: under a nanosecond per second.
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

/// Nanoseconds in a second, which is the unit on both sides of every
/// conversion here.
pub const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

/// The largest multiplier a conversion may carry.
///
/// Thirty-two bits, so that `u64 × multiplier` cannot overflow a `u128` and
/// so that multiplier and shift together fit one `u64`.
const MAX_MULTIPLIER: u64 = u32::MAX as u64;

/// The largest shift, and the reason [`Timebase::from_bits`] masks: a shift
/// of sixty-four or more is undefined, and this type accepts bits it did not
/// write.
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
    /// The conversion of a clock that was never measured: everything it is
    /// asked converts to zero.
    ///
    /// Deliberately what a zeroed word decodes to, so a kernel publishing
    /// conversions into `static`s reads "no clock yet" without a second flag.
    pub const UNCALIBRATED: Self = Self {
        multiplier: 0,
        shift: 0,
    };

    /// The conversion that multiplies by `numerator / denominator`.
    ///
    /// `None` if either side is zero, or if the ratio is so large or so small
    /// that no shift represents it in thirty-two bits — a ratio above four
    /// billion, or below one part in about eighteen quintillion. Neither is a
    /// pair of real clocks.
    pub const fn new(numerator: u64, denominator: u64) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            return None;
        }
        let denominator = denominator as u128;
        let mut shift = MAX_SHIFT;
        loop {
            // The half-denominator rounds to nearest rather than towards
            // zero, halving the worst-case error of every conversion.
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

    /// Rebuild a conversion from published multiplier and shift fields.
    ///
    /// A zero multiplier remains a valid uncalibrated conversion. Other
    /// fields must fit the representation used by [`to_bits`](Self::to_bits).
    pub const fn from_parts(multiplier: u64, shift: u32) -> Option<Self> {
        if multiplier > MAX_MULTIPLIER {
            return None;
        }
        if shift > MAX_SHIFT {
            return None;
        }
        Some(Self { multiplier, shift })
    }

    /// The multiplier this conversion applies.
    ///
    /// For a clock that is *published* rather than called: a reader with no
    /// lock cannot ask a `Timebase` anything, so the two numbers have to be
    /// written into a page it can see.
    pub const fn multiplier(self) -> u64 {
        self.multiplier
    }

    /// The shift this conversion applies.
    pub const fn shift(self) -> u32 {
        self.shift
    }

    /// Read a published clock: whole nanoseconds, a carried fraction, and
    /// `ticks` of the counter since it was published.
    ///
    /// **One multiply-add and one shift, no division**, which is what D-152
    /// asks of the reader — a program with no lock and no syscall has to be
    /// able to do this in a few instructions.
    ///
    /// The base is split rather than stored wholly pre-shifted, and that is
    /// not a detail. `base_ns << shift` overflows a `u64` after about eight
    /// seconds of uptime at a 1 GHz counter's shift, so the obvious form is
    /// wrong on any machine that has been on for a minute. Linux splits it the
    /// same way — whole seconds beside a shifted remainder — for the same
    /// reason. What is carried here is a *fraction of a nanosecond*, always
    /// below `1 << shift`, so the sum below cannot overflow for any tick count
    /// a real update interval produces.
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

    /// Advance a published clock by `ticks`, keeping the fraction.
    ///
    /// What the kernel does at each update, and the reason the fraction exists
    /// at all: throwing it away would lose up to a nanosecond *per update*, so
    /// a clock updated a thousand times a second would fall a microsecond
    /// behind every second. Carrying it costs one mask.
    ///
    /// The reading this produces is the one [`resume`](Self::resume) would
    /// have given for the same arguments, which is what lets a program's own
    /// read and the kernel's agree rather than merely resemble each other.
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

    /// Apply the conversion, saturating rather than wrapping.
    ///
    /// Saturating because this is a clock: a reading past the end of the range
    /// should read as far ahead as the type allows, never as though it had
    /// wrapped to the beginning.
    pub const fn convert(self, value: u64) -> u64 {
        // Both factors are bounded — any `u64` and at most thirty-two bits —
        // so the product is at most ninety-six bits and cannot overflow.
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

    /// Read back what [`to_bits`](Self::to_bits) wrote.
    ///
    /// Total on every input: the shift is masked, so a word of nonsense
    /// converts to nonsense rather than to undefined behaviour.
    pub const fn from_bits(bits: u64) -> Self {
        Self {
            multiplier: bits & MAX_MULTIPLIER,
            shift: ((bits >> SHIFT_BITS) as u32) & MAX_SHIFT,
        }
    }
}

/// The sequence counter of a clock published to readers that cannot lock.
///
/// This type supplies only wrapping counter arithmetic. Odd marks an active
/// write; readers accept only matching even observations. The publisher and
/// reader must supply both memory-ordering barriers; `kernel::timepage` owns
/// them for the current caller (D-152). Correctness also requires that a full
/// `u32` counter cycle cannot occur inside one read.
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

    /// Whether a read that saw `before` and then `after` may keep what it read.
    ///
    /// Both conditions, and they are different: **equal** rules out a write
    /// that started and finished inside the read, and **even** rules out one
    /// that was already in progress when it started. Checking only equality
    /// accepts a read taken entirely inside an update, which is the mistake
    /// that makes a seqlock look like it works.
    pub const fn agrees(before: u32, after: u32) -> bool {
        before == after && Self::is_stable(before)
    }
}

/// A hardware poll bounded either by monotonic time or by boot-time attempts.
///
/// The mode is chosen once. After calibration, code generation and processor
/// speed cannot change the real duration; before calibration, a caller still
/// has a finite, explicit fallback rather than an accidental endless wait
/// (D-200).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PollLimit {
    /// Stop at this monotonic nanosecond.
    Deadline(u64),
    /// Permit exactly this many more hardware reads.
    Attempts(u32),
}

impl PollLimit {
    /// Choose a monotonic deadline when `now_ns` exists, or `fallback_attempts`
    /// while the boot clock is unavailable.
    pub const fn after_ms(now_ns: Option<u64>, timeout_ms: u64, fallback_attempts: u32) -> Self {
        match now_ns {
            Some(now) => Self::Deadline(now.saturating_add(timeout_ms.saturating_mul(1_000_000))),
            None => Self::Attempts(fallback_attempts),
        }
    }

    /// Whether one more hardware read is permitted at `now_ns`.
    ///
    /// A monotonic limit does not spend attempts: any number of fast polls
    /// sees the same real deadline. An attempt limit ignores a clock which
    /// appears later, so boot behaviour cannot change mode halfway through a
    /// transaction.
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
