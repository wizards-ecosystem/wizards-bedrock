//! What the kernel does when the CPU raises a vector.
//!
//! Every vector arrives here. A breakpoint resumes; everything else is a kernel
//! bug and ends the machine with a report. Nothing is recovered and nothing is
//! retried — containment is supervised restart (D-14), and there is no
//! supervisor yet.

use core::fmt;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use wost::arch::current::cpu;
use wost::arch::current::interrupt::{self, TrapFrame};
use x86_tables::exception;

/// Route every vector to this module.
pub(crate) fn init() {
    interrupt::init(on_trap);
}

/// The kernel's response to every vector.
///
/// Nothing here logs. Taking a console lock in trap context deadlocks against
/// interrupted code that already holds it — the hazard `docs/001-boot.md`
/// records against `SpinLock`. The panic path is exempt because it force-unlocks
/// the consoles first; the breakpoint path records into atomics and leaves the
/// printing to whoever planted the breakpoint.
fn on_trap(frame: &mut TrapFrame) {
    if frame.vector == u64::from(exception::BREAKPOINT) {
        LAST_BREAKPOINT.record(frame);
        return;
    }

    panic!("{}", Fault(frame));
}

/// A fault formatted for the panic message.
struct Fault<'a>(&'a TrapFrame);

impl fmt::Display for Fault<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let frame = self.0;
        let vector = frame.vector as u8;
        let name = exception::name(vector).unwrap_or("unexpected interrupt");

        write!(
            f,
            "{name} (vector {vector}, error {:#x}) at {:#018x}, cs {:#x}, flags {:#x}, rsp {:#018x}",
            frame.error_code,
            frame.instruction_pointer,
            frame.code_selector,
            frame.flags,
            frame.stack_pointer,
        )?;

        // The address is the whole content of a page fault report; every other
        // vector leaves CR2 holding whatever the last one did.
        if vector == exception::PAGE_FAULT {
            write!(f, ", faulting address {:#018x}", cpu::faulting_address())?;
        }

        Ok(())
    }
}

/// What the last breakpoint saw.
///
/// A breakpoint is the one exception that resumes, so unlike a fault it leaves
/// no trace unless something records it. Atomics rather than a lock because
/// this is written from trap context.
struct LastBreakpoint {
    count: AtomicUsize,
    vector: AtomicU64,
    error_code: AtomicU64,
    code_selector: AtomicU64,
    flags: AtomicU64,
    stack_pointer: AtomicU64,
}

static LAST_BREAKPOINT: LastBreakpoint = LastBreakpoint {
    count: AtomicUsize::new(0),
    vector: AtomicU64::new(0),
    error_code: AtomicU64::new(0),
    code_selector: AtomicU64::new(0),
    flags: AtomicU64::new(0),
    stack_pointer: AtomicU64::new(0),
};

impl LastBreakpoint {
    /// Record `frame`, publishing the count last so a reader that sees it sees
    /// the rest.
    fn record(&self, frame: &TrapFrame) {
        self.vector.store(frame.vector, Ordering::Relaxed);
        self.error_code.store(frame.error_code, Ordering::Relaxed);
        self.code_selector
            .store(frame.code_selector, Ordering::Relaxed);
        self.flags.store(frame.flags, Ordering::Relaxed);
        self.stack_pointer
            .store(frame.stack_pointer, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Release);
    }
}

/// A copy of what the trap handler recorded for the last breakpoint.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Breakpoint {
    /// How many breakpoints have been taken since boot.
    pub(crate) count: usize,
    /// The vector the handler was entered with. Three, if the gates are right.
    pub(crate) vector: u64,
    /// The error code slot, which a breakpoint does not fill.
    pub(crate) error_code: u64,
    /// The code selector at the breakpoint.
    pub(crate) code_selector: u64,
    /// The flags register at the breakpoint.
    pub(crate) flags: u64,
    /// The stack pointer at the breakpoint.
    pub(crate) stack_pointer: u64,
}

/// What the trap handler recorded for the most recent breakpoint.
///
/// The recording itself is unconditional — a trap handler is no place for
/// conditional compilation — but nothing in a shipped kernel reads it yet.
#[cfg(feature = "selftest")]
pub(crate) fn last_breakpoint() -> Breakpoint {
    Breakpoint {
        count: LAST_BREAKPOINT.count.load(Ordering::Acquire),
        vector: LAST_BREAKPOINT.vector.load(Ordering::Relaxed),
        error_code: LAST_BREAKPOINT.error_code.load(Ordering::Relaxed),
        code_selector: LAST_BREAKPOINT.code_selector.load(Ordering::Relaxed),
        flags: LAST_BREAKPOINT.flags.load(Ordering::Relaxed),
        stack_pointer: LAST_BREAKPOINT.stack_pointer.load(Ordering::Relaxed),
    }
}
