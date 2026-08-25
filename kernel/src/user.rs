//! The first thing that runs in ring 3.
//!
//! Not a program and not a process: a page of instructions, a page of stack,
//! and a thread that stops being the kernel's. What it proves is the boundary
//! — that the descriptors, the gate, the privilege stack and the return path
//! all agree — which is the thing every later slice builds on and the thing
//! that is hardest to debug once there is a loader in the way (D-96).
//!
//! It started as deliberately not an ELF, so that a failure here would be a
//! failure of the boundary rather than of a parser. Slice 7 brought
//! `libs/elf64` and an initrd into this same file, and the loader below now
//! parses attacker-supplied bytes — so a failure here can be either, and the
//! refusals are checked one mechanism at a time for exactly that reason
//! (D-100, D-105). Each program now runs in a table of its own (D-106).

use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};

use elf64::{Elf, ElfError, SegmentKind};
use handle_space::Rights;
use wost::arch::current::cpu;
use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::paging::{ActivateError, AddressSpace, MapError};
use wost::arch::current::task;
use wost::arch::current::usercopy;
use wost::boot::HandoffError;
use wost::mm;
use wost::mm::Paddr;
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

use crate::handle::{HandleError, HandleTable};
use crate::object::{self, ObjectRef, ObjectType};
use crate::{info, queue, thread, warn};

/// Where the first user program's page of code goes.
///
/// Low half, far from anything the kernel maps: an address a user mapping can
/// occupy is by definition one the kernel does not, and the split is bit 63
/// (`010` §10's LASS note).
#[allow(dead_code)]
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
/// Each program now has a table of its own (D-106), so this is no longer what
/// keeps them apart — the tables are. It is kept so that a program's addresses
/// differ from its neighbour's, which is what lets a check tell one crossing's
/// record from another's without reading the tables.
const APART: u64 = 0x1_0000;

/// Bytes of stack, which is one page: it makes one call and never returns.
const STACK_BYTES: u64 = PAGE_SIZE;

/// The program's first four bytes, for the check that reads them back across
/// the boundary.
///
/// `pop rax` and the `test` that follows it — the program reading the argument
/// the kernel left on its stack (D-102). Written by `xtask` into the image
/// (D-100) rather than by the kernel, so this is a statement about the initrd
/// and not a copy of something the kernel placed.
#[allow(dead_code)]
pub(crate) const PROGRAM: [u8; 4] = [0x58, 0x48, 0x85, 0xC0];

