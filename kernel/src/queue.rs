//! The async submission/completion ring, kernel side.
//!
//! `libs/ring-proto` proves the state arithmetic; this module owns the boundary
//! with pages ring 3 may rewrite at any moment (D-79, D-112).
//!
//! **The rule that shapes every function below: acquire the peer's publication,
//! copy out exactly once, then validate the copy.** CVE-2025-39816 is literally
//! "always use `READ_ONCE()` to read ring pointers", and a double fetch is how
//! a validated value becomes a different value before it is used. The two
//! peer-owned indices are acquire-loaded into one [`Snapshot`]; entry and data
//! bytes are volatile-copied once; locally owned indices are release-stored
//! only after the corresponding copy. The acquire/release pair is the compiler
//! and SMP contract, while the spin lock serializes only kernel callers
//! (D-113, D-221).
//!
//! **The pages are the kernel's**, mapped into the process rather than donated
//! to it: `map_shared_ring` writes no `OWNED` bit, so the space cannot hand
//! these frames back to the allocator, and the ring holds the run for its own
//! life. That is the arrangement D-79 asked for — kernel-allocated ring pages,
//! single-owner rings — and it is why a program cannot free the memory the
//! kernel is about to write a completion into.

use alloc::sync::Arc;
use core::num::NonZeroU64;
#[cfg(feature = "selftest")]
use core::sync::atomic::AtomicBool;
use core::sync::atomic::{AtomicUsize, Ordering};

use handle_space::Rights;
use ring_proto::{Capacity, Queues, SharedLayout};
use wost::arch::current::interrupt::TrapFrame;
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, SharedRing, USegment};
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

use crate::handle::HandleTable;
use crate::wait::WaitQueue;

mod ring;
#[cfg(feature = "selftest")]
mod selftest;
mod work;

use ring::write_u32_volatile;
#[cfg(feature = "selftest")]
use selftest::LIVE_QUEUES;
#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    live_for_test, pause_worker_for_test, pending_identity_exhaustion_is_terminal_for_test,
    queued_work_for_test, without_worker_for_test, worker_is_paused_for_test,
    worker_reaps_for_test, worker_retained_for_test, worker_runs_for_test,
};
use work::FILE_WORK;
pub(crate) use work::{request_reap, start_worker, worker_is_ready};

/// Bytes of shared page: the header and indices, the entries, and a data area.
const SHARED_BYTES: u64 = 3 * PAGE_SIZE;

const PUBLICATION_WORDS: usize = 4;

type RingPages = SharedRing<PUBLICATION_WORDS>;

/// Where the data area begins.
///
/// Ring ownership keeps completion from discovering a late-invalid user
/// buffer and avoids a user-copy fault window (D-173). Reads pay one copy
/// because the current ABI exposes no memory-object authority for zero-copy.
const DATA_AT: usize = 2 * PAGE_SIZE as usize;

/// Bytes of data area.
const DATA_BYTES: usize = PAGE_SIZE as usize;

/// Where the header ends and the submission entries begin.
const ENTRIES_AT: usize = PAGE_SIZE as usize;

/// Where the completion entries begin, after every submission entry.
const COMPLETIONS_AT: usize = ENTRIES_AT + SUBMISSIONS as usize * SUBMISSION_BYTES;

const SUBMISSION_BYTES: usize = 32;

/// Bytes in one completion entry.
///
/// Sixteen, unchanged: a token, a status and a result. A completion that
/// needed more would be an operation answering with something other than an
/// outcome.
const COMPLETION_BYTES: usize = 16;

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

/// Byte offsets in one submission entry.
mod submission {
    pub(super) const OPCODE: usize = 0;
    pub(super) const TOKEN: usize = 4;
    pub(super) const HANDLE: usize = 8;
    pub(super) const LENGTH: usize = 12;
    pub(super) const OFFSET: usize = 16;
    pub(super) const ARGUMENT: usize = 24;
}

/// Byte offsets in one completion entry.
mod completion {
    pub(super) const TOKEN: usize = 0;
    pub(super) const STATUS: usize = 4;
    pub(super) const VALUE: usize = 8;
}

