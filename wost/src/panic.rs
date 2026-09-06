//! The terminal panic path.
//!
//! There is no unwinding (`panic = "abort"`) and no supervisor to restart
//! anything yet, so a panic ends with the machine halted. What this module
//! provides is the proof that lets a handler reclaim locks safely on the way
//! out — see `docs/002-architecture.md` pillar 6.

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::{cpu, interrupt};

static PANICKING: AtomicBool = AtomicBool::new(false);

/// Proof kept inside the non-returning terminal boundary.
#[derive(Debug)]
pub(crate) struct PanicToken(());

/// Enter the terminal panic path, reclaim this crate's locks, and report.
///
/// A function pointer cannot carry a lock guard across reclamation. A second
/// call halts rather than recurse through a reporter whose locks may be held.
///
/// Single-CPU only: this stops the calling CPU from resuming interrupted code,
/// but does not stop any other CPU. Revisit when SMP lands.
pub fn terminate(info: &PanicInfo<'_>, report: fn(&PanicInfo<'_>) -> !) -> ! {
    let _disabled = interrupt::disable();
    if PANICKING.swap(true, Ordering::Relaxed) {
        cpu::halt_forever();
    }
    let token = PanicToken(());
    reclaim(&token);
    report(info)
}

/// Whether the terminal panic path has been entered.
///
/// What lets a lock relax a rule it would otherwise enforce: a
/// [`ThreadLock`](crate::sync::ThreadLock) refuses trap context, and the panic
/// handler is the one caller that may be in it and must be served anyway —
/// the code it interrupted is never resuming, so there is nothing left for the
/// rule to protect.
pub(crate) fn is_panicking() -> bool {
    PANICKING.load(Ordering::Relaxed)
}

/// Reclaim every lock `wost` owns, so the panic path can use what it needs.
///
/// The interrupt flag does not mask a fault, so any of these can be held by
/// the code a panic interrupted — and the panic path may want to claim a port
/// to report through, or read back what the legacy controllers are masking.
/// Reclaiming is sound for exactly the reason
/// [`SpinLock::force_unlock`](crate::sync::SpinLock::force_unlock) says: the
/// holder can never resume.
///
/// Every module holding a static lock has one door here; a source contract
/// derives that set from the parsed sources and refuses an omission (D-293).
fn reclaim(panic: &PanicToken) {
    crate::arch::current::pic::force_unlock(panic);
    crate::arch::current::port::force_unlock(panic);
    crate::mm::force_unlock(panic);
    crate::mm::heap::force_unlock(panic);
    crate::arch::current::task::force_unlock(panic);
    crate::arch::current::paging::force_unlock(panic);
    crate::arch::current::pci::force_unlock(panic);
}
