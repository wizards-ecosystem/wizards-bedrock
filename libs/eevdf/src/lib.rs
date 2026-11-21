#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

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

// Linux's `avg_vruntime` as running sums: `origin` is the floored mean, `0 <= above < weight`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Load {
    origin: u64,
    above: i128,
    weight: u64,
}

impl Load {
    pub const EMPTY: Self = Self {
        origin: 0,
        above: 0,
        weight: 0,
    };

    fn offset(&self, entity: &Entity) -> i128 {
        i128::from(entity.vruntime.wrapping_sub(self.origin) as i64) * i128::from(entity.weight)
    }

    pub fn add(&mut self, entity: &Entity) {
        if self.weight == 0 {
            self.origin = entity.vruntime;
        }
        self.above += self.offset(entity);
        self.weight += u64::from(entity.weight);
        self.settle();
    }

    pub fn remove(&mut self, entity: &Entity) {
        let Some(weight) = self.weight.checked_sub(u64::from(entity.weight)) else {
            panic!("an entity left a load it was not in");
        };
        self.above -= self.offset(entity);
        self.weight = weight;
        if weight == 0 {
            assert!(self.above == 0, "an entity left a load with other values");
        } else {
            self.settle();
        }
    }

    fn settle(&mut self) {
        let shift = self.above.div_euclid(i128::from(self.weight));
        self.origin = self.origin.wrapping_add(shift as u64);
        self.above -= shift * i128::from(self.weight);
    }

    // The mean with `with` counted as one more member, as the running entity is.
    pub fn average(&self, with: Option<&Entity>) -> Option<u64> {
        let mut load = *self;
        if let Some(entity) = with {
            load.add(entity);
        }
        (load.weight > 0).then_some(load.origin)
    }
}

// The earliest deadline among eligible entities offered one at a time; the first of equals wins.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pick {
    average: u64,
    deadline: Option<u64>,
}

impl Pick {
    pub const fn new(average: u64) -> Self {
        Self {
            average,
            deadline: None,
        }
    }

    // True when `entity` is now the best offered.
    pub fn offer(&mut self, entity: &Entity) -> bool {
        if !entity.is_eligible(self.average) {
            return false;
        }
        match self.deadline {
            Some(deadline) if !before(entity.deadline, deadline) => false,
            _ => {
                self.deadline = Some(entity.deadline);
                true
            }
        }
    }
}
