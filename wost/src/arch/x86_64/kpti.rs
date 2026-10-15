//! The linker-owned entry enclave and its fail-closed CPU-local route.
//!
//! D-407's stripped root may map only the ranges described by
//! [`EntryEnclave`]. A published route is authority only while its complete
//! root pair and guarded entry stacks match the state installed on this CPU.

use core::mem::{offset_of, size_of};
#[cfg(feature = "selftest")]
use core::sync::atomic::AtomicBool;
use core::sync::atomic::{AtomicU64, Ordering};

use super::cpu;
use super::interrupt;
use super::paging::{AddressSpaceRoots, KernelStackTop};
use super::segment;
use crate::boot::{self, Section};
use crate::cell::HardwareCell;

const PAGE_BYTES: u64 = 4096;
pub(super) const STACK_BYTES: usize = 16 * 1024;
const ROUTE_EMPTY: u64 = 0;
const ROUTE_PUBLISHED: u64 = 1;
const ROUTE_UPDATING: u64 = 2;

const ENTRY_ROOT_SWITCH_IMPLEMENTED: bool = true;

/// Permissions one enclave range has in one member of a root pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Access {
    /// Read-only and executable.
    Executable,
    /// Read-only and non-executable.
    ReadOnly,
    /// Writable and non-executable.
    Writable,
}

/// One page-exact range and its full/stripped-root permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EnclaveMapping {
    section: Section,
    full: Access,
    stripped: Access,
}

impl EnclaveMapping {
    /// The linker range this mapping covers.
    pub(super) const fn section(self) -> Section {
        self.section
    }

    /// The permission the full root must install.
    pub(super) const fn full_access(self) -> Access {
        self.full
    }

    /// The permission the stripped root must install.
    pub(super) const fn stripped_access(self) -> Access {
        self.stripped
    }
}

/// Validated, unforgeable linker geometry for D-407's entry enclave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EntryEnclave {
    mappings: [EnclaveMapping; 6],
    guards: [Section; 2],
}

impl EntryEnclave {
    /// Every range both roots map, in virtual-address order.
    pub(super) const fn mappings(self) -> [EnclaveMapping; 6] {
        self.mappings
    }

    /// The lower stack guards both roots must leave absent.
    pub(super) const fn guards(self) -> [Section; 2] {
        self.guards
    }
}

/// Validate the linker-provided enclave geometry and mint its opaque evidence.
pub(super) fn entry_enclave() -> Option<EntryEnclave> {
    let raw = boot::sections().entry_enclave;
    let exact = [
        raw.text,
        raw.rodata,
        raw.tables,
        raw.route,
        raw.transition_guard,
        raw.transition_stack,
        raw.ist_guard,
        raw.ist_stack,
        raw.data,
    ]
    .into_iter()
    .all(page_exact);
    let ordered = raw.text.end <= raw.rodata.start
        && raw.rodata.end <= raw.tables.start
        && raw.tables.end == raw.data.start
        && raw.data.start == raw.route.start
        && raw.route.end == raw.transition_guard.start
        && raw.transition_guard.end == raw.transition_stack.start
        && raw.transition_stack.end == raw.ist_guard.start
        && raw.ist_guard.end == raw.ist_stack.start
        && raw.ist_stack.end == raw.data.end;
    let fixed = raw.route.mapped_len() == PAGE_BYTES
        && raw.transition_guard.mapped_len() == PAGE_BYTES
        && raw.transition_stack.mapped_len() == STACK_BYTES as u64
        && raw.ist_guard.mapped_len() == PAGE_BYTES
        && raw.ist_stack.mapped_len() == STACK_BYTES as u64;
    if !exact || !ordered || !fixed {
        return None;
    }

    Some(EntryEnclave {
        mappings: [
            mapping(raw.text, Access::Executable, Access::Executable),
            mapping(raw.rodata, Access::ReadOnly, Access::ReadOnly),
            mapping(raw.tables, Access::Writable, Access::ReadOnly),
            mapping(raw.route, Access::Writable, Access::ReadOnly),
            mapping(raw.transition_stack, Access::Writable, Access::Writable),
            mapping(raw.ist_stack, Access::Writable, Access::Writable),
        ],
        guards: [raw.transition_guard, raw.ist_guard],
    })
}

