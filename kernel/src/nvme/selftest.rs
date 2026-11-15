use x86_paging::PAGE_SIZE;

use super::{BringUpError, CONTROLLER};

pub(crate) fn read_block_probe_for_test(lba: u64) -> Option<(usize, [u8; 8], [u8; 8])> {
    let mut held = CONTROLLER.lock();
    let (controller, buffer) = held.as_mut()?;
    let block = controller.block_bytes as usize;
    if !buffer.fill(0) || controller.read_blocks(lba, 1, buffer).is_err() {
        return None;
    }
    let mut head = [0u8; 8];
    let mut beyond = [0u8; 8];
    if !buffer.read_at(0, &mut head) || !buffer.read_at(block, &mut beyond) {
        return None;
    }
    Some((block, head, beyond))
}

pub(crate) fn queue_entries_for_test() -> Option<u32> {
    CONTROLLER
        .lock()
        .as_ref()
        .map(|(controller, _)| controller.queue_entries())
}

pub(crate) fn timeout_quiesces_for_test() -> bool {
    let mut held = CONTROLLER.lock();
    let Some((controller, buffer)) = held.as_mut() else {
        return false;
    };
    controller.a_timeout_quiesces_before_dma_returns(buffer)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FenceWitness {
    pub(crate) completed: bool,
    pub(crate) unchanged: bool,
    pub(crate) faulted: bool,
}

pub(crate) fn unmapped_write_is_fenced_for_test() -> Option<FenceWitness> {
    const PATTERN: u8 = 0xA5;

    let mut held = CONTROLLER.lock();
    let (controller, buffer) = held.as_mut()?;
    // The priming read maps and unmaps the buffer, so a missed IOTLB invalidation would show.
    controller.read_blocks(0, 1, buffer).ok()?;
    let source = {
        let mut installed = crate::iommu::installed();
        let isolation = installed.as_mut()?;
        isolation.drain_faults();
        let function = isolation.function()?;
        (u16::from(function.bus()) << 8)
            | (u16::from(function.device()) << 3)
            | u16::from(function.function())
    };
    if !buffer.fill(PATTERN) {
        return None;
    }
    let page = buffer.address()?.get() & !(PAGE_SIZE - 1);

    controller.leave_data_unmapped = true;
    let result = controller.read_blocks(0, 1, buffer);
    controller.leave_data_unmapped = false;

    let mut bytes = alloc::vec![0u8; controller.block_bytes as usize];
    if !buffer.read_at(0, &mut bytes) {
        return None;
    }
    let mut faulted = false;
    let mut installed = crate::iommu::installed();
    while let Some(fault) = installed.as_mut()?.take_fault() {
        faulted |= fault.source == source && fault.address == page && fault.write;
    }
    Some(FenceWitness {
        completed: matches!(result, Ok(()) | Err(BringUpError::Command(_))),
        unchanged: bytes.iter().all(|byte| *byte == PATTERN),
        faulted,
    })
}
