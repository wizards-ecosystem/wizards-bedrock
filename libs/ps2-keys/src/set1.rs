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
mod tests;
