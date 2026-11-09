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
use stack_layout::{Slot, StackLayout};
use x86_paging::activation::State as ActivationState;
use x86_paging::device::{DeviceRange as CheckedDeviceRange, DeviceRangeError};
use x86_paging::kpti::{
    EnclavePage, EnclavePermission, FlushPremise, FlushPremiseError, ObservedPage,
    PairCleanupState, PairConstructionLedger, PairFrameAuthority, PcidState,
    RootPair as CheckedRootPair, RootPairError, enclave_matches, shared_user_link_matches,
};
use x86_paging::{
    CanonicalAddress, ENTRIES, Entry, EntryError, Flags, LARGE_PAGE_SIZE, Level, MappingRange,
    MappingRangeError, PAGE_SIZE, TablePrivilege, page_identity_matches, user_mapping_fits,
};

use super::task::Released;
use super::{cpu, interrupt, pci};
use crate::mm::{self, Paddr, Segment, SharedBytes, SharedKeepalive, SharedRing, USegment};
use crate::mmio::MmioRegion;
use crate::sync::SpinLock;

mod activation;
mod device;
mod map;
mod reclaim;
mod roots;
#[cfg(feature = "selftest")]
mod selftest;
mod stack;
mod walk;

use super::kpti;
#[cfg(doc)]
use super::task;

#[cfg(feature = "selftest")]
use super::apic;

pub use activation::{activate, active_root, is_active};
pub use device::{
    AdmittedDeviceRange, BarMapFailure, BarUnmapFailure, BarUnmapOutcome, DeviceMapping,
    DeviceUnmapFailure, ReleasableDevice, admit_device_range,
};
use device::{DEVICE_WINDOW_END, DEVICE_WINDOW_START, DeviceExtent};
pub use map::BootRegion;
use map::Grain;
use roots::{enclave_access_policy, paired_flush_premise};
#[cfg(feature = "selftest")]
pub use selftest::{
    ActivationPublicationProbe, PaintedStack, activation_publication_for_test, cleared_on_drop,
    observe_activation_publication_for_test, observe_owned_byte_for_test,
    paint_current_permanent_stack_for_test,
    permanent_stack_rejects_lower_leaf_substitution_for_test, reclaimed_refusals,
    refuse_device_unmap_for_test, refuse_mapping_after_for_test,
    refuse_mapping_undo_after_for_test, refuse_stack_mapping_after_for_test,
    refuse_stack_rollback_after_for_test, restore_mapping_refusals_for_test,
    restore_stack_refusals_for_test,
};
#[cfg(feature = "selftest")]
use selftest::{
    CLEARED_ON_DROP, DEVICE_UNMAPS_BEFORE_REFUSAL, MAPPING_UNDOS_BEFORE_REFUSAL,
    MAPPING_WRITES_BEFORE_REFUSAL, RECLAIMED_REFUSALS, STACK_MAPPINGS_BEFORE_REFUSAL,
    STACK_ROLLBACKS_BEFORE_REFUSAL, mapping_refusal, one_shot_mapping_refusal, stack_refusal,
};
pub(crate) use stack::{KernelStackTop, TaskStackPin};
pub use stack::{MappedStack, PinnedStack, StackArena};
use stack::{StackIdentity, protected_stack_overlap};
pub use walk::translate;
use walk::{
    EntrySlot, clear_mapping_slot, current_mapping_root, direct_map_gap, entry_slot,
    kernel_image_gap, leaf_in, slot_in, translate_in, visit_present_leaves, walk,
};

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
    /// The full and stripped page-table roots do not form one valid pair.
    InvalidRootPair(RootPairError),
    /// PCID is enabled, so ordinary root loads do not provide D-407's flush.
    InvalidPairedFlush(FlushPremiseError),
    /// The entry enclave is missing, overbroad, or has the wrong identity.
    InvalidEntryEnclave(u64),
    /// The linker did not publish the exact entry-enclave geometry.
    EntryEnclaveUnavailable,
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
    /// Pages already returned before the refusal.
    pub returned_pages: usize,
    roots: AddressSpaceRoots,
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
    const fn remaining_stacks(&self) -> usize {
        match self {
            Self::One(_) => 1,
            Self::Prefix { remaining, .. } => *remaining,
        }
    }
}