/// Map the program and its stack, and report where to enter.
///
/// Both pages are user-accessible and neither is both writable and executable:
/// the code page is read-only and executable, the stack is writable and never
/// executable. W^X is not relaxed for user memory — it is the reason user
/// memory exists (D-12).
fn place_stack(space: &mut AddressSpace, which: u64) -> Option<u64> {
    let at = STACK + which * APART;
    let mut stack = mm::allocate_contiguous((STACK_BYTES / PAGE_SIZE) as usize, 1)?;
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
    /// It has nothing to load.
    Empty,
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Missing => f.write_str("no initrd"),
            Self::NotAnElf(error) => write!(f, "{error}"),
            Self::Malformed(at) => write!(f, "the segment at {at:#x} claims more file than memory"),
            Self::Truncated(at) => write!(f, "the segment at {at:#x} runs past the image"),
            Self::Overlapping(at) => write!(f, "the segment at {at:#x} overlaps another"),
            Self::TooManySegments => {
                f.write_str("the image has more segments than the loader places")
            }
            Self::WriteExecute(at) => {
                write!(
                    f,
                    "the segment at {at:#x} asks to be writable and executable"
                )
            }
            Self::NotUserMemory(at) => write!(f, "the segment at {at:#x} is not user memory"),
            Self::OutOfMemory => f.write_str("no memory for the program"),
            Self::NotMapped(error) => write!(f, "{error}"),
            Self::Empty => f.write_str("the program has nothing to load"),
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
fn load(
    space: &mut AddressSpace,
    shift: u64,
    regions: &mut [Option<(u64, u64)>; MAX_REGIONS],
    used: &mut usize,
) -> Result<u64, LoadError> {
    let bytes = (*IMAGE.lock_leaf()).ok_or(LoadError::Missing)?;
    let entry = load_image(space, bytes, shift)?;
    // What it mapped, so the same comes back. Read from the placements rather
    // than recomputed, so a change to how a segment is rounded cannot leave a
    // region behind (D-117).
    for (at, len) in last_placements() {
        remember(regions, used, at, len);
    }
    Ok(entry)
}

/// Where the last successful `load_image` put things.
///
/// A static rather than a return value because `load_image` is also called by
/// the checks with a doctored image, and threading an out-parameter through
/// every one of those would be noise around the thing they test. Written only
/// on success, and read only by `load` immediately after.
static PLACED: SpinLock<[Option<(u64, u64)>; MAX_REGIONS]> = SpinLock::new([None; MAX_REGIONS]);

/// Take what the last load placed.
fn last_placements() -> impl Iterator<Item = (u64, u64)> {
    let taken = core::mem::take(&mut *PLACED.lock_leaf());
    taken.into_iter().flatten()
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
        .map_err(LoadError::NotMapped)
}

/// Unmap what this load already placed, returning its frames.
fn undo(space: &mut AddressSpace, done: &[Option<Placement>]) {
    for placement in done.iter().flatten() {
        let len = placement.pages as u64 * PAGE_SIZE;
        // Nothing to do about a failure here: the frames stay in the space
        // rather than going back to the allocator, which is a leak and not a
        // double free. Reporting it would replace the error being returned.
        let _ = space.unmap(placement.at, len);
    }
}

/// Whether two validated placements want any page in common.
fn overlaps(a: &Placement, b: &Placement) -> bool {
    let a_end = a.at.saturating_add(a.pages as u64 * PAGE_SIZE);
    let b_end = b.at.saturating_add(b.pages as u64 * PAGE_SIZE);
    a.at < b_end && b.at < a_end
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
/// Two passes, and the split is the point. The single-pass version checked
/// segment *N*, mapped it, then looked at *N+1* — so a refusal on the second
/// segment left the first mapped `USER` at an address the image chose, with
/// its frames forgotten into the space and nothing owning them. The doc above
/// claimed the preflight this now performs (D-105).
pub(crate) fn load_image(
    space: &mut AddressSpace,
    bytes: &[u8],
    shift: u64,
) -> Result<u64, LoadError> {
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

        // Checked, because both halves are the image's: `p_vaddr` near the top
        // of the range wraps into a low address and walks straight past the
        // refusal below, and in a debug build it panics the kernel on bytes a
        // program supplied (D-101). `contents_fit` does this correctly one
        // function away, in `libs/elf64`.
        let at = segment
            .address()
            .checked_add(shift)
            .ok_or(LoadError::NotUserMemory(segment.address()))?;
        let end = at
            .checked_add(segment.memory_size())
            .ok_or(LoadError::NotUserMemory(at))?;
        if end > usercopy::USER_END || segment.memory_size() == 0 {
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
            pages: segment.memory_size().div_ceil(PAGE_SIZE) as usize,
            from: segment.offset() as usize,
            len: segment.file_size() as usize,
            flags,
        };

        // The mapping rounds each segment out to whole pages, so two segments
        // sharing one page collide in `map_owned` — after the first is already
        // mapped. Caught here instead, where nothing has been taken yet.
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

    for (done, placement) in placements.iter().flatten().enumerate() {
        match place(space, bytes, placement) {
            Ok(()) => {}
            Err(error) => {
                // Validation cannot rule out a frame shortage or a collision
                // with something already in the space, so the mapping pass
                // walks itself back the way `map_range` does (D-94): what is
                // returned here is what this call mapped, and nothing else.
                undo(space, &placements[..done]);
                return Err(error);
            }
        }
    }

    if loaded == 0 {
        return Err(LoadError::Empty);
    }

    // Recorded for `load`, which is the only caller that maps into a space
    // something will later reclaim.
    {
        let mut placed = PLACED.lock_leaf();
        *placed = [None; MAX_REGIONS];
        for (slot, placement) in placements.iter().flatten().enumerate() {
            if let Some(room) = placed.get_mut(slot) {
                *room = Some((placement.at, placement.pages as u64 * PAGE_SIZE));
            }
        }
    }

    elf.entry()
        .checked_add(shift)
        .ok_or(LoadError::NotUserMemory(elf.entry()))
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
    which: u64,
    root: Paddr,
}

/// One per thread slot. A leaf: nothing is called while it is held.
static LAUNCHES: SpinLock<[Option<Launch>; thread::CAPACITY]> =
    SpinLock::new([None; thread::CAPACITY]);

/// What a program is given at birth, and the whole of it.
///
/// `002`'s pillar 1, stated as an invariant a check can hold us to: **a
/// process receives exactly one handle at spawn — its bootstrap channel — and
/// every later handle arrived through a channel.** The table is the process's
/// whole power, so "one entry" is the strongest form the claim has (D-111).
/// Note that `len` bytes at `at` are this program's to have unmapped.
///
/// Silently drops the note past [`MAX_REGIONS`], which costs the frames rather
/// than the correctness — and cannot happen for an image the loader accepted,
/// since it places at most eight segments and this holds the stack beside them.
fn remember(regions: &mut [Option<(u64, u64)>; MAX_REGIONS], used: &mut usize, at: u64, len: u64) {
    if let Some(slot) = regions.get_mut(*used) {
        *slot = Some((at, len));
        *used += 1;
    }
}

/// The most user regions one program maps.
///
/// Its segments, its stack and its ring. Bounded because reclaiming means
/// unmapping exactly what was mapped, and a list that could grow is a list
/// that needs the allocator on a path that must not fail (D-117).
const MAX_REGIONS: usize = 8;

#[allow(dead_code)]
struct Bootstrap {
    /// Its tables, kept so they can be given back. `bind` borrows rather than
    /// consumes for exactly this reason (D-117).
    space: AddressSpace,
    /// What was mapped into them, so the same is unmapped.
    regions: [Option<(u64, u64)>; MAX_REGIONS],
    table: HandleTable,
    /// Its ring. Held here because the shared pages are the kernel's and must
    /// outlive the mapping that shows them to the program (D-113).
    ring: queue::Queue,
    /// The one handle. Kept so a check can ask what it names rather than
    /// inferring it from the count.
    handle: u32,
}

/// One per thread slot, alongside its [`Launch`].
static BOOTSTRAPS: SpinLock<[Option<Bootstrap>; thread::CAPACITY]> =
    SpinLock::new([const { None }; thread::CAPACITY]);

/// Handles a process may hold at once.
///
/// A quota, and the answer to kernel-memory exhaustion at the handle layer
/// (D-78). Small because nothing yet gives a program a second handle.
const HANDLE_QUOTA: usize = 64;

/// What the parent keeps of a spawned program.
///
/// The far end of its bootstrap channel: the only way to reach it, which is
/// what "no ambient authority" means from the other side.
#[allow(dead_code)]
pub(crate) struct Spawned {
    /// The thread slot it runs in.
    pub(crate) slot: usize,
    /// The parent's end of its bootstrap channel.
    pub(crate) channel: ObjectRef,
}

/// What a spawned program's table holds, for the check that counts it.
#[cfg(feature = "selftest")]
pub(crate) fn bootstrap_of(slot: usize) -> Option<(usize, ObjectType)> {
    let guard = BOOTSTRAPS.lock_leaf();
    let bootstrap = guard.get(slot)?.as_ref()?;
    let kind = bootstrap.table.kind_of(bootstrap.handle).ok()?;
    Some((bootstrap.table.live(), kind))
}

/// What the **kernel** believes a spawned program's ring is.
///
/// Not the header page, which the program can rewrite the moment it runs: a
/// check reading that is asking ring 3 what shape its own ring is, and one
/// arithmetic step later is a panic in the debug kernel on a number a program
/// chose (D-115).
#[cfg(feature = "selftest")]
pub(crate) fn ring_shape_of(slot: usize) -> Option<(u32, u32, u32)> {
    let guard = BOOTSTRAPS.lock_leaf();
    Some(guard.get(slot)?.as_ref()?.ring.shape())
}

/// Why a program could not be started.
#[derive(Debug)]
pub(crate) enum SpawnError {
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
    /// Its bootstrap channel could not be put in its table.
    NoBootstrap(HandleError),
    /// No memory for its ring.
    NoRing,
    /// No memory for its bootstrap channel.
    NoChannel,
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSpace(error) => write!(f, "no address space: {error}"),
            Self::Load(error) => write!(f, "{error}"),
            Self::NoStack => f.write_str("no memory for a stack"),
            Self::NotBindable(error) => write!(f, "the space cannot be installed: {error}"),
            Self::NoThread(error) => write!(f, "no thread: {error}"),
            Self::NoBootstrap(error) => write!(f, "no bootstrap channel: {error}"),
            Self::NoRing => f.write_str("no memory for a ring"),
            Self::NoChannel => f.write_str("no memory for a bootstrap channel"),
        }
    }
}

