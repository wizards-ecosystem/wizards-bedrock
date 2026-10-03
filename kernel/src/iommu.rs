//! The storage controller's DMA-remapping domain (D-357).
//!
//! Firmware's DMAR names the VT-d unit that governs a PCI function; this
//! module reads that table once at boot, then for each storage candidate maps
//! the governing unit's register window into the device window and hands it
//! to `wost`'s unit, which takes the hardware over and leaves translation on
//! with only firmware's reserved regions mapped, each refused first if it
//! reaches memory the kernel owns. The established unit lives in one
//! kernel-owned slot that every DMA seam locks and refuses without: queue
//! frames, transfer buffers and bus-master admission all go through it, and
//! the domain is released after the candidate's lease, so a retired candidate
//! restores every frame and mapping it took (D-199). When no unit can be
//! established the candidate is refused and the boot says `storage: no DMA
//! isolation`, because an unisolated controller is exactly the case the domain
//! exists to prevent.
//!
//! Nothing here allocates on the heap inside a candidate: a slab the heap grows
//! into never returns its frame, and the candidate baseline counts frames.

use acpi_tables::{Bdf, BusRange, Coverage, Dmar, Sdt};
use pci_config::Address;
use wost::arch::current::paging::{DeviceMapping, ReleasableDevice};
use wost::arch::current::vtd::{Release, Vtd};
use wost::mm;
use wost::sync::{SpinLock, ThreadLock, ThreadLockGuard};

use crate::mem::{self, DeviceRetirement};
use crate::thread::ThreadContext;
use crate::{acpi, info, pci, warn};

/// Bytes of register window one remapping unit occupies.
const REGISTER_WINDOW: u64 = 4096;

/// The PCI segment group the legacy configuration ports reach.
const SEGMENT: u16 = 0;

/// Reserved regions one function may need identity-mapped; more refuses it.
const RESERVED_LIMIT: usize = 8;

/// Firmware's validated DMAR, read once before any candidate exists and kept
/// for the rest of the boot.
static DMAR: SpinLock<Option<&'static [u8]>> = SpinLock::new(None);

/// The established unit, translation on: the candidate's while it runs and the
/// kept controller's once one is selected. A thread lock rather than a leaf
/// lock because the seams that take it run commands under the guard, which a
/// leaf guard forbids (D-121); it is the discipline the kept controller's own
/// lock already follows. A quarantined controller leaves its unit here, on,
/// for as long as the boot lasts.
static INSTALLED: ThreadLock<Option<Vtd>> = ThreadLock::new(None);

/// A register window whose exact unmap refused, kept owned (D-199).
static RETAINED_REGISTERS: SpinLock<Option<ReleasableDevice>> = SpinLock::new(None);

/// A register mapping the active root no longer showed, kept owned (D-199).
static RETAINED_MAPPING: SpinLock<Option<DeviceMapping>> = SpinLock::new(None);

/// Read and validate firmware's DMAR while the heap is still free to grow.
///
/// A missing or malformed table is logged here and every later candidate is
/// refused; nothing a candidate does can cure it.
pub(crate) fn discover(rsdp: u64) {
    let Some((firmware, root)) = acpi::parse_root(rsdp) else {
        return;
    };
    let Some(table) = acpi::find_table(firmware, root, Sdt::DMAR) else {
        warn!("iommu: firmware lists no dmar table");
        return;
    };
    let units = match Dmar::new(&table) {
        Ok(dmar) => dmar.unit_count(),
        Err(reason) => {
            warn!("iommu: {reason}");
            return;
        }
    };
    info!("iommu: dmar lists {units} remapping unit(s)");
    *DMAR.lock_leaf() = Some(table.leak());
}

/// The remapping unit firmware says governs one function.
struct Unit {
    register_base: u64,
    reserved: [(u64, u64); RESERVED_LIMIT],
    reserved_count: usize,
}

/// What the DMAR says about one function.
enum Lookup {
    NoTable,
    Ungoverned,
    TooManyReserved,
    Governed(Unit, Coverage),
}

