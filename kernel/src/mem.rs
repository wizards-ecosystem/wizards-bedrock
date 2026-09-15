//! Physical memory: what the bootloader reported, and what the kernel took.
//!
//! The frame allocator itself lives in `wost::mm` — owning a frame needs a raw
//! pointer, so it is `wost`'s (D-43). What is here is the part that needs no
//! privilege: deciding whether the handoff is usable at all, and saying so.

use alloc::vec::Vec;
use stack_layout::StackLayout;
use wost::arch::current::cpu;
#[cfg(feature = "selftest")]
use wost::arch::current::paging::UserMapError;
use wost::arch::current::paging::{
    self, AddressSpace, BarMapFailure, BarUnmapOutcome, BootRegion, DeviceUnmapFailure, MapError,
    MappedStack, ReclaimFailure, ReleasableDevice, StackMapError,
};
use wost::arch::current::pci as platform_pci;
use wost::arch::current::segment;
use wost::boot::{
    self, DirectMap, DirectMapCache, HandoffError, ImageAddress, MemoryKind, MemoryMap,
};
use wost::mm;
use wost::sync::SpinLock;
use x86_apic::io;
use x86_paging::{Flags, MappingRangeError};

use crate::acpi::Interrupts;
use crate::arch::current::ioapic::IoApic;
use crate::thread::ThreadContext;
use crate::{debug, info, thread, warn};

/// Bytes in a page frame.
#[cfg(feature = "selftest")]
const FRAME_SIZE: u64 = 4096;

/// Bring the frame allocator up over the bootloader's memory map.
///
/// Declines rather than panics when either half of the handoff is missing: a
/// kernel that cannot allocate is still worth booting far enough to say why,
/// and the checks that follow report the absence rather than faulting on it.
pub(crate) fn init(map: Result<MemoryMap, HandoffError>, direct: Result<DirectMap, HandoffError>) {
    let map = match map {
        Ok(map) => map,
        Err(reason) => {
            warn!("memory map: {reason}; no frame allocator");
            return;
        }
    };
    let direct = match direct {
        Ok(direct) => direct,
        Err(reason) => {
            warn!("direct map: {reason}; no frame allocator");
            return;
        }
    };

    describe(&map);
    info!("memory: direct map at {:#x}", direct.offset());

    let report = mm::init(&map, direct);
    if report.managed == 0 {
        warn!("memory: no usable frames; no frame allocator");
        return;
    }
    info!("memory: {report}");
}

/// The kernel's own address space, kept live after the switch.
///
/// Retained so mappings can be changed after activation (D-45, D-86).
static SPACE: SpinLock<Option<AddressSpace>> = SpinLock::new(None);

/// BAR owners whose exact retirement failed after register access ended.
static RETAINED_PCI_BARS: SpinLock<
    [Option<PciBarQuarantine>; crate::pci::FUNCTION_AUTHORITY_LIMIT * pci_config::BARS],
> = SpinLock::new([const { None }; crate::pci::FUNCTION_AUTHORITY_LIMIT * pci_config::BARS]);

/// One controller whose rollback could not safely begin.
static RETAINED_ROUTING: SpinLock<Option<Routing>> = SpinLock::new(None);

/// One unexposed mapping and every earlier routing candidate still owned.
static RETAINED_ROUTING_MAPPING: SpinLock<
    Option<(paging::DeviceMapping, Vec<IoApic<ReleasableDevice>>)>,
> = SpinLock::new(None);

/// One refused unmap and every earlier routing candidate still owned.
static RETAINED_ROUTING_REGISTERS: SpinLock<
    Option<(ReleasableDevice, Vec<IoApic<ReleasableDevice>>)>,
> = SpinLock::new(None);

/// Run one operation owned by this module against the live address space.
///
/// Private because `AddressSpace` also carries reclaim and arbitrary unmap
/// authority over the tables the CPU is using. Callers get only the operations
/// below, while this token-bearing acquisition preserves D-85's lock order.
fn with_active_space<T>(
    context: &mut ThreadContext,
    f: impl FnOnce(&mut AddressSpace) -> T,
) -> Option<T> {
    SPACE.lock(context).as_mut().map(f)
}

/// Map one physical device extent into the kernel's device window.
pub(crate) fn map_device(
    context: &mut ThreadContext,
    physical: u64,
    len: u64,
) -> Option<Result<paging::DeviceMapping, MapError>> {
    let admitted = match paging::admit_device_range(physical, len) {
        Ok(admitted) => admitted,
        Err(reason) => return Some(Err(reason)),
    };
    with_active_space(context, |space| space.map_device(admitted))
}

/// Map one function-bound PCI BAR into the kernel's device window.
pub(crate) fn map_pci_bar(
    context: &mut ThreadContext,
    admitted: platform_pci::AdmittedBar,
) -> Result<platform_pci::BarMapping, PciBarMapFailure> {
    let mut space = SPACE.lock(context);
    let Some(space) = space.as_mut() else {
        return Err(PciBarMapFailure::NoSpace(admitted));
    };
    space
        .map_pci_bar(admitted)
        .map_err(PciBarMapFailure::Mapping)
}

