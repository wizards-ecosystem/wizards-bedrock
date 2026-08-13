//! The kernel's output devices.
//!
//! Both live here as locked singletons so the panic handler can reach them from
//! any context without threading them through every call site.

use wost::boot::Framebuffer;
use wost::panic::PanicToken;
use wost::sync::SpinLock;

use crate::arch::current::uart::{self, Uart};
use crate::console::Console;
use crate::{debug, info};

static UART: SpinLock<Option<Uart>> = SpinLock::new(None);
static CONSOLE: SpinLock<Option<Console>> = SpinLock::new(None);

/// Bring up the serial console, returning whether one answered.
pub(crate) fn init_uart() -> bool {
    let Ok(uart) = Uart::new(uart::COM1) else {
        return false;
    };
    *UART.lock() = Some(uart);
    true
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
    *CONSOLE.lock() = Some(Console::new(framebuffer));
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

/// Reclaim both device locks on the terminal panic path.
pub(crate) fn force_unlock(token: &PanicToken) {
    UART.force_unlock(token);
    CONSOLE.force_unlock(token);
}
