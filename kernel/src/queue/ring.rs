#[cfg(feature = "selftest")]
use core::sync::atomic::Ordering;

use super::{
    COMPLETION_BYTES, COMPLETIONS_AT, Completion, DATA_AT, DATA_BYTES, ENTRIES_AT, Opcode, Queue,
    RingPages, SUBMISSION_BYTES, SUBMISSIONS, Snapshot, Submission, completion, header, submission,
};

impl Queue {
    /// Copy `length` bytes out of the data area at `offset`.
    pub(super) fn read_data(&self, offset: u64, length: u32, into: &mut [u8]) -> Option<usize> {
        let (at, len) = data_bounds(offset, length)?;
        if len > into.len() {
            return None;
        }
        self.ring
            .lock_leaf()
            .pages
            .read_payload_at(DATA_AT + at, &mut into[..len])
            .then_some(len)
    }

    /// Copy `bytes` into the data area at `offset`.
    pub(super) fn write_data(&self, at: usize, bytes: &[u8]) -> bool {
        self.ring
            .lock_leaf()
            .pages
            .write_payload_at(DATA_AT + at, bytes)
    }

    /// Copy the indices out of the shared page, once.
    pub(super) fn snapshot(&self) -> Snapshot {
        let ring = self.ring.lock_leaf();
        Snapshot {
            submission_tail: load_index(&ring.pages, header::SUBMISSION_TAIL),
            completion_head: load_index(&ring.pages, header::COMPLETION_HEAD),
        }
    }

    /// Admit and take one submission, if the snapshot says there is one.
    ///
    /// Two refusals, and they are different things. **Nothing published** ends
    /// the round normally. **No completion slot** is backpressure: the entry
    /// stays where it is and is served on a later call, which is what makes an
    /// unreportable operation impossible rather than merely unlikely.
    pub(super) fn take(&self, snapshot: &Snapshot) -> Option<Submission> {
        let mut bytes = [0u8; SUBMISSION_BYTES];
        {
            let mut ring = self.ring.lock_leaf();
            // The tail is userspace's claim, so it bounds nothing on its own;
            // a runaway tail would walk the kernel through unwritten slots.
            let published = snapshot
                .submission_tail
                .wrapping_sub(ring.protocol.submissions().head());
            if published == 0 || published > SUBMISSIONS {
                return None;
            }
            // The reservation, taken before the operation is accepted.
            ring.protocol.submit()?;
            // `submit` just produced this slot; losing it would strand the
            // reservation, so the contradiction is terminal in every build (D-206).
            let taken = ring.protocol.take();
            assert!(
                taken.is_some(),
                "a submission was reserved and then was not there"
            );
            let slot = taken?;

            let at = ENTRIES_AT + slot * SUBMISSION_BYTES;
            // Unreachable by construction, and terminal for the same reason:
            // this one has consumed the submission, so it loses the operation too.
            let in_bounds = ring.pages.read_payload_at(at, &mut bytes);
            assert!(in_bounds, "a submission slot was outside the shared pages");
            if !in_bounds {
                return None;
            }

            // The head is published after the copy, so a program cannot see
            // the kernel release a slot it is still reading.
            let head = ring.protocol.submissions().head();
            publish_index(&ring.pages, header::SUBMISSION_HEAD, head);
        }

        // Validated after the copy, and the copy is what is used. A second
        // read here would be the double fetch the whole design refuses.
        let opcode = Opcode::decode(read_entry_u32(&bytes, submission::OPCODE));
        let token = read_entry_u32(&bytes, submission::TOKEN);
        let handle = read_entry_u32(&bytes, submission::HANDLE);
        let length = read_entry_u32(&bytes, submission::LENGTH);
        let offset = read_entry_u64(&bytes, submission::OFFSET);
        let argument = read_entry_u64(&bytes, submission::ARGUMENT);

        Some(Submission {
            opcode,
            token,
            handle,
            length,
            offset,
            argument,
        })
    }

    /// Post one completion.
    ///
    /// `false` means no reservation accounting changed. The model says the
    /// production path cannot reach it; checks inject it before the state
    /// transition to prove every caller handles the refusal. A shared-page
    /// bounds failure after that transition is fatal instead: retrying an
    /// already-spent reservation would duplicate accounting (D-192).
    pub(super) fn post(&self, completion: Completion) -> bool {
        #[cfg(feature = "selftest")]
        if self.refuse_next_post.swap(false, Ordering::AcqRel) {
            return false;
        }
        let mut bytes = [0u8; COMPLETION_BYTES];
        write_entry_u32(&mut bytes, completion::TOKEN, completion.token);
        write_entry_u32(&mut bytes, completion::STATUS, completion.status);
        write_entry_u64(&mut bytes, completion::VALUE, completion.value);

        // One guard makes every completion path one logical producer; with
        // separate locks a later tail could publish before an earlier slot (D-221).
        let mut ring = self.ring.lock_leaf();
        let Some(slot) = ring.protocol.complete() else {
            return false;
        };
        let tail = ring.protocol.completions().tail();

        let at = COMPLETIONS_AT + slot * COMPLETION_BYTES;
        assert!(
            ring.pages.write_payload_at(at, &bytes),
            "an internal completion slot was outside the ring pages"
        );
        // The entry is in the page before the tail says so; `complete`
        // advanced the tail before the write, and it is published after it.
        publish_index(&ring.pages, header::COMPLETION_TAIL, tail);
        true
    }

    /// Let userspace's reaping through, so admission can proceed again.
    ///
    /// `claimed_head` is whatever the program wrote, taken once by the caller
    /// and passed in — this function reads no page. What it does with the
    /// value is bound it: see below.
    pub(crate) fn reap_up_to(&self, claimed_head: u32) -> usize {
        let mut ring = self.ring.lock_leaf();

        ring.protocol.reap_up_to(claimed_head) as usize
    }

    /// What userspace last said it had reaped.
    pub(crate) fn completion_head(&self) -> u32 {
        self.snapshot().completion_head
    }
}
/// Where in the data area `offset` and `length` name, if they name anything.
///
/// **Arithmetic, not a page walk.** The data area is part of the ring, so the
/// whole of "is this buffer usable" is two comparisons — and there is no
/// caller pointer, no SMAP window and no fault to recover (D-173).
pub(super) fn data_bounds(offset: u64, length: u32) -> Option<(usize, usize)> {
    let at = usize::try_from(offset).ok()?;
    let len = length as usize;
    if len == 0 {
        return None;
    }
    let end = at.checked_add(len)?;
    (end <= DATA_BYTES).then_some((at, len))
}

/// Write one immutable little-endian `u32` into the shared pages.
pub(super) fn write_u32_volatile(shared: &mut RingPages, at: usize, value: u32) {
    assert!(
        shared.write_payload_at(at, &value.to_le_bytes()),
        "an internal ring header field was outside the shared pages"
    );
}

/// Acquire one peer-owned queue index.
fn load_index(shared: &RingPages, at: usize) -> u32 {
    let Some(value) = shared.load_acquire_u32(at) else {
        panic!("a ring index was out of bounds or misaligned");
    };
    value
}

/// Release-publish one locally owned queue index.
pub(super) fn publish_index(shared: &RingPages, at: usize, value: u32) {
    if !shared.store_release_u32(at, value) {
        panic!("a ring index was out of bounds or misaligned");
    }
}

pub(super) fn read_entry_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub(super) fn read_entry_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
        bytes[at + 4],
        bytes[at + 5],
        bytes[at + 6],
        bytes[at + 7],
    ])
}

pub(super) fn write_entry_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn write_entry_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
