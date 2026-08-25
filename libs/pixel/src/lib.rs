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
//! use pixel::{Channel, FramebufferLayout, PixelFormat, Rgb, offset};
//!
//! // The layout QEMU and VMware both report: eight-bit channels, red high.
//! let format = PixelFormat::new(Channel::new(16, 8), Channel::new(8, 8), Channel::new(0, 8))
//!     .expect("valid channel masks");
//! let red = Rgb { red: 255, green: 0, blue: 0 };
//! assert_eq!(format.pack(red), 0x00ff_0000);
//!
//! // A pixel lands at its row times the pitch, plus its column.
//! assert_eq!(offset(2, 1, 4096, 4), Some(4104));
//!
//! let layout = FramebufferLayout::new(0x1000, 1280, 720, 5120, 4).unwrap();
//! assert_eq!(layout.span(), 3_686_400);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

/// Bytes in the framebuffer access width.
const WORD_SIZE: usize = size_of::<u32>();

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

/// Which component of a [`PixelFormat`] is malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourChannel {
    /// The red component.
    Red,
    /// The green component.
    Green,
    /// The blue component.
    Blue,
}

impl fmt::Display for ColourChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Red => f.write_str("red"),
            Self::Green => f.write_str("green"),
            Self::Blue => f.write_str("blue"),
        }
    }
}

/// Why colour-channel masks cannot form a 32-bit pixel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PixelFormatError {
    /// A colour contributes no bits.
    EmptyChannel(ColourChannel),
    /// A channel reaches past bit 31.
    ChannelOutOfRange {
        /// The malformed component.
        channel: ColourChannel,
        /// Its lowest bit.
        shift: u8,
        /// Bits it occupies.
        size: u8,
    },
    /// Two colour components claim at least one of the same bits.
    OverlappingChannels {
        /// One overlapping component.
        first: ColourChannel,
        /// The other overlapping component.
        second: ColourChannel,
    },
}

impl fmt::Display for PixelFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::EmptyChannel(channel) => write!(f, "{channel} channel has no bits"),
            Self::ChannelOutOfRange {
                channel,
                shift,
                size,
            } => write!(
                f,
                "{channel} channel at bit {shift} with width {size} leaves a 32-bit pixel"
            ),
            Self::OverlappingChannels { first, second } => {
                write!(f, "{first} and {second} channels overlap")
            }
        }
    }
}

impl core::error::Error for PixelFormatError {}

impl PixelFormat {
    /// A format with three non-empty, in-range, disjoint channel placements.
    ///
    /// # Errors
    ///
    /// [`PixelFormatError::EmptyChannel`] if any channel has no bits,
    /// [`PixelFormatError::ChannelOutOfRange`] if one does not fit in a `u32`,
    /// or [`PixelFormatError::OverlappingChannels`] if two share a bit.
    pub const fn new(
        red: Channel,
        green: Channel,
        blue: Channel,
    ) -> Result<Self, PixelFormatError> {
        let red_mask = match channel_mask(red, ColourChannel::Red) {
            Ok(mask) => mask,
            Err(error) => return Err(error),
        };
        let green_mask = match channel_mask(green, ColourChannel::Green) {
            Ok(mask) => mask,
            Err(error) => return Err(error),
        };
        let blue_mask = match channel_mask(blue, ColourChannel::Blue) {
            Ok(mask) => mask,
            Err(error) => return Err(error),
        };
        if red_mask & green_mask != 0 {
            return Err(PixelFormatError::OverlappingChannels {
                first: ColourChannel::Red,
                second: ColourChannel::Green,
            });
        }
        if red_mask & blue_mask != 0 {
            return Err(PixelFormatError::OverlappingChannels {
                first: ColourChannel::Red,
                second: ColourChannel::Blue,
            });
        }
        if green_mask & blue_mask != 0 {
            return Err(PixelFormatError::OverlappingChannels {
                first: ColourChannel::Green,
                second: ColourChannel::Blue,
            });
        }
        Ok(Self { red, green, blue })
    }

