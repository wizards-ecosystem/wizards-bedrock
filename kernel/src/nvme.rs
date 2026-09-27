//! An NVMe controller, polled.
//!
//! `libs/nvme` owns every number; `wost` owns the memory-mapped accesses and
//! the frames. What is here is the sequence: bring a controller up, ask it what
//! it is, build one I/O queue pair, and read a block.
//!
//! **Polled is what makes this affordable** (`012` §2). `Create I/O Completion
//! Queue` with `IEN = 0` disables interrupts for that queue, which removes
//! "this kernel has no interrupt sharing" from the critical path entirely. The
//! cost is that a read spins. D-200 confines that polling to the one file
//! worker, where timer and input interrupts may preempt it; `RingEnter` only
//! admits the request. PW-704 requires a measured workload before replacing it.
//!
//! **Three things decide whether this works at all, and none is optional.**
//! Bus master must be on or the controller cannot DMA and *there is no error
//! anywhere to read*. The PCI lease withholds it until controller-disable
//! readback is exact. The register window must be mapped **uncacheable**;
//! write-back is a silent, hard-to-debug bug. And `CAP` is read, never assumed:
//! the queue size, the doorbell stride, the timeout and the page-size range all
//! come from it, and `012` §11 records that VMware's values are unmeasured.
//!
//! **The ring parallel is a trap.** This looks like `kernel::queue` and is not:
//! there is no completion tail to read, entries are found by their phase tag,
//! completions are matched by command identifier rather than by slot, and
//! capacity is `size - 1`. The arithmetic is shared through `libs/nvme`; the
//! structure is not (D-169).

#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use nvme::{
    Capabilities, Capacity, Command, Completion, ControllerPreparation, DisabledPreparation,
    DisabledRegisters, DmaPublication, DmaState as DmaFlightState, NAMESPACE_IDENTIFY_BYTES,
    Namespace, NamespaceError, PollLimit, PreparationFailure, ReadTransfer, TransferError,
    configuration, register, status,
};
#[cfg(feature = "selftest")]
use stack_layout::{Slot as StackSlot, StackUsage};
#[cfg(feature = "selftest")]
use wost::arch::current::cpu;
use wost::arch::current::pci::MappedBar;
use wost::mm::{self, DmaSegment};
use wost::sync::ThreadLock;
use x86_paging::PAGE_SIZE;

#[cfg(feature = "selftest")]
use crate::thread;
use crate::thread::ThreadContext;
use crate::{info, mem};

#[cfg(feature = "selftest")]
static BME_PRECEDING_CC: AtomicUsize = AtomicUsize::new(usize::MAX);

#[cfg(feature = "selftest")]
static BME_PRECEDING_CSTS: AtomicUsize = AtomicUsize::new(usize::MAX);

#[cfg(feature = "selftest")]
static REFUSED_BME_PRECEDING_CC: AtomicUsize = AtomicUsize::new(usize::MAX);

#[cfg(feature = "selftest")]
static REFUSED_BME_PRECEDING_CSTS: AtomicUsize = AtomicUsize::new(usize::MAX);

#[cfg(feature = "selftest")]
static START_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
static SUBMISSION_DOORBELLS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
const STACK_METER_IDLE: usize = 0;
#[cfg(feature = "selftest")]
const STACK_METER_PREPARING: usize = 1;
#[cfg(feature = "selftest")]
const STACK_METER_ACTIVE: usize = 2;
#[cfg(feature = "selftest")]
const STACK_METER_OBSERVING: usize = 3;
#[cfg(feature = "selftest")]
const STACK_METER_FINISHING: usize = 4;

#[cfg(feature = "selftest")]
static STACK_METER_STATE: AtomicUsize = AtomicUsize::new(STACK_METER_IDLE);
#[cfg(feature = "selftest")]
static STACK_LOWEST: AtomicU64 = AtomicU64::new(u64::MAX);
#[cfg(feature = "selftest")]
static STACK_MAX_USED: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "selftest")]
static STACK_PREPARATIONS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static STACK_MEASUREMENTS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "selftest")]
static STACK_MEASUREMENT_VALID: AtomicBool = AtomicBool::new(true);

#[cfg(feature = "selftest")]
fn count(observation: &AtomicUsize) {
    assert!(
        observation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .is_ok(),
        "an NVMe self-test observation exhausted"
    );
}

#[cfg(feature = "selftest")]
#[derive(Debug)]
struct PreparationStackMeasurement {
    slot: StackSlot,
}

