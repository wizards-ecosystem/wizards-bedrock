//! Synchronisation primitives.

use core::cell::UnsafeCell;
use core::fmt;
use core::hint::spin_loop;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::interrupt::{self, InterruptState};
// Lock state is CPU-local so processors cannot share nesting evidence (D-154).
use crate::arch::current::percpu;
#[cfg(feature = "selftest")]
use crate::arch::current::percpu::LockKind;
use crate::panic::PanicToken;

/// How many spin locks this CPU holds.
///
/// Interrupting or switching away while this is nonzero can strand a lock;
/// the counter is the runtime backstop for paths type borrows cannot see.
pub(crate) fn spinlocks_held() -> usize {
    percpu::current().spinlocks_held()
}

/// Where a lock was taken while a leaf guard was held, if anywhere.
///
/// `None` when the leaf obligation held. Both locations expose the forbidden
/// ordering edge instead of reducing it to a count.
#[cfg(feature = "selftest")]
pub fn taken_under_a_leaf() -> Option<&'static core::panic::Location<'static>> {
    percpu::current().taken_under_a_leaf()
}

/// The leaf guard a violation happened under, if there was one.
#[cfg(feature = "selftest")]
pub fn the_leaf_it_was_under() -> Option<&'static core::panic::Location<'static>> {
    percpu::current().the_leaf_it_was_under()
}

/// Evidence that the caller is in thread context, not inside a trap handler.
///
/// Blocking APIs and [`SpinLock::lock`] borrow it, so a guard and another
/// token-using operation cannot coexist. It is neither `Send` nor `Sync`.
/// [`Self::claim`] can mint another token and leaf locks need none, so this is
/// a visible default discipline rather than unforgeable evidence (D-94).
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
    /// From trap context, where a block would carry the handler's stack frame
    /// away with it (D-55).
    pub fn require() -> Self {
        match Self::claim() {
            Some(context) => context,
            None => panic!("thread context required, but a handler is on the stack"),
        }
    }
}

/// A latch that admits exactly one caller.
///
/// Makes hardware `init` idempotent so a second caller cannot reload a table
/// the CPU is already using.
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
/// Clearing interrupts prevents same-CPU handler re-entry; the guard restores
/// the prior flag so nesting remains valid. Fault and NMI handlers must still
/// not lock. Long holders delay interrupts and belong behind [`ThreadLock`].
pub struct SpinLock<T> {
    locked: AtomicBool,
    #[cfg(feature = "selftest")]
    forgotten_for_test: AtomicBool,
    value: UnsafeCell<T>,
}

/// The observable ownership state of a spin lock in a machine-test image.
///
/// This exists only to distinguish a live ordinary holder from the controlled
/// forgotten-guard witness in D-194. Shipped code has neither this state nor
/// the operation that creates [`Self::Forgotten`].
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinLockWitness {
    /// No guard owns the lock.
    Free,
    /// A guard owns the lock normally.
    Contended,
    /// The controlled witness deliberately forgot the original guard.
    Forgotten,
}

// SAFETY: `SpinLock` hands out a reference to its value only through a guard,
// and only one guard can exist at a time because `locked` gates acquisition.
// That makes concurrent access from several CPUs sound whenever `T` may cross
// CPUs at all.
unsafe impl<T: Send> Sync for SpinLock<T> {}

// SAFETY: sending the lock sends the value it owns, which `T: Send` permits.
unsafe impl<T: Send> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// Reach the value without locking, given exclusive access to the lock.
    ///
    /// The exclusive borrow proves no guard exists. This preserves typed
    /// evidence across allocating code without holding a leaf guard (D-85,
    /// D-115).
    pub const fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    /// Create an unlocked `SpinLock`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            #[cfg(feature = "selftest")]
            forgotten_for_test: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Acquire the lock, spinning until it is free.
    ///
    /// The wait keeps the caller's interrupt state so a holder can run. The
    /// guard borrows `context`, excluding blocking and another lock through the
    /// same token; [`ThreadContext::claim`] and leaf locks remain explicit
    /// escape points (D-94).
    pub fn lock<'a>(&'a self, context: &'a mut ThreadContext) -> SpinLockGuard<'a, T> {
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
    pub fn try_lock<'a>(&'a self, context: &'a mut ThreadContext) -> Option<SpinLockGuard<'a, T>> {
        let _ = context;
        self.acquire(
            #[cfg(feature = "selftest")]
            LockKind::Ordered,
        )
    }

    /// Acquire the lock for an operation that is over before it returns.
    ///
    /// Its guard may enclose only take-mutate-return work with no calls or
    /// blocking. Requiring no context makes it usable from `Drop` and permits
    /// the leaf-under-token hierarchy (D-47, D-121).
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
        // Disable first: an interrupt between the exchange and the disable
        // would leave this CPU holding the lock with interrupts on.
        let interrupts = interrupt::disable();
        match self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => {
                // One call: the answer is about the state before this
                // acquisition, and a split read and write would have to order it (D-154).
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

    /// Classify this lock for D-194's controlled machine witness.
    #[cfg(feature = "selftest")]
    pub fn witness_for_test(&self) -> SpinLockWitness {
        if self.forgotten_for_test.load(Ordering::Acquire) {
            SpinLockWitness::Forgotten
        } else if self.is_locked() {
            SpinLockWitness::Contended
        } else {
            SpinLockWitness::Free
        }
    }

    /// Release the lock regardless of who holds it.
    ///
    /// [`PanicToken`] proves the interrupted holder never resumes. Its cleared
    /// interrupt state deliberately remains cleared.
    pub(crate) fn force_unlock(&self, _panic: &PanicToken) {
        self.locked.store(false, Ordering::Release);
    }
}

impl<T: fmt::Debug> fmt::Debug for SpinLock<T> {
    /// Never blocks: a `Debug` impl that could spin would hang the one context
    /// most likely to be printing it, the panic handler.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("SpinLock");
        match self.try_lock_leaf() {
            Some(guard) => out.field("value", &*guard),
            None => out.field("value", &format_args!("<locked>")),
        }
        .finish()
    }
}

