#[cfg(doc)]
use super::keep_io_apic;
use super::{CONSOLE, IO_APIC};
use crate::arch::current::ioapic::IoApic;
use crate::console::Console;

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

/// The global system interrupt the keyboard was routed on.
#[cfg(feature = "selftest")]
pub(crate) fn keyboard_gsi() -> u32 {
    KEYBOARD_GSI.load(core::sync::atomic::Ordering::Acquire)
}

/// Set by [`keep_io_apic`]; meaningless unless there is an I/O APIC.
#[cfg(feature = "selftest")]
pub(super) static KEYBOARD_GSI: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

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
