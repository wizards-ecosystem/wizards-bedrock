//! Physical memory: what the bootloader reported, and what the kernel took.
//!
//! The frame allocator itself lives in `wost::mm` — owning a frame needs a raw
//! pointer, so it is `wost`'s (D-43). What is here is the part that needs no
//! privilege: deciding whether the handoff is usable at all, and saying so.

use alloc::vec::Vec;
use stack_layout::StackLayout;
use wost::arch::current::cpu;
use wost::arch::current::paging::{
    self, AddressSpace, BootRegion, MapError, MappedStack, ReleasableDevice,
};
use wost::arch::current::segment;
use wost::boot::{self, DirectMap, HandoffError, ImageAddress, MemoryKind, MemoryMap};
use wost::mm;
use wost::sync::SpinLock;
use x86_apic::io;
use x86_paging::Flags;

use crate::acpi::Interrupts;
use crate::arch::current::ioapic::IoApic;
use crate::thread::ThreadContext;
use crate::{debug, info, thread, warn};

/// Bytes in a page frame.
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
/// D-45 made `activate` consume the space, so everything the kernel would ever
/// map had to be decided while the tables were still being built. D-86 ended
/// that; this is where the space lives afterwards.
static SPACE: SpinLock<Option<AddressSpace>> = SpinLock::new(None);

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
    with_active_space(context, |space| space.map_device(physical, len))
}

/// Remove one transient device mapping after its access owner is consumed.
pub(crate) fn unmap_device(
    context: &mut ThreadContext,
    region: paging::ReleasableDevice,
) -> Option<Result<(), MapError>> {
    with_active_space(context, |space| space.unmap_device(region))
}

/// Device extents currently owned by the running address space.
#[cfg(feature = "selftest")]
pub(crate) fn device_mappings(context: &mut ThreadContext) -> Option<usize> {
    with_active_space(context, |space| space.device_mappings())
}

/// Next unused byte in the running address space's device window.
#[cfg(feature = "selftest")]
pub(crate) fn device_window_next(context: &mut ThreadContext) -> Option<u64> {
    with_active_space(context, |space| space.device_window_next())
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

/// Add a user mapping to the running tables for the user-copy machine proof.
#[cfg(feature = "selftest")]
pub(crate) fn map_user_for_test(
    context: &mut ThreadContext,
    base: u64,
    run: mm::USegment,
    flags: Flags,
) -> Option<Result<(), MapError>> {
    with_active_space(context, |space| space.map_user(base, run, flags))
}

/// Map a guarded stack and retain the exact authority needed to remove it.
#[cfg(feature = "selftest")]
pub(crate) fn map_reclaimable_stack_for_test(
    context: &mut ThreadContext,
    layout: StackLayout,
) -> Option<Result<(MappedStack, ActiveMapping), MapError>> {
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
    })
}

/// Move the double-fault handler onto a guarded stack.
///
/// `002`'s security baseline records two stacks with nothing below them: the
/// boot thread's, which is the bootloader's, and this one, a `.bss` array. This
/// is the half that can be fixed now — a region mapped after the switch, with
/// the page below it deliberately absent, and the handler pointed at it (D-89).
///
/// The first shipped live mapping, and the reason D-86 is
/// load-bearing rather than a capability waiting for one.
fn guard_the_fault_stack() -> Result<u64, MapError> {
    let layout = thread::fault_stack_layout().ok_or(MapError::OutOfMemory)?;
    let slot = layout.slot(0).ok_or(MapError::OutOfMemory)?;

    // Through `map_stack` rather than by hand, so the guard below it is
    // *verified* absent at map time rather than only in a build that runs the
    // self-checks (D-94). Doing this by hand was the first version, and the
    // difference is exactly the evidence.
    let mut context = ThreadContext::require();
    let stack = with_active_space(&mut context, |space| space.map_pinned_stack(&slot))
        .ok_or(MapError::OutOfMemory)??;

    let top = stack.top();
    if !segment::set_double_fault_stack(&stack) {
        return Err(MapError::Misaligned(top));
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
        Err(reason) => {
            warn!("stacks: {:#x} not mapped: {reason}", slot.stack_base());
            None
        }
    }
}