/// Exclusive access to a [`SpinLock`]'s value, released on drop.
///
/// Not `Send`: the flag this guard restores belongs to the CPU that cleared
/// it, so releasing on another one would leave the first interruptible inside
/// its own critical section. [`InterruptState`] carries that fact for every
/// owner of the state rather than making each owner repeat it (D-68, D-223).
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
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

impl<'a, T> SpinLockGuard<'a, T> {
    /// Deliberately forget this guard while retaining test-only recovery.
    ///
    /// Transfers interrupt state to a non-dereferencing recovery owner before
    /// using safe `core::mem::forget`; available only for D-194's self-test.
    #[cfg(feature = "selftest")]
    pub fn forget_for_test(mut self) -> ForgottenSpinLockGuard<'a, T> {
        let interrupts = self.interrupts.take_for_transfer();
        let recovery = ForgottenSpinLockGuard {
            lock: self.lock,
            kind: self.kind,
            interrupts,
            stays_here: PhantomData,
        };
        self.lock.forgotten_for_test.store(true, Ordering::Release);
        core::mem::forget(self);
        recovery
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    /// Release, then restore. The other order would open a window in which an
    /// interrupt could arrive at code still holding this lock.
    fn drop(&mut self) {
        percpu::current().released_lock(
            #[cfg(feature = "selftest")]
            self.kind,
        );
        self.lock.locked.store(false, Ordering::Release);
        self.interrupts.restore_in_place();
    }
}

/// Test-only ownership which recovers a deliberately forgotten spin guard.
///
/// It deliberately exposes no reference to the protected value. Dropping it
/// performs the one release the consumed original guard can no longer perform.
#[cfg(feature = "selftest")]
#[must_use = "forgetting the recovery owner would deliberately strand the test kernel"]
pub struct ForgottenSpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
    kind: LockKind,
    interrupts: InterruptState,
    stays_here: PhantomData<*const ()>,
}

#[cfg(feature = "selftest")]
impl<T> fmt::Debug for ForgottenSpinLockGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForgottenSpinLockGuard")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "selftest")]
impl<T> Drop for ForgottenSpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.forgotten_for_test.store(false, Ordering::Release);
        percpu::current().released_lock(self.kind);
        self.lock.locked.store(false, Ordering::Release);
        self.interrupts.restore_in_place();
    }
}

/// A mutual-exclusion lock that spins while contended, held with interrupts
/// **on** — and refuses to be taken from a trap handler.
///
/// A holder may be preempted, so long non-blocking work does not delay every
/// interrupt. [`lock`](Self::lock) refuses trap context and a held [`SpinLock`]
/// because either waiter could depend on code that cannot run. Panic reporting
/// uses the non-blocking [`try_lock`](Self::try_lock) path.
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
    /// The guard may cover long work but cannot cross a scheduler block. It is
    /// therefore a leaf acquisition with interrupts and preemption available
    /// to the machine (D-200).
    ///
    /// # Panics
    ///
    /// Outside the panic path, if called from a trap handler or while this CPU
    /// holds a spin lock; either context can prevent the holder from running.
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

    /// Acquire the lock if it is free, without spinning or checking context.
    ///
    /// It is safe for handlers, diagnostics and panic reporting because it
    /// never waits.
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
///
/// Not `Send`, as [`SpinLockGuard`] is not: a lock acquired on one CPU is
/// released on that one (D-68).
#[derive(Debug)]
pub struct ThreadLockGuard<'a, T> {
    lock: &'a ThreadLock<T>,
    stays_here: PhantomData<*const ()>,
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
