use core::fmt;
use core::marker::PhantomData;

use pci_config::Address;
use register_bits::{low_u32, u32_halves, usize_from_u64};
use vtd::{
    AddressWidth, Capability, ContextEntry, ExtendedCapability, FaultRecording, FaultStatus,
    InvalidationDescriptor, RootEntry, Version, compatibility_interrupts_enabled, context_command,
    enable_interrupt_remapping, enable_translation, fault_event_control, global_command,
    global_status, interrupt_remapping_enabled, interrupt_table_address, interrupt_table_base,
    interrupt_table_has_two_entries, interrupt_table_pointer_set, invalidation_queue_address,
    iotlb_command, root_table_address, root_table_pointer_set, second_level,
    set_interrupt_table_pointer, set_root_table_pointer, takeover_ready, translation_enabled,
};

use crate::arch::x86_64::paging::ReleasableDevice;
use crate::mm::{DmaMappingOwner, DmaSegment, IommuTable, PublishedIommuTable, TableWalks};

const DOMAIN_ID: u16 = 1;

const WAIT_NS: u64 = 1_000_000_000;

// Counts polls only while the clock cannot say how long a wait has run.
const POLL_LIMIT: u32 = 1_000_000;

const PAGE: u64 = 4096;

const ENTRIES: usize = (PAGE / 8) as usize;

const ENTRY_BYTES: u64 = 16;

const IOTLB_BYTES: u64 = 16;

// Two 128-bit descriptors.
const INVALIDATION_TAIL: u64 = 32;

// The completion word sits past the two submitted descriptors.
const INVALIDATION_STATUS_INDEX: usize = 8;

const INVALIDATION_COMPLETE: u32 = 1;

// Two 128-bit entries, both kept not-present.
const INTERRUPT_TABLE_WORDS: usize = 4;

// Allocated whole, so mapping never allocates; each 2 MiB region costs a leaf.
const TABLE_LIMIT: usize = 24;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    InvalidRange,
    OutsideAddressWidth,
    Register(usize),
    RegistersOutsideWindow(u64),
    UnsupportedRegisterInvalidationVersion { version: u32 },
    NoAddressWidth { capability: u64 },
    NoDmaDraining,
    NoInterruptRemapping,
    NoExtendedInterruptMode,
    OutOfTables,
    Misaligned,
    FirmwareFacilitiesActive { status: u32 },
    RootTableRefused,
    InterruptTableRefused,
    InterruptRemappingTimeout,
    FaultInterruptMaskRefused,
    InterruptBoundaryLost,
    NoActiveDomain,
    DomainStillActive,
    InvalidationQueueRefused,
    QueuedInvalidationEnableTimeout,
    QueuedInvalidationIncomplete,
    QueuedInvalidationDisableTimeout,
    TranslationTimeout,
    InvalidationTimeout,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange => {
                formatter.write_str("the DMA run is empty or wraps the address space")
            }
            Self::OutsideAddressWidth => {
                formatter.write_str("the DMA run extends beyond the unit's address width")
            }
            Self::Register(offset) => {
                write!(formatter, "the register window refused offset {offset:#x}")
            }
            Self::RegistersOutsideWindow(end) => {
                write!(formatter, "unit registers end at {end:#x}, past the window")
            }
            Self::UnsupportedRegisterInvalidationVersion { version } => {
                let version = Version::from_bits(*version);
                write!(
                    formatter,
                    "VT-d version {}.{} does not support register invalidation",
                    version.major(),
                    version.minor()
                )
            }
            Self::NoAddressWidth { capability } => {
                write!(
                    formatter,
                    "no modelled page-table depth in capability {capability:#x}"
                )
            }
            Self::NoDmaDraining => {
                formatter.write_str("the unit does not drain both DMA reads and writes")
            }
            Self::NoInterruptRemapping => {
                formatter.write_str("the unit lacks interrupt remapping or queued invalidation")
            }
            Self::NoExtendedInterruptMode => {
                formatter.write_str("the unit cannot remap interrupts in x2apic mode")
            }
            Self::OutOfTables => formatter.write_str("no frame for a remapping table"),
            Self::Misaligned => formatter.write_str("a remapping table is not page aligned"),
            Self::FirmwareFacilitiesActive { status } => write!(
                formatter,
                "firmware left a remapping facility active in status {status:#x}"
            ),
            Self::RootTableRefused => {
                formatter.write_str("the root-table pointer was not accepted")
            }
            Self::InterruptTableRefused => {
                formatter.write_str("the interrupt-table pointer was not accepted")
            }
            Self::InterruptRemappingTimeout => {
                formatter.write_str("interrupt remapping did not switch")
            }
            Self::FaultInterruptMaskRefused => {
                formatter.write_str("the VT-d fault-event interrupt did not remain masked")
            }
            Self::InterruptBoundaryLost => {
                formatter.write_str("the unit lost its default-deny interrupt boundary")
            }
            Self::NoActiveDomain => formatter.write_str("the unit has no active DMA domain"),
            Self::DomainStillActive => formatter.write_str("the prior DMA domain is still active"),
            Self::InvalidationQueueRefused => {
                formatter.write_str("the invalidation-queue address was not accepted")
            }
            Self::QueuedInvalidationEnableTimeout => {
                formatter.write_str("queued invalidation did not enable")
            }
            Self::QueuedInvalidationIncomplete => {
                formatter.write_str("queued invalidation descriptors did not complete")
            }
            Self::QueuedInvalidationDisableTimeout => {
                formatter.write_str("queued invalidation did not disable")
            }
            Self::TranslationTimeout => formatter.write_str("translation did not switch"),
            Self::InvalidationTimeout => formatter.write_str("an invalidation did not complete"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SegmentChangeError {
    Refused(Error),
    Retained(Error),
}

impl fmt::Display for SegmentChangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(reason) => {
                write!(formatter, "DMA segment change refused cleanly: {reason}")
            }
            Self::Retained(reason) => {
                write!(
                    formatter,
                    "DMA segment retained after an uncertain change: {reason}"
                )
            }
        }
    }
}

