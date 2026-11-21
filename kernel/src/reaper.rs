use world_tree::{NodeRef, Progress, WorldStep};
use wost::mm::pool;
use wost::sync::{Leaf, SpinLock};

use crate::process::{self, ProcessRef};
use crate::thread::{self, ThreadContext};
use crate::world;

// Every process here holds a thread slot until its last step; an arena node holds its world's
// teardown, one collection of its world's endpoints and one left by the node's last world, which
// the next round pops, so the ring cannot overflow.
const DEPTH: usize = thread::CAPACITY + 3 * pool::capacity() as usize;

#[derive(Clone, Copy, Debug)]
enum Work {
    Process(ProcessRef),
    World(NodeRef, WorldStep),
    Endpoints(NodeRef),
}

#[derive(Debug)]
struct Pending {
    entries: [Option<Work>; DEPTH],
    head: usize,
    len: usize,
    // Taken out for a round of steps and not yet done or returned to the tail.
    stepping: usize,
}

static PENDING: SpinLock<Leaf, Pending> = SpinLock::new(Pending {
    entries: [None; DEPTH],
    head: 0,
    len: 0,
    stepping: 0,
});

impl Pending {
    fn push(&mut self, work: Work) {
        assert!(
            self.len < DEPTH,
            "more teardowns are pending than threads and arenas exist"
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

// From the retiring thread, interrupts off: no allocation and no block, only a leaf.
pub(crate) fn submit(process: ProcessRef) {
    PENDING.lock_leaf().push(Work::Process(process));
    crate::queue::request_reap();
}

// Once per world, when its condemned chain turns non-empty.
pub(crate) fn submit_endpoints(node: NodeRef) {
    PENDING.lock_leaf().push(Work::Endpoints(node));
    crate::queue::request_reap();
}

pub(crate) fn submit_world(node: NodeRef) {
    PENDING.lock_leaf().push(Work::World(node, WorldStep::Kill));
    crate::queue::request_reap();
}

// One round in submission order: each record runs until a step must wait, then rejoins the tail.
pub(crate) fn run(context: &mut ThreadContext) -> bool {
    let round = PENDING.lock_leaf().len;
    for _ in 0..round {
        let Some(mut work) = PENDING.lock_leaf().pop() else {
            break;
        };
        loop {
            let finished = match &mut work {
                Work::Process(process) => match process::step(*process, context) {
                    Progress::Done => process.hold().is_none().then_some(true),
                    Progress::Pending => Some(false),
                },
                Work::Endpoints(node) => {
                    crate::channel::collect(*node);
                    Some(true)
                }
                Work::World(node, step) => match world::step(*node, *step) {
                    Progress::Done if *step == WorldStep::Finished => Some(true),
                    Progress::Done => {
                        *step = step.next(Progress::Done);
                        None
                    }
                    Progress::Pending => Some(false),
                },
            };
            match finished {
                None => {}
                Some(true) => {
                    PENDING.lock_leaf().stepping -= 1;
                    break;
                }
                Some(false) => {
                    let mut pending = PENDING.lock_leaf();
                    pending.stepping -= 1;
                    pending.push(work);
                    break;
                }
            }
        }
    }
    PENDING.lock_leaf().len > 0
}

#[cfg(feature = "selftest")]
pub(crate) fn is_idle() -> bool {
    let pending = PENDING.lock_leaf();
    pending.len == 0 && pending.stepping == 0
}
