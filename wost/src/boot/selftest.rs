use super::Framebuffer;

impl Framebuffer {
    /// Where the pixels are reachable.
    #[cfg(feature = "selftest")]
    pub fn address(&self) -> u64 {
        self.region.base_address() as u64
    }

    /// Bytes the mapping covers, which every pixel offset must land inside.
    #[cfg(feature = "selftest")]
    pub fn mapped_bytes(&self) -> usize {
        self.layout.span()
    }

    /// Whether every framebuffer word has a shadow.
    #[cfg(feature = "selftest")]
    pub fn has_complete_back_buffer(&self) -> bool {
        !self.back.is_empty() && self.back.len() == self.words()
    }

    /// The pixel at (`x`, `y`), or `None` outside the visible area.
    ///
    /// Read from the shadow to avoid an uncached device read and make stray
    /// cursor marks observable (D-137).
    #[cfg(feature = "selftest")]
    pub fn pixel_at(&self, x: usize, y: usize) -> Option<u32> {
        Some(self.word_at(self.layout.offset(x, y)?))
    }
}
