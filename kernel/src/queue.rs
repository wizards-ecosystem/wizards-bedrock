//! The async submission/completion ring, kernel side.
//!
//! D-79 settled the shape, D-112 checked the protocol against every reachable
//! state before this file existed, and `libs/ring-proto` holds the arithmetic
//! under proptest. What is here is the part neither of those can be: the
//! boundary with pages ring 3 can rewrite at any moment.
//!
//! **The rule that shapes every function below: copy out exactly once, then
//! validate the copy.** CVE-2025-39816 is literally "always use `READ_ONCE()`
//! to read ring pointers", and a double fetch is how a validated value becomes
//! a different value before it is used. So every read of the shared pages
//! happens through [`Snapshot`], which is taken once and never refreshed, and
//! no function here takes an index it did not compute itself (D-113).
//!
//! **The pages are the kernel's**, mapped into the process rather than donated
//! to it: `map_shared` writes no `OWNED` bit, so the space can never hand
//! these frames back to the allocator, and the ring holds the run for its own
//! life. That is the arrangement D-79 asked for — kernel-allocated ring pages,
//! single-owner rings — and it is why a program cannot free the memory the
//! kernel is about to write a completion into.

use ring_proto::{Capacity, Queues};
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, USegment};
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

use crate::wait::WaitQueue;

/// Bytes of shared page: one for the header and the indices, one for the
/// entries.
const SHARED_BYTES: u64 = 2 * PAGE_SIZE;

/// Where the header ends and the submission entries begin.
const ENTRIES_AT: usize = PAGE_SIZE as usize;

/// Bytes in one submission or completion entry.
///
/// Sixteen: an opcode, a flags word, and one eight-byte argument. The opcode
/// surface stays minimal on purpose — io_uring's exploit record is a warning
/// about surface growth, and Google's kernelCTF disables it outright (`002`).
const ENTRY_BYTES: usize = 16;

/// Submission entries. Small, and a power of two.
const SUBMISSIONS: u32 = 8;

/// Completion entries: a power of two and at least twice the submissions,
/// which Windows requires of RIO and D-79 adopted.
const COMPLETIONS: u32 = 16;

/// Byte offsets into the header page, which is the ABI.
mod header {
    /// Protocol version. IoRing's improvement over io_uring: a ring that
    /// cannot say what shape it is cannot be extended without guessing.
    pub(super) const VERSION: usize = 0;
    /// Submission entries.
    pub(super) const SUBMISSIONS: usize = 4;
    /// Completion entries.
    pub(super) const COMPLETIONS: usize = 8;
    /// Where the kernel has read up to. Kernel writes, userspace reads.
    pub(super) const SUBMISSION_HEAD: usize = 16;
    /// Where userspace has written up to. Userspace writes, kernel reads.
    pub(super) const SUBMISSION_TAIL: usize = 20;
    /// Where userspace has reaped up to. Userspace writes, kernel reads.
    pub(super) const COMPLETION_HEAD: usize = 24;
    /// Where the kernel has posted up to. Kernel writes, userspace reads.
    pub(super) const COMPLETION_TAIL: usize = 28;
}

/// What one submission asks for.
///
/// Two opcodes and no privileged ones (`002`). Anything else is refused with a
/// completion rather than ignored, because D-79's rule is that every operation
/// posts exactly one terminal completion, always.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opcode {
    /// Return the argument. The smallest thing that proves a round trip.
    Echo,
    /// Return the kernel's uptime in nanoseconds.
    Uptime,
    /// Anything else.
    Unknown(u32),
}

impl Opcode {
    const fn decode(value: u32) -> Self {
        match value {
            1 => Self::Echo,
            2 => Self::Uptime,
            other => Self::Unknown(other),
        }
    }
}

