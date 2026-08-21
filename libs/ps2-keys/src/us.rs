//! The US layout: from key identity to character.
//!
//! Layout is policy over the decoder's facts, kept apart from it so a second
//! layout is a sibling module rather than a decoder change. Keys that type
//! nothing — modifiers, navigation, function keys — map to `None`; the
//! keypad maps as its digits, because nothing tracks NumLock yet and the
//! digits are the useful half.

use crate::key::Key;
use crate::shift::Shift;

/// The character `key` types under the US layout, if it types one.
pub fn char_for(key: Key, shift: Shift) -> Option<char> {
    let pick = |plain: char, shifted: char| {
        if shift.is_down() { shifted } else { plain }
    };

    Some(match key {
        Key::A => pick('a', 'A'),
        Key::B => pick('b', 'B'),
        Key::C => pick('c', 'C'),
        Key::D => pick('d', 'D'),
        Key::E => pick('e', 'E'),
        Key::F => pick('f', 'F'),
        Key::G => pick('g', 'G'),
        Key::H => pick('h', 'H'),
        Key::I => pick('i', 'I'),
        Key::J => pick('j', 'J'),
        Key::K => pick('k', 'K'),
        Key::L => pick('l', 'L'),
        Key::M => pick('m', 'M'),
        Key::N => pick('n', 'N'),
        Key::O => pick('o', 'O'),
        Key::P => pick('p', 'P'),
        Key::Q => pick('q', 'Q'),
        Key::R => pick('r', 'R'),
        Key::S => pick('s', 'S'),
        Key::T => pick('t', 'T'),
        Key::U => pick('u', 'U'),
        Key::V => pick('v', 'V'),
        Key::W => pick('w', 'W'),
        Key::X => pick('x', 'X'),
        Key::Y => pick('y', 'Y'),
        Key::Z => pick('z', 'Z'),
        Key::Digit1 => pick('1', '!'),
        Key::Digit2 => pick('2', '@'),
        Key::Digit3 => pick('3', '#'),
        Key::Digit4 => pick('4', '$'),
        Key::Digit5 => pick('5', '%'),
        Key::Digit6 => pick('6', '^'),
        Key::Digit7 => pick('7', '&'),
        Key::Digit8 => pick('8', '*'),
        Key::Digit9 => pick('9', '('),
        Key::Digit0 => pick('0', ')'),
        Key::Minus => pick('-', '_'),
        Key::Equal => pick('=', '+'),
        Key::LeftBracket => pick('[', '{'),
        Key::RightBracket => pick(']', '}'),
        Key::Backslash => pick('\\', '|'),
        Key::Semicolon => pick(';', ':'),
        Key::Apostrophe => pick('\'', '"'),
        Key::Grave => pick('`', '~'),
        Key::Comma => pick(',', '<'),
        Key::Period => pick('.', '>'),
        Key::Slash => pick('/', '?'),
        Key::Space => ' ',
        Key::Enter | Key::KeypadEnter => '\n',
        Key::Tab => '\t',
        Key::Keypad0 => '0',
        Key::Keypad1 => '1',
        Key::Keypad2 => '2',
        Key::Keypad3 => '3',
        Key::Keypad4 => '4',
        Key::Keypad5 => '5',
        Key::Keypad6 => '6',
        Key::Keypad7 => '7',
        Key::Keypad8 => '8',
        Key::Keypad9 => '9',
        Key::KeypadPeriod => '.',
        Key::KeypadPlus => '+',
        Key::KeypadMinus => '-',
        Key::KeypadMultiply => '*',
        Key::KeypadDivide => '/',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
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
        assert_eq!(char_for(Key::Digit1, Shift::new()), Some('1'));
        assert_eq!(char_for(Key::Digit1, held()), Some('!'));
        assert_eq!(char_for(Key::Digit0, held()), Some(')'));
    }

    #[test]
    fn punctuation_shifts() {
        assert_eq!(char_for(Key::Semicolon, held()), Some(':'));
        assert_eq!(char_for(Key::Apostrophe, held()), Some('"'));
        assert_eq!(char_for(Key::Slash, held()), Some('?'));
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
        assert_eq!(char_for(Key::Keypad7, Shift::new()), Some('7'));
        assert_eq!(char_for(Key::Keypad7, held()), Some('7'));
        assert_eq!(char_for(Key::KeypadDivide, Shift::new()), Some('/'));
    }

    #[test]
    fn a_key_that_types_nothing_maps_to_nothing() {
        for key in [
            Key::F1,
            Key::ArrowUp,
            Key::ShiftLeft,
            Key::CtrlRight,
            Key::Escape,
            Key::Backspace,
            Key::Pause,
        ] {
            assert_eq!(char_for(key, Shift::new()), None, "{key:?}");
        }
    }
}
