use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use handle_space::Rights;
use wost::sync::SpinLock;

use super::ring::data_bounds;
#[cfg(feature = "selftest")]
use super::selftest::{
    REFUSE_WORKER, WORK_PAUSED, WORK_RUNS, WORKER_IS_PAUSED, WORKER_REAPS, WORKER_RETAINED,
};
use super::{
    Completion, Pending, PendingKind, Queue, ReadRequest, STATUS_BAD_BUFFER, STATUS_BAD_HANDLE,
    STATUS_BAD_PATH, STATUS_DENIED, STATUS_NO_ENTRY, STATUS_NO_HANDLE_SLOT, STATUS_NO_ROOM,
    STATUS_OK, Submission, WorkPhase, refuse,
};
use crate::handle::{HandleTable, SlotReservation};
use crate::wait::WaitQueue;

/// Global file-work admission. One worker makes controller ownership and
/// completion order explicit; the bound makes overload an immediate refusal.
const WORK_DEPTH: usize = 8;

#[derive(Debug)]
pub(super) struct Work {
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
pub(super) struct FileWorkQueue {
    state: SpinLock<WorkState>,
    pub(super) queued: AtomicUsize,
    pub(super) waiters: WaitQueue,
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

    pub(super) fn remove(&self, ring: &Queue, pending: usize, id: u64) -> Option<Work> {
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

pub(super) static FILE_WORK: FileWorkQueue = FileWorkQueue::new();

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
    /// Resolve through a directory without widening its authority.
    /// The output slot is reserved before any work that would owe a result.
    pub(super) fn open_at(
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
    pub(super) fn read(
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
}
#[cfg(feature = "selftest")]
pub(super) fn work_state_round_trip_is_exact_for_test() -> bool {
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
    // Keep the empty ring's modulo relation valid so only the exclusive head
    // bound rejects this forged state (D-396).
    state.head = WORK_DEPTH;
    state.tail = 0;
    let bad_head = !state.is_consistent();
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
        && bad_head
        && bad_tail
        && bad_length
        && bad_window
}

#[cfg(feature = "selftest")]
pub(super) fn reap_request_accounting_is_exact_for_test() -> bool {
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
