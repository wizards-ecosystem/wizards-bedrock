//! The first thing that runs in ring 3.
//!
//! Not a program and not a process: a page of instructions, a page of stack,
//! and a thread that stops being the kernel's. What it proves is the boundary
//! — that the descriptors, the gate, the privilege stack and the return path
//! all agree — before a loader makes disagreement harder to diagnose (D-96).

use core::fmt;
#[cfg(feature = "selftest")]
use core::sync::atomic::AtomicBool;
use core::sync::atomic::{AtomicUsize, Ordering};

use elf64::{Elf, ElfError, SegmentKind};
use handle_space::Rights;
use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::paging::{ActivateError, AddressSpace, MapError, ReclaimError, Unmapped};
use wost::arch::current::task;
use wost::arch::current::usercopy;
use wost::arch::current::{cpu, xstate};
use wost::boot::HandoffError;
use wost::mm;
use wost::mm::Paddr;
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

use crate::handle::{HandleError, HandleTable};
use crate::object;
#[cfg(feature = "selftest")]
use crate::object::ObjectType;
#[cfg(feature = "selftest")]
use crate::wait::WaitQueue;
use crate::{info, queue, thread, warn};

/// Where the first user program's page of code goes.
///
/// Low half, far from anything the kernel maps: an address a user mapping can
/// occupy is by definition one the kernel does not, and the split is bit 63
/// (`010` §10's LASS note).
#[cfg(feature = "selftest")]
pub(crate) const CODE: u64 = 0x0000_4000_0000_0000;

/// Where its stack goes, well away from the code so a run-off hits nothing.
pub(crate) const STACK: u64 = 0x0000_3000_0000_0000;

/// Where its ring's shared pages go.
///
/// Every program gets one at the same address in its own space, which is what
/// having a space of its own buys: a fixed ABI address costs nothing when no
/// two programs share a table (D-113).
pub(crate) const RING: u64 = 0x0000_2000_0000_0000;

/// How far apart two programs' pages sit *within* their own spaces.
///
/// Page tables enforce isolation; this offset keeps neighbouring programs'
/// recorded addresses distinct without re-reading those tables (D-106).
const APART: u64 = 0x1_0000;

/// Bytes of the *program*'s stack, which is one page: it makes one call and
/// never returns. Named apart from `thread::STACK_BYTES`, which is the
/// thirty-two kilobytes a *kernel* thread gets — one name for two sizes reads
/// like one size (D-124).
const USER_STACK_BYTES: u64 = PAGE_SIZE;

/// The program's first four bytes, for the check that reads them back across
/// the boundary.
///
/// `pop rcx` and the start of its version comparison — the program reading and
/// validating the entry ABI before either payload word (D-102, D-182). Written
/// by `xtask` into the image (D-100) rather than by the kernel, so this is a
/// statement about the initrd and not a copy of something the kernel placed.
#[cfg(feature = "selftest")]
pub(crate) const PROGRAM: [u8; 4] = [0x59, 0x48, 0x83, 0xF9];

/// Map the program and its stack, and report where to enter.
///
/// Both pages are user-accessible and neither is both writable and executable:
/// the code page is read-only and executable, the stack is writable and never
/// executable. W^X is not relaxed for user memory — it is the reason user
/// memory exists (D-12).
fn place_stack(space: &mut AddressSpace, which: u64) -> Option<u64> {
    let at = STACK + which * APART;
    let mut stack = mm::allocate_contiguous((USER_STACK_BYTES / PAGE_SIZE) as usize, 1)?;
    if !stack.zero() {
        return None;
    }
    // Untyped from here: the program writes this stack, so nothing the kernel
    // later reads out of these frames may be believed (D-107).
    space
        .map_user(at, stack.into(), Flags::WRITABLE.union(Flags::NO_EXECUTE))
        .ok()?;
    Some(at)
}

/// Where the boot handoff left the program, kept for the loader.
static IMAGE: SpinLock<Option<&'static [u8]>> = SpinLock::new(None);

/// The initrd's bytes, for the checks that doctor a copy of them.
#[cfg(feature = "selftest")]
pub(crate) fn image() -> Option<&'static [u8]> {
    *IMAGE.lock_leaf()
}

/// Remember the initrd the bootloader handed over.
pub(crate) fn keep(module: Result<&'static [u8], HandoffError>) {
    match module {
        Ok(bytes) => {
            info!("initrd: {} bytes", bytes.len());
            *IMAGE.lock_leaf() = Some(bytes);
        }
        Err(reason) => warn!("initrd: none: {reason}"),
    }
}

/// Why a program could not be loaded.
#[derive(Debug, Clone, Copy)]
pub(crate) enum LoadError {
    /// No initrd was handed over.
    Missing,
    /// It is not an ELF this kernel reads.
    NotAnElf(ElfError),
    /// A segment claims more file than memory.
    Malformed(u64),
    /// A segment's contents run past the end of the image.
    Truncated(u64),
    /// Two segments want pages that overlap.
    Overlapping(u64),
    /// A segment's address is not page-aligned after relocation.
    Unaligned(u64),
    /// A segment's ELF alignment is invalid or file-incongruent.
    InvalidAlignment { at: u64, alignment: u64 },
    /// The image has more segments than the loader will place.
    TooManySegments,
    /// A segment asks to be both writable and executable.
    WriteExecute(u64),
    /// A segment is not in the half of the address space user code lives in.
    NotUserMemory(u64),
    /// There was no memory for it.
    OutOfMemory,
    /// It could not be mapped.
    NotMapped(MapError),
    /// A failed mapping could not undo every earlier placement.
    Rollback {
        /// Why rollback stopped.
        reason: MapError,
        /// Pages returned before that refusal.
        pages: usize,
    },
    /// Its entry point is not inside an executable load segment.
    InvalidEntry(u64),
    /// It has nothing to load.
    Empty,
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Missing => core::fmt::Write::write_str(f, "no initrd"),
            Self::NotAnElf(error) => write!(f, "{error}"),
            Self::Malformed(at) => write!(f, "the segment at {at:#x} claims more file than memory"),
            Self::Truncated(at) => write!(f, "the segment at {at:#x} runs past the image"),
            Self::Overlapping(at) => write!(f, "the segment at {at:#x} overlaps another"),
            Self::Unaligned(at) => write!(f, "the segment at {at:#x} is not page-aligned"),
            Self::InvalidAlignment { at, alignment } => write!(
                f,
                "the segment at {at:#x} has invalid alignment {alignment:#x}"
            ),
            Self::TooManySegments => {
                core::fmt::Write::write_str(f, "the image has more segments than the loader places")
            }
            Self::WriteExecute(at) => {
                write!(
                    f,
                    "the segment at {at:#x} asks to be writable and executable"
                )
            }
            Self::NotUserMemory(at) => write!(f, "the segment at {at:#x} is not user memory"),
            Self::OutOfMemory => core::fmt::Write::write_str(f, "no memory for the program"),
            Self::NotMapped(error) => write!(f, "{error}"),
            Self::Rollback { reason, pages } => {
                write!(f, "loader rollback refused after {pages} page(s): {reason}")
            }
            Self::InvalidEntry(at) => {
                write!(
                    f,
                    "the entry point at {at:#x} is not executable program memory"
                )
            }
            Self::Empty => core::fmt::Write::write_str(f, "the program has nothing to load"),
        }
    }
}

/// Load the initrd's program into the running address space, at `shift` pages
/// above where it asks to be.
///
/// Every segment is checked before *any* page is taken: inside the image,
/// inside user memory, never both writable and executable, and never
/// overlapping another segment's pages. **W^X is not relaxed for a program
/// that asks nicely** — an image wanting a writable executable page is
/// refused, because an image that needs one does not work without it and
/// pretending otherwise loads something that will fault later and blame the
/// kernel (D-100).
///
/// The pages are filled *before* they are mapped, so a segment that is
/// executable is never writable even for the instant it is being written.
fn load(space: &mut AddressSpace, shift: u64) -> Result<LoadedImage, LoadError> {
    let bytes = (*IMAGE.lock_leaf()).ok_or(LoadError::Missing)?;
    load_image(space, bytes, shift)
}

/// One image load and the exact ranges it installed.
///
/// Placement is transaction data rather than global last-call state, so two
/// constructions cannot take each other's cleanup record (D-191).
#[derive(Debug, Clone, Copy)]
pub(crate) struct LoadedImage {
    entry: u64,
    regions: [Option<(u64, u64)>; MAX_SEGMENTS],
}

impl LoadedImage {
    /// Address at which execution begins.
    const fn entry(&self) -> u64 {
        self.entry
    }

    /// Ranges this load installed, for cleanup evidence.
    pub(crate) fn regions(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        self.regions.iter().copied().flatten()
    }
}

