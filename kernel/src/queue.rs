// Every entry is copied out once and only the copy is validated; the page is never re-read.

use core::num::NonZeroU64;
use core::sync::atomic::{AtomicUsize, Ordering};

use channel_state::CallEvent;
use native_abi::Rights;
use native_abi::ring::{
    COMPLETIONS, Completion, Opcode, SUBMISSIONS, Submission, flags, header, status,
};
use ring_proto::{Capacity, Queues, SharedLayout};
use world_tree::NodeRef;
use wost::mm::pool::{Earmark, FrameKind};
use wost::mm::slab::{Hold, Refusal, Removal};
use wost::mm::{SharedBytes, SharedFrame, SharedRing};
use wost::sync::{Leaf, SpinLock};

use crate::arena::Local;
use crate::channel::Endpoint;
use crate::handle::{HandleError, Table, TableRef};
use crate::thread::ThreadContext;
use crate::timepage::WorldClock;
use crate::wait::WaitQueue;
use crate::world::{QueuePool, World, registry};

mod call;
mod channel;
mod memory;
mod ring;
#[cfg(feature = "selftest")]
mod selftest;
mod work;
mod world;

#[cfg(feature = "selftest")]
pub(crate) use selftest::TestQueue;

pub(crate) use call::{CallCell, Caller, abandon, close_call};
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
    // Ends when the world's teardown credits its frames; a cancel ends only the wait.
    Kill,
    // Armed on its endpoint until a write readies it; served at the next ring wait.
    ChannelRead {
        handle: u64,
        at: u64,
        endpoint: Local<Endpoint>,
        ready: bool,
    },
    // Its outcome is in the cell of the same index; ready once the peer reached one.
    ChannelCall {
        deadline_ns: u64,
        ready: bool,
    },
}

impl PendingKind {
    const fn is_cancellable(self) -> bool {
        matches!(
            self,
            Self::Timer { .. } | Self::Kill | Self::ChannelRead { .. } | Self::ChannelCall { .. }
        ) || self.is_queued_work()
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
            Self::ChannelCall { deadline_ns, .. } if deadline_ns != call::NEVER => {
                Some(deadline_ns)
            }
            Self::OpenAt { .. }
            | Self::Read { .. }
            | Self::Kill
            | Self::ChannelRead { .. }
            | Self::ChannelCall { .. } => None,
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
pub(crate) struct SharedQueue {
    pages: RingPages,
    data: SharedBytes,
    protocol: Queues,
}

#[derive(Debug)]
pub(crate) struct Queue {
    ring: SpinLock<Leaf, SharedQueue>,
    waiters: WaitQueue,
    pending: SpinLock<Leaf, PendingTable>,
    // Kernel-owned, so ring 3 cannot forge the edge a waiter samples before it sleeps.
    served: AtomicUsize,
    // Bumped by each write that readies a read here, so a sleeping waiter sees it.
    readied: AtomicUsize,
    clock: &'static WorldClock,
    // One call cell and its prepaid staging per pending slot.
    calls: [call::CallSlot; SUBMISSIONS as usize],
}

// The largest `Random` draw held on the stack at once.
const RANDOM_CHUNK: usize = 256;

// Names a queue by its world and its place in that world's arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct QueueRef {
    world: NodeRef,
    queue: Local<Queue>,
}

impl QueueRef {
    // The ring page and buffer 0, one data frame and one object frame for its holders each, beside
    // the queue object.
    pub(crate) fn frames_for() -> Option<u32> {
        QueuePool::frames_for(1)?.checked_add(4)
    }

    pub(crate) fn create(
        earmark: &mut Earmark,
        world_ref: NodeRef,
        world: &World,
        clock: &'static WorldClock,
        ring: SharedQueue,
    ) -> Self {
        let queue = world.queues.insert(earmark, || Queue::new(clock, ring));
        Self {
            world: world_ref,
            queue,
        }
    }

    pub(crate) fn hold(self) -> Option<Hold<Queue>> {
        registry::hold(self.world)?.queues.hold(self.queue)
    }