/// What one submission asks for.
///
/// Seven opcodes and no privileged ones (`002`). Anything else is refused with a
/// completion rather than ignored, because D-79's rule is that every operation
/// posts exactly one terminal completion, always.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opcode {
    /// Return the argument. The smallest thing that proves a round trip.
    Echo,
    /// Return the kernel's uptime in nanoseconds.
    Uptime,
    /// Duplicate a handle this program holds, narrowing its rights.
    ///
    /// The operation whose completion carries authority. `handle` names the
    /// source and `argument` carries the rights the program will retain.
    Duplicate,
    /// Complete once `argument` nanoseconds have passed.
    ///
    /// The first operation whose honest answer is *not yet*. Admitted now,
    /// completed later, and the reservation it took at admission is what
    /// guarantees the later completion has somewhere to go.
    Timer,
    /// End an operation still in flight, naming it by its token.
    ///
    /// **Its own operation**, with its own reservation and its own completion.
    /// It never substitutes for the target's, which still gets exactly one —
    /// carrying [`STATUS_CANCELLED`] instead of a result. That separation is
    /// the antidote to the double-completion class that produced two io_uring
    /// CVEs (`012` §8).
    Cancel,
    /// Resolve a path from a directory handle and produce a handle for what it
    /// names.
    ///
    /// `handle` is the directory, while `argument`/`length` bound the path in
    /// the ring data area. The completion **carries the new handle**, which is
    /// D-151's whole point arriving at the operation that most wants it: this
    /// is how a program comes to hold a file at all, and it holds it because
    /// something it already held could name it.
    OpenAt,
    /// Read bytes of a file into the ring-owned data area.
    ///
    /// `handle` is the file, `offset` says where to start, and
    /// `argument`/`length` bound a region in the shared data page. The bounds
    /// are checked at submission, before the file is read (D-173, D-174).
    Read,
    /// Anything else.
    Unknown(u32),
}

impl Opcode {
    const fn decode(value: u32) -> Self {
        match value {
            1 => Self::Echo,
            2 => Self::Uptime,
            3 => Self::Duplicate,
            4 => Self::Timer,
            5 => Self::Cancel,
            6 => Self::OpenAt,
            7 => Self::Read,
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
    /// The handle the operation is about, where it is about one.
    pub(crate) handle: u32,
    /// How many bytes, where that is a question.
    pub(crate) length: u32,
    /// Where in a file, where that is a question.
    ///
    /// **Per operation**, which is `012` §6 and D-152 arriving in the ABI:
    /// there is no cursor on a handle, so every read says where it starts.
    pub(crate) offset: u64,
    /// The operation's remaining argument, whose meaning is the opcode's.
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
/// The table has no room for the handle this operation would produce.
///
/// Reported at submission, before work is accepted that cannot return its
/// result (D-151).
pub(crate) const STATUS_NO_HANDLE_SLOT: u32 = 2;
/// The handle named is not one this program holds.
pub(crate) const STATUS_BAD_HANDLE: u32 = 3;
/// The rights asked for are not ones the program could pass on.
pub(crate) const STATUS_DENIED: u32 = 4;
/// The operation was ended early by a [`Opcode::Cancel`].
pub(crate) const STATUS_CANCELLED: u32 = 5;
/// A cancellation named no operation in flight.
///
/// Advisory, as io_uring's `-EALREADY` is: an operation that completed between
/// the submission of the cancel and its service is not an error, and reporting
/// it as one would make a race into a failure.
pub(crate) const STATUS_NOT_FOUND: u32 = 6;
/// Nothing left to track an in-flight operation with.
pub(crate) const STATUS_NO_ROOM: u32 = 7;
/// The path or buffer range is outside the ring-owned data area.
///
/// Checked at **submission**, before anything is read or written. An invalid
/// offset or length is refused before the operation can observe partial work
/// (`012` §8, D-173).
pub(crate) const STATUS_BAD_BUFFER: u32 = 8;
/// The path names nothing, or names something of the wrong kind.
pub(crate) const STATUS_NO_ENTRY: u32 = 9;
/// The path itself is malformed — absolute, or escaping.
///
/// **Not a denial.** There is no root and no parent, so there is nothing that
/// could have been denied (`012` §5, D-171).
pub(crate) const STATUS_BAD_PATH: u32 = 10;
/// The service this operation requires is not available on this boot.
///
/// A submission-time refusal: no pending slot is retained and no completion
/// promise depends on a clock which never started (D-201).
pub(crate) const STATUS_UNAVAILABLE: u32 = 11;

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

/// An operation admitted and not yet completed.
///
/// Its completion slot is **already reserved** — taken when the operation was
/// admitted — so the completion below can never fail to find room. That is
/// what makes deferring safe rather than merely convenient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pending {
    /// Private identity, so a stale worker cannot act on a reused slot.
    id: u64,
    /// The submission's tag, which is also how a cancellation names it.
    token: u32,
    /// What still owns the operation.
    kind: PendingKind,
    /// A terminal path owns this entry while posting its completion.
    settling: bool,
}

/// Why an admitted operation is still live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingKind {
    /// Waiting for the monotonic clock.
    Timer { deadline_ns: u64 },
    /// Waiting for, or already owned by, the one file worker.
    OpenAt { phase: WorkPhase },
    /// Waiting for, or already owned by, the one file worker.
    Read {
        request: ReadRequest,
        phase: WorkPhase,
    },
}

impl PendingKind {
    const fn is_cancellable(self) -> bool {
        matches!(
            self,
            Self::Timer { .. }
                | Self::OpenAt {
                    phase: WorkPhase::Queued
                }
                | Self::Read {
                    phase: WorkPhase::Queued,
                    ..
                }
        )
    }

