//! Synchronisation primitives.

use core::cell::UnsafeCell;
use core::fmt;
use core::hint::spin_loop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::arch::current::interrupt::{self, InterruptState};
use crate::panic::PanicToken;

/// How many [`SpinLock`]s this CPU is holding.
///
/// A [`ThreadLock`] may not be taken while any is held, and this is what says
/// so. The hazard is not obvious and it is silent: a `SpinLock` holder has
/// interrupts off, so if the `ThreadLock` it then waits for is held by a
/// thread that has been preempted, that thread can never be scheduled and the
/// wait never ends. It is a deadlock produced by two locks that are each
/// individually correct, and it cost one debugging session before this counter
/// existed.
///
/// One CPU, one counter; per-CPU when SMP lands (D-56).
static SPINLOCKS_HELD: AtomicUsize = AtomicUsize::new(0);

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
/// The cost is interrupt latency: a section holding one of these cannot be
/// preempted, so a long one delays every interrupt behind it. D-34 named that
/// a scheduler's problem; the scheduler's answer is [`ThreadLock`], which
/// keeps interrupts on and refuses trap context, and which the longest
/// section in this kernel — a console scroll — now uses instead (D-63).
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
            Ok(_) => {
                SPINLOCKS_HELD.fetch_add(1, Ordering::Relaxed);
                Some(SpinLockGuard {
                    lock: self,
                    interrupts,
                })
            }
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
        SPINLOCKS_HELD.fetch_sub(1, Ordering::Relaxed);
        self.lock.locked.store(false, Ordering::Release);
        self.interrupts.restore();
    }
}

/// A mutual-exclusion lock that spins while contended, held with interrupts
/// **on** — and refuses to be taken from a trap handler.
///
/// The other half of the pair [`SpinLock`] is one of, and the one D-34 said
/// would earn its keep once there was a scheduler. A `SpinLock` keeps
/// interrupts off for as long as it is held, which is what makes it safe for a
/// handler to take — and what makes every interrupt on the machine wait behind
/// its longest holder. For the console that holder is a scroll, tens of
/// milliseconds of framebuffer, and no interrupt should wait that long.
///
/// So this lock trades the one property for the other. It excludes nothing but
/// other holders, which means a handler that took it could deadlock against
/// the code it interrupted — and that is why it refuses: [`lock`](Self::lock)
/// asserts it is not in trap context, using the depth count D-55 already
/// keeps. The rule "nothing logs from a handler" (D-31) stops being prose and
/// starts being a panic that names itself.
///
/// A holder can be preempted, which a `SpinLock` holder cannot. On one CPU
/// that costs the waiter the rest of its slice and no more: the waiter spins
/// with interrupts on, so the timer takes the CPU back and gives it to the
/// holder. It is not a deadlock, and the scheduler is what makes that true.
///
/// Which is exactly why one of these may not be taken while a [`SpinLock`] is
/// held: there the waiter's interrupts are *off*, the timer never comes, and
/// the preempted holder is never scheduled again. That is the second thing
/// [`lock`](Self::lock) asserts, against a count of the spin locks held.
///
/// The panic path is exempt from both, because the code it interrupted can
/// never resume.
pub struct ThreadLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: as `SpinLock` — the value is reachable only through a guard, and
// `locked` admits one guard at a time.
unsafe impl<T: Send> Sync for ThreadLock<T> {}

// SAFETY: sending the lock sends the value it owns, which `T: Send` permits.
unsafe impl<T: Send> Send for ThreadLock<T> {}

impl<T> ThreadLock<T> {
    /// Create an unlocked `ThreadLock`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Acquire the lock, spinning until it is free.
    ///
    /// # Panics
    ///
    /// Panics if called from a trap handler, outside the panic path. That is
    /// the whole discipline of this type, checked rather than documented:
    /// a handler that took this lock could be the interrupt that landed on
    /// the holder, and it would spin for ever against itself.
    pub fn lock(&self) -> ThreadLockGuard<'_, T> {
        assert!(
            !interrupt::in_trap() || crate::panic::is_panicking(),
            "a trap handler may not take a thread lock"
        );
        assert!(
            SPINLOCKS_HELD.load(Ordering::Relaxed) == 0 || crate::panic::is_panicking(),
            "a spin lock is held, so waiting for a thread lock could never end"
        );
        loop {
            if let Some(guard) = self.try_lock() {
                return guard;
            }
            while self.is_locked() {
                spin_loop();
            }
        }
    }

    /// Acquire the lock if it is free, without spinning or checking context.
    ///
    /// Context-free because it cannot wait: a handler that finds the lock
    /// taken is told so and moves on, which is the one way a handler may
    /// touch one of these at all.
    pub fn try_lock(&self) -> Option<ThreadLockGuard<'_, T>> {
        match self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => Some(ThreadLockGuard { lock: self }),
            Err(_) => None,
        }
    }

    /// Whether the lock is currently held.
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// Release the lock regardless of who holds it. As
    /// [`SpinLock::force_unlock`].
    pub fn force_unlock(&self, _panic: &PanicToken) {
        self.locked.store(false, Ordering::Release);
    }
}

impl<T: fmt::Debug> fmt::Debug for ThreadLock<T> {
    /// Never blocks, for the same reason [`SpinLock`]'s does not.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_lock() {
            Some(guard) => formatter
                .debug_struct("ThreadLock")
                .field("value", &*guard)
                .finish(),
            None => formatter
                .debug_struct("ThreadLock")
                .field("value", &"<locked>")
                .finish(),
        }
    }
}

/// Exclusive access to a [`ThreadLock`]'s value, released on drop.
#[derive(Debug)]
pub struct ThreadLockGuard<'a, T> {
    lock: &'a ThreadLock<T>,
}

impl<T> Deref for ThreadLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: this guard's existence is the proof that no other reference
        // to the value is live — `locked` admits one guard at a time.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for ThreadLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as `deref`, and the guard is held by unique reference here.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for ThreadLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}
