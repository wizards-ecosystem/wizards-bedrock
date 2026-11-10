use super::*;

impl AddressSpace {
    /// Map `len` bytes at `virtual_base` onto `physical_base`, adding `PRESENT` and no other flag.
    pub(super) fn map_range(
        &mut self,
        virtual_base: u64,
        physical_base: Paddr,
        len: u64,
        flags: Flags,
        grain: Grain,
    ) -> Result<(), MapError> {
        let range = MappingRange::new(virtual_base, physical_base, len)?;
        let flags = flags.union(Flags::PRESENT);

        // A pair's lower tables are shared with its stripped root, so every present lower leaf must
        // be `USER`: a supervisor leaf leaks kernel bytes to transient ring-three reads.
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

        if self.preflight(&range, flags, grain)? == MappingState::Identical {
            return Ok(());
        }

        let table_mark = self.tables.len();
        let mut offset = 0;
        while offset < range.len() {
            let (virtual_address, physical, level) = Self::step(&range, offset, grain);
            #[cfg(feature = "selftest")]
            let result = if mapping_refusal(&MAPPING_WRITES_BEFORE_REFUSAL) {
                Err(MapError::OutOfMemory)
            } else {
                self.set(virtual_address, physical, flags, level)
            };
            #[cfg(not(feature = "selftest"))]
            let result = self.set(virtual_address, physical, flags, level);
            if let Err(reason) = result {
                // Rollback leaves no live entry over the frames a refused owned mapping returns.
                self.undo(&range, offset, grain);
                self.rollback_tables(table_mark);
                return Err(reason);
            }
            offset += level.span();
        }
        Ok(())
    }

    /// Establish that every target is absent or every target is identical.
    fn preflight(
        &self,
        range: &MappingRange,
        flags: Flags,
        grain: Grain,
    ) -> Result<MappingState, MapError> {
        let mut transaction = None;
        let mut offset = 0;
        while offset < range.len() {
            let (virtual_address, physical, level) = Self::step(range, offset, grain);
            self.verify_shared_lower_link(virtual_address)?;
            let state = self.mapping_state(virtual_address, physical, flags, level)?;
            match transaction {
                None => transaction = Some(state),
                Some(found) if found == state => {}
                Some(_) => return Err(MapError::Conflict(virtual_address)),
            }
            offset += level.span();
        }
        transaction.ok_or(MapError::Misaligned(range.virtual_base()))
    }

    /// The state of one target before a transaction writes anything.
    fn mapping_state(
        &self,
        virtual_address: u64,
        physical: Paddr,
        flags: Flags,
        target: Level,
    ) -> Result<MappingState, MapError> {
        let wanted = Entry::page(target, physical, flags).map_err(MapError::Encoding)?;
        let mut table = self.root();
        for level in Level::WALK {
            let slot = entry_slot(table, level.index(virtual_address))?;
            let current = EntrySlot::read(&slot);
            if level == target {
                return if !current.is_present() {
                    Ok(MappingState::Absent)
                } else if current == wanted {
                    Ok(MappingState::Identical)
                } else {
                    Err(MapError::Conflict(virtual_address))
                };
            }
            if !current.is_present() {
                return Ok(MappingState::Absent);
            }
            if current.is_huge()
                || flags.contains(Flags::USER) && !current.flags().contains(Flags::USER)
            {
                return Err(MapError::Conflict(virtual_address));
            }
            table = current.address();
        }
        Err(MapError::Conflict(virtual_address))
    }

