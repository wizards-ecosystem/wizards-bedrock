use native_abi::address::STACK;
use wost::arch::current::paging::{self, ActivateError, AddressSpace, MapError, ReclaimError};
use wost::arch::current::task;
use wost::arch::current::usercopy::{self, UserCopyError};
use wost::boot;
use wost::mm;
use x86_paging::{Flags, PAGE_SIZE};

use super::processes::AllocatorSnapshot;
use super::runs_until;
use crate::thread::ThreadContext;
use crate::{mem, user};

// Samples are offsets from a section's base: adding two higher-half addresses overflows.
pub(super) fn the_image_obeys_write_xor_execute() -> bool {
    let sections = boot::sections();
    let enclave = sections.entry_enclave;
    let Some(direct) = mm::direct_map() else {
        return false;
    };

    let image = [
        (sections.text, false, true),
        (sections.rodata, false, false),
        (sections.data, true, false),
        (enclave.text, false, true),
        (enclave.rodata, false, false),
        (enclave.tables, true, false),
        (enclave.data, true, false),
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
                let Some(alias) = direct.virtual_address(physical.get()) else {
                    return false;
                };
                let Some((alias_physical, alias_flags)) = paging::translate(alias) else {
                    return false;
                };
                !flags.is_write_execute()
                    && flags.contains(Flags::WRITABLE) == writable
                    && !flags.contains(Flags::NO_EXECUTE) == executable
                    && alias_flags.contains(Flags::WRITABLE) == writable
                    && alias_flags.contains(Flags::NO_EXECUTE)
                    && alias_physical == physical
            })
    });

    let Some(frame) = mm::allocate() else {
        return false;
    };
    let direct_map = direct
        .virtual_address(frame.address().get())
        .and_then(paging::translate)
        .is_some_and(|(physical, flags)| {
            physical == frame.address()
                && flags.contains(Flags::WRITABLE)
                && flags.contains(Flags::NO_EXECUTE)
        });

    image && direct_map
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

pub(super) fn two_spaces_map_one_address_to_different_memory() -> bool {
    if !runs_until(2000, crate::reaper::is_idle) {
        return false;
    }
    let allocator_before = AllocatorSnapshot::capture();
    let (Ok(mut first), Ok(mut second)) = (AddressSpace::for_user(), AddressSpace::for_user())
    else {
        return false;
    };
    let checked = first.root() != second.root() && one_address_maps_apart(&mut first, &mut second);
    let reclaimed = first.reclaim(None).is_ok() && second.reclaim(None).is_ok();
    drop((first, second));
    checked && reclaimed && AllocatorSnapshot::capture().has_same_ownership(&allocator_before)
}

fn one_address_maps_apart(first: &mut AddressSpace, second: &mut AddressSpace) -> bool {
    const SHARED: u64 = user::CODE + 0x10_0000;

    let writable = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let (Some(one), Some(two)) = (user_page(), user_page()) else {
        return false;
    };
    let (one_at, two_at) = (one.address(), two.address());
    if first.map_user(SHARED, one, writable).is_err()
        || second.map_user(SHARED, two, writable).is_err()
    {
        return false;
    }

    let user_frame = |space: &AddressSpace, expected: mm::PhysicalAddress| {
        space
            .lookup(SHARED)
            .is_some_and(|(frame, flags)| frame == expected && flags.contains(Flags::USER))
    };
    let separate = one_at != two_at && user_frame(first, one_at) && user_frame(second, two_at);

    let text = boot::sections().text.start;
    let kernel = first.lookup(text);
    let shared_kernel = kernel.is_some_and(|(_, flags)| !flags.contains(Flags::USER))
        && second.lookup(text) == kernel;

    let unmapped = first.unmap(SHARED, PAGE_SIZE).map(drop).is_ok()
        && second.unmap(SHARED, PAGE_SIZE).map(drop).is_ok();
    separate && shared_kernel && unmapped
}

fn user_page() -> Option<mm::USegment> {
    mm::allocate_contiguous(1, 1).and_then(mm::USegment::zeroed)
}

fn marked_page(marker: u8) -> Option<(mm::USegment, mm::PhysicalAddress)> {
    let mut run = user_page()?;
    run.write_at(0, &[marker; 16]).then_some(())?;
    let address = run.address();
    Some((run, address))
}

