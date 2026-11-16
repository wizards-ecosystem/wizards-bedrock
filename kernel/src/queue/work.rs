use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use native_abi::Rights;
use native_abi::ring::{Completion, Submission, status};
use wost::sync::{Leaf, SpinLock};

use super::ring::data_bounds;
use super::{Pending, PendingKind, Queue, ReadRequest, WorkPhase, refuse};
use crate::handle::{HandleTable, SlotReservation};
use crate::vfs;
use crate::wait::WaitQueue;

const WORK_DEPTH: usize = 8;

#[derive(Debug)]
pub(super) struct Work {
    ring: Arc<Queue>,
    pending: usize,
    expected: Pending,
    tag: WorkTag,
    // Inline, so trap-side admission makes no heap allocation.
    open_at: Option<OpenRequest>,
}

#[derive(Debug)]
struct OpenRequest {
    directory: vfs::Directory,
    held: Rights,
    path: [u8; MAX_PATH],
    len: usize,
    reservation: SlotReservation,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    state: SpinLock<Leaf, WorkState>,
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
            // The pending entry is claimed only after this guard is gone: it takes the ring's leaf.
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

static WORKER_READY: AtomicBool = AtomicBool::new(false);

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

    fn finish(self, status: u32, value: u64) {
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

        self.work
            .ring
            .settle_pending(self.work.pending, entry, status, value);
        self.work.ring.served.fetch_add(1, Ordering::Release);
        self.work.ring.waiters.wake_all();
    }
}

impl Queue {
    pub(super) fn open_at(
        self: &Arc<Self>,
        submission: Submission,
        table: &Arc<HandleTable>,
    ) -> Option<Completion> {
        let token = submission.token;

        let Ok(reservation) = table.reserve_slot() else {
            return Some(refuse(token, status::NO_HANDLE_SLOT));
        };
        let mut path = [0u8; MAX_PATH];
        let Some(len) = self.read_data(submission.argument, submission.length, &mut path) else {
            return Some(refuse(token, status::BAD_BUFFER));
        };

        let Ok((object, held)) = table.resolve(submission.handle) else {
            return Some(refuse(token, status::BAD_HANDLE));
        };
        let Some(directory) = object.as_directory().copied() else {
            return Some(refuse(token, status::BAD_HANDLE));
        };
        if !held.contains(Rights::TRAVERSE) {
            return Some(refuse(token, status::DENIED));
        }

        let Some((pending, expected)) = self.defer_open_at(token) else {
            return Some(refuse(token, status::NO_ROOM));
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
                reservation,
            },
        );
        if let Some(work) = FILE_WORK.submit(work) {
            work.remove_queued();
            return Some(refuse(token, status::NO_ROOM));
        }
        None
    }

    pub(super) fn read(
        self: &Arc<Self>,
        submission: Submission,
        table: &Arc<HandleTable>,
    ) -> Option<Completion> {
        let token = submission.token;

        let Ok((object, held)) = table.resolve(submission.handle) else {
            return Some(refuse(token, status::BAD_HANDLE));
        };
        let Some(file) = object.as_file().copied() else {
            return Some(refuse(token, status::BAD_HANDLE));
        };
        if !held.contains(Rights::READ) {
            return Some(refuse(token, status::DENIED));
        }

        let Some(bounds) = data_bounds(submission.argument, submission.length) else {
            return Some(refuse(token, status::BAD_BUFFER));
        };

        let wanted = (bounds.1).min(MAX_READ);
        // An offset past every possible file reads nothing; a cast would wrap it inside the file.
        let offset = u32::try_from(submission.offset).unwrap_or(u32::MAX);
        let request = ReadRequest {
            file,
            offset,
            at: bounds.0,
            wanted,
        };
        let Some((pending, expected)) = self.defer_read(token, request) else {
            return Some(refuse(token, status::NO_ROOM));
        };
        let work = Work::read(Arc::clone(self), pending, expected);
        if let Some(work) = FILE_WORK.submit(work) {
            work.remove_queued();
            return Some(refuse(token, status::NO_ROOM));
        }
        None
    }

    fn defer_open_at(&self, token: u32) -> Option<(usize, Pending)> {
        self.pending.lock_leaf().insert(
            token,
            PendingKind::OpenAt {
                phase: WorkPhase::Queued,
            },
        )
    }

    fn defer_read(&self, token: u32, request: ReadRequest) -> Option<(usize, Pending)> {
        self.pending.lock_leaf().insert(
            token,
            PendingKind::Read {
                request,
                phase: WorkPhase::Queued,
            },
        )
    }

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

