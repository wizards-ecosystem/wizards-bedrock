use world_tree::{NodeRef, Progress, WorldStep};
use wost::mm::pool;
use wost::sync::SpinLock;

use crate::process;
use crate::rank;
use crate::thread::{self, ThreadContext, ThreadRef};
use crate::world;

// An arena node holds its world's teardown, one collection of its world's endpoints and one left
// by the node's last world, which the next round pops, so the ring cannot overflow.
const DEPTH: usize = 3 * pool::capacity() as usize;

#[derive(Clone, Copy, Debug)]
enum Work {
    World(NodeRef, WorldStep),
    Endpoints(NodeRef),
}

#[derive(Debug)]
struct Pending {
    // Exited threads, linked through their own records, so the list costs nothing to grow.
    first: Option<ThreadRef>,
    last: Option<ThreadRef>,
    threads: usize,
    entries: [Option<Work>; DEPTH],
    head: usize,
    len: usize,
    // Taken out for a round of steps and not yet done or returned to the tail.
    stepping: usize,
}

static PENDING: SpinLock<rank::Reap, Pending> = SpinLock::new(Pending {
    first: None,
    last: None,
    threads: 0,
    entries: [None; DEPTH],
    head: 0,
    len: 0,
    stepping: 0,
});

impl Pending {
    fn push_thread(&mut self, exited: ThreadRef) {
        thread::link_reap(exited, None);
        match self.last {
            Some(last) => thread::link_reap(last, Some(exited)),
            None => self.first = Some(exited),
        }
        self.last = Some(exited);
        self.threads += 1;
    }

    fn pop_thread(&mut self) -> Option<ThreadRef> {
        let first = self.first?;
        self.first = thread::reap_next(first);
        if self.first.is_none() {
            self.last = None;
        }
        self.threads -= 1;
        self.stepping += 1;
        Some(first)
    }

    fn push(&mut self, work: Work) {
        assert!(
            self.len < DEPTH,
            "more world teardowns are pending than arenas exist"
        );
        let at = (self.head + self.len) % DEPTH;
        self.entries[at] = Some(work);
        self.len += 1;
    }

    fn pop(&mut self) -> Option<Work> {
        if self.len == 0 {
            return None;
        }
        let taken = self.entries[self.head].take();
        self.head = (self.head + 1) % DEPTH;
        self.len -= 1;
        self.stepping += 1;
        taken
    }
}

// From the exiting thread, interrupts off, before it leaves its processor.
pub(crate) fn submit(context: &mut ThreadContext, exited: ThreadRef) {
    PENDING.lock(context).push_thread(exited);
    crate::queue::request_reap();
}

// Once per world, when its condemned chain turns non-empty.
pub(crate) fn submit_endpoints(node: NodeRef) {
    PENDING.lock_ranked().push(Work::Endpoints(node));
    crate::queue::request_reap();
}

pub(crate) fn submit_world(node: NodeRef) {
    PENDING
        .lock_ranked()
        .push(Work::World(node, WorldStep::Kill));
    crate::queue::request_reap();
}

// One round in submission order: each thread is reaped and each world steps until it must wait,
// or it rejoins the tail for the next.
pub(crate) fn run(context: &mut ThreadContext) -> bool {
    let round = PENDING.lock(context).threads;
    for _ in 0..round {
        let Some(exited) = PENDING.lock(context).pop_thread() else {
            break;
        };
        let done = reap(context, exited);
        let mut pending = PENDING.lock(context);
        pending.stepping -= 1;
        if !done {
            pending.push_thread(exited);
        }
    }
    let round = PENDING.lock(context).len;
    for _ in 0..round {
        let Some(mut work) = PENDING.lock(context).pop() else {
            break;
        };
        let done = match &mut work {
            Work::Endpoints(node) => {
                crate::channel::collect(*node);
                true
            }
            Work::World(node, step) => loop {
                match world::step(*node, *step) {
                    Progress::Done if *step == WorldStep::Finished => break true,
                    Progress::Done => *step = step.next(Progress::Done),
                    Progress::Pending => break false,
                }
            },
        };
        let mut pending = PENDING.lock(context);
        pending.stepping -= 1;
        if !done {
            pending.push(work);
        }
    }
    let pending = PENDING.lock(context);
    pending.threads + pending.len > 0
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
    pending.threads == 0 && pending.len == 0 && pending.stepping == 0
}
