use native_abi::ring::{
    COMPLETION_BYTES, COMPLETIONS_AT, Completion, DATA_BYTES, ENTRIES_AT, SUBMISSION_BYTES,
    SUBMISSIONS, Submission, header,
};

use super::{Queue, RingPages};

impl Queue {
    pub(super) fn read_data(&self, submission: &Submission, into: &mut [u8]) -> Option<usize> {
        let (at, len) = buffer_bounds(submission)?;
        if len > into.len() {
            return None;
        }
        self.ring
            .lock_leaf()
            .data
            .read_at(at, &mut into[..len])
            .then_some(len)
    }

    pub(super) fn write_data(&self, at: usize, bytes: &[u8]) -> bool {
        self.ring.lock_leaf().data.write_at(at, bytes)
    }

    pub(super) fn submission_tail(&self) -> u32 {
        load_index(&self.ring.lock_leaf().pages, header::SUBMISSION_TAIL)
    }

    pub(super) fn take(&self, submission_tail: u32) -> Option<Submission> {
        let mut bytes = [0u8; SUBMISSION_BYTES];
        {
            let mut ring = self.ring.lock_leaf();
            // The tail is ring 3's claim and bounds nothing alone.
            let published = submission_tail.wrapping_sub(ring.protocol.submissions().head());
            if published == 0 || published > SUBMISSIONS {
                return None;
            }
            // No completion room is backpressure: the entry stays published for a later call.
            ring.protocol.submit()?;
            let Some(slot) = ring.protocol.take() else {
                panic!("a submission was reserved and then was not there");
            };

            let at = ENTRIES_AT + slot * SUBMISSION_BYTES;
            assert!(
                ring.pages.read_payload_at(at, &mut bytes),
                "a submission slot was outside the shared pages"
            );

            // The head is published after the copy, so no slot is released while still being read.
            let head = ring.protocol.submissions().head();
            publish_index(&ring.pages, header::SUBMISSION_HEAD, head);
        }

        // Only the copy is validated and used: a second read of the page is a double fetch.
        Some(Submission::decode(&bytes))
    }

    pub(super) fn post(&self, completion: Completion) {
        let bytes = completion.encode();

        // One guard makes every completion path one producer, so no tail passes an unwritten slot.
        let mut ring = self.ring.lock_leaf();
        let Some(slot) = ring.protocol.complete() else {
            panic!("a completion was posted without the reservation its admission took");
        };
        let tail = ring.protocol.completions().tail();

        let at = COMPLETIONS_AT + slot * COMPLETION_BYTES;
        assert!(
            ring.pages.write_payload_at(at, &bytes),
            "an internal completion slot was outside the ring pages"
        );
        // The tail is published only after the entry is written.
        publish_index(&ring.pages, header::COMPLETION_TAIL, tail);
    }

    pub(crate) fn reap_up_to(&self, claimed_head: u32) {
        self.ring.lock_leaf().protocol.reap_up_to(claimed_head);
    }

    pub(crate) fn completion_head(&self) -> u32 {
        load_index(&self.ring.lock_leaf().pages, header::COMPLETION_HEAD)
    }
}
// Buffer 0 is the data page; buffers 1 to 15 exist only once a program registers them.
pub(super) fn buffer_bounds(submission: &Submission) -> Option<(usize, usize)> {
    if submission.buffer != 0 {
        return None;
    }
    let at = usize::try_from(submission.at).ok()?;
    let len = submission.length as usize;
    if len == 0 {
        return None;
    }
    let end = at.checked_add(len)?;
    (end <= DATA_BYTES).then_some((at, len))
}

pub(super) fn write_u32(shared: &mut RingPages, at: usize, value: u32) {
    assert!(
        shared.write_payload_at(at, &value.to_le_bytes()),
        "an internal ring header field was outside the shared pages"
    );
}

fn load_index(shared: &RingPages, at: usize) -> u32 {
    let Some(value) = shared.load_acquire_u32(at) else {
        panic!("a ring index was out of bounds or misaligned");
    };
    value
}

pub(super) fn publish_index(shared: &RingPages, at: usize, value: u32) {
    assert!(
        shared.store_release_u32(at, value),
        "a ring index was out of bounds or misaligned"
    );
}
