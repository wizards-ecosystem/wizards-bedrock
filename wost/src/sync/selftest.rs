use core::fmt;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use super::{SpinLock, SpinLockGuard};
use crate::arch::current::interrupt::InterruptState;
use crate::arch::current::percpu::{self, LockKind};

/// Where a lock was taken while a leaf guard was held, or `None` when the leaf obligation held.
#[cfg(feature = "selftest")]
pub fn taken_under_a_leaf() -> Option<&'static core::panic::Location<'static>> {
    percpu::current().taken_under_a_leaf()
}

/// The leaf guard a violation happened under, if there was one.
#[cfg(feature = "selftest")]
pub fn the_leaf_it_was_under() -> Option<&'static core::panic::Location<'static>> {
    percpu::current().the_leaf_it_was_under()
}

/// A spin lock's ownership in a machine-test image, telling a live holder from a forgotten guard.
///
/// Shipped code has neither this state nor the operation that creates [`Self::Forgotten`].
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpinLockWitness {
    Free,
    /// A guard owns the lock normally.
    Contended,
    /// The controlled witness deliberately forgot the original guard.
    Forgotten,
}

impl<T> SpinLock<T> {
    /// Classify this lock for the controlled forgotten-guard machine witness.
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
}

impl<'lock, T> SpinLockGuard<'lock, T> {
    /// Forget this guard after moving its interrupt state to a non-dereferencing recovery owner.
    #[cfg(feature = "selftest")]
    pub fn forget_for_test(mut self) -> ForgottenSpinLockGuard<'lock, T> {
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

/// Test-only owner of a forgotten spin guard, exposing no reference to the protected value.
///
/// Dropping it performs the one release the consumed original guard can no longer perform.
#[cfg(feature = "selftest")]
#[must_use = "forgetting the recovery owner would deliberately strand the test kernel"]
pub struct ForgottenSpinLockGuard<'lock, T> {
    lock: &'lock SpinLock<T>,
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
