//! The kernel's own page tables (D-45).
//!
//! [`AddressSpace`] owns four-level tables; the host-tested encoding lives in
//! `x86_paging`, while raw walks and `mov cr3` stay here (D-30, D-45).
//! [`activate`] validates its own code and stack before switching.
//!
//! Present entries accept only identical rewrites. A clear invalidates when
//! the current `CR3` reaches its exact slot; distinct lower-half slots rely on
//! the no-PCID, non-global flush contract when their root is loaded (D-86).
//!
//! Mapping authority is typed: owned frames, boot-described memory, and device
//! holes enter through separate doors (D-87, D-199).

use alloc::vec::Vec;
use core::fmt;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicBool, AtomicUsize};

use mmio_range::{AddressRange, first_fit_by};
use stack_layout::{Slot, StackLayout, ranges_overlap};
use x86_paging::activation::State as ActivationState;
use x86_paging::device::{DeviceRange as CheckedDeviceRange, DeviceRangeError};
use x86_paging::{
    CanonicalAddress, ENTRIES, Entry, EntryError, Flags, LARGE_PAGE_SIZE, Level, MappingRange,
    MappingRangeError, PAGE_SIZE, TablePrivilege, page_identity_matches, user_mapping_fits,
};

use super::task::Released;
use super::{cpu, interrupt, pci};
use crate::mm::{self, Paddr, Segment, SharedBytes, SharedKeepalive, SharedRing, USegment};
use crate::mmio::MmioRegion;
use crate::sync::SpinLock;

/// Why a mapping could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MapError {
    /// No frame was free for a table this mapping needed.
    OutOfMemory,
    /// A table this mapping needs is outside the direct map, so it cannot be
    /// reached to be written.
    Unreachable(Paddr),
    /// The address range is already mapped by a larger page.
    Conflict(u64),
    /// The virtual address is not canonical, so no walk can reach it.
    NonCanonical(u64),
    /// The range's start or length is not page-aligned.
    Misaligned(u64),
    /// A device's registers are not four-byte aligned, so an [`MmioRegion`]
    /// over them would make misaligned accesses.
    UnalignedDevice(Paddr),
    /// A physical address did not fit the entry encoding.
    Encoding(EntryError),
    /// The complete mapping range failed arithmetic or architecture checks.
    InvalidRange(MappingRangeError),
    /// A device range was empty or its page-rounded extent overflowed.
    InvalidDeviceRange(Paddr),
    /// The running processor did not publish a usable `MAXPHYADDR`.
    PhysicalWidthUnavailable,
    /// A device mapping needs a physical-address bit this processor reserves.
    OutsidePhysicalWidth {
        /// Reported `MAXPHYADDR`.
        bits: u8,
        /// First requested register byte.
        physical: Paddr,
    },
    /// The range overlaps RAM or firmware storage rather than a device hole.
    NotDeviceMemory(Paddr),
    /// Another mapping in this address space already owns physical bytes.
    DeviceOverlap(Paddr),
    /// The bounded device-ownership table is full.
    TooManyDevices,
    /// The boot handoff never described this physical range, so there is no
    /// evidence it is memory rather than a guess (D-87).
    Undescribed(Paddr),
    /// Nothing is mapped at this address, so there is nothing to take away.
    NotMapped(u64),
    /// The mapping here is not one this space owns the frame under, so
    /// unmapping it would hand the allocator memory that was never its (D-88).
    NotOwned(u64),
    /// A large page covers this address. Splitting one is not offered: the
    /// direct map is deliberately unsplittable (D-45).
    Unsplittable(u64),
    /// The range's frames are not one ascending run, so they were not mapped
    /// by a single `map_owned` and cannot be returned as one segment.
    Discontiguous(u64),
    /// The range is outside the half this space owns. A space that shares the
    /// kernel's tables is not a handle onto them (D-105).
    NotYours(u64),
    /// The range touches a retained stack or its guard.
    Pinned(u64),
    /// A shared mapping requests a cache policy that would alias the direct
    /// map's write-back view of the same frames.
    CacheAlias(u64),
    /// A present mapping would be both writable and executable.
    WriteExecute(u64),
    /// A hardware-retained stack was requested in a space this CPU is not
    /// currently walking.
    Inactive(Paddr),
    /// Activation froze the kernel's top-level entries, so a later mapping
    /// cannot create a root that older user spaces would not share.
    KernelTopologyFrozen(u64),
}

/// A refused user mapping which retains its external-byte allocation.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UserMapError {
    /// Page-table construction refused before ownership moved into a leaf.
    Mapping {
        /// Why no mapping was committed.
        reason: MapError,
        /// The unchanged run the caller may retry or safely drop.
        run: USegment,
    },
}

impl UserMapError {
    /// Why no mapping was committed.
    pub const fn reason(&self) -> MapError {
        match self {
            Self::Mapping { reason, .. } => *reason,
        }
    }

    /// Recover the unchanged run for a retry.
    #[cfg(feature = "selftest")]
    pub fn into_run(self) -> USegment {
        match self {
            Self::Mapping { run, .. } => run,
        }
    }
}

impl fmt::Display for UserMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        core::fmt::Display::fmt(&self.reason(), f)
    }
}

impl core::error::Error for UserMapError {}

/// A stack rollback which stopped before every committed range was removed.
#[derive(Debug, PartialEq, Eq)]
pub struct StackRollback {
    /// Why the next stack could not be removed.
    pub reason: MapError,
    /// Stack mappings present when rollback began.
    pub mapped_stacks: usize,
    /// Pages already returned before the refusal.
    pub returned_pages: usize,
    root: Paddr,
    pending: StackCleanup,
}

#[derive(Debug, PartialEq, Eq)]
enum StackCleanup {
    One(Slot),
    Prefix {
        layout: StackLayout,
        remaining: usize,
    },
}

impl StackCleanup {
    #[cfg(feature = "selftest")]
    const fn address(&self) -> u64 {
        match self {
            Self::One(slot) => slot.stack_base(),
            Self::Prefix { layout, remaining } => match remaining.checked_sub(1) {
                Some(index) => match layout.slot(index) {
                    Some(slot) => slot.stack_base(),
                    None => layout.base(),
                },
                None => layout.base(),
            },
        }
    }

    const fn remaining_stacks(&self) -> usize {
        match self {
            Self::One(_) => 1,
            Self::Prefix { remaining, .. } => *remaining,
        }
    }
}

impl StackRollback {
    /// Stack mappings which still await removal.
    pub const fn remaining_stacks(&self) -> usize {
        self.pending.remaining_stacks()
    }
}

impl fmt::Display for StackRollback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "stack rollback returned {} page(s), then refused with {}",
            self.returned_pages, self.reason
        )
    }
}

impl core::error::Error for StackRollback {}

/// Why guarded-stack construction could not finish atomically.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StackMapError {
    /// Construction stopped without leaving a stack mapping behind.
    Mapping(MapError),
    /// Construction failed and removing its committed mappings also refused.
    Rollback {
        /// The construction failure which began rollback.
        cause: MapError,
        /// The exact partial cleanup result.
        cleanup: StackRollback,
    },
}

impl fmt::Display for StackMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mapping(reason) => fmt::Display::fmt(reason, f),
            Self::Rollback { cause, cleanup } => {
                write!(f, "{cause}; {cleanup}")
            }
        }
    }
}

impl core::error::Error for StackMapError {}

impl From<MapError> for StackMapError {
    fn from(reason: MapError) -> Self {
        Self::Mapping(reason)
    }
}

impl From<MappingRangeError> for MapError {
    fn from(error: MappingRangeError) -> Self {
        Self::InvalidRange(error)
    }
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::OutOfMemory => core::fmt::Write::write_str(f, "no free frame for a page table"),
            Self::Unreachable(address) => {
                write!(f, "page table at {address:#x} is outside the direct map")
            }
            Self::Conflict(address) => write!(f, "{address:#x} is already mapped by a larger page"),
            Self::NonCanonical(address) => write!(f, "{address:#x} is not a canonical address"),
            Self::Misaligned(address) => write!(f, "{address:#x} is not page-aligned"),
            Self::UnalignedDevice(address) => {
                write!(
                    f,
                    "device registers at {address:#x} are not four-byte aligned"
                )
            }
            Self::Encoding(error) => write!(f, "{error}"),
            Self::InvalidRange(error) => write!(f, "{error}"),
            Self::InvalidDeviceRange(address) => {
                write!(f, "device range at {address:#x} is empty or overflows")
            }
            Self::PhysicalWidthUnavailable => {
                core::fmt::Write::write_str(f, "the processor did not report a usable MAXPHYADDR")
            }
            Self::OutsidePhysicalWidth { bits, physical } => write!(
                f,
                "device range at {physical:#x} needs bits outside MAXPHYADDR {bits}"
            ),
            Self::Undescribed(address) => {
                write!(f, "{address:#x} is not described by the memory map")
            }
            Self::NotMapped(address) => write!(f, "nothing is mapped at {address:#x}"),
            Self::NotOwned(address) => {
                write!(f, "the mapping at {address:#x} does not own its frame")
            }
            Self::Unsplittable(address) => {
                write!(f, "{address:#x} is inside a large page, which is not split")
            }
            Self::NotYours(address) => {
                write!(f, "{address:#x} is not in the half this space owns")
            }
            Self::Pinned(address) => {
                write!(
                    f,
                    "the hardware-pinned stack at {address:#x} cannot be unmapped"
                )
            }
            Self::CacheAlias(address) => write!(
                f,
                "shared mapping at {address:#x} conflicts with the direct-map cache policy"
            ),
            Self::WriteExecute(address) => {
                write!(f, "mapping at {address:#x} is writable and executable")
            }
            Self::Inactive(root) => {
                write!(f, "the address space at {root:#x} is not active")
            }
            Self::KernelTopologyFrozen(address) => write!(
                f,
                "the active kernel cannot add a top-level mapping at {address:#x}"
            ),
            Self::Discontiguous(address) => {
                write!(f, "the frames behind {address:#x} are not one run")
            }
            Self::NotDeviceMemory(address) => {
                write!(f, "device range at {address:#x} overlaps ordinary memory")
            }
            Self::DeviceOverlap(address) => {
                write!(f, "device range at {address:#x} is already owned")
            }
            Self::TooManyDevices => {
                core::fmt::Write::write_str(f, "device ownership table is full")
            }
        }
    }
}

impl core::error::Error for MapError {}

/// Why a table was refused at the moment of the switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActivateError {
    /// A second caller tried to switch. The kernel's address space is
    /// installed once.
    AlreadyActive,
    /// The space's tables have gone back to the allocator, so it has no root
    /// to install (D-139).
    Reclaimed,
    /// The new tables do not map the code that would execute the switch.
    CodeUnmapped(u64),
    /// The new tables map the executing code without execute permission.
    CodeNotExecutable(u64),
    /// The new tables do not map the stack the switch is running on.
    StackUnmapped(u64),
    /// The new tables map the stack without write permission.
    StackReadOnly(u64),
    /// The new tables expose the kernel stack or give it a device cache mode.
    StackUnsafe(u64),
    /// A live virtual address was redirected to different physical memory.
    MappingChanged(u64),
    /// A kernel-image page is missing or has the wrong protection class.
    ImageChanged(u64),
    /// The new tables do not give this address of the direct map the writable,
    /// never-executable mapping every frame access assumes.
    DirectMapGap(u64),
    /// A space offered for binding disagrees with the kernel's about the
    /// higher half, at this top-level slot.
    KernelHalfDiffers(usize),
    /// A space offered for binding was not built to share the kernel's half.
    NotAUserSpace,
    /// The device window's shared top-level table could not be reserved before
    /// activation froze kernel topology.
    SharedTopology(MapError),
}

impl fmt::Display for ActivateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::AlreadyActive => {
                core::fmt::Write::write_str(f, "an address space is already installed")
            }
            Self::Reclaimed => core::fmt::Write::write_str(f, "the space has been reclaimed"),
            Self::CodeUnmapped(address) => {
                write!(f, "the switch's own code at {address:#x} is unmapped")
            }
            Self::CodeNotExecutable(address) => {
                write!(f, "the switch's own code at {address:#x} is not executable")
            }
            Self::StackUnmapped(address) => write!(f, "the stack at {address:#x} is unmapped"),
            Self::StackReadOnly(address) => write!(f, "the stack at {address:#x} is read-only"),
            Self::StackUnsafe(address) => {
                write!(f, "the stack at {address:#x} has unsafe permissions")
            }
            Self::MappingChanged(address) => {
                write!(f, "the mapping at {address:#x} changes physical memory")
            }
            Self::ImageChanged(address) => {
                write!(f, "the kernel image mapping at {address:#x} is wrong")
            }
            Self::DirectMapGap(address) => {
                write!(f, "the direct map at {address:#x} is missing or wrong")
            }
            Self::KernelHalfDiffers(index) => write!(
                f,
                "top-level slot {index} of the higher half is not the kernel's"
            ),
            Self::NotAUserSpace => {
                core::fmt::Write::write_str(f, "this space does not share the kernel's higher half")
            }
            Self::SharedTopology(reason) => {
                write!(f, "shared kernel topology could not be reserved: {reason}")
            }
        }
    }
}

impl core::error::Error for ActivateError {}

/// Why an address space kept its page-table frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReclaimError {
    /// This space was already reclaimed.
    Reclaimed,
    /// A task still names this bound space and no release evidence was given.
    StillBound(Paddr),
    /// The release evidence names a different space.
    WrongRelease {
        /// Root this space owns.
        expected: Paddr,
        /// Root the evidence releases.
        found: Paddr,
    },
    /// Release evidence was supplied to a space that was never bound.
    UnexpectedRelease(Paddr),
    /// The CPU is currently walking this space.
    Active(Paddr),
    /// Owned leaf mappings still depend on these tables.
    Mapped(usize),
    /// Device mappings still depend on these tables.
    Devices(usize),
}

impl fmt::Display for ReclaimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Reclaimed => {
                core::fmt::Write::write_str(f, "the address space was already reclaimed")
            }
            Self::StillBound(root) => write!(f, "a task still names the space at {root:#x}"),
            Self::WrongRelease { expected, found } => write!(
                f,
                "release evidence names {found:#x}, not the space at {expected:#x}"
            ),
            Self::UnexpectedRelease(found) => write!(
                f,
                "release evidence for {found:#x} was supplied to an unbound space"
            ),
            Self::Active(root) => write!(f, "the CPU is walking the space at {root:#x}"),
            Self::Mapped(pages) => write!(f, "{pages} owned page(s) remain mapped"),
            Self::Devices(mappings) => {
                write!(f, "{mappings} device mapping(s) remain live")
            }
        }
    }
}

impl core::error::Error for ReclaimError {}

/// A refused table reclaim and any one-shot release evidence still usable for retry.
#[derive(Debug)]
#[must_use = "a refused address-space reclaim may return release authority"]
pub struct ReclaimFailure {
    /// Why the tables remained owned.
    pub reason: ReclaimError,
    /// Task-table release evidence not consumed by the refusal.
    pub released: Option<Released>,
}

impl fmt::Display for ReclaimFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.reason, f)
    }
}

impl core::error::Error for ReclaimFailure {}

/// A page-rounded MMIO range admitted by this CPU and the immutable boot map.
#[derive(Debug)]
pub struct AdmittedDeviceRange {
    checked: CheckedDeviceRange,
    len: usize,
    physical_bits: u8,
}

/// A refused PCI BAR map with its unchanged function-bound authority.
#[derive(Debug)]
#[must_use = "a refused BAR map returns its mapping authority"]
pub struct BarMapFailure {
    /// Why no mapping was installed.
    pub reason: MapError,
    /// The unchanged BAR and physical-range authority.
    pub admitted: pci::AdmittedBar,
}

/// A refused PCI BAR unmap with its unchanged register and BAR authority.
#[derive(Debug)]
#[must_use = "a refused BAR unmap returns its cleanup authority"]
pub struct BarUnmapFailure {
    /// Why no mapping was removed.
    pub reason: MapError,
    /// The unchanged register access and removal authority.
    pub bar: pci::ReleasableBar,
}

/// Result of removing a function-bound BAR mapping.
#[derive(Debug)]
#[must_use = "a refused BAR unmap retains the only cleanup authority"]
pub enum BarUnmapOutcome {
    /// Every leaf and the device ledger entry were removed.
    Unmapped(pci_config::BarAuthority),
    /// Nothing changed and the exact mapping can be retried.
    Refused(BarUnmapFailure),
}

impl AdmittedDeviceRange {
    pub(crate) const fn physical(&self) -> Paddr {
        self.checked.physical()
    }

