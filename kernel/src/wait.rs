use wost::arch::current::interrupt::TrapFrame;
use wost::arch::current::task;
use wost::sync::SpinLock;

use crate::thread::{self, CAPACITY, ThreadContext};

const _: () = assert!(
    CAPACITY <= u32::BITS as usize,
    "the waiter set is one word wide"
);

// A queue's lock and `THREADS` are never held together.
#[derive(Debug)]
pub(crate) struct WaitQueue {
    waiters: SpinLock<u32>,
}

impl WaitQueue {
    pub(crate) const fn new() -> Self {
        Self {
            waiters: SpinLock::new(0),
        }
    }

    // `ready` runs with interrupts off under a queue lock, so it must only read state.
    pub(crate) fn wait_unless(&self, context: &mut ThreadContext, ready: impl Fn() -> bool) {
        let slot = task::current();
        // Off from the check to the block, which makes them one step on one CPU.
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

    // `ready` must only read state, as for `wait_unless`.
    pub(crate) fn wait_from_trap_until(
        &self,
        frame: &TrapFrame,
        until: Option<u64>,
        ready: impl Fn() -> bool,
    ) -> bool {
        let slot = task::current();
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

    // Safe from a trap handler.
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
}
