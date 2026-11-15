#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::fmt;

const WORD_SIZE: usize = size_of::<u32>();

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb {
    pub const fn from_hex(hex: u32) -> Self {
        Self {
            red: (hex >> 16) as u8,
            green: (hex >> 8) as u8,
            blue: hex as u8,
        }
    }

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Channel {
    shift: u8,
    size: u8,
}

impl Channel {
    pub const fn new(shift: u8, size: u8) -> Self {
        Self { shift, size }
    }

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PixelFormat {
    red: Channel,
    green: Channel,
    blue: Channel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColourChannel {
    Red,
    Green,
    Blue,
}

impl fmt::Display for ColourChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Red => formatter.write_str("red"),
            Self::Green => formatter.write_str("green"),
            Self::Blue => formatter.write_str("blue"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PixelFormatError {
    EmptyChannel(ColourChannel),
    ChannelOutOfRange {
        channel: ColourChannel,
        shift: u8,
        size: u8,
    },
    OverlappingChannels {
        first: ColourChannel,
        second: ColourChannel,
    },
}

impl fmt::Display for PixelFormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::EmptyChannel(channel) => write!(formatter, "{channel} channel has no bits"),
            Self::ChannelOutOfRange {
                channel,
                shift,
                size,
            } => write!(
                formatter,
                "{channel} channel at bit {shift} with width {size} leaves a 32-bit pixel"
            ),
            Self::OverlappingChannels { first, second } => {
                write!(formatter, "{first} and {second} channels overlap")
            }
        }
    }
}

impl core::error::Error for PixelFormatError {}

impl PixelFormat {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FramebufferLayout {
    width: usize,
    height: usize,
    pitch: usize,
    bytes_per_pixel: usize,
    span: usize,
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RowCopySpan {
    source_start: usize,
    source_end: usize,
    destination_start: usize,
    destination_end: usize,
    destination_offset: usize,
}

impl RowCopySpan {
    pub fn source(self, shadow: &[u32]) -> Option<&[u32]> {
        if !self.fits(shadow.len()) {
            return None;
        }
        shadow.get(self.source_start..self.source_end)
    }

    pub fn copy_within(self, shadow: &mut [u32]) -> Option<()> {
        if !self.fits(shadow.len()) {
            return None;
        }
        shadow.copy_within(self.source_start..self.source_end, self.destination_start);
        Some(())
    }

    pub const fn destination_offset(self) -> usize {
        self.destination_offset
    }

    const fn fits(self, length: usize) -> bool {
        self.source_end <= length && self.destination_end <= length
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FramebufferLayoutError {
    ZeroWidth,
    ZeroHeight,
    ZeroBytesPerPixel,
    UnsupportedBytesPerPixel(usize),
    RowBytesOverflow,
    PitchTooSmall { row_bytes: usize, pitch: usize },
    MisalignedBase(usize),
    MisalignedPitch(usize),
    SpanOverflow,
    AddressRangeOverflow,
}

impl fmt::Display for FramebufferLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ZeroWidth => formatter.write_str("framebuffer width is zero"),
            Self::ZeroHeight => formatter.write_str("framebuffer height is zero"),
            Self::ZeroBytesPerPixel => formatter.write_str("framebuffer bytes per pixel is zero"),
            Self::UnsupportedBytesPerPixel(bytes) => {
                write!(
                    formatter,
                    "framebuffer uses {bytes} bytes per pixel, expected {WORD_SIZE}"
                )
            }
            Self::RowBytesOverflow => formatter.write_str("visible row byte count overflows"),
            Self::PitchTooSmall { row_bytes, pitch } => {
                write!(
                    formatter,
                    "row needs {row_bytes} bytes but pitch is {pitch}"
                )
            }
            Self::MisalignedBase(address) => {
                write!(
                    formatter,
                    "framebuffer base {address:#x} is not {WORD_SIZE}-byte aligned"
                )
            }
            Self::MisalignedPitch(pitch) => {
                write!(
                    formatter,
                    "framebuffer pitch {pitch} is not {WORD_SIZE}-byte aligned"
                )
            }
            Self::SpanOverflow => formatter.write_str("framebuffer span overflows"),
            Self::AddressRangeOverflow => {
                formatter.write_str("framebuffer address range overflows")
            }
        }
    }
}

impl core::error::Error for FramebufferLayoutError {}

impl FramebufferLayout {
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

    pub const fn span(self) -> usize {
        self.span
    }

    pub const fn offset(self, x: usize, y: usize) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        offset(x, y, self.pitch, self.bytes_per_pixel)
    }

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

const fn offset(x: usize, y: usize, pitch: usize, bytes_per_pixel: usize) -> Option<usize> {
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
