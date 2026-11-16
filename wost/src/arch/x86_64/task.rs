use core::cell::UnsafeCell;
use core::fmt;
use core::marker::PhantomData;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use x86_paging::{CanonicalAddress, Flags, PAGE_SIZE};

use super::cpu;
use super::interrupt::{self, ExclusionLedger, TrapFrame};
use super::kpti;
use super::msr;
use super::paging::{self, AddressSpaceRoots, Binding, MappedStack, TaskStack};
use super::percpu;
use super::xstate;
use crate::mm::{self, pool::ArenaFrame};
use crate::sync::ThreadContext;

pub const STACK_FRAMES: usize = 8;

pub type Entry = fn(&mut ThreadContext, usize) -> !;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum TaskError {
    Boot,
    Started,
    OnProcessor,
    Uninterruptible,
    NotInTrap,
    InTrap,
    ForeignFrame,
    UserFrame,
}

impl fmt::Display for TaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match *self {
            Self::Boot => "the boot task has no arena record",
            Self::Started => "the task has already run",
            Self::OnProcessor => "the task is on a processor",
            Self::Uninterruptible => "the interrupted code holds a critical section",
            Self::NotInTrap => "not inside a trap handler",
            Self::InTrap => "inside a trap handler",
            Self::ForeignFrame => "not the frame the live handler was entered with",
            Self::UserFrame => "a ring 3 frame switches only through its handoff",
        })
    }
}

impl core::error::Error for TaskError {}

const FRESH: u8 = 0;
const STARTED: u8 = 1;
const DEAD: u8 = 2;

// One thread's switch state, at the start of a frame its task owns, or the boot thread's static.
#[repr(C, align(64))]
pub(super) struct Record {
    extended: UnsafeCell<[u8; xstate::CEILING]>,
    saved: UnsafeCell<u64>,
    thread_base: UnsafeCell<u64>,
    page_table: UnsafeCell<Option<AddressSpaceRoots>>,
    entry: UnsafeCell<Option<(Entry, usize)>>,
    starts_interruptible: AtomicBool,
    // Held by the processor running the task and by whoever changes it while it runs nowhere.
    on_cpu: AtomicBool,
    state: AtomicU8,
    named: AtomicBool,
    tag: [AtomicU64; 2],
    pub(super) ranks: AtomicU64,
    pub(super) ledger: ExclusionLedger,
    stack_top: Option<u64>,
}

const _: () = assert!(
    size_of::<Record>() <= PAGE_SIZE as usize,
    "a task record must fit its frame"
);

// SAFETY: the `UnsafeCell` fields are touched only by the holder of `on_cpu`, whose Acquire take
// and Release clear order every access, or by the boot thread on its own static record.
unsafe impl Sync for Record {}

impl Record {
    const fn new(
        stack_top: Option<u64>,
        entry: Option<(Entry, usize)>,
        interruptible: bool,
    ) -> Self {
        Self {
            extended: UnsafeCell::new([0; xstate::CEILING]),
            saved: UnsafeCell::new(0),
            thread_base: UnsafeCell::new(0),
            page_table: UnsafeCell::new(None),
            entry: UnsafeCell::new(entry),
            starts_interruptible: AtomicBool::new(interruptible),
            on_cpu: AtomicBool::new(false),
            state: AtomicU8::new(FRESH),
            named: AtomicBool::new(false),
            tag: [AtomicU64::new(0), AtomicU64::new(0)],
            ranks: AtomicU64::new(0),
            ledger: ExclusionLedger::new(),
            stack_top,
        }
    }

    fn address(&self) -> usize {
        core::ptr::from_ref(self).addr()
    }

    fn page_table(&self) -> Option<AddressSpaceRoots> {
        // SAFETY: the caller holds `on_cpu` or is this record's running thread.
        unsafe { *self.page_table.get() }
    }

