use alloc::vec::Vec;

use x86_paging::kpti::{EnclavePage, EnclavePermission, ObservedPage, enclave_matches};
use x86_paging::{ENTRIES, Flags, Level, PAGE_SIZE, TablePrivilege};

use super::{
    AddressSpace, DEVICE_WINDOW_END, DEVICE_WINDOW_START, MapError, Reach, TableRoot,
    current_mapping_root, entry_slot, translate_in, visit_present_leaves, walk,
};
use crate::boot::Section;
use crate::mm::{self, PhysicalAddress};

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
        let entry_window_end = super::percpu::WINDOW + super::percpu::WINDOW_BYTES;
        for (start, end) in [
            (DEVICE_WINDOW_START, DEVICE_WINDOW_END),
            (super::percpu::WINDOW, entry_window_end),
        ] {
            assert_eq!(
                Level::Pml4.index(start),
                Level::Pml4.index(end - 1),
                "a shared kernel window must fit in one top-level slot"
            );
            let slot = entry_slot(self.root(), Level::Pml4.index(start))?;
            let current = slot.read();
            if current.is_present() {
                if current.is_huge() || current.flags().contains(Flags::USER) {
                    return Err(MapError::Conflict(start));
                }
                continue;
            }

            let (_, link) = self.take_owned_table(
                TableRoot::Full,
                start,
                Level::Pml4,
                TablePrivilege::Supervisor,
            )?;
            slot.write(link);
        }
        Ok(())
    }

    // Never unmapped: a processor runs on its entry area for as long as it runs.
    pub(crate) fn map_entry_area(
        &mut self,
        section: Section,
        access: EnclavePermission,
    ) -> Result<(), MapError> {
        if self.reach != Reach::Everywhere || current_mapping_root(self.roots()).is_none() {
            return Err(MapError::NotYours(section.start));
        }
        let pages =
            usize::try_from(section.mapped_len() / PAGE_SIZE).map_err(|_| MapError::OutOfMemory)?;
        let mut run = mm::allocate_contiguous(pages, 1).ok_or(MapError::OutOfMemory)?;
        if !run.zero() {
            return Err(MapError::Unreachable(run.address()));
        }
        let physical = run.address().get();
        core::mem::forget(run);
        let mut offset = 0;
        while offset < section.mapped_len() {
            self.set(
                TableRoot::Full,
                section.start + offset,
                physical + offset,
                access.flags(),
                Level::Pt,
            )?;
            offset += PAGE_SIZE;
        }
        Ok(())
    }
}

fn enclave_pages(enclave: super::kpti::EntryEnclave) -> Result<usize, MapError> {
    enclave.mappings().try_fold(0usize, |total, mapping| {
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