/// Fill and map one validated placement.
fn place(space: &mut AddressSpace, bytes: &[u8], placement: &Placement) -> Result<(), LoadError> {
    let mut run = mm::allocate_contiguous(placement.pages, 1).ok_or(LoadError::OutOfMemory)?;
    if !run.zero() {
        return Err(LoadError::OutOfMemory);
    }
    let Some(contents) = bytes.get(placement.from..placement.from + placement.len) else {
        return Err(LoadError::Truncated(placement.at));
    };
    if !run.write_at(0, contents) {
        return Err(LoadError::Truncated(placement.at));
    }
    space
        .map_user(placement.at, run.into(), placement.flags)
        .map_err(|error| LoadError::NotMapped(error.reason()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnmapRefusal {
    pages: usize,
    reason: MapError,
}

#[cfg(feature = "selftest")]
const CLEANUP_REFUSAL_DISABLED: usize = usize::MAX;
#[cfg(feature = "selftest")]
static CLEANUP_UNMAPS_BEFORE_REFUSAL: AtomicUsize = AtomicUsize::new(CLEANUP_REFUSAL_DISABLED);
#[cfg(feature = "selftest")]
static CLEANUP_REFUSAL_INJECTED: AtomicBool = AtomicBool::new(false);

fn unmap_for_cleanup(space: &mut AddressSpace, at: u64, len: u64) -> Result<Unmapped, MapError> {
    #[cfg(feature = "selftest")]
    if matches!(
        CLEANUP_UNMAPS_BEFORE_REFUSAL.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |remaining| match remaining {
                CLEANUP_REFUSAL_DISABLED => None,
                0 => Some(CLEANUP_REFUSAL_DISABLED),
                count => Some(count - 1),
            },
        ),
        Ok(0)
    ) {
        CLEANUP_REFUSAL_INJECTED.store(true, Ordering::Release);
        return Err(MapError::Pinned(at));
    }
    space.unmap(at, len)
}

fn unmap_regions(
    space: &mut AddressSpace,
    regions: impl IntoIterator<Item = (u64, u64)>,
) -> Result<usize, UnmapRefusal> {
    let mut pages = 0;
    for (at, len) in regions {
        let returned =
            unmap_for_cleanup(space, at, len).map_err(|reason| UnmapRefusal { pages, reason })?;
        pages = pages
            .checked_add(returned.pages() as usize)
            .unwrap_or_else(|| panic!("mapping cleanup page accounting overflowed"));
        drop(returned);
    }
    Ok(pages)
}

/// Refuse cleanup after `successes` owned regions have been unmapped.
#[cfg(feature = "selftest")]
pub(crate) fn refuse_cleanup_unmap_after_for_test(successes: usize) {
    CLEANUP_UNMAPS_BEFORE_REFUSAL.store(successes, Ordering::Release);
}

/// Clear an unused cleanup refusal after a test setup failed early.
#[cfg(feature = "selftest")]
pub(crate) fn restore_cleanup_unmap_for_test() {
    CLEANUP_UNMAPS_BEFORE_REFUSAL.store(CLEANUP_REFUSAL_DISABLED, Ordering::Release);
    CLEANUP_REFUSAL_INJECTED.store(false, Ordering::Release);
}

fn undo(space: &mut AddressSpace, done: &[Option<Placement>]) -> Result<usize, UnmapRefusal> {
    unmap_regions(
        space,
        done.iter()
            .flatten()
            .map(|placement| (placement.at, placement.mapped_end - placement.at)),
    )
}

/// Whether two validated placements want any page in common.
fn overlaps(a: &Placement, b: &Placement) -> bool {
    a.at < b.mapped_end && b.at < a.mapped_end
}

/// The most segments this loader will place.
///
/// A bound rather than a heap allocation, because the validation pass has to
/// hold every placement before the first page is taken, and an image with more
/// than this is refused rather than partly loaded (D-105).
const MAX_SEGMENTS: usize = 8;

/// Where one validated segment goes, and what it is allowed to be.
#[derive(Clone, Copy)]
struct Placement {
    at: u64,
    end: u64,
    mapped_end: u64,
    pages: usize,
    from: usize,
    len: usize,
    flags: Flags,
}

/// Load `bytes`, which need not be the initrd's.
///
/// Split out so the checks can hand it a doctored image: the refusals below
/// are the load-bearing half of a loader, and a refusal that is never
/// exercised is a claim (D-100).
///
/// Preflight every placement before mapping so a later refusal cannot abandon
/// earlier user mappings and their frames (D-105).
pub(crate) fn load_image(
    space: &mut AddressSpace,
    bytes: &[u8],
    shift: u64,
) -> Result<LoadedImage, LoadError> {
    let elf = Elf::parse(bytes).map_err(LoadError::NotAnElf)?;

    let mut placements = [None; MAX_SEGMENTS];
    let mut loaded = 0usize;
    for segment in elf
        .segments()
        .filter(|segment| segment.kind() == SegmentKind::Load)
    {
        if !segment.contents_fit(bytes.len()) {
            return Err(LoadError::Truncated(segment.address()));
        }
        if segment.is_oversized() {
            return Err(LoadError::Malformed(segment.address()));
        }
        if segment.is_write_execute() {
            return Err(LoadError::WriteExecute(segment.address()));
        }
        if !segment.has_valid_alignment() {
            return Err(LoadError::InvalidAlignment {
                at: segment.address(),
                alignment: segment.alignment(),
            });
        }

        // Checked: a `p_vaddr` near the top wraps past the refusal below, and
        // a debug build would panic on bytes a program supplied (D-101).
        let at = segment
            .address()
            .checked_add(shift)
            .ok_or(LoadError::NotUserMemory(segment.address()))?;
        if !at.is_multiple_of(PAGE_SIZE) {
            return Err(LoadError::Unaligned(at));
        }
        let end = at
            .checked_add(segment.memory_size())
            .ok_or(LoadError::NotUserMemory(at))?;
        let mapped_len = segment
            .memory_size()
            .div_ceil(PAGE_SIZE)
            .checked_mul(PAGE_SIZE)
            .ok_or(LoadError::NotUserMemory(at))?;
        let mapped_end = at
            .checked_add(mapped_len)
            .ok_or(LoadError::NotUserMemory(at))?;
        if !x86_paging::user_mapping_fits(at, mapped_len) || segment.memory_size() == 0 {
            return Err(LoadError::NotUserMemory(at));
        }

        let mut flags = Flags::USER;
        if segment.is_writable() {
            flags = flags.union(Flags::WRITABLE);
        }
        if !segment.is_executable() {
            flags = flags.union(Flags::NO_EXECUTE);
        }
        let placement = Placement {
            at,
            end,
            mapped_end,
            pages: (mapped_len / PAGE_SIZE) as usize,
            from: segment.offset() as usize,
            len: segment.file_size() as usize,
            flags,
        };

        // Two segments sharing one page would collide in `map_owned` after
        // the first is mapped; caught here, before anything is taken.
        if placements
            .iter()
            .flatten()
            .any(|other: &Placement| overlaps(other, &placement))
        {
            return Err(LoadError::Overlapping(at));
        }

        *placements
            .get_mut(loaded)
            .ok_or(LoadError::TooManySegments)? = Some(placement);
        loaded += 1;
    }

    if loaded == 0 {
        return Err(LoadError::Empty);
    }

    let entry = elf
        .entry()
        .checked_add(shift)
        .ok_or(LoadError::NotUserMemory(elf.entry()))?;
    let entry_is_executable = placements.iter().flatten().any(|placement| {
        !placement.flags.contains(Flags::NO_EXECUTE)
            && entry >= placement.at
            && entry < placement.end
    });
    if !entry_is_executable {
        return Err(LoadError::InvalidEntry(entry));
    }

    for (done, placement) in placements.iter().flatten().enumerate() {
        match place(space, bytes, placement) {
            Ok(()) => {}
            Err(error) => {
                // Validation cannot rule out a frame shortage or a collision,
                // so the pass walks back exactly what it mapped (D-94).
                if let Err(refusal) = undo(space, &placements[..done]) {
                    return Err(LoadError::Rollback {
                        reason: refusal.reason,
                        pages: refusal.pages,
                    });
                }
                return Err(error);
            }
        }
    }

    let mut regions = [None; MAX_SEGMENTS];
    for (slot, placement) in placements.iter().flatten().enumerate() {
        regions[slot] = Some((placement.at, placement.mapped_end - placement.at));
    }
    Ok(LoadedImage { entry, regions })
}

/// What a thread needs to find when it wakes up in a space of its own.
///
/// Arranged by [`spawn`] while the slot is still `Reserved`, so it is there
/// before the thread's first instruction rather than racing it.
#[derive(Clone, Copy)]
struct Launch {
    entry: u64,
    stack: u64,
    argument: u64,
    bootstrap_handle: u32,
    which: u64,
    root: Paddr,
}

/// Version of the words placed at a program's initial stack pointer.
pub(crate) const ENTRY_VERSION: u64 = 1;

/// Bytes in the versioned entry block.
pub(crate) const ENTRY_BYTES: usize = 24;

/// Byte offsets in the versioned entry block.
mod entry {
    /// The entry ABI version.
    pub(super) const VERSION: usize = 0;
    /// The launcher's operation-specific argument.
    pub(super) const ARGUMENT: usize = 8;
    /// The numeric identity of the sole bootstrap handle.
    pub(super) const BOOTSTRAP_HANDLE: usize = 16;
}

/// Stack tail occupied so the initial pointer remains sixteen-byte aligned.
const ENTRY_STACK_BYTES: u64 = (ENTRY_BYTES as u64 + 15) & !15;

/// One per thread slot. A leaf: nothing is called while it is held.
static LAUNCHES: SpinLock<[Option<Launch>; thread::CAPACITY]> =
    SpinLock::new([None; thread::CAPACITY]);

fn remember(
    regions: &mut [(u64, u64); MAX_REGIONS],
    used: &mut usize,
    at: u64,
    len: u64,
) -> Result<(), SpawnError> {
    let Some(slot) = regions.get_mut(*used) else {
        return Err(SpawnError::TooManyMappings);
    };
    *slot = (at, len);
    *used += 1;
    Ok(())
}

const MAX_REGIONS: usize = MAX_SEGMENTS + 1;

/// Whether every maximum-sized image region and its stack fit exactly once.
#[cfg(feature = "selftest")]
pub(crate) fn maximum_cleanup_ledger_is_total_for_test() -> bool {
    let mut regions = [(0, 0); MAX_REGIONS];
    let mut used = 0;
    for index in 0..MAX_SEGMENTS {
        if remember(
            &mut regions,
            &mut used,
            0x1000 + index as u64 * 0x2000,
            PAGE_SIZE,
        )
        .is_err()
        {
            return false;
        }
    }
    let stack = 0x9000_0000;
    remember(&mut regions, &mut used, stack, USER_STACK_BYTES).is_ok()
        && used == MAX_REGIONS
        && regions.get(MAX_SEGMENTS) == Some(&(stack, USER_STACK_BYTES))
        && matches!(
            remember(&mut regions, &mut used, stack + PAGE_SIZE, PAGE_SIZE),
            Err(SpawnError::TooManyMappings)
        )
}

struct Bootstrap {
    /// Its tables, kept so they can be given back. `bind` borrows rather than
    /// consumes for exactly this reason (D-117).
    space: AddressSpace,
    /// The task table's one-shot authority to release these tables.
    authority: Option<BootstrapAuthority>,
    /// What was mapped into them, so the same is unmapped.
    regions: [(u64, u64); MAX_REGIONS],
    mapped: usize,
    returned_pages: usize,
    abandoned: usize,
    /// Its handle table, behind an `Arc` for the reason [`Bootstrap::ring`] is:
    /// `kind_of` takes the global arena's lock, and taking it under this
    /// table's leaf guard is what `lock_leaf` forbids (D-121, D-124).
    table: alloc::sync::Arc<HandleTable>,
    /// Its ring. Held here because the shared pages are the kernel's and must
    /// outlive the mapping that shows them to the program (D-113).
    ///
    /// Behind an `Arc` so serving can release `BOOTSTRAPS`' guard before
    /// reading user memory, taking other locks and waking threads (D-121,
    /// D-124).
    ring: alloc::sync::Arc<queue::Queue>,
    /// The birth handle retained for lifecycle checks.
    #[cfg(feature = "selftest")]
    observed_birth_handle: u32,
}

enum BootstrapAuthority {
    Bound(task::Bound),
    Released(task::Released),
}

/// A process that is not yet observable and therefore must be undone on drop.
struct ProcessConstruction {
    space: Option<AddressSpace>,
    authority: thread::SpaceAuthority,
    regions: [(u64, u64); MAX_REGIONS],
    mapped: usize,
}

impl ProcessConstruction {
    fn new(space: AddressSpace) -> Self {
        Self {
            space: Some(space),
            authority: thread::SpaceAuthority::None,
            regions: [(0, 0); MAX_REGIONS],
            mapped: 0,
        }
    }

    fn space(&mut self) -> Result<&mut AddressSpace, SpawnError> {
        self.space.as_mut().ok_or(SpawnError::Incomplete)
    }

    fn remember(&mut self, at: u64, len: u64) -> Result<(), SpawnError> {
        remember(&mut self.regions, &mut self.mapped, at, len)
    }

    fn retain_authority(&mut self, authority: thread::SpaceAuthority) {
        self.authority = authority;
    }
}

impl Drop for ProcessConstruction {
    fn drop(&mut self) {
        let released = match core::mem::replace(&mut self.authority, thread::SpaceAuthority::None) {
            thread::SpaceAuthority::None => None,
            thread::SpaceAuthority::Released(released) => Some(released),
        };
        let Some(space) = self.space.as_mut() else {
            return;
        };

        #[cfg(feature = "selftest")]
        let mut observed_returned_pages = 0usize;
        while self.mapped != 0 {
            let index = self.mapped - 1;
            let (at, len) = self.regions[index];
            match unmap_for_cleanup(space, at, len) {
                Ok(returned) => {
                    #[cfg(feature = "selftest")]
                    {
                        observed_returned_pages = observed_returned_pages
                            .checked_add(returned.pages() as usize)
                            .unwrap_or_else(|| panic!("failed-spawn page accounting overflowed"));
                    }
                    drop(returned);
                    self.mapped = index;
                }
                Err(reason) => {
                    #[cfg(feature = "selftest")]
                    if CLEANUP_REFUSAL_INJECTED.swap(false, Ordering::AcqRel) {
                        REFUSED_MAPPING_RECLAIMS.fetch_add(1, Ordering::AcqRel);
                        continue;
                    }
                    panic!("failed-spawn mapping cleanup refused: {reason}");
                }
            }
        }

        match space.reclaim(released) {
            #[cfg(feature = "selftest")]
            Ok(tables) => {
                ROLLED_BACK_PAGES.fetch_add(observed_returned_pages, Ordering::AcqRel);
                ROLLED_BACK_TABLES.fetch_add(tables, Ordering::AcqRel);
            }
            #[cfg(not(feature = "selftest"))]
            Ok(_) => {}
            Err(reason) => {
                panic!("failed-spawn table cleanup refused: {reason}");
            }
        }
    }
}

struct PreparedProcess {
    construction: ProcessConstruction,
    table: alloc::sync::Arc<HandleTable>,
    ring: alloc::sync::Arc<queue::Queue>,
    #[cfg(feature = "selftest")]
    observed_birth_handle: u32,
    launch: Launch,
}

struct CommitOutcome {
    retained: Option<(PreparedProcess, task::Bound)>,
}

impl CommitOutcome {
    const fn committed() -> Self {
        Self { retained: None }
    }

    fn refused(process: PreparedProcess, bound: task::Bound) -> Self {
        Self {
            retained: Some((process, bound)),
        }
    }

    fn into_retained(self) -> Option<(PreparedProcess, task::Bound)> {
        self.retained
    }
}

impl PreparedProcess {
    fn commit(
        mut self,
        slot: usize,
        bound: task::Bound,
        #[cfg(feature = "selftest")] refuse: bool,
    ) -> CommitOutcome {
        #[cfg(feature = "selftest")]
        if refuse {
            return CommitOutcome::refused(self, bound);
        }
        if slot_has_zombie(slot) {
            return CommitOutcome::refused(self, bound);
        }
        assert!(
            LAUNCHES.lock_leaf().get(slot).is_some_and(Option::is_none),
            "a reserved process slot retained launch state"
        );
        assert!(
            BOOTSTRAPS
                .lock_leaf()
                .get(slot)
                .is_some_and(Option::is_none),
            "a reserved process slot retained a live owner"
        );
        let Some(space) = self.construction.space.take() else {
            return CommitOutcome::refused(self, bound);
        };
        LAUNCHES.lock_leaf()[slot] = Some(self.launch);
        BOOTSTRAPS.lock_leaf()[slot] = Some(Bootstrap {
            space,
            authority: Some(BootstrapAuthority::Bound(bound)),
            regions: self.construction.regions,
            mapped: self.construction.mapped,
            returned_pages: 0,
            abandoned: 0,
            table: self.table,
            ring: self.ring,
            #[cfg(feature = "selftest")]
            observed_birth_handle: self.observed_birth_handle,
        });
        CommitOutcome::committed()
    }

    #[cfg(feature = "selftest")]
    fn record_birth(&self, slot: usize) -> bool {
        let Some((_, ring_flags)) = self
            .construction
            .space
            .as_ref()
            .and_then(|space| space.lookup(RING))
        else {
            return false;
        };
        let Ok((bootstrap, _)) = self.table.resolve(self.observed_birth_handle) else {
            return false;
        };
        let birth = Birth {
            table: self.table.identity_for_test(),
            handles: self.table.live(),
            bootstrap: bootstrap.kind(),
            ring: self.ring.shape(),
            ring_flags,
        };
        let mut births = BIRTHS.lock_leaf();
        let Some(room) = births.get_mut(slot) else {
            return false;
        };
        *room = Some(birth);
        true
    }
}

/// One per thread slot, alongside its [`Launch`].
static BOOTSTRAPS: SpinLock<[Option<Bootstrap>; thread::CAPACITY]> =
    SpinLock::new([const { None }; thread::CAPACITY]);

/// Handles a process may hold at once.
///
/// A quota, and the answer to kernel-memory exhaustion at the handle layer
/// (D-78). Small enough to bound kernel memory and large enough for delegation.
const HANDLE_QUOTA: usize = 64;

/// Identity of a spawned program.
pub(crate) struct Spawned {
    /// The thread slot it runs in.
    pub(crate) slot: usize,
    /// The exact authority-table incarnation recorded at birth.
    #[cfg(feature = "selftest")]
    pub(crate) observed_table_incarnation: u64,
}

/// What process construction committed before the first user instruction.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy)]
pub(crate) struct Birth {
    /// The non-repeating identity of the process's authority table.
    pub(crate) table: u64,
    /// Handles in that table at commit.
    pub(crate) handles: usize,
    /// What its bootstrap entry named.
    pub(crate) bootstrap: ObjectType,
    /// The kernel-owned shape of its shared ring.
    pub(crate) ring: (u32, u32, u32),
    /// Flags on the committed ring mapping.
    pub(crate) ring_flags: Flags,
}

#[cfg(feature = "selftest")]
static BIRTHS: SpinLock<[Option<Birth>; thread::CAPACITY]> =
    SpinLock::new([const { None }; thread::CAPACITY]);

/// Immutable birth evidence for this exact process incarnation.
#[cfg(feature = "selftest")]
pub(crate) fn birth_of(spawned: &Spawned) -> Option<Birth> {
    BIRTHS
        .lock_leaf()
        .get(spawned.slot)
        .copied()
        .flatten()
        .filter(|birth| birth.table == spawned.observed_table_incarnation)
}

/// A program's ring and authority table, for lifecycle checks.
#[cfg(feature = "selftest")]
pub(crate) fn authority_for_test(
    slot: usize,
) -> Option<(
    alloc::sync::Arc<queue::Queue>,
    alloc::sync::Arc<HandleTable>,
)> {
    authority_of(slot)
}

/// A completed construction step after which a check may force refusal.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpawnEdge {
    /// The private address-space root exists.
    Space,
    /// Every image segment is mapped and recorded.
    Image,
    /// The user stack is mapped and recorded.
    Stack,
    /// The shared time page is visible.
    TimePage,
    /// The ring and its frames exist.
    Ring,
    /// The ring is mapped into the new space.
    RingMapping,
    /// The opaque bootstrap identity exists.
    Channel,
    /// The handle table has committed its quota.
    Table,
    /// The bootstrap identity is installed as the sole handle.
    Handle,
    /// The new space passed binding validation.
    Binding,
    /// A scheduler slot is reserved.
    ThreadReserved,
    /// A task record and kernel stack exist.
    ThreadFabricated,
    /// The task record names the new address space.
    ThreadBound,
    /// The reserved thread refuses its launch records before becoming runnable.
    ThreadArrangement,
}

#[cfg(feature = "selftest")]
impl SpawnEdge {
    /// Every fallible construction boundary in order.
    pub(crate) const ALL: [Self; 14] = [
        Self::Space,
        Self::Image,
        Self::Stack,
        Self::TimePage,
        Self::Ring,
        Self::RingMapping,
        Self::Channel,
        Self::Table,
        Self::Handle,
        Self::Binding,
        Self::ThreadReserved,
        Self::ThreadFabricated,
        Self::ThreadBound,
        Self::ThreadArrangement,
    ];
}

#[cfg(feature = "selftest")]
#[derive(Clone, Copy)]
enum SpawnFault {
    Never,
    At(SpawnEdge),
}

#[cfg(feature = "selftest")]
impl SpawnFault {
    const fn refuses(self, edge: SpawnEdge) -> bool {
        match self {
            Self::Never => false,
            Self::At(wanted) => wanted as u8 == edge as u8,
        }
    }

    const fn thread_point(self) -> Option<thread::SpawnPoint> {
        match self {
            Self::At(SpawnEdge::ThreadReserved) => Some(thread::SpawnPoint::Reserved),
            Self::At(SpawnEdge::ThreadFabricated) => Some(thread::SpawnPoint::Fabricated),
            Self::At(SpawnEdge::ThreadBound) => Some(thread::SpawnPoint::Bound),
            Self::Never | Self::At(_) => None,
        }
    }
}

/// Why a program could not be started.
#[derive(Debug)]
pub(crate) enum SpawnError {
    /// Boot did not admit maskable interrupts.
    NoInterruptAdmission,
    /// Shared kernel mappings have no accepted RDCL isolation evidence.
    NoSharedKernelIsolation,
    /// The CPU is not enforcing non-executable mappings.
    NoExecuteProtection,
    /// Thread switches cannot isolate architectural extended state.
    NoExtendedState,
    /// No calibrated clock can preempt or complete a timer.
    NoClock,
    /// No kernel service thread can reclaim the process after exit.
    NoServiceWorker,
    /// The kernel itself is still running on an unguarded boot stack.
    UnguardedBootStack,
    /// No address space could be built for it.
    NoSpace(MapError),
    /// The image would not load.
    Load(LoadError),
    /// No memory for its stack.
    NoStack,
    /// The space could not be bound to a thread.
    NotBindable(ActivateError),
    /// No thread could be created.
    NoThread(thread::SpawnError),
    /// An unfinished transaction lost the address space it owns.
    Incomplete,
    /// The fixed cleanup ledger cannot retain another owned mapping.
    TooManyMappings,
    /// Its bootstrap channel could not be put in its table.
    NoBootstrap(HandleError),
    /// The time page could not be shown to it.
    NoTimePage(crate::timepage::TimePageError),
    /// No memory for its ring.
    NoRing,
    /// A self-check stopped construction at one exact edge.
    #[cfg(feature = "selftest")]
    Injected(SpawnEdge),
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoInterruptAdmission => core::fmt::Write::write_str(
                f,
                "maskable interrupts are not admitted; ring 3 is closed",
            ),
            Self::NoSharedKernelIsolation => core::fmt::Write::write_str(
                f,
                "shared kernel mappings have no isolation proof; ring 3 is closed",
            ),
            Self::NoExecuteProtection => {
                core::fmt::Write::write_str(f, "no execute protection; ring 3 is closed")
            }
            Self::NoExtendedState => {
                core::fmt::Write::write_str(f, "no isolated extended state; ring 3 is closed")
            }
            Self::NoClock => {
                core::fmt::Write::write_str(f, "no calibrated clock; ring 3 is closed")
            }
            Self::NoServiceWorker => {
                core::fmt::Write::write_str(f, "no service worker; ring 3 is closed")
            }
            Self::UnguardedBootStack => core::fmt::Write::write_str(
                f,
                "the boot thread has no guarded stack; ring 3 is closed",
            ),
            Self::NoSpace(error) => write!(f, "no address space: {error}"),
            Self::Load(error) => write!(f, "{error}"),
            Self::NoStack => core::fmt::Write::write_str(f, "no memory for a stack"),
            Self::NotBindable(error) => write!(f, "the space cannot be installed: {error}"),
            Self::NoThread(error) => write!(f, "no thread: {error}"),
            Self::Incomplete => {
                core::fmt::Write::write_str(f, "the process construction is incomplete")
            }
            Self::TooManyMappings => {
                core::fmt::Write::write_str(f, "the process has too many owned mappings")
            }
            Self::NoBootstrap(error) => write!(f, "no bootstrap channel: {error}"),
            Self::NoTimePage(error) => write!(f, "the time page: {error}"),
            Self::NoRing => core::fmt::Write::write_str(f, "no memory for a ring"),
            #[cfg(feature = "selftest")]
            Self::Injected(edge) => write!(f, "construction was stopped after {edge:?}"),
        }
    }
}

