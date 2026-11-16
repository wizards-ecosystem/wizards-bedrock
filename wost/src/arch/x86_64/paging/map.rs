use core::mem::ManuallyDrop;

use x86_paging::kpti::shared_user_link_matches;
use x86_paging::{
    ENTRIES, Entry, Flags, LARGE_PAGE_SIZE, Level, MappingRange, PAGE_SIZE, TablePrivilege,
    user_mapping_fits,
};

use super::{
    AddressSpace, CACHE_TYPE, EntrySlot, MapError, NOT_THE_CALLERS, Reach, TableRoot, UserMapError,
    clear_mapping_slot, entry_slot, is_user_address, leaf_in, protected_stack_overlap,
    ram_cache_policy, slot_in,
};
use crate::mm::{
    self, PhysicalAddress, Segment, SharedBytes, SharedKeepalive, SharedRing, USegment,
};

impl AddressSpace {
    pub fn unmap(&mut self, virtual_base: u64, len: u64) -> Result<Unmapped, MapError> {
        if !virtual_base.is_multiple_of(PAGE_SIZE) || len == 0 || !len.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Misaligned(virtual_base));
        }
        // Both ends: a walk masks bits 48 and up, so a noncanonical alias could not be invalidated.
        let end = virtual_base
            .checked_add(len)
            .ok_or(MapError::NonCanonical(virtual_base))?;
        if !x86_paging::is_canonical(virtual_base) || !x86_paging::is_canonical(end - 1) {
            return Err(MapError::NonCanonical(virtual_base));
        }
        self.may_reach(virtual_base, len)?;
        if let Some(pinned) = protected_stack_overlap(self.roots(), virtual_base, len) {
            return Err(MapError::Pinned(pinned));
        }
        let pages = len / PAGE_SIZE;
        let remaining_owned = self
            .owned_pages
            .checked_sub(pages as usize)
            .ok_or(MapError::NotOwned(virtual_base))?;

        // Preflight the whole range before clearing anything, so a refusal changes nothing.
        let mut first = None;
        let mut was_user = false;
        for page in 0..pages {
            let address = virtual_base
                .checked_add(page * PAGE_SIZE)
                .ok_or(MapError::NonCanonical(virtual_base))?;
            let slot = leaf_in(self.root(), address)?;
            let entry = slot.read();
            if !entry.flags().contains(Flags::OWNED) {
                return Err(MapError::NotOwned(address));
            }
            // One user-reachable page makes the whole run user bytes.
            was_user |= entry.flags().contains(Flags::USER);
            match first {
                None => first = Some(PhysicalAddress::from_entry(entry)),
                Some(base) if entry.address() == base.get() + page * PAGE_SIZE => {}
                Some(_) => return Err(MapError::Discontiguous(address)),
            }
        }
        let base = first.ok_or(MapError::NotMapped(virtual_base))?;

        for page in 0..pages {
            let address = virtual_base + page * PAGE_SIZE;
            let slot = leaf_in(self.root(), address)?;
            clear_mapping_slot(slot, address, Level::Pt);
        }
        self.owned_pages = remaining_owned;

        Ok(Unmapped {
            pages,
            // SAFETY: preflight proved the `pages` frames at `base` one ascending run of owned leaves,
            // each frame moved here by a `map_owned` and named by no other handle, and those leaves
            // were just cleared; `Segment` frees frame by frame, so a sub-run or merged run is exact.
            frames: ManuallyDrop::new(unsafe { Segment::from_raw(base, pages as usize) }),
            was_user,
        })
    }

    pub(super) fn map_range(
        &mut self,
        virtual_base: u64,
        physical_base: u64,
        len: u64,
        flags: Flags,
        grain: Grain,
    ) -> Result<(), MapError> {
        let range = MappingRange::new(virtual_base, physical_base, len)?;
        let flags = flags.union(Flags::PRESENT);

        // A pair shares lower tables with its stripped root, so a supervisor leaf leaks to ring 3.
        if self.roots().is_paired() && is_user_address(virtual_base) && !flags.contains(Flags::USER)
        {
            return Err(MapError::NotYours(virtual_base));
        }

        if flags.is_write_execute() {
            return Err(MapError::WriteExecute(virtual_base));
        }

        if let Some(stack) = protected_stack_overlap(self.roots(), virtual_base, len) {
            return Err(MapError::Pinned(stack));
        }

        self.preflight(&range, flags, grain)?;

        let table_mark = self.tables.len();
        let mut offset = 0;
        while offset < range.len() {
            let (virtual_address, physical, level) = Self::step(&range, offset, grain);
            if let Err(reason) = self.set(TableRoot::Full, virtual_address, physical, flags, level)
            {
                self.undo(&range, offset, grain);
                self.rollback_tables(table_mark);
                return Err(reason);
            }
            offset += level.span();
        }
        Ok(())
    }

    // Every target must be absent, so rollback owns every leaf the transaction writes.
    fn preflight(&self, range: &MappingRange, flags: Flags, grain: Grain) -> Result<(), MapError> {
        let mut offset = 0;
        while offset < range.len() {
            let (virtual_address, physical, level) = Self::step(range, offset, grain);
            self.verify_shared_lower_link(virtual_address)?;
            self.verify_absent(virtual_address, physical, flags, level)?;
            offset += level.span();
        }
        Ok(())
    }

    fn verify_absent(
        &self,
        virtual_address: u64,
        physical: u64,
        flags: Flags,
        target: Level,
    ) -> Result<(), MapError> {
        Entry::page(target, physical, flags).map_err(MapError::Encoding)?;
        let mut table = self.root();
        for level in Level::WALK {
            let slot = entry_slot(table, level.index(virtual_address))?;
            let current = slot.read();
            if level == target {
                return if current.is_present() {
                    Err(MapError::Conflict(virtual_address))
                } else {
                    Ok(())
                };
            }
            if !current.is_present() {
                return Ok(());
            }
            if current.is_huge()
                || flags.contains(Flags::USER) && !current.flags().contains(Flags::USER)
            {
                return Err(MapError::Conflict(virtual_address));
            }
            table = PhysicalAddress::from_entry(current);
        }
        Err(MapError::Conflict(virtual_address))
    }

    pub(super) fn may_reach(&self, at: u64, len: u64) -> Result<(), MapError> {
        // A reclaimed space's tables went back, so mapping would walk frames owned elsewhere.
        if self.is_reclaimed() {
            return Err(MapError::NotYours(at));
        }
        if self.reach == Reach::Everywhere {
            return Ok(());
        }
        at.checked_add(len).ok_or(MapError::NonCanonical(at))?;
        if !user_mapping_fits(at, len) {
            return Err(MapError::NotYours(at));
        }
        Ok(())
    }

    // `undo` retraces this exactly; a rollback that disagreed would clear the wrong entry.
    fn step(range: &MappingRange, offset: u64, grain: Grain) -> (u64, u64, Level) {
        let virtual_address = range.virtual_base() + offset;
        let physical = range.physical_base() + offset;
        let large = grain == Grain::Largest
            && range.len() - offset >= LARGE_PAGE_SIZE
            && virtual_address.is_multiple_of(LARGE_PAGE_SIZE)
            && physical.is_multiple_of(LARGE_PAGE_SIZE);
        (
            virtual_address,
            physical,
            if large { Level::Pd } else { Level::Pt },
        )
    }

    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the leaves are cleared through the direct map in tables this borrow owns"
    )]
    pub(super) fn undo(&mut self, range: &MappingRange, written: u64, grain: Grain) {
        let mut offset = 0;
        while offset < written {
            let (virtual_address, _, level) = Self::step(range, offset, grain);
            let slot = self
                .slot_at(virtual_address, level)
                .unwrap_or_else(|reason| panic!("mapping rollback lost a written leaf: {reason}"));
            clear_mapping_slot(slot, virtual_address, level);
            offset += level.span();
        }
    }

    pub(super) fn rollback_tables(&mut self, mark: usize) {
        assert!(
            mark <= self.tables.len(),
            "mapping rollback lost its table mark"
        );
        while self.tables.len() > mark {
            let owned = *self
                .tables
                .last()
                .unwrap_or_else(|| panic!("mapping rollback lost an owned table"));
            for index in 0..ENTRIES {
                let slot = entry_slot(owned.frame, index).unwrap_or_else(|reason| {
                    panic!("mapping rollback could not inspect an owned table: {reason}")
                });
                let entry = slot.read();
                assert!(
                    entry == Entry::ABSENT,
                    "mapping rollback tried to return a nonempty table"
                );
            }
            let parent = match owned.root {
                TableRoot::Full => self.slot_at(owned.virtual_address, owned.level),
                TableRoot::Stripped => self
                    .roots()
                    .stripped_root()
                    .ok_or(MapError::NotMapped(owned.virtual_address))
                    .and_then(|root| slot_in(root, owned.virtual_address, owned.level)),
            }
            .unwrap_or_else(|reason| panic!("mapping rollback lost an owned table link: {reason}"));
            let current = parent.read();
            assert!(
                owned.link.table_identity_matches(current),
                "mapping rollback found a different owned table link"
            );
            let paired = if owned.root == TableRoot::Full && owned.level == Level::Pml4 {
                self.stripped_lower_slot(owned.virtual_address)
                    .unwrap_or_else(|reason| {
                        panic!("mapping rollback lost its paired table link: {reason}")
                    })
            } else {
                None
            };
            if let Some(paired) = paired.as_ref() {
                assert!(
                    shared_user_link_matches(current, paired.read()),
                    "mapping rollback found a divergent paired table link"
                );
            }
            clear_mapping_slot(parent, owned.virtual_address, owned.level);
            if let Some(paired) = paired {
                clear_mapping_slot(paired, owned.virtual_address, owned.level);
            }
            // SAFETY: `take_owned_table` forgot exactly this allocation, and
            // the only page-table link to it is now absent.
            drop(unsafe { mm::Frame::from_raw(owned.frame) });
            self.tables.pop();
        }
    }

    fn slot_at(&self, virtual_address: u64, target: Level) -> Result<EntrySlot, MapError> {
        let mut table = self.root();
        for level in Level::WALK {
            let slot = entry_slot(table, level.index(virtual_address))?;
            if level == target {
                return Ok(slot);
            }
            let entry = slot.read();
            if !entry.is_present() || entry.is_huge() {
                return Err(MapError::NotMapped(virtual_address));
            }
            table = PhysicalAddress::from_entry(entry);
        }
        Err(MapError::NotMapped(virtual_address))
    }

    fn stripped_lower_slot(&self, virtual_address: u64) -> Result<Option<EntrySlot>, MapError> {
        if !is_user_address(virtual_address) {
            return Ok(None);
        }
        self.roots()
            .stripped_root()
            .map(|root| entry_slot(root, Level::Pml4.index(virtual_address)))
            .transpose()
    }

    fn verify_shared_lower_link(&self, virtual_address: u64) -> Result<(), MapError> {
        let Some(stripped) = self.stripped_lower_slot(virtual_address)? else {
            return Ok(());
        };
        let full = entry_slot(self.root(), Level::Pml4.index(virtual_address))?;
        if !shared_user_link_matches(full.read(), stripped.read()) {
            return Err(MapError::Conflict(virtual_address));
        }
        Ok(())
    }

    pub fn map_boot(
        &mut self,
        virtual_base: u64,
        region: BootRegion,
        flags: Flags,
    ) -> Result<(), MapError> {
        if flags.contains(Flags::USER) {
            return Err(MapError::NotYours(virtual_base));
        }
        if flags.intersection(CACHE_TYPE) != region.cache_type() {
            return Err(MapError::CacheAlias(virtual_base));
        }
        self.may_reach(virtual_base, region.len)?;
        // Memory outside the kernel image is reused, so an executable alias would outlive it.
        if !flags.contains(Flags::NO_EXECUTE) && !mm::is_kernel_image(region.base.get(), region.len)
        {
            return Err(MapError::WriteExecute(virtual_base));
        }
        self.map_range(
            virtual_base,
            region.base.get(),
            region.len,
            flags.difference(NOT_THE_CALLERS),
            Grain::Largest,
        )
    }

    pub fn map_owned(
        &mut self,
        virtual_base: u64,
        run: Segment,
        flags: Flags,
    ) -> Result<(), MapError> {
        // A kernel run holds kernel bytes, so ring 3 never maps it; `map_user` takes user bytes.
        if flags.contains(Flags::USER) {
            return Err(MapError::NotYours(virtual_base));
        }
        self.owned_pages =
            self.map_owned_frames(virtual_base, run.address(), run.len(), run.frames(), flags)?;
        core::mem::forget(run);
        Ok(())
    }

    pub fn map_user(
        &mut self,
        virtual_base: u64,
        run: USegment,
        flags: Flags,
    ) -> Result<(), UserMapError> {
        if !user_mapping_fits(virtual_base, run.len()) {
            return Err(UserMapError::Mapping {
                reason: MapError::NotYours(virtual_base),
                run,
            });
        }
        match self.map_owned_frames(
            virtual_base,
            run.address(),
            run.len(),
            run.frames(),
            flags.union(Flags::USER),
        ) {
            Ok(owned_pages) => {
                run.commit_mapping();
                self.owned_pages = owned_pages;
                Ok(())
            }
            Err(reason) => Err(UserMapError::Mapping { reason, run }),
        }
    }

    pub fn map_shared(
        &mut self,
        virtual_base: u64,
        run: &SharedBytes,
        flags: Flags,
    ) -> Result<(), MapError> {
        self.map_shared_allocation(
            virtual_base,
            run.address(),
            run.len(),
            run.keepalive(),
            flags,
        )
    }

    pub fn map_shared_ring<const WORDS: usize>(
        &mut self,
        virtual_base: u64,
        run: &SharedRing<WORDS>,
        flags: Flags,
    ) -> Result<(), MapError> {
        self.map_shared_allocation(
            virtual_base,
            run.address(),
            run.len(),
            run.keepalive(),
            flags,
        )
    }

    fn map_shared_allocation(
        &mut self,
        virtual_base: u64,
        physical_base: PhysicalAddress,
        len: u64,
        keepalive: SharedKeepalive,
        flags: Flags,
    ) -> Result<(), MapError> {
        ram_cache_policy(virtual_base, flags)?;
        if !user_mapping_fits(virtual_base, len) {
            return Err(MapError::NotYours(virtual_base));
        }
        self.may_reach(virtual_base, len)?;
        if flags.contains(Flags::WRITABLE) && !flags.contains(Flags::NO_EXECUTE) {
            return Err(MapError::WriteExecute(virtual_base));
        }
        self.shared
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        // Shared bytes are never executed: another alias may write them.
        self.map_range(
            virtual_base,
            physical_base.get(),
            len,
            flags
                .difference(NOT_THE_CALLERS)
                .union(Flags::USER)
                .union(Flags::NO_EXECUTE),
            Grain::Pages,
        )?;
        self.shared.push(keepalive);
        Ok(())
    }

    fn map_owned_frames(
        &mut self,
        virtual_base: u64,
        physical_base: PhysicalAddress,
        len: u64,
        frames: usize,
        flags: Flags,
    ) -> Result<usize, MapError> {
        ram_cache_policy(virtual_base, flags)?;
        self.may_reach(virtual_base, len)?;
        let owned_pages = self
            .owned_pages
            .checked_add(frames)
            .ok_or(MapError::OutOfMemory)?;
        // An owned large page could not be split back, so owned mappings use 4 KiB pages.
        self.map_range(
            virtual_base,
            physical_base.get(),
            len,
            flags.difference(NOT_THE_CALLERS).union(Flags::OWNED),
            Grain::Pages,
        )?;
        Ok(owned_pages)
    }

    pub(super) fn set(
        &mut self,
        root: TableRoot,
        virtual_address: u64,
        physical: u64,
        flags: Flags,
        target: Level,
    ) -> Result<(), MapError> {
        let mut table = match root {
            TableRoot::Full => self.root(),
            TableRoot::Stripped => self
                .roots()
                .stripped_root()
                .ok_or(MapError::InvalidEntryEnclave(virtual_address))?,
        };

        for level in Level::WALK {
            let slot = entry_slot(table, level.index(virtual_address))?;

            if level == target {
                let entry = Entry::page(target, physical, flags).map_err(MapError::Encoding)?;
                if slot.read().is_present() {
                    return Err(MapError::Conflict(virtual_address));
                }
                slot.write(entry);
                return Ok(());
            }

            let current = slot.read();
            let paired = if root == TableRoot::Full && level == Level::Pml4 {
                self.stripped_lower_slot(virtual_address)?
            } else {
                None
            };
            if let Some(paired) = paired.as_ref()
                && !shared_user_link_matches(current, paired.read())
            {
                return Err(MapError::Conflict(virtual_address));
            }
            table = if current.is_present() {
                if current.is_huge() {
                    return Err(MapError::Conflict(virtual_address));
                }
                // Upgrading an interior entry to `USER` would hand ring 3 every mapping under it.
                if flags.contains(Flags::USER) && !current.flags().contains(Flags::USER) {
                    return Err(MapError::Conflict(virtual_address));
                }
                PhysicalAddress::from_entry(current)
            } else {
                let privilege = if paired.is_some() || flags.contains(Flags::USER) {
                    TablePrivilege::User
                } else {
                    TablePrivilege::Supervisor
                };
                let (frame, link) =
                    self.take_owned_table(root, virtual_address, level, privilege)?;
                slot.write(link);
                if let Some(paired) = paired.as_ref() {
                    paired.write(link);
                }
                frame
            };
        }

        // Unreachable: `target` is `Pt` or `Pd`, both in `Level::WALK`.
        Err(MapError::Conflict(virtual_address))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Grain {
    Largest,
    Pages,
}

// Not `Copy`: a copy would let a caller reuse the evidence for a range nobody checked.
#[derive(Debug)]
pub struct BootRegion {
    base: PhysicalAddress,
    len: u64,
    cache: crate::boot::DirectMapCache,
}

impl BootRegion {
    // Device holes are refused because write-back MMIO aliases are unsafe.
    pub fn describe(base: u64, len: u64) -> Option<Self> {
        let end = base.checked_add(len)?;
        if len == 0 || !base.is_multiple_of(PAGE_SIZE) {
            return None;
        }
        let cache = mm::map_direct_cache(base, end)?;
        Some(Self {
            base: PhysicalAddress::new(base)?,
            len,
            cache,
        })
    }

    const fn cache_type(&self) -> Flags {
        match self.cache {
            crate::boot::DirectMapCache::Normal => Flags::NONE,
            crate::boot::DirectMapCache::Framebuffer => Flags::WRITE_THROUGH,
        }
    }
}

#[derive(Debug)]
#[must_use = "the detached frames remain held until this token is consumed or dropped"]
pub struct Unmapped {
    pages: u64,
    frames: ManuallyDrop<Segment>,
    was_user: bool,
}

impl Unmapped {
    pub const fn pages(&self) -> u64 {
        self.pages
    }
}

impl Drop for Unmapped {
    fn drop(&mut self) {
        // A failed clear leaks the frames so no user bytes are disclosed.
        if self.was_user && !self.frames.zero() {
            return;
        }
        // SAFETY: this is the one drop of `frames`, and nothing reads the field after it.
        unsafe { ManuallyDrop::drop(&mut self.frames) };
    }
}
