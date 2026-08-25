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
use core::sync::atomic::{AtomicUsize, Ordering};

use super::cpu;
use super::interrupt::{self, TrapFrame};
use super::msr;
use super::paging::{self, Binding, MappedStack, StackArena};
use super::xstate;
use crate::cell::HardwareCell;
use crate::panic::PanicToken;
use crate::sync::{Once, SpinLock};
use x86_paging::Flags;

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
    /// The frame handed to [`switch_from_trap`] is not the one the live
    /// handler was entered with, so nothing it claims about the interrupted
    /// code is evidence.
    ForeignFrame,
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
            Self::ForeignFrame => f.write_str("not the frame the live handler was entered with"),
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
    /// Whether the thread should be accepting interrupts on its first run.
    starts_interruptible: bool,
    /// The tables to install when this thread runs, or zero to keep the
    /// kernel's. Written only through [`bind_address_space`], which takes the
    /// evidence that the space agrees with the kernel about the higher half.
    page_table: u64,
    /// Where this thread's thread-local storage begins.
    ///
    /// The fourth thing that follows a thread, after the privilege stack
    /// (D-99), the tables (D-106) and the extended state (D-108). Nothing
    /// native needed it — it was writing an mlibc sysdep that asked, because
    /// nothing native has thread-local storage (D-114, D-118).
    thread_base: u64,
}

