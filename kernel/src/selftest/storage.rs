use wost::arch::current::paging;
use wost::boot;
use wost::mm;
use x86_paging::{Flags, PAGE_SIZE};

use crate::info;
use crate::iommu;
use crate::nvme;

pub(super) fn the_controller_was_read_rather_than_assumed() -> bool {
    nvme::controller_facts_hold_for_test() && nvme::bus_master_followed_disable_for_test()
}

pub(super) fn nvme_candidates_preserve_boot_stack_headroom() -> bool {
    const CAPACITY: u64 = 32 * 1024;
    const MINIMUM_HEADROOM: u64 = 2 * PAGE_SIZE;

    let Some(stack) = nvme::candidate_stack_usage_for_test() else {
        return false;
    };
    info!(
        "selftest: nvme-stack v2 used={} capacity={} headroom={} leaf={}",
        stack.deepest.used(),
        stack.deepest.capacity(),
        stack.deepest.headroom(),
        stack.leaf.used()
    );
    stack.deepest.capacity() == CAPACITY && stack.deepest.headroom() >= MINIMUM_HEADROOM
}

pub(super) fn a_failed_bus_master_admission_stops_before_nvme_start() -> bool {
    crate::refused_bus_master_retires_before_nvme_start()
}

pub(super) fn invalid_nvme_namespace_stops_before_io_queues() -> bool {
    nvme::malformed_namespace_is_bounded_for_test()
}

pub(super) fn invalid_nvme_reads_stop_before_submission() -> bool {
    nvme::malformed_reads_are_bounded_for_test()
}

pub(super) fn a_block_read_returns_the_block_that_was_asked_for() -> bool {
    for lba in [0u64, 3, 1, 7] {
        let Some((block, head, beyond)) = nvme::read_block_probe_for_test(lba) else {
            return false;
        };
        if block < 8
            || u64::from_le_bytes(head) != disk_signature(lba)
            || u64::from_le_bytes(beyond) != 0
        {
            return false;
        }
    }
    true
}

pub(super) fn the_queues_wrap_and_the_phase_tag_still_works() -> bool {
    let Some(entries) = nvme::queue_entries_for_test() else {
        return false;
    };
    let laps = entries * 3;
    for turn in 0..laps {
        let lba = u64::from(turn % 8);
        let Some((_, head, _)) = nvme::read_block_probe_for_test(lba) else {
            return false;
        };
        if u64::from_le_bytes(head) != disk_signature(lba) {
            return false;
        }
    }
    laps > entries
}

pub(super) fn the_controllers_registers_are_uncacheable() -> bool {
    nvme::controller_window_for_test()
        .and_then(paging::translate)
        .is_some_and(|(_, flags)| {
            flags.contains(Flags::NO_CACHE) && !flags.contains(Flags::WRITE_THROUGH)
        })
}

pub(super) fn an_nvme_timeout_quiesces_before_dma_memory_returns() -> bool {
    nvme::timeout_quiesces_for_test()
}

pub(super) fn translation_is_active_for_the_storage_function() -> bool {
    nvme::isolation_is_active_for_test().is_some_and(|(minted, _)| minted)
}

pub(super) fn every_shared_vtd_requester_was_fenced_across_controller_fallback() -> bool {
    iommu::requester_fences_cover_shared_unit_for_test()
}

pub(super) fn the_unit_walks_the_kernels_root_table() -> bool {
    nvme::isolation_is_active_for_test().is_some_and(|(_, walks_kernel_root)| walks_kernel_root)
}

/// Read back from the second-level tables the unit walks, not from the driver's index of them:
/// the reachable set is exactly the live queue frames while no transfer is published.
pub(super) fn the_dma_domain_reaches_exactly_the_live_queue_frames() -> bool {
    nvme::reachable_frames_for_test()
        .is_some_and(|(reachable, expected)| !expected.is_empty() && reachable == expected)
}

pub(super) fn a_domain_proof_is_refused_while_translation_is_off() -> bool {
    nvme::domain_proof_follows_translation_for_test()
}

/// The hook reads into the buffer once first, so a stale cached translation would let the
/// unmapped write through; the fault proves the unmap invalidated the unit's cache and the device
/// cannot reach the frame.
pub(super) fn a_device_write_into_an_unmapped_frame_faults_and_changes_nothing() -> bool {
    nvme::unmapped_write_is_fenced_for_test().is_some_and(|witness| {
        witness.completed && witness.drained && witness.unchanged && witness.faulted
    })
}

pub(super) fn a_proven_vtd_revocation_releases_only_its_mapping_owner() -> bool {
    use wost::arch::current::vtd::SegmentChangeError;

    let before = mm::free_frames();
    let Some(segment) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let mut dma = mm::DmaSegment::from(segment);
    let checked = {
        let mut installed = crate::iommu::installed();
        let Some(isolation) = installed.as_mut() else {
            return false;
        };
        let drained = isolation.completed_dma_drains_for_test();
        let initially_unmapped = matches!(
            isolation.unmap_segment(&mut dma),
            Err(SegmentChangeError::Refused(_))
        ) && isolation.completed_dma_drains_for_test() == drained;
        if isolation.map_segment(&mut dma).is_err() {
            return false;
        }
        let duplicate = matches!(
            isolation.map_segment(&mut dma),
            Err(SegmentChangeError::Refused(_))
        );
        let revoked = isolation.unmap_segment(&mut dma).is_ok();
        let once = isolation.completed_dma_drains_for_test() == drained + 1;
        let again = matches!(
            isolation.unmap_segment(&mut dma),
            Err(SegmentChangeError::Refused(_))
        ) && isolation.completed_dma_drains_for_test() == drained + 1;
        initially_unmapped && duplicate && revoked && once && again
    };
    drop(dma);
    checked && mm::free_frames() == before
}

