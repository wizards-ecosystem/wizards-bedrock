use wost::boot::FramebufferInfo;

use super::Console;

impl Console {
    /// Geometry of the framebuffer being drawn on.
    #[cfg(feature = "selftest")]
    pub(crate) const fn geometry(&self) -> FramebufferInfo {
        self.geometry
    }
}