/// Start a program in an address space of its own.
///
/// The order is forced and each step is why the next one is possible: a space
/// that shares only the kernel's higher half (D-104), the image loaded into
/// *that* space rather than into the running one, a stack, the binding that
/// proves the space is installable, and a thread whose tables are set before
/// anything can schedule it (D-106).
///
/// A failure partway leaves no thread naming anything, but it does leak: an
/// `AddressSpace` has no `Drop`, because its table frames are leaked on
/// purpose, and the frames a half-load mapped go with them. Bounded — this is
/// called three times a boot — and it is the same debt `undo` pays down inside
/// one load. D-44's accounting is where it is settled.
///
/// # Errors
///
/// [`SpawnError`], naming which of those steps refused.
pub(crate) fn spawn(name: &'static str, which: u64, argument: u64) -> Result<Spawned, SpawnError> {
    let mut space = AddressSpace::for_user().map_err(SpawnError::NoSpace)?;
    // Every range mapped into it, so the same ranges come back (D-117). The
    // loader reports its own; the stack and the ring are one each.
    let mut regions = [None; MAX_REGIONS];
    let mut mapped = 0;

    let entry =
        load(&mut space, which * APART, &mut regions, &mut mapped).map_err(SpawnError::Load)?;
    let stack = place_stack(&mut space, which).ok_or(SpawnError::NoStack)?;
    remember(&mut regions, &mut mapped, stack, STACK_BYTES);

    // Its ring, at the same address every program's is — which is what having
    // a space of its own buys. The pages stay the kernel's: `map_shared`
    // writes no owned bit, so a program cannot free the memory a completion
    // will be written into (D-113).
    //
    // Deliberately *not* remembered as a region: those frames are the ring's,
    // not the space's, so unmapping them must not return them — `map_shared`
    // wrote no owned bit and `unmap` would refuse anyway. They go back when
    // the ring drops.
    let mut ring = queue::Queue::new().ok_or(SpawnError::NoRing)?;
    ring.install(&mut space, RING)
        .map_err(SpawnError::NoSpace)?;

    let root = space.root();
    let binding = space.bind().map_err(SpawnError::NotBindable)?;

    // Its whole power, made before the thread exists so the table is never
    // observable holding anything else. The child's end can be used and sent
    // on; the parent keeps the other (D-111).
    let (theirs, ours) = object::channel().ok_or(SpawnError::NoChannel)?;
    let mut table = HandleTable::new(HANDLE_QUOTA);
    let handle = table
        .insert(
            theirs,
            Rights::UNIVERSAL.union(Rights::READ).union(Rights::WRITE),
        )
        .map_err(SpawnError::NoBootstrap)?;

    let mut bootstrap = Some(Bootstrap {
        space,
        regions,
        table,
        handle,
        ring,
    });
    let slot = thread::spawn_in_space(name, cross, binding, |slot| {
        *LAUNCHES
            .lock_leaf()
            .get_mut(slot)
            .expect("a slot wost accepted is inside the table") = Some(Launch {
            entry,
            stack,
            argument,
            which,
            root,
        });
        *BOOTSTRAPS
            .lock_leaf()
            .get_mut(slot)
            .expect("a slot wost accepted is inside the table") = bootstrap.take();
    })
    .map_err(SpawnError::NoThread)?;

    Ok(Spawned {
        slot,
        channel: ours,
    })
}

