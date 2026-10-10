//! The local advanced programmable interrupt controller.
//!
//! The x2APIC register encoding needs no mapping. Initialization freezes each
//! local source's vector, and EOI requires authority read from live ISR state.

use core::fmt;
#[cfg(feature = "selftest")]
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use x86_apic::Register;
use x86_apic::base::{self, Base};
use x86_apic::command::Command;
use x86_apic::in_service::{DismissalOwnership, FrameLiveness};
use x86_apic::lifecycle::{
    BeginError, InitStage, InitTransaction, Interrupts, Lifecycle, ProgressError,
};
use x86_apic::local_vector::{DeliverableVector, Entry};
use x86_apic::register;
use x86_apic::spurious::Spurious;
use x86_apic::timer::Divide;
use x86_apic::vector::InterruptVector;
use x86_apic::version::Version;

use super::cpu;
use super::interrupt::{self, TrapFrame};
use super::msr;

/// The initialization state published by this CPU's controller.
static APIC_STATE: AtomicU8 = AtomicU8::new(Lifecycle::Cold.bits());

#[cfg(feature = "selftest")]
static INIT_OBSERVATIONS: AtomicU8 = AtomicU8::new(0);

#[cfg(feature = "selftest")]
static INIT_OBSERVATION_FAILED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
const PUBLICATION_OBSERVATION: u8 = 1 << InitStage::ALL.len();

#[cfg(feature = "selftest")]
const ALL_INIT_OBSERVATIONS: u8 = PUBLICATION_OBSERVATION | (PUBLICATION_OBSERVATION - 1);

/// The vector assigned to the self-directed fixed-delivery source.
static SELF_INTERRUPT_VECTOR: AtomicU8 = AtomicU8::new(0);

/// The vector assigned to the local one-shot timer.
static TIMER_VECTOR: AtomicU8 = AtomicU8::new(0);

/// The nondeliverable vector which distinguishes a real APIC source.
static SPURIOUS_VECTOR: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy)]
struct Controller<'state> {
    state: &'state AtomicU8,
    self_interrupt_vector: &'state AtomicU8,
    timer_vector: &'state AtomicU8,
    spurious_vector: &'state AtomicU8,
}

impl Controller<'_> {
    fn lifecycle(self) -> Lifecycle {
        lifecycle_at(self.state)
    }

    fn ready_registers(self) -> Option<Registers> {
        self.lifecycle().is_ready().then_some(Registers)
    }

    fn read(self, register: Register) -> u64 {
        self.ready_registers()
            .map_or(0, |registers| Registers::read(&registers, register))
    }

    fn write(self, register: Register, value: u64) {
        if let Some(registers) = self.ready_registers() {
            Registers::write(&registers, register, value);
        }
    }

    fn is_enabled(self) -> bool {
        let Some(registers) = self.ready_registers() else {
            return false;
        };
        Spurious::from_bits(Registers::read(&registers, register::SPURIOUS_INTERRUPT_VECTOR) as u32)
            .is_enabled()
    }

    fn published_vector(self, source: &AtomicU8) -> Option<InterruptVector> {
        if !self.is_enabled() {
            return None;
        }
        InterruptVector::new(source.load(Ordering::Relaxed))
    }

    fn send_self_interrupt(self) -> Result<(), DeliveryError> {
        let Some(vector) = self.published_vector(self.self_interrupt_vector) else {
            return Err(DeliveryError::Unavailable);
        };
        let Some(spurious) = self.published_vector(self.spurious_vector) else {
            return Err(DeliveryError::Unavailable);
        };
        let Some(vector) = DeliverableVector::configured(vector, spurious) else {
            return Err(DeliveryError::InvalidConfiguration);
        };
        let command = Command::self_interrupt(vector);
        Controller::write(self, register::COMMAND, command.bits().into());
        Ok(())
    }
}

fn controller() -> Controller<'static> {
    Controller {
        state: &APIC_STATE,
        self_interrupt_vector: &SELF_INTERRUPT_VECTOR,
        timer_vector: &TIMER_VECTOR,
        spurious_vector: &SPURIOUS_VECTOR,
    }
}

/// Whether one live token already owns the current EOI opportunity.
static EOI_CLAIMED: AtomicBool = AtomicBool::new(false);

/// Accept every priority class. The APIC will not deliver an interrupt whose
/// class is at or below the task priority, and this kernel has no priorities to
/// express.
const ACCEPT_EVERYTHING: u64 = 0;

/// Error delivery is entry five and is required by this controller contract.
const LAST_REQUIRED_LVT_ENTRY: u8 = 5;

/// Why the local APIC could not be brought up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApicError {
    /// This CPU's APIC was already configured.
    AlreadyInitialized,
    /// Another initialization owns the unpublished controller state.
    InitializationInProgress,
    /// A previous hardware transition failed after retry became unsafe.
    Poisoned,
    /// Initialization lost its internally owned interrupt exclusion.
    InterruptsEnabled,
    /// `cpuid` reports no local APIC on this processor.
    Absent,
    /// The processor has a local APIC but no x2APIC mode, which is the only
    /// one implemented.
    NoExtendedMode,
    /// The hardware enable bit did not stick. Some processors make clearing it
    /// permanent until reset, and this is what that looks like from software.
    Disabled,
    /// The processor accepted the switch to x2APIC mode and then reported
    /// itself not to be in it.
    ModeRefused,
    /// The version or final software-enable value is not a usable APIC.
    NotResponding,
}

impl fmt::Display for ApicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyInitialized => {
                core::fmt::Write::write_str(f, "the local apic is already initialized")
            }
            Self::InitializationInProgress => {
                core::fmt::Write::write_str(f, "the local apic is already being initialized")
            }
            Self::Poisoned => {
                core::fmt::Write::write_str(f, "the local apic failed during initialization")
            }
            Self::InterruptsEnabled => {
                core::fmt::Write::write_str(f, "local apic initialization lost exclusion")
            }
            Self::Absent => core::fmt::Write::write_str(f, "this processor reports no local apic"),
            Self::NoExtendedMode => core::fmt::Write::write_str(
                f,
                "this processor has no x2apic mode, and xapic needs page tables",
            ),
            Self::Disabled => {
                core::fmt::Write::write_str(f, "the local apic is disabled and will not re-enable")
            }
            Self::ModeRefused => {
                core::fmt::Write::write_str(f, "the processor would not enter x2apic mode")
            }
            Self::NotResponding => core::fmt::Write::write_str(
                f,
                "the registers do not retain a usable apic configuration",
            ),
        }
    }
}

impl core::error::Error for ApicError {}

