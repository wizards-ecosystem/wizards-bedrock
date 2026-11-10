//! The local APIC in x2APIC mode, whose register encoding needs no mapping.
//!
//! Initialization freezes each local source's vector, and EOI needs authority read from live ISR.

use core::fmt;
#[cfg(feature = "selftest")]
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use register_bits::low_u32;
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

#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
pub use selftest::{
    InitializationFailureProbe, initialization_failures_for_test,
    initialization_was_atomic_for_test, is_enabled,
    pending_audit_refuses_queued_self_interrupt_for_test, timer_entry,
};
#[cfg(feature = "selftest")]
use selftest::{record_publication_observation, record_stage_observation};

/// The initialization state published by this CPU's controller.
static APIC_STATE: AtomicU8 = AtomicU8::new(Lifecycle::Cold.bits());

/// The vector assigned to the self-directed fixed-delivery source.
static SELF_INTERRUPT_VECTOR: AtomicU8 = AtomicU8::new(0);

/// The vector assigned to the local one-shot timer.
static TIMER_VECTOR: AtomicU8 = AtomicU8::new(0);

/// The nondeliverable vector which distinguishes a real APIC source.
static SPURIOUS_VECTOR: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, Debug)]
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
        Spurious::from_bits(low_u32(Registers::read(
            &registers,
            register::SPURIOUS_INTERRUPT_VECTOR,
        )))
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

/// The APIC delivers no class at or below the task priority, and this kernel has none to express.
const ACCEPT_EVERYTHING: u64 = 0;

/// Error delivery is entry five, which this controller contract requires.
const LAST_REQUIRED_LVT_ENTRY: u8 = 5;

/// Why the local APIC could not be brought up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// The processor has no x2APIC mode, the only one implemented.
    NoExtendedMode,
    /// The enable bit did not stick, as on processors that keep it clear until reset.
    Disabled,
    /// The processor accepted the switch to x2APIC mode and then reported itself not in it.
    ModeRefused,
    /// The version or final software-enable value is not a usable APIC.
    NotResponding,
}

impl fmt::Display for ApicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyInitialized => f.write_str("the local apic is already initialized"),
            Self::InitializationInProgress => {
                f.write_str("the local apic is already being initialized")
            }
            Self::Poisoned => f.write_str("the local apic failed during initialization"),
            Self::InterruptsEnabled => f.write_str("local apic initialization lost exclusion"),
            Self::Absent => f.write_str("this processor reports no local apic"),
            Self::NoExtendedMode => {
                f.write_str("this processor has no x2apic mode, and xapic needs page tables")
            }
            Self::Disabled => f.write_str("the local apic is disabled and will not re-enable"),
            Self::ModeRefused => f.write_str("the processor would not enter x2apic mode"),
            Self::NotResponding => {
                f.write_str("the registers do not retain a usable apic configuration")
            }
        }
    }
}

impl core::error::Error for ApicError {}

/// Why the first global interrupt-enable transition was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
            Self::Unavailable => f.write_str("the local apic is unavailable"),
            Self::InterruptsEnabled => f.write_str("maskable interrupts are already enabled"),
            Self::Pending(vector) => write!(
                f,
                "local apic vector {vector:#04x} was pending before interrupt admission"
            ),
        }
    }
}

impl core::error::Error for PendingInterruptError {}

/// The vectors the local APIC delivers on its own behalf, frozen by [`init`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Vectors {
    /// Delivered when the APIC committed to a delivery with nothing to say; its low four bits must
    /// all be set.
    pub spurious: InterruptVector,
    /// Delivered when the APIC records an error.
    pub error: InterruptVector,
    /// Delivered by a fixed self-directed interrupt command.
    pub self_interrupt: InterruptVector,
    /// Delivered when the local one-shot timer expires.
    pub timer: InterruptVector,
}

/// Why a configured APIC source could not deliver.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
            Self::Unavailable => f.write_str("the local apic source is unavailable"),
            Self::InvalidConfiguration => {
                f.write_str("the local apic source configuration is invalid")
            }
        }
    }
}

impl core::error::Error for DeliveryError {}

