use core::sync::atomic::AtomicUsize;

use crate::thread::{self, NO_WAITER, ThreadContext};

// Waiters are thread slots linked under the scheduler lock, so a wake allocates nothing.
#[derive(Debug)]
pub(crate) struct WaitQueue {
    head: AtomicUsize,
}

impl WaitQueue {
    pub(crate) const fn new() -> Self {
        Self {
            head: AtomicUsize::new(NO_WAITER),
        }
    }

    // `ready` runs under the scheduler lock, so it must only read state.
    pub(crate) fn wait_unless(&self, context: &mut ThreadContext, ready: impl Fn() -> bool) {
        thread::wait_on(context, &self.head, None, ready);
    }

    // As `wait_unless`, but the wait also ends at `until` on the kernel clock.
    pub(crate) fn wait_until(
        &self,
        context: &mut ThreadContext,
        until: Option<u64>,
        ready: impl Fn() -> bool,
    ) {
        thread::wait_on(context, &self.head, until, ready);
    }

    // Safe from a trap handler.
    pub(crate) fn wake_all(&self) -> usize {
        thread::wake_waiters(&self.head)
    }
}