#[cfg(feature = "selftest")]
fn stop_after(fault: SpawnFault, edge: SpawnEdge) -> Result<(), SpawnError> {
    if fault.refuses(edge) {
        #[cfg(feature = "selftest")]
        return Err(SpawnError::Injected(edge));
    }
    Ok(())
}

/// Start a program in an address space of its own.
///
/// The order is forced and each step is why the next one is possible: a space
/// that shares only the kernel's higher half (D-104), the image loaded into
/// *that* space rather than into the running one, a stack, the binding that
/// proves the space is installable, and a thread whose tables are set before
/// anything can schedule it (D-106).
///
/// Until the reserved thread is made runnable, [`ProcessConstruction`] owns
/// every mapped region and the tables that reach it. Any refusal drops that
/// transaction, retires a partially fabricated task if necessary and returns
/// every resource before the error escapes (D-191).
///
/// # Errors
///
/// [`SpawnError`], naming which of those steps refused.
pub(crate) fn spawn(name: &'static str, which: u64, argument: u64) -> Result<Spawned, SpawnError> {
    #[cfg(feature = "selftest")]
    return spawn_transaction(name, which, argument, SpawnFault::Never);
    #[cfg(not(feature = "selftest"))]
    spawn_transaction(name, which, argument)
}

fn spawn_transaction(
    name: &'static str,
    which: u64,
    argument: u64,
    #[cfg(feature = "selftest")] fault: SpawnFault,
) -> Result<Spawned, SpawnError> {
    if !crate::maskable_interrupts_are_ready() {
        return Err(SpawnError::NoInterruptAdmission);
    }
    if !crate::user_launch_is_safe() {
        return Err(SpawnError::UnguardedBootStack);
    }
    if !crate::shared_kernel_mappings_are_safe() {
        return Err(SpawnError::NoSharedKernelIsolation);
    }
    if !no_execute_is_enabled() {
        return Err(SpawnError::NoExecuteProtection);
    }
    if !extended_state_is_isolated() {
        return Err(SpawnError::NoExtendedState);
    }
    if !crate::time::is_calibrated() {
        return Err(SpawnError::NoClock);
    }
    if !crate::queue::worker_is_ready() {
        return Err(SpawnError::NoServiceWorker);
    }
    let space = AddressSpace::for_user().map_err(SpawnError::NoSpace)?;
    let mut construction = ProcessConstruction::new(space);
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Space)?;

    let image = load(construction.space()?, which * APART).map_err(SpawnError::Load)?;
    for (at, len) in image.regions() {
        construction.remember(at, len)?;
    }
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Image)?;

    let stack = place_stack(construction.space()?, which).ok_or(SpawnError::NoStack)?;
    construction.remember(stack, USER_STACK_BYTES)?;
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Stack)?;

    // Time before the ring: a process is refused rather than launched with a
    // hole in its ABI (D-166). Ring pages stay the ring's, not the ledger's (D-113).
    crate::timepage::install(construction.space()?).map_err(SpawnError::NoTimePage)?;
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::TimePage)?;

    let mut ring = queue::Queue::new().ok_or(SpawnError::NoRing)?;
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Ring)?;
    alloc::sync::Arc::get_mut(&mut ring)
        .ok_or(SpawnError::Incomplete)?
        .install(construction.space()?, RING)
        .map_err(SpawnError::NoSpace)?;
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::RingMapping)?;

    let root = construction.space()?.root();

    // Create the bootstrap identity before the thread can observe it.
    let bootstrap = object::bootstrap_identity();
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Channel)?;
    let table = HandleTable::new(HANDLE_QUOTA).map_err(SpawnError::NoBootstrap)?;
    #[cfg(feature = "selftest")]
    let table_identity = table.identity_for_test();
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Table)?;
    let handle = table
        .insert(bootstrap, Rights::UNIVERSAL)
        .map_err(SpawnError::NoBootstrap)?;
    #[cfg(feature = "selftest")]
    stop_after(fault, SpawnEdge::Handle)?;

    let binding = construction
        .space()?
        .bind()
        .map_err(SpawnError::NotBindable)?;
    #[cfg(feature = "selftest")]
    if let Err(error) = stop_after(fault, SpawnEdge::Binding) {
        construction.retain_authority(thread::SpaceAuthority::Released(task::release_uninstalled(
            binding,
        )));
        return Err(error);
    }

    let mut prepared = Some(PreparedProcess {
        construction,
        table: alloc::sync::Arc::new(table),
        #[cfg(feature = "selftest")]
        observed_birth_handle: handle,
        ring,
        launch: Launch {
            entry: image.entry(),
            stack,
            argument,
            bootstrap_handle: handle,
            which,
            root,
        },
    });
    let arrange = |slot, bound| {
        if slot >= thread::CAPACITY {
            return Err(bound);
        }
        if prepared.is_none() {
            return Err(bound);
        }
        #[cfg(feature = "selftest")]
        if !prepared
            .as_ref()
            .is_some_and(|process| process.record_birth(slot))
        {
            return Err(bound);
        }
        let Some(process) = prepared.take() else {
            return Err(bound);
        };
        match process
            .commit(
                slot,
                bound,
                #[cfg(feature = "selftest")]
                fault.refuses(SpawnEdge::ThreadArrangement),
            )
            .into_retained()
        {
            None => Ok(()),
            Some((process, bound)) => {
                prepared = Some(process);
                Err(bound)
            }
        }
    };
    #[cfg(feature = "selftest")]
    let spawned = match fault.thread_point() {
        Some(point) => thread::spawn_in_space_refusing(name, cross, binding, point, arrange),
        None => thread::spawn_in_space(name, cross, binding, arrange),
    };
    #[cfg(not(feature = "selftest"))]
    let spawned = thread::spawn_in_space(name, cross, binding, arrange);
    let slot = match spawned {
        Ok(slot) => slot,
        Err(failure) => {
            #[cfg(feature = "selftest")]
            if matches!(failure.reason, thread::SpawnError::Retirement { .. }) {
                REFUSED_TASK_RETIREMENTS.fetch_add(1, Ordering::AcqRel);
            }
            if let Some(process) = prepared.as_mut() {
                process.construction.retain_authority(failure.authority);
            }
            return Err(SpawnError::NoThread(failure.reason));
        }
    };

    Ok(Spawned {
        slot,
        #[cfg(feature = "selftest")]
        observed_table_incarnation: table_identity,
    })
}

