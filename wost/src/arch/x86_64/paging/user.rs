use x86_paging::{Flags, Level, PAGE_SIZE, user_mapping_fits};

use super::map::Grain;
use super::{
    AddressSpace, Flush, MapError, NOT_THE_CALLERS, Reach, clear_mapping_slot, leaf_in,
    protected_stack_overlap,
};
use crate::mm::PhysicalAddress;
use crate::mm::pool::{ArenaFrame, FrameKind, Mappings, SealedFrame};

#[derive(Clone, Copy, Debug)]
pub enum FrameRef<'frame> {
    Writable(&'frame ArenaFrame),
    Sealed(&'frame SealedFrame),
}

impl<'frame> FrameRef<'frame> {
    const fn address(self) -> PhysicalAddress {
        match self {
            Self::Writable(frame) => frame.address(),
            Self::Sealed(frame) => frame.address(),
        }
    }

    const fn kind(self) -> FrameKind {
        match self {
            Self::Writable(frame) => frame.kind(),
            Self::Sealed(frame) => frame.kind(),
        }
    }

    const fn mappings(self) -> &'frame Mappings {
        match self {
            Self::Writable(frame) => frame.mappings(),
            Self::Sealed(frame) => frame.mappings(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UserAccess {
    Read,
    Write,
    Execute,
}

impl AddressSpace {
    // A writable frame is never executable and a sealed one never writable; each leaf is counted
    // in the frame's token, so the frame cannot leave it while ring 3 can reach it.
    pub fn map_frame(
        &mut self,
        at: u64,
        frame: FrameRef<'_>,
        access: UserAccess,
    ) -> Result<(), MapError> {
        let flags = match (frame, access) {
            (FrameRef::Writable(_), UserAccess::Execute)
            | (FrameRef::Sealed(_), UserAccess::Write) => {
                return Err(MapError::WriteExecute(at));
            }
            (_, UserAccess::Read) => Flags::USER.union(Flags::NO_EXECUTE),
            (_, UserAccess::Write) => Flags::USER.union(Flags::NO_EXECUTE).union(Flags::WRITABLE),
            (_, UserAccess::Execute) => Flags::USER,
        };
        // Only data frames back no kernel value, so only they may reach ring 3.
        if frame.kind() != FrameKind::ArenaData
            || self.reach != Reach::UserHalfOnly
            || !user_mapping_fits(at, PAGE_SIZE)
        {
            return Err(MapError::NotYours(at));
        }
        self.may_reach(at, PAGE_SIZE)?;
        let referenced = self
            .referenced_pages
            .checked_add(1)
            .ok_or(MapError::OutOfMemory)?;
        let writable = access == UserAccess::Write;
        if !frame.mappings().add(writable) {
            return Err(MapError::OutOfMemory);
        }
        // Counted first, so no moment exists where a leaf names a frame its token does not count.
        if let Err(reason) = self.map_range(
            at,
            frame.address().get(),
            PAGE_SIZE,
            flags.difference(NOT_THE_CALLERS),
            Grain::Pages,
        ) {
            frame.mappings().remove(writable);
            return Err(reason);
        }
        self.referenced_pages = referenced;
        Ok(())
    }

    // Clears only a leaf `map_frame` wrote for this frame; the caller waits for the flush, and the
    // frame's own token waits again before it is sealed or freed.
    pub fn clear_frame(&mut self, at: u64, frame: FrameRef<'_>) -> Result<Flush, MapError> {
        if !at.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Misaligned(at));
        }
        if self.reach != Reach::UserHalfOnly || !user_mapping_fits(at, PAGE_SIZE) {
            return Err(MapError::NotYours(at));
        }
        self.may_reach(at, PAGE_SIZE)?;
        if let Some(pinned) = protected_stack_overlap(self.roots(), at, PAGE_SIZE) {
            return Err(MapError::Pinned(pinned));
        }
        let slot = leaf_in(self.root(), at)?;
        let entry = slot.read();
        let flags = entry.flags();
        if !entry.is_present()
            || flags.contains(Flags::OWNED)
            || !flags.contains(Flags::USER)
            || entry.address() != frame.address().get()
        {
            return Err(MapError::NotMapped(at));
        }
        let remaining = self
            .referenced_pages
            .checked_sub(1)
            .ok_or(MapError::NotMapped(at))?;
        let flush = clear_mapping_slot(slot, at, Level::Pt, self.roots());
        frame.mappings().remove(flags.contains(Flags::WRITABLE));
        self.referenced_pages = remaining;
        Ok(flush)
    }
}
