use core::fmt;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicBool, Ordering};

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

impl<T> SpinLock<T> {}

impl<'lock, T> SpinLockGuard<'lock, T> {}

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

/// A machine-test fault-injection switch, unarmed until a check holds it for one call.
#[cfg(feature = "selftest")]
#[derive(Debug)]
pub struct Injection(AtomicBool);

#[cfg(feature = "selftest")]
impl Injection {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// Whether a running check holds this switch armed.
    pub fn armed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Run `check` with the switch set to `value`, then restore the setting it replaced.
    pub fn with<T>(&self, value: bool, check: impl FnOnce() -> T) -> T {
        let previous = self.0.swap(value, Ordering::AcqRel);
        let outcome = check();
        self.0.store(previous, Ordering::Release);
        outcome
    }
}

#[cfg(feature = "selftest")]
impl Default for Injection {
    fn default() -> Self {
        Self::new()
    }
}
