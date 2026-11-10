//! Intel VT-d: take one unit over from firmware behind a default-deny root before PCI discovery.
//!
//! One storage function's context maps only the frames [`map_segment`](Vtd::map_segment) installs,
//! so every other requester and every address the device was not handed faults.
//! DMA invalidation uses the Context Command and IOTLB registers, and a unit with non-coherent
//! page walks gets its table lines flushed, never refused.
//! Every table is a fixed frame owner with no heap, since a storage candidate that allocates on
//! the heap cannot restore its resource baseline, and a unit moved by value must stay small enough
//! for the boot stack.

#[cfg(feature = "selftest")]
use alloc::vec::Vec;
use core::fmt;
use core::marker::PhantomData;

use pci_config::Address;
use register_bits::{low_u32, u32_halves, usize_from_u64};
use vtd::{
    AddressWidth, Capability, ContextEntry, ExtendedCapability, FaultRecording, FaultStatus,
    InvalidationDescriptor, RootEntry, Version, compatibility_interrupts_enabled, context_command,
    enable_interrupt_remapping, enable_translation, fault_event_control, gcmd, gsts,
    interrupt_remapping_enabled, interrupt_table_address, interrupt_table_base,
    interrupt_table_has_two_entries, interrupt_table_pointer_set, invalidation_queue_address,
    iotlb_command, root_table_address, root_table_pointer_set, second_level,
    set_interrupt_table_pointer, set_root_table_pointer, takeover_ready, translation_enabled,
};

use crate::arch::x86_64::paging::ReleasableDevice;
use crate::mm::{DmaMappingOwner, DmaSegment, IommuTable, PublishedIommuTable, TableWalks};

#[cfg(feature = "selftest")]
mod selftest;

/// The single domain identifier this unit programs.
const DOMAIN_ID: u16 = 1;

/// How many times a status poll spins before it gives up on the hardware.
const POLL_LIMIT: u32 = 1_000_000;

/// Four kibibytes: the frame size and second-level page granularity.
const PAGE: u64 = 4096;

/// Eight-byte entries in one second-level table.
const ENTRIES: usize = (PAGE / 8) as usize;

/// Bytes in one root, context or fault-recording entry.
const ENTRY_BYTES: u64 = 16;

/// Two 128-bit queued-invalidation descriptors.
const INVALIDATION_TAIL: u64 = 32;

/// The completion word's index, outside the submitted descriptor prefix.
const INVALIDATION_STATUS_INDEX: usize = 8;

/// Value written by the wait descriptor after the IEC invalidation completes.
const INVALIDATION_COMPLETE: u32 = 1;

/// Second-level tables one domain may hold, the top table included.
///
/// Each 2 MiB region a segment touches costs a leaf table plus one per upper level; the arena is
/// allocated whole at establishment, so mapping never allocates and a full arena refuses.
const TABLE_LIMIT: usize = 24;

/// Frames one unit owner retains: root, context, IR table, QI queue and the second-level arena.
pub const OWNED_TABLE_FRAMES: usize = TABLE_LIMIT + 4;

/// Why a VT-d domain could not be established or changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum VtdError {
    /// The supplied run is empty or its last byte wraps the address space.
    InvalidRange,
    /// The supplied run extends beyond this unit's effective address width.
    OutsideAddressWidth,
    /// The register window refused a read or write at the named offset.
    Register(usize),
    /// This architecture version ignores the register invalidation interface.
    UnsupportedRegisterInvalidationVersion { version: u32 },
    /// The unit reports no modelled page-table depth.
    NoAddressWidth { capability: u64 },
    /// The unit cannot prove that completed TE transitions drain both DMA directions.
    NoDmaDraining,
    /// Firmware did not advertise platform interrupt-remapping support.
    NoPlatformInterruptRemapping,
    /// The unit cannot remap interrupts or drive the invalidation queue.
    NoInterruptRemapping,
    /// The unit cannot deliver remapped interrupts in x2APIC mode.
    NoExtendedInterruptMode,
    /// The unit reports fewer domains than the one it is asked to hold.
    TooFewDomains,
    /// A table frame was unavailable, or the arena is full.
    OutOfTables,
    /// A table frame was not page aligned, so no entry can name it.
    Misaligned,
    /// Firmware left translation, queued invalidation, interrupt remapping or compatibility-format
    /// delivery active at takeover, in the Global Status the read-only preflight saw.
    FirmwareFacilitiesActive { status: u32 },
    /// The root-table pointer never reported set, or read back naming another table.
    RootTableRefused,
    /// The interrupt-table pointer did not take or read back exactly.
    InterruptTableRefused,
    /// Interrupt remapping did not reach the requested state.
    InterruptRemappingTimeout,
    /// The unit did not retain the mask on its non-remapped fault-event MSI.
    FaultInterruptMaskRefused,
    /// An established interrupt-remapping owner no longer proves deny-all.
    InterruptBoundaryLost,
    /// An operation needed a selected DMA function but the owner was parked.
    NoActiveDomain,
    /// Retargeting was attempted before the prior DMA function was parked.
    DomainStillActive,
    /// The queue address did not read back exactly.
    InvalidationQueueRefused,
    /// Queued invalidation did not reach the requested enabled state.
    QueuedInvalidationEnableTimeout,
    /// The submitted descriptor prefix did not complete.
    QueuedInvalidationIncomplete,
    /// Queued invalidation did not reach the requested disabled state.
    QueuedInvalidationDisableTimeout,
    /// Enabling or disabling translation did not take within the poll limit.
    TranslationTimeout,
    /// An invalidation did not complete within the poll limit.
    InvalidationTimeout,
}

