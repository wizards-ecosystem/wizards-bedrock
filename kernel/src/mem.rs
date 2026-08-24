//! Physical memory: what the bootloader reported, and what the kernel took.
//!
//! The frame allocator itself lives in `wost::mm` — owning a frame needs a raw
//! pointer, so it is `wost`'s (D-43). What is here is the part that needs no
//! privilege: deciding whether the handoff is usable at all, and saying so.

use wost::arch::current::cpu;
use wost::arch::current::paging::{self, AddressSpace, BootRegion, MapError, MappedStack};
use wost::arch::current::segment;
use wost::boot::{self, DirectMap, HandoffError, ImageAddress, MemoryKind, MemoryMap};
use wost::mm;
use wost::sync::SpinLock;
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

/// Run `f` against the kernel's live address space, if there is one.
///
/// A token-bearing acquisition (D-85): `f` is arbitrary, so this is not a leaf,
/// and the borrow means nothing that can block is reachable from inside it. The
/// frame allocation a new table needs is a leaf underneath, which is exactly
/// the nesting the two levels exist to make sound.
///
pub(crate) fn with_active_space<T>(
    context: &mut ThreadContext,
    f: impl FnOnce(&mut AddressSpace) -> T,
) -> Option<T> {
    SPACE.lock(context).as_mut().map(f)
}

/// Move the double-fault handler onto a guarded stack.
///
/// `002`'s security baseline records two stacks with nothing below them: the
/// boot thread's, which is the bootloader's, and this one, a `.bss` array. This
/// is the half that can be fixed now — a region mapped after the switch, with
/// the page below it deliberately absent, and the handler pointed at it (D-89).
///
/// The first shipped caller of [`with_active_space`], and the reason D-86 is
/// load-bearing rather than a capability waiting for one.
fn guard_the_fault_stack() -> Result<u64, MapError> {
    let layout = thread::fault_stack_layout().ok_or(MapError::OutOfMemory)?;
    let slot = layout.slot(0).ok_or(MapError::OutOfMemory)?;

    // Through `map_stack` rather than by hand, so the guard below it is
    // *verified* absent at map time rather than only in a build that runs the
    // self-checks (D-94). Doing this by hand was the first version, and the
    // difference is exactly the evidence.
    let mut context = ThreadContext::require();
    let stack = with_active_space(&mut context, |space| space.map_stack(&slot))
        .ok_or(MapError::OutOfMemory)??;

    let top = stack.top();
    if !segment::set_double_fault_stack(&stack) {
        return Err(MapError::Misaligned(top));
    }
    Ok(top)
}

/// Map the boot thread's own guarded stack.
///
/// `None` rather than an error when it cannot be had: a kernel that keeps the
/// bootloader's stack is the kernel every earlier phase shipped, so this
/// declining is a lost guard rather than a lost boot.
pub(crate) fn map_boot_stack() -> Option<MappedStack> {
    let layout = thread::boot_stack_layout()?;
    let slot = layout.slot(0)?;
    let mut context = ThreadContext::require();
    match with_active_space(&mut context, |space| space.map_stack(&slot))? {
        Ok(stack) => Some(stack),
        Err(reason) => {
            warn!("boot: stack not mapped: {reason}");
            None
        }
    }
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
) -> Option<Routing> {
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

    // Claimed after the switch, mapped before it.
    let pending = interrupts.and_then(|interrupts| {
        match space.map_device(u64::from(interrupts.io_apic().address), IO_APIC_WINDOW) {
            Ok(mapping) => Some((mapping, interrupts)),
            Err(reason) => {
                warn!("ioapic: not mapped: {reason}; no interrupt routing");
                None
            }
        }
    });

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

    let (mapping, interrupts) = pending?;
    let registers = mapping.claim()?;
    match IoApic::take(registers, interrupts.io_apic().gsi_base) {
        Ok(io_apic) => Some(Routing {
            io_apic,
            interrupts,
        }),
        Err(reason) => {
            warn!("ioapic: registers unreadable: {reason}; no interrupt routing");
            None
        }
    }
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
        let end = region
            .base
            .saturating_add(region.len)
            .next_multiple_of(FRAME_SIZE);
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