/// Give back everything a retired program held.
///
/// **Not called by the thread that owned it.** A retiring thread is standing
/// on the kernel stack it is about to free and running on the tables it is
/// about to reclaim; it cannot be the one to do this, and any design where it
/// tries is a use-after-free wearing a comment. So a retired program's
/// resources become a *zombie*, and something else collects (D-117).
///
/// The order is forced and each step is why the next is possible:
///
/// 1. **The mappings**, so the frames go back before the tables that reach
///    them do. `unmap` returns them because `map_user` wrote the owned bit;
///    the ring's pages are not among these, because `map_shared` did not.
/// 2. **The handle table**, whose `Drop` releases every object it names — the
///    bootstrap channel among them, so the parent's end learns its peer is
///    gone.
/// 3. **The ring**, whose `Shared` frees the pages the kernel kept.
/// 4. **The tables**, last, because everything above reached memory through
///    them.
///
/// Returns what went back, for the check that asks whether it did.
fn reclaim(bootstrap: Bootstrap) -> Reclaimed {
    let Bootstrap {
        mut space,
        regions,
        table,
        handle: _,
        ring,
    } = bootstrap;

    let mut unmapped = 0;
    for (at, len) in regions.into_iter().flatten() {
        // The flush token is dropped here, which is what makes the mapping
        // gone for the CPU as well as for the tables (D-88). Nothing to do
        // about a refusal: the frames stay in the space and go with its
        // tables, which is a leak and not a double free.
        if let Ok(returned) = space.unmap(at, len) {
            unmapped += returned.pages() as usize;
            drop(returned);
        }
    }

    // Before the tables: dropping an object can reach the frame allocator, and
    // doing that after the space is gone would be fine — doing it *first* is
    // simply the order that has nothing subtle in it.
    drop(table);
    drop(ring);

    // The task table is asked whether anything still runs on these, and the
    // answer is what `reclaim` takes. Refused if the CPU is walking them, or
    // if a live slot still names them — the second being the case a `CR3`
    // comparison cannot see (D-123).
    let released = task::release_address_space(space.root());
    // `None` means something still names it. The tables stay, which is a leak
    // and not a fault — and the alternative is a `CR3` pointing at re-issued
    // frames the next time that thread is scheduled.
    let (tables, lost) = space.reclaim(released).unwrap_or((0, 0));

    Reclaimed {
        pages: unmapped,
        tables,
        lost,
    }
}

