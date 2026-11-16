use core::fmt;

use stack_layout::StackLayout;
use wost::arch::current::cpu;
#[cfg(feature = "selftest")]
use wost::arch::current::paging::UserMapError;
use wost::arch::current::paging::{
    self, AddressSpace, BarMapFailure, BarUnmapOutcome, BootRegion, DeviceUnmapFailure, MapError,
    MappedStack, ReclaimFailure, ReleasableDevice, StackArena, StackMapError,
};
use wost::arch::current::pci as platform_pci;
use wost::arch::current::segment;
use wost::boot::{
    self, DirectMap, DirectMapCache, HandoffError, ImageAddress, MemoryKind, MemoryMap,
};
use wost::mm;
use wost::sync::SpinLock;
use x86_paging::{Flags, MappingRangeError};

use crate::acpi::Interrupts;
use crate::iommu::EarlyDmaBoundary;
use crate::thread::ThreadContext;
use crate::{debug, info, pci, thread, warn};

#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
pub(crate) use selftest::{map_user_page_for_test, unmap_for_test};

pub(crate) fn init(
    map: Result<MemoryMap, HandoffError>,
    direct: Result<DirectMap, HandoffError>,
    _dma: EarlyDmaBoundary,
) {
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

static SPACE: SpinLock<Option<AddressSpace>> = SpinLock::new(None);

// Private: `AddressSpace` also carries reclaim and unmap authority over the live tables.
fn with_active_space<T>(
    context: &mut ThreadContext,
    operation: impl FnOnce(&mut AddressSpace) -> T,
) -> Option<T> {
    SPACE.lock(context).as_mut().map(operation)
}

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

#[derive(Debug)]
pub(crate) enum PciBarMapFailure {
    NoSpace(platform_pci::AdmittedBar),
    Mapping(BarMapFailure),
}

impl PciBarMapFailure {
    pub(crate) const fn reason(&self) -> Option<MapError> {
        match self {
            Self::NoSpace(_) => None,
            Self::Mapping(failure) => Some(failure.reason),
        }
    }

    pub(crate) fn into_admitted(self) -> platform_pci::AdmittedBar {
        match self {
            Self::NoSpace(admitted) => admitted,
            Self::Mapping(failure) => failure.admitted,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[must_use = "a quarantined BAR still owns exact cleanup authority"]
pub(crate) enum PciBarRetirement {
    Retired,
    Quarantined,
}

pub(crate) fn unmap_pci_bar_exact(
    context: &mut ThreadContext,
    bar: platform_pci::ReleasableBar,
) -> PciBarRetirement {
    let mut active = SPACE.lock(context);
    let Some(space) = active.as_mut() else {
        drop(active);
        pci::retain_bar(pci::BarQuarantine::Releasable { _bar: bar });
        return PciBarRetirement::Quarantined;
    };
    let unmapped = unmap_pci_bar_twice(space, bar);
    drop(active);
    release_unmapped_pci_bar(unmapped)
}

#[derive(Debug)]
#[must_use = "a refused mapped-BAR retirement returns its access owner"]
pub(crate) enum MappedPciBarRetirement {
    Retired,
    Retained(platform_pci::MappedBar),
    Quarantined,
}

pub(crate) fn unmap_mapped_pci_bar_exact(
    context: &mut ThreadContext,
    bar: platform_pci::MappedBar,
) -> MappedPciBarRetirement {
    let mut active = SPACE.lock(context);
    let Some(space) = active.as_mut() else {
        return MappedPciBarRetirement::Retained(bar);
    };
    let unmapped = unmap_pci_bar_twice(space, bar.into_releasable());
    drop(active);
    match release_unmapped_pci_bar(unmapped) {
        PciBarRetirement::Retired => MappedPciBarRetirement::Retired,
        PciBarRetirement::Quarantined => MappedPciBarRetirement::Quarantined,
    }
}

fn unmap_pci_bar_twice(
    space: &mut AddressSpace,
    bar: platform_pci::ReleasableBar,
) -> BarUnmapOutcome {
    match space.unmap_pci_bar(bar) {
        BarUnmapOutcome::Refused(first) => space.unmap_pci_bar(first.bar),
        unmapped => unmapped,
    }
}

fn release_unmapped_pci_bar(unmapped: BarUnmapOutcome) -> PciBarRetirement {
    let authority = match unmapped {
        BarUnmapOutcome::Unmapped(authority) => authority,
        BarUnmapOutcome::Refused(failure) => {
            pci::retain_bar(pci::BarQuarantine::Releasable { _bar: failure.bar });
            return PciBarRetirement::Quarantined;
        }
    };
    match platform_pci::release_memory_bar(authority) {
        Ok(()) => PciBarRetirement::Retired,
        Err(failure) => {
            pci::retain_bar_authority(failure);
            PciBarRetirement::Quarantined
        }
    }
}

#[derive(Debug)]
#[must_use = "every refusal returns the exact device-mapping owner"]
enum DeviceUnmapOutcome {
    Unmapped,
    Refused(DeviceUnmapFailure),
    NoSpace(ReleasableDevice),
}

fn unmap_device(
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

#[derive(Debug)]
#[must_use = "a refused device retirement returns its exact mapping owner"]
pub(crate) enum DeviceRetirement {
    Retired,
    Retained(ReleasableDevice),
}

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

// Mapped after activation so its guard is truly absent; pinned, because the TSS points at it.
fn guard_the_fault_stack() -> Result<u64, StackMapError> {
    let layout = thread::fault_stack_layout().ok_or(MapError::OutOfMemory)?;
    let slot = layout.slot(0).ok_or(MapError::OutOfMemory)?;

    let mut context = ThreadContext::require();
    let stack = with_active_space(&mut context, |space| space.map_pinned_stack(&slot))
        .ok_or(MapError::OutOfMemory)??;

    let top = stack.top();
    if !segment::set_double_fault_stack(&stack) {
        panic!("a newly pinned stack must belong to the active address space");
    }
    Ok(top)
}

pub(crate) fn map_boot_stack() -> Option<MappedStack> {
    let mut context = ThreadContext::require();
    map_guarded_stack(&mut context, thread::boot_stack_layout()?)
}

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

// The pinned `limine` crate has no name for base revision 4's region type; it logs as unknown.
fn describe(map: &MemoryMap) {
    for region in map.regions() {
        debug!(
            "memory: {:#016x}..{:#016x} {}",
            region.base,
            region.base.saturating_add(region.len),
            region.kind
        );
    }
    if let Some(kind) = map.regions().map(|region| region.kind).find(unrecognised) {
        info!("memory: bootloader reported {kind}");
    }
}

fn unrecognised(kind: &MemoryKind) -> bool {
    matches!(kind, MemoryKind::Other(_))
}

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

    // Read ACPI while the bootloader's direct map still covers it, before building tables.
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
    let arena = map_thread_stacks(&mut space)?;

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

    let thread_slots = thread::start_threads(&mut space, arena);

    *SPACE.lock_leaf() = Some(space);

    report_guarded_paging(thread_slots, write_combining);
    interrupts
}

fn map_thread_stacks(space: &mut AddressSpace) -> Option<StackArena> {
    let Some(layout) = thread::layout() else {
        let cleanup = space.reclaim(None);
        assert!(
            cleanup.is_ok(),
            "an uninstalled address space must remain reclaimable"
        );
        warn!("threads: stack layout is invalid; staying on the bootloader's tables");
        return None;
    };
    match space.map_stacks(&layout) {
        Ok(arena) => Some(arena),
        Err(StackMapError::Mapping(reason)) => {
            let cleanup = space.reclaim(None);
            assert!(
                cleanup.is_ok(),
                "a failed stack transaction must leave no owned mapping"
            );
            warn!("threads: stacks not mapped: {reason}; no threads");
            None
        }
        Err(reason @ StackMapError::Rollback { .. }) => {
            panic!("threads: {reason}");
        }
        Err(reason) => panic!("threads: {reason}"),
    }
}

fn report_guarded_paging(
    thread_slots: Option<usize>,
    write_combining: Option<cpu::WriteCombining>,
) {
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
}

#[derive(Debug)]
enum BuildError {
    Mapping(MapError),
    Rollback {
        cause: MapError,
        cleanup: ReclaimFailure,
    },
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mapping(reason) => fmt::Display::fmt(reason, formatter),
            Self::Rollback { cause, cleanup } => {
                write!(
                    formatter,
                    "{cause}; address-space rollback refused: {cleanup}"
                )
            }
        }
    }
}

impl core::error::Error for BuildError {}

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
    let mut image_ranges = [(0, 0, Flags::NONE); 9];
    let result = map_image(&mut space, image, &mut image_ranges)
        .and_then(|()| map_direct(&mut space, map, direct, &image_ranges));

    match result {
        Ok(()) => Ok(space),
        Err(cause) => Err(rollback_build(space, cause)),
    }
}

fn map_image(
    space: &mut AddressSpace,
    image: ImageAddress,
    ranges: &mut [(u64, u64, Flags); 9],
) -> Result<(), MapError> {
    let sections = boot::sections();

    // Read-only executable, read-only, writable: no fourth combination exists, which is W^X.
    let enclave = sections.entry_enclave;
    let classes = [
        (sections.text, Flags::NONE, "text"),
        (sections.rodata, Flags::NO_EXECUTE, "rodata"),
        (
            sections.data,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
            "data",
        ),
        (enclave.text, Flags::NONE, "entry text"),
        (enclave.rodata, Flags::NO_EXECUTE, "entry rodata"),
        (
            enclave.tables,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
            "entry tables",
        ),
        (
            enclave.route,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
            "entry route",
        ),
        (
            enclave.transition_stack,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
            "entry transition stack",
        ),
        (
            enclave.ist_stack,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
            "entry IST stack",
        ),
    ];
    for ((section, flags, name), range) in classes.into_iter().zip(ranges.iter_mut()) {
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
    Ok(())
}

fn map_direct(
    space: &mut AddressSpace,
    map: &MemoryMap,
    direct: DirectMap,
    image_ranges: &[(u64, u64, Flags); 9],
) -> Result<(), MapError> {
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
            let region =
                BootRegion::describe(cursor, next - cursor).ok_or(MapError::Undescribed(cursor))?;
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
}
