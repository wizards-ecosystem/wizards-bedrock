mod activation;
mod device;
mod error;
mod map;
mod reclaim;
mod roots;
mod stack;
mod walk;

pub use activation::activate;
pub(crate) use activation::{active_root, is_active};
pub use device::{
    AdmittedDeviceRange, BarMapFailure, BarUnmapFailure, BarUnmapOutcome, DeviceMapping,
    DeviceUnmapFailure, ReleasableDevice, admit_device_range,
};
pub use error::{ActivateError, MapError, ReclaimError, ReclaimFailure, UserMapError};
pub use map::{BootRegion, Unmapped};
pub(crate) use stack::{KernelStackTop, TaskStackPin};
pub use stack::{MappedStack, PinnedStack, StackArena};
pub use walk::translate;

use alloc::vec::Vec;

use stack_layout::StackLayout;
use x86_paging::kpti::shared_user_link_matches;
use x86_paging::{ENTRIES, Entry, Flags, Level};

use super::cpu;
use super::kpti;
use super::task::Released;
use crate::mm::{self, PhysicalAddress, SharedKeepalive};
use crate::sync::SpinLock;
use device::{DEVICE_WINDOW_END, DEVICE_WINDOW_START, DeviceExtent};
use map::Grain;
use stack::{StackIdentity, protected_stack_overlap};
use walk::{
    EntrySlot, clear_mapping_slot, current_mapping_root, direct_map_gap, entry_slot,
    handed_out_gap, kernel_image_gap, leaf_in, slot_in, translate_in, visit_present_leaves, walk,
};

#[derive(Debug)]
pub struct AddressSpace {
    roots: Option<AddressSpaceRoots>,
    devices: Vec<Option<DeviceExtent>>,
    tables: Vec<OwnedTable>,
    reach: Reach,
    bound: bool,
    entry_enclave_ready: bool,
    shared: Vec<SharedKeepalive>,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AddressSpaceRoots {
    full: PhysicalAddress,
    user: PhysicalAddress,
}

impl AddressSpaceRoots {
    const fn shared(full: PhysicalAddress) -> Self {
        Self { full, user: full }
    }

    const fn paired(full: PhysicalAddress, stripped: PhysicalAddress) -> Self {
        Self {
            full,
            user: stripped,
        }
    }

    pub(crate) const fn full_root(self) -> PhysicalAddress {
        self.full
    }

    pub(crate) const fn user_root(self) -> PhysicalAddress {
        self.user
    }

    pub(crate) const fn stripped_root(self) -> Option<PhysicalAddress> {
        if self.is_paired() {
            Some(self.user_root())
        } else {
            None
        }
    }

    pub(crate) const fn is_paired(self) -> bool {
        self.user.get() != self.full.get()
    }

    const fn contains(self, root: PhysicalAddress) -> bool {
        let root = root.get();
        self.full.get() == root || self.is_paired() && self.user_root().get() == root
    }

    const fn frames(self) -> usize {
        if self.is_paired() { 2 } else { 1 }
    }
}

// A user space shares the kernel tables by reference, so it may change only its lower half.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reach {
    Everywhere,
    UserHalfOnly,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum UserRootKind {
    Shared,
    Paired,
}

// A literal because the C header reads this ABI value.
pub const USER_END: u64 = 0x0000_8000_0000_0000;

const _: () = assert!(
    USER_END == x86_paging::USER_END,
    "the ABI user end must match the paging layout"
);

// This also refuses the canonical hole at any walk width.
pub(crate) const fn is_user_address(address: u64) -> bool {
    address < USER_END
}

// `GLOBAL` would evade the root-load flush an inactive mutable root relies on.
const NOT_THE_CALLERS: Flags = Flags::OWNED.union(Flags::HUGE).union(Flags::GLOBAL);

const CACHE_TYPE: Flags = Flags::NO_CACHE.union(Flags::WRITE_THROUGH);

// RAM keeps its write-back direct-map type in every mapping; another type corrupts it.
fn ram_cache_policy(virtual_base: u64, flags: Flags) -> Result<(), MapError> {
    if flags.intersection(CACHE_TYPE) != Flags::NONE {
        return Err(MapError::CacheAlias(virtual_base));
    }
    Ok(())
}

impl AddressSpace {
    pub fn new() -> Result<Self, MapError> {
        let frame = mm::allocate_table().ok_or(MapError::OutOfMemory)?;
        let root = frame.address();
        core::mem::forget(frame);
        Ok(Self {
            roots: Some(AddressSpaceRoots::shared(root)),
            devices: Vec::new(),
            tables: Vec::new(),
            bound: false,
            entry_enclave_ready: true,
            reach: Reach::Everywhere,
            shared: Vec::new(),
            owned_pages: 0,
        })
    }

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

