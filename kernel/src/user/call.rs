use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use boot_record::{Fact, Record};
use native_abi::Call;
use native_abi::entry::{self, offset};
use native_abi::error::{EINVAL, ENOSYS};
use wost::arch::current::interrupt::UserRegisters;
use wost::arch::current::task;
use wost::arch::current::{cpu, usercopy};

use super::{BOOTSTRAPS, LAUNCHES, STACK_BYTES};
use crate::handle::HandleTable;
use crate::{info, queue, thread, warn};

const ENTRY_STACK_BYTES: u64 = (entry::BYTES as u64 + 15) & !15;

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
    info!("{}", Record::Fact(Fact::UserCall));
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

// The calling thread's own context: it may allocate, block and be preempted.
pub(crate) fn call(context: &mut thread::ThreadContext, registers: &mut UserRegisters<'_>) {
    let slot = task::current();
    EXECUTED.store(true, Ordering::Release);
    match Call::decode(registers.rax()) {
        Call::Exit => thread::retire(),
        Call::RingEnter => registers.set_return(serve_ring(slot)),
        Call::RingWait => registers.set_return(wait_ring(context, slot)),
        Call::ThreadBase => {
            // Refused, not faulted: `wrmsr` of a non-canonical base is a `#GP` in kernel context.
            let answer = if task::set_thread_base(registers.rdi()) {
                0
            } else {
                EINVAL
            };
            registers.set_return(answer);
        }
        Call::Unknown(number) => {
            // Answered, so a program can tell a wrong call from a dropped one.
            registers.set_return(ENOSYS);
            // Counted, and reported once by the idle thread, so a looping program cannot flood.
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

fn wait_ring(context: &mut thread::ThreadContext, slot: usize) -> u64 {
    let Some((ring, _)) = authority_of(slot) else {
        return ENOSYS;
    };

    ring.wait(context) as u64
}

// The switch that scheduled this thread already set `RSP0` and installed its tables.
pub(super) fn cross(context: &mut thread::ThreadContext, _: usize) {
    let slot = task::current();
    let Some(launch) = LAUNCHES.lock_leaf().get(slot).copied().flatten() else {
        warn!("user: thread {slot} has no program to run");
        thread::retire();
    };
    let which = launch.which;

    // Checked, not assumed: a missed switch would put one program's argument on another's stack.
    if cpu::page_table_base() != launch.root {
        warn!("user {which}: running on tables that are not its own");
        thread::retire();
    }

    let top = launch.stack + STACK_BYTES - ENTRY_STACK_BYTES;
    if usercopy::copy_to_user(top + offset::VERSION as u64, &entry::VERSION.to_le_bytes()).is_err()
        || usercopy::copy_to_user(
            top + offset::ARGUMENT as u64,
            &launch.argument.to_le_bytes(),
        )
        .is_err()
        || usercopy::copy_to_user(
            top + offset::BOOTSTRAP_HANDLE as u64,
            &u64::from(launch.bootstrap_handle).to_le_bytes(),
        )
        .is_err()
    {
        warn!("user {which}: could not write its entry block");
        thread::retire();
    }

    info!(
        "user {which}: entering ring 3 at {:#x} with {} on {:#x}",
        launch.entry, launch.argument, launch.root
    );
    if !crate::interrupts::maskable_interrupts_are_ready() {
        warn!("user {which}: maskable interrupts are not admitted");
        thread::retire();
    }
    let refused = task::enter_user(context, launch.entry, top, launch.argument);
    warn!("user {which}: refused entry to ring 3: {refused}");
    thread::retire();
}
