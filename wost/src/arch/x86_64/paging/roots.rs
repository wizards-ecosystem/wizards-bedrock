use alloc::vec::Vec;

use x86_paging::kpti::{EnclavePage, ObservedPage, enclave_matches};
use x86_paging::{ENTRIES, Flags, Level, PAGE_SIZE, TablePrivilege};

use super::{
    AddressSpace, DEVICE_WINDOW_END, DEVICE_WINDOW_START, MapError, TableRoot, entry_slot,
    translate_in, visit_present_leaves, walk,
};
use crate::mm::PhysicalAddress;

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

        let stripped = self
            .roots()
            .stripped_root()
            .ok_or(MapError::InvalidEntryEnclave(self.root().get()))?;
        for page in &expected_stripped {
            self.set(
                TableRoot::Stripped,
                page.virtual_address(),
                page.physical_address(),
                page.permission().flags(),
                Level::Pt,
            )?;
        }
        if let Some(address) = mapped_guard(stripped, enclave)? {
            return Err(MapError::InvalidEntryEnclave(address));
        }
        let observed = observe_stripped_higher_half(stripped, pages)?;
        if !enclave_matches(&expected_stripped, &observed) {
            return Err(MapError::InvalidEntryEnclave(
                expected_stripped
                    .first()
                    .map_or(stripped.get(), |page| page.virtual_address()),
            ));
        }
        Ok(())
    }

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
                expected_full.push(
                    EnclavePage::new(address, physical.get(), mapping.full_access())
                        .map_err(|_| MapError::InvalidEntryEnclave(address))?,
                );
                expected_stripped.push(
                    EnclavePage::new(address, physical.get(), mapping.stripped_access())
                        .map_err(|_| MapError::InvalidEntryEnclave(address))?,
                );
                observed_full.push(ObservedPage::new(address, physical.get(), span, flags));
                address = address
                    .checked_add(PAGE_SIZE)
                    .ok_or(MapError::InvalidEntryEnclave(address))?;
            }
        }
        if !enclave_matches(&expected_full, &observed_full) {
            return Err(MapError::InvalidEntryEnclave(
                expected_full
                    .first()
                    .map_or(self.root().get(), |page| page.virtual_address()),
            ));
        }
        Ok(expected_stripped)
    }

    pub(super) fn reserve_shared_kernel_topology(&mut self) -> Result<(), MapError> {
        assert_eq!(
            Level::Pml4.index(DEVICE_WINDOW_START),
            Level::Pml4.index(DEVICE_WINDOW_END - 1),
            "the device window must fit in one shared top-level slot"
        );
        let slot = entry_slot(self.root(), Level::Pml4.index(DEVICE_WINDOW_START))?;
        let current = slot.read();
        if current.is_present() {
            if current.is_huge() || current.flags().contains(Flags::USER) {
                return Err(MapError::Conflict(DEVICE_WINDOW_START));
            }
            return Ok(());
        }

        let (_, link) = self.take_owned_table(
            TableRoot::Full,
            DEVICE_WINDOW_START,
            Level::Pml4,
            TablePrivilege::Supervisor,
        )?;
        slot.write(link);
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

fn mapped_guard(
    root: PhysicalAddress,
    enclave: super::kpti::EntryEnclave,
) -> Result<Option<u64>, MapError> {
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

fn observe_stripped_higher_half(
    root: PhysicalAddress,
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
            observed.push(ObservedPage::new(address, physical.get(), span, flags));
            Ok(())
        },
    )?;
    Ok(observed)
}
