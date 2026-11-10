//! Synchronisation primitives.

#[cfg(feature = "selftest")]
mod selftest;

use core::cell::UnsafeCell;
use core::fmt;
use core::hint::spin_loop;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::interrupt::{self, InterruptState};
// Lock state is CPU-local so processors cannot share nesting evidence.
use crate::arch::current::percpu;
#[cfg(feature = "selftest")]
use crate::arch::current::percpu::LockKind;
use crate::panic::PanicToken;
#[cfg(feature = "selftest")]
pub use selftest::{
    ForgottenSpinLockGuard, Injection, SpinLockWitness, taken_under_a_leaf, the_leaf_it_was_under,
};

/// How many spin locks this CPU holds, the runtime backstop for paths type borrows cannot see.
///
/// Interrupting or switching away while it is nonzero can strand a lock.
pub(crate) fn spinlocks_held() -> usize {
    percpu::current().spinlocks_held()
}

/// Evidence of thread context, outside any trap handler; neither `Send` nor `Sync`.
///
/// Blocking APIs and [`SpinLock::lock`] borrow it, so a guard excludes other token operations.
/// Forgeable by design: [`Self::claim`] mints another token and leaf locks need none.
#[derive(Debug)]
pub struct ThreadContext(PhantomData<*const ()>);

impl ThreadContext {
    /// Mint one, or `None` inside a trap handler.
    pub fn claim() -> Option<Self> {
        (!interrupt::in_trap()).then_some(Self(PhantomData))
    }

    /// Mint one, panicking inside a trap handler.
    ///
    /// # Panics
    ///
    /// From trap context, where a block would carry the handler's stack frame away with it.
    pub fn require() -> Self {
        match Self::claim() {
            Some(context) => context,
            None => panic!("thread context required, but a handler is on the stack"),
        }
    }
}

/// A latch that admits exactly one caller, so a second `init` cannot reload a table in use.
#[derive(Debug)]
pub struct Once(AtomicBool);

impl Once {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// Claim the work this latch guards, true for the first caller only.
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

/// A spinning mutual-exclusion lock held with interrupts off, so no handler re-enters on this CPU.
///
/// The guard restores the prior flag so nesting holds; fault and NMI handlers must never lock.
/// Long holders delay interrupts and belong behind [`ThreadLock`].
pub struct SpinLock<T> {
    locked: AtomicBool,
    #[cfg(feature = "selftest")]
    forgotten_for_test: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the value is reachable only through a guard and `locked` admits one guard at a time,
// so access from several CPUs is sound whenever `T` may cross CPUs at all.
unsafe impl<T: Send> Sync for SpinLock<T> {}

// SAFETY: sending the lock sends the value it owns, which `T: Send` permits.
unsafe impl<T: Send> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// Reach the value without locking; the exclusive borrow proves no guard exists.
    ///
    /// It keeps typed evidence across allocating code without holding a leaf guard.
    pub const fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            #[cfg(feature = "selftest")]
            forgotten_for_test: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Acquire the lock, spinning with the caller's interrupt state kept so a holder can run.
    ///
    /// The guard borrows `context`, excluding blocking and another lock through the same token.
    pub fn lock<'lock>(&'lock self, context: &'lock mut ThreadContext) -> SpinLockGuard<'lock, T> {
        let _ = context;
        loop {
            if let Some(guard) = self.acquire(
                #[cfg(feature = "selftest")]
                LockKind::Ordered,
            ) {
                return guard;
            }
            while self.is_locked() {
                spin_loop();
            }
        }
    }

