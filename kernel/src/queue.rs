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

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use handle_space::Rights;
use ring_proto::{Capacity, Queues};
use wost::arch::current::interrupt::TrapFrame;
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, Shared, USegment};
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

use crate::handle::HandleTable;
use crate::wait::WaitQueue;

/// Bytes of shared page: the header and indices, the entries, and a data area.
const SHARED_BYTES: u64 = 3 * PAGE_SIZE;

/// Where the data area begins.
///
/// **A provided buffer, and the ring owns it.** `012` §8 warns that
/// `-ENOBUFS` at completion is the same late-failure shape D-151 refused for
/// handles, and the usual answer is to validate a caller's pointer at
/// submission. This kernel does something simpler and stronger: the buffer is
/// *part of the ring*, so it is always mapped, always the right size, and its
/// bounds are arithmetic rather than a walk of the caller's page tables. There
/// is no user pointer to check, no SMAP window to open and no fault to recover
/// (D-173).
///
/// The cost is a copy: a read goes device → kernel frame → here, where a
/// caller's own buffer could have been the DMA target. Zero-copy needs the
/// memory-object work of a later phase, and paying one copy for a boundary
/// with no pointer in it is the right trade until then.
const DATA_AT: usize = 2 * PAGE_SIZE as usize;

/// Bytes of data area.
const DATA_BYTES: usize = PAGE_SIZE as usize;

/// Where the header ends and the submission entries begin.
const ENTRIES_AT: usize = PAGE_SIZE as usize;

/// Where the completion entries begin, after every submission entry.
const COMPLETIONS_AT: usize = ENTRIES_AT + SUBMISSIONS as usize * SUBMISSION_BYTES;

/// Bytes in one submission entry.
///
/// **Thirty-two, widened in slice 9 from sixteen.** A read needs a handle, an
/// offset, a buffer and a length, which is more than one eight-byte argument
/// holds — and D-149 put the ABI slices first precisely so that the caller
/// which freezes the shape arrives *after* it can be changed. This is that
/// caller, and this is that change.
///
/// The opcode surface stays minimal on purpose: io_uring's exploit record is a
/// warning about surface growth, and Google's kernelCTF disables it outright
/// (`002`).
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
    /// The first operation whose completion carries **authority** rather than
    /// bytes, which is what D-151 said the ring was missing and what makes it
    /// a peer of the channel path rather than a side door around it. `handle`
    /// names the source and `argument` carries the rights the program is
    /// willing to hold.
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

impl Submission {
    /// A submission naming a handle and one argument about it.
    ///
    /// `Duplicate` and `Cancel`-by-handle: the handle goes in the field made
    /// for it rather than packed into the argument, which is what the widened
    /// entry bought (D-173).
    #[cfg_attr(not(feature = "selftest"), expect(dead_code))]
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

    /// A submission whose whole input is one argument.
    ///
    /// `Echo`, `Timer` and `Cancel`: the fields the widened entry added mean
    /// nothing to them. A constructor rather than four zeroes at every call
    /// site, because a zero handle is a *value* — writing it out invites a
    /// reader to think it matters, and the next person to add a field would
    /// have to touch every one of them (D-173).
    #[cfg_attr(not(feature = "selftest"), expect(dead_code))]
    pub(crate) const fn simple(opcode: Opcode, token: u32, argument: u64) -> Self {
        Self {
            opcode,
            token,
            handle: 0,
            length: 0,
            offset: 0,
            argument,
        }
    }
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
/// Reported at **submission**, which is the point: a program told now can
/// close something and ask again, where a program told at completion has
/// already been charged for work it cannot receive (D-151).
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
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone, Copy)]
enum PendingKind {
    /// Waiting for the monotonic clock.
    Timer { deadline_ns: u64 },
    /// Waiting for, or already owned by, the one file worker.
    Read {
        request: ReadRequest,
        state: ReadState,
    },
}

impl PendingKind {
    const fn is_cancellable(self) -> bool {
        matches!(
            self,
            Self::Timer { .. }
                | Self::Read {
                    state: ReadState::Queued,
                    ..
                }
        )
    }

    const fn deadline(self) -> Option<u64> {
        match self {
            Self::Timer { deadline_ns } => Some(deadline_ns),
            Self::Read { .. } => None,
        }
    }
}

/// Everything the worker needs after admission has dropped caller authority.
#[derive(Debug, Clone, Copy)]
struct ReadRequest {
    file: crate::vfs::File,
    offset: u32,
    at: usize,
    wanted: usize,
}

