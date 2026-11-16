use core::arch::asm;
use core::mem::offset_of;
use core::sync::atomic::{
    AtomicBool, AtomicPtr, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering,
};

use alloc::boxed::Box;
use limine::mp::Cpu;

use lock_order::HeldRanks;

use x86_paging::PAGE_SIZE;
use x86_paging::kpti::EnclavePermission::{ReadOnly, Writable};

use super::apic;
use super::cpu;
use super::interrupt::{self, ExclusionLedger, RawTrapFrame};
use super::kpti::{self, EnclaveMapping};
use super::msr;
use super::paging::{self, AddressSpace, MapError, MappedStack};
use super::segment::{self, Tables};
use super::task;
use crate::boot::{Section, Waiting};
use crate::mm::PhysicalAddress;
use crate::sync::Once;

// No physical address reaches this, so the next user root always differs from it.
const UNKNOWN_ROOT: u64 = u64::MAX;

pub(super) const GS_BASE: u32 = 0xC000_0101;

const KERNEL_GS_BASE: u32 = 0xC000_0102;

pub(super) const WINDOW: u64 = 0xffff_fe80_0000_0000;

pub(super) const WINDOW_BYTES: u64 = 1 << 39;

const STRIDE: u64 = 32 * PAGE_SIZE;

const AREAS: usize = (WINDOW_BYTES / STRIDE) as usize;

pub(super) const STACK_BYTES: u64 = 16 * 1024;

// Page 0 is the entry page and page 1 the GDT and TSS; each stack sits above an unmapped guard.
const TABLES: u64 = PAGE_SIZE;

const TRANSITION_TOP: u64 = 7 * PAGE_SIZE;

// Double fault, NMI and machine check, in IST order.
const FAULT_TOPS: [u64; 3] = [12 * PAGE_SIZE, 17 * PAGE_SIZE, 22 * PAGE_SIZE];

pub(super) const ENTRY_STACKS_START: u64 = TRANSITION_TOP - STACK_BYTES;

pub(super) const ENTRY_STACKS_END: u64 = FAULT_TOPS[2];

const _: () = assert!(
    ENTRY_STACKS_START == TABLES + 2 * PAGE_SIZE
        && FAULT_TOPS[0] - STACK_BYTES == TRANSITION_TOP + PAGE_SIZE
        && FAULT_TOPS[1] - STACK_BYTES == FAULT_TOPS[0] + PAGE_SIZE
        && FAULT_TOPS[2] - STACK_BYTES == FAULT_TOPS[1] + PAGE_SIZE
        && ENTRY_STACKS_END <= STRIDE
        && WINDOW.is_multiple_of(WINDOW_BYTES),
    "every entry stack sits above its own guard page inside one stride"
);

// The entry assembly reads these fields through `GS` before it may load the full root.
#[derive(Debug)]
#[repr(C)]
pub(super) struct EntryPage {
    this: AtomicU64,
    block: AtomicPtr<PerCpu>,
    tables: AtomicPtr<Tables>,
    pub(super) route_state: AtomicU64,
    pub(super) full_root: AtomicU64,
    pub(super) stripped_root: AtomicU64,
    pub(super) task_stack_top: AtomicU64,
    transition_stack_top: AtomicU64,
}

impl EntryPage {
    const fn new() -> Self {
        Self {
            this: AtomicU64::new(0),
            block: AtomicPtr::new(core::ptr::null_mut()),
            tables: AtomicPtr::new(core::ptr::null_mut()),
            route_state: AtomicU64::new(0),
            full_root: AtomicU64::new(0),
            stripped_root: AtomicU64::new(0),
            task_stack_top: AtomicU64::new(0),
            transition_stack_top: AtomicU64::new(0),
        }
    }

    pub(super) fn tables(&self) -> *mut Tables {
        self.tables.load(Ordering::Relaxed)
    }

