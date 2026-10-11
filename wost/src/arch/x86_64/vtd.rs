//! Intel VT-d: program one DMA-remapping domain over checked MMIO.
//!
//! The kernel maps the unit's register window (from the DMAR the firmware
//! reports) and hands it here. This unit takes the hardware over from firmware,
//! builds a root table, a context table for one storage function and a
//! second-level tree that maps only the frames [`map_segment`](Vtd::map_segment)
//! installs, so the device faults on any address it was not handed (D-357).
//! Every bit layout is `libs/vtd`; every frame is an `mm::IommuTable`;
//! the only volatile access is the checked register window, so this file adds
//! no `unsafe`. Invalidation goes through the Context Command and IOTLB
//! registers rather than an invalidation queue, and a unit that reports
//! non-coherent page walks gets its table lines flushed rather than refused
//! (D-358).
//!
//! The second-level tables live in one owned arena and are found by walking the
//! tree the hardware walks. Root, context, deny-all interrupt, private
//! invalidation-queue and second-level arenas are fixed frame owners with no
//! heap:
//! a storage candidate that allocates on the heap cannot restore its resource
//! baseline (D-199), and a unit moved by value through the candidate's frames
//! must stay small enough for the boot stack.

#[cfg(feature = "selftest")]
use alloc::vec::Vec;
use core::fmt;

use pci_config::Address;
use vtd::{
    AddressWidth, Capability, ContextEntry, ExtendedCapability, FaultRecording, FaultStatus,
    InvalidationDescriptor, RootEntry, Version, compatibility_interrupts_enabled, context_command,
    enable_interrupt_remapping, enable_translation, fault_event_control, gcmd, gsts,
    interrupt_remapping_enabled, interrupt_table_address, interrupt_table_base,
    interrupt_table_has_two_entries, interrupt_table_pointer_set, invalidation_queue_address,
    iotlb_command, quiesced, root_table_address, root_table_pointer_set, second_level,
    set_interrupt_table_pointer, set_root_table_pointer, takeover_command, translation_enabled,
};

use crate::arch::x86_64::paging::ReleasableDevice;
use crate::mm::{DmaSegment, IommuTable, TableWalks};

/// The single domain identifier this unit programs.
const DOMAIN_ID: u16 = 1;

/// How many times a status poll spins before it gives up on the hardware.
const POLL_LIMIT: u32 = 1_000_000;

/// Four kibibytes: the frame size and second-level page granularity.
const PAGE: u64 = 4096;

/// Eight-byte entries in one second-level table.
const ENTRIES: usize = (PAGE / 8) as usize;

/// Bytes in one root, context or fault-recording entry.
const ENTRY_BYTES: usize = 16;

/// Two 128-bit queued-invalidation descriptors occupy 32 bytes.
const INVALIDATION_TAIL: u64 = 32;

/// Completion word sits outside the submitted descriptor prefix.
const INVALIDATION_STATUS_INDEX: usize = 8;

/// Value written by the wait descriptor after the IEC invalidation completes.
const INVALIDATION_COMPLETE: u32 = 1;

/// Second-level tables one domain may hold, the top table included.
///
/// Every 2 MiB region a mapped segment touches costs one leaf table, plus one
/// table per upper level, and the four queues and one transfer buffer fit in a
/// handful. The arena is allocated whole when the domain is established, so
/// mapping never allocates and a full arena refuses rather than grows.
const TABLE_LIMIT: usize = 24;

/// Frames retained by one unit owner: root, context, IR table, QI queue and
/// the fixed second-level arena.
pub const OWNED_TABLE_FRAMES: usize = TABLE_LIMIT + 4;

