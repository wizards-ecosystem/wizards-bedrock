//! The terminal panic path. There is no unwinding (`panic = "abort"`) and no supervisor to restart
//! anything yet, so a panic ends with the machine halted.

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::{cpu, interrupt};

static PANICKING: AtomicBool = AtomicBool::new(false);

/// Proof kept inside the non-returning terminal boundary.
#[derive(Debug)]
pub(crate) struct PanicToken(());

/// Enter the terminal panic path, reclaim this crate's locks, and report.
///
/// A function pointer cannot carry a lock guard across reclamation.
///
/// Single-CPU only: this stops the calling CPU from resuming interrupted code, but does not stop
/// any other CPU.
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
/// Locks may relax a trap-context refusal here: the interrupted holder never resumes.
pub(crate) fn is_panicking() -> bool {
    PANICKING.load(Ordering::Relaxed)
}

/// Reclaim every lock `wost` owns, so the panic path can use what it needs.
///
/// A fault is not masked by the interrupt flag, so a panic can interrupt a holder who never
/// resumes.
fn reclaim(panic: &PanicToken) {
    crate::arch::current::pic::force_unlock(panic);
    crate::arch::current::port::force_unlock(panic);
    crate::mm::force_unlock(panic);
    crate::mm::heap::force_unlock(panic);
    crate::arch::current::task::force_unlock(panic);
    crate::arch::current::paging::force_unlock(panic);
    crate::arch::current::pci::force_unlock(panic);
}