    pub(crate) const fn page_start(&self) -> Paddr {
        self.checked.page_start()
    }

    pub(crate) const fn page_end(&self) -> Paddr {
        self.checked.page_end()
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) const fn physical_bits(&self) -> u8 {
        self.physical_bits
    }
}

/// Admit every physical byte a device mapping would reach.
///
/// The boot memory map is initialized once, so this proof cannot be invalidated
/// before a mapping consumes it.
///
/// # Errors
///
/// Refuses an absent or invalid CPU width, invalid range arithmetic, or any
/// page overlapping RAM or firmware storage.
pub fn admit_device_range(physical: Paddr, len: u64) -> Result<AdmittedDeviceRange, MapError> {
    let width = cpu::physical_address_width().ok_or(MapError::PhysicalWidthUnavailable)?;
    let physical_bits = width.bits();
    let checked = CheckedDeviceRange::new(physical, len, width).map_err(|reason| match reason {
        DeviceRangeError::Empty | DeviceRangeError::Overflow => {
            MapError::InvalidDeviceRange(physical)
        }
        DeviceRangeError::Unaligned(at) => MapError::UnalignedDevice(at),
        DeviceRangeError::OutsideWidth { bits, .. } => {
            MapError::OutsidePhysicalWidth { bits, physical }
        }
        _ => MapError::InvalidDeviceRange(physical),
    })?;
    if !mm::device_range_is_eligible(checked.page_start(), checked.page_end()) {
        return Err(MapError::NotDeviceMemory(physical));
    }
    let len = usize::try_from(len).map_err(|_| MapError::InvalidDeviceRange(physical))?;
    Ok(AdmittedDeviceRange {
        checked,
        len,
        physical_bits,
    })
}

/// A four-level page table under construction.
///
/// Tables remain owned until explicit reclaim; `unmap` removes leaves without
/// risking a live sibling or freeing beneath `CR3` (D-88).
#[derive(Debug)]
pub struct AddressSpace {
    root: Option<Paddr>,
    /// Physical ranges already promised to device mappings in this space.
    devices: Vec<Option<DeviceExtent>>,
    /// The frames this space's own tables are built from, so they can go back.
    ///
    /// Not the shared kernel entries `for_user` copies — those are the
    /// kernel's tables and belong to it. Only what `set` allocated walking
    /// down, plus the root (D-117).
    tables: Vec<OwnedTable>,
    /// Which half of the address space this one may be told to change.
    reach: Reach,
    /// Whether a thread already runs on these tables.
    bound: bool,
    /// Allocations reached by non-owning user mappings in this space.
    shared: Vec<SharedKeepalive>,
    /// Leaf pages whose frame ownership moved into this space.
    owned_pages: usize,
}

#[derive(Debug, Clone, Copy)]
struct OwnedTable {
    frame: Paddr,
    virtual_address: u64,
    level: Level,
    link: Entry,
}

/// How much of an address space its owner may map.
///
/// A user space shares live kernel tables by reference, so it may change only
/// its lower half. Otherwise a safe handle could edit mappings the kernel is
/// running on (D-104, D-105).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// The whole space. The kernel's own, which owns every table in it.
    Everywhere,
    /// The lower half only. Everything above is somebody else's tables.
    UserHalfOnly,
}

/// The first address that is not user memory.
///
/// The canonical hole and kernel half begin here. User-copy code re-exports
/// this value so copy and mapping bounds cannot drift (D-124). No user mapping
/// reaches the last page below it (D-348). Spelled as a literal because it is
/// an ABI constant the C header reads; the assertion binds it to the library's.
pub const USER_END: u64 = 0x0000_8000_0000_0000;

const _: () = assert!(USER_END == x86_paging::USER_END);

/// Whether `address` is one ring 3 could name.
///
/// This range check also rejects the canonical hole without assuming a fixed
/// page-walk width (D-83, D-118).
pub const fn is_user_address(address: u64) -> bool {
    address < USER_END
}

/// Flags a caller may not choose, whatever it asks for.
///
/// `OWNED` records allocator provenance, `HUGE` comes from the level, and
/// `GLOBAL` would evade the flush an inactive mutable root relies on. Only the
/// mapping implementation may choose them (D-88, D-105).
const NOT_THE_CALLERS: Flags = Flags::OWNED.union(Flags::HUGE).union(Flags::GLOBAL);

/// The cache-type bits a caller may name on a mapping of RAM.
const CACHE_TYPE: Flags = Flags::NO_CACHE.union(Flags::WRITE_THROUGH);

/// RAM keeps its write-back direct-map type in every other mapping; a second
/// alias of another type corrupts it (D-349).
fn ram_cache_policy(virtual_base: u64, flags: Flags) -> Result<(), MapError> {
    if flags.intersection(CACHE_TYPE) != Flags::NONE {
        return Err(MapError::CacheAlias(virtual_base));
    }
    Ok(())
}

impl AddressSpace {
    /// An address space mapping nothing.
    ///
    /// # Errors
    ///
    /// [`MapError::OutOfMemory`] if no frame is free for the top-level table.
    pub fn new() -> Result<Self, MapError> {
        let root = take_table()?;
        Ok(Self {
            root: Some(root),
            devices: Vec::new(),
            tables: Vec::new(),
            bound: false,
            reach: Reach::Everywhere,
            shared: Vec::new(),
            owned_pages: 0,
        })
    }

    /// The physical address of the top-level table — what `CR3` would hold.
    pub const fn root(&self) -> Paddr {
        match self.root {
            Some(root) => root,
            None => panic!("a reclaimed address space has no root"),
        }
    }

    /// Map `len` bytes at `virtual_base` onto physical memory at
    /// `physical_base`, with `flags`.
    ///
    /// Large pages wherever the addresses and remaining length permit, and
    /// `PRESENT` is added; no other flag is.
    ///
    /// # Errors
    ///
    /// [`MapError::Misaligned`] unless both bases are page-aligned,
    /// [`MapError::NonCanonical`] if the virtual range leaves canonical space,
    /// [`MapError::Conflict`] if a larger page already covers part of the
    /// range, [`MapError::WriteExecute`] if the resulting present mapping
    /// would be writable and executable, [`MapError::Pinned`] if it overlaps a
    /// retained stack or guard, and [`MapError::OutOfMemory`] if a table cannot
    /// be allocated.
    fn map_range(
        &mut self,
        virtual_base: u64,
        physical_base: Paddr,
        len: u64,
        flags: Flags,
        grain: Grain,
    ) -> Result<(), MapError> {
        let range = MappingRange::new(virtual_base, physical_base, len)?;
        let flags = flags.union(Flags::PRESENT);

        if flags.is_write_execute() {
            return Err(MapError::WriteExecute(virtual_base));
        }

        if let Some(stack) = protected_stack_overlap(self.root(), virtual_base, len) {
            return Err(MapError::Pinned(stack));
        }

        if self.preflight(&range, flags, grain)? == MappingState::Identical {
            return Ok(());
        }

        let table_mark = self.tables.len();
        let mut offset = 0;
        while offset < range.len() {
            let (virtual_address, physical, level) = Self::step(&range, offset, grain);
            #[cfg(feature = "selftest")]
            let result = if mapping_refusal(&MAPPING_WRITES_BEFORE_REFUSAL) {
                Err(MapError::OutOfMemory)
            } else {
                self.set(virtual_address, physical, flags, level)
            };
            #[cfg(not(feature = "selftest"))]
            let result = self.set(virtual_address, physical, flags, level);
            if let Err(error) = result {
                // Rollback prevents a refused owned mapping from leaving live
                // entries over frames returned to the allocator (D-94).
                self.undo(&range, offset, grain);
                self.rollback_tables(table_mark);
                return Err(error);
            }
            offset += level.span();
        }
        Ok(())
    }

    /// Establish that every target is absent or every target is identical.
    fn preflight(
        &self,
        range: &MappingRange,
        flags: Flags,
        grain: Grain,
    ) -> Result<MappingState, MapError> {
        let mut transaction = None;
        let mut offset = 0;
        while offset < range.len() {
            let (virtual_address, physical, level) = Self::step(range, offset, grain);
            let state = self.mapping_state(virtual_address, physical, flags, level)?;
            match transaction {
                None => transaction = Some(state),
                Some(found) if found == state => {}
                Some(_) => return Err(MapError::Conflict(virtual_address)),
            }
            offset += level.span();
        }
        transaction.ok_or(MapError::Misaligned(range.virtual_base()))
    }

    /// The state of one target before a transaction writes anything.
    fn mapping_state(
        &self,
        virtual_address: u64,
        physical: Paddr,
        flags: Flags,
        target: Level,
    ) -> Result<MappingState, MapError> {
        let wanted = Entry::page(target, physical, flags).map_err(MapError::Encoding)?;
        let mut table = self.root();
        for level in Level::WALK {
            let slot = entry_slot(table, level.index(virtual_address))?;
            let current = EntrySlot::read(&slot);
            if level == target {
                return if !current.is_present() {
                    Ok(MappingState::Absent)
                } else if current == wanted {
                    Ok(MappingState::Identical)
                } else {
                    Err(MapError::Conflict(virtual_address))
                };
            }
            if !current.is_present() {
                return Ok(MappingState::Absent);
            }
            if current.is_huge()
                || flags.contains(Flags::USER) && !current.flags().contains(Flags::USER)
            {
                return Err(MapError::Conflict(virtual_address));
            }
            table = current.address();
        }
        Err(MapError::Conflict(virtual_address))
    }

    /// Whether this space may be told to change `len` bytes at `at`.
    ///
    /// The check that makes [`for_user`](Self::for_user) safe to expose. A
    /// space sharing the kernel's higher half by reference must not be a
    /// handle onto the kernel's tables (D-105).
    ///
    /// # Errors
    ///
    /// [`MapError::NotYours`] for a range this space does not own outright.
    fn may_reach(&self, at: u64, len: u64) -> Result<(), MapError> {
        // A reclaimed space has no tables. Mapping into one would walk frames
        // the allocator has given to something else (D-123).
        if self.is_reclaimed() {
            return Err(MapError::NotYours(at));
        }
        if self.reach == Reach::Everywhere {
            return Ok(());
        }
        at.checked_add(len).ok_or(MapError::NonCanonical(at))?;
        if !user_mapping_fits(at, len) {
            return Err(MapError::NotYours(at));
        }
        Ok(())
    }

    /// Where the mapping of `range` at `offset` goes, and at what size.
    ///
    /// Split out so [`undo`](Self::undo) retraces exactly the same steps: a
    /// rollback that disagreed with the walk about where a large page began
    /// would clear the wrong entry.
    fn step(range: &MappingRange, offset: u64, grain: Grain) -> (u64, Paddr, Level) {
        let virtual_address = range.virtual_base() + offset;
        let physical = range.physical_base() + offset;
        let large = grain == Grain::Largest
            && range.len() - offset >= LARGE_PAGE_SIZE
            && virtual_address.is_multiple_of(LARGE_PAGE_SIZE)
            && physical.is_multiple_of(LARGE_PAGE_SIZE);
        (
            virtual_address,
            physical,
            if large { Level::Pd } else { Level::Pt },
        )
    }

    /// Clear the first `written` bytes of `range`, undoing a partial mapping.
    ///
    /// [`preflight`](Self::preflight) established that every entry in a
    /// transaction which can reach this path was absent before it began, so
    /// each entry below belongs to this call rather than an older mapping.
    fn undo(&mut self, range: &MappingRange, written: u64, grain: Grain) {
        let mut offset = 0;
        while offset < written {
            let (virtual_address, _, level) = Self::step(range, offset, grain);
            #[cfg(feature = "selftest")]
            if mapping_refusal(&MAPPING_UNDOS_BEFORE_REFUSAL) {
                panic!("mapping rollback lost a written leaf: injected refusal");
            }
            let slot = self
                .slot_at(virtual_address, level)
                .unwrap_or_else(|reason| panic!("mapping rollback lost a written leaf: {reason}"));
            clear_mapping_slot(slot, virtual_address, level);
            offset += level.span();
        }
    }

    fn rollback_tables(&mut self, mark: usize) {
        assert!(
            mark <= self.tables.len(),
            "mapping rollback lost its table mark"
        );
        while self.tables.len() > mark {
            let owned = *self
                .tables
                .last()
                .unwrap_or_else(|| panic!("mapping rollback lost an owned table"));
            for index in 0..ENTRIES {
                let slot = entry_slot(owned.frame, index).unwrap_or_else(|reason| {
                    panic!("mapping rollback could not inspect an owned table: {reason}")
                });
                let entry = EntrySlot::read(&slot);
                assert!(
                    entry == Entry::ABSENT,
                    "mapping rollback tried to return a nonempty table"
                );
            }
            let parent = self
                .slot_at(owned.virtual_address, owned.level)
                .unwrap_or_else(|reason| {
                    panic!("mapping rollback lost an owned table link: {reason}")
                });
            let current = EntrySlot::read(&parent);
            assert!(
                current == owned.link,
                "mapping rollback found a different owned table link"
            );
            EntrySlot::write(&parent, Entry::ABSENT);
            // SAFETY: `take_owned_table` forgot exactly this allocation, and
            // the only page-table link to it is now absent.
            drop(unsafe { mm::Frame::from_raw(owned.frame) });
            self.tables.pop();
        }
    }

    /// The existing entry slot for `virtual_address` at `target`, allocating
    /// nothing.
    ///
    /// [`MapError::NotMapped`] where the walk runs out of tables, which during
    /// a rollback means the forward walk never got that far.
    fn slot_at(&self, virtual_address: u64, target: Level) -> Result<EntrySlot, MapError> {
        let mut table = self.root();
        for level in Level::WALK {
            let slot = entry_slot(table, level.index(virtual_address))?;
            if level == target {
                return Ok(slot);
            }
            let entry = EntrySlot::read(&slot);
            if !entry.is_present() || entry.is_huge() {
                return Err(MapError::NotMapped(virtual_address));
            }
            table = entry.address();
        }
        Err(MapError::NotMapped(virtual_address))
    }

    /// Map memory the boot handoff described, at `virtual_base`.
    ///
    /// [`BootRegion`] proves the boot memory map covers the physical range, so
    /// no safe mapping door accepts an unchecked physical address (D-64, D-87).
    ///
    /// # Errors
    ///
    /// As [`map_owned`](Self::map_owned).
    pub fn map_boot(
        &mut self,
        virtual_base: u64,
        region: BootRegion,
        flags: Flags,
    ) -> Result<(), MapError> {
        if flags.contains(Flags::USER) {
            return Err(MapError::NotYours(virtual_base));
        }
        if flags.intersection(CACHE_TYPE) != region.cache_type() {
            return Err(MapError::CacheAlias(virtual_base));
        }
        self.may_reach(virtual_base, region.len)?;
        self.map_range(
            virtual_base,
            region.base,
            region.len,
            flags.difference(NOT_THE_CALLERS),
            Grain::Largest,
        )
    }

    /// Map a run of frames this caller owns, at `virtual_base`.
    ///
    /// Ownership moves into the address space. A refusal rolls back every leaf
    /// before `run` returns to the allocator (D-44, D-94).
    ///
    /// # Errors
    ///
    /// [`MapError::Misaligned`] unless `virtual_base` is page-aligned,
    /// [`MapError::NonCanonical`] if the range leaves canonical space,
    /// [`MapError::Conflict`] if any page of it is already mapped to something
    /// else, [`MapError::WriteExecute`] if the mapping would be writable and
    /// executable, [`MapError::Pinned`] if it overlaps a retained stack or
    /// guard, and [`MapError::OutOfMemory`] if a table cannot be allocated.
    pub fn map_owned(
        &mut self,
        virtual_base: u64,
        run: Segment,
        flags: Flags,
    ) -> Result<(), MapError> {
        // A kernel run holds kernel bytes, so ring 3 may not write it here;
        // `map_user` takes a run whose type says its bytes are outside (D-107).
        if flags.contains(Flags::USER) {
            return Err(MapError::NotYours(virtual_base));
        }
        self.map_run(virtual_base, run, flags)
    }

    /// Map a run of frames ring 3 may reach, at `virtual_base`.
    ///
    /// [`USegment`] makes external-byte provenance explicit; this method adds
    /// `USER` and records ownership regardless of caller flags (D-44, D-107).
    ///
    /// # Errors
    ///
    /// As [`map_owned`](Self::map_owned).
    pub fn map_user(
        &mut self,
        virtual_base: u64,
        run: USegment,
        flags: Flags,
    ) -> Result<(), UserMapError> {
        match self.map_run_geometry(virtual_base, &run, flags.union(Flags::USER)) {
            Ok(owned_pages) => {
                run.commit_mapping();
                self.owned_pages = owned_pages;
                Ok(())
            }
            Err(reason) => Err(UserMapError::Mapping { reason, run }),
        }
    }

