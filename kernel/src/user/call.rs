use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::task;
use wost::arch::current::{cpu, usercopy};

use super::reap::orphan;
use super::{BOOTSTRAPS, LAUNCHES, USER_STACK_BYTES};
use crate::handle::HandleTable;
#[cfg(feature = "selftest")]
use crate::wait::WaitQueue;
use crate::{info, queue, thread, warn};

/// Version of the words placed at a program's initial stack pointer.
pub(crate) const ENTRY_VERSION: u64 = 1;

/// Bytes in the versioned entry block.
pub(crate) const ENTRY_BYTES: usize = 24;

/// Byte offsets in the versioned entry block.
mod entry {
    /// The entry ABI version.
    pub(super) const VERSION: usize = 0;
    /// The launcher's operation-specific argument.
    pub(super) const ARGUMENT: usize = 8;
    /// The numeric identity of the sole bootstrap handle.
    pub(super) const BOOTSTRAP_HANDLE: usize = 16;
}

/// Stack tail occupied so the initial pointer remains sixteen-byte aligned.
const ENTRY_STACK_BYTES: u64 = (ENTRY_BYTES as u64 + 15) & !15;

/// A program's ring and authority table, for lifecycle checks.
#[cfg(feature = "selftest")]
pub(crate) fn authority_for_test(
    slot: usize,
) -> Option<(
    alloc::sync::Arc<queue::Queue>,
    alloc::sync::Arc<HandleTable>,
)> {
    authority_of(slot)
}

/// What a program asked for, in `rax`.
///
/// The synchronous plane (`002` pillar 2): retirement, ring service and wait,
/// and thread-local setup. The *data* path is still the ring: Enter and Wait
/// serve work already posted there rather than transferring data in the trap
/// (D-116, D-164).
///
/// Four, and the surface stays small on purpose. io_uring's exploit record is
/// a warning about surface growth, and every call added here is one a hostile
/// program can reach.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Call {
    /// End this thread. The only one that does not return.
    Exit,
    /// Serve what has been submitted, and report how many completed.
    RingEnter,
    /// Wait until an operation in flight comes due, then serve it.
    ///
    /// The first call in this ABI whose honest answer is *not yet*. It gives
    /// the CPU up rather than spinning, which is what makes a deferred
    /// completion worth having (D-164).
    RingWait,
    /// Point this thread's local storage at `rdi`.
    ///
    /// mlibc's `sys_tcb_set`. Its body is WIRED and the platform route WORKS:
    /// writing the sysdeps found that `FS_BASE` is a fourth thing that must
    /// follow a thread, and a call that sets it is worth nothing until it does
    /// (D-114, D-118, D-181).
    ThreadBase,
    /// Anything else.
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

/// Calls refused because this kernel does not serve them.
static REFUSED_CALLS: AtomicUsize = AtomicUsize::new(0);

/// The last such number, so a report can say *what* was asked for.
static LAST_REFUSED: AtomicUsize = AtomicUsize::new(0);

/// Call answers that could not be written back.
///
/// Should stay at zero: the frame a handler is running on is the innermost
/// live one by construction.
static UNANSWERED: AtomicUsize = AtomicUsize::new(0);

/// RingEnter calls whose completion wake armed the timer in that trap.
#[cfg(feature = "selftest")]
static RING_ENTER_ARMS: AtomicUsize = AtomicUsize::new(0);

/// Whether the one deferred refusal warning has already been emitted.
static REPORTED: AtomicUsize = AtomicUsize::new(0);

static EXECUTED: AtomicBool = AtomicBool::new(false);
static EXECUTION_REPORTED: AtomicBool = AtomicBool::new(false);

/// Report one observed user call from ordinary thread context (D-427).
pub(crate) fn report_execution() -> bool {
    if !EXECUTED.load(Ordering::Acquire) || EXECUTION_REPORTED.swap(true, Ordering::AcqRel) {
        return false;
    }
    info!("user: ring 3 call observed");
    true
}

