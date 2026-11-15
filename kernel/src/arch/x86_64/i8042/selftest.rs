use core::sync::atomic::{AtomicBool, Ordering};

use super::{Controller, config};

#[cfg(feature = "selftest")]
pub(super) static REFUSE_INTERRUPT_READBACK: AtomicBool = AtomicBool::new(false);

impl Controller {
    /// Refuse the next post-write interrupt-byte readback.
    #[cfg(feature = "selftest")]
    pub(crate) fn refuse_interrupt_readback() {
        REFUSE_INTERRUPT_READBACK.store(true, Ordering::Relaxed);
    }

    /// The configuration byte, read live from the controller.
    #[cfg(feature = "selftest")]
    pub(crate) fn config(&mut self) -> Option<u8> {
        if !Self::flush(self) {
            return None;
        }
        self.read_config()
    }

    /// Turn the controller's translation on, and report whether it stuck.
    /// The self-test's only way to reach set 1 with a known key: QEMU honours this bit, so it
    /// reproduces what VMware does unasked, with an injected keystroke the harness chose.
    #[cfg(feature = "selftest")]
    pub(crate) fn enable_translation(&mut self) -> bool {
        if !Self::flush(self) {
            return false;
        }
        let Some(current) = self.read_config() else {
            return false;
        };
        self.write_config(current | config::TRANSLATE)
    }
}