    /// Pack a colour into a framebuffer word.
    pub const fn pack(self, colour: Rgb) -> u32 {
        self.red.encode(colour.red)
            | self.green.encode(colour.green)
            | self.blue.encode(colour.blue)
    }
}

const fn channel_mask(channel: Channel, name: ColourChannel) -> Result<u32, PixelFormatError> {
    let size = channel.size as u32;
    let shift = channel.shift as u32;
    if size == 0 {
        return Err(PixelFormatError::EmptyChannel(name));
    }
    if shift >= u32::BITS || size > u32::BITS - shift {
        return Err(PixelFormatError::ChannelOutOfRange {
            channel: name,
            shift: channel.shift,
            size: channel.size,
        });
    }
    let bits = if size == u32::BITS {
        u32::MAX
    } else {
        (1 << size) - 1
    };
    Ok(bits << shift)
}

/// Checked geometry for a linear framebuffer accessed as 32-bit pixels.
///
/// Construction proves that every visible pixel begins at a 4-byte-aligned
/// offset, every row fits within its pitch, and the complete mapping span can
/// be represented without wrapping either its length or its address range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramebufferLayout {
    width: usize,
    height: usize,
    pitch: usize,
    bytes_per_pixel: usize,
    span: usize,
}

/// Why framebuffer geometry is not safe for 32-bit pixel accesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FramebufferLayoutError {
    /// A framebuffer must expose at least one pixel in each row.
    ZeroWidth,
    /// A framebuffer must expose at least one row.
    ZeroHeight,
    /// A pixel must occupy at least one byte.
    ZeroBytesPerPixel,
    /// This framebuffer implementation accesses exactly one `u32` per pixel.
    UnsupportedBytesPerPixel(usize),
    /// The visible bytes in a row do not fit in `usize`.
    RowBytesOverflow,
    /// The pitch ends before the visible pixels in a row do.
    PitchTooSmall {
        /// Bytes the visible pixels require.
        row_bytes: usize,
        /// Bytes the framebuffer provides per row.
        pitch: usize,
    },
    /// The mapping base is not aligned for a `u32` access.
    MisalignedBase(usize),
    /// A row after the first would not begin on a `u32` boundary.
    MisalignedPitch(usize),
    /// The complete framebuffer span does not fit in `usize`.
    SpanOverflow,
    /// Adding the span to the mapping base would wrap the address space.
    AddressRangeOverflow,
}

impl fmt::Display for FramebufferLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ZeroWidth => f.write_str("framebuffer width is zero"),
            Self::ZeroHeight => f.write_str("framebuffer height is zero"),
            Self::ZeroBytesPerPixel => f.write_str("framebuffer bytes per pixel is zero"),
            Self::UnsupportedBytesPerPixel(bytes) => {
                write!(
                    f,
                    "framebuffer uses {bytes} bytes per pixel, expected {WORD_SIZE}"
                )
            }
            Self::RowBytesOverflow => f.write_str("visible row byte count overflows"),
            Self::PitchTooSmall { row_bytes, pitch } => {
                write!(f, "row needs {row_bytes} bytes but pitch is {pitch}")
            }
            Self::MisalignedBase(address) => {
                write!(
                    f,
                    "framebuffer base {address:#x} is not {WORD_SIZE}-byte aligned"
                )
            }
            Self::MisalignedPitch(pitch) => {
                write!(
                    f,
                    "framebuffer pitch {pitch} is not {WORD_SIZE}-byte aligned"
                )
            }
            Self::SpanOverflow => f.write_str("framebuffer span overflows"),
            Self::AddressRangeOverflow => f.write_str("framebuffer address range overflows"),
        }
    }
}

impl core::error::Error for FramebufferLayoutError {}