/// A refused PCI BAR map retaining its exact function-bound authority.
#[derive(Debug)]
pub(crate) enum PciBarMapFailure {
    /// The running address space was unavailable.
    NoSpace(platform_pci::AdmittedBar),
    /// Page-table or device-window admission refused.
    Mapping(BarMapFailure),
}

/// The exact remaining owner of a PCI BAR whose cleanup could not finish.
#[derive(Debug)]
enum PciBarQuarantine {
    /// Pages are installed, but register access was never exposed.
    Mapping { _mapping: platform_pci::BarMapping },
    /// Pages remain installed after an exact unmap refusal.
    Releasable { _bar: platform_pci::ReleasableBar },
    /// Pages are gone, but the function registry refused the BAR authority.
    Authority {
        _failure: pci_config::BarAuthorityFailure,
    },
}

fn retain_pci_bar(quarantine: PciBarQuarantine) {
    let mut retained = RETAINED_PCI_BARS.lock_leaf();
    let Some(slot) = retained.iter_mut().find(|slot| slot.is_none()) else {
        panic!("retained BAR authority exceeds every BAR of the PCI function registry");
    };
    *slot = Some(quarantine);
}

/// Keep an inaccessible BAR mapping reachable after its live-root check fails.
pub(crate) fn retain_pci_bar_mapping(mapping: platform_pci::BarMapping) {
    retain_pci_bar(PciBarQuarantine::Mapping { _mapping: mapping });
}

/// Keep BAR authority reachable after its function registry refuses release.
pub(crate) fn retain_pci_bar_authority(failure: pci_config::BarAuthorityFailure) {
    retain_pci_bar(PciBarQuarantine::Authority { _failure: failure });
}

impl PciBarMapFailure {
    /// The page-table reason, if mapping reached the active space.
    pub(crate) const fn reason(&self) -> Option<MapError> {
        match self {
            Self::NoSpace(_) => None,
            Self::Mapping(failure) => Some(failure.reason),
        }
    }

    /// Recover the unchanged BAR authority.
    pub(crate) fn into_admitted(self) -> platform_pci::AdmittedBar {
        match self {
            Self::NoSpace(admitted) => admitted,
            Self::Mapping(failure) => failure.admitted,
        }
    }
}

/// Result of returning an inaccessible PCI BAR mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a quarantined BAR still owns exact cleanup authority"]
pub(crate) enum PciBarRetirement {
    /// Mapping and function-bound BAR authority were both returned.
    Retired,
    /// The exact remaining owner is held in the kernel quarantine.
    Quarantined,
}

/// Return a PCI BAR mapping, retrying one unchanged preflight refusal.
pub(crate) fn unmap_pci_bar_exact(
    context: &mut ThreadContext,
    bar: platform_pci::ReleasableBar,
) -> PciBarRetirement {
    let mut active = SPACE.lock(context);
    let Some(space) = active.as_mut() else {
        drop(active);
        retain_pci_bar(PciBarQuarantine::Releasable { _bar: bar });
        return PciBarRetirement::Quarantined;
    };
    let authority = match space.unmap_pci_bar(bar) {
        BarUnmapOutcome::Unmapped(authority) => authority,
        BarUnmapOutcome::Refused(first) => match space.unmap_pci_bar(first.bar) {
            BarUnmapOutcome::Unmapped(authority) => authority,
            BarUnmapOutcome::Refused(second) => {
                let bar = second.bar;
                drop(active);
                retain_pci_bar(PciBarQuarantine::Releasable { _bar: bar });
                return PciBarRetirement::Quarantined;
            }
        },
    };
    drop(active);
    match platform_pci::release_memory_bar(authority) {
        Ok(()) => PciBarRetirement::Retired,
        Err(failure) => {
            retain_pci_bar_authority(failure);
            PciBarRetirement::Quarantined
        }
    }
}

/// Exact retirement or retained access for one mapped PCI BAR.
#[derive(Debug)]
#[must_use = "a refused mapped-BAR retirement returns its access owner"]
pub(crate) enum MappedPciBarRetirement {
    /// Mapping and function-bound BAR authority were both returned.
    Retired,
    /// No active address space was available, so the mapped BAR remains owned.
    Retained(platform_pci::MappedBar),
    /// Register access ended and the exact remaining owner is quarantined.
    Quarantined,
}

/// Return a mapped PCI BAR without losing it when no active space is available.
pub(crate) fn unmap_mapped_pci_bar_exact(
    context: &mut ThreadContext,
    bar: platform_pci::MappedBar,
) -> MappedPciBarRetirement {
    let mut active = SPACE.lock(context);
    let Some(space) = active.as_mut() else {
        return MappedPciBarRetirement::Retained(bar);
    };
    let authority = match space.unmap_pci_bar(bar.into_releasable()) {
        BarUnmapOutcome::Unmapped(authority) => authority,
        BarUnmapOutcome::Refused(first) => match space.unmap_pci_bar(first.bar) {
            BarUnmapOutcome::Unmapped(authority) => authority,
            BarUnmapOutcome::Refused(second) => {
                let bar = second.bar;
                drop(active);
                retain_pci_bar(PciBarQuarantine::Releasable { _bar: bar });
                return MappedPciBarRetirement::Quarantined;
            }
        },
    };
    drop(active);
    if let Err(failure) = platform_pci::release_memory_bar(authority) {
        retain_pci_bar_authority(failure);
        return MappedPciBarRetirement::Quarantined;
    }
    MappedPciBarRetirement::Retired
}

