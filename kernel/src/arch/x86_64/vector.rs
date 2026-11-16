use native_abi::GATE_VECTOR;
use wost::arch::current::apic::Vectors;
use x86_apic::vector::{Assignment, InterruptVector};

// The timer sits clear of the legacy 0x20 to 0x2F range, so a leaked PIC line cannot pass for it.
const SOURCES: [InterruptVector; 6] =
    match Assignment::new([0xff, 0xfe, 0x41, GATE_VECTOR, 0x42, 0x43]) {
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
    reschedule: RESCHEDULE,
};

// Inter-processor TLB shootdown, answered in `wost` before any kernel handler.
const SHOOTDOWN: InterruptVector = SOURCES[4];

pub(crate) const SYSCALL: InterruptVector = SOURCES[3];

// Sent to an idle processor whose queue just gained a thread.
pub(crate) const RESCHEDULE: InterruptVector = SOURCES[5];