const fn mapping(section: Section, full: Access, stripped: Access) -> EnclaveMapping {
    EnclaveMapping {
        section,
        full,
        stripped,
    }
}

const fn page_exact(section: Section) -> bool {
    section.start < section.end
        && section.start.is_multiple_of(PAGE_BYTES)
        && section.end.is_multiple_of(PAGE_BYTES)
}

const fn route_roots_are_valid(full: u64, stripped: u64) -> bool {
    full != stripped && full.is_multiple_of(PAGE_BYTES) && stripped.is_multiple_of(PAGE_BYTES)
}

const _: () = assert!(route_roots_are_valid(0, PAGE_BYTES));

/// Why a paired route could not be published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RouteError {
    /// The address-space identity has no distinct stripped root.
    SharedRoot,
    /// A root is not a page-exact physical address.
    InvalidRoot,
    /// The ordinary task stack is not a canonical aligned kernel address.
    InvalidTaskStack,
    /// PCID would let a root transition preserve tagged translations.
    PcidEnabled,
    /// The linker geometry is not the exact enclave contract.
    InvalidEnclave,
}

/// Fixed-offset state the assembly transition will consume.
#[repr(C)]
struct RoutingPage {
    state: AtomicU64,
    full_root: AtomicU64,
    stripped_root: AtomicU64,
    task_stack_top: AtomicU64,
    transition_stack_top: AtomicU64,
    interrupt_stack_top: AtomicU64,
    enclave_text_start: AtomicU64,
    enclave_text_end: AtomicU64,
    abi_version: AtomicU64,
    stripped_entries: AtomicU64,
    prepared_returns: AtomicU64,
}

impl RoutingPage {
    const fn new() -> Self {
        Self {
            state: AtomicU64::new(ROUTE_EMPTY),
            full_root: AtomicU64::new(0),
            stripped_root: AtomicU64::new(0),
            task_stack_top: AtomicU64::new(0),
            transition_stack_top: AtomicU64::new(0),
            interrupt_stack_top: AtomicU64::new(0),
            enclave_text_start: AtomicU64::new(0),
            enclave_text_end: AtomicU64::new(0),
            abi_version: AtomicU64::new(1),
            stripped_entries: AtomicU64::new(0),
            prepared_returns: AtomicU64::new(0),
        }
    }
}

const _: () = {
    assert!(offset_of!(RoutingPage, state) == 0);
    assert!(offset_of!(RoutingPage, full_root) == 8);
    assert!(offset_of!(RoutingPage, stripped_root) == 16);
    assert!(offset_of!(RoutingPage, task_stack_top) == 24);
    assert!(offset_of!(RoutingPage, transition_stack_top) == 32);
    assert!(offset_of!(RoutingPage, interrupt_stack_top) == 40);
    assert!(offset_of!(RoutingPage, enclave_text_start) == 48);
    assert!(offset_of!(RoutingPage, enclave_text_end) == 56);
    assert!(offset_of!(RoutingPage, abi_version) == 64);
    assert!(offset_of!(RoutingPage, stripped_entries) == 72);
    assert!(offset_of!(RoutingPage, prepared_returns) == 80);
    assert!(size_of::<RoutingPage>() == 88);
};

pub(super) const ROUTE_STATE_OFFSET: usize = offset_of!(RoutingPage, state);
pub(super) const ROUTE_FULL_ROOT_OFFSET: usize = offset_of!(RoutingPage, full_root);
pub(super) const ROUTE_STRIPPED_ROOT_OFFSET: usize = offset_of!(RoutingPage, stripped_root);
pub(super) const ROUTE_TASK_STACK_TOP_OFFSET: usize = offset_of!(RoutingPage, task_stack_top);
pub(super) const ROUTE_TRANSITION_STACK_TOP_OFFSET: usize =
    offset_of!(RoutingPage, transition_stack_top);
pub(super) const ROUTE_INTERRUPT_STACK_TOP_OFFSET: usize =
    offset_of!(RoutingPage, interrupt_stack_top);
