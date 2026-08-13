//! The console palette.
//!
//! Every colour the kernel draws is named here, so clearing the screen and
//! rendering a glyph can never disagree about what the background is.

use wost::boot::Rgb;

/// Deep midnight blue.
pub(crate) const BACKGROUND: Rgb = Rgb::from_hex(0x0B_10_21);

/// Parchment gold.
pub(crate) const FOREGROUND: Rgb = Rgb::from_hex(0xE8_D5_A3);

/// Dried blood, for the panic screen.
pub(crate) const PANIC_BACKGROUND: Rgb = Rgb::from_hex(0x3A_08_0C);
