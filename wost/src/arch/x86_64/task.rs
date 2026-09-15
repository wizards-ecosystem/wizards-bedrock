//! Kernel threads: the saved state of each, and the switch between them.
//!
//! Each fixed-table slot retains a verified arena stack and saved pointer;
//! callee-saved registers live on that stack. [`fabricate`] builds the initial
//! frame so the first switch returns into `thread_start`.
//!
//! The naked switch loads only state produced here on stacks [`init`] verified,
//! and dead slots cannot resume (D-51). Scheduling policy remains in the
//! kernel.

use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};

use super::cpu;
use super::interrupt::{self, TrapFrame};
use super::msr;
use super::paging::{self, Binding, KernelStackTop, MappedStack, StackArena, TaskStackPin};
use super::xstate;
use crate::cell::HardwareCell;
use crate::panic::PanicToken;
use crate::sync::{Once, SpinLock};
use x86_paging::{CanonicalAddress, Flags};

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
            Self::NotInitialised => {
                core::fmt::Write::write_str(f, "the task table is not initialised")
            }
            Self::Inactive => {
                core::fmt::Write::write_str(f, "the kernel's page tables are not active")
            }
            Self::Unmapped(address) => write!(f, "stack at {address:#x} is not mapped as a stack"),
            Self::TooMany(stacks) => write!(f, "{stacks} stacks exceed {MAX_TASKS} task slots"),
            Self::OutOfRange(slot) => write!(f, "slot {slot} does not exist"),
            Self::Vacant(slot) => write!(f, "slot {slot} holds no thread"),
            Self::Dead(slot) => write!(f, "slot {slot} has exited"),
            Self::Occupied(slot) => write!(f, "slot {slot} is live"),
            Self::Current(slot) => write!(f, "slot {slot} is already running"),
            Self::Boot => core::fmt::Write::write_str(f, "the boot slot cannot be fabricated"),
            Self::Uninterruptible => {
                core::fmt::Write::write_str(f, "the interrupted code holds a critical section")
            }
            Self::NotInTrap => core::fmt::Write::write_str(f, "not inside a trap handler"),
            Self::ForeignFrame => {
                core::fmt::Write::write_str(f, "not the frame the live handler was entered with")
            }
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
    /// The pinned arena index. Absent for the boot slot.
    stack_index: Option<usize>,
    entry: Option<(Entry, usize)>,
    /// Whether the thread should be accepting interrupts on its first run.
    starts_interruptible: bool,
    /// The tables to install when this thread runs, or `None` to keep the
    /// kernel's. Written only through [`bind_address_space`], which takes the
    /// evidence that the space agrees with the kernel about the higher half.
    page_table: Option<u64>,
    /// Where this thread's thread-local storage begins.
    ///
    /// Saved and restored with the privilege stack, page table, and extended
    /// state (D-99, D-106, D-108, D-118).
    thread_base: CanonicalAddress,
}

impl Slot {
    const VACANT: Self = Self {
        state: State::Vacant,
        stack_index: None,
        entry: None,
        starts_interruptible: false,
        page_table: None,
        thread_base: CanonicalAddress::ZERO,
    };
}

#[derive(Debug)]
struct Table {
    slots: [Slot; MAX_TASKS],
    /// Slots in use, the boot slot included.
    count: usize,
    /// Stack identities used to derive each retained-hardware pointer.
    stacks: Option<TaskStackPin>,
}

impl Table {
    fn kernel_stack_top(&self, slot: usize) -> Option<KernelStackTop<'_>> {
        let index = self.slots.get(slot)?.stack_index?;
        self.stacks.as_ref()?.stack_top(index)
    }
}

