//! The kernel's output devices.
//!
//! Both live here as locked singletons so the panic handler can reach them from
//! any context without threading them through every call site.
//!
//! Their locks keep interrupts on (D-63): writing a line can cost a scroll,
//! and no interrupt should wait tens of milliseconds behind one. The price is
//! that no trap handler may write to either device, which was already the
//! rule (D-31) and is now a lock that panics rather than a comment.

use wost::boot::Framebuffer;
use wost::sync::{SpinLock, ThreadLock};

use crate::arch::current::ioapic::IoApic;
use crate::arch::current::uart::{self, Uart, UartError};
use crate::console::Console;
use crate::theme;
use crate::{debug, info, warn};

static UART: ThreadLock<Option<Uart>> = ThreadLock::new(None);
static CONSOLE: ThreadLock<Option<Console>> = ThreadLock::new(None);

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
/// Separate from [`init_console`] because the console comes up during the boot
/// handoff and the heap does not exist until `mem::init` has run. Reports what
/// happened rather than returning it: there is exactly one caller and nothing
/// it could do differently.
pub(crate) fn attach_back_buffer() {
    // Reported after the guard is gone. Logging takes `UART`'s `ThreadLock`,
    // and doing that under `CONSOLE`'s would establish a `CONSOLE → UART`
    // order that nothing else in the kernel knows about — a lock order acquired
    // by accident is the one that closes a cycle later (D-105).
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
/// kernel and the boot-time checks read routing back out of it.
static IO_APIC: SpinLock<Option<IoApic>> = SpinLock::new(None);

/// Keep the controller the keyboard was just routed through, and the global
/// system interrupt it was routed on.
///
/// The interrupt number is kept because the boot-time check needs the line
/// the kernel actually chose: firmware may map any legacy IRQ to any global
/// system interrupt, and a check that assumed the identity would be reading
/// somebody else's entry.
pub(crate) fn keep_io_apic(io_apic: IoApic, gsi: u32) {
    *IO_APIC.lock_leaf() = Some(io_apic);
    KEYBOARD_GSI.store(gsi, core::sync::atomic::Ordering::Release);
}

/// The global system interrupt the keyboard was routed on.
#[cfg(feature = "selftest")]
pub(crate) fn keyboard_gsi() -> u32 {
    KEYBOARD_GSI.load(core::sync::atomic::Ordering::Acquire)
}

/// Set by [`keep_io_apic`]; meaningless unless there is an I/O APIC.
static KEYBOARD_GSI: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// How the I/O APIC routes one global system interrupt.
///
/// A value rather than a closure. `with_io_apic` used to hand a caller `&mut
/// IoApic` under a leaf guard, whose whole obligation is that *nothing is
/// called while it is live* — and a closure is a call by construction. The
/// obligation was on the caller and nothing checked it (D-105, D-121).
///
/// `None` where there is no I/O APIC, or where it does not route `gsi`. The
/// two are the same answer to the question a caller is asking, which is "what
/// happens to this line".
#[cfg(feature = "selftest")]
pub(crate) fn routing_of(gsi: u32) -> Option<Routing> {
    let mut guard = IO_APIC.lock_leaf();
    let io_apic = guard.as_mut()?;
    if !io_apic.handles(gsi) {
        return None;
    }
    Some(Routing {
        masked: io_apic.is_masked(gsi).ok()?,
        vector: io_apic.vector_of(gsi).ok()?,
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
