//! Bootloader handoff.
//!
//! [`entry!`](crate::entry) defines the kernel's entry point and the bootloader
//! request block. Everything the bootloader tells us arrives as [`BootInfo`],
//! whose types name no bootloader at all — swapping Limine out is a change to
//! `limine_glue.rs` and nothing else.

mod limine_glue;

use core::fmt;

pub use limine_glue::enter;
pub use pixel::{PixelFormat, Rgb};

use crate::mmio::MmioRegion;

/// Geometry and pixel layout of a linear framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramebufferInfo {
    /// Visible width in pixels.
    pub width: usize,
    /// Visible height in pixels.
    pub height: usize,
    /// Distance between the starts of consecutive rows, in bytes. Not always
    /// `width * bytes_per_pixel`.
    pub pitch: usize,
    /// Bytes occupied by one pixel.
    pub bytes_per_pixel: usize,
    /// How colour channels sit inside a pixel.
    pub format: PixelFormat,
}

/// A linear framebuffer the bootloader has already mapped for us.
#[derive(Debug)]
pub struct Framebuffer {
    region: MmioRegion,
    info: FramebufferInfo,
}

impl Framebuffer {
    /// Geometry and pixel layout.
    pub const fn info(&self) -> FramebufferInfo {
        self.info
    }

    /// Set the pixel at (`x`, `y`), ignoring coordinates outside the visible
    /// area.
    ///
    /// Clipping is deliberate: a glyph drawn near the right or bottom edge
    /// legitimately runs off it. Anything that survives the clip is still
    /// bounds-checked by the underlying region.
    pub fn write_pixel(&mut self, x: usize, y: usize, colour: Rgb) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }
        let offset = pixel::offset(x, y, self.info.pitch, self.info.bytes_per_pixel);
        let _ = self.region.write_u32(offset, self.info.format.pack(colour));
    }

    /// Paint every visible pixel one colour.
    pub fn fill(&mut self, colour: Rgb) {
        self.fill_rows(0, self.info.height, colour);
    }

    /// Paint `rows` of pixels starting at row `y` one colour.
    pub fn fill_rows(&mut self, y: usize, rows: usize, colour: Rgb) {
        let word = self.info.format.pack(colour);
        for row in y..(y + rows).min(self.info.height) {
            let _ = self
                .region
                .fill_u32(row * self.info.pitch, self.info.width, word);
        }
    }

    /// Copy `rows` of pixels from `from_y` up to `to_y`.
    ///
    /// Used by the console to scroll; rows exposed at the bottom are left
    /// untouched for the caller to clear.
    pub fn copy_rows(&mut self, from_y: usize, to_y: usize, rows: usize) {
        for row in 0..rows {
            for x in 0..self.info.width {
                let colour = self.read_pixel(x, from_y + row);
                self.write_pixel(x, to_y + row, colour);
            }
        }
    }

    fn read_pixel(&self, x: usize, y: usize) -> Rgb {
        let offset = pixel::offset(x, y, self.info.pitch, self.info.bytes_per_pixel);
        let word = self.region.read_u32(offset).unwrap_or(0);
        self.info.format.unpack(word)
    }
}

/// Why the kernel has no usable framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FramebufferError {
    /// The bootloader ignored the framebuffer request.
    NoResponse,
    /// The bootloader answered, but listed no framebuffers.
    NoFramebuffer,
    /// The framebuffer is not a direct RGB mapping.
    UnsupportedMemoryModel,
    /// Only 32 bits per pixel is implemented.
    UnsupportedBpp(u16),
}

impl fmt::Display for FramebufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoResponse => f.write_str("bootloader ignored the framebuffer request"),
            Self::NoFramebuffer => f.write_str("bootloader listed no framebuffers"),
            Self::UnsupportedMemoryModel => f.write_str("framebuffer is not direct RGB"),
            Self::UnsupportedBpp(bpp) => write!(f, "unsupported depth: {bpp} bits per pixel"),
        }
    }
}

impl core::error::Error for FramebufferError {}

/// Everything the bootloader handed the kernel.
#[derive(Debug)]
pub struct BootInfo {
    /// Whether the bootloader honoured the base revision the kernel asked for.
    pub base_revision_supported: bool,
    /// The framebuffer, or why there is no usable one.
    pub framebuffer: Result<Framebuffer, FramebufferError>,
}

/// Items [`entry!`](crate::entry) names in the calling crate.
///
/// Not a stable interface; re-exported only so downstream crates never depend
/// on the bootloader protocol crate directly.
#[doc(hidden)]
pub mod __macro {
    pub use limine::BaseRevision;
    pub use limine::request::{FramebufferRequest, RequestsEndMarker, RequestsStartMarker};
}

/// Define the kernel entry point.
///
/// Expands to the bootloader request block and the `kmain` symbol named by
/// `ENTRY()` in the linker script, then calls `$path`, which must be
/// `fn(BootInfo) -> !`.
///
/// The expansion lands in the calling crate rather than in `wost` on purpose:
/// `#[used]` statics inside an rlib can be dropped wholesale by the linker,
/// because nothing references the archive member holding them
/// (rust-lang/rust#47384). Every `-rt` crate in the ecosystem does it this way.
/// The `#[allow(unsafe_code)]` attributes are what let the calling crate carry
/// `#![deny(unsafe_code)]`; they cover `wost`-authored code that merely expands
/// there, which is why that crate uses `deny` rather than `forbid`.
#[macro_export]
macro_rules! entry {
    ($path:path) => {
        const __WOST_ENTRY_SIGNATURE: fn($crate::boot::BootInfo) -> ! = $path;

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests_start_marker")]
        static __WOST_REQUESTS_START: $crate::boot::__macro::RequestsStartMarker =
            $crate::boot::__macro::RequestsStartMarker::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_BASE_REVISION: $crate::boot::__macro::BaseRevision =
            $crate::boot::__macro::BaseRevision::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_FRAMEBUFFER: $crate::boot::__macro::FramebufferRequest =
            $crate::boot::__macro::FramebufferRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests_end_marker")]
        static __WOST_REQUESTS_END: $crate::boot::__macro::RequestsEndMarker =
            $crate::boot::__macro::RequestsEndMarker::new();

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        extern "C" fn kmain() -> ! {
            $crate::boot::enter(
                &__WOST_BASE_REVISION,
                &__WOST_FRAMEBUFFER,
                __WOST_ENTRY_SIGNATURE,
            )
        }
    };
}
