use ring_proto::SharedLayout;
use stack_layout::Slot as StackSlot;
use wost::arch::current::cpu::{self, Protections};
use wost::arch::current::paging::{
    self, ActivateError, AddressSpace, BootRegion, MapError, ReclaimError,
};
use wost::arch::current::usercopy::{self, UserCopyError};
use wost::arch::current::{interrupt, segment, task};
use wost::boot::{self, MemoryRegion};
use wost::mm;
use x86_paging::{CanonicalAddress, Flags, LARGE_PAGE_SIZE, PAGE_SIZE};

use super::processes::{AllocatorSnapshot, boundary_probe};
use super::runs_until;
use super::threads::{context, leave_at_once};
use crate::thread::ThreadContext;
use crate::user;
use crate::{mem, thread, warn};

pub(super) fn the_loaded_table_is_ours() -> bool {
    paging::is_active()
        && cpu::page_table_base() == paging::active_root()
        && cpu::page_table_context_ids_are_disabled_for_test()
}

pub(super) fn an_admitted_canonical_page_can_be_invalidated() -> bool {
    let raw = an_admitted_canonical_page_can_be_invalidated as *const () as u64;
    let Some(before) = paging::translate(raw) else {
        return false;
    };
    let Some(address) = CanonicalAddress::new(raw) else {
        return false;
    };
    cpu::invalidate_page(address);
    paging::translate(raw) == Some(before)
}

// Sample addresses are offsets from a section's base, never a sum of two higher-half addresses:
// those are within a hair of `u64::MAX` and adding a pair overflows.
pub(super) fn the_image_obeys_write_xor_execute() -> bool {
    let sections = boot::sections();
    let enclave = sections.entry_enclave;
    let Some(direct) = mm::direct_map() else {
        return false;
    };

    [
        (sections.text, false, true),
        (sections.rodata, false, false),
        (sections.data, true, false),
        (enclave.text, false, true),
        (enclave.rodata, false, false),
        (enclave.tables, true, false),
        (enclave.route, true, false),
        (enclave.transition_stack, true, false),
        (enclave.ist_stack, true, false),
    ]
    .into_iter()
    .all(|(section, writable, executable)| {
        let len = section.mapped_len();
        let middle = section.start + ((len / 2) & !4095);
        [section.start, middle, section.start + len - 4096]
            .into_iter()
            .all(|address| {
                let Some((physical, flags)) = paging::translate(address) else {
                    return false;
                };
                let Some(alias) = direct.virtual_address(physical) else {
                    return false;
                };
                let Some((alias_physical, alias_flags)) = paging::translate(alias) else {
                    return false;
                };
                [flags, alias_flags].into_iter().all(|flags| {
                    !flags.is_write_execute()
                        && flags.contains(Flags::WRITABLE) == writable
                        && !flags.contains(Flags::NO_EXECUTE) == executable
                }) && alias_physical == physical
            })
    })
}

pub(super) fn the_entry_enclave_guards_are_absent() -> bool {
    let enclave = boot::sections().entry_enclave;
    [enclave.transition_guard, enclave.ist_guard]
        .into_iter()
        .all(|guard| {
            let mut address = guard.start;
            while address < guard.end {
                if paging::translate(address).is_some() {
                    return false;
                }
                let Some(next) = address.checked_add(PAGE_SIZE) else {
                    return false;
                };
                address = next;
            }
            address == guard.end
        })
}

pub(super) fn supervisor_writes_obey_read_only_pages() -> bool {
    cpu::drive_write_protection_for_test()
        && cpu::write_protection_probe()
        && Protections::enabled().write_protect
}

// Any frame the allocator owns is inside a region the direct map covers, so its direct-map address
// is a fair sample of the whole mapping.
pub(super) fn the_direct_map_is_not_executable() -> bool {
    let Some(frame) = mm::allocate() else {
        return false;
    };
    let Some(address) = mm::direct_map().and_then(|map| map.virtual_address(frame.address()))
    else {
        return false;
    };

    match paging::translate(address) {
        Some((physical, flags)) => {
            physical == frame.address()
                && flags.contains(Flags::WRITABLE)
                && flags.contains(Flags::NO_EXECUTE)
                && !flags.is_write_execute()
        }
        None => false,
    }
}

// The undescribed address is inside the address space, so only the map can refuse it; an address
// at the top would be caught by an overflow guard instead.
pub(super) fn firmware_reads_are_bounded() -> bool {
    const WORD: usize = 8;

    let Some(map) = mm::memory_map_for_test() else {
        return false;
    };
    let pick = |wanted: fn(&MemoryRegion) -> bool| {
        map.regions()
            .find(|region| wanted(region) && region.len >= WORD as u64)
            .map(|region| region.base)
    };
    let addresses = (
        pick(|region| region.kind.is_allocatable()),
        pick(|region| !region.kind.is_firmware_table() && !region.kind.is_allocatable()),
        pick(|region| region.kind.is_firmware_table()),
        map.regions()
            .map(|region| region.base + region.len)
            .max()
            .and_then(|end| end.checked_next_multiple_of(PAGE_SIZE)),
    );

    let (Some(usable), Some(unreachable), Some(readable), Some(undescribed)) = addresses else {
        return false;
    };

    let Some(firmware) = mm::firmware_memory() else {
        return false;
    };
    let mut buffer = [0; WORD];
    !wost::mm::FirmwareMemory::read(&firmware, usable, &mut buffer)
        && !wost::mm::FirmwareMemory::read(&firmware, undescribed, &mut buffer)
        && !wost::mm::FirmwareMemory::read(&firmware, unreachable, &mut buffer)
        && wost::mm::FirmwareMemory::read(&firmware, readable, &mut buffer)
}

pub(super) fn in_reclaimed_space(
    constructor: impl FnOnce() -> Result<AddressSpace, MapError>,
    check: impl FnOnce(&mut AddressSpace) -> bool,
) -> bool {
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = constructor() else {
        return false;
    };
    let checked = check(&mut space);
    let reclaimed = space.reclaim(None).is_ok();
    drop(space);
    checked && reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

fn pages_above_the_boot_stack(pages: u64) -> Option<u64> {
    thread::boot_stack_layout().and_then(|layout| layout.end().checked_add(pages * PAGE_SIZE))
}

pub(super) fn a_second_switch_is_refused() -> bool {
    in_reclaimed_space(AddressSpace::new, |space| {
        paging::activate(space) == Err(ActivateError::AlreadyActive)
    })
}

// `OWNED` is expected because `map_owned` writes it: the entry is where provenance lives.
pub(super) fn a_mapping_added_after_the_switch_is_live() -> bool {
    let Some(probe) = pages_above_the_boot_stack(4) else {
        return false;
    };

    let free_before = mm::free_frames();
    let unmapped_before = paging::translate(probe).is_none();

    let Some(run) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let physical = run.address();

    let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let Some(Ok(mapping)) = mem::map_owned_for_test(&mut context(), probe, run, wanted) else {
        return false;
    };

    let expected = wanted.union(Flags::PRESENT).union(Flags::OWNED);
    let translated = paging::translate(probe) == Some((physical, expected));
    let on_the_live_root = paging::active_root() == cpu::page_table_base();
    let Some(Ok(returned)) = mem::unmap_for_test(&mut context(), mapping) else {
        return false;
    };
    drop(returned);
    let restored = paging::translate(probe).is_none() && mm::free_frames() == free_before;

    unmapped_before && translated && on_the_live_root && restored
}

pub(super) fn two_spaces_map_one_address_to_different_memory() -> bool {
    /// A user address neither space has anything else at.
    const SHARED: u64 = user::CODE + 0x10_0000;

    if !runs_until(2000, || !user::has_zombies()) {
        return false;
    }
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut first) = AddressSpace::for_user() else {
        return false;
    };
    let Ok(mut second) = AddressSpace::for_user() else {
        let reclaimed = first.reclaim(None).is_ok();
        drop(first);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a missing second space did not return the first space's tables"
        );
        return false;
    };

    let mut first_mapped = false;
    let mut second_mapped = false;
    let checked = (|| {
        if first.root() == second.root() {
            return false;
        }

        let writable = Flags::WRITABLE.union(Flags::NO_EXECUTE);
        let (Some(mut one), Some(mut two)) =
            (mm::allocate_contiguous(1, 1), mm::allocate_contiguous(1, 1))
        else {
            return false;
        };
        if !one.zero() || !two.zero() || !one.write_at(0, &[0xA5]) || !two.write_at(0, &[0x5A]) {
            return false;
        }
        let (one_at, two_at) = (one.address(), two.address());
        if one_at == two_at || first.map_user(SHARED, one.into(), writable).is_err() {
            return false;
        }
        first_mapped = true;
        if second.map_user(SHARED, two.into(), writable).is_err() {
            return false;
        }
        second_mapped = true;

        let (Some((first_frame, first_flags)), Some((second_frame, second_flags))) =
            (first.lookup(SHARED), second.lookup(SHARED))
        else {
            return false;
        };
        let separate = first_frame == one_at && second_frame == two_at;
        let both_user = first_flags.contains(Flags::USER) && second_flags.contains(Flags::USER);

        let text = boot::sections().text.start;
        let shared_kernel =
            first.lookup(text) == second.lookup(text) && first.lookup(text).is_some();
        separate && both_user && shared_kernel
    })();

    let first_unmapped = !first_mapped || first.unmap(SHARED, PAGE_SIZE).map(drop).is_ok();
    let second_unmapped = !second_mapped || second.unmap(SHARED, PAGE_SIZE).map(drop).is_ok();
    let first_reclaimed = first.reclaim(None).is_ok();
    let second_reclaimed = second.reclaim(None).is_ok();
    drop((first, second));

    checked
        && first_unmapped
        && second_unmapped
        && first_reclaimed
        && second_reclaimed
        && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

