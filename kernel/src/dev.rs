//! The kernel's output devices.
//!
//! Both live here as locked singletons so the panic handler can reach them from
//! any context without threading them through every call site.

use wost::boot::Framebuffer;
use wost::panic::PanicToken;
use wost::sync::SpinLock;

use crate::arch::current::ioapic::IoApic;
use crate::arch::current::uart::{self, Uart, UartError};
use crate::console::Console;
use crate::theme;
use crate::{debug, info, warn};

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
    IO_APIC.force_unlock(token);
}

/// Give the console's framebuffer a shadow in RAM.
///
/// Separate from [`init_console`] because the console comes up during the boot
/// handoff and the heap does not exist until `mem::init` has run. Reports what
/// happened rather than returning it: there is exactly one caller and nothing
/// it could do differently.
pub(crate) fn attach_back_buffer() {
    let mut guard = CONSOLE.lock();
    let Some(console) = guard.as_mut() else {
        return;
    };
    if console.attach_back_buffer() {
        info!("console: back buffer attached; scrolling no longer reads the device");
    } else {
        warn!("console: no memory for a back buffer; scrolling still reads the device");
    }
}

/// Whether the console scrolls out of RAM rather than out of device memory.
///
/// Read only by the boot-time check, so it exists only where that does.
#[cfg(feature = "selftest")]
pub(crate) fn console_has_back_buffer() -> bool {
    CONSOLE
        .lock()
        .as_ref()
        .is_some_and(|console| console.has_back_buffer())
}

/// The I/O APIC, once a line has been routed through it.
///
/// Kept because the controller owns its register window for the life of the
/// kernel and the boot-time checks read routing back out of it. On the panic
/// path's list like every other `static` lock.
static IO_APIC: SpinLock<Option<IoApic>> = SpinLock::new(None);

/// Keep the controller a line was just routed through.
pub(crate) fn keep_io_apic(io_apic: IoApic) {
    *IO_APIC.lock() = Some(io_apic);
}

/// Ask the I/O APIC something, if there is one.
#[cfg(feature = "selftest")]
pub(crate) fn with_io_apic<T>(f: impl FnOnce(&mut IoApic) -> T) -> Option<T> {
    IO_APIC.lock().as_mut().map(f)
}
