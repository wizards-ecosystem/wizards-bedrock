use super::*;

impl AddressSpace {
    pub(super) fn install_entry_enclave(
        &mut self,
        enclave: super::kpti::EntryEnclave,
    ) -> Result<(), MapError> {
        let pages = enclave_pages(enclave)?;
        let expected_stripped = self.expected_enclave(enclave, pages)?;
        if let Some(address) = mapped_guard(self.root(), enclave)? {
            return Err(MapError::InvalidEntryEnclave(address));
        }

        let mark = self.tables.len();
        let mut written = 0usize;
        for page in &expected_stripped {
            let flags = enclave_flags(page.permission());
            if let Err(reason) = self.set_in(
                TableRoot::Stripped,
                page.virtual_address(),
                page.physical_address(),
                flags,
                Level::Pt,
            ) {
                self.rollback_stripped_enclave(&expected_stripped, written, mark);
                return Err(reason);
            }
            written += 1;
        }

        let stripped = self
            .roots()
            .stripped_root()
            .ok_or(MapError::InvalidEntryEnclave(self.root()))?;
        if let Some(address) = mapped_guard(stripped, enclave)? {
            self.rollback_stripped_enclave(&expected_stripped, written, mark);
            return Err(MapError::InvalidEntryEnclave(address));
        }
        let observed = match observe_stripped_higher_half(stripped, pages) {
            Ok(observed) => observed,
            Err(reason) => {
                self.rollback_stripped_enclave(&expected_stripped, written, mark);
                return Err(reason);
            }
        };
        if !enclave_matches(&expected_stripped, &observed) {
            self.rollback_stripped_enclave(&expected_stripped, written, mark);
            return Err(MapError::InvalidEntryEnclave(
                expected_stripped
                    .first()
                    .map_or(stripped, |page| page.virtual_address()),
            ));
        }
        paired_flush_premise(self.roots())?;
        Ok(())
    }

    /// The stripped root's enclave pages, once the full root maps exactly the enclave.
    fn expected_enclave(
        &self,
        enclave: super::kpti::EntryEnclave,
        pages: usize,
    ) -> Result<Vec<EnclavePage>, MapError> {
        let mut expected_full = Vec::new();
        let mut expected_stripped = Vec::new();
        let mut observed_full = Vec::new();
        expected_full
            .try_reserve_exact(pages)
            .map_err(|_| MapError::OutOfMemory)?;
        expected_stripped
            .try_reserve_exact(pages)
            .map_err(|_| MapError::OutOfMemory)?;
        observed_full
            .try_reserve_exact(pages)
            .map_err(|_| MapError::OutOfMemory)?;

        for mapping in enclave.mappings() {
            let section = mapping.section();
            let mut address = section.start;
            while address < section.end {
                let Some((physical, flags, span)) = walk(self.root(), address) else {
                    return Err(MapError::InvalidEntryEnclave(address));
                };
                let full = enclave_permission(mapping.full_access());
                let stripped = enclave_permission(mapping.stripped_access());
                expected_full.push(
                    EnclavePage::new(address, physical, full)
                        .map_err(|_| MapError::InvalidEntryEnclave(address))?,
                );
                expected_stripped.push(
                    EnclavePage::new(address, physical, stripped)
                        .map_err(|_| MapError::InvalidEntryEnclave(address))?,
                );
                observed_full.push(ObservedPage::new(address, physical, span, flags));
                address = address
                    .checked_add(PAGE_SIZE)
                    .ok_or(MapError::InvalidEntryEnclave(address))?;
            }
        }
        if !enclave_matches(&expected_full, &observed_full) {
            return Err(MapError::InvalidEntryEnclave(
                expected_full
                    .first()
                    .map_or(self.root(), |page| page.virtual_address()),
            ));
        }
        Ok(expected_stripped)
    }

    fn rollback_stripped_enclave(
        &mut self,
        expected: &[EnclavePage],
        written: usize,
        table_mark: usize,
    ) {
        let stripped = self
            .roots()
            .stripped_root()
            .unwrap_or_else(|| panic!("paired enclave rollback lost its stripped root"));
        for page in &expected[..written] {
            let slot = leaf_in(stripped, page.virtual_address()).unwrap_or_else(|reason| {
                panic!("paired enclave rollback lost a written leaf: {reason}")
            });
            EntrySlot::write(&slot, Entry::ABSENT);
        }
        self.rollback_tables(table_mark);
    }