/// Result of one transient device-unmap attempt.
#[derive(Debug)]
#[must_use = "every refusal returns the exact device-mapping owner"]
pub(crate) enum DeviceUnmapOutcome {
    /// The complete mapping was removed.
    Unmapped,
    /// Preflight refused without changing the mapping.
    Refused(DeviceUnmapFailure),
    /// No active address space was available, so the mapping remains owned.
    NoSpace(ReleasableDevice),
}

/// Remove one transient device mapping after its access owner is consumed.
pub(crate) fn unmap_device(
    context: &mut ThreadContext,
    region: paging::ReleasableDevice,
) -> DeviceUnmapOutcome {
    let mut active = SPACE.lock(context);
    let Some(space) = active.as_mut() else {
        return DeviceUnmapOutcome::NoSpace(region);
    };
    match space.unmap_device(region) {
        Ok(()) => DeviceUnmapOutcome::Unmapped,
        Err(failure) => DeviceUnmapOutcome::Refused(failure),
    }
}

/// Exact retirement or retained ownership of one transient device mapping.
#[derive(Debug)]
#[must_use = "a refused device retirement returns its exact mapping owner"]
pub(crate) enum DeviceRetirement {
    /// The complete mapping was removed.
    Retired,
    /// Both preflight attempts refused or no active space was available.
    Retained(ReleasableDevice),
}

/// Return a transient device mapping, retrying one unchanged preflight refusal.
pub(crate) fn unmap_device_exact(
    context: &mut ThreadContext,
    region: ReleasableDevice,
) -> DeviceRetirement {
    match unmap_device(context, region) {
        DeviceUnmapOutcome::Unmapped => DeviceRetirement::Retired,
        DeviceUnmapOutcome::Refused(first) => match unmap_device(context, first.region) {
            DeviceUnmapOutcome::Unmapped => DeviceRetirement::Retired,
            DeviceUnmapOutcome::Refused(second) => DeviceRetirement::Retained(second.region),
            DeviceUnmapOutcome::NoSpace(region) => DeviceRetirement::Retained(region),
        },
        DeviceUnmapOutcome::NoSpace(region) => DeviceRetirement::Retained(region),
    }
}

/// Device extents currently owned by the running address space.
#[cfg(feature = "selftest")]
pub(crate) fn device_mappings(context: &mut ThreadContext) -> Option<usize> {
    with_active_space(context, |space| space.device_mappings())
}

/// Active-root and device-ledger reclaim guards, without attempting reclaim.
#[cfg(feature = "selftest")]
pub(crate) fn active_space_reclaim_is_guarded(context: &mut ThreadContext) -> Option<bool> {
    with_active_space(context, |space| {
        space.permanent_root_blocks_reclaim_for_test() && space.devices_block_reclaim_for_test()
    })
}

/// Whether activation prevents later creation of an unshared kernel root.
#[cfg(feature = "selftest")]
pub(crate) fn active_kernel_topology_is_frozen(context: &mut ThreadContext) -> Option<bool> {
    with_active_space(context, AddressSpace::kernel_topology_is_frozen_for_test)
}

/// A live owned mapping made by a machine check.
///
/// The fields are private and the value is not copyable: the only removal
/// door below consumes exactly the mapping this module just created.
#[cfg(feature = "selftest")]
pub(crate) struct ActiveMapping {
    base: u64,
    len: u64,
}

/// Divide exact removal authority without widening the represented range.
#[cfg(feature = "selftest")]
pub(crate) fn split_mapping_for_test(
    mapping: ActiveMapping,
    left_len: u64,
) -> Option<(ActiveMapping, ActiveMapping)> {
    if left_len == 0 || left_len >= mapping.len || !left_len.is_multiple_of(FRAME_SIZE) {
        return None;
    }
    let right_base = mapping.base.checked_add(left_len)?;
    Some((
        ActiveMapping {
            base: mapping.base,
            len: left_len,
        },
        ActiveMapping {
            base: right_base,
            len: mapping.len - left_len,
        },
    ))
}

/// Add an owned mapping to the running tables for a machine check.
#[cfg(feature = "selftest")]
pub(crate) fn map_owned_for_test(
    context: &mut ThreadContext,
    base: u64,
    run: mm::Segment,
    flags: Flags,
) -> Option<Result<ActiveMapping, MapError>> {
    let len = run.len();
    with_active_space(context, |space| space.map_owned(base, run, flags))
        .map(|result| result.map(|()| ActiveMapping { base, len }))
}

/// Exercise the production map door against an already-pinned stack guard.
#[cfg(feature = "selftest")]
pub(crate) fn pinned_stack_refuses_map_for_test(
    context: &mut ThreadContext,
    slot: &stack_layout::Slot,
) -> Option<bool> {
    with_active_space(context, |space| {
        space.pinned_stack_refuses_map_for_test(slot)
    })?
}