/// Whether the CPU will enforce every non-executable user mapping.
fn no_execute_is_enabled() -> bool {
    let enabled = cpu::Protections::enabled().nx;
    #[cfg(feature = "selftest")]
    let enabled = enabled && !REFUSE_NO_EXECUTE.load(Ordering::Acquire);
    enabled
}

fn extended_state_is_isolated() -> bool {
    xstate::user_state_is_isolated()
}

#[cfg(feature = "selftest")]
static REFUSE_NO_EXECUTE: AtomicBool = AtomicBool::new(false);

/// Run a check as if the protection read-back had omitted NX.
#[cfg(feature = "selftest")]
pub(crate) fn without_no_execute_for_test<T>(check: impl FnOnce() -> T) -> T {
    let refused = REFUSE_NO_EXECUTE.swap(true, Ordering::AcqRel);
    let result = check();
    REFUSE_NO_EXECUTE.store(refused, Ordering::Release);
    result
}

/// Run a check as if thread switches could not isolate extended state.
#[cfg(feature = "selftest")]
pub(crate) fn without_extended_state_for_test<T>(check: impl FnOnce() -> T) -> T {
    xstate::without_user_state_isolation_for_test(check)
}

/// Stop one never-run construction after `edge`.
#[cfg(feature = "selftest")]
pub(crate) fn spawn_refusing(edge: SpawnEdge) -> Result<(), SpawnError> {
    spawn_transaction("refused-user", 8, BEHAVE, SpawnFault::At(edge)).map(drop)
}