/// Why the first global interrupt-enable transition was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PendingInterruptError {
    /// The local APIC has not completed initialization.
    Unavailable,
    /// The audit was attempted after maskable interrupts were already live.
    InterruptsEnabled,
    /// A request accepted before the audit remains pending in the local APIC.
    Pending(u8),
}

impl fmt::Display for PendingInterruptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => core::fmt::Write::write_str(f, "the local apic is unavailable"),
            Self::InterruptsEnabled => {
                core::fmt::Write::write_str(f, "maskable interrupts are already enabled")
            }
            Self::Pending(vector) => write!(
                f,
                "local apic vector {vector:#04x} was pending before interrupt admission"
            ),
        }
    }
}

impl core::error::Error for PendingInterruptError {}

/// The vectors the local APIC delivers on its own behalf.
///
/// The complete local-source assignment, frozen by [`init`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vectors {
    /// Delivered when the APIC has committed to delivering something and then
    /// found it had nothing to say. Its low four bits must all be set.
    pub spurious: InterruptVector,
    /// Delivered when the APIC records an error.
    pub error: InterruptVector,
    /// Delivered by a fixed self-directed interrupt command.
    pub self_interrupt: InterruptVector,
    /// Delivered when the local one-shot timer expires.
    pub timer: InterruptVector,
}

/// Why a configured APIC source could not deliver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeliveryError {
    /// The local APIC has no completed source configuration.
    Unavailable,
    /// The source assignment is not a distinct deliverable vector.
    InvalidConfiguration,
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => {
                core::fmt::Write::write_str(f, "the local apic source is unavailable")
            }
            Self::InvalidConfiguration => {
                core::fmt::Write::write_str(f, "the local apic source configuration is invalid")
            }
        }
    }
}

impl core::error::Error for DeliveryError {}

/// Why the local timer could not change state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimerError {
    /// The local APIC has no completed timer configuration.
    Unavailable,
    /// The countdown did not read back as stopped.
    StillRunning,
    /// The timer and spurious source assignments are not distinct.
    InvalidConfiguration,
}

impl fmt::Display for TimerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => {
                core::fmt::Write::write_str(f, "the local apic timer is unavailable")
            }
            Self::StillRunning => {
                core::fmt::Write::write_str(f, "the local apic timer is still counting")
            }
            Self::InvalidConfiguration => {
                core::fmt::Write::write_str(f, "the local apic timer configuration is invalid")
            }
        }
    }
}

impl core::error::Error for TimerError {}

/// Proof that the timer count read back as zero.
#[derive(Debug)]
pub struct DisarmedTimer {
    _private: (),
}

/// Authority to dismiss one exact in-service vector.
pub struct EoiToken<'frame> {
    vector: InterruptVector,
    frame: &'frame TrapFrame,
    interrupts: Option<interrupt::InterruptState>,
}

impl fmt::Debug for EoiToken<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EoiToken")
            .field("vector", &self.vector)
            .finish_non_exhaustive()
    }
}

impl Drop for EoiToken<'_> {
    fn drop(&mut self) {
        EOI_CLAIMED.store(false, Ordering::Release);
        if let Some(interrupts) = self.interrupts.take() {
            interrupts.restore();
        }
    }
}

/// What the local APIC says about itself once it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    /// This APIC's identifier, which is how another CPU would address it.
    pub id: u32,
    /// The version number the APIC reports.
    pub version: u8,
    /// Whether this is the processor the firmware started.
    pub is_bootstrap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitializationBoundary {
    Stage(InitStage),
    Publication,
}

#[cfg(feature = "selftest")]
impl InitializationBoundary {
    const ALL: [Self; 6] = [
        Self::Stage(InitStage::Mode),
        Self::Stage(InitStage::Version),
        Self::Stage(InitStage::MaskSources),
        Self::Stage(InitStage::ConfigureSources),
        Self::Stage(InitStage::Identity),
        Self::Publication,
    ];

    const fn bit(self) -> u8 {
        match self {
            Self::Stage(stage) => 1 << stage_index(stage),
            Self::Publication => PUBLICATION_OBSERVATION,
        }
    }
}

trait InitializationObserver {
    fn reached(
        &mut self,
        boundary: InitializationBoundary,
        controller: Controller<'_>,
    ) -> Result<(), ApicError>;
}

struct Unobserved;

impl InitializationObserver for Unobserved {
    fn reached(
        &mut self,
        _boundary: InitializationBoundary,
        _controller: Controller<'_>,
    ) -> Result<(), ApicError> {
        Ok(())
    }
}

trait InitializationRegisters {
    fn read(&self, register: Register) -> u64;
    fn write(&self, register: Register, value: u64);
}

struct Initialization<'state> {
    controller: Controller<'state>,
    transaction: InitTransaction,
    interrupts: Option<interrupt::InterruptState>,
}

impl<'state> Initialization<'state> {
    fn begin(controller: Controller<'state>) -> Result<Self, ApicError> {
        let interrupts = interrupt::disable();
        let transaction = match InitTransaction::begin(controller.lifecycle()) {
            Ok(transaction) => transaction,
            Err(reason) => {
                interrupts.restore();
                return Err(admission_error(reason));
            }
        };
        if let Err(state) = controller.state.compare_exchange(
            Lifecycle::Cold.bits(),
            Lifecycle::Initializing.bits(),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            interrupts.restore();
            return Err(initialization_conflict(state));
        }
        Ok(Self {
            controller,
            transaction,
            interrupts: Some(interrupts),
        })
    }

    fn enter(&mut self, stage: InitStage) -> Result<(), ApicError> {
        self.transaction
            .enter(stage, interrupt_state())
            .map_err(progress_error)?;
        #[cfg(feature = "selftest")]
        if core::ptr::eq(self.controller.state, &APIC_STATE) {
            record_stage_observation(stage, &self.transaction);
        }
        Ok(())
    }

    fn commit(mut self, configured: Configured) -> Result<Local, ApicError> {
        let ready = self
            .transaction
            .complete(interrupt_state())
            .map_err(progress_error)?;
        #[cfg(feature = "selftest")]
        if core::ptr::eq(self.controller.state, &APIC_STATE) {
            record_publication_observation(&self.transaction);
        }
        self.controller.state.store(ready.bits(), Ordering::Release);
        if let Some(interrupts) = self.interrupts.take() {
            interrupts.restore();
        }
        Ok(configured.local)
    }
}

impl Drop for Initialization<'_> {
    fn drop(&mut self) {
        if !self.transaction.lifecycle().is_ready() {
            self.controller
                .state
                .store(self.transaction.refuse().bits(), Ordering::Release);
        }
        if let Some(interrupts) = self.interrupts.take() {
            interrupts.restore();
        }
    }
}

