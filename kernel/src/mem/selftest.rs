use super::*;

/// Bytes in a page frame.
#[cfg(feature = "selftest")]
const FRAME_SIZE: u64 = 4096;

/// Device extents currently owned by the running address space.
#[cfg(feature = "selftest")]
pub(crate) fn device_mappings(context: &mut ThreadContext) -> Option<usize> {
    with_active_space(context, |space| space.device_mappings())
}

/// Active-root and device-ledger reclaim guards, without attempting reclaim.
#[cfg(feature = "selftest")]
pub(crate) fn active_space_reclaim_is_guarded(context: &mut ThreadContext) -> Option<bool> {
    with_active_space(context, |space| {
        space.permanent_root_blocks_reclaim_for_test() && space.devices_block_reclaim_for_test()
    })
}

/// Whether activation prevents later creation of an unshared kernel root.
#[cfg(feature = "selftest")]
pub(crate) fn active_kernel_topology_is_frozen(context: &mut ThreadContext) -> Option<bool> {
    with_active_space(context, AddressSpace::kernel_topology_is_frozen_for_test)
}

/// A live owned mapping made by a machine check.
///
/// The fields are private and the value is not copyable: the only removal
/// door below consumes exactly the mapping this module just created.
#[cfg(feature = "selftest")]
pub(crate) struct ActiveMapping {
    base: u64,
    len: u64,
}

/// Divide exact removal authority without widening the represented range.
#[cfg(feature = "selftest")]
pub(crate) fn split_mapping_for_test(
    mapping: ActiveMapping,
    left_len: u64,
) -> Option<(ActiveMapping, ActiveMapping)> {
    if left_len == 0 || left_len >= mapping.len || !left_len.is_multiple_of(FRAME_SIZE) {
        return None;
    }
    let right_base = mapping.base.checked_add(left_len)?;
    Some((
        ActiveMapping {
            base: mapping.base,
            len: left_len,
        },
        ActiveMapping {
            base: right_base,
            len: mapping.len - left_len,
        },
    ))
}

/// Add an owned mapping to the running tables for a machine check.
#[cfg(feature = "selftest")]
pub(crate) fn map_owned_for_test(
    context: &mut ThreadContext,
    base: u64,
    run: mm::Segment,
    flags: Flags,
) -> Option<Result<ActiveMapping, MapError>> {
    let len = run.len();
    with_active_space(context, |space| space.map_owned(base, run, flags))
        .map(|result| result.map(|()| ActiveMapping { base, len }))
}

/// Exercise the production map door against an already-pinned stack guard.
#[cfg(feature = "selftest")]
pub(crate) fn pinned_stack_refuses_map_for_test(
    context: &mut ThreadContext,
    slot: &stack_layout::Slot,
) -> Option<bool> {
    with_active_space(context, |space| {
        space.pinned_stack_refuses_map_for_test(slot)
    })?
}

/// Add a user mapping to the running tables for the user-copy machine proof.
#[cfg(feature = "selftest")]
pub(crate) fn map_user_for_test(
    context: &mut ThreadContext,
    base: u64,
    run: mm::USegment,
    flags: Flags,
) -> Option<Result<(), UserMapError>> {
    with_active_space(context, |space| space.map_user(base, run, flags))
}

/// Add one user page and retain its exact removal authority for a machine check.
#[cfg(feature = "selftest")]
pub(crate) fn map_user_page_for_test(
    context: &mut ThreadContext,
    base: u64,
    run: mm::Segment,
    flags: Flags,
) -> Option<Result<ActiveMapping, UserMapError>> {
    if run.len() != FRAME_SIZE {
        return None;
    }
    with_active_space(context, |space| space.map_user(base, run.into(), flags)).map(|result| {
        result.map(|()| ActiveMapping {
            base,
            len: FRAME_SIZE,
        })
    })
}

/// Map a guarded stack and retain the exact authority needed to remove it.
#[cfg(feature = "selftest")]
pub(crate) fn map_reclaimable_stack_for_test(
    context: &mut ThreadContext,
    layout: StackLayout,
) -> Option<Result<(MappedStack, ActiveMapping), StackMapError>> {
    let slot = layout.slot(0)?;
    with_active_space(context, |space| space.map_stack(&slot)).map(|result| {
        result.map(|stack| {
            (
                stack,
                ActiveMapping {
                    base: slot.stack_base(),
                    len: slot.stack_len(),
                },
            )
        })
    })
}

