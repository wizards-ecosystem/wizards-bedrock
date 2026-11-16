use core::mem::offset_of;
use core::sync::atomic::{AtomicU64, Ordering};

use x86_paging::PAGE_SIZE;
use x86_paging::kpti::EnclavePermission;

use super::cpu;
use super::interrupt;
use super::paging::{AddressSpaceRoots, KernelStackTop};
use super::segment;
use crate::boot::{self, Section};
use crate::cell::HardwareCell;

pub(super) const STACK_BYTES: usize = 16 * 1024;
pub(super) const MAPPINGS: usize = 6;
const GUARDS: usize = 2;
const KERNEL_HALF_START: u64 = 0xffff_8000_0000_0000;
const STACK_ALIGN: u64 = 16;
const ROUTE_EMPTY: u64 = 0;
const ROUTE_PUBLISHED: u64 = 1;
const ROUTE_UPDATING: u64 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct EnclaveMapping {
    section: Section,
    full: EnclavePermission,
    stripped: EnclavePermission,
}

impl EnclaveMapping {
    pub(super) const fn section(self) -> Section {
        self.section
    }

    pub(super) const fn full_access(self) -> EnclavePermission {
        self.full
    }

    pub(super) const fn stripped_access(self) -> EnclavePermission {
        self.stripped
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct EntryEnclave {
    mappings: [EnclaveMapping; MAPPINGS],
    guards: [Section; GUARDS],
}

impl EntryEnclave {
    pub(super) const fn mappings(self) -> [EnclaveMapping; MAPPINGS] {
        self.mappings
    }

    pub(super) const fn guards(self) -> [Section; GUARDS] {
        self.guards
    }
}

pub(super) fn entry_enclave() -> Option<EntryEnclave> {
    use EnclavePermission::{Executable, ReadOnly, Writable};

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
        && raw.route.end == raw.transition_guard.start
        && raw.transition_guard.end == raw.transition_stack.start
        && raw.transition_stack.end == raw.ist_guard.start
        && raw.ist_guard.end == raw.ist_stack.start
        && raw.ist_stack.end == raw.data.end;
    let fixed = raw.route.mapped_len() == PAGE_SIZE
        && raw.transition_guard.mapped_len() == PAGE_SIZE
        && raw.transition_stack.mapped_len() == STACK_BYTES as u64
        && raw.ist_guard.mapped_len() == PAGE_SIZE
        && raw.ist_stack.mapped_len() == STACK_BYTES as u64;
    if !exact || !ordered || !fixed {
        return None;
    }

    Some(EntryEnclave {
        mappings: [
            mapping(raw.text, Executable, Executable),
            mapping(raw.rodata, ReadOnly, ReadOnly),
            mapping(raw.tables, Writable, ReadOnly),
            mapping(raw.route, Writable, ReadOnly),
            mapping(raw.transition_stack, Writable, Writable),
            mapping(raw.ist_stack, Writable, Writable),
        ],
        guards: [raw.transition_guard, raw.ist_guard],
    })
}

const fn mapping(
    section: Section,
    full: EnclavePermission,
    stripped: EnclavePermission,
) -> EnclaveMapping {
    EnclaveMapping {
        section,
        full,
        stripped,
    }
}

const fn page_exact(section: Section) -> bool {
    section.start < section.end
        && section.start.is_multiple_of(PAGE_SIZE)
        && section.end.is_multiple_of(PAGE_SIZE)
}

const fn route_roots_are_valid(full: u64, stripped: u64) -> bool {
    full != stripped && full.is_multiple_of(PAGE_SIZE) && stripped.is_multiple_of(PAGE_SIZE)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RouteError {
    SharedRoot,
    InvalidRoot,
    InvalidTaskStack,
    PcidEnabled,
    InvalidEnclave,
}

#[derive(Debug)]
#[repr(C)]
struct RoutingPage {
    state: AtomicU64,
    full_root: AtomicU64,
    stripped_root: AtomicU64,
    task_stack_top: AtomicU64,
    transition_stack_top: AtomicU64,
    interrupt_stack_top: AtomicU64,
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
        }
    }
}

const _: () = {
    assert!(
        offset_of!(RoutingPage, state) == 0,
        "the routing page layout must match the entry assembly"
    );
    assert!(
        offset_of!(RoutingPage, full_root) == 8,
        "the routing page layout must match the entry assembly"
    );
    assert!(
        offset_of!(RoutingPage, stripped_root) == 16,
        "the routing page layout must match the entry assembly"
    );
    assert!(
        offset_of!(RoutingPage, task_stack_top) == 24,
        "the routing page layout must match the entry assembly"
    );
    assert!(
        offset_of!(RoutingPage, transition_stack_top) == 32,
        "the routing page layout must match the entry assembly"
    );
    assert!(
        offset_of!(RoutingPage, interrupt_stack_top) == 40,
        "the routing page layout must match the entry assembly"
    );
    assert!(
        size_of::<RoutingPage>() == 48,
        "the routing page layout must match the entry assembly"
    );
};

pub(super) mod route {
    use core::mem::offset_of;

