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
    /// A wake may be someone else's, so every caller re-checks in a loop. The
    /// [`ThreadContext`] is what a handler cannot mint (D-62).
    ///
    /// # Panics
    ///
    /// Panics if `wost` refuses the switch: the two tables disagree.
    pub(crate) fn wait(&self, context: &mut ThreadContext) {
        self.wait_unless(context, || false);
    }

    /// Register, re-check `ready`, and block only if it is still false.
    ///
    /// **Register first, then check.** The other order loses wakeups, and the
    /// loss is permanent: a waiter that reads its condition, is preempted, and
    /// is signalled before it registers, blocks on a queue nothing will wake
    /// again. `Event::wait` had exactly that shape, and the claim that a
    /// sticky signal rescued it was wrong — the state can only be re-read
    /// *after* a block returns, and it returns only on a wake (D-95).
    ///
    /// The whole of registration and the re-check happens with the interrupt
    /// flag clear, which on one CPU is what makes them one step. `002` lists
    /// the same pattern as owed before this phase for the SMP case, where the
    /// flag excludes nothing and a full barrier on both sides is what closes
    /// it (D-70 rule 6).
    ///
    /// `ready` runs with interrupts off and a queue lock held, so it must read
    /// state and nothing else.
    pub(crate) fn wait_unless(&self, context: &mut ThreadContext, ready: impl Fn() -> bool) {
        let slot = thread::current_slot();
        let saved = wost::arch::current::interrupt::disable();

        let registered = {
            let mut waiters = self.waiters.lock(context);
            if ready() {
                // Nothing to wait for after all, and nothing registered — so
                // no stale bit is left behind for a later wake to trip over.
                false
            } else {
                *waiters |= 1 << slot;
                true
            }
        };

        if registered {
            thread::block(context);
            // Woken, or returned without ever leaving the CPU because there
            // was nobody to hand it to. Either way this thread is no longer
            // waiting here, and a bit left set would name it in someone else's
            // wake.
            let mut waiters = self.waiters.lock(context);
            *waiters &= !(1 << slot);
        }
        saved.restore();
    }

    /// Register and wait through a syscall frame until an event or deadline.
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

    /// Make every thread waiting here runnable, and report how many.
    ///
    /// Safe from a trap handler, and meant to be.
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