impl FramebufferLayout {
    /// Validate geometry for a framebuffer mapping starting at `base_address`.
    ///
    /// # Errors
    ///
    /// Returns a [`FramebufferLayoutError`] if a dimension is zero, pixels are
    /// not 32 bits wide, visible rows exceed the pitch, any required
    /// arithmetic wraps, or the mapping and its rows are not 4-byte aligned.
    pub const fn new(
        base_address: usize,
        width: usize,
        height: usize,
        pitch: usize,
        bytes_per_pixel: usize,
    ) -> Result<Self, FramebufferLayoutError> {
        if width == 0 {
            return Err(FramebufferLayoutError::ZeroWidth);
        }
        if height == 0 {
            return Err(FramebufferLayoutError::ZeroHeight);
        }
        if bytes_per_pixel == 0 {
            return Err(FramebufferLayoutError::ZeroBytesPerPixel);
        }
        if bytes_per_pixel != WORD_SIZE {
            return Err(FramebufferLayoutError::UnsupportedBytesPerPixel(
                bytes_per_pixel,
            ));
        }
        let Some(row_bytes) = width.checked_mul(bytes_per_pixel) else {
            return Err(FramebufferLayoutError::RowBytesOverflow);
        };
        if row_bytes > pitch {
            return Err(FramebufferLayoutError::PitchTooSmall { row_bytes, pitch });
        }
        if !base_address.is_multiple_of(WORD_SIZE) {
            return Err(FramebufferLayoutError::MisalignedBase(base_address));
        }
        if !pitch.is_multiple_of(WORD_SIZE) {
            return Err(FramebufferLayoutError::MisalignedPitch(pitch));
        }
        let Some(span) = pitch.checked_mul(height) else {
            return Err(FramebufferLayoutError::SpanOverflow);
        };
        if base_address.checked_add(span).is_none() {
            return Err(FramebufferLayoutError::AddressRangeOverflow);
        }
        Ok(Self {
            width,
            height,
            pitch,
            bytes_per_pixel,
            span,
        })
    }

    /// Visible pixels in one row.
    pub const fn width(self) -> usize {
        self.width
    }

    /// Visible rows in the framebuffer.
    pub const fn height(self) -> usize {
        self.height
    }

    /// Bytes from the start of one row to the next.
    pub const fn pitch(self) -> usize {
        self.pitch
    }

    /// Bytes occupied by one pixel.
    pub const fn bytes_per_pixel(self) -> usize {
        self.bytes_per_pixel
    }

    /// Bytes covered by the complete framebuffer mapping.
    pub const fn span(self) -> usize {
        self.span
    }

    /// Byte offset of a visible pixel, or `None` outside the visible area.
    pub const fn offset(self, x: usize, y: usize) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        offset(x, y, self.pitch, self.bytes_per_pixel)
    }
}