static TABLE: SpinLock<Table> = SpinLock::new(Table {
    slots: [Slot::VACANT; MAX_TASKS],
    count: 0,
    stacks: None,
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
/// Static storage makes switching allocation-free. Boot verifies the CPUID
/// size against each out-of-line ceiling-sized area (D-82, D-108).
#[repr(C, align(64))]
struct SaveAreas([[u8; xstate::CEILING]; MAX_TASKS]);

static EXTENDED: HardwareCell<SaveAreas> =
    HardwareCell::new(SaveAreas([[0; xstate::CEILING]; MAX_TASKS]));

static CURRENT: AtomicUsize = AtomicUsize::new(BOOT);

#[cfg(feature = "selftest")]
static REFUSE_NEXT_RETIREMENT: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static REFUSE_RELEASES: AtomicUsize = AtomicUsize::new(0);

static INIT: Once = Once::new();

/// A task-table initialization refusal and the still-owned stack arena.
#[derive(Debug)]
#[must_use = "a refused task-table initialization returns its stack arena"]
pub struct InitFailure {
    /// Why the task table refused the arena.
    pub reason: TaskError,
    /// The mappings no task-table slot retained.
    pub arena: StackArena,
}

impl fmt::Display for InitFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.reason, f)
    }
}

impl core::error::Error for InitFailure {}

/// Take the arena's stacks into the table, making the caller the boot thread.
///
/// Every guard and every original stack page is walked in the live tables
/// before the complete extents are pinned. Returns how many threads can be
/// fabricated.
///
/// # Errors
///
/// [`TaskError::Inactive`] before the kernel's tables are active,
/// [`TaskError::Unmapped`] naming the first stack the walk could not find,
/// [`TaskError::TooMany`] for an arena larger than the table, and
/// [`TaskError::NotInitialised`] on a second call. [`InitFailure`] returns the
/// arena unchanged because no slot retained it.
pub fn init(arena: StackArena) -> Result<usize, InitFailure> {
    let saved = interrupt::disable();
    let layout = match validate_arena(&arena) {
        Ok(layout) => layout,
        Err(reason) => {
            saved.restore();
            return Err(InitFailure { reason, arena });
        }
    };
    if !INIT.claim() {
        saved.restore();
        return Err(InitFailure {
            reason: TaskError::NotInitialised,
            arena,
        });
    }

    // Pin before taking TABLE's leaf guard: the pin has its own ledger lock.
    let pin = arena.pin();
    let mut table = TABLE.lock_leaf();
    table.slots[BOOT].state = State::Alive;
    for index in 0..layout.slots() {
        table.slots[index + 1].stack_index = Some(index);
    }
    table.count = layout.slots() + 1;
    table.stacks = Some(pin);
    drop(table);
    saved.restore();
    Ok(layout.slots())
}

fn validate_arena(arena: &StackArena) -> Result<stack_layout::StackLayout, TaskError> {
    if !paging::is_active() {
        return Err(TaskError::Inactive);
    }
    if let Some(address) = arena.invalid_page() {
        return Err(TaskError::Unmapped(address));
    }
    let layout = *arena.layout();
    if layout.slots() + 1 > MAX_TASKS {
        return Err(TaskError::TooMany(layout.slots()));
    }
    for index in 0..layout.slots() {
        if layout.slot(index).is_none() {
            return Err(TaskError::OutOfRange(index));
        }
    }
    Ok(layout)
}

/// Run the production arena validation against the active root.
#[cfg(feature = "selftest")]
pub fn validate_arena_for_test(arena: &StackArena) -> Result<(), TaskError> {
    validate_arena(arena).map(|_| ())
}

/// Substitute one noncurrent pinned stack leaf during its hardware-use check.
#[cfg(feature = "selftest")]
pub fn pinned_stack_rejects_lower_leaf_substitution_for_test(slot: usize) -> Option<bool> {
    if slot == current() {
        return None;
    }
    let table = TABLE.lock_leaf();
    let index = table.slots.get(slot)?.stack_index?;
    table
        .stacks
        .as_ref()?
        .rejects_lower_leaf_substitution_for_test(index)
}

/// The slot whose stack this CPU is running on.
pub fn current() -> usize {
    CURRENT.load(Ordering::Acquire)
}

