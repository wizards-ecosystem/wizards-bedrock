//! The kernel's own page tables: raw walks and `mov cr3`, over the encoding in `x86_paging`.
//!
//! Present entries accept only identical rewrites. A clear invalidates when the current `CR3`
//! reaches its exact slot; other lower-half slots rely on the no-PCID, non-global root-load flush.

mod activation;
mod device;
mod map;
mod reclaim;
mod roots;
#[cfg(feature = "selftest")]
mod selftest;
mod stack;
mod walk;

pub use activation::{activate, active_root, is_active};
pub use device::{
    AdmittedDeviceRange, BarMapFailure, BarUnmapFailure, BarUnmapOutcome, DeviceMapping,
    DeviceUnmapFailure, ReleasableDevice, admit_device_range,
};
pub use map::BootRegion;
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
pub(crate) use stack::{KernelStackTop, TaskStackPin};
pub use stack::{MappedStack, PinnedStack, StackArena};
pub use walk::translate;

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

#[cfg(feature = "selftest")]
use super::apic;
use super::kpti;
#[cfg(doc)]
use super::task;
use super::task::Released;
use super::{cpu, interrupt, pci};
use crate::mm::{
    self, PhysicalAddress, Segment, SharedBytes, SharedKeepalive, SharedRing, USegment,
};
use crate::mmio::MmioRegion;
use crate::sync::SpinLock;
use device::{DEVICE_WINDOW_END, DEVICE_WINDOW_START, DeviceExtent};
use map::Grain;
use roots::{enclave_access_policy, paired_flush_premise};
#[cfg(feature = "selftest")]
use selftest::{
    CLEARED_ON_DROP, DEVICE_UNMAPS_BEFORE_REFUSAL, MAPPING_UNDOS_BEFORE_REFUSAL,
    MAPPING_WRITES_BEFORE_REFUSAL, RECLAIMED_REFUSALS, STACK_MAPPINGS_BEFORE_REFUSAL,
    STACK_ROLLBACKS_BEFORE_REFUSAL, mapping_refusal, one_shot_mapping_refusal, stack_refusal,
};
use stack::{StackIdentity, protected_stack_overlap};
use walk::{
    EntrySlot, clear_mapping_slot, current_mapping_root, direct_map_gap, entry_slot,
    kernel_image_gap, leaf_in, slot_in, translate_in, visit_present_leaves, walk,
};

/// Why a mapping could not be made.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum MapError {
    /// No frame was free for a table this mapping needed.
    OutOfMemory,
    /// A table this mapping needs is outside the direct map, so it cannot be written.
    Unreachable(PhysicalAddress),
    /// The address range is already mapped by a larger page.
    Conflict(u64),
    /// The virtual address is not canonical, so no walk can reach it.
    NonCanonical(u64),
    /// The range's start or length is not page-aligned.
    Misaligned(u64),
    /// Device registers are not four-byte aligned, so [`MmioRegion`] accesses would be misaligned.
    UnalignedDevice(PhysicalAddress),
    /// A physical address did not fit the entry encoding.
    Encoding(EntryError),
    /// The complete mapping range failed arithmetic or architecture checks.
    InvalidRange(MappingRangeError),
    /// A device range was empty or its page-rounded extent overflowed.
    InvalidDeviceRange(PhysicalAddress),
    /// The running processor did not publish a usable `MAXPHYADDR`.
    PhysicalWidthUnavailable,
    /// A device mapping needs a physical-address bit this processor reserves.
    OutsidePhysicalWidth {
        /// Reported `MAXPHYADDR`.
        bits: u8,
        /// First requested register byte.
        physical: PhysicalAddress,
    },
    /// The range overlaps RAM or firmware storage, so it is no device hole.
    NotDeviceMemory(PhysicalAddress),
    /// Another mapping in this address space already owns physical bytes.
    DeviceOverlap(PhysicalAddress),
    /// The bounded device-ownership table is full.
    TooManyDevices,
    /// The boot handoff never described this physical range, so nothing shows it is memory.
    Undescribed(PhysicalAddress),
    /// Nothing is mapped at this address.
    NotMapped(u64),
    /// The space does not own this frame, so unmapping would free memory the allocator never had.
    NotOwned(u64),
    /// A large page covers this address, and large pages are never split, so the direct map holds.
    Unsplittable(u64),
    /// The frames are not the one ascending run a single `map_owned` makes.
    Discontiguous(u64),
    /// The range is outside the half this space owns; shared kernel tables are never its to edit.
    NotYours(u64),
    /// The range touches a retained stack or its guard.
    Pinned(u64),
    /// A shared mapping's cache policy would alias the direct map's write-back view of its frames.
    CacheAlias(u64),
    /// A present mapping would be both writable and executable.
    WriteExecute(u64),
    /// A hardware-retained stack was requested in a space this CPU is not walking.
    Inactive(PhysicalAddress),
    /// Activation froze kernel top-level entries: a new one would be missing from older user roots.
    KernelTopologyFrozen(u64),
    /// The full and stripped page-table roots do not form one valid pair.
    InvalidRootPair(RootPairError),
    /// PCID is enabled, so ordinary root loads do not provide the paired roots' flush.
    InvalidPairedFlush(FlushPremiseError),
    /// The entry enclave is missing, overbroad, or has the wrong identity.
    InvalidEntryEnclave(u64),
    /// The linker did not publish the exact entry-enclave geometry.
    EntryEnclaveUnavailable,
}