/// Say what the syscall handler could not, if there is anything new.
///
/// Called from the idle thread because a kernel thread may take the console's
/// lock and the handler may not (D-122). Reports at most once per boot and
/// then falls quiet, so a program in a loop cannot turn refusals into a flood.
///
/// `true` if something was said, so the caller knows it did work.
pub(crate) fn report_refusals() -> bool {
    let refused = REFUSED_CALLS.load(Ordering::Acquire);
    if refused == 0 || REPORTED.swap(1, Ordering::AcqRel) != 0 {
        return false;
    }
    let number = LAST_REFUSED.load(Ordering::Acquire);
    warn!("user: refused {refused} call(s), last was {number:#x}");
    true
}

/// Calls this kernel refused, and the last number asked for.
///
/// Read from outside trap context, which is the whole point: the handler
/// cannot take the console's lock, so it leaves a number for something that
/// can (D-122).
#[cfg(feature = "selftest")]
pub(crate) fn refusals() -> (usize, usize, usize) {
    (
        REFUSED_CALLS.load(Ordering::Acquire),
        LAST_REFUSED.load(Ordering::Acquire),
        UNANSWERED.load(Ordering::Acquire),
    )
}

/// Raw one-shot refusal-warning latch for the machine witness.
#[cfg(feature = "selftest")]
pub(crate) fn refusal_report_latch_for_test() -> usize {
    REPORTED.load(Ordering::Acquire)
}

/// RingEnter completion wakes observed to arm before the handler returned.
#[cfg(feature = "selftest")]
pub(crate) fn ring_enter_arms_for_test() -> usize {
    RING_ENTER_ARMS.load(Ordering::Acquire)
}

/// `-ENOSYS`, as `abis/linux` numbers it and as the ABI header states.
const ENOSYS: u64 = (-38i64) as u64;

/// `-EINVAL`, likewise.
const EINVAL: u64 = (-22i64) as u64;

fn answer_call(frame: &TrapFrame, value: u64) {
    let answered = interrupt::answer(frame, value);
    if !answered {
        UNANSWERED.fetch_add(1, Ordering::Relaxed);
    }
    assert!(answered, "the live syscall frame could not be answered");
}

/// Serve one call from ring 3.
///
/// Runs in trap context, so ordinary work here may not allocate, enter a
/// `ThreadContext` blocking path, or take a lock a handler cannot (D-55, D-63).
/// D-200's `wait_from_trap` is the narrow audited exception: it yields through
/// the saved trap frame. `Queue::serve` takes leaf locks only around bounded
/// state transitions and defers file service to the worker.
///
/// **Only `Exit` does not return.** Other calls preserve the caller so it can
/// act on the result (D-116).
pub(crate) fn call(frame: &TrapFrame) {
    let slot = task::current();
    if frame.code_selector() & 0b11 == 3 {
        EXECUTED.store(true, Ordering::Release);
    }
    match Call::decode(frame.rax()) {
        Call::Exit => {
            if !thread::retire_from_trap(frame) {
                panic!("a ring 3 thread called in and could not be retired");
            }
        }
        Call::RingEnter => {
            #[cfg(feature = "selftest")]
            let arms = thread::handler_arms();
            let served = serve_ring(slot);
            #[cfg(feature = "selftest")]
            if served > 0 && thread::handler_arms() > arms {
                RING_ENTER_ARMS.fetch_add(1, Ordering::Release);
            }
            answer_call(frame, served);
        }
        Call::RingWait => {
            let completed = wait_ring(slot, frame);
            answer_call(frame, completed);
        }
        Call::ThreadBase => {
            // Refused, not faulted: `wrmsr` of a non-canonical base is a `#GP`
            // in kernel context, a program crashing the handler.
            let answer = if task::set_thread_base(frame.rdi()) {
                0
            } else {
                EINVAL
            };
            answer_call(frame, answer);
        }
        Call::Unknown(number) => {
            // Refused with an answer: a program that gets nothing back cannot
            // tell a wrong call from a dropped one.
            answer_call(frame, ENOSYS);
            // Counted, not logged: trap context cannot take the console lock,
            // and once did (D-63, D-122); the idle thread reports these.
            REFUSED_CALLS.fetch_add(1, Ordering::Relaxed);
            LAST_REFUSED.store(number as usize, Ordering::Relaxed);
        }
    }
}