/// Why a VT-d domain could not be established or changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VtdError {
    /// The supplied run is empty or its last byte wraps the address space.
    InvalidRange,
    /// The supplied run extends beyond this unit's effective address width.
    OutsideAddressWidth,
    /// The register window refused a read or write at the named offset.
    Register(usize),
    /// This architecture version ignores the register invalidation interface.
    UnsupportedRegisterInvalidationVersion {
        /// Raw Version register for the refused unit.
        version: u32,
    },
    /// The unit reports no modelled page-table depth.
    NoAddressWidth {
        /// Raw capability register for the refused unit.
        capability: u64,
    },
    /// The unit cannot prove that completed TE transitions drain both DMA
    /// directions.
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
    /// A region firmware reserves for the function would not map.
    ReservedRegion {
        /// First byte of the region.
        base: u64,
        /// Last byte of the region.
        limit: u64,
    },
    /// Firmware's translation or invalidation queue did not reach the requested
    /// quiescent state, so the live state is uncertain.
    QuiescenceTimeout,
    /// The root-table pointer did not take: the status never reported it set,
    /// or the pointer read back names another table.
    RootTableRefused,
    /// The interrupt-table pointer did not take or read back exactly.
    InterruptTableRefused,
    /// Interrupt remapping did not reach the requested state.
    InterruptRemappingTimeout,
    /// The unit did not retain the mask on its non-remapped fault-event MSI.
    FaultInterruptMaskRefused,
    /// An established interrupt-remapping owner no longer proves deny-all.
    InterruptBoundaryLost,
    /// A parked owner unexpectedly still reports DMA translation enabled.
    TranslationStillActive,
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
            Self::ReservedRegion { base, limit } => {
                write!(f, "reserved region [{base:#x}, {limit:#x}] would not map")
            }
            Self::QuiescenceTimeout => f.write_str("firmware's translation state did not quiesce"),
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
            Self::TranslationStillActive => {
                f.write_str("the parked unit still reports translation enabled")
            }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A programmed VT-d domain owner: after successful establishment translation
/// is active through tables that reach only mapped segments; after an
/// uncertain transition the complete value is retained without assuming the
/// live hardware state.
#[derive(Debug)]
pub struct Vtd {
    registers: ReleasableDevice,
    function: Address,
    capability: Capability,
    extended: ExtendedCapability,
    width: AddressWidth,
    walks: TableWalks,
    iotlb_base: usize,
    fault_offset: usize,
    fault_registers: u16,
    /// The command word every write to `GCMD` is built from (D-358).
    gcmd: u32,
    root: IommuTable,
    context: IommuTable,
    /// A two-entry interrupt table. Both entries remain not-present.
    interrupts: IommuTable,
    /// One-page queue used only while globally invalidating the IEC.
    invalidation_queue: IommuTable,
    /// The second-level tables, one frame each; table 0 is the top level the
    /// context points at, and the first `table_count` are carved.
    tables: IommuTable,
    table_count: usize,
    #[cfg(feature = "selftest")]
    forced_invalidation_failures: u8,
    #[cfg(feature = "selftest")]
    completed_dma_drains: u64,
}

/// Proof that a DMA-remapping domain is active for one exact function.
///
/// Minted only by [`Vtd::domain`] after translation is confirmed on and the
/// function's context entry names the domain's own tables, and consumed by
/// `wost::pci::admit_bus_master`, so no path enables bus mastering without the
/// fence in place (D-357). Neither `Clone` nor `Copy`, so no proof outlives the
/// admission that spends it.
#[derive(Debug)]
#[must_use = "a DMA domain proof is required to admit bus mastering"]
pub struct DmaDomain {
    function: Address,
}

/// Linear proof that this unit blocks every interrupt from its exact function.
///
/// Minted only after IRE, EIME and the compatibility-format block read back,
/// IRTA names the owner's empty table, and queued invalidation is off. It is
/// consumed beside [`DmaDomain`] when PCI admits BME (D-372).
#[derive(Debug)]
#[must_use = "interrupt isolation proof is required to admit bus mastering"]
pub struct InterruptIsolation {
    function: Address,
}

impl InterruptIsolation {
    /// The function whose unit-wide interrupt boundary minted this proof.
    pub const fn function(&self) -> Address {
        self.function
    }
}

