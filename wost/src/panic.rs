use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::{cpu, interrupt};

static PANICKING: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub(crate) struct PanicToken(());

// A second call halts; `report` is a fn pointer so no guard crosses reclamation.
pub fn terminate(info: &PanicInfo<'_>, report: fn(&PanicInfo<'_>) -> !) -> ! {
    // Single-CPU only: this stops the calling CPU resuming interrupted code, never another.
    interrupt::disable_for_good();
    if PANICKING.swap(true, Ordering::Relaxed) {
        cpu::halt_forever();
    }
    let token = PanicToken(());
    reclaim(&token);
    report(info)
}

pub(crate) fn is_panicking() -> bool {
    PANICKING.load(Ordering::Relaxed)
}

// Every module that holds a static lock must have one door here.
fn reclaim(panic: &PanicToken) {
    crate::arch::current::pic::force_unlock(panic);
    crate::arch::current::port::force_unlock(panic);
    crate::mm::force_unlock(panic);
    crate::mm::heap::force_unlock(panic);
    crate::arch::current::task::force_unlock(panic);
    crate::arch::current::paging::force_unlock(panic);
    crate::arch::current::pci::force_unlock(panic);
    crate::sync::forget_held_ranks(panic);
}