    // The world teardown's answer to a `WorldKill`; an entry cancelled since is skipped by its id.
    pub(crate) fn complete_kill(self, pending: usize, id: u64) {
        let Some(queue) = self.hold() else {
            return;
        };
        let Some((index, entry)) =
            queue.begin_settlement(|entry| entry.id == id && entry.kind == PendingKind::Kill)
        else {
            return;
        };
        assert!(index == pending, "a kill's pending entry moved");
        queue.settle_pending(index, entry, status::OK, 0);
        queue.served.fetch_add(1, Ordering::Release);
        queue.waiters.wake_all();
    }

    // The process's reference goes; `false` while a hold still borrows the queue.
    pub(crate) fn destroy(self) -> bool {
        registry::hold(self.world).is_none_or(|world| {
            matches!(
                world.queues.destroy(self.queue),
                Ok(_) | Err(Removal::Refused(Refusal::Stale))
            )
        })
    }
}

impl SharedQueue {
    pub(crate) fn new(earmark: &mut Earmark) -> Option<Self> {
        let ring = earmark.take(FrameKind::ArenaData)?;
        let ring_home = earmark.take(FrameKind::ArenaObject)?;
        let data = earmark.take(FrameKind::ArenaData)?;
        let data_home = earmark.take(FrameKind::ArenaObject)?;
        // Shared, so a space mapping them keeps them until its leaves are gone.
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
        let mut shared = ring.share(ring_home).into_ring(layout).ok()?;

        // Written before the pages are ever mapped, so no program sees a ring it cannot read.
        write_u32(&mut shared, header::VERSION, native_abi::ring::VERSION);
        write_u32(&mut shared, header::SUBMISSIONS, SUBMISSIONS);
        write_u32(&mut shared, header::COMPLETIONS, COMPLETIONS);

        let submission = Capacity::new(SUBMISSIONS).ok()?;
        let completion = Capacity::new(COMPLETIONS).ok()?;
        let state = Queues::new(submission, completion).ok()?;

        Some(Self {
            pages: shared,
            data: data.share(data_home).into_bytes(),
            protocol: state,
        })
    }

    // The ring page and buffer 0, for the space that maps them.
    pub(crate) fn frames(&self) -> (SharedFrame, SharedFrame) {
        (self.pages.frame(), self.data.frame())
    }
}

impl Queue {
    fn new(clock: &'static WorldClock, ring: SharedQueue) -> Self {
        Self {
            ring: SpinLock::new(ring),
            waiters: WaitQueue::new(),
            pending: SpinLock::new(PendingTable::new()),
            served: AtomicUsize::new(0),
            readied: AtomicUsize::new(0),
            clock,
            calls: core::array::from_fn(|_| call::CallSlot::new()),
        }
    }

