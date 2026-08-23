//! Kernel threads: the saved state of each, and the switch between them.
//!
//! A thread here is a slot — an index into a small fixed table — with a stack
//! from the arena [`map_stacks`](super::paging::AddressSpace::map_stacks)
//! committed and a saved stack pointer. Everything else a switch must preserve
//! lives *on* that stack: the callee-saved registers the switch pushes, and for
//! a thread that has never run, a frame [`fabricate`] wrote so that the first
//! switch into it "returns" into `thread_start`.
//!
//! The switch is a naked function (D-51), and it is exposed safely on the
//! same grounds as [`activate`](super::paging::activate): the state it loads
//! can only have come from this module. A saved pointer is written either by
//! a previous switch or by `fabricate`, on a stack the arena mapped and
//! [`init`] confirmed, into a slot that is not running — and a slot that has
//! exited is refused forever after, so nothing can resume into a stack that
//! has since been handed to someone else.
//!
//! Policy — which slot runs next, and why — is the kernel's. This module only
//! knows how to get there.

use core::arch::naked_asm;
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use x86_paging::Flags;

use super::interrupt::{self, TrapFrame};
use super::paging::{self, StackArena};
use crate::cell::HardwareCell;
use crate::panic::PanicToken;
use crate::sync::{Once, SpinLock};

/// Slots the table holds, the boot thread's among them.
///
/// A cap on the arena rather than a promise: [`init`] accepts any arena of
/// fewer stacks, and the kernel's own constant decides how many it maps.
pub const MAX_TASKS: usize = 32;

/// The slot the boot thread occupies — the flow of control that called
/// [`init`], still on the stack the bootloader gave it.
pub const BOOT: usize = 0;

/// What a fabricated thread begins by running. Never returns: a thread that
/// is finished calls [`exit_to`].
pub type Entry = extern "C" fn(usize) -> !;

/// Why a request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskError {
    /// [`init`] has not run, or ran twice.
    NotInitialised,
    /// The kernel's own tables are not active, so the arena is not mapped.
    Inactive,
    /// A stack the arena claims to hold is not mapped writable and
    /// non-executable where the table is looking.
    Unmapped(u64),
    /// The arena holds more stacks than the table has slots.
    TooMany(usize),
    /// No such slot.
    OutOfRange(usize),
    /// The slot has never been fabricated.
    Vacant(usize),
    /// The slot's thread has exited, and nothing may resume it.
    Dead(usize),
    /// The slot's thread is live; fabricating over it would discard a stack
    /// something may still be standing on.
    Occupied(usize),
    /// The slot is the one already running.
    Current(usize),
    /// The boot slot has no arena stack to fabricate a frame on.
    Boot,
    /// The interrupted code was not accepting interrupts, so it is inside a
    /// critical section that must not be switched out of.
    Uninterruptible,
    /// [`switch_from_trap`] was called outside a trap handler.
    NotInTrap,
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotInitialised => f.write_str("the task table is not initialised"),
            Self::Inactive => f.write_str("the kernel's page tables are not active"),
            Self::Unmapped(address) => write!(f, "stack at {address:#x} is not mapped as a stack"),
            Self::TooMany(stacks) => write!(f, "{stacks} stacks exceed {MAX_TASKS} task slots"),
            Self::OutOfRange(slot) => write!(f, "slot {slot} does not exist"),
            Self::Vacant(slot) => write!(f, "slot {slot} holds no thread"),
            Self::Dead(slot) => write!(f, "slot {slot} has exited"),
            Self::Occupied(slot) => write!(f, "slot {slot} is live"),
            Self::Current(slot) => write!(f, "slot {slot} is already running"),
            Self::Boot => f.write_str("the boot slot cannot be fabricated"),
            Self::Uninterruptible => f.write_str("the interrupted code holds a critical section"),
            Self::NotInTrap => f.write_str("not inside a trap handler"),
        }
    }
}

impl core::error::Error for TaskError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Vacant,
    Alive,
    Dead,
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    state: State,
    /// One past the stack's highest byte. Zero for the boot slot.
    stack_top: u64,
    entry: Option<(Entry, usize)>,
}

impl Slot {
    const VACANT: Self = Self {
        state: State::Vacant,
        stack_top: 0,
        entry: None,
    };
}

