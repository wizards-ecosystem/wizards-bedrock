// The locks keep interrupts on, as scrolling is slow; trap handlers use only the `try_` ones.

use wost::boot::Framebuffer;
use wost::sync::{SpinLock, ThreadLock};

use crate::arch::current::ioapic::IoApic;
use crate::arch::current::uart::{Uart, UartError};
use crate::console::Console;
use crate::theme;
use crate::{debug, info, warn};

static UART: ThreadLock<Option<Uart>> = ThreadLock::new(None);
static CONSOLE: ThreadLock<Option<Console>> = ThreadLock::new(None);

pub(crate) fn init_uart() -> Result<(), UartError> {
    let uart = Uart::new()?;
    *UART.lock() = Some(uart);
    Ok(())
}

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

pub(crate) fn with_uart(operation: impl FnOnce(&mut Uart)) {
    let mut guard = UART.lock();
    if let Some(uart) = guard.as_mut() {
        operation(uart);
    }
}

pub(crate) fn with_console(operation: impl FnOnce(&mut Console)) {
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        operation(console);
    }
}

pub(crate) fn try_with_uart(operation: impl FnOnce(&mut Uart)) {
    if let Some(mut guard) = UART.try_lock()
        && let Some(uart) = guard.as_mut()
    {
        operation(uart);
    }
}

pub(crate) fn try_with_console(operation: impl FnOnce(&mut Console)) {
    if let Some(mut guard) = CONSOLE.try_lock()
        && let Some(console) = guard.as_mut()
    {
        operation(console);
    }
}

// Reports only after releasing CONSOLE, because logging takes UART's lock.
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

static IO_APIC: SpinLock<Option<IoApic>> = SpinLock::new(None);

pub(crate) fn keep_io_apic(io_apic: IoApic) {
    *IO_APIC.lock_leaf() = Some(io_apic);
}