    /// Map a run ring 3 may reach, **without** taking it.
    ///
    /// The mapping omits `OWNED`, retains a non-accessing owner, and therefore
    /// cannot recycle frames while either side can reach them. The one-way
    /// [`USegment::share`] conversion prevents owned/shared aliasing
    /// (D-113, D-115).
    ///
    /// # Errors
    ///
    /// As [`map_user`](Self::map_user), and [`MapError::CacheAlias`] if caller
    /// flags would conflict with the allocation's write-back direct-map view.
    pub fn map_shared(
        &mut self,
        virtual_base: u64,
        run: &SharedBytes,
        flags: Flags,
    ) -> Result<(), MapError> {
        ram_cache_policy(virtual_base, flags)?;
        self.map_shared_allocation(
            virtual_base,
            run.address(),
            run.len(),
            run.keepalive(),
            flags,
        )
    }

    /// Map a checked shared-ring allocation into a user address space.
    ///
    /// Ownership and mapping rules are identical to
    /// [`map_shared`](Self::map_shared); the distinct type retains the ring's
    /// payload/publication-word partition.
    ///
    /// # Errors
    ///
    /// As [`map_shared`](Self::map_shared).
    pub fn map_shared_ring<const WORDS: usize>(
        &mut self,
        virtual_base: u64,
        run: &SharedRing<WORDS>,
        flags: Flags,
    ) -> Result<(), MapError> {
        ram_cache_policy(virtual_base, flags)?;
        self.map_shared_allocation(
            virtual_base,
            run.address(),
            run.len(),
            run.keepalive(),
            flags,
        )
    }

    fn map_shared_allocation(
        &mut self,
        virtual_base: u64,
        physical_base: Paddr,
        len: u64,
        keepalive: SharedKeepalive,
        flags: Flags,
    ) -> Result<(), MapError> {
        self.may_reach(virtual_base, len)?;
        self.shared
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        self.map_range(
            virtual_base,
            physical_base,
            len,
            flags.difference(NOT_THE_CALLERS).union(Flags::USER),
            Grain::Pages,
        )?;
        self.shared.push(keepalive);
        Ok(())
    }

    fn map_run(&mut self, virtual_base: u64, run: Segment, flags: Flags) -> Result<(), MapError> {
        let owned_pages =
            self.map_owned_geometry(virtual_base, run.address(), run.len(), run.frames(), flags)?;
        // The run belongs to the mapping now; `unmap` is what gives it back.
        core::mem::forget(run);
        self.owned_pages = owned_pages;
        Ok(())
    }

    fn map_run_geometry(
        &mut self,
        virtual_base: u64,
        run: &USegment,
        flags: Flags,
    ) -> Result<usize, MapError> {
        self.map_owned_geometry(virtual_base, run.address(), run.len(), run.frames(), flags)
    }

    fn map_owned_geometry(
        &mut self,
        virtual_base: u64,
        physical_base: Paddr,
        len: u64,
        frames: usize,
        flags: Flags,
    ) -> Result<usize, MapError> {
        ram_cache_policy(virtual_base, flags)?;
        self.may_reach(virtual_base, len)?;
        let owned_pages = self
            .owned_pages
            .checked_add(frames)
            .ok_or(MapError::OutOfMemory)?;
        // Ownership lives in the leaf every unmap observes, never in caller
        // flags; an owned large page could not be split back (D-45, D-88, D-105).
        self.map_range(
            virtual_base,
            physical_base,
            len,
            flags.difference(NOT_THE_CALLERS).union(Flags::OWNED),
            Grain::Pages,
        )?;
        Ok(owned_pages)
    }

    /// Map one guarded stack, and hand back the evidence that it is one.
    ///
    /// The returned [`MappedStack`] proves zeroed writable pages, no execute
    /// permission, and an absent guard before
    /// [`task::continue_on_stack`](super::task::continue_on_stack) uses it.
    ///
    /// # Errors
    ///
    /// As [`map_owned`](Self::map_owned), plus [`MapError::Conflict`] naming
    /// the first guard page that turned out to be mapped.
    pub fn map_stack(&mut self, slot: &Slot) -> Result<MappedStack, StackMapError> {
        #[cfg(feature = "selftest")]
        if stack_refusal(&STACK_MAPPINGS_BEFORE_REFUSAL) {
            return Err(StackMapError::Mapping(MapError::OutOfMemory));
        }
        let pages = slot.stack_len().div_ceil(PAGE_SIZE) as usize;
        let mut run = mm::allocate_contiguous(pages, 1).ok_or(MapError::OutOfMemory)?;
        if !run.zero() {
            return Err(MapError::Unreachable(run.address()).into());
        }
        let physical_base = run.address();
        self.map_owned(
            slot.stack_base(),
            run,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )?;

        // Guard verification follows mapping, so refusal must return the
        // just-owned frames.
        if let Err(cause) = self.verify_guard(slot) {
            return match self.rollback_stack(slot, 1, 0) {
                Ok(_) => Err(StackMapError::Mapping(cause)),
                Err(cleanup) => Err(StackMapError::Rollback { cause, cleanup }),
            };
        }

        Ok(MappedStack {
            identity: StackIdentity {
                root: self.root(),
                slot: *slot,
                physical_base,
            },
        })
    }

    fn rollback_stack(
        &mut self,
        slot: &Slot,
        mapped_stacks: usize,
        returned_pages: usize,
    ) -> Result<usize, StackRollback> {
        #[cfg(feature = "selftest")]
        if stack_refusal(&STACK_ROLLBACKS_BEFORE_REFUSAL) {
            return Err(StackRollback {
                reason: MapError::Pinned(slot.stack_base()),
                mapped_stacks,
                returned_pages,
                root: self.root(),
                pending: StackCleanup::One(*slot),
            });
        }
        let returned = self
            .unmap(slot.stack_base(), slot.stack_len())
            .map_err(|reason| StackRollback {
                reason,
                mapped_stacks,
                returned_pages,
                root: self.root(),
                pending: StackCleanup::One(*slot),
            })?;
        let pages = returned.pages() as usize;
        drop(returned);
        Ok(pages)
    }

    fn rollback_stack_prefix(
        &mut self,
        layout: &StackLayout,
        mut remaining: usize,
        mapped_stacks: usize,
        mut returned_pages: usize,
    ) -> Result<usize, StackRollback> {
        assert!(
            remaining <= layout.slots(),
            "stack rollback progress exceeded its layout"
        );
        while remaining != 0 {
            let index = remaining - 1;
            let Some(slot) = layout.slot(index) else {
                panic!("stack rollback lost a checked layout slot");
            };
            #[cfg(feature = "selftest")]
            if stack_refusal(&STACK_ROLLBACKS_BEFORE_REFUSAL) {
                return Err(StackRollback {
                    reason: MapError::Pinned(slot.stack_base()),
                    mapped_stacks,
                    returned_pages,
                    root: self.root(),
                    pending: StackCleanup::Prefix {
                        layout: *layout,
                        remaining,
                    },
                });
            }
            let returned = self
                .unmap(slot.stack_base(), slot.stack_len())
                .map_err(|reason| StackRollback {
                    reason,
                    mapped_stacks,
                    returned_pages,
                    root: self.root(),
                    pending: StackCleanup::Prefix {
                        layout: *layout,
                        remaining,
                    },
                })?;
            returned_pages = returned_pages
                .checked_add(returned.pages() as usize)
                .unwrap_or_else(|| panic!("stack rollback page accounting overflowed"));
            drop(returned);
            remaining -= 1;
        }
        Ok(returned_pages)
    }

    /// Map one guarded stack and prevent its extent from changing.
    ///
    /// The returned evidence prevents every overlapping map or unmap while
    /// hardware may retain the stack address.
    ///
    /// # Errors
    ///
    /// As [`map_stack`](Self::map_stack), plus [`MapError::Pinned`] if this
    /// address space already has a hardware-pinned stack and
    /// [`MapError::Inactive`] if the CPU is not walking this space.
    pub fn map_pinned_stack(&mut self, slot: &Slot) -> Result<PinnedStack, StackMapError> {
        let root = self.root();
        if current_mapping_root(root).is_none() {
            return Err(MapError::Inactive(root).into());
        }
        if let Some(existing) = first_permanent_stack() {
            return Err(MapError::Pinned(existing).into());
        }
        let stack = self.map_stack(slot)?;
        if let Err(reason) = retain_permanent_stack(stack.identity) {
            return match self.rollback_stack(slot, 1, 0) {
                Ok(_) => Err(StackMapError::Mapping(reason)),
                Err(cleanup) => Err(StackMapError::Rollback {
                    cause: reason,
                    cleanup,
                }),
            };
        }
        Ok(PinnedStack { stack })
    }

    /// Exercise map admission against one already-pinned stack guard.
    #[cfg(feature = "selftest")]
    pub fn pinned_stack_refuses_map_for_test(&mut self, slot: &Slot) -> Option<bool> {
        let root = self.root();
        let guard = slot.guard_base();
        let stack = slot.stack_base();
        let top = slot.stack_top().checked_sub(1)?;
        let expected_pin = protected_stack_overlap(root, guard, PAGE_SIZE)?;
        let before_guard = self.lookup(guard);
        let before_stack = self.lookup(stack)?;
        let before_top = self.lookup(top)?;
        if before_guard.is_some() || expected_pin != stack {
            return Some(false);
        }

        let range = MappingRange::new(guard, before_stack.0, PAGE_SIZE).ok()?;
        let table_mark = self.tables.len();
        let owned_before = self.owned_pages;
        let result = self.map_range(
            guard,
            before_stack.0,
            PAGE_SIZE,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
            Grain::Pages,
        );
        let refused = matches!(result, Err(MapError::Pinned(address)) if address == stack);
        if result.is_ok() {
            self.undo(&range, PAGE_SIZE, Grain::Pages);
            self.rollback_tables(table_mark);
        }

        Some(
            refused
                && self.lookup(guard) == before_guard
                && self.lookup(stack) == Some(before_stack)
                && self.lookup(top) == Some(before_top)
                && self.tables.len() == table_mark
                && self.owned_pages == owned_before,
        )
    }

    /// Translate through this space's tables, active or not.
    ///
    /// The same walk [`translate`] makes, aimed at a space that may not be the
    /// one the CPU is using — which is the only way to ask anything about a
    /// space before it is installed.
    pub fn lookup(&self, virtual_address: u64) -> Option<(Paddr, Flags)> {
        // Reclaim removes the root owner; no later read may walk its recycled
        // frame (D-123, D-139).
        if self.is_reclaimed() {
            #[cfg(feature = "selftest")]
            RECLAIMED_REFUSALS.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        translate_in(self.root(), virtual_address)
    }

    /// A space for a program: empty below, and the kernel's above.
    ///
    /// The higher half shares the kernel's existing top-level entries by
    /// reference so traps retain code, stacks, and the direct map. The empty
    /// lower half is per-program isolation (D-104).
    ///
    /// Activation reserves the device window's top-level table and freezes new
    /// higher-half top-level entries. Later mappings may therefore change only
    /// tables every user root already shares.
    ///
    /// # Errors
    ///
    /// [`MapError::OutOfMemory`] for the root, and [`MapError::Unreachable`]
    /// if the direct map cannot reach a table. Also `OutOfMemory` before the
    /// kernel's own tables are installed, because there is nothing to share.
    pub fn for_user() -> Result<Self, MapError> {
        if !is_active() {
            return Err(MapError::OutOfMemory);
        }
        let kernel = active_root();
        let space = Self::new()?;

        let mut space = space;
        // Narrowed *before* the sharing, so a space that shares the kernel can
        // never be told to change it (D-105).
        space.reach = Reach::UserHalfOnly;

        for index in ENTRIES / 2..ENTRIES {
            let from = entry_slot(kernel, index)?;
            let into = entry_slot(space.root(), index)?;
            let entry = EntrySlot::read(&from);
            EntrySlot::write(&into, entry);
        }
        Ok(space)
    }

    /// Reserve the top-level table every later device mapping will share.
    fn reserve_shared_kernel_topology(&mut self) -> Result<(), MapError> {
        assert_eq!(
            Level::Pml4.index(DEVICE_WINDOW_START),
            Level::Pml4.index(DEVICE_WINDOW_END - 1),
            "the device window must fit in one shared top-level slot"
        );
        let slot = entry_slot(self.root(), Level::Pml4.index(DEVICE_WINDOW_START))?;
        let current = EntrySlot::read(&slot);
        if current.is_present() {
            if current.is_huge() || current.flags().contains(Flags::USER) {
                return Err(MapError::Conflict(DEVICE_WINDOW_START));
            }
            return Ok(());
        }

        let (_, link) =
            self.take_owned_table(DEVICE_WINDOW_START, Level::Pml4, TablePrivilege::Supervisor)?;
        EntrySlot::write(&slot, link);
        Ok(())
    }

    /// Give this space up to be installed on a context switch.
    ///
    /// A linear [`Binding`] prevents two task slots from sharing reclaim
    /// authority. Retirement produces the evidence [`reclaim`](Self::reclaim)
    /// accepts (D-117).
    ///
    /// Every higher-half top-level entry must match the kernel's address and
    /// software-controlled flags. Hardware may independently set `ACCESSED`;
    /// sampling addresses cannot prove the whole switch safe (D-106).
    ///
    /// # Errors
    ///
    /// [`ActivateError::NotAUserSpace`] for a space that does not share the
    /// kernel's half, [`ActivateError::KernelHalfDiffers`] naming the first
    /// slot that disagrees, and `AlreadyActive` inverted — before the kernel
    /// installs its own tables there is nothing to agree with.
    pub fn bind(&mut self) -> Result<Binding, ActivateError> {
        if !is_active() || self.bound {
            return Err(ActivateError::AlreadyActive);
        }
        // As `lookup`: binding a reclaimed space would read frame zero as its
        // top-level table, and then install it (D-139).
        if self.is_reclaimed() {
            #[cfg(feature = "selftest")]
            RECLAIMED_REFUSALS.fetch_add(1, Ordering::Relaxed);
            return Err(ActivateError::Reclaimed);
        }
        if self.reach != Reach::UserHalfOnly {
            return Err(ActivateError::NotAUserSpace);
        }
        let kernel = active_root();
        for index in ENTRIES / 2..ENTRIES {
            let theirs =
                entry_slot(kernel, index).map_err(|_| ActivateError::KernelHalfDiffers(index))?;
            let ours = entry_slot(self.root(), index)
                .map_err(|_| ActivateError::KernelHalfDiffers(index))?;
            let expected = EntrySlot::read(&theirs);
            let found = EntrySlot::read(&ours);
            if !expected.table_identity_matches(found) {
                return Err(ActivateError::KernelHalfDiffers(index));
            }
        }
        self.bound = true;
        Ok(Binding { root: self.root() })
    }

    /// Bind after introducing only hardware `ACCESSED` drift in one shared root entry.
    #[cfg(feature = "selftest")]
    pub fn bind_with_accessed_drift_for_test(&mut self) -> Option<Result<Binding, ActivateError>> {
        self.bind_with_root_flag_drift_for_test(Flags::ACCESSED)
    }

    /// Bind after introducing cache-policy drift in one shared root entry.
    #[cfg(feature = "selftest")]
    pub fn bind_with_cache_drift_for_test(&mut self) -> Option<Result<Binding, ActivateError>> {
        self.bind_with_root_flag_drift_for_test(Flags::NO_CACHE)
    }

    #[cfg(feature = "selftest")]
    fn bind_with_root_flag_drift_for_test(
        &mut self,
        flag: Flags,
    ) -> Option<Result<Binding, ActivateError>> {
        if self.is_reclaimed() || self.reach != Reach::UserHalfOnly || !is_active() {
            return None;
        }
        let index = Level::Pml4.index(Self::bind as *const () as u64);
        let expected_slot = entry_slot(active_root(), index).ok()?;
        let found_slot = entry_slot(self.root(), index).ok()?;
        let expected = EntrySlot::read(&expected_slot);
        let found = EntrySlot::read(&found_slot);
        if !expected.is_present() || !expected.table_identity_matches(found) {
            return None;
        }
        let bits = if expected.flags().contains(flag) {
            found.bits() & !flag.bits()
        } else {
            found.bits() | flag.bits()
        };
        EntrySlot::write(&found_slot, Entry::from_bits(bits));
        let result = self.bind();
        EntrySlot::write(&found_slot, found);
        Some(result)
    }

    /// Take a mapping away, and hand back what it owned.
    ///
    /// The range must be exactly what one [`map_owned`](Self::map_owned) put
    /// there: every page present, four-kilobyte, owned, and backed by one
    /// ascending run of frames.
    ///
    /// Translation debt is discharged before this returns. A cleared leaf is
    /// invalidated immediately when the current `CR3` reaches that exact slot,
    /// including through a shared higher-half table. Distinct lower-half slots
    /// have neither PCID tags nor global mutable leaves, so loading their root
    /// flushes prior translations. [`Unmapped`] retains only frame ownership
    /// and user provenance.
    ///
    /// # Errors
    ///
    /// [`MapError::Misaligned`] unless base and length are page-aligned,
    /// [`MapError::NotMapped`] for a hole, [`MapError::Unsplittable`] for a
    /// large page, [`MapError::NotOwned`] for a mapping whose frame is not the
    /// allocator's, [`MapError::Discontiguous`] if the frames are not one run,
    /// and [`MapError::Pinned`] if hardware retains any byte of the range.
    pub fn unmap(&mut self, virtual_base: u64, len: u64) -> Result<Unmapped, MapError> {
        if !virtual_base.is_multiple_of(PAGE_SIZE) || len == 0 || !len.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Misaligned(virtual_base));
        }
        // Both ends, because a walk masks bits 48 and above: a noncanonical
        // alias could clear a real leaf but could not be invalidated (D-94).
        let end = virtual_base
            .checked_add(len)
            .ok_or(MapError::NonCanonical(virtual_base))?;
        if !x86_paging::is_canonical(virtual_base) || !x86_paging::is_canonical(end - 1) {
            return Err(MapError::NonCanonical(virtual_base));
        }
        self.may_reach(virtual_base, len)?;
        if let Some(pinned) = protected_stack_overlap(self.root(), virtual_base, len) {
            return Err(MapError::Pinned(pinned));
        }
        let pages = len / PAGE_SIZE;
        let remaining_owned = self
            .owned_pages
            .checked_sub(pages as usize)
            .ok_or(MapError::NotOwned(virtual_base))?;