/// Why the local timer could not change state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
            Self::Unavailable => f.write_str("the local apic timer is unavailable"),
            Self::StillRunning => f.write_str("the local apic timer is still counting"),
            Self::InvalidConfiguration => {
                f.write_str("the local apic timer configuration is invalid")
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Local {
    /// This APIC's identifier, which is how another CPU would address it.
    pub id: u32,
    pub version: u8,
    /// Whether this is the processor the firmware started.
    pub bootstrap: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InitializationBoundary {
    Stage(InitStage),
    Publication,
}

trait InitializationObserver {
    fn reached(
        &mut self,
        boundary: InitializationBoundary,
        controller: Controller<'_>,
    ) -> Result<(), ApicError>;
}

#[derive(Debug)]
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

#[derive(Debug)]
struct Configured {
    local: Local,
}

/// The local-source programming one initialization applies.
#[derive(Clone, Copy, Debug)]
struct Sources {
    vectors: Vectors,
    error: Entry,
    spurious: Spurious,
}

impl Sources {
    fn new(vectors: Vectors) -> Self {
        Self {
            vectors,
            error: Entry::fixed(vectors.error),
            spurious: Spurious::enabled(vectors.spurious),
        }
    }
}

impl fmt::Display for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "x2apic id {} version {:#x}", self.id, self.version)?;
        if self.bootstrap {
            f.write_str(" (bootstrap)")?;
        }
        Ok(())
    }
}

/// Bring the local APIC up and start accepting what it delivers.
///
/// Firmware sources are masked and kernel sources configured, and the exact software-enable value
/// is read back before source identities or readiness become public.
/// The interrupt flag stays clear, since [`interrupt::enable`] has the separate legacy-controller
/// precondition; reinitialization is refused so live source identities cannot change.
///
/// # Errors
///
/// An [`ApicError`] if a cold transaction cannot begin, its owned interrupt exclusion is lost,
/// x2APIC mode cannot be read back, or the final software-enable value does not stick exactly.
///
/// # Panics
///
/// Before changing hardware, if the spurious vector's low four bits are not all set.
pub fn init(vectors: Vectors) -> Result<Local, ApicError> {
    let controller = controller();
    controller.lifecycle().admit().map_err(admission_error)?;
    let sources = Sources::new(vectors);

    if !cpu::has_local_apic() {
        return Err(ApicError::Absent);
    }
    if !cpu::has_x2apic() {
        return Err(ApicError::NoExtendedMode);
    }
    let mut observer = Unobserved;
    init_controller(controller, sources, enter_extended_mode, &mut observer)
}

/// Audit every local-APIC IRR bank and perform the first `sti` as one door.
///
/// A compatibility-format MSI accepted into IRR before IRE went live survives until `sti`, so any
/// pending vector is refused while interrupts are excluded, and `sti` follows eight empty banks.
///
/// # Errors
///
/// A [`PendingInterruptError`] if the APIC is unavailable, interrupts were already enabled, or any
/// vector is pending.
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
        *bits = low_u32(InitializationRegisters::read(access, register));
    }
    requests
}

fn init_controller<R: InitializationRegisters>(
    controller: Controller<'_>,
    sources: Sources,
    enter_mode: impl FnOnce() -> Result<(Base, R), ApicError>,
    observer: &mut impl InitializationObserver,
) -> Result<Local, ApicError> {
    let mut initialization = Initialization::begin(controller)?;
    let configured = init_stages(&mut initialization, sources, enter_mode, observer)?;
    InitializationObserver::reached(observer, InitializationBoundary::Publication, controller)?;
    initialization.commit(configured)
}