/// Where the direct map starts, for the checks that ask what a space holds.
#[cfg(feature = "selftest")]
pub(crate) fn direct_map_base() -> u64 {
    mm::direct_map().map_or(0, |map| map.virtual_address(0).unwrap_or(0))
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
            region.base.saturating_add(region.len),
            region.kind
        );
    }
    if map.dropped() != 0 {
        warn!(
            "memory: {} regions did not fit the handoff; some memory is lost",
            map.dropped()
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
/// Nothing is fatal. A kernel that fails to build its own tables keeps running
/// on the bootloader's, which is exactly where Phase 2 left it, and says why.
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

    // ACPI is read *before* the tables are built. D-45 forced that while
    // mapping was one-way; D-86 lifted the force and left the order, because
    // the bootloader's direct map still covers the tables here (D-49).
    let interrupts = match rsdp {
        Ok(rsdp) => crate::acpi::interrupts(rsdp),
        Err(reason) => {
            warn!("acpi: {reason}; no interrupt routing");
            None
        }
    };

    let mut space = match build(&map, direct, image) {
        Ok(space) => space,
        Err(reason) => {
            warn!("paging: {reason}; staying on the bootloader's tables");
            return None;
        }
    };

    // Thread stacks, committed now because nothing can be mapped later
    // (D-45, D-52). The guards between them are the pages this never maps.
    let arena = match thread::layout().map(|layout| space.map_stacks(&layout)) {
        Some(Ok(arena)) => Some(arena),
        Some(Err(reason)) => {
            warn!("threads: stacks not mapped: {reason}; no threads");
            None
        }
        None => {
            warn!("threads: stack layout is invalid; no threads");
            None
        }
    };

    // Before the switch: the CR3 load is the flush the PAT rewrite needs.
    let write_combining = cpu::enable_write_combining();

    match paging::activate(&mut space) {
        Ok(()) => info!("paging: running on our own tables"),
        Err(reason) => {
            warn!("paging: refused to switch: {reason}");
            return None;
        }
    }

    // Kept rather than dropped, which is the whole of D-86: the tables stay
    // reachable, so a mapping can arrive with the thread or the process that
    // needs it instead of being guessed while they are being built.
    *SPACE.lock_leaf() = Some(space);

    match guard_the_fault_stack() {
        Ok(top) => info!("faults: double-fault stack guarded, top {top:#x}"),
        Err(reason) => warn!("faults: double-fault stack left unguarded: {reason}"),
    }

    if let Some(arena) = arena {
        match thread::init(arena) {
            Ok(slots) => info!("threads: {slots} guarded stacks ready"),
            Err(reason) => warn!("threads: {reason}; no threads"),
        }
    }

    if write_combining.is_some_and(|transition| transition.commit()) {
        info!("framebuffer: write-combining");
    }

    interrupts
}

/// Probe every described I/O APIC and retain the unique owner of `gsi`.
///
/// Called before interrupts are enabled. Every controller is therefore masked
/// before firmware state could deliver an unowned vector, while the active
/// address space lets all temporary mappings be returned after selection.
pub(crate) fn prepare_routing(
    context: &mut ThreadContext,
    interrupts: Interrupts,
    gsi: u32,
) -> Option<Routing> {
    let count = interrupts.io_apic_count();
    let mut controllers = Vec::new();
    let mut ranges = Vec::new();
    if controllers.try_reserve_exact(count).is_err() || ranges.try_reserve_exact(count).is_err() {
        warn!("ioapic: no memory for {count} routing candidates; staying polled");
        return None;
    }

    for description in interrupts.io_apics() {
        let Some(controller) = probe_io_apic(context, description) else {
            release_io_apics(context, &mut controllers);
            return None;
        };
        ranges.push(controller.range());
        controllers.push(controller);
    }

    let owner = match io::owner_of(&ranges, gsi) {
        Ok(Some(owner)) => owner,
        Ok(None) => {
            warn!("keyboard: no io apic owns gsi {gsi}; staying polled");
            release_io_apics(context, &mut controllers);
            return None;
        }
        Err(reason) => {
            warn!("ioapic: {reason}; staying polled");
            release_io_apics(context, &mut controllers);
            return None;
        }
    };
    let Some(selected) = interrupts.io_apics().nth(owner) else {
        warn!("ioapic: selected controller has no firmware description; staying polled");
        release_io_apics(context, &mut controllers);
        return None;
    };

    // Releasing in reverse restores the transient device-window cursor. The
    // owner is then mapped once in its final permanent position, rather than
    // leaving holes for every controller inspected before it.
    if !release_io_apics(context, &mut controllers) {
        warn!("ioapic: candidate mappings did not retire exactly; staying polled");
        return None;
    }
    let io_apic = probe_io_apic(context, selected)?.retain();
    Some(Routing {
        io_apic,
        interrupts,
    })
}

/// Map, mask and retain removal authority for one routing candidate.
fn probe_io_apic(
    context: &mut ThreadContext,
    description: acpi_tables::IoApic,
) -> Option<IoApic<ReleasableDevice>> {
    let mapping = match map_device(context, u64::from(description.address()), IO_APIC_WINDOW) {
        Some(Ok(mapping)) => mapping,
        Some(Err(reason)) => {
            warn!(
                "ioapic {}: registers at {:#x} not mapped: {reason}; staying polled",
                description.id(),
                description.address()
            );
            return None;
        }
        None => {
            warn!("ioapic: no active address space; staying polled");
            return None;
        }
    };
    let registers = match mapping.claim_releasable() {
        Ok(registers) => registers,
        Err(_) => {
            warn!(
                "ioapic {}: mapped registers are not live; staying polled",
                description.id()
            );
            return None;
        }
    };
    match IoApic::take(registers, description.gsi_base()) {
        Ok(controller) => Some(controller),
        Err(failure) => {
            let reason = failure.reason;
            let released = matches!(unmap_device(context, failure.registers), Some(Ok(())));
            warn!(
                "ioapic {}: registers unusable: {reason}; mapping returned={released}; staying polled",
                description.id()
            );
            None
        }
    }
}

/// Return candidate mappings in the order that restores their window cursor.
fn release_io_apics(
    context: &mut ThreadContext,
    controllers: &mut Vec<IoApic<ReleasableDevice>>,
) -> bool {
    let mut exact = true;
    while let Some(controller) = controllers.pop() {
        match unmap_device(context, controller.into_releasable()) {
            Some(Ok(())) => {}
            Some(Err(reason)) => {
                warn!("ioapic: candidate mapping not returned: {reason}");
                exact = false;
            }
            None => {
                warn!("ioapic: active address space disappeared during cleanup");
                exact = false;
            }
        }
    }
    exact
}

/// Bytes of register window an I/O APIC occupies. The index/data pair sits in
/// the first 32; a page is the smallest thing worth mapping.
const IO_APIC_WINDOW: u64 = 4096;

/// The interrupt controller ACPI described, ready to route lines.
#[derive(Debug)]
pub(crate) struct Routing {
    /// The controller itself.
    pub(crate) io_apic: IoApic,
    /// What ACPI said about which line goes where.
    pub(crate) interrupts: Interrupts,
}

/// Lay out the kernel's address space.
///
/// Two things go in it, and the order says why each is there. The image, split
/// by protection class so that W^X holds by construction rather than by audit
/// (D-12). Then the direct map, at the offset the bootloader already used —
/// which is what keeps every pointer the kernel is already holding valid
/// across the switch, the framebuffer's and the stack's among them.
fn build(
    map: &MemoryMap,
    direct: DirectMap,
    image: ImageAddress,
) -> Result<AddressSpace, MapError> {
    let mut space = AddressSpace::new()?;
    let sections = boot::sections();

    // Read-only and executable; read-only and not; writable and not. No fourth
    // combination exists, which is the whole of W^X.
    for (section, flags, name) in [
        (sections.text, Flags::NONE, "text"),
        (sections.rodata, Flags::NO_EXECUTE, "rodata"),
        (
            sections.data,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
            "data",
        ),
    ] {
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
    }

    let image_ranges = [
        (
            image
                .to_physical(sections.text.start)
                .ok_or(MapError::Misaligned(sections.text.start))?,
            sections.text.mapped_len(),
            Flags::NONE,
        ),
        (
            image
                .to_physical(sections.rodata.start)
                .ok_or(MapError::Misaligned(sections.rodata.start))?,
            sections.rodata.mapped_len(),
            Flags::NO_EXECUTE,
        ),
        (
            image
                .to_physical(sections.data.start)
                .ok_or(MapError::Misaligned(sections.data.start))?,
            sections.data.mapped_len(),
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        ),
    ];

    let mut mapped = 0;
    for region in map
        .regions()
        .filter(|region| region.kind.is_direct_mapped())
    {
        // Outward to whole pages: a region that starts mid-page still needs
        // that page reachable, and the direct map is a map of memory, not of
        // the bootloader's bookkeeping.
        let base = region.base & !(FRAME_SIZE - 1);
        // Checked, not `next_multiple_of`: the input is a `saturating_add` of
        // the bootloader's own numbers, and rounding a value near `u64::MAX`
        // up panics with overflow checks on — which the debug boot has. In
        // release it wraps to zero and `while cursor < end` skips the region
        // entirely, so the two builds disagree about what the direct map
        // covers (D-124).
        let end = region
            .base
            .saturating_add(region.len)
            .checked_next_multiple_of(FRAME_SIZE)
            .unwrap_or(u64::MAX);
        let default_flags = Flags::WRITABLE.union(Flags::NO_EXECUTE).union(
            if region.kind == MemoryKind::Framebuffer {
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

    Ok(space)
}