        // Preflight the whole range before clearing anything, so a refusal
        // leaves the space exactly as it was — the rule D-77 gave `map`.
        let mut first = None;
        let mut was_user = false;
        for page in 0..pages {
            let address = virtual_base
                .checked_add(page * PAGE_SIZE)
                .ok_or(MapError::NonCanonical(virtual_base))?;
            let slot = self.leaf(address)?;
            let entry = EntrySlot::read(&slot);
            if !entry.flags().contains(Flags::OWNED) {
                return Err(MapError::NotOwned(address));
            }
            // One user-reachable page makes the whole run untyped: it is
            // handed back as one thing, and one page is a disclosure (D-123).
            was_user |= entry.flags().contains(Flags::USER);
            match first {
                None => first = Some(entry.address()),
                Some(base) if entry.address() == base + page * PAGE_SIZE => {}
                Some(_) => return Err(MapError::Discontiguous(address)),
            }
        }
        let base = first.ok_or(MapError::NotMapped(virtual_base))?;

        for page in 0..pages {
            let address = virtual_base + page * PAGE_SIZE;
            let slot = self.leaf(address)?;
            clear_mapping_slot(slot, address, Level::Pt);
        }
        self.owned_pages = remaining_owned;

        Ok(Unmapped {
            pages,
            // SAFETY: the preflight proved these `pages` frames from `base`
            // are one ascending run that a `map_owned` moved into this space
            // and that nothing else holds, and the entries naming them have
            // just been cleared.
            frames: Some(unsafe { Segment::from_raw(base, pages as usize) }),
            was_user,
        })
    }

    /// The four-kilobyte leaf entry for `address`, or why there is not one.
    fn leaf(&self, address: u64) -> Result<EntrySlot, MapError> {
        leaf_in(self.root(), address)
    }

    #[cfg(feature = "selftest")]
    /// Run one machine check under this compatible user root, then restore CR3.
    pub fn with_root_for_test<T>(&self, operation: impl FnOnce() -> T) -> Option<T> {
        if self.is_reclaimed() || self.reach != Reach::UserHalfOnly || !is_active() {
            return None;
        }

        let disabled = interrupt::disable();
        let current = cpu::page_table_base();
        let compatible = current == active_root()
            && current != self.root()
            && (ENTRIES / 2..ENTRIES).all(|index| {
                let (Ok(expected), Ok(found)) =
                    (entry_slot(current, index), entry_slot(self.root(), index))
                else {
                    return false;
                };
                let expected = EntrySlot::read(&expected);
                let found = EntrySlot::read(&found);
                expected.table_identity_matches(found)
            });
        if !compatible {
            disabled.restore();
            return None;
        }

        // SAFETY: the complete higher half was just compared with the current
        // root; activation freezes new higher-half top-level entries, and
        // `self` owns its live, aligned lower-half root for the call.
        unsafe { cpu::set_page_table_base(self.root()) };
        let result = operation();
        // SAFETY: `current` was the live root immediately before the temporary
        // switch, remains owned by the kernel, and maps this code and stack.
        unsafe { cpu::set_page_table_base(current) };
        disabled.restore();
        Some(result)
    }

    /// Map an admitted device range, and hand back the claim on it.
    ///
    /// Uncacheable and never executable, at an address in the device window
    /// rather than the physical one.
    ///
    /// # Errors
    ///
    /// As [`map_owned`](Self::map_owned), plus [`MapError::OutOfMemory`] if the device
    /// window is exhausted.
    pub fn map_device(&mut self, admitted: AdmittedDeviceRange) -> Result<DeviceMapping, MapError> {
        self.map_device_range(&admitted)
    }

    fn map_device_range(
        &mut self,
        admitted: &AdmittedDeviceRange,
    ) -> Result<DeviceMapping, MapError> {
        let physical = admitted.physical();
        let len_usize = admitted.len;
        let page_start = admitted.page_start();
        let page_end = admitted.page_end();
        self.may_reach(DEVICE_WINDOW_START, DEVICE_WINDOW_END - DEVICE_WINDOW_START)?;
        let Some(physical_extent) = AddressRange::new(page_start, page_end - page_start) else {
            panic!("an admitted device range always describes complete pages");
        };
        let base = page_start;
        let offset = physical - base;
        let pages = page_end - page_start;
        if self.devices.iter().flatten().any(|extent| {
            physical_extent.base() < extent.physical.end()
                && extent.physical.base() < physical_extent.end()
        }) {
            return Err(MapError::DeviceOverlap(physical));
        }
        let device_slot = match self.devices.iter().position(Option::is_none) {
            Some(slot) => slot,
            None if self.devices.len() >= MAX_DEVICE_MAPPINGS => {
                return Err(MapError::TooManyDevices);
            }
            None => {
                self.devices
                    .try_reserve(1)
                    .map_err(|_| MapError::OutOfMemory)?;
                self.devices.len()
            }
        };

        let Some(device_window) =
            AddressRange::new(DEVICE_WINDOW_START, DEVICE_WINDOW_END - DEVICE_WINDOW_START)
        else {
            return Err(MapError::OutOfMemory);
        };
        let virtual_base = first_fit_by(device_window, pages, PAGE_SIZE, &self.devices, |extent| {
            extent.map(|extent| extent.virtual_range)
        })
        .ok_or(MapError::OutOfMemory)?;
        let virtual_extent =
            AddressRange::new(virtual_base, pages).ok_or(MapError::InvalidDeviceRange(physical))?;

        let expected_flags = Flags::PRESENT
            .union(Flags::WRITABLE)
            .union(Flags::NO_EXECUTE)
            .union(Flags::NO_CACHE);
        self.map_range(
            virtual_base,
            base,
            pages,
            expected_flags.difference(Flags::PRESENT),
            Grain::Pages,
        )?;
        let extent = DeviceExtent {
            physical: physical_extent,
            virtual_range: virtual_extent,
        };
        if device_slot == self.devices.len() {
            self.devices.push(Some(extent));
        } else {
            let Some(slot) = self.devices.get_mut(device_slot) else {
                panic!("device ledger lost its vacant slot");
            };
            *slot = Some(extent);
        }

        Ok(DeviceMapping {
            address: virtual_base + offset,
            physical,
            len: len_usize,
            root: self.root(),
            virtual_base,
            pages,
            device_slot,
            expected_flags,
        })
    }

    /// Map one exact function-bound PCI BAR.
    ///
    /// # Errors
    ///
    /// Returns the unchanged BAR authority with the same reasons as
    /// [`map_device`](Self::map_device).
    pub fn map_pci_bar(
        &mut self,
        admitted: pci::AdmittedBar,
    ) -> Result<pci::BarMapping, BarMapFailure> {
        let pci::AdmittedBar { authority, range } = admitted;
        match self.map_device_range(&range) {
            Ok(mapping) => Ok(pci::BarMapping { authority, mapping }),
            Err(reason) => Err(BarMapFailure {
                reason,
                admitted: pci::AdmittedBar { authority, range },
            }),
        }
    }

    /// Remove the exact mapping behind a transient device region.
    ///
    /// The region is consumed and its access capability dropped before any
    /// leaf is cleared. The whole range is preflighted first, so a refusal
    /// changes neither page tables nor ownership records (D-199).
    ///
    /// # Errors
    ///
    /// [`MapError::NotYours`] if another space minted it,
    /// [`MapError::NotMapped`] if its ownership record or a leaf is gone, and
    /// [`MapError::Discontiguous`] if a leaf no longer names the exact device
    /// extent.
    pub fn unmap_device(&mut self, region: ReleasableDevice) -> Result<(), DeviceUnmapFailure> {
        if let Err(reason) = self.verify_device_unmap(&region) {
            return Err(DeviceUnmapFailure { reason, region });
        }
        let ReleasableDevice {
            mapping,
            registers: _,
        } = region;
        for page in 0..mapping.pages / PAGE_SIZE {
            let address = mapping.virtual_base + page * PAGE_SIZE;
            let Ok(slot) = self.leaf(address) else {
                panic!("a preflighted device leaf disappeared");
            };
            clear_mapping_slot(slot, address, Level::Pt);
        }
        let Some(slot) = self.devices.get_mut(mapping.device_slot) else {
            panic!("a preflighted device ledger slot disappeared");
        };
        *slot = None;
        Ok(())
    }

    /// Remove one PCI BAR mapping while retaining its exact BAR authority.
    ///
    /// A refusal returns the unchanged BAR on any device-unmap preflight
    /// failure.
    pub fn unmap_pci_bar(&mut self, bar: pci::ReleasableBar) -> BarUnmapOutcome {
        let pci::ReleasableBar { authority, region } = bar;
        match self.unmap_device(region) {
            Ok(()) => BarUnmapOutcome::Unmapped(authority),
            Err(failure) => BarUnmapOutcome::Refused(BarUnmapFailure {
                reason: failure.reason,
                bar: pci::ReleasableBar {
                    authority,
                    region: failure.region,
                },
            }),
        }
    }

    fn verify_device_unmap(&self, region: &ReleasableDevice) -> Result<(), MapError> {
        #[cfg(feature = "selftest")]
        if one_shot_mapping_refusal(&DEVICE_UNMAPS_BEFORE_REFUSAL) {
            return Err(MapError::OutOfMemory);
        }
        let mapping = &region.mapping;
        if mapping.root != self.root() {
            return Err(MapError::NotYours(mapping.address));
        }
        let offset = mapping.physical % PAGE_SIZE;
        let physical_base = mapping.physical - offset;
        let expected_extent = DeviceExtent {
            physical: AddressRange::new(physical_base, mapping.pages)
                .ok_or(MapError::InvalidDeviceRange(mapping.physical))?,
            virtual_range: AddressRange::new(mapping.virtual_base, mapping.pages)
                .ok_or(MapError::InvalidDeviceRange(mapping.physical))?,
        };
        if self.devices.get(mapping.device_slot).copied().flatten() != Some(expected_extent) {
            return Err(MapError::NotMapped(mapping.address));
        }

        for page in 0..mapping.pages / PAGE_SIZE {
            let address = mapping.virtual_base + page * PAGE_SIZE;
            let slot = self.leaf(address)?;
            let entry = EntrySlot::read(&slot);
            if entry.flags().contains(Flags::OWNED)
                || entry.address() != physical_base + page * PAGE_SIZE
            {
                return Err(MapError::Discontiguous(address));
            }
        }
        Ok(())
    }

    /// Device mappings this space currently owns.
    #[cfg(feature = "selftest")]
    pub fn device_mappings(&self) -> usize {
        self.devices.iter().flatten().count()
    }

    /// Map every stack of `layout`, leaving every guard gap unmapped.
    ///
    /// Writable and never executable, one physically contiguous run per slot,
    /// zeroed before it is mapped. The guards are guards precisely because
    /// this function never touches them: an overrun lands on an absent
    /// mapping, which faults, rather than on a neighbour's stack, which
    /// would not.
    ///
    /// # Errors
    ///
    /// As [`map_stack`](Self::map_stack). A refusal rolls every earlier stack
    /// back; [`StackMapError::Rollback`] reports any cleanup refusal exactly.
    pub fn map_stacks(&mut self, layout: &StackLayout) -> Result<StackArena, StackMapError> {
        let mut stacks = Vec::new();
        stacks
            .try_reserve_exact(layout.slots())
            .map_err(|_| StackMapError::Mapping(MapError::OutOfMemory))?;
        for mapped_stacks in 0..layout.slots() {
            let Some(slot) = layout.slot(mapped_stacks) else {
                let cause = MapError::OutOfMemory;
                return match self.rollback_stack_prefix(layout, mapped_stacks, mapped_stacks, 0) {
                    Ok(_) => Err(StackMapError::Mapping(cause)),
                    Err(cleanup) => Err(StackMapError::Rollback { cause, cleanup }),
                };
            };
            match self.map_stack(&slot) {
                Ok(stack) => stacks.push(stack.identity),
                Err(error) => {
                    let (cause, committed, returned_pages) = match error {
                        StackMapError::Mapping(cause) => (cause, mapped_stacks, 0),
                        StackMapError::Rollback { cause, cleanup } => {
                            assert!(
                                cleanup.root == self.root()
                                    && cleanup.mapped_stacks == 1
                                    && cleanup.returned_pages == 0
                                    && cleanup.remaining_stacks() == 1,
                                "single-stack rollback lost its cleanup authority"
                            );
                            (cause, mapped_stacks + 1, cleanup.returned_pages)
                        }
                    };
                    return match self.rollback_stack_prefix(
                        layout,
                        committed,
                        committed,
                        returned_pages,
                    ) {
                        Ok(_) => Err(StackMapError::Mapping(cause)),
                        Err(cleanup) => Err(StackMapError::Rollback { cause, cleanup }),
                    };
                }
            }
        }
        Ok(StackArena {
            layout: *layout,
            root: self.root(),
            stacks,
        })
    }

    /// Remove every stack represented by `arena`, returning its frames.
    pub fn unmap_stacks(&mut self, arena: StackArena) -> Result<usize, StackRollback> {
        if arena.root != self.root() {
            return Err(StackRollback {
                reason: MapError::NotYours(arena.layout.base()),
                mapped_stacks: arena.layout.slots(),
                returned_pages: 0,
                root: arena.root,
                pending: StackCleanup::Prefix {
                    layout: arena.layout,
                    remaining: arena.layout.slots(),
                },
            });
        }
        self.rollback_stack_prefix(&arena.layout, arena.layout.slots(), arena.layout.slots(), 0)
    }

    /// Continue the exact stack cleanup represented by `rollback`.
    #[cfg(feature = "selftest")]
    pub fn resume_stack_rollback(
        &mut self,
        rollback: StackRollback,
    ) -> Result<usize, StackRollback> {
        let StackRollback {
            mapped_stacks,
            returned_pages,
            root,
            pending,
            ..
        } = rollback;
        if root != self.root() {
            return Err(StackRollback {
                reason: MapError::NotYours(pending.address()),
                mapped_stacks,
                returned_pages,
                root,
                pending,
            });
        }
        match pending {
            StackCleanup::One(slot) => self.rollback_stack(&slot, mapped_stacks, returned_pages),
            StackCleanup::Prefix { layout, remaining } => {
                self.rollback_stack_prefix(&layout, remaining, mapped_stacks, returned_pages)
            }
        }
    }

    /// Install one entry at `target`, allocating the tables above it.
    fn set(
        &mut self,
        virtual_address: u64,
        physical: Paddr,
        flags: Flags,
        target: Level,
    ) -> Result<(), MapError> {
        let mut table = self.root();

        for level in Level::WALK {
            let slot = entry_slot(table, level.index(virtual_address))?;

            if level == target {
                let entry = Entry::page(target, physical, flags).map_err(MapError::Encoding)?;
                let current = EntrySlot::read(&slot);
                // Only an identical rewrite is allowed: anything else would
                // re-point a mapping some handle is evidence of.
                if current.is_present() && current != entry {
                    return Err(MapError::Conflict(virtual_address));
                }
                EntrySlot::write(&slot, entry);
                return Ok(());
            }

            let current = EntrySlot::read(&slot);
            table = if current.is_present() {
                if current.is_huge() {
                    return Err(MapError::Conflict(virtual_address));
                }
                // An interior entry covers everything below it, so upgrading
                // it to `USER` would hand ring 3 every mapping under it (D-105).
                if flags.contains(Flags::USER) && !current.flags().contains(Flags::USER) {
                    return Err(MapError::Conflict(virtual_address));
                }
                current.address()
            } else {
                let privilege = if flags.contains(Flags::USER) {
                    TablePrivilege::User
                } else {
                    TablePrivilege::Supervisor
                };
                let (frame, link) = self.take_owned_table(virtual_address, level, privilege)?;
                EntrySlot::write(&slot, link);
                frame
            };
        }

        // Unreachable: `target` is always `Pt` or `Pd`, both of which are in
        // `Level::WALK`, so the loop returns before it can end.
        Err(MapError::Conflict(virtual_address))
    }
}

