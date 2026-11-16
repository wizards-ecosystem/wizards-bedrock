use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use register_bits::low_u32;
use x86_apic::Register;
use x86_apic::base::{self, Base};
use x86_apic::in_service::{DismissalOwnership, FrameLiveness};
use x86_apic::lifecycle::{Interrupts, Lifecycle};
use x86_apic::local_vector::{DeliverableVector, Entry};
use x86_apic::register;
use x86_apic::spurious::Spurious;
use x86_apic::timer::Divide;
use x86_apic::vector::InterruptVector;
use x86_apic::version::Version;

use super::cpu;
use super::interrupt::{self, TrapFrame};
use super::msr;

static STATE: AtomicU8 = AtomicU8::new(Lifecycle::Cold.bits());

static TIMER_VECTOR: AtomicU8 = AtomicU8::new(0);

static SPURIOUS_VECTOR: AtomicU8 = AtomicU8::new(0);

static EOI_CLAIMED: AtomicBool = AtomicBool::new(false);

const ACCEPT_EVERYTHING: u64 = 0;

// Error delivery is entry five, the last one this driver programs.
const LAST_REQUIRED_LVT_ENTRY: u8 = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum InitError {
    AlreadyInitialized,
    InitializationInProgress,
    Poisoned,
    Absent,
    NoExtendedMode,
    Disabled,
    ModeRefused,
    NotResponding,
}

impl fmt::Display for InitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AlreadyInitialized => "the local apic is already initialized",
            Self::InitializationInProgress => "the local apic is already being initialized",
            Self::Poisoned => "the local apic failed during initialization",
            Self::Absent => "this processor reports no local apic",
            Self::NoExtendedMode => {
                "this processor has no x2apic mode, and xapic needs page tables"
            }
            Self::Disabled => "the local apic is disabled and will not re-enable",
            Self::ModeRefused => "the processor would not enter x2apic mode",
            Self::NotResponding => "the registers do not retain a usable apic configuration",
        })
    }
}

impl core::error::Error for InitError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum PendingInterruptError {
    Unavailable,
    InterruptsEnabled,
    Pending(u8),
}

impl fmt::Display for PendingInterruptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("the local apic is unavailable"),
            Self::InterruptsEnabled => {
                formatter.write_str("maskable interrupts are already enabled")
            }
            Self::Pending(vector) => write!(
                formatter,
                "local apic vector {vector:#04x} was pending before interrupt admission"
            ),
        }
    }
}

impl core::error::Error for PendingInterruptError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Vectors {
    pub spurious: InterruptVector,
    pub error: InterruptVector,
    pub timer: InterruptVector,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum TimerError {
    Unavailable,
    StillRunning,
    InvalidConfiguration,
}

impl fmt::Display for TimerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "the local apic timer is unavailable",
            Self::StillRunning => "the local apic timer is still counting",
            Self::InvalidConfiguration => "the local apic timer configuration is invalid",
        })
    }
}

impl core::error::Error for TimerError {}

#[derive(Debug)]
pub struct DisarmedTimer {
    _private: (),
}

pub struct EoiToken<'frame> {
    vector: InterruptVector,
    frame: &'frame TrapFrame,
    interrupts: Option<interrupt::InterruptState>,
}

impl fmt::Debug for EoiToken<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EoiToken")
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Local {
    pub id: u32,
    pub version: u8,
    pub bootstrap: bool,
}

impl fmt::Display for Local {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "x2apic id {} version {:#x}",
            self.id, self.version
        )?;
        if self.bootstrap {
            formatter.write_str(" (bootstrap)")?;
        }
        Ok(())
    }
}

// Interrupts stay clear: `interrupt::enable` has the separate legacy-controller precondition.
pub fn init(vectors: Vectors) -> Result<Local, InitError> {
    let error = Entry::fixed(vectors.error);
    let spurious = Spurious::enabled(vectors.spurious);

    if !cpu::has_local_apic() {
        return Err(InitError::Absent);
    }
    if !cpu::has_x2apic() {
        return Err(InitError::NoExtendedMode);
    }
    let interrupts = interrupt::disable();
    if let Err(state) = STATE.compare_exchange(
        Lifecycle::Cold.bits(),
        Lifecycle::Initializing.bits(),
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        interrupts.restore();
        return Err(initialization_conflict(state));
    }
    let outcome = configure(vectors, error, spurious);
    let settled = if outcome.is_ok() {
        Lifecycle::Ready
    } else {
        Lifecycle::Poisoned
    };
    STATE.store(settled.bits(), Ordering::Release);
    interrupts.restore();
    outcome
}

