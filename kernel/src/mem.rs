//! Physical memory: what the bootloader reported, and what the kernel took.
//!
//! The frame allocator itself lives in `wost::mm` — owning a frame needs a raw
//! pointer, so it is `wost`'s (D-43). What is here is the part that needs no
//! privilege: deciding whether the handoff is usable at all, and saying so.

use wost::arch::current::cpu;
use wost::arch::current::paging::{self, AddressSpace, DeviceMapping, MapError};
use wost::boot::{self, DirectMap, HandoffError, ImageAddress, MemoryKind, MemoryMap};
use wost::mm;
use x86_paging::Flags;

use crate::acpi::Interrupts;
use crate::arch::current::ioapic::IoApic;
use crate::{debug, dev, info, warn};

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

    // ACPI is read *before* the tables are built, and the ordering is forced:
    // D-45 made mapping one-way, so a device's window has to be known while
    // the tables are still being written. The tables are readable now because
    // the bootloader's direct map still covers them (D-49).
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

    // The framebuffer, mapped again by us so it can be write-combining. The
    // direct map already covers it write-back; this is a second mapping of the
    // same pixels with a memory type that suits a device nobody reads (D-50).
    let framebuffer = dev::framebuffer_extent(direct).and_then(|(physical, len)| {
        match space.map_device_write_combining(physical, len) {
            Ok(mapping) => Some(mapping),
            Err(reason) => {
                warn!("framebuffer: not remapped: {reason}; staying write-back");
                None
            }
        }
    });

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

    match paging::activate(space) {
        Ok(()) => info!("paging: running on our own tables"),
        Err(reason) => {
            warn!("paging: refused to switch: {reason}");
            return None;
        }
    }

    if cpu::enable_write_combining()
        && let Some(region) = framebuffer.and_then(DeviceMapping::claim)
    {
        dev::remap_framebuffer(region);
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
        space.map(section.start, physical, section.mapped_len(), flags)?;
        debug!(
            "paging: {name} {:#x}..{:#x} -> {physical:#x} {flags:?}",
            section.start,
            section.start + section.mapped_len()
        );
    }

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
        let Some(virtual_base) = direct.virtual_address(base) else {
            continue;
        };
        space.map(
            virtual_base,
            base,
            end - base,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )?;
        mapped += end - base;
    }
    info!(
        "paging: direct map covers {} MiB at {:#x}",
        mapped / (1 << 20),
        direct.offset()
    );

    Ok(space)
}