/// Give `slot` a fresh thread that will begin at `entry(arg)`.
///
/// The initial frame returns into `thread_start` with ABI alignment. Dead-slot
/// reuse is safe because no switch can resume the prior context.
///
/// Interrupt admission is captured from the creator before scheduler locking;
/// the switch itself always observes the scheduler's disabled state.
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
    assert!(
        interrupt::context_is_clear(slot),
        "a reusable task slot retains interrupt-exclusion ownership"
    );

    let extended = EXTENDED
        .as_mut_ptr()
        .cast::<u8>()
        .wrapping_add(slot * xstate::CEILING);
    // SAFETY: `slot` is in TABLE and therefore below `MAX_TASKS`; the table
    // guard and cleared interrupt flag exclude the switch path that uses this
    // complete save area, and the slot is not live yet.
    unsafe { extended.write_bytes(0, xstate::CEILING) };

    let Some(stack_index) = table.slots[slot].stack_index else {
        return Err(TaskError::Vacant(slot));
    };
    let Some(top) = table.kernel_stack_top(slot) else {
        panic!("a reusable task lost its pinned stack identity");
    };
    let base = top.address() - FRAME_BYTES;
    let frame: [u64; FRAME_WORDS] = [0, 0, 0, 0, 0, 0, thread_start as *const () as u64, 0];
    // SAFETY: `init` walked this stack's top page and found it mapped
    // writable; `base` is `FRAME_BYTES` below the top, inside that page, and
    // eight-byte aligned because the top is page-aligned. Nothing else is
    // using the stack: the slot is not `Alive`, and a dead slot's thread can
    // never be resumed. The lock is held, so no other fabrication races.
    unsafe { core::ptr::write(base as usize as *mut [u64; FRAME_WORDS], frame) };

    let word = CONTEXTS.as_mut_ptr().cast::<u64>().wrapping_add(slot);
    // SAFETY: `slot` is below `MAX_TASKS`; the lock is held, and the assembly
    // that also writes this word runs only with the flag clear.
    unsafe { core::ptr::write(word, base) };

    table.slots[slot] = Slot {
        state: State::Alive,
        stack_index: Some(stack_index),
        entry: Some((entry, arg)),
        starts_interruptible,
        page_table: None,
        thread_base: CanonicalAddress::ZERO,
    };
    Ok(())
}

/// Run `slot`, returning when something switches back to this one.
///
/// The caller's interrupt flag is restored on return; the target resumes its
/// own saved flag or inherits the creator's on first entry.
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
        interrupt::switch_context(slot);
        // SAFETY: `prepare` took `save` from this CPU's own slot and `load`
        // from a live slot's record, each written only by `fabricate` or by a
        // previous switch on a stack `init` confirmed. The flag is clear, so
        // nothing runs between the bookkeeping and the switch.
        unsafe { switch(save, load) };
    });
    saved.restore();
    result
}

/// Run `slot` from a trap handler, returning into that same handler later.
///
/// The live frame proves the interrupted code admitted interrupts; otherwise
/// switching could strand a critical section. Trap depth follows the thread
/// and is restored on resumption (D-34, D-55).
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
        interrupt::switch_context(slot);
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
/// Marks the leaving slot dead before switching, making its stack and slot
/// reusable without permitting the interrupted context to resume (D-117).
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
    match prepare(slot, State::Dead) {
        Ok((save, load)) => {
            // This handler and every exclusion parked on its stack die with
            // the context; the target's parked ledger becomes current.
            saved.commit_context_disabled();
            interrupt::switch_context(slot);
            // SAFETY: as in `switch_from_trap`. The slot being left is now
            // `Dead`, so this saved state is never loaded again.
            unsafe { switch(save, load) };
            unreachable!("a task retired from a trap was resumed");
        }
        Err(error) => {
            interrupt::resume_trap(suspended);
            saved.restore();
            Err(error)
        }
    }
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
            saved.commit_context_disabled();
            interrupt::switch_context(slot);
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

