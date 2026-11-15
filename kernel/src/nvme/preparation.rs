use nvme::{Capabilities, Capacity, configuration, register};
use wost::arch::current::pci::MappedBar;
use x86_paging::PAGE_SIZE;

use super::{
    BringUpError, Controller, DisabledController, DmaState, PrepareFailure, PrepareOutcome,
    QueuePair, WANTED_ENTRIES, disable, read, write,
};
use crate::mem;
use crate::thread::ThreadContext;

pub(crate) fn prepare(context: &mut ThreadContext, mut registers: MappedBar) -> PrepareOutcome {
    let capabilities = match read_capabilities(&registers) {
        Ok(capabilities) => capabilities,
        Err(reason) => {
            return PrepareOutcome::Failed(PrepareFailure::quarantined(reason, registers));
        }
    };
    // Disabled before any policy refusal, so firmware DMA is never left on.
    if let Err(reason) = disable(&mut registers, capabilities) {
        return PrepareOutcome::Failed(PrepareFailure::quarantined(reason, registers));
    }
    let (admin, capacity) = match allocate_admin(capabilities) {
        Ok(allocated) => allocated,
        Err(reason) => return PrepareOutcome::Failed(retire_disabled(context, registers, reason)),
    };
    if let Err(reason) = program_admin_registers(&mut registers, &admin, capacity) {
        return PrepareOutcome::Failed(retire_disabled(context, registers, reason));
    }

    // `MPS` is the host page size as `log2 - 12`; the entry sizes are fixed logarithms.
    let page_shift = PAGE_SIZE.trailing_zeros() - 12;
    let cc = configuration::ENABLE
        | (page_shift << configuration::PAGE_SIZE_SHIFT)
        | (nvme::IOSQES << configuration::SUBMISSION_ENTRY_SHIFT)
        | (nvme::IOCQES << configuration::COMPLETION_ENTRY_SHIFT);

    // Cleanup owns every published queue address before the enable write.
    let controller = Controller {
        registers: Some(registers),
        capabilities,
        admin,
        io: None,
        next_id: 1,
        dma: DmaState::idle(),
        #[cfg(feature = "selftest")]
        force_timeout: false,
        #[cfg(feature = "selftest")]
        leave_data_unmapped: false,
        blocks: 0,
        block_bytes: 0,
    };

    PrepareOutcome::Prepared(DisabledController {
        controller,
        configuration: cc,
        capacity,
    })
}

fn read_capabilities(registers: &MappedBar) -> Result<Capabilities, BringUpError> {
    // `CAP` in two halves: an `MmioRegion` makes 32-bit accesses, which this register allows.
    let low = read(registers, register::CAP)?;
    let high = read(registers, register::CAP + 4)?;
    Ok(Capabilities::from_bits(
        u64::from(low) | (u64::from(high) << 32),
    ))
}

fn allocate_admin(capabilities: Capabilities) -> Result<(QueuePair, Capacity), BringUpError> {
    if !capabilities.supports_nvm_command_set() {
        return Err(BringUpError::NoCommandSet);
    }
    if !capabilities.supports_page_size(PAGE_SIZE) {
        return Err(BringUpError::PageSize);
    }
    let most = Capacity::new(capabilities.max_queue_entries()).ok_or(BringUpError::QueueSize)?;
    let capacity =
        Capacity::new(WANTED_ENTRIES.min(most.entries())).ok_or(BringUpError::QueueSize)?;
    let admin = QueuePair::new(0, capacity).ok_or(BringUpError::NoMemory)?;
    Ok((admin, capacity))
}

fn program_admin_registers(
    registers: &mut MappedBar,
    admin: &QueuePair,
    capacity: Capacity,
) -> Result<(), BringUpError> {
    let submission = admin
        .submission
        .address()
        .ok_or(BringUpError::NoMemory)?
        .get();
    let completion = admin
        .completion
        .address()
        .ok_or(BringUpError::NoMemory)?
        .get();
    let aqa = (capacity.as_zero_based() << 16) | capacity.as_zero_based();
    // Completions are polled, so every pin-based or single-MSI vector stays masked.
    let writes = [
        (register::INTMS, u32::MAX),
        (register::AQA, aqa),
        (register::ASQ, submission as u32),
        (register::ASQ + 4, (submission >> 32) as u32),
        (register::ACQ, completion as u32),
        (register::ACQ + 4, (completion >> 32) as u32),
    ];
    for (offset, value) in writes {
        write(registers, offset, value)?;
    }
    Ok(())
}

fn retire_disabled(
    context: &mut ThreadContext,
    registers: MappedBar,
    reason: BringUpError,
) -> PrepareFailure {
    match mem::unmap_mapped_pci_bar_exact(context, registers) {
        mem::MappedPciBarRetirement::Retired => PrepareFailure::Retired(reason),
        mem::MappedPciBarRetirement::Retained(registers) => {
            PrepareFailure::quarantined(reason, registers)
        }
        mem::MappedPciBarRetirement::Quarantined => PrepareFailure::memory_quarantine(reason),
    }
}
