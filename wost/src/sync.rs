use core::cell::UnsafeCell;
use core::fmt;
use core::hint::spin_loop;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::current::interrupt::{self, InterruptState};
use crate::arch::current::percpu;
use crate::panic::PanicToken;

// Interrupting or switching away while this is nonzero can strand a lock.
pub(crate) fn spinlocks_held() -> usize {
    percpu::current().spinlocks_held()
}

// Minted only where a thread begins on its own stack; `!Send`, so no static can hold one.
#[derive(Debug)]
pub struct ThreadContext(PhantomData<*const ()>);

impl ThreadContext {
    pub(crate) fn mint() -> Self {
        assert!(
            !interrupt::in_trap(),
            "a thread context minted inside a trap"
        );
        Self(PhantomData)
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
pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the value is reachable only through a guard and `locked` admits one guard at a time,
// so access from several CPUs is sound whenever `T` may cross CPUs at all.
unsafe impl<T: Send> Sync for SpinLock<T> {}

// SAFETY: sending the lock sends the value it owns, which `T: Send` permits.
unsafe impl<T: Send> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    pub fn lock<'lock>(&'lock self, context: &'lock mut ThreadContext) -> SpinLockGuard<'lock, T> {
        let _ = context;
        loop {
            if let Some(guard) = self.acquire() {
                return guard;
            }
            while self.is_locked() {
                spin_loop();
            }
        }
    }

    // The guard may enclose only take-mutate-return work, never a call or a block.
    pub fn lock_leaf(&self) -> SpinLockGuard<'_, T> {
        loop {
            if let Some(guard) = self.acquire() {
                return guard;
            }
            while self.is_locked() {
                spin_loop();
            }
        }
    }

    pub fn try_lock_leaf(&self) -> Option<SpinLockGuard<'_, T>> {
        self.acquire()
    }

    fn acquire(&self) -> Option<SpinLockGuard<'_, T>> {
        // Disable first, or an interrupt after the exchange finds the lock held with interrupts on.
        let interrupts = interrupt::disable();
        match self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(_) => {
                percpu::current().took_lock();
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

    pub(crate) fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    // The holder's cleared interrupt state deliberately stays cleared.
    pub(crate) fn force_unlock(&self, _panic: &PanicToken) {
        self.locked.store(false, Ordering::Release);
    }
}

impl<T: fmt::Debug> fmt::Debug for SpinLock<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("SpinLock");
        match self.try_lock_leaf() {
            Some(guard) => debug.field("value", &*guard),
            None => debug.field("value", &format_args!("<locked>")),
        }
        .finish()
    }
}

pub struct SpinLockGuard<'lock, T> {
    lock: &'lock SpinLock<T>,
    interrupts: InterruptState,
}

impl<T: fmt::Debug> fmt::Debug for SpinLockGuard<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
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
    fn drop(&mut self) {
        // Release, then restore: the other order lets an interrupt reach code that holds the lock.
        percpu::current().released_lock();
        self.lock.locked.store(false, Ordering::Release);
        self.interrupts.restore_in_place();
    }
}

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

    // The guard may cover long work but never a scheduler block.
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

    pub fn try_lock(&self) -> Option<ThreadLockGuard<'_, T>> {
        self.acquire()
    }

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

    pub(crate) fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}

impl<T: fmt::Debug> fmt::Debug for ThreadLock<T> {
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