#[derive(Debug)]
struct Table {
    slots: [Slot; MAX_TASKS],
    /// Slots in use, the boot slot included.
    count: usize,
}

static TABLE: SpinLock<Table> = SpinLock::new(Table {
    slots: [Slot::VACANT; MAX_TASKS],
    count: 0,
});

/// Each slot's saved stack pointer.
///
/// Written by the switch itself — assembly storing `rsp` — and so reached
/// through a raw pointer rather than a lock the assembly could not take.
/// Every access happens with the interrupt flag clear and the slot's state
/// settled under [`TABLE`], which on one CPU is exclusivity.
static CONTEXTS: HardwareCell<[u64; MAX_TASKS]> = HardwareCell::new([0; MAX_TASKS]);

static CURRENT: AtomicUsize = AtomicUsize::new(BOOT);

/// Whether the context that last switched away was accepting interrupts.
///
/// A thread resuming after a switch restores its own flag from its own stack.
/// A thread starting for the first time has no such record, so it inherits the
/// switcher's — the same rule a lock guard follows: put back what was found.
static HANDOFF_INTERRUPTS: AtomicBool = AtomicBool::new(false);

static INIT: Once = Once::new();

/// Take the arena's stacks into the table, making the caller the boot thread.
///
/// Every stack is walked in the live tables first and must be writable and
/// non-executable — the arena is the evidence they were mapped, and the walk
/// is the evidence the mapping is the one the CPU is using. Returns how many
/// threads can be fabricated.
///
/// # Errors
///
/// [`TaskError::Inactive`] before the kernel's tables are active,
/// [`TaskError::Unmapped`] naming the first stack the walk could not find,
/// [`TaskError::TooMany`] for an arena larger than the table, and
/// [`TaskError::NotInitialised`] on a second call.
pub fn init(arena: StackArena) -> Result<usize, TaskError> {
    if !paging::is_active() {
        return Err(TaskError::Inactive);
    }
    let layout = arena.layout();
    if layout.slots() + 1 > MAX_TASKS {
        return Err(TaskError::TooMany(layout.slots()));
    }

    let mut table = TABLE.lock();
    if !INIT.claim() {
        return Err(TaskError::NotInitialised);
    }

    table.slots[BOOT].state = State::Alive;
    for index in 0..layout.slots() {
        let slot = layout.slot(index).ok_or(TaskError::OutOfRange(index))?;
        for probe in [slot.stack_base, slot.stack_top() - FRAME_BYTES] {
            match paging::translate(probe) {
                Some((_, flags))
                    if flags.contains(Flags::WRITABLE) && flags.contains(Flags::NO_EXECUTE) => {}
                _ => return Err(TaskError::Unmapped(probe)),
            }
        }
        table.slots[index + 1].stack_top = slot.stack_top();
    }
    table.count = layout.slots() + 1;
    Ok(layout.slots())
}

/// The slot whose stack this CPU is running on.
pub fn current() -> usize {
    CURRENT.load(Ordering::Acquire)
}

/// Slots in the table, the boot slot included; zero before [`init`].
pub fn count() -> usize {
    TABLE.lock().count
}