fn init_stages<R: InitializationRegisters>(
    initialization: &mut Initialization<'_>,
    sources: Sources,
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
    let version = Version::from_bits(low_u32(InitializationRegisters::read(
        &registers,
        register::VERSION,
    )));
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
        if !Entry::from_bits(low_u32(InitializationRegisters::read(&registers, entry))).is_masked()
        {
            return Err(ApicError::NotResponding);
        }
    }
    InitializationObserver::reached(
        observer,
        InitializationBoundary::Stage(InitStage::MaskSources),
        initialization.controller,
    )?;

    initialization.enter(InitStage::ConfigureSources)?;
    InitializationRegisters::write(&registers, register::LVT_ERROR, sources.error.bits().into());
    // The error register latches on write, so this discards what accrued under the firmware.
    InitializationRegisters::write(&registers, register::ERROR_STATUS, 0);
    let _ = InitializationRegisters::read(&registers, register::ERROR_STATUS);

    InitializationRegisters::write(
        &registers,
        register::SPURIOUS_INTERRUPT_VECTOR,
        sources.spurious.bits().into(),
    );
    if InitializationRegisters::read(&registers, register::SPURIOUS_INTERRUPT_VECTOR)
        != u64::from(sources.spurious.bits())
    {
        return Err(ApicError::NotResponding);
    }

    initialization
        .controller
        .self_interrupt_vector
        .store(sources.vectors.self_interrupt.get(), Ordering::Relaxed);
    initialization
        .controller
        .timer_vector
        .store(sources.vectors.timer.get(), Ordering::Relaxed);
    initialization
        .controller
        .spurious_vector
        .store(sources.vectors.spurious.get(), Ordering::Relaxed);
    InitializationObserver::reached(
        observer,
        InitializationBoundary::Stage(InitStage::ConfigureSources),
        initialization.controller,
    )?;

    initialization.enter(InitStage::Identity)?;
    let configured = Configured {
        local: Local {
            id: low_u32(InitializationRegisters::read(&registers, register::ID)),
            version: version.number(),
            bootstrap: base.is_bootstrap(),
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
pub fn claim_eoi(frame: &TrapFrame, vector: InterruptVector) -> Option<EoiToken<'_>> {
    if interrupt::are_enabled()
        || !interrupt::is_live_frame(frame)
        || frame.vector() != vector.get()
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
pub fn end_of_interrupt(token: EoiToken<'_>) -> Result<(), EoiToken<'_>> {
    if !x86_apic::in_service::may_complete(
        interrupt_state(),
        frame_liveness(token.frame),
        u64::from(token.frame.vector()),
        token.vector,
        dismissal_ownership(token.vector),
    ) {
        return Err(token);
    }
    write(register::END_OF_INTERRUPT, 0);
    Ok(())
}

/// Deliver the configured self-interrupt source to this CPU, pending until `sti` while IF is clear.
///
/// The vector is frozen by [`init`] so callers cannot select another local source.
///
/// # Errors
///
/// [`DeliveryError::Unavailable`] before a complete initialization, or
/// [`DeliveryError::InvalidConfiguration`] if the frozen assignment cannot encode a delivery.
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
/// The programmed entry decides whether expiry is masked or one-shot. Does nothing before [`init`].
///
/// # Panics
///
/// If `count` is zero, which stops the timer; [`disarm_timer`] is how a caller asks for that.
pub fn arm_timer(count: u32) {
    assert!(count != 0, "a zero initial count stops the timer");
    write(register::INITIAL_COUNT, count.into());
}

/// Stop the countdown and prove the zero readback; an interrupt already delivered is not recalled.
///
/// # Errors
///
/// A [`TimerError`] if the timer is unavailable or does not stop.
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
/// The consumed proof requires a successful disarm, and the immediate readback closes the gap
/// between that proof and the unmasking write.
///
/// # Errors
///
/// A [`TimerError`], without writing the entry, if the source is unavailable or the countdown is
/// no longer stopped.
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
    low_u32(read(register::CURRENT_COUNT))
}

/// Errors the local APIC has recorded since this was last called, cleared as they are reported.
///
/// The register latches on write until the next one, so this writes first (SDM Vol. 3A §11.5.3).
pub fn error_status() -> u32 {
    // Zero specifically: the architecture faults on any other value here.
    write(register::ERROR_STATUS, 0);
    low_u32(read(register::ERROR_STATUS))
}

/// This processor's local APIC identifier, an I/O APIC entry's physical-mode destination.
///
/// Zero before the controller is up, also the boot processor's usual identifier, so this value
/// does not prove controller readiness.
pub fn id() -> u32 {
    low_u32(read(register::ID))
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
#[derive(Debug)]
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
        // SAFETY: `Registers` exists only after x2APIC mode reads back, and after `Ready` outside
        // initialization; values are `x86_apic` encodings, timer counts or required zero writes.
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
/// Enable, then extended mode, in two writes: one write setting both bits from a disabled APIC
/// raises a general protection fault, since the architecture has no edge straight to x2APIC.
fn enter_extended_mode() -> Result<(Base, Registers), ApicError> {
    let mut base = Base::from_bits(msr::read(base::MSR));

    if !base.is_enabled() {
        // SAFETY: `cpuid` reported `IA32_APIC_BASE`, and the only change is the enable bit; the
        // address field is written back unaltered, so nothing moves.
        unsafe { msr::write(base::MSR, base.enabled().bits()) };

        // Read back, since some processors keep a disabled APIC disabled until reset.
        base = Base::from_bits(msr::read(base::MSR));
        if !base.is_enabled() {
            return Err(ApicError::Disabled);
        }
    }

    if !base.is_x2apic() {
        // SAFETY: the same register, adding extended mode to an enabled APIC, the one transition
        // the architecture permits into it; `cpuid` reported the mode as implemented.
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
        *bits = low_u32(Registers::read(&access, register));
    }
    x86_apic::in_service::owns(&registers, vector)
}