#[cfg(feature = "selftest")]
impl PreparationStackMeasurement {
    fn begin() -> Option<Self> {
        let slot = thread::boot_stack_layout()?.slot(0)?;
        let pointer = cpu::stack_pointer();
        slot.usage_at(pointer)?;
        STACK_METER_STATE
            .compare_exchange(
                STACK_METER_IDLE,
                STACK_METER_PREPARING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()?;
        STACK_LOWEST.store(pointer, Ordering::Relaxed);
        STACK_METER_STATE.store(STACK_METER_ACTIVE, Ordering::Release);
        Some(Self { slot })
    }

    fn finish(self) -> Option<StackUsage> {
        STACK_METER_STATE
            .compare_exchange(
                STACK_METER_ACTIVE,
                STACK_METER_FINISHING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()?;
        let usage = self.slot.usage_at(STACK_LOWEST.load(Ordering::Acquire));
        STACK_METER_STATE.store(STACK_METER_IDLE, Ordering::Release);
        usage
    }
}

#[cfg(feature = "selftest")]
#[inline(never)]
fn observe_preparation_stack() {
    if STACK_METER_STATE
        .compare_exchange(
            STACK_METER_ACTIVE,
            STACK_METER_OBSERVING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }
    STACK_LOWEST.fetch_min(cpu::stack_pointer(), Ordering::AcqRel);
    STACK_METER_STATE.store(STACK_METER_ACTIVE, Ordering::Release);
}

#[cfg(feature = "selftest")]
fn finish_stack_measurement(measurement: Option<PreparationStackMeasurement>) {
    let Some(usage) = measurement.and_then(PreparationStackMeasurement::finish) else {
        STACK_MEASUREMENT_VALID.store(false, Ordering::Release);
        return;
    };
    STACK_MAX_USED.fetch_max(usage.used(), Ordering::AcqRel);
    count(&STACK_MEASUREMENTS);
}

#[cfg(feature = "selftest")]
pub(crate) fn preparation_stack_usage_for_test() -> Option<StackUsage> {
    let preparations = STACK_PREPARATIONS.load(Ordering::Acquire);
    if !STACK_MEASUREMENT_VALID.load(Ordering::Acquire)
        || preparations == 0
        || STACK_MEASUREMENTS.load(Ordering::Acquire) != preparations
    {
        return None;
    }
    let slot = thread::boot_stack_layout()?.slot(0)?;
    let pointer = slot
        .stack_top()
        .checked_sub(STACK_MAX_USED.load(Ordering::Acquire))?;
    slot.usage_at(pointer)
}

/// Entries in the queues this driver builds.
///
/// Small: one command is outstanding at a time while the path is polled, and a
/// deeper queue would be depth nothing uses. It is clamped against `CAP.MQES`
/// rather than assumed, because a controller may offer fewer.
const WANTED_ENTRIES: u32 = 8;

/// The identifier of the one I/O queue pair.
const IO_QUEUE: u16 = 1;

/// The namespace this driver reads from.
///
/// One, which is the first and the only one QEMU's controller presents. A
/// driver that enumerated namespaces would ask `Identify` for the list; this
/// one reads the namespace it was told about and says so.
const NAMESPACE: u32 = 1;

/// How many completion reads boot may attempt before the clock is calibrated.
///
/// Once calibrated, the command deadline is real time instead. The attempt
/// bound keeps early bring-up finite before that service exists (D-200).
const POLL_ATTEMPTS: u32 = 5_000_000;

/// Real-time bound for one command once the monotonic clock exists.
const COMMAND_TIMEOUT_MS: u64 = 5_000;

/// One submission queue and the completion queue that answers it.
#[derive(Debug)]
struct QueuePair {
    /// Which queue this is; zero is the admin pair.
    id: u16,
    /// How many entries each holds.
    capacity: Capacity,
    /// The submission entries, physically contiguous.
    submission: DmaSegment,
    /// The completion entries, likewise.
    completion: DmaSegment,
    /// Where the host will write next. **Shadowed**, because the doorbell is
    /// write-only and the controller's idea of the tail cannot be read back.
    tail: u32,
    submission_head: u32,
    completion_head: u32,
    /// The phase a *new* completion carries.
    ///
    /// There is no tail to read: an entry is new when its phase tag differs
    /// from the last pass's. A zeroed queue starts at zero, so the first pass
    /// looks for one.
    phase: bool,
}

impl QueuePair {
    /// Allocate and clear both queues.
    fn new(id: u16, capacity: Capacity) -> Option<Self> {
        let submission_frames = frames_for(capacity.submission_bytes());
        let completion_frames = frames_for(capacity.completion_bytes());

        let mut submission = DmaSegment::from(mm::allocate_contiguous(submission_frames, 1)?);
        let mut completion = DmaSegment::from(mm::allocate_contiguous(completion_frames, 1)?);
        // Cleared, because the phase tag of an unwritten entry is what says
        // "nothing here yet", and stale memory says whatever it last held.
        if !submission.zero() || !completion.zero() {
            return None;
        }

        Some(Self {
            id,
            capacity,
            submission,
            completion,
            tail: 0,
            submission_head: 0,
            completion_head: 0,
            phase: true,
        })
    }

    /// Withhold every queue frame a controller may still reach.
    fn retain(&mut self) {
        self.submission.retain();
        self.completion.retain();
    }
}

/// Frames enough to hold `bytes`.
const fn frames_for(bytes: u64) -> usize {
    bytes.div_ceil(PAGE_SIZE) as usize
}

/// A controller brought up and ready to be asked for blocks.
#[derive(Debug)]
pub(crate) struct Controller {
    /// Where the registers are readable. Kept so a check can ask **how they
    /// were mapped**: a register file mapped write-back is a driver whose
    /// writes arrive when the cache feels like it, and nothing about the
    /// driver's behaviour would say so on an emulator.
    #[cfg(feature = "selftest")]
    observed_window: u64,
    registers: Option<MappedBar>,
    capabilities: Capabilities,
    admin: QueuePair,
    io: Option<QueuePair>,
    /// The next command identifier. Wrapping is fine: a completion is matched
    /// by it and only one command is outstanding at a time.
    next_id: u16,
    #[cfg(feature = "selftest")]
    published_commands: u32,
    /// Whether the one outstanding command's memory is host-owned again.
    dma: DmaState,
    /// Makes the production timeout arm deterministic for its machine proof.
    #[cfg(feature = "selftest")]
    force_timeout: bool,
    /// Stop a validated read at the last seam before command construction.
    #[cfg(feature = "selftest")]
    intercept_read_submission: bool,
    /// Invalid requests which crossed validation during the current witness.
    #[cfg(feature = "selftest")]
    intercepted_read_submissions: u32,
    /// Stop I/O queue creation before allocation or device publication.
    #[cfg(feature = "selftest")]
    intercept_io_queue_creation: bool,
    /// Queue creations which crossed namespace validation during the witness.
    #[cfg(feature = "selftest")]
    intercepted_io_queue_creations: u32,
    /// What `Identify` said, kept so a check can ask rather than infer.
    pub(crate) blocks: u64,
    /// Bytes in one logical block.
    pub(crate) block_bytes: u32,
}

#[derive(Debug)]
struct DmaState {
    flight: DmaFlightState,
    disposition: DmaDisposition,
}

#[derive(Debug, PartialEq, Eq)]
enum DmaDisposition {
    Active,
    Quiesced,
    Retained,
}

#[derive(Debug)]
struct ControllerStopped<'controller> {
    _registers: &'controller MappedBar,
    _observed: DisabledRegisters,
    dma: &'controller mut DmaState,
}

impl ControllerStopped<'_> {
    fn quiesce(self) {
        if self.dma.disposition == DmaDisposition::Active {
            self.dma.flight.close();
            self.dma.disposition = DmaDisposition::Quiesced;
        }
    }
}

impl DmaState {
    const fn idle() -> Self {
        Self {
            flight: DmaFlightState::idle(),
            disposition: DmaDisposition::Active,
        }
    }

    fn reserve(&mut self) -> bool {
        self.disposition == DmaDisposition::Active && self.flight.reserve()
    }

    fn cancel_unpublished(&mut self) -> bool {
        self.disposition == DmaDisposition::Active && self.flight.cancel_unpublished()
    }

    fn publish_with<T>(&mut self, handoff: impl FnOnce(DmaPublication<'_>) -> T) -> Option<T> {
        if self.disposition != DmaDisposition::Active {
            return None;
        }
        self.flight.publish_with(handoff)
    }

    fn complete(&mut self) -> bool {
        self.disposition == DmaDisposition::Active && self.flight.complete()
    }

    fn retain_unquiesced(&mut self) {
        if self.disposition == DmaDisposition::Active {
            self.flight.close();
            self.disposition = DmaDisposition::Retained;
        }
    }

    const fn is_settled(&self) -> bool {
        !matches!(&self.disposition, DmaDisposition::Active)
    }

    const fn is_quiesced(&self) -> bool {
        matches!(&self.disposition, DmaDisposition::Quiesced)
    }

    #[cfg(feature = "selftest")]
    const fn is_in_flight(&self) -> bool {
        self.flight.is_in_flight()
    }

    const fn must_retain(&self) -> bool {
        matches!(&self.disposition, DmaDisposition::Retained)
    }
}

/// A mapped controller observed disabled while PCI bus mastering was off.
#[derive(Debug)]
pub(crate) struct DisabledController {
    controller: Controller,
    configuration: u32,
    capacity: Capacity,
}

/// A disabled controller whose PCI function accepted bus-master admission.
///
/// Only [`DisabledController::admit_bus_master`] constructs this value, so
/// consuming it is proof that controller start cannot bypass a refused PCI
/// transition.
#[derive(Debug)]
pub(crate) struct BusMasterAdmittedController(DisabledController);

/// The two ownership-preserving results of bus-master admission.
#[derive(Debug)]
pub(crate) enum BusMasterAdmission {
    /// PCI admitted DMA and minted controller-start authority.
    Admitted(BusMasterAdmittedController),
    /// PCI refused DMA and returned retirement authority.
    Refused(BusMasterAdmissionFailure),
}

/// A refused bus-master transition with the unchanged disabled controller.
#[derive(Debug)]
pub(crate) struct BusMasterAdmissionFailure {
    reason: BusMasterAdmissionError,
    disabled: DisabledController,
}

/// Why fresh controller proof or the PCI command transition refused DMA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BusMasterAdmissionError {
    /// The disabled controller no longer retained its mapped register BAR.
    NotMapped,
    /// BAR identity, register access, or live disabled state was refused.
    Observation(wost::arch::current::pci::NvmeObservationError),
    /// The exact function's PCI command transition was refused.
    Command(pci_config::EnableError),
}