enum ReclaimRefusal {
    TaskRelease,
    Mapping(MapError),
    Tables(ReclaimError),
}

struct ReclaimRetention {
    refusal: ReclaimRefusal,
    #[cfg(feature = "selftest")]
    observed_returned_pages: usize,
}

struct ReclaimSuccess {
    reclaimed: Reclaimed,
    #[cfg(feature = "selftest")]
    observed_returned_pages: usize,
}

fn reclaim(
    bootstrap: &mut Bootstrap,
    abandoned: usize,
) -> Result<ReclaimSuccess, ReclaimRetention> {
    bootstrap.abandoned = bootstrap
        .abandoned
        .checked_add(abandoned)
        .unwrap_or_else(|| panic!("zombie abandonment accounting overflowed"));
    let Some(authority) = bootstrap.authority.take() else {
        panic!("a zombie always owns address-space authority");
    };
    bootstrap.authority = match authority {
        BootstrapAuthority::Released(released) => Some(BootstrapAuthority::Released(released)),
        BootstrapAuthority::Bound(bound) => match task::release_address_space(bound) {
            Ok(released) => Some(BootstrapAuthority::Released(released)),
            Err(bound) => {
                bootstrap.authority = Some(BootstrapAuthority::Bound(bound));
                return Err(ReclaimRetention {
                    refusal: ReclaimRefusal::TaskRelease,
                    #[cfg(feature = "selftest")]
                    observed_returned_pages: 0,
                });
            }
        },
    };

    #[cfg(feature = "selftest")]
    let mut observed_returned_pages = 0usize;
    while bootstrap.mapped != 0 {
        let index = bootstrap.mapped - 1;
        let (at, len) = bootstrap.regions[index];
        match bootstrap.space.unmap(at, len) {
            Ok(returned) => {
                let pages = returned.pages() as usize;
                drop(returned);
                bootstrap.mapped = index;
                bootstrap.returned_pages = bootstrap
                    .returned_pages
                    .checked_add(pages)
                    .unwrap_or_else(|| panic!("zombie returned-page accounting overflowed"));
                #[cfg(feature = "selftest")]
                {
                    observed_returned_pages = observed_returned_pages
                        .checked_add(pages)
                        .unwrap_or_else(|| {
                            panic!("one reclaim's returned-page accounting overflowed")
                        });
                }
            }
            Err(reason) => {
                return Err(ReclaimRetention {
                    refusal: ReclaimRefusal::Mapping(reason),
                    #[cfg(feature = "selftest")]
                    observed_returned_pages,
                });
            }
        }
    }

    let Some(BootstrapAuthority::Released(released)) = bootstrap.authority.take() else {
        unreachable!("task release was established above");
    };
    let tables = match bootstrap.space.reclaim(Some(released)) {
        Ok(tables) => tables,
        Err(failure) => {
            let reason = failure.reason;
            let Some(released) = failure.released else {
                panic!("zombie cleanup lost its retry authority: {reason}");
            };
            bootstrap.authority = Some(BootstrapAuthority::Released(released));
            return Err(ReclaimRetention {
                refusal: ReclaimRefusal::Tables(reason),
                #[cfg(feature = "selftest")]
                observed_returned_pages,
            });
        }
    };

    let reclaimed = Reclaimed {
        pages: bootstrap.returned_pages,
        tables,
        abandoned: bootstrap.abandoned,
    };
    Ok(ReclaimSuccess {
        reclaimed,
        #[cfg(feature = "selftest")]
        observed_returned_pages,
    })
}

/// What one reclaim gave back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Reclaimed {
    /// Frames the program's own mappings held.
    pub(crate) pages: usize,
    /// Frames its page tables were built from.
    pub(crate) tables: usize,
    /// Operations that were still in flight and were ended.
    pub(crate) abandoned: usize,
}

/// What one bounded collection attempt observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollectOutcome {
    /// No zombie owner was waiting.
    Empty,
    /// A zombie remains owned and needs a later retry.
    Retained,
    /// One zombie was reclaimed completely.
    Reclaimed(Reclaimed),
}

impl CollectOutcome {
    pub(crate) const fn is_some(self) -> bool {
        matches!(self, Self::Reclaimed(_))
    }

    #[cfg(feature = "selftest")]
    pub(crate) fn is_some_and(self, check: impl FnOnce(Reclaimed) -> bool) -> bool {
        match self {
            Self::Reclaimed(reclaimed) => check(reclaimed),
            Self::Empty | Self::Retained => false,
        }
    }
}

struct ZombieTable {
    owners: [Option<Bootstrap>; thread::CAPACITY],
    settling: [bool; thread::CAPACITY],
    collecting: [bool; thread::CAPACITY],
}

impl ZombieTable {
    const fn new() -> Self {
        Self {
            owners: [const { None }; thread::CAPACITY],
            settling: [false; thread::CAPACITY],
            collecting: [false; thread::CAPACITY],
        }
    }
}

/// Programs whose threads are gone and whose resources are not.
static ZOMBIES: SpinLock<ZombieTable> = SpinLock::new(ZombieTable::new());

/// What every reclaim so far gave back.
#[cfg(feature = "selftest")]
static RECLAIMED_PAGES: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static RECLAIMED_TABLES: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static REFUSED_MAPPING_RECLAIMS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static REFUSED_TABLE_RECLAIMS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static REFUSED_RING_RECLAIMS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static REFUSED_TASK_RETIREMENTS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static ROLLED_BACK_PAGES: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static ROLLED_BACK_TABLES: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static PAUSE_COLLECTION_AFTER_TAKE: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "selftest")]
static COLLECTION_IS_PAUSED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "selftest")]
static PAUSE_COLLECTION_BEFORE_SETTLEMENT: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "selftest")]
static COLLECTION_IS_SETTLING: AtomicBool = AtomicBool::new(false);

/// What a program held when it died, for machine checks.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy)]
pub(crate) struct LastRites {
    /// The non-repeating table identity of the program that left this record.
    pub(crate) table: u64,
    /// Handles in its table at the end.
    pub(crate) handles: usize,
    /// What its one bootstrap handle named.
    pub(crate) bootstrap: Option<ObjectType>,
    /// Completions its ring posted.
    pub(crate) served: usize,
    /// The first submission it wrote, and the completion that answered it.
    pub(crate) submitted: queue::Submission,
    pub(crate) completed: queue::Completion,
}

#[cfg(feature = "selftest")]
static RITES: SpinLock<[Option<LastRites>; thread::CAPACITY]> =
    SpinLock::new([const { None }; thread::CAPACITY]);

