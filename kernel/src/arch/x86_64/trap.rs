//! What the kernel does when the CPU raises a vector.
//!
//! Every vector arrives here — exceptions the CPU raised against itself, and
//! interrupts the local APIC delivered. An exception is a kernel bug and ends
//! the machine with a report, bar the breakpoint, which resumes. An interrupt
//! is dismissed and returned from. Nothing is recovered and nothing is retried
//! — containment is supervised restart (D-14), and there is no supervisor yet.

use core::fmt;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use trap_cause::TrapCause;
use wost::arch::current::apic;
use wost::arch::current::cpu;
use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::usercopy;

use super::vector;

/// Route every vector to this module.
pub(crate) fn init() {
    interrupt::init(on_trap, Some(vector::SYSCALL));
}

/// The kernel's response to every vector.
///
/// Nothing here logs. A fault can be raised while a lock is held — the
/// interrupt flag does not mask one — so taking the console lock in trap
/// context still deadlocks against the code it interrupted, even though
/// `SpinLock` now excludes maskable interrupts. The panic path is exempt
/// because it force-unlocks the consoles first; everything that has to survive
/// is recorded into atomics and printed by whoever reads them.
fn on_trap(frame: &TrapFrame) {
    // Dispatched on the decoded cause rather than the raw vector (D-81): the
    // architecture's meaning is read once, in `libs/trap-cause`, where a host
    // test walks all 256 of them — instead of being re-derived by every arm
    // that tests a number.
    let cause = TrapCause::decode(frame.vector() as u8, frame.error_code());
    match cause {
        TrapCause::Fault(trap_cause::Fault::Breakpoint) => LAST_BREAKPOINT.record(frame),

        TrapCause::Interrupt(vector::PING) => {
            LAST_PING.record(frame);
            apic::end_of_interrupt();
        }

        TrapCause::Interrupt(vector::TIMER) => {
            crate::time::on_expiry();
            // Dismissed before any switch: the APIC holds this priority class
            // until the end-of-interrupt, and another thread running inside
            // it would hold every later expiry hostage.
            apic::end_of_interrupt();
            crate::thread::reschedule(frame);
        }

        TrapCause::Interrupt(vector::KEYBOARD) => {
            let woke = crate::input::on_interrupt();
            apic::end_of_interrupt();
            // A thread woken by a keystroke must not wait for the next
            // expiry: there may not be one.
            if woke {
                crate::thread::reschedule(frame);
            }
        }

        // The one entry ring 3 has. Nothing is served yet: what is proved is
        // that a thread can cross into user mode, come back through a gate
        // meant for it, and be retired from the far side (D-96).
        TrapCause::Interrupt(vector::SYSCALL) => {
            LAST_SYSCALL.record(frame);
            // Whether SMAP is on *inside the handler*, which is the thing
            // D-101 is about: the caller's flags are in the frame, and this is
            // the CPU's own state now that the entry path has run.
            SMAP_ON_ENTRY.store(u64::from(!cpu::access_check_is_off()), Ordering::Release);
            crate::user::call(frame);
        }

        // Deliberately not dismissed: a spurious interrupt is the APIC saying
        // it had nothing to deliver, so it never set an in-service bit and
        // dismissing one would retire a real interrupt underneath it.
        TrapCause::Interrupt(vector::SPURIOUS) => {
            SPURIOUS.fetch_add(1, Ordering::Relaxed);
        }

        TrapCause::Interrupt(vector::ERROR) => panic!(
            "local apic error {:#x} at {:#018x}",
            apic::error_status(),
            frame.instruction_pointer()
        ),

        // A fault the *program* caused kills the program, not the machine.
        //
        // D-31 made a fault terminal when a fault was necessarily a kernel
        // bug. Since ring 3 exists it can be a program bug — a `ud2`, a
        // division by zero, a touch of a page that is not there — and D-97
        // already reasons from the right premise for the copy path: the
        // program is the untrusted party. This is the same premise applied to
        // the trap path (D-102).
        //
        // The privilege in the saved `CS` is the CPU's own account of where
        // the fault happened, so it cannot be forged by kernel code that
        // merely looks like a program.
        // A fault inside the user-copy window is neither the kernel's bug nor
        // the program's death: it is the page having gone away between the
        // walk and the copy, which interrupts being on between them makes
        // possible. The copy resumes at its fixup and reports how much it did
        // not move (D-120).
        //
        // **Before** the ring-3 test, because this fault is taken in ring 0 —
        // the kernel is the one dereferencing — so `is_the_programs` would say
        // no and the machine would stop.
        TrapCause::Fault(trap_cause::Fault::PageFault(_)) if usercopy::recover(frame) => {
            USER_COPY_RECOVERIES.fetch_add(1, Ordering::Relaxed);
        }

        TrapCause::Fault(fault) if is_the_programs(frame, fault) => {
            LAST_USER_FAULT.record(frame);
            if !crate::thread::retire_from_trap(frame) {
                panic!("{}", Report { frame, cause });
            }
        }

        _ => panic!("{}", Report { frame, cause }),
    }
}

