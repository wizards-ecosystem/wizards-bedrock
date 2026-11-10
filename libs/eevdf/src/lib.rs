//! Earliest eligible virtual deadline first, as arithmetic.
//!
//! The policy D-15 chose and D-53 keeps separable: every runnable thread has
//! a *virtual runtime* that advances as it runs, slower the heavier its
//! weight; the queue's weighted average of those is the point a fair share
//! would have reached; a thread behind that point is *eligible*, one ahead of
//! it is owed nothing yet. Among the eligible, the one whose *virtual
//! deadline* — where its current slice ends — comes first runs. That is the
//! whole of it, and because it is arithmetic it is all here, where the host
//! can run it.
//!
//! Virtual time wraps. Every comparison is the signed difference of two
//! `u64`s, the `vruntime` idiom Linux uses, so a clock that has been running
//! for longer than a `u64` holds keeps ordering correctly. Related times must
//! stay less than half the ring apart; exactly half is deliberately unordered,
//! and slice construction caps its distance before it can cross that boundary.
//!
//! # Examples
//!
//! ```
//! use eevdf::{Entity, average_vruntime, pick};
//!
//! let mut heavy = Entity::new(2048);
//! let mut light = Entity::new(1024);
//! heavy.charge(100);
//! light.charge(100);
//! // The same real time costs the heavy thread half the virtual time.
//! assert_eq!(heavy.vruntime() * 2, light.vruntime());
//!
//! let queue = [heavy, light];
//! let average = average_vruntime(queue.iter()).unwrap();
//! // The heavy thread is behind the average, so it is the eligible one.
//! assert_eq!(pick(queue.iter().enumerate(), average), Some(0));
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// The weight at which one unit of real time is one unit of virtual time.
///
/// Linux's `NICE_0_LOAD`; a thread of twice this weight pays half.
pub const BASE_WEIGHT: u32 = 1024;

/// Virtual-time units per unit of real time, at [`BASE_WEIGHT`].
///
/// Real time is charged in whatever unit the caller keeps, and a single one
/// of those at twice the base weight would truncate to nothing. Scaling by
/// this first is what keeps a heavy thread's clock moving; the fairness test
/// below is the one that found it standing still. It also buys the headroom
/// [`Entity::remaining_slice`] converts back through: the round trip loses at
/// most one unit of real time, at any weight up to [`MAX_WEIGHT`].
pub const RESOLUTION: u64 = 1024;

/// The largest weight a thread may carry — [`RESOLUTION`] × [`BASE_WEIGHT`].
///
/// Past it a one-unit charge truncates to no virtual time at all: the clock
/// stops, the thread is always eligible, and everyone else starves.
/// [`Entity::new`] clamps to it.
pub const MAX_WEIGHT: u32 = RESOLUTION as u32 * BASE_WEIGHT;

/// Largest virtual-time distance the wrapping order can distinguish.
const MAX_ORDERED_DELTA: u64 = i64::MAX as u64;

/// Whether virtual time `a` comes before `b`, across a wrap.
///
/// Times exactly half the `u64` ring apart are ambiguous, so neither comes
/// before the other. Scheduler state keeps every meaningful comparison inside
/// that open half-range.
pub const fn before(a: u64, b: u64) -> bool {
    let difference = a.wrapping_sub(b);
    difference != 1u64 << 63 && (difference as i64) < 0
}

/// One schedulable thread's share of virtual time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entity {
    vruntime: u64,
    deadline: u64,
    weight: u32,
}

impl Entity {
    /// A thread of `weight` at virtual time zero, with no slice yet.
    ///
    /// A zero weight is taken as one, and a weight above [`MAX_WEIGHT`] as
    /// [`MAX_WEIGHT`]: outside that range the arithmetic stops meaning
    /// anything.
    pub const fn new(weight: u32) -> Self {
        Self {
            vruntime: 0,
            deadline: 0,
            weight: if weight == 0 {
                1
            } else if weight > MAX_WEIGHT {
                MAX_WEIGHT
            } else {
                weight
            },
        }
    }

    /// How far this thread has run, in virtual time.
    pub const fn vruntime(&self) -> u64 {
        self.vruntime
    }

    /// Convert without discarding wraps, for distances that must stay ordered.
    const fn virtual_units_wide(&self, real: u64) -> u128 {
        // Widened so that a large charge at a small weight cannot overflow
        // before the division brings it back down.
        (real as u128 * RESOLUTION as u128 * BASE_WEIGHT as u128) / self.weight as u128
    }

    /// A virtual duration that cannot make the wrapping order ambiguous.
    const fn ordered_virtual_units(&self, real: u64) -> u64 {
        let units = self.virtual_units_wide(real);
        if units > MAX_ORDERED_DELTA as u128 {
            MAX_ORDERED_DELTA
        } else {
            units as u64
        }
    }