// Distinct kernel bytes keep accidental publication from looking zeroed, and a successful
// reclaim alone does not prove that the bytes were cleared.
pub(super) fn kernel_memory_cannot_be_published_to_ring_three() -> bool {
    /// A user address nothing else uses.
    const PROBE: u64 = user::STACK + 0x40_0000;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some(mut run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !run.zero() {
            return false;
        }

        let secret = [0xDEu8; 8];
        if !run.write_at(0, &secret) {
            return false;
        }

        let refused = matches!(
            space.map_owned(PROBE, run, Flags::USER.union(Flags::WRITABLE)),
            Err(MapError::NotYours(PROBE))
        );
        if !refused {
            return false;
        }

        let Some(mut giving) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !giving.zero() || !giving.write_at(0, &secret) {
            return false;
        }
        let untyped: mm::USegment = giving.into();
        let address = untyped.address();

        let Some(back) = untyped.reclaim() else {
            return false;
        };
        let mut recovered = [0xFFu8; 8];
        let cleared =
            back.address() == address && back.read_at(0, &mut recovered) && recovered == [0u8; 8];
        drop(back);

        let Some(mut given) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !given.zero() {
            return false;
        }
        if space
            .map_user(
                PROBE,
                given.into(),
                Flags::WRITABLE.union(Flags::NO_EXECUTE),
            )
            .is_err()
        {
            return false;
        }
        let user_accessible = space
            .lookup(PROBE)
            .is_some_and(|(_, flags)| flags.contains(Flags::USER));
        let unmapped = space.unmap(PROBE, PAGE_SIZE).map(drop).is_ok();

        refused && cleared && user_accessible && unmapped
    })
}

fn mark_external_run(run: &mut mm::Segment, marker: u8) -> bool {
    let expected = [marker; 16];
    let mut found = [0; 16];
    run.write_at(0, &expected) && run.read_at(0, &mut found) && found == expected
}

pub(super) fn external_user_bytes_are_zeroed_before_free() -> bool {
    const DIRECT: u8 = 0xa1;
    const SHARED: u8 = 0xb2;
    const REFUSED: u8 = 0xc3;
    const OUTSIDE_USER_HALF: u64 = 0xffff_8000_0010_0000;

    let direct = {
        let Some(mut run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !mark_external_run(&mut run, DIRECT) {
            return false;
        }
        let address = run.address();
        drop(mm::USegment::from(run));
        mm::freed_external_run_is_zero_for_test(address, 1)
    };

    let shared = {
        let Some(mut run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !mark_external_run(&mut run, SHARED) {
            return false;
        }
        let address = run.address();
        drop(mm::USegment::from(run).share());
        mm::freed_external_run_is_zero_for_test(address, 1)
    };

    let refused = in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some(mut run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !mark_external_run(&mut run, REFUSED) {
            return false;
        }
        let address = run.address();
        let Err(error) = space.map_user(
            OUTSIDE_USER_HALF,
            run.into(),
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        ) else {
            return false;
        };
        let reason = error.reason();
        let run = error.into_run();
        let retained = reason == MapError::NotYours(OUTSIDE_USER_HALF) && run.address() == address;
        drop(run);
        retained && mm::freed_external_run_is_zero_for_test(address, 1)
    });

    direct && shared && refused
}

pub(super) fn a_refused_external_clear_withholds_its_run() -> bool {
    const MARKER: u8 = 0xd4;

    let free_before = mm::free_frames();
    let Some(mut run) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !mark_external_run(&mut run, MARKER) {
        return false;
    }
    let withheld = mm::refused_external_clear_withholds_for_test(run, &[MARKER; 16]);
    withheld && mm::free_frames() == free_before
}

pub(super) fn only_a_space_that_matches_the_kernel_can_be_installed() -> bool {
    let bare_refused = in_reclaimed_space(AddressSpace::new, |bare| {
        matches!(bare.bind(), Err(ActivateError::NotAUserSpace))
    });

    let binds = {
        let free_before = mm::free_frames();
        let Ok(mut shared) = AddressSpace::for_user() else {
            return false;
        };
        let released = shared.bind().ok().map(task::release_uninstalled);
        let bound = released.is_some();
        let reclaimed = shared.reclaim(released).is_ok();
        bound && reclaimed && mm::free_frames() == free_before
    };

    let device_refused = in_reclaimed_space(AddressSpace::for_user, |probe| {
        matches!(
            paging::admit_device_range(0xfee0_0000, PAGE_SIZE)
                .and_then(|range| probe.map_device(range)),
            Err(MapError::NotYours(_))
        )
    });

    bare_refused && binds && device_refused
}

pub(super) fn an_active_unmap_exposes_only_its_replacement() -> bool {
    const FIRST: [u8; 16] = [0xa5; 16];
    const SECOND: [u8; 16] = [0x5a; 16];

    let Some((_, stack)) = boundary_probe() else {
        return false;
    };
    let Some(at) = stack.checked_add(PAGE_SIZE) else {
        return false;
    };
    let Some(mut first) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !first.zero() || !first.write_at(0, &FIRST) {
        return false;
    }
    let first_physical = first.address();
    let first_flags = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let mut context = context();
    let Some(Ok(first_mapping)) = mem::map_user_page_for_test(&mut context, at, first, first_flags)
    else {
        return false;
    };

    let mut warmed = [0; FIRST.len()];
    let warmed_first = usercopy::copy_from_user(at, &mut warmed).is_ok() && warmed == FIRST;
    let Some(Ok(first_taken)) = mem::unmap_for_test(&mut context, first_mapping) else {
        return false;
    };

    let Some(mut second) = mm::allocate_contiguous(1, 1) else {
        drop(first_taken);
        return false;
    };
    if !second.zero() || !second.write_at(0, &SECOND) {
        drop(first_taken);
        return false;
    }
    let second_physical = second.address();
    let Some(Ok(second_mapping)) =
        mem::map_user_page_for_test(&mut context, at, second, Flags::NONE)
    else {
        drop(first_taken);
        return false;
    };

    let replacement_is_distinct = first_physical != second_physical
        && paging::translate(at).is_some_and(|(physical, flags)| {
            physical == second_physical
                && flags.contains(Flags::PRESENT)
                && flags.contains(Flags::USER)
                && flags.contains(Flags::OWNED)
                && !flags.contains(Flags::WRITABLE)
                && !flags.contains(Flags::NO_EXECUTE)
        });
    let mut observed = [0; SECOND.len()];
    let replacement_was_read =
        usercopy::copy_from_user(at, &mut observed).is_ok() && observed == SECOND;
    let replacement_is_read_only = matches!(
        usercopy::copy_to_user(at, &FIRST),
        Err(UserCopyError::ReadOnly(found)) if found == at
    );

    let second_removed = mem::unmap_for_test(&mut context, second_mapping)
        .is_some_and(|result| result.map(drop).is_ok());
    drop(first_taken);

    warmed_first
        && replacement_is_distinct
        && replacement_was_read
        && replacement_is_read_only
        && second_removed
}

pub(super) fn a_shared_kernel_unmap_evicts_the_live_user_root() -> bool {
    let Some(probe) = pages_above_the_boot_stack(24) else {
        return false;
    };

    let Ok(mut observer) = AddressSpace::for_user() else {
        return false;
    };
    let checked = the_observer_reads_the_replaced_shared_leaf(&observer, probe);
    let reclaimed = observer.reclaim(None).is_ok();
    checked && reclaimed
}

fn the_observer_reads_the_replaced_shared_leaf(observer: &AddressSpace, probe: u64) -> bool {
    const FIRST: u8 = 0xa6;
    const SECOND: u8 = 0x59;

    if observer.lookup(probe).is_some() {
        return false;
    }
    let Some(mut first) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !first.zero() || !first.write_at(0, &[FIRST]) {
        return false;
    }
    let first_physical = first.address();
    let flags = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let mut context = context();
    let Some(Ok(mapping)) = mem::map_owned_for_test(&mut context, probe, first, flags) else {
        return false;
    };
    let mut live_mapping = Some(mapping);

    let observed = observer
        .with_root_for_test(|| {
            let warmed = paging::observe_owned_byte_for_test(probe) == Some(FIRST);
            let Some(mapping) = live_mapping.take() else {
                return false;
            };
            let first_taken = match mem::unmap_for_test(&mut context, mapping) {
                Some(Ok(taken)) => taken,
                _ => panic!("the shared-root fixture could not remove its first leaf"),
            };

            let Some(mut second) = mm::allocate_contiguous(1, 1) else {
                return false;
            };
            if !second.zero() || !second.write_at(0, &[SECOND]) {
                return false;
            }
            let second_physical = second.address();
            let Some(Ok(second_mapping)) =
                mem::map_owned_for_test(&mut context, probe, second, flags)
            else {
                return false;
            };
            let table_changed = first_physical != second_physical
                && paging::translate(probe)
                    .is_some_and(|(physical, _)| physical == second_physical);
            let replacement = paging::observe_owned_byte_for_test(probe) == Some(SECOND);
            let second_taken = match mem::unmap_for_test(&mut context, second_mapping) {
                Some(Ok(taken)) => taken,
                _ => panic!("the shared-root fixture could not remove its replacement leaf"),
            };
            drop(second_taken);
            drop(first_taken);
            warmed && table_changed && replacement
        })
        .unwrap_or(false);

    let cleaned = match live_mapping.take() {
        Some(mapping) => match mem::unmap_for_test(&mut context, mapping) {
            Some(Ok(taken)) => {
                drop(taken);
                true
            }
            _ => panic!("the shared-root fixture could not clean its unused leaf"),
        },
        None => true,
    };
    observed && cleaned && paging::translate(probe).is_none()
}

// The ownership snapshots describe only this fixture, so no timer may hand the CPU to the cleanup
// worker, whose retirement of older zombies between those snapshots would make correct local
// reclamation look imbalanced.
pub(super) fn an_inactive_lower_root_unmap_preserves_the_live_observer() -> bool {
    const AT: u64 = 0x0000_0500_0000_0000;

    let disabled = interrupt::disable();
    let result = (|| {
        let allocator_before = AllocatorSnapshot::capture();
        let Ok(mut observer) = AddressSpace::for_user() else {
            return false;
        };
        let Ok(mut victim) = AddressSpace::for_user() else {
            if let Err(reason) = observer.reclaim(None) {
                warn!("selftest: observer cleanup after victim-space setup refusal: {reason}");
            }
            return false;
        };

        let checked = the_victim_unmap_spares_the_observer(&mut observer, &mut victim, AT);

        let observer_cleaned =
            observer.lookup(AT).is_none() || observer.unmap(AT, PAGE_SIZE).map(drop).is_ok();
        let victim_cleaned =
            victim.lookup(AT).is_none() || victim.unmap(AT, PAGE_SIZE).map(drop).is_ok();
        let observer_reclaimed = observer.reclaim(None).is_ok();
        let victim_reclaimed = victim.reclaim(None).is_ok();
        drop(observer);
        drop(victim);
        let allocator_after = AllocatorSnapshot::capture();
        let ownership_preserved = allocator_after.has_same_ownership(&allocator_before);

        let result = checked
            && observer_cleaned
            && victim_cleaned
            && observer_reclaimed
            && victim_reclaimed
            && ownership_preserved;
        if !result {
            warn!(
                "selftest: inactive-unmap cleanup checked={checked} observer-cleaned={observer_cleaned} victim-cleaned={victim_cleaned} observer-reclaimed={observer_reclaimed} victim-reclaimed={victim_reclaimed} ownership-preserved={ownership_preserved} before={allocator_before:?} after={allocator_after:?}"
            );
        }
        result
    })();
    disabled.restore();
    result
}

fn the_victim_unmap_spares_the_observer(
    observer: &mut AddressSpace,
    victim: &mut AddressSpace,
    at: u64,
) -> bool {
    const OBSERVER: u8 = 0x3c;
    const VICTIM: u8 = 0xc3;

    if observer.root() == victim.root() {
        return false;
    }
    let (Some(mut observer_page), Some(mut victim_page)) =
        (mm::allocate_contiguous(1, 1), mm::allocate_contiguous(1, 1))
    else {
        return false;
    };
    if !observer_page.zero()
        || !observer_page.write_at(0, &[OBSERVER])
        || !victim_page.zero()
        || !victim_page.write_at(0, &[VICTIM])
    {
        return false;
    }
    let observer_physical = observer_page.address();
    let victim_physical = victim_page.address();
    let flags = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    if observer.map_user(at, observer_page.into(), flags).is_err()
        || victim.map_user(at, victim_page.into(), flags).is_err()
    {
        return false;
    }
    let leaves_are_distinct = observer_physical != victim_physical
        && observer
            .lookup(at)
            .is_some_and(|(physical, _)| physical == observer_physical)
        && victim
            .lookup(at)
            .is_some_and(|(physical, _)| physical == victim_physical);
    if !leaves_are_distinct {
        return false;
    }

    observer
        .with_root_for_test(|| {
            let mut byte = [0];
            let warmed = usercopy::copy_from_user(at, &mut byte).is_ok() && byte == [OBSERVER];
            let warmed_translation = paging::translate(at);
            let warmed_leaf = observer.lookup(at);
            let invalidations_before = cpu::page_invalidations_for_test();
            let removed = victim.unmap(at, PAGE_SIZE).map(drop).is_ok();
            let invalidations_after = cpu::page_invalidations_for_test();
            let no_eviction = invalidations_after == invalidations_before;
            byte = [0];
            let observer_unchanged = paging::translate(at) == warmed_translation
                && observer.lookup(at) == warmed_leaf
                && usercopy::copy_from_user(at, &mut byte).is_ok()
                && byte == [OBSERVER];
            let victim_cleared = victim.lookup(at).is_none();
            let result = warmed
                && warmed_translation.is_some()
                && warmed_translation == warmed_leaf
                && removed
                && victim_cleared
                && no_eviction
                && observer_unchanged;
            if !result {
                warn!(
                    "selftest: inactive-unmap evidence warmed={warmed} translation={} leaf-match={} removed={removed} victim-cleared={victim_cleared} invalidations={invalidations_before}->{invalidations_after} observer-unchanged={observer_unchanged}",
                    warmed_translation.is_some(),
                    warmed_translation == warmed_leaf,
                );
            }
            result
        })
        .unwrap_or(false)
}

// An unmapped user page comes back untyped, and only a clearing conversion makes it the kernel's
// again; the check follows the handle and never asks the allocator. The kernel conversion
// cannot hand a user range over as a `Segment`.
// Reallocation need not return these frames, so a tally observes the drop clear without relying
// on same-frame readback.
pub(super) fn a_user_page_comes_back_cleared() -> bool {
    /// Two user addresses nothing else uses.
    const FIRST: u64 = user::STACK + 0xA0_0000;
    const SECOND: u64 = user::STACK + 0xA1_0000;
    /// What a program would have left in the page.
    const SECRET: [u8; 8] = [0xDE; 8];

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let writable = Flags::WRITABLE.union(Flags::NO_EXECUTE);

        let Some(mut run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !run.zero() || !run.write_at(0, &SECRET) {
            return false;
        }
        let address = run.address();
        if space.map_user(FIRST, run.into(), writable).is_err() {
            return false;
        }

        let Ok(unmapped) = space.unmap(FIRST, PAGE_SIZE) else {
            return false;
        };
        let knows = unmapped.was_user();
        let Some(untyped) = unmapped.into_user() else {
            return false;
        };
        let Some(back) = untyped.reclaim() else {
            return false;
        };
        let mut recovered = [0xFFu8; 8];
        let cleared = back.address() == address
            && back.read_at(0, &mut recovered)
            && recovered == [0u8; 8]
            && recovered != SECRET;
        drop(back);

        let Some(mut other) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !other.zero() {
            return false;
        }
        if space.map_user(SECOND, other.into(), writable).is_err() {
            return false;
        }
        let Ok(second) = space.unmap(SECOND, PAGE_SIZE) else {
            return false;
        };
        let refused_as_kernel = second.into_kernel().is_none();

        let cleared_before = paging::cleared_on_drop();
        let Some(mut third) = mm::allocate_contiguous(2, 1) else {
            return false;
        };
        if !third.zero() {
            return false;
        }
        if space
            .map_user(SECOND + PAGE_SIZE * 4, third.into(), writable)
            .is_err()
        {
            return false;
        }
        let Ok(dropped) = space.unmap(SECOND + PAGE_SIZE * 4, 2 * PAGE_SIZE) else {
            return false;
        };
        drop(dropped);
        let counted = paging::cleared_on_drop() == cleared_before + 2;

        knows && cleared && refused_as_kernel && counted
    })
}