pub(crate) fn start_worker() {
    match crate::thread::spawn("service-worker", work_forever, 0) {
        Ok(_) => {
            WORKER_READY.store(true, Ordering::Release);
            crate::info!("kernel: service worker ready; file queue has {WORK_DEPTH} slots");
        }
        Err(reason) => crate::warn!(
            "kernel: no service worker: {reason}; file operations and user launch are unavailable"
        ),
    }
}

pub(crate) fn worker_is_ready() -> bool {
    WORKER_READY.load(Ordering::Acquire)
}

pub(crate) fn request_reap() {
    assert!(
        add_reap_request(&REAP_REQUESTS),
        "process reap request accounting overflowed"
    );
    FILE_WORK.waiters.wake_all();
}

fn collect_requested_reap() -> bool {
    matches!(
        crate::user::collect(),
        crate::user::CollectOutcome::Retained
    )
}

fn work_forever(context: &mut crate::thread::ThreadContext, _: usize) {
    let mut retry_reap = false;
    loop {
        if retry_reap {
            if crate::time::is_calibrated() {
                crate::thread::sleep_ms(context, 1);
            } else {
                crate::thread::yield_now(context);
            }
        }
        if retry_reap || take_reap_request(&REAP_REQUESTS) {
            retry_reap = collect_requested_reap();
        }
        if let Some(work) = FILE_WORK.take() {
            service(work);
            continue;
        }
        // Queued work that `take` refused is being settled by a context this worker preempted.
        if FILE_WORK.queued.load(Ordering::Acquire) > 0 {
            crate::thread::yield_now(context);
            continue;
        }
        FILE_WORK.waiters.wait_unless(context, || {
            retry_reap
                || FILE_WORK.queued.load(Ordering::Acquire) > 0
                || REAP_REQUESTS.load(Ordering::Acquire) > 0
        });
    }
}

fn service(mut running: RunningWork) {
    let (status, value) = match running.work.tag {
        WorkTag::Read => {
            assert!(
                running.work.open_at.is_none(),
                "read file work carried an OpenAt payload"
            );
            let request = running.read_request();
            let mut scratch = [0u8; MAX_READ];
            match vfs::read(request.file, request.offset, &mut scratch[..request.wanted]) {
                Ok(read) => {
                    assert!(
                        running.work.ring.write_data(request.at, &scratch[..read]),
                        "a read bounded at admission did not fit the data area"
                    );
                    (status::OK, read as u64)
                }
                Err(_) => (status::IO, 0),
            }
        }
        WorkTag::OpenAt => {
            let OpenRequest {
                directory,
                held,
                path,
                len,
                reservation,
            } = running
                .work
                .open_at
                .take()
                .unwrap_or_else(|| panic!("OpenAt file work lost its request"));
            let node = match vfs::resolve(directory, &path[..len]) {
                Ok(node) => node,
                Err(error) => {
                    drop(reservation);
                    let status = match error {
                        vfs::Error::Path(_) => status::BAD_PATH,
                        vfs::Error::NotADirectory | vfs::Error::NotFound => status::NO_ENTRY,
                        vfs::Error::Backend(_) | vfs::Error::NoVolume => status::IO,
                    };
                    return running.finish(status, 0);
                }
            };
            let (produced, offered) = match node {
                vfs::Node::File(file) => (
                    crate::object::file(file),
                    held.intersection(Rights::UNIVERSAL).union(Rights::READ),
                ),
                vfs::Node::Directory(directory) => (crate::object::directory(directory), held),
                vfs::Node::Device(_) => {
                    drop(reservation);
                    return running.finish(status::DENIED, 0);
                }
            };
            match reservation.install(produced, offered, offered) {
                Ok(handle) => (status::OK, u64::from(handle)),
                Err(_) => (status::DENIED, 0),
            }
        }
    };

    running.finish(status, value);
}

// Far below a page, because the buffer is on a 32 KiB kernel stack.
const MAX_PATH: usize = 256;

// A larger request gets a short read, from a stack buffer.
const MAX_READ: usize = 512;
