use super::Framebuffer;

impl Framebuffer {
    /// Where the pixels are reachable.
    #[cfg(feature = "selftest")]
    pub fn address(&self) -> u64 {
        self.region.base_address() as u64
    }
}
