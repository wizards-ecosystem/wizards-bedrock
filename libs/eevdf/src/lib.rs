#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::borrow::Borrow;

// Linux's `NICE_0_LOAD`: at this weight one unit of real time is one unit of virtual time.
pub const BASE_WEIGHT: u32 = 1024;

const RESOLUTION: u64 = 1024;

// Past this weight a one-unit charge would stop the clock.
const MAX_WEIGHT: u32 = RESOLUTION as u32 * BASE_WEIGHT;

const MAX_ORDERED_DELTA: u64 = i64::MAX as u64;

// Times exactly half the ring apart are unordered.
const fn before(first: u64, second: u64) -> bool {
    let difference = first.wrapping_sub(second);
    difference != 1u64 << 63 && (difference as i64) < 0
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entity {
    vruntime: u64,
    deadline: u64,
    weight: u32,
}

impl Entity {
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

    pub const fn vruntime(&self) -> u64 {
        self.vruntime
    }

    const fn virtual_units_wide(&self, real: u64) -> u128 {
        (real as u128 * RESOLUTION as u128 * BASE_WEIGHT as u128) / self.weight as u128
    }

    const fn ordered_virtual_units(&self, real: u64) -> u64 {
        let units = self.virtual_units_wide(real);
        if units > MAX_ORDERED_DELTA as u128 {
            MAX_ORDERED_DELTA
        } else {
            units as u64
        }
    }

    const fn real_units(&self, virtual_time: u64) -> u64 {
        ((virtual_time as u128 * self.weight as u128) / (RESOLUTION as u128 * BASE_WEIGHT as u128))
            as u64
    }

    pub const fn remaining_slice(&self) -> u64 {
        if self.slice_is_spent() {
            return 0;
        }
        self.real_units(self.deadline.wrapping_sub(self.vruntime))
    }

    pub fn charge(&mut self, ran: u64) {
        self.vruntime = self.vruntime.wrapping_add(self.ordered_virtual_units(ran));
    }

    pub const fn lag(&self, average: u64) -> i64 {
        average.wrapping_sub(self.vruntime) as i64
    }

    const fn is_eligible(&self, average: u64) -> bool {
        self.lag(average) >= 0
    }

    // Lag is clamped to one slice so sleep neither banks credit nor sheds debt.
    pub fn place(&mut self, average: u64, lag: i64, slice: u64) {
        let bound = self.ordered_virtual_units(slice) as i64;
        let lag = lag.clamp(-bound, bound);
        self.vruntime = average.wrapping_sub(lag as u64);
        self.renew_deadline(slice);
    }

    pub const fn slice_is_spent(&self) -> bool {
        !before(self.vruntime, self.deadline)
    }

    pub fn renew_deadline(&mut self, slice: u64) {
        self.deadline = self
            .vruntime
            .wrapping_add(self.ordered_virtual_units(slice));
    }
}

// Offsets are taken from the first entity and the result floored so someone is eligible.
pub fn average_vruntime<E: Borrow<Entity>>(entities: impl IntoIterator<Item = E>) -> Option<u64> {
    let mut entities = entities.into_iter();
    let first = entities.next()?;
    let origin = first.borrow().vruntime;

    let (mut weighted, mut weight) = (0i128, 0i128);
    for entity in core::iter::once(first).chain(entities) {
        let entity = entity.borrow();
        let offset = entity.vruntime.wrapping_sub(origin) as i64;
        weighted += i128::from(offset) * i128::from(entity.weight);
        weight += i128::from(entity.weight);
    }
    let offset = weighted.div_euclid(weight);
    Some(origin.wrapping_add(offset as u64))
}

pub fn pick<K: Copy, E: Borrow<Entity>>(
    entities: impl IntoIterator<Item = (K, E)>,
    average: u64,
) -> Option<K> {
    let mut best: Option<(K, u64)> = None;
    for (key, entity) in entities {
        let entity = entity.borrow();
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