struct Configured {
    local: Local,
}

impl fmt::Display for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "x2apic id {} version {:#x}", self.id, self.version)?;
        if self.is_bootstrap {
            core::fmt::Write::write_str(f, " (bootstrap)")?;
        }
        Ok(())
    }
}

/// Bring the local APIC up and start accepting what it delivers.
///
/// Firmware-programmable sources are masked, kernel-owned hardware sources are
/// configured, and the exact software-enable value is read back before source
/// identities or readiness become public.
///
/// This does not set the interrupt flag; [`interrupt::enable`] has the separate
/// legacy-controller precondition. Reinitialization is refused so live source
/// identities cannot change.
///
/// # Errors
///
/// Returns an [`ApicError`] if a cold transaction cannot begin, its internally
/// owned interrupt exclusion is lost, x2APIC mode cannot be read back, or the
/// final software-enable value does not stick exactly.
///
/// # Panics
///
/// Panics before changing hardware if the spurious vector's low four bits are
/// not all set.
pub fn init(vectors: Vectors) -> Result<Local, ApicError> {
    let controller = controller();
    controller.lifecycle().admit().map_err(admission_error)?;
    let error = Entry::fixed(vectors.error);
    let spurious = Spurious::enabled(vectors.spurious);

    if !cpu::has_local_apic() {
        return Err(ApicError::Absent);
    }
    if !cpu::has_x2apic() {
        return Err(ApicError::NoExtendedMode);
    }
    let mut observer = Unobserved;
    initialize_controller(
        controller,
        vectors,
        error,
        spurious,
        enter_extended_mode,
        &mut observer,
    )
}

/// Audit every local-APIC IRR bank and perform the first `sti` as one door.
///
/// A PCI function can have placed a compatibility-format MSI in IRR before
/// VT-d interrupt remapping was taken over. Once IRE is live, new such writes
/// are denied, but the already accepted request would survive until `sti`.
/// This door therefore refuses any pending vector while interrupts are still
/// excluded and enables them only after all eight banks read empty.
///
/// # Errors
///
/// Returns [`PendingInterruptError`] if the APIC is unavailable, interrupts
/// were already enabled, or any vector is pending.
pub fn enable_after_pending_audit() -> Result<(), PendingInterruptError> {
    if interrupt::are_enabled() {
        return Err(PendingInterruptError::InterruptsEnabled);
    }
    let Some(access) = ready_registers() else {
        return Err(PendingInterruptError::Unavailable);
    };
    if let Some(vector) = pending_vector(&access) {
        return Err(PendingInterruptError::Pending(vector));
    }
    interrupt::enable();
    Ok(())
}

fn pending_vector(access: &impl InitializationRegisters) -> Option<u8> {
    let requests = pending_requests(access);
    x86_apic::pending::highest(&requests)
}

fn pending_requests(access: &impl InitializationRegisters) -> [u32; 8] {
    let mut requests = [0u32; 8];
    for (bits, register) in requests.iter_mut().zip(register::INTERRUPT_REQUEST) {
        *bits = InitializationRegisters::read(access, register) as u32;
    }
    requests
}

#[cfg(feature = "selftest")]
fn is_pending(access: &impl InitializationRegisters, vector: InterruptVector) -> bool {
    let requests = pending_requests(access);
    x86_apic::pending::contains(&requests, vector)
}

/// Exercise the production pending-vector audit with a real queued self-IPI.
#[cfg(feature = "selftest")]
pub fn pending_audit_refuses_queued_self_interrupt_for_test(
    mut delivery_count: impl FnMut() -> usize,
) -> bool {
    const ATTEMPTS: usize = 1_000_000;

    if !interrupt::are_enabled() {
        return false;
    }
    let controller = controller();
    let Some(expected) = controller.published_vector(controller.self_interrupt_vector) else {
        return false;
    };
    let before = delivery_count();
    let excluded = interrupt::disable();
    excluded.commit_disabled();

    let sent = controller.send_self_interrupt().is_ok();
    let queued = (0..ATTEMPTS).any(|_| {
        core::hint::spin_loop();
        ready_registers().is_some_and(|access| is_pending(&access, expected))
    });
    // A timer may become pending after exclusion. The production door must
    // refuse whichever vector is highest, while `queued` above proves that
    // the self-IPI this witness sent is independently present in IRR.
    let refusal = matches!(
        enable_after_pending_audit(),
        Err(PendingInterruptError::Pending(_))
    );
    let stayed_excluded = !interrupt::are_enabled() && delivery_count() == before;

    if !interrupt::are_enabled() {
        interrupt::enable();
    }
    let arrived = (0..ATTEMPTS).any(|_| {
        core::hint::spin_loop();
        delivery_count() == before + 1
    });
    sent && queued && refusal && stayed_excluded && arrived && interrupt::are_enabled()
}

fn initialize_controller<R: InitializationRegisters>(
    controller: Controller<'_>,
    vectors: Vectors,
    error: Entry,
    spurious: Spurious,
    enter_mode: impl FnOnce() -> Result<(Base, R), ApicError>,
    observer: &mut impl InitializationObserver,
) -> Result<Local, ApicError> {
    let mut initialization = Initialization::begin(controller)?;
    let configured = initialize(
        &mut initialization,
        vectors,
        error,
        spurious,
        enter_mode,
        observer,
    )?;
    InitializationObserver::reached(observer, InitializationBoundary::Publication, controller)?;
    initialization.commit(configured)
}

