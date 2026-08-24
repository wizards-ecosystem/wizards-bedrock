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

use super::vector;

/// Route every vector to this module.
pub(crate) fn init() {
    interrupt::init(on_trap);
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

        _ => panic!("{}", Report { frame, cause }),
    }
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

/// What the last vector of one kind saw.
///
/// A vector that resumes leaves no trace unless something records it, and both
/// of the ones that resume — the breakpoint and the ping — are deliberate acts
/// whose whole point is to be observed afterwards. Atomics rather than a lock
/// because this is written from trap context.
struct Recorder {
    count: AtomicUsize,
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
