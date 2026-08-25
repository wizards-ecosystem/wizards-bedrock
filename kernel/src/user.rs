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

use elf64::{Elf, ElfError, SegmentKind};
use handle_space::Rights;
use wost::arch::current::cpu;
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
fn load(space: &mut AddressSpace, shift: u64) -> Result<u64, LoadError> {
    let bytes = (*IMAGE.lock_leaf()).ok_or(LoadError::Missing)?;
    load_image(space, bytes, shift)
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
struct Bootstrap {
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
    let entry = load(&mut space, which * APART).map_err(SpawnError::Load)?;
    let stack = place_stack(&mut space, which).ok_or(SpawnError::NoStack)?;

    // Its ring, at the same address every program's is — which is what having
    // a space of its own buys. The pages stay the kernel's: `map_shared`
    // writes no owned bit, so a program cannot free the memory a completion
    // will be written into (D-113). Mapped before the binding, because the
    // binding consumes the space.
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
pub(crate) fn run_second() -> Result<Spawned, SpawnError> {
    spawn("user-two", 1, BEHAVE)
}

/// A fourth, for the check that counts what a program is given at birth.
#[cfg(feature = "selftest")]
pub(crate) fn run_for_bootstrap() -> Result<Spawned, SpawnError> {
    spawn("user-bootstrap", 3, BEHAVE)
}

/// A third that faults on purpose, to show the machine survives it.
pub(crate) fn run_faulting() -> Result<Spawned, SpawnError> {
    spawn("user-fault", 2, FAULT)
}

/// The argument that makes the program call the kernel.
const BEHAVE: u64 = 0;

/// The argument that makes it execute `ud2` instead.
const FAULT: u64 = 1;