fn initialize<R: InitializationRegisters>(
    initialization: &mut Initialization<'_>,
    vectors: Vectors,
    error: Entry,
    spurious: Spurious,
    enter_mode: impl FnOnce() -> Result<(Base, R), ApicError>,
    observer: &mut impl InitializationObserver,
) -> Result<Configured, ApicError> {
    initialization.enter(InitStage::Mode)?;
    let (base, registers) = enter_mode()?;
    InitializationObserver::reached(
        observer,
        InitializationBoundary::Stage(InitStage::Mode),
        initialization.controller,
    )?;

    initialization.enter(InitStage::Version)?;
    let version =
        Version::from_bits(InitializationRegisters::read(&registers, register::VERSION) as u32);
    if !version.is_integrated() {
        return Err(ApicError::NotResponding);
    }
    let Some(local_sources) = advertised_local_sources(version) else {
        return Err(ApicError::NotResponding);
    };
    InitializationObserver::reached(
        observer,
        InitializationBoundary::Stage(InitStage::Version),
        initialization.controller,
    )?;

    initialization.enter(InitStage::MaskSources)?;
    InitializationRegisters::write(&registers, register::TASK_PRIORITY, ACCEPT_EVERYTHING);

    for &entry in local_sources {
        InitializationRegisters::write(&registers, entry, Entry::MASKED.bits().into());
        if !Entry::from_bits(InitializationRegisters::read(&registers, entry) as u32).is_masked() {
            return Err(ApicError::NotResponding);
        }
    }
    InitializationObserver::reached(
        observer,
        InitializationBoundary::Stage(InitStage::MaskSources),
        initialization.controller,
    )?;

    initialization.enter(InitStage::ConfigureSources)?;
    InitializationRegisters::write(&registers, register::LVT_ERROR, error.bits().into());
    // The error register latches on write, so this discards whatever accrued
    // while the APIC belonged to the firmware rather than reporting it as ours.
    InitializationRegisters::write(&registers, register::ERROR_STATUS, 0);
    let _ = InitializationRegisters::read(&registers, register::ERROR_STATUS);

    InitializationRegisters::write(
        &registers,
        register::SPURIOUS_INTERRUPT_VECTOR,
        spurious.bits().into(),
    );
    if InitializationRegisters::read(&registers, register::SPURIOUS_INTERRUPT_VECTOR)
        != u64::from(spurious.bits())
    {
        return Err(ApicError::NotResponding);
    }

    initialization
        .controller
        .self_interrupt_vector
        .store(vectors.self_interrupt.get(), Ordering::Relaxed);
    initialization
        .controller
        .timer_vector
        .store(vectors.timer.get(), Ordering::Relaxed);
    initialization
        .controller
        .spurious_vector
        .store(vectors.spurious.get(), Ordering::Relaxed);
    InitializationObserver::reached(
        observer,
        InitializationBoundary::Stage(InitStage::ConfigureSources),
        initialization.controller,
    )?;

    initialization.enter(InitStage::Identity)?;
    let configured = Configured {
        local: Local {
            id: InitializationRegisters::read(&registers, register::ID) as u32,
            version: version.number(),
            is_bootstrap: base.is_bootstrap(),
        },
    };
    InitializationObserver::reached(
        observer,
        InitializationBoundary::Stage(InitStage::Identity),
        initialization.controller,
    )?;
    Ok(configured)
}

fn advertised_local_sources(version: Version) -> Option<&'static [Register]> {
    let maximum = usize::from(version.maximum_lvt_entry());
    if maximum < usize::from(LAST_REQUIRED_LVT_ENTRY) {
        return None;
    }
    register::LOCAL_VECTOR_TABLE.get(..=maximum)
}

/// Claim the exact APIC dismissal owned by the innermost live trap.
pub fn claim_eoi<'frame>(
    frame: &'frame TrapFrame,
    vector: InterruptVector,
) -> Option<EoiToken<'frame>> {
    if interrupt::are_enabled()
        || !interrupt::is_live_frame(frame)
        || frame.vector() != u64::from(vector.get())
    {
        return None;
    }
    let interrupts = interrupt::disable();
    if EOI_CLAIMED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        interrupts.restore();
        return None;
    }
    if interrupt::are_enabled() || !interrupt::is_live_frame(frame) || !owns_in_service(vector) {
        EOI_CLAIMED.store(false, Ordering::Release);
        interrupts.restore();
        return None;
    }
    Some(EoiToken {
        vector,
        frame,
        interrupts: Some(interrupts),
    })
}

/// Dismiss the exact vector represented by `token`.
///
/// A changed target returns the token unchanged and performs no write.
pub fn end_of_interrupt<'frame>(token: EoiToken<'frame>) -> Result<(), EoiToken<'frame>> {
    if !x86_apic::in_service::may_complete(
        interrupt_state(),
        frame_liveness(token.frame),
        token.frame.vector(),
        token.vector,
        dismissal_ownership(token.vector),
    ) {
        return Err(token);
    }
    write(register::END_OF_INTERRUPT, 0);
    Ok(())
}

/// Deliver the configured self-interrupt source to this CPU.
///
/// With the interrupt flag clear, delivery remains pending until `sti`. The
/// vector is frozen by [`init`] so callers cannot select another local source.
///
/// # Errors
///
/// Returns [`DeliveryError::Unavailable`] before a complete initialization,
/// or [`DeliveryError::InvalidConfiguration`] if the frozen source assignment
/// cannot encode a delivery.
pub fn send_self_interrupt() -> Result<(), DeliveryError> {
    controller().send_self_interrupt()
}

/// Set how the bus clock is divided before it drives the timer's countdown.
///
/// Does nothing before [`init`].
pub fn set_timer_divide(divide: Divide) {
    write(register::DIVIDE_CONFIGURATION, divide.bits().into());
}

/// Start the timer's countdown from `count` without touching its delivery.
///
/// The programmed entry determines whether expiry is masked or one-shot.
/// Does nothing before [`init`].
///
/// # Panics
///
/// Panics if `count` is zero, which stops the timer rather than arming it —
/// [`disarm_timer`] is how a caller says that on purpose.
pub fn arm_timer(count: u32) {
    assert!(count != 0, "a zero initial count stops the timer");
    write(register::INITIAL_COUNT, count.into());
}

/// Stop the countdown and prove the zero readback.
///
/// An interrupt already delivered is not recalled.
///
/// # Errors
///
/// Returns [`TimerError`] if the timer is unavailable or does not stop.
pub fn disarm_timer() -> Result<DisarmedTimer, TimerError> {
    let controller = controller();
    if controller
        .published_vector(controller.timer_vector)
        .is_none()
    {
        return Err(TimerError::Unavailable);
    }
    write(register::INITIAL_COUNT, 0);
    if timer_current_count() != 0 {
        return Err(TimerError::StillRunning);
    }
    Ok(DisarmedTimer { _private: () })
}

/// Make each expiry deliver the configured timer source once, and stop.
///
/// The consumed proof requires a successful disarm, and the immediate
/// readback closes the gap between that proof and the unmasking write.
///
/// # Errors
///
/// Returns [`TimerError`] without writing the entry if the source is
/// unavailable or the countdown is no longer stopped.
pub fn set_timer_one_shot(_disarmed: DisarmedTimer) -> Result<(), TimerError> {
    let controller = controller();
    let Some(vector) = controller.published_vector(controller.timer_vector) else {
        return Err(TimerError::Unavailable);
    };
    let Some(spurious) = controller.published_vector(controller.spurious_vector) else {
        return Err(TimerError::Unavailable);
    };
    let Some(vector) = DeliverableVector::configured(vector, spurious) else {
        return Err(TimerError::InvalidConfiguration);
    };
    let entry = match x86_apic::timer::one_shot_entry(timer_current_count(), vector) {
        Ok(entry) => entry,
        Err(x86_apic::timer::OneShotError::Running) => return Err(TimerError::StillRunning),
        Err(_) => return Err(TimerError::InvalidConfiguration),
    };
    write(register::LVT_TIMER, entry.bits().into());
    Ok(())
}