impl fmt::Display for VtdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange => f.write_str("the DMA run is empty or wraps the address space"),
            Self::OutsideAddressWidth => {
                f.write_str("the DMA run extends beyond the unit's address width")
            }
            Self::Register(offset) => {
                write!(f, "the register window refused offset {offset:#x}")
            }
            Self::UnsupportedRegisterInvalidationVersion { version } => {
                let version = Version::from_bits(*version);
                write!(
                    f,
                    "VT-d version {}.{} does not support register invalidation",
                    version.major(),
                    version.minor()
                )
            }
            Self::NoAddressWidth { capability } => {
                write!(
                    f,
                    "no modelled page-table depth in capability {capability:#x}"
                )
            }
            Self::NoDmaDraining => f.write_str("the unit does not drain both DMA reads and writes"),
            Self::NoPlatformInterruptRemapping => {
                f.write_str("firmware does not permit platform interrupt remapping")
            }
            Self::NoInterruptRemapping => {
                f.write_str("the unit lacks interrupt remapping or queued invalidation")
            }
            Self::NoExtendedInterruptMode => {
                f.write_str("the unit cannot remap interrupts in x2apic mode")
            }
            Self::TooFewDomains => f.write_str("the unit reports no domain to isolate into"),
            Self::OutOfTables => f.write_str("no frame for a remapping table"),
            Self::Misaligned => f.write_str("a remapping table is not page aligned"),
            Self::FirmwareFacilitiesActive { status } => write!(
                f,
                "firmware left a remapping facility active in status {status:#x}"
            ),
            Self::RootTableRefused => f.write_str("the root-table pointer was not accepted"),
            Self::InterruptTableRefused => {
                f.write_str("the interrupt-table pointer was not accepted")
            }
            Self::InterruptRemappingTimeout => f.write_str("interrupt remapping did not switch"),
            Self::FaultInterruptMaskRefused => {
                f.write_str("the VT-d fault-event interrupt did not remain masked")
            }
            Self::InterruptBoundaryLost => {
                f.write_str("the unit lost its default-deny interrupt boundary")
            }
            Self::NoActiveDomain => f.write_str("the unit has no active DMA domain"),
            Self::DomainStillActive => f.write_str("the prior DMA domain is still active"),
            Self::InvalidationQueueRefused => {
                f.write_str("the invalidation-queue address was not accepted")
            }
            Self::QueuedInvalidationEnableTimeout => {
                f.write_str("queued invalidation did not enable")
            }
            Self::QueuedInvalidationIncomplete => {
                f.write_str("queued invalidation descriptors did not complete")
            }
            Self::QueuedInvalidationDisableTimeout => {
                f.write_str("queued invalidation did not disable")
            }
            Self::TranslationTimeout => f.write_str("translation did not switch"),
            Self::InvalidationTimeout => f.write_str("an invalidation did not complete"),
        }
    }
}

/// Whether a failed segment change left ownership safe to reuse.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum SegmentChangeError {
    /// No translation for the segment remains reachable or cached.
    Refused(VtdError),
    /// Device reachability is uncertain, so the segment was retained.
    Retained(VtdError),
}

impl fmt::Display for SegmentChangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(reason) => write!(f, "DMA segment change refused cleanly: {reason}"),
            Self::Retained(reason) => {
                write!(
                    f,
                    "DMA segment retained after an uncertain change: {reason}"
                )
            }
        }
    }
}

impl core::error::Error for SegmentChangeError {}

impl core::error::Error for VtdError {}

/// What the unit reported about itself when it was taken over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UnitFacts {
    /// The raw Capability register.
    pub capability: u64,
    /// The raw Extended Capability register.
    pub extended: u64,
    /// Page-table depth of the second-level tree the domain uses.
    pub levels: u8,
    /// Whether the unit snoops processor caches when it walks tables.
    pub coherent: bool,
    /// Whether not-present entries may be cached, so every change invalidates.
    pub caching_mode: bool,
    /// Whether the unit needs an explicit write-buffer flush before invalidation.
    pub write_buffer_flush: bool,
    /// Whether the unit offers queued invalidation, used for IEC invalidation.
    pub queued_invalidation: bool,
    /// Whether the unit offers interrupt remapping, owned default-deny here.
    pub interrupt_remapping: bool,
    /// Fault-recording registers the unit has.
    pub fault_registers: u16,
    /// Whether completed translation transitions drain DMA reads and writes.
    pub dma_draining: bool,
}

