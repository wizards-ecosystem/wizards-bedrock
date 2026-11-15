use core::fmt;
use core::ops::{Deref, DerefMut};

use acpi_tables::{
    Bdf, BusRange, DmaCoverage, DmaHandoff, DmaHandoffError, Dmar, RequesterInventory,
    RequesterInventoryError, Sdt,
};
use pci_config::{
    Address, EnableError, HandoffFencePolicy, QualifiedNonRequester, QuiescedIsaRequester,
};
use wost::arch::current::cpu;
use wost::arch::current::paging::{DeviceMapping, ReleasableDevice};
use wost::arch::current::pci as platform_pci;
use wost::arch::current::pci::DmaHandoffFenceError;
use wost::arch::current::vtd::{DmaDomain, Establishment, InterruptIsolation, Retargeting, Vtd};
use wost::boot::BootInfo;
use wost::sync::{SpinLock, ThreadLock, ThreadLockGuard};

use crate::mem::{self, DeviceRetirement};
use crate::thread::ThreadContext;
use crate::{acpi, info, nvme, pci, warn};

const REGISTER_WINDOW: u64 = 4096;

const SEGMENT: u16 = 0;

#[derive(Debug)]
pub(crate) struct EarlyDmaBoundary {
    functions: u32,
    disabled: u32,
    non_requester: Option<QualifiedNonRequester>,
    quiesced: Option<QuiescedIsaRequester>,
}

impl EarlyDmaBoundary {
    pub(crate) const fn quiesced_requesters(&self) -> u32 {
        if self.quiesced.is_some() { 1 } else { 0 }
    }

    pub(crate) fn take_quiesced_requester(&mut self) -> Option<QuiescedIsaRequester> {
        self.quiesced.take()
    }

    pub(crate) const fn functions(&self) -> u32 {
        self.functions
    }

    pub(crate) const fn disabled(&self) -> u32 {
        self.disabled
    }

    pub(crate) const fn retained_non_requesters(&self) -> u32 {
        if self.non_requester.is_some() { 1 } else { 0 }
    }

    pub(crate) fn take_non_requester(&mut self) -> Option<QualifiedNonRequester> {
        self.non_requester.take()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EarlyDmaError {
    UnauthenticatedPlatform,
    BaseRevision,
    MissingEvidence(&'static str),
    Table(acpi::TableError),
    Contract(DmaHandoffError),
    Segment(u16),
    Configuration(DmaHandoffFenceError),
}

impl fmt::Display for EarlyDmaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnauthenticatedPlatform => formatter
                .write_str("CPU state does not authenticate a qualified DMA-handoff platform"),
            Self::BaseRevision => formatter.write_str("bootloader rejected Limine base revision 4"),
            Self::MissingEvidence(name) => write!(formatter, "boot handoff has no {name}"),
            Self::Table(reason) => write!(formatter, "DMA handoff authority: {reason}"),
            Self::Contract(reason) => write!(formatter, "DMA handoff contract: {reason}"),
            Self::Segment(segment) => {
                write!(
                    formatter,
                    "DMA handoff names unsupported PCI segment {segment}"
                )
            }
            Self::Configuration(reason) => write!(formatter, "DMA handoff fence: {reason}"),
        }
    }
}

impl core::error::Error for EarlyDmaError {}

