use crate::nvme;

pub(super) fn a_block_read_returns_the_block_that_was_asked_for() -> bool {
    let Some(entries) = nvme::queue_entries_for_test() else {
        return false;
    };
    // One read past a full lap of the I/O queue, so its completion phase tag flips.
    let lap = (0..=entries).map(|turn| u64::from(turn % 8));
    let read = [0u64, 3, 1, 7].into_iter().chain(lap).all(|lba| {
        nvme::read_block_probe_for_test(lba).is_some_and(|(block, head, beyond)| {
            block >= 8
                && u64::from_le_bytes(head) == disk_signature(lba)
                && u64::from_le_bytes(beyond) == 0
        })
    });
    read && nvme::read_block_probe_for_test(u64::MAX).is_none()
}

pub(super) fn a_device_write_into_an_unmapped_frame_faults_and_changes_nothing() -> bool {
    nvme::unmapped_write_is_fenced_for_test()
        .is_some_and(|witness| witness.completed && witness.unchanged && witness.faulted)
}

pub(super) fn an_nvme_timeout_quiesces_before_dma_memory_returns() -> bool {
    nvme::timeout_quiesces_for_test()
}

// Kept in step with the signature xtask writes at the head of every block of the check disk.
const fn disk_signature(lba: u64) -> u64 {
    0x5749_5A44_0000_0000 | lba
}