    /// Reserve the top-level table every later device mapping will share.
    pub(super) fn reserve_shared_kernel_topology(&mut self) -> Result<(), MapError> {
        assert_eq!(
            Level::Pml4.index(DEVICE_WINDOW_START),
            Level::Pml4.index(DEVICE_WINDOW_END - 1),
            "the device window must fit in one shared top-level slot"
        );
        let slot = entry_slot(self.root(), Level::Pml4.index(DEVICE_WINDOW_START))?;
        let current = EntrySlot::read(&slot);
        if current.is_present() {
            if current.is_huge() || current.flags().contains(Flags::USER) {
                return Err(MapError::Conflict(DEVICE_WINDOW_START));
            }
            return Ok(());
        }

        let (_, link) =
            self.take_owned_table(DEVICE_WINDOW_START, Level::Pml4, TablePrivilege::Supervisor)?;
        EntrySlot::write(&slot, link);
        Ok(())
    }
}

fn enclave_pages(enclave: super::kpti::EntryEnclave) -> Result<usize, MapError> {
    enclave
        .mappings()
        .iter()
        .try_fold(0usize, |total, mapping| {
            let pages = usize::try_from(mapping.section().mapped_len() / PAGE_SIZE)
                .map_err(|_| MapError::OutOfMemory)?;
            total.checked_add(pages).ok_or(MapError::OutOfMemory)
        })
}

fn mapped_guard(root: Paddr, enclave: super::kpti::EntryEnclave) -> Result<Option<u64>, MapError> {
    for guard in enclave.guards() {
        let mut address = guard.start;
        while address < guard.end {
            if translate_in(root, address).is_some() {
                return Ok(Some(address));
            }
            address = address
                .checked_add(PAGE_SIZE)
                .ok_or(MapError::InvalidEntryEnclave(address))?;
        }
    }
    Ok(None)
}

pub(super) const fn enclave_permission(access: super::kpti::Access) -> EnclavePermission {
    match access {
        super::kpti::Access::Executable => EnclavePermission::Executable,
        super::kpti::Access::ReadOnly => EnclavePermission::ReadOnly,
        super::kpti::Access::Writable => EnclavePermission::Writable,
    }
}

pub(super) const fn enclave_access_policy(access: super::kpti::Access) -> (bool, bool) {
    match access {
        super::kpti::Access::Executable => (false, true),
        super::kpti::Access::ReadOnly => (false, false),
        super::kpti::Access::Writable => (true, false),
    }
}

const fn enclave_flags(permission: EnclavePermission) -> Flags {
    match permission {
        EnclavePermission::Executable => Flags::PRESENT,
        EnclavePermission::ReadOnly => Flags::PRESENT.union(Flags::NO_EXECUTE),
        EnclavePermission::Writable => Flags::PRESENT
            .union(Flags::WRITABLE)
            .union(Flags::NO_EXECUTE),
    }
}

pub(super) fn observe_stripped_higher_half(
    root: Paddr,
    expected_pages: usize,
) -> Result<Vec<ObservedPage>, MapError> {
    let mut observed = Vec::new();
    observed
        .try_reserve_exact(expected_pages)
        .map_err(|_| MapError::OutOfMemory)?;
    visit_present_leaves(
        root,
        ENTRIES / 2,
        ENTRIES,
        |address, physical, flags, span| {
            if observed.len() == expected_pages {
                return Err(MapError::InvalidEntryEnclave(address));
            }
            observed.push(ObservedPage::new(address, physical, span, flags));
            Ok(())
        },
    )?;
    Ok(observed)
}

pub(super) fn paired_flush_premise(roots: AddressSpaceRoots) -> Result<FlushPremise, MapError> {
    let Some(stripped) = roots.stripped_root() else {
        return Err(MapError::InvalidEntryEnclave(roots.full_root()));
    };
    let mut flags = Vec::new();
    for root in [roots.full_root(), stripped] {
        visit_present_leaves(root, 0, ENTRIES, |_, _, leaf_flags, _| {
            flags.try_reserve(1).map_err(|_| MapError::OutOfMemory)?;
            flags.push(leaf_flags);
            Ok(())
        })?;
    }
    let pcid = if cpu::page_table_context_ids_are_disabled() {
        PcidState::Disabled
    } else {
        PcidState::Enabled
    };
    FlushPremise::new(pcid, &flags).map_err(MapError::InvalidPairedFlush)
}
