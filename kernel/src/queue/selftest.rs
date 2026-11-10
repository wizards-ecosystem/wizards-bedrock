use core::num::NonZeroU64;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use wost::sync::Injection;

use super::ring::{
    publish_index, read_entry_u32, read_entry_u64, write_entry_u32, write_entry_u64,
};
use super::work::{
    FILE_WORK, reap_request_accounting_is_exact_for_test, work_state_round_trip_is_exact_for_test,
};
use super::{
    COMPLETION_BYTES, COMPLETIONS_AT, Completion, DATA_AT, ENTRIES_AT, Opcode, PendingKind,
    PendingTable, Queue, RingPages, SUBMISSION_BYTES, Submission, completion, header, submission,
};

#[cfg(feature = "selftest")]
pub(super) static REFUSE_WORKER: Injection = Injection::new();

#[cfg(feature = "selftest")]
pub(super) static WORKER_REAPS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
pub(super) static WORKER_RETAINED: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
pub(super) static WORK_PAUSED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static WORKER_IS_PAUSED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static WORK_RUNS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
pub(super) static LIVE_QUEUES: AtomicUsize = AtomicUsize::new(0);

impl Submission {
    /// A submission naming a handle and one argument about it.
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

    /// This submission with `length` bytes at its argument's place in the data area.
    #[cfg(feature = "selftest")]
    pub(crate) const fn length(self, length: u32) -> Self {
        Self { length, ..self }
    }

    /// This submission starting at `offset` in its file.
    #[cfg(feature = "selftest")]
    pub(crate) const fn offset(self, offset: u64) -> Self {
        Self { offset, ..self }
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

impl Queue {
    /// Reserve `count` completion slots without serving anything, to check admission alone.
    ///
    /// A check must use this or publish a tail, never both, which would double-count.
    #[cfg(feature = "selftest")]
    pub(crate) fn reserve_for_test(&self, count: usize) -> usize {
        let mut ring = self.ring.lock_leaf();
        (0..count)
            .take_while(|_| ring.protocol.submit().is_some())
            .count()
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

    /// Write a submission into the shared page as a program would, since the page is the boundary.
    #[cfg(feature = "selftest")]
    pub(crate) fn post_submission_for_test(&self, slot: usize, submission: Submission) {
        // Exhaustive, so an opcode added to `decode` and forgotten here fails the build.
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

    /// Read a submission back out of the shared page, so a check can show a hostile rewrite landed.
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

    /// The shape the kernel believes this ring has, from `protocol`, which ring 3 cannot reach.
    ///
    /// The header's copy is a program's to rewrite, even to a count whose double overflows a `u32`
    /// and panics the debug kernel.
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
    /// Ring 3's once installed, so a caller must treat it as publication, never shape, and compare
    /// it with [`shape`](Self::shape) only on a ring no program has been given.
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
/// Run a check through the production missing-worker predicate.
#[cfg(feature = "selftest")]
pub(crate) fn without_worker_for_test<T>(check: impl FnOnce() -> T) -> T {
    REFUSE_WORKER.with(true, check)
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
