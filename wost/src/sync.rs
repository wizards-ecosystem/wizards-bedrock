//! Synchronisation primitives.

use core::cell::UnsafeCell;
use core::fmt;
use core::hint::spin_loop;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::interrupt::{self, InterruptState};
// The counters and the leaf-violation record live here now: they were
// `static`s, which on one processor is indistinguishable from per-CPU and
// wrong the moment a second starts (D-154).
use crate::arch::current::percpu;
use crate::panic::PanicToken;

/// How many [`SpinLock`]s this CPU is holding.
/// How many spin locks this CPU holds.
///
/// For the two invariants that read it from outside this module: setting the
/// interrupt flag with one held, and switching threads with one held, are both
/// ways to strand a lock on a context that can never resume (D-66). The borrow
/// checker rules out both statically now (D-69); this stays as the backstop
/// for the paths it cannot see.
pub fn spinlocks_held() -> usize {
    percpu::current().spinlocks_held()
}

/// Where a lock was taken while a leaf guard was held, if anywhere.
///
/// `None` on a boot where the obligation held everywhere. This is the check
/// that `pic::silence` needed and did not have: it took `CONTROLLERS`' leaf
/// guard and claimed ports under it, which takes `CLAIMED`'s — an order
/// `CONTROLLERS → CLAIMED` established by accident, which is the kind that
/// closes a cycle later (D-124).
pub fn taken_under_a_leaf() -> Option<&'static core::panic::Location<'static>> {
    percpu::current().taken_under_a_leaf()
}

/// The leaf guard a violation happened under, if there was one.
pub fn the_leaf_it_was_under() -> Option<&'static core::panic::Location<'static>> {
    percpu::current().the_leaf_it_was_under()
}