pub(super) const ROUTE_STRIPPED_ENTRIES_OFFSET: usize = offset_of!(RoutingPage, stripped_entries);
pub(super) const ROUTE_PREPARED_RETURNS_OFFSET: usize = offset_of!(RoutingPage, prepared_returns);
pub(super) const RECORD_TRANSITION_EVIDENCE: bool = cfg!(feature = "selftest");

#[unsafe(no_mangle)]
#[unsafe(link_section = ".kpti.state.route")]
static WOST_KPTI_ROUTE: RoutingPage = RoutingPage::new();

/// Public constants which bind the linker object to the assembly ABI.
#[used]
#[unsafe(link_section = ".kpti.rodata.geometry")]
static ENTRY_ABI: [u64; 4] = [1, PAGE_BYTES, STACK_BYTES as u64, 6];

#[repr(C, align(4096))]
struct TransitionStack([u8; STACK_BYTES]);

#[unsafe(link_section = ".kpti.state.transition_stack")]
static TRANSITION_STACK: HardwareCell<TransitionStack> =
    HardwareCell::new(TransitionStack([0; STACK_BYTES]));

#[cfg(feature = "selftest")]
static RUST_USER_ENTRIES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "selftest")]
static INVALID_RUST_USER_ENTRIES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "selftest")]
static COMPLETE_PAIR_SWITCHES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "selftest")]
static DISTINCT_PAIR_SWITCHES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "selftest")]
static HAVE_LAST_PAIR: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "selftest")]
static LAST_FULL_ROOT: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "selftest")]
static LAST_STRIPPED_ROOT: AtomicU64 = AtomicU64::new(0);

/// Publish one inseparable pair and its ordinary kernel-stack destination.
pub(super) fn publish_route(
    roots: &AddressSpaceRoots,
    stack: KernelStackTop<'_>,
) -> Result<(), RouteError> {
    assert!(
        !interrupt::are_enabled(),
        "a paired entry route changes only with interrupts excluded"
    );
    assert!(
        WOST_KPTI_ROUTE.state.load(Ordering::Acquire) == ROUTE_UPDATING,
        "a paired entry route publishes only inside a route transaction"
    );
    let Some(stripped) = roots.stripped_root() else {
        return refuse_route(RouteError::SharedRoot);
    };
    let full = roots.full_root();
    if !route_roots_are_valid(full, stripped) {
        return refuse_route(RouteError::InvalidRoot);
    }
    if !cpu::page_table_context_ids_are_disabled() {
        return refuse_route(RouteError::PcidEnabled);
    }
    let Some(enclave) = entry_enclave() else {
        return refuse_route(RouteError::InvalidEnclave);
    };
    let task_stack_top = stack.address();
    if task_stack_top < 0xffff_8000_0000_0000 || !task_stack_top.is_multiple_of(16) {
        return refuse_route(RouteError::InvalidTaskStack);
    }

    WOST_KPTI_ROUTE.full_root.store(full, Ordering::Relaxed);
    WOST_KPTI_ROUTE
        .stripped_root
        .store(stripped, Ordering::Relaxed);
    WOST_KPTI_ROUTE
        .task_stack_top
        .store(task_stack_top, Ordering::Relaxed);
    WOST_KPTI_ROUTE
        .transition_stack_top
        .store(transition_stack_top().address(), Ordering::Relaxed);
    WOST_KPTI_ROUTE.interrupt_stack_top.store(
        segment::enclave_ist_stack_top().address(),
        Ordering::Relaxed,
    );
    WOST_KPTI_ROUTE
        .enclave_text_start
        .store(enclave.mappings()[0].section().start, Ordering::Relaxed);
    WOST_KPTI_ROUTE
        .enclave_text_end
        .store(enclave.mappings()[0].section().end, Ordering::Relaxed);
    Ok(())
}

fn refuse_route(reason: RouteError) -> Result<(), RouteError> {
    // `ROUTE_UPDATING` is deliberately retained. It is not entry authority,
    // and assembly makes every interrupt terminal until the caller either
    // clears the transaction or stops, so a refused successor cannot revive
    // the previously published task.
    Err(reason)
}

