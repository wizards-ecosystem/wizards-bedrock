//! The ring's kernel side: acquire peer indices, copy out once, validate the copy (CVE-2025-39816).

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

const SHARED_BYTES: u64 = 3 * PAGE_SIZE;

const PUBLICATION_WORDS: usize = 4;

type RingPages = SharedRing<PUBLICATION_WORDS>;

/// Ring-owned, so completion never meets a late-invalid user buffer or a user-copy fault window.
/// Reads pay one copy because the ABI has no memory-object authority for zero-copy.
const DATA_AT: usize = 2 * PAGE_SIZE as usize;

const DATA_BYTES: usize = PAGE_SIZE as usize;

const ENTRIES_AT: usize = PAGE_SIZE as usize;

const COMPLETIONS_AT: usize = ENTRIES_AT + SUBMISSIONS as usize * SUBMISSION_BYTES;

const SUBMISSION_BYTES: usize = 32;

const COMPLETION_BYTES: usize = 16;

/// A power of two.
const SUBMISSIONS: u32 = 8;

/// A power of two and at least twice the submissions, which Windows requires of RIO.
const COMPLETIONS: u32 = 16;

/// Byte offsets into the header page, which is the ABI.
mod header {
    /// So a ring can say its shape and be extended without guessing.
    pub(super) const VERSION: usize = 0;
    pub(super) const SUBMISSIONS: usize = 4;
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

mod submission {
    pub(super) const OPCODE: usize = 0;
    pub(super) const TOKEN: usize = 4;
    pub(super) const HANDLE: usize = 8;
    pub(super) const LENGTH: usize = 12;
    pub(super) const OFFSET: usize = 16;
    pub(super) const ARGUMENT: usize = 24;
}

mod completion {
    pub(super) const TOKEN: usize = 0;
    pub(super) const STATUS: usize = 4;
    pub(super) const VALUE: usize = 8;
}

/// What one submission asks for; none is privileged (`002`).
///
/// Every operation posts exactly one terminal completion, so an unknown opcode is refused with one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Opcode {
    /// Return the argument.
    Echo,
    /// Return the kernel's uptime in nanoseconds.
    Uptime,
    /// Duplicate `handle`, narrowing its rights to those `argument` carries.
    Duplicate,
    /// Complete once `argument` nanoseconds have passed.
    Timer,
    /// End the operation in flight whose token is `argument`.
    ///
    /// Its own operation and completion; the target still gets exactly one, carrying
    /// [`STATUS_CANCELLED`], which closes the double-completion class of io_uring (`012` §8).
    Cancel,
    /// Resolve a path from a directory handle and produce a handle for what it names.
    ///
    /// `handle` is the directory, and `argument` and `length` bound the path in the data area.
    OpenAt,
    /// Read bytes of file `handle` from `offset` into the data area.
    ///
    /// `argument` and `length` bound the region, checked at submission before the file is read.
    Read,
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

/// A submission copied out of the shared page exactly once.
///
/// Nothing here borrows the page, so nothing can be re-read after it has been validated.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Submission {
    pub(crate) opcode: Opcode,
    /// The caller's own tag, returned in the completion so it can match them.
    pub(crate) token: u32,
    pub(crate) handle: u32,
    pub(crate) length: u32,
    /// No handle has a cursor, so every read says where it starts (`012` §6).
    pub(crate) offset: u64,
    /// The operation's remaining argument, whose meaning is the opcode's.
    pub(crate) argument: u64,
}

/// What the kernel wrote back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Completion {
    pub(crate) token: u32,
    /// Zero, or the refusal.
    pub(crate) status: u32,
    pub(crate) value: u64,
}