/// What the program that last held `slot` left behind.
#[cfg(feature = "selftest")]
pub(crate) fn last_rites_of(slot: usize) -> Option<LastRites> {
    RITES.lock_leaf().get(slot).copied().flatten()
}

/// Mark `slot`'s program dead, without freeing anything.
///
/// Called from trap context on the retiring thread itself, so it must not
/// allocate, block, or touch the space it is running on. Moving one `Option`
/// between two arrays does none of those.
pub(crate) fn orphan(slot: usize) {
    let taken = BOOTSTRAPS.lock_leaf().get_mut(slot).and_then(Option::take);
    let stored = taken.is_some();
    if let Some(bootstrap) = taken {
        #[cfg(feature = "selftest")]
        {
            let rites = LastRites {
                table: bootstrap.table.identity_for_test(),
                handles: bootstrap.table.live(),
                bootstrap: bootstrap
                    .table
                    .kind_of(bootstrap.observed_birth_handle)
                    .ok(),
                served: bootstrap.ring.served_count(),
                submitted: bootstrap.ring.read_submission(0),
                completed: bootstrap.ring.read_completion(0),
            };
            let mut rites_table = RITES.lock_leaf();
            let Some(room) = rites_table.get_mut(slot) else {
                panic!("a retiring process slot remains in range");
            };
            *room = Some(rites);
        }

        let mut zombies = ZOMBIES.lock_leaf();
        let settling = zombies.settling.get(slot).copied().unwrap_or(false);
        let collecting = zombies.collecting.get(slot).copied().unwrap_or(false);
        let Some(room) = zombies.owners.get_mut(slot) else {
            panic!("a retiring process slot remains in range");
        };
        assert!(
            room.is_none() && !settling && !collecting,
            "a process slot cannot orphan twice"
        );
        *room = Some(bootstrap);
    }
    // Cleared, so a slot reused by a later program never finds the previous
    // one's entry point (D-117).
    let mut launches = LAUNCHES.lock_leaf();
    let Some(room) = launches.get_mut(slot) else {
        panic!("a retiring process slot remains in range");
    };
    *room = None;
    drop(launches);
    if stored {
        crate::queue::request_reap();
    }
}

/// Collect one dead program's resources, if there are any.
///
/// One per call rather than all of them: this runs where the machine had
/// nothing better to do, and a loop that frees sixteen processes with
/// interrupts on is a loop that can be preempted halfway through anyway. One
/// at a time makes that unremarkable instead of something to reason about.
///
pub(crate) fn collect() -> CollectOutcome {
    // The owner stays in the zombie table until its ring is quiet, so a
    // refused terminal completion retries rather than reusing live pages (D-192).
    let (found, another_collector) = {
        let mut zombies = ZOMBIES.lock_leaf();
        let another_collector = zombies.settling.iter().any(|owned| *owned)
            || zombies.collecting.iter().any(|owned| *owned);
        let found = (0..thread::CAPACITY).find_map(|slot| {
            if zombies.settling[slot] {
                return None;
            }
            let bootstrap = zombies.owners[slot].as_ref()?;
            Some((slot, alloc::sync::Arc::clone(&bootstrap.ring)))
        });
        if let Some((slot, _)) = found.as_ref() {
            zombies.settling[*slot] = true;
        }
        (found, another_collector)
    };
    let Some((slot, ring)) = found else {
        return if another_collector {
            CollectOutcome::Retained
        } else {
            CollectOutcome::Empty
        };
    };

    #[cfg(feature = "selftest")]
    {
        COLLECTION_IS_SETTLING.store(true, Ordering::Release);
        while PAUSE_COLLECTION_BEFORE_SETTLEMENT.load(Ordering::Acquire) {
            crate::thread::yield_now(&mut crate::thread::ThreadContext::require());
        }
        COLLECTION_IS_SETTLING.store(false, Ordering::Release);
    }

    // Before the ring's pages go anywhere: every outstanding operation gets
    // its terminal completion while the zombie still owns them (D-167, D-192).
    let abandoned = ring.abandon();
    if !ring.is_quiet() {
        let recorded = {
            let mut zombies = ZOMBIES.lock_leaf();
            let settling = zombies.settling.get(slot).copied().unwrap_or(false);
            let collecting = zombies.collecting.get(slot).copied().unwrap_or(false);
            let recorded = match zombies.owners.get_mut(slot).and_then(Option::as_mut) {
                None => 0,
                Some(bootstrap) if !alloc::sync::Arc::ptr_eq(&bootstrap.ring, &ring) => 1,
                Some(_) if !settling || collecting => 2,
                Some(bootstrap) => match bootstrap.abandoned.checked_add(abandoned) {
                    Some(total) => {
                        bootstrap.abandoned = total;
                        4
                    }
                    None => 3,
                },
            };
            if recorded == 4 {
                zombies.settling[slot] = false;
            }
            recorded
        };
        match recorded {
            4 => {}
            0 => panic!("the unsettled zombie left its slot"),
            1 => panic!("a zombie changed owners during settlement"),
            2 => panic!("a zombie lost exclusive settlement ownership"),
            3 => panic!("zombie abandonment accounting overflowed"),
            _ => unreachable!(),
        }
        warn!("user: ring cleanup refused; the zombie remains owned");
        #[cfg(feature = "selftest")]
        REFUSED_RING_RECLAIMS.fetch_add(1, Ordering::AcqRel);
        return CollectOutcome::Retained;
    }
    // Taken out under the guard and reclaimed outside it: `reclaim` frees
    // frames, which reaches the allocator, and this lock is a leaf (D-85).
    let bootstrap = {
        let mut zombies = ZOMBIES.lock_leaf();
        let settling = zombies.settling.get(slot).copied().unwrap_or(false);
        let Some(room) = zombies.owners.get_mut(slot) else {
            panic!("a selected zombie slot remains in range");
        };
        let Some(current) = room.as_ref() else {
            return if zombies.collecting[slot] {
                CollectOutcome::Retained
            } else {
                CollectOutcome::Empty
            };
        };
        if !alloc::sync::Arc::ptr_eq(&current.ring, &ring) {
            return CollectOutcome::Retained;
        }
        if !settling {
            return CollectOutcome::Retained;
        }
        let Some(bootstrap) = room.take() else {
            unreachable!("the retained zombie was checked above");
        };
        zombies.settling[slot] = false;
        zombies.collecting[slot] = true;
        bootstrap
    };
    drop(ring);
    #[cfg(feature = "selftest")]
    {
        COLLECTION_IS_PAUSED.store(true, Ordering::Release);
        while PAUSE_COLLECTION_AFTER_TAKE.load(Ordering::Acquire) {
            crate::thread::yield_now(&mut crate::thread::ThreadContext::require());
        }
        COLLECTION_IS_PAUSED.store(false, Ordering::Release);
    }
    let mut bootstrap = bootstrap;
    match reclaim(&mut bootstrap, abandoned) {
        Ok(success) => {
            drop(bootstrap);
            #[cfg(feature = "selftest")]
            RECLAIMED_PAGES.fetch_add(success.observed_returned_pages, Ordering::AcqRel);
            #[cfg(feature = "selftest")]
            RECLAIMED_TABLES.fetch_add(success.reclaimed.tables, Ordering::AcqRel);
            let mut zombies = ZOMBIES.lock_leaf();
            let Some(room) = zombies.owners.get_mut(slot) else {
                panic!("a collected zombie's slot remains in range");
            };
            assert!(
                room.is_none() && zombies.collecting[slot],
                "a collecting zombie cannot change owners"
            );
            zombies.collecting[slot] = false;
            CollectOutcome::Reclaimed(success.reclaimed)
        }
        Err(retention) => {
            #[cfg(feature = "selftest")]
            RECLAIMED_PAGES.fetch_add(retention.observed_returned_pages, Ordering::AcqRel);
            match retention.refusal {
                ReclaimRefusal::TaskRelease => {
                    warn!("user: task release refused; the zombie remains owned");
                }
                ReclaimRefusal::Mapping(reason) => {
                    #[cfg(feature = "selftest")]
                    REFUSED_MAPPING_RECLAIMS.fetch_add(1, Ordering::AcqRel);
                    warn!("user: mapping cleanup refused: {reason}; the zombie remains owned");
                }
                ReclaimRefusal::Tables(reason) => {
                    #[cfg(feature = "selftest")]
                    REFUSED_TABLE_RECLAIMS.fetch_add(1, Ordering::AcqRel);
                    warn!("user: table cleanup refused: {reason}; the zombie remains owned");
                }
            }
            let mut zombies = ZOMBIES.lock_leaf();
            let collecting = zombies.collecting.get(slot).copied().unwrap_or(false);
            let Some(room) = zombies.owners.get_mut(slot) else {
                panic!("a collected zombie's slot remains in range");
            };
            assert!(
                room.is_none() && collecting,
                "a collecting zombie cannot change owners"
            );
            *room = Some(bootstrap);
            zombies.collecting[slot] = false;
            CollectOutcome::Retained
        }
    }
}

#[cfg(feature = "selftest")]
pub(crate) fn pause_collection_after_take_for_test(paused: bool) {
    PAUSE_COLLECTION_AFTER_TAKE.store(paused, Ordering::Release);
}

#[cfg(feature = "selftest")]
pub(crate) fn collection_is_paused_for_test() -> bool {
    COLLECTION_IS_PAUSED.load(Ordering::Acquire)
}

/// Pause one collector after it owns settlement but before it touches the ring.
#[cfg(feature = "selftest")]
pub(crate) fn pause_collection_before_settlement_for_test(paused: bool) {
    PAUSE_COLLECTION_BEFORE_SETTLEMENT.store(paused, Ordering::Release);
}

/// Whether one collector exclusively owns a zombie's settlement pass.
#[cfg(feature = "selftest")]
pub(crate) fn collection_is_settling_for_test() -> bool {
    COLLECTION_IS_SETTLING.load(Ordering::Acquire)
}

