//! What the kernel does when the CPU raises a vector.
//!
//! Every vector arrives here — exceptions the CPU raised against itself, and
//! interrupts the local APIC delivered. An interrupt is dismissed and returned
//! from; an unexpected one still ends the machine, which is what keeps a leaked
//! legacy line from arriving as something else.
//!
//! Fault disposition follows its origin. A user-copy fault resumes at the
//! fixup (D-120), a ring-3 fault retires the program (D-102), and other faults
//! halt the kernel. Breakpoints resume; supervised restart remains absent
//! (D-14, D-31).

use core::fmt;
#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use trap_cause::TrapCause;
use wost::arch::current::apic;
use wost::arch::current::cpu;
use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::usercopy;

#[cfg(feature = "selftest")]
use wost::arch::current::paging;
#[cfg(feature = "selftest")]
use wost::arch::current::segment;

use super::vector;

static HANDLERS: interrupt::Handlers =
    interrupt::Handlers::new(on_trap, reject_invalid_user_return);

/// Route every vector to this module.
pub(crate) fn init() {
    interrupt::init(&HANDLERS, Some(vector::SYSCALL));
}

fn reject_invalid_user_return(frame: &TrapFrame, error: interrupt::UserReturnError) -> ! {
    #[cfg(feature = "selftest")]
    {
        LAST_INVALID_USER_RETURN_KIND.store(invalid_user_return_kind(error), Ordering::Relaxed);
        LAST_INVALID_USER_RETURN.record(frame);
    }
    if crate::thread::retire_from_trap(frame) {
        unreachable!("retiring an invalid user return resumed its context");
    }
    panic!("invalid user return could not be retired: {error:?}");
}

/// The kernel's response to every vector.
///
/// Nothing here logs. A fault can be raised while a lock is held, so taking the
/// console lock in trap context can deadlock against the interrupted code.
fn on_trap(frame: &TrapFrame) {
    // Dispatched on the decoded cause, not the raw vector (D-81): the
    // architecture's meaning is read once, in `libs/trap-cause`, host-tested.
    let cause = TrapCause::decode(frame.vector() as u8, frame.error_code());
    match cause {
        TrapCause::Fault(trap_cause::Fault::Breakpoint) => {
            #[cfg(feature = "selftest")]
            LAST_BREAKPOINT.record(frame);
        }

        TrapCause::Interrupt(raw) if raw == vector::PING.get() => {
            let eoi = claim_eoi(frame, vector::PING);
            #[cfg(feature = "selftest")]
            {
                paging::observe_activation_publication_for_test();
                if apic::claim_eoi(frame, vector::PING).is_none() {
                    REJECTED_DUPLICATE_EOI_CLAIMS.fetch_add(1, Ordering::Relaxed);
                }
                let eoi = maybe_reclaim_ping(frame, eoi);
                LAST_PING.record(frame);
                if let Some(eoi) = eoi {
                    dismiss(eoi);
                }
            }
            #[cfg(not(feature = "selftest"))]
            dismiss(eoi);
        }

        TrapCause::Interrupt(raw) if raw == vector::TIMER.get() => {
            let eoi = claim_eoi(frame, vector::TIMER);
            crate::time::on_expiry();
            // Dismissed before any switch: the APIC holds this priority class
            // until EOI, and a switched-in thread would hold every expiry hostage.
            dismiss(eoi);
            crate::thread::reschedule(frame);
        }

        TrapCause::Interrupt(raw) if raw == vector::KEYBOARD.get() => {
            let eoi = claim_eoi(frame, vector::KEYBOARD);
            let woke = crate::input::on_interrupt();
            dismiss(eoi);
            // A thread woken by a keystroke must not wait for the next
            // expiry: there may not be one.
            if woke {
                crate::thread::reschedule(frame);
            }
        }

        // The CPU's saved privilege decides: a kernel-context `int 0x80` must
        // not be served as a ring-3 call (D-96, D-124).
        TrapCause::Interrupt(raw)
            if raw == vector::SYSCALL.get() && frame.code_selector() & 0b11 == 3 =>
        {
            #[cfg(feature = "selftest")]
            KERNEL_STACK_SELECTOR_ON_ENTRY.store(
                u64::from(segment::stack_selector().bits()),
                Ordering::Release,
            );
            #[cfg(feature = "selftest")]
            LAST_SYSCALL.record(frame);
            // Whether SMAP is on inside the handler, after the entry path ran,
            // which is what D-101 is about.
            #[cfg(feature = "selftest")]
            SMAP_ON_ENTRY.store(u64::from(!cpu::access_check_is_off()), Ordering::Release);
            crate::user::call(frame);
        }

        // Not dismissed: a spurious interrupt sets no in-service bit, so an
        // EOI here would retire a real interrupt underneath it.
        TrapCause::Interrupt(raw) if raw == vector::SPURIOUS.get() => {
            #[cfg(feature = "selftest")]
            SPURIOUS_OBSERVATIONS.fetch_add(1, Ordering::Relaxed);
        }

        TrapCause::Interrupt(raw) if raw == vector::ERROR.get() => panic!(
            "local apic error {:#x} at {:#018x}",
            apic::error_status(),
            frame.instruction_pointer()
        ),

        // The controlled ring-0 write-protection probe has an exact error code
        // and private recovery interval; ordinary kernel faults remain fatal.
        #[cfg(feature = "selftest")]
        TrapCause::Fault(trap_cause::Fault::PageFault(fault))
            if cpu::recover_write_protection_probe(frame, fault) => {}

        // Only a ring-0 fault inside the user-copy interval recovers; a ring-3
        // fault at the same address is the program's (D-120, D-126).
        TrapCause::Fault(trap_cause::Fault::PageFault(fault))
            if usercopy::recover(frame, fault) =>
        {
            #[cfg(feature = "selftest")]
            USER_COPY_RECOVERIES.fetch_add(1, Ordering::Relaxed);
        }

        // Other survivable ring-3 faults retire the program, not the machine.
        TrapCause::Fault(fault) if is_the_programs(frame, fault) => {
            #[cfg(feature = "selftest")]
            LAST_USER_FAULT.record(frame);
            if !crate::thread::retire_from_trap(frame) {
                panic!("{}", Report { frame, cause });
            }
        }

        _ => panic!("{}", Report { frame, cause }),
    }
}

