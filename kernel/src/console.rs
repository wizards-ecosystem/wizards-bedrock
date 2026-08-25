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

    /// Where the framebuffer is reachable now.
    #[cfg(feature = "selftest")]
    pub(crate) fn framebuffer_address(&self) -> u64 {
        self.framebuffer.address()
    }

    /// Give the framebuffer a shadow in RAM, so scrolling stops reading it.
    ///
    /// Called once the heap exists — the console is built long before it.
    pub(crate) fn attach_back_buffer(&mut self) -> bool {
        self.framebuffer.attach_back_buffer()
    }

    /// Whether scrolling reads RAM rather than the device.
    ///
    /// Read only by the boot-time check, so it exists only where that does.
    #[cfg(feature = "selftest")]
    pub(crate) fn has_back_buffer(&self) -> bool {
        self.framebuffer.has_back_buffer()
    }

    /// Where the next character would be drawn.
    ///
    /// Read by the boot-time check that the editing keys move the cursor the
    /// way a person expects — the only property of this console anything
    /// automated has ever asserted.
    #[cfg(feature = "selftest")]
    pub(crate) const fn cursor(&self) -> (usize, usize) {
        (self.cursor_x, self.cursor_y)
    }

    /// The width of one character cell, so a check can say how far the cursor
    /// should have moved.
    #[cfg(feature = "selftest")]
    pub(crate) const fn cell_size(&self) -> (usize, usize) {
        (self.glyph_width, self.line_height)
    }

    /// Geometry of the framebuffer being drawn on.
    #[cfg(feature = "selftest")]
    pub(crate) const fn geometry(&self) -> FramebufferInfo {
        self.geometry
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
        // The cursor is a mark on the framebuffer like any other, so it is
        // taken down before the text moves and put back where the text left
        // off. Without this a scroll would carry a copy of it up the screen.
        self.set_cursor(false);
        for character in text.chars() {
            self.write_char(character);
        }
        self.set_cursor(true);
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

    /// Move back one cell and blank it.
    ///
    /// Stops at the left margin rather than wrapping to the line above: this
    /// console keeps no record of where the previous line ended, so backing
    /// onto it would leave the cursor somewhere the text is not. A line editor
    /// that tracks its own buffer can do better; an echo of raw keystrokes
    /// cannot, and pretending otherwise would erase the wrong glyph.
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

    /// Draw the cursor, or erase it, at its current cell.
    ///
    /// An underline rather than a block, so it never hides the character it
    /// sits on. Drawn only while the console is idle — every write hides it
    /// first and shows it again afterwards, which is what keeps it from being
    /// scrolled up the screen as a stray mark.
    fn set_cursor(&mut self, visible: bool) {
        let colour = if visible {
            self.palette.foreground
        } else {
            self.palette.background
        };
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
