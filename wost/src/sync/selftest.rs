use core::fmt;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

use super::{SpinLock, SpinLockGuard};
use crate::arch::current::interrupt::InterruptState;
use crate::arch::current::percpu::{self, LockKind};

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

impl<T> SpinLock<T> {
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