    /// Convert this thread's virtual time back into real units.
    const fn real_units(&self, virtual_time: u64) -> u64 {
        ((virtual_time as u128 * self.weight as u128) / (RESOLUTION as u128 * BASE_WEIGHT as u128))
            as u64
    }

    /// Real time this thread may still run before its deadline, in whatever
    /// unit its slice was given in.
    ///
    /// What a tickless kernel arms its timer for: the answer says when this
    /// thread stops being the earliest deadline, so there is no reason to
    /// interrupt it before then. Zero once the slice is spent, which is the
    /// honest answer to "how long until now". May under-report by one unit at
    /// weights that do not divide the scale — an early wake, never a late one.
    pub const fn remaining_slice(&self) -> u64 {
        if self.slice_is_spent() {
            return 0;
        }
        self.real_units(self.deadline.wrapping_sub(self.vruntime))
    }

    /// Account `ran` units of real time the thread has just spent running.
    ///
    /// Clamped, like `place` and `renew_deadline`, rather than truncated.
    /// `virtual_units` narrows the widened product with `as`, so a charge whose
    /// virtual value is a multiple of `2^64` advanced the clock by *nothing* —
    /// the outcome `MAX_WEIGHT`'s own doc says it exists to prevent, reached
    /// by the other door (D-105).
    pub fn charge(&mut self, ran: u64) {
        self.vruntime = self.vruntime.wrapping_add(self.ordered_virtual_units(ran));
    }

    /// How far behind (positive) or ahead of (negative) `average` the thread
    /// is.
    pub const fn lag(&self, average: u64) -> i64 {
        average.wrapping_sub(self.vruntime) as i64
    }

    /// Whether the thread is owed time: its lag is not negative.
    pub const fn is_eligible(&self, average: u64) -> bool {
        self.lag(average) >= 0
    }

    /// Put the thread on the queue at `average` adjusted by `lag`, and give
    /// it a fresh slice.
    ///
    /// The lag is clamped to one `slice` either way. A slice too large for the
    /// wrapping order is itself capped at `i64::MAX` virtual units. That clamp is the
    /// anti-gaming rule `008-research-notes-2.md` §4.4 warns is easy to omit:
    /// a thread that sleeps cannot bank unbounded credit, and one that sleeps
    /// briefly does not shed a debt — it comes back exactly as far behind as
    /// it left.
    pub fn place(&mut self, average: u64, lag: i64, slice: u64) {
        let bound = self.ordered_virtual_units(slice) as i64;
        let lag = lag.clamp(-bound, bound);
        self.vruntime = average.wrapping_sub(lag as u64);
        self.renew_deadline(slice);
    }

    /// Whether the current slice has been used up.
    pub const fn slice_is_spent(&self) -> bool {
        !before(self.vruntime, self.deadline)
    }

    /// Start a new slice from where the thread now stands.
    ///
    /// A duration reaching half the virtual-time ring is capped just below it,
    /// where the wrapping order remains unambiguous.
    pub fn renew_deadline(&mut self, slice: u64) {
        self.deadline = self
            .vruntime
            .wrapping_add(self.ordered_virtual_units(slice));
    }
}

/// The weighted average virtual runtime of a queue, or `None` if it is empty.
///
/// Computed relative to the first entity so that a queue straddling a wrap
/// still averages correctly.
pub fn average_vruntime<'a>(entities: impl IntoIterator<Item = &'a Entity>) -> Option<u64> {
    let mut entities = entities.into_iter();
    let first = core::iter::Iterator::next(&mut entities)?;
    let origin = first.vruntime;

    let (mut weighted, mut weight) = (0i128, 0i128);
    for entity in core::iter::once(first).chain(entities) {
        let offset = entity.vruntime.wrapping_sub(origin) as i64;
        weighted += i128::from(offset) * i128::from(entity.weight);
        weight += i128::from(entity.weight);
    }
    // A flooring division keeps the average at or above the smallest
    // vruntime, which is what keeps at least one entity eligible.
    let offset = weighted.div_euclid(weight);
    Some(origin.wrapping_add(offset as u64))
}

/// The eligible entity with the earliest deadline, by the key it was given.
///
/// `None` only for an empty queue: the entity with the smallest virtual
/// runtime is never ahead of the average, so a non-empty queue always has
/// someone eligible — a property the tests assert rather than assume.
pub fn pick<'a, K: Copy>(
    entities: impl IntoIterator<Item = (K, &'a Entity)>,
    average: u64,
) -> Option<K> {
    let mut best: Option<(K, u64)> = None;
    for (key, entity) in entities {
        if !entity.is_eligible(average) {
            continue;
        }
        match best {
            Some((_, deadline)) if !before(entity.deadline, deadline) => {}
            _ => best = Some((key, entity.deadline)),
        }
    }
    best.map(|(key, _)| key)
}

#[cfg(test)]
mod tests;
