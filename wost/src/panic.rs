//! The terminal panic path.
//!
//! There is no unwinding (`panic = "abort"`) and no supervisor to restart
//! anything yet, so a panic ends with the machine halted. What this module
//! provides is the proof that lets a handler reclaim locks safely on the way
//! out — see `docs/002-architecture.md` pillar 6.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::cpu;

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
    cpu::disable_interrupts();
    if PANICKING.swap(true, Ordering::Relaxed) {
        return None;
    }
    Some(PanicToken(()))
}
