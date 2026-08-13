//! The console palette.
//!
//! Every colour the kernel draws is named here, so clearing the screen and
//! rendering a glyph can never disagree about what the background is.

use wost::boot::Rgb;

/// A background and the text drawn on it.
///
/// The pair travels together so the two colours cannot be passed the wrong way
/// round — they are both `Rgb`, and the compiler would not notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    pub(crate) background: Rgb,
    pub(crate) foreground: Rgb,
}

/// Parchment gold on deep midnight blue.
pub(crate) const NORMAL: Palette = Palette {
    background: Rgb::from_hex(0x0B_10_21),
    foreground: Rgb::from_hex(0xE8_D5_A3),
};

/// The same text on dried blood, for the panic screen.
pub(crate) const PANIC: Palette = Palette {
    background: Rgb::from_hex(0x3A_08_0C),
    foreground: NORMAL.foreground,
};
