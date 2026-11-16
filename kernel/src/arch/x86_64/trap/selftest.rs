use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use wost::arch::current::cpu;
use wost::arch::current::interrupt::TrapFrame;

// SMAP is read inside the handler, after the entry path ran.
pub(super) fn observe_syscall_entry(frame: &TrapFrame) {
    LAST_SYSCALL.record(frame);
    SMAP_ON_ENTRY.store(!cpu::access_flag_is_set(), Ordering::Release);
}

pub(super) static USER_COPY_RECOVERIES: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn user_copy_recoveries() -> usize {
    USER_COPY_RECOVERIES.load(Ordering::Acquire)
}

pub(super) static LAST_USER_FAULT: Recorder = Recorder::new();

pub(crate) fn last_user_fault() -> Record {
    LAST_USER_FAULT.read()
}

pub(super) static LAST_INVALID_USER_RETURN: Recorder = Recorder::new();

pub(crate) fn last_invalid_user_return() -> Record {
    LAST_INVALID_USER_RETURN.read()
}

static SMAP_ON_ENTRY: AtomicBool = AtomicBool::new(false);

pub(crate) fn smap_survived_the_last_syscall() -> bool {
    SMAP_ON_ENTRY.load(Ordering::Acquire)
}

static LAST_SYSCALL: Recorder = Recorder::new();

pub(crate) fn last_syscall() -> Record {
    LAST_SYSCALL.read()
}

// Atomics, not a lock, because this is written from trap context.
pub(super) struct Recorder {
    count: AtomicUsize,
    slot: AtomicUsize,
    rax: AtomicU64,
    vector: AtomicU64,
    flags: AtomicU64,
    stack_pointer: AtomicU64,
}

impl Recorder {
    const fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            slot: AtomicUsize::new(0),
            rax: AtomicU64::new(0),
            vector: AtomicU64::new(0),
            flags: AtomicU64::new(0),
            stack_pointer: AtomicU64::new(0),
        }
    }

    // The count is published last, so a reader that sees it sees the rest.
    pub(super) fn record(&self, frame: &TrapFrame) {
        self.slot
            .store(wost::arch::current::task::current(), Ordering::Relaxed);
        self.rax.store(frame.rax(), Ordering::Relaxed);
        self.vector
            .store(u64::from(frame.vector()), Ordering::Relaxed);
        self.flags.store(frame.flags(), Ordering::Relaxed);
        self.stack_pointer
            .store(frame.stack_pointer(), Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Release);
    }

    fn read(&self) -> Record {
        Record {
            count: self.count.load(Ordering::Acquire),
            slot: self.slot.load(Ordering::Relaxed),
            rax: self.rax.load(Ordering::Relaxed),
            vector: self.vector.load(Ordering::Relaxed),
            flags: self.flags.load(Ordering::Relaxed),
            stack_pointer: self.stack_pointer.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Record {
    pub(crate) count: usize,
    pub(crate) slot: usize,
    pub(crate) rax: u64,
    pub(crate) vector: u64,
    pub(crate) flags: u64,
    pub(crate) stack_pointer: u64,
}