    fn kernel_stack_top(&self) -> Option<KernelStackTop<'_>> {
        self.stack_top.map(|address| KernelStackTop {
            address,
            record: PhantomData,
        })
    }

    // Taken by compare-exchange, so a racing claim or change finds it held and refuses.
    fn hold(&self) -> bool {
        self.on_cpu
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    fn let_go(&self) {
        self.on_cpu.store(false, Ordering::Release);
    }
}

// The running boot thread starts on this processor, so the switch and the boot path own it.
static BOOT_RECORD: Record = Record {
    on_cpu: AtomicBool::new(true),
    state: AtomicU8::new(STARTED),
    ..Record::new(None, None, false)
};

static BOOT_TAKEN: AtomicBool = AtomicBool::new(false);

static PREDICTOR_BARRIER: AtomicBool = AtomicBool::new(false);

pub(super) fn boot_record() -> usize {
    BOOT_RECORD.address()
}

// Every processor shares the boot processor's posture, so one answer serves all.
pub fn init() {
    PREDICTOR_BARRIER.store(cpu::has_predictor_barrier(), Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct KernelStackTop<'record> {
    address: u64,
    record: PhantomData<&'record Record>,
}

impl KernelStackTop<'_> {
    pub(crate) const fn address(self) -> u64 {
        self.address
    }
}

// A thread's record and kernel stack; it outlives every processor that runs it.
#[derive(Debug)]
pub struct Task {
    owned: Option<Owned>,
}

#[derive(Debug)]
struct Owned {
    record: ArenaFrame,
    stack: TaskStack,
}

impl Task {
    #[must_use]
    pub fn boot() -> Option<Self> {
        (!BOOT_TAKEN.swap(true, Ordering::AcqRel)).then_some(Self { owned: None })
    }

    // The stack is mapped and the record frame is this task's alone, so the writes reach no one.
    pub(super) fn fabricate(
        record: ArenaFrame,
        mut stack: TaskStack,
        entry: Entry,
        argument: usize,
        starts_interruptible: bool,
    ) -> Self {
        let top = stack.top();
        let words: [u64; FRAME_WORDS] = [0, 0, 0, 0, 0, 0, thread_start as *const () as u64, 0];
        let mut bytes = [0; FRAME_BYTES as usize];
        for (chunk, word) in bytes.chunks_exact_mut(8).zip(words) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        assert!(
            stack
                .top_frame()
                .write_at(PAGE_SIZE as usize - FRAME_BYTES as usize, &bytes),
            "a task stack's top frame refused its switch frame"
        );
        let Some(pointer) = mm::direct_pointer(record.address(), PAGE_SIZE) else {
            panic!("an arena frame lost its direct-map address");
        };
        let pointer = pointer.cast::<Record>();
        // SAFETY: the frame is a zeroed page this task owns and nothing else references, its
        // direct-map address is page aligned, and `Record` fits in it.
        unsafe {
            pointer.write(Record::new(
                Some(top),
                Some((entry, argument)),
                starts_interruptible,
            ));
        }
        let task = Self {
            owned: Some(Owned { record, stack }),
        };
        let record = task.record();
        // SAFETY: the area is the record's 64-byte-aligned `CEILING` bytes, held by no one else.
        unsafe { xstate::write_initial(record.extended.get().cast()) };
        // SAFETY: as above; no switch reads the word before the task is first claimed.
        unsafe { *record.saved.get() = top - FRAME_BYTES };
        task
    }

    pub(super) fn record(&self) -> &Record {
        let Some(owned) = &self.owned else {
            return &BOOT_RECORD;
        };
        let Some(pointer) = mm::direct_pointer(owned.record.address(), PAGE_SIZE) else {
            panic!("an arena frame lost its direct-map address");
        };
        // SAFETY: `fabricate` wrote a `Record` at the start of this frame, which the task owns
        // for its whole life and which nothing else writes.
        unsafe { &*pointer.cast::<Record>() }
    }

    // On a processor now, or until the thread that follows it there has run.
    pub fn is_on_cpu(&self) -> bool {
        self.record().on_cpu.load(Ordering::Acquire)
    }

