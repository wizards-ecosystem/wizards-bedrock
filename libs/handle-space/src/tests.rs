use super::*;

/// A mixer that is not zero, so the mixing is actually exercised.
const MIXER: Mixer = Mixer::new(0x5A5A_1234);

#[test]
fn a_decoded_slot_reports_the_index_and_generation_it_holds() {
    // Distinct values expose swapped accessors (D-136).
    const INDEX: u32 = 37;
    const GENERATION: u32 = 1234;

    let mixer = Mixer::for_table(9);
    let handle = Slot::new(INDEX, GENERATION)
        .expect("inside the arena")
        .encode(mixer);
    let decoded = Slot::decode(handle, mixer).expect("round trips");

    assert_eq!(decoded.index(), INDEX, "index() gave something else");
    assert_eq!(
        decoded.generation(),
        GENERATION,
        "generation() gave something else"
    );
    assert_ne!(
        decoded.index(),
        decoded.generation(),
        "the two accessors return the same field"
    );
}

#[test]
fn the_widths_tile_the_word() {
    // Pin the policy values, not only their internally consistent formulas.
    assert_eq!(RESERVED_BITS, 2, "the low two bits are reserved (D-78)");
    assert_eq!(INDEX_BITS, 18, "eighteen bits of arena index");
    assert_eq!(GENERATION_BITS, 12, "and twelve of generation");
    assert_eq!(SLOTS, 262_144);
    assert_eq!(GENERATIONS, 4096);

    assert_eq!(RESERVED_BITS + INDEX_BITS + GENERATION_BITS, 32);
    assert_eq!(RESERVED, (1 << RESERVED_BITS) - 1);
    assert_eq!(SLOTS, 1 << INDEX_BITS);
    assert_eq!(GENERATIONS, 1 << GENERATION_BITS);
}

#[test]
fn a_slot_round_trips() {
    let corners = [0, 1, SLOTS / 2, SLOTS - 2, SLOTS - 1];
    let ages = [0, 1, GENERATIONS / 2, GENERATIONS - 2, GENERATIONS - 1];
    for index in corners {
        for generation in ages {
            let slot = Slot::new(index, generation).expect("within range");
            assert_eq!(
                Slot::decode(slot.encode(MIXER), MIXER),
                Some(slot),
                "slot {index}/{generation} did not survive a round trip"
            );
        }
    }
}

#[test]
fn a_malformed_value_never_decodes() {
    for value in [0, 1, 2, 4, 8, 16, 100, u32::MAX / 2, u32::MAX - 1] {
        let broken = (value ^ MIXER.bits()) & RESERVED != RESERVED;
        assert_eq!(
            Slot::decode(value, MIXER).is_none(),
            broken,
            "{value:#x} disagreed about being well formed"
        );
    }

    for low in 0..(1u32 << 8) {
        let value = low ^ MIXER.bits();
        let intact = low & RESERVED == RESERVED;
        assert_eq!(
            Slot::decode(value, MIXER).is_some(),
            intact,
            "low bits {low:#b} disagreed about being well formed"
        );
    }
}

#[test]
fn zero_is_never_a_handle() {
    for seed in [0, 1, 0xFFFF_FFFF, 0x5A5A_1234, 0xDEAD_BEEF] {
        let mixer = Mixer::new(seed);
        assert_eq!(
            Slot::decode(0, mixer),
            None,
            "zero decoded under mixer {seed:#x}"
        );
    }
}

#[test]
fn a_reused_slot_never_repeats_before_it_wraps() {
    let mut slot = Slot::new(9, 0).expect("within range");
    let first = slot.encode(MIXER);
    let mut seen = alloc_set();

    for step in 0..GENERATIONS {
        let value = slot.encode(MIXER);
        assert!(
            seen.insert(value),
            "slot 9 repeated a handle after {step} reuses"
        );
        slot = slot.reused();
    }

    assert_eq!(
        slot.generation(),
        0,
        "the counter did not wrap where it says"
    );
    assert_eq!(slot.encode(MIXER), first, "the wrap is where it repeats");
}

#[test]
fn no_two_slots_share_a_handle() {
    let mut seen = alloc_set();
    for index in [0, 1, 2, SLOTS - 1] {
        for generation in [0, 1, GENERATIONS - 1] {
            let slot = Slot::new(index, generation).expect("within range");
            assert!(
                seen.insert(slot.encode(MIXER)),
                "slot {index}/{generation} collided with an earlier one"
            );
        }
    }
}

#[test]
fn a_handle_does_not_travel_between_tables() {
    let slot = Slot::new(3, 5).expect("within range");
    let here = Mixer::new(0x1111_1111);
    let there = Mixer::new(0x2222_2222);
    assert_ne!(slot.encode(here), slot.encode(there));

    assert_ne!(Slot::decode(slot.encode(here), there), Some(slot));
}

#[test]
fn no_table_number_gets_a_mixer_that_mixes_nothing() {
    // Newton iteration derives the inverse instead of restating it.
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

#[test]
fn identity_detection_reads_the_mixer_bits() {
    assert!(Mixer::new(0).is_identity());
    assert!(!Mixer::new(1 << RESERVED_BITS).is_identity());
}

#[test]
fn table_numbers_spread_across_mixers() {
    let mut seen = Vec::new();
    for id in 0..256 {
        seen.push(Mixer::for_table(id).bits());
    }
    seen.sort_unstable();
    let before = seen.len();
    seen.dedup();
    assert_eq!(seen.len(), before, "two table numbers shared a mixer");
}

#[test]
fn a_mixer_cannot_break_the_reserved_bits() {
    for seed in [0, 1, 2, 3, 0xFFFF_FFFF, 0xDEAD_BEEF] {
        let mixer = Mixer::new(seed);
        assert_eq!(mixer.bits() & RESERVED, 0, "seed {seed:#x} kept low bits");
        let slot = Slot::new(1, 1).expect("within range");
        assert_eq!(Slot::decode(slot.encode(mixer), mixer), Some(slot));
    }
}

#[test]
fn an_out_of_range_slot_is_refused() {
    assert_eq!(Slot::new(SLOTS, 0), None);
    assert_eq!(Slot::new(0, GENERATIONS), None);
    assert_eq!(Slot::new(u32::MAX, u32::MAX), None);
    assert!(Slot::new(SLOTS - 1, GENERATIONS - 1).is_some());
}

#[test]
fn table_identity_exhaustion_never_wraps() {
    assert_eq!(next_table_identity(u64::MAX - 1), Some(u64::MAX));
    assert_eq!(next_table_identity(u64::MAX), None);
}

fn alloc_set() -> std::collections::HashSet<u32> {
    std::collections::HashSet::new()
}

mod quota;
mod rights;