pub(super) fn a_user_page_comes_back_cleared() -> bool {
    const AT: u64 = STACK + 0xA0_0000;
    const OUTSIDE_USER_HALF: u64 = 0xffff_8000_0010_0000;

    let unmapped = in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some((run, address)) = marked_page(0xde) else {
            return false;
        };
        if space
            .map_user(AT, run, Flags::WRITABLE.union(Flags::NO_EXECUTE))
            .is_err()
        {
            return false;
        }
        let Ok(taken) = space.unmap(AT, PAGE_SIZE) else {
            return false;
        };
        drop(taken);
        mm::is_freed_run_zero(address, 1)
    });

    let direct = marked_page(0xa1).is_some_and(|(run, address)| {
        drop(run);
        mm::is_freed_run_zero(address, 1)
    });

    let shared = marked_page(0xb2).is_some_and(|(run, address)| {
        drop(run.share());
        mm::is_freed_run_zero(address, 1)
    });

    let refused = in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some((run, address)) = marked_page(0xc3) else {
            return false;
        };
        let Err(error) = space.map_user(
            OUTSIDE_USER_HALF,
            run,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        ) else {
            return false;
        };
        let reason = error.reason();
        drop(error);
        reason == MapError::NotYours(OUTSIDE_USER_HALF) && mm::is_freed_run_zero(address, 1)
    });

    unmapped && direct && shared && refused
}

pub(super) fn an_active_unmap_exposes_only_its_replacement(context: &mut ThreadContext) -> bool {
    const AT: u64 = STACK + 0x30_0000;
    const FIRST: [u8; 16] = [0xa5; 16];
    const SECOND: [u8; 16] = [0x5a; 16];

    let Some(mut first) = user_page() else {
        return false;
    };
    if !first.write_at(0, &FIRST) {
        return false;
    }
    let first_physical = first.address();
    let first_flags = Flags::WRITABLE.union(Flags::NO_EXECUTE);
    let Some(Ok(first_mapping)) = mem::map_user_page_for_test(context, AT, first, first_flags)
    else {
        return false;
    };

    let mut warmed = [0; FIRST.len()];
    let warmed_first = usercopy::copy_from_user(AT, &mut warmed).is_ok() && warmed == FIRST;
    let Some(Ok(first_taken)) = mem::unmap_for_test(context, first_mapping) else {
        return false;
    };

    let Some(mut second) = user_page() else {
        drop(first_taken);
        return false;
    };
    if !second.write_at(0, &SECOND) {
        drop(first_taken);
        return false;
    }
    let second_physical = second.address();
    let Some(Ok(second_mapping)) =
        mem::map_user_page_for_test(context, AT, second, Flags::NO_EXECUTE)
    else {
        drop(first_taken);
        return false;
    };

    let replacement_is_distinct = first_physical != second_physical
        && paging::translate(AT).is_some_and(|(physical, flags)| {
            physical == second_physical
                && flags.contains(Flags::PRESENT)
                && flags.contains(Flags::USER)
                && flags.contains(Flags::OWNED)
                && !flags.contains(Flags::WRITABLE)
                && flags.contains(Flags::NO_EXECUTE)
        });
    let mut observed = [0; SECOND.len()];
    let replacement_was_read =
        usercopy::copy_from_user(AT, &mut observed).is_ok() && observed == SECOND;
    let replacement_is_read_only = matches!(
        usercopy::copy_to_user(AT, &FIRST),
        Err(UserCopyError::ReadOnly(found)) if found == AT
    );

    let second_removed =
        mem::unmap_for_test(context, second_mapping).is_some_and(|result| result.map(drop).is_ok());
    drop(first_taken);

    warmed_first
        && replacement_is_distinct
        && replacement_was_read
        && replacement_is_read_only
        && second_removed
}

pub(super) fn a_bound_space_is_not_reclaimed() -> bool {
    let free_before = mm::free_frames();
    let Ok(mut space) = AddressSpace::for_user() else {
        return false;
    };
    let root = space.root();
    let Ok(binding) = space.bind() else {
        let _cleanup = space.reclaim(None);
        return false;
    };

    let bound_once = match space.bind() {
        Err(ActivateError::AlreadyActive) => true,
        Err(_) => false,
        Ok(duplicate) => {
            let _duplicate_release = task::release_uninstalled(duplicate);
            false
        }
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

    let released = task::release_uninstalled(binding);
    let reclaimed =
        space.reclaim(Some(released)).is_ok_and(|tables| tables > 0) && space.is_reclaimed();
    let once_only = matches!(
        space.reclaim(None),
        Err(paging::ReclaimFailure {
            reason: ReclaimError::Reclaimed,
            ..
        })
    );
    drop(space);

    bound_once && refused && reclaimed && once_only && mm::free_frames() == free_before
}

// The blocker sits on the second page, so a refusal that kept the first page would show.
pub(super) fn a_refused_mapping_leaves_no_trace() -> bool {
    const BASE: u64 = STACK + 0xC0_0000;
    const BLOCKER: u64 = BASE + PAGE_SIZE;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let wanted = Flags::WRITABLE.union(Flags::NO_EXECUTE);
        let (Some(blocker), Some(run)) =
            (mm::allocate_contiguous(1, 1), mm::allocate_contiguous(2, 1))
        else {
            return false;
        };
        if space.map_owned(BLOCKER, blocker, wanted).is_err() {
            return false;
        }
        let free_with_blocker = mm::free_frames() + 2;

        let rejected = space.map_owned(BASE, run, wanted) == Err(MapError::Conflict(BLOCKER));
        let no_trace = space.lookup(BASE).is_none();
        let returned = mm::free_frames() == free_with_blocker;
        let blocker_intact = space.lookup(BLOCKER).is_some();
        let cleaned = space.unmap(BLOCKER, PAGE_SIZE).map(drop).is_ok();

        rejected && no_trace && returned && blocker_intact && cleaned
    })
}