fn look_up(bdf: Bdf) -> Lookup {
    // Copied out so the bridge reads below run with no guard held (D-121).
    let table = *DMAR.lock_leaf();
    let Some(table) = table else {
        return Lookup::NoTable;
    };
    let Ok(dmar) = Dmar::new(table) else {
        return Lookup::NoTable;
    };
    let Some(governing) = dmar.unit_for(bdf, bridge_buses) else {
        return Lookup::Ungoverned;
    };
    let mut reserved = [(0, 0); RESERVED_LIMIT];
    let mut reserved_count = 0;
    for region in dmar.reserved_regions_for(bdf, bridge_buses) {
        let Some(slot) = reserved.get_mut(reserved_count) else {
            return Lookup::TooManyReserved;
        };
        *slot = region;
        reserved_count += 1;
    }
    Lookup::Governed(
        Unit {
            register_base: governing.register_base,
            reserved,
            reserved_count,
        },
        governing.coverage,
    )
}

/// The buses behind the bridge at `at`, read live from configuration space.
///
/// `None` for an absent function, one that is not a bridge, or a segment the
/// configuration ports do not reach.
fn bridge_buses(at: Bdf) -> Option<BusRange> {
    if at.segment() != SEGMENT {
        return None;
    }
    let numbers = pci::bridge_numbers(at.bus(), at.device(), at.function())?;
    Some(BusRange {
        secondary: pci_config::secondary_bus(numbers),
        subordinate: pci_config::subordinate_bus(numbers),
    })
}

/// Say which DMAR scope tied the function to its unit.
fn report_coverage(at: Address, register_base: u64, coverage: Coverage) {
    let (bus, device, function) = (at.bus(), at.device(), at.function());
    match coverage {
        Coverage::Endpoint => info!(
            "iommu: {bus:02x}:{device:02x}.{function} is governed by the unit at {register_base:#x} through an endpoint scope"
        ),
        Coverage::Bridge(bridge) => info!(
            "iommu: {bus:02x}:{device:02x}.{function} is governed by the unit at {register_base:#x} through the bridge scope of {:02x}:{:02x}.{}",
            bridge.bus(),
            bridge.device(),
            bridge.function()
        ),
        Coverage::CatchAll => info!(
            "iommu: {bus:02x}:{device:02x}.{function} is governed by the catch-all unit at {register_base:#x}"
        ),
    }
}

fn unit_for(at: Address) -> Option<Unit> {
    let bdf = Bdf::new(SEGMENT, at.bus(), at.device(), at.function());
    match look_up(bdf) {
        Lookup::Governed(unit, coverage) => {
            report_coverage(at, unit.register_base, coverage);
            Some(unit)
        }
        Lookup::NoTable => {
            warn!("iommu: no validated dmar; every candidate stays unisolated");
            None
        }
        Lookup::Ungoverned => {
            warn!(
                "iommu: no remapping unit governs {:02x}:{:02x}.{}",
                at.bus(),
                at.device(),
                at.function()
            );
            None
        }
        Lookup::TooManyReserved => {
            warn!("iommu: more than {RESERVED_LIMIT} reserved regions name the function");
            None
        }
    }
}

/// Whether every region firmware reserves for the function stays clear of the
/// kernel's memory, which the domain would otherwise hand the device; the first
/// that does not is logged.
fn reserved_regions_are_clear(reserved: &[(u64, u64)]) -> bool {
    match reserved
        .iter()
        .find(|&&(base, limit)| !mm::reserved_region_is_eligible(base, limit))
    {
        Some(&(base, limit)) => {
            warn!("iommu: reserved region [{base:#x}, {limit:#x}] reaches memory the kernel owns");
            false
        }
        None => true,
    }
}

/// The screen `establish` applies, for the machine check that feeds it regions.
#[cfg(feature = "selftest")]
pub(crate) fn reserved_regions_are_clear_for_test(reserved: &[(u64, u64)]) -> bool {
    reserved_regions_are_clear(reserved)
}

/// The installed unit, or nothing: every DMA seam locks this and refuses
/// without it.
pub(crate) fn installed() -> ThreadLockGuard<'static, Option<Vtd>> {
    INSTALLED.lock()
}

