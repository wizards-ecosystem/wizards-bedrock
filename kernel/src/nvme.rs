use core::fmt;

use boot_record::{Fact, Record};
use nvme::{
    Capabilities, Capacity, DisabledRegisters, DmaPublication, DmaState as DmaFlightState,
    NamespaceError, PollLimit, TransferError, register, status,
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
pub(crate) use selftest::{
    queue_entries_for_test, read_block_probe_for_test, timeout_quiesces_for_test,
    unmapped_write_is_fenced_for_test,
};

// One polled command is outstanding at a time; clamped to `CAP.MQES`, which may be less.
const WANTED_ENTRIES: u32 = 8;

const IO_QUEUE: u16 = 1;

// The first namespace, the only one QEMU presents; none is enumerated.
const NAMESPACE: u32 = 1;

// Completion polls allowed before the clock is calibrated, so bring-up stays finite.
const POLL_ATTEMPTS: u32 = 5_000_000;

const COMMAND_TIMEOUT_MS: u64 = 5_000;

const MINIMUM_READY_TIMEOUT_MS: u64 = 500;

const READY_POLLS_PER_MS: u64 = 20_000;

const MAX_READY_POLLS: u64 = 20_000_000;

#[derive(Debug)]
pub(crate) struct Controller {
    // Must be mapped uncacheable; bus mastering follows only an exact disabled readback.
    registers: Option<MappedBar>,
    capabilities: Capabilities,
    admin: QueuePair,
    io: Option<QueuePair>,
    // Wrapping is safe while one command is outstanding at a time.
    next_id: u16,
    dma: DmaState,
    #[cfg(feature = "selftest")]
    force_timeout: bool,
    #[cfg(feature = "selftest")]
    leave_data_unmapped: bool,
    blocks: u64,
    block_bytes: u32,
}

#[derive(Debug)]
struct DmaState {
    flight: DmaFlightState,
    disposition: DmaDisposition,
}

#[derive(PartialEq, Eq, Debug)]
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

    const fn must_retain(&self) -> bool {
        matches!(&self.disposition, DmaDisposition::Retained)
    }
}

#[derive(Debug)]
pub(crate) struct DisabledController {
    controller: Controller,
    configuration: u32,
    capacity: Capacity,
}

#[derive(Debug)]
pub(crate) struct BusMasterAdmittedController(DisabledController);

#[derive(Debug)]
pub(crate) enum BusMasterAdmission {
    Admitted(BusMasterAdmittedController),
    Refused(BusMasterAdmissionFailure),
}

#[derive(Debug)]
pub(crate) struct BusMasterAdmissionFailure {
    reason: BusMasterAdmissionError,
    disabled: DisabledController,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BusMasterAdmissionError {
    Unisolated,
    QueuesUnreachable,
    NotMapped,
    Observation(wost::arch::current::pci::NvmeObservationError),
    Command(pci_config::EnableError),
}

impl fmt::Display for BusMasterAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unisolated => formatter.write_str("no DMA domain isolates the function"),
            Self::QueuesUnreachable => {
                formatter.write_str("the admin queues could not enter the DMA domain")
            }
            Self::NotMapped => formatter.write_str("register BAR is absent"),
            Self::Observation(reason) => write!(formatter, "controller observation: {reason}"),
            Self::Command(reason) => write!(formatter, "PCI command: {reason}"),
        }
    }
}

impl core::error::Error for BusMasterAdmissionError {}

impl BusMasterAdmissionFailure {
    pub(crate) const fn reason(&self) -> BusMasterAdmissionError {
        self.reason
    }

