//! Scan code set 2's two lookup tables.
//!
//! The base table for bare make codes, and the extended table for codes
//! arriving behind an `E0` prefix. Values are the IBM PS/2 specification's
//! set 2 assignment. An unknown code maps to nothing: multimedia and
//! vendor keys are swallowed upstream rather than being errors, because a
//! keyboard is allowed to have buttons this kernel has no name for.

use crate::key::Key;

/// The key a bare set 2 make code names.
///
/// Note F7: `0x83`, the one assignment above `0x7F`, which is why this table
/// is over the full byte and why break codes need their `F0` prefix rather
/// than a high bit.
pub(crate) fn key_for(code: u8) -> Option<Key> {
    Some(match code {
        0x01 => Key::F9,
        0x03 => Key::F5,
        0x04 => Key::F3,
        0x05 => Key::F1,
        0x06 => Key::F2,
        0x07 => Key::F12,
        0x09 => Key::F10,
        0x0A => Key::F8,
        0x0B => Key::F6,
        0x0C => Key::F4,
        0x0D => Key::Tab,
        0x0E => Key::Grave,
        0x11 => Key::AltLeft,
        0x12 => Key::ShiftLeft,
        0x14 => Key::CtrlLeft,
        0x15 => Key::Q,
        0x16 => Key::Digit1,
        0x1A => Key::Z,
        0x1B => Key::S,
        0x1C => Key::A,
        0x1D => Key::W,
        0x1E => Key::Digit2,
        0x21 => Key::C,
        0x22 => Key::X,
        0x23 => Key::D,
        0x24 => Key::E,
        0x25 => Key::Digit4,
        0x26 => Key::Digit3,
        0x29 => Key::Space,
        0x2A => Key::V,
        0x2B => Key::F,
        0x2C => Key::T,
        0x2D => Key::R,
        0x2E => Key::Digit5,
        0x31 => Key::N,
        0x32 => Key::B,
        0x33 => Key::H,
        0x34 => Key::G,
        0x35 => Key::Y,
        0x36 => Key::Digit6,
        0x3A => Key::M,
        0x3B => Key::J,
        0x3C => Key::U,
        0x3D => Key::Digit7,
        0x3E => Key::Digit8,
        0x41 => Key::Comma,
        0x42 => Key::K,
        0x43 => Key::I,
        0x44 => Key::O,
        0x45 => Key::Digit0,
        0x46 => Key::Digit9,
        0x49 => Key::Period,
        0x4A => Key::Slash,
        0x4B => Key::L,
        0x4C => Key::Semicolon,
        0x4D => Key::P,
        0x4E => Key::Minus,
        0x52 => Key::Apostrophe,
        0x54 => Key::LeftBracket,
        0x55 => Key::Equal,
        0x58 => Key::CapsLock,
        0x59 => Key::ShiftRight,
        0x5A => Key::Enter,
        0x5B => Key::RightBracket,
        0x5D => Key::Backslash,
        0x66 => Key::Backspace,
        0x69 => Key::Keypad1,
        0x6B => Key::Keypad4,
        0x6C => Key::Keypad7,
        0x70 => Key::Keypad0,
        0x71 => Key::KeypadPeriod,
        0x72 => Key::Keypad2,
        0x73 => Key::Keypad5,
        0x74 => Key::Keypad6,
        0x75 => Key::Keypad8,
        0x76 => Key::Escape,
        0x77 => Key::NumLock,
        0x78 => Key::F11,
        0x79 => Key::KeypadPlus,
        0x7A => Key::Keypad3,
        0x7B => Key::KeypadMinus,
        0x7C => Key::KeypadMultiply,
        0x7D => Key::Keypad9,
        0x7E => Key::ScrollLock,
        0x83 => Key::F7,
        _ => return None,
    })
}