    pub fn is_dead(&self) -> bool {
        self.record().state.load(Ordering::Acquire) == DEAD
    }

    pub fn name(&self, tag: [u64; 2]) {
        let record = self.record();
        record.tag[0].store(tag[0], Ordering::Relaxed);
        record.tag[1].store(tag[1], Ordering::Relaxed);
        record.named.store(true, Ordering::Release);
    }

    // Exactly one claim holds a task, and only a live one off every processor.
    #[must_use = "a dropped claim lets the task go again"]
    pub fn claim(&self) -> Option<Claimed> {
        let record = self.record();
        if !record.hold() {
            return None;
        }
        if record.state.load(Ordering::Acquire) == DEAD {
            record.let_go();
            return None;
        }
        Some(Claimed {
            record: NonNull::from(record),
        })
    }

    pub(super) fn take_owned(&mut self) -> Option<(ArenaFrame, TaskStack)> {
        self.owned.take().map(|owned| (owned.record, owned.stack))
    }

    // Held for good on success: the record is about to go with its frame.
    pub(super) fn hold_for_release(&self) -> Result<(), TaskError> {
        if self.owned.is_none() {
            return Err(TaskError::Boot);
        }
        let record = self.record();
        if !record.hold() {
            return Err(TaskError::OnProcessor);
        }
        if record.state.load(Ordering::Acquire) == STARTED || record.page_table().is_some() {
            record.let_go();
            return Err(TaskError::Started);
        }
        assert!(
            interrupt::context_is_clear(&record.ledger),
            "a released task retains interrupt-exclusion ownership"
        );
        Ok(())
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        assert!(
            self.owned.is_none(),
            "a task leaves only by unmapping its stack"
        );
    }
}

// Proof that this processor holds the task; the record lives while it does.
#[derive(Debug)]
pub struct Claimed {
    record: NonNull<Record>,
}

impl Claimed {
    fn record(&self) -> &Record {
        // SAFETY: the claim holds `on_cpu`, and no task is released while that is held.
        unsafe { self.record.as_ref() }
    }

    fn into_record(self) -> NonNull<Record> {
        let record = self.record;
        core::mem::forget(self);
        record
    }
}

impl Drop for Claimed {
    fn drop(&mut self) {
        self.record().let_go();
    }
}

pub fn current_tag() -> Option<[u64; 2]> {
    let record = current_record()?;
    record.named.load(Ordering::Acquire).then(|| {
        [
            record.tag[0].load(Ordering::Relaxed),
            record.tag[1].load(Ordering::Relaxed),
        ]
    })
}

pub fn has_thread() -> bool {
    percpu::current().current_task.load(Ordering::Acquire) != percpu::NO_THREAD
}

pub(super) fn current_record() -> Option<&'static Record> {
    let address = percpu::current().current_task.load(Ordering::Acquire);
    if address == percpu::NO_THREAD {
        return None;
    }
    // SAFETY: a processor's current task is claimed by it, so its record lives at least until a
    // later task here lets it go, and the caller runs on this task.
    Some(unsafe { &*(address as *const Record) })
}

// Only for this processor's exclusion owner, which `interrupt` checks before it asks.
pub(super) fn owner_ledger(owner: usize) -> &'static ExclusionLedger {
    let pointer = owner as *const Record;
    // SAFETY: a processor's exclusion owner is a record it holds: its running task's, or across a
    // switch the one it leaves or the one it claimed, each kept alive by its `on_cpu`.
    unsafe { &(*pointer).ledger }
}

// A processor running no thread keeps its thread-lock ranks in its own block.
pub(crate) fn current_ranks() -> &'static AtomicU64 {
    current_record().map_or(&percpu::current().threadless_ranks, |record| &record.ranks)
}

fn running() -> &'static Record {
    current_record().unwrap_or_else(|| panic!("this processor runs no thread"))
}