/// A programmed VT-d domain owner, retained whole after an uncertain transition.
///
/// After establishment, translation is active through tables that reach only mapped segments.
#[derive(Debug)]
pub struct Vtd {
    registers: ReleasableDevice,
    function: Option<Address>,
    capability: Capability,
    extended: ExtendedCapability,
    width: AddressWidth,
    walks: TableWalks,
    iotlb_base: u64,
    fault_offset: u64,
    fault_registers: u16,
    /// The command word every write to `GCMD` is built from.
    gcmd: u32,
    root: PublishedIommuTable,
    context: PublishedIommuTable,
    /// A two-entry interrupt table. Both entries remain not-present.
    interrupts: PublishedIommuTable,
    /// One-page queue used only while globally invalidating the IEC.
    invalidation_queue: PublishedIommuTable,
    /// One frame per second-level table; table 0 is the context's top level, and the first
    /// `table_count` are carved.
    tables: PublishedIommuTable,
    table_count: usize,
    #[cfg(feature = "selftest")]
    forced_invalidation_failures: u8,
    #[cfg(feature = "selftest")]
    completed_dma_drains: u64,
}

/// Proof that a DMA-remapping domain is active for one exact function.
///
/// Only [`Vtd::domain`] mints it and `wost::pci::admit_bus_master` consumes it, so no bus mastering
/// precedes the fence; it borrows the unit, so its tables cannot change while it is usable.
#[derive(Debug)]
#[must_use = "a DMA domain proof is required to admit bus mastering"]
pub struct DmaDomain<'unit> {
    function: Address,
    _unit: PhantomData<&'unit Vtd>,
}

/// Linear proof that this unit blocks every interrupt from its exact function.
///
/// Minted only once IRE, EIME and the compatibility block read back, IRTA names the empty table and
/// queued invalidation is off; consumed beside [`DmaDomain`] at BME, the unit borrowed until then.
#[derive(Debug)]
#[must_use = "interrupt isolation proof is required to admit bus mastering"]
pub struct InterruptIsolation<'unit> {
    function: Address,
    _unit: PhantomData<&'unit Vtd>,
}

impl InterruptIsolation<'_> {
    /// The function whose unit-wide interrupt boundary minted this proof.
    pub const fn function(&self) -> Address {
        self.function
    }
}

impl DmaDomain<'_> {
    /// The function this domain isolates.
    pub const fn function(&self) -> Address {
        self.function
    }
}

/// Result of reusing a live interrupt owner for another DMA requester.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[must_use = "a retargeted VT-d owner must be installed or quarantined"]
pub enum Retargeting {
    /// Translation is active through the replacement requester's tables.
    Retargeted,
    /// Only parked, unpublished memory was changed; another candidate is safe.
    Refused(VtdError),
    /// Hardware state may have changed, so selection must stop with the owner.
    Retained(VtdError),
}

/// The ownership-preserving result of establishing one VT-d domain.
#[derive(Debug)]
#[must_use = "a post-transition VT-d owner must be installed or quarantined"]
pub enum Establishment {
    /// Translation is active through the complete new owner.
    Established(Vtd),
    /// Read-only preflight or unpublished table preparation refused.
    Refused {
        /// Why establishment did not begin.
        reason: VtdError,
        /// Register authority remains safe to return.
        registers: ReleasableDevice,
    },
    /// A unit-wide write was attempted, so every possible hardware owner stays inseparable.
    Retained {
        /// Why the transition could not be proved complete.
        reason: VtdError,
        /// Registers and every table the unit may be using.
        isolation: Vtd,
    },
}

/// One recorded translation fault: the device named an address it was not handed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fault {
    /// The requester's bus and device-and-function, as the unit saw them.
    pub source: u16,
    /// The VT-d fault reason code.
    pub reason: u8,
    /// The page the device named.
    pub address: u64,
    /// Whether the device was writing into host memory.
    pub write: bool,
}

/// What the register window said before any table existed.
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

/// Which caches a register-based invalidation clears.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Caches {
    ContextAndIotlb,
    Iotlb,
}

