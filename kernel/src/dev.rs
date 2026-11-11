//! The kernel's output devices, as locked singletons the panic path reaches without plumbing.
//! Their locks keep interrupts on because framebuffer scrolling can be slow; trap handlers
//! therefore use only the non-blocking accessors.

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

/// Run `operation` against the serial console, doing nothing if there is none.
pub(crate) fn with_uart(operation: impl FnOnce(&mut Uart)) {
    let mut guard = UART.lock();
    if let Some(uart) = guard.as_mut() {
        operation(uart);
    }
}

/// Run `operation` against the framebuffer console, doing nothing if there is none.
pub(crate) fn with_console(operation: impl FnOnce(&mut Console)) {
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        operation(console);
    }
}

/// Use the serial console only if no interrupted code holds its lock.
pub(crate) fn try_with_uart(operation: impl FnOnce(&mut Uart)) {
    if let Some(mut guard) = UART.try_lock()
        && let Some(uart) = guard.as_mut()
    {
        operation(uart);
    }
}

/// Use the framebuffer console only if no interrupted code holds its lock.
pub(crate) fn try_with_console(operation: impl FnOnce(&mut Console)) {
    if let Some(mut guard) = CONSOLE.try_lock()
        && let Some(console) = guard.as_mut()
    {
        operation(console);
    }
}

/// Give the console's framebuffer a shadow in RAM; separate from [`init_console`] because the
/// heap does not yet exist when the boot handoff installs the console.
/// Logging takes UART's lock, so this reports only after releasing CONSOLE, adding no otherwise
/// unchecked CONSOLE then UART order.
pub(crate) fn attach_back_buffer() {
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
/// Kept because it owns its register window for the kernel's life and the boot-time checks read
/// routing back out of it.
static IO_APIC: SpinLock<Option<IoApic>> = SpinLock::new(None);

/// Keep the controller the keyboard was just routed through.
/// Self-test also retains the chosen GSI so readback follows firmware routing, not the legacy
/// identity.
pub(crate) fn keep_io_apic(io_apic: IoApic, #[cfg(feature = "selftest")] gsi: u32) {
    *IO_APIC.lock_leaf() = Some(io_apic);
    #[cfg(feature = "selftest")]
    KEYBOARD_GSI.store(gsi, core::sync::atomic::Ordering::Release);
}
