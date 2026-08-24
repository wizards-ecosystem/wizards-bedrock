//! The first thing that runs in ring 3.
//!
//! Not a program and not a process: a page of instructions, a page of stack,
//! and a thread that stops being the kernel's. What it proves is the boundary
//! — that the descriptors, the gate, the privilege stack and the return path
//! all agree — which is the thing every later slice builds on and the thing
//! that is hardest to debug once there is a loader in the way (D-96).
//!
//! It is deliberately not an ELF. Slice 7 brings `libs/elf64`, an initrd and
//! `spawn`; this is the layer underneath, and keeping them apart means a
//! failure here is a failure of the boundary rather than of a parser.

use elf64::{Elf, ElfError, SegmentKind};
use wost::arch::current::paging::MapError;
use wost::arch::current::task;
use wost::arch::current::usercopy;
use wost::boot::HandoffError;
use wost::mm;
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

use crate::thread::ThreadContext;
use crate::{info, mem, warn};

/// Where the first user program's page of code goes.
///
/// Low half, far from anything the kernel maps: an address a user mapping can
/// occupy is by definition one the kernel does not, and the split is bit 63
/// (`010` §10's LASS note).
pub(crate) const CODE: u64 = 0x0000_4000_0000_0000;

/// Where its stack goes, well away from the code so a run-off hits nothing.
pub(crate) const STACK: u64 = 0x0000_3000_0000_0000;

/// How far apart two programs' pages sit.
///
/// They share one address space until slice 7 gives each a table of its own,
/// so for now "another program" means "another pair of addresses". That is not
/// isolation and is not claimed to be — what the second one proves is that
/// `RSP0` follows the thread (D-99).
const APART: u64 = 0x1_0000;

/// Bytes of stack, which is one page: it makes one call and never returns.
const STACK_BYTES: u64 = PAGE_SIZE;

/// What the program's first instructions are, for the check that reads them
/// back across the boundary.
///
/// `int 0x80`, then a jump to itself. Written by `xtask` into the image
/// (D-100) rather than by the kernel, so this is a statement about the initrd
/// and not a copy of something the kernel placed.
pub(crate) const PROGRAM: [u8; 4] = [0xCD, 0x80, 0xEB, 0xFE];

/// Map the program and its stack, and report where to enter.
///
/// Both pages are user-accessible and neither is both writable and executable:
/// the code page is read-only and executable, the stack is writable and never
/// executable. W^X is not relaxed for user memory — it is the reason user
/// memory exists (D-12).
fn place_stack(context: &mut ThreadContext, which: u64) -> Option<u64> {
    let at = STACK + which * APART;
    let mut stack = mm::allocate_contiguous((STACK_BYTES / PAGE_SIZE) as usize, 1)?;
    if !stack.zero() {
        return None;
    }
    mem::with_active_space(context, |space| {
        space.map_owned(
            at,
            stack,
            Flags::USER.union(Flags::WRITABLE).union(Flags::NO_EXECUTE),
        )
    })?
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
    /// A segment's contents run past the image, or claim more file than
    /// memory.
    Malformed(u64),
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
            Self::Malformed(at) => write!(f, "the segment at {at:#x} does not fit its image"),
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
/// Every segment is checked before a page is taken: inside the image, inside
/// user memory, and never both writable and executable. **W^X is not relaxed
/// for a program that asks nicely** — an image wanting a writable executable
/// page is refused, because an image that needs one does not work without it
/// and pretending otherwise loads something that will fault later and blame
/// the kernel (D-100).
///
/// The pages are filled *before* they are mapped, so a segment that is
/// executable is never writable even for the instant it is being written.
fn load(context: &mut ThreadContext, shift: u64) -> Result<u64, LoadError> {
    let bytes = (*IMAGE.lock_leaf()).ok_or(LoadError::Missing)?;
    load_image(context, bytes, shift)
}

/// Load `bytes`, which need not be the initrd's.
///
/// Split out so the checks can hand it a doctored image: the refusals below
/// are the load-bearing half of a loader, and a refusal that is never
/// exercised is a claim (D-100).
pub(crate) fn load_image(
    context: &mut ThreadContext,
    bytes: &[u8],
    shift: u64,
) -> Result<u64, LoadError> {
    let elf = Elf::parse(bytes).map_err(LoadError::NotAnElf)?;

    let mut loaded = 0usize;
    for segment in elf
        .segments()
        .filter(|segment| segment.kind() == SegmentKind::Load)
    {
        if !segment.contents_fit(bytes.len()) || segment.is_oversized() {
            return Err(LoadError::Malformed(segment.address()));
        }
        if segment.is_write_execute() {
            return Err(LoadError::WriteExecute(segment.address()));
        }

        let at = segment.address() + shift;
        let end = at
            .checked_add(segment.memory_size())
            .ok_or(LoadError::NotUserMemory(at))?;
        if end > usercopy::USER_END || segment.memory_size() == 0 {
            return Err(LoadError::NotUserMemory(at));
        }

        let pages = segment.memory_size().div_ceil(PAGE_SIZE) as usize;
        let mut run = mm::allocate_contiguous(pages, 1).ok_or(LoadError::OutOfMemory)?;
        if !run.zero() {
            return Err(LoadError::OutOfMemory);
        }
        let from = segment.offset() as usize;
        let len = segment.file_size() as usize;
        if !run.write_at(0, &bytes[from..from + len]) {
            return Err(LoadError::Malformed(segment.address()));
        }

        let mut flags = Flags::USER;
        if segment.is_writable() {
            flags = flags.union(Flags::WRITABLE);
        }
        if !segment.is_executable() {
            flags = flags.union(Flags::NO_EXECUTE);
        }
        mem::with_active_space(context, |space| space.map_owned(at, run, flags))
            .ok_or(LoadError::OutOfMemory)?
            .map_err(LoadError::NotMapped)?;
        loaded += 1;
    }

    if loaded == 0 {
        return Err(LoadError::Empty);
    }
    Ok(elf.entry() + shift)
}

/// Cross into ring 3, and never come back on this thread.
///
/// Runs as an ordinary kernel thread until its last instruction, and is retired
/// by the syscall handler from the far side of the boundary.
///
/// No stack is arranged for the trap out: the context switch that scheduled
/// this thread already pointed `RSP0` at its own kernel stack, and will point
/// it somewhere else the moment another thread runs (D-99). That is what lets
/// there be more than one of these.
fn cross(which: u64) {
    let mut context = ThreadContext::require();

    // The program comes off the initrd through the same reader `xtask` builds
    // it with (D-100); its stack is the kernel's to provide, because an image
    // does not carry one.
    let entry = match load(&mut context, which * APART) {
        Ok(entry) => entry,
        Err(reason) => {
            warn!("user {which}: could not load the program: {reason}");
            return;
        }
    };
    let Some(stack) = place_stack(&mut context, which) else {
        warn!("user {which}: could not map a stack");
        return;
    };

    info!("user {which}: entering ring 3 at {entry:#x}");
    let refused = task::enter_user(entry, stack + STACK_BYTES);
    warn!("user {which}: refused entry to ring 3: {refused}");
}

/// The first user program.
pub(crate) fn run() {
    cross(0);
}

/// A second one, at its own addresses, to show `RSP0` moves with the thread.
pub(crate) fn run_second() {
    cross(1);
}