    const fn deadline(self) -> Option<u64> {
        match self {
            Self::Timer { deadline_ns } => Some(deadline_ns),
            Self::OpenAt { .. } | Self::Read { .. } => None,
        }
    }
}

/// Everything the worker needs after admission has dropped caller authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReadRequest {
    file: crate::vfs::File,
    offset: u32,
    at: usize,
    wanted: usize,
}

/// The cancellation boundary for deferred file work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkPhase {
    Queued,
    Running,
}

/// Fixed per-ring ownership for deferred operations.
#[derive(Debug)]
struct PendingTable {
    slots: [Option<Pending>; SUBMISSIONS as usize],
    next_id: Option<NonZeroU64>,
}

impl PendingTable {
    const fn new() -> Self {
        Self {
            slots: [None; SUBMISSIONS as usize],
            next_id: Some(NonZeroU64::MIN),
        }
    }

    fn insert(&mut self, token: u32, kind: PendingKind) -> Option<(usize, Pending)> {
        let index = self.slots.iter().position(Option::is_none)?;
        let id = self.next_id?;
        self.next_id = id.get().checked_add(1).and_then(NonZeroU64::new);
        let entry = Pending {
            id: id.get(),
            token,
            kind,
            settling: false,
        };
        self.slots[index] = Some(entry);
        Some((index, entry))
    }
}

/// The shared pages and the private state which orders their publication.
#[derive(Debug)]
struct SharedQueue {
    /// The shared pages. The kernel's, for the ring's whole life.
    pages: RingPages,
    /// The kernel's own protocol state, which userspace cannot reach.
    protocol: Queues,
}

/// A submission/completion ring bound to one process.
#[derive(Debug)]
pub(crate) struct Queue {
    /// Pages and protocol state under one bounded publication lock.
    ///
    /// Keeping them together makes concurrent kernel completion paths one
    /// logical producer: no later tail can become visible before every entry
    /// below it has been written (D-221).
    ring: SpinLock<SharedQueue>,
    /// Woken when a completion is posted.
    waiters: WaitQueue,
    /// Operations admitted and not yet completed.
    ///
    /// At most one per submission slot, which is not a coincidence: an
    /// operation occupies a submission's worth of the ring's admission until
    /// it completes, so a table any larger could never be filled.
    pending: SpinLock<PendingTable>,
    /// Completion generation sampled by production waiters.
    ///
    /// Kernel ownership prevents ring 3 from forging the edge that closes the
    /// wait queue's check-before-sleep window.
    served: AtomicUsize,
    /// Terminal completions refused before changing accounting.
    #[cfg(feature = "selftest")]
    settlement_refusals: AtomicUsize,
    /// Make one terminal post see the missing-reservation path.
    #[cfg(feature = "selftest")]
    refuse_next_post: AtomicBool,
}

