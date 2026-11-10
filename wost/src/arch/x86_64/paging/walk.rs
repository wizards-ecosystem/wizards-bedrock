use super::*;

#[derive(PartialEq, Eq, Debug)]
pub(super) struct EntrySlot {
    pointer: NonNull<Entry>,
}

impl EntrySlot {
    pub(super) fn read(&self) -> Entry {
        // SAFETY: the private callers supply a live table frame; construction
        // proves this aligned pointer names one complete entry inside it.
        unsafe { self.pointer.as_ptr().read_volatile() }
    }

    pub(super) fn write(&self, entry: Entry) {
        // SAFETY: private callers supply a live table frame and own the protocol excluding other
        // writers; construction proves this aligned pointer names one complete entry inside it.
        unsafe { self.pointer.as_ptr().write_volatile(entry) };
    }
}

pub(super) fn entry_slot(table: Paddr, index: usize) -> Result<EntrySlot, MapError> {
    assert!(index < ENTRIES, "a level's index is always below 512");
    let base = mm::direct_pointer(table, PAGE_SIZE).ok_or(MapError::Unreachable(table))?;
    let pointer = base.cast::<Entry>().wrapping_add(index);
    let pointer = NonNull::new(pointer).ok_or(MapError::Unreachable(table))?;
    Ok(EntrySlot { pointer })
}

pub(super) fn slot_in(root: Paddr, address: u64, target: Level) -> Result<EntrySlot, MapError> {
    let mut table = root;
    for level in Level::WALK {
        let slot = entry_slot(table, level.index(address))?;
        if level == target {
            return Ok(slot);
        }
        let entry = EntrySlot::read(&slot);
        if !entry.is_present() {
            return Err(MapError::NotMapped(address));
        }
        if entry.is_huge() {
            return Err(MapError::Unsplittable(address));
        }
        table = entry.address();
    }
    Err(MapError::NotMapped(address))
}

pub(super) fn leaf_in(root: Paddr, address: u64) -> Result<EntrySlot, MapError> {
    let slot = slot_in(root, address, Level::Pt)?;
    EntrySlot::read(&slot)
        .is_present()
        .then_some(slot)
        .ok_or(MapError::NotMapped(address))
}

pub(super) fn clear_mapping_slot(slot: EntrySlot, address: u64, level: Level) {
    let address = canonical_mapping_address(address);
    let disabled = interrupt::disable();
    let current_slot = slot_in(cpu::page_table_base(), address.get(), level).ok();
    EntrySlot::write(&slot, Entry::ABSENT);
    if current_slot == Some(slot) {
        cpu::invalidate_page(address);
    }
    disabled.restore();
}

pub(super) fn current_mapping_root(recorded: AddressSpaceRoots) -> Option<Paddr> {
    if !is_active() {
        return None;
    }
    let current = cpu::page_table_base();
    recorded.contains(current).then_some(current)
}

pub(super) fn canonical_mapping_address(address: u64) -> CanonicalAddress {
    let Some(address) = CanonicalAddress::new(address) else {
        panic!("a validated mapping produced a noncanonical invalidation address");
    };
    address
}

/// First kernel-image page redirected or assigned the wrong protection class.
pub(super) fn kernel_image_gap(candidate: Paddr, current: Paddr) -> Option<u64> {
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
    let Some(enclave) = super::kpti::entry_enclave() else {
        return Some(crate::boot::sections().entry_enclave.text.start);
    };
    for guard in enclave.guards() {
        let mut address = guard.start;
        while address < guard.end {
            if translate_in(candidate, address).is_some() {
                return Some(address);
            }
            address = address.checked_add(PAGE_SIZE)?;
        }
    }
    None
}

fn image_sections() -> Option<[(crate::boot::Section, bool, bool); 9]> {
    let sections = crate::boot::sections();
    let [text, rodata, tables, route, transition_stack, ist_stack] =
        super::kpti::entry_enclave()?.mappings();
    let described = [text, rodata, tables, route, transition_stack, ist_stack].map(|mapping| {
        let (writable, executable) = enclave_access_policy(mapping.full_access());
        (mapping.section(), writable, executable)
    });
    Some([
        (sections.text, false, true),
        (sections.rodata, false, false),
        (sections.data, true, false),
        described[0],
        described[1],
        described[2],
        described[3],
        described[4],
        described[5],
    ])
}