impl core::error::Error for SegmentChangeError {}

impl core::error::Error for Error {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UnitFacts {
    pub capability: u64,
    pub extended: u64,
    pub levels: u8,
}

#[derive(Debug)]
pub struct Vtd {
    registers: ReleasableDevice,
    clock: fn() -> Option<u64>,
    function: Option<Address>,
    capability: Capability,
    extended: ExtendedCapability,
    width: AddressWidth,
    // GCMD fields are write-only, so every command is built from this shadow.
    global_command: u32,
    root: PublishedIommuTable,
    context: PublishedIommuTable,
    interrupts: PublishedIommuTable,
    invalidation_queue: PublishedIommuTable,
    // Table 0 is the domain's top level; only the first `table_count` are carved.
    tables: PublishedIommuTable,
    table_count: usize,
}

#[derive(Debug)]
#[must_use = "a DMA domain proof is required to admit bus mastering"]
pub struct DmaDomain<'unit> {
    function: Address,
    _unit: PhantomData<&'unit Vtd>,
}

#[derive(Debug)]
#[must_use = "interrupt isolation proof is required to admit bus mastering"]
pub struct InterruptIsolation<'unit> {
    function: Address,
    _unit: PhantomData<&'unit Vtd>,
}

impl InterruptIsolation<'_> {
    pub const fn function(&self) -> Address {
        self.function
    }
}