/// Give `slot` a fresh thread that will begin at `entry(arg)`.
///
/// Writes the frame the first switch into the slot will pop, so that the
/// switch's `ret` lands in `thread_start` with the stack aligned as the ABI
/// requires at a function's entry. A slot whose thread has exited may be
/// fabricated again: its stack is reused, and that is safe because the table
/// refuses to resume a dead slot, so nothing can be standing on it.
///
/// # Errors
///
/// [`TaskError::Occupied`] if the slot's thread is live, [`TaskError::Boot`]
/// for the boot slot, [`TaskError::OutOfRange`] past the table, and
/// [`TaskError::NotInitialised`] before [`init`].
pub fn fabricate(slot: usize, entry: Entry, arg: usize) -> Result<(), TaskError> {
    let mut table = TABLE.lock();
    if table.count == 0 {
        return Err(TaskError::NotInitialised);
    }
    if slot == BOOT {
        return Err(TaskError::Boot);
    }
    if slot >= table.count {
        return Err(TaskError::OutOfRange(slot));
    }
    if table.slots[slot].state == State::Alive {
        return Err(TaskError::Occupied(slot));
    }

    let top = table.slots[slot].stack_top;
    let base = top - FRAME_BYTES;
    let frame: [u64; FRAME_WORDS] = [0, 0, 0, 0, 0, 0, thread_start as *const () as u64];
    // SAFETY: `init` walked this stack's top page and found it mapped
    // writable; `base` is `FRAME_BYTES` below the top, inside that page, and
    // eight-byte aligned because the top is page-aligned. Nothing else is
    // using the stack: the slot is not `Alive`, and a dead slot's thread can
    // never be resumed. The lock is held, so no other fabrication races.
    unsafe { (base as usize as *mut [u64; FRAME_WORDS]).write(frame) };

    // SAFETY: `slot` is below `MAX_TASKS`, so the element exists.
    let word = unsafe { CONTEXTS.as_mut_ptr().cast::<u64>().add(slot) };
    // SAFETY: the lock is held, and the assembly that also writes this word
    // runs only with the flag clear, which the lock guarantees for the
    // duration of this write.
    unsafe { word.write(base) };

    table.slots[slot] = Slot {
        state: State::Alive,
        stack_top: top,
        entry: Some((entry, arg)),
    };
    Ok(())
}

/// Run `slot`, returning when something switches back to this one.
///
/// The interrupt flag is clear across the switch and restored on return, so a
/// caller sees exactly the flag it had — while the thread switched *to*
/// resumes with whatever its own last switch saved, or, on its first run,
/// inherits this caller's.
///
/// # Errors
///
/// [`TaskError::Current`] for the running slot, [`TaskError::Dead`] for an
/// exited one, [`TaskError::Vacant`] for an unfabricated one,
/// [`TaskError::OutOfRange`] past the table, and
/// [`TaskError::NotInitialised`] before [`init`].
pub fn switch_to(slot: usize) -> Result<(), TaskError> {
    let was_enabled = interrupt::are_enabled();
    let saved = interrupt::disable();
    let result = prepare(slot, State::Alive).map(|(save, load)| {
        HANDOFF_INTERRUPTS.store(was_enabled, Ordering::Relaxed);
        // SAFETY: `prepare` took `save` from this CPU's own slot and `load`
        // from a live slot's record, each written only by `fabricate` or by a
        // previous switch on a stack `init` confirmed. The flag is clear, so
        // nothing runs between the bookkeeping and the switch.
        unsafe { switch(save, load) };
    });
    saved.restore();
    result
}

/// Run `slot` from inside a trap handler, returning when something switches
/// back — at which point the caller is still inside its handler, and
/// returning from it resumes the code that was interrupted.
///
/// The handler's frame stays on the preempted thread's own stack; what this
/// saves is the handler's path down to here. Two things make it safe to do
/// from a handler at all. The interrupted code must have been accepting
/// interrupts — `frame` is the evidence, and code that was not (a lock
/// holder, D-34) is refused, so no critical section is ever switched out of.
/// And the trap depth goes with the thread: the one switched to runs at its
/// own depth, and this one gets its back on resumption (D-55).
///
/// The thread switched to inherits an *enabled* interrupt flag on its first
/// run, because the code this handler interrupted had one.
///
/// # Errors
///
/// [`TaskError::Uninterruptible`] if the interrupted code had the flag
/// clear, [`TaskError::NotInTrap`] outside a handler, and otherwise as
/// [`switch_to`].
pub fn switch_from_trap(slot: usize, frame: &TrapFrame) -> Result<(), TaskError> {
    if !interrupt::in_trap() {
        return Err(TaskError::NotInTrap);
    }
    if frame.flags & interrupt::FLAGS_INTERRUPT == 0 {
        return Err(TaskError::Uninterruptible);
    }

    let saved = interrupt::disable();
    let depth = interrupt::suspend_trap();
    let result = prepare(slot, State::Alive).map(|(save, load)| {
        HANDOFF_INTERRUPTS.store(true, Ordering::Relaxed);
        // SAFETY: as in `switch_to` — `prepare` vouches for both pointers and
        // the flag is clear. The handler's own frame is below the saved
        // pointer on this thread's stack, and this thread is the only thing
        // that will ever unwind it.
        unsafe { switch(save, load) };
    });
    interrupt::resume_trap(depth);
    saved.restore();
    result
}

