//! The local advanced programmable interrupt controller.
//!
//! Every interrupt this kernel will ever take arrives through the local APIC:
//! its own timer, its error line, the inter-processor interrupts it sends
//! itself, and eventually everything an I/O APIC routes to it. This module
//! brings it up, and dismisses what it delivers.
//!
//! **x2APIC only.** The controller has two encodings. The older one is a page
//! of memory-mapped registers at a physical address no bootloader maps and this
//! kernel cannot map for itself until Phase 3 gives it page tables. The newer
//! one puts the same registers in model-specific registers, which need no
//! mapping at all — so a phase about the CPU can bring up the interrupt
//! controller without first becoming a phase about memory. Every x86_64
//! processor since 2008 has it; one without is reported, not worked around.
//!
//! Module state rather than a handle, for the same reason `segment` and
//! `interrupt` are: there is exactly one of this hardware per CPU, and
//! [`end_of_interrupt`] has to be reachable from a trap handler, which is a
//! bare `fn` with nowhere to keep one.

use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use x86_tables::exception;

use x86_apic::Register;
use x86_apic::base::{self, Base};
use x86_apic::command::Command;
use x86_apic::local_vector::Entry;
use x86_apic::register;
use x86_apic::spurious::Spurious;
use x86_apic::timer::Divide;
use x86_apic::version::Version;

use super::cpu;
use super::msr;

/// Whether the register block is reachable — that is, whether the processor is
/// in x2APIC mode.
///
/// Outside that mode every one of these registers raises a general protection
/// fault on access, so this is not an optimisation: it is what makes
/// [`end_of_interrupt`] safe to call from a handler that ran before the
/// controller came up, or on a machine where it never did.
static REGISTERS_READY: AtomicBool = AtomicBool::new(false);

/// Accept every priority class. The APIC will not deliver an interrupt whose
/// class is at or below the task priority, and this kernel has no priorities to
/// express.
const ACCEPT_EVERYTHING: u64 = 0;

/// Why the local APIC could not be brought up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApicError {
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
    /// The registers answer, but not as an APIC.
    NotResponding,
}

impl fmt::Display for ApicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => f.write_str("this processor reports no local apic"),
            Self::NoExtendedMode => {
                f.write_str("this processor has no x2apic mode, and xapic needs page tables")
            }
            Self::Disabled => f.write_str("the local apic is disabled and will not re-enable"),
            Self::ModeRefused => f.write_str("the processor would not enter x2apic mode"),
            Self::NotResponding => f.write_str("the registers do not answer as an apic"),
        }
    }
}

impl core::error::Error for ApicError {}

/// The vectors the local APIC delivers on its own behalf.
///
/// Which vector carries what is the kernel's decision, not this module's, so it
/// arrives as an argument. Both are validated by the encoding — the spurious
/// vector's low four bits, and neither being one the APIC refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vectors {
    /// Delivered when the APIC has committed to delivering something and then
    /// found it had nothing to say. Its low four bits must all be set.
    pub spurious: u8,
    /// Delivered when the APIC records an error.
    pub error: u8,
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

impl fmt::Display for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "x2apic id {} version {:#x}", self.id, self.version)?;
        if self.is_bootstrap {
            f.write_str(" (bootstrap)")?;
        }
        Ok(())
    }
}

/// Bring the local APIC up and start accepting what it delivers.
///
/// The order is the design. Everything the firmware may have programmed is
/// masked, and the two vectors the APIC needs for itself are installed, before
/// the write that software-enables it — so the first interrupt it is capable of
/// delivering arrives at a gate the kernel chose.
///
/// This does not set the interrupt flag. That is
/// [`interrupt::enable`](super::interrupt::enable), and it is a separate step
/// because it has a separate precondition: the legacy controller.
///
/// Calling this a second time reconfigures the same hardware to the same state,
/// which is harmless but pointless; the kernel calls it once.
pub fn init(vectors: Vectors) -> Result<Local, ApicError> {
    if !cpu::has_local_apic() {
        return Err(ApicError::Absent);
    }
    if !cpu::has_x2apic() {
        return Err(ApicError::NoExtendedMode);
    }

    let base = enter_extended_mode()?;
    REGISTERS_READY.store(true, Ordering::Release);

    let version = Version::from_bits(read(register::VERSION) as u32);
    if !version.is_integrated() {
        REGISTERS_READY.store(false, Ordering::Release);
        return Err(ApicError::NotResponding);
    }

    write(register::TASK_PRIORITY, ACCEPT_EVERYTHING);

    // The two local pins are what firmware actually programs: LINT0 as the
    // legacy controller's output and LINT1 as the non-maskable interrupt.
    // Masking them is the other half of silencing the 8259s, and masking the
    // timer covers firmware that left one running.
    for entry in [
        register::LVT_LINT0,
        register::LVT_LINT1,
        register::LVT_TIMER,
    ] {
        write(entry, Entry::MASKED.bits().into());
    }

    write(
        register::LVT_ERROR,
        Entry::fixed(vectors.error).bits().into(),
    );
    // The error register latches on write, so this discards whatever accrued
    // while the APIC belonged to the firmware rather than reporting it as ours.
    let _ = error_status();

    write(
        register::SPURIOUS_INTERRUPT_VECTOR,
        Spurious::enabled(vectors.spurious).bits().into(),
    );

    Ok(Local {
        id: read(register::ID) as u32,
        version: version.number(),
        is_bootstrap: base.is_bootstrap(),
    })
}

/// Dismiss the interrupt currently in service.
///
/// Until this is called nothing of the same priority class or lower can be
/// delivered, so a handler that forgets it takes exactly one interrupt and then
/// goes quiet for ever. Does nothing before [`init`].
///
/// Safe to call from a trap handler, and meant to be: it is one register
/// written, taking no lock and reading nothing another context could be
/// part-way through changing.
pub fn end_of_interrupt() {
    write(register::END_OF_INTERRUPT, 0);
}

