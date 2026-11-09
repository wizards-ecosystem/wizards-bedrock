use super::*;
use crate::event::{KeyEvent, KeyState};

fn held() -> Shift {
    let mut shift = Shift::new();
    shift.observe(KeyEvent {
        key: Key::ShiftLeft,
        state: KeyState::Pressed,
    });
    shift
}

#[test]
fn letters_follow_shift() {
    assert_eq!(char_for(Key::A, Shift::new()), Some('a'));
    assert_eq!(char_for(Key::A, held()), Some('A'));
}

#[test]
fn the_digit_row_shifts_to_symbols() {
    for (key, plain, shifted) in [
        (Key::Digit1, '1', '!'),
        (Key::Digit2, '2', '@'),
        (Key::Digit3, '3', '#'),
        (Key::Digit4, '4', '$'),
        (Key::Digit5, '5', '%'),
        (Key::Digit6, '6', '^'),
        (Key::Digit7, '7', '&'),
        (Key::Digit8, '8', '*'),
        (Key::Digit9, '9', '('),
        (Key::Digit0, '0', ')'),
    ] {
        assert_eq!(char_for(key, Shift::new()), Some(plain), "{key:?}");
        assert_eq!(char_for(key, held()), Some(shifted), "{key:?} shifted");
    }
}

#[test]
fn punctuation_shifts() {
    assert_eq!(char_for(Key::Semicolon, held()), Some(':'));
    assert_eq!(char_for(Key::Apostrophe, held()), Some('"'));
    assert_eq!(char_for(Key::Slash, held()), Some('?'));
}

/// The whole punctuation row, both halves, keyed to the legends on the
/// physical board — including the backslash, whose escaping makes it the
/// arm most worth pinning.
#[test]
fn every_punctuation_pair_matches_the_keycap() {
    for (key, plain, shifted) in [
        (Key::Grave, '`', '~'),
        (Key::Minus, '-', '_'),
        (Key::Equal, '=', '+'),
        (Key::LeftBracket, '[', '{'),
        (Key::RightBracket, ']', '}'),
        (Key::Backslash, '\\', '|'),
        (Key::Semicolon, ';', ':'),
        (Key::Apostrophe, '\'', '"'),
        (Key::Comma, ',', '<'),
        (Key::Period, '.', '>'),
        (Key::Slash, '/', '?'),
    ] {
        assert_eq!(char_for(key, Shift::new()), Some(plain), "{key:?}");
        assert_eq!(char_for(key, held()), Some(shifted), "{key:?} shifted");
    }
}

/// The three crates' worth of path, end to end: raw set 2 bytes through
/// the decoder, shift folded in as the driver folds it, characters out.
/// Shift down, 'a' becomes 'A'; shift released mid-stream, it goes back.
#[test]
fn a_decoded_stream_types_through_the_layout() {
    let mut decoder = crate::Decoder::new();
    let mut shift = Shift::new();
    let mut typed = String::new();

    let wire = [
        0x1C, 0xF0, 0x1C, // a down, a up
        0x12, // left shift down
        0x1C, 0xF0, 0x1C, // A
        0x16, 0xF0, 0x16, // !
        0xF0, 0x12, // left shift up
        0x1C, 0xF0, 0x1C, // a again
    ];
    for byte in wire {
        let Some(event) = decoder.feed(byte) else {
            continue;
        };
        shift.observe(event);
        if event.state == crate::KeyState::Pressed
            && let Some(character) = char_for(event.key, shift)
        {
            typed.push(character);
        }
    }

    assert_eq!(typed, "aA!a");
}

#[test]
fn whitespace_maps() {
    assert_eq!(char_for(Key::Space, Shift::new()), Some(' '));
    assert_eq!(char_for(Key::Enter, Shift::new()), Some('\n'));
    assert_eq!(char_for(Key::KeypadEnter, Shift::new()), Some('\n'));
    assert_eq!(char_for(Key::Tab, Shift::new()), Some('\t'));
}

#[test]
fn the_keypad_types_its_digits_regardless_of_shift() {
    for (key, character) in [
        (Key::Keypad0, '0'),
        (Key::Keypad1, '1'),
        (Key::Keypad2, '2'),
        (Key::Keypad3, '3'),
        (Key::Keypad4, '4'),
        (Key::Keypad5, '5'),
        (Key::Keypad6, '6'),
        (Key::Keypad7, '7'),
        (Key::Keypad8, '8'),
        (Key::Keypad9, '9'),
        (Key::KeypadPeriod, '.'),
        (Key::KeypadPlus, '+'),
        (Key::KeypadMinus, '-'),
        (Key::KeypadMultiply, '*'),
        (Key::KeypadDivide, '/'),
    ] {
        assert_eq!(char_for(key, Shift::new()), Some(character), "{key:?}");
        assert_eq!(char_for(key, held()), Some(character), "{key:?} shifted");
    }
}

#[test]
fn the_control_characters_a_console_acts_on_are_typed() {
    // Not printable, but each is an ASCII character a console implements,
    // so callers need no special case.
    assert_eq!(char_for(Key::Enter, Shift::new()), Some('\n'));
    assert_eq!(char_for(Key::KeypadEnter, Shift::new()), Some('\n'));
    assert_eq!(char_for(Key::Tab, Shift::new()), Some('\t'));
    assert_eq!(char_for(Key::Backspace, Shift::new()), Some('\u{8}'));
}

#[test]
fn a_key_that_types_nothing_maps_to_nothing() {
    for key in [
        Key::F1,
        Key::ArrowUp,
        Key::ShiftLeft,
        Key::CtrlRight,
        Key::Escape,
        Key::Pause,
    ] {
        assert_eq!(char_for(key, Shift::new()), None, "{key:?}");
    }
}
