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
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use handle_space::Rights;
use ring_proto::{Capacity, Queues, SharedLayout};
use wost::arch::current::interrupt::TrapFrame;
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, SharedRing, USegment};
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

use crate::handle::{HandleTable, SlotReservation};
use crate::wait::WaitQueue;

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

impl Submission {
    /// A submission naming a handle and one argument about it.
    ///
    /// `Duplicate` and `Cancel`-by-handle: the handle goes in the field made
    /// for it rather than packed into the argument, which is what the widened
    /// entry bought (D-173).
    #[cfg(feature = "selftest")]
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
    #[cfg(feature = "selftest")]
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

#[cfg(feature = "selftest")]
pub(crate) fn pending_identity_exhaustion_is_terminal_for_test() -> bool {
    let mut pending = PendingTable::new();
    pending.next_id = NonZeroU64::new(u64::MAX - 1);
    let kind = PendingKind::Timer { deadline_ns: 1 };

    let first = pending
        .insert(1, kind)
        .is_some_and(|(index, entry)| index == 0 && entry.id == u64::MAX - 1);
    pending.slots[0] = None;
    let last = pending
        .insert(2, kind)
        .is_some_and(|(index, entry)| index == 0 && entry.id == u64::MAX);
    pending.slots[0] = None;
    let stopped = pending.insert(3, kind).is_none()
        && pending.next_id.is_none()
        && pending.slots.iter().all(Option::is_none);

    first
        && last
        && stopped
        && work_state_round_trip_is_exact_for_test()
        && reap_request_accounting_is_exact_for_test()
}

/// Global file-work admission. One worker makes controller ownership and
/// completion order explicit; the bound makes overload an immediate refusal.
const WORK_DEPTH: usize = 8;

#[derive(Debug)]
struct Work {
    ring: Arc<Queue>,
    pending: usize,
    expected: Pending,
    tag: WorkTag,
    /// Present exactly for `OpenAt`. Keeping the bounded path inline avoids a
    /// fallible heap allocation in trap-side admission while every queue slot
    /// remains statically accounted (D-381).
    open_at: Option<OpenRequest>,
}

#[derive(Debug)]
struct OpenRequest {
    directory: crate::vfs::Directory,
    held: Rights,
    path: [u8; MAX_PATH],
    len: usize,
    reservation: Option<SlotReservation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkTag {
    OpenAt,
    Read,
}

impl WorkTag {
    const fn matches(self, kind: PendingKind) -> bool {
        matches!(
            (self, kind),
            (Self::OpenAt, PendingKind::OpenAt { .. }) | (Self::Read, PendingKind::Read { .. })
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkClaim {
    Claimed(Pending),
    Retry,
    Stale,
}

#[derive(Debug)]
struct WorkKey {
    ring: Arc<Queue>,
    pending: usize,
    expected: Pending,
    tag: WorkTag,
}

#[derive(Debug)]
struct RunningWork {
    work: Work,
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

    /// Return the unconsumed owner when the fixed queue is full.
    fn push(&mut self, work: Work) -> Option<Work> {
        self.assert_invariants();
        if self.len == WORK_DEPTH {
            return Some(work);
        }
        assert!(
            self.slots[self.tail].is_none(),
            "file work tail points at an occupied slot"
        );
        self.slots[self.tail] = Some(work);
        self.tail = (self.tail + 1) % WORK_DEPTH;
        self.len += 1;
        self.assert_invariants();
        None
    }

    #[cfg(feature = "selftest")]
    fn pop(&mut self) -> Option<Work> {
        self.assert_invariants();
        if self.len == 0 {
            return None;
        }
        let Some(work) = self.slots[self.head].take() else {
            panic!("file work head points at an empty slot");
        };
        self.head = (self.head + 1) % WORK_DEPTH;
        self.len -= 1;
        self.assert_invariants();
        Some(work)
    }

    fn front(&self) -> Option<&Work> {
        self.slots.get(self.head).and_then(Option::as_ref)
    }

    fn remove(&mut self, wanted: impl Fn(&Work) -> bool) -> Option<Work> {
        self.assert_invariants();
        let position = (0..self.len).find(|offset| {
            self.slots[(self.head + offset) % WORK_DEPTH]
                .as_ref()
                .is_some_and(&wanted)
        })?;
        let index = (self.head + position) % WORK_DEPTH;
        let removed = self.slots[index].take();
        for offset in position..self.len - 1 {
            let from = (self.head + offset + 1) % WORK_DEPTH;
            let to = (self.head + offset) % WORK_DEPTH;
            self.slots[to] = self.slots[from].take();
        }
        self.len -= 1;
        self.tail = (self.head + self.len) % WORK_DEPTH;
        self.assert_invariants();
        removed
    }

    fn assert_invariants(&self) {
        assert!(
            self.is_consistent(),
            "file work queue state is inconsistent"
        );
    }

    fn is_consistent(&self) -> bool {
        if self.head >= WORK_DEPTH || self.tail >= WORK_DEPTH || self.len > WORK_DEPTH {
            return false;
        }
        self.tail == (self.head + self.len) % WORK_DEPTH
            && (0..WORK_DEPTH).all(|offset| {
                self.slots[(self.head + offset) % WORK_DEPTH].is_some() == (offset < self.len)
            })
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

    /// Return the unconsumed owner when the fixed queue is full.
    fn submit(&self, work: Work) -> Option<Work> {
        let mut state = self.state.lock_leaf();
        let refused = state.push(work);
        if refused.is_some() {
            return refused;
        }
        self.queued.store(state.len, Ordering::Release);
        drop(state);
        self.waiters.wake_all();
        None
    }

    fn take(&self) -> Option<RunningWork> {
        loop {
            // Copy only the key while holding the queue leaf. Claiming the
            // pending entry takes the ring's distinct leaf and therefore must
            // happen after this guard is gone (D-85).
            let key = {
                let state = self.state.lock_leaf();
                state.front().map(Work::key)
            }?;
            match key.ring.begin_work(&key) {
                WorkClaim::Retry => return None,
                WorkClaim::Stale => {
                    let mut state = self.state.lock_leaf();
                    let work = state.remove(|work| work.matches(&key));
                    self.queued.store(state.len, Ordering::Release);
                    drop(state);
                    drop(work);
                }
                WorkClaim::Claimed(expected) => {
                    let mut state = self.state.lock_leaf();
                    let work = state.remove(|work| work.matches(&key));
                    self.queued.store(state.len, Ordering::Release);
                    drop(state);
                    let Some(mut work) = work else {
                        panic!("claimed file work vanished from the bounded queue");
                    };
                    work.expected = expected;
                    return Some(RunningWork { work });
                }
            }
        }
    }

    fn remove(&self, ring: &Queue, pending: usize, id: u64) -> Option<Work> {
        let mut state = self.state.lock_leaf();
        let work = state.remove(|work| {
            core::ptr::eq(work.ring.as_ref(), ring)
                && work.pending == pending
                && work.expected.id == id
        });
        self.queued.store(state.len, Ordering::Release);
        work
    }
}

static FILE_WORK: FileWorkQueue = FileWorkQueue::new();

/// Set only after the runtime worker has a runnable scheduler slot.
static WORKER_READY: AtomicBool = AtomicBool::new(false);

/// One request per program moved into the zombie table.
static REAP_REQUESTS: AtomicUsize = AtomicUsize::new(0);

fn add_reap_request(requests: &AtomicUsize) -> bool {
    requests
        .fetch_update(Ordering::Release, Ordering::Relaxed, |pending| {
            pending.checked_add(1)
        })
        .is_ok()
}

fn take_reap_request(requests: &AtomicUsize) -> bool {
    requests
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
            pending.checked_sub(1)
        })
        .is_ok()
}

#[cfg(feature = "selftest")]
static REFUSE_WORKER: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
static WORKER_REAPS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
static WORKER_RETAINED: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
static WORK_PAUSED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
static WORKER_IS_PAUSED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
static WORK_RUNS: AtomicUsize = AtomicUsize::new(0);

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

#[cfg(feature = "selftest")]
static LIVE_QUEUES: AtomicUsize = AtomicUsize::new(0);

impl Work {
    fn open_at(ring: Arc<Queue>, pending: usize, expected: Pending, request: OpenRequest) -> Self {
        Self {
            ring,
            pending,
            expected,
            tag: WorkTag::OpenAt,
            open_at: Some(request),
        }
    }

    fn read(ring: Arc<Queue>, pending: usize, expected: Pending) -> Self {
        Self {
            ring,
            pending,
            expected,
            tag: WorkTag::Read,
            open_at: None,
        }
    }

    fn key(&self) -> WorkKey {
        WorkKey {
            ring: Arc::clone(&self.ring),
            pending: self.pending,
            expected: self.expected,
            tag: self.tag,
        }
    }

    fn matches(&self, key: &WorkKey) -> bool {
        Arc::ptr_eq(&self.ring, &key.ring)
            && self.pending == key.pending
            && self.expected == key.expected
            && self.tag == key.tag
    }

    fn remove_queued(self) {
        let mut pending = self.ring.pending.lock_leaf();
        let Some(current) = pending.slots.get(self.pending).copied().flatten() else {
            panic!("refused file work lost its pending entry");
        };
        assert!(
            !self.expected.settling
                && self.expected.kind.is_queued_work()
                && current == self.expected,
            "refused file work no longer owns its queued operation"
        );
        pending.slots[self.pending] = None;
    }
}

impl PendingKind {
    const fn is_queued_work(self) -> bool {
        matches!(
            self,
            Self::OpenAt {
                phase: WorkPhase::Queued
            } | Self::Read {
                phase: WorkPhase::Queued,
                ..
            }
        )
    }
}

impl RunningWork {
    fn read_request(&self) -> ReadRequest {
        let PendingKind::Read {
            request,
            phase: WorkPhase::Running,
        } = self.work.expected.kind
        else {
            unreachable!("running file work was made without a running read");
        };
        request
    }

    /// Return the still-live owner when terminal publication is refused.
    fn finish(self, status: u32, value: u64) -> Option<Self> {
        let entry = {
            let mut pending = self.work.ring.pending.lock_leaf();
            let Some(entry) = pending
                .slots
                .get_mut(self.work.pending)
                .and_then(Option::as_mut)
            else {
                panic!("running file work lost its pending entry");
            };
            assert!(
                !entry.settling && *entry == self.work.expected,
                "running file work no longer owns its operation"
            );
            entry.settling = true;
            *entry
        };

        if !self
            .work
            .ring
            .settle_pending(self.work.pending, entry, status, value)
        {
            return Some(self);
        }
        self.work.ring.served.fetch_add(1, Ordering::Release);
        self.work.ring.waiters.wake_all();
        None
    }
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
        let mut ring = self.ring.lock_leaf();
        (0..count)
            .take_while(|_| ring.protocol.submit().is_some())
            .count()
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

    /// Resolve through a directory without widening its authority.
    /// The output slot is reserved before any work that would owe a result.
    fn open_at(
        self: &Arc<Self>,
        submission: Submission,
        table: &Arc<HandleTable>,
    ) -> Option<Completion> {
        let token = submission.token;

        let Ok(reservation) = table.reserve_slot() else {
            return Some(refuse(token, STATUS_NO_HANDLE_SLOT));
        };
        let mut path = [0u8; MAX_PATH];
        let Some(len) = self.read_data(submission.argument, submission.length, &mut path) else {
            return Some(refuse(token, STATUS_BAD_BUFFER));
        };

        let Ok((object, held)) = table.resolve(submission.handle) else {
            return Some(refuse(token, STATUS_BAD_HANDLE));
        };
        let Some(directory) = object.as_directory().copied() else {
            return Some(refuse(token, STATUS_BAD_HANDLE));
        };
        if !held.contains(Rights::TRAVERSE) {
            return Some(refuse(token, STATUS_DENIED));
        }

        if !worker_is_ready() {
            return Some(refuse(token, STATUS_NO_ROOM));
        }
        let Some((pending, expected)) = self.defer_open_at(token) else {
            return Some(refuse(token, STATUS_NO_ROOM));
        };
        let work = Work::open_at(
            Arc::clone(self),
            pending,
            expected,
            OpenRequest {
                directory,
                held,
                path,
                len,
                reservation: Some(reservation),
            },
        );
        if let Some(work) = FILE_WORK.submit(work) {
            work.remove_queued();
            return Some(refuse(token, STATUS_NO_ROOM));
        }
        None
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

        // Bounded before anything is read: a length past the data area would
        // be the kernel writing outside the pages it shares.
        let Some(bounds) = data_bounds(submission.argument, submission.length) else {
            return Some(refuse(token, STATUS_BAD_BUFFER));
        };

        let wanted = (bounds.1).min(MAX_READ);
        // An offset past every possible file reads nothing; a cast would wrap
        // it to somewhere inside the file (D-174).
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
        let Some((pending, expected)) = self.defer_read(token, request) else {
            return Some(refuse(token, STATUS_NO_ROOM));
        };
        let work = Work::read(Arc::clone(self), pending, expected);
        if let Some(work) = FILE_WORK.submit(work) {
            work.remove_queued();
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
        self.ring
            .lock_leaf()
            .pages
            .read_payload_at(DATA_AT + at, &mut into[..len])
            .then_some(len)
    }

    /// Copy `bytes` into the data area at `offset`.
    fn write_data(&self, at: usize, bytes: &[u8]) -> bool {
        self.ring
            .lock_leaf()
            .pages
            .write_payload_at(DATA_AT + at, bytes)
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

    /// Record an open before offering it to the global worker.
    fn defer_open_at(&self, token: u32) -> Option<(usize, Pending)> {
        self.pending.lock_leaf().insert(
            token,
            PendingKind::OpenAt {
                phase: WorkPhase::Queued,
            },
        )
    }

    /// Record a read before offering it to the global worker.
    fn defer_read(&self, token: u32, request: ReadRequest) -> Option<(usize, Pending)> {
        let mut pending = self.pending.lock_leaf();
        let (index, expected) = pending.insert(
            token,
            PendingKind::Read {
                request,
                phase: WorkPhase::Queued,
            },
        )?;
        Some((index, expected))
    }

    /// Transfer one exact queued item to the thread-context worker.
    fn begin_work(&self, key: &WorkKey) -> WorkClaim {
        let mut pending = self.pending.lock_leaf();
        let Some(entry) = pending.slots.get_mut(key.pending).and_then(Option::as_mut) else {
            return WorkClaim::Stale;
        };
        if entry.settling {
            let live = Pending {
                settling: false,
                ..*entry
            };
            return if live == key.expected {
                WorkClaim::Retry
            } else {
                WorkClaim::Stale
            };
        }
        if *entry != key.expected || !key.tag.matches(entry.kind) {
            return WorkClaim::Stale;
        }
        let phase = match &mut entry.kind {
            PendingKind::OpenAt { phase } | PendingKind::Read { phase, .. } => phase,
            PendingKind::Timer { .. } => return WorkClaim::Stale,
        };
        if *phase != WorkPhase::Queued {
            return WorkClaim::Stale;
        }
        *phase = WorkPhase::Running;
        WorkClaim::Claimed(*entry)
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

    #[cfg(feature = "selftest")]
    pub(crate) fn quiet_completion_is_counted_for_test(&self) -> bool {
        if !self.is_quiet() {
            return false;
        }
        let before = self.served_count();
        self.served.fetch_add(1, Ordering::Release);
        self.is_quiet() && self.served_since(before) == 1
    }

    /// Wait for this ring's next completion through the production wait queue.
    #[cfg(feature = "selftest")]
    pub(crate) fn wait_for_test(
        &self,
        context: &mut crate::thread::ThreadContext,
        cancelled: &AtomicUsize,
    ) {
        let before = self.served_count();
        self.waiters.wait_unless(context, || {
            self.served_count() != before || cancelled.load(Ordering::Acquire) != 0
        });
    }

    /// Whether the production wait queue currently names a waiter.
    #[cfg(feature = "selftest")]
    pub(crate) fn has_waiter_for_test(&self) -> bool {
        !self.waiters.is_idle()
    }

    /// Release a test waiter when a failed orchestration has no completion.
    #[cfg(feature = "selftest")]
    pub(crate) fn release_waiter_for_test(&self) -> usize {
        self.waiters.wake_all()
    }

    /// How many operations are in flight.
    #[cfg(feature = "selftest")]
    pub(crate) fn in_flight(&self) -> usize {
        self.pending.lock_leaf().slots.iter().flatten().count()
    }

    /// Copy the indices out of the shared page, once.
    fn snapshot(&self) -> Snapshot {
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
    fn take(&self, snapshot: &Snapshot) -> Option<Submission> {
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
    fn post(&self, completion: Completion) -> bool {
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

    /// Operations admitted and not yet reported.
    #[cfg(feature = "selftest")]
    pub(crate) fn outstanding(&self) -> u32 {
        self.ring.lock_leaf().protocol.outstanding()
    }

    /// Whether the accounting still says a completion cannot be dropped.
    #[cfg(feature = "selftest")]
    pub(crate) fn is_consistent(&self) -> bool {
        self.ring.lock_leaf().protocol.is_consistent()
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
        // Exhaustive on purpose: an opcode added to `decode` and forgotten
        // here is a build failure, not a check exercising `Unknown` (D-161).
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
        assert!(
            self.ring.lock_leaf().pages.write_payload_at(at, &bytes),
            "a self-test submission slot was outside the shared pages"
        );
    }

    /// Move the submission tail as a program would.
    #[cfg(feature = "selftest")]
    pub(crate) fn publish_tail_for_test(&self, tail: u32) {
        publish_index(&self.ring.lock_leaf().pages, header::SUBMISSION_TAIL, tail);
    }

    /// Move the completion head as a program that has reaped would.
    #[cfg(feature = "selftest")]
    pub(crate) fn publish_completion_head_for_test(&self, head: u32) {
        publish_index(&self.ring.lock_leaf().pages, header::COMPLETION_HEAD, head);
    }

    /// Read a submission back out of the shared page.
    ///
    /// So a check can show that a hostile rewrite *landed* — without it, a
    /// check comparing only the completion is comparing two places the rewrite
    /// never touched, which is true whatever the kernel does (D-115).
    #[cfg(feature = "selftest")]
    pub(crate) fn read_submission(&self, slot: usize) -> Submission {
        let mut bytes = [0u8; SUBMISSION_BYTES];
        let at = ENTRIES_AT + slot * SUBMISSION_BYTES;
        assert!(
            self.ring.lock_leaf().pages.read_payload_at(at, &mut bytes),
            "a self-test submission slot was outside the shared pages"
        );
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
        self.ring
            .lock_leaf()
            .pages
            .read_payload_at(DATA_AT + at, into)
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
    #[cfg(feature = "selftest")]
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

    /// The shape the **kernel** believes this ring has.
    ///
    /// From `protocol`, which ring 3 cannot reach. The header page carries the
    /// same three numbers for a program to read, and a check that asked *it*
    /// would be asking ring 3 what shape its own ring is — which a program
    /// installed at `user::RING` can answer however it likes, including with a
    /// `submissions` whose double overflows a `u32` and panics the debug
    /// kernel (D-115).
    #[cfg(feature = "selftest")]
    pub(crate) fn shape(&self) -> (u32, u32, u32) {
        let ring = self.ring.lock_leaf();
        (
            ring_proto::VERSION,
            ring.protocol.submissions().capacity().entries(),
            ring.protocol.completions().capacity().entries(),
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
        let ring = self.ring.lock_leaf();
        (
            read_u32_volatile(&ring.pages, header::VERSION),
            read_u32_volatile(&ring.pages, header::SUBMISSIONS),
            read_u32_volatile(&ring.pages, header::COMPLETIONS),
        )
    }
}

#[cfg(feature = "selftest")]
impl Drop for Queue {
    fn drop(&mut self) {
        LIVE_QUEUES.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(feature = "selftest")]
fn work_state_round_trip_is_exact_for_test() -> bool {
    let Some(ring) = Queue::new() else {
        return false;
    };
    let work = |id| {
        Work::read(
            Arc::clone(&ring),
            0,
            Pending {
                id,
                token: 0,
                kind: PendingKind::Read {
                    request: ReadRequest {
                        file: crate::vfs::File::Tmp { at: 0, len: 0 },
                        offset: 0,
                        at: 0,
                        wanted: 0,
                    },
                    phase: WorkPhase::Queued,
                },
                settling: false,
            },
        )
    };
    let mut state = WorkState::new();

    for id in 1..=WORK_DEPTH as u64 {
        if state.push(work(id)).is_some() {
            return false;
        }
    }
    let refused = state
        .push(work(WORK_DEPTH as u64 + 1))
        .is_some_and(|work| work.expected.id == WORK_DEPTH as u64 + 1);
    for id in 1..=WORK_DEPTH as u64 / 2 {
        if state.pop().is_none_or(|work| work.expected.id != id) {
            return false;
        }
    }
    for id in WORK_DEPTH as u64 + 1..=WORK_DEPTH as u64 + WORK_DEPTH as u64 / 2 {
        if state.push(work(id)).is_some() {
            return false;
        }
    }
    let exact = (WORK_DEPTH as u64 / 2 + 1..=WORK_DEPTH as u64 + WORK_DEPTH as u64 / 2)
        .all(|id| state.pop().is_some_and(|work| work.expected.id == id));

    let empty = state.pop().is_none() && state.is_consistent();
    state.head = WORK_DEPTH;
    let bad_bounds = !state.is_consistent();
    state.head = 0;
    state.tail = WORK_DEPTH;
    let bad_tail = !state.is_consistent();
    state.tail = 0;
    state.len = WORK_DEPTH + 1;
    let bad_length = !state.is_consistent();
    state.len = 1;
    state.tail = 1;
    let bad_window = !state.is_consistent();
    state.len = 0;
    state.tail = 0;

    let removal_ready = (1..=3).all(|id| state.push(work(id)).is_none());
    let middle_removed = state
        .remove(|candidate| candidate.expected.id == 2)
        .is_some_and(|removed| removed.expected.id == 2);
    let removal_exact = state
        .pop()
        .is_some_and(|retained| retained.expected.id == 1)
        && state
            .pop()
            .is_some_and(|retained| retained.expected.id == 3)
        && state.pop().is_none()
        && state.is_consistent();

    refused
        && exact
        && empty
        && removal_ready
        && middle_removed
        && removal_exact
        && bad_bounds
        && bad_tail
        && bad_length
        && bad_window
}

#[cfg(feature = "selftest")]
fn reap_request_accounting_is_exact_for_test() -> bool {
    let requests = AtomicUsize::new(0);
    let added = add_reap_request(&requests)
        && add_reap_request(&requests)
        && requests.load(Ordering::Acquire) == 2;
    let drained = take_reap_request(&requests)
        && requests.load(Ordering::Acquire) == 1
        && take_reap_request(&requests)
        && requests.load(Ordering::Acquire) == 0
        && !take_reap_request(&requests);

    let exhausted = AtomicUsize::new(usize::MAX);
    let stopped = !add_reap_request(&exhausted) && exhausted.load(Ordering::Acquire) == usize::MAX;

    added && drained && stopped
}

/// Give deferred file service and process reclamation one runtime owner.
pub(crate) fn start_worker() {
    match crate::thread::spawn("service-worker", work_forever) {
        Ok(_) => {
            WORKER_READY.store(true, Ordering::Release);
            crate::info!("kernel: service worker ready; file queue has {WORK_DEPTH} slots");
        }
        Err(reason) => crate::warn!(
            "kernel: no service worker: {reason}; file operations and user launch are unavailable"
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
    assert!(
        add_reap_request(&REAP_REQUESTS),
        "process reap request accounting overflowed"
    );
    FILE_WORK.waiters.wake_all();
}

fn collect_requested_reap() -> bool {
    match crate::user::collect() {
        crate::user::CollectOutcome::Empty => false,
        crate::user::CollectOutcome::Retained => {
            #[cfg(feature = "selftest")]
            WORKER_RETAINED.fetch_add(1, Ordering::AcqRel);
            true
        }
        crate::user::CollectOutcome::Reclaimed(_) => {
            #[cfg(feature = "selftest")]
            WORKER_REAPS.fetch_add(1, Ordering::AcqRel);
            false
        }
    }
}

/// Drain bounded work, sleeping without a tick while nothing is queued.
fn work_forever() {
    let mut context = crate::thread::ThreadContext::require();
    let mut retry_reap = false;
    loop {
        #[cfg(feature = "selftest")]
        if WORK_PAUSED.load(Ordering::Acquire) {
            WORKER_IS_PAUSED.store(true, Ordering::Release);
            crate::thread::yield_now(&mut context);
            continue;
        }
        #[cfg(feature = "selftest")]
        WORKER_IS_PAUSED.store(false, Ordering::Release);

        if retry_reap {
            if crate::time::is_calibrated() {
                crate::thread::sleep_ms(&mut context, 1);
            } else {
                crate::thread::yield_now(&mut context);
            }
        }
        if retry_reap || take_reap_request(&REAP_REQUESTS) {
            retry_reap = collect_requested_reap();
        }
        if let Some(work) = FILE_WORK.take() {
            service(work);
            continue;
        }
        FILE_WORK.waiters.wait_unless(&mut context, || {
            retry_reap
                || FILE_WORK.queued.load(Ordering::Acquire) > 0
                || REAP_REQUESTS.load(Ordering::Acquire) > 0
        });
    }
}

/// Execute one file operation outside the syscall handler and every spin lock.
fn service(mut running: RunningWork) {
    #[cfg(feature = "selftest")]
    WORK_RUNS.fetch_add(1, Ordering::AcqRel);
    let (status, value) = match running.work.tag {
        WorkTag::Read => {
            assert!(
                running.work.open_at.is_none(),
                "read file work carried an OpenAt payload"
            );
            let request = running.read_request();
            let mut scratch = [0u8; MAX_READ];
            match crate::vfs::read(request.file, request.offset, &mut scratch[..request.wanted]) {
                Ok(read) if running.work.ring.write_data(request.at, &scratch[..read]) => {
                    (STATUS_OK, read as u64)
                }
                Ok(_) => (STATUS_BAD_BUFFER, 0),
                Err(crate::vfs::VfsError::Path(_)) => (STATUS_BAD_PATH, 0),
                Err(_) => (STATUS_NO_ENTRY, 0),
            }
        }
        WorkTag::OpenAt => {
            let mut request = running
                .work
                .open_at
                .take()
                .unwrap_or_else(|| panic!("OpenAt file work lost its request"));
            let reservation = request
                .reservation
                .take()
                .unwrap_or_else(|| panic!("OpenAt file work spent its reservation twice"));
            let node = match crate::vfs::resolve(request.directory, &request.path[..request.len]) {
                Ok(node) => node,
                Err(crate::vfs::VfsError::Path(_)) => {
                    drop(reservation);
                    return finish_work(running, STATUS_BAD_PATH, 0);
                }
                Err(_) => {
                    drop(reservation);
                    return finish_work(running, STATUS_NO_ENTRY, 0);
                }
            };
            let (produced, offered) = match node {
                crate::vfs::Node::File(file) => (
                    crate::object::file(file),
                    request
                        .held
                        .intersection(Rights::UNIVERSAL)
                        .union(Rights::READ),
                ),
                crate::vfs::Node::Directory(directory) => {
                    (crate::object::directory(directory), request.held)
                }
                crate::vfs::Node::Device(_) => {
                    drop(reservation);
                    return finish_work(running, STATUS_DENIED, 0);
                }
            };
            match reservation.install(produced, offered, offered) {
                Ok(handle) => (STATUS_OK, u64::from(handle)),
                Err(_) => (STATUS_DENIED, 0),
            }
        }
    };

    finish_work(running, status, value);
}

fn finish_work(mut running: RunningWork, status: u32, value: u64) {
    // Admission reserved the completion; the only refusal is D-192's
    // transient self-test injection, so keep ownership until it posts.
    loop {
        match running.finish(status, value) {
            None => break,
            Some(retry) => {
                running = retry;
                core::hint::spin_loop();
            }
        }
    }
}

/// Stop or resume work before it leaves the bounded queue.
#[cfg(feature = "selftest")]
pub(crate) fn pause_worker_for_test(paused: bool) {
    WORKER_IS_PAUSED.store(false, Ordering::Release);
    WORK_PAUSED.store(paused, Ordering::Release);
    if paused {
        FILE_WORK.waiters.wake_all();
    }
}

/// Whether the service worker has reached its requested pause barrier.
#[cfg(feature = "selftest")]
pub(crate) fn worker_is_paused_for_test() -> bool {
    WORKER_IS_PAUSED.load(Ordering::Acquire)
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

#[cfg(feature = "selftest")]
pub(crate) fn worker_retained_for_test() -> usize {
    WORKER_RETAINED.load(Ordering::Acquire)
}

/// Rings whose storage is still live.
#[cfg(feature = "selftest")]
pub(crate) fn live_for_test() -> usize {
    LIVE_QUEUES.load(Ordering::Acquire)
}

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

/// Read one immutable little-endian `u32` out of the shared pages.
#[cfg(feature = "selftest")]
fn read_u32_volatile(shared: &RingPages, at: usize) -> u32 {
    let mut bytes = [0u8; 4];
    assert!(
        shared.read_payload_at(at, &mut bytes),
        "an internal ring header field was outside the shared pages"
    );
    u32::from_le_bytes(bytes)
}

/// Write one immutable little-endian `u32` into the shared pages.
fn write_u32_volatile(shared: &mut RingPages, at: usize, value: u32) {
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
fn publish_index(shared: &RingPages, at: usize, value: u32) {
    if !shared.store_release_u32(at, value) {
        panic!("a ring index was out of bounds or misaligned");
    }
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
