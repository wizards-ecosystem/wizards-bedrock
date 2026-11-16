use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};

use x86_paging::{CanonicalAddress, Flags};

use super::cpu;
use super::interrupt::{self, TrapFrame};
use super::kpti;
use super::msr;
use super::paging::{
    self, AddressSpaceRoots, Binding, KernelStackTop, MappedStack, StackArena, TaskStackPin,
};
use super::xstate;
use crate::cell::HardwareCell;
use crate::panic::PanicToken;
use crate::sync::{Once, SpinLock};

pub(crate) const MAX_TASKS: usize = 32;

pub const BOOT: usize = 0;

pub type Entry = extern "C" fn(usize) -> !;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum TaskError {
    NotInitialised,
    Inactive,
    Unmapped(u64),
    TooMany(usize),
    OutOfRange(usize),
    Vacant(usize),
    Dead(usize),
    Occupied(usize),
    Current(usize),
    Boot,
    Uninterruptible,
    NotInTrap,
    InTrap,
    ForeignFrame,
}

impl fmt::Display for TaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotInitialised => formatter.write_str("the task table is not initialised"),
            Self::Inactive => formatter.write_str("the kernel's page tables are not active"),
            Self::Unmapped(address) => {
                write!(formatter, "stack at {address:#x} is not mapped as a stack")
            }
            Self::TooMany(stacks) => {
                write!(formatter, "{stacks} stacks exceed {MAX_TASKS} task slots")
            }
            Self::OutOfRange(slot) => write!(formatter, "slot {slot} does not exist"),
            Self::Vacant(slot) => write!(formatter, "slot {slot} holds no thread"),
            Self::Dead(slot) => write!(formatter, "slot {slot} has exited"),
            Self::Occupied(slot) => write!(formatter, "slot {slot} is live"),
            Self::Current(slot) => write!(formatter, "slot {slot} is already running"),
            Self::Boot => formatter.write_str("the boot slot has no arena stack"),
            Self::Uninterruptible => {
                formatter.write_str("the interrupted code holds a critical section")
            }
            Self::NotInTrap => formatter.write_str("not inside a trap handler"),
            Self::InTrap => formatter.write_str("inside a trap handler"),
            Self::ForeignFrame => {
                formatter.write_str("not the frame the live handler was entered with")
            }
        }
    }
}

impl core::error::Error for TaskError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Vacant,
    Alive,
    Dead,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    state: State,
    stack_index: Option<usize>,
    entry: Option<(Entry, usize)>,
    starts_interruptible: bool,
    page_table: Option<AddressSpaceRoots>,
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
    count: usize,
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

// Switch-assembly state, touched only with interrupts clear and the slot settled under `TABLE`.
static CONTEXTS: HardwareCell<[u64; MAX_TASKS]> = HardwareCell::new([0; MAX_TASKS]);

#[derive(Debug)]
#[repr(C, align(64))]
struct SaveAreas([[u8; xstate::CEILING]; MAX_TASKS]);

static EXTENDED: HardwareCell<SaveAreas> =
    HardwareCell::new(SaveAreas([[0; xstate::CEILING]; MAX_TASKS]));

static CURRENT: AtomicUsize = AtomicUsize::new(BOOT);
static INIT: Once = Once::new();

#[derive(Debug)]
#[must_use = "a refused task-table initialization returns its stack arena"]
pub struct InitFailure {
    pub reason: TaskError,
    pub arena: StackArena,
}

impl fmt::Display for InitFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.reason, formatter)
    }
}

impl core::error::Error for InitFailure {}

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

pub fn current() -> usize {
    CURRENT.load(Ordering::Acquire)
}

pub fn fabricate(slot: usize, entry: Entry, argument: usize) -> Result<(), TaskError> {
    // Before the lock, which clears the flag: the caller's own flag is wanted.
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
    // SAFETY: `slot` is below `count`, so below `MAX_TASKS`, and `extended` starts one
    // `CEILING`-byte element of `EXTENDED`; the lock and clear flag exclude the switch path, its
    // only other user, and the slot is not live.
    unsafe { xstate::write_initial(extended) };

    let stack_index = table.slots[slot].stack_index;
    let Some(top) = table.kernel_stack_top(slot) else {
        panic!("a reusable task lost its pinned stack identity");
    };
    let base = top.address() - FRAME_BYTES;
    let frame: [u64; FRAME_WORDS] = [0, 0, 0, 0, 0, 0, thread_start as *const () as u64, 0];
    // SAFETY: `init` found the top page writable; `base`, `FRAME_BYTES` below the page-aligned top,
    // is aligned in that page, nothing uses a non-`Alive` slot's stack, and the lock is held.
    unsafe { core::ptr::write(base as usize as *mut [u64; FRAME_WORDS], frame) };

    let word = CONTEXTS.as_mut_ptr().cast::<u64>().wrapping_add(slot);
    // SAFETY: `slot` is below `MAX_TASKS`; the lock is held, and the assembly that also writes
    // this word runs only with the flag clear.
    unsafe { core::ptr::write(word, base) };

    table.slots[slot] = Slot {
        state: State::Alive,
        stack_index,
        entry: Some((entry, argument)),
        starts_interruptible,
        page_table: None,
        thread_base: CanonicalAddress::ZERO,
    };
    Ok(())
}