/// Where the timer's countdown currently stands. Zero before [`init`].
pub fn timer_current_count() -> u32 {
    read(register::CURRENT_COUNT) as u32
}

/// The timer's local vector entry, read back from the hardware rather than
/// remembered from what was written.
#[cfg(feature = "selftest")]
pub fn timer_entry() -> Entry {
    Entry::from_bits(read(register::LVT_TIMER) as u32)
}

/// Errors the local APIC has recorded since this was last called.
///
/// The register latches on write and holds that value until the next one, which
/// is the only way a single read of it can be trusted (SDM Vol. 3A §11.5.3).
/// So this writes first, and clears what it reports.
pub fn error_status() -> u32 {
    // Zero specifically: the architecture faults on any other value here.
    write(register::ERROR_STATUS, 0);
    read(register::ERROR_STATUS) as u32
}

/// This processor's local APIC identifier.
///
/// What an I/O APIC redirection entry names as an interrupt's destination in
/// physical mode. Reads as zero before the controller is up — which is also
/// the boot processor's usual identifier — so this value does not prove
/// controller readiness.
pub fn id() -> u32 {
    read(register::ID) as u32
}

/// Whether the APIC is software-enabled and therefore delivering.
#[cfg(feature = "selftest")]
pub fn is_enabled() -> bool {
    controller().is_enabled()
}

#[cfg(feature = "selftest")]
fn lifecycle() -> Lifecycle {
    controller().lifecycle()
}

fn lifecycle_at(state: &AtomicU8) -> Lifecycle {
    let Some(state) = Lifecycle::from_bits(state.load(Ordering::Acquire)) else {
        panic!("the local APIC lifecycle byte is internal");
    };
    state
}

fn admission_error(reason: BeginError) -> ApicError {
    match reason {
        BeginError::InProgress => ApicError::InitializationInProgress,
        BeginError::Ready => ApicError::AlreadyInitialized,
        BeginError::Poisoned => ApicError::Poisoned,
        _ => ApicError::Poisoned,
    }
}

fn progress_error(reason: ProgressError) -> ApicError {
    match reason {
        ProgressError::InterruptsEnabled => ApicError::InterruptsEnabled,
        ProgressError::OutOfOrder | ProgressError::Incomplete | ProgressError::Settled => {
            ApicError::Poisoned
        }
        _ => ApicError::Poisoned,
    }
}

fn interrupt_state() -> Interrupts {
    if interrupt::are_enabled() {
        Interrupts::Enabled
    } else {
        Interrupts::Excluded
    }
}

fn frame_liveness(frame: &TrapFrame) -> FrameLiveness {
    if interrupt::is_live_frame(frame) {
        FrameLiveness::Live
    } else {
        FrameLiveness::Stale
    }
}

fn dismissal_ownership(vector: InterruptVector) -> DismissalOwnership {
    if owns_in_service(vector) {
        DismissalOwnership::Owned
    } else {
        DismissalOwnership::Unowned
    }
}

#[cfg(feature = "selftest")]
fn record_stage_observation(stage: InitStage, transaction: &InitTransaction) {
    let state = lifecycle();
    let valid = state == Lifecycle::Initializing
        && transaction.lifecycle() == Lifecycle::Initializing
        && interrupt_state() == Interrupts::Excluded
        && state.admit() == Err(BeginError::InProgress);
    INIT_OBSERVATION_FAILED.fetch_or(!valid, Ordering::Relaxed);
    INIT_OBSERVATIONS.fetch_or(1 << stage_index(stage), Ordering::Relaxed);
}

#[cfg(feature = "selftest")]
fn record_publication_observation(transaction: &InitTransaction) {
    let state = lifecycle();
    let valid = state == Lifecycle::Initializing
        && transaction.lifecycle() == Lifecycle::Ready
        && interrupt_state() == Interrupts::Excluded
        && state.admit() == Err(BeginError::InProgress);
    INIT_OBSERVATION_FAILED.fetch_or(!valid, Ordering::Relaxed);
    INIT_OBSERVATIONS.fetch_or(PUBLICATION_OBSERVATION, Ordering::Relaxed);
}

#[cfg(feature = "selftest")]
const fn stage_index(stage: InitStage) -> u32 {
    match stage {
        InitStage::Mode => 0,
        InitStage::Version => 1,
        InitStage::MaskSources => 2,
        InitStage::ConfigureSources => 3,
        InitStage::Identity => 4,
    }
}

/// Whether the real boot transaction hid every partial stage from observers.
#[cfg(feature = "selftest")]
pub fn initialization_was_atomic_for_test() -> bool {
    INIT_OBSERVATIONS.load(Ordering::Acquire) == ALL_INIT_OBSERVATIONS
        && !INIT_OBSERVATION_FAILED.load(Ordering::Acquire)
        && lifecycle() == Lifecycle::Ready
}

/// Results from injecting every late APIC initialization refusal.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitializationFailureProbe {
    unavailable_and_terminal: bool,
    ignored_spurious_write_is_terminal: bool,
    delivery_waited_for_unwind: bool,
    advertised_sources_masked: bool,
    source_mask_refusals_are_terminal: bool,
}

#[cfg(feature = "selftest")]
impl InitializationFailureProbe {
    /// Whether every refused stage stayed unpublished and rejected retry.
    pub const fn unavailable_and_terminal(self) -> bool {
        self.unavailable_and_terminal
    }

    /// Whether an ignored software-enable write refused and poisoned setup.
    pub const fn ignored_spurious_write_is_terminal(self) -> bool {
        self.ignored_spurious_write_is_terminal
    }

    /// Whether each pending self-interrupt arrived only after exclusion unwound.
    pub const fn delivery_waited_for_unwind(self) -> bool {
        self.delivery_waited_for_unwind
    }

    /// Whether six-entry and CMCI-capable inventories mask exactly their sources.
    pub const fn advertised_sources_masked(self) -> bool {
        self.advertised_sources_masked
    }

    /// Whether incomplete, unknown and unverified source inventories poison setup.
    pub const fn source_mask_refusals_are_terminal(self) -> bool {
        self.source_mask_refusals_are_terminal
    }
}

#[cfg(feature = "selftest")]
#[derive(Clone, Copy)]
enum SpuriousWrite {
    Retain,
    Ignore,
}