pub fn switch_to(next: Claimed) -> Result<(), TaskError> {
    if interrupt::in_trap() {
        return Err(TaskError::InTrap);
    }
    let saved = interrupt::disable();
    let (save, load) = prepare(next, STARTED);
    // SAFETY: `prepare` took `save` from the running record and `load` from the claimed one,
    // written only by `fabricate` or a switch; IF is clear, so nothing runs in between.
    unsafe { switch(save, load) };
    finish_switch();
    saved.restore();
    Ok(())
}

pub fn switch_from_trap(next: Claimed, frame: &TrapFrame) -> Result<(), TaskError> {
    if !interrupt::in_trap() {
        return Err(TaskError::NotInTrap);
    }
    if !interrupt::is_live_frame(frame) {
        return Err(TaskError::ForeignFrame);
    }
    if frame.is_from_user() {
        return Err(TaskError::UserFrame);
    }
    if !frame.was_interruptible() {
        return Err(TaskError::Uninterruptible);
    }

    let saved = interrupt::disable();
    let suspended = interrupt::suspend_trap();
    let (save, load) = prepare(next, STARTED);
    // SAFETY: as in `switch_to`; the handler's frame lies below the saved pointer, and only this
    // thread will unwind it.
    unsafe { switch(save, load) };
    finish_switch();
    interrupt::resume_trap(suspended);
    saved.restore();
    Ok(())
}

pub fn exit_to(next: Claimed) -> TaskError {
    if interrupt::in_trap() {
        return TaskError::InTrap;
    }
    let saved = interrupt::disable();
    saved.commit_context_disabled();
    let (save, load) = prepare(next, DEAD);
    // SAFETY: as in `switch_to`; the record left is now dead, which every claim refuses, so the
    // save is never loaded.
    unsafe { switch(save, load) };
    unreachable!("a thread marked dead was resumed")
}

// From a processor that runs no thread yet; its bring-up context is never resumed.
pub fn enter(next: Claimed) -> ! {
    assert!(!has_thread(), "only a threadless processor enters a task");
    let saved = interrupt::disable();
    saved.commit_context_disabled();
    let (save, load) = prepare(next, DEAD);
    // SAFETY: as in `switch_to`; `save` is this processor's scratch word, which nothing loads.
    unsafe { switch(save, load) };
    unreachable!("an abandoned bring-up context was resumed")
}

// The arriving thread lets the one it replaced go, after that one's last save.
fn finish_switch() {
    let previous = percpu::current()
        .previous_task
        .swap(percpu::NO_THREAD, Ordering::Relaxed);
    if previous == percpu::NO_THREAD {
        return;
    }
    // SAFETY: `prepare` stored the left task's record, which its `on_cpu` keeps alive until this
    // store, the last access this processor makes to it.
    let previous = unsafe { &*(previous as *const Record) };
    previous.let_go();
}

