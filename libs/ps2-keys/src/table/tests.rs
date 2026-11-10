use super::*;

/// Written out from Andries Brouwer's *Keyboard scancodes*, section 10, "Scan code set 2".
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

/// The inner sweep is widened to `u16` because `code + 1` overflows at `0xFF`.
#[test]
fn no_two_codes_share_a_key_within_a_table() {
    for table in [key_for, extended_key_for] {
        for code in 0..=u8::MAX {
            let Some(key) = table(code) else { continue };
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
