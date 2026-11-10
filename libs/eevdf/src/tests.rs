use super::*;

const SLICE: u64 = 30;

/// Past the deadline `deadline.wrapping_sub(vruntime)` wraps, which would arm the timer far away.
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

    entity.charge(SLICE);
    assert!(entity.slice_is_spent());
    assert_eq!(entity.remaining_slice(), 0, "spent exactly is spent");

    entity.charge(SLICE / 2);
    assert!(entity.slice_is_spent(), "still spent after overrunning");
    assert_eq!(
        entity.remaining_slice(),
        0,
        "an overrun slice reported time remaining, which arms the timer far away"
    );

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
    assert_eq!(entity.weight, MAX_WEIGHT);
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
    assert_eq!(entity.weight, 1);
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

#[test]
fn a_later_candidate_cannot_replace_an_earlier_deadline() {
    let mut earliest = at(0, BASE_WEIGHT);
    earliest.renew_deadline(10);
    let mut latest = at(0, BASE_WEIGHT);
    latest.renew_deadline(30);
    let mut middle = at(0, BASE_WEIGHT);
    middle.renew_deadline(20);

    let queue = [earliest, latest, middle];
    assert_eq!(pick(queue.iter().enumerate(), 0), Some(0));
}

/// The kernel's fallback path exists to catch this property failing.
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
    assert_eq!(fresh.deadline, AVERAGE + slice_v);
}

/// At every weight, because the conversion out and back is where a weight could be lost.
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

/// Milliseconds of nanoseconds would overflow either conversion without its widening.
#[test]
fn a_slice_of_nanoseconds_survives_the_round_trip() {
    const NANOSECONDS: u64 = 3_000_000;

    for weight in [1, BASE_WEIGHT, 64 * BASE_WEIGHT, MAX_WEIGHT] {
        let mut entity = Entity::new(weight);
        entity.place(0, 0, NANOSECONDS);
        assert_eq!(entity.remaining_slice(), NANOSECONDS, "weight {weight}");
    }
}

/// A caller arming a timer from this must be told to fire now rather than never.
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

    assert_eq!(entity.deadline, MAX_ORDERED_DELTA);
    assert!(!entity.slice_is_spent());
    assert_eq!(entity.remaining_slice(), MAX_ORDERED_DELTA);
}

/// Drive the policy as the kernel does: one unit per step, charged to whoever was picked.
fn simulate(weights: &[u32], steps: u32) -> Vec<u32> {
    let mut queue: Vec<Entity> = weights.iter().map(|weight| Entity::new(*weight)).collect();
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

/// Without the clamp, a long sleeper could bank any credit and then run unopposed.
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

/// At weight one, 2^44 real units is exactly 2^64 virtual, which a `u64` narrows to zero.
#[test]
fn a_huge_charge_still_moves_the_clock() {
    let mut entity = Entity::new(1);
    let before = entity.vruntime();
    entity.charge(1 << 44);
    assert_ne!(entity.vruntime(), before, "the clock did not move");
    assert!(entity.slice_is_spent(), "a charge that large left slice");
}

/// Every clamped charge stays inside the half-ring the wrapping comparison needs.
#[test]
fn charging_never_moves_the_clock_backwards_in_order() {
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

/// Other tests let `BASE_WEIGHT` and `RESOLUTION` cancel, so literals anchor both scales.
#[test]
fn the_scale_constants_are_what_the_doc_says() {
    assert_eq!(BASE_WEIGHT, 1024);
    assert_eq!(RESOLUTION, 1024);
    assert_eq!(MAX_WEIGHT, 1_048_576);

    let mut at_base = Entity::new(1024);
    at_base.charge(100);
    assert_eq!(at_base.vruntime(), 102_400);

    for slice in [1u64, 3, 3_000_000] {
        let mut entity = Entity::new(MAX_WEIGHT);
        entity.place(0, 0, slice);
        assert_eq!(entity.remaining_slice(), slice, "at the heaviest weight");
    }
}