// Refused with no evidence and left untouched, so the same value can be reclaimed once the task
// table supplies the evidence, which shows the refusal is not a reclaim that never works.
pub(super) fn a_bound_space_is_not_reclaimed() -> bool {
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let root = space.root();
    let Ok(binding) = space.bind() else {
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a refused binding did not return its address-space tables"
        );
        return false;
    };

    let mut bound = None;
    let slot =
        match thread::spawn_in_space("bound-space", leave_at_once, binding, |_, installed| {
            bound = Some(installed);
            Ok(())
        }) {
            Ok(slot) => slot,
            Err(failure) => {
                let released = match failure.authority {
                    thread::SpaceAuthority::None => None,
                    thread::SpaceAuthority::Released(released) => Some(released),
                };
                let reclaimed = space.reclaim(released).is_ok();
                drop(space);
                assert!(
                    reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
                    "a refused bound-space setup did not return its tables"
                );
                return false;
            }
        };
    let bound = bound.expect("a successful arrangement retains its binding");

    let bound = match task::release_address_space(bound) {
        Ok(released) => {
            let reclaimed = space.reclaim(Some(released)).is_ok_and(|tables| tables > 0);
            drop(space);
            assert!(
                reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
                "an unexpected bound-space release did not return its tables"
            );
            return false;
        }
        Err(bound) => bound,
    };

    let held = mm::free_frames();
    let refused = matches!(
        space.reclaim(None),
        Err(paging::ReclaimFailure {
            reason: ReclaimError::StillBound(found),
            ..
        }) if found == root
    ) && !space.is_reclaimed()
        && mm::free_frames() == held;

    let gone = runs_until(500, || !task::slot_is_live_for_test(slot));
    assert!(gone, "the bound-space witness did not retire");
    let released = task::release_address_space(bound)
        .expect("a retired bound-space witness releases its binding");
    let reclaimed = release_evidence_survives_other_targets(
        &mut space,
        released,
        ReclaimError::UnexpectedRelease(root),
    );
    drop(space);

    refused && reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

// Release evidence survives a foreign and a repeated target, and the rightful space is still
// reclaimable with it afterwards.
fn release_evidence_survives_other_targets(
    space: &mut AddressSpace,
    released: task::Released,
    foreign_refusal: ReclaimError,
) -> bool {
    let Ok(mut foreign) = AddressSpace::for_user() else {
        assert!(
            space.reclaim(Some(released)).is_ok(),
            "a failed foreign-space setup stranded release evidence"
        );
        return false;
    };
    let foreign_failure = match foreign.reclaim(Some(released)) {
        Err(failure) => failure,
        Ok(_) => panic!("a foreign space consumed release evidence"),
    };
    let foreign_refused = foreign_failure.reason == foreign_refusal && !foreign.is_reclaimed();
    let Some(released) = foreign_failure.released else {
        panic!("a foreign-space refusal lost release evidence");
    };
    let foreign_reclaimed = foreign.reclaim(None).is_ok() && foreign.is_reclaimed();
    let repeated_failure = match foreign.reclaim(Some(released)) {
        Err(failure) => failure,
        Ok(_) => panic!("a reclaimed space consumed release evidence"),
    };
    let repeated_refused = repeated_failure.reason == ReclaimError::Reclaimed;
    let Some(released) = repeated_failure.released else {
        panic!("a repeated reclaim refusal lost release evidence");
    };
    let reclaimed = foreign_refused
        && foreign_reclaimed
        && repeated_refused
        && space.reclaim(Some(released)).is_ok_and(|tables| tables > 0)
        && space.is_reclaimed();
    drop(foreign);
    reclaimed
}