fn configure(vectors: Vectors, error: Entry, spurious: Spurious) -> Result<Local, InitError> {
    let (base, registers) = enter_extended_mode()?;

    let version = Version::from_bits(low_u32(registers.read(register::VERSION)));
    if !version.is_integrated() {
        return Err(InitError::NotResponding);
    }
    let Some(local_sources) = advertised_local_sources(version) else {
        return Err(InitError::NotResponding);
    };

    registers.write(register::TASK_PRIORITY, ACCEPT_EVERYTHING);
    for &entry in local_sources {
        registers.write(entry, Entry::MASKED.bits().into());
        if !Entry::from_bits(low_u32(registers.read(entry))).is_masked() {
            return Err(InitError::NotResponding);
        }
    }

    // A software-disabled APIC holds every LVT mask bit set, so it is enabled before any unmask.
    registers.write(register::SPURIOUS_INTERRUPT_VECTOR, spurious.bits().into());
    if registers.read(register::SPURIOUS_INTERRUPT_VECTOR) != u64::from(spurious.bits()) {
        return Err(InitError::NotResponding);
    }

    registers.write(register::LVT_ERROR, error.bits().into());
    if Entry::from_bits(low_u32(registers.read(register::LVT_ERROR))).is_masked() {
        return Err(InitError::NotResponding);
    }
    // The error register latches on write, so this discards what accrued under the firmware.
    registers.write(register::ERROR_STATUS, 0);
    let _ = registers.read(register::ERROR_STATUS);

    TIMER_VECTOR.store(vectors.timer.get(), Ordering::Relaxed);
    SPURIOUS_VECTOR.store(vectors.spurious.get(), Ordering::Relaxed);

    Ok(Local {
        id: low_u32(registers.read(register::ID)),
        version: version.number(),
        bootstrap: base.is_bootstrap(),
    })
}

fn advertised_local_sources(version: Version) -> Option<&'static [Register]> {
    let maximum = usize::from(version.maximum_lvt_entry());
    if maximum < usize::from(LAST_REQUIRED_LVT_ENTRY) {
        return None;
    }
    let known = register::LOCAL_VECTOR_TABLE.len();
    register::LOCAL_VECTOR_TABLE.get(..maximum.saturating_add(1).min(known))
}

fn initialization_conflict(bits: u8) -> InitError {
    match lifecycle_from(bits) {
        Lifecycle::Ready => InitError::AlreadyInitialized,
        Lifecycle::Initializing | Lifecycle::Cold => InitError::InitializationInProgress,
        Lifecycle::Poisoned => InitError::Poisoned,
    }
}

// A compatibility-format MSI accepted into IRR before remapping went live survives until `sti`.
pub fn enable_after_pending_audit() -> Result<(), PendingInterruptError> {
    if interrupt::are_enabled() {
        return Err(PendingInterruptError::InterruptsEnabled);
    }
    let Some(registers) = ready_registers() else {
        return Err(PendingInterruptError::Unavailable);
    };
    let requests = registers.bank(register::INTERRUPT_REQUEST);
    if let Some(vector) = x86_apic::pending::highest(&requests) {
        return Err(PendingInterruptError::Pending(vector));
    }
    interrupt::enable();
    Ok(())
}

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

// A changed target returns the token unchanged and performs no write.
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

pub fn set_timer_divide(divide: Divide) {
    write(register::DIVIDE_CONFIGURATION, divide.bits().into());
}

pub fn arm_timer(count: u32) {
    assert!(count != 0, "a zero initial count stops the timer");
    write(register::INITIAL_COUNT, count.into());
}

pub fn disarm_timer() -> Result<DisarmedTimer, TimerError> {
    if published_vector(&TIMER_VECTOR).is_none() {
        return Err(TimerError::Unavailable);
    }
    write(register::INITIAL_COUNT, 0);
    if timer_current_count() != 0 {
        return Err(TimerError::StillRunning);
    }
    Ok(DisarmedTimer { _private: () })
}