impl DmaDomain<'_> {
    pub const fn function(&self) -> Address {
        self.function
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[must_use = "a retargeted VT-d owner must be installed or quarantined"]
pub enum Retargeting {
    Retargeted,
    Refused(Error),
    Retained(Error),
}

#[derive(Debug)]
#[must_use = "a post-transition VT-d owner must be installed or quarantined"]
pub enum Establishment {
    Established(Vtd),
    Refused {
        reason: Error,
        registers: ReleasableDevice,
    },
    Retained {
        reason: Error,
        isolation: Vtd,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fault {
    pub source: u16,
    pub address: u64,
    pub write: bool,
}

#[derive(Debug)]
struct Plan {
    capability: Capability,
    extended: ExtendedCapability,
    width: AddressWidth,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Switch {
    On,
    Off,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Caches {
    ContextAndIotlb,
    Iotlb,
}

impl Vtd {
    pub fn establish_deny_all(
        registers: ReleasableDevice,
        clock: fn() -> Option<u64>,
        window: u64,
    ) -> Establishment {
        let plan = match Plan::read(&registers, window) {
            Ok(plan) => plan,
            Err(reason) => return Establishment::Refused { reason, registers },
        };
        let walks = if plan.extended.page_walk_coherent() {
            TableWalks::Coherent
        } else {
            TableWalks::Uncached
        };
        let (Some(root), Some(context), Some(interrupts), Some(invalidation_queue), Some(tables)) = (
            IommuTable::new(1, walks),
            IommuTable::new(1, walks),
            IommuTable::new(1, walks),
            IommuTable::new(1, walks),
            IommuTable::new(TABLE_LIMIT, walks),
        ) else {
            return Establishment::Refused {
                reason: Error::OutOfTables,
                registers,
            };
        };
        let mut unit = Self {
            registers,
            clock,
            function: None,
            capability: plan.capability,
            extended: plan.extended,
            width: plan.width,
            global_command: 0,
            root: PublishedIommuTable::new(root),
            context: PublishedIommuTable::new(context),
            interrupts: PublishedIommuTable::new(interrupts),
            invalidation_queue: PublishedIommuTable::new(invalidation_queue),
            tables: PublishedIommuTable::new(tables),
            table_count: 1,
        };
        match unit.transition() {
            Ok(()) => Establishment::Established(unit),
            Err(reason) => Establishment::Retained {
                reason,
                isolation: unit,
            },
        }
    }

    // The bus root is published last, after the context entry it names.
    fn prepare(&mut self, function: Address) -> Result<(), Error> {
        let device_function = Self::device_function(function);
        let bus = function.bus();
        let context_entry = ContextEntry::try_present(self.table_address(0), self.width, DOMAIN_ID)
            .ok_or(Error::Misaligned)?;
        write_entry(
            &mut self.context,
            device_function,
            context_entry.lo,
            context_entry.hi,
        )?;
        let root_entry =
            RootEntry::try_present(self.context.address().get()).ok_or(Error::Misaligned)?;
        write_entry(&mut self.root, bus, root_entry.lo, root_entry.hi)
    }

    // The drain after TE settles every request that entered before the boundary.
    fn transition(&mut self) -> Result<(), Error> {
        self.mask_fault_interrupt()?;
        self.set_root_table()?;
        self.invalidate(Caches::ContextAndIotlb)?;
        self.set_interrupt_table()?;
        self.invalidate_interrupt_cache()?;
        self.enable_interrupt_remapping()?;
        self.enable_translation()?;
        self.drain_dma()?;
        self.is_boundary_active()
            .then_some(())
            .ok_or(Error::InterruptBoundaryLost)
    }

    const fn device_function(function: Address) -> u8 {
        (function.device() << 3) | function.function()
    }

    fn table_address(&self, index: usize) -> u64 {
        self.tables.address().get() + index as u64 * PAGE
    }

    fn table_index(&self, entry: u64) -> Option<usize> {
        let offset = second_level::address(entry).checked_sub(self.tables.address().get())?;
        let index = usize::try_from(offset / PAGE).ok()?;
        (offset % PAGE == 0 && index < self.table_count).then_some(index)
    }

    pub const fn function(&self) -> Option<Address> {
        self.function
    }

    pub fn facts(&self) -> UnitFacts {
        UnitFacts {
            capability: self.capability.bits(),
            extended: self.extended.bits(),
            levels: self.width.levels(),
        }
    }

    #[must_use = "an unmapped segment cannot be reached by the device"]
    pub fn map_segment_for_device_read(
        &mut self,
        segment: &mut DmaSegment,
    ) -> Result<(), SegmentChangeError> {
        self.map_segment_with::<true, false>(segment)
    }

    #[must_use = "an unmapped segment cannot be reached by the device"]
    pub fn map_segment_for_device_write(
        &mut self,
        segment: &mut DmaSegment,
    ) -> Result<(), SegmentChangeError> {
        self.map_segment_with::<false, true>(segment)
    }

    fn map_segment_with<const DEVICE_READS: bool, const DEVICE_WRITES: bool>(
        &mut self,
        segment: &mut DmaSegment,
    ) -> Result<(), SegmentChangeError> {
        let Some((physical, length)) = segment.address().zip(segment.len()) else {
            return Err(SegmentChangeError::Refused(Error::InvalidRange));
        };
        let owner = DmaMappingOwner::new(self.root.address());
        if !segment.begin_mapping(owner) {
            return Err(SegmentChangeError::Refused(Error::InvalidRange));
        }
        match self.map_range::<DEVICE_READS, DEVICE_WRITES>(physical.get(), length) {
            Err(error @ SegmentChangeError::Retained(_)) => {
                segment.retain();
                Err(error)
            }
            Err(error @ SegmentChangeError::Refused(_)) => {
                assert!(
                    segment.release_mapping(owner),
                    "mapping owner changed during rollback"
                );
                Err(error)
            }
            Ok(()) => Ok(()),
        }
    }

    fn map_range<const DEVICE_READS: bool, const DEVICE_WRITES: bool>(
        &mut self,
        physical: u64,
        length: u64,
    ) -> Result<(), SegmentChangeError> {
        let Some(last) = length
            .checked_sub(1)
            .and_then(|last| physical.checked_add(last))
        else {
            return Err(SegmentChangeError::Refused(Error::InvalidRange));
        };
        if !self.width.covers_on(self.capability, last) {
            return Err(SegmentChangeError::Refused(Error::OutsideAddressWidth));
        }
        let start = physical & !(PAGE - 1);
        let mut page = start;
        while page <= last {
            if let Err(reason) = self.map_page::<DEVICE_READS, DEVICE_WRITES>(page) {
                if page == start {
                    return Err(SegmentChangeError::Refused(reason));
                }
                return match self.rollback_range(start, page - PAGE) {
                    Ok(()) => Err(SegmentChangeError::Refused(reason)),
                    Err(rollback) => Err(SegmentChangeError::Retained(rollback)),
                };
            }
            page += PAGE;
        }
        match self.invalidate(Caches::Iotlb) {
            Ok(()) => Ok(()),
            Err(reason) => match self.rollback_range(start, last) {
                Ok(()) => Err(SegmentChangeError::Refused(reason)),
                Err(rollback) => Err(SegmentChangeError::Retained(rollback)),
            },
        }
    }

    #[must_use = "a still-mapped segment is still reachable by the device"]
    pub fn unmap_segment(&mut self, segment: &mut DmaSegment) -> Result<(), SegmentChangeError> {
        let owner = DmaMappingOwner::new(self.root.address());
        if !segment.is_mapped_by(owner) {
            return Err(SegmentChangeError::Refused(Error::InvalidRange));
        }
        let Some((physical, length)) = segment.address().zip(segment.len()) else {
            return Err(SegmentChangeError::Refused(Error::InvalidRange));
        };
        let Some(last) = length
            .checked_sub(1)
            .and_then(|last| physical.get().checked_add(last))
        else {
            return Err(SegmentChangeError::Refused(Error::InvalidRange));
        };
        let mut page = physical.get() & !(PAGE - 1);
        let mut failure = None;
        while page <= last {
            if let Err(reason) = self.unmap_page(page) {
                failure.get_or_insert(reason);
            }
            page += PAGE;
        }
        if let Err(reason) = self.drain_dma() {
            failure.get_or_insert(reason);
        }
        if let Some(reason) = failure {
            segment.retain();
            Err(SegmentChangeError::Retained(reason))
        } else {
            assert!(
                segment.release_mapping(owner),
                "mapping owner changed during revocation"
            );
            Ok(())
        }
    }

    fn rollback_range(&mut self, start: u64, last: u64) -> Result<(), Error> {
        let mut page = start;
        let mut failure = None;
        while page <= last {
            if let Err(reason) = self.unmap_page(page) {
                failure.get_or_insert(reason);
            }
            page += PAGE;
        }
        if let Err(reason) = self.drain_dma() {
            failure.get_or_insert(reason);
        }
        failure.map_or(Ok(()), Err)
    }

    #[must_use]
    pub fn domain(&self) -> Option<DmaDomain<'_>> {
        let function = self.function?;
        let status = read_u32(&self.registers, vtd::register::GLOBAL_STATUS).ok()?;
        if !translation_enabled(status) {
            return None;
        }
        let root = RootEntry {
            lo: self.root.read_u64(usize::from(function.bus()) * 2)?,
            hi: self.root.read_u64(usize::from(function.bus()) * 2 + 1)?,
        };
        if !root.is_present() || root.context_table_address() != self.context.address().get() {
            return None;
        }
        let entry = ContextEntry {
            lo: self
                .context
                .read_u64(usize::from(Self::device_function(function)) * 2)?,
            hi: self
                .context
                .read_u64(usize::from(Self::device_function(function)) * 2 + 1)?,
        };
        let names_this_domain = entry.is_present()
            && entry.translation_type() == 0
            && entry.second_level_address() == self.table_address(0)
            && entry.address_width_encoding() == self.width.context_encoding()
            && entry.domain_id() == DOMAIN_ID;
        names_this_domain.then_some(DmaDomain {
            function,
            _unit: PhantomData,
        })
    }

    #[must_use]
    pub fn interrupt_isolation(&self) -> Option<InterruptIsolation<'_>> {
        let function = self.function?;
        self.is_boundary_active().then_some(InterruptIsolation {
            function,
            _unit: PhantomData,
        })
    }

    #[must_use]
    pub fn is_boundary_active(&self) -> bool {
        let Ok(pointer) = read_u64(&self.registers, vtd::register::ROOT_TABLE_ADDRESS) else {
            return false;
        };
        let Ok(status) = read_u32(&self.registers, vtd::register::GLOBAL_STATUS) else {
            return false;
        };
        let Ok(fault_control) = read_u32(&self.registers, vtd::register::FAULT_EVENT_CONTROL)
        else {
            return false;
        };
        if !translation_enabled(status)
            || !interrupt_remapping_enabled(status)
            || compatibility_interrupts_enabled(status)
            || status & global_status::QIES != 0
            || !fault_event_control::masked(fault_control)
            || root_table_address(pointer) != self.root.address().get()
        {
            return false;
        }
        let Some(expected) = interrupt_table_address(self.interrupts.address().get()) else {
            return false;
        };
        let Ok(observed) = read_u64(
            &self.registers,
            vtd::register::INTERRUPT_REMAPPING_TABLE_ADDRESS,
        ) else {
            return false;
        };
        if observed != expected
            || interrupt_table_base(observed) != self.interrupts.address().get()
            || !interrupt_table_has_two_entries(observed)
        {
            return false;
        }
        for index in 0..INTERRUPT_TABLE_WORDS {
            if self.interrupts.read_u64(index) != Some(0) {
                return false;
            }
        }
        true
    }

    // FRI is latched only when PPF rises, so the walk starts there and wraps until a clear record.
    pub fn drain_faults(&mut self, mut observe: impl FnMut(Fault)) -> usize {
        let Ok(bits) = read_u32(&self.registers, vtd::register::FAULT_STATUS) else {
            return 0;
        };
        let status = FaultStatus::from_bits(bits);
        let mut drained = 0;
        let registers = self.capability.number_of_fault_recording_registers();
        let mut index = u16::from(status.fault_record_index());
        if status.primary_pending() && index < registers {
            while drained < usize::from(registers) {
                let Some(fault) = self.take_fault(index) else {
                    break;
                };
                observe(fault);
                drained += 1;
                index = (index + 1) % registers;
            }
        }
        if status.overflow() {
            let _ = write_u32(
                &mut self.registers,
                vtd::register::FAULT_STATUS,
                FaultStatus::acknowledge_overflow(),
            );
        }
        drained
    }

    fn take_fault(&mut self, index: u16) -> Option<Fault> {
        let base = self.capability.fault_recording_offset_bytes() + u64::from(index) * ENTRY_BYTES;
        let record = FaultRecording {
            lo: read_u64(&self.registers, base).ok()?,
            hi: read_u64(&self.registers, base + 8).ok()?,
        };
        if !record.is_fault() {
            return None;
        }
        write_u64(&mut self.registers, base + 8, FaultRecording::clear().hi).ok()?;
        Some(Fault {
            source: record.source_id(),
            address: record.fault_address(),
            write: !record.is_read_request(),
        })
    }

    // Tables are cleared only after the root entry is absent and older DMA drained.
    pub fn park(&mut self) -> Result<(), Error> {
        if !self.is_boundary_active() {
            return Err(Error::InterruptBoundaryLost);
        }
        let function = self.function.ok_or(Error::NoActiveDomain)?;
        write_entry(&mut self.root, function.bus(), 0, 0)?;
        self.invalidate(Caches::ContextAndIotlb)?;
        self.drain_dma()?;
        self.clear_dma_tables()?;
        self.function = None;
        self.is_boundary_active()
            .then_some(())
            .ok_or(Error::InterruptBoundaryLost)
    }

    // Tables are prepared behind an absent root entry, so publishing it goes live.
    pub fn retarget(&mut self, function: Address) -> Retargeting {
        if !self.is_boundary_active() {
            return Retargeting::Retained(Error::InterruptBoundaryLost);
        }
        if self.function.is_some() {
            return Retargeting::Retained(Error::DomainStillActive);
        }
        if let Err(reason) = self.clear_dma_tables() {
            return Retargeting::Refused(reason);
        }
        self.table_count = 1;
        if let Err(reason) = self.prepare(function) {
            return Retargeting::Refused(reason);
        }
        self.function = Some(function);
        if let Err(reason) = self.invalidate(Caches::ContextAndIotlb) {
            return Retargeting::Retained(reason);
        }
        if self.domain().is_none() || self.interrupt_isolation().is_none() {
            return Retargeting::Retained(Error::InterruptBoundaryLost);
        }
        Retargeting::Retargeted
    }

    fn clear_dma_tables(&mut self) -> Result<(), Error> {
        for index in 0..ENTRIES {
            if !self.root.write_u64(index, 0) || !self.context.write_u64(index, 0) {
                return Err(Error::OutOfTables);
            }
        }
        for index in 0..TABLE_LIMIT * ENTRIES {
            if !self.tables.write_u64(index, 0) {
                return Err(Error::OutOfTables);
            }
        }
        Ok(())
    }

    fn map_page<const DEVICE_READS: bool, const DEVICE_WRITES: bool>(
        &mut self,
        page: u64,
    ) -> Result<(), Error> {
        let mut table = 0usize;
        let top = self.width.levels() - 1;
        for level in (1..=top).rev() {
            let slot = table * ENTRIES + usize::from(self.width.index(page, level));
            let entry = self.tables.read_u64(slot).ok_or(Error::OutOfTables)?;
            table = if second_level::is_present(entry) {
                self.table_index(entry).ok_or(Error::OutOfTables)?
            } else {
                let child = self.table_count;
                if child >= TABLE_LIMIT {
                    return Err(Error::OutOfTables);
                }
                // The arena is zeroed at establishment and on retarget, so a carved table is empty.
                let pointer = second_level::table_pointer(self.table_address(child));
                if !self.tables.write_u64(slot, pointer) {
                    return Err(Error::OutOfTables);
                }
                self.table_count = child + 1;
                child
            };
        }
        let slot = table * ENTRIES + usize::from(self.width.index(page, 0));
        if self
            .tables
            .write_u64(slot, second_level::pte(page, DEVICE_READS, DEVICE_WRITES))
        {
            Ok(())
        } else {
            Err(Error::OutOfTables)
        }
    }

    fn unmap_page(&mut self, page: u64) -> Result<(), Error> {
        let mut table = 0usize;
        let top = self.width.levels() - 1;
        for level in (1..=top).rev() {
            let slot = table * ENTRIES + usize::from(self.width.index(page, level));
            let entry = self.tables.read_u64(slot).ok_or(Error::OutOfTables)?;
            if !second_level::is_present(entry) {
                return Ok(());
            }
            table = self.table_index(entry).ok_or(Error::OutOfTables)?;
        }
        let slot = table * ENTRIES + usize::from(self.width.index(page, 0));
        if self.tables.write_u64(slot, second_level::NOT_PRESENT) {
            Ok(())
        } else {
            Err(Error::OutOfTables)
        }
    }

    fn set_root_table(&mut self) -> Result<(), Error> {
        write_u64(
            &mut self.registers,
            vtd::register::ROOT_TABLE_ADDRESS,
            self.root.address().get(),
        )?;
        let command = set_root_table_pointer(self.global_command);
        write_u32(&mut self.registers, vtd::register::GLOBAL_COMMAND, command)?;
        self.poll_status(root_table_pointer_set, Error::RootTableRefused)?;
        let pointer = read_u64(&self.registers, vtd::register::ROOT_TABLE_ADDRESS)?;
        if root_table_address(pointer) == self.root.address().get() {
            Ok(())
        } else {
            Err(Error::RootTableRefused)
        }
    }

    fn set_interrupt_table(&mut self) -> Result<(), Error> {
        let pointer = interrupt_table_address(self.interrupts.address().get())
            .ok_or(Error::InterruptTableRefused)?;
        write_u64(
            &mut self.registers,
            vtd::register::INTERRUPT_REMAPPING_TABLE_ADDRESS,
            pointer,
        )?;
        let command = set_interrupt_table_pointer(self.global_command);
        write_u32(&mut self.registers, vtd::register::GLOBAL_COMMAND, command)?;
        self.poll_status(interrupt_table_pointer_set, Error::InterruptTableRefused)?;
        let observed = read_u64(
            &self.registers,
            vtd::register::INTERRUPT_REMAPPING_TABLE_ADDRESS,
        )?;
        if observed == pointer
            && interrupt_table_base(observed) == self.interrupts.address().get()
            && interrupt_table_has_two_entries(observed)
        {
            Ok(())
        } else {
            Err(Error::InterruptTableRefused)
        }
    }

    // SIRTP does not invalidate the interrupt-entry cache on baseline hardware.
    fn invalidate_interrupt_cache(&mut self) -> Result<(), Error> {
        let invalidate = InvalidationDescriptor::global_interrupt_entry_cache();
        let status_address =
            self.invalidation_queue.address().get() + 8 * INVALIDATION_STATUS_INDEX as u64;
        let wait = InvalidationDescriptor::wait_status(status_address, INVALIDATION_COMPLETE)
            .ok_or(Error::Misaligned)?;
        if !self
            .invalidation_queue
            .write_u64(INVALIDATION_STATUS_INDEX, 0)
            || !self.invalidation_queue.write_u64(0, invalidate.lo)
            || !self.invalidation_queue.write_u64(1, invalidate.hi)
            || !self.invalidation_queue.write_u64(2, wait.lo)
            || !self.invalidation_queue.write_u64(3, wait.hi)
        {
            return Err(Error::OutOfTables);
        }
        let queue = invalidation_queue_address(self.invalidation_queue.address().get())
            .ok_or(Error::Misaligned)?;
        write_u64(
            &mut self.registers,
            vtd::register::INVALIDATION_QUEUE_TAIL,
            0,
        )?;
        write_u64(
            &mut self.registers,
            vtd::register::INVALIDATION_QUEUE_ADDRESS,
            queue,
        )?;
        self.switch_queued_invalidation(Switch::On)?;
        let observed_queue = read_u64(&self.registers, vtd::register::INVALIDATION_QUEUE_ADDRESS)?;
        if observed_queue != queue {
            return Err(Error::InvalidationQueueRefused);
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        write_u64(
            &mut self.registers,
            vtd::register::INVALIDATION_QUEUE_TAIL,
            INVALIDATION_TAIL,
        )?;
        let completed = self.wait(|unit| {
            if unit
                .invalidation_queue
                .read_u64(INVALIDATION_STATUS_INDEX)
                .is_some_and(|value| low_u32(value) == INVALIDATION_COMPLETE)
            {
                return Ok(true);
            }
            // The status read also paces the fallback count like every register poll.
            let status = read_u32(&unit.registers, vtd::register::GLOBAL_STATUS)?;
            if status & global_status::QIES == 0 {
                return Err(Error::QueuedInvalidationIncomplete);
            }
            Ok(false)
        });
        // Register invalidation is unusable while QI is enabled, so QI goes off again.
        let disabled = self.switch_queued_invalidation(Switch::Off);
        match completed {
            Ok(true) => disabled,
            Ok(false) => Err(Error::QueuedInvalidationIncomplete),
            Err(reason) => Err(reason),
        }
    }

    fn switch_queued_invalidation(&mut self, state: Switch) -> Result<(), Error> {
        let on = state == Switch::On;
        self.global_command = if on {
            self.global_command | global_command::QIE
        } else {
            self.global_command & !global_command::QIE
        };
        write_u32(
            &mut self.registers,
            vtd::register::GLOBAL_COMMAND,
            self.global_command,
        )?;
        self.poll_status(
            |status| (status & global_status::QIES != 0) == on,
            if on {
                Error::QueuedInvalidationEnableTimeout
            } else {
                Error::QueuedInvalidationDisableTimeout
            },
        )
    }

    fn enable_interrupt_remapping(&mut self) -> Result<(), Error> {
        self.global_command = enable_interrupt_remapping(self.global_command);
        write_u32(
            &mut self.registers,
            vtd::register::GLOBAL_COMMAND,
            self.global_command,
        )?;
        self.poll_status(
            |status| {
                interrupt_remapping_enabled(status) && !compatibility_interrupts_enabled(status)
            },
            Error::InterruptRemappingTimeout,
        )
    }

    // The fault-event MSI bypasses interrupt remapping, so it stays masked.
    fn mask_fault_interrupt(&mut self) -> Result<(), Error> {
        let prior = read_u32(&self.registers, vtd::register::FAULT_EVENT_CONTROL)?;
        write_u32(
            &mut self.registers,
            vtd::register::FAULT_EVENT_CONTROL,
            prior | fault_event_control::MASK,
        )?;
        let observed = read_u32(&self.registers, vtd::register::FAULT_EVENT_CONTROL)?;
        if fault_event_control::masked(observed) {
            Ok(())
        } else {
            Err(Error::FaultInterruptMaskRefused)
        }
    }

    fn enable_translation(&mut self) -> Result<(), Error> {
        self.global_command = enable_translation(self.global_command);
        write_u32(
            &mut self.registers,
            vtd::register::GLOBAL_COMMAND,
            self.global_command,
        )?;
        self.poll_status(translation_enabled, Error::TranslationTimeout)
    }

    // Callers drain only once every requester is translated or unmapped, so later DMA faults.
    fn drain_dma(&mut self) -> Result<(), Error> {
        self.flush_write_buffer()?;
        let command_register = self.extended.iotlb_register_offset_bytes() + iotlb_command::COMMAND;
        write_u64(
            &mut self.registers,
            command_register,
            iotlb_command::global_with_drain(),
        )?;
        self.poll_u64(command_register, iotlb_command::global_completed)
    }

    fn invalidate(&mut self, caches: Caches) -> Result<(), Error> {
        self.flush_write_buffer()?;
        if caches == Caches::ContextAndIotlb {
            write_u64(
                &mut self.registers,
                vtd::register::CONTEXT_COMMAND,
                context_command::global(),
            )?;
            self.poll_u64(
                vtd::register::CONTEXT_COMMAND,
                context_command::global_completed,
            )?;
        }
        let command_register = self.extended.iotlb_register_offset_bytes() + iotlb_command::COMMAND;
        write_u64(
            &mut self.registers,
            command_register,
            iotlb_command::global(),
        )?;
        self.poll_u64(command_register, iotlb_command::global_completed)
    }

    fn flush_write_buffer(&mut self) -> Result<(), Error> {
        if self.capability.requires_write_buffer_flush() {
            let command = self.global_command | global_command::WBF;
            write_u32(&mut self.registers, vtd::register::GLOBAL_COMMAND, command)?;
            self.poll_status(
                |status| (status & global_status::WBFS) == 0,
                Error::InvalidationTimeout,
            )?;
        }
        Ok(())
    }

    fn poll_status(&self, ready: impl Fn(u32) -> bool, timeout: Error) -> Result<(), Error> {
        let register = vtd::register::GLOBAL_STATUS;
        self.wait(|unit| Ok(ready(read_u32(&unit.registers, register)?)))?
            .then_some(())
            .ok_or(timeout)
    }

    fn poll_u64(&self, register: u64, done: impl Fn(u64) -> bool) -> Result<(), Error> {
        self.wait(|unit| Ok(done(read_u64(&unit.registers, register)?)))?
            .then_some(())
            .ok_or(Error::InvalidationTimeout)
    }

    fn wait(&self, mut done: impl FnMut(&Self) -> Result<bool, Error>) -> Result<bool, Error> {
        let deadline = (self.clock)().map(|now| now.saturating_add(WAIT_NS));
        let mut attempts = POLL_LIMIT;
        loop {
            if done(self)? {
                return Ok(true);
            }
            match (deadline, (self.clock)()) {
                (Some(deadline), Some(now)) if now >= deadline => return Ok(false),
                (Some(_), Some(_)) => {}
                _ if attempts == 0 => return Ok(false),
                _ => attempts -= 1,
            }
            core::hint::spin_loop();
        }
    }
}

impl Plan {
    fn read(registers: &ReleasableDevice, window: u64) -> Result<Self, Error> {
        let version = Version::from_bits(read_u32(registers, vtd::register::VERSION)?);
        if !version.register_invalidation_supported() {
            return Err(Error::UnsupportedRegisterInvalidationVersion {
                version: version.bits(),
            });
        }
        let capability = Capability::from_bits(read_u64(registers, vtd::register::CAPABILITY)?);
        let extended =
            ExtendedCapability::from_bits(read_u64(registers, vtd::register::EXTENDED_CAPABILITY)?);
        let width = AddressWidth::choose(capability).ok_or(Error::NoAddressWidth {
            capability: capability.bits(),
        })?;
        if !capability.dma_draining() {
            return Err(Error::NoDmaDraining);
        }

        if !extended.interrupt_remapping() || !extended.queued_invalidation() {
            return Err(Error::NoInterruptRemapping);
        }
        if !extended.extended_interrupt_mode() {
            return Err(Error::NoExtendedInterruptMode);
        }
        let end = (extended.iotlb_register_offset_bytes() + IOTLB_BYTES).max(
            capability.fault_recording_offset_bytes()
                + u64::from(capability.number_of_fault_recording_registers()) * ENTRY_BYTES,
        );
        if end > window {
            return Err(Error::RegistersOutsideWindow(end));
        }
        let status = read_u32(registers, vtd::register::GLOBAL_STATUS)?;
        if !takeover_ready(status) {
            return Err(Error::FirmwareFacilitiesActive { status });
        }
        Ok(Self {
            capability,
            extended,
            width,
        })
    }
}

fn read_u64(registers: &ReleasableDevice, offset: u64) -> Result<u64, Error> {
    let low = read_u32(registers, offset)?;
    let high = read_u32(registers, offset + 4)?;
    Ok(u64::from(low) | (u64::from(high) << 32))
}

// Low half first, so a command bit in the high half fires last.
fn write_u64(registers: &mut ReleasableDevice, offset: u64, value: u64) -> Result<(), Error> {
    let (low, high) = u32_halves(value);
    write_u32(registers, offset, low)?;
    write_u32(registers, offset + 4, high)
}

fn read_u32(registers: &ReleasableDevice, offset: u64) -> Result<u32, Error> {
    let offset = usize_from_u64(offset);
    registers
        .read_u32(offset)
        .map_err(|_| Error::Register(offset))
}

fn write_u32(registers: &mut ReleasableDevice, offset: u64, value: u32) -> Result<(), Error> {
    let offset = usize_from_u64(offset);
    registers
        .write_u32(offset, value)
        .map_err(|_| Error::Register(offset))
}

fn write_entry(
    table: &mut PublishedIommuTable,
    index: u8,
    low: u64,
    high: u64,
) -> Result<(), Error> {
    if table.write_u64(usize::from(index) * 2, low)
        && table.write_u64(usize::from(index) * 2 + 1, high)
    {
        Ok(())
    } else {
        Err(Error::OutOfTables)
    }
}
