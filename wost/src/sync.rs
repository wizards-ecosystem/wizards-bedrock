use core::cell::UnsafeCell;
use core::fmt;
use core::hint::spin_loop;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use lock_order::{HeldRanks, LEAF, LockBefore, Unlocked};
pub use lock_order::{Level, impl_lock_after};

use crate::arch::current::interrupt::{self, InterruptState};
use crate::arch::current::{percpu, task};
use crate::panic::PanicToken;

#[derive(Debug)]
pub enum Leaf {}

impl Level for Leaf {
    const RANK: u8 = LEAF;
}

impl_lock_after!(Unlocked => Leaf);

// Interrupting or switching away while this holds can strand a lock.
pub(crate) fn holds_spin_lock() -> bool {
    percpu::current().held_ranks() != HeldRanks::empty()
}

pub(crate) fn holds_leaf() -> bool {
    percpu::current().held_ranks().take(LEAF).is_none()
}

// Thread locks move with their thread when it is preempted, so their ranks are kept per thread.
fn thread_ranks() -> &'static AtomicU64 {
    task::current_ranks()
}

pub(crate) fn forget_held_ranks(_panic: &PanicToken) {
    percpu::current().set_held_ranks(HeldRanks::empty());
    thread_ranks().store(0, Ordering::Relaxed);
}

fn admit(held: HeldRanks, rank: u8, leaf: bool) -> Option<HeldRanks> {
    let held = held.take(rank)?;
    if leaf && rank != LEAF {
        held.take(LEAF)
    } else {
        Some(held)
    }
}

fn release(held: HeldRanks, rank: u8, leaf: bool) -> Option<HeldRanks> {
    let held = if leaf && rank != LEAF {
        held.release(LEAF)?
    } else {
        held
    };
    held.release(rank)
}

// Minted only where a thread begins on its own stack; `!Send`, so no static can hold one.
pub struct Held<L> {
    level: PhantomData<fn() -> L>,
    stays_here: PhantomData<*const ()>,
}

pub type ThreadContext = Held<Unlocked>;

impl ThreadContext {
    pub(crate) fn mint() -> Self {
        assert!(
            !interrupt::in_trap(),
            "a thread context minted inside a trap"
        );
        Self {
            level: PhantomData,
            stays_here: PhantomData,
        }
    }
}

impl<L> fmt::Debug for Held<L> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Held")
    }
}

#[derive(Debug)]
pub struct Once(AtomicBool);

impl Once {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    pub fn claim(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }
}

impl Default for Once {
    fn default() -> Self {
        Self::new()
    }
}

// Held with interrupts off; fault and NMI handlers must never lock.
pub struct SpinLock<L, T> {
    locked: AtomicBool,
    level: PhantomData<fn() -> L>,
    value: UnsafeCell<T>,
}

// SAFETY: the value is reachable only through a guard and `locked` admits one guard at a time,
// so access from several CPUs is sound whenever `T` may cross CPUs at all.
unsafe impl<L, T: Send> Sync for SpinLock<L, T> {}

// SAFETY: sending the lock sends the value it owns, which `T: Send` permits.
unsafe impl<L, T: Send> Send for SpinLock<L, T> {}

impl<L, T> SpinLock<L, T> {
    pub const fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            level: PhantomData,
            value: UnsafeCell::new(value),
        }
    }

    pub(crate) fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    // The holder's cleared interrupt state deliberately stays cleared.
    pub(crate) fn force_unlock(&self, _panic: &PanicToken) {
        self.locked.store(false, Ordering::Release);
    }
}

