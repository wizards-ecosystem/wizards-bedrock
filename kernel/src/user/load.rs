use elf64::{Elf, ElfError, SegmentKind};
use memory_object::Seal;
use world_tree::NodeRef;
use wost::arch::current::paging::{MapError, UserAccess};
use wost::boot::HandoffError;
use wost::mm::pool::Earmark;
use wost::sync::{Leaf, SpinLock};
use x86_paging::PAGE_SIZE;

use crate::memory::{Kind, MemoryObject, Payer, SealRefusal};
use crate::space::{MapRefusal, Mapping, Space, Target};
use crate::thread::ThreadContext;
use crate::world::{MemoryPool, World};
use crate::{info, warn};

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
    NoQuota(u64),
    NotMapped(MapError),
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
            Self::NoQuota(at) => write!(
                formatter,
                "the code quota cannot cover the segment at {at:#x}"
            ),
            Self::NotMapped(reason) => write!(formatter, "{reason}"),
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

pub(super) fn plan() -> Result<Plan<'static>, LoadError> {
    let bytes = (*IMAGE.lock_leaf()).ok_or(LoadError::Missing)?;
    plan_image(bytes)
}

const MAX_SEGMENTS: usize = 8;

#[derive(Clone, Copy, Debug)]
struct Placement {
    at: u64,
    end: u64,
    mapped_end: u64,
    pages: u32,
    from: usize,
    len: usize,
    access: UserAccess,
}

// A validated image: what each segment needs, known before anything is taken for it.
#[derive(Debug)]
pub(crate) struct Plan<'image> {
    bytes: &'image [u8],
    entry: u64,
    placements: [Option<Placement>; MAX_SEGMENTS],
}

impl Plan<'_> {
    // The arena frames each segment's object and its pool record need.
    pub(super) fn frames(&self) -> Option<u32> {
        self.placements
            .iter()
            .flatten()
            .try_fold(0u32, |sum, placement| {
                sum.checked_add(MemoryPool::frames_for(1)?)?
                    .checked_add(MemoryObject::frames_for(placement.pages)?)
            })
    }

    // Each segment is written into its own object, sealed unless writable, then mapped by
    // record, so no frame is ever writable and executable.
    pub(super) fn load(
        &self,
        context: &mut ThreadContext,
        world_ref: NodeRef,
        world: &World,
        earmark: &mut Earmark,
        space: &mut Space,
    ) -> Result<u64, LoadError> {
        for placement in self.placements.iter().flatten() {
            self.place(context, world_ref, world, earmark, space, placement)?;
        }
        Ok(self.entry)
    }

    fn place(
        &self,
        context: &mut ThreadContext,
        world_ref: NodeRef,
        world: &World,
        earmark: &mut Earmark,
        space: &mut Space,
        placement: &Placement,
    ) -> Result<(), LoadError> {
        let at = placement.at;
        let contents = self
            .bytes
            .get(placement.from..placement.from + placement.len)
            .ok_or(LoadError::Truncated(at))?;
        let object = MemoryObject::new(earmark, Kind::Sealable, placement.pages);
        let written = object.acquire_writer().is_ok() && object.write(context, 0, contents);
        assert!(written, "a fresh segment object refused its contents");
        let sealed = match placement.access {
            UserAccess::Write => Ok(()),
            UserAccess::Read => {
                object.release_writer();
                object.seal(context, Seal::Read, None)
            }
            UserAccess::Execute => {
                object.release_writer();
                let payer = Payer {
                    world: world_ref,
                    quota: &world.code,
                };
                object.seal(context, Seal::Execute, Some(payer))
            }
        };
        match sealed {
            Ok(()) => {}
            Err(SealRefusal::NoQuota) => return Err(LoadError::NoQuota(at)),
            Err(SealRefusal::State(refusal)) => {
                panic!("a segment object with no other reference refused its seal: {refusal}")
            }
        }
        // A writable segment's writer becomes its record's.
        let memory = world.memories.insert(earmark, object);
        let mapping = Mapping {
            access: placement.access,
            target: Target::Object { memory, first: 0 },
        };
        space
            .map(context, world, at, u64::from(placement.pages), mapping)
            .map_err(|refusal| match refusal {
                MapRefusal::Taken => LoadError::Overlapping(at),
                MapRefusal::Full => LoadError::TooManySegments,
                MapRefusal::Frames(reason) => LoadError::NotMapped(reason),
            })
    }
}

fn overlaps(first: &Placement, second: &Placement) -> bool {
    first.at < second.mapped_end && second.at < first.mapped_end
}

pub(crate) fn plan_image(bytes: &[u8]) -> Result<Plan<'_>, LoadError> {
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
        let pages = u32::try_from(mapped_len / PAGE_SIZE).map_err(|_| LoadError::OutOfMemory)?;

        let access = if segment.is_writable() {
            UserAccess::Write
        } else if segment.is_executable() {
            UserAccess::Execute
        } else {
            UserAccess::Read
        };
        let placement = Placement {
            at,
            end,
            mapped_end,
            pages,
            from: segment.offset() as usize,
            len: segment.file_size() as usize,
            access,
        };

        // Two segments sharing a page would collide in their records, so refuse before taking any.
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
        placement.access == UserAccess::Execute && entry >= placement.at && entry < placement.end
    });
    if !entry_is_executable {
        return Err(LoadError::InvalidEntry(entry));
    }

    Ok(Plan {
        bytes,
        entry,
        placements,
    })
}