impl StackRollback {
    /// Stack mappings present when rollback began.
    pub fn mapped_stacks(&self) -> usize {
        match &self.pending {
            StackCleanup::One(_) => 1,
            StackCleanup::Prefix { layout, remaining } => {
                let returned_stacks = match layout.slot(0) {
                    Some(slot) => {
                        let pages = slot.stack_len() / PAGE_SIZE;
                        self.returned_pages / pages as usize
                    }
                    None => 0,
                };
                remaining.saturating_add(returned_stacks)
            }
        }
    }

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
            Self::InvalidRootPair(error) => write!(f, "invalid paired roots: {error}"),
            Self::InvalidPairedFlush(error) => write!(f, "invalid paired-root flush: {error}"),
            Self::InvalidEntryEnclave(address) => {
                write!(f, "the entry enclave is invalid at {address:#x}")
            }
            Self::EntryEnclaveUnavailable => {
                core::fmt::Write::write_str(f, "the entry enclave is unavailable")
            }
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
    /// A paired space's lower-half top-level links do not name one subtree.
    UserHalfDiffers(usize),
    /// A space offered for binding was not built to share the kernel's half.
    NotAUserSpace,
    /// A paired space has not acquired a validated entry enclave.
    EntryEnclaveIncomplete,
    /// PCID is enabled, invalidating paired roots' ordinary-load flush proof.
    PairedFlushUnavailable,
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
            Self::UserHalfDiffers(index) => {
                write!(f, "top-level slot {index} does not share one user subtree")
            }
            Self::NotAUserSpace => {
                core::fmt::Write::write_str(f, "this space does not share the kernel's higher half")
            }
            Self::EntryEnclaveIncomplete => core::fmt::Write::write_str(
                f,
                "the paired address space has no validated entry enclave",
            ),
            Self::PairedFlushUnavailable => {
                core::fmt::Write::write_str(f, "PCID is enabled for a paired address space")
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

/// A four-level page table under construction.
///
/// Tables remain owned until explicit reclaim; `unmap` removes leaves without
/// risking a live sibling or freeing beneath `CR3` (D-88).
#[derive(Debug)]
pub struct AddressSpace {
    roots: Option<AddressSpaceRoots>,
    /// Linear ownership of both paired roots and every private table frame.
    pair_authority: Option<PairFrameAuthority>,
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
    /// Paired roots cannot become bindable until the exact entry enclave has
    /// been installed and validated by the KPTI owner (D-407).
    entry_enclave_ready: bool,
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
    root: TableRoot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableRoot {
    Full,
    Stripped,
}

/// The inseparable roots which identify one address space.
///
/// A shared-root space has no stripped member, so [`user_root`](Self::user_root)
/// names the full root. A paired space keeps both identities in this one value;
/// callers cannot manufacture or separate them into independently bindable
/// authorities (D-407).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressSpaceRoots {
    full: Paddr,
    /// The user root, with its otherwise-unused low bit marking a pair.
    ///
    /// Root addresses are page aligned, so this keeps the inseparable token
    /// to two words without reserving physical frame zero as a sentinel.
    user_tagged: Paddr,
}

impl AddressSpaceRoots {
    const PAIRED_TAG: Paddr = 1;

    const fn shared(full: Paddr) -> Self {
        Self {
            full,
            user_tagged: full,
        }
    }

    fn paired(full: Paddr, stripped: Paddr) -> Result<Self, MapError> {
        let pair = CheckedRootPair::new(full, stripped).map_err(MapError::InvalidRootPair)?;
        Ok(Self {
            full: pair.full(),
            user_tagged: pair.stripped() | Self::PAIRED_TAG,
        })
    }

    /// The root under which ordinary kernel code may execute.
    pub const fn full_root(self) -> Paddr {
        self.full
    }

    /// The root to install for ring-three execution.
    ///
    /// This equals [`full_root`](Self::full_root) only for D-250's positively
    /// admitted shared-root mode.
    pub const fn user_root(self) -> Paddr {
        if self.is_paired() {
            self.user_tagged & !Self::PAIRED_TAG
        } else {
            self.full
        }
    }

    /// The distinct stripped root, when this is a KPTI pair.
    pub const fn stripped_root(self) -> Option<Paddr> {
        if self.is_paired() {
            Some(self.user_root())
        } else {
            None
        }
    }

    /// Whether this identity contains a distinct stripped root.
    pub const fn is_paired(self) -> bool {
        self.user_tagged & Self::PAIRED_TAG != 0
    }

    const fn contains(self, root: Paddr) -> bool {
        self.full == root || self.is_paired() && self.user_root() == root
    }

    const fn frames(self) -> usize {
        if self.is_paired() { 2 } else { 1 }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserRootKind {
    Shared,
    Paired,
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
            roots: Some(AddressSpaceRoots::shared(root)),
            pair_authority: None,
            devices: Vec::new(),
            tables: Vec::new(),
            bound: false,
            entry_enclave_ready: true,
            reach: Reach::Everywhere,
            shared: Vec::new(),
            owned_pages: 0,
        })
    }

    /// The physical address of the top-level table — what `CR3` would hold.
    pub const fn root(&self) -> Paddr {
        match self.roots {
            Some(roots) => roots.full_root(),
            None => panic!("a reclaimed address space has no root"),
        }
    }

    const fn roots(&self) -> AddressSpaceRoots {
        match self.roots {
            Some(roots) => roots,
            None => panic!("a reclaimed address space has no roots"),
        }
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
        Self::for_user_roots(UserRootKind::Shared)
    }

    /// Build a user address space with a validated full/stripped root pair.
    ///
    /// The KPTI owner must mint the exact linker witness; mapping or validation
    /// refusal returns both roots and every intermediate table before this
    /// returns.
    pub fn for_user_isolated() -> Result<Self, MapError> {
        if !super::kpti::entry_path_is_implemented() {
            return Err(MapError::EntryEnclaveUnavailable);
        }
        let enclave = super::kpti::entry_enclave().ok_or(MapError::EntryEnclaveUnavailable)?;
        Self::for_user_paired(enclave)
    }

    fn for_user_paired(enclave: super::kpti::EntryEnclave) -> Result<Self, MapError> {
        let mut space = Self::for_user_roots(UserRootKind::Paired)?;
        if let Err(reason) = space.install_entry_enclave(enclave) {
            let returned = space.reclaim(None).unwrap_or_else(|failure| {
                panic!("unpublished paired-root cleanup refused: {failure}")
            });
            assert!(returned >= 2, "paired-root cleanup lost a root frame");
            return Err(reason);
        }
        space.entry_enclave_ready = true;
        Ok(space)
    }

    fn for_user_roots(kind: UserRootKind) -> Result<Self, MapError> {
        if !is_active() {
            return Err(MapError::OutOfMemory);
        }
        let kernel = active_root();
        let mut construction = match kind {
            UserRootKind::Shared => None,
            UserRootKind::Paired => Some(PairConstructionLedger::new()),
        };
        let full = mm::allocate_zeroed().ok_or(MapError::OutOfMemory)?;
        if let Some(ledger) = construction {
            construction = Some(
                ledger
                    .acquire_root()
                    .unwrap_or_else(|| panic!("paired construction acquired too many roots")),
            );
        }
        let stripped = match kind {
            UserRootKind::Shared => None,
            UserRootKind::Paired => Some(mm::allocate_zeroed().ok_or(MapError::OutOfMemory)?),
        };
        if let Some(ledger) = construction {
            construction = Some(
                ledger
                    .acquire_root()
                    .unwrap_or_else(|| panic!("paired construction acquired too many roots")),
            );
        }
        let full_root = full.address();
        let stripped_root = stripped.as_ref().map(mm::Frame::address);

        for index in ENTRIES / 2..ENTRIES {
            let from = entry_slot(kernel, index)?;
            let into = entry_slot(full_root, index)?;
            let entry = EntrySlot::read(&from);
            EntrySlot::write(&into, entry);
        }

        let roots = match stripped_root {
            Some(stripped) => AddressSpaceRoots::paired(full_root, stripped)?,
            None => AddressSpaceRoots::shared(full_root),
        };
        let pair_authority = construction.map(|ledger| {
            ledger
                .publish()
                .unwrap_or_else(|| panic!("paired construction published incomplete roots"))
        });
        core::mem::forget(full);
        if let Some(stripped) = stripped {
            core::mem::forget(stripped);
        }
        Ok(Self {
            roots: Some(roots),
            pair_authority,
            devices: Vec::new(),
            tables: Vec::new(),
            reach: Reach::UserHalfOnly,
            bound: false,
            entry_enclave_ready: kind == UserRootKind::Shared,
            shared: Vec::new(),
            owned_pages: 0,
        })
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
        if !self.entry_enclave_ready {
            return Err(ActivateError::EntryEnclaveIncomplete);
        }
        let roots = self.roots();
        if let Some(stripped) = roots.stripped_root() {
            paired_flush_premise(roots).map_err(|_| ActivateError::PairedFlushUnavailable)?;
            for index in 0..ENTRIES / 2 {
                let full = entry_slot(roots.full_root(), index)
                    .map_err(|_| ActivateError::UserHalfDiffers(index))?;
                let stripped = entry_slot(stripped, index)
                    .map_err(|_| ActivateError::UserHalfDiffers(index))?;
                if !shared_user_link_matches(EntrySlot::read(&full), EntrySlot::read(&stripped)) {
                    return Err(ActivateError::UserHalfDiffers(index));
                }
            }
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
        Ok(Binding { roots })
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
        if let Some(pinned) = protected_stack_overlap(self.roots(), virtual_base, len) {
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

impl Drop for AddressSpace {
    fn drop(&mut self) {
        if self.roots.is_some_and(|roots| {
            self.bound || Self::roots_are_active(roots, cpu::page_table_base())
        }) && !self.shared.is_empty()
        {
            let keepalives = core::mem::take(&mut self.shared);
            core::mem::forget(keepalives);
        }
    }
}

/// The double-fault table and the boot continuation each retain one stack.
const MAX_PERMANENT_STACKS: usize = 2;

static PERMANENT_STACKS: SpinLock<[Option<StackIdentity>; MAX_PERMANENT_STACKS]> =
    SpinLock::new([None; MAX_PERMANENT_STACKS]);

static TASK_STACK_PIN: SpinLock<Option<(AddressSpaceRoots, StackLayout)>> = SpinLock::new(None);

/// Reclaim the stack ledgers on the terminal panic path.
pub(crate) fn force_unlock(panic: &crate::panic::PanicToken) {
    PERMANENT_STACKS.force_unlock(panic);
    TASK_STACK_PIN.force_unlock(panic);
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

/// A space that may be installed on a context switch.
///
/// The evidence that [`AddressSpace::bind`] checked the higher half, carried
/// as a value so a caller cannot install a space that was never compared.
/// Neither `Copy` nor `Clone`: two slots naming one space would let a thread
/// exit and take another's tables with it.
#[derive(Debug)]
pub struct Binding {
    roots: AddressSpaceRoots,
}

impl Binding {
    /// The inseparable root identity checked for this binding.
    pub const fn roots(&self) -> AddressSpaceRoots {
        self.roots
    }
}