#[cfg(feature = "selftest")]
#[derive(Clone, Copy)]
enum LvtFault {
    None,
    IgnoreMask(usize),
    ClearMaskReadback(usize),
}

#[cfg(feature = "selftest")]
struct FixtureRegisters<'probe> {
    reads: &'probe AtomicUsize,
    writes: &'probe AtomicUsize,
    spurious: &'probe AtomicUsize,
    spurious_write: SpuriousWrite,
    version: u32,
    lvt_masked: &'probe AtomicU8,
    lvt_covered: &'probe AtomicU8,
    lvt_fault: LvtFault,
}

#[cfg(feature = "selftest")]
impl InitializationRegisters for FixtureRegisters<'_> {
    fn read(&self, register: Register) -> u64 {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if register == register::VERSION {
            u64::from(self.version)
        } else if register == register::ID {
            0xA5
        } else if register == register::SPURIOUS_INTERRUPT_VECTOR {
            self.spurious.load(Ordering::Relaxed) as u64
        } else if let Some(index) = lvt_index(register) {
            let masked = self.lvt_masked.load(Ordering::Relaxed) & (1 << index) != 0;
            let corrupted = matches!(
                self.lvt_fault,
                LvtFault::ClearMaskReadback(fault) if fault == index
            );
            u64::from(Entry::from_bits(u32::from(masked && !corrupted) << 16).bits())
        } else {
            0
        }
    }

    fn write(&self, register: Register, value: u64) {
        self.writes.fetch_add(1, Ordering::Relaxed);
        if let Some(index) = lvt_index(register) {
            let bit = 1 << index;
            if Entry::from_bits(value as u32).is_masked() {
                if !matches!(self.lvt_fault, LvtFault::IgnoreMask(fault) if fault == index) {
                    self.lvt_masked.fetch_or(bit, Ordering::Relaxed);
                    self.lvt_covered.fetch_or(bit, Ordering::Relaxed);
                }
            } else {
                self.lvt_masked.fetch_and(!bit, Ordering::Relaxed);
            }
        }
        if register == register::SPURIOUS_INTERRUPT_VECTOR
            && matches!(self.spurious_write, SpuriousWrite::Retain)
        {
            self.spurious.store(value as usize, Ordering::Relaxed);
        }
    }
}

#[cfg(feature = "selftest")]
fn lvt_index(wanted: Register) -> Option<usize> {
    register::LOCAL_VECTOR_TABLE
        .iter()
        .position(|register| *register == wanted)
}

#[cfg(feature = "selftest")]
struct FailureObserver<'probe, DeliveryCount> {
    target: InitializationBoundary,
    mode_entered: &'probe AtomicBool,
    reads: &'probe AtomicUsize,
    writes: &'probe AtomicUsize,
    lvt_masked: &'probe AtomicU8,
    lvt_covered: &'probe AtomicU8,
    delivery_count: &'probe mut DeliveryCount,
    reached: u8,
    body_completed: bool,
    gates_held: bool,
    delivery_excluded: bool,
    queued_at: Option<usize>,
}

#[cfg(feature = "selftest")]
impl<DeliveryCount: FnMut() -> usize> InitializationObserver
    for FailureObserver<'_, DeliveryCount>
{
    fn reached(
        &mut self,
        boundary: InitializationBoundary,
        fixture: Controller<'_>,
    ) -> Result<(), ApicError> {
        if boundary != self.target {
            return Ok(());
        }

        let (expected_reads, expected_writes, expected_masked, expected_coverage) = match boundary {
            InitializationBoundary::Stage(InitStage::Mode) => (0, 0, 0, 0),
            InitializationBoundary::Stage(InitStage::Version) => (1, 0, 0, 0),
            InitializationBoundary::Stage(InitStage::MaskSources) => (7, 7, 0x3F, 0x3F),
            InitializationBoundary::Stage(InitStage::ConfigureSources) => (9, 10, 0x1F, 0x3F),
            InitializationBoundary::Stage(InitStage::Identity)
            | InitializationBoundary::Publication => (10, 10, 0x1F, 0x3F),
        };
        self.reached |= boundary.bit();
        self.body_completed = self.mode_entered.load(Ordering::Acquire)
            && self.reads.load(Ordering::Acquire) == expected_reads
            && self.writes.load(Ordering::Acquire) == expected_writes
            && self.lvt_masked.load(Ordering::Acquire) == expected_masked
            && self.lvt_covered.load(Ordering::Acquire) == expected_coverage;

        let delivered_before = (self.delivery_count)();
        let public_gates = !fixture.is_enabled()
            && fixture.send_self_interrupt() == Err(DeliveryError::Unavailable);
        let reentry = matches!(
            Initialization::begin(fixture),
            Err(ApicError::InitializationInProgress)
        );
        let excluded = !interrupt::are_enabled();
        let queued = controller().send_self_interrupt().is_ok();
        let stayed_pending = (self.delivery_count)() == delivered_before;
        self.gates_held = public_gates && reentry;
        self.delivery_excluded = excluded && queued && stayed_pending;
        self.queued_at = queued.then_some(delivered_before);
        Err(ApicError::NotResponding)
    }
}