/// Add a user mapping to the running tables for the user-copy machine proof.
#[cfg(feature = "selftest")]
pub(crate) fn map_user_for_test(
    context: &mut ThreadContext,
    base: u64,
    run: mm::USegment,
    flags: Flags,
) -> Option<Result<(), UserMapError>> {
    with_active_space(context, |space| space.map_user(base, run, flags))
}

/// Add one user page and retain its exact removal authority for a machine check.
#[cfg(feature = "selftest")]
pub(crate) fn map_user_page_for_test(
    context: &mut ThreadContext,
    base: u64,
    run: mm::Segment,
    flags: Flags,
) -> Option<Result<ActiveMapping, UserMapError>> {
    if run.len() != FRAME_SIZE {
        return None;
    }
    with_active_space(context, |space| space.map_user(base, run.into(), flags)).map(|result| {
        result.map(|()| ActiveMapping {
            base,
            len: FRAME_SIZE,
        })
    })
}

/// Map a guarded stack and retain the exact authority needed to remove it.
#[cfg(feature = "selftest")]
pub(crate) fn map_reclaimable_stack_for_test(
    context: &mut ThreadContext,
    layout: StackLayout,
) -> Option<Result<(MappedStack, ActiveMapping), StackMapError>> {
    let slot = layout.slot(0)?;
    with_active_space(context, |space| space.map_stack(&slot)).map(|result| {
        result.map(|stack| {
            (
                stack,
                ActiveMapping {
                    base: slot.stack_base(),
                    len: slot.stack_len(),
                },
            )
        })
    })
}

/// Map one stack arena into the active root with exact removal authority.
#[cfg(feature = "selftest")]
pub(crate) fn map_stack_arena_for_test(
    context: &mut ThreadContext,
    layout: StackLayout,
) -> Option<Result<(paging::StackArena, ActiveMapping), StackMapError>> {
    if layout.slots() != 1 {
        return None;
    }
    let slot = layout.slot(0)?;
    with_active_space(context, |space| space.map_stacks(&layout)).map(|result| {
        result.map(|arena| {
            (
                arena,
                ActiveMapping {
                    base: slot.stack_base(),
                    len: slot.stack_len(),
                },
            )
        })
    })
}

/// Remove only the active mapping whose unforgeable value is consumed here.
#[cfg(feature = "selftest")]
pub(crate) fn unmap_for_test(
    context: &mut ThreadContext,
    mapping: ActiveMapping,
) -> Option<Result<paging::Unmapped, MapError>> {
    with_active_space(context, |space| space.unmap(mapping.base, mapping.len))
}

/// Exercise fixed invalid-unmap requests without exporting an address-taking door.
#[cfg(feature = "selftest")]
pub(crate) fn invalid_unmaps_are_refused(context: &mut ThreadContext) -> Option<bool> {
    const HOLE: u64 = 0xffff_fd00_0004_0000;

    let text = boot::sections().text.start;
    let direct = mm::direct_map()?.virtual_address(x86_paging::LARGE_PAGE_SIZE)?;
    let task_stack = thread::layout()?.slot(0)?;
    let task_extent = task_stack
        .stack_top()
        .checked_sub(task_stack.guard_base())?;
    with_active_space(context, |space| {
        matches!(space.unmap(HOLE, FRAME_SIZE), Err(MapError::NotMapped(_)))
            && matches!(
                space.unmap(HOLE + 1, FRAME_SIZE),
                Err(MapError::Misaligned(_))
            )
            && matches!(space.unmap(HOLE, 0), Err(MapError::Misaligned(_)))
            && matches!(
                space.unmap(HOLE & 0x0000_ffff_ffff_ffff, FRAME_SIZE),
                Err(MapError::NonCanonical(_))
            )
            && matches!(space.unmap(text, FRAME_SIZE), Err(MapError::NotOwned(_)))
            && matches!(
                space.unmap(direct, FRAME_SIZE),
                Err(MapError::Unsplittable(_))
            )
            && matches!(
                space.unmap(task_stack.guard_base(), task_extent),
                Err(MapError::Pinned(found)) if found == task_stack.stack_base()
            )
            && space.lookup(task_stack.stack_base()).is_some()
    })
}

/// Move the double-fault handler onto a guarded stack.
///
/// Mapping this stack after kernel page-table activation makes its guard page
/// genuinely absent (D-89). Pinning keeps a live hardware pointer from being
/// unmapped (D-86).
fn guard_the_fault_stack() -> Result<u64, StackMapError> {
    let layout = thread::fault_stack_layout().ok_or(MapError::OutOfMemory)?;
    let slot = layout.slot(0).ok_or(MapError::OutOfMemory)?;

    // `map_stack` verifies the guard is absent even outside self-test builds
    // (D-94).
    let mut context = ThreadContext::require();
    let stack = with_active_space(&mut context, |space| space.map_pinned_stack(&slot))
        .ok_or(MapError::OutOfMemory)??;

    let top = stack.top();
    if !segment::set_double_fault_stack(&stack) {
        panic!("a newly pinned stack must belong to the active address space");
    }
    Ok(top)
}

/// Try to remove the hardware-retained fault stack through the normal door.
#[cfg(feature = "selftest")]
pub(crate) fn unmap_fault_stack_for_test(
    context: &mut ThreadContext,
) -> Option<Result<paging::Unmapped, MapError>> {
    let slot = thread::fault_stack_layout()?.slot(0)?;
    with_active_space(context, |space| {
        space.unmap(slot.stack_base(), slot.stack_len())
    })
}

