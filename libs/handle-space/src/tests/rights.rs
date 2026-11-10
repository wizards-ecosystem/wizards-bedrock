use crate::*;

/// The rights are read from the source instead of from a second list.
#[test]
fn the_two_halves_do_not_overlap() {
    const COMPOSITE: [&str; 4] = ["NONE", "UNIVERSAL", "ALL", "EMPTY"];
    let named: Vec<&str> = include_str!("../lib.rs")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("pub const "))
        .filter(|rest| rest.contains(": Self = Self("))
        .filter_map(|rest| rest.split_once(':').map(|(name, _)| name))
        .filter(|name| !COMPOSITE.contains(name))
        .collect();
    assert!(named.len() >= 11, "only {} rights were read", named.len());

    let bits = |name: &str| -> u32 {
        let line = include_str!("../lib.rs")
            .lines()
            .find(|line| {
                line.trim()
                    .starts_with(&format!("pub const {name}: Self = Self("))
            })
            .unwrap_or_else(|| panic!("no {name}"));
        let inside = line
            .trim()
            .trim_start_matches(&format!("pub const {name}: Self = Self("))
            .trim_end_matches(");");
        let (shift, _) = inside
            .split_once(" << ")
            .unwrap_or_else(|| panic!("{name} is not a single bit: {inside}"));
        let _ = shift;
        let place = <u32 as core::str::FromStr>::from_str(
            inside.split(" << ").nth(1).expect("a shift").trim(),
        )
        .expect("a bit position");
        1 << place
    };

    let mut seen = 0u32;
    for name in &named {
        let bit = bits(name);
        assert_eq!(bit.count_ones(), 1, "{name} is one bit");
        assert_eq!(seen & bit, 0, "{name} shares a bit with another right");
        seen |= bit;
    }
    assert_eq!(seen, Rights::ALL.bits(), "ALL is not every right");

    assert_eq!(
        Rights::UNIVERSAL.bits() & (seen & !Rights::UNIVERSAL.bits()),
        0
    );
    assert_eq!(Rights::UNIVERSAL.bits().count_ones(), 4);
}

#[test]
fn an_undefined_right_is_refused() {
    let defined = Rights::ALL.bits();
    let mut checked = 0;
    for bit in 0..32u32 {
        if defined & (1 << bit) != 0 {
            continue;
        }
        assert_eq!(
            Rights::from_bits(1 << bit),
            None,
            "bit {bit} is not a right"
        );
        checked += 1;
    }
    assert!(checked > 0, "every bit is defined, so nothing was checked");

    assert_eq!(Rights::from_bits(defined), Some(Rights::ALL));
    assert_eq!(Rights::from_bits(0), Some(Rights::NONE));
    let first_undefined = (0..32u32)
        .find(|bit| defined & (1 << bit) == 0)
        .expect("some bit is undefined");
    assert_eq!(Rights::from_bits(defined | (1 << first_undefined)), None);
}

#[test]
fn attenuation_never_widens_the_held_rights() {
    let held = Rights::READ.union(Rights::WRITE).union(Rights::TRANSFER);

    assert_eq!(held.union(Rights::READ), held, "union removed a held right");
    assert_eq!(held.attenuate(Rights::READ), Some(Rights::READ));
    assert_eq!(held.attenuate(Rights::NONE), Some(Rights::NONE));
    assert_eq!(held.attenuate(held), Some(held));
    assert_eq!(
        held.intersection(Rights::UNIVERSAL),
        Rights::TRANSFER,
        "intersection restored or discarded a held universal right"
    );

    assert_eq!(held.attenuate(Rights::EXECUTE), None, "gained a right");
    assert_eq!(
        held.attenuate(held.union(Rights::MAP)),
        None,
        "gained a right by asking for everything plus one"
    );

    for bits in 0..=Rights::ALL.bits() {
        let Some(wanted) = Rights::from_bits(bits) else {
            continue;
        };
        if let Some(attenuated) = held.attenuate(wanted) {
            assert!(held.contains(attenuated), "attenuation widened {bits:#x}");
        }
    }
}

#[test]
fn transfer_can_be_stripped_on_its_own() {
    let held = Rights::READ.union(Rights::TRANSFER).union(Rights::WAIT);
    let confined = held.difference(Rights::TRANSFER);

    assert!(!confined.contains(Rights::TRANSFER));
    assert!(confined.contains(Rights::READ) && confined.contains(Rights::WAIT));
    assert_eq!(held.attenuate(confined), Some(confined));
}

#[test]
fn contains_means_every_one() {
    let held = Rights::READ.union(Rights::WRITE);
    assert!(held.contains(Rights::READ.union(Rights::WRITE)));
    assert!(!held.contains(Rights::READ.union(Rights::EXECUTE)));
    assert!(Rights::ALL.contains(held));
    assert!(held.contains(Rights::NONE), "everything contains nothing");
}

#[test]
fn a_right_that_means_nothing_for_a_type_is_not_meaningful() {
    let none = Rights::NONE;
    assert!(Rights::UNIVERSAL.are_meaningful_for(none));
    assert!(Rights::DUPLICATE.are_meaningful_for(none));
    assert!(
        !Rights::READ.are_meaningful_for(none),
        "read means nothing here"
    );
    assert!(!Rights::ALL.are_meaningful_for(none));

    let memory = Rights::READ
        .union(Rights::WRITE)
        .union(Rights::EXECUTE)
        .union(Rights::MAP);
    assert!(
        Rights::UNIVERSAL.union(memory).are_meaningful_for(memory),
        "its own rights and the universal four"
    );
    assert!(Rights::READ.union(Rights::WAIT).are_meaningful_for(memory));
    assert!(!Rights::ALL.are_meaningful_for(memory));
    assert!(!Rights::TRAVERSE.are_meaningful_for(memory));

    let directory = Rights::TRAVERSE
        .union(Rights::ENUMERATE)
        .union(Rights::MODIFY);
    assert!(Rights::TRAVERSE.are_meaningful_for(directory));
    assert!(
        !Rights::READ.are_meaningful_for(directory),
        "a directory has no bytes"
    );
    assert!(!Rights::MAP.are_meaningful_for(directory));

    assert!(Rights::UNIVERSAL.are_meaningful_for(Rights::NONE));
}

#[test]
fn a_raw_counter_at_the_top_of_the_range_still_advances() {
    assert_eq!(next_generation(u32::MAX), 0);
    assert_eq!(next_generation(GENERATIONS - 1), 0);
    assert_eq!(next_generation(GENERATIONS), 1);
}