pub fn switch_to(slot: usize) -> Result<(), TaskError> {
    if interrupt::in_trap() {
        return Err(TaskError::InTrap);
    }
    let saved = interrupt::disable();
    let outcome = prepare(slot, State::Alive).map(|(save, load)| {
        interrupt::switch_context(slot);
        // SAFETY: `prepare` took `save` from this CPU's slot and `load` from a live record, written
        // only by `fabricate` or a verified-stack switch; IF is clear, so nothing runs in between.
        unsafe { switch(save, load) };
    });
    saved.restore();
    outcome
}

pub fn switch_from_trap(slot: usize, frame: &TrapFrame) -> Result<(), TaskError> {
    if !interrupt::in_trap() {
        return Err(TaskError::NotInTrap);
    }
    if !interrupt::is_live_frame(frame) {
        return Err(TaskError::ForeignFrame);
    }
    if !frame.was_interruptible() {
        return Err(TaskError::Uninterruptible);
    }

    let saved = interrupt::disable();
    let suspended = interrupt::suspend_trap();
    let outcome = prepare(slot, State::Alive).map(|(save, load)| {
        interrupt::switch_context(slot);
        // SAFETY: as in `switch_to`, `prepare` vouches for both pointers with the flag clear; the
        // handler's frame lies below the saved pointer, and only this thread will unwind it.
        unsafe { switch(save, load) };
    });
    interrupt::resume_trap(suspended);
    saved.restore();
    outcome
}