/// Byte offset of the pixel at (`x`, `y`), or `None` if it does not fit.
///
/// `pitch` is the distance between the starts of consecutive rows, which is not
/// always `width * bytes_per_pixel`.
pub const fn offset(x: usize, y: usize, pitch: usize, bytes_per_pixel: usize) -> Option<usize> {
    let Some(row) = y.checked_mul(pitch) else {
        return None;
    };
    let Some(column) = x.checked_mul(bytes_per_pixel) else {
        return None;
    };
    row.checked_add(column)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout QEMU and VMware both report: 8-bit channels, R high.
    fn rgb888() -> PixelFormat {
        PixelFormat::new(Channel::new(16, 8), Channel::new(8, 8), Channel::new(0, 8))
            .expect("valid RGB888")
    }

    /// The same depth with the channels the other way round.
    fn bgr888() -> PixelFormat {
        PixelFormat::new(Channel::new(0, 8), Channel::new(8, 8), Channel::new(16, 8))
            .expect("valid BGR888")
    }

    fn rgb565() -> PixelFormat {
        PixelFormat::new(Channel::new(11, 5), Channel::new(5, 6), Channel::new(0, 5))
            .expect("valid RGB565")
    }

    fn layout() -> FramebufferLayout {
        FramebufferLayout::new(0x1000, 1280, 720, 5120, 4).expect("valid layout")
    }

    /// A padded framebuffer spans its **pitch**, not its visible width.
    ///
    /// Every other test here uses geometry where `pitch == width *
    /// bytes_per_pixel`, so the two are interchangeable and
    /// `row_bytes.checked_mul(height)` survives. On any framebuffer whose
    /// rows are padded — which is most real hardware, and what `pitch` exists
    /// to express — that maps short by `(pitch - row_bytes) * height` bytes,
    /// and the bottom of the screen is off the end of the mapping (D-155).
    #[test]
    fn a_padded_framebuffer_spans_its_pitch() {
        const WIDTH: usize = 1024;
        const HEIGHT: usize = 768;
        const BYTES_PER_PIXEL: usize = 4;
        /// Deliberately more than `WIDTH * BYTES_PER_PIXEL`, which is 4096.
        const PITCH: usize = 4352;

        let layout = FramebufferLayout::new(0x1000, WIDTH, HEIGHT, PITCH, BYTES_PER_PIXEL)
            .expect("padded geometry is valid");

        assert_eq!(layout.span(), PITCH * HEIGHT, "the span is the pitch's");
        assert_ne!(
            layout.span(),
            WIDTH * BYTES_PER_PIXEL * HEIGHT,
            "the two are distinguishable here, which is the point"
        );

        // And the last visible pixel is inside that span — the property the
        // span exists to guarantee.
        let last = layout
            .offset(WIDTH - 1, HEIGHT - 1)
            .expect("the bottom-right pixel is visible");
        assert!(last + BYTES_PER_PIXEL <= layout.span());
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
        assert_eq!(Channel::new(0, 0).encode(u8::MAX), 0);
    }

    #[test]
    fn every_pixel_format_channel_must_be_nonempty() {
        for (red, green, blue, empty) in [
            (
                Channel::new(16, 0),
                Channel::new(8, 8),
                Channel::new(0, 8),
                ColourChannel::Red,
            ),
            (
                Channel::new(16, 8),
                Channel::new(8, 0),
                Channel::new(0, 8),
                ColourChannel::Green,
            ),
            (
                Channel::new(16, 8),
                Channel::new(8, 8),
                Channel::new(0, 0),
                ColourChannel::Blue,
            ),
        ] {
            assert_eq!(
                PixelFormat::new(red, green, blue),
                Err(PixelFormatError::EmptyChannel(empty))
            );
        }
    }

    #[test]
    fn every_pixel_format_channel_must_fit_the_word() {
        for (red, green, blue, channel) in [
            (
                Channel::new(31, 2),
                Channel::new(8, 8),
                Channel::new(0, 8),
                ColourChannel::Red,
            ),
            (
                Channel::new(16, 8),
                Channel::new(31, 2),
                Channel::new(0, 8),
                ColourChannel::Green,
            ),
            (
                Channel::new(16, 8),
                Channel::new(8, 8),
                Channel::new(31, 2),
                ColourChannel::Blue,
            ),
        ] {
            assert_eq!(
                PixelFormat::new(red, green, blue),
                Err(PixelFormatError::ChannelOutOfRange {
                    channel,
                    shift: 31,
                    size: 2,
                })
            );
        }
    }

    #[test]
    fn every_pixel_format_channel_pair_must_be_disjoint() {
        for (red, green, blue, first, second) in [
            (
                Channel::new(16, 8),
                Channel::new(16, 8),
                Channel::new(0, 8),
                ColourChannel::Red,
                ColourChannel::Green,
            ),
            (
                Channel::new(16, 8),
                Channel::new(8, 8),
                Channel::new(16, 8),
                ColourChannel::Red,
                ColourChannel::Blue,
            ),
            (
                Channel::new(16, 8),
                Channel::new(8, 8),
                Channel::new(8, 8),
                ColourChannel::Green,
                ColourChannel::Blue,
            ),
        ] {
            assert_eq!(
                PixelFormat::new(red, green, blue),
                Err(PixelFormatError::OverlappingChannels { first, second })
            );
        }
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
        )
        .expect("valid 30-bit format");

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
    fn checked_layout_preserves_geometry_and_bounds_offsets() {
        let layout = layout();
        assert_eq!(layout.width(), 1280);
        assert_eq!(layout.height(), 720);
        assert_eq!(layout.pitch(), 5120);
        assert_eq!(layout.bytes_per_pixel(), 4);
        assert_eq!(layout.span(), 5120 * 720);
        assert_eq!(layout.offset(1279, 719), Some(719 * 5120 + 1279 * 4));
        assert_eq!(layout.offset(1280, 0), None);
        assert_eq!(layout.offset(0, 720), None);
    }

    #[test]
    fn a_zero_width_is_refused() {
        assert_eq!(
            FramebufferLayout::new(0, 0, 1, 4, 4),
            Err(FramebufferLayoutError::ZeroWidth)
        );
    }

    #[test]
    fn a_zero_height_is_refused() {
        assert_eq!(
            FramebufferLayout::new(0, 1, 0, 4, 4),
            Err(FramebufferLayoutError::ZeroHeight)
        );
    }

    #[test]
    fn a_zero_pixel_size_is_refused() {
        assert_eq!(
            FramebufferLayout::new(0, 1, 1, 4, 0),
            Err(FramebufferLayoutError::ZeroBytesPerPixel)
        );
    }

    #[test]
    fn a_non_u32_pixel_size_is_refused() {
        assert_eq!(
            FramebufferLayout::new(0, 1, 1, 8, 8),
            Err(FramebufferLayoutError::UnsupportedBytesPerPixel(8))
        );
    }

    #[test]
    fn a_visible_row_that_overflows_is_refused() {
        assert_eq!(
            FramebufferLayout::new(0, usize::MAX, 1, usize::MAX, 4),
            Err(FramebufferLayoutError::RowBytesOverflow)
        );
    }

    #[test]
    fn a_pitch_shorter_than_the_visible_row_is_refused() {
        assert_eq!(
            FramebufferLayout::new(0, 2, 1, 4, 4),
            Err(FramebufferLayoutError::PitchTooSmall {
                row_bytes: 8,
                pitch: 4
            })
        );
    }

    /// The modulus is four, not merely even.
    ///
    /// The witnesses were both odd, so the checks passed identically as
    /// `is_multiple_of(2)` — under which a base of 2 constructs and every row
    /// after the first starts two bytes out of alignment. `write_u32` then
    /// refuses each one, and `wost/src/boot/mod.rs` discards that result: a
    /// blank framebuffer with no diagnostic, on the only output a kernel has
    /// (D-105).
    #[test]
    fn a_misaligned_mapping_base_is_refused() {
        assert_eq!(
            FramebufferLayout::new(1, 1, 1, 4, 4),
            Err(FramebufferLayoutError::MisalignedBase(1))
        );
        // Even, and still not a multiple of four.
        assert_eq!(
            FramebufferLayout::new(2, 1, 1, 4, 4),
            Err(FramebufferLayoutError::MisalignedBase(2))
        );
        assert!(FramebufferLayout::new(4, 1, 1, 4, 4).is_ok());
    }

    #[test]
    fn a_misaligned_pitch_is_refused() {
        assert_eq!(
            FramebufferLayout::new(0, 1, 1, 5, 4),
            Err(FramebufferLayoutError::MisalignedPitch(5))
        );
        // Even, and still not a multiple of four.
        assert_eq!(
            FramebufferLayout::new(0, 1, 1, 6, 4),
            Err(FramebufferLayoutError::MisalignedPitch(6))
        );
        assert!(FramebufferLayout::new(0, 1, 1, 8, 4).is_ok());
    }

    #[test]
    fn a_framebuffer_span_that_overflows_is_refused() {
        assert_eq!(
            FramebufferLayout::new(0, 1, usize::MAX, 8, 4),
            Err(FramebufferLayoutError::SpanOverflow)
        );
    }

    #[test]
    fn a_framebuffer_address_range_that_wraps_is_refused() {
        assert_eq!(
            FramebufferLayout::new(usize::MAX - 3, 1, 1, 4, 4),
            Err(FramebufferLayoutError::AddressRangeOverflow)
        );
    }

    #[test]
    fn offsets_follow_the_pitch_not_the_width() {
        // 1280 visible pixels but a 5120-byte pitch: row 1 starts at 5120.
        assert_eq!(offset(0, 0, 5120, 4), Some(0));
        assert_eq!(offset(1, 0, 5120, 4), Some(4));
        assert_eq!(offset(0, 1, 5120, 4), Some(5120));
        assert_eq!(offset(2, 3, 5120, 4), Some(3 * 5120 + 8));
    }

    #[test]
    fn offsets_respect_a_padded_pitch() {
        // A 100-pixel row padded to 512 bytes rather than 400.
        assert_eq!(offset(0, 1, 512, 4), Some(512));
        assert_ne!(offset(0, 1, 512, 4), offset(100, 0, 512, 4));
    }

    #[test]
    fn an_offset_with_an_overflowing_column_is_refused() {
        assert_eq!(offset(usize::MAX, 0, 0, 2), None, "column product");
    }

    #[test]
    fn an_offset_with_an_overflowing_row_is_refused() {
        assert_eq!(offset(0, usize::MAX, 2, 0), None, "row product");
    }

    #[test]
    fn an_offset_with_an_overflowing_sum_is_refused() {
        assert_eq!(offset(1, 1, usize::MAX - 1, 2), None, "row plus column");
    }

    /// A channel that exactly fills the word is accepted, and one bit more is
    /// refused — at *both* copies of the bound.
    ///
    /// `channel_mask` duplicates `encode`'s `size > u32::BITS - shift`, and
    /// only `encode`'s copy had the boundary pair. Widening `channel_mask`'s
    /// to `>=` left every test green while refusing a legal RGBx8888 report —
    /// a kernel with no console, gate green (D-105).
    #[test]
    fn a_channel_that_exactly_fills_the_word_is_accepted_by_both_copies() {
        // 24 + 8 == 32: the top channel of a standard RGBx8888 report.
        let top = Channel::new(24, 8);
        assert_ne!(top.encode(0xFF), 0, "encode refused a channel that fits");

        let format = PixelFormat::new(Channel::new(0, 8), Channel::new(8, 8), top);
        assert!(format.is_ok(), "channel_mask refused a channel that fits");

        // And one bit past the end is refused by both.
        let over = Channel::new(25, 8);
        assert_eq!(
            over.encode(0xFF),
            0,
            "encode accepted a channel that does not fit"
        );
        assert!(
            PixelFormat::new(Channel::new(0, 8), Channel::new(8, 8), over).is_err(),
            "channel_mask accepted a channel that does not fit"
        );
    }

    /// Blending really reads both operands.
    ///
    /// The endpoints `0x000000` and `0xFFFFFF` make `mix(0, 255, t) == t`, so
    /// the monotonicity sweep asserted `t >= t - 1` and multiplied the `from`
    /// operand by literal zero every iteration — it passed with `from` dropped
    /// from `mix` entirely (D-105).
    #[test]
    fn blending_reads_the_background_as_well_as_the_foreground() {
        let from = Rgb::from_hex(0x4080C0);
        let to = Rgb::from_hex(0xC04000);

        assert_eq!(from.blend(to, 0), from, "t = 0 is the background");
        assert_eq!(from.blend(to, 255), to, "t = 255 is the foreground");

        // Halfway depends on both: a `blend` that ignored `from` would give
        // 96 / 32 / 0 instead of 128 / 95 / 95.
        let middle = from.blend(to, 128);
        assert_eq!(middle.red, mix(0x40, 0xC0, 128));
        assert_eq!(middle.red, 0x80);
        assert_eq!(middle.green, 95);
        assert_eq!(middle.blue, 95);
    }
}
