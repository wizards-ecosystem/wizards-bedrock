use core::sync::atomic::Ordering;

use x86_paging::PAGE_SIZE;
use x86_paging::kpti::EnclavePermission;

use super::cpu;
use super::interrupt;
use super::paging::{AddressSpaceRoots, KernelStackTop};
use super::percpu;
use super::segment;
use crate::boot::{self, Section};

pub(super) const IMAGE_MAPPINGS: usize = 4;
const KERNEL_HALF_START: u64 = 0xffff_8000_0000_0000;
const STACK_ALIGN: u64 = 16;
pub(super) const ROUTE_EMPTY: u64 = 0;
const ROUTE_PUBLISHED: u64 = 1;
const ROUTE_UPDATING: u64 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct EnclaveMapping {
    section: Section,
    full: EnclavePermission,
    stripped: EnclavePermission,
}

impl EnclaveMapping {
    pub(super) const fn new(
        section: Section,
        full: EnclavePermission,
        stripped: EnclavePermission,
    ) -> Self {
        Self {
            section,
            full,
            stripped,
        }
    }

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

// The image's entry sections and the entry areas of processors `0..areas`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct EntryEnclave {
    image: [EnclaveMapping; IMAGE_MAPPINGS],
    areas: usize,
}

impl EntryEnclave {
    pub(super) fn mappings(self) -> impl Iterator<Item = EnclaveMapping> {
        self.image
            .into_iter()
            .chain((0..self.areas).flat_map(percpu::area_mappings))
    }

    pub(super) fn guards(self) -> impl Iterator<Item = Section> {
        (0..self.areas).flat_map(percpu::area_guards)
    }
}

pub(super) fn image_enclave() -> Option<[EnclaveMapping; IMAGE_MAPPINGS]> {
    use EnclavePermission::{Executable, ReadOnly, Writable};

    let raw = boot::sections().entry_enclave;
    let exact = [raw.text, raw.rodata, raw.tables, raw.data]
        .into_iter()
        .all(page_exact);
    let ordered = raw.text.end <= raw.rodata.start
        && raw.rodata.end <= raw.tables.start
        && raw.tables.end == raw.data.start;
    if !exact || !ordered {
        return None;
    }
    Some([
        EnclaveMapping::new(raw.text, Executable, Executable),
        EnclaveMapping::new(raw.rodata, ReadOnly, ReadOnly),
        EnclaveMapping::new(raw.tables, Writable, ReadOnly),
        EnclaveMapping::new(raw.data, Writable, ReadOnly),
    ])
}

// Freezes the processor set: a stripped root holds only the areas that existed when it was built.
pub(super) fn entry_enclave() -> Option<EntryEnclave> {
    let areas = percpu::freeze_areas();
    if areas == 0 {
        return None;
    }
    Some(EntryEnclave {
        image: image_enclave()?,
        areas,
    })
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

// A refusal leaves `ROUTE_UPDATING`, which assembly treats as no entry authority.
pub(super) fn publish_route(
    roots: &AddressSpaceRoots,
    stack: KernelStackTop<'_>,
) -> Result<(), RouteError> {
    assert!(
        !interrupt::are_enabled(),
        "a paired entry route changes only with interrupts excluded"
    );
    let page = percpu::entry();
    assert!(
        page.route_state.load(Ordering::Acquire) == ROUTE_UPDATING,
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
    if !percpu::on_entry_area() || entry_enclave().is_none() {
        return Err(RouteError::InvalidEnclave);
    }
    let task_stack_top = stack.address();
    if task_stack_top < KERNEL_HALF_START || !task_stack_top.is_multiple_of(STACK_ALIGN) {
        return Err(RouteError::InvalidTaskStack);
    }

    page.full_root.store(full.get(), Ordering::Relaxed);
    page.stripped_root.store(stripped.get(), Ordering::Relaxed);
    page.task_stack_top.store(task_stack_top, Ordering::Relaxed);
    Ok(())
}

pub(super) fn begin_route_update() {
    assert!(
        !interrupt::are_enabled(),
        "a paired entry route changes only with interrupts excluded"
    );
    percpu::entry()
        .route_state
        .store(ROUTE_UPDATING, Ordering::Release);
}

pub(super) fn clear_route() {
    assert!(
        !interrupt::are_enabled(),
        "a paired entry route clears only with interrupts excluded"
    );
    let page = percpu::entry();
    assert!(
        page.route_state.load(Ordering::Acquire) == ROUTE_UPDATING,
        "a paired entry route clears only inside a route transaction"
    );
    page.full_root.store(0, Ordering::Relaxed);
    page.stripped_root.store(0, Ordering::Relaxed);
    page.task_stack_top.store(0, Ordering::Relaxed);
    page.route_state.store(ROUTE_EMPTY, Ordering::Release);
}

pub(super) fn install_transition_rsp0() -> bool {
    let page = percpu::entry();
    if page.route_state.load(Ordering::Acquire) != ROUTE_UPDATING {
        return false;
    }
    let Some(top) = page.transition_stack_top() else {
        return false;
    };
    segment::write_transition_stack(TransitionStackTop(top));
    true
}

pub(super) fn finish_route_update(roots: AddressSpaceRoots) -> bool {
    assert!(
        !interrupt::are_enabled(),
        "a paired entry route finishes only with interrupts excluded"
    );
    let page = percpu::entry();
    if page.route_state.load(Ordering::Acquire) != ROUTE_UPDATING
        || cpu::page_table_base() != roots.full_root()
        || page.full_root.load(Ordering::Acquire) != roots.full_root().get()
        || page.stripped_root.load(Ordering::Acquire) != roots.user_root().get()
        || !rsp0_is_transition_stack()
    {
        return false;
    }
    page.route_state.store(ROUTE_PUBLISHED, Ordering::Release);
    true
}

fn rsp0_is_transition_stack() -> bool {
    percpu::entry()
        .transition_stack_top()
        .is_some_and(|top| segment::kernel_stack() == top)
}

pub(super) fn entry_is_ready() -> bool {
    percpu::entry().route_state.load(Ordering::Acquire) == ROUTE_PUBLISHED
        && cpu::page_table_context_ids_are_disabled()
        && entry_enclave().is_some()
        && rsp0_is_transition_stack()
}

pub(super) fn entry_is_ready_for(roots: AddressSpaceRoots, stack: KernelStackTop<'_>) -> bool {
    let page = percpu::entry();
    entry_is_ready()
        && roots.stripped_root().is_some()
        && page.full_root.load(Ordering::Acquire) == roots.full_root().get()
        && page.stripped_root.load(Ordering::Acquire) == roots.user_root().get()
        && page.task_stack_top.load(Ordering::Acquire) == stack.address()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct TransitionStackTop(u64);

impl TransitionStackTop {
    pub(super) const fn address(self) -> u64 {
        self.0
    }
}