    pub(super) fn transition_stack_top(&self) -> Option<u64> {
        match self.transition_stack_top.load(Ordering::Relaxed) {
            0 => None,
            top => Some(top),
        }
    }
}

pub(super) mod entry_field {
    use core::mem::offset_of;

    use super::EntryPage;

    pub(in crate::arch::x86_64) const THIS: usize = offset_of!(EntryPage, this);
    pub(in crate::arch::x86_64) const BLOCK: usize = offset_of!(EntryPage, block);
    pub(in crate::arch::x86_64) const ROUTE_STATE: usize = offset_of!(EntryPage, route_state);
    pub(in crate::arch::x86_64) const FULL_ROOT: usize = offset_of!(EntryPage, full_root);
    pub(in crate::arch::x86_64) const STRIPPED_ROOT: usize = offset_of!(EntryPage, stripped_root);
    pub(in crate::arch::x86_64) const TASK_STACK_TOP: usize = offset_of!(EntryPage, task_stack_top);
    pub(in crate::arch::x86_64) const TRANSITION_STACK_TOP: usize =
        offset_of!(EntryPage, transition_stack_top);
}

const _: () = assert!(
    offset_of!(EntryPage, this) == 0 && size_of::<EntryPage>() <= PAGE_SIZE as usize,
    "the entry page layout must match the entry assembly"
);

// The current task of a processor that runs no thread, and the owner of its own exclusion ledger.
pub const NO_THREAD: usize = usize::MAX;

// Atomics, so trap and panic paths observe interrupted updates.
#[derive(Debug)]
pub(crate) struct PerCpu {
    pub(super) ledger: ExclusionLedger,
    pub(super) apic_state: AtomicU8,
    pub(super) eoi_claimed: AtomicBool,
    index: usize,
    flushed: AtomicU64,
    loaded_root: AtomicU64,
    apic_id: AtomicU32,
    stopped: AtomicBool,
    next_online: AtomicPtr<Self>,
    held_ranks: AtomicU64,
    last_user_root: AtomicU64,
    pub(super) trap_depth: AtomicUsize,
    pub(super) live_frame: AtomicPtr<RawTrapFrame>,
    pub(super) current_task: AtomicUsize,
    // The task this processor just left, let go by the next one once its switch completes.
    pub(super) previous_task: AtomicUsize,
    // Where a threadless context's registers go when it enters its first task.
    pub(super) abandoned: AtomicU64,
    pub(super) threadless_ranks: AtomicU64,
    pub(super) exclusion_owner: AtomicUsize,
}

impl PerCpu {
    const fn new(apic_id: u32, index: usize) -> Self {
        Self {
            ledger: ExclusionLedger::new(),
            apic_state: AtomicU8::new(apic::COLD),
            eoi_claimed: AtomicBool::new(false),
            index,
            flushed: AtomicU64::new(0),
            loaded_root: AtomicU64::new(0),
            apic_id: AtomicU32::new(apic_id),
            stopped: AtomicBool::new(false),
            next_online: AtomicPtr::new(core::ptr::null_mut()),
            held_ranks: AtomicU64::new(0),
            last_user_root: AtomicU64::new(UNKNOWN_ROOT),
            trap_depth: AtomicUsize::new(0),
            live_frame: AtomicPtr::new(core::ptr::null_mut()),
            current_task: AtomicUsize::new(NO_THREAD),
            previous_task: AtomicUsize::new(NO_THREAD),
            abandoned: AtomicU64::new(0),
            threadless_ranks: AtomicU64::new(0),
            exclusion_owner: AtomicUsize::new(NO_THREAD),
        }
    }

    #[must_use = "a changed user root needs a predictor barrier"]
    pub(crate) fn enter_user_root(&self, root: PhysicalAddress) -> bool {
        self.last_user_root.swap(root.get(), Ordering::Relaxed) != root.get()
    }