/// What one reclaim gave back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Reclaimed {
    /// Frames the program's own mappings held.
    pub(crate) pages: usize,
    /// Frames its page tables were built from.
    pub(crate) tables: usize,
    /// Table frames it had too many of to remember, and did not get back.
    pub(crate) lost: usize,
}

/// Programs whose threads are gone and whose resources are not.
static ZOMBIES: SpinLock<[Option<Bootstrap>; thread::CAPACITY]> =
    SpinLock::new([const { None }; thread::CAPACITY]);

/// What every reclaim so far gave back.
static RECLAIMED_PAGES: AtomicUsize = AtomicUsize::new(0);
static RECLAIMED_TABLES: AtomicUsize = AtomicUsize::new(0);

/// What a program held when it died.
///
/// Taken at `orphan` and kept after the resources are gone, because "what did
/// that program do" is a question worth answering once it cannot be asked of
/// the program itself. It is also the only way a check can observe a process
/// that reclamation has since collected — before D-117 a retired program's
/// bootstrap simply stayed, and every check that read one was reading a
/// corpse the kernel had not got round to burying (D-117).
/// Read only by the checks in the shipped kernel — the record is kept for
/// every program either way, because a boot that wants to know what a program
/// did should not have to be rebuilt to find out (D-117).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct LastRites {
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

/// One per slot, overwritten by the next program to use it.
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
    if let Some(bootstrap) = taken {
        // The record first, while there is still something to read.
        let rites = LastRites {
            handles: bootstrap.table.live(),
            bootstrap: bootstrap.table.kind_of(bootstrap.handle).ok(),
            served: bootstrap.ring.served_count(),
            submitted: bootstrap.ring.read_submission(0),
            completed: bootstrap.ring.read_completion(0),
        };
        if let Some(room) = RITES.lock_leaf().get_mut(slot) {
            *room = Some(rites);
        }
        if let Some(room) = ZOMBIES.lock_leaf().get_mut(slot) {
            *room = Some(bootstrap);
        }
    }
    // Cleared, so a slot reused by a later program never finds the previous
    // one's entry point (D-117).
    if let Some(room) = LAUNCHES.lock_leaf().get_mut(slot) {
        *room = None;
    }
}