impl<L: Level, T> SpinLock<L, T> {
    pub fn lock<'lock, M: LockBefore<L>>(
        &'lock self,
        context: &'lock mut Held<M>,
    ) -> SpinLockGuard<'lock, L, T> {
        let _ = context;
        self.spin(false)
    }

    // Takes the rank alone, with no token, for scheduler locks that trap code takes and leaves nest
    // under; the rank word orders it, and a switch or flush wait under any spin lock panics.
    pub fn lock_ranked(&self) -> SpinLockGuard<'_, L, T> {
        self.spin(false)
    }

    // The guard may enclose only take-mutate-return work, never a call or a block.
    pub fn lock_leaf(&self) -> SpinLockGuard<'_, L, T> {
        self.spin(true)
    }

    pub fn try_lock_leaf(&self) -> Option<SpinLockGuard<'_, L, T>> {
        self.acquire(true).ok().flatten()
    }

    fn spin(&self, leaf: bool) -> SpinLockGuard<'_, L, T> {
        loop {
            match self.acquire(leaf) {
                Ok(Some(guard)) => return guard,
                Ok(None) => {
                    while self.is_locked() {
                        spin_loop();
                    }
                }
                Err(held) => panic!(
                    "lock order: rank {} taken while holding ranks {:#x}",
                    L::RANK,
                    held.bits()
                ),
            }
        }
    }

    fn acquire(&self, leaf: bool) -> Result<Option<SpinLockGuard<'_, L, T>>, HeldRanks> {
        // Disable first, or an interrupt after the exchange finds the lock held with interrupts on.
        let interrupts = interrupt::disable();
        let percpu = percpu::current();
        let held = percpu.held_ranks();
        let Some(next) = admit(held, L::RANK, leaf) else {
            interrupts.restore();
            return Err(held);
        };
        match self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => {
                percpu.set_held_ranks(next);
                Ok(Some(SpinLockGuard {
                    lock: self,
                    interrupts,
                    leaf,
                }))
            }
            Err(_) => {
                interrupts.restore();
                Ok(None)
            }
        }
    }
}

impl<L: Level, T: fmt::Debug> fmt::Debug for SpinLock<L, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("SpinLock");
        match self.try_lock_leaf() {
            Some(guard) => debug.field("value", &*guard),
            None => debug.field("value", &format_args!("<locked>")),
        }
        .finish()
    }
}

pub struct SpinLockGuard<'lock, L: Level, T> {
    lock: &'lock SpinLock<L, T>,
    interrupts: InterruptState,
    leaf: bool,
}

impl<L: Level, T: fmt::Debug> fmt::Debug for SpinLockGuard<'_, L, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

impl<L: Level, T> Deref for SpinLockGuard<'_, L, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: holding this guard means we hold the lock, so no other
        // reference to the value exists.
        unsafe { &*self.lock.value.get() }
    }
}

impl<L: Level, T> DerefMut for SpinLockGuard<'_, L, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: holding this guard means we hold the lock, so no other
        // reference to the value exists.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<L: Level, T> Drop for SpinLockGuard<'_, L, T> {
    fn drop(&mut self) {
        let percpu = percpu::current();
        let Some(held) = release(percpu.held_ranks(), L::RANK, self.leaf) else {
            panic!("lock order: rank {} released but not held", L::RANK);
        };
        // Release, then restore: the other order lets an interrupt reach code that holds the lock.
        percpu.set_held_ranks(held);
        self.lock.locked.store(false, Ordering::Release);
        self.interrupts.restore_in_place();
    }
}

// Interrupts off and the rank held for one bounded step, so no block or lower lock runs under it.
pub struct PinGuard<L: Level> {
    interrupts: InterruptState,
    level: PhantomData<fn() -> L>,
    stays_here: PhantomData<*const ()>,
}

impl<L: Level> PinGuard<L> {
    pub fn take() -> Self {
        let interrupts = interrupt::disable();
        let percpu = percpu::current();
        let held = percpu.held_ranks();
        let Some(next) = admit(held, L::RANK, false) else {
            panic!(
                "lock order: pin rank {} taken while holding ranks {:#x}",
                L::RANK,
                held.bits()
            );
        };
        percpu.set_held_ranks(next);
        Self {
            interrupts,
            level: PhantomData,
            stays_here: PhantomData,
        }
    }
}