/// Validate `slot`, move bookkeeping to it, and return the two stack pointers.
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

    // Every switch must move `RSP0` with its thread or the next ring-3 trap can
    // land on another thread's stack. The ring-0-only boot slot has none (D-99).
    if table.slots[slot].stack_index.is_some() {
        let Some(landing) = table.kernel_stack_top(slot) else {
            panic!("a live task lost its pinned stack identity");
        };
        super::segment::write_kernel_stack(landing);
    }

    // Every switch moves page-table and privilege-stack state together; slots
    // without a private space use the kernel root (D-106).
    let wanted = table.slots[slot]
        .page_table
        .unwrap_or_else(paging::active_root);
    if wanted != cpu::page_table_base() {
        // SAFETY: `bind_address_space` proved every higher-half top-level entry
        // matches the kernel root. Paging activation freezes new top-level
        // entries, so later device mappings remain below shared entries. IF is
        // clear and the table lock prevents an interleaving CR3 change.
        unsafe { cpu::set_page_table_base(wanted) };
    }

    // Eager save/restore is outside the naked switch because `+soft-float`
    // guarantees the intervening kernel path cannot disturb it (D-82, D-108).
    if xstate::is_enabled() {
        let areas = EXTENDED.as_mut_ptr().cast::<u8>();
        let leaving = areas.wrapping_add(current * xstate::CEILING);
        let arriving = areas.wrapping_add(slot * xstate::CEILING);
        // SAFETY: both indices are below `MAX_TASKS`; each area is aligned and
        // at least `bytes()` long, and the lock and clear flag exclude access.
        unsafe { xstate::save(leaving) };
        // SAFETY: as above. The area holds either initial-state zeros or a
        // complete value written by `save`.
        unsafe { xstate::restore(arriving) };
    }

    // Read FS_BASE at the switch because ring 3 can change its own value
    // without updating remembered state (D-118).
    let Some(leaving) = CanonicalAddress::new(read_thread_base()) else {
        panic!("the processor retained a non-canonical FS base");
    };
    table.slots[current].thread_base = leaving;
    let arriving = table.slots[slot].thread_base;
    if arriving.get() != read_thread_base() {
        write_thread_base(arriving);
    }

    let contexts = CONTEXTS.as_mut_ptr().cast::<u64>();
    let save = contexts.wrapping_add(current);
    let target = contexts.wrapping_add(slot);
    // SAFETY: both indices are below `MAX_TASKS`; the lock and clear flag
    // exclude the assembly writer, and `fabricate` or a switch wrote the word.
    let load = unsafe { core::ptr::read(target) };
    Ok((save, load))
}

/// Install `binding`'s tables whenever `slot` runs.
///
/// Consumes checked [`Binding`] evidence, preventing a bare unverified `CR3`
/// or a second slot from naming the same space.
///
/// # Errors
///
/// [`TaskError::OutOfRange`] past the table, [`TaskError::Vacant`] for a slot
/// with no thread, and [`TaskError::Current`] for the running one — changing
/// the tables under a thread that is already on them is a job for the switch,
/// not for this. A refusal returns the unconsumed binding with the error.
pub fn bind_address_space(slot: usize, binding: Binding) -> Result<Bound, (TaskError, Binding)> {
    let root = binding.root();
    let saved = interrupt::disable();
    let mut table = TABLE.lock_leaf();
    let result = if slot >= table.count {
        Err(TaskError::OutOfRange(slot))
    } else if table.slots[slot].state != State::Alive {
        Err(TaskError::Vacant(slot))
    } else if slot == CURRENT.load(Ordering::Acquire) {
        Err(TaskError::Current(slot))
    } else {
        table.slots[slot].page_table = Some(root);
        Ok(())
    };
    drop(table);
    saved.restore();
    result
        .map(|()| Bound { root })
        .map_err(|error| (error, binding))
}

