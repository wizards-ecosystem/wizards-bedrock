// Every entry is copied out once and only the copy is validated; the page is never re-read.

use alloc::sync::Arc;
use core::num::NonZeroU64;
use core::sync::atomic::{AtomicUsize, Ordering};

use native_abi::Rights;
use native_abi::ring::{COMPLETIONS, Completion, Opcode, SUBMISSIONS, Submission, header, status};
use ring_proto::{Capacity, Queues, SharedLayout};
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::mm::{self, SharedRing, USegment};
use wost::sync::{Leaf, SpinLock};
use x86_paging::{Flags, PAGE_SIZE};

use crate::handle::HandleTable;
use crate::thread::ThreadContext;
use crate::timepage::WorldClock;
use crate::wait::WaitQueue;

mod ring;
#[cfg(feature = "selftest")]
mod selftest;
mod work;

use ring::write_u32;
use work::FILE_WORK;
pub(crate) use work::{request_reap, start_worker, worker_is_ready};

const PUBLICATION_WORDS: usize = 4;

type RingPages = SharedRing<PUBLICATION_WORDS>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Pending {
    // Never reused, so a stale worker cannot act on a reused slot.
    id: u64,
    token: u32,
    kind: PendingKind,
    // A terminal path owns this entry while posting its completion; every other path skips it.
    settling: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PendingKind {
    // Host time, the clock the scheduler wakes on.
    Timer {
        deadline_ns: u64,
    },
    OpenAt {
        phase: WorkPhase,
    },
    Read {
        request: ReadRequest,
        phase: WorkPhase,
    },
}

impl PendingKind {
    const fn is_cancellable(self) -> bool {
        matches!(self, Self::Timer { .. }) || self.is_queued_work()
    }

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

    const fn deadline(self) -> Option<u64> {
        match self {
            Self::Timer { deadline_ns } => Some(deadline_ns),
            Self::OpenAt { .. } | Self::Read { .. } => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ReadRequest {
    file: crate::vfs::File,
    offset: u32,
    at: usize,
    wanted: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WorkPhase {
    Queued,
    Running,
}

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

#[derive(Debug)]
struct SharedQueue {
    pages: RingPages,
    protocol: Queues,
}

#[derive(Debug)]
pub(crate) struct Queue {
    ring: SpinLock<Leaf, SharedQueue>,
    waiters: WaitQueue,
    pending: SpinLock<Leaf, PendingTable>,
    // Kernel-owned, so ring 3 cannot forge the edge a waiter samples before it sleeps.
    served: AtomicUsize,
    clock: &'static WorldClock,
}

// The largest `Random` draw held on the stack at once.
const RANDOM_CHUNK: usize = 256;

impl Queue {
    pub(crate) fn new(clock: &'static WorldClock) -> Option<Arc<Self>> {
        let run = mm::allocate_contiguous(native_abi::ring::BYTES / PAGE_SIZE as usize, 1)?;
        // Shared, never mapped as owned, so no unmap frees them while a program uses them.
        let layout = SharedLayout::new(
            native_abi::ring::BYTES,
            [
                header::SUBMISSION_HEAD,
                header::SUBMISSION_TAIL,
                header::COMPLETION_HEAD,
                header::COMPLETION_TAIL,
            ],
        )
        .ok()?;
        let mut shared = USegment::zeroed(run)?.share().into_ring(layout).ok()?;

        // Written before the pages are ever mapped, so no program sees a ring it cannot read.
        write_u32(&mut shared, header::VERSION, native_abi::ring::VERSION);
        write_u32(&mut shared, header::SUBMISSIONS, SUBMISSIONS);
        write_u32(&mut shared, header::COMPLETIONS, COMPLETIONS);

        let submission = Capacity::new(SUBMISSIONS).ok()?;
        let completion = Capacity::new(COMPLETIONS).ok()?;
        let state = Queues::new(submission, completion).ok()?;

        Some(Arc::new(Self {
            ring: SpinLock::new(SharedQueue {
                pages: shared,
                protocol: state,
            }),
            waiters: WaitQueue::new(),
            pending: SpinLock::new(PendingTable::new()),
            served: AtomicUsize::new(0),
            clock,
        }))
    }

    // `&mut self` skips the leaf guard, which may not span the mapper's allocation.
    pub(crate) fn install(&mut self, space: &mut AddressSpace, at: u64) -> Result<(), MapError> {
        space.map_shared_ring(
            at,
            &self.ring.get_mut().pages,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )
    }

    pub(crate) fn serve(self: &Arc<Self>, table: &Arc<HandleTable>) -> usize {
        // Read once per call: a tail rewritten mid-service changes only the next call.
        let submission_tail = self.submission_tail();
        let mut served = 0;

        while let Some(submission) = self.take(submission_tail) {
            let Some(completion) = self.admit(submission, table) else {
                continue;
            };
            self.post(completion);
            served += 1;
        }

        if served > 0 {
            self.served.fetch_add(served, Ordering::Release);
            self.waiters.wake_all();
        }
        served
    }

    pub(crate) fn served_count(&self) -> usize {
        self.served.load(Ordering::Acquire)
    }

    fn admit(
        self: &Arc<Self>,
        submission: Submission,
        table: &Arc<HandleTable>,
    ) -> Option<Completion> {
        let token = submission.token;
        Some(match submission.opcode {
            Opcode::Echo => Completion {
                token,
                status: status::OK,
                value: submission.argument,
            },
            Opcode::Uptime => Completion {
                token,
                status: status::OK,
                value: self.clock.now_ns(),
            },
            Opcode::Duplicate => Self::duplicate(submission, table),
            Opcode::Timer => {
                if !crate::time::is_calibrated() {
                    return Some(refuse(token, status::UNAVAILABLE));
                }
                if self.defer(submission).is_some() {
                    return None;
                }
                refuse(token, status::NO_ROOM)
            }
            Opcode::Cancel => self.cancel(submission),
            Opcode::OpenAt => return Self::open_at(self, submission, table),
            Opcode::Read => return Self::read(self, submission, table),
            Opcode::Random => self.random(submission),
            Opcode::Unknown(_) => refuse(token, status::UNKNOWN_OPCODE),
        })
    }

    fn duplicate(submission: Submission, table: &Arc<HandleTable>) -> Completion {
        let token = submission.token;
        let named = submission.handle;
        let Ok(reservation) = table.reserve_slot() else {
            return refuse(token, status::NO_HANDLE_SLOT);
        };

        // A cast would drop the high half of a word ring 3 wrote and admit undefined rights bits.
        let Some(declared) = u32::try_from(submission.argument)
            .ok()
            .and_then(Rights::from_bits)
        else {
            return refuse(token, status::DENIED);
        };

        let Ok((object, held)) = table.resolve(named) else {
            return refuse(token, status::BAD_HANDLE);
        };
        if !held.contains(Rights::DUPLICATE) {
            return refuse(token, status::DENIED);
        }

        match reservation.install(object, held, declared) {
            Ok(handle) => Completion {
                token,
                status: status::OK,
                value: u64::from(handle),
            },
            Err(_) => refuse(token, status::DENIED),
        }
    }

    // Filled in rounds, so a refused write or an unseeded generator stops before the next one.
    fn random(&self, submission: Submission) -> Completion {
        let token = submission.token;
        let Some((at, len)) = ring::data_bounds(submission.offset, submission.length) else {
            return refuse(token, status::BAD_BUFFER);
        };
        let mut chunk = [0u8; RANDOM_CHUNK];
        let mut written = 0;
        while written < len {
            let round = &mut chunk[..RANDOM_CHUNK.min(len - written)];
            if crate::random::fill(round).is_err() {
                return refuse(token, status::UNAVAILABLE);
            }
            if !self.write_data(at + written, round) {
                return refuse(token, status::BAD_BUFFER);
            }
            written += round.len();
        }
        Completion {
            token,
            status: status::OK,
            value: len as u64,
        }
    }

    fn defer(&self, submission: Submission) -> Option<u64> {
        // The clock is read before the leaf guard is taken, because a leaf guard encloses no call.
        let deadline_ns = self
            .clock
            .to_host_ns(self.clock.deadline_after(submission.argument));
        self.pending
            .lock_leaf()
            .insert(submission.token, PendingKind::Timer { deadline_ns })
            .map(|_| deadline_ns)
    }

    fn begin_settlement(&self, wanted: impl Fn(Pending) -> bool) -> Option<(usize, Pending)> {
        let mut pending = self.pending.lock_leaf();
        let (index, entry) = pending
            .slots
            .iter_mut()
            .enumerate()
            .filter_map(|(index, slot)| Some((index, slot.as_mut()?)))
            .filter(|(_, entry)| !entry.settling && wanted(**entry))
            .min_by_key(|(_, entry)| entry.id)?;
        entry.settling = true;
        Some((index, *entry))
    }

    fn settle_pending(&self, index: usize, entry: Pending, status: u32, value: u64) {
        self.post(Completion {
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
        pending.slots[index] = None;
    }

    fn cancel(&self, submission: Submission) -> Completion {
        // Truncating a wider word would let a program end an operation it did not name.
        let Ok(target) = u32::try_from(submission.argument) else {
            return refuse(submission.token, status::NOT_FOUND);
        };
        let Some((index, entry)) =
            self.begin_settlement(|entry| entry.token == target && entry.kind.is_cancellable())
        else {
            return refuse(submission.token, status::NOT_FOUND);
        };
        self.settle_pending(index, entry, status::CANCELLED, 0);
        // Unclaimed work goes now, so a queued OpenAt returns its slot promise before this reports.
        drop(FILE_WORK.remove(self, index, entry.id));
        self.served.fetch_add(1, Ordering::Release);
        Completion {
            token: submission.token,
            status: status::OK,
            value: u64::from(entry.token),
        }
    }

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
                (status::OK, self.clock.to_world_ns(deadline))
            } else {
                (status::UNAVAILABLE, 0)
            };
            self.settle_pending(index, entry, status, value);
            due += 1;
        }

        if due > 0 {
            self.served.fetch_add(due, Ordering::Release);
            self.waiters.wake_all();
        }
        due
    }

    pub(crate) fn abandon(&self) -> usize {
        let mut ended = 0;
        while let Some((index, entry)) = self.begin_settlement(|entry| entry.kind.is_cancellable())
        {
            self.settle_pending(index, entry, status::CANCELLED, 0);
            drop(FILE_WORK.remove(self, index, entry.id));
            ended += 1;
        }

        if ended > 0 {
            self.served.fetch_add(ended, Ordering::Release);
        }
        ended
    }

    // A teardown must see this before it releases anything an operation could still reach.
    pub(crate) fn is_quiet(&self) -> bool {
        self.pending.lock_leaf().slots.iter().all(Option::is_none)
    }

    pub(crate) fn earliest_deadline(&self) -> Option<u64> {
        self.pending
            .lock_leaf()
            .slots
            .iter()
            .flatten()
            .filter_map(|entry| entry.kind.deadline())
            .min()
    }

    pub(crate) fn wait(&self, context: &mut ThreadContext) -> usize {
        let before = self.served_count();
        let due = self.expire(crate::time::now_ns());
        if due > 0 || self.is_quiet() {
            return self.served_since(before);
        }

        self.waiters
            .wait_until(context, self.earliest_deadline(), || {
                self.served_count() != before
            });
        self.expire(crate::time::now_ns());
        self.served_since(before)
    }

    pub(crate) fn served_since(&self, before: usize) -> usize {
        self.served_count().wrapping_sub(before)
    }
}

fn refuse(token: u32, status: u32) -> Completion {
    Completion {
        token,
        status,
        value: 0,
    }
}