impl<L: Level> fmt::Debug for PinGuard<L> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PinGuard")
    }
}

impl<L: Level> Drop for PinGuard<L> {
    fn drop(&mut self) {
        let percpu = percpu::current();
        let Some(held) = release(percpu.held_ranks(), L::RANK, false) else {
            panic!("lock order: pin rank {} released but not held", L::RANK);
        };
        percpu.set_held_ranks(held);
        self.interrupts.restore_in_place();
    }
}

pub struct ThreadLock<L, T> {
    locked: AtomicBool,
    level: PhantomData<fn() -> L>,
    value: UnsafeCell<T>,
}

// SAFETY: the value is reachable only through a guard, and `locked` admits one guard at a time.
unsafe impl<L, T: Send> Sync for ThreadLock<L, T> {}

// SAFETY: sending the lock sends the value it owns, which `T: Send` permits.
unsafe impl<L, T: Send> Send for ThreadLock<L, T> {}

impl<L, T> ThreadLock<L, T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            level: PhantomData,
            value: UnsafeCell::new(value),
        }
    }

    pub(crate) fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}

impl<L: Level, T> ThreadLock<L, T> {
    // The guard may cover long work but never a scheduler block.
    pub fn lock(&self) -> ThreadLockGuard<'_, L, T> {
        assert!(
            !interrupt::in_trap() || crate::panic::is_panicking(),
            "a trap handler may not take a thread lock"
        );
        assert!(
            !holds_spin_lock() || crate::panic::is_panicking(),
            "a spin lock is held, so waiting for a thread lock could never end"
        );
        loop {
            match self.acquire() {
                Ok(Some(guard)) => return guard,
                Ok(None) => {
                    while self.is_locked() {
                        spin_loop();
                    }
                }
                Err(held) => panic!(
                    "lock order: thread rank {} taken while holding ranks {:#x}",
                    L::RANK,
                    held.bits()
                ),
            }
        }
    }

    pub fn try_lock(&self) -> Option<ThreadLockGuard<'_, L, T>> {
        self.acquire().ok().flatten()
    }

    fn acquire(&self) -> Result<Option<ThreadLockGuard<'_, L, T>>, HeldRanks> {
        let ranks = thread_ranks();
        let held = HeldRanks::from_bits(ranks.load(Ordering::Relaxed));
        let Some(next) = held.take(L::RANK) else {
            return Err(held);
        };
        match self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => {
                ranks.store(next.bits(), Ordering::Relaxed);
                Ok(Some(ThreadLockGuard {
                    lock: self,
                    stays_here: PhantomData,
                }))
            }
            Err(_) => Ok(None),
        }
    }
}

impl<L: Level, T: fmt::Debug> fmt::Debug for ThreadLock<L, T> {
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

pub struct ThreadLockGuard<'lock, L: Level, T> {
    lock: &'lock ThreadLock<L, T>,
    stays_here: PhantomData<*const ()>,
}

impl<L: Level, T: fmt::Debug> fmt::Debug for ThreadLockGuard<'_, L, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

impl<L: Level, T> Deref for ThreadLockGuard<'_, L, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: this guard's existence proves no other reference to the value is live, since
        // `locked` admits one guard at a time.
        unsafe { &*self.lock.value.get() }
    }
}

impl<L: Level, T> DerefMut for ThreadLockGuard<'_, L, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: this guard's existence proves no other reference is live, and `&mut self` makes
        // this borrow the unique one.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<L: Level, T> Drop for ThreadLockGuard<'_, L, T> {
    fn drop(&mut self) {
        let ranks = thread_ranks();
        let Some(held) = HeldRanks::from_bits(ranks.load(Ordering::Relaxed)).release(L::RANK)
        else {
            panic!("lock order: thread rank {} released but not held", L::RANK);
        };
        ranks.store(held.bits(), Ordering::Relaxed);
        self.lock.locked.store(false, Ordering::Release);
    }
}