/// Whether any program is waiting to be collected.
pub(crate) fn has_zombies() -> bool {
    let zombies = ZOMBIES.lock_leaf();
    zombies.owners.iter().any(Option::is_some)
        || zombies.settling.iter().any(|owned| *owned)
        || zombies.collecting.iter().any(|owned| *owned)
}

/// Scheduler slots which still have a retained process owner.
pub(crate) fn zombie_slots_for_spawn() -> [bool; thread::CAPACITY] {
    let zombies = ZOMBIES.lock_leaf();
    core::array::from_fn(|slot| {
        zombies.owners[slot].is_some() || zombies.settling[slot] || zombies.collecting[slot]
    })
}

fn slot_has_zombie(slot: usize) -> bool {
    let zombies = ZOMBIES.lock_leaf();
    zombies.owners.get(slot).is_some_and(Option::is_some)
        || zombies.settling.get(slot).is_some_and(|owned| *owned)
        || zombies.collecting.get(slot).is_some_and(|owned| *owned)
}

/// Whether a process owner still excludes this scheduler slot.
#[cfg(feature = "selftest")]
pub(crate) fn zombie_owns_slot_for_test(slot: usize) -> bool {
    slot_has_zombie(slot)
}

/// Frames given back by every collection so far.
#[cfg(feature = "selftest")]
pub(crate) fn reclaimed() -> (usize, usize) {
    (
        RECLAIMED_PAGES.load(Ordering::Acquire),
        RECLAIMED_TABLES.load(Ordering::Acquire),
    )
}

/// Address-space table cleanups refused so far.
#[cfg(feature = "selftest")]
pub(crate) fn refused_table_reclaims() -> usize {
    REFUSED_TABLE_RECLAIMS.load(Ordering::Acquire)
}

/// Owned-mapping cleanups refused so far.
#[cfg(feature = "selftest")]
pub(crate) fn refused_mapping_reclaims() -> usize {
    REFUSED_MAPPING_RECLAIMS.load(Ordering::Acquire)
}

/// Prepared-task retirement first attempts which required a successful retry.
#[cfg(feature = "selftest")]
pub(crate) fn refused_task_retirements() -> usize {
    REFUSED_TASK_RETIREMENTS.load(Ordering::Acquire)
}

/// Zombie collections retained because terminal ring work could not settle.
#[cfg(feature = "selftest")]
pub(crate) fn refused_ring_reclaims() -> usize {
    REFUSED_RING_RECLAIMS.load(Ordering::Acquire)
}

/// Pages and table frames returned by failed constructions.
#[cfg(feature = "selftest")]
pub(crate) fn spawn_rollbacks() -> (usize, usize) {
    (
        ROLLED_BACK_PAGES.load(Ordering::Acquire),
        ROLLED_BACK_TABLES.load(Ordering::Acquire),
    )
}

/// Launch, process and zombie records currently occupied.
#[cfg(feature = "selftest")]
pub(crate) fn process_records_for_test() -> (usize, usize, usize) {
    let launches = LAUNCHES.lock_leaf().iter().flatten().count();
    let bootstraps = BOOTSTRAPS.lock_leaf().iter().flatten().count();
    let zombies = ZOMBIES.lock_leaf();
    let zombies = zombies.owners.iter().flatten().count()
        + zombies.collecting.iter().filter(|owned| **owned).count();
    (launches, bootstraps, zombies)
}

/// What a program asked for, in `rax`.
///
/// The synchronous plane (`002` pillar 2): retirement, ring service and wait,
/// and thread-local setup. The *data* path is still the ring: Enter and Wait
/// serve work already posted there rather than transferring data in the trap
/// (D-116, D-164).
///
/// Four, and the surface stays small on purpose. io_uring's exploit record is
/// a warning about surface growth, and every call added here is one a hostile
/// program can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
    /// End this thread. The only one that does not return.
    Exit,
    /// Serve what has been submitted, and report how many completed.
    RingEnter,
    /// Wait until an operation in flight comes due, then serve it.
    ///
    /// The first call in this ABI whose honest answer is *not yet*. It gives
    /// the CPU up rather than spinning, which is what makes a deferred
    /// completion worth having (D-164).
    RingWait,
    /// Point this thread's local storage at `rdi`.
    ///
    /// mlibc's `sys_tcb_set`. Its body is WIRED and the platform route WORKS:
    /// writing the sysdeps found that `FS_BASE` is a fourth thing that must
    /// follow a thread, and a call that sets it is worth nothing until it does
    /// (D-114, D-118, D-181).
    ThreadBase,
    /// Anything else.
    Unknown(u64),
}

impl Call {
    const fn decode(value: u64) -> Self {
        match value {
            0 => Self::Exit,
            1 => Self::RingEnter,
            2 => Self::ThreadBase,
            3 => Self::RingWait,
            other => Self::Unknown(other),
        }
    }
}

/// Calls refused because this kernel does not serve them.
static REFUSED_CALLS: AtomicUsize = AtomicUsize::new(0);

/// The last such number, so a report can say *what* was asked for.
static LAST_REFUSED: AtomicUsize = AtomicUsize::new(0);

/// Call answers that could not be written back.
///
/// Should stay at zero: the frame a handler is running on is the innermost
/// live one by construction.
static UNANSWERED: AtomicUsize = AtomicUsize::new(0);

/// RingEnter calls whose completion wake armed the timer in that trap.
#[cfg(feature = "selftest")]
static RING_ENTER_ARMS: AtomicUsize = AtomicUsize::new(0);

/// Whether the one deferred refusal warning has already been emitted.
static REPORTED: AtomicUsize = AtomicUsize::new(0);

/// Say what the syscall handler could not, if there is anything new.
///
/// Called from the idle thread because a kernel thread may take the console's
/// lock and the handler may not (D-122). Reports at most once per boot and
/// then falls quiet, so a program in a loop cannot turn refusals into a flood.
///
/// `true` if something was said, so the caller knows it did work.
pub(crate) fn report_refusals() -> bool {
    let refused = REFUSED_CALLS.load(Ordering::Acquire);
    if refused == 0 || REPORTED.swap(1, Ordering::AcqRel) != 0 {
        return false;
    }
    let number = LAST_REFUSED.load(Ordering::Acquire);
    warn!("user: refused {refused} call(s), last was {number:#x}");
    true
}

/// Calls this kernel refused, and the last number asked for.
///
/// Read from outside trap context, which is the whole point: the handler
/// cannot take the console's lock, so it leaves a number for something that
/// can (D-122).
#[cfg(feature = "selftest")]
pub(crate) fn refusals() -> (usize, usize, usize) {
    (
        REFUSED_CALLS.load(Ordering::Acquire),
        LAST_REFUSED.load(Ordering::Acquire),
        UNANSWERED.load(Ordering::Acquire),
    )
}

/// RingEnter completion wakes observed to arm before the handler returned.
#[cfg(feature = "selftest")]
pub(crate) fn ring_enter_arms_for_test() -> usize {
    RING_ENTER_ARMS.load(Ordering::Acquire)
}

/// `-ENOSYS`, as `abis/linux` numbers it and as the ABI header states.
const ENOSYS: u64 = (-38i64) as u64;

/// `-EINVAL`, likewise.
const EINVAL: u64 = (-22i64) as u64;

fn answer_call(frame: &TrapFrame, value: u64) {
    let answered = interrupt::answer(frame, value);
    if !answered {
        UNANSWERED.fetch_add(1, Ordering::Relaxed);
    }
    assert!(answered, "the live syscall frame could not be answered");
}

/// Serve one call from ring 3.
///
/// Runs in trap context, so ordinary work here may not allocate, enter a
/// `ThreadContext` blocking path, or take a lock a handler cannot (D-55, D-63).
/// D-200's `wait_from_trap` is the narrow audited exception: it yields through
/// the saved trap frame. `Queue::serve` takes leaf locks only around bounded
/// state transitions and defers file service to the worker.
///
/// **Only `Exit` does not return.** Other calls preserve the caller so it can
/// act on the result (D-116).
pub(crate) fn call(frame: &TrapFrame) {
    let slot = task::current();
    match Call::decode(frame.rax()) {
        Call::Exit => {
            if !thread::retire_from_trap(frame) {
                panic!("a ring 3 thread called in and could not be retired");
            }
        }
        Call::RingEnter => {
            #[cfg(feature = "selftest")]
            let arms = thread::handler_arms();
            let served = serve_ring(slot);
            #[cfg(feature = "selftest")]
            if served > 0 && thread::handler_arms() > arms {
                RING_ENTER_ARMS.fetch_add(1, Ordering::Release);
            }
            answer_call(frame, served);
        }
        Call::RingWait => {
            let completed = wait_ring(slot, frame);
            answer_call(frame, completed);
        }
        Call::ThreadBase => {
            // Refused, not faulted: `wrmsr` of a non-canonical base is a `#GP`
            // in kernel context, a program crashing the handler.
            let answer = if task::set_thread_base(frame.rdi()) {
                0
            } else {
                EINVAL
            };
            answer_call(frame, answer);
        }
        Call::Unknown(number) => {
            // Refused with an answer: a program that gets nothing back cannot
            // tell a wrong call from a dropped one.
            answer_call(frame, ENOSYS);
            // Counted, not logged: trap context cannot take the console lock,
            // and once did (D-63, D-122); the idle thread reports these.
            REFUSED_CALLS.fetch_add(1, Ordering::Relaxed);
            LAST_REFUSED.store(number as usize, Ordering::Relaxed);
        }
    }
}

