//! Framebuffer text console.
//!
//! Renders Noto Sans Mono bitmap glyphs onto the framebuffer the bootloader
//! handed us. This is user-facing output; diagnostics go to the serial log.

use core::fmt;

use noto_sans_mono_bitmap::{
    FontWeight, RasterHeight, RasterizedChar, get_raster, get_raster_width,
};
use wost::boot::{Framebuffer, FramebufferInfo};

use crate::theme::Palette;

const FONT_HEIGHT: RasterHeight = RasterHeight::Size16;
const FONT_WEIGHT: FontWeight = FontWeight::Regular;

/// Blank border kept around the text area.
/// Where a line starts, and the floor `erase_previous` will not go below.
pub(crate) const MARGIN: usize = 8;

/// Extra pixels between baselines, so ascenders and descenders do not touch.
const LINE_GAP: usize = 2;

/// Drawn in place of any character the font does not carry.
const FALLBACK_GLYPH: char = '?';

/// ASCII backspace: move back a cell and blank it.
const BACKSPACE: char = '\u{8}';

/// A monospaced text console over a linear framebuffer.
#[derive(Debug)]
pub(crate) struct Console {
    framebuffer: Framebuffer,
    geometry: FramebufferInfo,
    glyph_width: usize,
    line_height: usize,
    cursor_x: usize,
    cursor_y: usize,
    palette: Palette,
}

impl Console {
    /// Take over `framebuffer` and clear it to `palette`.
    pub(crate) fn new(framebuffer: Framebuffer, palette: Palette) -> Self {
        let mut console = Self {
            geometry: framebuffer.info(),
            framebuffer,
            glyph_width: get_raster_width(FONT_WEIGHT, FONT_HEIGHT),
            line_height: FONT_HEIGHT.val() + LINE_GAP,
            cursor_x: MARGIN,
            cursor_y: MARGIN,
            palette,
        };
        console.reset(palette);
        console
    }

    /// Bytes the framebuffer mapping covers.
    #[cfg(feature = "selftest")]
    pub(crate) fn framebuffer_bytes(&self) -> usize {
        self.framebuffer.mapped_bytes()
    }

    /// Where the framebuffer is reachable now.
    #[cfg(feature = "selftest")]
    pub(crate) fn framebuffer_address(&self) -> u64 {
        self.framebuffer.address()
    }

    /// Add the shadow that scrolling reads once the heap is available.
    #[must_use = "a refused console shadow was not attached"]
    pub(crate) fn attach_back_buffer(&mut self) -> bool {
        self.framebuffer.attach_back_buffer()
    }

    /// Whether every framebuffer word scrolling may read has a shadow.
    #[cfg(feature = "selftest")]
    pub(crate) fn has_complete_back_buffer(&self) -> bool {
        self.framebuffer.has_complete_back_buffer()
    }

    /// Where the next character would be drawn.
    #[cfg(feature = "selftest")]
    pub(crate) const fn cursor(&self) -> (usize, usize) {
        (self.cursor_x, self.cursor_y)
    }

    /// Dimensions of one character cell.
    #[cfg(feature = "selftest")]
    pub(crate) const fn cell_size(&self) -> (usize, usize) {
        (self.glyph_width, self.line_height)
    }

    /// The colour at (`x`, `y`), for the check that hunts a stray cursor.
    #[cfg(feature = "selftest")]
    pub(crate) fn pixel_at(&self, x: usize, y: usize) -> Option<u32> {
        self.framebuffer.pixel_at(x, y)
    }

    /// Foreground colour for framebuffer checks.
    #[cfg(feature = "selftest")]
    pub(crate) const fn foreground(&self) -> wost::boot::Rgb {
        self.palette.foreground
    }

    /// Scroll stride in pixels.
    #[cfg(feature = "selftest")]
    pub(crate) const fn line_height(&self) -> usize {
        self.line_height
    }

    /// Geometry of the framebuffer being drawn on.
    #[cfg(feature = "selftest")]
    pub(crate) const fn geometry(&self) -> FramebufferInfo {
        self.geometry
    }

