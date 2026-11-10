use super::*;

impl AddressSpace {
    /// Map `len` bytes at `virtual_base` onto physical memory at
    /// `physical_base`, with `flags`.
    ///
    /// Large pages wherever the addresses and remaining length permit, and
    /// `PRESENT` is added; no other flag is.
    ///
    /// # Errors
    ///
    /// [`MapError::Misaligned`] unless both bases are page-aligned,
    /// [`MapError::NonCanonical`] if the virtual range leaves canonical space,
    /// [`MapError::Conflict`] if a larger page already covers part of the
    /// range, [`MapError::WriteExecute`] if the resulting present mapping
    /// would be writable and executable, [`MapError::Pinned`] if it overlaps a
    /// retained stack or guard, and [`MapError::OutOfMemory`] if a table cannot
    /// be allocated.
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

        // The stripped root shares every lower-half table by reference with
        // its full partner. Consequently every present lower leaf in a pair
        // must be a user mapping: a supervisor-only leaf would still expose
        // kernel or firmware bytes to transient ring-three reads (D-407).
        // `map_user` and the typed shared-user doors add `USER`; the general
        // kernel mapping doors must remain closed in this one root shape.
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
            if let Err(error) = result {
                // Rollback prevents a refused owned mapping from leaving live
                // entries over frames returned to the allocator (D-94).
                self.undo(&range, offset, grain);
                self.rollback_tables(table_mark);
                return Err(error);
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

    /// Whether this space may be told to change `len` bytes at `at`.
    ///
    /// The check that makes [`for_user`](Self::for_user) safe to expose. A
    /// space sharing the kernel's higher half by reference must not be a
    /// handle onto the kernel's tables (D-105).
    ///
    /// # Errors
    ///
    /// [`MapError::NotYours`] for a range this space does not own outright.
    pub(super) fn may_reach(&self, at: u64, len: u64) -> Result<(), MapError> {
        // A reclaimed space has no tables. Mapping into one would walk frames
        // the allocator has given to something else (D-123).
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

    /// Where the mapping of `range` at `offset` goes, and at what size.
    ///
    /// Split out so [`undo`](Self::undo) retraces exactly the same steps: a
    /// rollback that disagreed with the walk about where a large page began
    /// would clear the wrong entry.
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

    /// Clear the first `written` bytes of `range`, undoing a partial mapping.
    ///
    /// [`preflight`](Self::preflight) established that every entry in a
    /// transaction which can reach this path was absent before it began, so
    /// each entry below belongs to this call rather than an older mapping.
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

    /// The existing entry slot for `virtual_address` at `target`, allocating
    /// nothing.
    ///
    /// [`MapError::NotMapped`] where the walk runs out of tables, which during
    /// a rollback means the forward walk never got that far.
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

    /// Map memory the boot handoff described, at `virtual_base`.
    ///
    /// [`BootRegion`] proves the boot memory map covers the physical range, so
    /// no safe mapping door accepts an unchecked physical address (D-64, D-87).
    ///
    /// # Errors
    ///
    /// As [`map_owned`](Self::map_owned).
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

    /// Map a run of frames this caller owns, at `virtual_base`.
    ///
    /// Ownership moves into the address space. A refusal rolls back every leaf
    /// before `run` returns to the allocator (D-44, D-94).
    ///
    /// # Errors
    ///
    /// [`MapError::Misaligned`] unless `virtual_base` is page-aligned,
    /// [`MapError::NonCanonical`] if the range leaves canonical space,
    /// [`MapError::Conflict`] if any page of it is already mapped to something
    /// else, [`MapError::WriteExecute`] if the mapping would be writable and
    /// executable, [`MapError::Pinned`] if it overlaps a retained stack or
    /// guard, and [`MapError::OutOfMemory`] if a table cannot be allocated.
    pub fn map_owned(
        &mut self,
        virtual_base: u64,
        run: Segment,
        flags: Flags,
    ) -> Result<(), MapError> {
        // A kernel run holds kernel bytes, so ring 3 may not write it here;
        // `map_user` takes a run whose type says its bytes are outside (D-107).
        if flags.contains(Flags::USER) {
            return Err(MapError::NotYours(virtual_base));
        }
        self.map_run(virtual_base, run, flags)
    }

    /// Map a run of frames ring 3 may reach, at `virtual_base`.
    ///
    /// [`USegment`] makes external-byte provenance explicit; this method adds
    /// `USER` and records ownership regardless of caller flags (D-44, D-107).
    ///
    /// # Errors
    ///
    /// As [`map_owned`](Self::map_owned).
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

    /// Map a run ring 3 may reach, **without** taking it.
    ///
    /// The mapping omits `OWNED`, retains a non-accessing owner, and therefore
    /// cannot recycle frames while either side can reach them. The one-way
    /// [`USegment::share`] conversion prevents owned/shared aliasing
    /// (D-113, D-115).
    ///
    /// # Errors
    ///
    /// As [`map_user`](Self::map_user), and [`MapError::CacheAlias`] if caller
    /// flags would conflict with the allocation's write-back direct-map view.
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

    /// Map a checked shared-ring allocation into a user address space.
    ///
    /// Ownership and mapping rules are identical to
    /// [`map_shared`](Self::map_shared); the distinct type retains the ring's
    /// payload/publication-word partition.
    ///
    /// # Errors
    ///
    /// As [`map_shared`](Self::map_shared).
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
        // Ownership lives in the leaf every unmap observes, never in caller
        // flags; an owned large page could not be split back (D-45, D-88, D-105).
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
                // Only an identical rewrite is allowed: anything else would
                // re-point a mapping some handle is evidence of.
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
                // An interior entry covers everything below it, so upgrading
                // it to `USER` would hand ring 3 every mapping under it (D-105).
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

        // Unreachable: `target` is always `Pt` or `Pd`, both of which are in
        // `Level::WALK`, so the loop returns before it can end.
        Err(MapError::Conflict(virtual_address))
    }
}

/// How large the pages of a mapping may be.
///
/// Four-kilobyte granularity keeps owned mappings splittable by `unmap`; the
/// direct map instead takes the largest aligned pages available.
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

/// Evidence that a physical range is one the boot handoff described.
///
/// The only way to reach [`AddressSpace::map_boot`], and constructible only
/// from a range the bootloader's memory map covers. Not `Copy`: it is a
/// statement about one range, made once, and a copy would let a caller reuse
/// the evidence for a range nobody checked.
#[derive(Debug)]
pub struct BootRegion {
    base: Paddr,
    len: u64,
    cache: crate::boot::DirectMapCache,
}

impl BootRegion {
    /// Describe `len` bytes at `base`, or `None` unless the memory map covers
    /// all of it *with memory* of one direct-map cache type.
    ///
    /// Device holes are excluded because write-back direct-map aliases of MMIO
    /// are unsafe, and a range mixing framebuffer and ordinary memory has no
    /// single type a mapping could share. Returns `None` before the map exists
    /// (D-45, D-105, D-349).
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