impl Slot {
    const VACANT: Self = Self {
        state: State::Vacant,
        stack_top: 0,
        entry: None,
        starts_interruptible: false,
        page_table: 0,
        thread_base: 0,
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

/// One extended-state save area per slot, out of line and aligned.
///
/// In `.bss` rather than on the heap because the switch must not allocate and
/// must not fail: a thread cannot be told "no room to save your registers".
/// Out of line rather than inside the thread struct, which is what D-82 asked
/// for — the *size* is `CPUID`'s and is checked against this ceiling at boot,
/// so the array is room and not layout (D-108).
#[repr(C, align(64))]
struct SaveAreas([[u8; xstate::CEILING]; MAX_TASKS]);

static EXTENDED: HardwareCell<SaveAreas> =
    HardwareCell::new(SaveAreas([[0; xstate::CEILING]; MAX_TASKS]));

static CURRENT: AtomicUsize = AtomicUsize::new(BOOT);

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
    if let Some(address) = arena.invalid_page() {
        return Err(TaskError::Unmapped(address));
    }
    let layout = arena.layout();
    if layout.slots() + 1 > MAX_TASKS {
        return Err(TaskError::TooMany(layout.slots()));
    }

    let mut table = TABLE.lock_leaf();
    if !INIT.claim() {
        return Err(TaskError::NotInitialised);
    }

    table.slots[BOOT].state = State::Alive;
    for index in 0..layout.slots() {
        let slot = layout.slot(index).ok_or(TaskError::OutOfRange(index))?;
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
    TABLE.lock_leaf().count
}

/// Give `slot` a fresh thread that will begin at `entry(arg)`.
///
/// Writes the frame the first switch into the slot will pop, so that the
/// switch's `ret` lands in `thread_start` with the stack aligned as the ABI
/// requires at a function's entry. A slot whose thread has exited may be
/// fabricated again: its stack is reused, and that is safe because the table
/// refuses to resume a dead slot, so nothing can be standing on it.
///
/// The interrupt flag the thread will start with is captured here, from this
/// caller — the thread's creator. It cannot be captured at the switch: a
/// scheduler clears the flag around every decision it makes, so by the time
/// the switch runs, the only flag left to read is the scheduler's own, and a
/// thread that started from it would be uninterruptible for life. A new
/// thread holds no lock and has nothing to protect, so what it inherits is
/// whether the machine was accepting interrupts at all when it was made.
///
/// # Errors
///
/// [`TaskError::Occupied`] if the slot's thread is live, [`TaskError::Boot`]
/// for the boot slot, [`TaskError::OutOfRange`] past the table, and
/// [`TaskError::NotInitialised`] before [`init`].
pub fn fabricate(slot: usize, entry: Entry, arg: usize) -> Result<(), TaskError> {
    // Before the lock, which clears the flag: what is wanted is the caller's
    // own, not the one taking this lock leaves behind.
    let starts_interruptible = interrupt::are_enabled();
    let mut table = TABLE.lock_leaf();
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
    let frame: [u64; FRAME_WORDS] = [0, 0, 0, 0, 0, 0, thread_start as *const () as u64, 0];
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
        starts_interruptible,
        // Kernel tables until something binds a space to it. A thread that
        // never enters ring 3 never needs its own.
        page_table: 0,
        // And no thread-local storage until it asks for some.
        thread_base: 0,
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
    let saved = interrupt::disable();
    let result = prepare(slot, State::Alive).map(|(save, load)| {
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
/// interrupts — `frame` is the evidence, accepted only if it is the frame the
/// live handler was entered with, and code that was not accepting (a lock
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
/// clear, [`TaskError::NotInTrap`] outside a handler,
/// [`TaskError::ForeignFrame`] for a frame the live handler was not entered
/// with, and otherwise as [`switch_to`].
pub fn switch_from_trap(slot: usize, frame: &TrapFrame) -> Result<(), TaskError> {
    if !interrupt::in_trap() {
        return Err(TaskError::NotInTrap);
    }
    if !interrupt::is_live_frame(frame) {
        return Err(TaskError::ForeignFrame);
    }
    if frame.flags() & interrupt::FLAGS_INTERRUPT == 0 {
        return Err(TaskError::Uninterruptible);
    }

    let saved = interrupt::disable();
    let suspended = interrupt::suspend_trap();
    let result = prepare(slot, State::Alive).map(|(save, load)| {
        // SAFETY: as in `switch_to` — `prepare` vouches for both pointers and
        // the flag is clear. The handler's own frame is below the saved
        // pointer on this thread's stack, and this thread is the only thing
        // that will ever unwind it.
        unsafe { switch(save, load) };
    });
    interrupt::resume_trap(suspended);
    saved.restore();
    result
}

/// Leave the running thread for good, from trap context, and run `slot`.
///
/// [`switch_from_trap`] with the leaving slot marked **dead** rather than
/// alive: the thread is not coming back, so its stack is free for a later
/// [`fabricate`] and its slot for a later spawn. Without it the kernel's own
/// table frees the slot while this one keeps it `Alive`, and every later spawn
/// that picks that index is refused for ever (D-117).
///
/// The frame is still the innermost live one and the interrupted context is
/// still on this stack — nothing unwinds it, because nothing resumes here.
///
/// # Errors
///
/// As [`switch_from_trap`].
pub fn retire_to(slot: usize, frame: &TrapFrame) -> Result<(), TaskError> {
    if !interrupt::in_trap() {
        return Err(TaskError::NotInTrap);
    }
    if !interrupt::is_live_frame(frame) {
        return Err(TaskError::ForeignFrame);
    }
    if frame.flags() & interrupt::FLAGS_INTERRUPT == 0 {
        return Err(TaskError::Uninterruptible);
    }

    let saved = interrupt::disable();
    let suspended = interrupt::suspend_trap();
    let result = prepare(slot, State::Dead).map(|(save, load)| {
        // SAFETY: as in `switch_from_trap`. The slot being left is now `Dead`,
        // which every later switch refuses, so the state saved here is never
        // loaded — which is what makes its stack reusable.
        unsafe { switch(save, load) };
    });
    interrupt::resume_trap(suspended);
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
    let saved = interrupt::disable();
    match prepare(slot, State::Dead) {
        Ok((save, load)) => {
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
///
/// # Panics
///
/// Panics if a spin lock is held. A switch would leave it locked on a
/// context nothing can schedule, and the next taker spins with the flag
/// clear (D-66).
fn prepare(slot: usize, leaving: State) -> Result<(*mut u64, u64), TaskError> {
    assert!(
        crate::sync::spinlocks_held() == 0,
        "a spin lock is held, so the thread left behind could never release it"
    );
    let mut table = TABLE.lock_leaf();
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

    // The stack a trap from ring 3 will land on, moved with the thread. Set
    // here rather than by the kernel because this is the one place every
    // switch passes through, and a thread that entered ring 3 while `RSP0`
    // still named someone else's stack would take its next trap on a stack
    // another thread is standing on (D-99).
    //
    // The boot thread's slot has no arena stack, and its zero would name
    // nothing — it is also the one thread that cannot be in ring 3, since it
    // is the one that put every other thread there.
    let landing = table.slots[slot].stack_top;
    if landing != 0 {
        super::segment::write_kernel_stack(landing);
    }

    // The tables move with the thread, for the same reason `RSP0` does: this
    // is the one place every switch passes through. A slot with no space of
    // its own runs on the kernel's, which is where the boot thread and every
    // kernel thread live (D-106).
    let wanted = if table.slots[slot].page_table == 0 {
        paging::active_root()
    } else {
        table.slots[slot].page_table
    };
    if wanted != 0 && wanted != cpu::page_table_base() {
        // SAFETY: `bind_address_space` compared every top-level entry of the
        // higher half against the kernel's before this root was stored, so
        // the code about to execute the switch, the stack it is on, the trap
        // tables and the direct map are all mapped identically across it. The
        // interrupt flag is clear and the lock is held, so nothing runs
        // between the write and the return.
        unsafe { cpu::set_page_table_base(wanted) };
    }

    // Eager, and here because this is the one place every switch passes
    // through. Between these two instructions and the switch itself the kernel
    // executes only integer code — `+soft-float` is what guarantees that, and
    // it is the reason this can sit before the switch rather than inside it
    // (D-82, D-108).
    if xstate::is_enabled() {
        let areas = EXTENDED.as_mut_ptr().cast::<u8>();
        // SAFETY: both indices are below `MAX_TASKS`, so each offset names a
        // whole `CEILING`-byte area inside the array; the array is 64-byte
        // aligned and `CEILING` is a multiple of 64, so every area is too.
        let leaving = unsafe { areas.add(current * xstate::CEILING) };
        // SAFETY: as above.
        let arriving = unsafe { areas.add(slot * xstate::CEILING) };
        // SAFETY: aligned and `CEILING` long, which is at least `bytes()` —
        // `init` refused otherwise. The lock is held and the flag is clear, so
        // nothing else touches these bytes.
        unsafe { xstate::save(leaving) };
        // SAFETY: as above, and the area holds either zeros — which `XRSTOR`
        // reads as the initial state — or what a `save` wrote.
        unsafe { xstate::restore(arriving) };
    }

    // The fourth thing that moves with a thread, and the cheapest: two MSR
    // accesses, on the path that already writes `RSP0` and `CR3`. Saved from
    // the register rather than from a remembered value, because ring 3 can
    // change it — a program that set its own would otherwise have the kernel
    // restore the one it was given (D-118).
    table.slots[current].thread_base = read_thread_base();
    let arriving = table.slots[slot].thread_base;
    if arriving != read_thread_base() {
        // SAFETY: `FS_BASE` names no memory the kernel dereferences — `fs:` is
        // never used by compiled code here — so any value is a value ring 3
        // could have set itself, and a non-canonical one faults on the write
        // rather than corrupting anything. The lock is held and the flag is
        // clear, so nothing runs between this and the switch.
        unsafe { write_thread_base(arriving) };
    }

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

/// Install `binding`'s tables whenever `slot` runs.
///
/// Takes the evidence rather than a bare address: a `CR3` naming tables that
/// do not map the kernel is a triple fault at the next instruction, and
/// [`paging::AddressSpace::bind`](super::paging::AddressSpace::bind) is where
/// that is checked. The binding is consumed, so one space serves one slot.
///
/// # Errors
///
/// [`TaskError::OutOfRange`] past the table, [`TaskError::Vacant`] for a slot
/// with no thread, and [`TaskError::Current`] for the running one — changing
/// the tables under a thread that is already on them is a job for the switch,
/// not for this.
pub fn bind_address_space(slot: usize, binding: Binding) -> Result<(), TaskError> {
    let saved = interrupt::disable();
    let mut table = TABLE.lock_leaf();
    let result = if slot >= table.count {
        Err(TaskError::OutOfRange(slot))
    } else if table.slots[slot].state != State::Alive {
        Err(TaskError::Vacant(slot))
    } else if slot == CURRENT.load(Ordering::Acquire) {
        Err(TaskError::Current(slot))
    } else {
        table.slots[slot].page_table = binding.root();
        Ok(())
    };
    drop(table);
    saved.restore();
    result
}

/// Give a fabricated slot back before anything has run on it.
///
/// For a caller whose own setup failed between [`fabricate`] and making the
/// thread runnable. Without it that slot stays `Alive` here while the kernel's
/// table marks it free, and every later spawn that picks it is refused — a
/// permanent loss rather than a leak that ends (D-115).
///
/// # Errors
///
/// [`TaskError::OutOfRange`] past the table, [`TaskError::Current`] for the
/// running slot — a thread cannot retire itself this way — and
/// [`TaskError::Vacant`] for one nothing fabricated.
pub fn retire(slot: usize) -> Result<(), TaskError> {
    let saved = interrupt::disable();
    let mut table = TABLE.lock_leaf();
    let result = if slot >= table.count {
        Err(TaskError::OutOfRange(slot))
    } else if slot == CURRENT.load(Ordering::Acquire) {
        Err(TaskError::Current(slot))
    } else if table.slots[slot].state != State::Alive {
        Err(TaskError::Vacant(slot))
    } else {
        table.slots[slot] = Slot::VACANT;
        Ok(())
    };
    drop(table);
    saved.restore();
    result
}

/// Which tables `slot` runs on, or `None` for the kernel's.
///
/// The [`Root`] is evidence, not a number: it says these tables were installed
/// by a [`Binding`], which means their higher half was compared against the
/// kernel's. Handing out a bare `Paddr` let a safe caller walk any physical
/// address it liked (D-115).
pub fn address_space_of(slot: usize) -> Option<Root> {
    let saved = interrupt::disable();
    let root = TABLE.lock_leaf().slots.get(slot).map_or(0, |slot| {
        // A dead slot's tables are nobody's. The question is which tables this
        // *thread* runs on, and a thread that has exited runs on none — so
        // reporting the last root it had is an answer about the past (D-123).
        if slot.state == State::Alive {
            slot.page_table
        } else {
            0
        }
    });
    saved.restore();
    (root != 0).then_some(Root {
        slot,
        address: root,
    })
}

/// Evidence that no live slot will be switched onto `root`.
///
/// Minted only by [`release_address_space`], which looks. A space is safe to
/// reclaim when nothing names it, and "nothing names it" is a property of the
/// *task table* — `AddressSpace` cannot see that, so it takes the answer
/// rather than guessing at it with a `CR3` comparison that is true of one
/// instant (D-123).
#[derive(Debug)]
pub struct Released {
    root: u64,
}

impl Released {
    /// The root nothing names.
    pub const fn root(&self) -> u64 {
        self.root
    }
}

/// Give up `root`, if no live slot runs on it.
///
/// Clears the binding from every slot naming it — which are all dead by then —
/// and hands back the evidence [`AddressSpace::reclaim`](super::paging::AddressSpace::reclaim) needs.
///
/// `None` if any **live** slot still names it, because that thread will be
/// switched onto those tables and freeing them is a fault deferred to whenever
/// it is next scheduled.
pub fn release_address_space(root: u64) -> Option<Released> {
    if root == 0 {
        return None;
    }
    let saved = interrupt::disable();
    let mut table = TABLE.lock_leaf();
    let live = table
        .slots
        .iter()
        .any(|slot| slot.state == State::Alive && slot.page_table == root);
    if !live {
        for slot in &mut table.slots {
            if slot.page_table == root {
                slot.page_table = 0;
            }
        }
    }
    drop(table);
    saved.restore();
    (!live).then_some(Released { root })
}

/// A page-table root this kernel installed.
///
/// Mintable only by [`address_space_of`], so a caller cannot invent one. The
/// walk it authorises reads through the direct map, and a root that was never
/// a table would be an arbitrary physical read — which is what D-77 named as
/// an authority debt on the mapping side and what this closes on the reading
/// side (D-115).
///
/// **Unforgeable is not the same as fresh.** The value records which slot it
/// came from, and [`is_live`](Self::is_live) asks the task table again. Without
/// that a caller could hold one across the thread's exit: `collect` reclaims
/// the space, the root frame goes back to the allocator, and a later
/// `map_user` hands it to a *program* — so the walk reads ring-3-authored
/// bytes as top-level entries. That is the arbitrary physical read this type
/// exists to close, reopened by staleness rather than by forgery.
///
/// Every sibling evidence type here is deliberately not `Copy`
/// — `Binding`, `MappedStack`, `StackArena` — because one caller owning the
/// evidence is half of what makes it evidence. `Root` is `Copy` and keeps
/// being: copying it is harmless once the *reading* is guarded, and the walk
/// is the only thing it authorises (D-140).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Root {
    slot: usize,
    address: u64,
}

impl Root {
    /// The physical address of the top-level table.
    pub const fn address(self) -> u64 {
        self.address
    }

    /// Whether the thread this came from still runs on these tables.
    ///
    /// False once the thread has exited, which is when its root stops being
    /// its root and starts being whatever the allocator did with the frame.
    pub fn is_live(self) -> bool {
        address_space_of(self.slot) == Some(self)
    }
}

/// `IA32_FS_BASE`, where a thread's local storage begins.
const FS_BASE: u32 = 0xC000_0100;

/// Where the running thread's local storage begins.
pub fn read_thread_base() -> u64 {
    msr::read(FS_BASE)
}

/// Point the running thread's local storage at `base`.
///
/// # Safety
///
/// `base` must be canonical: `wrmsr` to `FS_BASE` with a non-canonical value
/// raises a general protection fault. Nothing else is required — this kernel's
/// compiled code never uses `fs:`, so the register names no memory anything
/// here dereferences, and a program that sets its own is only ever pointing at
/// its own memory.
pub unsafe fn write_thread_base(base: u64) {
    // SAFETY: the caller's.
    unsafe { msr::write(FS_BASE, base) };
}

/// Point the running thread's local storage at `base`, if it may.
///
/// The safe door, and the one a syscall reaches: a non-canonical base is
/// refused rather than faulting, and a base in the higher half is refused
/// because a program asking the kernel to point `fs:` at kernel memory is a
/// program asking for something it cannot be given a reason for (D-118).
///
/// `false` if the base was refused, having written nothing.
pub fn set_thread_base(base: u64) -> bool {
    if !crate::arch::current::paging::is_user_address(base) {
        return false;
    }
    // SAFETY: the check above establishes canonicality, which is `wrmsr`'s
    // only precondition here.
    unsafe { write_thread_base(base) };
    true
}

/// Bytes the fabricated frame occupies below the stack top.
///
/// Six callee-saved registers, the return address, and one padding word: the
/// pops and the `ret` consume seven, leaving `rsp` eight below the
/// page-aligned top — the entry alignment the ABI promises.
const FRAME_BYTES: u64 = FRAME_WORDS as u64 * 8;

const FRAME_WORDS: usize = 8;

/// Where every fabricated thread begins: on its own stack, with nothing in its
/// registers, and the interrupt flag its creator had.
///
/// The flag is set here rather than left to the switch because the switch
/// arrives with it clear by construction — see [`fabricate`].
extern "C" fn thread_start() -> ! {
    let slot = current();
    let (entry, interruptible) = {
        let table = TABLE.lock_leaf();
        (
            table.slots[slot].entry,
            table.slots[slot].starts_interruptible,
        )
    };
    let Some((entry, arg)) = entry else {
        panic!("slot {slot} started with no entry point");
    };
    if interruptible {
        interrupt::enable();
    }
    entry(arg)
}

/// Why the CPU would not be sent to ring 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UserError {
    /// The kernel's own tables are not installed.
    Inactive,
    /// Nothing is mapped at the entry point, or it is not user-executable.
    EntryNotExecutable(u64),
    /// The stack is not mapped, or not user-writable, or executable.
    StackNotWritable(u64),
    /// The stack pointer is not sixteen-byte aligned, which the ABI requires.
    StackMisaligned(u64),
    /// No ring-0 stack is set, so a trap out of ring 3 would have nowhere to
    /// land.
    NoKernelStack,
    /// `syscall` or `sysenter` is still enabled — a second door into ring 0
    /// that this kernel does not use and did not choose (D-139).
    FastSyscallOpen,
}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Inactive => f.write_str("the kernel's tables are not installed"),
            Self::EntryNotExecutable(at) => {
                write!(f, "{at:#x} is not executable from ring 3")
            }
            Self::StackNotWritable(at) => write!(f, "{at:#x} is not writable from ring 3"),
            Self::StackMisaligned(at) => write!(f, "{at:#x} is not sixteen-byte aligned"),
            Self::NoKernelStack => f.write_str("no ring-0 stack is set for traps from ring 3"),
            Self::FastSyscallOpen => f.write_str("syscall or sysenter is still enabled"),
        }
    }
}

impl core::error::Error for UserError {}

/// Enter ring 3 at `entry`, on `stack_top`. Returns only on refusal.
///
/// Safe, and the whole of why is that it **checks** rather than trusts. The
/// `iretq` underneath will happily send the CPU to an unmapped address in a
/// privilege level that cannot report it, so every precondition is walked
/// first: the tables are live, the entry is present and user-executable, the
/// stack is present and user-writable and not executable, the stack pointer is
/// aligned, and `RSP0` names somewhere for a trap to land. A caller that gets
/// any of them wrong is told, rather than triple-faulting (D-96).
///
/// Interrupts are enabled in the frame that is pushed, because a ring-3 thread
/// that cannot be preempted is a ring-3 thread that owns the machine.
///
/// # Errors
///
/// See [`UserError`]. On success this never returns: the only way back is a
/// trap.
pub fn enter_user(entry: u64, stack_top: u64) -> UserError {
    if !paging::is_active() {
        return UserError::Inactive;
    }
    // Checked here and not only at boot: this is the last instruction before a
    // program runs, and the claim being defended is about what *that* program
    // can reach. `cpu::enable_protections` shuts both doors; asking again
    // costs two MSR reads once per program, and makes the claim a property of
    // the crossing rather than of the boot (D-139).
    if !cpu::fast_syscall_doors_are_shut() {
        return UserError::FastSyscallOpen;
    }
    match paging::translate(entry) {
        Some((_, flags)) if flags.contains(Flags::USER) && !flags.contains(Flags::NO_EXECUTE) => {}
        _ => return UserError::EntryNotExecutable(entry),
    }
    if !stack_top.is_multiple_of(16) {
        return UserError::StackMisaligned(stack_top);
    }
    // The page the first push lands in, which is below the top.
    let first_push = stack_top.wrapping_sub(8);
    match paging::translate(first_push) {
        Some((_, flags))
            if flags.contains(Flags::USER)
                && flags.contains(Flags::WRITABLE)
                && flags.contains(Flags::NO_EXECUTE) => {}
        _ => return UserError::StackNotWritable(first_push),
    }
    // *This* thread's stack, not merely a non-zero one. `prepare` skips the
    // write for a slot with no arena stack — the boot thread's — so after a
    // switch to it, `RSP0` still names whichever arena thread ran last. A
    // caller entering ring 3 then would take its first trap on a stack another
    // thread may be standing on, which is the corruption D-99 was written to
    // prevent and which its prose, not its code, was preventing (D-101).
    let mine = TABLE.lock_leaf().slots[current()].stack_top;
    if mine == 0 || super::segment::kernel_stack() != mine {
        return UserError::NoKernelStack;
    }
    // The same rule `prepare` and `continue_on_stack` enforce, and for the
    // strongest reason of the three: this call never returns, so a guard live
    // in the caller's frame is a lock nothing can ever release.
    assert!(
        crate::sync::spinlocks_held() == 0,
        "a spin lock is held, so entering ring 3 would strand it"
    );

    // SAFETY: the walks above establish exactly what `iretq` needs — an
    // executable user mapping at `entry` and a writable user mapping under
    // `stack_top` — in the tables this CPU is walking, with a ring-0 stack for
    // the trap that brings it back.
    unsafe { leave_for_user(entry, stack_top) }
}

/// Push a ring-3 frame and `iretq` into it.
///
/// # Safety
///
/// `entry` must be user-executable and `stack_top` user-writable in the live
/// tables, and `RSP0` must name a mapped ring-0 stack.
#[unsafe(naked)]
unsafe extern "C" fn leave_for_user(entry: u64, stack_top: u64) -> ! {
    naked_asm!(
        // The frame `iretq` pops, top down: SS, RSP, RFLAGS, CS, RIP.
        "push {user_data}",
        "push rsi",
        // Bit 1 is reserved and must be set; bit 9 is the interrupt flag, so
        // ring 3 is preemptible from its first instruction.
        "push 0x202",
        "push {user_code}",
        "push rdi",
        // Nothing of the kernel's goes with it: a register left set is a
        // pointer handed to ring 3.
        "xor rax, rax",
        "xor rbx, rbx",
        "xor rcx, rcx",
        "xor rdx, rdx",
        "xor rsi, rsi",
        "xor rdi, rdi",
        "xor rbp, rbp",
        "xor r8, r8",
        "xor r9, r9",
        "xor r10, r10",
        "xor r11, r11",
        "xor r12, r12",
        "xor r13, r13",
        "xor r14, r14",
        "xor r15, r15",
        // The buffer flush belongs here as much as on the return path, and
        // more: the kernel has just parsed an initrd, walked page tables and
        // touched the frame allocator. D-98 put it only in the entry stub's
        // tail, which is reached on a *return* to ring 3 — and this kernel
        // retires its callers, so there are almost none (D-101).
        "cmp byte ptr [rip + {clear}], 0",
        "je 2f",
        "sub rsp, 8",
        "mov word ptr [rsp], {kernel_data}",
        "verw word ptr [rsp]",
        "add rsp, 8",
        "inc qword ptr [rip + {clears}]",
        "2:",
        "iretq",
        user_data = const (super::segment::USER_DATA_BITS as u64),
        user_code = const (super::segment::USER_CODE_BITS as u64),
        kernel_data = const (super::segment::KERNEL_DATA_BITS as u32),
        clear = sym super::interrupt::MDS_CLEAR,
        clears = sym super::interrupt::BUFFER_CLEARS,
    )
}

/// Abandon the current stack and continue on `stack`, never returning.
///
/// The boot thread runs on the bootloader's stack, which has nothing below it
/// and sits inside the two-megabyte direct map, so it cannot be guarded where
/// it is (D-89). This moves the flow of control onto a stack that is guarded,
/// and the frames below the call are simply abandoned — the bootloader's stack
/// is reclaimable memory and nothing returns to it.
///
/// Safe, on the same grounds as `switch`: everything the load needs is proved
/// by its arguments. [`MappedStack`] can only come from
/// [`AddressSpace::map_stack`](super::paging::AddressSpace::map_stack), which
/// mapped the region writable and confirmed its guard is absent, and
/// [`is_installed`](MappedStack::is_installed) confirms it belongs to the
/// space this CPU is actually walking. `continuation` cannot return, so there
/// is no return address to be missing.
///
/// # Panics
///
/// If the stack belongs to an address space that is not installed, which would
/// otherwise be a jump onto memory this CPU cannot reach.
pub fn continue_on_stack(stack: MappedStack, continuation: Entry, arg: usize) -> ! {
    assert!(
        stack.is_installed(),
        "a stack from another address space cannot be stood on"
    );
    // The same rule `prepare` enforces before a context switch, for the same
    // reason: a guard live on the stack being abandoned is a lock nothing can
    // ever release, because the frame that would drop it is gone (D-66, D-94).
    assert!(
        crate::sync::spinlocks_held() == 0,
        "a spin lock is held, so the stack being abandoned would strand it"
    );
    // No interrupt may arrive between the load and the first push: the frame
    // it would build belongs to whichever stack the CPU thinks it is on.
    let _disabled = interrupt::disable();

    // SAFETY: `stack` is evidence that its top is the top of a mapped,
    // writable, guarded region that nothing else holds, and `is_installed`
    // has just confirmed the CPU walks the space that mapped it.
    // `continuation` never returns, so no return address is owed.
    unsafe { load_stack(stack.top(), continuation, arg) }
}

/// Load `stack_top` into `rsp` and jump to `continuation(arg)`.
///
/// # Safety
///
/// `stack_top` must be the sixteen-byte-aligned top of a mapped, writable
/// region nothing else is using, reachable in the live tables.
#[unsafe(naked)]
unsafe extern "C" fn load_stack(stack_top: u64, continuation: Entry, arg: usize) -> ! {
    naked_asm!(
        // The ABI wants `rsp + 8` sixteen-byte aligned at a function's first
        // instruction, because a `call` would have pushed eight bytes. This
        // arrives by `jmp`, so the eight bytes are subtracted here instead —
        // the same off-by-one-word that D-76 found in the fabricated frame.
        "lea rsp, [rdi - 8]",
        // Nothing above this frame: a walker that follows `rbp` stops here
        // rather than wandering into the abandoned stack.
        "xor rbp, rbp",
        "mov rdi, rdx",
        "jmp rsi",
    )
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