    /// Acquire the lock if it is free, without spinning.
    pub fn try_lock<'lock>(
        &'lock self,
        context: &'lock mut ThreadContext,
    ) -> Option<SpinLockGuard<'lock, T>> {
        let _ = context;
        self.acquire(
            #[cfg(feature = "selftest")]
            LockKind::Ordered,
        )
    }

    /// Acquire the lock for an operation that is over before it returns.
    ///
    /// Its guard may enclose only take-mutate-return work, never a call or a block.
    /// Needing no context makes it usable from `Drop` and lets a leaf nest under a token lock.
    #[cfg_attr(feature = "selftest", track_caller)]
    pub fn lock_leaf(&self) -> SpinLockGuard<'_, T> {
        loop {
            if let Some(guard) = self.acquire(
                #[cfg(feature = "selftest")]
                LockKind::Leaf,
            ) {
                return guard;
            }
            while self.is_locked() {
                spin_loop();
            }
        }
    }

    /// Acquire a leaf lock if it is free, without spinning.
    #[cfg_attr(feature = "selftest", track_caller)]
    pub fn try_lock_leaf(&self) -> Option<SpinLockGuard<'_, T>> {
        self.acquire(
            #[cfg(feature = "selftest")]
            LockKind::Leaf,
        )
    }

    /// The acquisition itself, with no discipline attached.
    #[cfg_attr(feature = "selftest", track_caller)]
    fn acquire(&self, #[cfg(feature = "selftest")] kind: LockKind) -> Option<SpinLockGuard<'_, T>> {
        // Disable first, or an interrupt after the exchange finds the lock held with interrupts on.
        let interrupts = interrupt::disable();
        match self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => {
                // One call, since a split read and write would have to order the state it reports.
                percpu::current().took_lock(
                    #[cfg(feature = "selftest")]
                    kind,
                    #[cfg(feature = "selftest")]
                    core::panic::Location::caller(),
                );
                Some(SpinLockGuard {
                    lock: self,
                    #[cfg(feature = "selftest")]
                    kind,
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

    /// Release the lock whoever holds it; [`PanicToken`] proves that holder never resumes.
    ///
    /// The holder's cleared interrupt state deliberately stays cleared.
    pub(crate) fn force_unlock(&self, _panic: &PanicToken) {
        self.locked.store(false, Ordering::Release);
    }
}

impl<T: fmt::Debug> fmt::Debug for SpinLock<T> {
    /// Never blocks, since a `Debug` that could spin would hang the panic handler printing it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("SpinLock");
        match self.try_lock_leaf() {
            Some(guard) => debug.field("value", &*guard),
            None => debug.field("value", &format_args!("<locked>")),
        }
        .finish()
    }
}

/// Exclusive access to a [`SpinLock`]'s value, released on drop.
///
/// Not `Send`, by [`InterruptState`]: the flag it restores belongs to the CPU that cleared it.
pub struct SpinLockGuard<'lock, T> {
    lock: &'lock SpinLock<T>,
    #[cfg(feature = "selftest")]
    kind: LockKind,
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
    /// Release, then restore; the other order lets an interrupt reach code still holding the lock.
    fn drop(&mut self) {
        percpu::current().released_lock(
            #[cfg(feature = "selftest")]
            self.kind,
        );
        self.lock.locked.store(false, Ordering::Release);
        self.interrupts.restore_in_place();
    }
}

/// A spinning mutual-exclusion lock held with interrupts on, so long work delays no interrupt.
///
/// [`lock`](Self::lock) refuses trap context and a held [`SpinLock`], where a holder may never run.
/// Panic reporting uses the non-blocking [`try_lock`](Self::try_lock).
pub struct ThreadLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the value is reachable only through a guard, and `locked` admits one guard at a time.
unsafe impl<T: Send> Sync for ThreadLock<T> {}

// SAFETY: sending the lock sends the value it owns, which `T: Send` permits.
unsafe impl<T: Send> Send for ThreadLock<T> {}

impl<T> ThreadLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Acquire the lock, spinning; the guard may cover long work but never a scheduler block.
    ///
    /// # Panics
    ///
    /// Outside the panic path, from a trap handler or while this CPU holds a spin lock.
    pub fn lock(&self) -> ThreadLockGuard<'_, T> {
        assert!(
            !interrupt::in_trap() || crate::panic::is_panicking(),
            "a trap handler may not take a thread lock"
        );
        assert!(
            percpu::current().spinlocks_held() == 0 || crate::panic::is_panicking(),
            "a spin lock is held, so waiting for a thread lock could never end"
        );
        loop {
            if let Some(guard) = self.acquire() {
                return guard;
            }
            while self.is_locked() {
                spin_loop();
            }
        }
    }

    /// Acquire the lock if it is free, never waiting or checking context, so any context may.
    pub fn try_lock(&self) -> Option<ThreadLockGuard<'_, T>> {
        self.acquire()
    }

    /// The acquisition itself, with no discipline attached.
    fn acquire(&self) -> Option<ThreadLockGuard<'_, T>> {
        match self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => Some(ThreadLockGuard {
                lock: self,
                stays_here: PhantomData,
            }),
            Err(_) => None,
        }
    }

    /// Whether the lock is currently held.
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}

impl<T: fmt::Debug> fmt::Debug for ThreadLock<T> {
    /// Never blocks, for the same reason [`SpinLock`]'s does not.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_lock() {
            Some(guard) => f
                .debug_struct("ThreadLock")
                .field("value", &*guard)
                .finish(),
            None => f
                .debug_struct("ThreadLock")
                .field("value", &"<locked>")
                .finish(),
        }
    }
}

/// Exclusive access to a [`ThreadLock`]'s value; not `Send`, so it releases on the acquiring CPU.
#[derive(Debug)]
pub struct ThreadLockGuard<'lock, T> {
    lock: &'lock ThreadLock<T>,
    stays_here: PhantomData<*const ()>,
}

impl<T> Deref for ThreadLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: this guard's existence proves no other reference to the value is live, since
        // `locked` admits one guard at a time.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for ThreadLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: this guard's existence proves no other reference is live, and `&mut self` makes
        // this borrow the unique one.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for ThreadLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}
