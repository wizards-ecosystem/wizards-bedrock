//! Waiting on an event rather than a deadline.
//!
//! The waiter set is a bitmask: the waker runs in interrupt context and cannot allocate. This lock
//! and `THREADS` are never held together.

use wost::sync::SpinLock;

use crate::thread::{self, CAPACITY, ThreadContext};
use wost::arch::current::interrupt::TrapFrame;

const _: () = assert!(
    CAPACITY <= u32::BITS as usize,
    "the waiter set is one word wide"
);

/// A place threads wait until something wakes them.
#[derive(Debug)]
pub(crate) struct WaitQueue {
    /// One bit per thread slot: set while that thread is waiting here.
    waiters: SpinLock<u32>,
}

impl WaitQueue {
    /// A queue nobody is waiting on.
    pub(crate) const fn new() -> Self {
        Self {
            waiters: SpinLock::new(0),
        }
    }

    /// Wait until [`wake_all`](Self::wake_all) is called. The [`ThreadContext`] is what a handler
    /// cannot mint.
    ///
    /// # Panics
    ///
    /// Panics if `wost` refuses the switch: the two tables disagree.
    pub(crate) fn wait(&self, context: &mut ThreadContext) {
        self.wait_unless(context, || false);
    }

    /// Re-check `ready` under this queue's lock, register if it is still false, and only then
    /// block. [`wake_all`](Self::wake_all) takes that same lock, so it cannot observe "not ready,
    /// and not registered": either `ready` is already true, or the bit is set before any wake runs.
    pub(crate) fn wait_unless(&self, context: &mut ThreadContext, ready: impl Fn() -> bool) {
        let slot = thread::current_slot();
        let mut saved = wost::arch::current::interrupt::disable();

        let registered = {
            let mut waiters = self.waiters.lock(context);
            if ready() {
                // Nothing to wait for after all, and nothing registered — so no stale bit is left
                // behind for a later wake to trip over.
                false
            } else {
                *waiters |= 1 << slot;
                true
            }
        };

        if registered {
            thread::block(context, &mut saved);
            // Woken, or never left the CPU; either way a bit left set would name this thread in
            // someone else's wake.
            let mut waiters = self.waiters.lock(context);
            *waiters &= !(1 << slot);
        }
        saved.restore();
    }

    /// Wait through a syscall frame until an event or deadline. Registration uses the same lock
    /// order as [`wait_unless`](Self::wait_unless): `ready` is sampled under the waiter lock, and
    /// the bit is set before that lock is released.
    pub(crate) fn wait_from_trap_until(
        &self,
        frame: &TrapFrame,
        until: Option<u64>,
        ready: impl Fn() -> bool,
    ) -> bool {
        let slot = thread::current_slot();
        let saved = wost::arch::current::interrupt::disable();
        let registered = {
            let mut waiters = self.waiters.lock_leaf();
            if ready() {
                false
            } else {
                *waiters |= 1 << slot;
                true
            }
        };

        let switched = registered && thread::wait_from_trap(frame, until);
        if registered {
            *self.waiters.lock_leaf() &= !(1 << slot);
        }
        saved.restore();
        switched
    }

    /// Make every thread waiting here runnable, and report how many. Safe from a trap handler, and
    /// meant to be.
    pub(crate) fn wake_all(&self) -> usize {
        let waiting = {
            let mut waiters = self.waiters.lock_leaf();
            core::mem::take(&mut *waiters)
        };
        if waiting == 0 {
            return 0;
        }
        thread::unblock(waiting)
    }

    /// Whether anything is waiting here.
    #[cfg(feature = "selftest")]
    pub(crate) fn is_idle(&self) -> bool {
        *self.waiters.lock_leaf() == 0
    }
}