pub(super) fn one_space_has_one_binding() -> bool {
    let free_before = mm::free_frames();
    let slots_before = thread::free_slots();
    let active_before = paging::active_root();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let root = space.root();
    let Ok(first) = space.bind() else {
        assert!(
            space.reclaim(None).is_ok() && mm::free_frames() == free_before,
            "a refused first binding did not return its address-space tables"
        );
        return false;
    };

    let (refused, duplicate) = match space.bind() {
        Err(ActivateError::AlreadyActive) => (true, None),
        Err(_) => (false, None),
        Ok(binding) => (false, Some(binding)),
    };
    let unchanged = first.roots().full_root() == root
        && space.root() == root
        && paging::active_root() == active_before
        && thread::free_slots() == slots_before;

    if let Some(binding) = duplicate {
        let _duplicate_release = task::release_uninstalled(binding);
    }
    let released = task::release_uninstalled(first);
    let reclaimed = space.reclaim(Some(released)).is_ok_and(|tables| tables > 0)
        && space.is_reclaimed()
        && mm::free_frames() == free_before;

    refused && unchanged && reclaimed
}

pub(super) fn binding_ignores_only_accessed_root_drift() -> bool {
    let free_before = mm::free_frames();
    let Ok(mut accessed) = AddressSpace::for_user() else {
        return false;
    };
    let (accepts_accessed, accessed_reclaimed) = match accessed.bind_with_accessed_drift_for_test()
    {
        Some(Ok(binding)) => {
            let released = task::release_uninstalled(binding);
            (
                true,
                accessed
                    .reclaim(Some(released))
                    .is_ok_and(|tables| tables > 0),
            )
        }
        Some(Err(_)) | None => (false, accessed.reclaim(None).is_ok_and(|tables| tables > 0)),
    };

    let Ok(mut cache) = AddressSpace::for_user() else {
        return false;
    };
    let (rejects_cache, cache_reclaimed) = match cache.bind_with_cache_drift_for_test() {
        Some(Err(ActivateError::KernelHalfDiffers(_))) => match cache.bind() {
            Ok(binding) => {
                let released = task::release_uninstalled(binding);
                (
                    true,
                    cache.reclaim(Some(released)).is_ok_and(|tables| tables > 0),
                )
            }
            Err(_) => (false, cache.reclaim(None).is_ok_and(|tables| tables > 0)),
        },
        Some(Ok(binding)) => {
            let released = task::release_uninstalled(binding);
            (
                false,
                cache.reclaim(Some(released)).is_ok_and(|tables| tables > 0),
            )
        }
        Some(Err(_)) | None => (false, cache.reclaim(None).is_ok_and(|tables| tables > 0)),
    };

    accepts_accessed
        && accessed_reclaimed
        && rejects_cache
        && cache_reclaimed
        && mm::free_frames() == free_before
}

pub(super) fn the_boot_thread_stands_on_a_guarded_stack() -> bool {
    let Some(layout) = thread::boot_stack_layout() else {
        return false;
    };
    let Some(slot) = layout.slot(0) else {
        return false;
    };

    let pointer = cpu::stack_pointer();
    if pointer < slot.stack_base() || pointer >= slot.stack_top() {
        return false;
    }

    let mut guard = slot.guard_base();
    while guard < slot.stack_base() {
        if paging::translate(guard).is_some() {
            return false;
        }
        guard += PAGE_SIZE;
    }

    let mut page = slot.stack_base();
    while page < slot.stack_top() {
        match paging::translate(page) {
            Some((_, flags))
                if flags.contains(Flags::WRITABLE) && flags.contains(Flags::NO_EXECUTE) => {}
            _ => return false,
        }
        page += PAGE_SIZE;
    }
    pinned_guard_refuses_mapping(&slot)
}

pub(super) fn the_double_fault_stack_is_guarded() -> bool {
    let Some(layout) = thread::fault_stack_layout() else {
        return false;
    };
    let Some(slot) = layout.slot(0) else {
        return false;
    };

    if segment::double_fault_stack() != slot.stack_top() {
        return false;
    }

    let mut guard = slot.guard_base();
    while guard < slot.stack_base() {
        if paging::translate(guard).is_some() {
            return false;
        }
        guard += PAGE_SIZE;
    }

    let mut page = slot.stack_base();
    while page < slot.stack_top() {
        match paging::translate(page) {
            Some((_, flags))
                if flags.contains(Flags::WRITABLE) && flags.contains(Flags::NO_EXECUTE) => {}
            _ => return false,
        }
        page += PAGE_SIZE;
    }

    true
}

pub(super) fn the_double_fault_stack_cannot_be_unmapped() -> bool {
    let Some(layout) = thread::fault_stack_layout() else {
        return false;
    };
    let Some(slot) = layout.slot(0) else {
        return false;
    };
    let inactive_refused = in_reclaimed_space(AddressSpace::new, |inactive| {
        let inactive_root = inactive.root();
        matches!(
            inactive.map_pinned_stack(&slot),
            Err(paging::StackMapError::Mapping(MapError::Inactive(root))) if root == inactive_root
        )
    });
    let mut context = ThreadContext::require();

    inactive_refused
        && matches!(
            mem::unmap_fault_stack_for_test(&mut context),
            Some(Err(MapError::Pinned(address))) if address == slot.stack_base()
        )
        && pinned_guard_refuses_mapping(&slot)
        && the_double_fault_stack_is_guarded()
}

pub(super) fn pinned_guard_refuses_mapping(slot: &StackSlot) -> bool {
    let free_before = mm::free_frames();
    let guard_before = paging::translate(slot.guard_base());
    let stack_before = paging::translate(slot.stack_base());
    mem::pinned_stack_refuses_map_for_test(&mut context(), slot) == Some(true)
        && paging::translate(slot.guard_base()) == guard_before
        && paging::translate(slot.stack_base()) == stack_before
        && mm::free_frames() == free_before
}

fn lower_stack_leaf_substitution_is_restored(
    slot: &StackSlot,
    probe: impl FnOnce() -> Option<bool>,
) -> bool {
    let free_before = mm::free_frames();
    let lower = slot.stack_base();
    let Some(upper) = slot.stack_top().checked_sub(PAGE_SIZE) else {
        return false;
    };
    let guard_before = paging::translate(slot.guard_base());
    let lower_before = paging::translate(lower);
    let upper_before = paging::translate(upper);

    guard_before.is_none()
        && lower_before.is_some()
        && upper_before.is_some()
        && probe() == Some(true)
        && paging::translate(slot.guard_base()) == guard_before
        && paging::translate(lower) == lower_before
        && paging::translate(upper) == upper_before
        && mm::free_frames() == free_before
}

pub(super) fn the_boot_stack_rejects_a_lower_leaf_substitution() -> bool {
    let Some(slot) = thread::boot_stack_layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    let pointer = cpu::stack_pointer();
    (slot.stack_base()..slot.stack_top()).contains(&pointer)
        && lower_stack_leaf_substitution_is_restored(&slot, || {
            paging::permanent_stack_rejects_lower_leaf_substitution_for_test(slot.stack_base())
        })
        && (slot.stack_base()..slot.stack_top()).contains(&cpu::stack_pointer())
}

pub(super) fn the_double_fault_stack_rejects_a_lower_leaf_substitution() -> bool {
    let Some(slot) = thread::fault_stack_layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    let hardware_before = segment::double_fault_stack();
    hardware_before == slot.stack_top()
        && lower_stack_leaf_substitution_is_restored(&slot, || {
            paging::permanent_stack_rejects_lower_leaf_substitution_for_test(slot.stack_base())
        })
        && segment::double_fault_stack() == hardware_before
}

pub(super) fn a_noncurrent_task_stack_rejects_a_lower_leaf_substitution() -> bool {
    const FIRST_TASK: usize = task::BOOT + 1;

    let Some(slot) = thread::layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    task::current() == task::BOOT
        && lower_stack_leaf_substitution_is_restored(&slot, || {
            task::pinned_stack_rejects_lower_leaf_substitution_for_test(FIRST_TASK)
        })
        && task::current() == task::BOOT
}

pub(super) fn protected_stacks_refuse_overlapping_maps() -> bool {
    let Some(task_stack) = thread::layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    let Some(fault_stack) = thread::fault_stack_layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    let Some(boot_stack) = thread::boot_stack_layout().and_then(|layout| layout.slot(0)) else {
        return false;
    };
    let free_before = mm::free_frames();
    let all_refused = [task_stack, fault_stack, boot_stack].iter().all(|slot| {
        let guard_before = paging::translate(slot.guard_base());
        let lower_before = paging::translate(slot.stack_base());
        let upper_before = slot.stack_top().checked_sub(1).and_then(paging::translate);
        let refused = mem::pinned_stack_refuses_map_for_test(&mut context(), slot) == Some(true);
        refused
            && paging::translate(slot.guard_base()) == guard_before
            && paging::translate(slot.stack_base()) == lower_before
            && slot.stack_top().checked_sub(1).and_then(paging::translate) == upper_before
    });
    all_refused && mm::free_frames() == free_before
}

// The shared half is not reachable from ring 3: a top-level entry carrying `USER` would hand
// every program the kernel.
pub(super) fn a_second_space_holds_the_kernel_and_nothing_else() -> bool {
    in_reclaimed_space(AddressSpace::for_user, |space| {
        let text = boot::sections().text.start;
        let Some(stack_layout) = thread::layout() else {
            return false;
        };
        let Some(slot) = stack_layout.slot(0) else {
            return false;
        };
        let Some(direct_map) = mem::direct_map_base() else {
            return false;
        };
        let shared = [text, slot.stack_base(), direct_map].into_iter().all(|at| {
            match (paging::translate(at), space.lookup(at)) {
                (Some(here), Some(there)) => here == there,
                _ => false,
            }
        });

        let Some((probe, _)) = boundary_probe() else {
            return false;
        };
        let user_is_absent = paging::translate(probe).is_some() && space.lookup(probe).is_none();
        let kernel_is_not_user = space
            .lookup(text)
            .is_some_and(|(_, flags)| !flags.contains(Flags::USER));

        shared && user_is_absent && kernel_is_not_user
    })
}

