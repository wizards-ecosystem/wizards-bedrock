use world_tree::Progress;
use wost::sync::SpinLock;

use crate::process;
use crate::rank;
use crate::thread::{self, ThreadContext, ThreadRef};

// Exited threads, linked through their own records, so the list costs nothing to grow.
#[derive(Debug)]
struct Pending {
    first: Option<ThreadRef>,
    last: Option<ThreadRef>,
    len: usize,
    // Taken out for a round of steps and not yet done or returned to the tail.
    stepping: usize,
}

static PENDING: SpinLock<rank::Reap, Pending> = SpinLock::new(Pending {
    first: None,
    last: None,
    len: 0,
    stepping: 0,
});

impl Pending {
    fn push(&mut self, exited: ThreadRef) {
        thread::link_reap(exited, None);
        match self.last {
            Some(last) => thread::link_reap(last, Some(exited)),
            None => self.first = Some(exited),
        }
        self.last = Some(exited);
        self.len += 1;
    }

    fn pop(&mut self) -> Option<ThreadRef> {
        let first = self.first?;
        self.first = thread::reap_next(first);
        if self.first.is_none() {
            self.last = None;
        }
        self.len -= 1;
        self.stepping += 1;
        Some(first)
    }
}

// From the exiting thread, interrupts off, before it leaves its processor.
pub(crate) fn submit(context: &mut ThreadContext, exited: ThreadRef) {
    PENDING.lock(context).push(exited);
    crate::queue::request_reap();
}

// One round in submission order: each thread is reaped or rejoins the tail for the next.
pub(crate) fn run(context: &mut ThreadContext) -> bool {
    let round = PENDING.lock(context).len;
    for _ in 0..round {
        let Some(exited) = PENDING.lock(context).pop() else {
            break;
        };
        let done = reap(context, exited);
        let mut pending = PENDING.lock(context);
        pending.stepping -= 1;
        if !done {
            pending.push(exited);
        }
    }
    PENDING.lock(context).len > 0
}

// Off its processor first, then its process's teardown, then its own record and stack.
fn reap(context: &mut ThreadContext, exited: ThreadRef) -> bool {
    if !thread::is_off_cpu(exited) {
        return false;
    }
    if let Some(owner) = thread::owner(exited) {
        loop {
            match process::step(owner, context) {
                Progress::Done if owner.hold().is_none() => break,
                Progress::Done => {}
                Progress::Pending => return false,
            }
        }
    }
    thread::release(context, exited)
}

#[cfg(feature = "selftest")]
pub(crate) fn is_idle() -> bool {
    let pending = PENDING.lock_leaf();
    pending.len == 0 && pending.stepping == 0
}