    fn forget_user_root(&self, root: PhysicalAddress) {
        let _ = self.last_user_root.compare_exchange(
            root.get(),
            UNKNOWN_ROOT,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }

    pub(super) fn flushed(&self) -> u64 {
        self.flushed.load(Ordering::Acquire)
    }

    pub(super) fn record_flush(&self, generation: u64) {
        self.flushed.fetch_max(generation, Ordering::AcqRel);
    }

    // Published before the root load, so an unmap that misses it is ordered before this walk.
    pub(super) fn load_root(&self, user: Option<PhysicalAddress>) {
        self.loaded_root
            .store(user.map_or(0, PhysicalAddress::get), Ordering::SeqCst);
    }

    pub(crate) fn held_ranks(&self) -> HeldRanks {
        HeldRanks::from_bits(self.held_ranks.load(Ordering::Relaxed))
    }

    pub(crate) fn set_held_ranks(&self, held: HeldRanks) {
        self.held_ranks.store(held.bits(), Ordering::Relaxed);
    }
}

// The boot processor runs before memory exists, so its block and first entry page are static.
static BOOT_BLOCK: PerCpu = PerCpu::new(0, 0);

// Every online processor's block, newest first; a block joins once and never leaves.
static ONLINE: AtomicPtr<PerCpu> = AtomicPtr::new(core::ptr::null_mut());

// A stop answered within this many polls of every other processor lets a panic force locks.
const STOP_POLLS: u32 = 50_000_000;

static BOOT_ENTRY: EntryPage = EntryPage::new();

static BOOTSTRAP: Once = Once::new();

const FROZEN: usize = 1 << (usize::BITS - 1);

// Areas claimed so far; frozen once a stripped root copies them, since a later one would be absent.
static CLAIMED: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn enter_bootstrap() {
    assert!(BOOTSTRAP.claim(), "the boot entry page is loaded once");
    BOOT_ENTRY.this.store(
        core::ptr::from_ref(&BOOT_ENTRY).addr() as u64,
        Ordering::Relaxed,
    );
    BOOT_ENTRY.block.store(
        core::ptr::from_ref(&BOOT_BLOCK).cast_mut(),
        Ordering::Relaxed,
    );
    BOOT_ENTRY
        .tables
        .store(segment::boot_tables(), Ordering::Relaxed);
    BOOT_BLOCK
        .current_task
        .store(task::boot_record(), Ordering::Release);
    BOOT_BLOCK
        .exclusion_owner
        .store(task::boot_record(), Ordering::Relaxed);
    load_base(BOOT_ENTRY.this.load(Ordering::Relaxed));
}

fn load_base(entry: u64) {
    // SAFETY: ring 3 has not run on this processor, so the base it will see is zero, as every GDT
    // base is and `CR4.FSGSBASE` keeps it.
    unsafe { msr::write(KERNEL_GS_BASE, 0) };
    // SAFETY: `entry` is a permanently mapped entry page whose self, block and table fields are
    // written, so every `GS`-relative read from here on names this processor's state.
    unsafe { msr::write(GS_BASE, entry) };
}

pub(super) fn entry() -> &'static EntryPage {
    let this: u64;
    // SAFETY: from `enter_bootstrap` on, ring 0 Rust runs with `GS` at an entry page whose first
    // word is its own address; ring 3's base is swapped in only by assembly that calls no Rust.
    unsafe {
        asm!(
            "mov {this}, qword ptr gs:[{offset}]",
            this = out(reg) this,
            offset = const entry_field::THIS,
            options(nostack, readonly, preserves_flags),
        );
    }
    // SAFETY: that address is `BOOT_ENTRY` or a window area mapped in every root and never
    // unmapped, and every field is atomic, so a shared reference is sound on any processor.
    unsafe { &*(this as usize as *const EntryPage) }
}