/// Map the boot thread's own guarded stack.
///
/// `None` rather than an error when it cannot be had: kernel services can
/// still report and degrade on the bootloader's stack, but the user boundary
/// remains closed (D-193).
pub(crate) fn map_boot_stack() -> Option<MappedStack> {
    let mut context = ThreadContext::require();
    map_guarded_stack(&mut context, thread::boot_stack_layout()?)
}

/// Map the one guarded stack `layout` describes.
///
/// The shared half of every stack this kernel maps after the switch: allocate,
/// zero, map writable and never executable, and verify the guard below is
/// absent — the last of which is `map_stack`'s and is why it is used rather
/// than open-coded (D-94).
pub(crate) fn map_guarded_stack(
    context: &mut ThreadContext,
    layout: StackLayout,
) -> Option<MappedStack> {
    let slot = layout.slot(0)?;
    match with_active_space(context, |space| space.map_stack(&slot))? {
        Ok(stack) => Some(stack),
        Err(StackMapError::Mapping(reason)) => {
            warn!("stacks: {:#x} not mapped: {reason}", slot.stack_base());
            None
        }
        Err(reason @ StackMapError::Rollback { .. }) => {
            panic!("stacks: {reason}");
        }
        Err(reason) => panic!("stacks: {reason}"),
    }
}

/// Where the direct map starts, for the checks that ask what a space holds.
#[cfg(feature = "selftest")]
pub(crate) fn direct_map_base() -> Option<u64> {
    mm::direct_map()?.virtual_address(0)
}

/// Log every region the bootloader described.
///
/// At debug level because it runs to a few dozen lines, and worth having at
/// all because it is the only place an unrecognised region type becomes
/// visible — the pinned `limine` crate has no constant for the one base
/// revision 4 added, so it arrives here as `unknown type N` (D-42).
fn describe(map: &MemoryMap) {
    for region in map.regions() {
        debug!(
            "memory: {:#016x}..{:#016x} {}",
            region.base,
            region.base + region.len,
            region.kind
        );
    }
    if let Some(kind) = map.regions().map(|region| region.kind).find(unrecognised) {
        info!("memory: bootloader reported {kind}");
    }
}

/// Whether a region type is one this kernel has no name for.
fn unrecognised(kind: &MemoryKind) -> bool {
    matches!(kind, MemoryKind::Unknown(_))
}

/// Build the kernel's own page tables and switch to them.
///
/// Every decision here is policy — which ranges exist and what each may do —
/// and policy is safe Rust. `wost` owns the table writes and the one `mov cr3`
/// underneath; this function cannot express an unsafe operation and does not
/// need to (D-13, D-45).
///
/// Handoff and allocation failures keep the bootloader's tables. A cleanup
/// contradiction is fatal because continuing would discard live ownership.
pub(crate) fn init_paging(
    map: Result<MemoryMap, HandoffError>,
    direct: Result<DirectMap, HandoffError>,
    image: Result<ImageAddress, HandoffError>,
    rsdp: Result<u64, HandoffError>,
) -> Option<Interrupts> {
    let (Ok(map), Ok(direct), Ok(image)) = (map, direct, image) else {
        warn!("paging: incomplete handoff; staying on the bootloader's tables");
        return None;
    };

    // Read ACPI before building tables while the bootloader's direct map
    // already covers it (D-45, D-49, D-86).
    let interrupts = match rsdp {
        Ok(rsdp) => crate::acpi::interrupts(rsdp),
        Err(reason) => {
            warn!("acpi: {reason}; no interrupt routing");
            None
        }
    };

    let mut space = match build(&map, direct, image) {
        Ok(space) => space,
        Err(BuildError::Mapping(reason)) => {
            warn!("paging: {reason}; staying on the bootloader's tables");
            return None;
        }
        Err(reason @ BuildError::Rollback { .. }) => {
            panic!("paging: {reason}");
        }
    };

    let Some(layout) = thread::layout() else {
        let cleanup = space.reclaim(None);
        assert!(
            cleanup.is_ok(),
            "an uninstalled address space must remain reclaimable"
        );
        warn!("threads: stack layout is invalid; staying on the bootloader's tables");
        return None;
    };
    let arena = match space.map_stacks(&layout) {
        Ok(arena) => arena,
        Err(StackMapError::Mapping(reason)) => {
            let cleanup = space.reclaim(None);
            assert!(
                cleanup.is_ok(),
                "a failed stack transaction must leave no owned mapping"
            );
            warn!("threads: stacks not mapped: {reason}; no threads");
            return None;
        }
        Err(reason @ StackMapError::Rollback { .. }) => {
            panic!("threads: {reason}");
        }
        Err(reason) => panic!("threads: {reason}"),
    };

    // Before the switch: the CR3 load is the flush the PAT rewrite needs.
    let write_combining = cpu::enable_write_combining();

    match paging::activate(&mut space) {
        Ok(()) => info!("paging: running on our own tables"),
        Err(reason) => {
            let returned = space.unmap_stacks(arena);
            assert!(
                returned.is_ok(),
                "an inactive stack arena must remain removable"
            );
            let cleanup = space.reclaim(None);
            assert!(
                cleanup.is_ok(),
                "a refused activation must return its address space"
            );
            warn!("paging: refused to switch: {reason}");
            return None;
        }
    }

    let thread_slots = match thread::init(arena) {
        Ok(slots) => Some(slots),
        Err(failure) => {
            let reason = failure.reason;
            let returned = space.unmap_stacks(failure.arena);
            assert!(
                returned.is_ok(),
                "an arena refused by the task table must remain removable"
            );
            warn!("threads: {reason}; no threads");
            None
        }
    };

    // Kept, which is the whole of D-86: a mapping can arrive with the thread
    // that needs it instead of being guessed while the tables are built.
    *SPACE.lock_leaf() = Some(space);

    match guard_the_fault_stack() {
        Ok(top) => info!("faults: double-fault stack guarded, top {top:#x}"),
        Err(StackMapError::Mapping(reason)) => {
            warn!("faults: double-fault stack left unguarded: {reason}");
        }
        Err(reason @ StackMapError::Rollback { .. }) => {
            panic!("faults: {reason}");
        }
        Err(reason) => panic!("faults: {reason}"),
    }

    if let Some(slots) = thread_slots {
        info!("threads: {slots} guarded stacks ready");
    }

    if write_combining.is_some_and(|transition| transition.commit()) {
        info!("framebuffer: write-combining");
    }

    interrupts
}

