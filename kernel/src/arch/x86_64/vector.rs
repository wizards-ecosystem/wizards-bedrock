use wost::arch::current::apic::Vectors;
use x86_apic::vector::{Assignment, InterruptVector};

// The timer sits clear of the legacy 0x20 to 0x2F range, so a leaked PIC line cannot pass for it.
const SOURCES: [InterruptVector; 5] = match Assignment::new([0xff, 0xfe, 0x41, 0x80, 0x42]) {
    Some(assignment) => assignment.into_array(),
    None => panic!("kernel interrupt-vector assignment is invalid"),
};

pub(crate) const SPURIOUS: InterruptVector = SOURCES[0];

pub(crate) const ERROR: InterruptVector = SOURCES[1];

pub(crate) const TIMER: InterruptVector = SOURCES[2];

pub(crate) const LOCAL_APIC: Vectors = Vectors {
    spurious: SPURIOUS,
    error: ERROR,
    timer: TIMER,
    shootdown: SHOOTDOWN,
};

// Inter-processor TLB shootdown, answered in `wost` before any kernel handler.
const SHOOTDOWN: InterruptVector = SOURCES[4];

pub(crate) const SYSCALL: InterruptVector = SOURCES[3];