    pub(crate) fn into_disabled(self) -> DisabledController {
        self.disabled
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BringUpError {
    NotMapped,
    NoCommandSet,
    PageSize,
    QueueSize,
    NoMemory,
    NotReady,
    Command(u16),
    Timeout,
    UnexpectedCompletion {
        queue: u16,
        id: u16,
        submission_head: u16,
    },
    InvalidTransfer(TransferError),
    InvalidNamespace(NamespaceError),
    Isolation,
}

impl fmt::Display for BringUpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMapped => formatter.write_str("the register window could not be mapped"),
            Self::NoCommandSet => formatter.write_str("no nvm command set"),
            Self::PageSize => {
                formatter.write_str("this kernel's page size is outside mpsmin..mpsmax")
            }
            Self::QueueSize => formatter.write_str("the reported queue size holds nothing"),
            Self::NoMemory => formatter.write_str("no memory for the queues"),
            Self::NotReady => formatter.write_str("the controller never became ready"),
            Self::Command(status) => write!(formatter, "a command was refused: status {status:#x}"),
            Self::Timeout => write!(
                formatter,
                "a command went unanswered for {COMMAND_TIMEOUT_MS} ms"
            ),
            Self::UnexpectedCompletion {
                queue,
                id,
                submission_head,
            } => write!(
                formatter,
                "completion names queue {queue}, command {id}, submission head {submission_head}"
            ),
            Self::InvalidTransfer(reason) => write!(formatter, "an invalid transfer: {reason}"),
            Self::InvalidNamespace(reason) => write!(formatter, "an invalid namespace: {reason}"),
            Self::Isolation => formatter.write_str("no dma domain admits the buffer"),
        }
    }
}

impl core::error::Error for BringUpError {}

#[derive(Debug)]
pub(crate) struct StartFailure {
    reason: BringUpError,
    retirement: ControllerRetirement,
}

#[derive(Debug)]
#[must_use = "controller retirement may return quarantine ownership"]
pub(crate) enum ControllerRetirement {
    Retired,
    Quarantined(ControllerQuarantine),
}

#[derive(Debug)]
#[must_use = "the controller and PCI lease must remain reachable"]
pub(crate) struct ControllerQuarantine {
    _private: (),
}

#[derive(Debug)]
#[must_use = "controller preparation returns ownership in both outcomes"]
#[cfg_attr(
    feature = "selftest",
    expect(
        clippy::large_enum_variant,
        reason = "the self-test hooks widen the controller, which moves by value without a heap"
    )
)]
pub(crate) enum PrepareOutcome {
    Prepared(DisabledController),
    Failed(PrepareFailure),
}

#[derive(Debug)]
#[must_use = "a preparation failure may retain mapped BAR authority"]
pub(crate) enum PrepareFailure {
    Retired(BringUpError),
    Quarantined(PrepareQuarantine),
}

#[derive(Debug)]
#[must_use = "the mapped BAR and PCI lease must remain reachable"]
pub(crate) struct PrepareQuarantine {
    reason: BringUpError,
    _registers: QuarantinedRegisters,
}

#[derive(Debug)]
enum QuarantinedRegisters {
    Mapped { _registers: MappedBar },
    MemoryQuarantine,
}

