//! What the kernel does when the CPU raises a vector.
//!
//! Every vector arrives here — exceptions the CPU raised against itself, and
//! interrupts the local APIC delivered. An exception is a kernel bug and ends
//! the machine with a report, bar the breakpoint, which resumes. An interrupt
//! is dismissed and returned from. Nothing is recovered and nothing is retried
//! — containment is supervised restart (D-14), and there is no supervisor yet.

use core::fmt;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use wost::arch::current::apic;
use wost::arch::current::cpu;
use wost::arch::current::interrupt::{self, TrapFrame};
use x86_tables::exception;

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
    match frame.vector() as u8 {
        exception::BREAKPOINT => LAST_BREAKPOINT.record(frame),

        vector::PING => {
            LAST_PING.record(frame);
            apic::end_of_interrupt();
        }

        vector::TIMER => {
            crate::time::on_expiry();
            // Dismissed before any switch: the APIC holds this priority class
            // until the end-of-interrupt, and another thread running inside
            // it would hold every later expiry hostage.
            apic::end_of_interrupt();
            crate::thread::reschedule(frame);
        }

        vector::KEYBOARD => {
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
        vector::SPURIOUS => {
            SPURIOUS.fetch_add(1, Ordering::Relaxed);
        }

        vector::ERROR => panic!(
            "local apic error {:#x} at {:#018x}",
            apic::error_status(),
            frame.instruction_pointer()
        ),

        _ => panic!("{}", Fault(frame)),
    }
}

/// How many spurious interrupts have been delivered.
///
/// Should stay at zero. A count that climbs means an interrupt was dismissed
/// that should not have been, or a vector was reprogrammed while one was in
/// flight — neither of which shows up any other way.
static SPURIOUS: AtomicUsize = AtomicUsize::new(0);

/// A fault formatted for the panic message.
struct Fault<'a>(&'a TrapFrame);

impl fmt::Display for Fault<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let frame = self.0;
        let vector = frame.vector() as u8;
        let name = exception::name(vector).unwrap_or("unexpected interrupt");

        write!(
            f,
            "{name} (vector {vector}, error {:#x}) at {:#018x}, cs {:#x}, flags {:#x}, rsp {:#018x}",
            frame.error_code(),
            frame.instruction_pointer(),
            frame.code_selector(),
            frame.flags(),
            frame.stack_pointer(),
        )?;

        // The address is the whole content of a page fault report; every other
        // vector leaves CR2 holding whatever the last one did.
        if vector == exception::PAGE_FAULT {
            write!(f, ", faulting address {:#018x}", cpu::faulting_address())?;
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