pub fn set_timer_one_shot(_disarmed: DisarmedTimer) -> Result<(), TimerError> {
    let Some(vector) = published_vector(&TIMER_VECTOR) else {
        return Err(TimerError::Unavailable);
    };
    let Some(spurious) = published_vector(&SPURIOUS_VECTOR) else {
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

pub fn timer_current_count() -> u32 {
    low_u32(read(register::CURRENT_COUNT))
}

// The register latches on write, so this writes first, and only zero may be written.
pub fn error_status() -> u32 {
    write(register::ERROR_STATUS, 0);
    low_u32(read(register::ERROR_STATUS))
}

// Zero before the controller is up, which is also the usual boot processor's identifier.
pub fn id() -> u32 {
    low_u32(read(register::ID))
}

fn lifecycle_from(bits: u8) -> Lifecycle {
    let Some(state) = Lifecycle::from_bits(bits) else {
        panic!("the local APIC lifecycle byte holds a value only this module writes");
    };
    state
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

// Only `enter_extended_mode` and `ready_registers` construct this, each after x2APIC readback.
#[derive(Debug)]
struct Registers;

#[expect(
    clippy::unused_self,
    reason = "the receiver is the proof that x2APIC mode read back"
)]
impl Registers {
    fn write(&self, register: Register, value: u64) {
        // SAFETY: the processor is in x2APIC mode (see `Registers`), so `register.msr()` names an
        // implemented APIC register; an APIC write changes interrupt delivery, never memory, and
        // every caller passes an `x86_apic` encoding, a nonzero timer count or a required zero.
        unsafe { msr::write(register.msr(), value) };
    }

    fn read(&self, register: Register) -> u64 {
        msr::read(register.msr())
    }

    fn bank(&self, registers: [Register; 8]) -> [u32; 8] {
        registers.map(|register| low_u32(self.read(register)))
    }
}

fn ready_registers() -> Option<Registers> {
    lifecycle_from(STATE.load(Ordering::Acquire))
        .is_ready()
        .then_some(Registers)
}

fn write(register: Register, value: u64) {
    if let Some(registers) = ready_registers() {
        registers.write(register, value);
    }
}

fn read(register: Register) -> u64 {
    ready_registers().map_or(0, |registers| registers.read(register))
}

fn published_vector(source: &AtomicU8) -> Option<InterruptVector> {
    let registers = ready_registers()?;
    let bits = low_u32(registers.read(register::SPURIOUS_INTERRUPT_VECTOR));
    if !Spurious::from_bits(bits).is_enabled() {
        return None;
    }
    InterruptVector::new(source.load(Ordering::Relaxed))
}

// Enable, then extended mode, in two writes: setting both from a disabled APIC raises `#GP`.
fn enter_extended_mode() -> Result<(Base, Registers), InitError> {
    let mut base = Base::from_bits(msr::read(base::MSR));

    if !base.is_enabled() {
        // SAFETY: `cpuid` reported a local APIC, so `IA32_APIC_BASE` exists; only the enable bit
        // is added and the address field is written back unaltered, so nothing moves.
        unsafe { msr::write(base::MSR, base.enabled().bits()) };

        // Some processors keep a disabled APIC disabled until reset.
        base = Base::from_bits(msr::read(base::MSR));
        if !base.is_enabled() {
            return Err(InitError::Disabled);
        }
    }

    if !base.is_x2apic() {
        // SAFETY: the same register, adding extended mode to an APIC that reads back enabled, the
        // one transition the architecture permits into it; `cpuid` reported the mode.
        unsafe { msr::write(base::MSR, base.x2apic().bits()) };

        base = Base::from_bits(msr::read(base::MSR));
        if !base.is_x2apic() {
            return Err(InitError::ModeRefused);
        }
    }

    Ok((base, Registers))
}

fn owns_in_service(vector: InterruptVector) -> bool {
    let Some(registers) = ready_registers() else {
        return false;
    };
    x86_apic::in_service::owns(&registers.bank(register::IN_SERVICE), vector)
}
