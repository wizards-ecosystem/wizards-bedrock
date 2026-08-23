//! Waiting for something that has not happened yet.
//!
//! A [`WaitQueue`] is the other half of [`thread::sleep_ms`]: a sleep names a
//! *time* to come back at, and this names an *event*. A thread that waits
//! stops being runnable and costs nothing until something wakes it — no spin,
//! no poll, and no timer armed on its behalf, so a machine whose only
//! outstanding business is a wait takes no interrupts at all until the thing
//! it is waiting for arrives.
//!
//! **No allocation, because the waker is an interrupt handler.** The set of
//! waiters is a bitmask of thread slots living in the queue itself, so waking
//! is a scan of one word and a state change per bit — no list to splice, no
//! node to own, and nothing that could ask the heap in a context D-55 forbids
//! it in.
//!
//! **The lock order** (`docs/009-multitasking.md`) gains a second leaf rather
//! than an edge. A queue's lock and `THREADS` are never held at the same
//! time: registering as a waiter and ceasing to be runnable are two separate
//! holds, and what makes them one step is the interrupt flag, clear across
//! both, so nothing can observe the half-state between them. On one CPU that
//! is exclusion; the SMP phase owes this pair a second look.
//!
//! **The token is the proof.** [`WaitQueue::wait`] takes a
//! [`ThreadContext`], which only thread context can
//! mint — `008-research-notes-2.md` §4.2's idea, deferred by D-55 until there
//! was an API that could sleep. This is that API, and the token means a
//! handler that tries to wait does not compile rather than deadlocking the
//! machine it was interrupting.

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

    /// Wait until [`wake_all`](Self::wake_all) is called, then return.
    ///
    /// Registers this thread, blocks it, and hands the CPU on. It comes back
    /// runnable when the queue is woken, and returns when the policy picks
    /// it — which may be after the condition it woke for has been taken by
    /// someone else, so the caller re-checks. Every caller here is a loop.
    ///
    /// The interrupt flag is clear from before the registration until after
    /// the switch, so a wake cannot land between "I am a waiter" and "I am
    /// not runnable" and leave a thread blocked with nobody left to wake it.
    ///
    /// # Panics
    ///
    /// Panics if `wost` refuses the switch, which would mean the thread table
    /// and the task table disagree.
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

    /// Make every thread waiting here runnable again.
    ///
    /// Safe from a trap handler, and meant to be: it takes this queue's lock
    /// and the thread table's, in that order, and neither is ever held by
    /// code that a handler could have interrupted — D-34 clears the flag for
    /// the duration of both.
    ///
    /// Returns how many threads it woke, which is zero whenever nothing was
    /// waiting — the common case, and the reason this is cheap enough to call
    /// on every event.
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