/// A submission, copied out of the shared page exactly once.
///
/// Not a view: a *copy*. Nothing here borrows the page, so nothing can be
/// re-read after it has been validated, which is the property the type exists
/// to enforce rather than to document.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Submission {
    /// What was asked for.
    pub(crate) opcode: Opcode,
    /// The caller's own tag, returned in the completion so it can match them.
    pub(crate) token: u32,
    /// The operation's one argument.
    pub(crate) argument: u64,
}

/// What the kernel wrote back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Completion {
    /// The submission's tag, copied back.
    pub(crate) token: u32,
    /// Zero, or the refusal.
    pub(crate) status: u32,
    /// The operation's result.
    pub(crate) value: u64,
}

/// Refusals a completion can carry. Zero is success.
pub(crate) const STATUS_OK: u32 = 0;
/// The opcode is not one this kernel serves.
pub(crate) const STATUS_UNKNOWN_OPCODE: u32 = 1;

/// The indices, copied out of the shared page in one read.
///
/// Taken once per [`Queue::serve`] and never refreshed. Every bound checked
/// below is checked against *this*, so a rewrite landing mid-service changes
/// what the next call sees and nothing about what this one does.
#[derive(Debug, Clone, Copy)]
struct Snapshot {
    submission_tail: u32,
    completion_head: u32,
}

/// A submission/completion ring bound to one process.
#[derive(Debug)]
pub(crate) struct Queue {
    /// The shared pages. The kernel's, for the ring's whole life.
    shared: SpinLock<USegment>,
    /// The kernel's own protocol state, which userspace cannot reach.
    state: SpinLock<Queues>,
    /// Woken when a completion is posted.
    waiters: WaitQueue,
}

impl Queue {
    /// Build a ring and its pages, ready to be mapped.
    ///
    /// # Errors
    ///
    /// `None` if the frames cannot be allocated or cleared.
    pub(crate) fn new() -> Option<Self> {
        let mut run = mm::allocate_contiguous((SHARED_BYTES / PAGE_SIZE) as usize, 1)?;
        if !run.zero() {
            return None;
        }
        let mut shared: USegment = run.into();

        // The header is written before the pages are ever mapped, so a
        // program never sees a ring whose shape it cannot read.
        write_u32(&mut shared, header::VERSION, ring_proto::VERSION);
        write_u32(&mut shared, header::SUBMISSIONS, SUBMISSIONS);
        write_u32(&mut shared, header::COMPLETIONS, COMPLETIONS);

        let submission = Capacity::new(SUBMISSIONS).ok()?;
        let completion = Capacity::new(COMPLETIONS).ok()?;
        let state = Queues::new(submission, completion).ok()?;

        Some(Self {
            shared: SpinLock::new(shared),
            state: SpinLock::new(state),
            waiters: WaitQueue::new(),
        })
    }

    /// Map the ring's pages into `space` at `at`.
    ///
    /// Writable, never executable, and **not owned by the space**: the frames
    /// stay the ring's, so a program that unmapped them could not free them.
    ///
    /// # Errors
    ///
    /// As [`AddressSpace::map_shared`].
    pub(crate) fn install(&self, space: &mut AddressSpace, at: u64) -> Result<(), MapError> {
        space.map_shared(
            at,
            &self.shared.lock_leaf(),
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )
    }

    /// Serve every submission userspace has posted, and report how many.
    ///
    /// One terminal completion per operation, always — including for an opcode
    /// this kernel does not serve, which is refused *with* a completion rather
    /// than ignored. Admission already reserved the room, so none of these can
    /// fail to find a slot; if one did, `submit` admitted work it had nowhere
    /// to report and the model that says it cannot is wrong.
    pub(crate) fn serve(&self) -> usize {
        let snapshot = self.snapshot();
        let mut served = 0;

        while let Some(submission) = self.take(&snapshot) {
            let completion = perform(submission);
            if !self.post(completion) {
                break;
            }
            served += 1;
        }

        if served > 0 {
            self.waiters.wake_all();
        }
        served
    }

