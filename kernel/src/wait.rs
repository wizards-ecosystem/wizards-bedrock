use crate::thread::{self, ThreadContext, WaitLinks, WaitList};

// Waiters are threads linked through their own records under this list's lock, so a wake
// allocates nothing.
#[derive(Debug)]
pub(crate) struct WaitQueue {
    links: WaitList,
}

impl WaitQueue {
    pub(crate) const fn new() -> Self {
        Self {
            links: WaitList::new(WaitLinks::new()),
        }
    }

    // `ready` runs under the list's lock, so it must only read state.
    pub(crate) fn wait_unless(&self, context: &mut ThreadContext, ready: impl Fn() -> bool) {
        thread::wait_on(context, &self.links, None, ready);
    }

    // As `wait_unless`, but the wait also ends at `until` on the kernel clock.
    pub(crate) fn wait_until(
        &self,
        context: &mut ThreadContext,
        until: Option<u64>,
        ready: impl Fn() -> bool,
    ) {
        thread::wait_on(context, &self.links, until, ready);
    }

    // Safe from a trap handler.
    pub(crate) fn wake_all(&self) -> usize {
        thread::wake_waiters(&self.links)
    }
}