impl AddressSpace {
    /// Nothing is mapped in the pages below `slot`'s stack.
    fn verify_guard(&self, slot: &Slot) -> Result<(), MapError> {
        let mut guard = slot.guard_base();
        while guard < slot.stack_base() {
            if self.lookup(guard).is_some() {
                return Err(MapError::Conflict(guard));
            }
            guard = guard
                .checked_add(PAGE_SIZE)
                .ok_or(MapError::Conflict(guard))?;
        }
        Ok(())
    }
}

/// A zeroed frame to serve as a page table, forgotten by the allocator.
///
/// The forgetting is what makes a table frame reachable through the direct map
/// with no handle naming it, which is what a page table has to be. Whether it
/// ever comes back is the *space's* business: the kernel's own never does, and
/// a user space records each one so [`AddressSpace::reclaim`] can give them
/// back (D-117).
fn take_table() -> Result<Paddr, MapError> {
    let frame = mm::allocate_zeroed().ok_or(MapError::OutOfMemory)?;
    let address = frame.address();
    core::mem::forget(frame);
    Ok(address)
}

impl AddressSpace {
    /// Take and retain one table frame, or change nothing.
    fn take_owned_table(
        &mut self,
        virtual_address: u64,
        level: Level,
        privilege: TablePrivilege,
    ) -> Result<(Paddr, Entry), MapError> {
        if level == Level::Pml4
            && !is_user_address(virtual_address)
            && is_active()
            && self.root() == active_root()
        {
            return Err(MapError::KernelTopologyFrozen(virtual_address));
        }
        self.tables
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let frame = mm::allocate_zeroed().ok_or(MapError::OutOfMemory)?;
        let address = frame.address();
        let link = Entry::table(address, privilege).map_err(MapError::Encoding)?;
        core::mem::forget(frame);
        self.tables.push(OwnedTable {
            frame: address,
            virtual_address,
            level,
            link,
        });
        Ok((address, link))
    }

    /// Give this space's own table frames back to the allocator.
    ///
    /// Mappings must be removed first; reclaim owns only table frames. It
    /// refuses an active or bound space so no current or future `CR3` can walk
    /// freed tables.
    ///
    /// A refusal leaves the space and one-shot release evidence reusable. A
    /// success empties the space and makes every later operation refuse
    /// (D-123).
    pub fn reclaim(&mut self, released: Option<Released>) -> Result<usize, ReclaimFailure> {
        // A bound space needs the task table's linear `Released` evidence, and
        // a repeat could mint an owner for frame zero (D-123, D-139).
        if self.is_reclaimed() {
            #[cfg(feature = "selftest")]
            RECLAIMED_REFUSALS.fetch_add(1, Ordering::Relaxed);
            return Err(ReclaimFailure {
                reason: ReclaimError::Reclaimed,
                released,
            });
        }
        let root = self.root();
        if self.owned_pages != 0 {
            return Err(ReclaimFailure {
                reason: ReclaimError::Mapped(self.owned_pages),
                released,
            });
        }
        if let Some(reason) = self.device_reclaim_error() {
            return Err(ReclaimFailure { reason, released });
        }
        if !self.bound
            && let Some(evidence) = released.as_ref()
        {
            return Err(ReclaimFailure {
                reason: ReclaimError::UnexpectedRelease(evidence.root()),
                released,
            });
        }
        if self.bound {
            match released.as_ref() {
                None => {
                    return Err(ReclaimFailure {
                        reason: ReclaimError::StillBound(root),
                        released,
                    });
                }
                Some(evidence) if evidence.root() != root => {
                    return Err(ReclaimFailure {
                        reason: ReclaimError::WrongRelease {
                            expected: root,
                            found: evidence.root(),
                        },
                        released,
                    });
                }
                Some(_) => {}
            }
        }
        if self.root_is_active(root, cpu::page_table_base()) {
            return Err(ReclaimFailure {
                reason: ReclaimError::Active(root),
                released,
            });
        }

        let returned = self.tables.len() + 1;
        for table in self.tables.drain(..) {
            // SAFETY: each was allocated by `take_table` for this space and
            // forgotten there, so exactly one handle is being rebuilt for it;
            // the space is terminally emptied here. The refusal above
            // establishes that nothing is walking it.
            drop(unsafe { mm::Frame::from_raw(table.frame) });
        }
        // The root, which `new` took before there was anywhere to record it.
        // SAFETY: as above.
        drop(unsafe { mm::Frame::from_raw(root) });
        // Kept as an empty value so later safe operations refuse.
        self.root = None;
        self.shared.clear();

        Ok(returned)
    }

    /// Whether this space's tables have gone back to the allocator.
    ///
    /// A reclaimed space answers nothing and maps nothing. It is kept as a
    /// value rather than consumed so a refused reclaim can give it back.
    pub const fn is_reclaimed(&self) -> bool {
        self.root.is_none()
    }

    /// Prove physical zero is represented as a live root, not empty state.
    #[cfg(feature = "selftest")]
    pub fn zero_root_is_live_for_test(&mut self) -> bool {
        let saved = self.root;
        self.root = Some(0);
        let live = !self.is_reclaimed() && self.root() == 0;
        self.root = saved;
        live
    }

    fn root_is_active(&self, root: Paddr, current_root: Paddr) -> bool {
        current_root == root || (is_active() && active_root() == root)
    }

    fn device_reclaim_error(&self) -> Option<ReclaimError> {
        let mappings = self.devices.iter().flatten().count();
        (mappings != 0).then_some(ReclaimError::Devices(mappings))
    }

    /// Exercise the permanent-root half independently of the current CR3.
    #[cfg(feature = "selftest")]
    pub fn permanent_root_blocks_reclaim_for_test(&self) -> bool {
        self.root
            .is_some_and(|root| self.root_is_active(root, root ^ PAGE_SIZE))
    }

    /// Whether the device ledger independently blocks table reclaim.
    #[cfg(feature = "selftest")]
    pub fn devices_block_reclaim_for_test(&self) -> bool {
        matches!(self.device_reclaim_error(), Some(ReclaimError::Devices(_)))
    }

    /// Whether reservation refuses an existing huge top-level entry unchanged.
    #[cfg(feature = "selftest")]
    pub fn rejects_huge_shared_topology_for_test(&mut self) -> bool {
        let Ok(table) = Entry::table(self.root(), TablePrivilege::Supervisor) else {
            return false;
        };
        self.rejects_existing_shared_topology_for_test(Entry::from_bits(
            table.bits() | Flags::HUGE.bits(),
        ))
    }

    /// Whether reservation refuses an existing user top-level entry unchanged.
    #[cfg(feature = "selftest")]
    pub fn rejects_user_shared_topology_for_test(&mut self) -> bool {
        let Ok(table) = Entry::table(self.root(), TablePrivilege::User) else {
            return false;
        };
        self.rejects_existing_shared_topology_for_test(table)
    }

    #[cfg(feature = "selftest")]
    fn rejects_existing_shared_topology_for_test(&mut self, entry: Entry) -> bool {
        let Ok(slot) = entry_slot(self.root(), Level::Pml4.index(DEVICE_WINDOW_START)) else {
            return false;
        };
        if EntrySlot::read(&slot).is_present() {
            return false;
        }
        let mark = self.tables.len();
        EntrySlot::write(&slot, entry);
        let result = self.reserve_shared_kernel_topology();
        let unchanged = EntrySlot::read(&slot) == entry && self.tables.len() == mark;
        EntrySlot::write(&slot, Entry::ABSENT);
        matches!(result, Err(MapError::Conflict(at)) if at == DEVICE_WINDOW_START) && unchanged
    }

    /// Whether reservation creates a supervisor-only top-level table.
    #[cfg(feature = "selftest")]
    pub fn reserves_supervisor_shared_topology_for_test(&mut self) -> bool {
        let Ok(slot) = entry_slot(self.root(), Level::Pml4.index(DEVICE_WINDOW_START)) else {
            return false;
        };
        if EntrySlot::read(&slot).is_present() {
            return false;
        }
        let mark = self.tables.len();
        if self.reserve_shared_kernel_topology().is_err() {
            return false;
        }
        let entry = EntrySlot::read(&slot);
        entry.is_present()
            && !entry.is_huge()
            && !entry.flags().contains(Flags::USER)
            && mark.checked_add(1) == Some(self.tables.len())
            && self.tables.last().is_some_and(|table| table.link == entry)
    }

    /// Try to allocate one absent higher-half top-level table after activation.
    #[cfg(feature = "selftest")]
    pub fn kernel_topology_is_frozen_for_test(&mut self) -> bool {
        for index in ENTRIES / 2..ENTRIES {
            let Ok(slot) = entry_slot(self.root(), index) else {
                return false;
            };
            if EntrySlot::read(&slot).is_present() {
                continue;
            }
            let virtual_address = 0xffff_0000_0000_0000 | (index as u64) << 39;
            let mark = self.tables.len();
            return match self.take_owned_table(
                virtual_address,
                Level::Pml4,
                TablePrivilege::Supervisor,
            ) {
                Err(MapError::KernelTopologyFrozen(found)) => {
                    found == virtual_address
                        && self.tables.len() == mark
                        && !EntrySlot::read(&slot).is_present()
                }
                Ok((_, link)) => {
                    EntrySlot::write(&slot, link);
                    self.rollback_tables(mark);
                    false
                }
                Err(_) => false,
            };
        }
        true
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        if self
            .root
            .is_some_and(|root| self.bound || self.root_is_active(root, cpu::page_table_base()))
            && !self.shared.is_empty()
        {
            let keepalives = core::mem::take(&mut self.shared);
            core::mem::forget(keepalives);
        }
    }
}

/// Doors refused because the space had been reclaimed.
///
/// Counted inside each guard because a recycled table can coincidentally
/// produce the same absent answer as a refusal (D-139).
#[cfg(feature = "selftest")]
static RECLAIMED_REFUSALS: AtomicUsize = AtomicUsize::new(0);

/// How many doors have refused a reclaimed space.
#[cfg(feature = "selftest")]
pub fn reclaimed_refusals() -> usize {
    RECLAIMED_REFUSALS.load(Ordering::Acquire)
}

#[derive(Debug, PartialEq, Eq)]
struct EntrySlot {
    pointer: NonNull<Entry>,
}

impl EntrySlot {
    fn read(&self) -> Entry {
        // SAFETY: the private callers supply a live table frame; construction
        // proves this aligned pointer names one complete entry inside it.
        unsafe { self.pointer.as_ptr().read_volatile() }
    }

    fn write(&self, entry: Entry) {
        // SAFETY: the private callers supply a live table frame; construction
        // proves this aligned pointer names one complete entry inside it.
        // Callers own the protocol that excludes conflicting writers.
        unsafe { self.pointer.as_ptr().write_volatile(entry) };
    }
}

fn entry_slot(table: Paddr, index: usize) -> Result<EntrySlot, MapError> {
    assert!(index < ENTRIES, "a level's index is always below 512");
    let base = mm::direct_pointer(table, PAGE_SIZE).ok_or(MapError::Unreachable(table))?;
    let pointer = base.cast::<Entry>().wrapping_add(index);
    let pointer = NonNull::new(pointer).ok_or(MapError::Unreachable(table))?;
    Ok(EntrySlot { pointer })
}

fn slot_in(root: Paddr, address: u64, target: Level) -> Result<EntrySlot, MapError> {
    let mut table = root;
    for level in Level::WALK {
        let slot = entry_slot(table, level.index(address))?;
        if level == target {
            return Ok(slot);
        }
        let entry = EntrySlot::read(&slot);
        if !entry.is_present() {
            return Err(MapError::NotMapped(address));
        }
        if entry.is_huge() {
            return Err(MapError::Unsplittable(address));
        }
        table = entry.address();
    }
    Err(MapError::NotMapped(address))
}

fn leaf_in(root: Paddr, address: u64) -> Result<EntrySlot, MapError> {
    let slot = slot_in(root, address, Level::Pt)?;
    EntrySlot::read(&slot)
        .is_present()
        .then_some(slot)
        .ok_or(MapError::NotMapped(address))
}

fn clear_mapping_slot(slot: EntrySlot, address: u64, level: Level) {
    let address = canonical_mapping_address(address);
    let disabled = interrupt::disable();
    let current_slot = slot_in(cpu::page_table_base(), address.get(), level).ok();
    EntrySlot::write(&slot, Entry::ABSENT);
    if current_slot == Some(slot) {
        cpu::invalidate_page(address);
    }
    disabled.restore();
}

fn current_mapping_root(recorded: Paddr) -> Option<Paddr> {
    if !is_active() {
        return None;
    }
    let published = active_root();
    let current = cpu::page_table_base();
    (published == current && recorded == current).then_some(current)
}

fn canonical_mapping_address(address: u64) -> CanonicalAddress {
    let Some(address) = CanonicalAddress::new(address) else {
        panic!("a validated mapping produced a noncanonical invalidation address");
    };
    address
}

/// Activation state published to observers of the installed address space.
static ACTIVE_STATE: AtomicU8 = AtomicU8::new(ActivationState::Cold.bits());

/// The root published before activation becomes ready.
static ACTIVE_ROOT: AtomicU64 = AtomicU64::new(0);

struct Activation {
    state: &'static AtomicU8,
    root: &'static AtomicU64,
    interrupts: Option<interrupt::InterruptState>,
    committed: bool,
}

impl Activation {
    fn begin(state: &'static AtomicU8, root: &'static AtomicU64) -> Result<Self, ActivateError> {
        Self::begin_with(state, root, || {})
    }

    fn begin_with(
        state: &'static AtomicU8,
        root: &'static AtomicU64,
        after_initializing: impl FnOnce(),
    ) -> Result<Self, ActivateError> {
        let Some(initializing) = ActivationState::Cold.begin() else {
            panic!("cold paging activation cannot begin");
        };
        let interrupts = Some(interrupt::disable());
        match state.compare_exchange(
            ActivationState::Cold.bits(),
            initializing.bits(),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                let activation = Self {
                    state,
                    root,
                    interrupts,
                    committed: false,
                };
                after_initializing();
                Ok(activation)
            }
            Err(found) => {
                if let Some(interrupts) = interrupts {
                    interrupts.restore();
                }
                let Some(found) = ActivationState::from_bits(found) else {
                    panic!("paging activation state is corrupt");
                };
                assert!(
                    found != ActivationState::Cold,
                    "a strong activation exchange refused cold state"
                );
                Err(ActivateError::AlreadyActive)
            }
        }
    }

    fn commit(self, root: Paddr) {
        self.commit_with(root, || {});
    }

    fn commit_with(self, root: Paddr, after_root: impl FnOnce()) {
        let mut this = self;
        this.root.store(root, Ordering::Release);
        after_root();
        let Some(ready) = ActivationState::Initializing.complete() else {
            panic!("paging initialization cannot complete");
        };
        this.committed = true;
        this.state.store(ready.bits(), Ordering::Release);
        if let Some(interrupts) = this.interrupts.take() {
            interrupts.restore();
        }
    }
}