impl PrepareFailure {
    fn quarantined(reason: BringUpError, registers: MappedBar) -> Self {
        Self::Quarantined(PrepareQuarantine {
            reason,
            _registers: QuarantinedRegisters::Mapped {
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

    pub(crate) const fn reason(&self) -> BringUpError {
        match self {
            Self::Retired(reason) => *reason,
            Self::Quarantined(quarantine) => quarantine.reason,
        }
    }

    pub(crate) fn into_quarantine(self) -> Option<PrepareQuarantine> {
        match self {
            Self::Retired(_) => None,
            Self::Quarantined(quarantine) => Some(quarantine),
        }
    }
}

impl StartFailure {
    pub(crate) const fn reason(&self) -> BringUpError {
        self.reason
    }

    pub(crate) fn into_retirement(self) -> ControllerRetirement {
        self.retirement
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.stop_dma(None);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    let observed = disable(registers, *capabilities)?;
    Ok(ControllerStopped {
        _registers: registers,
        _observed: observed,
        dma,
    })
}

fn disable(
    registers: &mut MappedBar,
    capabilities: Capabilities,
) -> Result<DisabledRegisters, BringUpError> {
    write(registers, register::CC, 0)?;
    wait_for_ready(registers, capabilities, Readiness::NotReady)?;
    let cc = read(registers, register::CC)?;
    let csts = read(registers, register::CSTS)?;
    DisabledRegisters::from_bits(cc, csts).map_err(|_| BringUpError::NotReady)
}

// `CFS` ends only an enable wait; a disable waits for `RDY = 0` whatever `CFS` says.
fn wait_for_ready(
    registers: &MappedBar,
    capabilities: Capabilities,
    wanted: Readiness,
) -> Result<(), BringUpError> {
    let timeout_ms = capabilities.timeout_ms().max(MINIMUM_READY_TIMEOUT_MS);
    let fallback = fallback_polls(timeout_ms);
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

// Register polls allowed before the clock is calibrated; the cap keeps the cast exact.
fn fallback_polls(timeout_ms: u64) -> u32 {
    (timeout_ms.saturating_mul(READY_POLLS_PER_MS)).min(MAX_READY_POLLS) as u32
}

pub(crate) fn clock_ns() -> Option<u64> {
    crate::time::is_calibrated().then(crate::time::now_ns)
}

fn read(registers: &MappedBar, offset: usize) -> Result<u32, BringUpError> {
    registers
        .read_u32(offset)
        .map_err(|_| BringUpError::NotMapped)
}

fn write(registers: &mut MappedBar, offset: usize, value: u32) -> Result<(), BringUpError> {
    registers
        .write_u32(offset, value)
        .map_err(|_| BringUpError::NotMapped)
}

fn write_submission(
    _publication: DmaPublication<'_>,
    registers: &mut MappedBar,
    offset: usize,
    tail: u32,
) -> Result<(), BringUpError> {
    write(registers, offset, tail)
}

// The selector may try a sibling only when the failure's retirement was exact.
pub(crate) fn start(
    context: &mut ThreadContext,
    device: &mut crate::pci::Device,
    prepared: BusMasterAdmittedController,
) -> Result<Controller, StartFailure> {
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
    info!("{}", Record::Fact(Fact::NvmeUp));
    Ok(controller)
}

impl DisabledController {
    // Separate so each refusal builds a small error: debug builds give temporaries a slot each.
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
        crate::iommu::admit_upstream_bus_master(&domain, &interrupts)
            .map_err(BusMasterAdmissionError::Command)?;
        device
            .admit_bus_master(authority, domain, interrupts)
            .map_err(BusMasterAdmissionError::Command)
    }

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

    pub(crate) fn retire(
        self,
        device: &mut crate::pci::Device,
        context: &mut ThreadContext,
    ) -> ControllerRetirement {
        self.controller.retire(device, context)
    }
}

static CONTROLLER: ThreadLock<Option<(Controller, DmaSegment)>> = ThreadLock::new(None);

static UNCOMMITTED: ThreadLock<Option<(Controller, DmaSegment)>> = ThreadLock::new(None);

static PREPARE_QUARANTINE: ThreadLock<Option<(PrepareQuarantine, crate::pci::Device)>> =
    ThreadLock::new(None);

static CONTROLLER_QUARANTINE: ThreadLock<Option<RetainedController>> = ThreadLock::new(None);

static IOMMU_QUARANTINE: ThreadLock<Option<(DisabledController, crate::pci::Device)>> =
    ThreadLock::new(None);

#[derive(Debug)]
struct RetainedController {
    _controller: Controller,
    device: Option<crate::pci::Device>,
}

pub(crate) fn keep(controller: Controller, buffer: DmaSegment) {
    *CONTROLLER.lock() = Some((controller, buffer));
}

pub(crate) fn keep_uncommitted(controller: Controller, buffer: DmaSegment) {
    let mut retained = UNCOMMITTED.lock();
    assert!(
        retained.is_none(),
        "two NVMe controllers were retained after refused PCI commits"
    );
    *retained = Some((controller, buffer));
}

pub(crate) fn keep_prepare_quarantine(quarantine: PrepareQuarantine, device: crate::pci::Device) {
    let mut retained = PREPARE_QUARANTINE.lock();
    assert!(
        retained.is_none(),
        "NVMe selection continued after quarantining a preparation failure"
    );
    *retained = Some((quarantine, device));
}

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

pub(crate) fn namespace_blocks() -> Option<u64> {
    CONTROLLER
        .lock()
        .as_ref()
        .map(|(controller, _)| controller.blocks)
}

pub(crate) fn read_sector(sector: u64, into: &mut [u8; fat32::SECTOR_BYTES]) -> Result<(), ()> {
    let mut held = CONTROLLER.lock();
    let (controller, buffer) = held.as_mut().ok_or(())?;
    if controller.block_bytes as usize != fat32::SECTOR_BYTES {
        return Err(());
    }
    controller.read_blocks(sector, 1, buffer).map_err(|_| ())?;
    buffer.read_at(0, into).then_some(()).ok_or(())
}