impl DmaDomain {
    /// The function this domain isolates.
    pub const fn function(&self) -> Address {
        self.function
    }
}

/// Result of reusing a live interrupt owner for another DMA requester.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// A unit-wide write was attempted, so all possible hardware owners stay
    /// inseparable.
    Retained {
        /// Why the transition could not be proved complete.
        reason: VtdError,
        /// Registers and every table the unit may be using.
        isolation: Vtd,
    },
}

/// One recorded translation fault: the device named an address it was not
/// handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    gcmd: u32,
}

impl Vtd {
    /// Take the unit over and establish a domain for `function`.
    ///
    /// Reads the unit's capabilities, switches off whatever translation or
    /// invalidation queue firmware left running, builds the root, context and
    /// top second-level tables, maps `reserved` (the first and last byte of
    /// each region firmware reserves for the function), sets the root-table
    /// pointer and enables translation. On return, translation is active and
    /// every address the device names outside those regions faults until
    /// [`map_segment`](Self::map_segment) installs it.
    ///
    /// # Errors
    ///
    /// The register window comes back with the reason when the window refuses
    /// an access, the unit reports no usable width or domain, firmware's state
    /// will not quiesce, a table frame is unavailable or misaligned, a reserved
    /// region would not map, or the hardware does not accept the root-table
    /// pointer or translation.
    pub fn establish(
        registers: ReleasableDevice,
        function: Address,
        reserved: &[(u64, u64)],
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
        let mut unit = Self {
            registers,
            function,
            capability: plan.capability,
            extended: plan.extended,
            width: plan.width,
            walks,
            iotlb_base: plan.extended.iotlb_register_offset_bytes() as usize,
            fault_offset: plan.capability.fault_recording_offset_bytes() as usize,
            fault_registers: plan.capability.number_of_fault_recording_registers(),
            gcmd: plan.gcmd,
            root,
            context,
            interrupts,
            invalidation_queue,
            tables,
            table_count: 1,
            #[cfg(feature = "selftest")]
            forced_invalidation_failures: 0,
            #[cfg(feature = "selftest")]
            completed_dma_drains: 0,
        };
        if let Err(reason) = unit.prepare(reserved) {
            let Self { registers, .. } = unit;
            return Establishment::Refused { reason, registers };
        }
        match unit.transition() {
            Ok(()) => Establishment::Established(unit),
            Err(reason) => Establishment::Retained {
                reason,
                isolation: unit,
            },
        }
    }

    /// Build every unpublished table entry before the first unit-wide write.
    fn prepare(&mut self, reserved: &[(u64, u64)]) -> Result<(), VtdError> {
        let devfn = self.devfn();
        let bus = self.function.bus();
        let context_entry = ContextEntry::try_present(self.table_address(0), self.width, DOMAIN_ID)
            .ok_or(VtdError::Misaligned)?;
        write_entry(&mut self.context, devfn, context_entry.lo, context_entry.hi)?;
        let root_entry =
            RootEntry::try_present(self.context.address()).ok_or(VtdError::Misaligned)?;
        write_entry(&mut self.root, bus, root_entry.lo, root_entry.hi)?;
        for &(base, limit) in reserved {
            let len = limit.saturating_sub(base).saturating_add(1);
            if self.map_unpublished_range(base, len).is_err() {
                return Err(VtdError::ReservedRegion { base, limit });
            }
        }

        Ok(())
    }

    /// Quiesce the unit, drain every request issued before its BME fences,
    /// publish the prepared root, invalidate stale translations and enable the
    /// new domain.
    fn transition(&mut self) -> Result<(), VtdError> {
        self.mask_fault_interrupt()?;
        self.quiesce()?;
        self.drain_dma()?;
        self.set_root_table()?;
        self.invalidate(true)?;
        self.set_interrupt_table()?;
        self.invalidate_interrupt_cache()?;
        self.switch_interrupt_remapping(true)?;
        self.switch_translation(true)
    }

