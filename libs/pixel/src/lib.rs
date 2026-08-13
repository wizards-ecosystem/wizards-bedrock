//! Colour and framebuffer address arithmetic.
//!
//! Pure logic with no hardware behind it: this is the layer that decides which
//! byte a pixel lands on and what value goes into it. It lives in its own crate
//! so it can be unit-tested on the host, which is where arithmetic like this
//! actually gets verified — see `docs/005-code-conventions.md`.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// An 8-bit-per-channel colour, independent of any framebuffer's packing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    /// Red intensity.
    pub red: u8,
    /// Green intensity.
    pub green: u8,
    /// Blue intensity.
    pub blue: u8,
}

impl Rgb {
    /// Build a colour from a `0xRRGGBB` literal.
    pub const fn from_hex(hex: u32) -> Self {
        Self {
            red: (hex >> 16) as u8,
            green: (hex >> 8) as u8,
            blue: hex as u8,
        }
    }

    /// Linear blend from `self` towards `other` by `intensity / 255`.
    pub const fn blend(self, other: Self, intensity: u8) -> Self {
        Self {
            red: mix(self.red, other.red, intensity),
            green: mix(self.green, other.green, intensity),
            blue: mix(self.blue, other.blue, intensity),
        }
    }
}

const fn mix(from: u8, to: u8, intensity: u8) -> u8 {
    let (from, to, t) = (from as u32, to as u32, intensity as u32);
    ((from * (255 - t) + to * t) / 255) as u8
}

/// One colour channel's position and width within a packed pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Channel {
    shift: u8,
    size: u8,
}

impl Channel {
    /// A channel of `size` bits starting at bit `shift`.
    pub const fn new(shift: u8, size: u8) -> Self {
        Self { shift, size }
    }

    /// Scale an 8-bit intensity to this channel's width and shift it into place.
    pub const fn encode(self, intensity: u8) -> u32 {
        if self.size == 0 {
            return 0;
        }
        let intensity = intensity as u32;
        let scaled = if self.size >= 8 {
            intensity
        } else {
            intensity >> (8 - self.size)
        };
        scaled << self.shift
    }

    /// Recover an 8-bit intensity from a packed word.
    ///
    /// Exact for 8-bit channels; for narrower ones the low bits are lost, which
    /// is why a scrolled console can only be as faithful as its display depth.
    pub const fn decode(self, word: u32) -> u8 {
        if self.size == 0 {
            return 0;
        }
        let mask = if self.size >= 32 {
            u32::MAX
        } else {
            (1 << self.size) - 1
        };
        let raw = (word >> self.shift) & mask;
        if self.size >= 8 {
            raw as u8
        } else {
            (raw << (8 - self.size)) as u8
        }
    }
}

/// How a framebuffer packs colour channels into a word.
///
/// Reading this from the bootloader rather than assuming `0xRRGGBB` is what
/// makes the console correct on a BGR or reduced-depth display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFormat {
    red: Channel,
    green: Channel,
    blue: Channel,
}

impl PixelFormat {
    /// A format with the given channel placements.
    pub const fn new(red: Channel, green: Channel, blue: Channel) -> Self {
        Self { red, green, blue }
    }

    /// Pack a colour into a framebuffer word.
    pub const fn pack(self, colour: Rgb) -> u32 {
        self.red.encode(colour.red)
            | self.green.encode(colour.green)
            | self.blue.encode(colour.blue)
    }

    /// Recover a colour from a framebuffer word.
    pub const fn unpack(self, word: u32) -> Rgb {
        Rgb {
            red: self.red.decode(word),
            green: self.green.decode(word),
            blue: self.blue.decode(word),
        }
    }
}

