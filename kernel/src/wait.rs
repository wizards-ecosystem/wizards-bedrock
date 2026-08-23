//! Waiting for something that has not happened yet.
//!
//! A [`WaitQueue`] is the other half of `thread::sleep_ms`: a sleep names a
//! time to come back at, this names an event. A waiting thread is not
//! runnable and nothing is armed for it, so a machine whose only outstanding
//! business is a wait takes no interrupts until the event arrives.
//!
//! The waiter set is a bitmask of slots in the queue itself: the waker is an
//! interrupt handler, where D-55 forbids the heap.
//!
//! A queue's lock and `THREADS` are never held together. What makes
//! registering and blocking one step is the interrupt flag, clear across
//! both; SMP owes this pair a second look.

use wost::panic::PanicToken;
use wost::sync::SpinLock;

use crate::thread::{self, CAPACITY, ThreadContext};

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
    /// A wake may be someone else's, so every caller re-checks in a loop. The
    /// [`ThreadContext`] is what a handler cannot mint (D-62).
    ///
    /// # Panics
    ///
    /// Panics if `wost` refuses the switch: the two tables disagree.
    pub(crate) fn wait(&self, context: &ThreadContext) {
        let slot = thread::current_slot();
        let saved = wost::arch::current::interrupt::disable();
        {
            let mut waiters = self.waiters.lock();
            *waiters |= 1 << slot;
        }
        thread::block(context);
        saved.restore();
    }

    /// Make every thread waiting here runnable, and report how many.
    ///
    /// Safe from a trap handler, and meant to be.
    pub(crate) fn wake_all(&self) -> usize {
        let waiting = {
            let mut waiters = self.waiters.lock();
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
        *self.waiters.lock() == 0
    }

    /// Reclaim this queue's lock on the terminal panic path.
    pub(crate) fn force_unlock(&self, token: &PanicToken) {
        self.waiters.force_unlock(token);
    }
}