/// Result of proving every described external interrupt controller inert.
#[derive(Debug)]
pub(crate) struct RoutingPreparation {
    routing: Option<Routing>,
    controllers_masked: bool,
}

/// Authority to enable external interrupts after every controller was masked.
#[derive(Debug)]
pub(crate) struct ExternalControllersMasked {
    routing: Option<Routing>,
}

struct ControllersReleased {
    _private: (),
}

impl RoutingPreparation {
    fn ready(routing: Routing) -> Self {
        Self {
            routing: Some(routing),
            controllers_masked: true,
        }
    }

    const fn polled_safe(_released: ControllersReleased) -> Self {
        Self {
            routing: None,
            controllers_masked: true,
        }
    }

    pub(crate) const fn unproven() -> Self {
        Self {
            routing: None,
            controllers_masked: false,
        }
    }

    /// Extract interrupt-enable authority only from a complete mask proof.
    pub(crate) fn into_masked(self) -> Option<ExternalControllersMasked> {
        if !self.controllers_masked {
            return None;
        }
        Some(ExternalControllersMasked {
            routing: self.routing,
        })
    }
}

impl ExternalControllersMasked {
    /// The selected keyboard controller, when the proven inventory has one.
    pub(crate) fn into_routing(self) -> Option<Routing> {
        self.routing
    }
}

/// Probe every described I/O APIC and return the unique removable owner of `gsi`.
///
/// Called before interrupts are enabled. Every controller is therefore masked
/// before firmware state could deliver an unowned vector, while the active
/// address space lets all temporary mappings be returned after selection.
pub(crate) fn prepare_routing(
    context: &mut ThreadContext,
    interrupts: Interrupts,
    gsi: u32,
) -> RoutingPreparation {
    let count = interrupts.io_apic_count();
    if count == 0 {
        warn!("ioapic: firmware supplied no controller inventory");
        return RoutingPreparation::unproven();
    }
    let mut controllers = Vec::new();
    let mut ranges = Vec::new();
    if controllers.try_reserve_exact(count).is_err() || ranges.try_reserve_exact(count).is_err() {
        warn!("ioapic: no memory for {count} routing candidates; interrupts stay off");
        return RoutingPreparation::unproven();
    }

    let mut descriptions = interrupts.io_apics();
    let mut next_description = core::iter::Iterator::next(&mut descriptions);
    while let Some(description) = next_description {
        next_description = core::iter::Iterator::next(&mut descriptions);
        let controller = match probe_io_apic(context, description) {
            IoApicProbe::Ready(controller) => controller,
            IoApicProbe::Refused => {
                if release_io_apics(context, &mut controllers).is_none() {
                    warn!("ioapic: candidate cleanup was incomplete; interrupts stay off");
                }
                return RoutingPreparation::unproven();
            }
            IoApicProbe::MappingRetained(mapping) => {
                retain_routing_mapping(mapping, controllers);
                return RoutingPreparation::unproven();
            }
            IoApicProbe::RegistersRetained(registers) => {
                retain_routing_registers(registers, controllers);
                return RoutingPreparation::unproven();
            }
        };
        ranges.push(controller.range());
        controllers.push(controller);
    }

    let owner = match io::owner_of(&ranges, gsi) {
        Ok(Some(owner)) => owner,
        Ok(None) => {
            warn!("keyboard: no io apic owns gsi {gsi}; staying polled");
            return match release_io_apics(context, &mut controllers) {
                Some(released) => RoutingPreparation::polled_safe(released),
                None => RoutingPreparation::unproven(),
            };
        }
        Err(reason) => {
            warn!("ioapic: {reason}; staying polled");
            return match release_io_apics(context, &mut controllers) {
                Some(released) => RoutingPreparation::polled_safe(released),
                None => RoutingPreparation::unproven(),
            };
        }
    };
    let Some(selected) = interrupts.io_apics().nth(owner) else {
        warn!("ioapic: selected controller has no firmware description; interrupts stay off");
        if release_io_apics(context, &mut controllers).is_none() {
            warn!("ioapic: candidate cleanup was incomplete; interrupts stay off");
        }
        return RoutingPreparation::unproven();
    };

    // The selected controller is rebuilt only after every probe mapping is
    // gone, so temporary inventory state cannot survive into its ownership.
    if release_io_apics(context, &mut controllers).is_none() {
        warn!("ioapic: candidate mappings did not retire exactly; interrupts stay off");
        return RoutingPreparation::unproven();
    }
    let io_apic = match probe_io_apic(context, selected) {
        IoApicProbe::Ready(controller) => controller,
        IoApicProbe::Refused => return RoutingPreparation::unproven(),
        IoApicProbe::MappingRetained(mapping) => {
            retain_routing_mapping(mapping, controllers);
            return RoutingPreparation::unproven();
        }
        IoApicProbe::RegistersRetained(registers) => {
            retain_routing_registers(registers, controllers);
            return RoutingPreparation::unproven();
        }
    };
    RoutingPreparation::ready(Routing {
        io_apic,
        interrupts,
    })
}