fn claim_eoi<'frame>(
    frame: &'frame TrapFrame,
    vector: x86_apic::vector::InterruptVector,
) -> apic::EoiToken<'frame> {
    let Some(token) = apic::claim_eoi(frame, vector) else {
        panic!(
            "vector {:#x} has no matching APIC in-service authority",
            vector.get()
        );
    };
    token
}

#[cfg(feature = "selftest")]
fn maybe_reclaim_ping<'frame>(
    frame: &'frame TrapFrame,
    eoi: apic::EoiToken<'frame>,
) -> Option<apic::EoiToken<'frame>> {
    if LAST_PING.count.load(Ordering::Relaxed) != 1 {
        return Some(eoi);
    }

    drop(eoi);
    let reclaimed = apic::claim_eoi(frame, vector::PING);
    if reclaimed.is_some() {
        RECLAIMED_ABANDONED_EOI.fetch_add(1, Ordering::Relaxed);
    }
    reclaimed
}

fn dismiss(token: apic::EoiToken<'_>) {
    if apic::end_of_interrupt(token).is_err() {
        panic!("APIC in-service authority changed before dismissal");
    }
}

/// Faults recovered inside the user-copy window during self-test.
#[cfg(feature = "selftest")]
static USER_COPY_RECOVERIES: AtomicUsize = AtomicUsize::new(0);

/// How many user copies have been resumed at their fixup.
#[cfg(feature = "selftest")]
pub(crate) fn user_copy_recoveries() -> usize {
    USER_COPY_RECOVERIES.load(Ordering::Acquire)
}

/// Whether this fault is a program's to die of rather than the machine's.
///
/// Two conditions, and both are needed. It must have come from ring 3 — the
/// kernel faulting is still a kernel bug and still terminal. And it must be a
/// fault a program can cause and the machine can survive: a double fault means
/// the fault path itself failed, and a machine check is the hardware reporting
/// its own failure. Neither becomes survivable because a program was running.
fn is_the_programs(frame: &TrapFrame, fault: trap_cause::Fault) -> bool {
    let from_ring_three = frame.code_selector() & 0b11 == 3;
    let survivable = !matches!(
        fault,
        trap_cause::Fault::DoubleFault
            | trap_cause::Fault::MachineCheck
            | trap_cause::Fault::NonMaskable
    );
    from_ring_three && survivable
}

/// How many spurious interrupts have been delivered.
///
/// Should stay at zero. A count that climbs means an interrupt was dismissed
/// that should not have been, or a vector was reprogrammed while one was in
/// flight — neither of which shows up any other way.
#[cfg(feature = "selftest")]
static SPURIOUS_OBSERVATIONS: AtomicUsize = AtomicUsize::new(0);

/// A fault formatted for the panic message.
struct Report<'a> {
    frame: &'a TrapFrame,
    cause: TrapCause,
}