pub(crate) fn current() -> &'static PerCpu {
    let block: *const PerCpu;
    // SAFETY: as in `entry`: `GS` names this processor's entry page in ring 0 Rust.
    unsafe {
        asm!(
            "mov {block}, qword ptr gs:[{offset}]",
            block = out(reg) block,
            offset = const entry_field::BLOCK,
            options(nostack, readonly, preserves_flags),
        );
    }
    // SAFETY: every entry page's block is `BOOT_BLOCK` or a block its bring-up made permanent.
    unsafe { &*block }
}

// The boot processor is index 0 before its area exists as after.
pub fn processor() -> usize {
    let this = entry().this.load(Ordering::Relaxed);
    if (WINDOW..WINDOW + WINDOW_BYTES).contains(&this) {
        ((this - WINDOW) / STRIDE) as usize
    } else {
        0
    }
}

pub(super) fn on_entry_area() -> bool {
    let this = entry().this.load(Ordering::Relaxed);
    (WINDOW..WINDOW + WINDOW_BYTES).contains(&this)
}

pub(super) fn freeze_areas() -> usize {
    let seen = CLAIMED.load(Ordering::Acquire);
    if seen & FROZEN != 0 {
        return seen & !FROZEN;
    }
    CLAIMED.fetch_or(FROZEN, Ordering::AcqRel) & !FROZEN
}

const fn area_base(index: usize) -> u64 {
    WINDOW + index as u64 * STRIDE
}

pub(super) fn area_mappings(index: usize) -> [EnclaveMapping; 5] {
    let base = area_base(index);
    let stack = |top: u64| {
        let section = Section {
            start: base + top - STACK_BYTES,
            end: base + top,
        };
        EnclaveMapping::new(section, Writable, Writable)
    };
    let pages = Section {
        start: base,
        end: base + TABLES + PAGE_SIZE,
    };
    [
        EnclaveMapping::new(pages, Writable, ReadOnly),
        stack(TRANSITION_TOP),
        stack(FAULT_TOPS[0]),
        stack(FAULT_TOPS[1]),
        stack(FAULT_TOPS[2]),
    ]
}

pub(super) fn area_guards(index: usize) -> [Section; 4] {
    let base = area_base(index);
    [TRANSITION_TOP, FAULT_TOPS[0], FAULT_TOPS[1], FAULT_TOPS[2]].map(|top| Section {
        start: base + top - STACK_BYTES - PAGE_SIZE,
        end: base + top - STACK_BYTES,
    })
}

// A claimed area whose mapping failed fails closed: no stripped root can be built without it.
fn map_area(
    space: &mut AddressSpace,
    index: usize,
    block: &'static PerCpu,
) -> Result<u64, MapError> {
    let base = area_base(index);
    if index >= AREAS
        || CLAIMED
            .compare_exchange(index, index + 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return Err(MapError::Conflict(base));
    }
    for mapping in area_mappings(index) {
        space.map_entry_area(mapping.section(), mapping.full_access())?;
    }
    // SAFETY: `map_entry_area` just mapped this page zeroed and writable in every root that shares
    // the kernel half, never to be unmapped, and all-zero is a valid `EntryPage`.
    let page = unsafe { &*(base as usize as *const EntryPage) };
    page.this.store(base, Ordering::Relaxed);
    page.block
        .store(core::ptr::from_ref(block).cast_mut(), Ordering::Relaxed);
    page.tables
        .store((base + TABLES) as usize as *mut Tables, Ordering::Relaxed);
    page.transition_stack_top
        .store(base + TRANSITION_TOP, Ordering::Release);
    Ok(base)
}

// Run on the processor that owns the area, with interrupts off; it touches no `GS` state first.
fn load_area(base: u64) {
    assert!(
        !interrupt::are_enabled(),
        "a processor changes its entry area only with interrupts excluded"
    );
    // SAFETY: `map_area` mapped and filled this page for good before handing out `base`.
    let page = unsafe { &*(base as usize as *const EntryPage) };
    let fault_stacks = FAULT_TOPS.map(|top| base + top);
    // SAFETY: the tables page was mapped zeroed and writable for good by `map_area`, and no
    // processor has loaded it, as each area is claimed once and loaded only here.
    unsafe { segment::install(page.tables(), fault_stacks) };
    load_base(base);
}