#[derive(Clone, Copy)]
struct ImagePhysicalExtent {
    start: Paddr,
    end: Paddr,
    writable: bool,
    executable: bool,
}

fn image_physical_extents(current_root: Paddr) -> Option<[ImagePhysicalExtent; 9]> {
    let mut extents = [ImagePhysicalExtent {
        start: 0,
        end: 0,
        writable: false,
        executable: false,
    }; 9];
    for (slot, (section, writable, executable)) in extents.iter_mut().zip(image_sections()?) {
        let start = translate_in(current_root, section.start)?.0;
        let end = start.checked_add(section.mapped_len())?;
        let mut offset = 0;
        while offset < section.mapped_len() {
            let virtual_address = section.start.checked_add(offset)?;
            if translate_in(current_root, virtual_address)?.0 != start.checked_add(offset)? {
                return None;
            }
            offset = offset.checked_add(PAGE_SIZE)?;
        }
        *slot = ImagePhysicalExtent {
            start,
            end,
            writable,
            executable,
        };
    }
    Some(extents)
}

/// The first direct-map address `root` gets wrong, refused while the switch can still fail,
/// because every frame access after it is raw arithmetic against this mapping.
pub(super) fn direct_map_gap(root: Paddr, current_root: Paddr) -> Option<u64> {
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
                |section| (section.writable, section.executable, false),
            );
            match walk(root, virtual_address) {
                Some((mapped, flags, span))
                    if mapped == physical
                        && !flags.contains(Flags::USER)
                        && !flags.contains(Flags::OWNED)
                        && !flags.contains(Flags::NO_CACHE)
                        && flags.contains(Flags::WRITE_THROUGH) == write_through
                        && flags.contains(Flags::WRITABLE) == writable
                        && !flags.contains(Flags::NO_EXECUTE) == executable =>
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

/// What the table in `CR3`, never a remembered root, maps `virtual_address` to.
pub fn translate(virtual_address: u64) -> Option<(Paddr, Flags)> {
    translate_in(cpu::page_table_base(), virtual_address)
}

pub(super) fn translate_in(root: Paddr, virtual_address: u64) -> Option<(Paddr, Flags)> {
    walk(root, virtual_address).map(|(physical, flags, _)| (physical, flags))
}

/// As [`translate_in`], also naming the leaf's span so a sweep can step by whole entries.
pub(super) fn walk(root: Paddr, virtual_address: u64) -> Option<(Paddr, Flags, u64)> {
    if !x86_paging::is_canonical(virtual_address) {
        return None;
    }

    // The fold is host-tested in `x86-paging`; no safe parent is narrower than its child.
    let mut walked = Flags::EVERYTHING_ALLOWED;

    let mut table = root;
    for level in Level::WALK {
        let slot = entry_slot(table, level.index(virtual_address)).ok()?;
        let entry = EntrySlot::read(&slot);
        if !entry.is_present() {
            return None;
        }

        // The huge bit means something else in a leaf table, so the level is checked first.
        walked = walked.through(entry.flags());

        if level == Level::Pt || entry.is_huge() {
            let offset = virtual_address & (level.span() - 1);
            let effective = walked.effective_for(entry.flags());
            return Some((entry.address() + offset, effective, level.span()));
        }
        table = entry.address();
    }
    None
}

pub(super) fn visit_present_leaves(
    root: Paddr,
    first_pml4: usize,
    end_pml4: usize,
    mut visit: impl FnMut(u64, Paddr, Flags, u64) -> Result<(), MapError>,
) -> Result<(), MapError> {
    for pml4 in first_pml4..end_pml4 {
        let pml4_entry = EntrySlot::read(&entry_slot(root, pml4)?);
        if !pml4_entry.is_present() {
            continue;
        }
        let mut pml4_address = (pml4 as u64) << 39;
        if pml4 >= ENTRIES / 2 {
            pml4_address |= 0xffff_0000_0000_0000;
        }
        if pml4_entry.is_huge() {
            return Err(MapError::InvalidEntryEnclave(pml4_address));
        }
        for pdpt in 0..ENTRIES {
            let pdpt_entry = EntrySlot::read(&entry_slot(pml4_entry.address(), pdpt)?);
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
                let pd_entry = EntrySlot::read(&entry_slot(pdpt_entry.address(), pd)?);
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
                    let entry = EntrySlot::read(&entry_slot(pd_entry.address(), pt)?);
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