impl core::fmt::Display for BusMasterAdmissionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotMapped => core::fmt::Write::write_str(formatter, "register BAR is absent"),
            Self::Observation(reason) => write!(formatter, "controller observation: {reason}"),
            Self::Command(reason) => write!(formatter, "PCI command: {reason}"),
        }
    }
}

impl BusMasterAdmissionFailure {
    /// Why PCI refused bus-master admission.
    pub(crate) const fn reason(&self) -> BusMasterAdmissionError {
        self.reason
    }

    /// Recover the disabled controller for exact retirement.
    pub(crate) fn into_disabled(self) -> DisabledController {
        self.disabled
    }
}

/// Why a controller could not be brought up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BringUpError {
    /// The register window could not be mapped.
    NotMapped,
    /// The controller reports no NVM command set, so it has no blocks.
    NoCommandSet,
    /// The controller cannot work in this kernel's page size.
    PageSize,
    /// `CAP.MQES` describes a queue that holds nothing.
    QueueSize,
    /// No memory for the queues.
    NoMemory,
    /// The controller did not become ready, or gave up.
    NotReady,
    /// A command was refused, or never answered.
    Command(u16),
    /// A completion did not come from the command and queue just published.
    UnexpectedCompletion {
        queue: u16,
        id: u16,
        submission_head: u16,
    },
    /// A read did not fit its command, namespace or DMA owner.
    InvalidTransfer(TransferError),
    /// Identify Namespace data did not fit the supported storage contract.
    InvalidNamespace(NamespaceError),
}

impl core::fmt::Display for BringUpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotMapped => {
                core::fmt::Write::write_str(f, "the register window could not be mapped")
            }
            Self::NoCommandSet => core::fmt::Write::write_str(f, "no nvm command set"),
            Self::PageSize => {
                core::fmt::Write::write_str(f, "this kernel's page size is outside mpsmin..mpsmax")
            }
            Self::QueueSize => {
                core::fmt::Write::write_str(f, "the reported queue size holds nothing")
            }
            Self::NoMemory => core::fmt::Write::write_str(f, "no memory for the queues"),
            Self::NotReady => core::fmt::Write::write_str(f, "the controller never became ready"),
            Self::Command(status) => write!(f, "a command was refused: status {status:#x}"),
            Self::UnexpectedCompletion {
                queue,
                id,
                submission_head,
            } => write!(
                f,
                "completion names queue {queue}, command {id}, submission head {submission_head}"
            ),
            Self::InvalidTransfer(reason) => write!(f, "an invalid transfer: {reason}"),
            Self::InvalidNamespace(reason) => write!(f, "an invalid namespace: {reason}"),
        }
    }
}

/// A bring-up failure paired with its exact cleanup disposition.
#[derive(Debug)]
pub(crate) struct StartFailure {
    reason: BringUpError,
    retirement: ControllerRetirement,
}

/// Exact result of attempting to retire a controller and its mapped BAR.
#[derive(Debug)]
#[must_use = "controller retirement may return quarantine ownership"]
pub(crate) enum ControllerRetirement {
    /// Controller DMA stopped and its mapped BAR retired exactly.
    Retired,
    /// Cleanup could not be proven, so the controller remains owned.
    Quarantined(ControllerQuarantine),
}

/// Proof that an incompletely retired controller remains in kernel quarantine.
#[derive(Debug)]
#[must_use = "the controller and PCI lease must remain reachable"]
pub(crate) struct ControllerQuarantine {
    _private: (),
}

/// Result of preparing a mapped controller before bus-master admission.
#[derive(Debug)]
#[must_use = "controller preparation returns ownership in both outcomes"]
pub(crate) enum PrepareOutcome {
    /// The controller is verified disabled and ready for DMA admission.
    Prepared(DisabledController),
    /// Preparation refused with an exact ownership disposition.
    Failed(PrepareFailure),
}

/// A pre-DMA failure with an exact retirement or quarantine disposition.
#[derive(Debug)]
#[must_use = "a preparation failure may retain mapped BAR authority"]
pub(crate) enum PrepareFailure {
    /// The disabled controller mapping was retired exactly.
    Retired(BringUpError),
    /// Controller state stayed uncertain, so its complete ownership is retained.
    Quarantined(PrepareQuarantine),
}

/// A mapped controller which cannot yet be safely retired.
#[derive(Debug)]
#[must_use = "the mapped BAR and PCI lease must remain reachable"]
pub(crate) struct PrepareQuarantine {
    reason: BringUpError,
    _registers: QuarantinedRegisters,
}

#[derive(Debug)]
enum QuarantinedRegisters {
    Unverified {
        _preparation: ControllerPreparation<MappedBar>,
    },
    Disabled {
        _registers: MappedBar,
    },
    MemoryQuarantine,
}

impl PrepareFailure {
    fn quarantined(reason: BringUpError, preparation: ControllerPreparation<MappedBar>) -> Self {
        Self::Quarantined(PrepareQuarantine {
            reason,
            _registers: QuarantinedRegisters::Unverified {
                _preparation: preparation,
            },
        })
    }

    fn disabled_quarantine(reason: BringUpError, registers: MappedBar) -> Self {
        Self::Quarantined(PrepareQuarantine {
            reason,
            _registers: QuarantinedRegisters::Disabled {
                _registers: registers,
            },
        })
    }

    fn memory_quarantine(reason: BringUpError) -> Self {
        Self::Quarantined(PrepareQuarantine {
            reason,
            _registers: QuarantinedRegisters::MemoryQuarantine,
        })
    }

    /// Why preparation failed.
    pub(crate) const fn reason(&self) -> BringUpError {
        match self {
            Self::Retired(reason) => *reason,
            Self::Quarantined(quarantine) => quarantine.reason,
        }
    }

    /// Recover ownership which must remain quarantined with its PCI lease.
    pub(crate) fn into_quarantine(self) -> Option<PrepareQuarantine> {
        match self {
            Self::Retired(_) => None,
            Self::Quarantined(quarantine) => Some(quarantine),
        }
    }
}

impl StartFailure {
    /// Why bring-up failed.
    pub(crate) const fn reason(&self) -> BringUpError {
        self.reason
    }

    /// Recover the exact retirement or quarantine outcome.
    pub(crate) fn into_retirement(self) -> ControllerRetirement {
        self.retirement
    }
}

/// Bring a controller up on its mapped register window.
///
/// The specification's initialisation sequence, in order, with the two waits
/// bounded by `CAP.TO` rather than by a number chosen here.
pub(crate) fn prepare(context: &mut ThreadContext, registers: MappedBar) -> PrepareOutcome {
    #[cfg(feature = "selftest")]
    count(&STACK_PREPARATIONS);
    #[cfg(feature = "selftest")]
    let stack_measurement = PreparationStackMeasurement::begin();

    let outcome = prepare_stages(context, registers);

    #[cfg(feature = "selftest")]
    finish_stack_measurement(stack_measurement);
    outcome
}