fn prepare(next: Claimed, leaving: u8) -> (*mut u64, u64) {
    assert!(
        !crate::sync::holds_spin_lock(),
        "a spin lock is held, so the thread left behind could never release it"
    );
    let block = percpu::current();
    let current = current_record();
    let next = next.into_record();
    // SAFETY: the claim held `on_cpu`, which now passes to this processor's current task.
    let next = unsafe { next.as_ref() };

    // Revoke the old route before changing the current task, so an NMI here fails closed.
    kpti::begin_route_update();
    if let Some(current) = current {
        current.state.store(leaving, Ordering::Release);
    }
    next.state.store(STARTED, Ordering::Release);
    block.previous_task.store(
        current.map_or(percpu::NO_THREAD, Record::address),
        Ordering::Relaxed,
    );
    block.current_task.store(next.address(), Ordering::Release);

    // Root, route and privilege stack move together before the target can run.
    let roots = next.page_table();
    let wanted = roots.map_or_else(paging::active_root, AddressSpaceRoots::full_root);
    let paired = roots.is_some_and(AddressSpaceRoots::is_paired);
    if paired {
        let Some(landing) = next.kernel_stack_top() else {
            panic!("a paired task has no kernel stack");
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
        if let Some(landing) = next.kernel_stack_top() {
            super::segment::write_kernel_stack(landing);
        }
        kpti::clear_route();
    }
    block.load_root(roots.map(AddressSpaceRoots::full_root));
    if wanted != cpu::page_table_base() {
        // SAFETY: `bind_address_space` matched the higher half to the kernel root, frozen at
        // activation so device maps stay under shared entries; IF is clear.
        unsafe { cpu::set_page_table_base(wanted) };
    }
    // Kernel-only tasks leave the predictor's user history as it was.
    if let Some(roots) = roots
        && block.enter_user_root(roots.full_root())
        && PREDICTOR_BARRIER.load(Ordering::Relaxed)
    {
        cpu::barrier_predictions();
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
        if let Some(current) = current {
            // SAFETY: the record's area is 64-byte aligned `CEILING` bytes, `init` refused any
            // `bytes()` above `CEILING`, and this processor holds the running record.
            unsafe { xstate::save(current.extended.get().cast()) };
        }
        // SAFETY: as above for the claimed record, whose area holds the initial state or a save.
        unsafe { xstate::restore(next.extended.get().cast_const().cast()) };
    }

    switch_thread_base(current, next);
    interrupt::switch_context(next.address());

    let save = current.map_or_else(|| block.abandoned.as_ptr(), |current| current.saved.get());
    // SAFETY: this processor holds the claimed record, and `fabricate` or a switch wrote the word.
    let load = unsafe { *next.saved.get() };
    (save, load)
}

// Ring 3 can change its own FS base, so the switch reads it rather than trusting the record.
fn switch_thread_base(current: Option<&Record>, next: &Record) {
    let Some(leaving) = CanonicalAddress::new(read_thread_base()) else {
        panic!("the processor retained a non-canonical FS base");
    };
    if let Some(current) = current {
        // SAFETY: this processor holds the running record.
        unsafe { *current.thread_base.get() = leaving.get() };
    }
    super::segment::reset_data_selectors();
    // SAFETY: this processor holds the claimed record.
    let arriving = unsafe { *next.thread_base.get() };
    let Some(arriving) = CanonicalAddress::new(arriving) else {
        panic!("a task recorded a non-canonical FS base");
    };
    write_thread_base(arriving);
}

pub fn bind_address_space(task: &Task, binding: Binding) -> Result<Bound, (TaskError, Binding)> {
    let roots = binding.roots();
    if task.owned.is_none() {
        return Err((TaskError::Boot, binding));
    }
    let record = task.record();
    if !record.hold() {
        return Err((TaskError::OnProcessor, binding));
    }
    if record.state.load(Ordering::Acquire) != FRESH {
        record.let_go();
        return Err((TaskError::Started, binding));
    }
    // SAFETY: the hold excludes every claim and change until it is let go.
    unsafe { *record.page_table.get() = Some(roots) };
    record.let_go();
    Ok(Bound {
        roots,
        record: record.address(),
    })
}

// A task that never ran is marked dead, so no claim can start it.
pub fn retire(task: &Task) -> Result<(), TaskError> {
    if task.owned.is_none() {
        return Err(TaskError::Boot);
    }
    let record = task.record();
    if !record.hold() {
        return Err(TaskError::OnProcessor);
    }
    let outcome = if record.state.load(Ordering::Acquire) == FRESH {
        record.state.store(DEAD, Ordering::Release);
        Ok(())
    } else {
        Err(TaskError::Started)
    };
    record.let_go();
    outcome
}

#[derive(Debug)]
pub struct Bound {
    roots: AddressSpaceRoots,
    record: usize,
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

// Refused until the bound task is dead and off every processor.
pub fn release_address_space(bound: Bound) -> Result<Released, Bound> {
    // SAFETY: a task's stack is unmapped, and its record freed, only once its page table is
    // unbound, which only this function does, consuming the one `Bound`.
    let record = unsafe { &*(bound.record as *const Record) };
    if !record.hold() {
        return Err(bound);
    }
    if record.state.load(Ordering::Acquire) != DEAD {
        record.let_go();
        return Err(bound);
    }
    assert!(
        record.page_table() == Some(bound.roots),
        "a bound task lost its page table"
    );
    // SAFETY: the hold excludes every claim and change.
    unsafe { *record.page_table.get() = None };
    percpu::forget_user_root(bound.roots.full_root());
    record.let_go();
    Ok(Released { roots: bound.roots })
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

// A switch or a stack move arrives with interrupts clear, so the creator's flag is restored here.
extern "C" fn thread_start() -> ! {
    finish_switch();
    let record = running();
    // SAFETY: this processor holds the running record.
    let entry = unsafe { *record.entry.get() };
    let Some((entry, argument)) = entry else {
        panic!("a task started with no entry point");
    };
    if record.starts_interruptible.load(Ordering::Relaxed) {
        interrupt::enable();
    }
    entry(&mut ThreadContext::mint(), argument)
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
    NoEntryArea,
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
            Self::NoEntryArea => formatter.write_str("this processor has no entry area"),
        }
    }
}

impl core::error::Error for UserError {}

pub fn enter_user(
    context: &mut ThreadContext,
    entry: u64,
    stack_top: u64,
    argument: u64,
) -> UserError {
    let _ = context;
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
    unsafe { interrupt::leave_for_user(entry, stack_top, argument) }
}

fn user_entry_preflight(entry: u64, stack_top: u64) -> Result<(), UserError> {
    if !paging::is_active() {
        return Err(UserError::Inactive);
    }
    if !percpu::on_entry_area() {
        return Err(UserError::NoEntryArea);
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
    let record = running();
    let mine = record.kernel_stack_top();
    match record.page_table() {
        Some(roots) if roots.is_paired() => paired_isolation(roots, mine),
        _ => shared_isolation(mine),
    }?;
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
        !crate::sync::holds_spin_lock(),
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

// Consumes the token on the abandoned stack; `thread_start` mints the next one on the new stack.
pub fn continue_on_stack(
    _context: ThreadContext,
    stack: MappedStack,
    continuation: Entry,
    argument: usize,
) -> ! {
    // The exclusion spans the re-walk and the stack load, so the mapping cannot retire between.
    let disabled = interrupt::disable();
    assert!(
        stack.is_installed(),
        "a stack from another address space cannot be stood on"
    );
    let stack = stack
        .retain()
        .unwrap_or_else(|reason| panic!("the boot stack could not be retained: {reason}"));
    let record = running();
    // SAFETY: this is the running record, which only its own thread changes.
    unsafe { *record.entry.get() = Some((continuation, argument)) };
    record.starts_interruptible.store(false, Ordering::Relaxed);
    // A guard live on the abandoned stack could never be released.
    assert!(
        !crate::sync::holds_spin_lock(),
        "a spin lock is held, so the stack being abandoned would strand it"
    );
    disabled.commit_disabled();

    // SAFETY: `stack` proves a mapped, writable, guarded top nothing else holds; `is_installed` saw
    // the CPU walk its space; `thread_start` never returns, so no return address is owed.
    unsafe { load_stack(stack.top()) }
}

/// # Safety
///
/// `stack_top` must be the sixteen-byte-aligned top of a mapped, writable region nothing else is
/// using, reachable in the live tables.
#[unsafe(naked)]
unsafe extern "C" fn load_stack(stack_top: u64) -> ! {
    ::core::arch::naked_asm!(
        // A jump supplies no return word, so subtract one for the alignment a call would create.
        "lea rsp, [rdi - 8]",
        // Nothing above this frame, so a walker following `rbp` stops before the abandoned stack.
        "xor rbp, rbp",
        "jmp {start}",
        start = sym thread_start,
    )
}

/// # Safety
///
/// `save` must point at a word this thread's saved pointer may be written to, and `load` must be a
/// pointer a previous call stored or [`Task::fabricate`] wrote, on a stack nothing else is using.
/// The interrupt flag must be clear.
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
