//! The kernel's output devices.
//!
//! Both live here as locked singletons so the panic handler can reach them from
//! any context without threading them through every call site.

use wost::boot::Framebuffer;
use wost::panic::PanicToken;
use wost::sync::SpinLock;

use crate::arch::current::uart::{self, Uart, UartError};
use crate::console::Console;
use crate::theme;
use crate::{debug, info};

static UART: SpinLock<Option<Uart>> = SpinLock::new(None);
static CONSOLE: SpinLock<Option<Console>> = SpinLock::new(None);

/// Bring up the serial console.
pub(crate) fn init_uart() -> Result<(), UartError> {
    let uart = Uart::new(uart::COM1)?;
    *UART.lock() = Some(uart);
    Ok(())
}

/// Take over the framebuffer with a text console.
pub(crate) fn init_console(framebuffer: Framebuffer) {
    let geometry = framebuffer.info();
    info!(
        "framebuffer: {}x{} pitch={} bpp={}",
        geometry.width,
        geometry.height,
        geometry.pitch,
        geometry.bytes_per_pixel * 8
    );
    debug!("framebuffer: {:?}", geometry.format);
    *CONSOLE.lock() = Some(Console::new(framebuffer, theme::NORMAL));
}

/// Run `f` against the serial console, doing nothing if there is none.
pub(crate) fn with_uart(f: impl FnOnce(&mut Uart)) {
    let mut guard = UART.lock();
    if let Some(uart) = guard.as_mut() {
        f(uart);
    }
}

/// Run `f` against the framebuffer console, doing nothing if there is none.
pub(crate) fn with_console(f: impl FnOnce(&mut Console)) {
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        f(console);
    }
}

/// Geometry of the framebuffer console, if one came up.
#[cfg(feature = "selftest")]
pub(crate) fn console_geometry() -> Option<wost::boot::FramebufferInfo> {
    let guard = CONSOLE.lock();
    guard.as_ref().map(Console::geometry)
}

/// Reclaim both output-device locks on the terminal panic path.
///
/// This module's two; the keyboard's is `input`'s and `wost`'s are reclaimed
/// by [`wost::panic::reclaim`]. All three are called together from the panic
/// handler.
pub(crate) fn force_unlock(token: &PanicToken) {
    UART.force_unlock(token);
    CONSOLE.force_unlock(token);
}