/// The key an `E0`-prefixed code names — the navigation cluster, the right
/// modifiers, and the two keypad keys the base table has no room for.
pub(crate) fn extended_key_for(code: u8) -> Option<Key> {
    Some(match code {
        0x11 => Key::AltRight,
        0x14 => Key::CtrlRight,
        0x1F => Key::SuperLeft,
        0x27 => Key::SuperRight,
        0x2F => Key::Menu,
        0x4A => Key::KeypadDivide,
        0x5A => Key::KeypadEnter,
        0x69 => Key::End,
        0x6B => Key::ArrowLeft,
        0x6C => Key::Home,
        0x70 => Key::Insert,
        0x71 => Key::Delete,
        0x72 => Key::ArrowDown,
        0x74 => Key::ArrowRight,
        0x75 => Key::ArrowUp,
        0x7A => Key::PageDown,
        0x7C => Key::PrintScreen,
        0x7D => Key::PageUp,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spot anchors transcribed from the set 2 table, one from every region:
    /// letters, digits, punctuation, function row, modifiers, keypad, and the
    /// one code above `0x7F`.
    /// Every single-byte set 2 make code, transcribed from the specification.
    ///
    /// Twenty-one of about eighty-nine codes were anchored before this, so a
    /// swap of two unanchored keys survived the suite — `0x33`/`0x34` would
    /// have exchanged H and G on every keyboard and no test could say so
    /// (D-156).
    ///
    /// Source: Andries Brouwer, *Keyboard scancodes*, section 10, "Scan code
    /// set 2" — the table Linux's own documentation points at. The numbers are
    /// the specification's, so they are written out rather than derived.
    ///
    /// **Backslash was the one code left unanchored, and is now anchored.**
    /// D-156 recorded a conflict — the source read as `0x5C` where the table
    /// has `0x5D` — and refused to assert either without a second source. The
    /// conflict was a misread column, not an ambiguity: Brouwer's
    /// correspondence table gives backslash as `0x2B` in set 1, `0x5D` in set
    /// **2**, and `0x5C` in set **3**, and Vetra's translation table agrees
    /// independently. `0x5C` is the set-3 value for a keyboard this kernel
    /// never puts in set 3 (D-162).
    #[test]
    fn every_single_byte_make_code_is_the_specification_s() {
        const SET_2: [(u8, Key); 69] = [
            (0x1C, Key::A),
            (0x32, Key::B),
            (0x21, Key::C),
            (0x23, Key::D),
            (0x24, Key::E),
            (0x2B, Key::F),
            (0x34, Key::G),
            (0x33, Key::H),
            (0x43, Key::I),
            (0x3B, Key::J),
            (0x42, Key::K),
            (0x4B, Key::L),
            (0x3A, Key::M),
            (0x31, Key::N),
            (0x44, Key::O),
            (0x4D, Key::P),
            (0x15, Key::Q),
            (0x2D, Key::R),
            (0x1B, Key::S),
            (0x2C, Key::T),
            (0x3C, Key::U),
            (0x2A, Key::V),
            (0x1D, Key::W),
            (0x22, Key::X),
            (0x35, Key::Y),
            (0x1A, Key::Z),
            (0x45, Key::Digit0),
            (0x16, Key::Digit1),
            (0x1E, Key::Digit2),
            (0x26, Key::Digit3),
            (0x25, Key::Digit4),
            (0x2E, Key::Digit5),
            (0x36, Key::Digit6),
            (0x3D, Key::Digit7),
            (0x3E, Key::Digit8),
            (0x46, Key::Digit9),
            (0x76, Key::Escape),
            (0x66, Key::Backspace),
            (0x0D, Key::Tab),
            (0x58, Key::CapsLock),
            (0x12, Key::ShiftLeft),
            (0x59, Key::ShiftRight),
            (0x14, Key::CtrlLeft),
            (0x11, Key::AltLeft),
            (0x29, Key::Space),
            (0x5A, Key::Enter),
            (0x4E, Key::Minus),
            (0x55, Key::Equal),
            (0x54, Key::LeftBracket),
            (0x5B, Key::RightBracket),
            (0x4C, Key::Semicolon),
            (0x52, Key::Apostrophe),
            (0x0E, Key::Grave),
            (0x41, Key::Comma),
            (0x49, Key::Period),
            (0x4A, Key::Slash),
            (0x5D, Key::Backslash),
            (0x05, Key::F1),
            (0x06, Key::F2),
            (0x04, Key::F3),
            (0x0C, Key::F4),
            (0x03, Key::F5),
            (0x0B, Key::F6),
            (0x83, Key::F7),
            (0x0A, Key::F8),
            (0x01, Key::F9),
            (0x09, Key::F10),
            (0x78, Key::F11),
            (0x07, Key::F12),
        ];

        for (code, key) in SET_2 {
            assert_eq!(
                key_for(code),
                Some(key),
                "make code {code:#04X} is not {key:?}"
            );
        }

        // And the mapping is injective: two codes naming one key would mean a
        // transcription error that the loop above cannot see.
        for (index, (code, key)) in SET_2.iter().enumerate() {
            for (earlier_code, earlier_key) in &SET_2[..index] {
                assert_ne!(code, earlier_code, "{code:#04X} appears twice");
                assert_ne!(
                    key, earlier_key,
                    "{key:?} is named by both {earlier_code:#04X} and {code:#04X}"
                );
            }
        }
    }

    #[test]
    fn make_codes_match_the_set_two_table() {
        for (code, key) in [
            (0x1C, Key::A),
            (0x32, Key::B),
            (0x15, Key::Q),
            (0x1A, Key::Z),
            (0x16, Key::Digit1),
            (0x45, Key::Digit0),
            (0x5A, Key::Enter),
            (0x76, Key::Escape),
            (0x66, Key::Backspace),
            (0x0D, Key::Tab),
            (0x29, Key::Space),
            (0x58, Key::CapsLock),
            (0x12, Key::ShiftLeft),
            (0x59, Key::ShiftRight),
            (0x14, Key::CtrlLeft),
            (0x11, Key::AltLeft),
            (0x05, Key::F1),
            (0x83, Key::F7),
            (0x77, Key::NumLock),
            (0x4E, Key::Minus),
            (0x55, Key::Equal),
        ] {
            assert_eq!(key_for(code), Some(key), "{code:#04x}");
        }
    }

    #[test]
    fn extended_codes_match_the_set_two_table() {
        for (code, key) in [
            (0x14, Key::CtrlRight),
            (0x11, Key::AltRight),
            (0x75, Key::ArrowUp),
            (0x6B, Key::ArrowLeft),
            (0x72, Key::ArrowDown),
            (0x74, Key::ArrowRight),
            (0x70, Key::Insert),
            (0x71, Key::Delete),
            (0x6C, Key::Home),
            (0x69, Key::End),
            (0x7D, Key::PageUp),
            (0x7A, Key::PageDown),
            (0x5A, Key::KeypadEnter),
            (0x4A, Key::KeypadDivide),
            (0x1F, Key::SuperLeft),
            (0x2F, Key::Menu),
            (0x7C, Key::PrintScreen),
        ] {
            assert_eq!(extended_key_for(code), Some(key), "{code:#04x}");
        }
    }

    /// No code may name two keys, within a table or its own sweep.
    #[test]
    fn no_two_codes_share_a_key_within_a_table() {
        for table in [key_for, extended_key_for] {
            for code in 0..=u8::MAX {
                let Some(key) = table(code) else { continue };
                // Widened: `code + 1` overflows the loop counter at `0xFF`,
                // so assigning that code a key would make this test panic in
                // its own arithmetic rather than report (D-105).
                for other in (u16::from(code) + 1)..=u16::from(u8::MAX) {
                    let other = other as u8;
                    assert_ne!(table(other), Some(key), "{code:#04x} vs {other:#04x}");
                }
            }
        }
    }

    #[test]
    fn an_unassigned_code_names_nothing() {
        assert_eq!(key_for(0x00), None);
        assert_eq!(key_for(0xF7), None);
        assert_eq!(extended_key_for(0x1C), None);
    }
}