    /// Whether this space may change `len` bytes at `at`; a user space never edits kernel tables.
    pub(super) fn may_reach(&self, at: u64, len: u64) -> Result<(), MapError> {
        // A reclaimed space's tables went back, so mapping would walk frames now owned elsewhere.
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

    /// Where `range` at `offset` maps and how large; [`undo`](Self::undo) retraces it exactly,
    /// since a rollback that disagreed about where a large page began would clear the wrong entry.
    fn step(range: &MappingRange, offset: u64, grain: Grain) -> (u64, Paddr, Level) {
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

    /// Clear the first `written` bytes of `range`, whose entries [`preflight`](Self::preflight)
    /// proved absent before, so every one belongs to this transaction.
    pub(super) fn undo(&mut self, range: &MappingRange, written: u64, grain: Grain) {
        let mut offset = 0;
        while offset < written {
            let (virtual_address, _, level) = Self::step(range, offset, grain);
            #[cfg(feature = "selftest")]
            if mapping_refusal(&MAPPING_UNDOS_BEFORE_REFUSAL) {
                panic!("mapping rollback lost a written leaf: injected refusal");
            }
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
                let entry = EntrySlot::read(&slot);
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
            let current = EntrySlot::read(&parent);
            assert!(
                current == owned.link,
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
                    shared_user_link_matches(current, EntrySlot::read(paired)),
                    "mapping rollback found a divergent paired table link"
                );
            }
            EntrySlot::write(&parent, Entry::ABSENT);
            if let Some(paired) = paired.as_ref() {
                EntrySlot::write(paired, Entry::ABSENT);
            }
            // SAFETY: `take_owned_table` forgot exactly this allocation, and
            // the only page-table link to it is now absent.
            drop(unsafe { mm::Frame::from_raw(owned.frame) });
            self.tables.pop();
            if let Some(authority) = self.pair_authority {
                self.pair_authority = Some(authority.release_table().unwrap_or_else(|| {
                    panic!("paired rollback lost its intermediate-table authority")
                }));
            }
        }
        assert!(
            self.pair_authority
                .is_none_or(|authority| authority.table_frames() == self.tables.len()),
            "paired rollback retained incomplete frame authority"
        );
    }

    /// The existing slot for `virtual_address` at `target`, or `NotMapped` where the tables end.
    fn slot_at(&self, virtual_address: u64, target: Level) -> Result<EntrySlot, MapError> {
        let mut table = self.root();
        for level in Level::WALK {
            let slot = entry_slot(table, level.index(virtual_address))?;
            if level == target {
                return Ok(slot);
            }
            let entry = EntrySlot::read(&slot);
            if !entry.is_present() || entry.is_huge() {
                return Err(MapError::NotMapped(virtual_address));
            }
            table = entry.address();
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
        if !shared_user_link_matches(EntrySlot::read(&full), EntrySlot::read(&stripped)) {
            return Err(MapError::Conflict(virtual_address));
        }
        Ok(())
    }

    /// Map boot-described memory at `virtual_base`; no safe door takes an unchecked physical range.
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
        self.map_range(
            virtual_base,
            region.base,
            region.len,
            flags.difference(NOT_THE_CALLERS),
            Grain::Largest,
        )
    }

    /// Map a caller-owned run here; a refusal rolls back every leaf before `run` is freed.
    pub fn map_owned(
        &mut self,
        virtual_base: u64,
        run: Segment,
        flags: Flags,
    ) -> Result<(), MapError> {
        // A kernel run holds kernel bytes, so ring 3 never maps it; `map_user` takes outside bytes.
        if flags.contains(Flags::USER) {
            return Err(MapError::NotYours(virtual_base));
        }
        self.map_run(virtual_base, run, flags)
    }

    /// Map an outside-byte run ring 3 may reach, adding `USER` and ownership whatever the flags.
    pub fn map_user(
        &mut self,
        virtual_base: u64,
        run: USegment,
        flags: Flags,
    ) -> Result<(), UserMapError> {
        match self.map_run_geometry(virtual_base, &run, flags.union(Flags::USER)) {
            Ok(owned_pages) => {
                run.commit_mapping();
                self.owned_pages = owned_pages;
                Ok(())
            }
            Err(reason) => Err(UserMapError::Mapping { reason, run }),
        }
    }

    /// Map a run ring 3 may reach without taking it, refusing a cache alias of its direct map.
    ///
    /// It omits `OWNED` and keeps a non-accessing owner, so no frame recycles while either side
    /// reaches it; the one-way [`USegment::share`] prevents owned and shared aliases.
    pub fn map_shared(
        &mut self,
        virtual_base: u64,
        run: &SharedBytes,
        flags: Flags,
    ) -> Result<(), MapError> {
        ram_cache_policy(virtual_base, flags)?;
        self.map_shared_allocation(
            virtual_base,
            run.address(),
            run.len(),
            run.keepalive(),
            flags,
        )
    }

    /// Map a shared ring as [`map_shared`](Self::map_shared) does, keeping its publication words.
    pub fn map_shared_ring<const WORDS: usize>(
        &mut self,
        virtual_base: u64,
        run: &SharedRing<WORDS>,
        flags: Flags,
    ) -> Result<(), MapError> {
        ram_cache_policy(virtual_base, flags)?;
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
        physical_base: Paddr,
        len: u64,
        keepalive: SharedKeepalive,
        flags: Flags,
    ) -> Result<(), MapError> {
        self.may_reach(virtual_base, len)?;
        self.shared
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        self.map_range(
            virtual_base,
            physical_base,
            len,
            flags.difference(NOT_THE_CALLERS).union(Flags::USER),
            Grain::Pages,
        )?;
        self.shared.push(keepalive);
        Ok(())
    }

    fn map_run(&mut self, virtual_base: u64, run: Segment, flags: Flags) -> Result<(), MapError> {
        let owned_pages =
            self.map_owned_geometry(virtual_base, run.address(), run.len(), run.frames(), flags)?;
        // The run belongs to the mapping now; `unmap` is what gives it back.
        core::mem::forget(run);
        self.owned_pages = owned_pages;
        Ok(())
    }

    fn map_run_geometry(
        &mut self,
        virtual_base: u64,
        run: &USegment,
        flags: Flags,
    ) -> Result<usize, MapError> {
        self.map_owned_geometry(virtual_base, run.address(), run.len(), run.frames(), flags)
    }

    fn map_owned_geometry(
        &mut self,
        virtual_base: u64,
        physical_base: Paddr,
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
        // Ownership lives in the leaf unmap reads, never in caller flags; an owned large page
        // could not be split back.
        self.map_range(
            virtual_base,
            physical_base,
            len,
            flags.difference(NOT_THE_CALLERS).union(Flags::OWNED),
            Grain::Pages,
        )?;
        Ok(owned_pages)
    }

    /// Install one entry at `target`, allocating the tables above it.
    fn set(
        &mut self,
        virtual_address: u64,
        physical: Paddr,
        flags: Flags,
        target: Level,
    ) -> Result<(), MapError> {
        self.set_in(TableRoot::Full, virtual_address, physical, flags, target)
    }

    pub(super) fn set_in(
        &mut self,
        root: TableRoot,
        virtual_address: u64,
        physical: Paddr,
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
                let current = EntrySlot::read(&slot);
                // Only an identical rewrite: anything else re-points a mapping a handle proves.
                if current.is_present() && current != entry {
                    return Err(MapError::Conflict(virtual_address));
                }
                EntrySlot::write(&slot, entry);
                return Ok(());
            }

            let current = EntrySlot::read(&slot);
            let paired = if root == TableRoot::Full && level == Level::Pml4 {
                self.stripped_lower_slot(virtual_address)?
            } else {
                None
            };
            if let Some(paired) = paired.as_ref()
                && !shared_user_link_matches(current, EntrySlot::read(paired))
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
                current.address()
            } else {
                let privilege = if paired.is_some() || flags.contains(Flags::USER) {
                    TablePrivilege::User
                } else {
                    TablePrivilege::Supervisor
                };
                let (frame, link) =
                    self.take_owned_table_in(root, virtual_address, level, privilege)?;
                EntrySlot::write(&slot, link);
                if let Some(paired) = paired.as_ref() {
                    EntrySlot::write(paired, link);
                }
                frame
            };
        }

        // Unreachable: `target` is `Pt` or `Pd`, both in `Level::WALK`, so the loop returns first.
        Err(MapError::Conflict(virtual_address))
    }
}