/// Refusals a completion can carry. Zero is success.
pub(crate) const STATUS_OK: u32 = 0;
/// The opcode is not one this kernel serves.
pub(crate) const STATUS_UNKNOWN_OPCODE: u32 = 1;
/// No table slot for the handle this would produce, reported before the work is accepted.
pub(crate) const STATUS_NO_HANDLE_SLOT: u32 = 2;
/// The handle named is not one this program holds.
pub(crate) const STATUS_BAD_HANDLE: u32 = 3;
/// The rights asked for are not ones the program could pass on.
pub(crate) const STATUS_DENIED: u32 = 4;
/// The operation was ended early by a [`Opcode::Cancel`].
pub(crate) const STATUS_CANCELLED: u32 = 5;
/// A cancellation named no operation in flight.
///
/// Advisory, as io_uring's `-EALREADY` is, so a target that just completed makes no race a failure.
pub(crate) const STATUS_NOT_FOUND: u32 = 6;
/// Nothing left to track an in-flight operation with.
pub(crate) const STATUS_NO_ROOM: u32 = 7;
/// The path or buffer range is outside the data area, refused before anything is read or written.
pub(crate) const STATUS_BAD_BUFFER: u32 = 8;
/// The path names nothing, or names something of the wrong kind.
pub(crate) const STATUS_NO_ENTRY: u32 = 9;
/// The path is absolute or escaping; with no root and no parent, nothing could have been denied.
pub(crate) const STATUS_BAD_PATH: u32 = 10;
/// The service this needs is not available on this boot.
///
/// Refused at submission: no pending slot is kept and no completion waits on a clock never started.
pub(crate) const STATUS_UNAVAILABLE: u32 = 11;

/// The peer indices, read once per [`Queue::serve`] and never refreshed.
///
/// Every bound is checked against this copy, so a rewrite mid-service changes only the next call.
#[derive(Clone, Copy, Debug)]
struct Snapshot {
    submission_tail: u32,
    completion_head: u32,
}

/// An operation admitted and not yet completed, whose completion slot admission reserved.
///
/// That reservation is why its later completion can never fail to find room.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ReadRequest {
    file: crate::vfs::File,
    offset: u32,
    at: usize,
    wanted: usize,
}

/// The cancellation boundary for deferred file work.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// The kernel's for the ring's whole life: mapped into the process, never donated to it.
    pages: RingPages,
    /// The kernel's own protocol state, which userspace cannot reach.
    protocol: Queues,
}