// The user page is user-accessible all the way down; the kernel text is not, and neither are its
// parents. The direct map's writable, never-executable pair is what W^X depends on being
// reported truthfully.
pub(super) fn a_walk_reports_the_permission_the_cpu_grants() -> bool {
    let Some((user_at, _)) = boundary_probe() else {
        return false;
    };
    let Some((_, user_flags)) = paging::translate(user_at) else {
        return false;
    };
    let user_is_user = user_flags.contains(Flags::USER);

    let text = boot::sections().text.start;
    let Some((_, text_flags)) = paging::translate(text) else {
        return false;
    };
    let kernel_is_not_user = !text_flags.contains(Flags::USER);

    let Some(direct) = mm::direct_map().and_then(|map| map.virtual_address(LARGE_PAGE_SIZE)) else {
        return false;
    };
    let Some((_, direct_flags)) = paging::translate(direct) else {
        return false;
    };
    let direct_is_data =
        direct_flags.contains(Flags::WRITABLE) && direct_flags.contains(Flags::NO_EXECUTE);

    user_is_user && kernel_is_not_user && direct_is_data
}

// The blocker occupies the second page of the range, so the mapping gets one page in before it is
// refused.
pub(super) fn a_refused_mapping_leaves_no_trace() -> bool {
    let Some(base) = pages_above_the_boot_stack(8) else {
        return false;
    };
    let blocker = base + PAGE_SIZE;
    let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let free_before = mm::free_frames();

    let Some(taken) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let Some(Ok(blocker_mapping)) = mem::map_owned_for_test(&mut context(), blocker, taken, wanted)
    else {
        return false;
    };

    let free_with_blocker = mm::free_frames();
    let Some(run) = mm::allocate_contiguous(2, 1) else {
        let cleaned = mem::unmap_for_test(&mut context(), blocker_mapping)
            .is_some_and(|result| result.map(drop).is_ok());
        assert!(
            cleaned && paging::translate(blocker).is_none() && mm::free_frames() == free_before,
            "a refused mapping setup did not return its blocker"
        );
        return false;
    };
    let refused = mem::map_owned_for_test(&mut context(), base, run, wanted);

    let rejected = matches!(refused, Some(Err(MapError::Conflict(address))) if address == blocker);
    let no_trace = paging::translate(base).is_none();
    let returned = mm::free_frames() == free_with_blocker;
    let blocker_intact = paging::translate(blocker).is_some();
    let cleaned = mem::unmap_for_test(&mut context(), blocker_mapping)
        .is_some_and(|result| result.map(drop).is_ok())
        && paging::translate(blocker).is_none()
        && mm::free_frames() == free_before;

    rejected && no_trace && returned && blocker_intact && cleaned
}

pub(super) fn a_partial_mapping_failure_returns_its_tables() -> bool {
    const BASE: u64 = 0x0000_3700_0000_0000;
    let flags = Flags::WRITABLE.union(Flags::NO_EXECUTE);

    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let allocator_with_root = AllocatorSnapshot::capture();
    let Some(run) = mm::allocate_contiguous(2, 1) else {
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a refused mapping allocation did not return its root"
        );
        return false;
    };
    paging::refuse_mapping_after_for_test(1);
    let result = space.map_owned(BASE, run, flags);
    paging::restore_mapping_refusals_for_test();

    let refused = result == Err(MapError::OutOfMemory);
    let absent = space.lookup(BASE).is_none() && space.lookup(BASE + PAGE_SIZE).is_none();
    let transaction_returned = AllocatorSnapshot::capture()
        .accounted_physical_frames()
        .is_some_and(|accounted| {
            allocator_with_root.accounted_physical_frames() == Some(accounted)
        });
    let root_reclaimed = space.reclaim(None).is_ok_and(|tables| tables == 1);
    drop(space);
    let root_returned =
        root_reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before);
    refused && absent && transaction_returned && root_returned
}

pub(super) fn a_refused_remap_preserves_every_older_entry() -> bool {
    const BASE: u64 = 0x0000_3400_0000_0000;
    let Some(physical) = mm::first_memory_run(PAGE_SIZE * 3) else {
        return false;
    };
    let Some(first) = BootRegion::describe(physical, PAGE_SIZE) else {
        return false;
    };
    let Some(blocker) = BootRegion::describe(physical + 2 * PAGE_SIZE, PAGE_SIZE) else {
        return false;
    };
    let Some(requested) = BootRegion::describe(physical, 2 * PAGE_SIZE) else {
        return false;
    };
    in_reclaimed_space(AddressSpace::for_user, |space| {
        let flags = Flags::NO_EXECUTE;
        if space.map_boot(BASE, first, flags).is_err()
            || space.map_boot(BASE + PAGE_SIZE, blocker, flags).is_err()
        {
            return false;
        }

        let refused = matches!(
            space.map_boot(BASE, requested, flags),
            Err(MapError::Conflict(address)) if address == BASE + PAGE_SIZE
        );
        let expected = flags.union(Flags::PRESENT);
        let first_intact = space.lookup(BASE) == Some((physical, expected));
        let blocker_intact =
            space.lookup(BASE + PAGE_SIZE) == Some((physical + 2 * PAGE_SIZE, expected));
        refused && first_intact && blocker_intact
    })
}

pub(super) fn a_shared_mapping_keeps_its_frame() -> bool {
    const PROBE: u64 = 0x0000_3500_0000_0000;
    in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some(run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        let physical = run.address();
        let mut shared = mm::USegment::from(run).share().into_bytes();
        if !shared.write_volatile_at(0, &[0xC7]) {
            return false;
        }
        let flags = Flags::NO_EXECUTE;
        if space.map_shared(PROBE, &shared, flags).is_err() {
            return false;
        }
        let held = mm::free_frames();

        drop(shared);
        let withheld = mm::free_frames() == held;
        let expected = flags.union(Flags::PRESENT).union(Flags::USER);
        let mapping_intact = space.lookup(PROBE) == Some((physical, expected));

        withheld && mapping_intact
    })
}

fn shared_bytes_reject_cache_alias(virtual_base: u64, flags: Flags) -> bool {
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let checked = match mm::allocate_contiguous(1, 1) {
        Some(mut run) => {
            if run.zero() {
                let shared = mm::USegment::from(run).share().into_bytes();
                let held = mm::free_frames();
                let refused = space.map_shared(virtual_base, &shared, flags)
                    == Err(MapError::CacheAlias(virtual_base))
                    && space.lookup(virtual_base).is_none();
                drop(shared);
                let owner_released = held.checked_add(1) == Some(mm::free_frames());
                refused && owner_released
            } else {
                false
            }
        }
        None => false,
    };
    let tables_unchanged = space.reclaim(None).is_ok_and(|tables| tables == 1);
    checked && tables_unchanged
}

fn shared_ring_reject_cache_alias(virtual_base: u64, flags: Flags) -> bool {
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let checked = match mm::allocate_contiguous(1, 1) {
        Some(mut run) => {
            if run.zero() {
                match SharedLayout::<1>::new(PAGE_SIZE as usize, [0]) {
                    Ok(layout) => match mm::USegment::from(run).share().into_ring(layout) {
                        Ok(shared) => {
                            let held = mm::free_frames();
                            let refused = space.map_shared_ring(virtual_base, &shared, flags)
                                == Err(MapError::CacheAlias(virtual_base))
                                && space.lookup(virtual_base).is_none();
                            drop(shared);
                            let owner_released = held.checked_add(1) == Some(mm::free_frames());
                            refused && owner_released
                        }
                        Err(_) => false,
                    },
                    Err(_) => false,
                }
            } else {
                false
            }
        }
        None => false,
    };
    let tables_unchanged = space.reclaim(None).is_ok_and(|tables| tables == 1);
    checked && tables_unchanged
}

pub(super) fn shared_mappings_reject_cache_aliases() -> bool {
    const BYTES: u64 = 0x0000_3510_0000_0000;
    const RING: u64 = 0x0000_3520_0000_0000;
    let mut all_refused = true;
    for cache in [
        Flags::NO_CACHE,
        Flags::WRITE_THROUGH,
        Flags::NO_CACHE.union(Flags::WRITE_THROUGH),
    ] {
        let flags = Flags::NO_EXECUTE.union(cache);
        all_refused &= shared_bytes_reject_cache_alias(BYTES, flags);
        all_refused &= shared_ring_reject_cache_alias(RING, flags);
    }
    all_refused
}

pub(super) fn ram_mappings_keep_the_direct_map_cache_type() -> bool {
    const OWNED: u64 = 0x0000_3530_0000_0000;
    const USER: u64 = 0x0000_3540_0000_0000;
    const BOOT: u64 = 0x0000_3550_0000_0000;

    let mut refused = true;
    for cache in [
        Flags::NO_CACHE,
        Flags::WRITE_THROUGH,
        Flags::NO_CACHE.union(Flags::WRITE_THROUGH),
    ] {
        let flags = Flags::NO_EXECUTE.union(cache);
        refused &= in_reclaimed_space(AddressSpace::for_user, |space| {
            let Some(owned) = mm::allocate_contiguous(1, 1) else {
                return false;
            };
            let owned_refused =
                space.map_owned(OWNED, owned, flags) == Err(MapError::CacheAlias(OWNED));
            let Some(user) = mm::allocate_contiguous(1, 1) else {
                return false;
            };
            let address = user.address();
            let user_refused = match space.map_user(USER, user.into(), flags) {
                Ok(()) => false,
                Err(error) => {
                    let reason = error.reason();
                    let run = error.into_run();
                    let returned = run.address() == address;
                    drop(run);
                    reason == MapError::CacheAlias(USER) && returned
                }
            };
            let Some(region) = probe_page(0) else {
                return false;
            };
            let boot_refused =
                space.map_boot(BOOT, region, flags) == Err(MapError::CacheAlias(BOOT));
            owned_refused
                && user_refused
                && boot_refused
                && [OWNED, USER, BOOT]
                    .into_iter()
                    .all(|at| space.lookup(at).is_none())
        });
    }

    // Each boot range maps with exactly its own direct-map type: the framebuffer is refused
    // without write-through and accepted with it.
    let framebuffer = mm::memory_map_for_test().and_then(|map| {
        map.page_runs()
            .find(|run| run.cache() == wost::boot::DirectMapCache::Framebuffer)
            .map(|run| run.base())
    });
    let exact = in_reclaimed_space(AddressSpace::for_user, |space| {
        let (Some(ordinary), Some(base)) = (probe_page(0), framebuffer) else {
            return false;
        };
        let (Some(bare_screen), Some(screen)) = (
            BootRegion::describe(base, PAGE_SIZE),
            BootRegion::describe(base, PAGE_SIZE),
        ) else {
            return false;
        };
        let ordinary_mapped = space.map_boot(BOOT, ordinary, Flags::NO_EXECUTE).is_ok();
        let screen_refused = space.map_boot(OWNED, bare_screen, Flags::NO_EXECUTE)
            == Err(MapError::CacheAlias(OWNED));
        let screen_mapped = space
            .map_boot(USER, screen, Flags::NO_EXECUTE.union(Flags::WRITE_THROUGH))
            .is_ok();
        ordinary_mapped && screen_refused && screen_mapped
    });
    refused && exact
}