pub(crate) fn secure_handoff(boot: &BootInfo) -> Result<EarlyDmaBoundary, EarlyDmaError> {
    let platform = cpu::dma_handoff_platform().ok_or(EarlyDmaError::UnauthenticatedPlatform)?;
    let policy = match platform {
        cpu::DmaHandoffPlatform::QemuTcg => HandoffFencePolicy::strict(),
        cpu::DmaHandoffPlatform::Vmware => HandoffFencePolicy::vmware_440bx(),
    };
    if !boot.base_revision_supported {
        return Err(EarlyDmaError::BaseRevision);
    }
    let map = boot
        .memory_map
        .as_ref()
        .map_err(|_| EarlyDmaError::MissingEvidence("complete memory map"))?;
    let direct = *boot
        .direct_map
        .as_ref()
        .map_err(|_| EarlyDmaError::MissingEvidence("direct map"))?;
    let rsdp = *boot
        .rsdp
        .as_ref()
        .map_err(|_| EarlyDmaError::MissingEvidence("ACPI root pointer"))?;
    let table = acpi::find_unique_early::<{ DmaHandoff::LEN }>(map, direct, rsdp, Sdt::DMA_HANDOFF)
        .map_err(EarlyDmaError::Table)?;
    let contract = DmaHandoff::new(&table).map_err(EarlyDmaError::Contract)?;
    if contract.segment() != SEGMENT {
        return Err(EarlyDmaError::Segment(contract.segment()));
    }
    if !platform_pci::init() {
        return Err(EarlyDmaError::Configuration(
            DmaHandoffFenceError::Unavailable,
        ));
    }
    let mut report =
        platform_pci::fence_dma_handoff(policy).map_err(EarlyDmaError::Configuration)?;
    Ok(EarlyDmaBoundary {
        functions: report.functions(),
        disabled: report.disabled(),
        non_requester: report.take_non_requester(),
        quiesced: report.take_quiesced_requester(),
    })
}

static DMAR: SpinLock<Option<&'static [u8]>> = SpinLock::new(None);

static DMA_COVERAGE: SpinLock<Option<&'static [u8]>> = SpinLock::new(None);

// A thread lock, because commands run under the guard, which a leaf guard forbids.
static INSTALLED: ThreadLock<Option<InstalledUnit>> = ThreadLock::new(None);

static RETAINED_REGISTERS: SpinLock<Option<ReleasableDevice>> = SpinLock::new(None);

static RETAINED_MAPPING: SpinLock<Option<DeviceMapping>> = SpinLock::new(None);

static RETAINED_FENCES: SpinLock<Option<UnitFences>> = SpinLock::new(None);

static UNIT_FENCE_SLOTS: SpinLock<[Option<pci::DmaFence>; pci::FUNCTION_AUTHORITY_LIMIT]> =
    SpinLock::new([const { None }; pci::FUNCTION_AUTHORITY_LIMIT]);