    /// Whether one newline would perform a framebuffer scroll.
    #[cfg(feature = "selftest")]
    pub(crate) fn next_line_scrolls(&self) -> bool {
        self.cursor_y
            .checked_add(self.line_height)
            .and_then(|next| next.checked_add(self.line_height))
            .and_then(|bottom| bottom.checked_add(MARGIN))
            .is_none_or(|bottom| bottom > self.geometry.height)
    }

    /// Adopt `palette`, clear the screen, and home the cursor.
    pub(crate) fn reset(&mut self, palette: Palette) {
        self.palette = palette;
        self.framebuffer.fill(palette.background);
        self.cursor_x = MARGIN;
        self.cursor_y = MARGIN;
    }

    /// Write `text` and its line break with the cursor hidden throughout.
    pub(crate) fn write_line(&mut self, text: &str) {
        self.hide_cursor();
        self.put(text);
        self.newline();
        self.show_cursor();
    }

    fn write_text(&mut self, text: &str) {
        // Moving visible framebuffer content would copy the cursor mark too.
        self.hide_cursor();
        self.put(text);
        self.show_cursor();
    }

    fn put(&mut self, text: &str) {
        for character in text.chars() {
            self.write_char(character);
        }
    }

    fn write_char(&mut self, character: char) {
        if character == '\n' {
            self.newline();
            return;
        }
        if character == BACKSPACE {
            self.erase_previous();
            return;
        }
        if self.cursor_x + self.glyph_width + MARGIN > self.geometry.width {
            self.newline();
        }
        let Some(raster) = raster_for(character) else {
            return;
        };
        for (row, intensities) in raster.raster().iter().enumerate() {
            for (column, &intensity) in intensities.iter().enumerate() {
                let colour = self
                    .palette
                    .background
                    .blend(self.palette.foreground, intensity);
                self.framebuffer
                    .write_pixel(self.cursor_x + column, self.cursor_y + row, colour);
            }
        }
        self.cursor_x += self.glyph_width;
    }

    fn erase_previous(&mut self) {
        if self.cursor_x < MARGIN + self.glyph_width {
            return;
        }
        self.cursor_x -= self.glyph_width;
        self.blank_cell();
    }

    /// Paint the cell under the cursor in the background colour.
    fn blank_cell(&mut self) {
        for row in 0..self.line_height {
            for column in 0..self.glyph_width {
                self.framebuffer.write_pixel(
                    self.cursor_x + column,
                    self.cursor_y + row,
                    self.palette.background,
                );
            }
        }
    }

    fn show_cursor(&mut self) {
        self.paint_cursor(self.palette.foreground);
    }

    fn hide_cursor(&mut self) {
        self.paint_cursor(self.palette.background);
    }

    fn paint_cursor(&mut self, colour: wost::boot::Rgb) {
        let row = self.cursor_y + self.line_height - 1;
        for column in 0..self.glyph_width {
            self.framebuffer
                .write_pixel(self.cursor_x + column, row, colour);
        }
    }

    fn newline(&mut self) {
        self.cursor_x = MARGIN;
        self.cursor_y += self.line_height;
        if self.cursor_y + self.line_height + MARGIN > self.geometry.height {
            self.scroll();
        }
    }

    /// Shift the text area up one line and clear the line that opens up.
    fn scroll(&mut self) {
        let shift = self.line_height;
        let retained = self.cursor_y.saturating_sub(MARGIN + shift);
        self.framebuffer.copy_rows(MARGIN + shift, MARGIN, retained);
        self.cursor_y -= shift;
        self.framebuffer
            .fill_rows(self.cursor_y, shift, self.palette.background);
    }
}

impl fmt::Write for Console {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.write_text(text);
        Ok(())
    }
}

/// Look up a glyph, falling back to [`FALLBACK_GLYPH`] so rendering can never
/// fail on an unexpected character.
fn raster_for(character: char) -> Option<RasterizedChar> {
    get_raster(character, FONT_WEIGHT, FONT_HEIGHT)
        .or_else(|| get_raster(FALLBACK_GLYPH, FONT_WEIGHT, FONT_HEIGHT))
}