impl Drop for Activation {
    fn drop(&mut self) {
        if !self.committed {
            let Some(cold) = ActivationState::Initializing.abort() else {
                panic!("paging initialization cannot abort");
            };
            self.state.store(cold.bits(), Ordering::Release);
        }
        if let Some(interrupts) = self.interrupts.take() {
            interrupts.restore();
        }
    }
}

fn activation_state(state: &AtomicU8) -> ActivationState {
    let bits = state.load(Ordering::Acquire);
    let Some(state) = ActivationState::from_bits(bits) else {
        panic!("paging activation state is corrupt");
    };
    state
}

#[cfg(feature = "selftest")]
const PROBE_INITIAL_ROOT: Paddr = 0x1234_5000;
#[cfg(feature = "selftest")]
static PROBE_STATE: AtomicU8 = AtomicU8::new(ActivationState::Cold.bits());
#[cfg(feature = "selftest")]
static PROBE_ROOT: AtomicU64 = AtomicU64::new(PROBE_INITIAL_ROOT);
#[cfg(feature = "selftest")]
static PROBE_EXPECTED_ROOT: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "selftest")]
static PROBE_ARMED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "selftest")]
static PROBE_OBSERVATIONS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static PROBE_PARTIAL_OBSERVATIONS: AtomicUsize = AtomicUsize::new(0);

/// Results from pausing both paging-activation publication boundaries.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivationPublicationProbe {
    initializing_is_excluded: bool,
    root_precedes_ready: bool,
}

#[cfg(feature = "selftest")]
impl ActivationPublicationProbe {
    /// Whether the initializing state remained unpublished under exclusion.
    pub const fn initializing_is_excluded(self) -> bool {
        self.initializing_is_excluded
    }

    /// Whether the root was published before readiness became observable.
    pub const fn root_precedes_ready(self) -> bool {
        self.root_precedes_ready
    }
}

#[cfg(feature = "selftest")]
#[derive(Clone, Copy)]
enum ProbeBoundary {
    BeforeRoot,
    AfterRoot,
}

/// Pause both activation boundaries with one pending self-interrupt each.
#[cfg(feature = "selftest")]
pub fn activation_publication_for_test() -> ActivationPublicationProbe {
    const FIRST_ROOT: Paddr = 0x2345_6000;
    const SECOND_ROOT: Paddr = 0x3456_7000;

    let initializing_is_excluded = probe_activation(ProbeBoundary::BeforeRoot, FIRST_ROOT);
    let root_precedes_ready = probe_activation(ProbeBoundary::AfterRoot, SECOND_ROOT);
    ActivationPublicationProbe {
        initializing_is_excluded,
        root_precedes_ready,
    }
}

#[cfg(feature = "selftest")]
fn probe_activation(boundary: ProbeBoundary, expected_root: Paddr) -> bool {
    const ATTEMPTS: usize = 1_000_000;

    if !interrupt::are_enabled()
        || PROBE_ARMED.load(Ordering::Acquire)
        || activation_state(&PROBE_STATE) != ActivationState::Cold
    {
        return false;
    }
    PROBE_ROOT.store(PROBE_INITIAL_ROOT, Ordering::Release);
    PROBE_EXPECTED_ROOT.store(expected_root, Ordering::Relaxed);
    let observations_before = PROBE_OBSERVATIONS.load(Ordering::Acquire);
    let partial_before = PROBE_PARTIAL_OBSERVATIONS.load(Ordering::Acquire);

    let mut boundary_holds = true;
    let mut sent = false;
    let activation = match boundary {
        ProbeBoundary::BeforeRoot => Activation::begin_with(&PROBE_STATE, &PROBE_ROOT, || {
            boundary_holds &= activation_state(&PROBE_STATE) == ActivationState::Initializing
                && PROBE_ROOT.load(Ordering::Acquire) == PROBE_INITIAL_ROOT
                && !interrupt::are_enabled();
            PROBE_ARMED.store(true, Ordering::Release);
            sent = super::apic::send_self_interrupt().is_ok();
            boundary_holds &= PROBE_OBSERVATIONS.load(Ordering::Acquire) == observations_before;
        }),
        ProbeBoundary::AfterRoot => Activation::begin(&PROBE_STATE, &PROBE_ROOT),
    };
    let Ok(activation) = activation else {
        PROBE_ARMED.store(false, Ordering::Release);
        return false;
    };
    match boundary {
        ProbeBoundary::BeforeRoot => activation.commit(expected_root),
        ProbeBoundary::AfterRoot => {
            boundary_holds &= activation_state(&PROBE_STATE) == ActivationState::Initializing
                && PROBE_ROOT.load(Ordering::Acquire) == PROBE_INITIAL_ROOT
                && !interrupt::are_enabled();
            activation.commit_with(expected_root, || {
                boundary_holds &= activation_state(&PROBE_STATE) == ActivationState::Initializing
                    && PROBE_ROOT.load(Ordering::Acquire) == expected_root
                    && !interrupt::are_enabled();
                PROBE_ARMED.store(true, Ordering::Release);
                sent = super::apic::send_self_interrupt().is_ok();
                boundary_holds &= PROBE_OBSERVATIONS.load(Ordering::Acquire) == observations_before;
            });
        }
    }

    let arrived = (0..ATTEMPTS).any(|_| {
        core::hint::spin_loop();
        PROBE_OBSERVATIONS.load(Ordering::Acquire) == observations_before + 1
    });
    PROBE_ARMED.store(false, Ordering::Release);
    let holds = sent
        && arrived
        && boundary_holds
        && PROBE_OBSERVATIONS.load(Ordering::Acquire) == observations_before + 1
        && PROBE_PARTIAL_OBSERVATIONS.load(Ordering::Acquire) == partial_before
        && activation_state(&PROBE_STATE) == ActivationState::Ready
        && PROBE_ROOT.load(Ordering::Acquire) == expected_root
        && interrupt::are_enabled();
    PROBE_ROOT.store(PROBE_INITIAL_ROOT, Ordering::Relaxed);
    PROBE_STATE.store(ActivationState::Cold.bits(), Ordering::Release);
    holds
}

/// Record what a self-interrupt observed at an activation pause.
#[cfg(feature = "selftest")]
pub fn observe_activation_publication_for_test() {
    if !PROBE_ARMED.load(Ordering::Acquire) {
        return;
    }
    let expected_root = PROBE_EXPECTED_ROOT.load(Ordering::Relaxed);
    if activation_state(&PROBE_STATE) != ActivationState::Ready
        || PROBE_ROOT.load(Ordering::Acquire) != expected_root
    {
        PROBE_PARTIAL_OBSERVATIONS.fetch_add(1, Ordering::Relaxed);
    }
    PROBE_OBSERVATIONS.fetch_add(1, Ordering::Release);
}

/// The double-fault table and the boot continuation each retain one stack.
const MAX_PERMANENT_STACKS: usize = 2;
static PERMANENT_STACKS: SpinLock<[Option<StackIdentity>; MAX_PERMANENT_STACKS]> =
    SpinLock::new([None; MAX_PERMANENT_STACKS]);

static TASK_STACK_PIN: SpinLock<Option<(Paddr, StackLayout)>> = SpinLock::new(None);

/// Reclaim the stack ledgers on the terminal panic path.
pub(crate) fn force_unlock(panic: &crate::panic::PanicToken) {
    PERMANENT_STACKS.force_unlock(panic);
    TASK_STACK_PIN.force_unlock(panic);
}

fn first_permanent_stack() -> Option<u64> {
    core::iter::Iterator::next(&mut PERMANENT_STACKS.lock_leaf().iter().flatten())
        .map(|stack| stack.slot.stack_base())
}

fn retain_permanent_stack(identity: StackIdentity) -> Result<(), MapError> {
    let mut pinned = PERMANENT_STACKS.lock_leaf();
    let Some(room) = pinned.iter_mut().find(|room| room.is_none()) else {
        let existing = core::iter::Iterator::next(&mut pinned.iter().flatten())
            .map_or(identity.slot.stack_base(), |stack| stack.slot.stack_base());
        return Err(MapError::Pinned(existing));
    };
    *room = Some(identity);
    Ok(())
}

fn permanent_stack_overlap(root: Paddr, base: u64, len: u64) -> Option<u64> {
    let pinned = PERMANENT_STACKS.lock_leaf();
    pinned.iter().flatten().find_map(|identity| {
        (identity.root == root && identity.overlaps(base, len))
            .then_some(identity.slot.stack_base())
    })
}

fn task_stack_overlap(root: Paddr, base: u64, len: u64) -> Option<u64> {
    let pinned = *TASK_STACK_PIN.lock_leaf();
    let (pinned_root, layout) = pinned?;
    if pinned_root != root {
        return None;
    }
    for index in 0..layout.slots() {
        let slot = layout.slot(index)?;
        let protected_len = slot.stack_top().checked_sub(slot.guard_base())?;
        if ranges_overlap(slot.guard_base(), protected_len, base, len) {
            return Some(slot.stack_base());
        }
    }
    None
}

fn protected_stack_overlap(root: Paddr, base: u64, len: u64) -> Option<u64> {
    permanent_stack_overlap(root, base, len).or_else(|| task_stack_overlap(root, base, len))
}

#[cfg(feature = "selftest")]
const STACK_REFUSAL_DISABLED: usize = usize::MAX;
#[cfg(feature = "selftest")]
static STACK_MAPPINGS_BEFORE_REFUSAL: AtomicUsize = AtomicUsize::new(STACK_REFUSAL_DISABLED);
#[cfg(feature = "selftest")]
static STACK_ROLLBACKS_BEFORE_REFUSAL: AtomicUsize = AtomicUsize::new(STACK_REFUSAL_DISABLED);
#[cfg(feature = "selftest")]
static MAPPING_WRITES_BEFORE_REFUSAL: AtomicUsize = AtomicUsize::new(STACK_REFUSAL_DISABLED);
#[cfg(feature = "selftest")]
static MAPPING_UNDOS_BEFORE_REFUSAL: AtomicUsize = AtomicUsize::new(STACK_REFUSAL_DISABLED);
#[cfg(feature = "selftest")]
static DEVICE_UNMAPS_BEFORE_REFUSAL: AtomicUsize = AtomicUsize::new(STACK_REFUSAL_DISABLED);

#[cfg(feature = "selftest")]
fn stack_refusal(counter: &AtomicUsize) -> bool {
    matches!(
        counter.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |remaining| match remaining {
                STACK_REFUSAL_DISABLED => None,
                0 => Some(0),
                count => Some(count - 1),
            }
        ),
        Ok(0)
    )
}

#[cfg(feature = "selftest")]
fn mapping_refusal(counter: &AtomicUsize) -> bool {
    stack_refusal(counter)
}

