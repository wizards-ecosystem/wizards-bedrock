//! The storage controller's DMA-remapping domain (D-357).
//!
//! Firmware's DMAR names the VT-d unit that governs a PCI function; this
//! module reads that table once at boot, then for each storage candidate proves
//! a complete bounded inventory of the live PCI requesters governed by the
//! exact unit. The candidate is already BME-off; every sibling requester is
//! exclusively claimed and fenced BME-off before `wost` takes over the
//! unit-wide registers. The installed owner keeps those fences together with
//! every table and register mapping until observed release, and uncertain
//! transition or owner return retains the whole boundary. Only firmware's
//! eligible reserved regions begin mapped, each refused first if it reaches
//! memory the kernel owns. Queue frames, transfer buffers and bus-master
//! admission all go through the installed domain, while candidate retirement
//! still restores every ordinary frame and mapping it took (D-199, D-370).
//! When no complete unit can be established, the candidate is refused and the
//! boot says `storage: no DMA isolation`, because an unisolated requester is
//! exactly the case the domain exists to prevent.
//!
//! Nothing here allocates on the heap inside a candidate: a slab the heap grows
//! into never returns its frame, and the candidate baseline counts frames.

use acpi_tables::{Bdf, BusRange, Dmar, RequesterInventory, RequesterInventoryError, Sdt};
use core::ops::{Deref, DerefMut};
#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicUsize, Ordering};
use pci_config::Address;
use wost::arch::current::paging::{DeviceMapping, ReleasableDevice};
use wost::arch::current::vtd::{Establishment, Release, Vtd};
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

/// The established unit: the candidate's while it runs and the kept
/// controller's once one is selected. A thread lock rather than a leaf
/// lock because the seams that take it run commands under the guard, which a
/// leaf guard forbids (D-121); it is the discipline the kept controller's own
/// lock already follows. A quarantined transition leaves its complete unit
/// owner here for as long as the boot lasts without guessing whether TES is
/// on or off.
static INSTALLED: ThreadLock<Option<InstalledUnit>> = ThreadLock::new(None);

/// A register window whose exact unmap refused, kept owned (D-199).
static RETAINED_REGISTERS: SpinLock<Option<ReleasableDevice>> = SpinLock::new(None);

/// A register mapping the active root no longer showed, kept owned (D-199).
static RETAINED_MAPPING: SpinLock<Option<DeviceMapping>> = SpinLock::new(None);

/// Requester fences whose exact return refused after the unit was settled.
static RETAINED_FENCES: SpinLock<Option<UnitFences>> = SpinLock::new(None);

/// The opaque function and command authorities behind one unit-wide fence.
///
/// Keeping these large linear owners in a single fixed-capacity vault avoids
/// materializing the whole array at every unoptimized move boundary. The
/// private [`UnitFences`] token is still the only authority that may operate on
/// occupied slots; dropping it leaks safe claims rather than making them
/// available without an observed return.
static UNIT_FENCE_SLOTS: SpinLock<[Option<pci::DmaFence>; pci::FUNCTION_AUTHORITY_LIMIT]> =
    SpinLock::new([const { None }; pci::FUNCTION_AUTHORITY_LIMIT]);

/// Last complete live requester inventory installed by a successful takeover.
#[cfg(feature = "selftest")]
static INSTALLED_REQUESTERS: AtomicUsize = AtomicUsize::new(0);

/// Successful unit takeovers, including the candidate replaced by fallback.
#[cfg(feature = "selftest")]
static SUCCESSFUL_TAKEOVERS: AtomicUsize = AtomicUsize::new(0);

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
    requesters: RequesterInventory<{ pci::FUNCTION_AUTHORITY_LIMIT }>,
}

/// Every non-selected requester kept BME-off while its DRHD may transition.
#[derive(Debug)]
struct UnitFences {
    count: usize,
}

impl UnitFences {
    fn new() -> Self {
        assert!(
            UNIT_FENCE_SLOTS.lock_leaf().iter().all(Option::is_none),
            "storage selection continued while requester fence authorities remained vaulted"
        );
        Self { count: 0 }
    }

