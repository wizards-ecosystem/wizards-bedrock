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

/// Geometry of the framebuffer console, if one came up.
#[cfg(feature = "selftest")]
pub(crate) fn console_geometry() -> Option<wost::boot::FramebufferInfo> {
    let guard = CONSOLE.lock();
    guard.as_ref().map(Console::geometry)
}

/// Bytes the framebuffer mapping covers, if a console came up.
#[cfg(feature = "selftest")]
pub(crate) fn framebuffer_bytes() -> Option<usize> {
    CONSOLE.lock().as_ref().map(Console::framebuffer_bytes)
}

/// What the console holds at (`x`, `y`), and the marks a check compares to.
///
/// Returns the pixel, the packed foreground, and the line height — read under
/// one guard, because three separate calls would let a log line land between
/// them and move what the first answered about.
#[cfg(feature = "selftest")]
pub(crate) fn console_probe(x: usize, y: usize) -> Option<(u32, u32, usize)> {
    let guard = CONSOLE.lock();
    let console = guard.as_ref()?;
    let pixel = console.pixel_at(x, y)?;
    let foreground = console.geometry().format.pack(console.foreground());
    Some((pixel, foreground, console.line_height()))
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

/// Whether the console has RAM for every framebuffer word it may scroll.
///
/// Read only by the boot-time check, so it exists only where that does.
#[cfg(feature = "selftest")]
pub(crate) fn console_has_complete_back_buffer() -> bool {
    CONSOLE
        .lock()
        .as_ref()
        .is_some_and(|console| console.has_complete_back_buffer())
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

/// The global system interrupt the keyboard was routed on.
#[cfg(feature = "selftest")]
pub(crate) fn keyboard_gsi() -> u32 {
    KEYBOARD_GSI.load(core::sync::atomic::Ordering::Acquire)
}

/// Set by [`keep_io_apic`]; meaningless unless there is an I/O APIC.
#[cfg(feature = "selftest")]
static KEYBOARD_GSI: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// How the I/O APIC routes one global system interrupt.
///
/// Returns a value instead of letting a closure escape under a leaf guard.
/// `None` means either no controller or no route for `gsi` (D-105, D-121).
#[cfg(feature = "selftest")]
pub(crate) fn routing_of(gsi: u32) -> Option<Routing> {
    let mut guard = IO_APIC.lock_leaf();
    let io_apic = guard.as_mut()?;
    if !io_apic.handles(gsi) {
        return None;
    }
    let (low, high) = io_apic.entry_of(gsi).ok()?;
    Some(Routing {
        masked: low & (1 << 16) != 0,
        vector: low as u8,
        low,
        high,
    })
}

/// What the I/O APIC does with one line.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Routing {
    /// Whether the line is masked.
    pub(crate) masked: bool,
    /// The vector it is delivered on.
    pub(crate) vector: u8,
    /// Complete low redirection word read from hardware.
    pub(crate) low: u32,
    /// Complete high redirection word read from hardware.
    pub(crate) high: u32,
}

/// The lowest line the I/O APIC routes, if there is one.
#[cfg(feature = "selftest")]
pub(crate) fn io_apic_base() -> Option<u32> {
    IO_APIC.lock_leaf().as_ref().map(IoApic::base)
}

/// Where the console's framebuffer is reachable now.
#[cfg(feature = "selftest")]
pub(crate) fn framebuffer_address() -> Option<u64> {
    CONSOLE
        .lock()
        .as_ref()
        .map(|console| console.framebuffer_address())
}

/// Where the console's cursor is, and how big one character cell is.
#[cfg(feature = "selftest")]
pub(crate) fn console_cursor() -> Option<((usize, usize), (usize, usize))> {
    let guard = CONSOLE.lock();
    let console = guard.as_ref()?;
    Some((console.cursor(), console.cell_size()))
}