pub(super) fn a_rolled_back_vtd_mapping_releases_its_mapping_owner() -> bool {
    use wost::arch::current::vtd::SegmentChangeError;

    let before = mm::free_frames();
    let Some(segment) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let mut dma = mm::DmaSegment::from(segment);
    let checked = {
        let mut installed = crate::iommu::installed();
        let Some(isolation) = installed.as_mut() else {
            return false;
        };
        let drained = isolation.completed_dma_drains_for_test();
        isolation.force_invalidation_failures_for_test(1);
        let refused = matches!(
            isolation.map_segment(&mut dma),
            Err(SegmentChangeError::Refused(_))
        ) && !dma.is_retained()
            && isolation.completed_dma_drains_for_test() == drained + 1;
        isolation.force_invalidation_failures_for_test(0);
        let remapped = isolation.map_segment(&mut dma).is_ok();
        let revoked = isolation.unmap_segment(&mut dma).is_ok();
        refused && remapped && revoked
    };
    drop(dma);
    checked && mm::free_frames() == before
}

pub(super) fn an_uncertain_vtd_mapping_never_recycles_its_frames() -> bool {
    let before = mm::free_frames();
    let Some(segment) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let mut dma = mm::DmaSegment::from(segment);
    let reserved = mm::free_frames() == before - 1;
    let retained = {
        let mut installed = crate::iommu::installed();
        let Some(isolation) = installed.as_mut() else {
            return false;
        };
        // The first refusal makes the just-installed translation uncertain;
        // the second makes rollback uncertainty explicit.
        isolation.force_invalidation_failures_for_test(2);
        matches!(
            isolation.map_segment(&mut dma),
            Err(wost::arch::current::vtd::SegmentChangeError::Retained(_))
        ) && dma.is_retained()
    };
    drop(dma);
    reserved && retained && mm::free_frames() == before - 1
}

pub(super) fn an_uncertain_vtd_unmap_never_recycles_its_frames() -> bool {
    let before = mm::free_frames();
    let Some(segment) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let mut dma = mm::DmaSegment::from(segment);
    let reserved = mm::free_frames() == before - 1;
    let retained = {
        let mut installed = crate::iommu::installed();
        let Some(isolation) = installed.as_mut() else {
            return false;
        };
        if isolation.map_segment(&mut dma).is_err() {
            return false;
        }
        // The leaf is removed, but a failed invalidation means the device may still hold its old
        // translation. Ownership must therefore disappear into the DMA segment's boot-lifetime
        // retention state.
        isolation.force_invalidation_failures_for_test(1);
        matches!(
            isolation.unmap_segment(&mut dma),
            Err(wost::arch::current::vtd::SegmentChangeError::Retained(_))
        ) && dma.is_retained()
    };
    drop(dma);
    reserved && retained && mm::free_frames() == before - 1
}

/// Firmware-reserved DMA regions carry no authority without a typed physical
/// classification: neither a device hole nor usable RAM is admitted.
pub(super) fn firmware_reserved_dma_regions_never_become_mapping_authority() -> bool {
    let Some(map) = mm::memory_map_for_test() else {
        return false;
    };
    let device = |kind: boot::MemoryKind| {
        matches!(
            kind,
            boot::MemoryKind::Reserved | boot::MemoryKind::Unknown(_)
        )
    };
    let mut hole = None;
    let mut attempted = 0usize;
    'regions: for region in map.regions().filter(|region| device(region.kind)) {
        let (Some(mut physical), Some(region_end)) = (
            region.base.checked_next_multiple_of(PAGE_SIZE),
            region.base.checked_add(region.len),
        ) else {
            continue;
        };
        while let Some(end) = physical
            .checked_add(PAGE_SIZE)
            .filter(|&end| end <= region_end)
        {
            attempted += 1;
            if attempted > 256 {
                break 'regions;
            }
            if map.regions().all(|other| {
                let overlaps = physical < other.base + other.len && other.base < end;
                !overlaps || device(other.kind)
            }) {
                hole = Some(physical);
                break 'regions;
            }
            physical = end;
        }
    }
    let usable = map
        .regions()
        .filter(|region| region.kind == boot::MemoryKind::Usable)
        .find_map(|region| {
            let base = region.base.checked_next_multiple_of(PAGE_SIZE)?;
            (base.checked_add(PAGE_SIZE)? <= region.base.checked_add(region.len)?).then_some(base)
        });
    let (Some(hole), Some(usable)) = (hole, usable) else {
        return false;
    };
    iommu::reserved_regions_are_supported_for_test(&[])
        && !iommu::reserved_regions_are_supported_for_test(&[(hole, hole + PAGE_SIZE - 1)])
        && !iommu::reserved_regions_are_supported_for_test(&[(usable, usable + PAGE_SIZE - 1)])
}

/// The first eight bytes of block `lba` on the check machine's namespace, written by `xtask` when
/// it makes the backing file and transcribed here; a host test compares the two copies.
const fn disk_signature(lba: u64) -> u64 {
    0x5749_5A44_0000_0000 | lba
}