/// Serve the calling thread's own ring, and count the completions.
///
/// The ring is reached through the thread's `Bootstrap`, which is where its
/// authority lives — a program cannot name another's, because there is no
/// argument here that could say which.
fn serve_ring(slot: usize) -> u64 {
    // Cloned out before anything is served: `serve` takes four more locks,
    // which `lock_leaf` forbids under this guard (D-121, D-124).
    let Some((ring, table)) = authority_of(slot) else {
        return ENOSYS;
    };

    // The program's own claim about what it has reaped, taken once and bounded
    // inside (D-113, D-115).
    let head = ring.completion_head();
    ring.reap_up_to(head);
    // Due operations complete before new ones are admitted, so finished work
    // never costs the next submission a slot the kernel already owes.
    ring.expire(crate::time::now_ns());
    ring.serve(&table) as u64
}

/// The ring and the handle table of the thread in `slot`.
///
/// Both are cloned out and the guard dropped before either is used. `serve`
/// reads the program's memory, performs each operation and wakes whatever was
/// waiting, and `install` takes the handle arena's leaf — doing any of that
/// under this table's guard is what `lock_leaf` forbids (D-121, D-124).
fn authority_of(
    slot: usize,
) -> Option<(
    alloc::sync::Arc<queue::Queue>,
    alloc::sync::Arc<HandleTable>,
)> {
    let guard = BOOTSTRAPS.lock_leaf();
    let bootstrap = guard.get(slot).and_then(Option::as_ref)?;
    Some((
        alloc::sync::Arc::clone(&bootstrap.ring),
        alloc::sync::Arc::clone(&bootstrap.table),
    ))
}

/// Wait for an operation in flight, then serve what came due.
///
/// Three answers, and they are different things. **Nothing in flight** returns
/// zero at once — there is nothing to wait for, and blocking would be a hang.
/// **Something already due** is completed without sleeping. Otherwise the
/// thread sleeps until the soonest deadline and completes what it finds when
/// it wakes, which may be later than the deadline and never earlier.
///
/// The completion is posted by the timer path or file worker, not by the
/// program asking again: a woken program finds the entry already in its queue.
/// That is what makes this a wait rather than a poll (D-164, D-200).
fn wait_ring(slot: usize, frame: &TrapFrame) -> u64 {
    let Some((ring, _)) = authority_of(slot) else {
        return ENOSYS;
    };

    ring.wait_from_trap(frame) as u64

    // May return having done nothing (no peer, or no clock); the answer is
    // still the truth about what is due now.
}

/// Cross into ring 3, and never come back on this thread.
///
/// Runs as an ordinary kernel thread until its last instruction, and is retired
/// by the syscall handler from the far side of the boundary.
///
/// No stack is arranged for the trap out: the context switch that scheduled
/// this thread already pointed `RSP0` at its own kernel stack, and will point
/// it somewhere else the moment another thread runs (D-99). The same switch
/// installed this thread's tables, which is what makes the copy below reach
/// *its* stack and no other program's (D-106).
fn cross() {
    let slot = task::current();
    let Some(launch) = LAUNCHES.lock_leaf().get(slot).copied().flatten() else {
        warn!("user: thread {slot} has no program to run");
        refuse_cross(slot);
    };
    let which = launch.which;

    #[cfg(feature = "selftest")]
    if GATE_NEXT_CROSS.swap(false, Ordering::AcqRel) {
        let mut context = thread::ThreadContext::require();
        CROSS_GATE.wait_unless(&mut context, || CROSS_MAY_ENTER.load(Ordering::Acquire));
    }

    // Checked, not assumed: a missed switch would put one program's argument
    // on another's stack.
    if cpu::page_table_base() != launch.root {
        warn!("user {which}: running on tables that are not its own");
        refuse_cross(slot);
    }

    // Version, argument and bootstrap handle, written with the checked copy
    // (D-97, D-182); the tail keeps the initial pointer aligned.
    let top = launch.stack + USER_STACK_BYTES - ENTRY_STACK_BYTES;
    if usercopy::copy_to_user(top + entry::VERSION as u64, &ENTRY_VERSION.to_le_bytes()).is_err()
        || usercopy::copy_to_user(top + entry::ARGUMENT as u64, &launch.argument.to_le_bytes())
            .is_err()
        || usercopy::copy_to_user(
            top + entry::BOOTSTRAP_HANDLE as u64,
            &u64::from(launch.bootstrap_handle).to_le_bytes(),
        )
        .is_err()
    {
        warn!("user {which}: could not write its entry block");
        refuse_cross(slot);
    }

    info!(
        "user {which}: entering ring 3 at {:#x} with {} on {:#x}",
        launch.entry, launch.argument, launch.root
    );
    if !crate::maskable_interrupts_are_ready() {
        warn!("user {which}: maskable interrupts are not admitted");
        refuse_cross(slot);
    }
    let refused = task::enter_user(launch.entry, top);
    warn!("user {which}: refused entry to ring 3: {refused}");
    refuse_cross(slot);
}

fn refuse_cross(slot: usize) -> ! {
    let _disabled = interrupt::disable();
    orphan(slot);
    thread::exit()
}

/// The first user program, which calls the kernel.
pub(crate) fn run() -> Result<Spawned, SpawnError> {
    spawn("user", 0, BEHAVE)
}

/// A second one, in a space of its own, at its own addresses.
#[cfg(feature = "selftest")]
pub(crate) fn run_second() -> Result<Spawned, SpawnError> {
    spawn("user-two", 1, BEHAVE)
}

/// A fifth, which asks for a call this kernel does not serve.
#[cfg(feature = "selftest")]
pub(crate) fn run_probing() -> Result<Spawned, SpawnError> {
    spawn("user-probe", 4, PROBE)
}

/// The argument that makes the program ask for an unserved call.
///
/// Must match the behaviour `xtask/src/image/program.rs` emits for it.
#[cfg(feature = "selftest")]
const PROBE: u64 = 2;

/// A fourth, for the check that counts what a program is given at birth.
#[cfg(feature = "selftest")]
pub(crate) fn run_for_bootstrap() -> Result<Spawned, SpawnError> {
    spawn("user-bootstrap", 3, BEHAVE)
}

/// A child which measures one real Echo call across the ring-3 gate.
#[cfg(feature = "selftest")]
pub(crate) fn run_measuring() -> Result<Spawned, SpawnError> {
    spawn("user-baseline", 8, MEASURE)
}

/// A seventh, which duplicates its bootstrap channel through its own ring.
#[cfg(feature = "selftest")]
pub(crate) fn run_duplicating() -> Result<Spawned, SpawnError> {
    spawn("user-duplicate", 6, DUPLICATE)
}

/// An eighth, which defers a timer and blocks in `RingWait` for its result.
#[cfg(feature = "selftest")]
pub(crate) fn run_waiting() -> Result<Spawned, SpawnError> {
    spawn("user-wait", 7, WAIT)
}

/// An echoing program held just before ring-3 entry for wake orchestration.
#[cfg(feature = "selftest")]
pub(crate) fn run_gated_for_wake() -> Result<Spawned, SpawnError> {
    if !CROSS_GATE.is_idle() || GATE_NEXT_CROSS.load(Ordering::Acquire) {
        return Err(SpawnError::Incomplete);
    }
    CROSS_MAY_ENTER.store(false, Ordering::Release);
    if GATE_NEXT_CROSS.swap(true, Ordering::AcqRel) {
        CROSS_MAY_ENTER.store(true, Ordering::Release);
        return Err(SpawnError::Incomplete);
    }
    let spawned = spawn("user-wake", 9, BEHAVE);
    if spawned.is_err() {
        GATE_NEXT_CROSS.store(false, Ordering::Release);
        CROSS_MAY_ENTER.store(true, Ordering::Release);
    }
    spawned
}

/// Whether the gated child has registered and stopped being runnable.
#[cfg(feature = "selftest")]
pub(crate) fn gated_cross_is_waiting() -> bool {
    !CROSS_GATE.is_idle()
}

/// Let the gated child enter ring 3 and report how many threads were released.
#[cfg(feature = "selftest")]
pub(crate) fn release_gated_cross() -> usize {
    CROSS_MAY_ENTER.store(true, Ordering::Release);
    CROSS_GATE.wake_all()
}

/// A sixth, which jumps into the kernel's user-copy window from ring 3.
///
/// The argument *is* the address: the program jumps to anything it is given in
/// the kernel's half (D-126). Ring 3 cannot execute kernel memory, so this is
/// an instruction-fetch fault whose `RIP` is inside the window the recovery
/// arm looks at — which is the state that arm accepted without ever checking
/// who faulted.
#[cfg(feature = "selftest")]
pub(crate) fn run_into_the_copy_window() -> Result<Spawned, SpawnError> {
    let (start, _) = usercopy::fault_window();
    spawn("user-window", 5, start)
}

/// A third that faults on purpose, to show the machine survives it.
#[cfg(feature = "selftest")]
pub(crate) fn run_faulting() -> Result<Spawned, SpawnError> {
    spawn("user-fault", 2, FAULT)
}

/// The argument that makes the program call the kernel.
const BEHAVE: u64 = 0;

/// The argument that makes it execute `ud2` instead.
#[cfg(feature = "selftest")]
const FAULT: u64 = 1;

/// The argument that makes it submit Duplicate for its bootstrap handle.
#[cfg(feature = "selftest")]
const DUPLICATE: u64 = 3;

/// The argument that makes it submit a Timer and enter `RingWait`.
#[cfg(feature = "selftest")]
const WAIT: u64 = 4;

/// The argument that brackets Echo's RingEnter with timestamp reads.
#[cfg(feature = "selftest")]
const MEASURE: u64 = 5;

/// The next child reaching [`cross`] must wait for the machine check.
#[cfg(feature = "selftest")]
static GATE_NEXT_CROSS: AtomicBool = AtomicBool::new(false);

/// The condition paired with [`CROSS_GATE`].
#[cfg(feature = "selftest")]
static CROSS_MAY_ENTER: AtomicBool = AtomicBool::new(true);

/// Stops one real child before its first user instruction.
#[cfg(feature = "selftest")]
static CROSS_GATE: WaitQueue = WaitQueue::new();
