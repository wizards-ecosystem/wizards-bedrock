//! Waiting for an event: a waiting thread is not runnable and nothing is armed for it.
//!
//! The waiter set is a bitmask of slots in the queue itself, because the waker is an interrupt
//! handler, which may not use the heap. A queue's lock and `THREADS` are never held together.
//! The interrupt flag, clear across registering and blocking, makes them one step; SMP owes this
//! pair a second look.

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

    /// Wait until [`wake_all`](Self::wake_all) is called.
    ///
    /// A wake may be someone else's, so every caller re-checks in a loop. The [`ThreadContext`] is
    /// what a handler cannot mint.
    ///
    /// # Panics
    ///
    /// Panics if `wost` refuses the switch: the two tables disagree.
    pub(crate) fn wait(&self, context: &mut ThreadContext) {
        self.wait_unless(context, || false);
    }

    /// Check `ready` under the lock `wake_all` takes, and set this thread's bit only if false.
    ///
    /// The check and the registration both happen with the interrupt flag clear, which on one CPU
    /// makes them one step. Under SMP the flag excludes nothing; full barriers on both sides must
    /// replace it.
    ///
    /// `ready` runs with interrupts off and a queue lock held, so it must read state and nothing
    /// else.
    pub(crate) fn wait_unless(&self, context: &mut ThreadContext, ready: impl Fn() -> bool) {
        let slot = thread::current_slot();
        let mut saved = wost::arch::current::interrupt::disable();

        let registered = {
            let mut waiters = self.waiters.lock(context);
            if ready() {
                // Nothing registered, so no stale bit is left for a later wake to trip over.
                false
            } else {
                *waiters |= 1 << slot;
                true
            }
        };

        if registered {
            thread::block(context, &mut saved);
            // Woken or not, a bit left set would name this thread in someone else's wake.
            let mut waiters = self.waiters.lock(context);
            *waiters &= !(1 << slot);
        }
        saved.restore();
    }

    /// Check `ready` under the lock `wake_all` takes and, only if false, wait through a syscall
    /// frame until a wake or `until`, reporting whether this thread switched away.
    ///
    /// `ready` has the same atomic-only contract as [`wait_unless`](Self::wait_unless).
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

    /// Make every thread waiting here runnable, and report how many; safe from a trap handler.
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
