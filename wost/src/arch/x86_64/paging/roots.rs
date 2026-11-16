use x86_paging::kpti::{EnclavePage, EnclavePermission, ObservedPage, enclave_matches};
use x86_paging::{ENTRIES, Flags, Level, PAGE_SIZE, TablePrivilege};

use super::kpti::{EnclaveMapping, EntryEnclave};
use super::{
    AddressSpace, DEVICE_WINDOW_END, DEVICE_WINDOW_START, MapError, Reach, TableRoot,
    current_mapping_root, entry_slot, translate_in, visit_present_leaves, walk,
};
use crate::boot::Section;
use crate::mm::pool::Earmark;
use crate::mm::{self, PhysicalAddress};

impl AddressSpace {
    // Every check walks the tables again rather than keeping a list, so building a user space takes
    // nothing from the kernel heap.
    pub(super) fn install_entry_enclave(
        &mut self,
        enclave: EntryEnclave,
        tables: &mut Earmark,
    ) -> Result<(), MapError> {
        let pages = enclave_pages(enclave)?;
        let root = self.root();
        let first = enclave_page(enclave, 0).map_or(root.get(), |(address, _)| address);
        let refused = MapError::InvalidEntryEnclave(first);
        if !enclave_matches(
            pages,
            |index| expected_page(root, enclave, index, false),
            |index| observed_page(root, enclave, index),
        ) {
            return Err(refused);
        }
        if let Some(address) = mapped_guard(root, enclave)? {
            return Err(MapError::InvalidEntryEnclave(address));
        }

        let stripped = self
            .roots()
            .stripped_root()
            .ok_or(MapError::InvalidEntryEnclave(root.get()))?;
        for index in 0..pages {
            let page = expected_page(root, enclave, index, true).ok_or(refused)?;
            self.set(
                TableRoot::Stripped,
                page.virtual_address(),
                page.physical_address(),
                page.permission().flags(),
                Level::Pt,
                Some(&mut *tables),
            )?;
        }
        if let Some(address) = mapped_guard(stripped, enclave)? {
            return Err(MapError::InvalidEntryEnclave(address));
        }
        let mut leaves = 0;
        visit_present_leaves(stripped, ENTRIES / 2, ENTRIES, |_, _, _, _| {
            leaves += 1;
            Ok(())
        })?;
        if leaves != pages
            || !enclave_matches(
                pages,
                |index| expected_page(root, enclave, index, true),
                |index| observed_page(stripped, enclave, index),
            )
        {
            return Err(refused);
        }
        Ok(())
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
                None,
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
        let run = mm::allocate_contiguous(pages, 1).ok_or(MapError::OutOfMemory)?;
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
                None,
            )?;
            offset += PAGE_SIZE;
        }
        Ok(())
    }
}

fn enclave_pages(enclave: EntryEnclave) -> Result<usize, MapError> {
    enclave.mappings().try_fold(0usize, |total, mapping| {
        let pages = usize::try_from(mapping.section().mapped_len() / PAGE_SIZE)
            .map_err(|_| MapError::OutOfMemory)?;
        total.checked_add(pages).ok_or(MapError::OutOfMemory)
    })
}

// The enclave's `index`th page and the mapping it belongs to, counted through the sections.
fn enclave_page(enclave: EntryEnclave, mut index: usize) -> Option<(u64, EnclaveMapping)> {
    for mapping in enclave.mappings() {
        let section = mapping.section();
        let pages = usize::try_from(section.mapped_len() / PAGE_SIZE).ok()?;
        if index < pages {
            return Some((section.start + index as u64 * PAGE_SIZE, mapping));
        }
        index -= pages;
    }
    None
}

// The page as the full root maps it, with the access the full or the stripped root must give it.
fn expected_page(
    root: PhysicalAddress,
    enclave: EntryEnclave,
    index: usize,
    stripped: bool,
) -> Option<EnclavePage> {
    let (address, mapping) = enclave_page(enclave, index)?;
    let (physical, _, _) = walk(root, address)?;
    let access = if stripped {
        mapping.stripped_access()
    } else {
        mapping.full_access()
    };
    EnclavePage::new(address, physical.get(), access).ok()
}

fn observed_page(
    root: PhysicalAddress,
    enclave: EntryEnclave,
    index: usize,
) -> Option<ObservedPage> {
    let (address, _) = enclave_page(enclave, index)?;
    let (physical, flags, span) = walk(root, address)?;
    Some(ObservedPage::new(address, physical.get(), span, flags))
}

fn mapped_guard(root: PhysicalAddress, enclave: EntryEnclave) -> Result<Option<u64>, MapError> {
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
