use crate::Quota;

#[test]
fn a_reserved_slot_and_a_taken_slot_cost_the_same() {
    let limit = 4;

    let reserved = Quota::reserve(Quota::EMPTY, limit).expect("room");
    let committed = Quota::commit(reserved).expect("a promise to keep");

    let taken = Quota::take(Quota::EMPTY, limit).expect("room");

    assert_eq!(committed, taken);
    assert_eq!(Quota::live(committed), 1);
    assert_eq!(Quota::reserved(committed), 0);
}

#[test]
fn a_promise_holds_the_last_slot_against_an_insert() {
    let limit = 1;

    let promised = Quota::reserve(Quota::EMPTY, limit).expect("the one slot");
    assert_eq!(Quota::live(promised), 0, "no handle exists yet");

    assert!(
        Quota::take(promised, limit).is_none(),
        "an insert took the slot an admitted operation was promised"
    );
    assert!(
        Quota::reserve(promised, limit).is_none(),
        "a second operation was promised the same slot"
    );

    let released = Quota::release(promised).expect("a promise to give back");
    assert_eq!(released, Quota::EMPTY);
    assert!(Quota::take(released, limit).is_some());
}

#[test]
fn releasing_a_promise_leaves_the_held_handles_alone() {
    let limit = 8;
    let held = Quota::take(Quota::take(Quota::EMPTY, limit).unwrap(), limit).unwrap();
    let promised = Quota::reserve(held, limit).expect("room");

    let released = Quota::release(promised).expect("a promise to give back");
    assert_eq!(Quota::live(released), 2);
    assert_eq!(Quota::reserved(released), 0);

    let committed = Quota::commit(promised).expect("a promise to keep");
    assert_eq!(Quota::live(committed), 3);
    assert_eq!(Quota::reserved(committed), 0);
}

#[test]
fn spending_a_promise_that_was_not_made_is_refused() {
    assert!(Quota::commit(Quota::EMPTY).is_none());
    assert!(Quota::release(Quota::EMPTY).is_none());

    let held = Quota::take(Quota::EMPTY, 4).expect("room");
    assert!(Quota::commit(held).is_none(), "nothing was promised");
    assert!(Quota::release(held).is_none(), "nothing was promised");
}

#[test]
fn two_counts_that_sum_past_a_word_do_not_wrap_the_limit_check() {
    let brimming = u64::from(u32::MAX) | (u64::from(u32::MAX) << 32);

    assert_eq!(Quota::charged(brimming), u64::from(u32::MAX) * 2);
    assert!(
        Quota::reserve(brimming, usize::MAX).is_none(),
        "a full table admitted an operation"
    );
    assert!(Quota::take(brimming, usize::MAX).is_none());
}

#[test]
fn the_two_counts_do_not_read_each_others_bits() {
    let limit = 16;
    let mut bits = Quota::EMPTY;
    for _ in 0..5 {
        bits = Quota::take(bits, limit).expect("room");
    }
    for _ in 0..3 {
        bits = Quota::reserve(bits, limit).expect("room");
    }

    assert_eq!(Quota::live(bits), 5);
    assert_eq!(Quota::reserved(bits), 3);
    assert_eq!(Quota::charged(bits), 8);
}