#[derive(Debug)]
enum IoApicProbe {
    Ready(IoApic<ReleasableDevice>),
    Refused,
    MappingRetained(paging::DeviceMapping),
    RegistersRetained(ReleasableDevice),
}

fn probe_io_apic(context: &mut ThreadContext, description: acpi_tables::IoApic) -> IoApicProbe {
    let id = description.id();
    let mapping = match map_device(context, u64::from(description.address()), IO_APIC_WINDOW) {
        Some(Ok(mapping)) => mapping,
        Some(Err(reason)) => {
            warn!(
                "ioapic {}: registers at {:#x} not mapped: {reason}; interrupts stay off",
                id,
                description.address()
            );
            return IoApicProbe::Refused;
        }
        None => {
            warn!("ioapic: no active address space; interrupts stay off");
            return IoApicProbe::Refused;
        }
    };
    let registers = match mapping.claim_releasable() {
        Ok(registers) => registers,
        Err(mapping) => {
            warn!(
                "ioapic {}: mapped registers are not live; interrupts stay off",
                id
            );
            return IoApicProbe::MappingRetained(mapping);
        }
    };
    match IoApic::take(registers, description.gsi_base()) {
        Ok(controller) => IoApicProbe::Ready(controller),
        Err(failure) => {
            let reason = failure.reason;
            match unmap_device_exact(context, failure.registers) {
                DeviceRetirement::Retired => {
                    warn!(
                        "ioapic {}: registers unusable: {reason}; mapping retired; interrupts stay off",
                        id
                    );
                    IoApicProbe::Refused
                }
                DeviceRetirement::Retained(registers) => {
                    warn!(
                        "ioapic {}: registers unusable: {reason}; mapping retained; interrupts stay off",
                        id
                    );
                    IoApicProbe::RegistersRetained(registers)
                }
            }
        }
    }
}

fn release_io_apics(
    context: &mut ThreadContext,
    controllers: &mut Vec<IoApic<ReleasableDevice>>,
) -> Option<ControllersReleased> {
    while let Some(controller) = controllers.pop() {
        if let DeviceRetirement::Retained(registers) =
            unmap_device_exact(context, controller.into_releasable())
        {
            warn!("ioapic: candidate mapping remained owned during cleanup");
            retain_routing_registers(registers, core::mem::take(controllers));
            return None;
        }
    }
    Some(ControllersReleased { _private: () })
}

/// Bytes of register window an I/O APIC occupies. The index/data pair sits in
/// the first 32; a page is the smallest thing worth mapping.
const IO_APIC_WINDOW: u64 = 4096;

/// The interrupt controller ACPI described, ready to route lines.
#[derive(Debug)]
pub(crate) struct Routing {
    /// The controller itself.
    pub(crate) io_apic: IoApic<ReleasableDevice>,
    /// What ACPI said about which line goes where.
    pub(crate) interrupts: Interrupts,
}

fn retain_routing_mapping(
    mapping: paging::DeviceMapping,
    controllers: Vec<IoApic<ReleasableDevice>>,
) {
    let mut retained = RETAINED_ROUTING_MAPPING.lock_leaf();
    assert!(
        retained.is_none(),
        "routing continued after an exact mapping owner was quarantined"
    );
    *retained = Some((mapping, controllers));
}

fn retain_routing_registers(
    registers: ReleasableDevice,
    controllers: Vec<IoApic<ReleasableDevice>>,
) {
    let mut retained = RETAINED_ROUTING_REGISTERS.lock_leaf();
    assert!(
        retained.is_none(),
        "routing continued after an exact register owner was quarantined"
    );
    *retained = Some((registers, controllers));
}

/// Keep a routing controller reachable when cleanup cannot safely start.
pub(crate) fn retain_routing(routing: Routing) {
    let mut retained = RETAINED_ROUTING.lock_leaf();
    assert!(
        retained.is_none(),
        "routing continued after a controller was quarantined"
    );
    *retained = Some(routing);
}