impl From<MappingRangeError> for MapError {
    fn from(reason: MappingRangeError) -> Self {
        Self::InvalidRange(reason)
    }
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::OutOfMemory => f.write_str("no free frame for a page table"),
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
            Self::Encoding(reason) => write!(f, "{reason}"),
            Self::InvalidRange(reason) => write!(f, "{reason}"),
            Self::InvalidDeviceRange(address) => {
                write!(f, "device range at {address:#x} is empty or overflows")
            }
            Self::PhysicalWidthUnavailable => {
                f.write_str("the processor did not report a usable MAXPHYADDR")
            }
            Self::OutsidePhysicalWidth { bits, physical } => write!(
                f,
                "device range at {physical:#x} needs bits outside MAXPHYADDR {bits}"
            ),
            Self::NotDeviceMemory(address) => {
                write!(f, "device range at {address:#x} overlaps ordinary memory")
            }
            Self::DeviceOverlap(address) => {
                write!(f, "device range at {address:#x} is already owned")
            }
            Self::TooManyDevices => f.write_str("device ownership table is full"),
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
            Self::Discontiguous(address) => {
                write!(f, "the frames behind {address:#x} are not one run")
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
            Self::InvalidRootPair(reason) => write!(f, "invalid paired roots: {reason}"),
            Self::InvalidPairedFlush(reason) => write!(f, "invalid paired-root flush: {reason}"),
            Self::InvalidEntryEnclave(address) => {
                write!(f, "the entry enclave is invalid at {address:#x}")
            }
            Self::EntryEnclaveUnavailable => f.write_str("the entry enclave is unavailable"),
        }
    }
}

impl core::error::Error for MapError {}

/// A refused user mapping which retains its external-byte allocation.
#[derive(PartialEq, Eq, Debug)]
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
        fmt::Display::fmt(&self.reason(), f)
    }
}

impl core::error::Error for UserMapError {}