/// Map one stack arena into the active root with exact removal authority.
#[cfg(feature = "selftest")]
pub(crate) fn map_stack_arena_for_test(
    context: &mut ThreadContext,
    layout: StackLayout,
) -> Option<Result<(paging::StackArena, ActiveMapping), StackMapError>> {
    if layout.slots() != 1 {
        return None;
    }
    let slot = layout.slot(0)?;
    with_active_space(context, |space| space.map_stacks(&layout)).map(|result| {
        result.map(|arena| {
            (
                arena,
                ActiveMapping {
                    base: slot.stack_base(),
                    len: slot.stack_len(),
                },
            )
        })
    })
}

/// Remove only the active mapping whose unforgeable value is consumed here.
#[cfg(feature = "selftest")]
pub(crate) fn unmap_for_test(
    context: &mut ThreadContext,
    mapping: ActiveMapping,
) -> Option<Result<paging::Unmapped, MapError>> {
    with_active_space(context, |space| space.unmap(mapping.base, mapping.len))
}

/// Exercise fixed invalid-unmap requests without exporting an address-taking door.
#[cfg(feature = "selftest")]
pub(crate) fn invalid_unmaps_are_refused(context: &mut ThreadContext) -> Option<bool> {
    const HOLE: u64 = 0xffff_fd00_0004_0000;

    let text = boot::sections().text.start;
    let direct = mm::direct_map()?.virtual_address(x86_paging::LARGE_PAGE_SIZE)?;
    let task_stack = thread::layout()?.slot(0)?;
    let task_extent = task_stack
        .stack_top()
        .checked_sub(task_stack.guard_base())?;
    with_active_space(context, |space| {
        matches!(space.unmap(HOLE, FRAME_SIZE), Err(MapError::NotMapped(_)))
            && matches!(
                space.unmap(HOLE + 1, FRAME_SIZE),
                Err(MapError::Misaligned(_))
            )
            && matches!(space.unmap(HOLE, 0), Err(MapError::Misaligned(_)))
            && matches!(
                space.unmap(HOLE & 0x0000_ffff_ffff_ffff, FRAME_SIZE),
                Err(MapError::NonCanonical(_))
            )
            && matches!(space.unmap(text, FRAME_SIZE), Err(MapError::NotOwned(_)))
            && matches!(
                space.unmap(direct, FRAME_SIZE),
                Err(MapError::Unsplittable(_))
            )
            && matches!(
                space.unmap(task_stack.guard_base(), task_extent),
                Err(MapError::Pinned(found)) if found == task_stack.stack_base()
            )
            && space.lookup(task_stack.stack_base()).is_some()
    })
}

/// Try to remove the hardware-retained fault stack through the normal door.
#[cfg(feature = "selftest")]
pub(crate) fn unmap_fault_stack_for_test(
    context: &mut ThreadContext,
) -> Option<Result<paging::Unmapped, MapError>> {
    let slot = thread::fault_stack_layout()?.slot(0)?;
    with_active_space(context, |space| {
        space.unmap(slot.stack_base(), slot.stack_len())
    })
}

/// Where the direct map starts, for the checks that ask what a space holds.
#[cfg(feature = "selftest")]
pub(crate) fn direct_map_base() -> Option<u64> {
    mm::direct_map()?.virtual_address(0)
}

/// A partially populated boot space returns its complete table ledger.
#[cfg(feature = "selftest")]
pub(crate) fn partial_build_failure_is_atomic_for_test() -> bool {
    const PROBE: u64 = 0xffff_9100_0000_0000;

    let Some(physical) = mm::first_memory_run(FRAME_SIZE) else {
        return false;
    };
    let attempt = || -> Option<bool> {
        let mut space = AddressSpace::new().ok()?;
        let region = BootRegion::describe(physical, FRAME_SIZE)?;
        space.map_boot(PROBE, region, Flags::NO_EXECUTE).ok()?;
        let mapped = space.lookup(PROBE).is_some();
        Some(
            mapped
                && matches!(
                    rollback_build(space, MapError::OutOfMemory),
                    BuildError::Mapping(MapError::OutOfMemory)
                ),
        )
    };

    if attempt() != Some(true) {
        return false;
    }
    let free_before = mm::free_frames();
    attempt() == Some(true) && mm::free_frames() == free_before
}