/// Give a fabricated slot back before anything has run on it.
///
/// Rolls back setup between [`fabricate`] and scheduler publication so the two
/// slot ledgers cannot disagree (D-115).
///
/// # Errors
///
/// [`TaskError::OutOfRange`] past the table, [`TaskError::Current`] for the
/// running slot — a thread cannot retire itself this way — and
/// [`TaskError::Vacant`] for one nothing fabricated.
pub fn retire(slot: usize) -> Result<(), TaskError> {
    #[cfg(feature = "selftest")]
    if REFUSE_NEXT_RETIREMENT.swap(0, Ordering::AcqRel) != 0 {
        return Err(TaskError::Current(slot));
    }
    let saved = interrupt::disable();
    let mut table = TABLE.lock_leaf();
    let result = if slot >= table.count {
        Err(TaskError::OutOfRange(slot))
    } else if slot == CURRENT.load(Ordering::Acquire) {
        Err(TaskError::Current(slot))
    } else if table.slots[slot].state != State::Alive {
        Err(TaskError::Vacant(slot))
    } else {
        let stack_index = table.slots[slot].stack_index;
        table.slots[slot] = Slot {
            stack_index,
            ..Slot::VACANT
        };
        Ok(())
    };
    drop(table);
    saved.restore();
    result
}

/// Make the next prepared-slot retirement refuse without changing the slot.
#[cfg(feature = "selftest")]
pub fn refuse_next_retirement_for_test() {
    REFUSE_NEXT_RETIREMENT.store(1, Ordering::Release);
}

/// Clear an unused retirement refusal after a test setup failed early.
#[cfg(feature = "selftest")]
pub fn restore_retirement_for_test() {
    REFUSE_NEXT_RETIREMENT.store(0, Ordering::Release);
}

/// Whether a task-table slot is still live.
#[cfg(feature = "selftest")]
pub fn slot_is_live_for_test(slot: usize) -> bool {
    let saved = interrupt::disable();
    let live = TABLE
        .lock_leaf()
        .slots
        .get(slot)
        .is_some_and(|slot| slot.state == State::Alive);
    saved.restore();
    live
}

/// Evidence that one task-table slot names an address space.
///
/// Minted only when [`bind_address_space`] installs a [`Binding`]. The token
/// stays with the process owner until retirement, so release cannot be asked
/// about a root that still has an uninstalled binding elsewhere.
#[derive(Debug)]
pub struct Bound {
    root: u64,
}

impl Bound {
    /// The root installed in the task table.
    pub const fn root(&self) -> u64 {
        self.root
    }
}

/// Evidence that no binding can install or revisit `root`.
///
/// Minted by consuming either an uninstalled [`Binding`] or a retired
/// [`Bound`]. `AddressSpace` cannot inspect the task table, so it takes this
/// answer instead of guessing from the current `CR3` (D-123).
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

/// Cancel a binding that was never installed in the task table.
///
/// Consuming the one-shot [`Binding`] proves that no later caller can install
/// it. This is the rollback path for construction that stops before
/// [`bind_address_space`].
pub fn release_uninstalled(binding: Binding) -> Released {
    Released {
        root: binding.root(),
    }
}

/// Give up an installed binding, if no live slot still runs on it.
///
/// Clears the binding from every slot naming it — which are all dead by then —
/// and hands back the evidence [`AddressSpace::reclaim`](super::paging::AddressSpace::reclaim) needs.
///
/// Returns the token unchanged if a live slot still names it.
pub fn release_address_space(bound: Bound) -> Result<Released, Bound> {
    #[cfg(feature = "selftest")]
    if REFUSE_RELEASES.load(Ordering::Acquire) != 0 {
        return Err(bound);
    }
    let root = bound.root();
    let saved = interrupt::disable();
    let mut table = TABLE.lock_leaf();
    let live = table
        .slots
        .iter()
        .any(|slot| slot.state == State::Alive && slot.page_table == Some(root));
    if !live {
        for slot in &mut table.slots {
            if slot.page_table == Some(root) {
                slot.page_table = None;
            }
        }
    }
    drop(table);
    saved.restore();
    if live {
        Err(bound)
    } else {
        Ok(Released { root })
    }
}