/// Invalidate the old route before changing any task entry state.
pub(super) fn begin_route_update() {
    assert!(
        !interrupt::are_enabled(),
        "a paired entry route changes only with interrupts excluded"
    );
    WOST_KPTI_ROUTE
        .state
        .store(ROUTE_UPDATING, Ordering::Release);
}

/// Remove any staged paired route before a shared-root task may run.
pub(super) fn clear_route() {
    assert!(
        !interrupt::are_enabled(),
        "a paired entry route clears only with interrupts excluded"
    );
    assert!(
        WOST_KPTI_ROUTE.state.load(Ordering::Acquire) == ROUTE_UPDATING,
        "a paired entry route clears only inside a route transaction"
    );
    WOST_KPTI_ROUTE.full_root.store(0, Ordering::Relaxed);
    WOST_KPTI_ROUTE.stripped_root.store(0, Ordering::Relaxed);
    WOST_KPTI_ROUTE.task_stack_top.store(0, Ordering::Relaxed);
    WOST_KPTI_ROUTE
        .transition_stack_top
        .store(0, Ordering::Relaxed);
    WOST_KPTI_ROUTE
        .interrupt_stack_top
        .store(0, Ordering::Relaxed);
    WOST_KPTI_ROUTE.state.store(ROUTE_EMPTY, Ordering::Release);
}

/// Point privilege entry at the guarded enclave transition stack.
pub(super) fn install_transition_rsp0() -> bool {
    if !ENTRY_ROOT_SWITCH_IMPLEMENTED
        || WOST_KPTI_ROUTE.state.load(Ordering::Acquire) != ROUTE_UPDATING
    {
        return false;
    }
    assert!(
        !interrupt::are_enabled(),
        "entry stacks change only with interrupts excluded"
    );
    segment::install_entry_stacks(transition_stack_top(), segment::enclave_ist_stack_top());
    true
}

/// Publish a fully installed pair after its full root is active.
pub(super) fn finish_route_update(roots: AddressSpaceRoots) -> bool {
    assert!(
        !interrupt::are_enabled(),
        "a paired entry route finishes only with interrupts excluded"
    );
    if WOST_KPTI_ROUTE.state.load(Ordering::Acquire) != ROUTE_UPDATING
        || cpu::page_table_base() != roots.full_root()
        || WOST_KPTI_ROUTE.full_root.load(Ordering::Acquire) != roots.full_root()
        || WOST_KPTI_ROUTE.stripped_root.load(Ordering::Acquire) != roots.user_root()
        || segment::kernel_stack() != transition_stack_top().address()
        || segment::double_fault_stack_address() != segment::enclave_ist_stack_top().address()
    {
        return false;
    }
    WOST_KPTI_ROUTE
        .state
        .store(ROUTE_PUBLISHED, Ordering::Release);
    #[cfg(feature = "selftest")]
    record_complete_pair_switch(roots);
    true
}