fn prepare_stages(context: &mut ThreadContext, registers: MappedBar) -> PrepareOutcome {
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

    // `CAP` in two halves, because an `MmioRegion` makes thirty-two-bit
    // accesses. The specification allows it for this register.
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
        disabled,
        capabilities,
        admin,
        capacity,
        submission,
        completion,
        #[cfg(feature = "selftest")]
        observed_window,
    } = preparation;
    let aqa = (capacity.as_zero_based() << 16) | capacity.as_zero_based();
    let disabled = match write_disabled_register(disabled, register::AQA, aqa) {
        DisabledRegisterWrite::Written(disabled) => disabled,
        DisabledRegisterWrite::Failed(failure) => {
            return PrepareOutcome::Failed(retire_failed_preparation_access(context, failure));
        }
    };
    // The controller latches ASQ and ACQ when their high half is written.
    let disabled = match write_disabled_register(disabled, register::ASQ, submission as u32) {
        DisabledRegisterWrite::Written(disabled) => disabled,
        DisabledRegisterWrite::Failed(failure) => {
            return PrepareOutcome::Failed(retire_failed_preparation_access(context, failure));
        }
    };
    let disabled =
        match write_disabled_register(disabled, register::ASQ + 4, (submission >> 32) as u32) {
            DisabledRegisterWrite::Written(disabled) => disabled,
            DisabledRegisterWrite::Failed(failure) => {
                return PrepareOutcome::Failed(retire_failed_preparation_access(context, failure));
            }
        };
    let disabled = match write_disabled_register(disabled, register::ACQ, completion as u32) {
        DisabledRegisterWrite::Written(disabled) => disabled,
        DisabledRegisterWrite::Failed(failure) => {
            return PrepareOutcome::Failed(retire_failed_preparation_access(context, failure));
        }
    };
    let disabled =
        match write_disabled_register(disabled, register::ACQ + 4, (completion >> 32) as u32) {
            DisabledRegisterWrite::Written(disabled) => disabled,
            DisabledRegisterWrite::Failed(failure) => {
                return PrepareOutcome::Failed(retire_failed_preparation_access(context, failure));
            }
        };
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
    // `MPS` is the host page size as `log2 - 12`, and the entry sizes are the
    // specification's fixed logarithms.
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

impl Controller {
    /// Commands this controller has been given.
    ///
    /// Incremented only after the submission doorbell write succeeds, so a
    /// local refusal cannot satisfy evidence that the device was asked.
    #[cfg(feature = "selftest")]
    pub(crate) const fn commands(&self) -> u32 {
        self.published_commands
    }

    /// Where the register window is readable.
    #[cfg(feature = "selftest")]
    pub(crate) const fn window(&self) -> u64 {
        self.observed_window
    }

    /// What `CAP` said.
    pub(crate) const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Entries in each queue.
    pub(crate) const fn queue_entries(&self) -> u32 {
        self.admin.capacity.entries()
    }

    /// Whether an I/O queue pair was built.
    #[cfg(feature = "selftest")]
    pub(crate) const fn has_io_queue(&self) -> bool {
        self.io.is_some()
    }

    /// Ask the namespace how large it is and how large its blocks are.
    fn identify_namespace(&mut self, capacity: Capacity) -> Result<(), BringUpError> {
        let mut buffer =
            DmaSegment::from(mm::allocate_contiguous(1, 1).ok_or(BringUpError::NoMemory)?);
        if !buffer.zero() {
            return Err(BringUpError::NoMemory);
        }
        let address = buffer.address().ok_or(BringUpError::NoMemory)?;

        let command = Command::identify_namespace(self.take_id(), NAMESPACE, address, PAGE_SIZE)
            .ok_or(BringUpError::NotReady)?;
        self.run_admin(command, Some(&mut buffer))?;

        let mut identify = [0u8; NAMESPACE_IDENTIFY_BYTES];
        if !buffer.read_at(0, &mut identify) {
            return Err(BringUpError::NoMemory);
        }
        self.configure_namespace(&identify, capacity)
    }

    /// Validate and commit namespace geometry before I/O queues can exist.
    fn configure_namespace(
        &mut self,
        identify: &[u8; NAMESPACE_IDENTIFY_BYTES],
        capacity: Capacity,
    ) -> Result<(), BringUpError> {
        let namespace = Namespace::parse(identify).map_err(BringUpError::InvalidNamespace)?;
        self.create_io_queues(capacity)?;
        self.blocks = namespace.blocks();
        self.block_bytes = namespace.block_bytes();
        Ok(())
    }

    /// Build the one I/O queue pair, with its interrupts disabled.
    fn create_io_queues(&mut self, capacity: Capacity) -> Result<(), BringUpError> {
        #[cfg(feature = "selftest")]
        if self.intercept_io_queue_creation {
            self.intercepted_io_queue_creations += 1;
            return Err(BringUpError::NotReady);
        }

        let negotiation =
            Command::number_of_queues(self.take_id(), 1, 1).ok_or(BringUpError::QueueSize)?;
        self.run_admin(negotiation, None)?;

        let pair = QueuePair::new(IO_QUEUE, capacity).ok_or(BringUpError::NoMemory)?;
        let completion_address = pair.completion.address().ok_or(BringUpError::NoMemory)?;
        let submission_address = pair.submission.address().ok_or(BringUpError::NoMemory)?;
        // Once either command is published, the controller can retain these
        // runs. Keeping them here gives failure cleanup ownership of both.
        self.io = Some(pair);

        // Completion first because submission names it; `IEN` stays clear so
        // bootstrap remains polled (`012` §2).
        let completion = Command::create_completion_queue(
            self.take_id(),
            IO_QUEUE,
            capacity,
            completion_address,
            PAGE_SIZE,
        )
        .ok_or(BringUpError::QueueSize)?;
        self.run_admin(completion, None)?;

        // `CDW11` here is the completion queue's identifier in the high half,
        // and `PC` in bit 0.
        let submission = Command::create_submission_queue(
            self.take_id(),
            IO_QUEUE,
            IO_QUEUE,
            capacity,
            submission_address,
            PAGE_SIZE,
        )
        .ok_or(BringUpError::QueueSize)?;
        self.run_admin(submission, None)?;

        Ok(())
    }

    /// Read `blocks` logical blocks starting at `lba` into `into`.
    ///
    /// # Errors
    ///
    /// [`BringUpError::InvalidTransfer`] before submission when the command,
    /// namespace or DMA extent does not fit; otherwise the controller's
    /// command, memory and readiness errors.
    pub(crate) fn read_blocks(
        &mut self,
        lba: u64,
        blocks: u32,
        into: &mut DmaSegment,
    ) -> Result<(), BringUpError> {
        let buffer_bytes = into.len().ok_or(BringUpError::NotReady)?;
        let request = self.validate_read(lba, blocks, buffer_bytes)?;
        self.submit_read(request, into)
    }

