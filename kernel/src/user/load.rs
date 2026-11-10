#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use elf64::{Elf, ElfError, SegmentKind};
use wost::arch::current::paging::{AddressSpace, MapError, Unmapped};
use wost::boot::HandoffError;
use wost::mm;
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

use super::{APART, STACK, USER_STACK_BYTES};
use crate::{info, warn};

/// Map the program and its stack, and report where to enter.
///
/// Both pages are user-accessible and neither is both writable and executable:
/// the code page is read-only and executable, the stack is writable and never
/// executable. W^X is not relaxed for user memory — it is the reason user
/// memory exists (D-12).
pub(super) fn place_stack(space: &mut AddressSpace, which: u64) -> Option<u64> {
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

impl core::error::Error for LoadError {}

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
pub(super) fn load(space: &mut AddressSpace, shift: u64) -> Result<LoadedImage, LoadError> {
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
    pub(super) const fn entry(&self) -> u64 {
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
pub(super) static CLEANUP_REFUSAL_INJECTED: AtomicBool = AtomicBool::new(false);

pub(super) fn unmap_for_cleanup(
    space: &mut AddressSpace,
    at: u64,
    len: u64,
) -> Result<Unmapped, MapError> {
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
pub(super) const MAX_SEGMENTS: usize = 8;

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
