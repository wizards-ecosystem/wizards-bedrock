//! Where each register this kernel touches sits in the APIC page, per SDM Vol. 3A table 11-1.

use crate::Register;

/// This APIC's identifier: the whole register in x2APIC mode, its top byte in xAPIC mode.
pub const ID: Register = Register::new(0x020);

/// Version, and how many local vector table entries this APIC implements.
pub const VERSION: Register = Register::new(0x030);

/// Interrupts in this priority class or below are not delivered.
pub const TASK_PRIORITY: Register = Register::new(0x080);

/// Interrupt-service state, one word for each thirty-two-vector bank.
pub const IN_SERVICE: [Register; 8] = [
    Register::new(0x100),
    Register::new(0x110),
    Register::new(0x120),
    Register::new(0x130),
    Register::new(0x140),
    Register::new(0x150),
    Register::new(0x160),
    Register::new(0x170),
];

/// Requests accepted but not yet delivered, one word for each thirty-two-vector bank.
pub const INTERRUPT_REQUEST: [Register; 8] = [
    Register::new(0x200),
    Register::new(0x210),
    Register::new(0x220),
    Register::new(0x230),
    Register::new(0x240),
    Register::new(0x250),
    Register::new(0x260),
    Register::new(0x270),
];

/// Written to dismiss the interrupt in service; until then its class and lower stay blocked.
pub const END_OF_INTERRUPT: Register = Register::new(0x0B0);

/// The spurious vector, and the bit that software-enables the whole APIC.
pub const SPURIOUS_INTERRUPT_VECTOR: Register = Register::new(0x0F0);

/// Latches on write, so it is written before it is read.
pub const ERROR_STATUS: Register = Register::new(0x280);

/// Writing it sends the interrupt.
pub const COMMAND: Register = Register::new(0x300);

const LVT_CMCI: Register = Register::new(0x2F0);

pub const LVT_TIMER: Register = Register::new(0x320);

const LVT_THERMAL: Register = Register::new(0x330);

const LVT_PERFORMANCE: Register = Register::new(0x340);

/// Local interrupt pin 0, which firmware conventionally wires to the legacy controller.
pub const LVT_LINT0: Register = Register::new(0x350);

/// Local interrupt pin 1, conventionally the non-maskable interrupt.
pub const LVT_LINT1: Register = Register::new(0x360);

pub const LVT_ERROR: Register = Register::new(0x370);

/// Local vector entries in version-register order, with CMCI sixth despite its lower address.
pub const LOCAL_VECTOR_TABLE: [Register; 7] = [
    LVT_TIMER,
    LVT_THERMAL,
    LVT_PERFORMANCE,
    LVT_LINT0,
    LVT_LINT1,
    LVT_ERROR,
    LVT_CMCI,
];

/// Writing it starts the timer's countdown from this value.
pub const INITIAL_COUNT: Register = Register::new(0x380);

/// Where the timer's countdown currently stands; read-only.
pub const CURRENT_COUNT: Register = Register::new(0x390);

/// How the bus clock is divided before it drives the timer's countdown.
pub const DIVIDE_CONFIGURATION: Register = Register::new(0x3E0);

#[cfg(test)]
mod tests;
