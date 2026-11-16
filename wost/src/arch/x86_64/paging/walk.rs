use core::ptr::NonNull;

use x86_paging::{CanonicalAddress, ENTRIES, Entry, Flags, Level, PAGE_SIZE};

use super::super::{cpu, interrupt};
use super::{AddressSpaceRoots, MapError, is_active};
use crate::mm::{self, PhysicalAddress};

#[derive(PartialEq, Eq, Debug)]
pub(super) struct EntrySlot {
    pointer: NonNull<Entry>,
}

impl EntrySlot {
    pub(super) fn read(&self) -> Entry {
        // SAFETY: only tables this crate built, or the bootloader's while its root is loaded, are
        // walked, and construction proves this aligned pointer names one entry of that frame.
        unsafe { self.pointer.as_ptr().read_volatile() }
    }

    pub(super) fn write(&self, entry: Entry) {
        // SAFETY: as `read`, and every writer holds the owning `AddressSpace` mutably or the frame
        // before any root links it, so no other write races this one.
        unsafe { self.pointer.as_ptr().write_volatile(entry) };
    }
}

pub(super) fn entry_slot(table: PhysicalAddress, index: usize) -> Result<EntrySlot, MapError> {
    assert!(index < ENTRIES, "a level's index is always below 512");
    let base = mm::direct_pointer(table, PAGE_SIZE).ok_or(MapError::Unreachable(table))?;
    let pointer = base.cast::<Entry>().wrapping_add(index);
    let pointer = NonNull::new(pointer).ok_or(MapError::Unreachable(table))?;
    Ok(EntrySlot { pointer })
}

pub(super) fn slot_in(
    root: PhysicalAddress,
    address: u64,
    target: Level,
) -> Result<EntrySlot, MapError> {
    let mut table = root;
    for level in Level::WALK {
        let slot = entry_slot(table, level.index(address))?;
        if level == target {
            return Ok(slot);
        }
        let entry = slot.read();
        if !entry.is_present() {
            return Err(MapError::NotMapped(address));
        }
        if entry.is_huge() {
            return Err(MapError::Unsplittable(address));
        }
        table = PhysicalAddress::from_entry(entry);
    }
    Err(MapError::NotMapped(address))
}

pub(super) fn leaf_in(root: PhysicalAddress, address: u64) -> Result<EntrySlot, MapError> {
    let slot = slot_in(root, address, Level::Pt)?;
    slot.read()
        .is_present()
        .then_some(slot)
        .ok_or(MapError::NotMapped(address))
}

pub(super) fn clear_mapping_slot(slot: EntrySlot, address: u64, level: Level) {
    let Some(address) = CanonicalAddress::new(address) else {
        panic!("a validated mapping produced a noncanonical invalidation address");
    };
    let disabled = interrupt::disable();
    let current_slot = slot_in(cpu::page_table_base(), address.get(), level).ok();
    slot.write(Entry::ABSENT);
    if current_slot == Some(slot) {
        cpu::invalidate_page(address);
    }
    disabled.restore();
}

pub(super) fn current_mapping_root(recorded: AddressSpaceRoots) -> Option<PhysicalAddress> {
    if !is_active() {
        return None;
    }
    let current = cpu::page_table_base();
    recorded.contains(current).then_some(current)
}

pub(super) fn kernel_image_gap(
    candidate: PhysicalAddress,
    current: PhysicalAddress,
) -> Option<u64> {
    let Some(sections) = image_sections() else {
        return Some(crate::boot::sections().entry_enclave.text.start);
    };
    for (section, writable, executable) in sections {
        let Some(end) = section.start.checked_add(section.mapped_len()) else {
            return Some(section.start);
        };
        let mut address = section.start;
        while address < end {
            let Some((current_physical, _)) = translate_in(current, address) else {
                return Some(address);
            };
            let Some((candidate_physical, flags)) = translate_in(candidate, address) else {
                return Some(address);
            };
            if candidate_physical != current_physical
                || flags.is_write_execute()
                || flags.contains(Flags::USER)
                || flags.contains(Flags::OWNED)
                || flags.contains(Flags::NO_CACHE)
                || flags.contains(Flags::WRITE_THROUGH)
                || flags.contains(Flags::WRITABLE) != writable
                || !flags.contains(Flags::NO_EXECUTE) != executable
            {
                return Some(address);
            }
            let Some(next) = address.checked_add(PAGE_SIZE) else {
                return Some(address);
            };
            address = next;
        }
    }
    None
}

const IMAGE_SECTIONS: usize = 3 + super::kpti::IMAGE_MAPPINGS;

const SIGN_EXTENSION: u64 = 0xffff_0000_0000_0000;