    fn push(&mut self, fence: pci::DmaFence) {
        let mut slots = UNIT_FENCE_SLOTS.lock_leaf();
        let Some(slot) = slots.get_mut(self.count) else {
            panic!("a validated requester inventory exceeded its fence capacity");
        };
        assert!(
            slot.is_none(),
            "a requester fence authority slot was occupied twice"
        );
        *slot = Some(fence);
        self.count += 1;
    }

    /// Return whether every fence settled and the exact remainder, kept in a
    /// fixed vault so an ownership-carrying refusal needs neither heap
    /// indirection nor a pathologically large stack value.
    fn release(self) -> (bool, Self) {
        let mut retained_count = 0;
        for index in 0..self.count {
            let fence = {
                let mut slots = UNIT_FENCE_SLOTS.lock_leaf();
                let Some(fence) = slots[index].take() else {
                    panic!("a live requester fence token named an empty vault slot");
                };
                fence
            };
            if let Err(failure) = fence.release() {
                warn!("iommu: requester fence return refused: {}", failure.reason);
                let mut slots = UNIT_FENCE_SLOTS.lock_leaf();
                let slot = &mut slots[retained_count];
                assert!(
                    slot.is_none(),
                    "a retained requester fence overwrote another authority"
                );
                *slot = Some(failure.fence);
                retained_count += 1;
            }
        }
        (
            retained_count == 0,
            Self {
                count: retained_count,
            },
        )
    }
}

/// The unit-wide owner: hardware tables and every sibling command fence stay
/// inseparable through takeover, steady state and release.
#[derive(Debug)]
pub(crate) struct InstalledUnit {
    isolation: Vtd,
    fences: UnitFences,
}

impl Deref for InstalledUnit {
    type Target = Vtd;

    fn deref(&self) -> &Self::Target {
        &self.isolation
    }
}

impl DerefMut for InstalledUnit {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.isolation
    }
}

/// What the DMAR says about one function.
enum LookupError {
    NoTable,
    UnsafeInventory(RequesterInventoryError),
    TooManyReserved,
}

