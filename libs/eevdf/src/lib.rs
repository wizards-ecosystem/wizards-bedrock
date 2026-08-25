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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Where its current slice ends, in virtual time.
    pub const fn deadline(&self) -> u64 {
        self.deadline
    }

    /// Its weight relative to [`BASE_WEIGHT`].
    pub const fn weight(&self) -> u32 {
        self.weight
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
    let first = entities.next()?;
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
mod tests {
    use super::*;

    const SLICE: u64 = 30;

    /// A thread that has overrun its slice has none of it left.
    ///
    /// Every test that looked like it covered this landed the thread
    /// *exactly* on its deadline, where `wrapping_sub` gives zero anyway. Run
    /// one past, and deleting the `slice_is_spent` guard survives the suite —
    /// `deadline.wrapping_sub(vruntime)` then wraps to near `u64::MAX`, and
    /// `kernel/src/thread.rs` arms the tickless timer with it. The thread is
    /// never preempted again (D-136).
    #[test]
    fn a_thread_past_its_deadline_has_no_slice_left() {
        const SLICE: u64 = 3_000_000;

        let mut entity = Entity::new(BASE_WEIGHT);
        entity.place(0, 0, SLICE);
        assert!(!entity.slice_is_spent(), "a fresh slice is not spent");
        assert_eq!(
            entity.remaining_slice(),
            SLICE,
            "a fresh slice has all of itself left"
        );

        // Exactly on the deadline: the boundary the old tests reached.
        entity.charge(SLICE);
        assert!(entity.slice_is_spent());
        assert_eq!(entity.remaining_slice(), 0, "spent exactly is spent");

        // And **past** it, which is where the wrap lives. A tick of overrun is
        // ordinary: a thread is charged when it is descheduled, not when its
        // deadline passes.
        entity.charge(SLICE / 2);
        assert!(entity.slice_is_spent(), "still spent after overrunning");
        assert_eq!(
            entity.remaining_slice(),
            0,
            "an overrun slice reported time remaining, which arms the timer far away"
        );

        // Well past, at a spread that would wrap to most of the ring.
        entity.charge(SLICE * 1000);
        assert_eq!(entity.remaining_slice(), 0, "a long overrun is still zero");
    }

    #[test]
    fn ordering_survives_a_wrap() {
        assert!(before(0, 1));
        assert!(!before(1, 0));
        assert!(!before(5, 5));
        assert!(
            before(u64::MAX, 1),
            "just before the wrap is before just after it"
        );
        assert!(!before(1, u64::MAX));
    }

    #[test]
    fn times_half_a_ring_apart_are_not_cyclically_ordered() {
        let half = 1u64 << 63;
        assert!(!before(0, half));
        assert!(!before(half, 0));
    }

    #[test]
    fn a_heavier_thread_pays_less_virtual_time() {
        let mut base = Entity::new(BASE_WEIGHT);
        let mut double = Entity::new(2 * BASE_WEIGHT);
        let mut half = Entity::new(BASE_WEIGHT / 2);
        base.charge(100);
        double.charge(100);
        half.charge(100);
        assert_eq!(base.vruntime(), 100 * RESOLUTION);
        assert_eq!(double.vruntime(), 50 * RESOLUTION);
        assert_eq!(half.vruntime(), 200 * RESOLUTION);
    }

    #[test]
    fn a_gigantic_weight_still_pays() {
        let mut entity = Entity::new(u32::MAX);
        entity.charge(1);
        assert_eq!(entity.weight(), MAX_WEIGHT);
        assert_eq!(entity.vruntime(), 1, "the clock must never stand still");
    }

    #[test]
    fn a_round_trip_loses_at_most_one_unit_at_any_weight() {
        for weight in [
            1,
            2,
            3,
            819,
            820,
            1277,
            BASE_WEIGHT,
            8 * BASE_WEIGHT,
            MAX_WEIGHT,
        ] {
            let mut entity = Entity::new(weight);
            for slice in [1, 5, 1_000, 3_000_000] {
                entity.renew_deadline(slice);
                let remaining = entity.remaining_slice();
                assert!(
                    remaining <= slice && remaining + 1 >= slice,
                    "weight {weight}, slice {slice}: remaining {remaining}"
                );
            }
        }
    }

    #[test]
    fn a_zero_weight_is_taken_as_one() {
        let mut entity = Entity::new(0);
        entity.charge(1);
        assert_eq!(entity.weight(), 1);
        assert_eq!(entity.vruntime(), RESOLUTION * u64::from(BASE_WEIGHT));
    }

    fn at(vruntime: u64, weight: u32) -> Entity {
        let mut entity = Entity::new(weight);
        entity.charge(0);
        entity.vruntime = vruntime;
        entity
    }

    #[test]
    fn the_average_is_weighted() {
        let equal = [at(0, BASE_WEIGHT), at(100, BASE_WEIGHT)];
        assert_eq!(average_vruntime(&equal), Some(50));

        let skewed = [at(0, BASE_WEIGHT), at(100, 3 * BASE_WEIGHT)];
        assert_eq!(average_vruntime(&skewed), Some(75));

        assert_eq!(average_vruntime(&[]), None);
    }

    #[test]
    fn the_average_straddles_a_wrap() {
        let queue = [at(u64::MAX - 9, BASE_WEIGHT), at(10, BASE_WEIGHT)];
        assert_eq!(average_vruntime(&queue), Some(0));
    }

    #[test]
    fn eligibility_is_being_at_or_behind_the_average() {
        let behind = at(40, BASE_WEIGHT);
        let level = at(50, BASE_WEIGHT);
        let ahead = at(60, BASE_WEIGHT);
        assert!(behind.is_eligible(50));
        assert!(level.is_eligible(50));
        assert!(!ahead.is_eligible(50));
        assert_eq!(ahead.lag(50), -10);
    }

    #[test]
    fn the_pick_is_the_earliest_deadline_among_the_eligible() {
        let mut early_but_ahead = at(100, BASE_WEIGHT);
        early_but_ahead.renew_deadline(1);
        let mut late = at(0, BASE_WEIGHT);
        late.renew_deadline(50);
        let mut soon = at(10, BASE_WEIGHT);
        soon.renew_deadline(20);

        let queue = [early_but_ahead, late, soon];
        let average = average_vruntime(&queue).unwrap();
        assert_eq!(pick(queue.iter().enumerate(), average), Some(2));
        assert_eq!(pick(core::iter::empty::<(usize, &Entity)>(), 0), None);
    }

    /// The property the kernel's fallback path exists to catch if it ever
    /// fails: whatever the queue, someone is eligible.
    #[test]
    fn a_non_empty_queue_always_has_an_eligible_entity() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..1000 {
            let mut queue = [Entity::new(BASE_WEIGHT); 5];
            for entity in &mut queue {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                *entity = at(seed, 1 + (seed >> 60) as u32 * 512);
                entity.renew_deadline(SLICE);
            }
            let average = average_vruntime(&queue).unwrap();
            assert!(
                pick(queue.iter().enumerate(), average).is_some(),
                "no eligible entity in {queue:?} at average {average}"
            );
        }
    }

    #[test]
    fn placement_keeps_a_debt_and_caps_a_credit() {
        const AVERAGE: u64 = 1_000_000;
        let slice_v = SLICE * RESOLUTION;

        let mut owed = Entity::new(BASE_WEIGHT);
        owed.place(AVERAGE, -5, SLICE);
        assert_eq!(
            owed.vruntime(),
            AVERAGE + 5,
            "a brief sleep does not clear a debt"
        );
        assert!(!owed.is_eligible(AVERAGE));

        let mut banked = Entity::new(BASE_WEIGHT);
        banked.place(AVERAGE, 10 * slice_v as i64, SLICE);
        assert_eq!(
            banked.vruntime(),
            AVERAGE - slice_v,
            "credit is capped at one slice"
        );

        let mut fresh = Entity::new(BASE_WEIGHT);
        fresh.place(AVERAGE, 0, SLICE);
        assert_eq!(fresh.vruntime(), AVERAGE);
        assert_eq!(fresh.deadline(), AVERAGE + slice_v);
    }

    /// What a tickless kernel asks before it arms anything: the answer must
    /// be the slice it was given, count down as the thread runs, and reach
    /// zero exactly where `slice_is_spent` turns true — at every weight,
    /// because the conversion out and back is where a weight could be lost.
    #[test]
    fn the_remaining_slice_counts_down_to_the_deadline() {
        for weight in [1, BASE_WEIGHT / 4, BASE_WEIGHT, 8 * BASE_WEIGHT] {
            let mut entity = Entity::new(weight);
            entity.place(0, 0, SLICE);
            assert_eq!(entity.remaining_slice(), SLICE, "weight {weight}");

            entity.charge(SLICE / 3);
            assert_eq!(
                entity.remaining_slice(),
                SLICE - SLICE / 3,
                "weight {weight}"
            );

            entity.charge(SLICE - SLICE / 3 - 1);
            assert_eq!(entity.remaining_slice(), 1, "weight {weight}");

            entity.charge(1);
            assert!(entity.slice_is_spent(), "weight {weight}");
            assert_eq!(entity.remaining_slice(), 0, "weight {weight}");
        }
    }

    /// A slice long enough to matter to a tickless timer — milliseconds of
    /// nanoseconds — survives the trip through virtual time at every weight,
    /// which is the arithmetic that would overflow if the widening were
    /// dropped from either conversion.
    #[test]
    fn a_slice_of_nanoseconds_survives_the_round_trip() {
        const NANOSECONDS: u64 = 3_000_000;

        // Up to `MAX_WEIGHT`, which is where the doc's one-unit claim is
        // made — the sweep used to stop at 64 times the base, a factor of
        // sixteen short of the bound it was testing (D-124).
        for weight in [1, BASE_WEIGHT, 64 * BASE_WEIGHT, MAX_WEIGHT] {
            let mut entity = Entity::new(weight);
            entity.place(0, 0, NANOSECONDS);
            assert_eq!(entity.remaining_slice(), NANOSECONDS, "weight {weight}");
        }
    }

    /// An entity that has never been placed has no slice, and a caller that
    /// arms a timer from this must be told to fire now rather than never.
    #[test]
    fn an_unplaced_entity_has_nothing_remaining() {
        assert_eq!(Entity::new(BASE_WEIGHT).remaining_slice(), 0);
    }

    #[test]
    fn a_slice_is_spent_when_the_deadline_is_reached() {
        let mut entity = Entity::new(BASE_WEIGHT);
        entity.place(0, 0, SLICE);
        assert!(!entity.slice_is_spent());
        entity.charge(SLICE - 1);
        assert!(!entity.slice_is_spent());
        entity.charge(1);
        assert!(entity.slice_is_spent());
        entity.renew_deadline(SLICE);
        assert!(!entity.slice_is_spent());
    }

    #[test]
    fn a_slice_cannot_cross_the_wrapping_orders_half_range() {
        let mut entity = Entity::new(MAX_WEIGHT);
        entity.renew_deadline(u64::MAX);

        assert_eq!(entity.deadline(), MAX_ORDERED_DELTA);
        assert!(!entity.slice_is_spent());
        assert_eq!(entity.remaining_slice(), MAX_ORDERED_DELTA);
    }

    /// Drive the policy the way the kernel will: one unit of time per step,
    /// charged to whoever was picked, deadline renewed when spent.
    fn simulate(weights: &[u32], steps: u32) -> Vec<u32> {
        let mut queue: Vec<Entity> = weights.iter().map(|w| Entity::new(*w)).collect();
        for entity in &mut queue {
            entity.place(0, 0, SLICE);
        }
        let mut ran = vec![0u32; weights.len()];
        for _ in 0..steps {
            let average = average_vruntime(&queue).unwrap();
            let index = pick(queue.iter().enumerate(), average).unwrap();
            queue[index].charge(1);
            if queue[index].slice_is_spent() {
                queue[index].renew_deadline(SLICE);
            }
            ran[index] += 1;
        }
        ran
    }

    #[test]
    fn runtime_divides_in_proportion_to_weight() {
        let ran = simulate(&[BASE_WEIGHT, 2 * BASE_WEIGHT], 3000);
        assert_eq!(ran[0] + ran[1], 3000);
        let ratio = f64::from(ran[1]) / f64::from(ran[0]);
        assert!((1.9..=2.1).contains(&ratio), "ratio {ratio} from {ran:?}");
    }

    #[test]
    fn equal_weights_share_equally_and_nobody_starves() {
        let ran = simulate(&[BASE_WEIGHT; 3], 3000);
        for share in &ran {
            assert!((990..=1010).contains(share), "{ran:?}");
        }
    }

    /// Without the clamp, a thread that slept long enough could bank any
    /// amount of credit and then run unopposed. With it, the burst is one
    /// slice.
    #[test]
    fn a_returning_sleeper_cannot_monopolise_the_cpu() {
        let mut queue = [Entity::new(BASE_WEIGHT), Entity::new(BASE_WEIGHT)];
        queue[0].place(0, 0, SLICE);
        queue[1].place(0, 1000 * (SLICE * RESOLUTION) as i64, SLICE);
        let mut ran = [0u32; 2];
        for _ in 0..(4 * SLICE as u32) {
            let average = average_vruntime(&queue).unwrap();
            let index = pick(queue.iter().enumerate(), average).unwrap();
            queue[index].charge(1);
            if queue[index].slice_is_spent() {
                queue[index].renew_deadline(SLICE);
            }
            ran[index] += 1;
        }
        assert!(ran[0] > 0, "the sleeper ran unopposed: {ran:?}");
    }

    #[test]
    fn a_huge_charge_still_moves_the_clock() {
        // At weight one the virtual value of 2^44 real units is exactly 2^64,
        // which narrowing to `u64` turns into zero: the thread ran and its
        // clock did not move, so it stays eligible for ever and starves
        // everyone else (D-105).
        let mut entity = Entity::new(1);
        let before = entity.vruntime();
        entity.charge(1 << 44);
        assert_ne!(entity.vruntime(), before, "the clock did not move");
        assert!(entity.slice_is_spent(), "a charge that large left slice");
    }

    #[test]
    fn charging_never_moves_the_clock_backwards_in_order() {
        // Every clamped charge stays inside the half-ring the wrapping
        // comparison needs, so a sequence of huge charges keeps advancing.
        let mut entity = Entity::new(1);
        let mut previous = entity.vruntime();
        for _ in 0..8 {
            entity.charge(u64::MAX);
            assert!(
                before(previous, entity.vruntime()),
                "{previous} then {}",
                entity.vruntime()
            );
            previous = entity.vruntime();
        }
    }

    /// The two scale constants, transcribed, and the round trip exercised at
    /// the weight the doc bounds it at.
    ///
    /// Every other test spells its weights as multiples of `BASE_WEIGHT` and
    /// its expectations in terms of `RESOLUTION`, so both cancel identically
    /// — halving either survived the suite. And the round-trip sweep stopped
    /// at 64 times the base, a factor of sixteen short of `MAX_WEIGHT`, which
    /// is where the doc says the one-unit bound holds (D-124).
    #[test]
    fn the_scale_constants_are_what_the_doc_says() {
        assert_eq!(BASE_WEIGHT, 1024);
        assert_eq!(RESOLUTION, 1024);
        assert_eq!(MAX_WEIGHT, 1_048_576);

        // A charge at the base weight is real time times the resolution, with
        // the literal on the right rather than the constant.
        let mut at_base = Entity::new(1024);
        at_base.charge(100);
        assert_eq!(at_base.vruntime(), 102_400);

        // And the round trip holds at the heaviest weight the doc bounds it
        // at, which the sweep beside it stops a factor of sixteen short of.
        for slice in [1u64, 3, 3_000_000] {
            let mut entity = Entity::new(MAX_WEIGHT);
            entity.place(0, 0, slice);
            assert_eq!(entity.remaining_slice(), slice, "at the heaviest weight");
        }
    }
}
