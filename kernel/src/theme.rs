//! The console palette: every colour the kernel draws is named here, so clearing the screen and
//! rendering a glyph can never disagree about what the background is.

use wost::boot::Rgb;

/// A background and the text drawn on it, paired because two swapped `Rgb`s would compile.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Palette {
    pub(crate) background: Rgb,
    pub(crate) foreground: Rgb,
}

/// Soft white on charcoal, matching the product identity.
pub(crate) const NORMAL: Palette = Palette {
    background: Rgb::from_hex(0x23_25_22),
    foreground: Rgb::from_hex(0xF7_F6_F2),
};

/// The same text on oxblood, for the panic screen.
pub(crate) const PANIC: Palette = Palette {
    background: Rgb::from_hex(0x5F_1D_29),
    foreground: NORMAL.foreground,
};