/// A stack rollback which stopped before every committed range was removed.
#[derive(PartialEq, Eq, Debug)]
pub struct StackRollback {
    /// Why the next stack could not be removed.
    pub reason: MapError,
    /// Pages already returned before the refusal.
    pub returned_pages: usize,
    roots: AddressSpaceRoots,
    pending: StackCleanup,
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

#[derive(PartialEq, Eq, Debug)]
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

/// Why guarded-stack construction could not finish atomically.
#[derive(PartialEq, Eq, Debug)]
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

/// Why a table was refused at the moment of the switch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ActivateError {
    /// A second caller tried to switch; the kernel's address space is installed once.
    AlreadyActive,
    /// The space's tables went back to the allocator, so it has no root to install.
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
    /// The new tables lack the writable, never-executable direct-map page frame accesses assume.
    DirectMapGap(u64),
    /// A space offered for binding disagrees with the kernel's higher half at this top-level slot.
    KernelHalfDiffers(usize),
    /// A paired space's lower-half top-level links do not name one subtree.
    UserHalfDiffers(usize),
    /// A space offered for binding was not built to share the kernel's half.
    NotAUserSpace,
    /// A paired space has not acquired a validated entry enclave.
    EntryEnclaveIncomplete,
    /// PCID is enabled, invalidating paired roots' ordinary-load flush proof.
    PairedFlushUnavailable,
    /// The device window's shared top-level table could not be reserved before topology froze.
    SharedTopology(MapError),
}

impl fmt::Display for ActivateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::AlreadyActive => f.write_str("an address space is already installed"),
            Self::Reclaimed => f.write_str("the space has been reclaimed"),
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
                f.write_str("this space does not share the kernel's higher half")
            }
            Self::EntryEnclaveIncomplete => {
                f.write_str("the paired address space has no validated entry enclave")
            }
            Self::PairedFlushUnavailable => {
                f.write_str("PCID is enabled for a paired address space")
            }
            Self::SharedTopology(reason) => {
                write!(f, "shared kernel topology could not be reserved: {reason}")
            }
        }
    }
}

impl core::error::Error for ActivateError {}

/// Why an address space kept its page-table frames.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ReclaimError {
    /// This space was already reclaimed.
    Reclaimed,
    /// A task still names this bound space and no release evidence was given.
    StillBound(PhysicalAddress),
    /// The release evidence names a different space.
    WrongRelease {
        /// Root this space owns.
        expected: PhysicalAddress,
        /// Root the evidence releases.
        found: PhysicalAddress,
    },
    /// Release evidence was supplied to a space that was never bound.
    UnexpectedRelease(PhysicalAddress),
    /// The CPU is currently walking this space.
    Active(PhysicalAddress),
    /// Owned leaf mappings still depend on these tables.
    Mapped(usize),
    /// Device mappings still depend on these tables.
    Devices(usize),
}

impl fmt::Display for ReclaimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Reclaimed => f.write_str("the address space was already reclaimed"),
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

/// A four-level page table whose tables stay owned until an explicit reclaim.
///
/// `unmap` removes leaves without risking a live sibling or freeing a table beneath `CR3`.
#[derive(Debug)]
pub struct AddressSpace {
    roots: Option<AddressSpaceRoots>,
    /// Linear ownership of both paired roots and every private table frame.
    pair_authority: Option<PairFrameAuthority>,
    /// Physical ranges already promised to device mappings in this space.
    devices: Vec<Option<DeviceExtent>>,
    /// Tables `set` allocated walking down, returned with the roots; never the shared kernel ones.
    tables: Vec<OwnedTable>,
    /// Which half of the address space this one may be told to change.
    reach: Reach,
    /// Whether a thread already runs on these tables.
    bound: bool,
    /// A paired space binds only after the KPTI owner installs and validates its exact enclave.
    entry_enclave_ready: bool,
    /// Allocations reached by non-owning user mappings in this space.
    shared: Vec<SharedKeepalive>,
    /// Leaf pages whose frame ownership moved into this space.
    owned_pages: usize,
}

#[derive(Clone, Copy, Debug)]
struct OwnedTable {
    frame: PhysicalAddress,
    virtual_address: u64,
    level: Level,
    link: Entry,
    root: TableRoot,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TableRoot {
    Full,
    Stripped,
}

/// The inseparable roots of one address space; callers cannot make or split them into bindings.
///
/// A shared-root space has no stripped member, so [`user_root`](Self::user_root) is the full root.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AddressSpaceRoots {
    full: PhysicalAddress,
    /// The user root, its page-aligned low bit marking a pair, so frame zero needs no sentinel.
    user_tagged: PhysicalAddress,
}