/// Collect one dead program's resources, if there are any.
///
/// One per call rather than all of them: this runs where the machine had
/// nothing better to do, and a loop that frees sixteen processes with
/// interrupts on is a loop that can be preempted halfway through anyway. One
/// at a time makes that unremarkable instead of something to reason about.
///
/// `None` if nothing was waiting.
pub(crate) fn collect() -> Option<Reclaimed> {
    // Taken out under the guard and reclaimed outside it: `reclaim` frees
    // frames, which reaches the allocator, and this lock is a leaf (D-85).
    let found = {
        let mut zombies = ZOMBIES.lock_leaf();
        zombies.iter_mut().find_map(Option::take)
    };

    let bootstrap = found?;
    let reclaimed = reclaim(bootstrap);
    RECLAIMED_PAGES.fetch_add(reclaimed.pages, Ordering::AcqRel);
    RECLAIMED_TABLES.fetch_add(reclaimed.tables, Ordering::AcqRel);
    Some(reclaimed)
}

/// Whether any program is waiting to be collected.
pub(crate) fn has_zombies() -> bool {
    ZOMBIES.lock_leaf().iter().any(Option::is_some)
}

/// Frames given back by every collection so far.
#[cfg(feature = "selftest")]
pub(crate) fn reclaimed() -> (usize, usize) {
    (
        RECLAIMED_PAGES.load(Ordering::Acquire),
        RECLAIMED_TABLES.load(Ordering::Acquire),
    )
}

/// What a program asked for, in `rax`.
///
/// The synchronous plane (`002` pillar 2): setup, handle operations, memory
/// and blocking waits. The *data* path is the ring, and the one call here that
/// touches it does so by asking the kernel to serve what the program has
/// already posted — the trap is a doorbell, not a transfer (D-116).
///
/// Two, and the surface stays small on purpose. io_uring's exploit record is
/// a warning about surface growth, and every call added here is one a hostile
/// program can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
    /// End this thread. The only one that does not return.
    Exit,
    /// Serve what has been submitted, and report how many completed.
    RingEnter,
    /// Point this thread's local storage at `rdi`.
    ///
    /// mlibc's `sys_tcb_set`, and the reason it is here rather than on the
    /// owed list: writing the sysdeps found that `FS_BASE` is a fourth thing
    /// that must follow a thread, and a call that sets it is worth nothing
    /// until it does (D-114, D-118).
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
            other => Self::Unknown(other),
        }
    }
}

/// Calls refused because this kernel does not serve them.
static REFUSED_CALLS: AtomicUsize = AtomicUsize::new(0);

/// The last such number, so a report can say *what* was asked for.
static LAST_REFUSED: AtomicUsize = AtomicUsize::new(0);

/// Ring-enter answers that could not be written back.
///
/// Should stay at zero: the frame a handler is running on is the innermost
/// live one by construction.
static UNANSWERED: AtomicUsize = AtomicUsize::new(0);

/// Refusals already reported, so each is said once.
static REPORTED: AtomicUsize = AtomicUsize::new(0);