pub fn bring_up_boot_processor(space: &mut AddressSpace) -> Result<u64, MapError> {
    assert!(
        core::ptr::eq(current(), &BOOT_BLOCK),
        "only the boot processor runs on the boot block"
    );
    let base = map_area(space, 0, &BOOT_BLOCK)?;
    let disabled = interrupt::disable();
    assert_eq!(
        entry().route_state.load(Ordering::Acquire),
        kpti::ROUTE_EMPTY,
        "the boot processor moves to its entry area before any paired route exists"
    );
    load_area(base);
    join_online(&BOOT_BLOCK);
    disabled.restore();
    Ok(base)
}

fn join_online(block: &'static PerCpu) {
    let block_pointer = core::ptr::from_ref(block).cast_mut();
    let mut head = ONLINE.load(Ordering::Acquire);
    loop {
        block.next_online.store(head, Ordering::Relaxed);
        match ONLINE.compare_exchange(head, block_pointer, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return,
            Err(seen) => head = seen,
        }
    }
}

fn online() -> impl Iterator<Item = &'static PerCpu> + Clone {
    let first = ONLINE.load(Ordering::Acquire);
    // SAFETY: the list holds only `'static` blocks, each linked before it is published.
    core::iter::successors(unsafe { first.as_ref() }, |block| {
        // SAFETY: as above.
        unsafe { block.next_online.load(Ordering::Acquire).as_ref() }
    })
}

// A released root's frame can return as another space's root, which must still barrier.
pub(super) fn forget_user_root(root: PhysicalAddress) {
    current().forget_user_root(root);
    for block in online() {
        block.forget_user_root(root);
    }
}

pub fn online_count() -> usize {
    online().count()
}

fn bit(block: &PerCpu) -> u64 {
    1 << block.index
}

fn others_where(keep: impl Fn(&PerCpu) -> bool) -> u64 {
    let me = core::ptr::from_ref(current());
    online()
        .filter(|block| !core::ptr::eq(*block, me) && keep(block))
        .fold(0, |set, block| set | bit(block))
}

pub(super) fn others() -> u64 {
    others_where(|_| true)
}

pub(super) fn others_running(root: PhysicalAddress) -> u64 {
    others_where(|block| block.loaded_root.load(Ordering::SeqCst) == root.get())
}

// The processors in `set` that have not yet flushed for `generation`.
pub(super) fn behind(set: u64, generation: u64) -> u64 {
    online()
        .filter(|block| set & bit(block) != 0 && block.flushed() < generation)
        .fold(0, |lagging, block| lagging | bit(block))
}

// Every online processor, by the bit its index names.
pub fn online_set() -> u64 {
    online().fold(0, |set, block| set | bit(block))
}

pub fn send_reschedule(processor: usize) {
    if let Some(block) = online().find(|block| block.index == processor) {
        apic::send_reschedule(block.apic_id.load(Ordering::Relaxed));
    }
}

pub(super) fn send_shootdown(set: u64) {
    for block in online().filter(|block| set & bit(block) != 0) {
        apic::send_shootdown(block.apic_id.load(Ordering::Relaxed));
    }
}

pub type ApplicationEntry = fn(usize) -> !;

// Read by the started processor only, after the bootloader hands it `extra`.
#[derive(Debug)]
struct Launch {
    root: PhysicalAddress,
    page_attributes: Option<u64>,
    area: u64,
    stack_top: u64,
    entry: ApplicationEntry,
    index: usize,
}

// A started processor's block, admitted to the online set only once its checks pass.
#[derive(Debug)]
pub struct Joining {
    block: &'static PerCpu,
    index: usize,
}