impl AddressSpaceRoots {
    const PAIRED_TAG: PhysicalAddress = 1;

    const fn shared(full: PhysicalAddress) -> Self {
        Self {
            full,
            user_tagged: full,
        }
    }

    fn paired(full: PhysicalAddress, stripped: PhysicalAddress) -> Result<Self, MapError> {
        let pair = CheckedRootPair::new(full, stripped).map_err(MapError::InvalidRootPair)?;
        Ok(Self {
            full: pair.full(),
            user_tagged: pair.stripped() | Self::PAIRED_TAG,
        })
    }

    /// The root under which ordinary kernel code may execute.
    pub const fn full_root(self) -> PhysicalAddress {
        self.full
    }

    /// The ring-three root; it equals [`full_root`](Self::full_root) only in admitted shared mode.
    pub const fn user_root(self) -> PhysicalAddress {
        if self.is_paired() {
            self.user_tagged & !Self::PAIRED_TAG
        } else {
            self.full
        }
    }

    /// The distinct stripped root, when this is a KPTI pair.
    pub const fn stripped_root(self) -> Option<PhysicalAddress> {
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

    const fn contains(self, root: PhysicalAddress) -> bool {
        self.full == root || self.is_paired() && self.user_root() == root
    }

    const fn frames(self) -> usize {
        if self.is_paired() { 2 } else { 1 }
    }
}

/// How much of a space its owner may map: a user space shares live kernel tables by reference,
/// so it may change only its lower half, or a safe handle could edit what the kernel runs on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reach {
    /// The whole space: the kernel's own, which owns every table in it.
    Everywhere,
    /// The lower half only; everything above is the kernel's tables.
    UserHalfOnly,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum UserRootKind {
    Shared,
    Paired,
}

/// The first non-user address, re-exported by user-copy so copy and mapping bounds cannot drift.
///
/// No user mapping reaches the page below it; a literal because the C header reads this ABI value.
pub const USER_END: u64 = 0x0000_8000_0000_0000;

const _: () = assert!(USER_END == x86_paging::USER_END);

/// Whether ring 3 could name `address`; this also refuses the canonical hole at any walk width.
pub const fn is_user_address(address: u64) -> bool {
    address < USER_END
}

/// Flags only the mapping code chooses: `OWNED` is provenance and `HUGE` comes from the level.
/// `GLOBAL` would evade the root-load flush an inactive mutable root relies on.
const NOT_THE_CALLERS: Flags = Flags::OWNED.union(Flags::HUGE).union(Flags::GLOBAL);

/// The cache-type bits a caller may name on a mapping of RAM.
const CACHE_TYPE: Flags = Flags::NO_CACHE.union(Flags::WRITE_THROUGH);

/// RAM keeps its write-back direct-map type in every mapping; an alias of another type corrupts it.
fn ram_cache_policy(virtual_base: u64, flags: Flags) -> Result<(), MapError> {
    if flags.intersection(CACHE_TYPE) != Flags::NONE {
        return Err(MapError::CacheAlias(virtual_base));
    }
    Ok(())
}

impl AddressSpace {
    /// An address space mapping nothing, or [`MapError::OutOfMemory`] without a root frame.
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