#[cfg(feature = "selftest")]
fn record_complete_pair_switch(roots: AddressSpaceRoots) {
    let Some(stripped) = roots.stripped_root() else {
        return;
    };
    let full = roots.full_root();
    COMPLETE_PAIR_SWITCHES.fetch_add(1, Ordering::Relaxed);
    let had_last = HAVE_LAST_PAIR.swap(true, Ordering::AcqRel);
    let previous_full = LAST_FULL_ROOT.swap(full, Ordering::AcqRel);
    let previous_stripped = LAST_STRIPPED_ROOT.swap(stripped, Ordering::AcqRel);
    if !had_last || previous_full != full || previous_stripped != stripped {
        DISTINCT_PAIR_SWITCHES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Record the root on which a user-origin frame first reaches Rust.
#[cfg(feature = "selftest")]
pub(super) fn record_rust_entry(code_selector: u64) {
    if code_selector & 0b11 != 3 {
        return;
    }
    RUST_USER_ENTRIES.fetch_add(1, Ordering::Relaxed);
    let full = WOST_KPTI_ROUTE.full_root.load(Ordering::Acquire);
    if WOST_KPTI_ROUTE.state.load(Ordering::Acquire) != ROUTE_PUBLISHED
        || cpu::page_table_base() != full
    {
        INVALID_RUST_USER_ENTRIES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Whether every stripped-root arrival reached Rust only after the full load.
#[cfg(feature = "selftest")]
pub(super) fn every_user_entry_reached_rust_on_full_root() -> bool {
    let stripped = WOST_KPTI_ROUTE.stripped_entries.load(Ordering::Acquire);
    stripped > 0
        && RUST_USER_ENTRIES.load(Ordering::Acquire) == stripped
        && INVALID_RUST_USER_ENTRIES.load(Ordering::Acquire) == 0
}

/// Whether every prepared user return was next observed on the stripped root.
#[cfg(feature = "selftest")]
pub(super) fn every_user_return_installed_the_stripped_root() -> bool {
    let prepared = WOST_KPTI_ROUTE.prepared_returns.load(Ordering::Acquire);
    prepared > 0 && WOST_KPTI_ROUTE.stripped_entries.load(Ordering::Acquire) == prepared
}

/// Whether task switches installed complete routes for at least two pairs.
#[cfg(feature = "selftest")]
pub(super) fn context_switch_moved_complete_root_pairs() -> bool {
    COMPLETE_PAIR_SWITCHES.load(Ordering::Acquire) >= 2
        && DISTINCT_PAIR_SWITCHES.load(Ordering::Acquire) >= 2
}

/// Whether both guarded enclave stacks contain no retained transition bytes.
#[cfg(feature = "selftest")]
pub(super) fn entry_stacks_are_guarded_and_scrubbed() -> bool {
    if WOST_KPTI_ROUTE.state.load(Ordering::Acquire) != ROUTE_EMPTY {
        return false;
    }
    let Some(enclave) = entry_enclave() else {
        return false;
    };
    let guards_absent = enclave.guards().into_iter().all(|guard| {
        (guard.start..guard.end)
            .step_by(PAGE_BYTES as usize)
            .all(|address| super::paging::translate(address).is_none())
    });
    if !guards_absent || !segment::enclave_ist_stack_is_zeroed_for_test() {
        return false;
    }
    let bytes = TRANSITION_STACK.as_mut_ptr().cast::<u8>();
    (0..STACK_BYTES).all(|offset| {
        // SAFETY: the route is empty, so hardware cannot be using the
        // transition stack; the linker-owned allocation spans STACK_BYTES.
        let byte = unsafe { bytes.add(offset) };
        // SAFETY: `byte` is inside that same inactive allocation.
        unsafe { byte.read_volatile() == 0 }
    })
}

/// Whether the enclave contains the complete root-switch transaction.
pub(super) const fn entry_path_is_implemented() -> bool {
    ENTRY_ROOT_SWITCH_IMPLEMENTED
}

/// Whether every low-level prerequisite for paired entry is installed.
pub(super) fn entry_is_ready() -> bool {
    if !ENTRY_ROOT_SWITCH_IMPLEMENTED {
        return false;
    }
    WOST_KPTI_ROUTE.state.load(Ordering::Acquire) == ROUTE_PUBLISHED
        && cpu::page_table_context_ids_are_disabled()
        && entry_enclave().is_some()
        && segment::kernel_stack() == transition_stack_top().address()
        && segment::double_fault_stack_address() == segment::enclave_ist_stack_top().address()
}

/// Whether the live route and stacks name this exact bound task.
pub(super) fn entry_is_ready_for(roots: AddressSpaceRoots, stack: KernelStackTop<'_>) -> bool {
    entry_is_ready()
        && roots.stripped_root().is_some()
        && WOST_KPTI_ROUTE.full_root.load(Ordering::Acquire) == roots.full_root()
        && WOST_KPTI_ROUTE.stripped_root.load(Ordering::Acquire) == roots.user_root()
        && WOST_KPTI_ROUTE.task_stack_top.load(Ordering::Acquire) == stack.address()
}

/// A stack address only this module can mint from its linker-owned storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TransitionStackTop(u64);

impl TransitionStackTop {
    pub(super) const fn address(self) -> u64 {
        self.0
    }
}

fn transition_stack_top() -> TransitionStackTop {
    TransitionStackTop(TRANSITION_STACK.address() + STACK_BYTES as u64)
}
