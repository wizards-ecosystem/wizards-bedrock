use super::*;
use crate::table;

/// Both sets number the same keys, so a key missing from one table is a hole in it, and there is no third table to ask.
#[test]
fn both_sets_name_the_same_keys() {
    let mut only_in_set_1 = Vec::new();
    let mut only_in_set_2 = Vec::new();

    for code in 0..=u8::MAX {
        for (ours, theirs) in [
            (key_for(code), table::key_for(code)),
            (extended_key_for(code), table::extended_key_for(code)),
        ] {
            if let Some(key) = ours
                && !names(table::key_for, table::extended_key_for, key)
            {
                only_in_set_1.push((code, key));
            }
            if let Some(key) = theirs
                && !names(key_for, extended_key_for, key)
            {
                only_in_set_2.push((code, key));
            }
        }
    }

    assert!(
        only_in_set_1.is_empty() && only_in_set_2.is_empty(),
        "set 1 names {only_in_set_1:?} which set 2 does not; \
         set 2 names {only_in_set_2:?} which set 1 does not"
    );
}

/// Whether either of a set's two tables names `key`.
fn names(base: fn(u8) -> Option<Key>, extended: fn(u8) -> Option<Key>, key: Key) -> bool {
    (0..=u8::MAX).any(|code| base(code) == Some(key) || extended(code) == Some(key))
}

/// Repeated codes leave a later arm unreachable; this direction catches a key reachable through two codes.
#[test]
fn no_key_is_named_twice_within_a_table() {
    for table in [
        key_for as fn(u8) -> Option<Key>,
        extended_key_for as fn(u8) -> Option<Key>,
    ] {
        let named: Vec<Key> = (0..=u8::MAX).filter_map(table).collect();
        for (index, key) in named.iter().enumerate() {
            assert!(
                !named[index + 1..].contains(key),
                "{key:?} is named by two codes in one table"
            );
        }
    }
}

/// The one pair two independent sources verified for both sets, asserted as an anchor.
#[test]
fn backslash_is_the_correspondence_d162_settled() {
    assert_eq!(key_for(0x2B), Some(Key::Backslash));
    assert_eq!(table::key_for(0x5D), Some(Key::Backslash));
}

/// Set 1 numbers each letter row as a run with no gaps, so a swapped pair breaks the run.
#[test]
fn the_letter_rows_are_contiguous_runs() {
    for (start, row) in [
        (0x10_u8, "QWERTYUIOP"),
        (0x1E, "ASDFGHJKL"),
        (0x2C, "ZXCVBNM"),
    ] {
        for (offset, letter) in row.chars().enumerate() {
            let code = start + u8::try_from(offset).expect("a row is short");
            let key = key_for(code).expect("every code in a row names a key");
            assert_eq!(
                crate::us::char_for(key, crate::shift::Shift::new()),
                Some(letter.to_ascii_lowercase()),
                "set 1 code {code:#04x} should be {letter}"
            );
        }
    }
}