    /// Measure one read which fills the retained DMA buffer.
    #[cfg(feature = "selftest")]
    pub(crate) fn measure_buffered_read(&mut self, into: &mut DmaSegment) -> Option<(u64, u64)> {
        let buffer_bytes = into.len()?;
        let block_bytes = u64::from(self.block_bytes);
        if block_bytes == 0 {
            return None;
        }
        let blocks = u32::try_from(
            (buffer_bytes / block_bytes)
                .min(self.blocks)
                .min(u64::from(nvme::MAX_TRANSFER_BLOCKS)),
        )
        .ok()?;
        if blocks == 0 {
            return None;
        }
        let bytes = u64::from(blocks).checked_mul(block_bytes)?;
        let started = cpu::timestamp();
        self.read_blocks(0, blocks, into).ok()?;
        let ticks = cpu::timestamp().checked_sub(started)?;
        (ticks > 0).then_some((bytes, ticks))
    }

    /// Bind raw caller arguments to this namespace before submission.
    fn validate_read(
        &self,
        lba: u64,
        blocks: u32,
        buffer_bytes: u64,
    ) -> Result<ReadTransfer, BringUpError> {
        ReadTransfer::new(
            lba,
            blocks,
            self.blocks,
            u64::from(self.block_bytes),
            buffer_bytes,
        )
        .and_then(|request| request.within_limit(PAGE_SIZE))
        .map_err(BringUpError::InvalidTransfer)
    }

    /// Build and publish a read already proved to fit every outer bound.
    fn submit_read(
        &mut self,
        request: ReadTransfer,
        into: &mut DmaSegment,
    ) -> Result<(), BringUpError> {
        #[cfg(feature = "selftest")]
        if self.intercept_read_submission {
            self.intercepted_read_submissions += 1;
            return Err(BringUpError::NotReady);
        }

        if self.io.is_none() {
            return Err(BringUpError::NotReady);
        }
        let address = into.address().ok_or(BringUpError::NotReady)?;
        let command = Command::read(self.take_id(), NAMESPACE, address, PAGE_SIZE, request)
            .ok_or(BringUpError::NotReady)?;

        self.run_on(IO_QUEUE, command, Some(into)).map(|_| ())
    }

    /// Prove malformed Identify data cannot reach I/O queue creation.
    #[cfg(feature = "selftest")]
    pub(crate) fn malformed_namespace_stops_before_io_queues(&mut self) -> bool {
        const SIZE: usize = 0;
        const FORMAT_COUNT: usize = 25;
        const FORMATTED: usize = 26;
        const PROTECTION: usize = 29;
        const FIRST_FORMAT: usize = 128;
        const METADATA: usize = FIRST_FORMAT;
        const BLOCK_SHIFT: usize = FIRST_FORMAT + 2;
        const FORMAT_CONTROL: usize = FIRST_FORMAT + 3;

        let capacity = self.admin.capacity;
        let blocks_before = self.blocks;
        let block_bytes_before = self.block_bytes;
        let commands_before = self.commands();
        let queue_before = self.has_io_queue();
        let intercepted_before = self.intercepted_io_queue_creations;

        let mut valid = [0u8; NAMESPACE_IDENTIFY_BYTES];
        valid[SIZE] = 1;
        valid[BLOCK_SHIFT] = nvme::SUPPORTED_BLOCK_BYTES.trailing_zeros() as u8;

        self.intercept_io_queue_creation = true;

        let mut empty = valid;
        empty[SIZE] = 0;
        let empty = self.configure_namespace(&empty, capacity);

        let mut formats = valid;
        formats[FORMAT_COUNT] = 16;
        let formats = self.configure_namespace(&formats, capacity);

        let mut outside = valid;
        outside[FORMATTED] = 1;
        let outside = self.configure_namespace(&outside, capacity);

        let mut reserved_selector = valid;
        reserved_selector[FORMATTED] = 0x80;
        let reserved_selector = self.configure_namespace(&reserved_selector, capacity);

        let mut reserved_format = valid;
        reserved_format[FORMAT_CONTROL] = 4;
        let reserved_format = self.configure_namespace(&reserved_format, capacity);

        let mut metadata = valid;
        metadata[METADATA] = 1;
        let metadata = self.configure_namespace(&metadata, capacity);

        let mut protection = valid;
        protection[PROTECTION] = 1;
        let protection = self.configure_namespace(&protection, capacity);

        let mut block_size = valid;
        block_size[BLOCK_SHIFT] = 32;
        let block_size = self.configure_namespace(&block_size, capacity);

        self.intercept_io_queue_creation = false;

        let refused = [
            (empty, NamespaceError::Empty),
            (formats, NamespaceError::TooManyFormats(17)),
            (
                outside,
                NamespaceError::FormatOutsideList {
                    selected: 1,
                    formats: 1,
                },
            ),
            (
                reserved_selector,
                NamespaceError::ReservedFormatBits(0x80),
            ),
            (
                reserved_format,
                NamespaceError::ReservedLbaFormatBits(4),
            ),
            (metadata, NamespaceError::Metadata(1)),
            (protection, NamespaceError::Protection(1)),
            (block_size, NamespaceError::BlockSize(32)),
        ]
        .into_iter()
        .all(|(result, expected)| {
            matches!(result, Err(BringUpError::InvalidNamespace(found)) if found == expected)
        });

        refused
            && self.blocks == blocks_before
            && self.block_bytes == block_bytes_before
            && self.commands() == commands_before
            && self.has_io_queue() == queue_before
            && self.intercepted_io_queue_creations == intercepted_before
    }

    /// Prove malformed reads stop before the private submission seam.
    #[cfg(feature = "selftest")]
    pub(crate) fn malformed_reads_stop_before_submission(&mut self, into: &mut DmaSegment) -> bool {
        let Some(buffer_bytes) = into.len() else {
            return false;
        };
        let block_bytes = u64::from(self.block_bytes);
        if block_bytes == 0 {
            return false;
        }
        let short_blocks = buffer_bytes / block_bytes + 1;
        let Ok(short_blocks) = u32::try_from(short_blocks) else {
            return false;
        };
        if short_blocks > nvme::MAX_TRANSFER_BLOCKS || u64::from(short_blocks) > self.blocks {
            return false;
        }

        let commands_before = self.commands();
        let intercepted_before = self.intercepted_read_submissions;
        self.intercept_read_submission = true;
        let refused = [
            (
                self.read_blocks(0, 0, into),
                TransferError::Empty,
            ),
            (
                self.read_blocks(0, nvme::MAX_TRANSFER_BLOCKS + 1, into),
                TransferError::TooManyBlocks,
            ),
            (
                self.read_blocks(u64::MAX, 1, into),
                TransferError::OutsideNamespace,
            ),
            (
                self.read_blocks(self.blocks, 1, into),
                TransferError::OutsideNamespace,
            ),
            (
                self.read_blocks(0, short_blocks, into),
                TransferError::BufferTooSmall,
            ),
        ]
        .into_iter()
        .all(|(result, expected)| {
            matches!(result, Err(BringUpError::InvalidTransfer(found)) if found == expected)
        });
        self.intercept_read_submission = false;

        refused
            && self.commands() == commands_before
            && self.intercepted_read_submissions == intercepted_before
    }

    /// The next command identifier.
    fn take_id(&mut self) -> u16 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// Run one command on the admin queue.
    fn run_admin(
        &mut self,
        command: Command,
        data: Option<&mut DmaSegment>,
    ) -> Result<Completion, BringUpError> {
        self.run_on(0, command, data)
    }

