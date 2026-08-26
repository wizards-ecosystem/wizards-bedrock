//! Scan code set 1's two lookup tables.
//!
//! The set a translating controller delivers. Same shape as [`crate::table`] —
//! a base table and an `E0` table — over different numbers, and with the
//! release encoded in the high bit rather than an `F0` prefix, which is the
//! decoder's problem rather than this module's.
//!
//! Source: Andries Brouwer, *Keyboard scancodes*, "Scan code set 1", the same
//! reference [`crate::table`] is transcribed from. The correspondence D-162
//! settled is one of the entries here: backslash is `0x2B` in set 1 where it
//! is `0x5D` in set 2.

use crate::key::Key;

/// The key a bare set 1 make code names.
///
/// Codes run `0x01..=0x58` with gaps; everything above is a release, which
/// never reaches here.
pub(crate) fn key_for(code: u8) -> Option<Key> {
    Some(match code {
        0x01 => Key::Escape,
        0x02 => Key::Digit1,
        0x03 => Key::Digit2,
        0x04 => Key::Digit3,
        0x05 => Key::Digit4,
        0x06 => Key::Digit5,
        0x07 => Key::Digit6,
        0x08 => Key::Digit7,
        0x09 => Key::Digit8,
        0x0A => Key::Digit9,
        0x0B => Key::Digit0,
        0x0C => Key::Minus,
        0x0D => Key::Equal,
        0x0E => Key::Backspace,
        0x0F => Key::Tab,
        0x10 => Key::Q,
        0x11 => Key::W,
        0x12 => Key::E,
        0x13 => Key::R,
        0x14 => Key::T,
        0x15 => Key::Y,
        0x16 => Key::U,
        0x17 => Key::I,
        0x18 => Key::O,
        0x19 => Key::P,
        0x1A => Key::LeftBracket,
        0x1B => Key::RightBracket,
        0x1C => Key::Enter,
        0x1D => Key::CtrlLeft,
        0x1E => Key::A,
        0x1F => Key::S,
        0x20 => Key::D,
        0x21 => Key::F,
        0x22 => Key::G,
        0x23 => Key::H,
        0x24 => Key::J,
        0x25 => Key::K,
        0x26 => Key::L,
        0x27 => Key::Semicolon,
        0x28 => Key::Apostrophe,
        0x29 => Key::Grave,
        0x2A => Key::ShiftLeft,
        0x2B => Key::Backslash,
        0x2C => Key::Z,
        0x2D => Key::X,
        0x2E => Key::C,
        0x2F => Key::V,
        0x30 => Key::B,
        0x31 => Key::N,
        0x32 => Key::M,
        0x33 => Key::Comma,
        0x34 => Key::Period,
        0x35 => Key::Slash,
        0x36 => Key::ShiftRight,
        0x37 => Key::KeypadMultiply,
        0x38 => Key::AltLeft,
        0x39 => Key::Space,
        0x3A => Key::CapsLock,
        0x3B => Key::F1,
        0x3C => Key::F2,
        0x3D => Key::F3,
        0x3E => Key::F4,
        0x3F => Key::F5,
        0x40 => Key::F6,
        0x41 => Key::F7,
        0x42 => Key::F8,
        0x43 => Key::F9,
        0x44 => Key::F10,
        0x45 => Key::NumLock,
        0x46 => Key::ScrollLock,
        0x47 => Key::Keypad7,
        0x48 => Key::Keypad8,
        0x49 => Key::Keypad9,
        0x4A => Key::KeypadMinus,
        0x4B => Key::Keypad4,
        0x4C => Key::Keypad5,
        0x4D => Key::Keypad6,
        0x4E => Key::KeypadPlus,
        0x4F => Key::Keypad1,
        0x50 => Key::Keypad2,
        0x51 => Key::Keypad3,
        0x52 => Key::Keypad0,
        0x53 => Key::KeypadPeriod,
        0x57 => Key::F11,
        0x58 => Key::F12,
        _ => return None,
    })
}

/// The key an `E0`-prefixed set 1 code names.
pub(crate) fn extended_key_for(code: u8) -> Option<Key> {
    Some(match code {
        0x1C => Key::KeypadEnter,
        0x1D => Key::CtrlRight,
        0x35 => Key::KeypadDivide,
        0x37 => Key::PrintScreen,
        0x38 => Key::AltRight,
        0x47 => Key::Home,
        0x48 => Key::ArrowUp,
        0x49 => Key::PageUp,
        0x4B => Key::ArrowLeft,
        0x4D => Key::ArrowRight,
        0x4F => Key::End,
        0x50 => Key::ArrowDown,
        0x51 => Key::PageDown,
        0x52 => Key::Insert,
        0x53 => Key::Delete,
        0x5B => Key::SuperLeft,
        0x5C => Key::SuperRight,
        0x5D => Key::Menu,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table;

    /// Every key one set names, the other names too.
    ///
    /// The check that makes a second transcription worth more than the sum of
    /// two transcriptions: set 1 and set 2 are different numbers for the same
    /// hundred-odd keys, so a key present in one table and absent from the
    /// other is a hole in whichever is newer, and there is no third table to
    /// ask. A dropped entry, a duplicated arm shadowing its neighbour, and a
    /// key spelled into the wrong half — base where it belongs behind `E0` —
    /// are all caught here, and none of them by reading (D-179).
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

    /// No code names two keys, and no key has two codes.
    ///
    /// A transcription slip that repeats a number silently shadows the arm
    /// below it — the match arms are tried in order, so the second `0x1E` in a
    /// table is unreachable and the key it names is simply gone. The previous
    /// test would catch that one; this one catches the reverse, a key reachable
    /// two ways, which nothing else would.
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

    /// The correspondence `004`'s D-162 settled, as an anchor.
    ///
    /// That entry cost a research pass and two independent sources to resolve,
    /// and it names both sets — so it is the one pair in this table that
    /// arrives already verified, and it is asserted rather than trusted.
    #[test]
    fn backslash_is_the_correspondence_d162_settled() {
        assert_eq!(key_for(0x2B), Some(Key::Backslash));
        assert_eq!(table::key_for(0x5D), Some(Key::Backslash));
    }

    /// The rows a QWERTY keyboard lays out contiguously, in set 1.
    ///
    /// Set 1's numbering follows the physical rows, which is the property that
    /// makes a single mistyped entry visible: every letter row is a run with
    /// no gaps, so a swapped pair breaks the run.
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
}
