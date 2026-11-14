use super::*;

#[test]
fn a_handle_does_not_travel_between_tables() {
    let slot = Slot::new(3, 5).expect("within range");
    let here = Mixer::new(0x1111_1111);
    let there = Mixer::new(0x2222_2222);
    assert_ne!(slot.encode(here), slot.encode(there));

    assert_ne!(Slot::decode(slot.encode(here), there), Some(slot));
}

/// Newton iteration derives the multiplier's inverse instead of restating it.
#[test]
fn no_table_number_gets_a_mixer_that_mixes_nothing() {
    let mut inverse = MULTIPLIER;
    for _ in 0..5 {
        inverse = inverse.wrapping_mul(2u32.wrapping_sub(MULTIPLIER.wrapping_mul(inverse)));
    }
    assert_eq!(MULTIPLIER.wrapping_mul(inverse), 1, "not an inverse");

    for cleared in 0..=RESERVED {
        let id = cleared.wrapping_sub(ADDEND).wrapping_mul(inverse);
        assert_eq!(
            id.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND),
            cleared,
            "the inverse did not name the id landing on {cleared}"
        );
        assert!(
            Mixer::for_table(id).bits() != 0,
            "table {id} got a mixer that mixes nothing"
        );
    }

    for id in 0..1024 {
        assert!(Mixer::for_table(id).bits() != 0, "table {id}");
    }
}

mod rights;