    /// Submit `command` to queue `which` and spin until it is answered.
    fn run_on(
        &mut self,
        which: u16,
        command: Command,
        mut data: Option<&mut DmaSegment>,
    ) -> Result<Completion, BringUpError> {
        let expected = command.id();
        let capabilities = self.capabilities;
        let submission_doorbell = nvme::doorbell(capabilities, which, nvme::Doorbell::Submission)
            .ok_or(BringUpError::NotReady)?;

        {
            let queue = self.queue_mut(which).ok_or(BringUpError::NotReady)?;
            if queue.capacity.has_room(queue.submission_head, queue.tail) != Some(true) {
                return Err(BringUpError::NotReady);
            }
        }
        if self.registers.is_none() {
            return Err(BringUpError::NotMapped);
        }
        if !self.dma.reserve() {
            return Err(BringUpError::NotReady);
        }

        // Reserve ownership first: a refused preparation cancels it, and only
        // a rung doorbell needs controller quiescence to recover.
        let prepared = (|| {
            let queue = self.queue_mut(which).ok_or(BringUpError::NotReady)?;
            let at = queue.tail as usize * nvme::SUBMISSION_ENTRY_BYTES;
            if !queue.submission.write_at(at, &command.to_bytes()) {
                return Err(BringUpError::NoMemory);
            }
            queue.tail =
                Capacity::next(queue.capacity, queue.tail).ok_or(BringUpError::NotReady)?;
            Ok(queue.tail)
        })();
        let submission_tail = match prepared {
            Ok(tail) => tail,
            Err(reason) => {
                if !self.dma.cancel_unpublished() {
                    panic!("an unpublished DMA reservation must be cancellable");
                }
                return Err(reason);
            }
        };
        let registers = &mut self.registers;
        let Some(rung) = self.dma.publish_with(|publication| {
            registers
                .as_mut()
                .ok_or(BringUpError::NotMapped)
                .and_then(|registers| {
                    write_submission(publication, registers, submission_doorbell, submission_tail)
                })
        }) else {
            panic!("a prepared DMA reservation must publish exactly once");
        };
        if let Err(error) = rung {
            return self.fail_submitted(error, data.as_deref_mut());
        }
        #[cfg(feature = "selftest")]
        {
            self.published_commands = self.published_commands.saturating_add(1);
        }

        #[cfg(feature = "selftest")]
        if HOLD_COMMAND.load(Ordering::Acquire) {
            COMMAND_HELD.store(true, Ordering::Release);
            while HOLD_COMMAND.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
            COMMAND_HELD.store(false, Ordering::Release);
        }

        // There is no tail to read; the phase tag is how a new entry is
        // recognised (`012` §3).
        let mut limit = PollLimit::after_ms(clock_ns(), COMMAND_TIMEOUT_MS, POLL_ATTEMPTS);
        #[cfg(feature = "selftest")]
        if self.force_timeout {
            limit = PollLimit::Attempts(0);
        }
        while limit.permit(clock_ns()) {
            let found = {
                let Some(queue) = self.queue_mut(which) else {
                    return self.fail_submitted(BringUpError::NotReady, data.as_deref_mut());
                };
                let at = queue.completion_head as usize * nvme::COMPLETION_ENTRY_BYTES;
                let mut bytes = [0u8; nvme::COMPLETION_ENTRY_BYTES];
                if !queue.completion.read_at(at, &mut bytes) {
                    return self.fail_submitted(BringUpError::NoMemory, data.as_deref_mut());
                }
                if Completion::from_bytes(bytes).phase == queue.phase {
                    DmaSegment::observe_device();
                    if !queue.completion.read_at(at, &mut bytes) {
                        return self.fail_submitted(BringUpError::NoMemory, data.as_deref_mut());
                    }
                    Some(Completion::from_bytes(bytes))
                } else {
                    None
                }
            };

            if let Some(entry) = found {
                let provenance = self.queue_mut(which).is_some_and(|queue| {
                    entry.phase == queue.phase
                        && entry.matches_submission(which, expected, queue.tail, queue.capacity)
                });
                if !provenance {
                    return self.fail_submitted(
                        BringUpError::UnexpectedCompletion {
                            queue: entry.submission_queue,
                            id: entry.id,
                            submission_head: entry.submission_head,
                        },
                        data.as_deref_mut(),
                    );
                }
                let Some(queue) = self.queue_mut(which) else {
                    return self.fail_submitted(BringUpError::NotReady, data.as_deref_mut());
                };
                queue.submission_head = u32::from(entry.submission_head);
                let Some(completion_head) = Capacity::next(queue.capacity, queue.completion_head)
                else {
                    return self.fail_submitted(BringUpError::NotReady, data.as_deref_mut());
                };
                if completion_head == 0 {
                    queue.phase = !queue.phase;
                }
                queue.completion_head = completion_head;
                if !self.dma.complete() {
                    panic!("a matching completion must finish one published command");
                }

                let Some(doorbell) =
                    nvme::doorbell(capabilities, which, nvme::Doorbell::Completion)
                else {
                    return self.fail_controller(BringUpError::NotReady);
                };
                let Some(head) = self.queue_mut(which).map(|queue| queue.completion_head) else {
                    return self.fail_controller(BringUpError::NotReady);
                };
                let acknowledged = self
                    .registers
                    .as_mut()
                    .ok_or(BringUpError::NotMapped)
                    .and_then(|registers| write(registers, doorbell, head));
                if let Err(error) = acknowledged {
                    return self.fail_controller(error);
                }
                if !entry.is_success() {
                    return Err(BringUpError::Command(entry.status));
                }
                return Ok(entry);
            }
            core::hint::spin_loop();
        }
        self.fail_submitted(BringUpError::NotReady, data)
    }

    /// Stop uncertain DMA before exposing an error to the caller.
    fn fail_submitted<T>(
        &mut self,
        error: BringUpError,
        data: Option<&mut DmaSegment>,
    ) -> Result<T, BringUpError> {
        self.stop_dma(data);
        Err(error)
    }

    /// Fail closed after a completion made the transfer buffer safe.
    fn fail_controller<T>(&mut self, error: BringUpError) -> Result<T, BringUpError> {
        self.stop_dma(None);
        Err(error)
    }

    /// Disable the controller and verify it idle, or withhold all DMA memory.
    fn stop_dma(&mut self, data: Option<&mut DmaSegment>) {
        if !self.dma.is_settled() {
            match stop_controller(self) {
                Ok(stopped) => stopped.quiesce(),
                Err(_) => self.dma.retain_unquiesced(),
            }
        }

        if self.dma.must_retain() {
            self.admin.retain();
            if let Some(queue) = self.io.as_mut() {
                queue.retain();
            }
            if let Some(segment) = data {
                segment.retain();
            }
        }
    }

    /// Stop DMA and retire the mapping or return the complete controller owner.
    pub(crate) fn retire(mut self, context: &mut ThreadContext) -> ControllerRetirement {
        self.stop_dma(None);
        if !self.dma.is_quiesced() {
            return quarantine_controller(self);
        }
        let Some(registers) = self.registers.take() else {
            return quarantine_controller(self);
        };
        match mem::unmap_mapped_pci_bar_exact(context, registers) {
            mem::MappedPciBarRetirement::Retired => ControllerRetirement::Retired,
            mem::MappedPciBarRetirement::Retained(registers) => {
                self.registers = Some(registers);
                quarantine_controller(self)
            }
            mem::MappedPciBarRetirement::Quarantined => quarantine_controller(self),
        }
    }