impl fmt::Display for Report<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let frame = self.frame;
        let name = self.cause.name();
        let vector = self.cause.vector();

        write!(
            f,
            "{} (vector {}, error {:#x}) at {:#018x}, cs {:#x}, flags {:#x}, rsp {:#018x}",
            name,
            vector,
            frame.error_code(),
            frame.instruction_pointer(),
            frame.code_selector(),
            frame.flags(),
            frame.stack_pointer(),
        )?;

        // CR2 belongs only to page faults; other vectors leave its old value.
        if let TrapCause::Fault(trap_cause::Fault::PageFault(fault)) = self.cause {
            write!(
                f,
                ", faulting address {:#018x}: {fault:?}",
                cpu::faulting_address()
            )?;
        }

        Ok(())
    }
}

/// What the last fault from ring 3 looked like.
#[cfg(feature = "selftest")]
static LAST_USER_FAULT: Recorder = Recorder::new();

/// The frame of the last fault a program caused.
#[cfg(feature = "selftest")]
pub(crate) fn last_user_fault() -> Record {
    Recorder::read(&LAST_USER_FAULT)
}

/// What the last outer-privilege frame refusal looked like.
#[cfg(feature = "selftest")]
static LAST_INVALID_USER_RETURN: Recorder = Recorder::new();

#[cfg(feature = "selftest")]
static LAST_INVALID_USER_RETURN_KIND: AtomicUsize = AtomicUsize::new(0);

/// Which part of an outer-privilege frame was refused.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvalidUserReturnKind {
    /// No refusal has been recorded.
    None,
    /// The user code selector differed from the installed selector.
    CodeSelector,
    /// The user stack selector differed from the installed selector.
    StackSelector,
    /// The instruction pointer was not admitted user state.
    InstructionPointer,
    /// The instruction pointer was not executable in the live space.
    InstructionNotExecutable,
    /// The stack pointer was not admitted user state.
    StackPointer,
    /// The saved flags were unsafe for `iretq`.
    Flags,
}

#[cfg(feature = "selftest")]
fn invalid_user_return_kind(error: interrupt::UserReturnError) -> usize {
    match error {
        interrupt::UserReturnError::CodeSelector(_) => 1,
        interrupt::UserReturnError::StackSelector(_) => 2,
        interrupt::UserReturnError::InstructionPointer(_) => 3,
        interrupt::UserReturnError::InstructionNotExecutable(_) => 4,
        interrupt::UserReturnError::StackPointer(_) => 5,
        interrupt::UserReturnError::Flags(_) => 6,
        _ => 0,
    }
}

/// The most recent refused user return and its reason.
#[cfg(feature = "selftest")]
pub(crate) fn last_invalid_user_return() -> (Record, InvalidUserReturnKind) {
    let frame = Recorder::read(&LAST_INVALID_USER_RETURN);
    let kind = match LAST_INVALID_USER_RETURN_KIND.load(Ordering::Relaxed) {
        1 => InvalidUserReturnKind::CodeSelector,
        2 => InvalidUserReturnKind::StackSelector,
        3 => InvalidUserReturnKind::InstructionPointer,
        4 => InvalidUserReturnKind::InstructionNotExecutable,
        5 => InvalidUserReturnKind::StackPointer,
        6 => InvalidUserReturnKind::Flags,
        _ => InvalidUserReturnKind::None,
    };
    (frame, kind)
}

/// One if SMAP was still on when the last ring-3 call reached the handler.
#[cfg(feature = "selftest")]
static SMAP_ON_ENTRY: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "selftest")]
static KERNEL_STACK_SELECTOR_ON_ENTRY: AtomicU64 = AtomicU64::new(0);

/// The live stack selector observed on the most recent call from ring 3.
#[cfg(feature = "selftest")]
pub(crate) fn kernel_stack_selector_on_last_syscall() -> u64 {
    KERNEL_STACK_SELECTOR_ON_ENTRY.load(Ordering::Acquire)
}

/// Whether the last call from ring 3 arrived with SMAP still on.
#[cfg(feature = "selftest")]
pub(crate) fn smap_survived_the_last_syscall() -> bool {
    SMAP_ON_ENTRY.load(Ordering::Acquire) == 1
}

/// What the last call from ring 3 looked like.
#[cfg(feature = "selftest")]
static LAST_SYSCALL: Recorder = Recorder::new();

/// The frame of the last call from ring 3.
#[cfg(feature = "selftest")]
pub(crate) fn last_syscall() -> Record {
    Recorder::read(&LAST_SYSCALL)
}

/// What the last vector of one kind saw.
///
/// A vector that resumes leaves no trace unless something records it, and both
/// of the ones that resume — the breakpoint and the ping — are deliberate acts
/// whose whole point is to be observed afterwards. Atomics rather than a lock
/// because this is written from trap context.
#[cfg(feature = "selftest")]
struct Recorder {
    count: AtomicUsize,
    slot: AtomicUsize,
    rax: AtomicU64,
    frame_address: AtomicU64,
    vector: AtomicU64,
    error_code: AtomicU64,
    code_selector: AtomicU64,
    flags: AtomicU64,
    stack_pointer: AtomicU64,
    stack_selector: AtomicU64,
}