/// Keep address-space release from consuming its `Bound` token.
#[cfg(feature = "selftest")]
pub fn refuse_releases_for_test() {
    REFUSE_RELEASES.store(1, Ordering::Release);
}

/// Clear an unused address-space release refusal.
#[cfg(feature = "selftest")]
pub fn restore_release_for_test() {
    REFUSE_RELEASES.store(0, Ordering::Release);
}

/// `IA32_FS_BASE`, where a thread's local storage begins.
const FS_BASE: u32 = 0xC000_0100;

/// Where the running thread's local storage begins.
pub fn read_thread_base() -> u64 {
    msr::read(FS_BASE)
}

fn write_thread_base(base: CanonicalAddress) {
    // SAFETY: the token excludes the non-canonical values `wrmsr` rejects.
    unsafe { msr::write(FS_BASE, base.get()) };
}

/// Point the running thread's local storage at `base`, if it may.
///
/// Non-canonical and higher-half bases are refused without writing (D-118).
#[must_use = "a refused thread-base write changed no register"]
pub fn set_thread_base(base: u64) -> bool {
    let Some(base) = CanonicalAddress::new(base) else {
        return false;
    };
    if !crate::arch::current::paging::is_user_address(base.get()) {
        return false;
    }
    write_thread_base(base);
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
    /// Maskable interrupts were not admitted before the transition.
    InterruptsNotAdmitted,
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
    /// The processor is not enforcing non-executable page mappings.
    NoExecuteProtection,
    /// Thread switches are not saving and restoring user FP/SIMD state.
    NoExtendedState,
    /// Actual CPU evidence does not permit shared higher-half mappings.
    NoSharedKernelIsolation,
}

impl fmt::Display for UserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InterruptsNotAdmitted => {
                core::fmt::Write::write_str(f, "maskable interrupts are not admitted")
            }
            Self::Inactive => {
                core::fmt::Write::write_str(f, "the kernel's tables are not installed")
            }
            Self::EntryNotExecutable(at) => {
                write!(f, "{at:#x} is not executable from ring 3")
            }
            Self::StackNotWritable(at) => write!(f, "{at:#x} is not writable from ring 3"),
            Self::StackMisaligned(at) => write!(f, "{at:#x} is not sixteen-byte aligned"),
            Self::NoKernelStack => {
                core::fmt::Write::write_str(f, "no ring-0 stack is set for traps from ring 3")
            }
            Self::FastSyscallOpen => {
                core::fmt::Write::write_str(f, "syscall or sysenter is still enabled")
            }
            Self::NoExecuteProtection => {
                core::fmt::Write::write_str(f, "no-execute protection is not active")
            }
            Self::NoExtendedState => {
                core::fmt::Write::write_str(f, "extended user state is not isolated")
            }
            Self::NoSharedKernelIsolation => core::fmt::Write::write_str(
                f,
                "shared kernel mappings have no hardware isolation proof",
            ),
        }
    }
}

impl core::error::Error for UserError {}

/// Enter ring 3 at `entry`, on `stack_top`. Returns only on refusal.
///
/// Preflight walks the live entry, user stack, and current `RSP0` before
/// `iretq`; invalid mappings are reported rather than triple-faulting
/// (D-96).
///
/// # Errors
///
/// See [`UserError`]. On success this never returns: the only way back is a
/// trap.
pub fn enter_user(entry: u64, stack_top: u64) -> UserError {
    let disabled = interrupt::disable();
    if !disabled.were_enabled() {
        disabled.restore();
        return UserError::InterruptsNotAdmitted;
    }
    if let Err(error) = user_entry_preflight(entry, stack_top) {
        disabled.restore();
        return error;
    }
    disabled.commit_disabled();

    // SAFETY: the preflight ran with interrupts excluded through this jump,
    // so none of its live page-table or task-stack facts can change first.
    unsafe { leave_for_user(entry, stack_top) }
}