/// Serve the calling thread's own ring, and count the completions.
///
/// The ring is reached through the thread's `Bootstrap`, which is where its
/// authority lives — a program cannot name another's, because there is no
/// argument here that could say which.
fn serve_ring(slot: usize) -> u64 {
    // Cloned out before anything is served: `serve` takes four more locks,
    // which `lock_leaf` forbids under this guard (D-121, D-124).
    let Some((ring, table)) = authority_of(slot) else {
        return ENOSYS;
    };

    // The program's own claim about what it has reaped, taken once and bounded
    // inside (D-113, D-115).
    let head = ring.completion_head();
    ring.reap_up_to(head);
    // Due operations complete before new ones are admitted, so finished work
    // never costs the next submission a slot the kernel already owes.
    ring.expire(crate::time::now_ns());
    ring.serve(&table) as u64
}

/// The ring and the handle table of the thread in `slot`.
///
/// Both are cloned out and the guard dropped before either is used. `serve`
/// reads the program's memory, performs each operation and wakes whatever was
/// waiting, and `install` takes the handle arena's leaf — doing any of that
/// under this table's guard is what `lock_leaf` forbids (D-121, D-124).
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

/// Wait for an operation in flight, then serve what came due.
///
/// Three answers, and they are different things. **Nothing in flight** returns
/// zero at once — there is nothing to wait for, and blocking would be a hang.
/// **Something already due** is completed without sleeping. Otherwise the
/// thread sleeps until the soonest deadline and completes what it finds when
/// it wakes, which may be later than the deadline and never earlier.
///
/// The completion is posted by the timer path or file worker, not by the
/// program asking again: a woken program finds the entry already in its queue.
/// That is what makes this a wait rather than a poll (D-164, D-200).
fn wait_ring(slot: usize, frame: &TrapFrame) -> u64 {
    let Some((ring, _)) = authority_of(slot) else {
        return ENOSYS;
    };

    ring.wait_from_trap(frame) as u64

    // May return having done nothing (no peer, or no clock); the answer is
    // still the truth about what is due now.
}

/// Cross into ring 3, and never come back on this thread.
///
/// Runs as an ordinary kernel thread until its last instruction, and is retired
/// by the syscall handler from the far side of the boundary.
///
/// No stack is arranged for the trap out: the context switch that scheduled
/// this thread already pointed `RSP0` at its own kernel stack, and will point
/// it somewhere else the moment another thread runs (D-99). The same switch
/// installed this thread's tables, which is what makes the copy below reach
/// *its* stack and no other program's (D-106).
pub(super) fn cross() {
    let slot = task::current();
    let Some(launch) = LAUNCHES.lock_leaf().get(slot).copied().flatten() else {
        warn!("user: thread {slot} has no program to run");
        refuse_cross(slot);
    };
    let which = launch.which;

    #[cfg(feature = "selftest")]
    if GATE_NEXT_CROSS.swap(false, Ordering::AcqRel) {
        let mut context = thread::ThreadContext::require();
        CROSS_GATE.wait_unless(&mut context, || CROSS_MAY_ENTER.load(Ordering::Acquire));
    }

    // Checked, not assumed: a missed switch would put one program's argument
    // on another's stack.
    if cpu::page_table_base() != launch.root {
        warn!("user {which}: running on tables that are not its own");
        refuse_cross(slot);
    }

    // Version, argument and bootstrap handle, written with the checked copy
    // (D-97, D-182); the tail keeps the initial pointer aligned.
    let top = launch.stack + USER_STACK_BYTES - ENTRY_STACK_BYTES;
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
    if !crate::maskable_interrupts_are_ready() {
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

/// The next child reaching [`cross`] must wait for the machine check.
#[cfg(feature = "selftest")]
pub(super) static GATE_NEXT_CROSS: AtomicBool = AtomicBool::new(false);

/// The condition paired with [`CROSS_GATE`].
#[cfg(feature = "selftest")]
pub(super) static CROSS_MAY_ENTER: AtomicBool = AtomicBool::new(true);

/// Stops one real child before its first user instruction.
#[cfg(feature = "selftest")]
pub(super) static CROSS_GATE: WaitQueue = WaitQueue::new();