    /// The physical address of the top-level table, which `CR3` would hold.
    pub const fn root(&self) -> PhysicalAddress {
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

    /// Translate through this space's tables, active or not, as [`translate`] does for `CR3`.
    pub fn lookup(&self, virtual_address: u64) -> Option<(PhysicalAddress, Flags)> {
        // Reclaim removes the root owner; no later read may walk its recycled frame.
        if self.is_reclaimed() {
            #[cfg(feature = "selftest")]
            RECLAIMED_REFUSALS.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        translate_in(self.root(), virtual_address)
    }

    /// A program's space: an empty lower half and the kernel's top-level entries by reference.
    ///
    /// Activation freezes new higher-half top-level entries, so later mappings change only tables
    /// every user root shares. `OutOfMemory` before activation or without a root frame.
    pub fn for_user() -> Result<Self, MapError> {
        Self::for_user_roots(UserRootKind::Shared)
    }

    /// A user space with a validated root pair; the KPTI owner must mint the exact linker witness.
    ///
    /// A mapping or validation refusal returns both roots and every intermediate table first.
    pub fn for_user_isolated() -> Result<Self, MapError> {
        if !kpti::entry_path_is_implemented() {
            return Err(MapError::EntryEnclaveUnavailable);
        }
        let enclave = kpti::entry_enclave().ok_or(MapError::EntryEnclaveUnavailable)?;
        Self::for_user_paired(enclave)
    }

    fn for_user_paired(enclave: kpti::EntryEnclave) -> Result<Self, MapError> {
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

    /// Give this space up for context switches; the linear [`Binding`] keeps reclaim single.
    ///
    /// Every higher-half top-level entry must match the kernel's address and software flags,
    /// ignoring hardware `ACCESSED`, since sampling addresses cannot prove the whole switch safe.
    /// Refuses `AlreadyActive` before activation and on a second bind. [`reclaim`](Self::reclaim)
    /// accepts the evidence retirement produces.
    pub fn bind(&mut self) -> Result<Binding, ActivateError> {
        if !is_active() || self.bound {
            return Err(ActivateError::AlreadyActive);
        }
        // A reclaimed space's root would read as frame zero and then be installed.
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

    /// Take away exactly what one [`map_owned`](Self::map_owned) put there, and return its frames.
    ///
    /// Every page must be present, four-kilobyte, owned and in one ascending run, with no byte
    /// pinned. A cleared leaf is invalidated at once when the current `CR3` reaches its slot,
    /// shared tables included; other lower-half slots have no PCID tag or global leaf, so loading
    /// their root flushes. [`Unmapped`] therefore carries no translation debt.
    pub fn unmap(&mut self, virtual_base: u64, len: u64) -> Result<Unmapped, MapError> {
        if !virtual_base.is_multiple_of(PAGE_SIZE) || len == 0 || !len.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Misaligned(virtual_base));
        }
        // Both ends: a walk masks bits 48 and up, so a noncanonical alias could not be invalidated.
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

        // Preflight the whole range before clearing anything, so a refusal changes nothing.
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
            // One user-reachable page makes the whole run user bytes, since one page discloses.
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
            // SAFETY: preflight proved the `pages` frames at `base` one ascending run a `map_owned`
            // moved here that nothing else holds, and the entries naming them were just cleared.
            frames: Some(unsafe { Segment::from_raw(base, pages as usize) }),
            was_user,
        })
    }

    fn leaf(&self, address: u64) -> Result<EntrySlot, MapError> {
        leaf_in(self.root(), address)
    }
}

/// A zeroed table frame whose handle is forgotten, so only the direct map reaches it.
///
/// The kernel's own tables never return; a user space records each so its reclaim can.
fn take_table() -> Result<PhysicalAddress, MapError> {
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

/// Frames detached from an address space, with their provenance and no translation debt left.
///
/// User bytes never become a kernel allocation uncleared; forgetting this only leaks the frames.
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
    /// User bytes are cleared before reuse; a failed clear leaks the frames so none are disclosed.
    fn drop(&mut self) {
        if self.was_user
            && let Some(frames) = self.frames.as_mut()
        {
            if frames.zero() {
                #[cfg(feature = "selftest")]
                CLEARED_ON_DROP.fetch_add(frames.frames(), Ordering::Relaxed);
            } else {
                core::mem::forget(self.frames.take());
            }
        }
    }
}

/// Evidence [`AddressSpace::bind`] compared the higher half, so only a compared space installs.
///
/// Not `Copy` or `Clone`: two slots naming one space would let one thread's exit take its tables.
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