/// The cancellation boundary for a deferred read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadState {
    Queued,
    Running,
}

/// Fixed per-ring ownership for deferred operations.
#[derive(Debug)]
struct PendingTable {
    slots: [Option<Pending>; SUBMISSIONS as usize],
    next_id: u64,
}

impl PendingTable {
    const fn new() -> Self {
        Self {
            slots: [None; SUBMISSIONS as usize],
            next_id: 1,
        }
    }

    fn insert(&mut self, token: u32, kind: PendingKind) -> Option<(usize, u64)> {
        let index = self.slots.iter().position(Option::is_none)?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.slots[index] = Some(Pending {
            id,
            token,
            kind,
            settling: false,
        });
        Some((index, id))
    }
}

/// Global file-work admission. One worker makes controller ownership and
/// completion order explicit; the bound makes overload an immediate refusal.
const WORK_DEPTH: usize = 8;

#[derive(Debug)]
struct Work {
    ring: Arc<Queue>,
    pending: usize,
    id: u64,
}

#[derive(Debug)]
struct WorkState {
    slots: [Option<Work>; WORK_DEPTH],
    head: usize,
    tail: usize,
    len: usize,
}

impl WorkState {
    const fn new() -> Self {
        Self {
            slots: [const { None }; WORK_DEPTH],
            head: 0,
            tail: 0,
            len: 0,
        }
    }

    fn push(&mut self, work: Work) -> Result<(), Work> {
        if self.len == WORK_DEPTH {
            return Err(work);
        }
        self.slots[self.tail] = Some(work);
        self.tail = (self.tail + 1) % WORK_DEPTH;
        self.len += 1;
        Ok(())
    }

    fn pop(&mut self) -> Option<Work> {
        if self.len == 0 {
            return None;
        }
        let work = self.slots[self.head].take();
        self.head = (self.head + 1) % WORK_DEPTH;
        self.len -= 1;
        work
    }
}

#[derive(Debug)]
struct FileWorkQueue {
    state: SpinLock<WorkState>,
    queued: AtomicUsize,
    waiters: WaitQueue,
}

impl FileWorkQueue {
    const fn new() -> Self {
        Self {
            state: SpinLock::new(WorkState::new()),
            queued: AtomicUsize::new(0),
            waiters: WaitQueue::new(),
        }
    }

    fn submit(&self, work: Work) -> Result<(), Work> {
        self.state.lock_leaf().push(work)?;
        self.queued.fetch_add(1, Ordering::Release);
        self.waiters.wake_all();
        Ok(())
    }

    fn take(&self) -> Option<Work> {
        let work = self.state.lock_leaf().pop()?;
        self.queued.fetch_sub(1, Ordering::AcqRel);
        Some(work)
    }
}

static FILE_WORK: FileWorkQueue = FileWorkQueue::new();

/// Set only after the runtime worker has a runnable scheduler slot.
static WORKER_READY: AtomicBool = AtomicBool::new(false);

/// One request per program moved into the zombie table.
static REAP_REQUESTS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
static REFUSE_WORKER: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
static WORKER_REAPS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
static WORK_PAUSED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
static WORK_RUNS: AtomicUsize = AtomicUsize::new(0);

/// A submission/completion ring bound to one process.
#[derive(Debug)]
pub(crate) struct Queue {
    /// The shared pages. The kernel's, for the ring's whole life.
    shared: SpinLock<Shared>,
    /// The kernel's own protocol state, which userspace cannot reach.
    state: SpinLock<Queues>,
    /// Woken when a completion is posted.
    waiters: WaitQueue,
    /// Operations admitted and not yet completed.
    ///
    /// At most one per submission slot, which is not a coincidence: an
    /// operation occupies a submission's worth of the ring's admission until
    /// it completes, so a table any larger could never be filled.
    pending: SpinLock<PendingTable>,
    /// Completions posted since the ring was made.
    ///
    /// Kernel-side and monotonic, so a check can ask whether work was actually
    /// done without reading a number ring 3 can write (D-115's lesson applied
    /// where the answer matters most: the program's own tally is in the page).
    served: AtomicUsize,
    /// Terminal completions refused before changing accounting.
    settlement_refusals: AtomicUsize,
    /// Make one terminal post see the missing-reservation path.
    #[cfg(feature = "selftest")]
    refuse_next_post: AtomicBool,
}