/// Faults recovered inside the user-copy window.
///
/// Should stay at zero on a machine where nothing unmaps another thread's
/// memory; a count that climbs is the window being reached, which is worth
/// knowing rather than inferring.
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
static SPURIOUS: AtomicUsize = AtomicUsize::new(0);

/// A fault formatted for the panic message.
struct Report<'a> {
    frame: &'a TrapFrame,
    cause: TrapCause,
}

impl fmt::Display for Report<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let frame = self.frame;

        write!(
            f,
            "{} (vector {}, error {:#x}) at {:#018x}, cs {:#x}, flags {:#x}, rsp {:#018x}",
            self.cause.name(),
            self.cause.vector(),
            frame.error_code(),
            frame.instruction_pointer(),
            frame.code_selector(),
            frame.flags(),
            frame.stack_pointer(),
        )?;

        // A page fault is the one vector whose error code says something, and
        // saying it is why the decode exists: a missing mapping and a refused
        // one are different bugs, and this report could not tell them apart
        // before. CR2 is only this fault's address for this vector; every
        // other leaves whatever the last one put there.
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
static LAST_USER_FAULT: Recorder = Recorder::new();

/// The frame of the last fault a program caused.
#[cfg(feature = "selftest")]
pub(crate) fn last_user_fault() -> Record {
    LAST_USER_FAULT.read()
}

/// One if SMAP was still on when the last ring-3 call reached the handler.
static SMAP_ON_ENTRY: AtomicU64 = AtomicU64::new(0);

/// Whether the last call from ring 3 arrived with SMAP still on.
#[cfg(feature = "selftest")]
pub(crate) fn smap_survived_the_last_syscall() -> bool {
    SMAP_ON_ENTRY.load(Ordering::Acquire) == 1
}

/// What the last call from ring 3 looked like.
static LAST_SYSCALL: Recorder = Recorder::new();

/// The frame of the last call from ring 3.
#[cfg(feature = "selftest")]
pub(crate) fn last_syscall() -> Record {
    LAST_SYSCALL.read()
}

/// What the last vector of one kind saw.
///
/// A vector that resumes leaves no trace unless something records it, and both
/// of the ones that resume — the breakpoint and the ping — are deliberate acts
/// whose whole point is to be observed afterwards. Atomics rather than a lock
/// because this is written from trap context.
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

static LAST_BREAKPOINT: Recorder = Recorder::new();
static LAST_PING: Recorder = Recorder::new();

/// A copy of what the trap handler recorded for one vector.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Record {
    /// How many of this vector have been taken since boot.
    pub(crate) count: usize,
    /// The thread slot that took it.
    ///
    /// Recorded since D-117, because slots are reused now: two crossings from
    /// two threads can land on one stack, correctly, if the first is gone by
    /// the time the second starts. So "which stack" no longer identifies a
    /// caller and "which slot" does.
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
///
/// The recording itself is unconditional — a trap handler is no place for
/// conditional compilation — but nothing in a shipped kernel reads it yet.
#[cfg(feature = "selftest")]
pub(crate) fn last_breakpoint() -> Record {
    LAST_BREAKPOINT.read()
}

/// What the trap handler recorded for the most recent ping.
#[cfg(feature = "selftest")]
pub(crate) fn last_ping() -> Record {
    LAST_PING.read()
}

/// How many spurious interrupts the local APIC has delivered.
#[cfg(feature = "selftest")]
pub(crate) fn spurious_count() -> usize {
    SPURIOUS.load(Ordering::Acquire)
}
