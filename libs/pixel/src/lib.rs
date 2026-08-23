//! Colour and framebuffer address arithmetic.
//!
//! Pure logic with no hardware behind it: this is the layer that decides which
//! byte a pixel lands on and what value goes into it. It lives in its own crate
//! so it can be unit-tested on the host, which is where arithmetic like this
//! actually gets verified — see `docs/005-code-conventions.md`.
//!
//! # Examples
//!
//! ```
//! use pixel::{Channel, PixelFormat, Rgb, offset};
//!
//! // The layout QEMU and VMware both report: eight-bit channels, red high.
//! let format = PixelFormat::new(Channel::new(16, 8), Channel::new(8, 8), Channel::new(0, 8));
//! let red = Rgb { red: 255, green: 0, blue: 0 };
//! assert_eq!(format.pack(red), 0x00ff_0000);
//!
//! // A pixel lands at its row times the pitch, plus its column.
//! assert_eq!(offset(2, 1, 4096, 4), 4104);
//! ```

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
    ///
    /// The scaling is proportional — full intensity lands on full scale at
    /// every width — which is what makes a ten-bit channel reach white instead
    /// of stopping a quarter short of it. Total on every `shift` and `size` a
    /// bootloader could report, including ones no framebuffer has: a channel
    /// that does not fit a 32-bit pixel contributes nothing rather than
    /// shifting past the end of one.
    pub const fn encode(self, intensity: u8) -> u32 {
        let size = self.size as u32;
        let shift = self.shift as u32;
        if size == 0 || shift >= u32::BITS || size > u32::BITS - shift {
            return 0;
        }
        let full_scale = if size == u32::BITS {
            u32::MAX
        } else {
            (1 << size) - 1
        };
        // Both factors are bounded by `u32::MAX`, so this is done in 64 bits.
        let scaled = (intensity as u64 * full_scale as u64 / u8::MAX as u64) as u32;
        scaled << shift
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
    fn narrow_channels_stay_within_their_bits() {
        let packed = rgb565().pack(Rgb::from_hex(0xFF_FF_FF));
        assert_eq!(packed, 0xFFFF);
        assert_eq!(rgb565().pack(Rgb::from_hex(0x00_00_00)), 0);
    }

    #[test]
    fn a_zero_width_channel_contributes_nothing() {
        let format = PixelFormat::new(Channel::new(0, 0), Channel::new(0, 8), Channel::new(8, 8));
        assert_eq!(format.pack(Rgb::from_hex(0xFF_00_00)), 0);
    }

    /// The 30-bit-colour layout real displays report — ten bits a channel in a
    /// 32-bit pixel. Full intensity must reach full scale: scaling by shifting
    /// left instead would stop three quarters of a step short, and white would
    /// come out very slightly grey on hardware nobody tests on.
    #[test]
    fn a_ten_bit_channel_reaches_full_scale() {
        let xrgb2101010 = PixelFormat::new(
            Channel::new(20, 10),
            Channel::new(10, 10),
            Channel::new(0, 10),
        );

        assert_eq!(xrgb2101010.pack(Rgb::from_hex(0xFF_FF_FF)), 0x3FFF_FFFF);
        assert_eq!(xrgb2101010.pack(Rgb::from_hex(0x00_00_00)), 0);
        assert_eq!(
            xrgb2101010.pack(Rgb::from_hex(0xFF_00_00)),
            0x3FF << 20,
            "red alone fills its own ten bits and nothing else"
        );
    }

    /// Every width, from one bit to the whole word: nothing packs to more than
    /// its own bits, black is zero and white is every bit of the channel.
    #[test]
    fn every_channel_width_spans_exactly_its_own_bits() {
        for size in 1..=32u8 {
            let channel = Channel::new(0, size);
            let mask = if size == 32 {
                u32::MAX
            } else {
                (1u32 << size) - 1
            };

            assert_eq!(channel.encode(0), 0, "size {size} black");
            assert_eq!(channel.encode(u8::MAX), mask, "size {size} white");
            for intensity in 0..=u8::MAX {
                assert_eq!(channel.encode(intensity) & !mask, 0, "size {size} spill");
            }
        }
    }

    /// A brighter input can never encode darker, at any width — the property a
    /// scaling bug shows up as, and the one a shift-based version only has by
    /// accident.
    #[test]
    fn encoding_is_monotonic_at_every_width() {
        for size in 1..=16u8 {
            let channel = Channel::new(0, size);
            let mut previous = 0;
            for intensity in 0..=u8::MAX {
                let encoded = channel.encode(intensity);
                assert!(encoded >= previous, "size {size} at {intensity}");
                previous = encoded;
            }
        }
    }

    /// A shifted channel is the unshifted one moved, with nothing lost off
    /// either end — swept over every placement that fits a 32-bit pixel.
    #[test]
    fn shifting_a_channel_only_moves_it() {
        for size in 1..=8u8 {
            for shift in 0..=(32 - size) {
                let placed = Channel::new(shift, size).encode(0xA7);
                let unplaced = Channel::new(0, size).encode(0xA7);

                assert_eq!(placed, unplaced << shift, "size {size} shift {shift}");
            }
        }
    }

    /// Nothing a bootloader can report may shift past the end of a pixel: the
    /// channel drops out instead, which loses a colour rather than panicking in
    /// debug or wrapping in release.
    #[test]
    fn a_channel_that_cannot_fit_the_word_contributes_nothing() {
        for (shift, size) in [(32, 8), (25, 8), (0, 33), (255, 255), (31, 2)] {
            assert_eq!(
                Channel::new(shift, size).encode(u8::MAX),
                0,
                "shift {shift} size {size}"
            );
        }
    }

    /// The boundary the case above sits next to: a channel ending exactly at
    /// bit 31 is legal and must still encode.
    #[test]
    fn a_channel_ending_at_the_top_of_the_word_still_encodes() {
        assert_eq!(Channel::new(24, 8).encode(u8::MAX), 0xFF00_0000);
        assert_eq!(Channel::new(31, 1).encode(u8::MAX), 0x8000_0000);
        assert_eq!(Channel::new(0, 32).encode(u8::MAX), u32::MAX);
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