impl Vtd {
    /// Take the unit over behind an empty, default-deny DMA root.
    ///
    /// Refuses every active firmware facility and installs empty kernel tables from observed TE
    /// off; every requester then faults until [`retarget`](Self::retarget) publishes one context.
    ///
    /// # Errors
    ///
    /// The window with the reason, if it refuses an access, the unit has no usable width or domain,
    /// firmware will not quiesce, a table frame is unavailable or misaligned, or the hardware
    /// refuses the root-table pointer or translation.
    pub fn establish_deny_all(
        registers: ReleasableDevice,
        platform_interrupt_remapping: bool,
    ) -> Establishment {
        let plan = match Plan::read(&registers, platform_interrupt_remapping) {
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
                reason: VtdError::OutOfTables,
                registers,
            };
        };
        let status = match read_u32(&registers, vtd::GSTS) {
            Ok(status) => status,
            Err(reason) => return Establishment::Refused { reason, registers },
        };
        if !takeover_ready(status) {
            return Establishment::Refused {
                reason: VtdError::FirmwareFacilitiesActive { status },
                registers,
            };
        }
        let mut unit = Self {
            registers,
            function: None,
            capability: plan.capability,
            extended: plan.extended,
            width: plan.width,
            walks,
            iotlb_base: plan.extended.iotlb_register_offset_bytes(),
            fault_offset: plan.capability.fault_recording_offset_bytes(),
            fault_registers: plan.capability.number_of_fault_recording_registers(),
            gcmd: 0,
            root: PublishedIommuTable::new(root),
            context: PublishedIommuTable::new(context),
            interrupts: PublishedIommuTable::new(interrupts),
            invalidation_queue: PublishedIommuTable::new(invalidation_queue),
            tables: PublishedIommuTable::new(tables),
            table_count: 1,
            #[cfg(feature = "selftest")]
            forced_invalidation_failures: 0,
            #[cfg(feature = "selftest")]
            completed_dma_drains: 0,
        };
        match unit.transition() {
            Ok(()) => Establishment::Established(unit),
            Err(reason) => Establishment::Retained {
                reason,
                isolation: unit,
            },
        }
    }

    /// Build one selected context and publish its bus root last.
    fn prepare(&mut self, function: Address) -> Result<(), VtdError> {
        let devfn = Self::devfn(function);
        let bus = function.bus();
        let context_entry = ContextEntry::try_present(self.table_address(0), self.width, DOMAIN_ID)
            .ok_or(VtdError::Misaligned)?;
        write_entry(&mut self.context, devfn, context_entry.lo, context_entry.hi)?;
        let root_entry =
            RootEntry::try_present(self.context.address()).ok_or(VtdError::Misaligned)?;
        write_entry(&mut self.root, bus, root_entry.lo, root_entry.hi)
    }

    /// Publish the kernel's empty roots from a firmware-quiescent state, preflight just repeated.
    ///
    /// A non-identical root is never installed while TE is active; after TE, a read and write drain
    /// settles every request that entered before the default-deny boundary went live.
    fn transition(&mut self) -> Result<(), VtdError> {
        self.mask_fault_interrupt()?;
        self.set_root_table()?;
        self.invalidate(Caches::ContextAndIotlb)?;
        self.set_interrupt_table()?;
        self.invalidate_interrupt_cache()?;
        self.switch_interrupt_remapping(Switch::On)?;
        self.switch_translation(Switch::On)?;
        self.drain_dma()?;
        self.boundary_active()
            .then_some(())
            .ok_or(VtdError::InterruptBoundaryLost)
    }

    const fn devfn(function: Address) -> u8 {
        (function.device() << 3) | function.function()
    }

    /// The physical address of arena table `index`.
    const fn table_address(&self, index: usize) -> u64 {
        self.tables.address() + index as u64 * PAGE
    }

    /// Which arena table a present entry's address names, if it is one.
    fn table_index(&self, entry: u64) -> Option<usize> {
        let offset = second_level::address(entry).checked_sub(self.tables.address())?;
        let index = usize::try_from(offset / PAGE).ok()?;
        (offset % PAGE == 0 && index < self.table_count).then_some(index)
    }

    /// The function this domain was established for.
    pub const fn function(&self) -> Option<Address> {
        self.function
    }

    /// What the unit reported about itself.
    pub fn facts(&self) -> UnitFacts {
        UnitFacts {
            capability: self.capability.bits(),
            extended: self.extended.bits(),
            levels: self.width.levels(),
            coherent: self.walks == TableWalks::Coherent,
            caching_mode: self.capability.caching_mode(),
            write_buffer_flush: self.capability.requires_write_buffer_flush(),
            queued_invalidation: self.extended.queued_invalidation(),
            interrupt_remapping: self.extended.interrupt_remapping(),
            fault_registers: self.fault_registers,
            dma_draining: self.capability.dma_draining(),
        }
    }

    /// Identity-map `segment` into the domain and invalidate the unit's cache.
    ///
    /// A failure that cannot prove rollback retains the segment before its frames can be recycled.
    #[must_use = "an unmapped segment cannot be reached by the device"]
    pub fn map_segment(&mut self, segment: &mut DmaSegment) -> Result<(), SegmentChangeError> {
        self.map_segment_with::<true, true>(segment)
    }

    /// Identity-map `segment` so the requester may read but not write it.
    #[must_use = "an unmapped segment cannot be reached by the device"]
    pub fn map_segment_for_device_read(
        &mut self,
        segment: &mut DmaSegment,
    ) -> Result<(), SegmentChangeError> {
        self.map_segment_with::<true, false>(segment)
    }

    /// Identity-map `segment` so the requester may write but not read it.
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
        let Some((physical, len)) = segment.address().zip(segment.len()) else {
            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));
        };
        let owner = DmaMappingOwner::new(self.root.address());
        if !segment.begin_mapping(owner) {
            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));
        }
        match self.map_range::<DEVICE_READS, DEVICE_WRITES>(physical, len) {
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
        len: u64,
    ) -> Result<(), SegmentChangeError> {
        let Some(last) = len
            .checked_sub(1)
            .and_then(|last| physical.checked_add(last))
        else {
            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));
        };
        if !self.width.covers_on(self.capability, last) {
            return Err(SegmentChangeError::Refused(VtdError::OutsideAddressWidth));
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

    /// Remove `segment` and drain every older DMA request; an uncertain failure retains it.
    #[must_use = "a still-mapped segment is still reachable by the device"]
    pub fn unmap_segment(&mut self, segment: &mut DmaSegment) -> Result<(), SegmentChangeError> {
        let owner = DmaMappingOwner::new(self.root.address());
        if !segment.is_mapped_by(owner) {
            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));
        }
        let Some((physical, len)) = segment.address().zip(segment.len()) else {
            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));
        };
        let Some(last) = len
            .checked_sub(1)
            .and_then(|last| physical.checked_add(last))
        else {
            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));
        };
        let mut page = physical & !(PAGE - 1);
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

    /// Clear an inclusive page range and drain every request that could use a cached translation.
    fn rollback_range(&mut self, start: u64, last: u64) -> Result<(), VtdError> {
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

    /// Mint proof of isolation: translation is on, the bus's root entry names the context table,
    /// and the function's context entry names this domain's own tree.
    #[must_use]
    pub fn domain(&self) -> Option<DmaDomain<'_>> {
        let function = self.function?;
        let status = read_u32(&self.registers, vtd::GSTS).ok()?;
        if !translation_enabled(status) {
            return None;
        }
        let root = RootEntry {
            lo: self.root.read_u64(usize::from(function.bus()) * 2)?,
            hi: self.root.read_u64(usize::from(function.bus()) * 2 + 1)?,
        };
        if !root.is_present() || root.context_table_address() != self.context.address() {
            return None;
        }
        let entry = ContextEntry {
            lo: self
                .context
                .read_u64(usize::from(Self::devfn(function)) * 2)?,
            hi: self
                .context
                .read_u64(usize::from(Self::devfn(function)) * 2 + 1)?,
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

    /// Mint proof that this unit's interrupt path is kernel-owned and deny-all.
    #[must_use]
    pub fn interrupt_isolation(&self) -> Option<InterruptIsolation<'_>> {
        let function = self.function?;
        self.boundary_active().then_some(InterruptIsolation {
            function,
            _unit: PhantomData,
        })
    }

    /// Whether translation and deny-all interrupt remapping remain kernel-owned.
    #[must_use]
    pub fn boundary_active(&self) -> bool {
        let Ok(pointer) = read_u64(&self.registers, vtd::RTADDR) else {
            return false;
        };
        let Ok(status) = read_u32(&self.registers, vtd::GSTS) else {
            return false;
        };
        let Ok(fault_control) = read_u32(&self.registers, vtd::FECTL) else {
            return false;
        };
        if !translation_enabled(status)
            || !interrupt_remapping_enabled(status)
            || compatibility_interrupts_enabled(status)
            || status & gsts::QIES != 0
            || !fault_event_control::masked(fault_control)
            || root_table_address(pointer) != self.root.address()
        {
            return false;
        }
        let Some(expected) = interrupt_table_address(self.interrupts.address()) else {
            return false;
        };
        let Ok(observed) = read_u64(&self.registers, vtd::IRTA) else {
            return false;
        };
        if observed != expected
            || interrupt_table_base(observed) != self.interrupts.address()
            || !interrupt_table_has_two_entries(observed)
        {
            return false;
        }
        for index in 0..4 {
            if self.interrupts.read_u64(index) != Some(0) {
                return false;
            }
        }
        true
    }

    /// Read and clear the first recorded translation fault, if any.
    ///
    /// An overflow is acknowledged first, since the unit records nothing new while it stands.
    #[must_use]
    pub fn take_fault(&mut self) -> Option<Fault> {
        let status = FaultStatus::from_bits(read_u32(&self.registers, vtd::FSTS).ok()?);
        if status.overflow() {
            write_u32(
                &mut self.registers,
                vtd::FSTS,
                FaultStatus::acknowledge_overflow(),
            )
            .ok()?;
        }
        if !status.primary_pending() {
            return None;
        }
        let index = u16::from(status.fault_record_index());
        if index >= self.fault_registers {
            return None;
        }
        let base = self.fault_offset + u64::from(index) * ENTRY_BYTES;
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
            reason: record.fault_reason(),
            address: record.fault_address(),
            write: !record.is_read_request(),
        })
    }

    /// Clear every recorded fault, returning how many there were.
    pub fn drain_faults(&mut self) -> usize {
        let mut drained = 0;
        while drained < usize::from(self.fault_registers) && self.take_fault().is_some() {
            drained += 1;
        }
        drained
    }

    /// Revoke the selected context while keeping translation default-deny.
    ///
    /// The root entry goes absent before invalidation and a full drain, and only then are context
    /// and second-level bytes cleared; TE, IRE, the window and both roots stay owned for the boot.
    pub fn park(&mut self) -> Result<(), VtdError> {
        if !self.boundary_active() {
            return Err(VtdError::InterruptBoundaryLost);
        }
        let function = self.function.ok_or(VtdError::NoActiveDomain)?;
        write_entry(&mut self.root, function.bus(), 0, 0)?;
        self.invalidate(Caches::ContextAndIotlb)?;
        self.drain_dma()?;
        self.clear_dma_tables()?;
        self.function = None;
        self.boundary_active()
            .then_some(())
            .ok_or(VtdError::InterruptBoundaryLost)
    }

    /// Reuse a live default-deny owner for a replacement DMA requester.
    ///
    /// Tables are prepared behind an absent root entry, whose publication is the first live change;
    /// global invalidation follows while TE and IRE stay enabled.
    pub fn retarget(&mut self, function: Address) -> Retargeting {
        if !self.boundary_active() {
            return Retargeting::Retained(VtdError::InterruptBoundaryLost);
        }
        if self.function.is_some() {
            return Retargeting::Retained(VtdError::DomainStillActive);
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
            return Retargeting::Retained(VtdError::InterruptBoundaryLost);
        }
        Retargeting::Retargeted
    }

    /// Clear the DMA tables of a parked unit, leaving the distinct interrupt-table frame untouched.
    fn clear_dma_tables(&mut self) -> Result<(), VtdError> {
        for index in 0..ENTRIES {
            if !self.root.write_u64(index, 0) || !self.context.write_u64(index, 0) {
                return Err(VtdError::OutOfTables);
            }
        }
        for index in 0..TABLE_LIMIT * ENTRIES {
            if !self.tables.write_u64(index, 0) {
                return Err(VtdError::OutOfTables);
            }
        }
        Ok(())
    }

    fn map_page<const DEVICE_READS: bool, const DEVICE_WRITES: bool>(
        &mut self,
        page: u64,
    ) -> Result<(), VtdError> {
        let mut table = 0usize;
        let top = self.width.levels() - 1;
        for level in (1..=top).rev() {
            let slot = table * ENTRIES + usize::from(self.width.index(page, level));
            let entry = self.tables.read_u64(slot).ok_or(VtdError::OutOfTables)?;
            table = if second_level::is_present(entry) {
                self.table_index(entry).ok_or(VtdError::OutOfTables)?
            } else {
                let child = self.table_count;
                if child >= TABLE_LIMIT {
                    return Err(VtdError::OutOfTables);
                }
                // Carved from the arena zeroed at establishment, so it is empty before it is named.
                let pointer = second_level::table_pointer(self.table_address(child));
                if !self.tables.write_u64(slot, pointer) {
                    return Err(VtdError::OutOfTables);
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
            Err(VtdError::OutOfTables)
        }
    }

    fn unmap_page(&mut self, page: u64) -> Result<(), VtdError> {
        let mut table = 0usize;
        let top = self.width.levels() - 1;
        for level in (1..=top).rev() {
            let slot = table * ENTRIES + usize::from(self.width.index(page, level));
            let entry = self.tables.read_u64(slot).ok_or(VtdError::OutOfTables)?;
            if !second_level::is_present(entry) {
                // Nothing below this entry is mapped, so the page is not either.
                return Ok(());
            }
            table = self.table_index(entry).ok_or(VtdError::OutOfTables)?;
        }
        let slot = table * ENTRIES + usize::from(self.width.index(page, 0));
        if self.tables.write_u64(slot, second_level::NOT_PRESENT) {
            Ok(())
        } else {
            Err(VtdError::OutOfTables)
        }
    }

    fn set_root_table(&mut self) -> Result<(), VtdError> {
        write_u64(&mut self.registers, vtd::RTADDR, self.root.address())?;
        let command = set_root_table_pointer(self.gcmd);
        write_u32(&mut self.registers, vtd::GCMD, command)?;
        self.poll_status(root_table_pointer_set, VtdError::RootTableRefused)?;
        let pointer = read_u64(&self.registers, vtd::RTADDR)?;
        if root_table_address(pointer) == self.root.address() {
            Ok(())
        } else {
            Err(VtdError::RootTableRefused)
        }
    }

    fn set_interrupt_table(&mut self) -> Result<(), VtdError> {
        let pointer = interrupt_table_address(self.interrupts.address())
            .ok_or(VtdError::InterruptTableRefused)?;
        write_u64(&mut self.registers, vtd::IRTA, pointer)?;
        let command = set_interrupt_table_pointer(self.gcmd);
        write_u32(&mut self.registers, vtd::GCMD, command)?;
        self.poll_status(interrupt_table_pointer_set, VtdError::InterruptTableRefused)?;
        let observed = read_u64(&self.registers, vtd::IRTA)?;
        if observed == pointer
            && interrupt_table_base(observed) == self.interrupts.address()
            && interrupt_table_has_two_entries(observed)
        {
            Ok(())
        } else {
            Err(VtdError::InterruptTableRefused)
        }
    }

    /// Globally invalidate the interrupt-entry cache through a private queue.
    ///
    /// SIRTP does not imply this on the supported baseline hardware. The wait descriptor's status
    /// write marks completion; QI is disabled again before register DMA invalidation can be used.
    fn invalidate_interrupt_cache(&mut self) -> Result<(), VtdError> {
        let invalidate = InvalidationDescriptor::global_interrupt_entry_cache();
        let status_address =
            self.invalidation_queue.address() + 8 * INVALIDATION_STATUS_INDEX as u64;
        let wait = InvalidationDescriptor::wait_status(status_address, INVALIDATION_COMPLETE)
            .ok_or(VtdError::Misaligned)?;
        if !self
            .invalidation_queue
            .write_u64(INVALIDATION_STATUS_INDEX, 0)
            || !self.invalidation_queue.write_u64(0, invalidate.lo)
            || !self.invalidation_queue.write_u64(1, invalidate.hi)
            || !self.invalidation_queue.write_u64(2, wait.lo)
            || !self.invalidation_queue.write_u64(3, wait.hi)
        {
            return Err(VtdError::OutOfTables);
        }
        let queue = invalidation_queue_address(self.invalidation_queue.address())
            .ok_or(VtdError::Misaligned)?;
        write_u64(&mut self.registers, vtd::IQT, 0)?;
        write_u64(&mut self.registers, vtd::IQA, queue)?;
        self.switch_queued_invalidation(Switch::On)?;
        let observed_queue = read_u64(&self.registers, vtd::IQA)?;
        if observed_queue != queue {
            return Err(VtdError::InvalidationQueueRefused);
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        write_u64(&mut self.registers, vtd::IQT, INVALIDATION_TAIL)?;
        let mut completed = false;
        for _ in 0..POLL_LIMIT {
            if self
                .invalidation_queue
                .read_u64(INVALIDATION_STATUS_INDEX)
                .is_some_and(|value| low_u32(value) == INVALIDATION_COMPLETE)
            {
                completed = true;
                break;
            }
            core::hint::spin_loop();
        }
        let disabled = self.switch_queued_invalidation(Switch::Off);
        if completed {
            disabled
        } else {
            Err(VtdError::QueuedInvalidationIncomplete)
        }
    }

    fn switch_queued_invalidation(&mut self, state: Switch) -> Result<(), VtdError> {
        let on = state == Switch::On;
        self.gcmd = if on {
            self.gcmd | gcmd::QIE
        } else {
            self.gcmd & !gcmd::QIE
        };
        write_u32(&mut self.registers, vtd::GCMD, self.gcmd)?;
        self.poll_status(
            |status| (status & gsts::QIES != 0) == on,
            if on {
                VtdError::QueuedInvalidationEnableTimeout
            } else {
                VtdError::QueuedInvalidationDisableTimeout
            },
        )
    }

    fn switch_interrupt_remapping(&mut self, state: Switch) -> Result<(), VtdError> {
        let on = state == Switch::On;
        self.gcmd = if on {
            enable_interrupt_remapping(self.gcmd)
        } else {
            self.gcmd & !(gcmd::IRE | gcmd::CFI)
        };
        write_u32(&mut self.registers, vtd::GCMD, self.gcmd)?;
        self.poll_status(
            |status| {
                interrupt_remapping_enabled(status) == on
                    && !compatibility_interrupts_enabled(status)
            },
            VtdError::InterruptRemappingTimeout,
        )
    }

    /// Permanently mask the unit's own fault-event MSI.
    ///
    /// It never traverses interrupt remapping, so polling fault records is the only default-deny.
    fn mask_fault_interrupt(&mut self) -> Result<(), VtdError> {
        let prior = read_u32(&self.registers, vtd::FECTL)?;
        write_u32(
            &mut self.registers,
            vtd::FECTL,
            prior | fault_event_control::MASK,
        )?;
        let observed = read_u32(&self.registers, vtd::FECTL)?;
        if fault_event_control::masked(observed) {
            Ok(())
        } else {
            Err(VtdError::FaultInterruptMaskRefused)
        }
    }

    fn switch_translation(&mut self, state: Switch) -> Result<(), VtdError> {
        let on = state == Switch::On;
        self.gcmd = if on {
            enable_translation(self.gcmd)
        } else {
            self.gcmd & !gcmd::TE
        };
        write_u32(&mut self.registers, vtd::GCMD, self.gcmd)?;
        self.poll_status(
            |status| translation_enabled(status) == on,
            VtdError::TranslationTimeout,
        )
    }

    /// Complete an architected read and write drain once every requester is behind translation or
    /// its root or leaves are clear, so later requests fault before reaching recyclable memory.
    fn drain_dma(&mut self) -> Result<(), VtdError> {
        self.prepare_invalidation()?;
        let command_register = self.iotlb_base + iotlb_command::COMMAND_REGISTER;
        write_u64(
            &mut self.registers,
            command_register,
            iotlb_command::global_with_drain(),
        )?;
        self.poll_u64(command_register, iotlb_command::global_completed)?;
        #[cfg(feature = "selftest")]
        {
            self.completed_dma_drains = self.completed_dma_drains.saturating_add(1);
        }
        Ok(())
    }

    fn invalidate(&mut self, caches: Caches) -> Result<(), VtdError> {
        self.prepare_invalidation()?;
        if caches == Caches::ContextAndIotlb {
            write_u64(&mut self.registers, vtd::CCMD, context_command::global())?;
            self.poll_u64(vtd::CCMD, context_command::global_completed)?;
        }
        let command_register = self.iotlb_base + iotlb_command::COMMAND_REGISTER;
        write_u64(
            &mut self.registers,
            command_register,
            iotlb_command::global(),
        )?;
        self.poll_u64(command_register, iotlb_command::global_completed)
    }

    fn prepare_invalidation(&mut self) -> Result<(), VtdError> {
        #[cfg(feature = "selftest")]
        if self.forced_invalidation_failures > 0 {
            self.forced_invalidation_failures -= 1;
            return Err(VtdError::InvalidationTimeout);
        }
        self.flush_write_buffer()
    }

    fn flush_write_buffer(&mut self) -> Result<(), VtdError> {
        if self.capability.requires_write_buffer_flush() {
            let command = self.gcmd | gcmd::WBF;
            write_u32(&mut self.registers, vtd::GCMD, command)?;
            self.poll_status(
                |status| (status & gsts::WBFS) == 0,
                VtdError::InvalidationTimeout,
            )?;
        }
        Ok(())
    }

    fn poll_status(&self, ready: impl Fn(u32) -> bool, timeout: VtdError) -> Result<(), VtdError> {
        for _ in 0..POLL_LIMIT {
            if ready(read_u32(&self.registers, vtd::GSTS)?) {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(timeout)
    }

    fn poll_u64(&self, register: u64, done: impl Fn(u64) -> bool) -> Result<(), VtdError> {
        for _ in 0..POLL_LIMIT {
            if done(read_u64(&self.registers, register)?) {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(VtdError::InvalidationTimeout)
    }
}

impl Plan {
    /// Read and validate every capability needed before the first unit write.
    ///
    /// Every firmware facility must already be inactive; this read-only boundary refuses a live
    /// non-identical SRTP and never guesses at firmware queue or remapping quiescence.
    fn read(
        registers: &ReleasableDevice,
        platform_interrupt_remapping: bool,
    ) -> Result<Self, VtdError> {
        let version = Version::from_bits(read_u32(registers, vtd::VERSION)?);
        if !version.register_invalidation_supported() {
            return Err(VtdError::UnsupportedRegisterInvalidationVersion {
                version: version.bits(),
            });
        }
        let capability = Capability::from_bits(read_u64(registers, vtd::CAP)?);
        let extended = ExtendedCapability::from_bits(read_u64(registers, vtd::ECAP)?);
        let width = AddressWidth::choose(capability).ok_or(VtdError::NoAddressWidth {
            capability: capability.bits(),
        })?;
        if u32::from(DOMAIN_ID) >= capability.number_of_domains() {
            return Err(VtdError::TooFewDomains);
        }
        if !capability.dma_draining() {
            return Err(VtdError::NoDmaDraining);
        }
        if !platform_interrupt_remapping {
            return Err(VtdError::NoPlatformInterruptRemapping);
        }
        if !extended.interrupt_remapping() || !extended.queued_invalidation() {
            return Err(VtdError::NoInterruptRemapping);
        }
        if !extended.extended_interrupt_mode() {
            return Err(VtdError::NoExtendedInterruptMode);
        }
        let status = read_u32(registers, vtd::GSTS)?;
        if !takeover_ready(status) {
            return Err(VtdError::FirmwareFacilitiesActive { status });
        }
        Ok(Self {
            capability,
            extended,
            width,
        })
    }
}

/// Read a 64-bit register as its two aligned 32-bit halves, low first.
fn read_u64(registers: &ReleasableDevice, offset: u64) -> Result<u64, VtdError> {
    let low = read_u32(registers, offset)?;
    let high = read_u32(registers, offset + 4)?;
    Ok(u64::from(low) | (u64::from(high) << 32))
}

/// Write a 64-bit register as two aligned 32-bit halves, low first so a command bit in the high
/// half triggers after the low half is in place.
fn write_u64(registers: &mut ReleasableDevice, offset: u64, value: u64) -> Result<(), VtdError> {
    let (low, high) = u32_halves(value);
    write_u32(registers, offset, low)?;
    write_u32(registers, offset + 4, high)
}

fn read_u32(registers: &ReleasableDevice, offset: u64) -> Result<u32, VtdError> {
    let offset = usize_from_u64(offset);
    registers
        .read_u32(offset)
        .map_err(|_| VtdError::Register(offset))
}

fn write_u32(registers: &mut ReleasableDevice, offset: u64, value: u32) -> Result<(), VtdError> {
    let offset = usize_from_u64(offset);
    registers
        .write_u32(offset, value)
        .map_err(|_| VtdError::Register(offset))
}

/// Write one sixteen-byte table entry as its low and high eight-byte halves.
fn write_entry(
    table: &mut PublishedIommuTable,
    index: u8,
    lo: u64,
    hi: u64,
) -> Result<(), VtdError> {
    if table.write_u64(usize::from(index) * 2, lo)
        && table.write_u64(usize::from(index) * 2 + 1, hi)
    {
        Ok(())
    } else {
        Err(VtdError::OutOfTables)
    }
}