/// Deliver `vector` to this CPU.
///
/// The interrupt is not taken until the CPU is accepting them: with the flag
/// clear it stays pending in the APIC and arrives at the `sti`. This is how the
/// kernel proves its own interrupt path — and, when there is more than one CPU,
/// how one will ask another to do something.
///
/// # Panics
///
/// Panics if `vector` is below 16, which the APIC would refuse to deliver, or
/// if the CPU pushes an error code for it.
///
/// The second is not a nicety. The entry stubs push a synthetic zero error
/// code exactly where the CPU does not, so a vector the CPU *would* have
/// pushed for enters a stub that assumes one is there — and the frame is eight
/// bytes out for the rest of its life. The tail's `add rsp, 16; iretq` then
/// pops `CS` as `RIP`, and the ring-3 test reads the wrong slot. An APIC
/// delivery pushes no error code whatever the vector means, so `self_interrupt`
/// on 17 was a safe call into that state (D-123).
pub fn send_self_interrupt(vector: u8) {
    assert!(
        !exception::pushes_error_code(vector),
        "the cpu pushes an error code for this vector, and a delivered          interrupt does not"
    );
    // The upper half of the command is the destination, which the self
    // shorthand makes irrelevant, so it is written as the zeros the
    // architecture reserves it to be. No wait for delivery afterwards: x2APIC
    // dropped the delivery-status bit because the write is synchronous.
    write(
        register::COMMAND,
        Command::self_interrupt(vector).bits().into(),
    );
}

/// Set how the bus clock is divided before it drives the timer's countdown.
///
/// Does nothing before [`init`].
pub fn set_timer_divide(divide: Divide) {
    write(register::DIVIDE_CONFIGURATION, divide.bits().into());
}

/// Start the timer's countdown from `count` without touching its delivery.
///
/// Writing the initial count is what starts the timer; that is the
/// architecture's rule, not this module's. What the expiry then does is
/// whatever the entry says: nothing at all while it is masked, as [`init`]
/// leaves it and as calibration wants it, or one interrupt after
/// [`set_timer_one_shot`]. Does nothing before [`init`].
///
/// # Panics
///
/// Panics if `count` is zero, which stops the timer rather than arming it —
/// [`disarm_timer`] is how a caller says that on purpose.
pub fn arm_timer(count: u32) {
    assert!(count != 0, "a zero initial count stops the timer");
    write(register::INITIAL_COUNT, count.into());
}

/// Stop the countdown.
///
/// A zero initial count is how the architecture spells "not counting", and
/// reading the current count back as zero is how a caller confirms it. An
/// interrupt the timer has already delivered is not recalled by this; a
/// handler that finds nothing to do is the cost of the race, and it is
/// cheaper than the lock that would close it. Does nothing before [`init`].
pub fn disarm_timer() {
    write(register::INITIAL_COUNT, 0);
}

/// Make each expiry deliver `vector` once, and stop.
///
/// Only the entry: the timer stays as armed or as stopped as it already was,
/// so a caller unmasking a countdown it has not disarmed will get an
/// interrupt whenever that countdown happens to reach zero. Arrange to be
/// disarmed first. Does nothing before [`init`].
///
/// # Panics
///
/// Panics if `vector` is below 16, which the APIC would refuse, or if the CPU
/// pushes an error code for it — see [`send_self_interrupt`] (D-123).
pub fn set_timer_one_shot(vector: u8) {
    assert!(
        !exception::pushes_error_code(vector),
        "the cpu pushes an error code for this vector, and a timer interrupt          does not"
    );
    write(register::LVT_TIMER, Entry::one_shot(vector).bits().into());
}

/// Where the timer's countdown currently stands. Zero before [`init`].
pub fn timer_current_count() -> u32 {
    read(register::CURRENT_COUNT) as u32
}

/// The timer's local vector entry, read back from the hardware rather than
/// remembered from what was written.
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
/// the boot processor's usual identifier, so a caller that must tell the two
/// apart asks [`is_enabled`] first.
pub fn id() -> u32 {
    read(register::ID) as u32
}

/// Whether the APIC is software-enabled and therefore delivering.
pub fn is_enabled() -> bool {
    REGISTERS_READY.load(Ordering::Acquire)
        && Spurious::from_bits(read(register::SPURIOUS_INTERRUPT_VECTOR) as u32).is_enabled()
}

/// Put the APIC into x2APIC mode, and report the register as it now stands.
///
/// Two writes, in this order, because the architecture's state machine has no
/// edge from disabled straight to extended mode: a single write setting both
/// bits from a disabled APIC raises a general protection fault.
fn enter_extended_mode() -> Result<Base, ApicError> {
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

    Ok(base)
}

/// Write one register, doing nothing outside x2APIC mode.
fn write(register: Register, value: u64) {
    if !REGISTERS_READY.load(Ordering::Acquire) {
        return;
    }
    // SAFETY: the processor is in x2APIC mode, which is what makes this
    // register number one it implements — `REGISTERS_READY` says so, and only
    // `enter_extended_mode` sets it, after reading the mode back. Every value
    // written comes from an encoding in `x86_apic`, is the timer's initial
    // count (all thirty-two bits defined), or is the zero the architecture
    // accepts for the task priority and mandates for the error status.
    unsafe { msr::write(register.msr(), value) };
}

/// Read one register, answering zero outside x2APIC mode.
fn read(register: Register) -> u64 {
    if !REGISTERS_READY.load(Ordering::Acquire) {
        return 0;
    }
    msr::read(register.msr())
}