    /// Map a range into tables hardware cannot yet walk.
    ///
    /// Unlike [`Self::map_range`], this does not issue an invalidation command:
    /// on first takeover those registers may still belong to firmware, and on
    /// retarget the owner is deliberately parked with translation off.
    fn map_unpublished_range(&mut self, paddr: u64, len: u64) -> Result<(), VtdError> {
        let Some(last) = len.checked_sub(1).and_then(|last| paddr.checked_add(last)) else {
            return Err(VtdError::InvalidRange);
        };
        if !self.width.covers_on(self.capability, last) {
            return Err(VtdError::OutsideAddressWidth);
        }
        let start = paddr & !(PAGE - 1);
        let mut page = start;
        while page <= last {
            if let Err(reason) = self.map_page(page) {
                let mut rollback = start;
                while rollback < page {
                    self.unmap_page(rollback)?;
                    rollback += PAGE;
                }
                return Err(reason);
            }
            page += PAGE;
        }
        Ok(())
    }

    /// The function's index into the context table.
    const fn devfn(&self) -> u8 {
        (self.function.device() << 3) | self.function.function()
    }

    /// The physical address of second-level table `index` in the arena.
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
    pub const fn function(&self) -> Address {
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

    /// Map `segment` into the domain, identity, and invalidate the unit's
    /// cache. Any failure that cannot prove rollback retains the segment here,
    /// before a caller can accidentally recycle device-reachable frames.
    #[must_use = "an unmapped segment cannot be reached by the device"]
    pub fn map_segment(&mut self, segment: &mut DmaSegment) -> Result<(), SegmentChangeError> {
        let Some((paddr, len)) = segment.address().zip(segment.len()) else {
            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));
        };
        match self.map_range(paddr, len) {
            Err(error @ SegmentChangeError::Retained(_)) => {
                segment.retain();
                Err(error)
            }
            outcome => outcome,
        }
    }

    fn map_range(&mut self, paddr: u64, len: u64) -> Result<(), SegmentChangeError> {
        let Some(last) = len.checked_sub(1).and_then(|last| paddr.checked_add(last)) else {
            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));
        };
        if !self.width.covers_on(self.capability, last) {
            return Err(SegmentChangeError::Refused(VtdError::OutsideAddressWidth));
        }
        let start = paddr & !(PAGE - 1);
        let mut page = start;
        while page <= last {
            if let Err(reason) = self.map_page(page) {
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
        match self.invalidate(false) {
            Ok(()) => Ok(()),
            Err(reason) => match self.rollback_range(start, last) {
                Ok(()) => Err(SegmentChangeError::Refused(reason)),
                Err(rollback) => Err(SegmentChangeError::Retained(rollback)),
            },
        }
    }

    /// Remove `segment` from the domain and drain every older DMA request before
    /// returning. Any uncertain failure retains the segment before returning.
    #[must_use = "a still-mapped segment is still reachable by the device"]
    pub fn unmap_segment(&mut self, segment: &mut DmaSegment) -> Result<(), SegmentChangeError> {
        let Some((paddr, len)) = segment.address().zip(segment.len()) else {
            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));
        };
        let Some(last) = len.checked_sub(1).and_then(|last| paddr.checked_add(last)) else {
            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));
        };
        let mut page = paddr & !(PAGE - 1);
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
            Ok(())
        }
    }

    /// Clear an inclusive page range and drain every request that could still
    /// use one of its cached translations.
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

    /// Inject a bounded invalidation refusal for the machine ownership test.
    #[cfg(feature = "selftest")]
    pub fn force_invalidation_failures_for_test(&mut self, count: u8) {
        self.forced_invalidation_failures = count;
    }

    /// Number of read-and-write-drained IOTLB invalidations completed by this
    /// owner, including the takeover drain and every live revocation drain.
    #[cfg(feature = "selftest")]
    pub const fn completed_dma_drains_for_test(&self) -> u64 {
        self.completed_dma_drains
    }

    /// Mint a proof that this domain isolates the function it was established
    /// for: translation is on, the root entry for the bus names the context
    /// table, and the function's context entry names this domain's own tree.
    #[must_use]
    pub fn domain(&self) -> Option<DmaDomain> {
        let status = read_u32(&self.registers, vtd::GSTS as usize).ok()?;
        if !translation_enabled(status) {
            return None;
        }
        let root = RootEntry {
            lo: self.root.read_u64(usize::from(self.function.bus()) * 2)?,
            hi: self
                .root
                .read_u64(usize::from(self.function.bus()) * 2 + 1)?,
        };
        if !root.is_present() || root.context_table_address() != self.context.address() {
            return None;
        }
        let entry = ContextEntry {
            lo: self.context.read_u64(usize::from(self.devfn()) * 2)?,
            hi: self.context.read_u64(usize::from(self.devfn()) * 2 + 1)?,
        };
        let names_this_domain = entry.is_present()
            && entry.translation_type() == 0
            && entry.second_level_address() == self.table_address(0)
            && entry.address_width_encoding() == self.width.context_encoding()
            && entry.domain_id() == DOMAIN_ID;
        names_this_domain.then_some(DmaDomain {
            function: self.function,
        })
    }

    /// Mint proof that this unit's interrupt path is kernel-owned and deny-all.
    #[must_use]
    pub fn interrupt_isolation(&self) -> Option<InterruptIsolation> {
        let status = read_u32(&self.registers, vtd::GSTS as usize).ok()?;
        let fault_control = read_u32(&self.registers, vtd::FECTL as usize).ok()?;
        if !interrupt_remapping_enabled(status)
            || compatibility_interrupts_enabled(status)
            || status & gsts::QIES != 0
            || !fault_event_control::masked(fault_control)
        {
            return None;
        }
        let expected = interrupt_table_address(self.interrupts.address())?;
        let observed = read_u64(&self.registers, vtd::IRTA as usize).ok()?;
        if observed != expected
            || interrupt_table_base(observed) != self.interrupts.address()
            || !interrupt_table_has_two_entries(observed)
        {
            return None;
        }
        for index in 0..4 {
            if self.interrupts.read_u64(index)? != 0 {
                return None;
            }
        }
        Some(InterruptIsolation {
            function: self.function,
        })
    }

    /// Whether the unit's root-table pointer names this domain's root table.
    #[cfg(feature = "selftest")]
    pub fn walks_kernel_root_for_test(&self) -> Option<bool> {
        let pointer = read_u64(&self.registers, vtd::RTADDR as usize).ok()?;
        Some(root_table_address(pointer) == self.root.address())
    }

    /// Read and clear the first recorded translation fault, if any.
    ///
    /// An overflowed fault log is acknowledged first, because the unit records
    /// nothing new while the overflow stands.
    #[must_use]
    pub fn take_fault(&mut self) -> Option<Fault> {
        let status = FaultStatus::from_bits(read_u32(&self.registers, vtd::FSTS as usize).ok()?);
        if status.overflow() {
            write_u32(
                &mut self.registers,
                vtd::FSTS as usize,
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
        let base = self.fault_offset + usize::from(index) * ENTRY_BYTES;
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

    /// Drain and switch DMA translation off without ever dropping the unit's
    /// interrupt-remapping boundary.
    ///
    /// Once PCI devices become hostile, clearing IRE would reopen a
    /// compatibility-format vector injection interval between storage
    /// candidates. The register window and empty IR table therefore remain
    /// owned for the rest of the boot.
    pub fn park(&mut self) -> Result<(), VtdError> {
        self.switch_translation(false)?;
        self.interrupt_isolation()
            .map(|_| ())
            .ok_or(VtdError::InterruptBoundaryLost)
    }

    /// Reuse a parked interrupt owner for a replacement DMA requester.
    ///
    /// Table preparation changes only memory while TE is observed off. The
    /// first hardware publication separates a clean refusal from an uncertain
    /// transition. IRE, IRTA, the empty IR table and the fault-event mask are
    /// never changed.
    pub fn retarget(&mut self, function: Address, reserved: &[(u64, u64)]) -> Retargeting {
        let status = match read_u32(&self.registers, vtd::GSTS as usize) {
            Ok(status) => status,
            Err(reason) => return Retargeting::Retained(reason),
        };
        if translation_enabled(status) {
            return Retargeting::Retained(VtdError::TranslationStillActive);
        }
        if self.interrupt_isolation().is_none() {
            return Retargeting::Retained(VtdError::InterruptBoundaryLost);
        }
        if let Err(reason) = self.clear_dma_tables() {
            return Retargeting::Refused(reason);
        }
        self.function = function;
        self.table_count = 1;
        if let Err(reason) = self.prepare(reserved) {
            return Retargeting::Refused(reason);
        }
        if let Err(reason) = self
            .set_root_table()
            .and_then(|()| self.invalidate(true))
            .and_then(|()| self.switch_translation(true))
        {
            return Retargeting::Retained(reason);
        }
        if self.interrupt_isolation().is_none() {
            return Retargeting::Retained(VtdError::InterruptBoundaryLost);
        }
        Retargeting::Retargeted
    }

    /// Clear only the DMA tables while the unit is parked. The interrupt table
    /// is a distinct owned frame and is deliberately untouched.
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

    /// Every page the second-level tree the unit walks currently reaches,
    /// read back from the table frames rather than from any shadow of them.
    #[cfg(feature = "selftest")]
    pub fn reachable_pages_for_test(&self) -> Option<Vec<u64>> {
        let mut pages = Vec::new();
        self.collect_reachable(0, self.width.levels() - 1, &mut pages)?;
        Some(pages)
    }

    #[cfg(feature = "selftest")]
    fn collect_reachable(&self, table: usize, level: u8, pages: &mut Vec<u64>) -> Option<()> {
        for slot in 0..ENTRIES {
            let entry = self.tables.read_u64(table * ENTRIES + slot)?;
            if !second_level::is_present(entry) {
                continue;
            }
            if level == 0 {
                pages.push(second_level::address(entry));
            } else {
                self.collect_reachable(self.table_index(entry)?, level - 1, pages)?;
            }
        }
        Some(())
    }

    fn map_page(&mut self, page: u64) -> Result<(), VtdError> {
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
                // Carved from the arena zeroed at establishment, so the new
                // table is empty before any entry points at it.
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
            .write_u64(slot, second_level::pte(page, true, true))
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
        write_u64(
            &mut self.registers,
            vtd::RTADDR as usize,
            self.root.address(),
        )?;
        let command = set_root_table_pointer(self.gcmd);
        write_u32(&mut self.registers, vtd::GCMD as usize, command)?;
        self.poll_status(root_table_pointer_set, VtdError::RootTableRefused)?;
        let pointer = read_u64(&self.registers, vtd::RTADDR as usize)?;
        if root_table_address(pointer) == self.root.address() {
            Ok(())
        } else {
            Err(VtdError::RootTableRefused)
        }
    }

    fn set_interrupt_table(&mut self) -> Result<(), VtdError> {
        let pointer = interrupt_table_address(self.interrupts.address())
            .ok_or(VtdError::InterruptTableRefused)?;
        write_u64(&mut self.registers, vtd::IRTA as usize, pointer)?;
        let command = set_interrupt_table_pointer(self.gcmd);
        write_u32(&mut self.registers, vtd::GCMD as usize, command)?;
        self.poll_status(interrupt_table_pointer_set, VtdError::InterruptTableRefused)?;
        let observed = read_u64(&self.registers, vtd::IRTA as usize)?;
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
    /// SIRTP does not imply this invalidation on the baseline hardware the
    /// kernel supports. The wait descriptor's status write is the completion
    /// boundary; QI is disabled again before register-based DMA invalidation
    /// can be used.
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
        write_u64(&mut self.registers, vtd::IQT as usize, 0)?;
        write_u64(&mut self.registers, vtd::IQA as usize, queue)?;
        self.switch_queued_invalidation(true)?;
        let observed_queue = read_u64(&self.registers, vtd::IQA as usize)?;
        if observed_queue != queue {
            return Err(VtdError::InvalidationQueueRefused);
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        write_u64(&mut self.registers, vtd::IQT as usize, INVALIDATION_TAIL)?;
        let mut completed = false;
        for _ in 0..POLL_LIMIT {
            if self
                .invalidation_queue
                .read_u64(INVALIDATION_STATUS_INDEX)
                .is_some_and(|value| value as u32 == INVALIDATION_COMPLETE)
            {
                completed = true;
                break;
            }
            core::hint::spin_loop();
        }
        let disabled = self.switch_queued_invalidation(false);
        if completed {
            disabled
        } else {
            Err(VtdError::QueuedInvalidationIncomplete)
        }
    }

    fn switch_queued_invalidation(&mut self, on: bool) -> Result<(), VtdError> {
        self.gcmd = if on {
            self.gcmd | gcmd::QIE
        } else {
            self.gcmd & !gcmd::QIE
        };
        write_u32(&mut self.registers, vtd::GCMD as usize, self.gcmd)?;
        self.poll_status(
            |status| (status & gsts::QIES != 0) == on,
            if on {
                VtdError::QueuedInvalidationEnableTimeout
            } else {
                VtdError::QueuedInvalidationDisableTimeout
            },
        )
    }

    fn switch_interrupt_remapping(&mut self, on: bool) -> Result<(), VtdError> {
        self.gcmd = if on {
            enable_interrupt_remapping(self.gcmd)
        } else {
            self.gcmd & !(gcmd::IRE | gcmd::CFI)
        };
        write_u32(&mut self.registers, vtd::GCMD as usize, self.gcmd)?;
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
    /// Intel specifies that this message never traverses interrupt remapping,
    /// so polling fault records is the only compatible default-deny policy.
    fn mask_fault_interrupt(&mut self) -> Result<(), VtdError> {
        let prior = read_u32(&self.registers, vtd::FECTL as usize)?;
        write_u32(
            &mut self.registers,
            vtd::FECTL as usize,
            prior | fault_event_control::MASK,
        )?;
        let observed = read_u32(&self.registers, vtd::FECTL as usize)?;
        if fault_event_control::masked(observed) {
            Ok(())
        } else {
            Err(VtdError::FaultInterruptMaskRefused)
        }
    }

    fn switch_translation(&mut self, on: bool) -> Result<(), VtdError> {
        self.gcmd = if on {
            enable_translation(self.gcmd)
        } else {
            self.gcmd & !gcmd::TE
        };
        write_u32(&mut self.registers, vtd::GCMD as usize, self.gcmd)?;
        self.poll_status(
            |status| translation_enabled(status) == on,
            VtdError::TranslationTimeout,
        )
    }

    /// Clear firmware translation and queued invalidation after the caller has
    /// fenced every declared requester. A required TES-off transition drains;
    /// an already-quiesced unit still needs [`Self::drain_dma`] because the
    /// observed status alone says nothing about older requests.
    fn quiesce(&mut self) -> Result<(), VtdError> {
        let status = read_u32(&self.registers, vtd::GSTS as usize)?;
        if quiesced(status) {
            return Ok(());
        }
        write_u32(&mut self.registers, vtd::GCMD as usize, self.gcmd)?;
        self.poll_status(quiesced, VtdError::QuiescenceTimeout)
    }

    /// Complete an architected read/write drain. During a unit transition every
    /// requester is BME-fenced, so no later request can enter behind this
    /// boundary before the new root is published. During live revocation the
    /// affected leaves are already clear, so later requests fault instead of
    /// reaching memory that the caller may recycle.
    fn drain_dma(&mut self) -> Result<(), VtdError> {
        self.prepare_invalidation()?;
        let command_register = self.iotlb_base + iotlb_command::COMMAND_REGISTER as usize;
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

    fn invalidate(&mut self, context: bool) -> Result<(), VtdError> {
        self.prepare_invalidation()?;
        if context {
            write_u64(
                &mut self.registers,
                vtd::CCMD as usize,
                context_command::global(),
            )?;
            self.poll_u64(vtd::CCMD as usize, context_command::global_completed)?;
        }
        let command_register = self.iotlb_base + iotlb_command::COMMAND_REGISTER as usize;
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
            write_u32(&mut self.registers, vtd::GCMD as usize, command)?;
            self.poll_status(
                |status| (status & gsts::WBFS) == 0,
                VtdError::InvalidationTimeout,
            )?;
        }
        Ok(())
    }

    fn poll_status(&self, ready: impl Fn(u32) -> bool, timeout: VtdError) -> Result<(), VtdError> {
        for _ in 0..POLL_LIMIT {
            if ready(read_u32(&self.registers, vtd::GSTS as usize)?) {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(timeout)
    }

    fn poll_u64(&self, register: usize, done: impl Fn(u64) -> bool) -> Result<(), VtdError> {
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
    /// The command shadow inherits no firmware translation or interrupt state;
    /// the ownership-preserving transition clears all four facilities before
    /// publishing either kernel table (D-358, D-370, D-372).
    fn read(
        registers: &ReleasableDevice,
        platform_interrupt_remapping: bool,
    ) -> Result<Self, VtdError> {
        let version = Version::from_bits(read_u32(registers, vtd::VERSION as usize)?);
        if !version.register_invalidation_supported() {
            return Err(VtdError::UnsupportedRegisterInvalidationVersion {
                version: version.bits(),
            });
        }
        let capability = Capability::from_bits(read_u64(registers, vtd::CAP as usize)?);
        let extended = ExtendedCapability::from_bits(read_u64(registers, vtd::ECAP as usize)?);
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
        let status = read_u32(registers, vtd::GSTS as usize)?;
        let gcmd = takeover_command(status);
        Ok(Self {
            capability,
            extended,
            width,
            gcmd,
        })
    }
}

/// Read a 64-bit register as its two aligned 32-bit halves, low first.
fn read_u64(registers: &ReleasableDevice, offset: usize) -> Result<u64, VtdError> {
    let low = read_u32(registers, offset)?;
    let high = read_u32(registers, offset + 4)?;
    Ok(u64::from(low) | (u64::from(high) << 32))
}

/// Write a 64-bit register as its two aligned 32-bit halves, low first so a
/// command bit in the high half triggers after the low half is in place.
fn write_u64(registers: &mut ReleasableDevice, offset: usize, value: u64) -> Result<(), VtdError> {
    write_u32(registers, offset, value as u32)?;
    write_u32(registers, offset + 4, (value >> 32) as u32)
}

fn read_u32(registers: &ReleasableDevice, offset: usize) -> Result<u32, VtdError> {
    registers
        .read_u32(offset)
        .map_err(|_| VtdError::Register(offset))
}

fn write_u32(registers: &mut ReleasableDevice, offset: usize, value: u32) -> Result<(), VtdError> {
    registers
        .write_u32(offset, value)
        .map_err(|_| VtdError::Register(offset))
}

/// Write one sixteen-byte table entry as its low and high eight-byte halves.
fn write_entry(table: &mut IommuTable, index: u8, lo: u64, hi: u64) -> Result<(), VtdError> {
    if table.write_u64(usize::from(index) * 2, lo)
        && table.write_u64(usize::from(index) * 2 + 1, hi)
    {
        Ok(())
    } else {
        Err(VtdError::OutOfTables)
    }
}
