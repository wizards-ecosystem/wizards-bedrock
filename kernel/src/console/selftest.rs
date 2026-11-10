use wost::boot::FramebufferInfo;

use super::{Console, MARGIN};

impl Console {
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
}
