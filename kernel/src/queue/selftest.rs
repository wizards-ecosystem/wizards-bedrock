use super::ring::{
    publish_index, read_entry_u32, read_entry_u64, write_entry_u32, write_entry_u64,
};
use super::{
    COMPLETION_BYTES, COMPLETIONS_AT, Completion, DATA_AT, ENTRIES_AT, Opcode, Queue,
    SUBMISSION_BYTES, SUBMISSIONS, Submission, completion, header, submission,
};

impl Submission {
    pub(crate) const fn about(opcode: Opcode, token: u32, handle: u32, argument: u64) -> Self {
        Self {
            opcode,
            token,
            handle,
            length: 0,
            offset: 0,
            argument,
        }
    }

    pub(crate) const fn simple(opcode: Opcode, token: u32, argument: u64) -> Self {
        Self::about(opcode, token, 0, argument)
    }

    pub(crate) const fn length(self, length: u32) -> Self {
        Self { length, ..self }
    }

    pub(crate) const fn offset(self, offset: u64) -> Self {
        Self { offset, ..self }
    }
}

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

    // `index` counts submissions, as a program's tail does; the slot is where it wraps to.
    pub(crate) fn post_submission_for_test(&self, index: usize, submission: Submission) {
        // Exhaustive, so an opcode added to `decode` and forgotten here fails the build.
        let opcode = match submission.opcode {
            Opcode::Echo => 1,
            Opcode::Uptime => 2,
            Opcode::Duplicate => 3,
            Opcode::Timer => 4,
            Opcode::Cancel => 5,
            Opcode::OpenAt => 6,
            Opcode::Read => 7,
            Opcode::Unknown(other) => other,
        };
        let mut bytes = [0u8; SUBMISSION_BYTES];
        write_entry_u32(&mut bytes, submission::OPCODE, opcode);
        write_entry_u32(&mut bytes, submission::TOKEN, submission.token);
        write_entry_u32(&mut bytes, submission::HANDLE, submission.handle);
        write_entry_u32(&mut bytes, submission::LENGTH, submission.length);
        write_entry_u64(&mut bytes, submission::OFFSET, submission.offset);
        write_entry_u64(&mut bytes, submission::ARGUMENT, submission.argument);
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
        Completion {
            token: read_entry_u32(&bytes, completion::TOKEN),
            status: read_entry_u32(&bytes, completion::STATUS),
            value: read_entry_u64(&bytes, completion::VALUE),
        }
    }
}
