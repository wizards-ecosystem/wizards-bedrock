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
const MARGIN: usize = 8;

/// Extra pixels between baselines, so ascenders and descenders do not touch.
const LINE_GAP: usize = 2;

/// Drawn in place of any character the font does not carry.
const FALLBACK_GLYPH: char = '?';

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

    /// Adopt `palette`, clear the screen, and home the cursor.
    pub(crate) fn reset(&mut self, palette: Palette) {
        self.palette = palette;
        self.framebuffer.fill(palette.background);
        self.cursor_x = MARGIN;
        self.cursor_y = MARGIN;
    }

    /// Write `text` followed by a line break.
    pub(crate) fn write_line(&mut self, text: &str) {
        self.write_text(text);
        self.newline();
    }

    fn write_text(&mut self, text: &str) {
        for character in text.chars() {
            self.write_char(character);
        }
    }

    fn write_char(&mut self, character: char) {
        if character == '\n' {
            self.newline();
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
