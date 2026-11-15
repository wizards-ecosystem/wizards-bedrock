use elf64::{Elf, ElfError, SegmentKind};
use wost::arch::current::paging::{AddressSpace, MapError};
use wost::boot::HandoffError;
use wost::mm;
use wost::sync::SpinLock;
use x86_paging::{Flags, PAGE_SIZE};

use super::{APART, Regions, STACK, STACK_BYTES};
use crate::{info, warn};

/// Map program `which`'s stack, writable and never executable: W^X is not relaxed for user
/// memory, because it is the reason user memory exists.
pub(super) fn place_stack(space: &mut AddressSpace, which: u64) -> Option<u64> {
    let at = STACK + which * APART;
    let mut stack = mm::allocate_contiguous((STACK_BYTES / PAGE_SIZE) as usize, 1)?;
    if !stack.zero() {
        return None;
    }
    // Untyped from here: the program writes this stack, so nothing the kernel later reads out of
    // these frames may be believed.
    space
        .map_user(at, stack.into(), Flags::WRITABLE.union(Flags::NO_EXECUTE))
        .ok()?;
    Some(at)
}

/// Where the boot handoff left the program, kept for the loader.
static IMAGE: SpinLock<Option<&'static [u8]>> = SpinLock::new(None);

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
#[derive(Clone, Copy, Debug)]
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
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Missing => formatter.write_str("no initrd"),
            Self::NotAnElf(reason) => write!(formatter, "{reason}"),
            Self::Malformed(at) => write!(
                formatter,
                "the segment at {at:#x} claims more file than memory"
            ),
            Self::Truncated(at) => write!(formatter, "the segment at {at:#x} runs past the image"),
            Self::Overlapping(at) => write!(formatter, "the segment at {at:#x} overlaps another"),
            Self::Unaligned(at) => write!(formatter, "the segment at {at:#x} is not page-aligned"),
            Self::InvalidAlignment { at, alignment } => write!(
                formatter,
                "the segment at {at:#x} has invalid alignment {alignment:#x}"
            ),
            Self::TooManySegments => {
                formatter.write_str("the image has more segments than the loader places")
            }
            Self::WriteExecute(at) => {
                write!(
                    formatter,
                    "the segment at {at:#x} asks to be writable and executable"
                )
            }
            Self::NotUserMemory(at) => {
                write!(formatter, "the segment at {at:#x} is not user memory")
            }
            Self::OutOfMemory => formatter.write_str("no memory for the program"),
            Self::NotMapped(reason) => write!(formatter, "{reason}"),
            Self::Rollback { reason, pages } => {
                write!(
                    formatter,
                    "loader rollback refused after {pages} page(s): {reason}"
                )
            }
            Self::InvalidEntry(at) => {
                write!(
                    formatter,
                    "the entry point at {at:#x} is not executable program memory"
                )
            }
            Self::Empty => formatter.write_str("the program has nothing to load"),
        }
    }
}

impl core::error::Error for LoadError {}

/// Load the initrd's program into `space`, at `shift` above where it asks to be.
///
/// Every segment is checked before *any* page is taken: inside the image, inside user memory,
/// never both writable and executable, never overlapping another segment's pages. W^X is not
/// relaxed for a program that asks: an image wanting a writable executable page is refused, since
/// one that needs it does not work without it and would only fault later and blame the kernel.
/// Pages are filled before they are mapped, so an executable segment is never writable even for
/// the instant it is being written.
pub(super) fn load(space: &mut AddressSpace, shift: u64) -> Result<LoadedImage, LoadError> {
    let bytes = (*IMAGE.lock_leaf()).ok_or(LoadError::Missing)?;
    load_image(space, bytes, shift)
}

#[derive(Debug)]
pub(crate) struct LoadedImage {
    pub(super) entry: u64,
    pub(super) regions: Regions,
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
        .map_err(|reason| LoadError::NotMapped(reason.reason()))
}

/// Whether two validated placements want any page in common.
fn overlaps(first: &Placement, second: &Placement) -> bool {
    first.at < second.mapped_end && second.at < first.mapped_end
}

/// The most segments this loader will place: a fixed bound because validation holds every
/// placement before the first page is taken, and an image with more segments is refused, never
/// partly loaded.
pub(super) const MAX_SEGMENTS: usize = 8;

/// Where one validated segment goes, and what it is allowed to be.
#[derive(Clone, Copy, Debug)]
struct Placement {
    at: u64,
    end: u64,
    mapped_end: u64,
    pages: usize,
    from: usize,
    len: usize,
    flags: Flags,
}

/// Load `bytes`, which need not be the initrd's, so the checks can exercise every refusal with a
/// doctored image; every placement is preflighted first, so a later refusal cannot abandon
/// earlier user mappings and their frames.
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

        // Checked: a `p_vaddr` near the top wraps past the refusal below, and a debug build
        // would panic on bytes a program supplied.
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

        // Two segments sharing one page would collide in `map_owned` after the first is mapped;
        // caught here, before anything is taken.
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

    let mut regions = Regions::new();
    for placement in placements.iter().flatten() {
        if let Err(reason) = place(space, bytes, placement) {
            // Validation cannot rule out a frame shortage or a collision, so the pass walks
            // back exactly what it mapped.
            if let Err(refusal) = regions.unmap(space) {
                return Err(LoadError::Rollback {
                    reason: refusal.reason,
                    pages: refusal.pages,
                });
            }
            return Err(reason);
        }
        assert!(
            regions.record(placement.at, placement.mapped_end - placement.at),
            "the region ledger holds fewer ranges than the loader places"
        );
    }

    Ok(LoadedImage { entry, regions })
}
