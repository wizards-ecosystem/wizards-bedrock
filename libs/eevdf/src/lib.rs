//! Earliest eligible virtual deadline first, as arithmetic over wrapping virtual time.
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

/// The weight at which one unit of real time is one unit of virtual time (Linux's `NICE_0_LOAD`).
pub const BASE_WEIGHT: u32 = 1024;

/// Virtual-time units per unit of real time at [`BASE_WEIGHT`], so a heavy thread's clock moves.
pub const RESOLUTION: u64 = 1024;

/// The largest weight, past which a one-unit charge would stop the clock; [`Entity::new`] clamps.
pub const MAX_WEIGHT: u32 = RESOLUTION as u32 * BASE_WEIGHT;

/// Largest virtual-time distance the wrapping order can distinguish.
const MAX_ORDERED_DELTA: u64 = i64::MAX as u64;

/// Whether virtual time `a` precedes `b` across a wrap; times half the ring apart are unordered.
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
    /// A thread of `weight`, clamped to one through [`MAX_WEIGHT`], at virtual time zero.
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

    /// Widened so that a large charge at a small weight cannot overflow before the division.
    const fn virtual_units_wide(&self, real: u64) -> u128 {
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

    /// Real time left before the deadline, zero once spent and at most one unit early.
    pub const fn remaining_slice(&self) -> u64 {
        if self.slice_is_spent() {
            return 0;
        }
        self.real_units(self.deadline.wrapping_sub(self.vruntime))
    }

    /// Account `ran` units of real time, clamped so that a huge charge still moves the clock.
    pub fn charge(&mut self, ran: u64) {
        self.vruntime = self.vruntime.wrapping_add(self.ordered_virtual_units(ran));
    }

    /// How far behind (positive) or ahead of (negative) `average` the thread is.
    pub const fn lag(&self, average: u64) -> i64 {
        average.wrapping_sub(self.vruntime) as i64
    }

    /// Whether the thread is owed time: its lag is not negative.
    pub const fn is_eligible(&self, average: u64) -> bool {
        self.lag(average) >= 0
    }

    /// Place at `average` less `lag`, clamped to a `slice` so sleep banks no credit nor sheds debt.
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

    /// Start a new slice from where the thread now stands, capped below half the ring.
    pub fn renew_deadline(&mut self, slice: u64) {
        self.deadline = self
            .vruntime
            .wrapping_add(self.ordered_virtual_units(slice));
    }
}

/// The weighted average vruntime, offset from the first entity and floored so someone is eligible.
pub fn average_vruntime<'entity>(
    entities: impl IntoIterator<Item = &'entity Entity>,
) -> Option<u64> {
    let mut entities = entities.into_iter();
    let first = core::iter::Iterator::next(&mut entities)?;
    let origin = first.vruntime;

    let (mut weighted, mut weight) = (0i128, 0i128);
    for entity in core::iter::once(first).chain(entities) {
        let offset = entity.vruntime.wrapping_sub(origin) as i64;
        weighted += i128::from(offset) * i128::from(entity.weight);
        weight += i128::from(entity.weight);
    }
    let offset = weighted.div_euclid(weight);
    Some(origin.wrapping_add(offset as u64))
}

/// The key of the eligible entity with the earliest deadline, `None` only for an empty queue.
pub fn pick<'entity, K: Copy>(
    entities: impl IntoIterator<Item = (K, &'entity Entity)>,
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