    /// Admit `count` operations, or as many as there is room to report.
    ///
    /// The kernel's side of admission, used by the checks: reserving here is
    /// what makes a completion impossible to drop, and it is the one place
    /// backpressure is expressed.
    pub(crate) fn admit(&self, count: usize) -> usize {
        let mut state = self.state.lock_leaf();
        (0..count).take_while(|_| state.submit().is_some()).count()
    }

    /// Copy the indices out of the shared page, once.
    fn snapshot(&self) -> Snapshot {
        let shared = self.shared.lock_leaf();
        Snapshot {
            submission_tail: read_u32(&shared, header::SUBMISSION_TAIL),
            completion_head: read_u32(&shared, header::COMPLETION_HEAD),
        }
    }

    /// Take one submission, if the snapshot says there is one to take.
    fn take(&self, snapshot: &Snapshot) -> Option<Submission> {
        let slot = {
            let mut state = self.state.lock_leaf();
            // The tail is userspace's number, so it bounds nothing on its own:
            // what it says is how many operations userspace *claims* to have
            // posted, and the kernel serves the smaller of that and what it
            // admitted. A tail that ran away would otherwise walk the kernel
            // through slots nothing was written into.
            let posted = snapshot
                .submission_tail
                .wrapping_sub(state.submissions().head());
            if posted == 0 || posted > SUBMISSIONS {
                return None;
            }
            state.take()?
        };

        let mut bytes = [0u8; ENTRY_BYTES];
        let at = ENTRIES_AT + slot * ENTRY_BYTES;
        if !self.shared.lock_leaf().read_at(at, &mut bytes) {
            return None;
        }

        // Validated after the copy, and the copy is what is used. A second
        // read here would be the double fetch the whole design refuses.
        let opcode = Opcode::decode(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
        let token = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let argument = u64::from_le_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        ]);

        // The head userspace reads is published after the entry has been
        // copied out, so a program that watches it cannot see the kernel
        // release a slot it is still reading.
        let head = self.state.lock_leaf().submissions().head();
        write_u32(&mut self.shared.lock_leaf(), header::SUBMISSION_HEAD, head);

