use world_tree::Progress;
use wost::sync::{Leaf, SpinLock};

use crate::process::{self, ProcessRef};
use crate::thread::{self, ThreadContext};

// Every process here holds a thread slot until its last step, so the ring cannot overflow.
const DEPTH: usize = thread::CAPACITY;

#[derive(Debug)]
struct Pending {
    entries: [Option<ProcessRef>; DEPTH],
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
    fn push(&mut self, process: ProcessRef) {
        assert!(
            self.len < DEPTH,
            "more processes await teardown than threads exist"
        );
        let at = (self.head + self.len) % DEPTH;
        self.entries[at] = Some(process);
        self.len += 1;
    }

    fn pop(&mut self) -> Option<ProcessRef> {
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
    PENDING.lock_leaf().push(process);
    crate::queue::request_reap();
}

// One round in submission order: each process runs until a step must wait, then rejoins the tail.
pub(crate) fn run(context: &mut ThreadContext) -> bool {
    let round = PENDING.lock_leaf().len;
    for _ in 0..round {
        let Some(process) = PENDING.lock_leaf().pop() else {
            break;
        };
        loop {
            match process::step(process, context) {
                Progress::Done if process.hold().is_none() => {
                    PENDING.lock_leaf().stepping -= 1;
                    break;
                }
                Progress::Done => {}
                Progress::Pending => {
                    let mut pending = PENDING.lock_leaf();
                    pending.stepping -= 1;
                    pending.push(process);
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