/// How large a mapping's pages may be; owned mappings stay four-kilobyte so `unmap` can split them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Grain {
    /// The largest page each aligned stretch allows.
    Largest,
    /// Four kilobytes throughout.
    Pages,
}

/// Whether every leaf named by one mapping existed before it began.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MappingState {
    /// Every target was absent, so rollback owns every leaf it wrote.
    Absent,
    /// Every target already contained the exact requested mapping.
    Identical,
}

/// Evidence the boot memory map covers one range, the only way into [`AddressSpace::map_boot`].
///
/// Not `Copy`: a copy would let a caller reuse the evidence for a range nobody checked.
#[derive(Debug)]
pub struct BootRegion {
    base: Paddr,
    len: u64,
    cache: crate::boot::DirectMapCache,
}

impl BootRegion {
    /// Describe `len` bytes at `base`, or `None` before the map or unless memory of one direct-map
    /// cache type covers it; device holes are refused because write-back MMIO aliases are unsafe.
    pub fn describe(base: Paddr, len: u64) -> Option<Self> {
        let end = base.checked_add(len)?;
        if len == 0 || !base.is_multiple_of(PAGE_SIZE) {
            return None;
        }
        let cache = crate::mm::map_direct_cache(base, end)?;
        Some(Self { base, len, cache })
    }

    /// The cache-type bits every mapping of this range must carry.
    const fn cache_type(&self) -> Flags {
        match self.cache {
            crate::boot::DirectMapCache::Normal => Flags::NONE,
            crate::boot::DirectMapCache::Framebuffer => Flags::WRITE_THROUGH,
        }
    }
}