impl Queue {
    /// Build a ring and its pages, ready to be mapped.
    ///
    /// # Errors
    ///
    /// `None` if the frames cannot be allocated or cleared.
    pub(crate) fn new() -> Option<Arc<Self>> {
        let mut run = mm::allocate_contiguous((SHARED_BYTES / PAGE_SIZE) as usize, 1)?;
        if !run.zero() {
            return None;
        }
        // Pinned for sharing, one-way: these frames can never be mapped as
        // owned, so no unmap returns them while the program reaches them (D-115).
        let layout = SharedLayout::new(
            SHARED_BYTES as usize,
            [
                header::SUBMISSION_HEAD,
                header::SUBMISSION_TAIL,
                header::COMPLETION_HEAD,
                header::COMPLETION_TAIL,
            ],
        )
        .ok()?;
        let mut shared = USegment::from(run).share().into_ring(layout).ok()?;

        // The header is written before the pages are ever mapped, so a
        // program never sees a ring whose shape it cannot read.
        write_u32_volatile(&mut shared, header::VERSION, ring_proto::VERSION);
        write_u32_volatile(&mut shared, header::SUBMISSIONS, SUBMISSIONS);
        write_u32_volatile(&mut shared, header::COMPLETIONS, COMPLETIONS);

        let submission = Capacity::new(SUBMISSIONS).ok()?;
        let completion = Capacity::new(COMPLETIONS).ok()?;
        let state = Queues::new(submission, completion).ok()?;

        #[cfg(feature = "selftest")]
        LIVE_QUEUES.fetch_add(1, Ordering::AcqRel);
        Some(Arc::new(Self {
            ring: SpinLock::new(SharedQueue {
                pages: shared,
                protocol: state,
            }),
            waiters: WaitQueue::new(),
            pending: SpinLock::new(PendingTable::new()),
            served: AtomicUsize::new(0),
            #[cfg(feature = "selftest")]
            settlement_refusals: AtomicUsize::new(0),
            #[cfg(feature = "selftest")]
            refuse_next_post: AtomicBool::new(false),
        }))
    }

    /// Map the ring's pages into `space` at `at`.
    ///
    /// Writable, never executable, and **not owned by the space**: the frames
    /// stay the ring's, so a program that unmapped them could not free them.
    ///
    /// # Errors
    ///
    /// As [`AddressSpace::map_shared_ring`].
    /// `&mut self`, so the run is reached through `get_mut` rather than
    /// through a guard: mapping allocates page-table frames, and holding
    /// a leaf guard across an allocation is what `lock_leaf` forbids (D-85).
    /// Copying the address out instead would hand the mapper a bare number
    /// where the type is the evidence, which is the hole D-115 closed one
    /// function away.
    pub(crate) fn install(&mut self, space: &mut AddressSpace, at: u64) -> Result<(), MapError> {
        space.map_shared_ring(
            at,
            &self.ring.get_mut().pages,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )
    }

    /// Serve every submission userspace has posted, and report how many.
    ///
    /// **Admission happens here**, when the doorbell rings, because writing an
    /// entry and moving its tail does not otherwise notify the kernel (D-116).
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
    pub(crate) fn serve(self: &Arc<Self>, table: &Arc<HandleTable>) -> usize {
        let snapshot = self.snapshot();
        let mut served = 0;

        while let Some(submission) = self.take(&snapshot) {
            // `None` is an admitted operation that completes later; its held
            // reservation is what distinguishes deferring from dropping.
            let Some(completion) = self.admit(submission, table) else {
                continue;
            };
            // Admission reserved this slot, so a failure is the model's
            // invariant breaking; a silent wedge is made terminal (D-79, D-206).
            let posted = self.post(completion);
            assert!(posted, "a reserved completion slot was not there");
            served += 1;
        }

        if served > 0 {
            self.served.fetch_add(served, Ordering::Release);
            self.waiters.wake_all();
        }
        served
    }

    /// Current completion generation.
    ///
    /// Production waiters sample this before sleeping and recheck it in their
    /// wake condition, so a completion between those operations is not lost.
    /// Self-tests observe that same synchronization state (D-115, D-116).
    pub(crate) fn served_count(&self) -> usize {
        self.served.load(Ordering::Acquire)
    }

    /// Carry out one submission, or admit it as still in flight.
    ///
    /// `None` means the operation was accepted and its completion will be
    /// posted by [`expire`](Self::expire) or by a cancellation. Everything
    /// else — including every refusal — returns the one terminal completion
    /// D-79 requires.
    fn admit(
        self: &Arc<Self>,
        submission: Submission,
        table: &Arc<HandleTable>,
    ) -> Option<Completion> {
        let token = submission.token;
        Some(match submission.opcode {
            Opcode::Echo => Completion {
                token,
                status: STATUS_OK,
                value: submission.argument,
            },
            Opcode::Uptime => Completion {
                token,
                status: STATUS_OK,
                value: crate::time::now_ns(),
            },
            Opcode::Duplicate => self.duplicate(submission, table),
            Opcode::Timer => {
                if !crate::time::is_calibrated() {
                    return Some(refuse(token, STATUS_UNAVAILABLE));
                }
                if self.defer(submission).is_some() {
                    return None;
                }
                refuse(token, STATUS_NO_ROOM)
            }
            Opcode::Cancel => self.cancel(submission),
            Opcode::OpenAt => return Self::open_at(self, submission, table),
            Opcode::Read => return Self::read(self, submission, table),
            Opcode::Unknown(_) => refuse(token, STATUS_UNKNOWN_OPCODE),
        })
    }

