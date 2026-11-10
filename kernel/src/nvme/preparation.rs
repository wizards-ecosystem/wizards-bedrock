use nvme::{
    Capabilities, Capacity, ControllerPreparation, DisabledPreparation, PollLimit,
    PreparationFailure, configuration, register, status,
};
use wost::arch::current::pci::MappedBar;
use x86_paging::PAGE_SIZE;

use super::{
    BringUpError, Controller, DisabledController, DmaState, PrepareFailure, PrepareOutcome,
    QueuePair, WANTED_ENTRIES, clock_ns, fallback_polls, read, write,
};
use crate::mem;
use crate::thread::ThreadContext;

/// Bring a controller up on its mapped register window: the specification's initialisation
/// sequence in order, with both waits bounded by `CAP.TO`.
pub(crate) fn prepare(context: &mut ThreadContext, registers: MappedBar) -> PrepareOutcome {
    #[cfg(feature = "selftest")]
    let observed_window = registers.address();

    let (preparation, capabilities) = match read_capabilities(registers) {
        CapabilityRead::Read(preparation, capabilities) => (preparation, capabilities),
        CapabilityRead::Failed(failure) => return PrepareOutcome::Failed(failure),
    };
    let disabled = match disable_preparation(preparation, capabilities) {
        DisableOutcome::Disabled(disabled) => disabled,
        DisableOutcome::Failed(failure) => return PrepareOutcome::Failed(failure),
    };
    let admin = match allocate_admin(
        context,
        disabled,
        capabilities,
        #[cfg(feature = "selftest")]
        observed_window,
    ) {
        AdminAllocation::Allocated(admin) => admin,
        AdminAllocation::Failed(failure) => return PrepareOutcome::Failed(failure),
    };
    program_admin_registers(context, admin)
}

struct AdminPreparation {
    disabled: DisabledPreparation<MappedBar>,
    capabilities: Capabilities,
    admin: QueuePair,
    capacity: Capacity,
    submission: u64,
    completion: u64,
    #[cfg(feature = "selftest")]
    observed_window: u64,
}

enum CapabilityRead {
    Read(ControllerPreparation<MappedBar>, Capabilities),
    Failed(PrepareFailure),
}

enum DisableOutcome {
    Disabled(DisabledPreparation<MappedBar>),
    Failed(PrepareFailure),
}

enum AdminAllocation {
    Allocated(AdminPreparation),
    Failed(PrepareFailure),
}

enum DisabledRegisterWrite {
    Written(DisabledPreparation<MappedBar>),
    Failed(DisabledPreparationFailure),
}

fn read_capabilities(registers: MappedBar) -> CapabilityRead {
    let preparation = ControllerPreparation::new(registers);

    // `CAP` in two halves: an `MmioRegion` makes 32-bit accesses, which this register allows.
    let (preparation, low) = match preparation.access(|registers| read(registers, register::CAP)) {
        Ok(result) => result,
        Err(failure) => {
            return CapabilityRead::Failed(quarantine_preparation_access(failure));
        }
    };
    let (preparation, high) =
        match preparation.access(|registers| read(registers, register::CAP + 4)) {
            Ok(result) => result,
            Err(failure) => {
                return CapabilityRead::Failed(quarantine_preparation_access(failure));
            }
        };
    let capabilities = Capabilities::from_bits(u64::from(low) | (u64::from(high) << 32));
    CapabilityRead::Read(preparation, capabilities)
}

