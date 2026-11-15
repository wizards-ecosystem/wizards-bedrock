use super::CONSOLE;
#[cfg(doc)]
use super::keep_io_apic;

use crate::console::Console;

/// Geometry of the framebuffer console, if one came up.
#[cfg(feature = "selftest")]
pub(crate) fn console_geometry() -> Option<wost::boot::FramebufferInfo> {
    let guard = CONSOLE.lock();
    guard.as_ref().map(Console::geometry)
}

/// Set by [`keep_io_apic`]; meaningless unless there is an I/O APIC.
#[cfg(feature = "selftest")]
pub(super) static KEYBOARD_GSI: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
