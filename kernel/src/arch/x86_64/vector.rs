//! The kernel's complete interrupt-vector assignment.

use wost::arch::current::apic::Vectors;
use x86_apic::vector::{Assignment, InterruptVector};

const SOURCES: [InterruptVector; 6] = match Assignment::new([0xff, 0xfe, 0x40, 0x41, 0x80, 0x42]) {
    Some(assignment) => assignment.into_array(),
    None => panic!("kernel interrupt-vector assignment is invalid"),
};

/// Delivered when the local APIC committed to delivering something and had nothing to say.
/// The low four bits are set because the earliest APICs hardwire them; any other value would
/// silently become this one anyway.
pub(crate) const SPURIOUS: InterruptVector = SOURCES[0];

/// Delivered when the local APIC records an error against itself.
pub(crate) const ERROR: InterruptVector = SOURCES[1];

/// The kernel sends this to itself, so controller configuration, gate entry, frame preservation
/// and dismissal are observable without device traffic.
pub(crate) const PING: InterruptVector = SOURCES[2];

/// The local APIC timer's expiry, clear of the legacy range 0x20 to 0x2F so a leaked legacy
/// interrupt is unexpected instead of passing for an expiry.
pub(crate) const TIMER: InterruptVector = SOURCES[3];

/// What the local APIC is told to use for itself.
pub(crate) const LOCAL_APIC: Vectors = Vectors {
    spurious: SPURIOUS,
    error: ERROR,
    self_interrupt: PING,
    timer: TIMER,
};

/// The kernel's entry from ring 3, and the only gate in the table ring 3 may reach.
/// `0x80` is clear of the architectural range and of every APIC and I/O APIC vector.
pub(crate) const SYSCALL: InterruptVector = SOURCES[4];

/// The keyboard controller's line through the I/O APIC, clear of the legacy range 0x20 to 0x2F
/// so a leaked legacy interrupt is unexpected instead of passing for a keystroke.
pub(crate) const KEYBOARD: InterruptVector = SOURCES[5];