    /// Force the ordinary timeout arm and prove its cleanup before returning.
    #[cfg(feature = "selftest")]
    pub(crate) fn a_timeout_quiesces_before_dma_returns(&mut self, data: &mut DmaSegment) -> bool {
        self.force_timeout = true;
        let result = self.read_blocks(0, 1, data);
        self.force_timeout = false;

        let protected = matches!(result, Err(BringUpError::NotReady))
            && self.dma.is_quiesced()
            && !data.is_retained();
        // Record the expected failure before quiescing the deliberately
        // outstanding DMA command for the remainder of the boot.
        if self.dma.is_in_flight() {
            self.stop_dma(Some(data));
        }
        protected
    }

    /// The queue pair with this identifier.
    fn queue_mut(&mut self, which: u16) -> Option<&mut QueuePair> {
        if which == self.admin.id {
            return Some(&mut self.admin);
        }
        self.io.as_mut().filter(|queue| queue.id == which)
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.stop_dma(None);
    }
}

/// Spin until `CSTS.RDY` matches `wanted`, bounded by `CAP.TO`.
///
/// `CSTS.CFS` ends an enable wait. During disable, observed `RDY = 0` wins:
/// fatal status does not mean the controller can still reach host memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Readiness {
    Ready,
    NotReady,
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
    let fallback = (timeout_ms.saturating_mul(20_000)).min(20_000_000) as u32;
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

fn stop_controller(controller: &mut Controller) -> Result<ControllerStopped<'_>, BringUpError> {
    let Controller {
        registers,
        capabilities,
        dma,
        ..
    } = controller;
    let registers = registers.as_mut().ok_or(BringUpError::NotMapped)?;
    write(registers, register::CC, 0)?;
    wait_for_ready(registers, *capabilities, Readiness::NotReady)?;
    let cc = read(registers, register::CC)?;
    let csts = read(registers, register::CSTS)?;
    let observed = DisabledRegisters::from_bits(cc, csts).map_err(|_| BringUpError::NotReady)?;
    Ok(ControllerStopped {
        _registers: registers,
        _observed: observed,
        dma,
    })
}

fn wait_for_ready(
    registers: &mut MappedBar,
    capabilities: Capabilities,
    wanted: Readiness,
) -> Result<(), BringUpError> {
    let timeout_ms = capabilities.timeout_ms().max(500);
    let fallback = (timeout_ms.saturating_mul(20_000)).min(20_000_000) as u32;
    let mut limit = PollLimit::after_ms(clock_ns(), timeout_ms, fallback);
    while limit.permit(clock_ns()) {
        let bits = read(registers, register::CSTS)?;
        let observed = if bits & status::READY != 0 {
            Readiness::Ready
        } else {
            Readiness::NotReady
        };
        if observed == wanted {
            return Ok(());
        }
        if wanted == Readiness::Ready && bits & status::FATAL != 0 {
            return Err(BringUpError::NotReady);
        }
        core::hint::spin_loop();
    }
    Err(BringUpError::NotReady)
}

/// Read the monotonic clock only after calibration has made it meaningful.
fn clock_ns() -> Option<u64> {
    crate::time::is_calibrated().then(crate::time::now_ns)
}

fn read(registers: &MappedBar, offset: usize) -> Result<u32, BringUpError> {
    #[cfg(feature = "selftest")]
    observe_preparation_stack();
    MappedBar::read_u32(registers, offset).map_err(|_| BringUpError::NotMapped)
}

fn write(registers: &mut MappedBar, offset: usize, value: u32) -> Result<(), BringUpError> {
    #[cfg(feature = "selftest")]
    observe_preparation_stack();
    MappedBar::write_u32(registers, offset, value).map_err(|_| BringUpError::NotMapped)
}

fn write_submission(
    _publication: DmaPublication<'_>,
    registers: &mut MappedBar,
    offset: usize,
    tail: u32,
) -> Result<(), BringUpError> {
    DmaSegment::publish_to_device();
    let result = write(registers, offset, tail);
    #[cfg(feature = "selftest")]
    if result.is_ok() {
        count(&SUBMISSION_DOORBELLS);
    }
    result
}

/// Bring the controller up and say what it is.
///
/// Reports a retirement-qualified failure: the selector may keep booting
/// without storage, but may try a sibling only when teardown was exact.
pub(crate) fn start(
    context: &mut ThreadContext,
    prepared: BusMasterAdmittedController,
) -> Result<Controller, StartFailure> {
    #[cfg(feature = "selftest")]
    count(&START_ATTEMPTS);
    let BusMasterAdmittedController(DisabledController {
        mut controller,
        configuration,
        capacity,
    }) = prepared;
    let capabilities = controller.capabilities;
    let enabled = controller
        .registers
        .as_mut()
        .ok_or(BringUpError::NotMapped)
        .and_then(|registers| write(registers, register::CC, configuration))
        .and_then(|()| {
            let registers = controller
                .registers
                .as_mut()
                .ok_or(BringUpError::NotMapped)?;
            wait_for_ready(registers, capabilities, Readiness::Ready)
        });
    if let Err(reason) = enabled {
        let retirement = controller.retire(context);
        return Err(StartFailure { reason, retirement });
    }
    if let Err(reason) = controller.identify_namespace(capacity) {
        let retirement = controller.retire(context);
        return Err(StartFailure { reason, retirement });
    }

    info!(
        "nvme: up — {} entries a queue, {} blocks of {} bytes, doorbells {} apart",
        controller.queue_entries(),
        controller.blocks,
        controller.block_bytes,
        controller.capabilities().doorbell_bytes()
    );
    Ok(controller)
}

impl DisabledController {
    /// Admit DMA from a fresh observation through the retained mapped BAR.
    pub(crate) fn admit_bus_master(self, device: &mut crate::pci::Device) -> BusMasterAdmission {
        let Some(registers) = self.controller.registers.as_ref() else {
            return BusMasterAdmission::Refused(BusMasterAdmissionFailure {
                reason: BusMasterAdmissionError::NotMapped,
                disabled: self,
            });
        };
        let authority = match registers.observe_nvme_disabled() {
            Ok(authority) => authority,
            Err(reason) => {
                return BusMasterAdmission::Refused(BusMasterAdmissionFailure {
                    reason: BusMasterAdmissionError::Observation(reason),
                    disabled: self,
                });
            }
        };
        #[cfg(feature = "selftest")]
        let observed = authority.observation_for_test();
        #[cfg(feature = "selftest")]
        {
            BME_PRECEDING_CC.store(observed.0 as usize, Ordering::Release);
            BME_PRECEDING_CSTS.store(observed.1 as usize, Ordering::Release);
        }
        match device.admit_bus_master(authority) {
            Ok(()) => BusMasterAdmission::Admitted(BusMasterAdmittedController(self)),
            Err(reason) => {
                #[cfg(feature = "selftest")]
                {
                    REFUSED_BME_PRECEDING_CC.store(observed.0 as usize, Ordering::Release);
                    REFUSED_BME_PRECEDING_CSTS.store(observed.1 as usize, Ordering::Release);
                }
                BusMasterAdmission::Refused(BusMasterAdmissionFailure {
                    reason: BusMasterAdmissionError::Command(reason),
                    disabled: self,
                })
            }
        }
    }

    /// Return the mapping and all unpublished queues while DMA is still off.
    pub(crate) fn retire(self, context: &mut ThreadContext) -> ControllerRetirement {
        self.controller.retire(context)
    }
}

