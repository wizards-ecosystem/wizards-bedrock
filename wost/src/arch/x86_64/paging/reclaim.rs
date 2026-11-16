use core::ops::Range;

use x86_paging::{ENTRIES, Entry, Level, TablePrivilege};

use super::super::{cpu, task::Released};
use super::{
    AddressSpace, AddressSpaceRoots, MapError, Reach, ReclaimError, ReclaimFailure, TableRoot,
    active_root, entry_slot, is_active, is_user_address,
};
use crate::mm::pool::{ArenaFrame, Earmark, FrameKind};
use crate::mm::{self, PhysicalAddress};

impl AddressSpace {
    pub(super) fn take_owned_table(
        &mut self,
        root: TableRoot,
        virtual_address: u64,
        level: Level,
        privilege: TablePrivilege,
        supply: Option<&mut Earmark>,
    ) -> Result<(PhysicalAddress, Entry), MapError> {
        if level == Level::Pml4
            && !is_user_address(virtual_address)
            && is_active()
            && root == TableRoot::Full
            && self.root() == active_root()
        {
            return Err(MapError::KernelTopologyFrozen(virtual_address));
        }
        if let Some(tables) = supply {
            if self.arena != Some(tables.arena()) {
                return Err(MapError::NotYours(virtual_address));
            }
            let frame = tables.take(FrameKind::Table).ok_or(MapError::OutOfMemory)?;
            let link =
                Entry::table(frame.address().get(), privilege).map_err(MapError::Encoding)?;
            self.arena_tables += 1;
            return Ok((frame.into_link(), link));
        }
        self.tables
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let frame = mm::allocate_table().ok_or(MapError::OutOfMemory)?;
        let address = frame.address();
        let link = Entry::table(address.get(), privilege).map_err(MapError::Encoding)?;
        core::mem::forget(frame);
        self.tables.push(address);
        Ok((address, link))
    }

    pub fn reclaim(&mut self, released: Option<Released>) -> Result<usize, ReclaimFailure> {
        // A repeat could mint an owner for frame zero.
        if self.is_reclaimed() {
            return Err(ReclaimFailure {
                reason: ReclaimError::Reclaimed,
                released,
            });
        }
        let roots = self.roots();
        let root = roots.full_root();
        if let Some(reason) = self.reclaim_refusal(roots, released.as_ref()) {
            return Err(ReclaimFailure { reason, released });
        }

        let returned = self.table_frames();
        for table in self.tables.drain(..) {
            // SAFETY: `take_owned_table` allocated and forgot each for this space, so this rebuilds
            // its one handle as the space empties for good; the refusals prove nothing walks it.
            drop(unsafe { mm::Frame::from_raw(table) });
        }
        // The refusals prove nothing walks them, so each returns zeroed to its arena.
        self.each_arena_table(drop);
        self.arena_tables = 0;
        if self.reach == Reach::UserHalfOnly {
            self.roots = None;
            return Ok(returned);
        }
        if let Some(stripped) = roots.stripped_root() {
            // SAFETY: paired construction allocated and forgot this distinct
            // root exactly once, and the active-root refusal covers it too.
            drop(unsafe { mm::Frame::from_raw(stripped) });
        }
        // SAFETY: construction allocated and forgot the full root exactly once, before anything
        // could record it, and the active-root refusal covers it too.
        drop(unsafe { mm::Frame::from_raw(root) });
        self.roots = None;

        Ok(returned)
    }

    fn reclaim_refusal(
        &self,
        roots: AddressSpaceRoots,
        released: Option<&Released>,
    ) -> Option<ReclaimError> {
        let root = roots.full_root();
        if self.owned_pages != 0 || self.referenced_pages != 0 {
            return Some(ReclaimError::Mapped(
                self.owned_pages + self.referenced_pages,
            ));
        }
        let devices = self.devices.iter().flatten().count();
        if devices != 0 {
            return Some(ReclaimError::Devices(devices));
        }
        if !self.bound
            && let Some(evidence) = released
        {
            return Some(ReclaimError::UnexpectedRelease(
                evidence.roots().full_root(),
            ));
        }
        if self.bound {
            match released {
                None => return Some(ReclaimError::StillBound(root)),
                Some(evidence) if evidence.roots() != roots => {
                    return Some(ReclaimError::WrongRelease {
                        expected: root,
                        found: evidence.roots().full_root(),
                    });
                }
                Some(_) => {}
            }
        }
        if Self::roots_are_active(roots, cpu::page_table_base()) {
            return Some(ReclaimError::Active(cpu::page_table_base()));
        }
        None
    }

    // Rebuilds the token of every arena table once, each after the tables below it: the user half
    // of the full root, the stripped root's higher half (the enclave), then the roots.
    pub(super) fn each_arena_table(&mut self, mut each: impl FnMut(ArenaFrame)) {
        let (Some(roots), Some(arena)) = (self.roots, self.arena) else {
            return;
        };
        let mut found = |table: PhysicalAddress| {
            if ArenaFrame::is_linked_table(table, arena) {
                // SAFETY: the table frames of this arena under these roots were linked by
                // `into_link` for this space alone, each in one slot, and the walk visits each
                // slot once, so this is each frame's only token.
                each(unsafe { ArenaFrame::from_link(table, arena) });
            }
        };
        below(roots.full_root(), 0..ENTRIES / 2, 0, &mut found);
        if let Some(stripped) = roots.stripped_root() {
            below(stripped, ENTRIES / 2..ENTRIES, 0, &mut found);
            found(stripped);
        }
        found(roots.full_root());
        self.arena = None;
    }

    pub const fn is_reclaimed(&self) -> bool {
        self.roots.is_none()
    }

    pub(super) fn roots_are_active(
        roots: AddressSpaceRoots,
        current_root: PhysicalAddress,
    ) -> bool {
        roots.contains(current_root) || (is_active() && roots.contains(active_root()))
    }
}

// Calls `each` for every table linked through `slots` of `table`, a table `depth` levels below a
// root, after the tables below that one.
fn below(
    table: PhysicalAddress,
    slots: Range<usize>,
    depth: u32,
    each: &mut dyn FnMut(PhysicalAddress),
) {
    for index in slots {
        let Ok(slot) = entry_slot(table, index) else {
            continue;
        };
        let entry = slot.read();
        if !entry.is_present() || entry.is_huge() {
            continue;
        }
        let child = PhysicalAddress::from_entry(entry);
        // A page directory's children are page tables, whose entries are leaves.
        if depth < 2 {
            below(child, 0..ENTRIES, depth + 1, each);
        }
        each(child);
    }
}
