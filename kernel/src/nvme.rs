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
use core::sync::atomic::Ordering;

use nvme::{
    Capabilities, Capacity, ControllerPreparation, DisabledRegisters, DmaPublication,
    DmaState as DmaFlightState, NamespaceError, PollLimit, TransferError, register, status,
};
use wost::arch::current::pci::MappedBar;
use wost::mm::DmaSegment;
use wost::sync::ThreadLock;

use crate::info;
use crate::thread::ThreadContext;

mod io;
mod preparation;
mod queues;
#[cfg(feature = "selftest")]
mod selftest;

pub(crate) use preparation::prepare;
use queues::QueuePair;
#[cfg(feature = "selftest")]
use selftest::{
    BME_PRECEDING_CC, BME_PRECEDING_CSTS, REFUSED_BME_PRECEDING_CC, REFUSED_BME_PRECEDING_CSTS,
    START_ATTEMPTS, SUBMISSION_DOORBELLS, count, observe_candidate_stack,
};
#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    CandidateStackMeasurement, activity_for_test, bus_master_followed_disable_for_test,
    candidate_stack_usage_for_test, command_count_for_test, command_is_held_for_test,
    controller_facts_hold_for_test, controller_is_unlocked_for_test, controller_window_for_test,
    domain_proof_follows_translation_for_test, finish_candidate_stack, hold_command_for_test,
    hold_requested_for_test, isolation_is_active_for_test, malformed_namespace_is_bounded_for_test,
    malformed_reads_are_bounded_for_test, measure_buffered_read_for_test, queue_entries_for_test,
    reachable_frames_for_test, read_block_probe_for_test,
    refused_bus_master_followed_disable_for_test, timeout_quiesces_for_test,
    unmapped_write_is_fenced_for_test,
};

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
    /// Publish the next transfer buffer without mapping it, so a check can
    /// watch the domain refuse the device.
    #[cfg(feature = "selftest")]
    leave_data_unmapped: bool,
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
    /// PCI admitted DMA behind the installed domain and minted controller-start
    /// authority.
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
    /// No DMA-remapping domain is installed for the function (D-357).
    Unisolated,
    /// The admin queues could not be made reachable through the domain.
    QueuesUnreachable,
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
            Self::Unisolated => {
                core::fmt::Write::write_str(formatter, "no DMA domain isolates the function")
            }
            Self::QueuesUnreachable => core::fmt::Write::write_str(
                formatter,
                "the admin queues could not enter the DMA domain",
            ),
            Self::NotMapped => core::fmt::Write::write_str(formatter, "register BAR is absent"),
            Self::Observation(reason) => write!(formatter, "controller observation: {reason}"),
            Self::Command(reason) => write!(formatter, "PCI command: {reason}"),
        }
    }
}

impl core::error::Error for BusMasterAdmissionError {}

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
    /// No DMA domain is installed, or the one installed refused a queue or
    /// transfer buffer, so nothing may be published (D-357).
    Isolation,
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
            Self::Isolation => core::fmt::Write::write_str(f, "no dma domain admits the buffer"),
        }
    }
}

impl core::error::Error for BringUpError {}

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
#[cfg_attr(
    feature = "selftest",
    expect(
        clippy::large_enum_variant,
        reason = "self-test observations enlarge the inline owner; candidate stack use is machine-checked (D-287)"
    )
)]
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
    observe_candidate_stack();
    MappedBar::read_u32(registers, offset).map_err(|_| BringUpError::NotMapped)
}

fn write(registers: &mut MappedBar, offset: usize, value: u32) -> Result<(), BringUpError> {
    #[cfg(feature = "selftest")]
    observe_candidate_stack();
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
    device: &mut crate::pci::Device,
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
        let retirement = controller.retire(device, context);
        return Err(StartFailure { reason, retirement });
    }
    if let Err(reason) = controller.identify_namespace(capacity) {
        let retirement = controller.retire(device, context);
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
    /// The admission steps: the installed domain reaches the admin queues and
    /// mints its proof, then the fresh disabled observation reaches PCI.
    ///
    /// Separate from [`admit_bus_master`](Self::admit_bus_master) so that each
    /// refusal builds a small value rather than a whole admission result: the
    /// debug build gives every such temporary its own stack slot (D-287).
    fn admit(&mut self, device: &mut crate::pci::Device) -> Result<(), BusMasterAdmissionError> {
        let mut installed = crate::iommu::installed();
        let Some(isolation) = installed.as_mut() else {
            return Err(BusMasterAdmissionError::Unisolated);
        };
        if !self.controller.admin.map_into(isolation) {
            return Err(BusMasterAdmissionError::QueuesUnreachable);
        }
        let domain = isolation
            .domain()
            .ok_or(BusMasterAdmissionError::Unisolated)?;
        let interrupts = isolation
            .interrupt_isolation()
            .ok_or(BusMasterAdmissionError::Unisolated)?;
        let Some(registers) = self.controller.registers.as_ref() else {
            return Err(BusMasterAdmissionError::NotMapped);
        };
        let authority = match registers.observe_nvme_disabled() {
            Ok(authority) => authority,
            Err(reason) => return Err(BusMasterAdmissionError::Observation(reason)),
        };
        #[cfg(feature = "selftest")]
        let observed = authority.observation_for_test();
        #[cfg(feature = "selftest")]
        {
            BME_PRECEDING_CC.store(observed.0 as usize, Ordering::Release);
            BME_PRECEDING_CSTS.store(observed.1 as usize, Ordering::Release);
        }
        match device.admit_bus_master(authority, domain, interrupts) {
            Ok(()) => Ok(()),
            Err(reason) => {
                #[cfg(feature = "selftest")]
                {
                    REFUSED_BME_PRECEDING_CC.store(observed.0 as usize, Ordering::Release);
                    REFUSED_BME_PRECEDING_CSTS.store(observed.1 as usize, Ordering::Release);
                }
                Err(BusMasterAdmissionError::Command(reason))
            }
        }
    }

    /// Admit DMA from a fresh observation through the retained mapped BAR,
    /// behind the installed domain, which must already reach the admin queues.
    pub(crate) fn admit_bus_master(
        mut self,
        device: &mut crate::pci::Device,
    ) -> BusMasterAdmission {
        match self.admit(device) {
            Ok(()) => BusMasterAdmission::Admitted(BusMasterAdmittedController(self)),
            Err(reason) => BusMasterAdmission::Refused(BusMasterAdmissionFailure {
                reason,
                disabled: self,
            }),
        }
    }

    /// Return the mapping and all unpublished queues while DMA is still off.
    pub(crate) fn retire(
        self,
        device: &mut crate::pci::Device,
        context: &mut ThreadContext,
    ) -> ControllerRetirement {
        self.controller.retire(device, context)
    }
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

/// A disabled controller kept with its PCI lease when unit-wide VT-d
/// ownership becomes uncertain before DMA admission.
static IOMMU_QUARANTINE: ThreadLock<Option<(DisabledController, crate::pci::Device)>> =
    ThreadLock::new(None);

struct RetainedController {
    _controller: Controller,
    device: Option<crate::pci::Device>,
}

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

/// Keep a prepared controller and its selected-function owner beside an
/// uncertain unit takeover.
pub(crate) fn keep_iommu_quarantine(controller: DisabledController, device: crate::pci::Device) {
    let mut retained = IOMMU_QUARANTINE.lock();
    assert!(
        retained.is_none(),
        "NVMe selection continued after quarantining VT-d takeover"
    );
    *retained = Some((controller, device));
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