#[cfg(feature = "selftest")]
fn one_shot_mapping_refusal(counter: &AtomicUsize) -> bool {
    counter
        .compare_exchange(
            0,
            STACK_REFUSAL_DISABLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

/// Refuse a range mapping after `writes` leaf entries.
#[cfg(feature = "selftest")]
pub fn refuse_mapping_after_for_test(writes: usize) {
    MAPPING_WRITES_BEFORE_REFUSAL.store(writes, Ordering::Release);
}

/// Refuse range rollback after `clears` leaf entries.
#[cfg(feature = "selftest")]
pub fn refuse_mapping_undo_after_for_test(clears: usize) {
    MAPPING_UNDOS_BEFORE_REFUSAL.store(clears, Ordering::Release);
}

/// Refuse one device-unmap preflight without consuming its authority.
#[cfg(feature = "selftest")]
pub fn refuse_device_unmap_for_test() {
    DEVICE_UNMAPS_BEFORE_REFUSAL.store(0, Ordering::Release);
}

/// Clear range mapping and rollback refusals.
#[cfg(feature = "selftest")]
pub fn restore_mapping_refusals_for_test() {
    MAPPING_WRITES_BEFORE_REFUSAL.store(STACK_REFUSAL_DISABLED, Ordering::Release);
    MAPPING_UNDOS_BEFORE_REFUSAL.store(STACK_REFUSAL_DISABLED, Ordering::Release);
}

/// Refuse stack construction after `successes` mapped stacks.
#[cfg(feature = "selftest")]
pub fn refuse_stack_mapping_after_for_test(successes: usize) {
    STACK_MAPPINGS_BEFORE_REFUSAL.store(successes, Ordering::Release);
}

/// Refuse stack rollback after `successes` returned stacks.
#[cfg(feature = "selftest")]
pub fn refuse_stack_rollback_after_for_test(successes: usize) {
    STACK_ROLLBACKS_BEFORE_REFUSAL.store(successes, Ordering::Release);
}

/// Clear stack construction and rollback refusals.
#[cfg(feature = "selftest")]
pub fn restore_stack_refusals_for_test() {
    STACK_MAPPINGS_BEFORE_REFUSAL.store(STACK_REFUSAL_DISABLED, Ordering::Release);
    STACK_ROLLBACKS_BEFORE_REFUSAL.store(STACK_REFUSAL_DISABLED, Ordering::Release);
}

/// Switch this CPU to `space`, keeping it for the life of the kernel.
///
/// Safe because it walks the new tables first, for this function's own code
/// and its stack. A table that would not survive the switch is refused with
/// the old one still loaded, so the failure is a `Result` and not a reset.
///
/// # Errors
///
/// [`ActivateError`] naming which address the new tables would have lost, or
/// [`ActivateError::AlreadyActive`] on a second call.
pub fn activate(space: &mut AddressSpace) -> Result<(), ActivateError> {
    /// Headroom below the stack pointer that must also be mapped, so the
    /// first push after the switch has somewhere to go.
    const STACK_HEADROOM: u64 = PAGE_SIZE;

    if space.is_reclaimed() {
        #[cfg(feature = "selftest")]
        RECLAIMED_REFUSALS.fetch_add(1, Ordering::Relaxed);
        return Err(ActivateError::Reclaimed);
    }

    space
        .reserve_shared_kernel_topology()
        .map_err(ActivateError::SharedTopology)?;
    let activation = Activation::begin(&ACTIVE_STATE, &ACTIVE_ROOT)?;
    let root = space.root();
    let current_root = cpu::page_table_base();
    let code = activate as *const () as u64;
    match translate_in(root, code) {
        None => return Err(ActivateError::CodeUnmapped(code)),
        Some((_, flags)) if flags.contains(Flags::NO_EXECUTE) => {
            return Err(ActivateError::CodeNotExecutable(code));
        }
        Some((physical, _)) => {
            if translate_in(current_root, code).map(|current| current.0) != Some(physical) {
                return Err(ActivateError::MappingChanged(code));
            }
        }
    }

    let stack = cpu::stack_pointer();
    for probe in [stack, stack.saturating_sub(STACK_HEADROOM)] {
        match translate_in(root, probe) {
            None => return Err(ActivateError::StackUnmapped(probe)),
            Some((_, flags)) if !flags.contains(Flags::WRITABLE) => {
                return Err(ActivateError::StackReadOnly(probe));
            }
            Some((_, flags))
                if flags.contains(Flags::USER)
                    || !flags.contains(Flags::NO_EXECUTE)
                    || flags.contains(Flags::NO_CACHE)
                    || flags.contains(Flags::WRITE_THROUGH) =>
            {
                return Err(ActivateError::StackUnsafe(probe));
            }
            Some((physical, _)) => {
                if translate_in(current_root, probe).map(|current| current.0) != Some(physical) {
                    return Err(ActivateError::MappingChanged(probe));
                }
            }
        }
    }

    if let Some(gap) = kernel_image_gap(root, current_root) {
        return Err(ActivateError::ImageChanged(gap));
    }

    if let Some(gap) = direct_map_gap(root, current_root) {
        return Err(ActivateError::DirectMapGap(gap));
    }

    // SAFETY: the walks above prove this code executable, its stack writable,
    // and the direct map complete at unchanged addresses. The device window's
    // top-level table now exists and later higher-half topology is frozen.
    // Active roots cannot be reclaimed, so every table outlives this switch.
    unsafe { cpu::set_page_table_base(root) };
    activation.commit(root);

    // `set` refuses to repoint present entries; a previously absent mapping
    // cannot already have a cached translation (D-86).
    Ok(())
}

/// First kernel-image page redirected or assigned the wrong protection class.
fn kernel_image_gap(candidate: Paddr, current: Paddr) -> Option<u64> {
    let sections = crate::boot::sections();
    for (section, writable, executable) in [
        (sections.text, false, true),
        (sections.rodata, false, false),
        (sections.data, true, false),
    ] {
        let Some(end) = section.start.checked_add(section.mapped_len()) else {
            return Some(section.start);
        };
        let mut address = section.start;
        while address < end {
            let Some((current_physical, _)) = translate_in(current, address) else {
                return Some(address);
            };
            let Some((candidate_physical, flags)) = translate_in(candidate, address) else {
                return Some(address);
            };
            if candidate_physical != current_physical
                || flags.is_write_execute()
                || flags.contains(Flags::USER)
                || flags.contains(Flags::OWNED)
                || flags.contains(Flags::NO_CACHE)
                || flags.contains(Flags::WRITE_THROUGH)
                || flags.contains(Flags::WRITABLE) != writable
                || !flags.contains(Flags::NO_EXECUTE) != executable
            {
                return Some(address);
            }
            let Some(next) = address.checked_add(PAGE_SIZE) else {
                return Some(address);
            };
            address = next;
        }
    }
    None
}

/// The first direct-map address `root` gets wrong, if any: every frame access
/// after the switch is raw arithmetic against this mapping, so a gap must be
/// refused while the refusal can still be a `Result`.
#[derive(Clone, Copy)]
struct ImagePhysicalExtent {
    start: Paddr,
    end: Paddr,
    writable: bool,
    executable: bool,
}

fn image_physical_extents(current_root: Paddr) -> Option<[ImagePhysicalExtent; 3]> {
    let sections = crate::boot::sections();
    let mut extents = [ImagePhysicalExtent {
        start: 0,
        end: 0,
        writable: false,
        executable: false,
    }; 3];
    for (slot, (section, writable, executable)) in extents.iter_mut().zip([
        (sections.text, false, true),
        (sections.rodata, false, false),
        (sections.data, true, false),
    ]) {
        let start = translate_in(current_root, section.start)?.0;
        let end = start.checked_add(section.mapped_len())?;
        let mut offset = 0;
        while offset < section.mapped_len() {
            let virtual_address = section.start.checked_add(offset)?;
            if translate_in(current_root, virtual_address)?.0 != start.checked_add(offset)? {
                return None;
            }
            offset = offset.checked_add(PAGE_SIZE)?;
        }
        *slot = ImagePhysicalExtent {
            start,
            end,
            writable,
            executable,
        };
    }
    Some(extents)
}

fn direct_map_gap(root: Paddr, current_root: Paddr) -> Option<u64> {
    let Some(direct) = mm::direct_map() else {
        return Some(0);
    };
    let offset = direct.offset();
    let Some(image) = image_physical_extents(current_root) else {
        return Some(offset);
    };

    // Copy the bounded map before walking a second lock-bearing structure.
    let Some(map) = mm::memory_map_snapshot() else {
        return Some(offset);
    };

    for extent in map.page_runs() {
        let mut physical = extent.base();
        let Some(extent_end) = extent.base().checked_add(extent.bytes()) else {
            return Some(physical);
        };
        while physical < extent_end {
            let Some(virtual_address) = offset.checked_add(physical) else {
                return Some(physical);
            };
            let image_policy = image
                .iter()
                .find(|section| section.start <= physical && physical < section.end);
            let policy_end = image_policy.map_or_else(
                || {
                    image
                        .iter()
                        .map(|section| section.start)
                        .filter(|start| *start > physical)
                        .min()
                        .unwrap_or(extent_end)
                },
                |section| section.end,
            );
            let (writable, executable, write_through) = image_policy.map_or(
                (
                    true,
                    false,
                    extent.cache() == crate::boot::DirectMapCache::Framebuffer,
                ),
                |section| (section.writable, section.executable, false),
            );
            match walk(root, virtual_address) {
                Some((mapped, flags, span))
                    if mapped == physical
                        && !flags.contains(Flags::USER)
                        && !flags.contains(Flags::OWNED)
                        && !flags.contains(Flags::NO_CACHE)
                        && flags.contains(Flags::WRITE_THROUGH) == write_through
                        && flags.contains(Flags::WRITABLE) == writable
                        && !flags.contains(Flags::NO_EXECUTE) == executable =>
                {
                    let Some(mapped_end) =
                        physical.checked_add(span - (virtual_address & (span - 1)))
                    else {
                        return Some(virtual_address);
                    };
                    physical = mapped_end.min(policy_end).min(extent_end);
                }
                _ => return Some(virtual_address),
            }
        }
    }
    None
}

/// Whether the kernel is running on tables it built itself.
pub fn is_active() -> bool {
    activation_state(&ACTIVE_STATE) == ActivationState::Ready
}

/// The root of the space this kernel installed.
///
/// Compare with [`cpu::page_table_base`] to verify the switch. [`is_active`]
/// distinguishes pre-activation because physical zero is a valid root.
pub fn active_root() -> Paddr {
    ACTIVE_ROOT.load(Ordering::Acquire)
}

/// What the table the CPU is walking maps `virtual_address` to.
///
/// Reads `CR3` rather than any remembered root, so it reports what the machine
/// is actually using — which is the whole point of a read-back check.
pub fn translate(virtual_address: u64) -> Option<(Paddr, Flags)> {
    translate_in(cpu::page_table_base(), virtual_address)
}

/// Volatile-read one byte from a live owned mapping for a TLB machine check.
#[cfg(feature = "selftest")]
pub fn observe_owned_byte_for_test(address: u64) -> Option<u8> {
    let address = CanonicalAddress::new(address)?;
    let disabled = interrupt::disable();
    let Some((_, flags)) = translate(address.get()) else {
        disabled.restore();
        return None;
    };
    if !flags.contains(Flags::OWNED) || flags.contains(Flags::USER) {
        disabled.restore();
        return None;
    }
    // SAFETY: the live walk under interrupt exclusion proved this canonical
    // byte is backed by owned supervisor RAM in the current address space.
    let value = unsafe { (address.get() as *const u8).read_volatile() };
    disabled.restore();
    Some(value)
}

fn translate_in(root: Paddr, virtual_address: u64) -> Option<(Paddr, Flags)> {
    walk(root, virtual_address).map(|(physical, flags, _)| (physical, flags))
}

/// As [`translate_in`], but also naming the leaf's span, so a sweep can step
/// by whole entries.
fn walk(root: Paddr, virtual_address: u64) -> Option<(Paddr, Flags, u64)> {
    if !x86_paging::is_canonical(virtual_address) {
        return None;
    }

    // The host-tested fold stays in `x86-paging`; safe construction cannot
    // build a hierarchy whose parent is narrower than its child (D-105).
    let mut walked = Flags::EVERYTHING_ALLOWED;

    let mut table = root;
    for level in Level::WALK {
        let slot = entry_slot(table, level.index(virtual_address)).ok()?;
        let entry = EntrySlot::read(&slot);
        if !entry.is_present() {
            return None;
        }

        // The huge bit means something else entirely in a leaf table, so the
        // level is checked first.
        walked = walked.through(entry.flags());

        if level == Level::Pt || entry.is_huge() {
            let offset = virtual_address & (level.span() - 1);
            let effective = walked.effective_for(entry.flags());
            return Some((entry.address() + offset, effective, level.span()));
        }
        table = entry.address();
    }
    None
}

/// First address of the window device registers are mapped into.
///
/// Well clear of the direct map below it and the kernel image above it, so a
/// device address is never mistaken for either.
const DEVICE_WINDOW_START: u64 = 0xffff_ff00_0000_0000;

/// One page past the last address device registers may occupy.
const DEVICE_WINDOW_END: u64 = 0xffff_ff01_0000_0000;

/// Device ranges one address space can own before activation.
const MAX_DEVICE_MAPPINGS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceExtent {
    physical: AddressRange,
    virtual_range: AddressRange,
}

/// The stacks [`AddressSpace::map_stacks`] committed, ready to run threads on.
///
/// Linear evidence lets [`task::init`](super::task::init) fabricate contexts
/// only for the ranges one mapping operation established.
#[derive(Debug)]
pub struct StackArena {
    layout: StackLayout,
    root: Paddr,
    stacks: Vec<StackIdentity>,
}

impl StackArena {
    /// The layout the stacks were mapped to.
    pub(crate) const fn layout(&self) -> &StackLayout {
        &self.layout
    }

    /// Retain every stack mapping permanently and return its task-table owner.
    pub(crate) fn pin(self) -> TaskStackPin {
        let mut pinned = TASK_STACK_PIN.lock_leaf();
        assert!(pinned.is_none(), "task stack arena pinned twice");
        *pinned = Some((self.root, self.layout));
        TaskStackPin {
            stacks: self.stacks,
        }
    }

    /// First guard or stack page that disagrees with this arena's proof.
    pub(crate) fn invalid_page(&self) -> Option<u64> {
        let Some(current) = current_mapping_root(self.root) else {
            return Some(self.layout.base());
        };
        self.mapping_mismatch_in(current)
    }

    fn mapping_mismatch_in(&self, root: Paddr) -> Option<u64> {
        if self.stacks.len() != self.layout.slots() {
            return Some(self.layout.base());
        }
        for (index, identity) in self.stacks.iter().enumerate() {
            let Some(slot) = self.layout.slot(index) else {
                return Some(self.layout.base());
            };
            if identity.root != self.root || identity.slot != slot {
                return Some(slot.guard_base());
            }
            if let Some(page) = identity.invalid_page_in(root) {
                return Some(page);
            }
        }
        None
    }
}

/// The task table's access to every permanently retained arena stack.
#[derive(Debug)]
pub(crate) struct TaskStackPin {
    stacks: Vec<StackIdentity>,
}

/// A task-stack top borrowed from a pinned arena identity.
#[derive(Debug)]
pub(crate) struct KernelStackTop<'pin>(&'pin StackIdentity);

impl KernelStackTop<'_> {
    /// The address hardware may retain in `RSP0`.
    pub(crate) const fn address(self) -> u64 {
        self.0.slot.stack_top()
    }
}

impl TaskStackPin {
    /// The pinned stack at `index`, as retained-hardware authority.
    pub(crate) fn stack_top(&self, index: usize) -> Option<KernelStackTop<'_>> {
        let identity = self.stacks.get(index)?;
        if !is_active() {
            return None;
        }
        let current = cpu::page_table_base();
        identity
            .invalid_page_in(current)
            .is_none()
            .then_some(KernelStackTop(identity))
    }

    /// Replace a noncurrent stack's lower leaf during a hardware-use check.
    #[cfg(feature = "selftest")]
    pub(crate) fn rejects_lower_leaf_substitution_for_test(&self, index: usize) -> Option<bool> {
        let identity = self.stacks.get(index)?;
        identity.rejects_page_substitution_for_test(0, || self.stack_top(index).is_none())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StackIdentity {
    root: Paddr,
    slot: Slot,
    physical_base: Paddr,
}

impl StackIdentity {
    fn overlaps(&self, base: u64, len: u64) -> bool {
        let Some(protected_len) = self.slot.stack_top().checked_sub(self.slot.guard_base()) else {
            return true;
        };
        ranges_overlap(self.slot.guard_base(), protected_len, base, len)
    }

    fn invalid_page_in(&self, root: Paddr) -> Option<u64> {
        let mut guard = self.slot.guard_base();
        while guard < self.slot.stack_base() {
            if translate_in(root, guard).is_some() {
                return Some(guard);
            }
            let Some(next) = guard.checked_add(PAGE_SIZE) else {
                return Some(guard);
            };
            guard = next;
        }

        let expected_flags = Flags::PRESENT
            .union(Flags::WRITABLE)
            .union(Flags::NO_EXECUTE)
            .union(Flags::OWNED);
        let mut stack = self.slot.stack_base();
        while stack < self.slot.stack_top() {
            let Some(offset) = stack.checked_sub(self.slot.stack_base()) else {
                return Some(stack);
            };
            let Some(physical) = self.physical_base.checked_add(offset) else {
                return Some(stack);
            };
            match walk(root, stack) {
                Some((found, flags, span))
                    if page_identity_matches(found, flags, span, physical, expected_flags) => {}
                _ => return Some(stack),
            }
            let Some(next) = stack.checked_add(PAGE_SIZE) else {
                return Some(stack);
            };
            stack = next;
        }
        None
    }

    #[cfg(feature = "selftest")]
    fn rejects_page_substitution_for_test(
        &self,
        page: usize,
        rejected: impl FnOnce() -> bool,
    ) -> Option<bool> {
        let pages = usize::try_from(self.slot.stack_len() / PAGE_SIZE).ok()?;
        if pages < 2 || page >= pages {
            return None;
        }
        let offset = u64::try_from(page).ok()?.checked_mul(PAGE_SIZE)?;
        let target_address = self.slot.stack_base().checked_add(offset)?;
        let target_end = target_address.checked_add(PAGE_SIZE)?;
        let other = if page == 0 { pages - 1 } else { 0 };
        let replacement_offset = u64::try_from(other).ok()?.checked_mul(PAGE_SIZE)?;
        let replacement_address = self.slot.stack_base().checked_add(replacement_offset)?;
        let pointer = cpu::stack_pointer();
        if (target_address..target_end).contains(&pointer) {
            return None;
        }

        let disabled = interrupt::disable();
        let result = (|| {
            let root = current_mapping_root(self.root)?;
            if self.invalid_page_in(root).is_some() {
                return Some(false);
            }
            let target_before = walk(root, target_address)?;
            let replacement_before = walk(root, replacement_address)?;
            let expected_target = self.physical_base.checked_add(offset)?;
            let expected_replacement = self.physical_base.checked_add(replacement_offset)?;
            if target_before.0 != expected_target
                || replacement_before.0 != expected_replacement
                || target_before.2 != PAGE_SIZE
                || replacement_before.2 != PAGE_SIZE
            {
                return Some(false);
            }

            let slot = leaf_in(root, target_address).ok()?;
            let original = EntrySlot::read(&slot);
            let changed = Entry::page(Level::Pt, expected_replacement, original.flags()).ok()?;
            EntrySlot::write(&slot, changed);
            cpu::invalidate_page(canonical_mapping_address(target_address));

            let changed_walk = walk(root, target_address);
            let rejected = rejected();

            EntrySlot::write(&slot, original);
            cpu::invalidate_page(canonical_mapping_address(target_address));

            Some(
                changed_walk == Some((expected_replacement, target_before.1, target_before.2))
                    && rejected
                    && walk(root, target_address) == Some(target_before)
                    && walk(root, replacement_address) == Some(replacement_before)
                    && self.invalid_page_in(root).is_none(),
            )
        })();
        disabled.restore();
        result
    }
}

/// Replace one permanent stack's lower leaf and require its identity to refuse it.
#[cfg(feature = "selftest")]
pub fn permanent_stack_rejects_lower_leaf_substitution_for_test(base: u64) -> Option<bool> {
    let identity = PERMANENT_STACKS
        .lock_leaf()
        .iter()
        .flatten()
        .find(|identity| identity.slot.stack_base() == base)
        .copied()?;
    identity.rejects_page_substitution_for_test(0, || {
        current_mapping_root(identity.root)
            .is_some_and(|root| identity.invalid_page_in(root) == Some(base))
    })
}

/// A permanent stack's dead span, painted so a later scan finds the deepest word written.
///
/// The paint is non-zero because a stack probe stores zero, and non-canonical so
/// no saved pointer can equal it.
#[cfg(feature = "selftest")]
#[derive(Debug)]
pub struct PaintedStack {
    identity: StackIdentity,
    span: core::ops::Range<u64>,
}

#[cfg(feature = "selftest")]
impl PaintedStack {
    const PAINT: u64 = 0x5741_5A44_5354_4B21;
    const WORD: u64 = size_of::<u64>() as u64;
    /// Room below the painter for its own calls, which a debug build makes for every word.
    const CLEARANCE: u64 = 1024;
    /// The scan runs no deeper than the paint, so half that room keeps the scan's calls clear.
    const SCAN_CLEARANCE: u64 = Self::CLEARANCE / 2;

    /// The lowest painted word anything has written since the paint.
    ///
    /// `None` when every word still holds the paint, or unless this frame runs on
    /// the painted stack in the same address space, at least half a kilobyte
    /// above the span.
    pub fn deepest_touch(&self) -> Option<u64> {
        let pointer = cpu::stack_pointer();
        let reach = self
            .identity
            .slot
            .span_below(pointer, Self::SCAN_CLEARANCE)?;
        if reach.end < self.span.end || current_mapping_root(self.identity.root).is_none() {
            return None;
        }
        let mut address = self.span.start;
        while address < self.span.end {
            // SAFETY: the word is in the painted span of the pinned stack this frame runs on,
            // at least `SCAN_CLEARANCE` below it, so it is mapped and holds no live value.
            if unsafe { (address as *const u64).read_volatile() } != Self::PAINT {
                return Some(address);
            }
            address += Self::WORD;
        }
        None
    }
}

/// Paint the dead span of the permanent stack this code is running on.
///
/// Finds the live-root permanent stack that holds the stack pointer, re-walks
/// its guard and pages, and fills every word from its base to a clearance below
/// this frame.
#[cfg(feature = "selftest")]
pub fn paint_current_permanent_stack_for_test() -> Option<PaintedStack> {
    let pointer = cpu::stack_pointer();
    let identity = PERMANENT_STACKS
        .lock_leaf()
        .iter()
        .flatten()
        .find(|identity| identity.slot.usage_at(pointer).is_some())
        .copied()?;
    let root = current_mapping_root(identity.root)?;
    if identity.invalid_page_in(root).is_some() {
        return None;
    }
    let span = identity.slot.span_below(pointer, PaintedStack::CLEARANCE)?;
    let mut address = span.start;
    while address < span.end {
        // SAFETY: the walk above proved this pinned stack mapped, owned and writable, and
        // `span_below` keeps the word between its base and `CLEARANCE` below this frame.
        unsafe { (address as *mut u64).write_volatile(PaintedStack::PAINT) };
        address += PaintedStack::WORD;
    }
    Some(PaintedStack { identity, span })
}

/// A stack this address space mapped, with its guard verified absent.
///
/// Not `Copy` or `Clone`, for the reason [`DeviceMapping`] is not: it is the
/// evidence that exactly one caller owns this region. The root ties it to the
/// space that made it, because a stack address means nothing without one.
#[derive(Debug)]
pub struct MappedStack {
    identity: StackIdentity,
}

impl MappedStack {
    /// The address the stack grows down from, sixteen-byte aligned.
    pub const fn top(&self) -> u64 {
        self.identity.slot.stack_top()
    }

    /// Whether the CPU is walking the space with this exact guarded stack.
    ///
    /// A stack mapped into a space that was never installed is not a stack this
    /// CPU can stand on, and the addresses collide across spaces, so the root
    /// is the only thing that can tell them apart.
    ///
    /// Every guard page must remain absent and every stack page must still be
    /// the original owned, supervisor, write-back, writable and non-executable
    /// mapping. The complete re-walk rejects partial and substituted stacks.
    pub fn is_installed(&self) -> bool {
        current_mapping_root(self.identity.root)
            .is_some_and(|current| self.identity.invalid_page_in(current).is_none())
    }

    /// Check this token while a compatible user root is actually loaded.
    #[cfg(feature = "selftest")]
    pub fn is_installed_under_for_test(&self, space: &AddressSpace) -> Option<bool> {
        space.with_root_for_test(|| self.is_installed())
    }

    /// Replace one live leaf for the duration of an installation check.
    #[cfg(feature = "selftest")]
    pub fn rejects_substituted_page_for_test(&self, page: usize) -> Option<bool> {
        self.identity
            .rejects_page_substitution_for_test(page, || !self.is_installed())
    }

    pub(crate) fn retain(self) -> Result<PinnedStack, MapError> {
        if !self.is_installed() {
            return Err(MapError::NotMapped(self.identity.slot.stack_base()));
        }
        retain_permanent_stack(self.identity)?;
        Ok(PinnedStack { stack: self })
    }
}

/// A guarded stack whose address space will not allow its extent to change.
///
/// Hardware retains stack addresses without retaining Rust values. This token
/// proves the address space records the stack and guard as pinned, so a safe
/// caller cannot remove, replace or fill either after hardware retains it.
#[derive(Debug)]
pub struct PinnedStack {
    stack: MappedStack,
}

impl PinnedStack {
    /// The address the stack grows down from, sixteen-byte aligned.
    pub const fn top(&self) -> u64 {
        self.stack.top()
    }

    pub(crate) fn is_installed(&self) -> bool {
        self.stack.is_installed()
    }
}

/// Frames detached from an address space, with their access provenance.
///
/// [`AddressSpace::unmap`] pays all translation debt before constructing this
/// value. The token keeps one owner for the frames and prevents user bytes from
/// becoming a kernel allocation without clearing. Forgetting it leaks frames
/// but cannot preserve a stale mapping.
#[derive(Debug)]
#[must_use = "the detached frames remain held until this token is consumed or dropped"]
pub struct Unmapped {
    pages: u64,
    frames: Option<Segment>,
    /// Whether these frames still contain bytes ring 3 could have written.
    was_user: bool,
}

impl Unmapped {
    /// Hand back kernel-only frames.
    ///
    /// Returns `None` when ring 3 could reach the range; those frames must pass
    /// through [`USegment::reclaim`](crate::mm::USegment::reclaim) to be cleared.
    #[cfg(feature = "selftest")]
    pub fn into_kernel(mut self) -> Option<Segment> {
        if self.was_user {
            return None;
        }
        self.frames.take()
    }

    /// Hand back what ring 3 could reach as untyped user frames.
    ///
    /// The only conversion to kernel-owned frames is [`USegment::reclaim`],
    /// which clears the program's bytes.
    #[cfg(feature = "selftest")]
    pub fn into_user(mut self) -> Option<USegment> {
        self.was_user
            .then(|| self.frames.take())
            .flatten()
            .map(Into::into)
    }

    /// Whether ring 3 could reach the range this took away.
    #[cfg(feature = "selftest")]
    pub const fn was_user(&self) -> bool {
        self.was_user
    }

    /// How many pages this took away.
    pub const fn pages(&self) -> u64 {
        self.pages
    }
}

impl Drop for Unmapped {
    /// User bytes are cleared before allocator reuse; a failed clear leaks the
    /// frames instead of disclosing them (D-107, D-123).
    fn drop(&mut self) {
        if self.was_user
            && let Some(frames) = self.frames.as_mut()
        {
            if frames.zero() {
                #[cfg(feature = "selftest")]
                CLEARED_ON_DROP.fetch_add(frames.frames(), Ordering::Relaxed);
            } else {
                // A failed clear leaks the frames rather than returning a
                // program's bytes to the allocator.
                core::mem::forget(self.frames.take());
            }
        }
    }
}

/// User frames cleared on the way back to the allocator.
///
/// A tally makes drop observable without assuming which freed frame the
/// allocator returns next (D-123).
#[cfg(feature = "selftest")]
static CLEARED_ON_DROP: AtomicUsize = AtomicUsize::new(0);

/// Frames cleared on their way back from a user mapping.
#[cfg(feature = "selftest")]
pub fn cleared_on_drop() -> usize {
    CLEARED_ON_DROP.load(Ordering::Acquire)
}

/// How large the pages of a mapping may be.
///
/// Four-kilobyte granularity keeps owned mappings splittable by `unmap`; the
/// direct map instead takes the largest aligned pages available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grain {
    /// The largest page each aligned stretch allows.
    Largest,
    /// Four kilobytes throughout.
    Pages,
}