    pub(crate) fn serve(
        &self,
        context: &mut ThreadContext,
        here: QueueRef,
        table: TableRef,
    ) -> usize {
        let Some((world, held)) = table.hold() else {
            return 0;
        };
        // Read once per call: a tail rewritten mid-service changes only the next call.
        let submission_tail = self.submission_tail();
        let mut served = 0;

        while let Some(submission) = self.take(submission_tail) {
            let process = Process {
                context: &mut *context,
                world: &world,
                table: &held,
                table_ref: table,
                here,
            };
            let Some(completion) = self.admit(process, submission) else {
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

    fn admit(&self, process: Process<'_, '_>, submission: Submission) -> Option<Completion> {
        let token = submission.token;
        let Some(used) = Fields::of(submission.opcode) else {
            return Some(refuse(token, status::UNKNOWN_OPCODE));
        };
        if !used.admit(&submission) {
            return Some(refuse(token, status::DENIED));
        }
        Some(match submission.opcode {
            Opcode::Echo => answer(token, submission.argument),
            Opcode::Uptime => answer(token, self.clock.now_ns()),
            Opcode::Duplicate => Self::duplicate(process, submission),
            Opcode::Close => Self::close(process, submission),
            Opcode::Timer => {
                if !crate::time::is_calibrated() {
                    return Some(refuse(token, status::UNAVAILABLE));
                }
                if self.defer(submission).is_some() {
                    return None;
                }
                refuse(token, status::NO_ROOM)
            }
            Opcode::Cancel => self.cancel(process.here, submission),
            Opcode::OpenAt => return self.open_at(process, submission),
            Opcode::Read => return self.read(process, submission),
            Opcode::Random => self.random(submission),
            Opcode::MemoryCreate => memory::create(process, submission),
            Opcode::MemoryCopy => memory::copy(process, submission),
            Opcode::MemorySeal => memory::seal(process, submission),
            Opcode::Map => memory::map(process, submission),
            Opcode::Unmap => memory::unmap(process, submission),
            Opcode::WorldCreate => world::create(self, process, submission),
            Opcode::WorldKill => return world::kill(self, process, submission),
            Opcode::WorldInspect => world::inspect(process, submission),
            Opcode::ChannelCreate => channel::create(process, submission),
            Opcode::ChannelWrite => channel::write(self, process, submission),
            Opcode::ChannelRead => return channel::read(self, process, submission),
            Opcode::ChannelCall => return call::call(self, process, submission),
            Opcode::Reply => call::reply(self, process, submission),
            Opcode::Unknown(_) => refuse(token, status::UNKNOWN_OPCODE),
        })
    }

    fn duplicate(process: Process<'_, '_>, submission: Submission) -> Completion {
        let token = submission.token;
        let Ok(reservation) = process.table_ref.reserve_slot() else {
            return refuse(token, status::NO_HANDLE_SLOT);
        };

        // A cast would drop the high half of a word ring 3 wrote and admit undefined rights bits.
        let Some(declared) = u32::try_from(submission.argument)
            .ok()
            .and_then(Rights::from_bits)
        else {
            return refuse(token, status::DENIED);
        };

        let (object, held) = match process.table.resolve(process.context, submission.handle) {
            Ok(resolved) => resolved,
            Err(error) => return refuse(token, unresolved(error)),
        };
        if !held.contains(Rights::DUPLICATE) || held.attenuate(declared).is_none() {
            return refuse(token, status::DENIED);
        }
        // A close racing this duplicate may have taken the last reference since the resolve.
        if !object.acquire(process.world) {
            return refuse(token, status::BAD_HANDLE);
        }
        match reservation.install(process.context, object, held, declared) {
            Ok(handle) => answer(token, handle),
            Err(_) => {
                object.release(process.world);
                refuse(token, status::DENIED)
            }
        }
    }

    fn close(process: Process<'_, '_>, submission: Submission) -> Completion {
        let closed = process
            .table
            .close(process.context, process.world, submission.handle);
        match closed {
            Ok(()) => answer(submission.token, 0),
            Err(_) => refuse(submission.token, status::BAD_HANDLE),
        }
    }

    // Filled in rounds, so a refused write or an unseeded generator stops before the next one.
    fn random(&self, submission: Submission) -> Completion {
        let token = submission.token;
        let Some((at, len)) = ring::buffer_bounds(&submission) else {
            return refuse(token, status::BAD_BUFFER);
        };
        let mut chunk = [0u8; RANDOM_CHUNK];
        let mut written = 0;
        let refusal = loop {
            if written == len {
                break None;
            }
            let round = &mut chunk[..RANDOM_CHUNK.min(len - written)];
            if crate::random::fill(round).is_err() {
                break Some(status::UNAVAILABLE);
            }
            if !self.write_data(at + written, round) {
                break Some(status::BAD_BUFFER);
            }
            written += round.len();
        };
        wost::mm::wipe(&mut chunk);
        match refusal {
            None => answer(token, len as u64),
            Some(code) => refuse(token, code),
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
        self.settle_with(
            index,
            entry,
            Completion {
                token: entry.token,
                status,
                value,
                extra: 0,
            },
        );
    }

    fn settle_with(&self, index: usize, entry: Pending, completion: Completion) {
        self.post(completion);
        let mut pending = self.pending.lock_leaf();
        let Some(current) = pending.slots.get(index).copied().flatten() else {
            panic!("terminal work lost its pending entry");
        };
        assert!(
            current.settling && current.id == entry.id,
            "terminal work no longer owns its pending entry"
        );
        pending.slots[index] = None;
    }

    fn cancel(&self, here: QueueRef, submission: Submission) -> Completion {
        // Truncating a wider word would let a program end an operation it did not name.
        let Ok(target) = u32::try_from(submission.argument) else {
            return refuse(submission.token, status::NOT_FOUND);
        };
        let Some((index, entry)) =
            self.begin_settlement(|entry| entry.token == target && entry.kind.is_cancellable())
        else {
            return refuse(submission.token, status::NOT_FOUND);
        };
        if let PendingKind::ChannelCall { .. } = entry.kind {
            // A call its peer already decided completes with that outcome instead.
            let Some(completion) = call::end(self, index, entry, CallEvent::Cancel) else {
                self.unclaim(index, entry);
                return refuse(submission.token, status::NOT_FOUND);
            };
            self.settle_with(index, entry, completion);
        } else {
            self.settle_pending(index, entry, status::CANCELLED, 0);
        }
        Self::disarm(here, index, entry);
        // Unclaimed work goes now, so a queued OpenAt returns its slot promise before this reports.
        drop(FILE_WORK.remove(here, index, entry.id));
        self.served.fetch_add(1, Ordering::Release);
        answer(submission.token, u64::from(entry.token))
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
            if let PendingKind::ChannelCall { .. } = entry.kind {
                match call::end(self, index, entry, CallEvent::Timeout) {
                    Some(completion) => self.settle_with(index, entry, completion),
                    None => {
                        self.outlive(index, entry);
                        continue;
                    }
                }
                due += 1;
                continue;
            }
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

    // A call whose reply is being copied in stops the round; the teardown comes back for it.
    pub(crate) fn abandon(&self, here: QueueRef) -> usize {
        let mut ended = 0;
        while let Some((index, entry)) = self.begin_settlement(|entry| entry.kind.is_cancellable())
        {
            if let PendingKind::ChannelCall { .. } = entry.kind {
                let decided = || {
                    let world = registry::hold(here.world)?;
                    call::forget(self, &world, index, entry)
                };
                let Some(completion) =
                    call::end(self, index, entry, CallEvent::Cancel).or_else(decided)
                else {
                    self.unclaim(index, entry);
                    break;
                };
                self.settle_with(index, entry, completion);
                ended += 1;
                continue;
            }
            self.settle_pending(index, entry, status::CANCELLED, 0);
            Self::disarm(here, index, entry);
            drop(FILE_WORK.remove(here, index, entry.id));
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

    pub(crate) fn wait(
        &self,
        context: &mut ThreadContext,
        here: QueueRef,
        table: TableRef,
    ) -> usize {
        let before = self.served_count();
        let readied = self.readied.load(Ordering::Acquire);
        let due = self.expire(crate::time::now_ns()) + self.serve_ready(context, here, table);
        if due > 0 || self.is_quiet() {
            return self.served_since(before);
        }

        self.waiters
            .wait_until(context, self.earliest_deadline(), || {
                self.served_count() != before || self.readied.load(Ordering::Acquire) != readied
            });
        self.expire(crate::time::now_ns());
        self.serve_ready(context, here, table);
        self.served_since(before)
    }

    // Each ready read is performed here, in the reader's context, and pays for what it installs.
    fn serve_ready(&self, context: &mut ThreadContext, here: QueueRef, table: TableRef) -> usize {
        let Some((world, held)) = table.hold() else {
            return 0;
        };
        let mut served = 0;
        while let Some((index, entry)) = self.begin_settlement(|entry| {
            matches!(
                entry.kind,
                PendingKind::ChannelRead { ready: true, .. }
                    | PendingKind::ChannelCall { ready: true, .. }
            )
        }) {
            let process = Process {
                context: &mut *context,
                world: &world,
                table: &held,
                table_ref: table,
                here,
            };
            let outcome = match entry.kind {
                PendingKind::ChannelCall { .. } => Some(call::serve(self, process, index, entry)),
                _ => channel::serve_ready(self, process, index, entry),
            };
            if let Some(completion) = outcome {
                self.settle_with(index, entry, completion);
                served += 1;
            }
        }
        if served > 0 {
            self.served.fetch_add(served, Ordering::Release);
            self.waiters.wake_all();
        }
        served
    }

    pub(crate) fn served_since(&self, before: usize) -> usize {
        self.served_count().wrapping_sub(before)
    }
}

// What one admission may reach: the caller's context, world and table, and this queue's name.
struct Process<'context, 'held> {
    context: &'context mut ThreadContext,
    world: &'held World,
    table: &'held Table,
    table_ref: TableRef,
    here: QueueRef,
}

// The submission fields an opcode reads and the flag bits it defines; every other field is zero.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Fields {
    handle: bool,
    second: bool,
    offset: bool,
    argument: bool,
    length: bool,
    buffer: bool,
    flags: u64,
}

impl Fields {
    const fn of(opcode: Opcode) -> Option<Self> {
        let none = Self {
            handle: false,
            second: false,
            offset: false,
            argument: false,
            length: false,
            buffer: false,
            flags: 0,
        };
        Some(match opcode {
            Opcode::Echo | Opcode::Timer | Opcode::Cancel => Self {
                argument: true,
                ..none
            },
            Opcode::Uptime | Opcode::ChannelCreate => none,
            Opcode::Duplicate => Self {
                handle: true,
                argument: true,
                ..none
            },
            Opcode::Close | Opcode::WorldKill | Opcode::WorldInspect => Self {
                handle: true,
                ..none
            },
            Opcode::OpenAt => Self {
                handle: true,
                length: true,
                buffer: true,
                ..none
            },
            Opcode::Read => Self {
                handle: true,
                offset: true,
                length: true,
                buffer: true,
                ..none
            },
            Opcode::Random => Self {
                length: true,
                buffer: true,
                ..none
            },
            Opcode::MemoryCreate => Self {
                argument: true,
                flags: flags::SHAREABLE,
                ..none
            },
            Opcode::MemoryCopy => Self {
                handle: true,
                second: true,
                offset: true,
                argument: true,
                length: true,
                ..none
            },
            Opcode::MemorySeal => Self {
                handle: true,
                second: true,
                flags: flags::SEAL_READ,
                ..none
            },
            Opcode::Map => Self {
                handle: true,
                second: true,
                offset: true,
                argument: true,
                length: true,
                flags: flags::MAP_WRITE | flags::MAP_EXECUTE,
                ..none
            },
            Opcode::Unmap => Self {
                handle: true,
                argument: true,
                length: true,
                ..none
            },
            Opcode::WorldCreate => Self {
                buffer: true,
                ..none
            },
            Opcode::ChannelWrite | Opcode::ChannelRead | Opcode::Reply => Self {
                handle: true,
                buffer: true,
                ..none
            },
            Opcode::ChannelCall => Self {
                handle: true,
                argument: true,
                buffer: true,
                ..none
            },
            Opcode::Unknown(_) => return None,
        })
    }

    const fn admit(self, submission: &Submission) -> bool {
        submission.flags & !self.flags == 0
            && (self.handle || submission.handle == 0)
            && (self.second || submission.second == 0)
            && (self.offset || submission.offset == 0)
            && (self.argument || submission.argument == 0)
            && (self.length || submission.length == 0)
            && (self.buffer || (submission.buffer == 0 && submission.at == 0))
    }
}

fn answer(token: u32, value: u64) -> Completion {
    Completion {
        token,
        status: status::OK,
        value,
        extra: 0,
    }
}

// A handle whose object died in transit answers `DEAD`; any other that does not resolve `BAD_HANDLE`.
fn unresolved(error: HandleError) -> u32 {
    if error == HandleError::Gone {
        status::DEAD
    } else {
        status::BAD_HANDLE
    }
}

fn refuse(token: u32, status: u32) -> Completion {
    Completion {
        token,
        status,
        value: 0,
        extra: 0,
    }
}