/// Say what the syscall handler could not, if there is anything new.
///
/// Called from the idle thread, which is a kernel thread and may take the
/// console's lock — the handler may not, and that assert is what a program
/// used to halt the machine with (D-122). Reports once per refusal and then
/// falls quiet, so a program in a loop cannot turn a refusal into a flood.
///
/// `true` if something was said, so the caller knows it did work.
pub(crate) fn report_refusals() -> bool {
    let refused = REFUSED_CALLS.load(Ordering::Acquire);
    let reported = REPORTED.load(Ordering::Acquire);
    if refused <= reported {
        return false;
    }
    let number = LAST_REFUSED.load(Ordering::Acquire);
    REPORTED.store(refused, Ordering::Release);
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

/// `-ENOSYS`, as `abis/linux` numbers it and as the ABI header states.
const ENOSYS: u64 = (-38i64) as u64;

/// `-EINVAL`, likewise.
const EINVAL: u64 = (-22i64) as u64;

/// Serve one call from ring 3.
///
/// Runs in trap context, so nothing here may allocate, block, or take a lock a
/// handler cannot (D-55, D-63). `Queue::serve` is written to that rule: it
/// takes two leaf locks in a fixed order and calls nothing while either is
/// held.
///
/// **Only `Exit` does not return.** Until D-116 the gate retired its caller
/// unconditionally, which made the boundary provable and the ABI unusable —
/// a program could cross once and never act on what it learned.
pub(crate) fn call(frame: &TrapFrame) {
    let slot = task::current();
    match Call::decode(frame.rax()) {
        Call::Exit => {
            if !thread::retire_from_trap(frame) {
                panic!("a ring 3 thread called in and could not be retired");
            }
        }
        Call::RingEnter => {
            let served = serve_ring(slot);
            // The frame is the innermost live one — this is a handler running
            // on it — so the write cannot be refused for any reason a caller
            // could cause. Counted rather than logged; see below.
            if !interrupt::answer(frame, served) {
                UNANSWERED.fetch_add(1, Ordering::Relaxed);
            }
        }
        Call::ThreadBase => {
            // Refused rather than faulted for a base the program may not have:
            // `wrmsr` on a non-canonical value is a `#GP` in *kernel* context,
            // which would be a program crashing the handler rather than
            // itself.
            let answer = if task::set_thread_base(frame.rdi()) {
                0
            } else {
                EINVAL
            };
            let _ = interrupt::answer(frame, answer);
        }
        Call::Unknown(number) => {
            // Refused *with an answer*, never ignored: a program that gets
            // nothing back cannot tell a call it made wrong from one the
            // kernel dropped, and would wait for ever on the difference.
            let _ = interrupt::answer(frame, ENOSYS);
            // **Counted, not logged.** This runs in trap context, and `warn!`
            // takes the console's `ThreadLock`, whose first line asserts it is
            // *not* in a trap (D-63). So logging here was a program halting
            // the machine with `mov rax, 3; int 0x80` — unprivileged,
            // unconditional, and shipped, because no check had ever issued an
            // unrecognised call number (D-122).
            //
            // The count and the last number are read by the boot report and
            // by the checks, from outside the handler where a lock is legal.
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
    let guard = BOOTSTRAPS.lock_leaf();
    let Some(bootstrap) = guard.get(slot).and_then(Option::as_ref) else {
        return ENOSYS;
    };
    // The program's own claim about what it has reaped, taken once and bounded
    // inside (D-113, D-115).
    let head = bootstrap.ring.completion_head();
    bootstrap.ring.reap_up_to(head);
    bootstrap.ring.serve() as u64
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
        return;
    };
    let which = launch.which;

    // The tables really are this thread's. Checked rather than assumed
    // because everything below writes to addresses that mean something
    // different in another space, and a missed switch would put one program's
    // argument on another's stack.
    if cpu::page_table_base() != launch.root {
        warn!("user {which}: running on tables that are not its own");
        return;
    }

    // One word of argument, on the program's own stack, written across the
    // boundary with the checked copy (D-97) — which is how a program will get
    // everything it is ever given. Sixteen bytes down so the pointer handed to
    // `iretq` is aligned as the ABI wants; the program's first instruction
    // pops it.
    let top = launch.stack + STACK_BYTES - 16;
    if usercopy::copy_to_user(top, &launch.argument.to_le_bytes()).is_err() {
        warn!("user {which}: could not write its argument");
        return;
    }

    info!(
        "user {which}: entering ring 3 at {:#x} with {} on {:#x}",
        launch.entry, launch.argument, launch.root
    );
    let refused = task::enter_user(launch.entry, top);
    warn!("user {which}: refused entry to ring 3: {refused}");
}

/// The first user program, which calls the kernel.
pub(crate) fn run() -> Result<Spawned, SpawnError> {
    spawn("user", 0, BEHAVE)
}

/// A second one, in a space of its own, at its own addresses.
#[allow(dead_code)]
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

/// A third that faults on purpose, to show the machine survives it.
#[allow(dead_code)]
pub(crate) fn run_faulting() -> Result<Spawned, SpawnError> {
    spawn("user-fault", 2, FAULT)
}

/// The argument that makes the program call the kernel.
const BEHAVE: u64 = 0;

/// The argument that makes it execute `ud2` instead.
const FAULT: u64 = 1;
