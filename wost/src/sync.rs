//! Synchronisation primitives.

use core::cell::UnsafeCell;
use core::fmt;
use core::hint::spin_loop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::interrupt::{self, InterruptState};
use crate::panic::PanicToken;

/// A latch that admits exactly one caller.
///
/// Used to make a hardware `init` idempotent: the second caller returns having
/// done nothing, rather than reloading a table the CPU is already using.
#[derive(Debug)]
pub struct Once(AtomicBool);

impl Once {
    /// Create an unclaimed latch.
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// Claim the work this latch guards. True for the first caller only.
    pub fn claim(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }

    /// Whether the work this latch guards has been claimed.
    pub fn is_claimed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl Default for Once {
    fn default() -> Self {
        Self::new()
    }
}

/// A mutual-exclusion lock that spins while contended, held with interrupts
/// off.
///
/// Acquiring one clears the interrupt flag and the guard restores it, which is
/// what keeps a lock safe to take from a handler as well as from ordinary
/// kernel code: an interrupt can no longer arrive on this CPU while the lock is
/// held, so it can no longer arrive at code that tries to take the lock again.
/// Without that, one CPU is enough to deadlock.
///
/// The flag is *restored*, not enabled, so nesting works: an inner lock taken
/// while an outer one is held puts back a clear flag on release.
///
/// Two hazards survive, both of them entries the flag does not mask. A fault
/// can be raised while a lock is held, and a non-maskable interrupt can arrive
/// at any time at all — so a handler for either must still not take a lock. See
/// `docs/006-cpu-fundamentals.md`.
///
/// The cost is interrupt latency: a section holding a lock cannot be preempted,
/// so a long one — scrolling the framebuffer console, say — delays every
/// interrupt behind it. That is a scheduler's problem and waits for one.
pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: `SpinLock` hands out a reference to its value only through a guard,
// and only one guard can exist at a time because `locked` gates acquisition.
// That makes concurrent access from several CPUs sound whenever `T` may cross
// CPUs at all.
unsafe impl<T: Send> Sync for SpinLock<T> {}

// SAFETY: sending the lock sends the value it owns, which `T: Send` permits.
unsafe impl<T: Send> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// Create an unlocked `SpinLock`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Acquire the lock, spinning until it is free.
    ///
    /// The wait runs with the interrupt flag as the caller left it. Spinning
    /// with interrupts off would mean waiting for a holder that cannot be
    /// scheduled and cannot be interrupted, which on one CPU is a hang and on
    /// several is merely slow.
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        loop {
            if let Some(guard) = self.try_lock() {
                return guard;
            }
            while self.is_locked() {
                spin_loop();
            }
        }
    }

    /// Acquire the lock if it is free, without spinning.
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        // Disabling first: taking an interrupt between the exchange and the
        // disable would leave this CPU holding the lock with interrupts on,
        // which is the state the whole design exists to rule out.
        let interrupts = interrupt::disable();
        match self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => Some(SpinLockGuard {
                lock: self,
                interrupts,
            }),
            Err(_) => {
                interrupts.restore();
                None
            }
        }
    }

    /// Whether the lock is currently held.
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// Release the lock regardless of who holds it.
    ///
    /// The panic handler needs this to report through a device whose lock the
    /// code it interrupted was holding. It takes no `unsafe` because a
    /// [`PanicToken`] is itself the proof that makes it sound: the previous
    /// holder can never resume, so the guard it may still own is never used
    /// again.
    ///
    /// The interrupt flag is left alone. Obtaining a [`PanicToken`] cleared it,
    /// and the machine is not going to need it back.
    pub fn force_unlock(&self, _panic: &PanicToken) {
        self.locked.store(false, Ordering::Release);
    }
}

impl<T: fmt::Debug> fmt::Debug for SpinLock<T> {
    /// Never blocks: a `Debug` impl that could spin would hang the one context
    /// most likely to be printing it, the panic handler.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("SpinLock");
        match self.try_lock() {
            Some(guard) => out.field("value", &*guard),
            None => out.field("value", &format_args!("<locked>")),
        }
        .finish()
    }
}

/// Exclusive access to a [`SpinLock`]'s value, released on drop.
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
    interrupts: InterruptState,
}

impl<T: fmt::Debug> fmt::Debug for SpinLockGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: holding this guard means we hold the lock, so no other
        // reference to the value exists.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: holding this guard means we hold the lock, so no other
        // reference to the value exists.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    /// Release, then restore. The other order would open a window in which an
    /// interrupt could arrive at code still holding this lock.
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        self.interrupts.restore();
    }
}