/// Evidence that the caller is in thread context, not inside a trap handler.
///
/// D-62 introduced it as the token every call that can block takes. D-69 gave
/// it a second job: [`SpinLock::lock`] borrows one for the guard's life, so
/// blocking with a guard held, or taking a second lock **through `lock`**, is a
/// borrow error. Neither `Send` nor `Sync`, so it cannot be stashed in a
/// `static` and produced later.
///
/// **It is a strong default, not a proof, and the difference is D-94's.** The
/// prose used to say a second lock is `E0499` full stop. It is not:
/// [`ThreadContext::claim`] mints a token wherever the interrupt flag says
/// thread context, so a second one can be minted inside a live guard — and
/// [`SpinLock::lock_leaf`] needs no token at all. What the borrow buys is that
/// defeating it takes one visible, greppable line, audited the way an `unsafe`
/// block is.
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
    /// `&mut self` is the proof: no other reference exists, so no other thread
    /// can be holding the guard, so there is nothing to wait for. The standard
    /// library's `Mutex::get_mut` is the same argument.
    ///
    /// Its reason for existing here is narrower than convenience. A caller
    /// that must pass the value to something which *allocates* cannot hold a
    /// leaf guard across the call (D-85) — and copying the value out instead
    /// means handing on a bare number where a type was the evidence (D-115).
    /// This is the third option: no guard, and the type intact.
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
    /// The wait runs with the interrupt flag as the caller left it. Spinning
    /// with interrupts off would mean waiting for a holder that cannot be
    /// scheduled and cannot be interrupted, which on one CPU is a hang and on
    /// several is merely slow.
    ///
    /// The guard borrows `context` for its life, which is what makes calling
    /// anything that can block — or taking a second lock *through this same
    /// method* — a borrow error while this one is held (D-69). It does not make
    /// a second lock impossible: `claim` and [`Self::lock_leaf`] both reach one
    /// without the token, which D-94 corrected the prose about. Where a nesting
    /// is genuinely wanted, the caller drops this guard, acquires what it
    /// needs, and re-checks.
    pub fn lock<'a>(&'a self, context: &'a mut ThreadContext) -> SpinLockGuard<'a, T> {
        // The discipline is the signature: `'a` ties this borrow to the
        // guard's life, so the token is unavailable until the guard drops.
        let _ = context;
        loop {
            if let Some(guard) = self.acquire(false) {
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
        self.acquire(false)
    }

    /// Acquire the lock for an operation that is over before it returns.
    ///
    /// The second of this kernel's two lock levels. A *leaf* lock guards data
    /// whose every acquisition is take-mutate-return: no call is made while
    /// the guard is live, so no second lock can be taken and nothing can
    /// block. That makes a token meaningless — there is nothing for it to
    /// exclude — and it is the only acquisition a `Drop` impl can perform,
    /// since `drop` takes no arguments and a freed `Frame` must reach the
    /// allocator from wherever it dies.
    ///
    /// The obligation is on the *body*, not the caller, and it is short enough
    /// to read: **nothing is called while this guard is live.** A leaf lock
    /// may therefore be taken while a token-bearing lock is held, which is
    /// what makes D-47's `HEAP` → `FRAMES` order sound rather than an
    /// exception to D-69.
    #[track_caller]
    pub fn lock_leaf(&self) -> SpinLockGuard<'_, T> {
        loop {
            if let Some(guard) = self.acquire(true) {
                return guard;
            }
            while self.is_locked() {
                spin_loop();
            }
        }
    }

    /// Acquire a leaf lock if it is free, without spinning.
    #[track_caller]
    pub fn try_lock_leaf(&self) -> Option<SpinLockGuard<'_, T>> {
        self.acquire(true)
    }

    /// Acquire the lock if it is free, from a trap handler or the panic path.
    ///
    /// Context-free because it cannot wait, which is what a handler needs and
    /// what a `Debug` impl that must never block needs too.
    #[track_caller]
    pub fn try_lock_from_trap(&self) -> Option<SpinLockGuard<'_, T>> {
        self.acquire(true)
    }

    /// The acquisition itself, with no discipline attached.
    #[track_caller]
    fn acquire(&self, leaf: bool) -> Option<SpinLockGuard<'_, T>> {
        // Disabling first: taking an interrupt between the exchange and the
        // disable would leave this CPU holding the lock with interrupts on,
        // which is the state the whole design exists to rule out.
        let interrupts = interrupt::disable();
        match self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => {
                // One call, because the answer is about the state *before*
                // this acquisition and a caller splitting it into a read and a
                // write would have to get the order right (D-154).
                percpu::current().took_lock(leaf, core::panic::Location::caller());
                Some(SpinLockGuard {
                    lock: self,
                    leaf,
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
    /// The panic handler needs this to report through a device whose lock the
    /// code it interrupted was holding. It takes no `unsafe` because a
    /// [`PanicToken`] is itself the proof that makes it sound: the previous
    /// holder can never resume, so the guard it may still own is never used
    /// again.
    ///
    /// The interrupt flag is left alone. Obtaining a [`PanicToken`] cleared it,
    /// and the machine is not going to need it back.
    pub(crate) fn force_unlock(&self, _panic: &PanicToken) {
        self.locked.store(false, Ordering::Release);
    }
}

impl<T: fmt::Debug> fmt::Debug for SpinLock<T> {
    /// Never blocks: a `Debug` impl that could spin would hang the one context
    /// most likely to be printing it, the panic handler.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("SpinLock");
        match self.try_lock_from_trap() {
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
    /// Whether this came from [`SpinLock::lock_leaf`], so its release knows
    /// which counter to undo.
    leaf: bool,
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
    /// This is the compile/runtime witness for D-194: it calls the same safe
    /// `core::mem::forget` available to any caller, but first transfers the
    /// interrupt state to an owner which cannot dereference the protected
    /// value. It exists only in a self-test image.
    #[cfg(feature = "selftest")]
    pub fn forget_for_test(mut self) -> ForgottenSpinLockGuard<'a, T> {
        let interrupts = core::mem::replace(&mut self.interrupts, interrupt::disable());
        let recovery = ForgottenSpinLockGuard {
            lock: self.lock,
            leaf: self.leaf,
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
        percpu::current().released_lock(self.leaf);
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
    leaf: bool,
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
        percpu::current().released_lock(self.leaf);
        self.lock.locked.store(false, Ordering::Release);
        self.interrupts.restore_in_place();
    }
}

/// A mutual-exclusion lock that spins while contended, held with interrupts
/// **on** — and refuses to be taken from a trap handler.
///
/// The lock D-34 said would earn its keep once there was a scheduler, and
/// D-63 gave to the console: a `SpinLock` makes every interrupt on the
/// machine wait behind its longest holder, and for the console that holder is
/// a scroll.
///
/// It excludes nothing but other holders, so [`lock`](Self::lock) asserts two
/// things. Not trap context, or a handler could deadlock against the code it
/// interrupted — which makes D-31 a panic rather than prose. And no
/// `SpinLock` held, or the waiter has interrupts off and the preempted holder
/// is never scheduled again.
///
/// A holder can be preempted, which a `SpinLock` holder cannot. That costs a
/// waiter the rest of its slice and no more, because the timer takes the CPU
/// back and gives it to the holder.
///
/// The panic path is exempt from both: the code it interrupted never resumes.
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
    /// A leaf acquisition, as [`SpinLock::lock_leaf`] is: the caller does not
    /// block while holding it. There is no token-bearing form because the
    /// guard itself cannot be carried through a scheduler block. Holders may
    /// still perform long, non-blocking work while interrupts and preemption
    /// remain available to the machine (D-200).
    ///
    /// # Panics
    ///
    /// If called from a trap handler, or with a spin lock held, outside the
    /// panic path. A handler that took this lock could be the interrupt that
    /// landed on the holder and would spin for ever against itself; and a
    /// waiter holding a spin lock has interrupts off, so the preempted holder
    /// is never scheduled again.
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
    /// Context-free because it cannot wait: a handler that finds the lock
    /// taken is told so and moves on, which is the one way a handler may
    /// touch one of these at all. It is also what a `Debug` impl needs, and
    /// what the panic path reports through.
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