/// Whether every leaf named by one mapping existed before it began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MappingState {
    /// Every target was absent, so rollback owns every leaf it wrote.
    Absent,
    /// Every target already contained the exact requested mapping.
    Identical,
}

/// A space that may be installed on a context switch.
///
/// The evidence that [`AddressSpace::bind`] checked the higher half, carried
/// as a value so a caller cannot install a space that was never compared.
/// Neither `Copy` nor `Clone`: two slots naming one space would let a thread
/// exit and take another's tables with it.
#[derive(Debug)]
pub struct Binding {
    root: Paddr,
}

impl Binding {
    /// The table to load into `CR3`.
    pub const fn root(&self) -> Paddr {
        self.root
    }
}

/// Evidence that a physical range is one the boot handoff described.
///
/// The only way to reach [`AddressSpace::map_boot`], and constructible only
/// from a range the bootloader's memory map covers. Not `Copy`: it is a
/// statement about one range, made once, and a copy would let a caller reuse
/// the evidence for a range nobody checked.
#[derive(Debug)]
pub struct BootRegion {
    base: Paddr,
    len: u64,
    cache: crate::boot::DirectMapCache,
}

impl BootRegion {
    /// Describe `len` bytes at `base`, or `None` unless the memory map covers
    /// all of it *with memory* of one direct-map cache type.
    ///
    /// Device holes are excluded because write-back direct-map aliases of MMIO
    /// are unsafe, and a range mixing framebuffer and ordinary memory has no
    /// single type a mapping could share. Returns `None` before the map exists
    /// (D-45, D-105, D-349).
    pub fn describe(base: Paddr, len: u64) -> Option<Self> {
        let end = base.checked_add(len)?;
        if len == 0 || !base.is_multiple_of(PAGE_SIZE) {
            return None;
        }
        let cache = crate::mm::map_direct_cache(base, end)?;
        Some(Self { base, len, cache })
    }

    /// The cache-type bits every mapping of this range must carry.
    const fn cache_type(&self) -> Flags {
        match self.cache {
            crate::boot::DirectMapCache::Normal => Flags::NONE,
            crate::boot::DirectMapCache::Framebuffer => Flags::WRITE_THROUGH,
        }
    }
}

/// A claim on a device's registers, made while the tables were being built.
///
/// Linear ownership prevents duplicate claims. The root distinguishes equal
/// window addresses minted by different spaces before
/// [`claim_releasable`](Self::claim_releasable).
#[derive(Debug)]
pub struct DeviceMapping {
    address: u64,
    physical: Paddr,
    len: usize,
    root: Paddr,
    virtual_base: u64,
    pages: u64,
    device_slot: usize,
    expected_flags: Flags,
}

#[cfg(feature = "selftest")]
enum DeviceSubstitution {
    Physical,
    CachePolicy,
}

impl DeviceMapping {
    /// Where the registers are readable, once the space is active.
    pub const fn address(&self) -> u64 {
        self.address
    }

    /// Take a region which can later return this exact device mapping.
    ///
    /// The mapping is not promised permanent: the unique region retains this
    /// token, and
    /// [`AddressSpace::unmap_device`] consumes both before clearing it.
    pub fn claim_releasable(self) -> Result<ReleasableDevice, Self> {
        if !self.is_live() {
            return Err(self);
        }
        // SAFETY: `is_live` verified the complete mapping in the active root.
        // Activation reserved its top-level device table before any user root
        // and freezes later top-level additions, so every runnable root shares
        // it. `self` moves beside the region and is consumed before unmapping;
        // `map_device` established four-byte alignment.
        let registers = unsafe { MmioRegion::new(self.address as *mut u8, self.len) };
        Ok(ReleasableDevice {
            mapping: self,
            registers,
        })
    }

    /// Whether the complete page-rounded extent is unchanged in the live root.
    fn is_live(&self) -> bool {
        let Some(current) = current_mapping_root(self.root) else {
            return false;
        };
        let mut offset = 0u64;
        while offset < self.pages {
            let Some(virtual_address) = self.virtual_base.checked_add(offset) else {
                return false;
            };
            let physical_base = self.physical - self.physical % PAGE_SIZE;
            let Some(expected) = physical_base.checked_add(offset) else {
                return false;
            };
            let Some((mapped, flags, span)) = walk(current, virtual_address) else {
                return false;
            };
            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {
                return false;
            }
            let Some(next) = offset.checked_add(PAGE_SIZE) else {
                return false;
            };
            offset = next;
        }
        true
    }

    /// Claim while a compatible user root is actually loaded by the CPU.
    #[cfg(feature = "selftest")]
    pub fn claim_releasable_under_for_test(
        self,
        space: &AddressSpace,
    ) -> (bool, Result<ReleasableDevice, Self>) {
        let mut mapping = Some(self);
        let outcome = space.with_root_for_test(|| {
            let Some(mapping) = mapping.take() else {
                panic!("a device mapping test consumed its token twice");
            };
            mapping.claim_releasable()
        });
        match outcome {
            Some(outcome) => (true, outcome),
            None => {
                let Some(mapping) = mapping else {
                    panic!("a device mapping test lost its unconsumed token");
                };
                (false, Err(mapping))
            }
        }
    }

    /// Substitute one physical leaf while exercising the real claim door.
    #[cfg(feature = "selftest")]
    pub fn rejects_physical_substitution_for_test(self, page: usize) -> (Self, bool) {
        self.rejects_substitution_for_test(page, DeviceSubstitution::Physical)
    }

    /// Substitute one leaf's cache policy while exercising the real claim door.
    #[cfg(feature = "selftest")]
    pub fn rejects_cache_substitution_for_test(self, page: usize) -> (Self, bool) {
        self.rejects_substitution_for_test(page, DeviceSubstitution::CachePolicy)
    }

    #[cfg(feature = "selftest")]
    fn rejects_substitution_for_test(
        self,
        page: usize,
        substitution: DeviceSubstitution,
    ) -> (Self, bool) {
        let Ok(page_count) = usize::try_from(self.pages / PAGE_SIZE) else {
            return (self, false);
        };
        if page_count < 2 || page >= page_count {
            return (self, false);
        }
        let Ok(page) = u64::try_from(page) else {
            return (self, false);
        };
        let Some(offset) = page.checked_mul(PAGE_SIZE) else {
            return (self, false);
        };
        let Some(address) = self.virtual_base.checked_add(offset) else {
            return (self, false);
        };
        let physical_base = self.physical - self.physical % PAGE_SIZE;
        let Some(expected) = physical_base.checked_add(offset) else {
            return (self, false);
        };
        let other = if page == 0 { page_count - 1 } else { 0 };
        let Ok(other) = u64::try_from(other) else {
            return (self, false);
        };
        let Some(replacement_offset) = other.checked_mul(PAGE_SIZE) else {
            return (self, false);
        };
        let Some(replacement) = physical_base.checked_add(replacement_offset) else {
            return (self, false);
        };

        let disabled = interrupt::disable();
        let Some(root) = current_mapping_root(self.root) else {
            disabled.restore();
            return (self, false);
        };
        let Ok(slot) = leaf_in(root, address) else {
            disabled.restore();
            return (self, false);
        };
        let original = EntrySlot::read(&slot);
        if !page_identity_matches(
            original.address(),
            original.flags(),
            PAGE_SIZE,
            expected,
            self.expected_flags,
        ) {
            disabled.restore();
            return (self, false);
        }
        let changed = match substitution {
            DeviceSubstitution::Physical => Entry::page(Level::Pt, replacement, original.flags()),
            DeviceSubstitution::CachePolicy => {
                if !original.flags().contains(Flags::NO_CACHE)
                    || original.flags().contains(Flags::WRITE_THROUGH)
                {
                    disabled.restore();
                    return (self, false);
                }
                Entry::page(
                    Level::Pt,
                    expected,
                    original
                        .flags()
                        .difference(Flags::NO_CACHE)
                        .union(Flags::WRITE_THROUGH),
                )
            }
        };
        let Ok(changed) = changed else {
            disabled.restore();
            return (self, false);
        };

        EntrySlot::write(&slot, changed);
        cpu::invalidate_page(canonical_mapping_address(address));
        let (mapping, rejected) = match self.claim_releasable() {
            Err(mapping) => (mapping, true),
            Ok(ReleasableDevice {
                mapping,
                registers: _,
            }) => (mapping, false),
        };
        EntrySlot::write(&slot, original);
        cpu::invalidate_page(canonical_mapping_address(address));
        let restored = mapping.is_live();
        disabled.restore();
        (mapping, rejected && restored)
    }
}

/// A unique live MMIO region paired with the token that can remove it.
#[derive(Debug)]
pub struct ReleasableDevice {
    mapping: DeviceMapping,
    registers: MmioRegion,
}

/// A refused device unmap with its unchanged retry authority.
#[derive(Debug)]
#[must_use = "a refused device unmap returns its cleanup authority"]
pub struct DeviceUnmapFailure {
    /// Why no leaf or ownership record changed.
    pub reason: MapError,
    /// The unique register access and removal token.
    pub region: ReleasableDevice,
}

impl fmt::Display for DeviceUnmapFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.reason, f)
    }
}

impl core::error::Error for DeviceUnmapFailure {}

impl ReleasableDevice {
    pub(crate) const fn address(&self) -> u64 {
        self.mapping.address
    }

    /// Read one register word.
    pub fn read_u32(&self, offset: usize) -> Result<u32, crate::mmio::MmioError> {
        MmioRegion::read_u32(&self.registers, offset)
    }

    /// Write one register word.
    pub fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), crate::mmio::MmioError> {
        MmioRegion::write_u32(&mut self.registers, offset, value)
    }

    /// Keep this mapping permanently and return its register access.
    ///
    /// Consuming the removal token makes the selected controller's mapping
    /// intentionally permanent; rejected candidates return theirs.
    #[must_use]
    pub fn retain(self) -> MmioRegion {
        let Self {
            mapping: _,
            registers,
        } = self;
        registers
    }
}