pub fn retire_to(slot: usize, frame: &TrapFrame) -> Result<(), TaskError> {
    if !interrupt::in_trap() {
        return Err(TaskError::NotInTrap);
    }
    if !interrupt::is_live_frame(frame) {
        return Err(TaskError::ForeignFrame);
    }
    if !frame.was_interruptible() {
        return Err(TaskError::Uninterruptible);
    }

    let saved = interrupt::disable();
    let suspended = interrupt::suspend_trap();
    match prepare(slot, State::Dead) {
        Ok((save, load)) => {
            // This handler and every exclusion parked on its stack die with the context.
            saved.commit_context_disabled();
            interrupt::switch_context(slot);
            // SAFETY: as in `switch_from_trap`, and the slot being left is now `Dead`, so this
            // saved state is never loaded again.
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

pub fn exit_to(slot: usize) -> TaskError {
    let saved = interrupt::disable();
    match prepare(slot, State::Dead) {
        Ok((save, load)) => {
            saved.commit_context_disabled();
            interrupt::switch_context(slot);
            // SAFETY: as in `switch_to`, and the slot being left is now `Dead`, which every later
            // switch refuses, so the save is never loaded.
            unsafe { switch(save, load) };
            unreachable!("a thread marked dead was resumed");
        }
        Err(error) => {
            saved.restore();
            error
        }
    }
}

fn prepare(slot: usize, leaving: State) -> Result<(*mut u64, u64), TaskError> {
    assert!(
        crate::sync::spinlocks_held() == 0,
        "a spin lock is held, so the thread left behind could never release it"
    );
    let mut table = TABLE.lock_leaf();
    let current = admit(&table, slot)?;

    // Revoke the old route before changing the current task, so an NMI here fails closed.
    kpti::begin_route_update();
    table.slots[current].state = leaving;
    CURRENT.store(slot, Ordering::Release);

    // Root, route and privilege stack move together before the target can run.
    let roots = table.slots[slot].page_table;
    let wanted = roots.map_or_else(paging::active_root, AddressSpaceRoots::full_root);
    let paired = roots.is_some_and(AddressSpaceRoots::is_paired);
    if paired {
        let Some(landing) = table.kernel_stack_top(slot) else {
            panic!("a paired task lost its pinned stack identity");
        };
        let Some(roots) = roots else {
            panic!("a paired task lost its root identity");
        };
        if let Err(reason) = kpti::publish_route(&roots, landing) {
            panic!("a bound root pair could not publish its entry route: {reason:?}");
        }
        assert!(
            kpti::install_transition_rsp0(),
            "a published root pair could not install its entry stacks"
        );
    } else {
        restore_ordinary_entry(&table, slot);
    }
    if wanted != cpu::page_table_base() {
        // SAFETY: `bind_address_space` matched the higher half to the kernel root, frozen at
        // activation so device maps stay under shared entries; IF is clear and the lock held.
        unsafe { cpu::set_page_table_base(wanted) };
    }
    if paired {
        let Some(roots) = roots else {
            panic!("a paired task lost its root identity after activation");
        };
        assert!(
            kpti::finish_route_update(roots),
            "an active root pair could not publish its complete entry route"
        );
    }

    // Saving outside the naked switch is sound only because the kernel is built soft-float.
    if xstate::is_enabled() {
        let areas = EXTENDED.as_mut_ptr().cast::<u8>();
        let leaving = areas.wrapping_add(current * xstate::CEILING);
        let arriving = areas.wrapping_add(slot * xstate::CEILING);
        // SAFETY: both indices are below `MAX_TASKS`, so each pointer starts a `CEILING`-byte
        // element of the 64-byte-aligned `EXTENDED` (1024 is a multiple of 64), and `init`
        // refused any `bytes()` above `CEILING`; the lock and clear flag exclude other access.
        unsafe { xstate::save(leaving) };
        // SAFETY: as above, and the area holds initial-state zeros or a complete `save`.
        unsafe { xstate::restore(arriving) };
    }

    switch_thread_base(&mut table, current, slot);

    let contexts = CONTEXTS.as_mut_ptr().cast::<u64>();
    let save = contexts.wrapping_add(current);
    let target = contexts.wrapping_add(slot);
    // SAFETY: both indices are below `MAX_TASKS`; the lock and clear flag exclude the assembly
    // writer, and `fabricate` or a switch wrote the word.
    let load = unsafe { core::ptr::read(target) };
    Ok((save, load))
}

fn admit(table: &Table, slot: usize) -> Result<usize, TaskError> {
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
    Ok(current)
}

fn restore_ordinary_entry(table: &Table, slot: usize) {
    if table.slots[slot].stack_index.is_some() {
        let Some(landing) = table.kernel_stack_top(slot) else {
            panic!("a live task lost its pinned stack identity");
        };
        super::segment::write_kernel_stack(landing);
    }
    super::segment::restore_ordinary_fault_stack();
    kpti::clear_route();
}

// Ring 3 can change its own FS base, so the switch reads it rather than trusting the slot.
fn switch_thread_base(table: &mut Table, current: usize, slot: usize) {
    let Some(leaving) = CanonicalAddress::new(read_thread_base()) else {
        panic!("the processor retained a non-canonical FS base");
    };
    table.slots[current].thread_base = leaving;
    super::segment::reset_data_selectors();
    write_thread_base(table.slots[slot].thread_base);
}

pub fn bind_address_space(slot: usize, binding: Binding) -> Result<Bound, (TaskError, Binding)> {
    let roots = binding.roots();
    let saved = interrupt::disable();
    let mut table = TABLE.lock_leaf();
    let outcome = if slot >= table.count {
        Err(TaskError::OutOfRange(slot))
    } else if slot == BOOT {
        Err(TaskError::Boot)
    } else if table.slots[slot].state != State::Alive {
        Err(TaskError::Vacant(slot))
    } else if slot == CURRENT.load(Ordering::Acquire) {
        Err(TaskError::Current(slot))
    } else {
        table.slots[slot].page_table = Some(roots);
        Ok(())
    };
    drop(table);
    saved.restore();
    outcome
        .map(|()| Bound { roots })
        .map_err(|error| (error, binding))
}

pub fn retire(slot: usize) -> Result<(), TaskError> {
    let saved = interrupt::disable();
    let mut table = TABLE.lock_leaf();
    let outcome = if slot >= table.count {
        Err(TaskError::OutOfRange(slot))
    } else if slot == CURRENT.load(Ordering::Acquire) {
        Err(TaskError::Current(slot))
    } else if table.slots[slot].state != State::Alive {
        Err(TaskError::Vacant(slot))
    } else {
        assert!(
            interrupt::context_is_clear(slot),
            "a retired task slot retains interrupt-exclusion ownership"
        );
        let stack_index = table.slots[slot].stack_index;
        table.slots[slot] = Slot {
            stack_index,
            ..Slot::VACANT
        };
        Ok(())
    };
    drop(table);
    saved.restore();
    outcome
}

#[derive(Debug)]
pub struct Bound {
    roots: AddressSpaceRoots,
}

impl Bound {
    pub(crate) const fn roots(&self) -> AddressSpaceRoots {
        self.roots
    }
}

#[derive(Debug)]
pub struct Released {
    roots: AddressSpaceRoots,
}

impl Released {
    pub(crate) const fn roots(&self) -> AddressSpaceRoots {
        self.roots
    }
}

pub fn release_uninstalled(binding: Binding) -> Released {
    Released {
        roots: binding.roots(),
    }
}

pub fn release_address_space(bound: Bound) -> Result<Released, Bound> {
    let roots = bound.roots();
    let saved = interrupt::disable();
    let mut table = TABLE.lock_leaf();
    let live = table
        .slots
        .iter()
        .any(|slot| slot.state == State::Alive && slot.page_table == Some(roots));
    if !live {
        for slot in &mut table.slots {
            if slot.page_table == Some(roots) {
                slot.page_table = None;
            }
        }
    }
    drop(table);
    saved.restore();
    if live {
        Err(bound)
    } else {
        Ok(Released { roots })
    }
}

const FS_BASE: u32 = 0xC000_0100;

pub fn read_thread_base() -> u64 {
    msr::read(FS_BASE)
}

fn write_thread_base(base: CanonicalAddress) {
    // SAFETY: the token excludes the non-canonical values `wrmsr` rejects.
    unsafe { msr::write(FS_BASE, base.get()) };
}

#[must_use = "a refused thread-base write changed no register"]
pub fn set_thread_base(base: u64) -> bool {
    let Some(base) = CanonicalAddress::new(base) else {
        return false;
    };
    if !paging::is_user_address(base.get()) {
        return false;
    }
    write_thread_base(base);
    true
}

// Six callee-saved registers, the return address and a pad: `rsp` ends eight below the top.
const FRAME_BYTES: u64 = FRAME_WORDS as u64 * 8;

const FRAME_WORDS: usize = 8;

// The switch always arrives with interrupts clear, so the creator's flag is restored here.
extern "C" fn thread_start() -> ! {
    let slot = current();
    let (entry, interruptible) = {
        let table = TABLE.lock_leaf();
        (
            table.slots[slot].entry,
            table.slots[slot].starts_interruptible,
        )
    };
    let Some((entry, argument)) = entry else {
        panic!("slot {slot} started with no entry point");
    };
    if interruptible {
        interrupt::enable();
    }
    entry(argument)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum UserError {
    InterruptsNotAdmitted,
    Inactive,
    EntryNotExecutable(u64),
    StackNotWritable(u64),
    StackMisaligned(u64),
    NoKernelStack,
    FastSyscallOpen,
    NoExecuteProtection,
    AccessClearUnarmed,
    NoExtendedState,
    NoSharedKernelIsolation,
    NoPairedKernelIsolation,
}

impl fmt::Display for UserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InterruptsNotAdmitted => {
                formatter.write_str("maskable interrupts are not admitted")
            }
            Self::Inactive => formatter.write_str("the kernel's tables are not installed"),
            Self::EntryNotExecutable(address) => {
                write!(formatter, "{address:#x} is not executable from ring 3")
            }
            Self::StackNotWritable(address) => {
                write!(formatter, "{address:#x} is not writable from ring 3")
            }
            Self::StackMisaligned(address) => {
                write!(formatter, "{address:#x} is not sixteen-byte aligned")
            }
            Self::NoKernelStack => {
                formatter.write_str("no ring-0 stack is set for traps from ring 3")
            }
            Self::FastSyscallOpen => formatter.write_str("syscall or sysenter is still enabled"),
            Self::NoExecuteProtection => formatter.write_str("no-execute protection is not active"),
            Self::AccessClearUnarmed => {
                formatter.write_str("smap is on but trap entry does not clear ac")
            }
            Self::NoExtendedState => formatter.write_str("extended user state is not isolated"),
            Self::NoSharedKernelIsolation => {
                formatter.write_str("shared kernel mappings have no hardware isolation proof")
            }
            Self::NoPairedKernelIsolation => {
                formatter.write_str("the paired user roots have no complete live entry route")
            }
        }
    }
}

impl core::error::Error for UserError {}

pub fn enter_user(entry: u64, stack_top: u64) -> UserError {
    let disabled = interrupt::disable();
    if !disabled.were_enabled() {
        disabled.restore();
        return UserError::InterruptsNotAdmitted;
    }
    if let Err(reason) = user_entry_preflight(entry, stack_top) {
        disabled.restore();
        return reason;
    }
    disabled.commit_disabled();

    // SAFETY: the preflight ran with interrupts excluded through this jump, so none of its live
    // page-table or task-stack facts can change first.
    unsafe { interrupt::leave_for_user(entry, stack_top) }
}

fn user_entry_preflight(entry: u64, stack_top: u64) -> Result<(), UserError> {
    if !paging::is_active() {
        return Err(UserError::Inactive);
    }
    // Recheck at the final boundary: closed syscall doors belong to this crossing, not only boot.
    if !cpu::fast_syscall_doors_are_shut() {
        return Err(UserError::FastSyscallOpen);
    }
    let protections = cpu::Protections::enabled();
    if !protections.nx {
        return Err(UserError::NoExecuteProtection);
    }
    if protections.smap && !interrupt::access_clear_is_armed() {
        return Err(UserError::AccessClearUnarmed);
    }
    if !xstate::user_state_is_isolated() {
        return Err(UserError::NoExtendedState);
    }
    let table = TABLE.lock_leaf();
    let current = current();
    let roots = table.slots[current].page_table;
    let mine = table.kernel_stack_top(current);
    let isolation = match roots {
        Some(roots) if roots.is_paired() => paired_isolation(roots, mine),
        _ => shared_isolation(mine),
    };
    drop(table);
    isolation?;
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
    // Entry never returns, so a guard live in the caller's frame could never be released.
    assert!(
        crate::sync::spinlocks_held() == 0,
        "a spin lock is held, so entering ring 3 would strand it"
    );

    Ok(())
}

fn paired_isolation(
    roots: AddressSpaceRoots,
    mine: Option<KernelStackTop<'_>>,
) -> Result<(), UserError> {
    let Some(stack) = mine else {
        return Err(UserError::NoPairedKernelIsolation);
    };
    if cpu::page_table_base() == roots.full_root() && kpti::entry_is_ready_for(roots, stack) {
        Ok(())
    } else {
        Err(UserError::NoPairedKernelIsolation)
    }
}

fn shared_isolation(mine: Option<KernelStackTop<'_>>) -> Result<(), UserError> {
    if !cpu::shared_kernel_mappings_are_safe() {
        return Err(UserError::NoSharedKernelIsolation);
    }
    let Some(stack) = mine else {
        return Err(UserError::NoKernelStack);
    };
    if super::segment::kernel_stack() == stack.address() {
        Ok(())
    } else {
        Err(UserError::NoKernelStack)
    }
}

pub fn continue_on_stack(stack: MappedStack, continuation: Entry, argument: usize) -> ! {
    // The exclusion spans the re-walk and the stack load, so the mapping cannot retire between.
    let disabled = interrupt::disable();
    assert!(
        stack.is_installed(),
        "a stack from another address space cannot be stood on"
    );
    let stack = stack
        .retain()
        .unwrap_or_else(|reason| panic!("the boot stack could not be retained: {reason}"));
    // A guard live on the abandoned stack could never be released.
    assert!(
        crate::sync::spinlocks_held() == 0,
        "a spin lock is held, so the stack being abandoned would strand it"
    );
    disabled.commit_disabled();

    // SAFETY: `stack` proves a mapped, writable, guarded top nothing else holds; `is_installed` saw
    // the CPU walk its space; `continuation` never returns, so no return address is owed.
    unsafe { load_stack(stack.top(), continuation, argument) }
}

/// # Safety
///
/// `stack_top` must be the sixteen-byte-aligned top of a mapped, writable region nothing else is
/// using, reachable in the live tables.
#[unsafe(naked)]
unsafe extern "C" fn load_stack(stack_top: u64, continuation: Entry, argument: usize) -> ! {
    ::core::arch::naked_asm!(
        // A jump supplies no return word, so subtract one for the alignment a call would create.
        "lea rsp, [rdi - 8]",
        // Nothing above this frame, so a walker following `rbp` stops before the abandoned stack.
        "xor rbp, rbp",
        "mov rdi, rdx",
        "jmp rsi",
    )
}

/// # Safety
///
/// `save` must point at a word this thread's saved pointer may be written to, and `load` must be a
/// pointer a previous call stored or [`fabricate`] wrote, on a stack nothing else is using. The
/// interrupt flag must be clear.
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

pub(crate) fn force_unlock(panic: &PanicToken) {
    TABLE.force_unlock(panic);
}