/// A submission/completion ring bound to one process.
#[derive(Debug)]
pub(crate) struct Queue {
    /// One lock makes every kernel completion path one logical producer, so no later tail becomes
    /// visible before the entries below it. It serializes only kernel callers; acquire and release
    /// on the indices order the peer.
    ring: SpinLock<SharedQueue>,
    /// Woken when a completion is posted.
    waiters: WaitQueue,
    /// At most one per submission slot: an operation holds a submission's admission until it
    /// completes, so a larger table could never be filled.
    pending: SpinLock<PendingTable>,
    /// Completion generation sampled by waiters; kernel-owned, so ring 3 cannot forge the edge that
    /// closes the wait queue's check-before-sleep window.
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
    /// `None` if the frames cannot be allocated or cleared.
    pub(crate) fn new() -> Option<Arc<Self>> {
        let mut run = mm::allocate_contiguous((SHARED_BYTES / PAGE_SIZE) as usize, 1)?;
        if !run.zero() {
            return None;
        }
        // Shared one-way: never mapped as owned, so no unmap frees them while a program uses them.
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

        // Written before the pages are ever mapped, so no program sees a ring it cannot read.
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

    /// Map the pages into `space` at `at`, writable, never executable and not owned by the space,
    /// so a program that unmapped them could not free them.
    ///
    /// # Errors
    ///
    /// As [`AddressSpace::map_shared_ring`].
    ///
    /// `&mut self` reaches the run through `get_mut`: no leaf guard may be held across the
    /// mapping's allocations, and copying the address out would hand the mapper a bare number.
    pub(crate) fn install(&mut self, space: &mut AddressSpace, at: u64) -> Result<(), MapError> {
        space.map_shared_ring(
            at,
            &self.ring.get_mut().pages,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )
    }

    /// Serve every submission userspace has posted, and report how many.
    ///
    /// Admission happens here because moving a tail notifies nothing. Each reserves its completion
    /// before acceptance (`models/Ring.tla`), so one with nowhere to report stays published.
    pub(crate) fn serve(self: &Arc<Self>, table: &Arc<HandleTable>) -> usize {
        let snapshot = self.snapshot();
        let mut served = 0;

        while let Some(submission) = self.take(&snapshot) {
            // `None` was admitted and completes later against the reservation it holds.
            let Some(completion) = self.admit(submission, table) else {
                continue;
            };
            // Admission reserved this slot, so a failure breaks the model's invariant: terminal.
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

    /// The completion generation, sampled before sleeping and rechecked on wake so none is lost.
    pub(crate) fn served_count(&self) -> usize {
        self.served.load(Ordering::Acquire)
    }

    /// Carry out one submission, or `None` once it is admitted in flight and completes later.
    ///
    /// Every other outcome, every refusal included, is the operation's one terminal completion.
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
    /// The slot is promised first, so a full table is refused at submission; `install` then
    /// attenuates the rights, and every refusal returns the promise through its destructor.
    fn duplicate(&self, submission: Submission, table: &Arc<HandleTable>) -> Completion {
        let token = submission.token;
        let named = submission.handle;
        let Ok(reservation) = table.reserve_slot() else {
            return refuse(token, STATUS_NO_HANDLE_SLOT);
        };

        // A cast would drop the high half of a word ring 3 wrote and admit undefined rights bits.
        let Some(declared) = u32::try_from(submission.argument)
            .ok()
            .and_then(Rights::from_bits)
        else {
            return refuse(token, STATUS_DENIED);
        };

        let Ok((object, held)) = table.resolve(named) else {
            return refuse(token, STATUS_BAD_HANDLE);
        };
        // The gate `HandleTable::duplicate` applies, restated because this path spends a promise.
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

    /// Record an operation as in flight and say when it is due, reading the clock before the leaf.
    fn defer(&self, submission: Submission) -> Option<u64> {
        let deadline_ns = crate::time::now_ns().saturating_add(submission.argument);
        self.pending
            .lock_leaf()
            .insert(submission.token, PendingKind::Timer { deadline_ns })
            .map(|_| deadline_ns)
    }

    /// Mark one matching operation as settling; it keeps its slot until its post succeeds.
    ///
    /// Other terminal paths skip a settling entry, so a refusal can restore the live state without
    /// racing a second completion.
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

    /// End an operation early: the target gets exactly one [`STATUS_CANCELLED`] completion.
    ///
    /// This one's own completion is separate, posted by [`serve`](Self::serve) against its own
    /// reservation.
    fn cancel(&self, submission: Submission) -> Completion {
        // Truncating a wider word would let a program end an operation it did not name.
        let Ok(target) = u32::try_from(submission.argument) else {
            return refuse(submission.token, STATUS_NOT_FOUND);
        };
        let Some((index, entry)) =
            self.begin_settlement(|entry| entry.token == target && entry.kind.is_cancellable())
        else {
            return refuse(submission.token, STATUS_NOT_FOUND);
        };
        if !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
            // The cancellation still gets its reserved completion, while the target stays live
            // and may be retried.
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
    /// The kernel posts it, so a program woken after its timer finds the entry already queued. A
    /// revoked clock settles every Timer as unavailable whatever its deadline.
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
    /// A refused post keeps its pending entry, so the caller must keep the owner and retry until
    /// [`is_quiet`](Self::is_quiet) permits release.
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

    /// Whether nothing is in flight, which a teardown must see before it releases anything an
    /// operation could still reach.
    pub(crate) fn is_quiet(&self) -> bool {
        self.pending.lock_leaf().slots.iter().all(Option::is_none)
    }

    /// When the soonest operation in flight comes due; `None` tells a waiter there is nothing to
    /// wait for.
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

/// A refusal with no result; every refusal goes through it, so none can forget to zero its value.
fn refuse(token: u32, status: u32) -> Completion {
    Completion {
        token,
        status,
        value: 0,
    }
}