/// Byte offset of the pixel at (`x`, `y`).
///
/// `pitch` is the distance between the starts of consecutive rows, which is not
/// always `width * bytes_per_pixel`.
pub const fn offset(x: usize, y: usize, pitch: usize, bytes_per_pixel: usize) -> usize {
    y * pitch + x * bytes_per_pixel
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout QEMU and VMware both report: 8-bit channels, R high.
    fn rgb888() -> PixelFormat {
        PixelFormat::new(Channel::new(16, 8), Channel::new(8, 8), Channel::new(0, 8))
    }

    /// The same depth with the channels the other way round.
    fn bgr888() -> PixelFormat {
        PixelFormat::new(Channel::new(0, 8), Channel::new(8, 8), Channel::new(16, 8))
    }

    fn rgb565() -> PixelFormat {
        PixelFormat::new(Channel::new(11, 5), Channel::new(5, 6), Channel::new(0, 5))
    }

    #[test]
    fn from_hex_splits_the_channels() {
        assert_eq!(
            Rgb::from_hex(0x0B_10_21),
            Rgb {
                red: 0x0B,
                green: 0x10,
                blue: 0x21
            }
        );
    }

    #[test]
    fn packing_rgb888_is_the_identity_on_a_hex_literal() {
        // The property the console relies on: on the framebuffers we actually
        // boot against, a 0xRRGGBB theme colour survives packing untouched.
        for hex in [0x00_00_00, 0x0B_10_21, 0xE8_D5_A3, 0xFF_FF_FF] {
            assert_eq!(rgb888().pack(Rgb::from_hex(hex)), hex);
        }
    }

    #[test]
    fn packing_honours_channel_order() {
        let colour = Rgb::from_hex(0xAA_BB_CC);
        assert_eq!(rgb888().pack(colour), 0xAA_BB_CC);
        assert_eq!(bgr888().pack(colour), 0xCC_BB_AA);
    }

    #[test]
    fn eight_bit_channels_round_trip_exactly() {
        for format in [rgb888(), bgr888()] {
            for hex in [0x00_00_00, 0x0B_10_21, 0xE8_D5_A3, 0xFF_FF_FF] {
                let colour = Rgb::from_hex(hex);
                assert_eq!(format.unpack(format.pack(colour)), colour);
            }
        }
    }

    #[test]
    fn narrow_channels_stay_within_their_bits() {
        let packed = rgb565().pack(Rgb::from_hex(0xFF_FF_FF));
        assert_eq!(packed, 0xFFFF);
        assert_eq!(rgb565().pack(Rgb::from_hex(0x00_00_00)), 0);
    }

    #[test]
    fn a_zero_width_channel_contributes_nothing() {
        let format = PixelFormat::new(Channel::new(0, 0), Channel::new(0, 8), Channel::new(8, 8));
        assert_eq!(format.pack(Rgb::from_hex(0xFF_00_00)), 0);
        assert_eq!(format.unpack(u32::MAX).red, 0);
    }

    #[test]
    fn blending_lands_on_the_endpoints() {
        let background = Rgb::from_hex(0x0B_10_21);
        let foreground = Rgb::from_hex(0xE8_D5_A3);
        assert_eq!(background.blend(foreground, 0), background);
        assert_eq!(background.blend(foreground, 255), foreground);
    }

    #[test]
    fn blending_moves_monotonically_towards_the_foreground() {
        let background = Rgb::from_hex(0x00_00_00);
        let foreground = Rgb::from_hex(0xFF_FF_FF);
        let mut previous = 0;
        for intensity in 0..=255u8 {
            let red = background.blend(foreground, intensity).red;
            assert!(red >= previous, "intensity {intensity} went backwards");
            previous = red;
        }
        assert_eq!(previous, 255);
    }

    #[test]
    fn offsets_follow_the_pitch_not_the_width() {
        // 1280 visible pixels but a 5120-byte pitch: row 1 starts at 5120.
        assert_eq!(offset(0, 0, 5120, 4), 0);
        assert_eq!(offset(1, 0, 5120, 4), 4);
        assert_eq!(offset(0, 1, 5120, 4), 5120);
        assert_eq!(offset(2, 3, 5120, 4), 3 * 5120 + 8);
    }

    #[test]
    fn offsets_respect_a_padded_pitch() {
        // A 100-pixel row padded to 512 bytes rather than 400.
        assert_eq!(offset(0, 1, 512, 4), 512);
        assert_ne!(offset(0, 1, 512, 4), offset(100, 0, 512, 4));
    }
}