fn image_sections() -> Option<[(crate::boot::Section, bool, bool); IMAGE_SECTIONS]> {
    let sections = crate::boot::sections();
    let described = super::kpti::image_enclave()?.map(|mapping| {
        let flags = mapping.full_access().flags();
        (
            mapping.section(),
            flags.contains(Flags::WRITABLE),
            !flags.contains(Flags::NO_EXECUTE),
        )
    });
    Some([
        (sections.text, false, true),
        (sections.rodata, false, false),
        (sections.data, true, false),
        described[0],
        described[1],
        described[2],
        described[3],
    ])
}

#[derive(Clone, Copy, Debug)]
struct ImagePhysicalExtent {
    start: u64,
    end: u64,
    writable: bool,
}

fn image_physical_extents(
    current_root: PhysicalAddress,
) -> Option<[ImagePhysicalExtent; IMAGE_SECTIONS]> {
    let mut extents = [ImagePhysicalExtent {
        start: 0,
        end: 0,
        writable: false,
    }; IMAGE_SECTIONS];
    for (slot, (section, writable, _)) in extents.iter_mut().zip(image_sections()?) {
        let start = translate_in(current_root, section.start)?.0.get();
        let end = start.checked_add(section.mapped_len())?;
        let mut offset = 0;
        while offset < section.mapped_len() {
            let virtual_address = section.start.checked_add(offset)?;
            if translate_in(current_root, virtual_address)?.0.get() != start.checked_add(offset)? {
                return None;
            }
            offset = offset.checked_add(PAGE_SIZE)?;
        }
        *slot = ImagePhysicalExtent {
            start,
            end,
            writable,
        };
    }
    Some(extents)
}

pub(super) fn direct_map_gap(root: PhysicalAddress, current_root: PhysicalAddress) -> Option<u64> {
    let Some(direct) = mm::direct_map() else {
        return Some(0);
    };
    let offset = direct.offset();
    let Some(image) = image_physical_extents(current_root) else {
        return Some(offset);
    };

    // Copy the bounded map before walking a second lock-bearing structure.
    let Some(map) = mm::memory_map_snapshot() else {
        return Some(offset);
    };

    for extent in map.page_runs() {
        let mut physical = extent.base();
        let Some(extent_end) = extent.base().checked_add(extent.bytes()) else {
            return Some(physical);
        };
        while physical < extent_end {
            let Some(virtual_address) = offset.checked_add(physical) else {
                return Some(physical);
            };
            let image_policy = image
                .iter()
                .find(|section| section.start <= physical && physical < section.end);
            let policy_end = image_policy.map_or_else(
                || {
                    image
                        .iter()
                        .map(|section| section.start)
                        .filter(|start| *start > physical)
                        .min()
                        .unwrap_or(extent_end)
                },
                |section| section.end,
            );
            let (writable, executable, write_through) = image_policy.map_or(
                (
                    true,
                    false,
                    extent.cache() == crate::boot::DirectMapCache::Framebuffer,
                ),
                |section| (section.writable, false, false),
            );
            match walk(root, virtual_address) {
                Some((mapped, flags, span))
                    if mapped.get() == physical
                        && !flags.contains(Flags::USER)
                        && !flags.contains(Flags::OWNED)
                        && !flags.contains(Flags::NO_CACHE)
                        && flags.contains(Flags::WRITE_THROUGH) == write_through
                        && flags.contains(Flags::WRITABLE) == writable
                        && !flags.contains(Flags::NO_EXECUTE) == executable
                        && (span == PAGE_SIZE || !extent.is_allocatable()) =>
                {
                    let Some(mapped_end) =
                        physical.checked_add(span - (virtual_address & (span - 1)))
                    else {
                        return Some(virtual_address);
                    };
                    physical = mapped_end.min(policy_end).min(extent_end);
                }
                _ => return Some(virtual_address),
            }
        }
    }
    None
}

pub(super) fn handed_out_gap(root: PhysicalAddress) -> Option<u64> {
    let Some(direct) = mm::direct_map() else {
        return Some(0);
    };
    let offset = direct.offset();
    if let Some((start, len)) = crate::boot::framebuffer_extent() {
        let Some(physical) = start.checked_sub(offset) else {
            return Some(start);
        };
        if let Some(gap) = direct_extent_gap(root, offset, physical, len) {
            return Some(gap);
        }
    }
    let Some(map) = mm::memory_map_snapshot() else {
        return Some(offset);
    };
    map.regions()
        .filter(|region| region.len != 0 && region.kind.is_firmware_table())
        .find_map(|region| direct_extent_gap(root, offset, region.base, region.len))
}