/// Exercise the final door while its interrupt admission is absent.
#[cfg(feature = "selftest")]
pub fn final_interrupt_admission_refusal_for_test() -> bool {
    if !interrupt::are_enabled() {
        return false;
    }
    let outer = interrupt::disable();
    let refused = enter_user(0, 0) == UserError::InterruptsNotAdmitted;
    let stayed_disabled = !interrupt::are_enabled();
    outer.restore();
    refused && stayed_disabled && interrupt::are_enabled()
}

fn user_entry_preflight(entry: u64, stack_top: u64) -> Result<(), UserError> {
    if !paging::is_active() {
        return Err(UserError::Inactive);
    }
    // Recheck at the final boundary so closed syscall doors are a property of
    // this crossing rather than only the boot sequence (D-139).
    if !cpu::fast_syscall_doors_are_shut() {
        return Err(UserError::FastSyscallOpen);
    }
    if !final_no_execute_is_active() {
        return Err(UserError::NoExecuteProtection);
    }
    if !final_extended_state_is_isolated() {
        return Err(UserError::NoExtendedState);
    }
    if !final_shared_mappings_are_safe() {
        return Err(UserError::NoSharedKernelIsolation);
    }
    match paging::translate(entry) {
        Some((_, flags)) if flags.contains(Flags::USER) && !flags.contains(Flags::NO_EXECUTE) => {}
        _ => return Err(UserError::EntryNotExecutable(entry)),
    }
    if !stack_top.is_multiple_of(16) {
        return Err(UserError::StackMisaligned(stack_top));
    }
    // The page the first push lands in, which is below the top.
    let first_push = stack_top.wrapping_sub(8);
    match paging::translate(first_push) {
        Some((_, flags))
            if flags.contains(Flags::USER)
                && flags.contains(Flags::WRITABLE)
                && flags.contains(Flags::NO_EXECUTE) => {}
        _ => return Err(UserError::StackNotWritable(first_push)),
    }
    // Require this slot's stack: the boot slot has none and can otherwise
    // inherit another thread's stale `RSP0` (D-99, D-101).
    let table = TABLE.lock_leaf();
    let mine = table
        .kernel_stack_top(current())
        .map(KernelStackTop::address);
    drop(table);
    if mine.is_none() || super::segment::kernel_stack() != mine.unwrap_or(0) {
        return Err(UserError::NoKernelStack);
    }
    // As `prepare` and `continue_on_stack`: this call never returns, so a
    // guard live in the caller's frame is a lock nothing can ever release.
    assert!(
        crate::sync::spinlocks_held() == 0,
        "a spin lock is held, so entering ring 3 would strand it"
    );

    Ok(())
}

#[cfg(feature = "selftest")]
static FORCED_USER_BOUNDARY_REFUSAL: AtomicUsize = AtomicUsize::new(0);

fn final_no_execute_is_active() -> bool {
    let enabled = cpu::Protections::enabled().nx;
    #[cfg(feature = "selftest")]
    let enabled = enabled && FORCED_USER_BOUNDARY_REFUSAL.load(Ordering::Acquire) != 1;
    enabled
}

#[cfg(feature = "selftest")]
fn forced_boundary_refuses(value: usize, expected: UserError) -> bool {
    let previous = FORCED_USER_BOUNDARY_REFUSAL.swap(value, Ordering::AcqRel);
    let found = user_entry_preflight(0, 0) == Err(expected);
    FORCED_USER_BOUNDARY_REFUSAL.store(previous, Ordering::Release);
    found
}

fn final_extended_state_is_isolated() -> bool {
    let enabled = xstate::user_state_is_isolated();
    #[cfg(feature = "selftest")]
    let enabled = enabled && FORCED_USER_BOUNDARY_REFUSAL.load(Ordering::Acquire) != 2;
    enabled
}

fn final_shared_mappings_are_safe() -> bool {
    let safe = cpu::shared_kernel_mappings_are_safe();
    #[cfg(feature = "selftest")]
    let safe = safe && FORCED_USER_BOUNDARY_REFUSAL.load(Ordering::Acquire) != 3;
    safe
}

