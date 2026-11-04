//! The kernel's interrupt-vector assignment.

use wost::arch::current::apic::Vectors;
use x86_apic::vector::{Assignment, InterruptVector};

const SOURCES: [InterruptVector; 6] = match Assignment::new([0xff, 0xfe, 0x40, 0x41, 0x80, 0x42]) {
    Some(assignment) => assignment.into_array(),
    None => panic!("kernel interrupt-vector assignment is invalid"),
};

/// Delivered when the local APIC has committed to delivering something and then found it had
/// nothing to say. The low four bits are all set because the earliest APICs hardwire them, so any
/// other value would silently become this one anyway.
pub(crate) const SPURIOUS: InterruptVector = SOURCES[0];

/// Delivered when the local APIC records an error against itself.
pub(crate) const ERROR: InterruptVector = SOURCES[1];

/// The kernel sends this to itself. This makes controller configuration, gate entry, frame
/// preservation and dismissal observable without relying on device traffic.
pub(crate) const PING: InterruptVector = SOURCES[2];

/// The local APIC timer's expiry.
pub(crate) const TIMER: InterruptVector = SOURCES[3];

/// What the local APIC is told to use for itself.
pub(crate) const LOCAL_APIC: Vectors = Vectors {
    spurious: SPURIOUS,
    error: ERROR,
    self_interrupt: PING,
    timer: TIMER,
};

/// The kernel's entry from ring 3. `0x80` for the reason everyone chooses it: clear of the
/// architectural range, clear of anything the local APIC or the I/O APIC delivers, and recognisable
/// in a disassembly.
pub(crate) const SYSCALL: InterruptVector = SOURCES[4];

/// The keyboard controller's line, routed through the I/O APIC.
pub(crate) const KEYBOARD: InterruptVector = SOURCES[5];