fn look_up(bdf: Bdf) -> Result<Unit, LookupError> {
    // Copied out so the bridge reads below run with no guard held (D-121).
    let table = *DMAR.lock_leaf();
    let Some(table) = table else {
        return Err(LookupError::NoTable);
    };
    let Ok(dmar) = Dmar::new(table) else {
        return Err(LookupError::NoTable);
    };
    let requesters = match dmar.requester_inventory::<{ pci::FUNCTION_AUTHORITY_LIMIT }>(
        bdf,
        bridge_buses,
        requester_is_present,
    ) {
        Ok(requesters) => requesters,
        Err(reason) => return Err(LookupError::UnsafeInventory(reason)),
    };
    let governing = requesters.unit();
    let mut reserved = [(0, 0); RESERVED_LIMIT];
    let mut reserved_count = 0;
    for region in dmar.reserved_regions_for(bdf, bridge_buses) {
        let Some(slot) = reserved.get_mut(reserved_count) else {
            return Err(LookupError::TooManyReserved);
        };
        *slot = region;
        reserved_count += 1;
    }
    Ok(Unit {
        register_base: governing.register_base,
        reserved,
        reserved_count,
        requesters,
    })
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

/// Whether one exact requester ID currently answers configuration reads.
///
/// Hierarchy inventories ask every function number independently; unlike the
/// ordinary topology walk, this proof does not trust a device's multifunction
/// bit to omit possible DMA sources.
fn requester_is_present(at: Bdf) -> bool {
    at.segment() == SEGMENT && pci::function_is_present(at.bus(), at.device(), at.function())
}

fn unit_for(at: Address) -> Option<Unit> {
    let bdf = Bdf::new(SEGMENT, at.bus(), at.device(), at.function());
    match look_up(bdf) {
        Ok(unit) => {
            info!(
                "iommu: {:02x}:{:02x}.{} is one of {} live PCI requester(s) governed by the unit at {:#x}",
                at.bus(),
                at.device(),
                at.function(),
                unit.requesters.len(),
                unit.register_base
            );
            Some(unit)
        }
        Err(LookupError::NoTable) => {
            warn!("iommu: no validated dmar; every candidate stays unisolated");
            None
        }
        Err(LookupError::UnsafeInventory(reason)) => {
            warn!(
                "iommu: no complete requester inventory governs {:02x}:{:02x}.{}: {reason}",
                at.bus(),
                at.device(),
                at.function()
            );
            None
        }
        Err(LookupError::TooManyReserved) => {
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

/// Whether the candidate acquired a complete unit-wide DMA boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EstablishOutcome {
    Established,
    Unavailable,
    Quarantined,
}

enum FenceAcquisition {
    Ready(UnitFences),
    Refused,
    Retained(UnitFences),
}

fn acquire_fences(
    requesters: &RequesterInventory<{ pci::FUNCTION_AUTHORITY_LIMIT }>,
    selected: &pci::Device,
) -> FenceAcquisition {
    let selected_at = selected.at().function_base();
    let mut fences = UnitFences::new();
    for requester in requesters.iter() {
        let Some(at) = Address::new(requester.bus(), requester.device(), requester.function(), 0)
        else {
            warn!("iommu: an inventoried function has an invalid PCI requester ID");
            return release_refused_fences(fences);
        };
        if at == selected_at {
            continue;
        }
        match pci::fence_dma_requester(at) {
            pci::DmaFenceOutcome::Fenced(fence) => fences.push(fence),
            pci::DmaFenceOutcome::Refused(reason) => {
                warn!(
                    "iommu: requester {:02x}:{:02x}.{} could not be fenced: {reason}",
                    at.bus(),
                    at.device(),
                    at.function()
                );
                return release_refused_fences(fences);
            }
            pci::DmaFenceOutcome::Retained { reason, fence } => {
                warn!(
                    "iommu: requester {:02x}:{:02x}.{} has uncertain fence state: {reason}",
                    at.bus(),
                    at.device(),
                    at.function()
                );
                fences.push(fence);
                return FenceAcquisition::Retained(fences);
            }
        }
    }
    if let Err(reason) = selected.verify_bus_master_fence() {
        warn!("iommu: selected requester lost its bus-master fence: {reason}");
        return FenceAcquisition::Retained(fences);
    }
    FenceAcquisition::Ready(fences)
}

fn release_refused_fences(fences: UnitFences) -> FenceAcquisition {
    let (released, fences) = fences.release();
    if released {
        FenceAcquisition::Refused
    } else {
        FenceAcquisition::Retained(fences)
    }
}

/// The installed unit, or nothing: every DMA seam locks this and refuses
/// without it.
pub(crate) fn installed() -> ThreadLockGuard<'static, Option<InstalledUnit>> {
    INSTALLED.lock()
}

/// Establish and install a unit-wide DMA-remapping owner for `device`.
pub(crate) fn establish(context: &mut ThreadContext, device: &pci::Device) -> EstablishOutcome {
    let at = device.at();
    let Some(unit) = unit_for(at) else {
        return EstablishOutcome::Unavailable;
    };
    let Unit {
        register_base,
        reserved,
        reserved_count,
        requesters,
    } = unit;
    let reserved = &reserved[..reserved_count];
    if !reserved_regions_are_clear(reserved) {
        return EstablishOutcome::Unavailable;
    }
    let fences = match acquire_fences(&requesters, device) {
        FenceAcquisition::Ready(fences) => fences,
        FenceAcquisition::Refused => return EstablishOutcome::Unavailable,
        FenceAcquisition::Retained(fences) => {
            retain_fences(fences);
            return EstablishOutcome::Quarantined;
        }
    };
    let mapping = match mem::map_device(context, register_base, REGISTER_WINDOW) {
        Some(Ok(mapping)) => mapping,
        Some(Err(reason)) => {
            warn!("iommu: registers at {register_base:#x} not mapped: {reason}");
            return release_unavailable_fences(fences);
        }
        None => {
            warn!("iommu: no active address space for the unit's registers");
            return release_unavailable_fences(fences);
        }
    };
    let registers = match mapping.claim_releasable() {
        Ok(registers) => registers,
        Err(mapping) => {
            warn!("iommu: the mapped registers are absent from the active root");
            retain_mapping(mapping);
            retain_fences_after_failure(fences);
            return EstablishOutcome::Quarantined;
        }
    };
    let mut isolation = match Vtd::establish(registers, at, reserved) {
        Establishment::Established(isolation) => isolation,
        Establishment::Refused { reason, registers } => {
            warn!("iommu: the unit at {register_base:#x} refused takeover: {reason}");
            let registers_returned = match mem::unmap_device_exact(context, registers) {
                DeviceRetirement::Retired => true,
                DeviceRetirement::Retained(registers) => {
                    retain_registers(registers);
                    false
                }
            };
            let fences_returned = release_fences(fences);
            return if registers_returned && fences_returned {
                EstablishOutcome::Unavailable
            } else {
                EstablishOutcome::Quarantined
            };
        }
        Establishment::Retained { reason, isolation } => {
            warn!("iommu: the unit at {register_base:#x} has uncertain takeover state: {reason}");
            install(InstalledUnit { isolation, fences });
            return EstablishOutcome::Quarantined;
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
        register_base,
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
    #[cfg(feature = "selftest")]
    {
        INSTALLED_REQUESTERS.store(requesters.len(), Ordering::Release);
        SUCCESSFUL_TAKEOVERS.fetch_add(1, Ordering::AcqRel);
    }
    install(InstalledUnit { isolation, fences });
    EstablishOutcome::Established
}

/// Whether the default machine established two successive owners while the
/// installed owner still holds one fence for every non-selected requester.
#[cfg(feature = "selftest")]
pub(crate) fn requester_fences_cover_shared_unit_for_test() -> bool {
    if DMAR.lock_leaf().is_none() {
        return true;
    }
    let requesters = INSTALLED_REQUESTERS.load(Ordering::Acquire);
    let installed = INSTALLED.lock();
    requesters > 1
        && SUCCESSFUL_TAKEOVERS.load(Ordering::Acquire) >= 2
        && installed
            .as_ref()
            .is_some_and(|unit| unit.fences.count + 1 == requesters)
        && pci::claims() == requesters
}

fn install(unit: InstalledUnit) {
    let mut installed = INSTALLED.lock();
    assert!(
        installed.is_none(),
        "a storage candidate began while a DMA domain was installed"
    );
    *installed = Some(unit);
}

fn release_unavailable_fences(fences: UnitFences) -> EstablishOutcome {
    if release_fences(fences) {
        EstablishOutcome::Unavailable
    } else {
        EstablishOutcome::Quarantined
    }
}

fn release_fences(fences: UnitFences) -> bool {
    let (released, fences) = fences.release();
    if !released {
        retain_fences(fences);
    }
    released
}

fn retain_fences_after_failure(fences: UnitFences) {
    let (released, fences) = fences.release();
    if !released {
        retain_fences(fences);
    }
}

/// Switch the candidate's translation off and return its register window,
/// after the function it isolated has stopped bus-mastering.
///
/// True when nothing was installed or everything came back. False means the
/// unit or its window had to be retained instead, so the caller halts
/// selection rather than carry on beside a leaked owner. A failed observation
/// after the disable write makes unit state uncertain, so the complete owner
/// remains installed without claiming whether TES is on or off.
pub(crate) fn release_candidate(context: &mut ThreadContext) -> bool {
    let mut installed = INSTALLED.lock();
    let Some(unit) = installed.take() else {
        return true;
    };
    let InstalledUnit { isolation, fences } = unit;
    let registers = match isolation.release() {
        Release::Released(registers) => registers,
        Release::Retained { isolation, reason } => {
            warn!("iommu: translation state is uncertain; unit retained: {reason}");
            *installed = Some(InstalledUnit { isolation, fences });
            return false;
        }
    };
    drop(installed);
    let registers_returned = match mem::unmap_device_exact(context, registers) {
        DeviceRetirement::Retired => true,
        DeviceRetirement::Retained(registers) => {
            warn!("iommu: the register window remained owned during cleanup");
            retain_registers(registers);
            false
        }
    };
    let fences_returned = release_fences(fences);
    registers_returned && fences_returned
}

/// Clear one inventory-bounded fault batch and report it once.
pub(crate) fn report_faults() {
    let drained = {
        let mut installed = INSTALLED.lock();
        let Some(isolation) = installed.as_mut() else {
            return;
        };
        isolation.drain_faults()
    };
    if drained != 0 {
        warn!("iommu: cleared a bounded batch of {drained} translation fault(s)");
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

fn retain_fences(fences: UnitFences) {
    if fences.count == 0 {
        return;
    }
    let mut retained = RETAINED_FENCES.lock_leaf();
    assert!(
        retained.is_none(),
        "storage selection continued after requester fences were quarantined"
    );
    *retained = Some(fences);
}