/// Exercise the exact final-door NX refusal.
#[cfg(feature = "selftest")]
pub fn final_no_execute_refusal_for_test() -> bool {
    forced_boundary_refuses(1, UserError::NoExecuteProtection)
}

/// Exercise the exact final-door extended-state refusal.
#[cfg(feature = "selftest")]
pub fn final_extended_state_refusal_for_test() -> bool {
    forced_boundary_refuses(2, UserError::NoExtendedState)
}

/// Exercise the exact final-door shared-mapping refusal.
#[cfg(feature = "selftest")]
pub fn final_shared_mapping_refusal_for_test() -> bool {
    forced_boundary_refuses(3, UserError::NoSharedKernelIsolation)
}

/// Push a ring-3 frame and `iretq` into it.
///
/// # Safety
///
/// `entry` must be user-executable and `stack_top` user-writable in the live
/// tables, and `RSP0` must name a mapped ring-0 stack.
#[unsafe(naked)]
unsafe extern "C" fn leave_for_user(entry: u64, stack_top: u64) -> ! {
    ::core::arch::naked_asm!(
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
        // First entry must flush too; this path need not have a prior trap
        // return on which the ordinary exit stub could do it (D-101).
        "cmp byte ptr [rip + {clear}], 0",
        "je 2f",
        "sub rsp, 8",
        "mov word ptr [rsp], {kernel_data}",
        "verw word ptr [rsp]",
        ".if {record_clears}",
        "jnz 3f",
        "inc qword ptr [rip + BUFFER_CLEARS]",
        "3:",
        ".endif",
        "add rsp, 8",
        "2:",
        "iretq",
        user_data = const (super::segment::USER_DATA_BITS as u64),
        user_code = const (super::segment::USER_CODE_BITS as u64),
        kernel_data = const (super::segment::KERNEL_DATA_BITS as u32),
        clear = sym super::interrupt::MDS_CLEAR,
        record_clears = const super::interrupt::RECORD_BUFFER_CLEARS,
    )
}

/// Abandon the current stack and continue on `stack`, never returning.
///
/// [`MappedStack`] proves writable pages and an absent guard;
/// [`is_installed`](MappedStack::is_installed) ties them to the active root
/// (D-89).
///
/// # Panics
///
/// If the stack belongs to an address space that is not installed, which would
/// otherwise be a jump onto memory this CPU cannot reach.
pub fn continue_on_stack(stack: MappedStack, continuation: Entry, arg: usize) -> ! {
    // Hold the exclusion across the live re-walk and the nonreturning stack
    // load, so the mapping cannot be retired between them.
    let disabled = interrupt::disable();
    assert!(
        stack.is_installed(),
        "a stack from another address space cannot be stood on"
    );
    let stack = stack
        .retain()
        .unwrap_or_else(|reason| panic!("the boot stack could not be retained: {reason}"));
    // As `prepare`: a guard live on the abandoned stack is a lock nothing can
    // release, because the frame that would drop it is gone (D-66, D-94).
    assert!(
        crate::sync::spinlocks_held() == 0,
        "a spin lock is held, so the stack being abandoned would strand it"
    );
    disabled.commit_disabled();

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
    ::core::arch::naked_asm!(
        // A jump supplies no return word, so subtract one to reproduce the
        // function-entry alignment a call would create (D-76).
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
/// Only ABI callee-saved registers cross. The flag must be clear: an interrupt
/// between the stack-pointer store and load would run on ambiguously owned
/// stack state.
///
/// # Safety
///
/// `save` must point at a word this thread's saved pointer may be written to,
/// and `load` must be a pointer a previous call stored or [`fabricate`]
/// wrote, on a stack nothing else is using. The interrupt flag must be clear.
#[unsafe(naked)]
unsafe extern "C" fn switch(save: *mut u64, load: u64) {
    ::core::arch::naked_asm!(
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