/// Establish and install a DMA-remapping domain for the function at `at`.
///
/// False means the candidate must not bus-master: no unit governs it, a
/// reserved region it needs reaches the kernel's memory or would not map, or
/// its unit could not be mapped or taken over. Every refusal has already been
/// logged, and a register window whose return refused is retained rather than
/// leaked.
pub(crate) fn establish(context: &mut ThreadContext, at: Address) -> bool {
    let Some(unit) = unit_for(at) else {
        return false;
    };
    let reserved = &unit.reserved[..unit.reserved_count];
    if !reserved_regions_are_clear(reserved) {
        return false;
    }
    let mapping = match mem::map_device(context, unit.register_base, REGISTER_WINDOW) {
        Some(Ok(mapping)) => mapping,
        Some(Err(reason)) => {
            warn!(
                "iommu: registers at {:#x} not mapped: {reason}",
                unit.register_base
            );
            return false;
        }
        None => {
            warn!("iommu: no active address space for the unit's registers");
            return false;
        }
    };
    let registers = match mapping.claim_releasable() {
        Ok(registers) => registers,
        Err(mapping) => {
            warn!("iommu: the mapped registers are absent from the active root");
            retain_mapping(mapping);
            return false;
        }
    };
    let mut isolation = match Vtd::establish(registers, at, reserved) {
        Ok(isolation) => isolation,
        Err((reason, registers)) => {
            warn!(
                "iommu: the unit at {:#x} refused takeover: {reason}",
                unit.register_base
            );
            if let DeviceRetirement::Retained(registers) =
                mem::unmap_device_exact(context, registers)
            {
                retain_registers(registers);
            }
            return false;
        }
    };
    for &(base, limit) in reserved {
        info!("iommu: reserved region [{base:#x}, {limit:#x}] stays reachable");
    }
    let stray = isolation.drain_faults();
    if stray != 0 {
        warn!("iommu: {stray} fault(s) were recorded before any segment was mapped");
    }
    let facts = isolation.facts();
    info!(
        "iommu: translation on for {:02x}:{:02x}.{} through the unit at {:#x}: {}-level walks{}{}{}, cap {:#x}, ecap {:#x}",
        at.bus(),
        at.device(),
        at.function(),
        unit.register_base,
        facts.levels,
        if facts.coherent {
            ""
        } else {
            ", flushed tables"
        },
        if facts.caching_mode {
            ", caching mode"
        } else {
            ""
        },
        if facts.write_buffer_flush {
            ", write-buffer flush"
        } else {
            ""
        },
        facts.capability,
        facts.extended
    );
    let mut installed = INSTALLED.lock();
    assert!(
        installed.is_none(),
        "a storage candidate began while a DMA domain was installed"
    );
    *installed = Some(isolation);
    true
}

/// Switch the candidate's translation off and return its register window,
/// after the function it isolated has stopped bus-mastering.
///
/// True when nothing was installed or everything came back. False means the
/// unit or its window had to be retained instead, so the caller halts
/// selection rather than carry on beside a leaked owner; a unit whose
/// translation will not switch off stays installed, on.
pub(crate) fn release_candidate(context: &mut ThreadContext) -> bool {
    let mut installed = INSTALLED.lock();
    let Some(isolation) = installed.take() else {
        return true;
    };
    let registers = match isolation.release() {
        Release::Released(registers) => registers,
        Release::Retained { isolation, reason } => {
            warn!("iommu: translation stays on: {reason}");
            *installed = Some(isolation);
            return false;
        }
    };
    drop(installed);
    match mem::unmap_device_exact(context, registers) {
        DeviceRetirement::Retired => true,
        DeviceRetirement::Retained(registers) => {
            warn!("iommu: the register window remained owned during cleanup");
            retain_registers(registers);
            false
        }
    }
}

/// Report and clear every fault the installed unit has recorded.
pub(crate) fn report_faults() {
    let mut installed = INSTALLED.lock();
    let Some(isolation) = installed.as_mut() else {
        return;
    };
    while let Some(fault) = isolation.take_fault() {
        warn!(
            "iommu: source {:#06x} {} {:#x}, reason {:#04x}",
            fault.source,
            if fault.write { "wrote" } else { "read" },
            fault.address,
            fault.reason
        );
    }
}

fn retain_mapping(mapping: DeviceMapping) {
    let mut retained = RETAINED_MAPPING.lock_leaf();
    assert!(
        retained.is_none(),
        "storage selection continued after a unit's mapping was quarantined"
    );
    *retained = Some(mapping);
}

fn retain_registers(registers: ReleasableDevice) {
    let mut retained = RETAINED_REGISTERS.lock_leaf();
    assert!(
        retained.is_none(),
        "storage selection continued after a unit's registers were quarantined"
    );
    *retained = Some(registers);
}