static LIVE_QUEUES: AtomicUsize = AtomicUsize::new(0);

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

        LIVE_QUEUES.fetch_add(1, Ordering::AcqRel);
        Some(Arc::new(Self {
            shared: SpinLock::new(shared),
            state: SpinLock::new(state),
            waiters: WaitQueue::new(),
            pending: SpinLock::new(PendingTable::new()),
            served: AtomicUsize::new(0),
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
    pub(crate) fn serve(self: &Arc<Self>, table: &Arc<HandleTable>) -> usize {
        let snapshot = self.snapshot();
        let mut served = 0;

        while let Some(submission) = self.take(&snapshot) {
            // `None` is an operation that has been admitted and will complete
            // later. Its reservation stays held — that is the difference
            // between deferring an operation and dropping one.
            let Some(completion) = self.admit(submission, table) else {
                continue;
            };
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
            Opcode::OpenAt => self.open_at(submission, table),
            Opcode::Read => return self.read(submission, table),
            Opcode::Unknown(_) => refuse(token, STATUS_UNKNOWN_OPCODE),
        })
    }

    /// Produce a handle into the caller's own table.
    ///
    /// Three things happen in an order that is the whole point. The **slot is
    /// promised first**, so a full table is reported at submission rather than
    /// discovered at completion. The rights are then **attenuated on arrival**
    /// by `install`, so the ring narrows exactly as a channel does and cannot
    /// be the widening path around D-110. And every refusal below drops the
    /// reservation, which gives the promise back — not because each path
    /// remembers to, but because the type's destructor does (D-163).
    fn duplicate(&self, submission: Submission, table: &Arc<HandleTable>) -> Completion {
        let token = submission.token;
        let named = submission.handle;
        let Ok(reservation) = table.reserve_slot() else {
            return refuse(token, STATUS_NO_HANDLE_SLOT);
        };

        // `try_from`, not `as`. A cast would drop the high half of a word ring
        // 3 wrote, so `0xFFFF_FFFF_0000_0003` would be read as `DUPLICATE |
        // TRANSFER` instead of being refused — and refusing a rights word that
        // holds bits this kernel does not define is the entire reason
        // `from_bits` returns an `Option` (D-174).
        let Some(declared) = u32::try_from(submission.argument)
            .ok()
            .and_then(Rights::from_bits)
        else {
            return refuse(token, STATUS_DENIED);
        };

        let Ok((object, held)) = table.resolve(named) else {
            return refuse(token, STATUS_BAD_HANDLE);
        };
        // The same gate `HandleTable::duplicate` applies, restated here rather
        // than reached through it: that path inserts as well, and this one has
        // a promise to spend instead.
        if !held.contains(Rights::DUPLICATE) {
            return refuse(token, STATUS_DENIED);
        }

        match table.install(reservation, object, held, declared) {
            Ok(handle) => Completion {
                token,
                status: STATUS_OK,
                value: u64::from(handle),
            },
            Err(_) => refuse(token, STATUS_DENIED),
        }
    }

    /// Resolve a path from a directory handle and hand back what it names.
    ///
    /// The operation this phase was for. Three things happen in the order that
    /// matters: the **slot is promised first**, so a full table is reported at
    /// submission; the path is read from the ring's own data area, so there is
    /// no caller pointer to validate; and the rights on what comes back are
    /// **derived from the directory's**, never asked for.
    ///
    /// The derivation never widens a universal right as `012` §4 requires:
    ///
    /// - A directory preserves its parent's same-type rights unchanged; a
    ///   caller may subsequently Duplicate to narrow them.
    /// - A file preserves only universal rights the directory held and gains
    ///   `READ` because `TRAVERSE` authorized reaching its bytes. It never gets
    ///   `WRITE`, because there is no write path and a right that grants
    ///   nothing is a syscall name in disguise (D-78).
    /// - A device is refused. What a program will hold for one is slice 4's
    ///   `Device` capability, and putting it in a table is Phase 7's (D-172).
    fn open_at(&self, submission: Submission, table: &Arc<HandleTable>) -> Completion {
        let token = submission.token;

        let Ok(reservation) = table.reserve_slot() else {
            return refuse(token, STATUS_NO_HANDLE_SLOT);
        };
        let mut path = [0u8; MAX_PATH];
        let Some(len) = self.read_data(submission.argument, submission.length, &mut path) else {
            return refuse(token, STATUS_BAD_BUFFER);
        };

        let Ok((object, held)) = table.resolve(submission.handle) else {
            return refuse(token, STATUS_BAD_HANDLE);
        };
        let Some(directory) = object.as_directory().copied() else {
            return refuse(token, STATUS_BAD_HANDLE);
        };
        if !held.contains(Rights::TRAVERSE) {
            return refuse(token, STATUS_DENIED);
        }

        let node = match crate::vfs::resolve(directory, &path[..len]) {
            Ok(node) => node,
            Err(crate::vfs::VfsError::Path(_)) => return refuse(token, STATUS_BAD_PATH),
            Err(_) => return refuse(token, STATUS_NO_ENTRY),
        };

        let (produced, offered) = match node {
            crate::vfs::Node::File(file) => (
                crate::object::file(file),
                held.intersection(Rights::UNIVERSAL).union(Rights::READ),
            ),
            crate::vfs::Node::Directory(directory) => (crate::object::directory(directory), held),
            crate::vfs::Node::Device(_) => return refuse(token, STATUS_DENIED),
        };

        match table.install(reservation, produced, offered, offered) {
            Ok(handle) => Completion {
                token,
                status: STATUS_OK,
                value: u64::from(handle),
            },
            Err(_) => refuse(token, STATUS_DENIED),
        }
    }

    /// Read bytes of a file into the ring's data area.
    ///
    /// **Offsets are per operation** (`012` §6, D-152): the submission says
    /// where to start, and nothing anywhere holds a cursor. Two handles to one
    /// file have no observable coupling, which in a capability system is the
    /// difference between an interface and a covert channel.
    fn read(
        self: &Arc<Self>,
        submission: Submission,
        table: &Arc<HandleTable>,
    ) -> Option<Completion> {
        let token = submission.token;

        let Ok((object, held)) = table.resolve(submission.handle) else {
            return Some(refuse(token, STATUS_BAD_HANDLE));
        };
        let Some(file) = object.as_file().copied() else {
            return Some(refuse(token, STATUS_BAD_HANDLE));
        };
        if !held.contains(Rights::READ) {
            return Some(refuse(token, STATUS_DENIED));
        }

        // The destination, bounded before anything is read: a length that ran
        // past the data area would be the kernel writing outside the pages it
        // shares, which is the one thing this arrangement exists to make
        // arithmetic rather than a walk.
        let Some(bounds) = data_bounds(submission.argument, submission.length) else {
            return Some(refuse(token, STATUS_BAD_BUFFER));
        };

        let wanted = (bounds.1).min(MAX_READ);
        // An offset past every file this kernel can hold reads nothing, which
        // is what an offset past the end reads everywhere else here. A cast
        // would instead wrap it to somewhere *inside* the file — so a program
        // asking for byte four billion and one would be handed byte one
        // (D-174).
        let offset = u32::try_from(submission.offset).unwrap_or(u32::MAX);
        let request = ReadRequest {
            file,
            offset,
            at: bounds.0,
            wanted,
        };
        if !worker_is_ready() {
            return Some(refuse(token, STATUS_NO_ROOM));
        }
        let Some((pending, id)) = self.defer_read(token, request) else {
            return Some(refuse(token, STATUS_NO_ROOM));
        };
        let work = Work {
            ring: Arc::clone(self),
            pending,
            id,
        };
        if FILE_WORK.submit(work).is_err() {
            self.remove_queued_read(pending, id);
            return Some(refuse(token, STATUS_NO_ROOM));
        }
        None
    }

    /// Copy `length` bytes out of the data area at `offset`.
    fn read_data(&self, offset: u64, length: u32, into: &mut [u8]) -> Option<usize> {
        let (at, len) = data_bounds(offset, length)?;
        if len > into.len() {
            return None;
        }
        self.shared
            .lock_leaf()
            .read_at(DATA_AT + at, &mut into[..len])
            .then_some(len)
    }

    /// Copy `bytes` into the data area at `offset`.
    fn write_data(&self, at: usize, bytes: &[u8]) -> bool {
        self.shared.lock_leaf().write_at(DATA_AT + at, bytes)
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

    /// Record a read before offering it to the global worker.
    fn defer_read(&self, token: u32, request: ReadRequest) -> Option<(usize, u64)> {
        self.pending.lock_leaf().insert(
            token,
            PendingKind::Read {
                request,
                state: ReadState::Queued,
            },
        )
    }

    /// Roll back a read which the bounded global queue refused.
    fn remove_queued_read(&self, index: usize, id: u64) {
        let mut pending = self.pending.lock_leaf();
        if pending
            .slots
            .get(index)
            .copied()
            .flatten()
            .is_some_and(|entry| {
                entry.id == id
                    && matches!(
                        entry.kind,
                        PendingKind::Read {
                            state: ReadState::Queued,
                            ..
                        }
                    )
            })
        {
            pending.slots[index] = None;
        }
    }

    /// Transfer one still-queued read to the worker.
    fn begin_read(&self, index: usize, id: u64) -> Option<ReadRequest> {
        let mut pending = self.pending.lock_leaf();
        let entry = pending.slots.get_mut(index)?.as_mut()?;
        if entry.id != id || entry.settling {
            return None;
        }
        let PendingKind::Read { request, state } = &mut entry.kind else {
            return None;
        };
        if *state != ReadState::Queued {
            return None;
        }
        *state = ReadState::Running;
        Some(*request)
    }

    /// Post the terminal result of the exact read the worker owns.
    fn finish_read(&self, index: usize, id: u64, status: u32, value: u64) -> bool {
        let entry = {
            let mut pending = self.pending.lock_leaf();
            let Some(entry) = pending.slots.get_mut(index).and_then(Option::as_mut) else {
                return true;
            };
            if entry.id != id
                || entry.settling
                || !matches!(
                    entry.kind,
                    PendingKind::Read {
                        state: ReadState::Running,
                        ..
                    }
                )
            {
                return true;
            }
            entry.settling = true;
            *entry
        };

        if !self.settle_pending(index, entry, status, value) {
            return false;
        }
        self.served.fetch_add(1, Ordering::Release);
        self.waiters.wake_all();
        true
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
        if posted {
            pending.slots[index] = None;
        } else {
            pending.slots[index] = Some(Pending {
                settling: false,
                ..entry
            });
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
        // A token is a `u32`, so a word that does not fit is not a token and
        // names no operation. Truncating would have `0x1_0000_DEAD` cancel
        // `0xDEAD`, which is a program ending an operation it did not name
        // (D-174).
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
    /// is woken finds the entry already in its queue.
    pub(crate) fn expire(&self, now_ns: u64) -> usize {
        let mut due = 0;
        while let Some((index, entry)) = self.begin_settlement(|entry| {
            entry
                .kind
                .deadline()
                .is_some_and(|deadline| deadline <= now_ns)
        }) {
            let deadline = entry.kind.deadline().unwrap_or(now_ns);
            if !self.settle_pending(index, entry, STATUS_OK, deadline) {
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
    /// **The teardown path**, and D-153 as `012` §10 amends it: *exit is a
    /// wait, not a signal.* io_uring cancelled out of line for six years and
    /// the symptom was mount points staying busy after the process had exited.
    /// So this does not mark anything abandoned and leave: it ends each
    /// operation and posts the completion it was owed, into pages the zombie
    /// still holds. It stops if a terminal post refuses; the caller observes
    /// that through [`is_quiet`](Self::is_quiet) and retains the owner for a
    /// later retry (D-192).
    ///
    /// The completions go into a ring nobody will read, and that is not waste.
    /// Posting them is what spends the reservations, and a reservation that is
    /// never spent is a ring whose accounting says work is outstanding for
    /// ever — which is invisible until something asks, and slice 5's device
    /// will ask.
    ///
    /// A queued read is cancelled here; a running read is deliberately left
    /// owned by its `Arc<Queue>` until the worker posts its one terminal
    /// completion. The caller retains the zombie until
    /// [`is_quiet`](Self::is_quiet) becomes true (D-200).
    pub(crate) fn abandon(&self) -> usize {
        let mut ended = 0;
        while let Some((index, entry)) = self.begin_settlement(|entry| entry.kind.is_cancellable())
        {
            if !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
                break;
            }
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
            return due;
        }

        self.waiters
            .wait_from_trap_until(frame, self.earliest_deadline(), || {
                self.served_count() != before
            });
        self.expire(crate::time::now_ns());
        self.served_count().wrapping_sub(before)
    }

    /// How many operations are in flight.
    #[cfg(feature = "selftest")]
    pub(crate) fn in_flight(&self) -> usize {
        self.pending.lock_leaf().slots.iter().flatten().count()
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
            // Unreachable: `submit` has just produced into this ring, so there
            // is something to consume. Loud in a debug build for the reason
            // D-115 gave `serve`'s twin two functions down — every path past
            // this point spends the reservation, and a `None` here holds it
            // for ever. The ring then admits nothing more and the program waits
            // on a completion that will not come: a permanent wedge, which is
            // worse than a panic because nothing can diagnose it (D-133).
            match state.take() {
                Some(slot) => slot,
                None => {
                    debug_assert!(false, "a submission was reserved and then was not there");
                    return None;
                }
            }
        };

        let mut bytes = [0u8; SUBMISSION_BYTES];
        let at = ENTRIES_AT + slot * SUBMISSION_BYTES;
        // Unreachable for the same reason and with the same cost: `slot` is
        // below `SUBMISSIONS` and the entries are a page in, so the range is
        // inside the shared run by construction. This one has *consumed* the
        // submission as well as reserved, so it loses the operation too.
        if !self.shared.lock_leaf().read_at(at, &mut bytes) {
            debug_assert!(false, "a submission slot was outside the shared pages");
            return None;
        }

        // Validated after the copy, and the copy is what is used. A second
        // read here would be the double fetch the whole design refuses.
        let opcode = Opcode::decode(read_entry_u32(&bytes, submission::OPCODE));
        let token = read_entry_u32(&bytes, submission::TOKEN);
        let handle = read_entry_u32(&bytes, submission::HANDLE);
        let length = read_entry_u32(&bytes, submission::LENGTH);
        let offset = read_entry_u64(&bytes, submission::OFFSET);
        let argument = read_entry_u64(&bytes, submission::ARGUMENT);

        // The head userspace reads is published after the entry has been
        // copied out, so a program that watches it cannot see the kernel
        // release a slot it is still reading.
        let head = self.state.lock_leaf().submissions().head();
        write_u32(&mut self.shared.lock_leaf(), header::SUBMISSION_HEAD, head);

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
    fn post(&self, completion: Completion) -> bool {
        #[cfg(feature = "selftest")]
        if self.refuse_next_post.swap(false, Ordering::AcqRel) {
            return false;
        }
        let Some(slot) = self.state.lock_leaf().complete() else {
            return false;
        };

        let mut bytes = [0u8; COMPLETION_BYTES];
        write_entry_u32(&mut bytes, completion::TOKEN, completion.token);
        write_entry_u32(&mut bytes, completion::STATUS, completion.status);
        write_entry_u64(&mut bytes, completion::VALUE, completion.value);

        // `state` before `shared`, which is the order `take` uses. The two are
        // never held together, and taking them in opposite orders in the two
        // functions was an inversion pre-seeded for SMP (D-70, D-115).
        let tail = self.state.lock_leaf().completions().tail();

        let at = COMPLETIONS_AT + slot * COMPLETION_BYTES;
        let mut shared = self.shared.lock_leaf();
        assert!(
            shared.write_at(at, &bytes),
            "an internal completion slot was outside the ring pages"
        );
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
    #[cfg_attr(not(feature = "selftest"), expect(dead_code))]
    pub(crate) fn outstanding(&self) -> u32 {
        self.state.lock_leaf().outstanding()
    }

    /// Whether the accounting still says a completion cannot be dropped.
    #[cfg_attr(not(feature = "selftest"), expect(dead_code))]
    pub(crate) fn is_consistent(&self) -> bool {
        self.state.lock_leaf().is_consistent()
    }

    /// Refuse the next completion before it changes the reservation account.
    #[cfg(feature = "selftest")]
    pub(crate) fn refuse_next_post_for_test(&self) {
        self.refuse_next_post.store(true, Ordering::Release);
    }

    /// Terminal posts refused while their operations remained live.
    #[cfg(feature = "selftest")]
    pub(crate) fn settlement_refusals_for_test(&self) -> usize {
        self.settlement_refusals.load(Ordering::Acquire)
    }

    /// Write a submission into the shared page as a program would.
    ///
    /// For the checks. The kernel side must be exercised against bytes that
    /// arrived through the page rather than through a function call, because
    /// the page is the boundary and the function call is not.
    #[cfg(feature = "selftest")]
    pub(crate) fn post_submission_for_test(&self, slot: usize, submission: Submission) {
        // Exhaustive on purpose, and the compiler is what keeps it so: an
        // opcode added to `decode` and forgotten here is a build failure
        // rather than a check that silently exercises `Unknown` (D-161).
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
        let at = ENTRIES_AT + slot * SUBMISSION_BYTES;
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
        let mut bytes = [0u8; SUBMISSION_BYTES];
        let at = ENTRIES_AT + slot * SUBMISSION_BYTES;
        self.shared.lock_leaf().read_at(at, &mut bytes);
        Submission {
            opcode: Opcode::decode(read_entry_u32(&bytes, submission::OPCODE)),
            token: read_entry_u32(&bytes, submission::TOKEN),
            handle: read_entry_u32(&bytes, submission::HANDLE),
            length: read_entry_u32(&bytes, submission::LENGTH),
            offset: read_entry_u64(&bytes, submission::OFFSET),
            argument: read_entry_u64(&bytes, submission::ARGUMENT),
        }
    }

    /// Put bytes in the data area, as a program would before submitting.
    #[cfg(feature = "selftest")]
    pub(crate) fn write_data_for_test(&self, at: usize, bytes: &[u8]) -> bool {
        self.write_data(at, bytes)
    }

    /// Read the data area back, as a program would after a completion.
    #[cfg(feature = "selftest")]
    pub(crate) fn read_data_for_test(&self, at: usize, into: &mut [u8]) -> bool {
        self.shared.lock_leaf().read_at(DATA_AT + at, into)
    }

    /// A submission that names a handle, an offset, a buffer and a length.
    ///
    /// Test support: the four-field form is what `Read` and `OpenAt` use, and
    /// building it here keeps the field order in one place.
    #[cfg(feature = "selftest")]
    pub(crate) const fn about_at(
        opcode: Opcode,
        token: u32,
        handle: u32,
        offset: u64,
        at: u64,
        length: u32,
    ) -> Submission {
        Submission {
            opcode,
            token,
            handle,
            length,
            offset,
            argument: at,
        }
    }

    /// Read a completion back out of the shared page as a program would.
    pub(crate) fn read_completion(&self, slot: usize) -> Completion {
        let mut bytes = [0u8; COMPLETION_BYTES];
        let at = COMPLETIONS_AT + slot * COMPLETION_BYTES;
        self.shared.lock_leaf().read_at(at, &mut bytes);
        Completion {
            token: read_entry_u32(&bytes, completion::TOKEN),
            status: read_entry_u32(&bytes, completion::STATUS),
            value: read_entry_u64(&bytes, completion::VALUE),
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

impl Drop for Queue {
    fn drop(&mut self) {
        LIVE_QUEUES.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Give deferred file service and process reclamation one runtime owner.
pub(crate) fn start_worker() {
    match crate::thread::spawn("service-worker", work_forever) {
        Ok(_) => {
            WORKER_READY.store(true, Ordering::Release);
            crate::info!("kernel: service worker ready; file queue has {WORK_DEPTH} slots");
        }
        Err(reason) => crate::warn!(
            "kernel: no service worker: {reason}; reads and user launch are unavailable"
        ),
    }
}

/// Whether deferred service and process reclamation can make progress.
pub(crate) fn worker_is_ready() -> bool {
    let ready = WORKER_READY.load(Ordering::Acquire);
    #[cfg(feature = "selftest")]
    let ready = ready && !REFUSE_WORKER.load(Ordering::Acquire);
    ready
}

/// Run a check through the production missing-worker predicate.
#[cfg(feature = "selftest")]
pub(crate) fn without_worker_for_test<T>(check: impl FnOnce() -> T) -> T {
    let refused = REFUSE_WORKER.swap(true, Ordering::AcqRel);
    let result = check();
    REFUSE_WORKER.store(refused, Ordering::Release);
    result
}

/// Wake the service worker after ownership reaches the zombie table.
pub(crate) fn request_reap() {
    REAP_REQUESTS.fetch_add(1, Ordering::Release);
    FILE_WORK.waiters.wake_all();
}

/// Drain bounded work, sleeping without a tick while nothing is queued.
fn work_forever() {
    let mut context = crate::thread::ThreadContext::require();
    let mut handled_reaps = 0usize;
    loop {
        #[cfg(feature = "selftest")]
        if WORK_PAUSED.load(Ordering::Acquire) {
            crate::thread::yield_now(&mut context);
            continue;
        }

        let requested = REAP_REQUESTS.load(Ordering::Acquire);
        if handled_reaps != requested {
            handled_reaps = handled_reaps.wrapping_add(1);
            if crate::user::collect().is_some() {
                #[cfg(feature = "selftest")]
                WORKER_REAPS.fetch_add(1, Ordering::AcqRel);
            }
        }
        if let Some(work) = FILE_WORK.take() {
            service(work);
            continue;
        }
        FILE_WORK.waiters.wait_unless(&mut context, || {
            FILE_WORK.queued.load(Ordering::Acquire) > 0
                || REAP_REQUESTS.load(Ordering::Acquire) != handled_reaps
        });
    }
}

/// Execute one read outside the syscall handler and every spin lock.
fn service(work: Work) {
    let Some(request) = work.ring.begin_read(work.pending, work.id) else {
        return;
    };
    #[cfg(feature = "selftest")]
    WORK_RUNS.fetch_add(1, Ordering::AcqRel);
    let mut scratch = [0u8; MAX_READ];
    let (status, value) =
        match crate::vfs::read(request.file, request.offset, &mut scratch[..request.wanted]) {
            Ok(read) if work.ring.write_data(request.at, &scratch[..read]) => {
                (STATUS_OK, read as u64)
            }
            Ok(_) => (STATUS_BAD_BUFFER, 0),
            Err(crate::vfs::VfsError::Path(_)) => (STATUS_BAD_PATH, 0),
            Err(_) => (STATUS_NO_ENTRY, 0),
        };

    // Admission reserved the completion before the work existed. The only
    // refusal is the self-test injection for D-192, which is transient; keep
    // ownership here until the promised terminal record has been posted.
    while !work.ring.finish_read(work.pending, work.id, status, value) {
        core::hint::spin_loop();
    }
}

/// Stop or resume work before it leaves the bounded queue.
#[cfg(feature = "selftest")]
pub(crate) fn pause_worker_for_test(paused: bool) {
    WORK_PAUSED.store(paused, Ordering::Release);
}

/// Work items which still occupy global admission slots.
#[cfg(feature = "selftest")]
pub(crate) fn queued_work_for_test() -> usize {
    FILE_WORK.queued.load(Ordering::Acquire)
}

/// Reads transferred from the bounded queue to the worker.
#[cfg(feature = "selftest")]
pub(crate) fn worker_runs_for_test() -> usize {
    WORK_RUNS.load(Ordering::Acquire)
}

/// Programs reclaimed by the service worker.
#[cfg(feature = "selftest")]
pub(crate) fn worker_reaps_for_test() -> usize {
    WORKER_REAPS.load(Ordering::Acquire)
}

/// Rings whose storage is still live.
#[cfg(feature = "selftest")]
pub(crate) fn live_for_test() -> usize {
    LIVE_QUEUES.load(Ordering::Acquire)
}

/// A completion that carries a refusal and no result.
///
/// Every refusal in this file goes through it, so a refusal that forgot to
/// zero its value could not be written. D-79's rule — one terminal completion,
/// always — is expressed by `admit` returning one on every path that is not a
/// deferral, and by a deferral holding the reservation that guarantees a later
/// one.
/// The longest path an `OpenAt` may name.
///
/// Bounded because the buffer is on this stack, and bounded *here* rather than
/// by the data area's size because a kernel stack is thirty-two kilobytes and
/// a page is four.
const MAX_PATH: usize = 256;

/// The most bytes one `Read` moves.
///
/// The same reasoning: it is a stack buffer, and a caller asking for more gets
/// fewer bytes rather than a refusal — which is what every short read in this
/// kernel does and what a caller must already handle.
const MAX_READ: usize = 512;

/// Where in the data area `offset` and `length` name, if they name anything.
///
/// **Arithmetic, not a page walk.** The data area is part of the ring, so the
/// whole of "is this buffer usable" is two comparisons — and there is no
/// caller pointer, no SMAP window and no fault to recover (D-173).
fn data_bounds(offset: u64, length: u32) -> Option<(usize, usize)> {
    let at = usize::try_from(offset).ok()?;
    let len = length as usize;
    if len == 0 {
        return None;
    }
    let end = at.checked_add(len)?;
    (end <= DATA_BYTES).then_some((at, len))
}

fn refuse(token: u32, status: u32) -> Completion {
    Completion {
        token,
        status,
        value: 0,
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

fn read_entry_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_entry_u64(bytes: &[u8], at: usize) -> u64 {
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

fn write_entry_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_entry_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
