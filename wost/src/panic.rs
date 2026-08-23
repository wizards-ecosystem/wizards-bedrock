//! The terminal panic path.
//!
//! There is no unwinding (`panic = "abort"`) and no supervisor to restart
//! anything yet, so a panic ends with the machine halted. What this module
//! provides is the proof that lets a handler reclaim locks safely on the way
//! out — see `docs/002-architecture.md` pillar 6.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::interrupt;

static PANICKING: AtomicBool = AtomicBool::new(false);

/// Proof that the machine is in its terminal panic path.
///
/// Obtainable only from [`enter`], which disables interrupts and admits a
/// single caller for the life of the boot. Holding one means the code this CPU
/// interrupted can never resume, which is what makes
/// [`SpinLock::force_unlock`](crate::sync::SpinLock::force_unlock) sound.
#[derive(Debug)]
pub struct PanicToken(());

/// Begin the terminal panic path.
///
/// Disables interrupts and yields a token the first time it is called. A second
/// call — a panic raised while reporting a panic — yields `None`, and that
/// caller must halt immediately rather than recurse through the same locks.
///
/// Single-CPU only: this stops the calling CPU from resuming interrupted code,
/// but does not stop any other CPU. Revisit when SMP lands.
pub fn enter() -> Option<PanicToken> {
    interrupt::disable();
    if PANICKING.swap(true, Ordering::Relaxed) {
        return None;
    }
    Some(PanicToken(()))
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
/// **Every lock in a `static` inside this crate belongs in this function.** A
/// new one that is not listed here is a deadlock waiting for the first panic
/// raised while it is held, and nothing but this comment will catch it. The
/// kernel reclaims its own locks separately; `wost` cannot see them.
pub fn reclaim(panic: &PanicToken) {
    crate::arch::current::pic::force_unlock(panic);
    crate::arch::current::port::force_unlock(panic);
    crate::mm::force_unlock(panic);
    crate::mm::heap::force_unlock(panic);
    crate::arch::current::task::force_unlock(panic);
}