#[cfg(feature = "selftest")]
pub(crate) fn bus_master_followed_disable_for_test() -> bool {
    let cc = BME_PRECEDING_CC.load(Ordering::Acquire);
    let csts = BME_PRECEDING_CSTS.load(Ordering::Acquire);
    cc != usize::MAX
        && csts != usize::MAX
        && cc & configuration::ENABLE as usize == 0
        && csts & status::READY as usize == 0
}

#[cfg(feature = "selftest")]
pub(crate) fn refused_bus_master_followed_disable_for_test() -> bool {
    let cc = REFUSED_BME_PRECEDING_CC.load(Ordering::Acquire);
    let csts = REFUSED_BME_PRECEDING_CSTS.load(Ordering::Acquire);
    cc != usize::MAX
        && csts != usize::MAX
        && cc & configuration::ENABLE as usize == 0
        && csts & status::READY as usize == 0
}

#[cfg(feature = "selftest")]
pub(crate) fn activity_for_test() -> (usize, usize) {
    (
        START_ATTEMPTS.load(Ordering::Acquire),
        SUBMISSION_DOORBELLS.load(Ordering::Acquire),
    )
}

/// The one controller, kept so anything that wants blocks can ask.
///
/// A `static` rather than a value threaded through boot because the driver
/// outlives the function that built it. Runtime reads reach it only through
/// the file worker (D-200).
static CONTROLLER: ThreadLock<Option<(Controller, DmaSegment)>> = ThreadLock::new(None);

static PREPARE_QUARANTINE: ThreadLock<Option<(PrepareQuarantine, crate::pci::Device)>> =
    ThreadLock::new(None);

static CONTROLLER_QUARANTINE: ThreadLock<Option<RetainedController>> = ThreadLock::new(None);

struct RetainedController {
    _controller: Controller,
    device: Option<crate::pci::Device>,
}

#[cfg(feature = "selftest")]
static HOLD_COMMAND: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
static COMMAND_HELD: AtomicBool = AtomicBool::new(false);

/// Keep a controller and the buffer it reads into.
pub(crate) fn keep(controller: Controller, buffer: DmaSegment) {
    *CONTROLLER.lock() = Some((controller, buffer));
}

/// Keep an uncertain mapping and its inseparable PCI lease reachable.
pub(crate) fn keep_prepare_quarantine(quarantine: PrepareQuarantine, device: crate::pci::Device) {
    let mut retained = PREPARE_QUARANTINE.lock();
    assert!(
        retained.is_none(),
        "NVMe selection continued after quarantining a preparation failure"
    );
    *retained = Some((quarantine, device));
}

/// Keep an incompletely retired controller and its PCI lease reachable.
pub(crate) fn keep_controller_quarantine(
    quarantine: ControllerQuarantine,
    device: crate::pci::Device,
) {
    let ControllerQuarantine { _private: () } = quarantine;
    let mut retained = CONTROLLER_QUARANTINE.lock();
    let Some(retained) = retained.as_mut() else {
        panic!("controller quarantine token has no retained controller");
    };
    assert!(
        retained.device.is_none(),
        "NVMe selection continued after quarantining controller retirement"
    );
    retained.device = Some(device);
}

fn quarantine_controller(controller: Controller) -> ControllerRetirement {
    let mut retained = CONTROLLER_QUARANTINE.lock();
    assert!(
        retained.is_none(),
        "two NVMe controller retirements reached one quarantine slot"
    );
    *retained = Some(RetainedController {
        _controller: controller,
        device: None,
    });
    ControllerRetirement::Quarantined(ControllerQuarantine { _private: () })
}

/// Whether a retained controller exists.
pub(crate) fn has_controller() -> bool {
    CONTROLLER.lock().is_some()
}

/// Read one FAT-sized logical block into owned caller bytes.
pub(crate) fn read_sector(sector: u64, into: &mut [u8; fat32::SECTOR_BYTES]) -> Result<(), ()> {
    let mut held = CONTROLLER.lock();
    let (controller, buffer) = held.as_mut().ok_or(())?;
    if controller.block_bytes as usize != fat32::SECTOR_BYTES {
        return Err(());
    }
    controller.read_blocks(sector, 1, buffer).map_err(|_| ())?;
    buffer.read_at(0, into).then_some(()).ok_or(())
}

#[cfg(feature = "selftest")]
pub(crate) fn command_count_for_test() -> Option<u32> {
    CONTROLLER
        .lock()
        .as_ref()
        .map(|(controller, _)| controller.commands())
}

#[cfg(feature = "selftest")]
pub(crate) fn controller_facts_hold_for_test() -> bool {
    let held = CONTROLLER.lock();
    let Some((controller, _)) = held.as_ref() else {
        return false;
    };
    let capabilities = controller.capabilities();
    capabilities.supports_nvm_command_set()
        && capabilities.supports_page_size(PAGE_SIZE)
        && controller.queue_entries() <= capabilities.max_queue_entries()
        && controller.queue_entries() >= 2
        && controller.has_io_queue()
        && controller.blocks > 0
        && controller.block_bytes.is_power_of_two()
        && controller.block_bytes >= 512
}

#[cfg(feature = "selftest")]
pub(crate) fn malformed_namespace_is_bounded_for_test() -> bool {
    let mut held = CONTROLLER.lock();
    held.as_mut()
        .is_some_and(|(controller, _)| controller.malformed_namespace_stops_before_io_queues())
}

#[cfg(feature = "selftest")]
pub(crate) fn malformed_reads_are_bounded_for_test() -> bool {
    let mut held = CONTROLLER.lock();
    let Some((controller, buffer)) = held.as_mut() else {
        return false;
    };
    controller.malformed_reads_stop_before_submission(buffer)
}

#[cfg(feature = "selftest")]
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

#[cfg(feature = "selftest")]
pub(crate) fn queue_entries_for_test() -> Option<u32> {
    CONTROLLER
        .lock()
        .as_ref()
        .map(|(controller, _)| controller.queue_entries())
}

#[cfg(feature = "selftest")]
pub(crate) fn controller_window_for_test() -> Option<u64> {
    CONTROLLER
        .lock()
        .as_ref()
        .map(|(controller, _)| controller.window())
}

#[cfg(feature = "selftest")]
pub(crate) fn controller_is_unlocked_for_test() -> bool {
    CONTROLLER.try_lock().is_some()
}

#[cfg(feature = "selftest")]
pub(crate) fn timeout_quiesces_for_test() -> bool {
    let mut held = CONTROLLER.lock();
    let Some((controller, buffer)) = held.as_mut() else {
        return false;
    };
    controller.a_timeout_quiesces_before_dma_returns(buffer)
}

#[cfg(feature = "selftest")]
pub(crate) fn measure_buffered_read_for_test() -> Option<(u64, u64)> {
    let mut held = CONTROLLER.lock();
    let (controller, buffer) = held.as_mut()?;
    controller.measure_buffered_read(buffer)
}

/// Hold the next submitted command inside its preemptible polling owner.
#[cfg(feature = "selftest")]
pub(crate) fn hold_command_for_test(held: bool) {
    HOLD_COMMAND.store(held, Ordering::Release);
}

/// Whether a submitted command is currently stopped by the test hook.
#[cfg(feature = "selftest")]
pub(crate) fn command_is_held_for_test() -> bool {
    COMMAND_HELD.load(Ordering::Acquire)
}