/// Return an uncommitted routing controller to the active address space.
pub(crate) fn release_routing(context: &mut ThreadContext, routing: Routing) -> bool {
    match unmap_device_exact(context, routing.io_apic.into_releasable()) {
        DeviceRetirement::Retired => true,
        DeviceRetirement::Retained(registers) => {
            retain_routing_registers(registers, Vec::new());
            false
        }
    }
}

#[derive(Debug)]
enum BuildError {
    Mapping(MapError),
    Rollback {
        cause: MapError,
        cleanup: ReclaimFailure,
    },
}

impl core::fmt::Display for BuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Mapping(reason) => core::fmt::Display::fmt(reason, f),
            Self::Rollback { cause, cleanup } => {
                write!(f, "{cause}; address-space rollback refused: {cleanup}")
            }
        }
    }
}

fn rollback_build(mut space: AddressSpace, cause: MapError) -> BuildError {
    match space.reclaim(None) {
        Ok(_) => BuildError::Mapping(cause),
        Err(cleanup) => BuildError::Rollback { cause, cleanup },
    }
}

fn build(
    map: &MemoryMap,
    direct: DirectMap,
    image: ImageAddress,
) -> Result<AddressSpace, BuildError> {
    let mut space = AddressSpace::new().map_err(BuildError::Mapping)?;
    let result = (|| -> Result<(), MapError> {
        let sections = boot::sections();

        // Read-only and executable; read-only and not; writable and not. No
        // fourth combination exists, which is the whole of W^X.
        let classes = [
            (sections.text, Flags::NONE, "text"),
            (sections.rodata, Flags::NO_EXECUTE, "rodata"),
            (
                sections.data,
                Flags::WRITABLE.union(Flags::NO_EXECUTE),
                "data",
            ),
        ];
        let mut image_ranges = [(0, 0, Flags::NONE); 3];
        for ((section, flags, name), range) in classes.into_iter().zip(image_ranges.iter_mut()) {
            let physical = image
                .to_physical(section.start)
                .ok_or(MapError::Misaligned(section.start))?;
            let region = BootRegion::describe(physical, section.mapped_len())
                .ok_or(MapError::Undescribed(physical))?;
            space.map_boot(section.start, region, flags)?;
            debug!(
                "paging: {name} {:#x}..{:#x} -> {physical:#x} {flags:?}",
                section.start,
                section.start + section.mapped_len()
            );
            *range = (physical, section.mapped_len(), flags);
        }

        let mut mapped = 0;
        for run in map.page_runs() {
            let base = run.base();
            let end = base.checked_add(run.bytes()).ok_or(MapError::InvalidRange(
                MappingRangeError::PhysicalEndOverflow,
            ))?;
            let default_flags = Flags::WRITABLE.union(Flags::NO_EXECUTE).union(
                if run.cache() == DirectMapCache::Framebuffer {
                    Flags::WRITE_THROUGH
                } else {
                    Flags::NONE
                },
            );
            let mut cursor = base;
            while cursor < end {
                let image_range = image_ranges.iter().find_map(|(start, len, flags)| {
                    let image_end = start.checked_add(*len)?;
                    (*start <= cursor && cursor < image_end).then_some((image_end, *flags))
                });
                let (next, flags) = match image_range {
                    Some((image_end, flags)) => (end.min(image_end), flags),
                    None => {
                        let next_image = image_ranges
                            .iter()
                            .map(|(start, _, _)| *start)
                            .filter(|start| *start > cursor)
                            .min()
                            .unwrap_or(end);
                        (end.min(next_image), default_flags)
                    }
                };
                let virtual_base = direct
                    .virtual_address(cursor)
                    .ok_or(MapError::NonCanonical(cursor))?;
                let region = BootRegion::describe(cursor, next - cursor)
                    .ok_or(MapError::Undescribed(cursor))?;
                space.map_boot(virtual_base, region, flags)?;
                mapped += next - cursor;
                cursor = next;
            }
        }
        info!(
            "paging: direct map covers {} MiB at {:#x}",
            mapped / (1 << 20),
            direct.offset()
        );

        Ok(())
    })();

    match result {
        Ok(()) => Ok(space),
        Err(cause) => Err(rollback_build(space, cause)),
    }
}

/// A partially populated boot space returns its complete table ledger.
#[cfg(feature = "selftest")]
pub(crate) fn partial_build_failure_is_atomic_for_test() -> bool {
    const PROBE: u64 = 0xffff_9100_0000_0000;

    let Some(physical) = mm::first_memory_run(FRAME_SIZE) else {
        return false;
    };
    let attempt = || -> Option<bool> {
        let mut space = AddressSpace::new().ok()?;
        let region = BootRegion::describe(physical, FRAME_SIZE)?;
        space.map_boot(PROBE, region, Flags::NO_EXECUTE).ok()?;
        let mapped = space.lookup(PROBE).is_some();
        Some(
            mapped
                && matches!(
                    rollback_build(space, MapError::OutOfMemory),
                    BuildError::Mapping(MapError::OutOfMemory)
                ),
        )
    };

    if attempt() != Some(true) {
        return false;
    }
    let free_before = mm::free_frames();
    attempt() == Some(true) && mm::free_frames() == free_before
}