pub(super) fn shared_topology_rejects_hostile_root_entries() -> bool {
    let huge = in_reclaimed_space(AddressSpace::new, |space| {
        space.rejects_huge_shared_topology_for_test()
    });
    let user = in_reclaimed_space(AddressSpace::new, |space| {
        space.rejects_user_shared_topology_for_test()
    });
    let supervisor = in_reclaimed_space(AddressSpace::new, |space| {
        space.reserves_supervisor_shared_topology_for_test()
    });
    huge && user && supervisor
}

pub(super) fn a_large_space_returns_every_table_frame() -> bool {
    const BASE: u64 = 0x0000_1000_0000_0000;
    const TABLES: u64 = 34;

    let Some(physical) = mm::first_memory_run(PAGE_SIZE) else {
        return false;
    };
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    for index in 0..TABLES {
        let Some(region) = BootRegion::describe(physical, PAGE_SIZE) else {
            let reclaimed = space.reclaim(None).is_ok();
            drop(space);
            assert!(
                reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
                "a refused large-space description did not return its tables"
            );
            return false;
        };
        let Some(at) = index
            .checked_mul(LARGE_PAGE_SIZE)
            .and_then(|offset| BASE.checked_add(offset))
        else {
            let reclaimed = space.reclaim(None).is_ok();
            drop(space);
            assert!(
                reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
                "a refused large-space address did not return its tables"
            );
            return false;
        };
        if space.map_boot(at, region, Flags::NO_EXECUTE).is_err() {
            let reclaimed = space.reclaim(None).is_ok();
            drop(space);
            assert!(
                reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
                "a refused large-space map did not return its tables"
            );
            return false;
        }
    }

    let allocator_while_mapped = AllocatorSnapshot::capture();
    let Ok(returned) = space.reclaim(None) else {
        return false;
    };
    drop(space);
    let table_frames_held = allocator_before
        .accounted_physical_frames()
        .zip(allocator_while_mapped.accounted_physical_frames())
        .and_then(|(before, mapped)| before.checked_sub(mapped))
        == Some(returned);

    returned > 32
        && table_frames_held
        && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

// The entries go when `unmap` returns, but its frames are withheld from the allocator until the
// token converts.
pub(super) fn an_unmapped_range_returns_its_frames() -> bool {
    /// More than one page, so the walk covers a run of slots.
    const PAGES: u64 = 4;
    let Some(probe) = pages_above_the_boot_stack(16) else {
        return false;
    };

    let free_before = mm::free_frames();
    let Some(run) = mm::allocate_contiguous(PAGES as usize, 1) else {
        return false;
    };
    let physical = run.address();

    let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let Some(Ok(mapping)) = mem::map_owned_for_test(&mut context(), probe, run, wanted) else {
        return false;
    };
    let mapped_here = paging::translate(probe).map(|(frame, _)| frame) == Some(physical);

    let free_while_mapped = mm::free_frames();

    let Some(Ok(token)) = mem::unmap_for_test(&mut context(), mapping) else {
        return false;
    };

    let gone = paging::translate(probe).is_none();
    let withheld = mm::free_frames() == free_while_mapped && token.pages() == PAGES;

    let returned = token.into_kernel().is_some_and(|segment| {
        let same = segment.address() == physical && segment.frames() == PAGES as usize;
        drop(segment);
        same
    });
    let counted = mm::free_frames() == free_before
        && free_before.checked_sub(free_while_mapped) == Some(PAGES as usize);

    mapped_here && gone && withheld && returned && counted
}

pub(super) fn unmapping_refuses_what_it_does_not_own() -> bool {
    mem::invalid_unmaps_are_refused(&mut context()).unwrap_or(false)
}

fn probe_page(index: u64) -> Option<BootRegion> {
    let base = mm::first_memory_run(PROBE_PAGES * PAGE_SIZE)?;
    BootRegion::describe(base.checked_add(index * PAGE_SIZE)?, PAGE_SIZE)
}

/// How many pages [`probe_page`] hands out, and the run it asks for.
const PROBE_PAGES: u64 = 2;

pub(super) fn the_map_offers_a_run_to_probe() -> bool {
    probe_page(0).is_some() && probe_page(PROBE_PAGES - 1).is_some()
}

pub(super) fn a_differing_remap_is_refused() -> bool {
    /// Canonical, and far from anything a fresh space would map.
    const PROBE: u64 = 0xffff_8800_0000_0000;

    in_reclaimed_space(AddressSpace::new, |space| {
        let (Some(here), Some(again_here), Some(elsewhere)) =
            (probe_page(0), probe_page(0), probe_page(1))
        else {
            return false;
        };
        let first = space.map_boot(PROBE, here, Flags::NO_EXECUTE);
        let again = space.map_boot(PROBE, again_here, Flags::NO_EXECUTE);
        let differing = space.map_boot(PROBE, elsewhere, Flags::NO_EXECUTE);
        first.is_ok() && again.is_ok() && differing == Err(MapError::Conflict(PROBE))
    })
}

// With the bit stripped, `unmap` sees a mapping nothing owns and refuses it, so no second owner of
// a boot frame is minted.
pub(super) fn a_boot_mapping_cannot_claim_ownership() -> bool {
    /// Canonical, and far from anything a fresh space maps.
    const PROBE: u64 = 0xffff_8880_0000_0000;

    in_reclaimed_space(AddressSpace::new, |space| {
        let Some(region) = probe_page(0) else {
            return false;
        };
        if space
            .map_boot(PROBE, region, Flags::NO_EXECUTE.union(Flags::OWNED))
            .is_err()
        {
            return false;
        }
        let provenance_removed = space
            .lookup(PROBE)
            .is_some_and(|(_, flags)| !flags.contains(Flags::OWNED));
        let before = mm::free_frames();
        let refused = space.unmap(PROBE, PAGE_SIZE).err() == Some(MapError::NotOwned(PROBE));
        provenance_removed && refused && mm::free_frames() == before
    })
}

pub(super) fn writable_executable_mappings_leave_no_trace() -> bool {
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let free_with_root = mm::free_frames();
    let checked = private_write_execute_maps_are_refused(&mut space, free_with_root)
        && shared_write_execute_maps_are_refused(&mut space, free_with_root);
    let root_only = space.reclaim(None).is_ok_and(|tables| tables == 1);
    drop(space);

    checked && root_only && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

fn private_write_execute_maps_are_refused(space: &mut AddressSpace, free_with_root: usize) -> bool {
    /// Canonical user memory, far from every mapping a fresh space inherits.
    const BOOT: u64 = 0x0000_0500_0000_0000;
    const OWNED: u64 = 0x0000_0600_0000_0000;
    const USER: u64 = 0x0000_0700_0000_0000;

    let hostile = Flags::WRITABLE;
    let Some(region) = probe_page(0) else {
        return false;
    };
    if space.map_boot(BOOT, region, hostile) != Err(MapError::WriteExecute(BOOT))
        || space.lookup(BOOT).is_some()
        || mm::free_frames() != free_with_root
    {
        return false;
    }

    let Some(owned) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if space.map_owned(OWNED, owned, hostile) != Err(MapError::WriteExecute(OWNED))
        || space.lookup(OWNED).is_some()
        || mm::free_frames() != free_with_root
    {
        return false;
    }

    let Some(mut user) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !user.zero() {
        return false;
    }
    let user_result = space.map_user(USER, user.into(), hostile);
    let user_refused = matches!(
        &user_result,
        Err(error) if error.reason() == MapError::WriteExecute(USER)
    ) && space.lookup(USER).is_none();
    drop(user_result);
    user_refused && mm::free_frames() == free_with_root
}

fn shared_write_execute_maps_are_refused(space: &mut AddressSpace, free_with_root: usize) -> bool {
    /// Canonical user memory, far from every mapping a fresh space inherits.
    const SHARED: u64 = 0x0000_0800_0000_0000;
    const RING: u64 = 0x0000_0900_0000_0000;

    let hostile = Flags::WRITABLE;
    let Some(mut shared) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !shared.zero() {
        return false;
    }
    let shared = mm::USegment::from(shared).share().into_bytes();
    let shared_refused = space.map_shared(SHARED, &shared, hostile)
        == Err(MapError::WriteExecute(SHARED))
        && space.lookup(SHARED).is_none();
    drop(shared);
    if !shared_refused || mm::free_frames() != free_with_root {
        return false;
    }

    let Some(mut ring) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    if !ring.zero() {
        return false;
    }
    let Ok(layout) = SharedLayout::<1>::new(PAGE_SIZE as usize, [0]) else {
        return false;
    };
    let Ok(ring) = mm::USegment::from(ring).share().into_ring(layout) else {
        return false;
    };
    let ring_refused = space.map_shared_ring(RING, &ring, hostile)
        == Err(MapError::WriteExecute(RING))
        && space.lookup(RING).is_none();
    drop(ring);

    ring_refused && mm::free_frames() == free_with_root
}

pub(super) fn caller_global_is_removed_from_safe_mappings() -> bool {
    const USER: u64 = 0x0000_0100_0000_0000;
    const SHARED: u64 = 0x0000_0200_0000_0000;
    const OWNED: u64 = 0x0000_0300_0000_0000;
    const BOOT: u64 = 0x0000_0400_0000_0000;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let hostile = Flags::GLOBAL
            .union(Flags::WRITABLE)
            .union(Flags::NO_EXECUTE);

        let Some(mut user) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !user.zero() || space.map_user(USER, user.into(), hostile).is_err() {
            return false;
        }

        let Some(mut shared) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !shared.zero() {
            return false;
        }
        let shared = mm::USegment::from(shared).share().into_bytes();
        if space.map_shared(SHARED, &shared, hostile).is_err() {
            return false;
        }

        let Some(mut owned) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !owned.zero() || space.map_owned(OWNED, owned, hostile).is_err() {
            return false;
        }

        let Some(region) = probe_page(0) else {
            return false;
        };
        if space.map_boot(BOOT, region, hostile).is_err() {
            return false;
        }

        let stripped = [USER, SHARED, OWNED, BOOT].into_iter().all(|at| {
            space
                .lookup(at)
                .is_some_and(|(_, flags)| !flags.contains(Flags::GLOBAL))
        });
        let authorities_hold =
            space.lookup(USER).is_some_and(|(_, flags)| {
                flags.contains(Flags::USER) && flags.contains(Flags::OWNED)
            }) && space.lookup(SHARED).is_some_and(|(_, flags)| {
                flags.contains(Flags::USER) && !flags.contains(Flags::OWNED)
            }) && space.lookup(OWNED).is_some_and(|(_, flags)| {
                !flags.contains(Flags::USER) && flags.contains(Flags::OWNED)
            }) && space.lookup(BOOT).is_some_and(|(_, flags)| {
                !flags.contains(Flags::USER) && !flags.contains(Flags::OWNED)
            });

        let user_removed = space.unmap(USER, PAGE_SIZE).map(drop).is_ok();
        let owned_removed = space.unmap(OWNED, PAGE_SIZE).map(drop).is_ok();
        stripped && authorities_hold && user_removed && owned_removed
    })
}

