use core::ops::Deref;

use native_abi::ring::{
    COMPLETION_BYTES, COMPLETIONS_AT, Completion, ENTRIES_AT, SUBMISSION_BYTES, SUBMISSIONS,
    Submission, header,
};
use wost::mm::slab::Hold;

use super::ring::publish_index;
use super::{Queue, QueueRef, SharedQueue};
use crate::handle::TestTable;
use crate::thread::ThreadContext;
use crate::world::{self, registry};

impl Queue {
    pub(crate) fn in_flight(&self) -> usize {
        self.pending.lock_leaf().slots.iter().flatten().count()
    }

    pub(crate) fn outstanding(&self) -> u32 {
        self.ring.lock_leaf().protocol.outstanding()
    }

    pub(crate) fn is_consistent(&self) -> bool {
        self.ring.lock_leaf().protocol.is_consistent()
    }

    pub(crate) fn post_submission_for_test(&self, index: usize, submission: Submission) {
        let bytes = submission.encode();
        let at = ENTRIES_AT + index % SUBMISSIONS as usize * SUBMISSION_BYTES;
        assert!(
            self.ring.lock_leaf().pages.write_payload_at(at, &bytes),
            "a self-test submission slot was outside the shared pages"
        );
    }

    pub(crate) fn publish_tail_for_test(&self, tail: u32) {
        publish_index(&self.ring.lock_leaf().pages, header::SUBMISSION_TAIL, tail);
    }

    pub(crate) fn write_data_for_test(&self, at: usize, bytes: &[u8]) -> bool {
        self.write_data(at, bytes)
    }

    pub(crate) fn read_data_for_test(&self, at: usize, into: &mut [u8]) -> bool {
        self.ring.lock_leaf().data.read_at(at, into)
    }

    pub(crate) fn read_completion(&self, slot: usize) -> Completion {
        let mut bytes = [0u8; COMPLETION_BYTES];
        let at = COMPLETIONS_AT + slot * COMPLETION_BYTES;
        assert!(
            self.ring.lock_leaf().pages.read_payload_at(at, &mut bytes),
            "a self-test completion slot was outside the shared pages"
        );
        Completion::decode(&bytes)
    }
}

// A queue in the root world that no program maps, destroyed when the check ends.
#[derive(Debug)]
pub(crate) struct TestQueue {
    queue: QueueRef,
    held: Option<Hold<Queue>>,
}

impl TestQueue {
    pub(crate) fn new(context: &mut ThreadContext) -> Option<Self> {
        let world_ref = world::root()?;
        let world = registry::hold(world_ref)?;
        let frames = QueueRef::frames_for()?;
        let mut earmark = wost::mm::pool::earmark(context, world.arena(), frames).ok()?;
        let ring = SharedQueue::new(&mut earmark)?;
        let queue = QueueRef::create(&mut earmark, world_ref, &world, ring);
        Some(Self {
            queue,
            held: queue.hold(),
        })
    }

    pub(crate) fn serve(&self, context: &mut ThreadContext, table: &TestTable) -> usize {
        Queue::serve(self, context, self.queue, table.table)
    }

    pub(crate) fn abandon(&self) -> usize {
        Queue::abandon(self, self.queue)
    }
}

impl Deref for TestQueue {
    type Target = Queue;

    fn deref(&self) -> &Queue {
        self.held
            .as_deref()
            .unwrap_or_else(|| panic!("a self-test queue vanished while the check held it"))
    }
}

impl Drop for TestQueue {
    fn drop(&mut self) {
        drop(self.held.take());
        assert!(self.queue.destroy(), "a self-test queue outlived its check");
    }
}