    use super::RoutingPage;

    pub(in crate::arch::x86_64) const STATE: usize = offset_of!(RoutingPage, state);
    pub(in crate::arch::x86_64) const FULL_ROOT: usize = offset_of!(RoutingPage, full_root);
    pub(in crate::arch::x86_64) const STRIPPED_ROOT: usize = offset_of!(RoutingPage, stripped_root);
    pub(in crate::arch::x86_64) const TASK_STACK_TOP: usize =
        offset_of!(RoutingPage, task_stack_top);
    pub(in crate::arch::x86_64) const TRANSITION_STACK_TOP: usize =
        offset_of!(RoutingPage, transition_stack_top);
    pub(in crate::arch::x86_64) const INTERRUPT_STACK_TOP: usize =
        offset_of!(RoutingPage, interrupt_stack_top);
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".kpti.state.route")]
static WOST_KPTI_ROUTE: RoutingPage = RoutingPage::new();

#[derive(Debug)]
#[repr(C, align(4096))]
struct TransitionStack([u8; STACK_BYTES]);

#[unsafe(link_section = ".kpti.state.transition_stack")]
static TRANSITION_STACK: HardwareCell<TransitionStack> =
    HardwareCell::new(TransitionStack([0; STACK_BYTES]));

// A refusal leaves `ROUTE_UPDATING`, which assembly treats as no entry authority.
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
        return Err(RouteError::SharedRoot);
    };
    let full = roots.full_root();
    if !route_roots_are_valid(full.get(), stripped.get()) {
        return Err(RouteError::InvalidRoot);
    }
    if !cpu::page_table_context_ids_are_disabled() {
        return Err(RouteError::PcidEnabled);
    }
    if entry_enclave().is_none() {
        return Err(RouteError::InvalidEnclave);
    }
    let task_stack_top = stack.address();
    if task_stack_top < KERNEL_HALF_START || !task_stack_top.is_multiple_of(STACK_ALIGN) {
        return Err(RouteError::InvalidTaskStack);
    }

    WOST_KPTI_ROUTE
        .full_root
        .store(full.get(), Ordering::Relaxed);
    WOST_KPTI_ROUTE
        .stripped_root
        .store(stripped.get(), Ordering::Relaxed);
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
    Ok(())
}

pub(super) fn begin_route_update() {
    assert!(
        !interrupt::are_enabled(),
        "a paired entry route changes only with interrupts excluded"
    );
    WOST_KPTI_ROUTE
        .state
        .store(ROUTE_UPDATING, Ordering::Release);
}

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

pub(super) fn install_transition_rsp0() -> bool {
    if WOST_KPTI_ROUTE.state.load(Ordering::Acquire) != ROUTE_UPDATING {
        return false;
    }
    assert!(
        !interrupt::are_enabled(),
        "entry stacks change only with interrupts excluded"
    );
    segment::install_entry_stacks(transition_stack_top(), segment::enclave_ist_stack_top());
    true
}

pub(super) fn finish_route_update(roots: AddressSpaceRoots) -> bool {
    assert!(
        !interrupt::are_enabled(),
        "a paired entry route finishes only with interrupts excluded"
    );
    if WOST_KPTI_ROUTE.state.load(Ordering::Acquire) != ROUTE_UPDATING
        || cpu::page_table_base() != roots.full_root()
        || WOST_KPTI_ROUTE.full_root.load(Ordering::Acquire) != roots.full_root().get()
        || WOST_KPTI_ROUTE.stripped_root.load(Ordering::Acquire) != roots.user_root().get()
        || segment::kernel_stack() != transition_stack_top().address()
        || segment::double_fault_stack_address() != segment::enclave_ist_stack_top().address()
    {
        return false;
    }
    WOST_KPTI_ROUTE
        .state
        .store(ROUTE_PUBLISHED, Ordering::Release);
    true
}

pub(super) fn entry_is_ready() -> bool {
    WOST_KPTI_ROUTE.state.load(Ordering::Acquire) == ROUTE_PUBLISHED
        && cpu::page_table_context_ids_are_disabled()
        && entry_enclave().is_some()
        && segment::kernel_stack() == transition_stack_top().address()
        && segment::double_fault_stack_address() == segment::enclave_ist_stack_top().address()
}

pub(super) fn entry_is_ready_for(roots: AddressSpaceRoots, stack: KernelStackTop<'_>) -> bool {
    entry_is_ready()
        && roots.stripped_root().is_some()
        && WOST_KPTI_ROUTE.full_root.load(Ordering::Acquire) == roots.full_root().get()
        && WOST_KPTI_ROUTE.stripped_root.load(Ordering::Acquire) == roots.user_root().get()
        && WOST_KPTI_ROUTE.task_stack_top.load(Ordering::Acquire) == stack.address()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct TransitionStackTop(u64);

impl TransitionStackTop {
    pub(super) const fn address(self) -> u64 {
        self.0
    }
}

fn transition_stack_top() -> TransitionStackTop {
    TransitionStackTop(TRANSITION_STACK.address() + STACK_BYTES as u64)
}