pub(super) fn a_shared_mapping_keeps_its_frame() -> bool {
    const PROBE: u64 = 0x0000_3500_0000_0000;
    in_reclaimed_space(AddressSpace::for_user, |space| {
        let Some(run) = user_page() else {
            return false;
        };
        let physical = run.address();
        let mut shared = run.share().into_bytes();
        if !shared.write_at(0, &[0xC7]) {
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

// The loader refuses write-and-execute first; only the doors a program image reaches are tried.
pub(super) fn a_user_space_cannot_reach_the_kernel_half() -> bool {
    const KERNEL: u64 = 0xffff_8000_0010_0000;
    const TOP: u64 = x86_paging::USER_MAPPING_END;
    const LAST_USER: u64 = TOP - PAGE_SIZE;
    const OWNED: u64 = 0x0000_0600_0000_0000;
    const SUPERVISOR: u64 = 0x0000_0000_4000_0000;
    const BESIDE_SUPERVISOR: u64 = SUPERVISOR + PAGE_SIZE;
    const WRITE_EXECUTE_USER: u64 = 0x0000_0700_0000_0000;
    const WRITE_EXECUTE_SHARED: u64 = 0x0000_0800_0000_0000;

    in_reclaimed_space(AddressSpace::for_user, |space| {
        let page = || mm::allocate_contiguous(1, 1);
        let no_execute = Flags::NO_EXECUTE;

        let Some(kernel) = page() else {
            return false;
        };
        let kernel_half = space.unmap(KERNEL, PAGE_SIZE).err() == Some(MapError::NotYours(KERNEL))
            && space.map_owned(KERNEL, kernel, no_execute) == Err(MapError::NotYours(KERNEL))
            && space.unmap(TOP, 2 * PAGE_SIZE).err() == Some(MapError::NonCanonical(TOP));

        let (Some(top), Some(last)) = (user_page(), user_page()) else {
            return false;
        };
        let top_refused = space
            .map_user(TOP, top, Flags::NO_EXECUTE)
            .err()
            .is_some_and(|error| error.reason() == MapError::NotYours(TOP))
            && space.lookup(TOP).is_none();
        let last_admitted = space.map_user(LAST_USER, last, Flags::NO_EXECUTE).is_ok()
            && space.lookup(LAST_USER).is_some()
            && space.unmap(LAST_USER, PAGE_SIZE).map(drop).is_ok();

        let Some(published) = page() else {
            return false;
        };
        let kernel_bytes_refused = space.map_owned(
            OWNED,
            published,
            Flags::USER.union(Flags::WRITABLE).union(no_execute),
        ) == Err(MapError::NotYours(OWNED));

        let (Some(supervisor), Some(beside)) = (page(), user_page()) else {
            return false;
        };
        if space.map_owned(SUPERVISOR, supervisor, no_execute).is_err() {
            return false;
        }
        let table_not_widened = space
            .map_user(BESIDE_SUPERVISOR, beside, no_execute)
            .err()
            .is_some_and(|error| error.reason() == MapError::Conflict(BESIDE_SUPERVISOR));
        let supervisor_removed = space.unmap(SUPERVISOR, PAGE_SIZE).map(drop).is_ok();

        let (Some(executable), Some(shared)) = (user_page(), user_page()) else {
            return false;
        };
        let user_write_execute_refused = space
            .map_user(WRITE_EXECUTE_USER, executable, Flags::WRITABLE)
            .err()
            .is_some_and(|error| error.reason() == MapError::WriteExecute(WRITE_EXECUTE_USER));
        let shared = shared.share().into_bytes();
        let shared_write_execute_refused =
            space.map_shared(WRITE_EXECUTE_SHARED, &shared, Flags::WRITABLE)
                == Err(MapError::WriteExecute(WRITE_EXECUTE_SHARED));
        drop(shared);
        let write_execute_left_nothing = [WRITE_EXECUTE_USER, WRITE_EXECUTE_SHARED]
            .into_iter()
            .all(|at| space.lookup(at).is_none());

        kernel_half
            && top_refused
            && last_admitted
            && kernel_bytes_refused
            && table_not_widened
            && supervisor_removed
            && user_write_execute_refused
            && shared_write_execute_refused
            && write_execute_left_nothing
    })
}

pub(super) fn device_mappings_refuse_allocatable_ram() -> bool {
    in_reclaimed_space(AddressSpace::new, |space| {
        let Some(frame) = mm::allocate() else {
            return false;
        };
        let address = frame.address().get();
        matches!(
            paging::admit_device_range(address, 4).and_then(|range| space.map_device(range)),
            Err(MapError::NotDeviceMemory(found)) if found == address
        )
    })
}