    pub fn lookup(&self, virtual_address: u64) -> Option<(PhysicalAddress, Flags)> {
        // Reclaim removes the root owner; no later read may walk its recycled frame.
        if self.is_reclaimed() {
            return None;
        }
        translate_in(self.root(), virtual_address)
    }

    // Activation freezes higher-half top-level entries, so kernel maps reach every user root.
    pub fn for_user() -> Result<Self, MapError> {
        Self::for_user_roots(UserRootKind::Shared)
    }

    pub fn for_user_isolated() -> Result<Self, MapError> {
        let enclave = kpti::entry_enclave().ok_or(MapError::EntryEnclaveUnavailable)?;
        let mut space = Self::for_user_roots(UserRootKind::Paired)?;
        if let Err(reason) = space.install_entry_enclave(enclave) {
            if let Err(failure) = space.reclaim(None) {
                panic!("unpublished paired-root cleanup refused: {failure}");
            }
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
        let full = mm::allocate_table().ok_or(MapError::OutOfMemory)?;
        let stripped = match kind {
            UserRootKind::Shared => None,
            UserRootKind::Paired => Some(mm::allocate_table().ok_or(MapError::OutOfMemory)?),
        };
        let full_root = full.address();

        for index in ENTRIES / 2..ENTRIES {
            let from = entry_slot(kernel, index)?;
            let into = entry_slot(full_root, index)?;
            let entry = from.read();
            into.write(entry);
        }

        let roots = match stripped.as_ref() {
            Some(stripped) => AddressSpaceRoots::paired(full_root, stripped.address()),
            None => AddressSpaceRoots::shared(full_root),
        };
        core::mem::forget(full);
        core::mem::forget(stripped);
        Ok(Self {
            roots: Some(roots),
            devices: Vec::new(),
            tables: Vec::new(),
            reach: Reach::UserHalfOnly,
            bound: false,
            entry_enclave_ready: kind == UserRootKind::Shared,
            shared: Vec::new(),
            owned_pages: 0,
        })
    }

    pub fn bind(&mut self) -> Result<Binding, ActivateError> {
        if !is_active() || self.bound {
            return Err(ActivateError::AlreadyActive);
        }
        // A reclaimed space's root would read as frame zero and then be installed.
        if self.is_reclaimed() {
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
            for index in 0..ENTRIES / 2 {
                let full = entry_slot(roots.full_root(), index)
                    .map_err(|_| ActivateError::UserHalfDiffers(index))?;
                let stripped = entry_slot(stripped, index)
                    .map_err(|_| ActivateError::UserHalfDiffers(index))?;
                if !shared_user_link_matches(full.read(), stripped.read()) {
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
            let expected = theirs.read();
            let found = ours.read();
            if !expected.table_identity_matches(found) {
                return Err(ActivateError::KernelHalfDiffers(index));
            }
        }
        self.bound = true;
        Ok(Binding { roots })
    }
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

const MAX_PERMANENT_STACKS: usize = 2;

static PERMANENT_STACKS: SpinLock<[Option<StackIdentity>; MAX_PERMANENT_STACKS]> =
    SpinLock::new([None; MAX_PERMANENT_STACKS]);

static TASK_STACK_PIN: SpinLock<Option<(AddressSpaceRoots, StackLayout)>> = SpinLock::new(None);

pub(crate) fn force_unlock(panic: &crate::panic::PanicToken) {
    PERMANENT_STACKS.force_unlock(panic);
    TASK_STACK_PIN.force_unlock(panic);
}

// Not `Clone`: two slots naming one space would let one thread's exit take its tables.
#[derive(Debug)]
pub struct Binding {
    roots: AddressSpaceRoots,
}

impl Binding {
    pub(crate) const fn roots(&self) -> AddressSpaceRoots {
        self.roots
    }
}