/// Leave the running thread for good and run `slot`.
///
/// The caller's slot is marked dead before the switch, so no later
/// [`switch_to`] can land on its stack — which is what makes fabricating a
/// new thread over that stack safe. Returns only if the switch was refused;
/// on success, the calling context is never resumed.
///
/// # Errors
///
/// As [`switch_to`].
pub fn exit_to(slot: usize) -> TaskError {
    let was_enabled = interrupt::are_enabled();
    let saved = interrupt::disable();
    match prepare(slot, State::Dead) {
        Ok((save, load)) => {
            HANDOFF_INTERRUPTS.store(was_enabled, Ordering::Relaxed);
            // SAFETY: as in `switch_to`. The slot being left is now `Dead`,
            // which every later switch refuses, so the save is never loaded.
            unsafe { switch(save, load) };
            unreachable!("a thread marked dead was resumed");
        }
        Err(error) => {
            saved.restore();
            error
        }
    }
}

/// Check `slot` can be run, move the bookkeeping to it, and hand back what
/// the switch needs: where to save this context, and what to load.
///
/// The leaving slot becomes `leaving`. Called with the interrupt flag clear.
fn prepare(slot: usize, leaving: State) -> Result<(*mut u64, u64), TaskError> {
    let mut table = TABLE.lock();
    if table.count == 0 {
        return Err(TaskError::NotInitialised);
    }
    if slot >= table.count {
        return Err(TaskError::OutOfRange(slot));
    }
    let current = CURRENT.load(Ordering::Acquire);
    if slot == current {
        return Err(TaskError::Current(slot));
    }
    match table.slots[slot].state {
        State::Alive => {}
        State::Vacant => return Err(TaskError::Vacant(slot)),
        State::Dead => return Err(TaskError::Dead(slot)),
    }

    table.slots[current].state = leaving;
    CURRENT.store(slot, Ordering::Release);

    let contexts = CONTEXTS.as_mut_ptr().cast::<u64>();
    // SAFETY: both indices are below `MAX_TASKS`; the lock is held and the
    // flag is clear, so the assembly that also touches these words is not
    // running.
    let save = unsafe { contexts.add(current) };
    // SAFETY: as above.
    let target = unsafe { contexts.add(slot) };
    // SAFETY: as above, and the word was written by `fabricate` or a switch.
    let load = unsafe { target.read() };
    Ok((save, load))
}

/// Bytes the fabricated frame occupies below the stack top.
///
/// Six callee-saved registers and a return address: `ret` pops the last,
/// leaving the stack pointer eight below the top, which is the alignment the
/// ABI promises a function on entry.
const FRAME_BYTES: u64 = FRAME_WORDS as u64 * 8;

const FRAME_WORDS: usize = 7;

/// Where every fabricated thread begins: on its own stack, with nothing in its
/// registers, and the interrupt flag as the thread that switched here had it.
extern "C" fn thread_start() -> ! {
    let slot = current();
    let entry = TABLE.lock().slots[slot].entry;
    let Some((entry, arg)) = entry else {
        panic!("slot {slot} started with no entry point");
    };
    if HANDOFF_INTERRUPTS.load(Ordering::Relaxed) {
        interrupt::enable();
    }
    entry(arg)
}

/// Save the callee-saved registers and the stack pointer, then load another
/// stack and return into it.
///
/// Everything the ABI lets a call clobber is already clobbered by the call,
/// so only `rbp`, `rbx` and `r12`–`r15` cross. Floating-point state does not:
/// the kernel is built without SSE. Interrupt flag handling is the caller's,
/// and the flag must be clear — an interrupt between the store and the load
/// would run on a stack whose pointer two records now claim.
///
/// # Safety
///
/// `save` must point at a word this thread's saved pointer may be written to,
/// and `load` must be a pointer a previous call stored or [`fabricate`]
/// wrote, on a stack nothing else is using. The interrupt flag must be clear.
#[unsafe(naked)]
unsafe extern "C" fn switch(save: *mut u64, load: u64) {
    naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}

/// Reclaim the table's lock on the terminal panic path.
pub(crate) fn force_unlock(panic: &PanicToken) {
    TABLE.force_unlock(panic);
}