fn direct_extent_gap(root: PhysicalAddress, offset: u64, physical: u64, len: u64) -> Option<u64> {
    let Some(end) = physical.checked_add(len) else {
        return Some(physical);
    };
    let mut page = physical - physical % PAGE_SIZE;
    while page < end {
        let Some(virtual_address) = offset.checked_add(page) else {
            return Some(page);
        };
        match translate_in(root, virtual_address) {
            Some((mapped, flags))
                if mapped.get() == page
                    && !flags.contains(Flags::USER)
                    && flags.contains(Flags::WRITABLE)
                    && flags.contains(Flags::NO_EXECUTE) => {}
            _ => return Some(virtual_address),
        }
        let Some(next) = page.checked_add(PAGE_SIZE) else {
            return Some(virtual_address);
        };
        page = next;
    }
    None
}

pub fn translate(virtual_address: u64) -> Option<(PhysicalAddress, Flags)> {
    translate_in(cpu::page_table_base(), virtual_address)
}

pub(super) fn translate_in(
    root: PhysicalAddress,
    virtual_address: u64,
) -> Option<(PhysicalAddress, Flags)> {
    walk(root, virtual_address).map(|(physical, flags, _)| (physical, flags))
}

pub(super) fn walk(
    root: PhysicalAddress,
    virtual_address: u64,
) -> Option<(PhysicalAddress, Flags, u64)> {
    if !x86_paging::is_canonical(virtual_address) {
        return None;
    }

    let mut walked = Flags::EVERYTHING_ALLOWED;

    let mut table = root;
    for level in Level::WALK {
        let slot = entry_slot(table, level.index(virtual_address)).ok()?;
        let entry = slot.read();
        if !entry.is_present() {
            return None;
        }

        // The huge bit means something else in a leaf table, so the level is checked first.
        walked = walked.through(entry.flags());

        if level == Level::Pt || entry.is_huge() {
            let span_mask = level.span() - 1;
            let offset = virtual_address & span_mask;
            let effective = walked.effective_for(entry.flags());
            // In a large leaf, bit 12 is PAT, not address.
            return Some((
                PhysicalAddress::new((entry.address() & !span_mask) + offset)?,
                effective,
                level.span(),
            ));
        }
        table = PhysicalAddress::from_entry(entry);
    }
    None
}

pub(super) fn visit_present_leaves(
    root: PhysicalAddress,
    first_pml4: usize,
    end_pml4: usize,
    mut visit: impl FnMut(u64, PhysicalAddress, Flags, u64) -> Result<(), MapError>,
) -> Result<(), MapError> {
    for pml4 in first_pml4..end_pml4 {
        let pml4_entry = entry_slot(root, pml4)?.read();
        if !pml4_entry.is_present() {
            continue;
        }
        let mut pml4_address = (pml4 as u64) << 39;
        if pml4 >= ENTRIES / 2 {
            pml4_address |= SIGN_EXTENSION;
        }
        if pml4_entry.is_huge() {
            return Err(MapError::InvalidEntryEnclave(pml4_address));
        }
        for pdpt in 0..ENTRIES {
            let pdpt_entry = entry_slot(PhysicalAddress::from_entry(pml4_entry), pdpt)?.read();
            if !pdpt_entry.is_present() {
                continue;
            }
            let pdpt_address = pml4_address | (pdpt as u64) << 30;
            if pdpt_entry.is_huge() {
                let Some((physical, flags, span)) = walk(root, pdpt_address) else {
                    return Err(MapError::InvalidEntryEnclave(pdpt_address));
                };
                visit(pdpt_address, physical, flags, span)?;
                continue;
            }
            for pd in 0..ENTRIES {
                let pd_entry = entry_slot(PhysicalAddress::from_entry(pdpt_entry), pd)?.read();
                if !pd_entry.is_present() {
                    continue;
                }
                let pd_address = pdpt_address | (pd as u64) << 21;
                if pd_entry.is_huge() {
                    let Some((physical, flags, span)) = walk(root, pd_address) else {
                        return Err(MapError::InvalidEntryEnclave(pd_address));
                    };
                    visit(pd_address, physical, flags, span)?;
                    continue;
                }
                for pt in 0..ENTRIES {
                    let entry = entry_slot(PhysicalAddress::from_entry(pd_entry), pt)?.read();
                    if !entry.is_present() {
                        continue;
                    }
                    let address = pd_address | (pt as u64) << 12;
                    let Some((physical, flags, span)) = walk(root, address) else {
                        return Err(MapError::InvalidEntryEnclave(address));
                    };
                    visit(address, physical, flags, span)?;
                }
            }
        }
    }
    Ok(())
}
