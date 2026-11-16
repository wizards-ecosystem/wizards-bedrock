use elf64::{Elf, ElfError, SegmentKind};
use wost::arch::current::paging::{AddressSpace, Flush, MapError};
use wost::boot::HandoffError;
use wost::mm::{self, USegment};
use wost::sync::{Leaf, SpinLock};
use x86_paging::{Flags, PAGE_SIZE};

use super::{Regions, STACK_BYTES};
use crate::{info, warn};

pub(super) fn place_stack(space: &mut AddressSpace, at: u64) -> Option<u64> {
    let stack = mm::allocate_contiguous((STACK_BYTES / PAGE_SIZE) as usize, 1)?;
    // Untyped from here: nothing the kernel later reads from the program's stack may be believed.
    let stack = USegment::zeroed(stack)?;
    space
        .map_user(at, stack, Flags::WRITABLE.union(Flags::NO_EXECUTE))
        .ok()?;
    Some(at)
}

static IMAGE: SpinLock<Leaf, Option<&'static [u8]>> = SpinLock::new(None);

#[cfg(feature = "selftest")]
pub(crate) fn image() -> Option<&'static [u8]> {
    *IMAGE.lock_leaf()
}

pub(crate) fn keep(module: Result<&'static [u8], HandoffError>) {
    match module {
        Ok(bytes) => {
            info!("initrd: {} bytes", bytes.len());
            *IMAGE.lock_leaf() = Some(bytes);
        }
        Err(reason) => warn!("initrd: none: {reason}"),
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum LoadError {
    Missing,
    NotAnElf(ElfError),
    Malformed(u64),
    Truncated(u64),
    Overlapping(u64),
    Unaligned(u64),
    InvalidAlignment { at: u64, alignment: u64 },
    TooManySegments,
    WriteExecute(u64),
    NotUserMemory(u64),
    OutOfMemory,
    NotMapped(MapError),
    Rollback { reason: MapError, pages: usize },
    InvalidEntry(u64),
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

pub(super) fn load(space: &mut AddressSpace) -> Result<LoadedImage, LoadError> {
    let bytes = (*IMAGE.lock_leaf()).ok_or(LoadError::Missing)?;
    load_image(space, bytes)
}

#[derive(Debug)]
pub(crate) struct LoadedImage {
    pub(super) entry: u64,
    pub(super) regions: Regions,
}

// Written, then sealed if executable, then mapped, so no frame is ever writable and executable.
fn place(space: &mut AddressSpace, bytes: &[u8], placement: &Placement) -> Result<(), LoadError> {
    let run = mm::allocate_contiguous(placement.pages, 1).ok_or(LoadError::OutOfMemory)?;
    let mut run = USegment::zeroed(run).ok_or(LoadError::OutOfMemory)?;
    let Some(contents) = bytes.get(placement.from..placement.from + placement.len) else {
        return Err(LoadError::Truncated(placement.at));
    };
    if !run.write_at(0, contents) {
        return Err(LoadError::Truncated(placement.at));
    }
    if placement.flags.contains(Flags::NO_EXECUTE) {
        return space
            .map_user(placement.at, run, placement.flags)
            .map_err(|reason| LoadError::NotMapped(reason.reason()));
    }
    let mut flush = Flush::none();
    let sealed = run.seal(&mut flush);
    assert!(
        flush.is_complete(),
        "sealing a program's code left a writable translation"
    );
    space
        .map_user_sealed(placement.at, sealed, placement.flags)
        .map_err(LoadError::NotMapped)
}

fn overlaps(first: &Placement, second: &Placement) -> bool {
    first.at < second.mapped_end && second.at < first.mapped_end
}

pub(super) const MAX_SEGMENTS: usize = 8;

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

pub(crate) fn load_image(space: &mut AddressSpace, bytes: &[u8]) -> Result<LoadedImage, LoadError> {
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

        let at = segment.address();
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

        // Two segments sharing a page would collide in `map_user`, so refuse before taking any.
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

    let entry = elf.entry();
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
            // Validation cannot rule out a frame shortage or a collision: undo what was mapped.
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
