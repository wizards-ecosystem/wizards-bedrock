use super::*;
use crate::{set1, table};

/// If either table grows a code past the probe's bound, this fails before a machine mis-decodes a keystroke.
#[test]
fn the_two_tables_still_bound_this_probe() {
    for code in 0..=u8::MAX {
        for (set_2, set_1) in [
            (table::key_for(code), set1::key_for(code)),
            (table::extended_key_for(code), set1::extended_key_for(code)),
        ] {
            assert!(
                set_2.is_none() || code <= SET_2_HIGHEST_MAKE,
                "set 2 make {code:#04x} is above the probe's bound"
            );
            assert!(
                set_1.is_none() || code | 0x80 <= SET_1_HIGHEST_BREAK,
                "set 1 make {code:#04x} releases above the probe's bound"
            );
        }
    }
    let unshifted = BREAK & !0x80;
    assert!(
        set1::key_for(unshifted).is_none() && set1::extended_key_for(unshifted).is_none(),
        "set 1 gained a {unshifted:#04x} key, so F0 is now one of its releases"
    );
}

/// `AB 41` and `AB 83` are the dishonest identity replies this probe routes around, so neither byte may decide.
#[test]
fn an_identity_reply_in_the_stream_decides_nothing() {
    for reply in [[0xABu8, 0x41], [0xAB, 0x83]] {
        for byte in reply {
            assert_eq!(
                SetProbe::observe(byte),
                Arriving::Undecided,
                "{byte:#04x} of an identity reply decided the numbering"
            );
        }
    }
}

#[test]
fn no_byte_proves_both_sets() {
    for byte in 0..=u8::MAX {
        let _ = SetProbe::observe(byte);
    }
    assert_eq!(SetProbe::observe(BREAK), Arriving::Set2);
    for byte in AMBIGUOUS {
        assert_eq!(
            SetProbe::observe(byte),
            Arriving::Undecided,
            "{byte:#04x} is a reply as well as a set 1 release"
        );
    }
    assert_eq!(SetProbe::observe(SET_2_HIGHEST_MAKE), Arriving::Undecided);
    assert_eq!(SetProbe::observe(SET_1_HIGHEST_BREAK), Arriving::Set1);
    assert_eq!(SetProbe::observe(0xDE), Arriving::Undecided);
}

/// The bytes two machines delivered: VMware's `c` as set 2 under a set 1 identity, and a translating controller's set 1.
#[test]
fn one_keystroke_from_either_machine_decides_it() {
    let set_2 = [0x21u8, 0xF0, 0x21];
    let decided = set_2.iter().map(|byte| SetProbe::observe(*byte));
    assert_eq!(
        decided.clone().find(|seen| *seen != Arriving::Undecided),
        Some(Arriving::Set2)
    );
    assert_eq!(decided.count(), 3, "the probe consumes nothing");

    let set_1 = [0x2Eu8, 0xAE];
    assert_eq!(
        set_1
            .iter()
            .map(|byte| SetProbe::observe(*byte))
            .find(|seen| *seen != Arriving::Undecided),
        Some(Arriving::Set1)
    );
}

/// Set 2's Pause sequence decides on its `F0`.
#[test]
fn the_longest_set_2_sequence_still_decides_set_2() {
    let pause = [0xE1u8, 0x14, 0x77, 0xE1, 0xF0, 0x14, 0xF0, 0x77];
    assert_eq!(
        pause
            .iter()
            .map(|byte| SetProbe::observe(*byte))
            .find(|seen| *seen != Arriving::Undecided),
        Some(Arriving::Set2)
    );
}

#[test]
fn the_longest_set_1_sequence_never_says_set_2() {
    let pause = [0xE1u8, 0x1D, 0x45, 0xE1, 0x9D, 0xC5];
    let seen: Vec<Arriving> = pause.iter().map(|byte| SetProbe::observe(*byte)).collect();
    assert!(!seen.contains(&Arriving::Set2), "{seen:?}");
    assert!(seen.contains(&Arriving::Set1));
}