// Runs before any candidate, while the heap may still grow; candidates never allocate.
pub(crate) fn discover(rsdp: u64) {
    match acpi::find_unique_table(rsdp, Sdt::DMA_COVERAGE) {
        Ok(table) => match DmaCoverage::new(&table) {
            Ok(coverage) => {
                info!(
                    "iommu: platform attests complete segment {} coverage at {:#x}",
                    coverage.segment(),
                    coverage.register_base()
                );
                *DMA_COVERAGE.lock_leaf() = Some(table.leak());
            }
            Err(reason) => warn!("iommu: DMA coverage attestation refused: {reason}"),
        },
        Err(acpi::TableError::Missing) => {}
        Err(reason) => warn!("iommu: DMA coverage attestation unreadable: {reason}"),
    }
    let table = match acpi::find_unique_table(rsdp, Sdt::DMAR) {
        Ok(table) => table,
        Err(reason) => {
            warn!("iommu: no usable dmar table: {reason}");
            return;
        }
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

pub(crate) fn establish_fabric_boundary(context: &mut ThreadContext) -> bool {
    if INSTALLED.lock().is_some() {
        return false;
    }
    let Some(table) = *DMAR.lock_leaf() else {
        warn!("iommu: no validated dmar for the early DMA boundary");
        return false;
    };
    let Ok(dmar) = Dmar::new(table) else {
        return false;
    };
    if !dmar.interrupt_remapping() || dmar.x2apic_opt_out() {
        warn!("iommu: platform interrupt remapping is unavailable or opts out of x2apic");
        return false;
    }
    let Some(register_base) = early_register_base(&dmar) else {
        warn!(
            "iommu: early ownership requires one RMRR-free segment-zero unit with complete static coverage"
        );
        return false;
    };
    let mapping = match mem::map_device(context, register_base, REGISTER_WINDOW) {
        Some(Ok(mapping)) => mapping,
        Some(Err(reason)) => {
            warn!("iommu: registers at {register_base:#x} not mapped: {reason}");
            return false;
        }
        None => {
            warn!("iommu: no active address space for the early DMA boundary");
            return false;
        }
    };
    let registers = match mapping.claim_releasable() {
        Ok(registers) => registers,
        Err(mapping) => {
            warn!("iommu: the early unit mapping is absent from the active root");
            retain_mapping(mapping);
            return false;
        }
    };
    let establishment = Vtd::establish_deny_all(registers, nvme::clock_ns, REGISTER_WINDOW);
    let isolation = match establishment {
        Establishment::Established(isolation) => isolation,
        Establishment::Refused { reason, registers } => {
            warn!("iommu: the early default-deny takeover refused: {reason}");
            if let DeviceRetirement::Retained(registers) =
                mem::unmap_device_exact(context, registers)
            {
                retain_registers(registers);
            }
            return false;
        }
        Establishment::Retained { reason, isolation } => {
            warn!("iommu: the early default-deny takeover is uncertain: {reason}");
            install(InstalledUnit {
                register_base,
                isolation,
                fences: UnitFences::empty(),
                state: InstalledState::Quarantined,
            });
            return false;
        }
    };
    if !isolation.is_boundary_active() {
        warn!("iommu: the early default-deny boundary did not read back intact");
        install(InstalledUnit {
            register_base,
            isolation,
            fences: UnitFences::empty(),
            state: InstalledState::Quarantined,
        });
        return false;
    }
    let facts = isolation.facts();
    info!(
        "iommu: early default-deny DMA and interrupt boundary active at {register_base:#x}: {}-level walks, cap {:#x}, ecap {:#x}",
        facts.levels, facts.capability, facts.extended
    );
    install(InstalledUnit {
        register_base,
        isolation,
        fences: UnitFences::empty(),
        state: InstalledState::Ready,
    });
    true
}

fn early_register_base(dmar: &Dmar<'_>) -> Option<u64> {
    if let Some(register_base) = dmar.sole_complete_unit_register_base(SEGMENT) {
        return Some(register_base);
    }
    let table = *DMA_COVERAGE.lock_leaf();
    let coverage = DmaCoverage::new(table?).ok()?;
    let register_base = dmar.sole_rmrr_free_unit_register_base(SEGMENT)?;
    (coverage.segment() == SEGMENT && coverage.register_base() == register_base)
        .then_some(register_base)
}

#[derive(Debug)]
struct Unit {
    register_base: u64,
    requesters: RequesterInventory<{ pci::FUNCTION_AUTHORITY_LIMIT }>,
}

#[derive(Debug)]
struct UnitFences {
    count: usize,
}

impl UnitFences {
    const fn empty() -> Self {
        Self { count: 0 }
    }

    fn new() -> Self {
        assert!(
            UNIT_FENCE_SLOTS.lock_leaf().iter().all(Option::is_none),
            "storage selection continued while requester fence authorities remained vaulted"
        );
        Self::empty()
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

    fn release(self) -> Result<(), Self> {
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
        if retained_count == 0 {
            Ok(())
        } else {
            Err(Self {
                count: retained_count,
            })
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InstalledState {
    Ready,
    Quarantined,
}

#[derive(Debug)]
pub(crate) struct InstalledUnit {
    register_base: u64,
    isolation: Vtd,
    fences: UnitFences,
    state: InstalledState,
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

#[derive(Debug)]
enum LookupError {
    NoTable,
    UnsafeTopology,
    UnsafeInventory(RequesterInventoryError),
    NoInterruptRemapping,
    IncompleteInterruptBoundary,
    UnsupportedReservedRegion,
}

fn look_up(bdf: Bdf) -> Result<Unit, LookupError> {
    if !pci::topology_is_sealed() {
        return Err(LookupError::UnsafeTopology);
    }
    // Copied out so topology queries below run with no DMAR guard held.
    let table = *DMAR.lock_leaf();
    let Some(table) = table else {
        return Err(LookupError::NoTable);
    };
    let Ok(dmar) = Dmar::new(table) else {
        return Err(LookupError::NoTable);
    };
    if !dmar.interrupt_remapping() || dmar.x2apic_opt_out() {
        return Err(LookupError::NoInterruptRemapping);
    }
    if dmar.sole_unit_segment() != Some(SEGMENT) {
        return Err(LookupError::IncompleteInterruptBoundary);
    }
    let requesters = match dmar.requester_inventory::<{ pci::FUNCTION_AUTHORITY_LIMIT }>(
        bdf,
        bridge_buses,
        hierarchy_contains,
        requester_is_present,
    ) {
        Ok(requesters) => requesters,
        Err(reason) => return Err(LookupError::UnsafeInventory(reason)),
    };
    if !covers_every_present_requester(&requesters) {
        return Err(LookupError::IncompleteInterruptBoundary);
    }
    if dmar
        .reserved_regions_for(bdf, bridge_buses, hierarchy_contains)
        .next()
        .is_some()
    {
        return Err(LookupError::UnsupportedReservedRegion);
    }
    let governing = requesters.unit();
    Ok(Unit {
        register_base: governing.register_base,
        requesters,
    })
}

fn covers_every_present_requester(
    requesters: &RequesterInventory<{ pci::FUNCTION_AUTHORITY_LIMIT }>,
) -> bool {
    for bus in 0..=u8::MAX {
        for device in 0..32 {
            for function in 0..8 {
                let Some(at) = Address::new(bus, device, function, 0) else {
                    return false;
                };
                if !pci::topology_contains_requester(at) {
                    continue;
                }
                let requester = Bdf::new(SEGMENT, bus, device, function);
                if !requesters.iter().any(|covered| covered == requester) {
                    return false;
                }
            }
        }
    }
    true
}

fn bridge_buses(at: Bdf) -> Option<BusRange> {
    if at.segment() != SEGMENT {
        return None;
    }
    let address = Address::new(at.bus(), at.device(), at.function(), 0)?;
    let range = pci::topology_bridge_range(address)?;
    Some(BusRange {
        secondary: range.secondary,
        subordinate: range.subordinate,
    })
}

fn hierarchy_contains(root: Bdf, target: Bdf) -> bool {
    if root.segment() != SEGMENT || target.segment() != SEGMENT {
        return false;
    }
    let Some(root) = Address::new(root.bus(), root.device(), root.function(), 0) else {
        return false;
    };
    let Some(target) = Address::new(target.bus(), target.device(), target.function(), 0) else {
        return false;
    };
    pci::topology_hierarchy_contains(root, target)
}

fn requester_is_present(at: Bdf) -> bool {
    if at.segment() != SEGMENT {
        return false;
    }
    Address::new(at.bus(), at.device(), at.function(), 0)
        .is_some_and(pci::topology_contains_requester)
}

fn unit_for(at: Address) -> Option<Unit> {
    let bdf = Bdf::new(SEGMENT, at.bus(), at.device(), at.function());
    match look_up(bdf) {
        Ok(unit) => {
            info!(
                "iommu: {at} is one of {} live PCI requester(s) governed by the unit at {:#x}",
                unit.requesters.len(),
                unit.register_base
            );
            Some(unit)
        }
        Err(LookupError::NoTable) => {
            warn!("iommu: no validated dmar; every candidate stays unisolated");
            None
        }
        Err(LookupError::UnsafeTopology) => {
            warn!("iommu: no sealed PCI topology; every candidate stays unisolated");
            None
        }
        Err(LookupError::UnsafeInventory(reason)) => {
            warn!("iommu: no complete requester inventory governs {at}: {reason}");
            None
        }
        Err(LookupError::NoInterruptRemapping) => {
            warn!("iommu: platform interrupt remapping is unavailable or opts out of x2apic");
            None
        }
        Err(LookupError::IncompleteInterruptBoundary) => {
            warn!("iommu: one remapping unit does not cover every present PCI requester");
            None
        }
        Err(LookupError::UnsupportedReservedRegion) => {
            warn!(
                "iommu: firmware-reserved DMA regions are unsupported without a trusted physical classification"
            );
            None
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EstablishOutcome {
    Established,
    Unavailable,
    Quarantined,
}

#[derive(Debug)]
enum FenceAcquisition {
    Ready(UnitFences),
    Refused,
    Retained(UnitFences),
}

fn acquire_fences(
    requesters: &RequesterInventory<{ pci::FUNCTION_AUTHORITY_LIMIT }>,
    selected: Option<&pci::Device>,
) -> FenceAcquisition {
    let selected_at = selected.map(|device| device.at().function_base());
    let mut fences = UnitFences::new();
    for requester in requesters.iter() {
        let Some(at) = Address::new(requester.bus(), requester.device(), requester.function(), 0)
        else {
            warn!("iommu: an inventoried function has an invalid PCI requester ID");
            return release_refused_fences(fences);
        };
        if Some(at) == selected_at {
            continue;
        }
        match pci::fence_dma_requester(at) {
            pci::DmaFenceOutcome::Fenced(fence) => fences.push(fence),
            pci::DmaFenceOutcome::Refused(reason) => {
                warn!("iommu: requester {at} could not be fenced: {reason}");
                return release_refused_fences(fences);
            }
            pci::DmaFenceOutcome::Retained { reason, fence } => {
                warn!("iommu: requester {at} has uncertain fence state: {reason}");
                fences.push(fence);
                return FenceAcquisition::Retained(fences);
            }
        }
    }
    if let Some(selected) = selected
        && let Err(reason) = selected.verify_bus_master_fence()
    {
        warn!("iommu: selected requester lost its bus-master fence: {reason}");
        return FenceAcquisition::Retained(fences);
    }
    FenceAcquisition::Ready(fences)
}

// The caller holds the installed unit, so no other context reaches the fence vault meanwhile.
pub(crate) fn admit_upstream_bus_master(
    domain: &DmaDomain<'_>,
    interrupts: &InterruptIsolation<'_>,
) -> Result<(), EnableError> {
    let function = domain.function();
    for index in 0..pci::FUNCTION_AUTHORITY_LIMIT {
        let Some(mut fence) = UNIT_FENCE_SLOTS
            .lock_leaf()
            .get_mut(index)
            .and_then(Option::take)
        else {
            continue;
        };
        let bridge = fence.address();
        let admitted = pci::topology_hierarchy_contains(bridge, function)
            .then(|| fence.admit_bridge_bus_master(domain, interrupts));
        if let Some(slot) = UNIT_FENCE_SLOTS.lock_leaf().get_mut(index) {
            *slot = Some(fence);
        }
        match admitted {
            None | Some(Ok(())) => {}
            Some(Err(EnableError::Restore)) => return Err(EnableError::Restore),
            // A clean refusal leaves the bridge as the entry fence did; emulators ignore its BME.
            Some(Err(reason)) => warn!("iommu: bridge {bridge} kept bus mastering off: {reason}"),
        }
    }
    Ok(())
}

fn release_refused_fences(fences: UnitFences) -> FenceAcquisition {
    match fences.release() {
        Ok(()) => FenceAcquisition::Refused,
        Err(fences) => FenceAcquisition::Retained(fences),
    }
}

pub(crate) fn installed() -> ThreadLockGuard<'static, Option<InstalledUnit>> {
    INSTALLED.lock()
}

pub(crate) fn establish(device: &pci::Device) -> EstablishOutcome {
    let at = device.at();
    let Some(unit) = unit_for(at) else {
        return EstablishOutcome::Unavailable;
    };
    let Unit {
        register_base,
        requesters,
    } = unit;
    let fences = match acquire_fences(&requesters, Some(device)) {
        FenceAcquisition::Ready(fences) => fences,
        FenceAcquisition::Refused => return EstablishOutcome::Unavailable,
        FenceAcquisition::Retained(fences) => {
            retain_fences(fences);
            return EstablishOutcome::Quarantined;
        }
    };
    let mut installed = INSTALLED.lock();
    let Some(owner) = installed.as_mut() else {
        drop(installed);
        return release_unavailable_fences(fences);
    };
    if owner.state != InstalledState::Ready {
        warn!("iommu: the early owner is quarantined and cannot be retargeted");
        drop(installed);
        let _ = release_fences(fences);
        return EstablishOutcome::Quarantined;
    }
    match owner.isolation.retarget(at) {
        Retargeting::Retargeted => {
            owner.fences = fences;
            let stray = owner.isolation.drain_faults(|_| {});
            if stray != 0 {
                warn!("iommu: {stray} fault(s) were recorded before any segment was mapped");
            }
            let facts = owner.isolation.facts();
            info!(
                "iommu: translation on for {at} through the unit at {register_base:#x}: {}-level walks, retargeted without dropping the default-deny boundary, cap {:#x}, ecap {:#x}",
                facts.levels, facts.capability, facts.extended
            );
            EstablishOutcome::Established
        }
        Retargeting::Refused(reason) => {
            warn!("iommu: parked unit refused DMA retargeting: {reason}");
            drop(installed);
            release_unavailable_fences(fences)
        }
        Retargeting::Retained(reason) => {
            warn!("iommu: DMA retargeting state is uncertain: {reason}");
            owner.fences = fences;
            owner.state = InstalledState::Quarantined;
            EstablishOutcome::Quarantined
        }
    }
}

pub(crate) fn ensure_interrupt_boundary() -> bool {
    match pci::topology_presence() {
        pci::TopologyPresence::Unsealed => return false,
        pci::TopologyPresence::Empty | pci::TopologyPresence::Present => {}
    }
    let installed = INSTALLED.lock();
    let Some(owner) = installed
        .as_ref()
        .filter(|owner| owner.state == InstalledState::Ready && owner.is_boundary_active())
    else {
        return false;
    };
    if matches!(pci::topology_presence(), pci::TopologyPresence::Empty) {
        return true;
    }
    if owner.interrupt_isolation().is_some() {
        return true;
    }
    let Some(anchor) = pci::first_topology_function() else {
        return false;
    };
    unit_for(anchor).is_some_and(|unit| unit.register_base == owner.register_base)
}

pub(crate) fn is_interrupt_boundary_active() -> bool {
    if matches!(pci::topology_presence(), pci::TopologyPresence::Unsealed) {
        return false;
    }
    INSTALLED
        .lock()
        .as_ref()
        .is_some_and(|unit| unit.state == InstalledState::Ready && unit.is_boundary_active())
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
    match fences.release() {
        Ok(()) => true,
        Err(fences) => {
            retain_fences(fences);
            false
        }
    }
}

// The candidate's bus mastering must already be off.
pub(crate) fn release_candidate() -> bool {
    let fences = {
        let mut installed = INSTALLED.lock();
        let Some(unit) = installed.as_mut() else {
            return true;
        };
        if unit.state != InstalledState::Ready {
            return false;
        }
        if unit.isolation.function().is_some()
            && let Err(reason) = unit.isolation.park()
        {
            warn!("iommu: translation state is uncertain; unit retained: {reason}");
            unit.state = InstalledState::Quarantined;
            return false;
        }
        core::mem::replace(&mut unit.fences, UnitFences::empty())
    };
    let Err(fences) = fences.release() else {
        return true;
    };
    let mut installed = INSTALLED.lock();
    if let Some(unit) = installed.as_mut() {
        assert_eq!(
            unit.fences.count, 0,
            "a released unit kept requester fences beside a refused return"
        );
        unit.fences = fences;
    } else {
        retain_fences(fences);
    }
    false
}

pub(crate) fn report_faults() {
    let drained = {
        let mut installed = INSTALLED.lock();
        let Some(isolation) = installed.as_mut() else {
            return;
        };
        isolation.drain_faults(|_| {})
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