fn disable_preparation(
    preparation: ControllerPreparation<MappedBar>,
    capabilities: Capabilities,
) -> DisableOutcome {
    // Disabled before any policy refusal, so even an unsupported controller
    // can be retired without leaving firmware DMA behind.
    let (preparation, ()) = match preparation.access(|registers| write(registers, register::CC, 0))
    {
        Ok(result) => result,
        Err(failure) => {
            return DisableOutcome::Failed(quarantine_preparation_access(failure));
        }
    };
    let preparation = match wait_for_disabled_preparation(preparation, capabilities) {
        PreparationWait::Disabled(preparation) => preparation,
        PreparationWait::Failed(failure) => return DisableOutcome::Failed(failure),
    };
    let (preparation, cc) = match preparation.access(|registers| read(registers, register::CC)) {
        Ok(result) => result,
        Err(failure) => {
            return DisableOutcome::Failed(quarantine_preparation_access(failure));
        }
    };
    let (preparation, csts) = match preparation.access(|registers| read(registers, register::CSTS))
    {
        Ok(result) => result,
        Err(failure) => {
            return DisableOutcome::Failed(quarantine_preparation_access(failure));
        }
    };
    match preparation.prove_disabled(cc, csts) {
        Ok(disabled) => DisableOutcome::Disabled(disabled),
        Err(failure) => {
            let (_reason, preparation) = failure.into_parts();
            DisableOutcome::Failed(PrepareFailure::quarantined(
                BringUpError::NotReady,
                preparation,
            ))
        }
    }
}

fn allocate_admin(
    context: &mut ThreadContext,
    disabled: DisabledPreparation<MappedBar>,
    capabilities: Capabilities,
    #[cfg(feature = "selftest")] observed_window: u64,
) -> AdminAllocation {
    if !capabilities.supports_nvm_command_set() {
        return AdminAllocation::Failed(retire_disabled(
            context,
            disabled,
            BringUpError::NoCommandSet,
        ));
    }
    if !capabilities.supports_page_size(PAGE_SIZE) {
        return AdminAllocation::Failed(retire_disabled(context, disabled, BringUpError::PageSize));
    }
    let Some(most) = Capacity::new(capabilities.max_queue_entries()) else {
        return AdminAllocation::Failed(retire_disabled(
            context,
            disabled,
            BringUpError::QueueSize,
        ));
    };
    let Some(capacity) = Capacity::new(WANTED_ENTRIES.min(most.entries())) else {
        return AdminAllocation::Failed(retire_disabled(
            context,
            disabled,
            BringUpError::QueueSize,
        ));
    };

    let Some(admin) = QueuePair::new(0, capacity) else {
        return AdminAllocation::Failed(retire_disabled(context, disabled, BringUpError::NoMemory));
    };
    let Some(submission) = admin.submission.address() else {
        return AdminAllocation::Failed(retire_disabled(context, disabled, BringUpError::NoMemory));
    };
    let Some(completion) = admin.completion.address() else {
        return AdminAllocation::Failed(retire_disabled(context, disabled, BringUpError::NoMemory));
    };
    AdminAllocation::Allocated(AdminPreparation {
        disabled,
        capabilities,
        admin,
        capacity,
        submission,
        completion,
        #[cfg(feature = "selftest")]
        observed_window,
    })
}

fn program_admin_registers(
    context: &mut ThreadContext,
    preparation: AdminPreparation,
) -> PrepareOutcome {
    let AdminPreparation {
        mut disabled,
        capabilities,
        admin,
        capacity,
        submission,
        completion,
        #[cfg(feature = "selftest")]
        observed_window,
    } = preparation;
    let aqa = (capacity.as_zero_based() << 16) | capacity.as_zero_based();
    // The controller latches ASQ and ACQ when their high half is written.
    let writes = [
        (register::AQA, aqa),
        (register::ASQ, submission as u32),
        (register::ASQ + 4, (submission >> 32) as u32),
        (register::ACQ, completion as u32),
        (register::ACQ + 4, (completion >> 32) as u32),
    ];
    for (offset, value) in writes {
        disabled = match write_disabled_register(disabled, offset, value) {
            DisabledRegisterWrite::Written(disabled) => disabled,
            DisabledRegisterWrite::Failed(failure) => {
                return PrepareOutcome::Failed(retire_failed_preparation_access(context, failure));
            }
        };
    }
    finish_preparation(
        disabled.into_owner(),
        capabilities,
        admin,
        capacity,
        #[cfg(feature = "selftest")]
        observed_window,
    )
}