/// Exercise the production APIC transaction with isolated failure state.
///
/// Register reads and writes use bounded counters because replaying controller
/// setup would disturb the live singleton. The transaction owner, stage
/// runner, public-operation gates, interrupt flag and queued self-IPI are the
/// production paths; `delivery_count` observes that real interrupt arriving.
#[cfg(feature = "selftest")]
pub fn initialization_failures_for_test(
    vectors: Vectors,
    mut delivery_count: impl FnMut() -> usize,
) -> InitializationFailureProbe {
    const DELIVERY_ATTEMPTS: usize = 1_000_000;

    let live = controller();
    let mut unavailable_and_terminal = interrupt::are_enabled()
        && live.published_vector(live.self_interrupt_vector) == Some(vectors.self_interrupt)
        && live.published_vector(live.timer_vector) == Some(vectors.timer)
        && live.published_vector(live.spurious_vector) == Some(vectors.spurious);
    let mut delivery_waited_for_unwind = unavailable_and_terminal;
    let mut reached = 0u8;

    for target in InitializationBoundary::ALL {
        let state = AtomicU8::new(Lifecycle::Cold.bits());
        let self_interrupt_vector = AtomicU8::new(vectors.self_interrupt.get());
        let timer_vector = AtomicU8::new(vectors.timer.get());
        let spurious_vector = AtomicU8::new(vectors.spurious.get());
        let fixture = Controller {
            state: &state,
            self_interrupt_vector: &self_interrupt_vector,
            timer_vector: &timer_vector,
            spurious_vector: &spurious_vector,
        };
        let mode_entered = AtomicBool::new(false);
        let reads = AtomicUsize::new(0);
        let writes = AtomicUsize::new(0);
        let spurious = AtomicUsize::new(0);
        let lvt_masked = AtomicU8::new(0);
        let lvt_covered = AtomicU8::new(0);

        let (outcome, body_completed, gates_held, delivery_excluded, queued_at, observed) = {
            let mut observer = FailureObserver {
                target,
                mode_entered: &mode_entered,
                reads: &reads,
                writes: &writes,
                lvt_masked: &lvt_masked,
                lvt_covered: &lvt_covered,
                delivery_count: &mut delivery_count,
                reached: 0,
                body_completed: false,
                gates_held: false,
                delivery_excluded: false,
                queued_at: None,
            };
            let outcome = initialize_controller(
                fixture,
                vectors,
                Entry::fixed(vectors.error),
                Spurious::enabled(vectors.spurious),
                || {
                    mode_entered.store(true, Ordering::Release);
                    Ok((
                        Base::from_bits(0),
                        FixtureRegisters {
                            reads: &reads,
                            writes: &writes,
                            spurious: &spurious,
                            spurious_write: SpuriousWrite::Retain,
                            version: 0x0005_0014,
                            lvt_masked: &lvt_masked,
                            lvt_covered: &lvt_covered,
                            lvt_fault: LvtFault::None,
                        },
                    ))
                },
                &mut observer,
            );
            (
                outcome,
                observer.body_completed,
                observer.gates_held,
                observer.delivery_excluded,
                observer.queued_at,
                observer.reached,
            )
        };
        reached |= observed;

        let poisoned = fixture.lifecycle() == Lifecycle::Poisoned;
        let unavailable = !fixture.is_enabled()
            && fixture.send_self_interrupt() == Err(DeliveryError::Unavailable);
        let retry_refused = matches!(Initialization::begin(fixture), Err(ApicError::Poisoned));
        unavailable_and_terminal &= outcome == Err(ApicError::NotResponding)
            && body_completed
            && gates_held
            && poisoned
            && unavailable
            && retry_refused;
        delivery_waited_for_unwind &= delivery_excluded;

        let arrived_after_unwind = queued_at.is_some_and(|before| {
            (0..DELIVERY_ATTEMPTS).any(|_| {
                core::hint::spin_loop();
                delivery_count() == before + 1
            })
        });
        delivery_waited_for_unwind &= interrupt::are_enabled() && arrived_after_unwind;
    }

    unavailable_and_terminal &= reached == ALL_INIT_OBSERVATIONS && live.is_enabled();
    InitializationFailureProbe {
        unavailable_and_terminal,
        ignored_spurious_write_is_terminal: ignored_spurious_write_is_terminal(vectors),
        delivery_waited_for_unwind,
        advertised_sources_masked: advertised_sources_masked(vectors),
        source_mask_refusals_are_terminal: source_mask_refusals_are_terminal(vectors),
    }
}

#[cfg(feature = "selftest")]
fn ignored_spurious_write_is_terminal(vectors: Vectors) -> bool {
    let state = AtomicU8::new(Lifecycle::Cold.bits());
    let self_interrupt_vector = AtomicU8::new(0);
    let timer_vector = AtomicU8::new(0);
    let spurious_vector = AtomicU8::new(0);
    let fixture = Controller {
        state: &state,
        self_interrupt_vector: &self_interrupt_vector,
        timer_vector: &timer_vector,
        spurious_vector: &spurious_vector,
    };
    let reads = AtomicUsize::new(0);
    let writes = AtomicUsize::new(0);
    let spurious = AtomicUsize::new(0);
    let lvt_masked = AtomicU8::new(0);
    let lvt_covered = AtomicU8::new(0);
    let entry_interrupts = interrupt::are_enabled();
    let mut observer = Unobserved;
    let outcome = initialize_controller(
        fixture,
        vectors,
        Entry::fixed(vectors.error),
        Spurious::enabled(vectors.spurious),
        || {
            Ok((
                Base::from_bits(0),
                FixtureRegisters {
                    reads: &reads,
                    writes: &writes,
                    spurious: &spurious,
                    spurious_write: SpuriousWrite::Ignore,
                    version: 0x0005_0014,
                    lvt_masked: &lvt_masked,
                    lvt_covered: &lvt_covered,
                    lvt_fault: LvtFault::None,
                },
            ))
        },
        &mut observer,
    );

    outcome == Err(ApicError::NotResponding)
        && reads.load(Ordering::Relaxed) == 9
        && writes.load(Ordering::Relaxed) == 10
        && lvt_covered.load(Ordering::Relaxed) == 0x3F
        && spurious.load(Ordering::Relaxed) == 0
        && fixture.lifecycle() == Lifecycle::Poisoned
        && self_interrupt_vector.load(Ordering::Relaxed) == 0
        && timer_vector.load(Ordering::Relaxed) == 0
        && spurious_vector.load(Ordering::Relaxed) == 0
        && fixture.send_self_interrupt() == Err(DeliveryError::Unavailable)
        && matches!(Initialization::begin(fixture), Err(ApicError::Poisoned))
        && interrupt::are_enabled() == entry_interrupts
}

#[cfg(feature = "selftest")]
fn advertised_sources_masked(vectors: Vectors) -> bool {
    let mut complete = true;
    for (maximum, expected) in [(5u8, 0x3Fu8), (6u8, 0x7Fu8)] {
        let state = AtomicU8::new(Lifecycle::Cold.bits());
        let self_interrupt_vector = AtomicU8::new(0);
        let timer_vector = AtomicU8::new(0);
        let spurious_vector = AtomicU8::new(0);
        let fixture = Controller {
            state: &state,
            self_interrupt_vector: &self_interrupt_vector,
            timer_vector: &timer_vector,
            spurious_vector: &spurious_vector,
        };
        let reads = AtomicUsize::new(0);
        let writes = AtomicUsize::new(0);
        let spurious = AtomicUsize::new(0);
        let lvt_masked = AtomicU8::new(0);
        let lvt_covered = AtomicU8::new(0);
        let mut observer = Unobserved;
        let outcome = initialize_controller(
            fixture,
            vectors,
            Entry::fixed(vectors.error),
            Spurious::enabled(vectors.spurious),
            || {
                Ok((
                    Base::from_bits(0),
                    FixtureRegisters {
                        reads: &reads,
                        writes: &writes,
                        spurious: &spurious,
                        spurious_write: SpuriousWrite::Retain,
                        version: (u32::from(maximum) << 16) | 0x14,
                        lvt_masked: &lvt_masked,
                        lvt_covered: &lvt_covered,
                        lvt_fault: LvtFault::None,
                    },
                ))
            },
            &mut observer,
        );

        complete &= outcome.is_ok()
            && fixture.lifecycle() == Lifecycle::Ready
            && lvt_covered.load(Ordering::Relaxed) == expected
            && lvt_masked.load(Ordering::Relaxed) == (expected & !(1 << 5))
            && self_interrupt_vector.load(Ordering::Relaxed) == vectors.self_interrupt.get()
            && timer_vector.load(Ordering::Relaxed) == vectors.timer.get()
            && spurious_vector.load(Ordering::Relaxed) == vectors.spurious.get();
    }
    complete
}

