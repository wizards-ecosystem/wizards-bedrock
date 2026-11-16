use core::mem::ManuallyDrop;

use x86_paging::kpti::shared_user_link_matches;
use x86_paging::{
    Entry, Flags, LARGE_PAGE_SIZE, Level, MappingRange, PAGE_SIZE, TablePrivilege,
    user_mapping_fits,
};

use super::{
    AddressSpace, CACHE_TYPE, EntrySlot, Flush, MapError, NOT_THE_CALLERS, Reach, TableRoot,
    UserMapError, clear_mapping_slot, entry_slot, is_active, is_user_address, leaf_in,
    protected_stack_overlap, ram_cache_policy,
};
use crate::mm::pool::Earmark;
use crate::mm::{self, PhysicalAddress, SealedSegment, Segment, USegment};

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
        let mut sealed = None;
        for page in 0..pages {
            let address = virtual_base
                .checked_add(page * PAGE_SIZE)
                .ok_or(MapError::NonCanonical(virtual_base))?;
            let slot = leaf_in(self.root(), address)?;
            let entry = slot.read();
            if !entry.flags().contains(Flags::OWNED) {
                return Err(MapError::NotOwned(address));
            }
            let frame = PhysicalAddress::from_entry(entry);
            // A run returns through one release, sealed or writable, so its frames must agree.
            let frame_sealed = mm::is_sealed(frame);
            if *sealed.get_or_insert(frame_sealed) != frame_sealed {
                return Err(MapError::Discontiguous(address));
            }
            match first {
                None => first = Some(frame),
                Some(base) if entry.address() == base.get() + page * PAGE_SIZE => {}
                Some(_) => return Err(MapError::Discontiguous(address)),
            }
        }
        let base = first.ok_or(MapError::NotMapped(virtual_base))?;

        let mut flush = Flush::none();
        for page in 0..pages {
            let address = virtual_base + page * PAGE_SIZE;
            let slot = leaf_in(self.root(), address)?;
            flush.merge(clear_mapping_slot(slot, address, Level::Pt, self.roots()));
        }
        self.owned_pages = remaining_owned;

        Ok(Unmapped {
            pages,
            // SAFETY: preflight proved the `pages` frames at `base` an ascending run of owned leaves;
            // each is allocated, its handle was forgotten when it was mapped, and no other handle
            // names it now that its leaf is cleared.
            frames: ManuallyDrop::new(unsafe { Segment::from_raw(base, pages as usize) }),
            sealed: sealed == Some(true),
            flush,
        })
    }

    pub(super) fn map_range(
        &mut self,
        virtual_base: u64,
        physical_base: u64,
        len: u64,
        flags: Flags,
        grain: Grain,
        mut supply: Option<&mut Earmark>,
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

        // Tables first: a refusal then leaves only empty tables, kept until reclaim, and no leaf
        // another processor could have cached, so nothing needs a flush.
        let mut offset = 0;
        while offset < range.len() {
            let (virtual_address, _, level) = Self::step(&range, offset, grain);
            self.table_for(
                TableRoot::Full,
                virtual_address,
                flags,
                level,
                supply.as_deref_mut(),
            )?;
            offset += level.span();
        }
        offset = 0;
        while offset < range.len() {
            let (virtual_address, physical, level) = Self::step(&range, offset, grain);
            if let Err(reason) = self.set(
                TableRoot::Full,
                virtual_address,
                physical,
                flags,
                level,
                supply.as_deref_mut(),
            ) {
                panic!("a preflighted mapping with its tables in place refused a leaf: {reason}");
            }
            offset += level.span();
        }
        Ok(())
    }

    // Every target must be absent, so the leaf pass cannot be refused.
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

    // Both passes retrace this exactly, so every leaf finds the table its pass linked.
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
        // `activate` checks the tables it installs; a boot mapping added later nobody checks.
        if is_active() {
            return Err(MapError::Activated(virtual_base));
        }
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
        // Allocatable RAM gets one leaf per frame at its direct-map address and nowhere else, so
        // sealing a frame rewrites its only boot mapping.
        let allocatable = mm::is_allocatable(region.base.get(), region.len);
        if allocatable
            && mm::direct_map().and_then(|direct| direct.virtual_address(region.base.get()))
                != Some(virtual_base)
        {
            return Err(MapError::AllocatableAlias(virtual_base));
        }
        let grain = if allocatable {
            Grain::Pages
        } else {
            Grain::Largest
        };
        self.map_range(
            virtual_base,
            region.base.get(),
            region.len,
            flags.difference(NOT_THE_CALLERS),
            grain,
            None,
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
        // The run stays writable through the direct map, so no alias of it may execute.
        if !flags.contains(Flags::NO_EXECUTE) {
            return Err(MapError::WriteExecute(virtual_base));
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
        // A writable frame is never executable anywhere; executable bytes come sealed.
        if !flags.contains(Flags::NO_EXECUTE) {
            return Err(UserMapError::Mapping {
                reason: MapError::WriteExecute(virtual_base),
                run,
            });
        }
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

    // A refusal drops the run, which unseals, zeroes and frees it.
    pub fn map_user_sealed(
        &mut self,
        virtual_base: u64,
        run: SealedSegment,
        flags: Flags,
    ) -> Result<(), MapError> {
        if flags.contains(Flags::WRITABLE) {
            return Err(MapError::WriteExecute(virtual_base));
        }
        if !user_mapping_fits(virtual_base, run.len()) {
            return Err(MapError::NotYours(virtual_base));
        }
        self.owned_pages = self.map_owned_frames(
            virtual_base,
            run.address(),
            run.len(),
            run.frames(),
            flags.union(Flags::USER),
        )?;
        run.commit_mapping();
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
            None,
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
        supply: Option<&mut Earmark>,
    ) -> Result<(), MapError> {
        let slot = self.table_for(root, virtual_address, flags, target, supply)?;
        let entry = Entry::page(target, physical, flags).map_err(MapError::Encoding)?;
        if slot.read().is_present() {
            return Err(MapError::Conflict(virtual_address));
        }
        slot.write(entry);
        Ok(())
    }

    // The slot at `target` for `virtual_address`, linking a fresh table at every absent level.
    fn table_for(
        &mut self,
        root: TableRoot,
        virtual_address: u64,
        flags: Flags,
        target: Level,
        mut supply: Option<&mut Earmark>,
    ) -> Result<EntrySlot, MapError> {
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
                return Ok(slot);
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
                let (frame, link) = self.take_owned_table(
                    root,
                    virtual_address,
                    level,
                    privilege,
                    supply.as_deref_mut(),
                )?;
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
    sealed: bool,
    flush: Flush,
}

impl Unmapped {
    pub const fn pages(&self) -> u64 {
        self.pages
    }
}

impl Drop for Unmapped {
    fn drop(&mut self) {
        // No frame returns to its pool while a processor may still reach it.
        core::mem::replace(&mut self.flush, Flush::none()).settle();
        if self.sealed {
            // SAFETY: this is the one move out of `frames`, and nothing reads the field after it.
            let frames = unsafe { ManuallyDrop::take(&mut self.frames) };
            let mut flush = Flush::none();
            mm::release_sealed_run(frames, &mut flush);
            assert!(
                flush.is_complete(),
                "an unmapped sealed run awaits a flush its drop cannot wait for"
            );
            return;
        }
        // SAFETY: this is the one drop of `frames`, and nothing reads the field after it.
        unsafe { ManuallyDrop::drop(&mut self.frames) };
    }
}