    /// Produce a handle into the caller's own table.
    ///
    /// Three things happen in an order that is the whole point. The **slot is
    /// promised first**, so a full table is reported at submission rather than
    /// discovered at completion. `install` then attenuates the rights on
    /// arrival. Every refusal drops the reservation and returns the promise
    /// through the type's destructor (D-163).
    fn duplicate(&self, submission: Submission, table: &Arc<HandleTable>) -> Completion {
        let token = submission.token;
        let named = submission.handle;
        let Ok(reservation) = table.reserve_slot() else {
            return refuse(token, STATUS_NO_HANDLE_SLOT);
        };

        // `try_from`, not `as`: a cast would drop the high half of a word
        // ring 3 wrote and admit undefined rights bits (D-174).
        let Some(declared) = u32::try_from(submission.argument)
            .ok()
            .and_then(Rights::from_bits)
        else {
            return refuse(token, STATUS_DENIED);
        };

        let Ok((object, held)) = table.resolve(named) else {
            return refuse(token, STATUS_BAD_HANDLE);
        };
        // The gate `HandleTable::duplicate` applies, restated because that
        // path inserts and this one spends a promise instead.
        if !held.contains(Rights::DUPLICATE) {
            return refuse(token, STATUS_DENIED);
        }

        match reservation.install(object, held, declared) {
            Ok(handle) => Completion {
                token,
                status: STATUS_OK,
                value: u64::from(handle),
            },
            Err(_) => refuse(token, STATUS_DENIED),
        }
    }

    /// Record an operation as in flight, and say when it comes due.
    ///
    /// The deadline is computed **before** the guard is taken: reading the
    /// clock is a call, and this lock is a leaf (D-85).
    fn defer(&self, submission: Submission) -> Option<u64> {
        let deadline_ns = crate::time::now_ns().saturating_add(submission.argument);
        self.pending
            .lock_leaf()
            .insert(submission.token, PendingKind::Timer { deadline_ns })
            .map(|_| deadline_ns)
    }

    /// Mark one matching operation as being terminally reported.
    ///
    /// The entry remains in its slot until [`settle_pending`](Self::settle_pending)
    /// posts successfully. Other terminal paths skip the marker, so a refusal
    /// can restore the live state without racing a second completion (D-192).
    fn begin_settlement(&self, wanted: impl Fn(Pending) -> bool) -> Option<(usize, Pending)> {
        let mut pending = self.pending.lock_leaf();
        for (index, slot) in pending.slots.iter_mut().enumerate() {
            let Some(entry) = slot.as_mut() else {
                continue;
            };
            if !entry.settling && wanted(*entry) {
                entry.settling = true;
                return Some((index, *entry));
            }
        }
        None
    }

    /// Post one pending operation's terminal record, or leave it live.
    fn settle_pending(&self, index: usize, entry: Pending, status: u32, value: u64) -> bool {
        let posted = self.post(Completion {
            token: entry.token,
            status,
            value,
        });
        let mut pending = self.pending.lock_leaf();
        let Some(current) = pending.slots.get(index).copied().flatten() else {
            panic!("terminal work lost its pending entry");
        };
        assert!(
            entry.settling && current == entry,
            "terminal work no longer owns its pending entry"
        );
        if posted {
            pending.slots[index] = None;
        } else {
            pending.slots[index] = Some(Pending {
                settling: false,
                ..entry
            });
            #[cfg(feature = "selftest")]
            self.settlement_refusals.fetch_add(1, Ordering::AcqRel);
        }
        posted
    }