impl Joining {
    pub const fn index(&self) -> usize {
        self.index
    }
}

pub fn start_processor(
    space: &mut AddressSpace,
    waiting: Waiting,
    stack: MappedStack,
    entry: ApplicationEntry,
) -> Result<Joining, MapError> {
    if !paging::is_active() {
        return Err(MapError::Inactive(cpu::page_table_base()));
    }
    let index = CLAIMED.load(Ordering::Acquire) & !FROZEN;
    // Flush tokens name processors by one bit of a word each.
    if index >= u64::BITS as usize {
        return Err(MapError::Conflict(area_base(index)));
    }
    let block: &'static PerCpu = Box::leak(Box::new(PerCpu::new(waiting.apic_id(), index)));
    let area = map_area(space, index, block)?;
    let stack = stack.retain()?;
    let launch = Box::leak(Box::new(Launch {
        root: paging::active_root(),
        page_attributes: cpu::page_attributes(),
        area,
        stack_top: stack.top(),
        entry,
        index,
    }));
    waiting.send(core::ptr::from_ref(launch).addr() as u64, start_here);
    Ok(Joining { block, index })
}

// Refused once a user root exists, since the stop NMI would then reach a processor it never saw.
#[must_use = "a refused admission leaves the processor out of the online set"]
pub fn admit(joining: Joining) -> bool {
    if CLAIMED.load(Ordering::Acquire) & FROZEN != 0 {
        return false;
    }
    join_online(joining.block);
    true
}

// Entered by the bootloader on the new processor, on its stack and tables, interrupts off.
unsafe extern "C" fn start_here(cpu: &Cpu) -> ! {
    let launch = cpu.extra.load(Ordering::Acquire) as usize as *const Launch;
    // SAFETY: `start_processor` leaked the launch record and stored its address before the jump.
    let launch = unsafe { &*launch };
    if let Some(attributes) = launch.page_attributes {
        // SAFETY: the boot processor's own value, and the root load below follows at once.
        unsafe { cpu::set_page_attributes(attributes) };
    }
    // SAFETY: the active kernel root maps this code, the bootloader's stack in the direct map, the
    // launch record on the heap and every entry area, at the addresses the bootloader used.
    unsafe { cpu::set_page_table_base(launch.root) };
    load_area(launch.area);
    interrupt::load_table();
    // SAFETY: `start_processor` retained the stack, mapped and guarded, for this processor alone.
    unsafe { run_on(launch.stack_top, core::ptr::from_ref(launch).addr()) }
}

/// # Safety
///
/// `stack_top` must be the sixteen-byte-aligned top of a mapped, writable stack nothing else uses,
/// and `launch` the address of a live `Launch`.
#[unsafe(naked)]
unsafe extern "C" fn run_on(stack_top: u64, launch: usize) -> ! {
    ::core::arch::naked_asm!(
        "lea rsp, [rdi - 8]",
        "xor ebp, ebp",
        "mov rdi, rsi",
        "jmp {joined}",
        joined = sym joined,
    )
}

extern "C" fn joined(launch: usize) -> ! {
    // SAFETY: `run_on` passes the address `start_here` read, of a leaked launch record.
    let launch = unsafe { &*(launch as *const Launch) };
    (launch.entry)(launch.index)
}

pub(super) fn acknowledge_stop() -> ! {
    current().stopped.store(true, Ordering::Release);
    cpu::halt_forever()
}

#[must_use = "an unanswered stop leaves locks another processor may hold"]
pub(crate) fn stop_others() -> bool {
    let me = core::ptr::from_ref(current());
    let others = online().filter(move |block| !core::ptr::eq(*block, me));
    for block in others.clone() {
        apic::send_stop(block.apic_id.load(Ordering::Relaxed));
    }
    for _ in 0..STOP_POLLS {
        if others
            .clone()
            .all(|block| block.stopped.load(Ordering::Acquire))
        {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}
