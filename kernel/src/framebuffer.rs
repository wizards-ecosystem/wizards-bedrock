//! Minimal framebuffer text console for early boot output.
//!
//! Renders glyphs from the Noto Sans Mono bitmap font onto the
//! Limine-provided linear framebuffer (assumed 32 bpp).

use limine::framebuffer::Framebuffer;
use noto_sans_mono_bitmap::{FontWeight, RasterHeight, get_raster, get_raster_width};

const FONT_HEIGHT: RasterHeight = RasterHeight::Size16;
const MARGIN: usize = 8;

pub struct Console<'a> {
    fb: &'a Framebuffer<'a>,
    font_width: usize,
    cursor_x: usize,
    cursor_y: usize,
}

impl<'a> Console<'a> {
    pub fn new(fb: &'a Framebuffer<'a>) -> Self {
        let font_width = get_raster_width(FontWeight::Regular, FONT_HEIGHT);
        let mut console = Self {
            fb,
            font_width,
            cursor_x: MARGIN,
            cursor_y: MARGIN,
        };
        console.clear();
        console
    }

    pub fn clear(&mut self) {
        for y in 0..self.fb.height() as usize {
            for x in 0..self.fb.width() as usize {
                self.put_pixel(x, y, 0x0B_10_21); // deep midnight blue
            }
        }
        self.cursor_x = MARGIN;
        self.cursor_y = MARGIN;
    }

    pub fn write_line(&mut self, text: &str) {
        for ch in text.chars() {
            self.write_char(ch);
        }
        self.newline();
    }

    fn write_char(&mut self, ch: char) {
        if ch == '\n' {
            self.newline();
            return;
        }
        if self.cursor_x + self.font_width > self.fb.width() as usize - MARGIN {
            self.newline();
        }
        let raster = get_raster(ch, FontWeight::Regular, FONT_HEIGHT)
            .unwrap_or_else(|| get_raster('?', FontWeight::Regular, FONT_HEIGHT).unwrap());
        for (dy, row) in raster.raster().iter().enumerate() {
            for (dx, &intensity) in row.iter().enumerate() {
                let color = blend(intensity, 0x0B_10_21, 0xE8_D5_A3); // parchment gold on midnight
                self.put_pixel(self.cursor_x + dx, self.cursor_y + dy, color);
            }
        }
        self.cursor_x += self.font_width;
    }

    fn newline(&mut self) {
        self.cursor_x = MARGIN;
        self.cursor_y += FONT_HEIGHT.val() + 2;
    }

    fn put_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x >= self.fb.width() as usize || y >= self.fb.height() as usize {
            return;
        }
        let offset = y * self.fb.pitch() as usize + x * (self.fb.bpp() as usize / 8);
        unsafe {
            let ptr = self.fb.addr().add(offset) as *mut u32;
            ptr.write_volatile(color);
        }
    }
}

/// Linear blend between background and foreground by glyph intensity.
fn blend(intensity: u8, bg: u32, fg: u32) -> u32 {
    let t = intensity as u32;
    let mut out = 0u32;
    for shift in [0, 8, 16] {
        let b = (bg >> shift) & 0xFF;
        let f = (fg >> shift) & 0xFF;
        let c = (b * (255 - t) + f * t) / 255;
        out |= c << shift;
    }
    out
}