        Some(Submission {
            opcode,
            token,
            argument,
        })
    }

    /// Post one completion. `false` only if the reservation was not there,
    /// which the model says cannot happen.
    fn post(&self, completion: Completion) -> bool {
        let Some(slot) = self.state.lock_leaf().complete() else {
            return false;
        };

        let mut bytes = [0u8; ENTRY_BYTES];
        bytes[0..4].copy_from_slice(&completion.token.to_le_bytes());
        bytes[4..8].copy_from_slice(&completion.status.to_le_bytes());
        bytes[8..16].copy_from_slice(&completion.value.to_le_bytes());

        let at = ENTRIES_AT + (SUBMISSIONS as usize * ENTRY_BYTES) + slot * ENTRY_BYTES;
        let mut shared = self.shared.lock_leaf();
        if !shared.write_at(at, &bytes) {
            return false;
        }

        // The entry is in the page before the tail says so: a program that
        // sees the tail move and reads the slot finds what was written, not
        // what was there before.
        let tail = self.state.lock_leaf().completions().tail();
        write_u32(&mut shared, header::COMPLETION_TAIL, tail);
        true
    }

    /// Let userspace's reaping through, so admission can proceed again.
    ///
    /// Reads the snapshot's `completion_head` rather than the page, for the
    /// same reason everything else here does.
    pub(crate) fn reap_up_to(&self, snapshot_head: u32) -> usize {
        let mut state = self.state.lock_leaf();
        let mut reaped = 0;
        while state.completions().head() != snapshot_head && state.reap().is_some() {
            reaped += 1;
        }
        reaped
    }

    /// What userspace last said it had reaped.
    pub(crate) fn completion_head(&self) -> u32 {
        self.snapshot().completion_head
    }

    /// Operations admitted and not yet reported.
    pub(crate) fn outstanding(&self) -> u32 {
        self.state.lock_leaf().outstanding()
    }

    /// Whether the accounting still says a completion cannot be dropped.
    pub(crate) fn is_consistent(&self) -> bool {
        self.state.lock_leaf().is_consistent()
    }

    /// Write a submission into the shared page as a program would.
    ///
    /// For the checks. The kernel side must be exercised against bytes that
    /// arrived through the page rather than through a function call, because
    /// the page is the boundary and the function call is not.
    #[cfg(feature = "selftest")]
    pub(crate) fn post_submission_for_test(&self, slot: usize, submission: Submission) {
        let opcode = match submission.opcode {
            Opcode::Echo => 1,
            Opcode::Uptime => 2,
            Opcode::Unknown(other) => other,
        };
        let mut bytes = [0u8; ENTRY_BYTES];
        bytes[0..4].copy_from_slice(&u32::to_le_bytes(opcode));
        bytes[4..8].copy_from_slice(&submission.token.to_le_bytes());
        bytes[8..16].copy_from_slice(&submission.argument.to_le_bytes());
        let at = ENTRIES_AT + slot * ENTRY_BYTES;
        let mut shared = self.shared.lock_leaf();
        shared.write_at(at, &bytes);
    }

    /// Move the submission tail as a program would.
    #[cfg(feature = "selftest")]
    pub(crate) fn publish_tail_for_test(&self, tail: u32) {
        write_u32(&mut self.shared.lock_leaf(), header::SUBMISSION_TAIL, tail);
    }

    /// Move the completion head as a program that has reaped would.
    #[cfg(feature = "selftest")]
    pub(crate) fn publish_completion_head_for_test(&self, head: u32) {
        write_u32(&mut self.shared.lock_leaf(), header::COMPLETION_HEAD, head);
    }

    /// Read a completion back out of the shared page as a program would.
    #[cfg(feature = "selftest")]
    pub(crate) fn read_completion_for_test(&self, slot: usize) -> Completion {
        let mut bytes = [0u8; ENTRY_BYTES];
        let at = ENTRIES_AT + (SUBMISSIONS as usize * ENTRY_BYTES) + slot * ENTRY_BYTES;
        self.shared.lock_leaf().read_at(at, &mut bytes);
        Completion {
            token: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            status: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            value: u64::from_le_bytes([
                bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
                bytes[15],
            ]),
        }
    }

    /// The header, for the check that a program can read the ring's shape.
    #[cfg(feature = "selftest")]
    pub(crate) fn shape_for_test(&self) -> (u32, u32, u32) {
        let shared = self.shared.lock_leaf();
        (
            read_u32(&shared, header::VERSION),
            read_u32(&shared, header::SUBMISSIONS),
            read_u32(&shared, header::COMPLETIONS),
        )
    }
}

/// Do what one submission asked, and say what happened.
///
/// Every path returns a completion. There is no way to reach the end of this
/// function without one, which is D-79's rule expressed as control flow rather
/// than as a comment.
fn perform(submission: Submission) -> Completion {
    match submission.opcode {
        Opcode::Echo => Completion {
            token: submission.token,
            status: STATUS_OK,
            value: submission.argument,
        },
        Opcode::Uptime => Completion {
            token: submission.token,
            status: STATUS_OK,
            value: crate::time::now_ns(),
        },
        Opcode::Unknown(_) => Completion {
            token: submission.token,
            status: STATUS_UNKNOWN_OPCODE,
            value: 0,
        },
    }
}

/// Read one little-endian `u32` out of the shared pages.
fn read_u32(shared: &USegment, at: usize) -> u32 {
    let mut bytes = [0u8; 4];
    shared.read_at(at, &mut bytes);
    u32::from_le_bytes(bytes)
}

/// Write one little-endian `u32` into the shared pages.
fn write_u32(shared: &mut USegment, at: usize, value: u32) {
    shared.write_at(at, &value.to_le_bytes());
}