    /// End an operation early, giving it the completion it was owed.
    ///
    /// The target still gets **exactly one** terminal completion, carrying
    /// [`STATUS_CANCELLED`]. This function's own completion is separate and is
    /// posted by [`serve`](Self::serve), against its own reservation.
    fn cancel(&self, submission: Submission) -> Completion {
        // A word that does not fit a `u32` is not a token; truncating would
        // let a program end an operation it did not name (D-174).
        let Ok(target) = u32::try_from(submission.argument) else {
            return refuse(submission.token, STATUS_NOT_FOUND);
        };
        let Some((index, entry)) =
            self.begin_settlement(|entry| entry.token == target && entry.kind.is_cancellable())
        else {
            return refuse(submission.token, STATUS_NOT_FOUND);
        };
        if !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
            // The cancellation itself still gets the completion reserved for
            // it, while the target remains live and may be retried (D-192).
            return refuse(submission.token, STATUS_NO_ROOM);
        }
        // If the worker has not claimed this operation, remove the payload
        // now. For OpenAt that drops its handle-slot promise before reporting
        // cancellation; for Read it avoids spending worker time on a tombstone.
        drop(FILE_WORK.remove(self, index, entry.id));
        self.served.fetch_add(1, Ordering::Release);
        Completion {
            token: submission.token,
            status: STATUS_OK,
            value: u64::from(entry.token),
        }
    }

    /// Complete every operation whose deadline has passed, and report how many.
    ///
    /// The completion is posted **after** the deadline and by the kernel, not
    /// by the program asking again: a program that submits a timer, waits, and
    /// is woken finds the entry already in its queue. Clock revocation settles
    /// every Timer as unavailable regardless of its former deadline.
    pub(crate) fn expire(&self, now_ns: u64) -> usize {
        let clock_live = crate::time::is_calibrated();
        let mut due = 0;
        while let Some((index, entry)) = self.begin_settlement(|entry| {
            entry
                .kind
                .deadline()
                .is_some_and(|deadline| !clock_live || deadline <= now_ns)
        }) {
            let deadline = entry.kind.deadline().unwrap_or(now_ns);
            let (status, value) = if clock_live {
                (STATUS_OK, deadline)
            } else {
                (STATUS_UNAVAILABLE, 0)
            };
            if !self.settle_pending(index, entry, status, value) {
                break;
            }
            due += 1;
        }

        if due > 0 {
            self.served.fetch_add(due, Ordering::Release);
            self.waiters.wake_all();
        }
        due
    }

    /// End every operation still in flight, and report how many.
    ///
    /// A terminal post may refuse without losing its pending entry. The caller
    /// retains the owner and retries until [`is_quiet`](Self::is_quiet) permits
    /// release.
    pub(crate) fn abandon(&self) -> usize {
        let mut ended = 0;
        while let Some((index, entry)) = self.begin_settlement(|entry| entry.kind.is_cancellable())
        {
            if !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
                break;
            }
            drop(FILE_WORK.remove(self, index, entry.id));
            ended += 1;
        }

        if ended > 0 {
            self.served.fetch_add(ended, Ordering::Release);
        }
        ended
    }

    /// Whether nothing is in flight.
    ///
    /// What [`abandon`](Self::abandon) leaves true, and what a teardown must
    /// see before it releases anything an operation could still reach.
    pub(crate) fn is_quiet(&self) -> bool {
        self.pending.lock_leaf().slots.iter().all(Option::is_none)
    }

    /// When the soonest operation still in flight comes due.
    ///
    /// `None` when nothing is, which is what tells a waiter there is nothing
    /// to wait for.
    pub(crate) fn earliest_deadline(&self) -> Option<u64> {
        self.pending
            .lock_leaf()
            .slots
            .iter()
            .flatten()
            .filter_map(|entry| entry.kind.deadline())
            .min()
    }

    /// Wait through a syscall frame for a timer or worker completion.
    pub(crate) fn wait_from_trap(&self, frame: &TrapFrame) -> usize {
        let before = self.served_count();
        let due = self.expire(crate::time::now_ns());
        if due > 0 || self.is_quiet() {
            return self.served_since(before);
        }

        self.waiters
            .wait_from_trap_until(frame, self.earliest_deadline(), || {
                self.served_count() != before
            });
        self.expire(crate::time::now_ns());
        self.served_since(before)
    }

    fn served_since(&self, before: usize) -> usize {
        self.served_count().wrapping_sub(before)
    }
}

#[cfg(feature = "selftest")]
impl Drop for Queue {
    fn drop(&mut self) {
        LIVE_QUEUES.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A completion that carries a refusal and no result.
///
/// Every refusal goes through it, so one cannot forget to zero its value.
fn refuse(token: u32, status: u32) -> Completion {
    Completion {
        token,
        status,
        value: 0,
    }
}
