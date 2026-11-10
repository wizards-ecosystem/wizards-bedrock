//! Colour and framebuffer address arithmetic, kept off the hardware so the host can test it.
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    #[must_use]
    pub const fn blend(self, other: Self, intensity: u8) -> Self {
        Self {
            red: mix(self.red, other.red, intensity),
            green: mix(self.green, other.green, intensity),
            blue: mix(self.blue, other.blue, intensity),
        }
    }
}

const fn mix(from: u8, to: u8, intensity: u8) -> u8 {
    let (from, to, weight) = (from as u32, to as u32, intensity as u32);
    ((from * (255 - weight) + to * weight) / 255) as u8
}

/// One colour channel's position and width within a packed pixel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Channel {
    shift: u8,
    size: u8,
}

impl Channel {
    /// A channel of `size` bits starting at bit `shift`.
    pub const fn new(shift: u8, size: u8) -> Self {
        Self { shift, size }
    }

    /// Scale an intensity proportionally to this channel's width, or zero if it leaves the pixel.
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
        let scaled = (intensity as u64 * full_scale as u64 / u8::MAX as u64) as u32;
        scaled << shift
    }
}

/// How a framebuffer packs colour channels into a word, as the bootloader reports it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PixelFormat {
    red: Channel,
    green: Channel,
    blue: Channel,
}

/// Which component of a [`PixelFormat`] is malformed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    pub const fn new(
        red: Channel,
        green: Channel,
        blue: Channel,
    ) -> Result<Self, PixelFormatError> {
        let red_mask = match channel_mask(red, ColourChannel::Red) {
            Ok(mask) => mask,
            Err(reason) => return Err(reason),
        };
        let green_mask = match channel_mask(green, ColourChannel::Green) {
            Ok(mask) => mask,
            Err(reason) => return Err(reason),
        };
        let blue_mask = match channel_mask(blue, ColourChannel::Blue) {
            Ok(mask) => mask,
            Err(reason) => return Err(reason),
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

/// Checked geometry for a linear framebuffer of aligned 32-bit pixels whose span cannot wrap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FramebufferLayout {
    width: usize,
    height: usize,
    pitch: usize,
    bytes_per_pixel: usize,
    span: usize,
}

/// Checked visible-row copies within a framebuffer shadow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RowCopyPlan {
    source_word: usize,
    destination_word: usize,
    destination_offset: usize,
    row_words: usize,
    stride_words: usize,
    pitch: usize,
    remaining: usize,
}

/// One checked visible-row copy within a framebuffer shadow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RowCopySpan {
    source_start: usize,
    source_end: usize,
    destination_start: usize,
    destination_end: usize,
    destination_offset: usize,
}

impl RowCopySpan {
    /// The source words, or `None` when `shadow` is shorter than the layout that made this span.
    pub fn source(self, shadow: &[u32]) -> Option<&[u32]> {
        if !self.fits(shadow.len()) {
            return None;
        }
        shadow.get(self.source_start..self.source_end)
    }

    /// Copy this row, or `None` without a change when `shadow` is shorter than its layout.
    pub fn copy_within(self, shadow: &mut [u32]) -> Option<()> {
        if !self.fits(shadow.len()) {
            return None;
        }
        shadow.copy_within(self.source_start..self.source_end, self.destination_start);
        Some(())
    }

    /// Byte offset where the copied row begins in device memory.
    pub const fn destination_offset(self) -> usize {
        self.destination_offset
    }

    const fn fits(self, len: usize) -> bool {
        self.source_end <= len && self.destination_end <= len
    }
}

impl Iterator for RowCopyPlan {
    type Item = RowCopySpan;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let span = RowCopySpan {
            source_start: self.source_word,
            source_end: self.source_word + self.row_words,
            destination_start: self.destination_word,
            destination_end: self.destination_word + self.row_words,
            destination_offset: self.destination_offset,
        };
        self.remaining -= 1;
        if self.remaining != 0 {
            self.source_word += self.stride_words;
            self.destination_word += self.stride_words;
            self.destination_offset += self.pitch;
        }
        Some(span)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for RowCopyPlan {}
impl core::iter::FusedIterator for RowCopyPlan {}

/// Why framebuffer geometry is not safe for 32-bit pixel accesses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

    /// Plan a clipped upward copy of visible rows, or `None` if `to_y` is below `from_y`.
    pub fn copy_rows_up(self, from_y: usize, to_y: usize, rows: usize) -> Option<RowCopyPlan> {
        if to_y > from_y {
            return None;
        }

        let stride_words = self.pitch / WORD_SIZE;
        let rows = rows
            .min(self.height.saturating_sub(from_y))
            .min(self.height.saturating_sub(to_y));
        if rows == 0 {
            return Some(RowCopyPlan {
                source_word: 0,
                destination_word: 0,
                destination_offset: 0,
                row_words: self.width,
                stride_words,
                pitch: self.pitch,
                remaining: 0,
            });
        }

        let source_word = from_y.checked_mul(stride_words)?;
        let destination_word = to_y.checked_mul(stride_words)?;
        let destination_offset = to_y.checked_mul(self.pitch)?;

        Some(RowCopyPlan {
            source_word,
            destination_word,
            destination_offset,
            row_words: self.width,
            stride_words,
            pitch: self.pitch,
            remaining: rows,
        })
    }
}

/// Byte offset of the pixel at (`x`, `y`) with rows `pitch` bytes apart, or `None` on overflow.
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
mod tests;
