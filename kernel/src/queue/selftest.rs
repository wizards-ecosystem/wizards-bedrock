use native_abi::ring::{
    COMPLETION_BYTES, COMPLETIONS_AT, Completion, DATA_AT, ENTRIES_AT, SUBMISSION_BYTES,
    SUBMISSIONS, Submission, header,
};

use super::Queue;
use super::ring::publish_index;

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
        self.ring
            .lock_leaf()
            .pages
            .read_payload_at(DATA_AT + at, into)
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