pub(super) fn a_boot_mapping_cannot_publish_kernel_memory() -> bool {
    const PROBE: u64 = 0x0000_0000_5000_0000;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some(region) = probe_page(0) else {
            return false;
        };
        space.map_boot(PROBE, region, Flags::USER.union(Flags::NO_EXECUTE))
            == Err(MapError::NotYours(PROBE))
            && space.lookup(PROBE).is_none()
    })
}

pub(super) fn a_paired_root_admits_only_user_leaves_below_the_split() -> bool {
    const BOOT: u64 = 0x0000_0060_0000_0000;
    const OWNED: u64 = 0x0000_0070_0000_0000;

    in_reclaimed_space(AddressSpace::for_user_isolated, |space| {
        let Some(region) = probe_page(0) else {
            return false;
        };
        let boot_refused =
            space.map_boot(BOOT, region, Flags::NO_EXECUTE) == Err(MapError::NotYours(BOOT));

        let Some(run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        let owned_refused =
            space.map_owned(OWNED, run, Flags::NO_EXECUTE) == Err(MapError::NotYours(OWNED));

        boot_refused
            && owned_refused
            && space.lookup(BOOT).is_none()
            && space.lookup(OWNED).is_none()
    })
}

pub(super) fn the_user_root_contains_only_the_entry_enclave() -> bool {
    in_reclaimed_space(AddressSpace::for_user_isolated, |space| {
        space.user_root_contains_only_entry_enclave_for_test()
    })
}

pub(super) fn paired_space_refusal_and_retirement_conserve_every_frame() -> bool {
    const PROBE: u64 = 0x0000_0050_0000_0000;

    let allocator_before = AllocatorSnapshot::capture();
    let checked = in_reclaimed_space(AddressSpace::for_user_isolated, |space| {
        let pair_before = AllocatorSnapshot::capture();
        let Some(mut refused_run) = mm::allocate_contiguous(2, 1) else {
            return false;
        };
        if !refused_run.zero() {
            return false;
        }
        paging::refuse_mapping_after_for_test(1);
        let refusal = space.map_user(
            PROBE,
            refused_run.into(),
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        );
        paging::restore_mapping_refusals_for_test();
        let refused = match refusal {
            Err(error) => {
                let exact = error.reason() == MapError::OutOfMemory;
                drop(error.into_run());
                exact
            }
            Ok(()) => {
                let cleaned = space.unmap(PROBE, 2 * PAGE_SIZE).map(drop).is_ok();
                assert!(cleaned, "unexpected paired mapping did not clean up");
                false
            }
        };
        let refusal_was_atomic = refused
            && space.lookup(PROBE).is_none()
            && space.lookup(PROBE + PAGE_SIZE).is_none()
            && AllocatorSnapshot::capture().has_same_ownership(&pair_before);
        if !refusal_was_atomic {
            return false;
        }

        let Some(mut run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !run.zero() {
            return false;
        }
        if let Err(error) =
            space.map_user(PROBE, run.into(), Flags::WRITABLE.union(Flags::NO_EXECUTE))
        {
            drop(error.into_run());
            return false;
        }

        let retained = matches!(
            space.reclaim(None),
            Err(paging::ReclaimFailure {
                reason: ReclaimError::Mapped(1),
                ..
            })
        ) && !space.is_reclaimed();
        let mapping_returned = space.unmap(PROBE, PAGE_SIZE).map(drop).is_ok();
        retained && mapping_returned
    });

    checked && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

// The direct map ensures the hostile walk reaches real entries to blank.
// The canonical-hole check precedes half ownership, so the straddle must report that distinct
// refusal. The last mappable user page reaches the mapping lookup rather than the half refusal.
pub(super) fn a_user_space_cannot_reach_the_kernel_half() -> bool {
    const KERNEL: u64 = 0xffff_8000_0010_0000;
    const HOLE_EDGE: u64 = x86_paging::USER_MAPPING_END;
    const LAST_USER: u64 = x86_paging::USER_MAPPING_END - PAGE_SIZE;

    in_reclaimed_space(AddressSpace::for_user, |user| {
        let Some(region) = probe_page(0) else {
            return false;
        };
        let unmapped = user.unmap(KERNEL, PAGE_SIZE);
        let mapped = user.map_boot(KERNEL, region, Flags::NO_EXECUTE);
        let straddles =
            user.unmap(HOLE_EDGE, 2 * PAGE_SIZE).err() == Some(MapError::NonCanonical(HOLE_EDGE));
        let reaches_its_own =
            user.unmap(LAST_USER, PAGE_SIZE).err() != Some(MapError::NotYours(LAST_USER));

        unmapped.err() == Some(MapError::NotYours(KERNEL))
            && mapped == Err(MapError::NotYours(KERNEL))
            && straddles
            && reaches_its_own
    })
}

// Code ending on this page would trap with a non-canonical return address, which `iretq` refuses
// in ring 0.
pub(super) fn the_last_canonical_page_is_never_user_memory() -> bool {
    const TOP: u64 = x86_paging::USER_MAPPING_END;
    const BELOW: u64 = TOP - PAGE_SIZE;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some(run) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        let address = run.address();
        let refused = match space.map_user(TOP, run.into(), Flags::NONE) {
            Ok(()) => false,
            Err(error) => {
                let reason = error.reason();
                let run = error.into_run();
                let returned = run.address() == address;
                drop(run);
                reason == MapError::NotYours(TOP) && returned
            }
        };
        let Some(below) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        let admitted = space.map_user(BELOW, below.into(), Flags::NONE).is_ok()
            && space.lookup(BELOW).is_some();
        let top_absent = space.lookup(TOP).is_none();
        let released = space
            .unmap(BELOW, PAGE_SIZE)
            .ok()
            .and_then(|unmapped| unmapped.into_user())
            .and_then(|user| user.reclaim())
            .is_some();
        refused && admitted && top_absent && released
    })
}

pub(super) fn a_user_mapping_cannot_widen_a_supervisor_table() -> bool {
    /// Two addresses one page apart, so they share every table above the leaf.
    const SUPERVISOR: u64 = 0x0000_0000_4000_0000;
    const USER: u64 = SUPERVISOR + PAGE_SIZE;

    in_reclaimed_space(AddressSpace::new, |space| {
        let Some(first) = probe_page(0) else {
            return false;
        };
        if space
            .map_boot(SUPERVISOR, first, Flags::NO_EXECUTE)
            .is_err()
        {
            return false;
        }
        let Some(mut second) = mm::allocate_contiguous(1, 1) else {
            return false;
        };
        if !second.zero() {
            return false;
        }
        space
            .map_user(USER, second.into(), Flags::NO_EXECUTE)
            .err()
            .is_some_and(|error| error.reason() == MapError::Conflict(USER))
    })
}

// Unaligned and empty ranges are refused before the map is consulted, and a range that runs off
// the end of physical space cannot overflow its way into looking described.
pub(super) fn an_undescribed_range_cannot_be_mapped() -> bool {
    /// Far above anything a PC memory map describes, and above `mm::CEILING`.
    const NOWHERE: u64 = 0x0000_7000_0000_0000;

    BootRegion::describe(NOWHERE, PAGE_SIZE).is_none()
        && BootRegion::describe(PAGE_SIZE + 1, PAGE_SIZE).is_none()
        && BootRegion::describe(PAGE_SIZE, 0).is_none()
        && BootRegion::describe(u64::MAX - PAGE_SIZE + 1, PAGE_SIZE).is_none()
}

pub(super) fn device_mappings_are_physically_exclusive() -> bool {
    const PROBE: u64 = 0xfee0_0000;
    let mut context = context();
    let Some(before) = mem::device_mappings(&mut context) else {
        return false;
    };
    let Some(Ok(first)) = mem::map_device(&mut context, PROBE + 4, 4) else {
        return false;
    };
    let Ok(first) = first.claim_releasable() else {
        return false;
    };
    let overlap = matches!(
        mem::map_device(&mut context, PROBE + 8, 4),
        Some(Err(MapError::DeviceOverlap(found))) if found == PROBE + 8
    );
    let removed = matches!(
        mem::unmap_device_exact(&mut context, first),
        mem::DeviceRetirement::Retired
    );
    overlap && removed && mem::device_mappings(&mut context) == Some(before)
}

pub(super) fn device_mappings_refuse_allocatable_ram() -> bool {
    in_reclaimed_space(AddressSpace::new, |space| {
        let Some(frame) = mm::allocate() else {
            return false;
        };
        let address = frame.address();
        matches!(
            paging::admit_device_range(address, 4).and_then(|range| space.map_device(range)),
            Err(MapError::NotDeviceMemory(found)) if found == address
        )
    })
}

pub(super) fn malformed_device_extents_are_refused() -> bool {
    in_reclaimed_space(AddressSpace::new, |space| {
        matches!(
            paging::admit_device_range(0xfee0_0000, 0).and_then(|range| space.map_device(range)),
            Err(MapError::InvalidDeviceRange(0xfee0_0000))
        ) && matches!(
            paging::admit_device_range(4, u64::MAX).and_then(|range| space.map_device(range)),
            Err(MapError::InvalidDeviceRange(4))
        )
    })
}

pub(super) fn device_window_reuses_arbitrary_order_gaps() -> bool {
    let Some(map) = mm::memory_map_for_test() else {
        return false;
    };
    let mut context = context();
    let mut attempted = 0usize;
    for region in map.regions().filter(|region| {
        matches!(
            region.kind,
            boot::MemoryKind::Reserved | boot::MemoryKind::Unknown(_)
        )
    }) {
        let Some(mut physical) = region.base.checked_next_multiple_of(PAGE_SIZE) else {
            continue;
        };
        let region_end = region.base + region.len;
        while attempted < 256 {
            let Some(end) = physical.checked_add(PAGE_SIZE * 2) else {
                break;
            };
            if end > region_end {
                break;
            }
            attempted += 1;
            let exclusively_device = map.regions().all(|other| {
                let overlaps = physical < other.base + other.len && other.base < end;
                !overlaps
                    || matches!(
                        other.kind,
                        boot::MemoryKind::Reserved | boot::MemoryKind::Unknown(_)
                    )
            });
            if exclusively_device
                && let Some(result) = device_gaps_are_reused_at(&mut context, physical)
            {
                return result;
            }
            let Some(next) = physical.checked_add(PAGE_SIZE) else {
                break;
            };
            physical = next;
        }
    }
    false
}

fn device_gaps_are_reused_at(context: &mut ThreadContext, physical: u64) -> Option<bool> {
    let before = mem::device_mappings(context)?;
    let first = match mem::map_device(context, physical, PAGE_SIZE)? {
        Ok(mapping) => mapping,
        Err(MapError::DeviceOverlap(_) | MapError::NotDeviceMemory(_)) => return None,
        Err(_) => return Some(false),
    };
    let first_address = first.address();
    let Ok(first) = first.claim_releasable() else {
        return Some(false);
    };

    let second = match mem::map_device(context, physical + PAGE_SIZE, PAGE_SIZE)? {
        Ok(mapping) => mapping,
        Err(MapError::DeviceOverlap(_) | MapError::NotDeviceMemory(_)) => {
            if matches!(
                mem::unmap_device_exact(context, first),
                mem::DeviceRetirement::Retired
            ) {
                return None;
            }
            return Some(false);
        }
        Err(_) => return Some(false),
    };
    let Ok(second) = second.claim_releasable() else {
        return Some(false);
    };

    paging::refuse_device_unmap_for_test();
    let first_removed = match mem::unmap_device(context, first) {
        mem::DeviceUnmapOutcome::Refused(failure) => {
            let unchanged = mem::device_mappings(context) == Some(before + 2);
            unchanged
                && matches!(
                    mem::unmap_device_exact(context, failure.region),
                    mem::DeviceRetirement::Retired
                )
        }
        _ => false,
    };
    let second_removed = matches!(
        mem::unmap_device_exact(context, second),
        mem::DeviceRetirement::Retired
    );
    if !first_removed || !second_removed {
        return Some(false);
    }

    let third = match mem::map_device(context, physical, PAGE_SIZE)? {
        Ok(mapping) => mapping,
        Err(_) => return Some(false),
    };
    let reused = third.address() == first_address;
    let Ok(third) = third.claim_releasable() else {
        return Some(false);
    };
    let restored = matches!(
        mem::unmap_device_exact(context, third),
        mem::DeviceRetirement::Retired
    ) && mem::device_mappings(context) == Some(before);
    Some(reused && restored)
}

pub(super) struct DeviceLiveness {
    pub(super) shared_topology: bool,
    pub(super) stale_root: bool,
    pub(super) first: bool,
    pub(super) middle: bool,
    pub(super) last: bool,
    pub(super) cache_policy: bool,
}

fn map_three_page_device_extent(context: &mut ThreadContext) -> Option<paging::DeviceMapping> {
    const EXTENT: u64 = 3 * PAGE_SIZE;

    let map = mm::memory_map_for_test()?;
    let mut attempted = 0usize;
    for region in map.regions().filter(|region| {
        matches!(
            region.kind,
            boot::MemoryKind::Reserved | boot::MemoryKind::Unknown(_)
        )
    }) {
        let Some(mut physical) = region.base.checked_next_multiple_of(PAGE_SIZE) else {
            continue;
        };
        let Some(region_end) = region.base.checked_add(region.len) else {
            continue;
        };
        while attempted < 256 {
            let Some(end) = physical.checked_add(EXTENT) else {
                break;
            };
            if end > region_end {
                break;
            }
            attempted += 1;
            match mem::map_device(context, physical, EXTENT)? {
                Ok(mapping) => return Some(mapping),
                Err(MapError::DeviceOverlap(_) | MapError::NotDeviceMemory(_)) => {}
                Err(_) => return None,
            }
            let Some(next) = physical.checked_add(PAGE_SIZE) else {
                break;
            };
            physical = next;
        }
    }
    None
}

pub(super) fn device_liveness_covers_the_current_root_and_extent() -> Option<DeviceLiveness> {
    let mut context = context();
    let before = mem::device_mappings(&mut context)?;
    let mut stale = AddressSpace::for_user().ok()?;
    let Some(mapping) = map_three_page_device_extent(&mut context) else {
        if let Err(reason) = stale.reclaim(None) {
            panic!("device liveness fixture could not reclaim its unused root: {reason}");
        }
        return None;
    };

    let address = mapping.address();
    let live_before = paging::translate(address);
    let shared_upper = live_before.is_some() && stale.lookup(address) == live_before;
    let (mapping, first) = mapping.rejects_physical_substitution_for_test(0);
    let (mapping, middle) = mapping.rejects_physical_substitution_for_test(1);
    let (mapping, last) = mapping.rejects_physical_substitution_for_test(2);
    let (mapping, cache_policy) = mapping.rejects_cache_substitution_for_test(1);

    let (loaded, under_stale) = mapping.claim_releasable_under_for_test(&stale);
    let (stale_root, region) = match under_stale {
        Err(mapping) => {
            let Ok(region) = mapping.claim_releasable() else {
                panic!("a restored device mapping could not be reclaimed by its test");
            };
            (loaded, region)
        }
        Ok(region) => (false, region),
    };
    let unchanged = paging::translate(address) == live_before;
    let removed = matches!(
        mem::unmap_device_exact(&mut context, region),
        mem::DeviceRetirement::Retired
    );
    let reclaimed = stale.reclaim(None).is_ok();
    let cleaned =
        removed && reclaimed && mem::device_mappings(&mut context) == Some(before) && unchanged;

    Some(DeviceLiveness {
        shared_topology: shared_upper && cleaned,
        stale_root: stale_root && cleaned,
        first: first && cleaned,
        middle: middle && cleaned,
        last: last && cleaned,
        cache_policy: cache_policy && cleaned,
    })
}

pub(super) fn active_device_tables_cannot_be_reclaimed() -> bool {
    mem::active_space_reclaim_is_guarded(&mut context()) == Some(true)
}

pub(super) fn active_kernel_topology_is_frozen() -> bool {
    mem::active_kernel_topology_is_frozen(&mut context()) == Some(true)
}

// A page is mapped first, so the later `None` is about the reclaim.
// Never bound, so reclaim exercises the cold-space path.
// Refusals are counted because frame zero reads as zeros, so an unguarded `lookup` returns the
// same `None` the guard does.
pub(super) fn a_reclaimed_space_answers_nothing() -> bool {
    let allocator_before = AllocatorSnapshot::capture();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let zero_is_live = space.zero_root_is_live_for_test();

    let Some(mut page) = mm::allocate_contiguous(1, 1) else {
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a missing reclaimed-space page did not return its tables"
        );
        return false;
    };
    if !page.zero() {
        drop(page);
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a refused reclaimed-space zero did not return setup resources"
        );
        return false;
    }
    let at = user::STACK + 0xB0_0000;
    if space
        .map_user(at, page.into(), Flags::WRITABLE.union(Flags::NO_EXECUTE))
        .is_err()
    {
        let reclaimed = space.reclaim(None).is_ok();
        drop(space);
        assert!(
            reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before),
            "a refused reclaimed-space map did not return setup resources"
        );
        return false;
    }
    let answered_before = space.lookup(at).is_some();
    let unmapped = space.unmap(at, PAGE_SIZE).map(drop).is_ok();

    let reclaimed = space.reclaim(None).is_ok();
    let empty = space.is_reclaimed();

    let refusals = paging::reclaimed_refusals();

    let silent = space.lookup(at).is_none();
    let unbindable = matches!(space.bind(), Err(ActivateError::Reclaimed));
    let unactivatable = matches!(paging::activate(&mut space), Err(ActivateError::Reclaimed));
    let once_only = matches!(
        space.reclaim(None),
        Err(paging::ReclaimFailure {
            reason: ReclaimError::Reclaimed,
            ..
        })
    );

    let all_four_refused = paging::reclaimed_refusals() == refusals + 4;

    let checked = answered_before
        && zero_is_live
        && unmapped
        && reclaimed
        && empty
        && silent
        && unbindable
        && unactivatable
        && once_only
        && all_four_refused;
    drop(space);
    checked && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}
