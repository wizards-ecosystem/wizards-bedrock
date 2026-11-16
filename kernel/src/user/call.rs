use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::task;
use wost::arch::current::{cpu, usercopy};

use super::reap::orphan;
use super::{BOOTSTRAPS, LAUNCHES, STACK_BYTES};
use crate::handle::HandleTable;
use crate::{info, queue, thread, warn};

const ENTRY_VERSION: u64 = 1;

const ENTRY_BYTES: usize = 24;

mod entry {
    pub(super) const VERSION: usize = 0;
    pub(super) const ARGUMENT: usize = 8;
    pub(super) const BOOTSTRAP_HANDLE: usize = 16;
}

const ENTRY_STACK_BYTES: u64 = (ENTRY_BYTES as u64 + 15) & !15;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Call {
    Exit,
    RingEnter,
    RingWait,
    ThreadBase,
    Unknown(u64),
}

impl Call {
    const fn decode(value: u64) -> Self {
        match value {
            0 => Self::Exit,
            1 => Self::RingEnter,
            2 => Self::ThreadBase,
            3 => Self::RingWait,
            other => Self::Unknown(other),
        }
    }
}

static REFUSED_CALLS: AtomicUsize = AtomicUsize::new(0);

static LAST_REFUSED: AtomicUsize = AtomicUsize::new(0);

// One warning per boot, so a looping program cannot flood the console.
static REPORTED: AtomicBool = AtomicBool::new(false);

static EXECUTED: AtomicBool = AtomicBool::new(false);
static EXECUTION_REPORTED: AtomicBool = AtomicBool::new(false);

pub(crate) fn report_execution() -> bool {
    if !EXECUTED.load(Ordering::Acquire) || EXECUTION_REPORTED.swap(true, Ordering::AcqRel) {
        return false;
    }
    info!("user: ring 3 call observed");
    true
}

pub(crate) fn report_refusals() -> bool {
    let refused = REFUSED_CALLS.load(Ordering::Acquire);
    if refused == 0 || REPORTED.swap(true, Ordering::AcqRel) {
        return false;
    }
    let number = LAST_REFUSED.load(Ordering::Acquire);
    warn!("user: refused {refused} call(s), last was {number:#x}");
    true
}

#[cfg(feature = "selftest")]
pub(crate) fn refusals() -> (usize, usize) {
    (
        REFUSED_CALLS.load(Ordering::Acquire),
        LAST_REFUSED.load(Ordering::Acquire),
    )
}

const ENOSYS: u64 = (-38i64) as u64;

const EINVAL: u64 = (-22i64) as u64;

fn answer_call(frame: &TrapFrame, value: u64) {
    assert!(
        interrupt::answer(frame, value),
        "the live syscall frame could not be answered"
    );
}

// Trap context: no allocation, no blocking path but `wait_from_trap`, no lock but a leaf.
pub(crate) fn call(frame: &TrapFrame) {
    let slot = task::current();
    EXECUTED.store(true, Ordering::Release);
    match Call::decode(frame.rax()) {
        Call::Exit => {
            assert!(
                thread::retire_from_trap(frame),
                "a ring 3 thread called in and could not be retired"
            );
        }
        Call::RingEnter => {
            let served = serve_ring(slot);
            answer_call(frame, served);
        }
        Call::RingWait => {
            let completed = wait_ring(slot, frame);
            answer_call(frame, completed);
        }
        Call::ThreadBase => {
            // Refused, not faulted: `wrmsr` of a non-canonical base is a `#GP` in kernel context.
            let answer = if task::set_thread_base(frame.rdi()) {
                0
            } else {
                EINVAL
            };
            answer_call(frame, answer);
        }
        Call::Unknown(number) => {
            // Answered, so a program can tell a wrong call from a dropped one.
            answer_call(frame, ENOSYS);
            // Counted, never logged: trap context cannot take the console lock.
            REFUSED_CALLS.fetch_add(1, Ordering::Relaxed);
            LAST_REFUSED.store(number as usize, Ordering::Relaxed);
        }
    }
}

fn serve_ring(slot: usize) -> u64 {
    // Cloned out before serving: `serve` takes locks that `lock_leaf` forbids under `BOOTSTRAPS`.
    let Some((ring, table)) = authority_of(slot) else {
        return ENOSYS;
    };

    // Ring 3's claim of what it reaped, read once and bounded inside.
    ring.reap_up_to(ring.completion_head());
    let before = ring.served_count();
    // Due work completes before admission, so it never costs a new submission its slot.
    ring.expire(crate::time::now_ns());
    ring.serve(&table);
    ring.served_since(before) as u64
}

fn authority_of(
    slot: usize,
) -> Option<(
    alloc::sync::Arc<queue::Queue>,
    alloc::sync::Arc<HandleTable>,
)> {
    let guard = BOOTSTRAPS.lock_leaf();
    let bootstrap = guard.get(slot).and_then(Option::as_ref)?;
    Some((
        alloc::sync::Arc::clone(&bootstrap.ring),
        alloc::sync::Arc::clone(&bootstrap.table),
    ))
}

fn wait_ring(slot: usize, frame: &TrapFrame) -> u64 {
    let Some((ring, _)) = authority_of(slot) else {
        return ENOSYS;
    };

    ring.wait_from_trap(frame) as u64
}

// The switch that scheduled this thread already set `RSP0` and installed its tables.
pub(super) fn cross(_: &mut thread::ThreadContext, _: usize) {
    let slot = task::current();
    let Some(launch) = LAUNCHES.lock_leaf().get(slot).copied().flatten() else {
        warn!("user: thread {slot} has no program to run");
        refuse_cross(slot);
    };
    let which = launch.which;

    // Checked, not assumed: a missed switch would put one program's argument on another's stack.
    if cpu::page_table_base() != launch.root {
        warn!("user {which}: running on tables that are not its own");
        refuse_cross(slot);
    }

    let top = launch.stack + STACK_BYTES - ENTRY_STACK_BYTES;
    if usercopy::copy_to_user(top + entry::VERSION as u64, &ENTRY_VERSION.to_le_bytes()).is_err()
        || usercopy::copy_to_user(top + entry::ARGUMENT as u64, &launch.argument.to_le_bytes())
            .is_err()
        || usercopy::copy_to_user(
            top + entry::BOOTSTRAP_HANDLE as u64,
            &u64::from(launch.bootstrap_handle).to_le_bytes(),
        )
        .is_err()
    {
        warn!("user {which}: could not write its entry block");
        refuse_cross(slot);
    }

    info!(
        "user {which}: entering ring 3 at {:#x} with {} on {:#x}",
        launch.entry, launch.argument, launch.root
    );
    if !crate::interrupts::maskable_interrupts_are_ready() {
        warn!("user {which}: maskable interrupts are not admitted");
        refuse_cross(slot);
    }
    let refused = task::enter_user(launch.entry, top);
    warn!("user {which}: refused entry to ring 3: {refused}");
    refuse_cross(slot);
}

fn refuse_cross(slot: usize) -> ! {
    let _disabled = interrupt::disable();
    orphan(slot);
    thread::exit()
}