#[cfg(feature = "selftest")]
impl Recorder {
    const fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            slot: AtomicUsize::new(0),
            rax: AtomicU64::new(0),
            frame_address: AtomicU64::new(0),
            vector: AtomicU64::new(0),
            error_code: AtomicU64::new(0),
            code_selector: AtomicU64::new(0),
            flags: AtomicU64::new(0),
            stack_pointer: AtomicU64::new(0),
            stack_selector: AtomicU64::new(0),
        }
    }

    /// Record `frame`, publishing the count last so a reader that sees it sees
    /// the rest.
    fn record(&self, frame: &TrapFrame) {
        self.slot
            .store(wost::arch::current::task::current(), Ordering::Relaxed);
        self.rax.store(frame.rax(), Ordering::Relaxed);
        self.frame_address
            .store(interrupt::live_frame_address(), Ordering::Relaxed);
        self.vector.store(frame.vector(), Ordering::Relaxed);
        self.error_code.store(frame.error_code(), Ordering::Relaxed);
        self.code_selector
            .store(frame.code_selector(), Ordering::Relaxed);
        self.flags.store(frame.flags(), Ordering::Relaxed);
        self.stack_pointer
            .store(frame.stack_pointer(), Ordering::Relaxed);
        self.stack_selector
            .store(frame.stack_selector(), Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Release);
    }

    /// A consistent copy of what was last recorded.
    #[cfg(feature = "selftest")]
    fn read(&self) -> Record {
        Record {
            count: self.count.load(Ordering::Acquire),
            slot: self.slot.load(Ordering::Relaxed),
            rax: self.rax.load(Ordering::Relaxed),
            frame_address: self.frame_address.load(Ordering::Relaxed),
            vector: self.vector.load(Ordering::Relaxed),
            error_code: self.error_code.load(Ordering::Relaxed),
            code_selector: self.code_selector.load(Ordering::Relaxed),
            flags: self.flags.load(Ordering::Relaxed),
            stack_pointer: self.stack_pointer.load(Ordering::Relaxed),
            stack_selector: self.stack_selector.load(Ordering::Relaxed),
        }
    }
}

#[cfg(feature = "selftest")]
static LAST_BREAKPOINT: Recorder = Recorder::new();
#[cfg(feature = "selftest")]
static LAST_PING: Recorder = Recorder::new();

#[cfg(feature = "selftest")]
static REJECTED_DUPLICATE_EOI_CLAIMS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
static RECLAIMED_ABANDONED_EOI: AtomicUsize = AtomicUsize::new(0);

/// A copy of what the trap handler recorded for one vector.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Record {
    /// How many of this vector have been taken since boot.
    pub(crate) count: usize,
    /// The thread slot that took it.
    ///
    /// Slots identify callers even when sequential threads correctly reuse a
    /// stack (D-117).
    pub(crate) slot: usize,
    /// The general register loaded with the interrupted stack pointer.
    pub(crate) rax: u64,
    /// Where the entry path's own frame begins on the interrupted stack.
    pub(crate) frame_address: u64,
    /// The vector the handler was entered with.
    pub(crate) vector: u64,
    /// The error code slot, which neither of these vectors fills.
    pub(crate) error_code: u64,
    /// The code selector at the point of the trap.
    pub(crate) code_selector: u64,
    /// The flags register at the point of the trap.
    pub(crate) flags: u64,
    /// The stack pointer at the point of the trap.
    pub(crate) stack_pointer: u64,
    /// The stack selector at the point of the trap.
    pub(crate) stack_selector: u64,
}

/// What the trap handler recorded for the most recent breakpoint.
#[cfg(feature = "selftest")]
pub(crate) fn last_breakpoint() -> Record {
    Recorder::read(&LAST_BREAKPOINT)
}

/// What the trap handler recorded for the most recent ping.
#[cfg(feature = "selftest")]
pub(crate) fn last_ping() -> Record {
    Recorder::read(&LAST_PING)
}

/// Duplicate completion claims the ping handler refused.
#[cfg(feature = "selftest")]
pub(crate) fn rejected_duplicate_eoi_claims() -> usize {
    REJECTED_DUPLICATE_EOI_CLAIMS.load(Ordering::Acquire)
}

/// Abandoned ping completions reclaimed for the same live delivery.
#[cfg(feature = "selftest")]
pub(crate) fn reclaimed_abandoned_eoi() -> usize {
    RECLAIMED_ABANDONED_EOI.load(Ordering::Acquire)
}

/// How many spurious interrupts the local APIC has delivered.
#[cfg(feature = "selftest")]
pub(crate) fn spurious_count() -> usize {
    SPURIOUS_OBSERVATIONS.load(Ordering::Acquire)
}
