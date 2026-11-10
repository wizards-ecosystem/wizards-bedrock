//! The kernel's output devices.
//!
//! Locked singletons keep the panic path independent of call-site plumbing.
//! Their locks keep interrupts on because framebuffer scrolling can be slow;
//! trap handlers therefore use only the non-blocking accessors (D-31, D-63).

use wost::boot::Framebuffer;
use wost::sync::{SpinLock, ThreadLock};

use crate::arch::current::ioapic::IoApic;
use crate::arch::current::uart::{Uart, UartError};
use crate::console::Console;
use crate::theme;
use crate::{debug, info, warn};

#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
use selftest::KEYBOARD_GSI;
#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    console_cursor, console_geometry, console_has_complete_back_buffer, console_probe,
    framebuffer_address, framebuffer_bytes, io_apic_base, keyboard_gsi, routing_of,
};

static UART: ThreadLock<Option<Uart>> = ThreadLock::new(None);
static CONSOLE: ThreadLock<Option<Console>> = ThreadLock::new(None);

/// Bring up the serial console.
pub(crate) fn init_uart() -> Result<(), UartError> {
    let uart = Uart::new()?;
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

/// Use the serial console only if no interrupted code holds its lock.
pub(crate) fn try_with_uart(f: impl FnOnce(&mut Uart)) {
    if let Some(mut guard) = UART.try_lock()
        && let Some(uart) = guard.as_mut()
    {
        f(uart);
    }
}

/// Use the framebuffer console only if no interrupted code holds its lock.
pub(crate) fn try_with_console(f: impl FnOnce(&mut Console)) {
    if let Some(mut guard) = CONSOLE.try_lock()
        && let Some(console) = guard.as_mut()
    {
        f(console);
    }
}

/// Give the console's framebuffer a shadow in RAM.
///
/// Separate from [`init_console`] because the heap does not yet exist when the
/// boot handoff installs the console.
pub(crate) fn attach_back_buffer() {
    // Logging takes UART's lock, so report only after releasing CONSOLE and
    // avoid adding an otherwise-unchecked CONSOLE → UART order (D-105).
    let attached = {
        let mut guard = CONSOLE.lock();
        let Some(console) = guard.as_mut() else {
            return;
        };
        console.attach_back_buffer()
    };
    if attached {
        info!("console: back buffer attached; scrolling no longer reads the device");
    } else {
        warn!("console: no memory for a back buffer; scrolling still reads the device");
    }
}

/// The I/O APIC, once a line has been routed through it.
///
/// Kept because the controller owns its register window for the life of the
/// kernel and the boot-time checks read routing back out of it.
static IO_APIC: SpinLock<Option<IoApic>> = SpinLock::new(None);

/// Keep the controller the keyboard was just routed through.
///
/// Self-test also retains the chosen GSI so readback follows firmware routing
/// rather than assuming the legacy identity.
pub(crate) fn keep_io_apic(io_apic: IoApic, #[cfg(feature = "selftest")] gsi: u32) {
    *IO_APIC.lock_leaf() = Some(io_apic);
    #[cfg(feature = "selftest")]
    KEYBOARD_GSI.store(gsi, core::sync::atomic::Ordering::Release);
}