fn write_disabled_register(
    disabled: DisabledPreparation<MappedBar>,
    offset: usize,
    value: u32,
) -> DisabledRegisterWrite {
    match disabled.access(|registers| write(registers, offset, value)) {
        Ok((disabled, ())) => DisabledRegisterWrite::Written(disabled),
        Err(failure) => DisabledRegisterWrite::Failed(failure),
    }
}

fn finish_preparation(
    registers: MappedBar,
    capabilities: Capabilities,
    admin: QueuePair,
    capacity: Capacity,
    #[cfg(feature = "selftest")] observed_window: u64,
) -> PrepareOutcome {
    // `MPS` is the host page size as `log2 - 12`; the entry sizes are fixed logarithms.
    let page_shift = PAGE_SIZE.trailing_zeros() - 12;
    let cc = configuration::ENABLE
        | (page_shift << configuration::PAGE_SIZE_SHIFT)
        | (nvme::IOSQES << configuration::SUBMISSION_ENTRY_SHIFT)
        | (nvme::IOCQES << configuration::COMPLETION_ENTRY_SHIFT);

    // Cleanup owns every published queue address before the enable write.
    let controller = Controller {
        #[cfg(feature = "selftest")]
        observed_window,
        registers: Some(registers),
        capabilities,
        admin,
        io: None,
        next_id: 1,
        #[cfg(feature = "selftest")]
        published_commands: 0,
        dma: DmaState::idle(),
        #[cfg(feature = "selftest")]
        force_timeout: false,
        #[cfg(feature = "selftest")]
        intercept_read_submission: false,
        #[cfg(feature = "selftest")]
        intercepted_read_submissions: 0,
        #[cfg(feature = "selftest")]
        intercept_io_queue_creation: false,
        #[cfg(feature = "selftest")]
        intercepted_io_queue_creations: 0,
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

/// Return a mapping while the controller is already verified disabled.
fn retire_disabled(
    context: &mut ThreadContext,
    registers: DisabledPreparation<MappedBar>,
    reason: BringUpError,
) -> PrepareFailure {
    match mem::unmap_mapped_pci_bar_exact(context, registers.into_owner()) {
        mem::MappedPciBarRetirement::Retired => PrepareFailure::Retired(reason),
        mem::MappedPciBarRetirement::Retained(registers) => {
            PrepareFailure::disabled_quarantine(reason, registers)
        }
        mem::MappedPciBarRetirement::Quarantined => PrepareFailure::memory_quarantine(reason),
    }
}

type UnverifiedPreparationFailure =
    PreparationFailure<ControllerPreparation<MappedBar>, BringUpError>;
type DisabledPreparationFailure = PreparationFailure<DisabledPreparation<MappedBar>, BringUpError>;

fn quarantine_preparation_access(failure: UnverifiedPreparationFailure) -> PrepareFailure {
    let (reason, preparation) = failure.into_parts();
    PrepareFailure::quarantined(reason, preparation)
}

fn retire_failed_preparation_access(
    context: &mut ThreadContext,
    failure: DisabledPreparationFailure,
) -> PrepareFailure {
    let (reason, preparation) = failure.into_parts();
    retire_disabled(context, preparation, reason)
}

enum PreparationWait {
    Disabled(ControllerPreparation<MappedBar>),
    Failed(PrepareFailure),
}

fn wait_for_disabled_preparation(
    mut preparation: ControllerPreparation<MappedBar>,
    capabilities: Capabilities,
) -> PreparationWait {
    let timeout_ms = capabilities.timeout_ms().max(500);
    let fallback = fallback_polls(timeout_ms);
    let mut limit = PollLimit::after_ms(clock_ns(), timeout_ms, fallback);
    while limit.permit(clock_ns()) {
        let (returned, bits) = match preparation.access(|registers| read(registers, register::CSTS))
        {
            Ok(result) => result,
            Err(failure) => {
                return PreparationWait::Failed(quarantine_preparation_access(failure));
            }
        };
        preparation = returned;
        if bits & status::READY == 0 {
            return PreparationWait::Disabled(preparation);
        }
        core::hint::spin_loop();
    }
    PreparationWait::Failed(PrepareFailure::quarantined(
        BringUpError::NotReady,
        preparation,
    ))
}
