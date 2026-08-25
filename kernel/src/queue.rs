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

use core::sync::atomic::{AtomicUsize, Ordering};

use ring_proto::{Capacity, Queues};
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, Shared, USegment};
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
    shared: SpinLock<Shared>,
    /// The kernel's own protocol state, which userspace cannot reach.
    state: SpinLock<Queues>,
    /// Woken when a completion is posted.
    waiters: WaitQueue,
    /// Completions posted since the ring was made.
    ///
    /// Kernel-side and monotonic, so a check can ask whether work was actually
    /// done without reading a number ring 3 can write (D-115's lesson applied
    /// where the answer matters most: the program's own tally is in the page).
    served: AtomicUsize,
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
        // Pinned for sharing, which is one-way: these frames can never also be
        // mapped as owned, so no unmap can return them while the program still
        // reaches them (D-115).
        let mut shared: Shared = USegment::from(run).share();

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
            served: AtomicUsize::new(0),
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
    /// `&mut self`, so the run is reached through `get_mut` rather than
    /// through a guard: `map_shared` allocates page-table frames, and holding
    /// a leaf guard across an allocation is what `lock_leaf` forbids (D-85).
    /// Copying the address out instead would hand `map_shared` a bare number
    /// where the type is the evidence, which is the hole D-115 closed one
    /// function away.
    pub(crate) fn install(&mut self, space: &mut AddressSpace, at: u64) -> Result<(), MapError> {
        space.map_shared(
            at,
            self.shared.get_mut(),
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )
    }

    /// Serve every submission userspace has posted, and report how many.
    ///
    /// **Admission happens here**, when the doorbell rings — not before. A
    /// program writes an entry and moves its tail; nothing tells the kernel
    /// until it is asked, and it is the asking that reserves. Reserving in a
    /// separate earlier call was the first design and it could not work: the
    /// kernel's own ring never learned about entries the program had written,
    /// so a program that did exactly what the ABI describes was served
    /// nothing (D-116).
    ///
    /// The reservation is still taken **before** the operation is accepted,
    /// which is the whole of D-79 and what the model checks (D-112): a
    /// submission with nowhere to report is left unserved rather than served
    /// and dropped. That is the backpressure, and it is why this can stop
    /// early with entries still published.
    ///
    /// One terminal completion per operation, always — including for an opcode
    /// this kernel does not serve, which is refused *with* a completion rather
    /// than ignored.
    pub(crate) fn serve(&self) -> usize {
        let snapshot = self.snapshot();
        let mut served = 0;

        while let Some(submission) = self.take(&snapshot) {
            let completion = perform(submission);
            if !self.post(completion) {
                // Unreachable: admission reserved this slot before the
                // operation was accepted, which is the whole of D-79's design,
                // and D-112's model says no reachable state loses one. If it
                // ever is reached the operation has been consumed with no
                // completion — so the reservation is held for ever, the ring
                // admits nothing more, and the program waits on something that
                // will not come. Loud in a debug build rather than a silent
                // wedge nobody can diagnose (D-115).
                debug_assert!(false, "a reserved completion slot was not there");
                break;
            }
            served += 1;
        }

        if served > 0 {
            self.served.fetch_add(served, Ordering::Release);
            self.waiters.wake_all();
        }
        served
    }

    /// Completions posted since this ring was made.
    ///
    /// The observation point for the check that a *program* drove this ring:
    /// the tally is kernel-side, so unlike the header page's numbers it is not
    /// something ring 3 can write to make the answer it wants (D-115, D-116).
    ///
    /// The `fetch_add` that feeds it is unconditional, so the shipped kernel
    /// and the checked one run the same code; only the reading is gated.
    pub(crate) fn served_count(&self) -> usize {
        self.served.load(Ordering::Acquire)
    }

    /// Reserve `count` completion slots without serving anything.
    ///
    /// **Not part of the ABI.** `serve` reserves as it goes, which is what the
    /// design requires; this exists so a check can exercise admission on its
    /// own — filling a completion queue and asking whether the next
    /// reservation is refused, without a program in the way. Using it and then
    /// publishing a tail would double-count, and the checks do one or the
    /// other.
    #[cfg(feature = "selftest")]
    pub(crate) fn reserve_for_test(&self, count: usize) -> usize {
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

    /// Admit and take one submission, if the snapshot says there is one.
    ///
    /// Two refusals, and they are different things. **Nothing published** ends
    /// the round normally. **No completion slot** is backpressure: the entry
    /// stays where it is and is served on a later call, which is what makes an
    /// unreportable operation impossible rather than merely unlikely.
    fn take(&self, snapshot: &Snapshot) -> Option<Submission> {
        let slot = {
            let mut state = self.state.lock_leaf();
            // The tail is userspace's number, so it bounds nothing on its own:
            // what it says is how many operations userspace *claims* to have
            // written, and a tail that ran away would otherwise walk the kernel
            // through slots nothing was written into.
            let published = snapshot
                .submission_tail
                .wrapping_sub(state.submissions().head());
            if published == 0 || published > SUBMISSIONS {
                return None;
            }
            // The reservation, taken before the operation is accepted.
            state.submit()?;
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

        // `state` before `shared`, which is the order `take` uses. The two are
        // never held together, and taking them in opposite orders in the two
        // functions was an inversion pre-seeded for SMP (D-70, D-115).
        let tail = self.state.lock_leaf().completions().tail();

        let at = ENTRIES_AT + (SUBMISSIONS as usize * ENTRY_BYTES) + slot * ENTRY_BYTES;
        let mut shared = self.shared.lock_leaf();
        if !shared.write_at(at, &bytes) {
            return false;
        }
        // The entry is in the page before the tail says so: a program that
        // sees the tail move and reads the slot finds what was written, not
        // what was there before — and `complete` already advanced the tail, so
        // this publishes a value taken before the write and stored after it.
        write_u32(&mut shared, header::COMPLETION_TAIL, tail);
        true
    }

    /// Let userspace's reaping through, so admission can proceed again.
    ///
    /// `claimed_head` is whatever the program wrote, taken once by the caller
    /// and passed in — this function reads no page. What it does with the
    /// value is bound it: see below.
    pub(crate) fn reap_up_to(&self, claimed_head: u32) -> usize {
        let mut state = self.state.lock_leaf();

        // Bounded before it drives anything, exactly as the submission tail is
        // (D-79's copy-out-then-*validate*). A head further ahead than there
        // are completions posted is not a large reap, it is a lie — and
        // honouring it reaps entries the program never read, which defeats
        // the one claim the completion queue exists to make. The loop's own
        // termination is not a bound: `reap` would happily drain all of them
        // (D-115).
        let posted = state.completions().occupied();
        let asked = claimed_head.wrapping_sub(state.completions().head());
        let allowed = asked.min(posted);

        let mut reaped = 0;
        while reaped < allowed && state.reap().is_some() {
            reaped += 1;
        }
        reaped as usize
    }

    /// What userspace last said it had reaped.
    pub(crate) fn completion_head(&self) -> u32 {
        self.snapshot().completion_head
    }

    /// Operations admitted and not yet reported.
    #[allow(dead_code)]
    pub(crate) fn outstanding(&self) -> u32 {
        self.state.lock_leaf().outstanding()
    }

    /// Whether the accounting still says a completion cannot be dropped.
    #[allow(dead_code)]
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

    /// Read a submission back out of the shared page.
    ///
    /// So a check can show that a hostile rewrite *landed* — without it, a
    /// check comparing only the completion is comparing two places the rewrite
    /// never touched, which is true whatever the kernel does (D-115).
    pub(crate) fn read_submission(&self, slot: usize) -> Submission {
        let mut bytes = [0u8; ENTRY_BYTES];
        let at = ENTRIES_AT + slot * ENTRY_BYTES;
        self.shared.lock_leaf().read_at(at, &mut bytes);
        Submission {
            opcode: Opcode::decode(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            token: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            argument: u64::from_le_bytes([
                bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
                bytes[15],
            ]),
        }
    }

    /// Read a completion back out of the shared page as a program would.
    pub(crate) fn read_completion(&self, slot: usize) -> Completion {
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

    /// The shape the **kernel** believes this ring has.
    ///
    /// From `state`, which ring 3 cannot reach. The header page carries the
    /// same three numbers for a program to read, and a check that asked *it*
    /// would be asking ring 3 what shape its own ring is — which a program
    /// installed at `user::RING` can answer however it likes, including with a
    /// `submissions` whose double overflows a `u32` and panics the debug
    /// kernel (D-115).
    #[cfg(feature = "selftest")]
    pub(crate) fn shape(&self) -> (u32, u32, u32) {
        let state = self.state.lock_leaf();
        (
            ring_proto::VERSION,
            state.submissions().capacity().entries(),
            state.completions().capacity().entries(),
        )
    }

    /// The header page, as a program would read it.
    ///
    /// Every number here is ring 3's the moment the ring is installed, so a
    /// caller must treat it as an assertion about *publication* and never as
    /// one about shape. The only honest use is comparing it against
    /// [`shape`](Self::shape) on a ring no program has been given.
    #[cfg(feature = "selftest")]
    pub(crate) fn published_shape(&self) -> (u32, u32, u32) {
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
fn read_u32(shared: &Shared, at: usize) -> u32 {
    let mut bytes = [0u8; 4];
    shared.read_at(at, &mut bytes);
    u32::from_le_bytes(bytes)
}

/// Write one little-endian `u32` into the shared pages.
fn write_u32(shared: &mut Shared, at: usize, value: u32) {
    shared.write_at(at, &value.to_le_bytes());
}