#[cfg(feature = "selftest")]
fn source_mask_refusals_are_terminal(vectors: Vectors) -> bool {
    let mut terminal = source_fixture_refuses(vectors, 4, LvtFault::None)
        & source_fixture_refuses(vectors, 7, LvtFault::None);
    for index in 0..register::LOCAL_VECTOR_TABLE.len() {
        terminal &= source_fixture_refuses(vectors, 6, LvtFault::IgnoreMask(index));
        terminal &= source_fixture_refuses(vectors, 6, LvtFault::ClearMaskReadback(index));
    }
    terminal
}

#[cfg(feature = "selftest")]
fn source_fixture_refuses(vectors: Vectors, maximum: u8, fault: LvtFault) -> bool {
    let state = AtomicU8::new(Lifecycle::Cold.bits());
    let self_interrupt_vector = AtomicU8::new(0);
    let timer_vector = AtomicU8::new(0);
    let spurious_vector = AtomicU8::new(0);
    let fixture = Controller {
        state: &state,
        self_interrupt_vector: &self_interrupt_vector,
        timer_vector: &timer_vector,
        spurious_vector: &spurious_vector,
    };
    let reads = AtomicUsize::new(0);
    let writes = AtomicUsize::new(0);
    let spurious = AtomicUsize::new(0);
    let lvt_masked = AtomicU8::new(0);
    let lvt_covered = AtomicU8::new(0);
    let entry_interrupts = interrupt::are_enabled();
    let mut observer = Unobserved;
    let outcome = initialize_controller(
        fixture,
        vectors,
        Entry::fixed(vectors.error),
        Spurious::enabled(vectors.spurious),
        || {
            Ok((
                Base::from_bits(0),
                FixtureRegisters {
                    reads: &reads,
                    writes: &writes,
                    spurious: &spurious,
                    spurious_write: SpuriousWrite::Retain,
                    version: (u32::from(maximum) << 16) | 0x14,
                    lvt_masked: &lvt_masked,
                    lvt_covered: &lvt_covered,
                    lvt_fault: fault,
                },
            ))
        },
        &mut observer,
    );

    outcome == Err(ApicError::NotResponding)
        && fixture.lifecycle() == Lifecycle::Poisoned
        && self_interrupt_vector.load(Ordering::Relaxed) == 0
        && timer_vector.load(Ordering::Relaxed) == 0
        && spurious_vector.load(Ordering::Relaxed) == 0
        && spurious.load(Ordering::Relaxed) == 0
        && fixture.send_self_interrupt() == Err(DeliveryError::Unavailable)
        && matches!(Initialization::begin(fixture), Err(ApicError::Poisoned))
        && interrupt::are_enabled() == entry_interrupts
}

fn initialization_conflict(bits: u8) -> ApicError {
    let Some(state) = Lifecycle::from_bits(bits) else {
        panic!("the local APIC lifecycle byte is internal");
    };
    match state {
        Lifecycle::Ready => ApicError::AlreadyInitialized,
        Lifecycle::Initializing | Lifecycle::Cold => ApicError::InitializationInProgress,
        Lifecycle::Poisoned => ApicError::Poisoned,
    }
}

/// Access to registers after x2APIC mode has been read back.
struct Registers;

impl InitializationRegisters for Registers {
    fn write(&self, register: Register, value: u64) {
        Registers::write(self, register, value);
    }

    fn read(&self, register: Register) -> u64 {
        Registers::read(self, register)
    }
}

impl Registers {
    fn write(&self, register: Register, value: u64) {
        // SAFETY: `Registers` is created only after x2APIC mode is read back,
        // and operations outside initialization obtain it only after the
        // lifecycle publishes `Ready`. Values come from `x86_apic` encodings,
        // defined timer counts, or architecturally required zero writes.
        unsafe { msr::write(register.msr(), value) };
    }

    fn read(&self, register: Register) -> u64 {
        msr::read(register.msr())
    }
}

fn ready_registers() -> Option<Registers> {
    controller().ready_registers()
}

/// Put the APIC into x2APIC mode, and report the register as it now stands.
///
/// Two writes, in this order, because the architecture's state machine has no
/// edge from disabled straight to extended mode: a single write setting both
/// bits from a disabled APIC raises a general protection fault.
fn enter_extended_mode() -> Result<(Base, Registers), ApicError> {
    let mut base = Base::from_bits(msr::read(base::MSR));

    if !base.is_enabled() {
        // SAFETY: the register named is `IA32_APIC_BASE`, which `cpuid` has
        // just reported this processor implements, and the only change to what
        // it already held is the enable bit. The address field is written back
        // unaltered, so nothing moves.
        unsafe { msr::write(base::MSR, base.enabled().bits()) };

        // Read back rather than assume: on some processors a disabled APIC
        // stays disabled until reset, and the write is simply ignored.
        base = Base::from_bits(msr::read(base::MSR));
        if !base.is_enabled() {
            return Err(ApicError::Disabled);
        }
    }

    if !base.is_x2apic() {
        // SAFETY: the same register, now with the extended-mode bit added to an
        // APIC already enabled — the one transition the architecture permits
        // into this mode. `cpuid` reported the mode as implemented.
        unsafe { msr::write(base::MSR, base.x2apic().bits()) };

        base = Base::from_bits(msr::read(base::MSR));
        if !base.is_x2apic() {
            return Err(ApicError::ModeRefused);
        }
    }

    Ok((base, Registers))
}

/// Write one register, doing nothing before complete publication.
fn write(register: Register, value: u64) {
    Controller::write(controller(), register, value);
}

/// Read one register, answering zero before complete publication.
fn read(register: Register) -> u64 {
    Controller::read(controller(), register)
}

fn owns_in_service(vector: InterruptVector) -> bool {
    let Some(access) = ready_registers() else {
        return false;
    };
    let mut registers = [0u32; 8];
    for (bits, register) in registers.iter_mut().zip(register::IN_SERVICE) {
        *bits = Registers::read(&access, register) as u32;
    }
    x86_apic::in_service::owns(&registers, vector)
}
