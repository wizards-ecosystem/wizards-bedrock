use wost::boot::Rgb;

// Paired, because two swapped `Rgb`s would compile.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Palette {
    pub(crate) background: Rgb,
    pub(crate) foreground: Rgb,
}

pub(crate) const NORMAL: Palette = Palette {
    background: Rgb::from_hex(0x23_25_22),
    foreground: Rgb::from_hex(0xF7_F6_F2),
};

pub(crate) const PANIC: Palette = Palette {
    background: Rgb::from_hex(0x5F_1D_29),
    foreground: NORMAL.foreground,
};
