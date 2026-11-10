//! The terminal panic path: no unwinding, so a panic reclaims locks, reports and halts.

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::{cpu, interrupt};

static PANICKING: AtomicBool = AtomicBool::new(false);

/// Proof kept inside the non-returning terminal boundary.
#[derive(Debug)]
pub(crate) struct PanicToken(());

/// Enter the terminal panic path, reclaim this crate's locks, and report.
///
/// A second call halts rather than recurse through a reporter whose locks may be held.
/// `report` is a function pointer because a lock guard cannot be carried across reclamation.
/// Single-CPU only: this stops the calling CPU resuming interrupted code, never another CPU.
pub fn terminate(info: &PanicInfo<'_>, report: fn(&PanicInfo<'_>) -> !) -> ! {
    let _disabled = interrupt::disable();
    if PANICKING.swap(true, Ordering::Relaxed) {
        cpu::halt_forever();
    }
    let token = PanicToken(());
    reclaim(&token);
    report(info)
}

/// Whether the panic path began, which waives both refusals of [`ThreadLock::lock`].
///
/// A trap handler or a held spin lock is then allowed, since the interrupted code never resumes.
///
/// [`ThreadLock::lock`]: crate::sync::ThreadLock::lock
pub(crate) fn is_panicking() -> bool {
    PANICKING.load(Ordering::Relaxed)
}

/// Reclaim every lock `wost` owns: a fault can interrupt any holder, and that holder never resumes.
///
/// Every module holding a static lock must have one door here; a source contract refuses a gap.
fn reclaim(panic: &PanicToken) {
    crate::arch::current::pic::force_unlock(panic);
    crate::arch::current::port::force_unlock(panic);
    crate::mm::force_unlock(panic);
    crate::mm::heap::force_unlock(panic);
    crate::arch::current::task::force_unlock(panic);
    crate::arch::current::paging::force_unlock(panic);
    crate::arch::current::pci::force_unlock(panic);
}
