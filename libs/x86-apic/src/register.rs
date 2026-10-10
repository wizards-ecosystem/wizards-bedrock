//! Where each register sits in the local APIC's page.
//!
//! Offsets are Intel SDM Vol. 3A table 11-1. Only the registers this kernel
//! touches are named; the rest are absent rather than listed and unused.

use crate::Register;

/// This APIC's identifier.
///
/// The whole register in x2APIC mode; the top byte of it in the memory-mapped
/// one. Another difference between the two encodings that only the reader of
/// this register has to care about.
pub const ID: Register = Register::new(0x020);

/// Version, and how many local vector table entries this APIC implements.
pub const VERSION: Register = Register::new(0x030);

/// Interrupts in this priority class or below are not delivered. Zero accepts
/// everything, which is what a kernel with no interrupt priorities wants.
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

/// Interrupt requests accepted by the local APIC but not yet delivered, one
/// word for each thirty-two-vector bank.
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

/// Written to dismiss the interrupt currently in service. Until it is, nothing
/// of the same priority class or lower can be delivered.
pub const END_OF_INTERRUPT: Register = Register::new(0x0B0);

/// The spurious vector, and the bit that software-enables the whole APIC.
pub const SPURIOUS_INTERRUPT_VECTOR: Register = Register::new(0x0F0);

/// What the APIC has gone wrong at. Latches on write, so it is written before
/// it is read.
pub const ERROR_STATUS: Register = Register::new(0x280);

/// The interrupt command register. Writing it sends the interrupt.
pub const COMMAND: Register = Register::new(0x300);

/// Local vector table entry for corrected machine-check interrupts.
const LVT_CMCI: Register = Register::new(0x2F0);

/// Local vector table entry for the APIC's own timer.
pub const LVT_TIMER: Register = Register::new(0x320);

/// Local vector table entry for the processor's thermal monitor.
const LVT_THERMAL: Register = Register::new(0x330);

/// Local vector table entry for performance-monitoring counter overflow.
const LVT_PERFORMANCE: Register = Register::new(0x340);

/// Local vector table entry for local interrupt pin 0, which firmware
/// conventionally wires to the legacy controller.
pub const LVT_LINT0: Register = Register::new(0x350);

/// Local vector table entry for local interrupt pin 1, conventionally the
/// non-maskable interrupt.
pub const LVT_LINT1: Register = Register::new(0x360);

/// Local vector table entry for the APIC's own error line.
pub const LVT_ERROR: Register = Register::new(0x370);

/// Local vector entries in the order used by the version register's maximum.
///
/// CMCI is entry six even though its register precedes the other entries in
/// address order.
pub const LOCAL_VECTOR_TABLE: [Register; 7] = [
    LVT_TIMER,
    LVT_THERMAL,
    LVT_PERFORMANCE,
    LVT_LINT0,
    LVT_LINT1,
    LVT_ERROR,
    LVT_CMCI,
];

/// The value the timer's countdown restarts from. Writing it starts the timer.
pub const INITIAL_COUNT: Register = Register::new(0x380);

/// Where the timer's countdown currently stands. Read-only.
pub const CURRENT_COUNT: Register = Register::new(0x390);

/// How the bus clock is divided before it drives the timer's countdown.
pub const DIVIDE_CONFIGURATION: Register = Register::new(0x3E0);

#[cfg(test)]
mod tests {
    use super::*;

    /// Every register named here, so the sweeps below cannot miss one.
    const ALL: &[(&str, Register)] = &[
        ("id", ID),
        ("version", VERSION),
        ("task priority", TASK_PRIORITY),
        ("in service 0", IN_SERVICE[0]),
        ("in service 1", IN_SERVICE[1]),
        ("in service 2", IN_SERVICE[2]),
        ("in service 3", IN_SERVICE[3]),
        ("in service 4", IN_SERVICE[4]),
        ("in service 5", IN_SERVICE[5]),
        ("in service 6", IN_SERVICE[6]),
        ("in service 7", IN_SERVICE[7]),
        ("interrupt request 0", INTERRUPT_REQUEST[0]),
        ("interrupt request 1", INTERRUPT_REQUEST[1]),
        ("interrupt request 2", INTERRUPT_REQUEST[2]),
        ("interrupt request 3", INTERRUPT_REQUEST[3]),
        ("interrupt request 4", INTERRUPT_REQUEST[4]),
        ("interrupt request 5", INTERRUPT_REQUEST[5]),
        ("interrupt request 6", INTERRUPT_REQUEST[6]),
        ("interrupt request 7", INTERRUPT_REQUEST[7]),
        ("end of interrupt", END_OF_INTERRUPT),
        ("spurious interrupt vector", SPURIOUS_INTERRUPT_VECTOR),
        ("error status", ERROR_STATUS),
        ("command", COMMAND),
        ("lvt cmci", LVT_CMCI),
        ("lvt timer", LVT_TIMER),
        ("lvt thermal", LVT_THERMAL),
        ("lvt performance", LVT_PERFORMANCE),
        ("lvt lint0", LVT_LINT0),
        ("lvt lint1", LVT_LINT1),
        ("lvt error", LVT_ERROR),
        ("initial count", INITIAL_COUNT),
        ("current count", CURRENT_COUNT),
        ("divide configuration", DIVIDE_CONFIGURATION),
    ];

    /// `ALL` names every register the crate defines.
    ///
    /// The comment above it says so, and nothing checked it. A fifteenth
    /// `pub const NAME: Register` added and left out would silently shrink
    /// three sweeps at once — the alignment check, the collision check and the
    /// MSR anchoring — and all three would go on reporting success (D-161).
    ///
    /// Compared by offset rather than by count: a list that dropped one
    /// register and repeated another has the right length, and only the
    /// offsets say so.
    #[test]
    fn all_names_every_register_the_crate_defines() {
        let mut defined: Vec<usize> = include_str!("register.rs")
            .lines()
            .filter_map(|line| line.split_once("Register::new(0x"))
            .filter_map(|(_, rest)| rest.split_once(')'))
            .filter_map(|(digits, _)| usize::from_str_radix(digits, 16).ok())
            .collect();
        assert!(
            defined.len() >= 14,
            "the scan read {} registers out of the source, so the scan is wrong",
            defined.len()
        );

        let mut listed: Vec<usize> = ALL.iter().map(|(_, register)| register.offset()).collect();
        defined.sort_unstable();
        listed.sort_unstable();
        assert_eq!(
            listed, defined,
            "ALL and the registers the crate defines are not the same set"
        );
    }

    /// `Register::new` enforces this, but it is a `const` assertion that fires
    /// at compile time — this is the check that would notice it being loosened.
    #[test]
    fn every_register_is_aligned_and_inside_the_page() {
        for (name, register) in ALL {
            assert_eq!(register.offset() % 16, 0, "{name} alignment");
            assert!(register.offset() < crate::REGISTER_BYTES, "{name} bounds");
        }
    }

    #[test]
    fn no_two_registers_share_an_offset() {
        for (index, (name, register)) in ALL.iter().enumerate() {
            for (other_name, other) in &ALL[index + 1..] {
                assert_ne!(register, other, "{name} and {other_name} collide");
            }
        }
    }

    /// Entries zero through five are consecutive slots. CMCI was added later
    /// at a lower address but remains entry six in the version-register order.
    #[test]
    fn the_local_vector_entries_sit_where_the_manual_puts_them() {
        assert_eq!(LVT_CMCI.offset(), 0x2F0);
        assert_eq!(LVT_TIMER.offset(), 0x320);
        assert_eq!(LVT_THERMAL.offset(), LVT_TIMER.offset() + 16);
        assert_eq!(LVT_PERFORMANCE.offset(), LVT_THERMAL.offset() + 16);
        assert_eq!(LVT_LINT0.offset(), LVT_PERFORMANCE.offset() + 16);
        assert_eq!(LVT_LINT1.offset(), LVT_LINT0.offset() + 16);
        assert_eq!(LVT_ERROR.offset(), LVT_LINT1.offset() + 16);
        assert_eq!(
            LOCAL_VECTOR_TABLE,
            [
                LVT_TIMER,
                LVT_THERMAL,
                LVT_PERFORMANCE,
                LVT_LINT0,
                LVT_LINT1,
                LVT_ERROR,
                LVT_CMCI,
            ]
        );
    }

    /// The two count registers are adjacent slots; the divide configuration
    /// sits apart from them, past the slots the architecture reserves.
    #[test]
    fn the_timer_registers_sit_where_the_manual_puts_them() {
        assert_eq!(INITIAL_COUNT.offset(), 0x380);
        assert_eq!(CURRENT_COUNT.offset(), INITIAL_COUNT.offset() + 16);
        assert_eq!(DIVIDE_CONFIGURATION.offset(), 0x3E0);
    }

    /// SDM Vol. 3A table 11-1's "MSR address" column, for every register named
    /// here. Transcribed rather than derived, so the derivation has something
    /// independent to be wrong against.
    #[test]
    fn every_register_lands_on_the_msr_the_manual_gives() {
        let anchored = [
            ("id", ID, 0x802),
            ("version", VERSION, 0x803),
            ("task priority", TASK_PRIORITY, 0x808),
            ("in service 0", IN_SERVICE[0], 0x810),
            ("in service 1", IN_SERVICE[1], 0x811),
            ("in service 2", IN_SERVICE[2], 0x812),
            ("in service 3", IN_SERVICE[3], 0x813),
            ("in service 4", IN_SERVICE[4], 0x814),
            ("in service 5", IN_SERVICE[5], 0x815),
            ("in service 6", IN_SERVICE[6], 0x816),
            ("in service 7", IN_SERVICE[7], 0x817),
            ("interrupt request 0", INTERRUPT_REQUEST[0], 0x820),
            ("interrupt request 1", INTERRUPT_REQUEST[1], 0x821),
            ("interrupt request 2", INTERRUPT_REQUEST[2], 0x822),
            ("interrupt request 3", INTERRUPT_REQUEST[3], 0x823),
            ("interrupt request 4", INTERRUPT_REQUEST[4], 0x824),
            ("interrupt request 5", INTERRUPT_REQUEST[5], 0x825),
            ("interrupt request 6", INTERRUPT_REQUEST[6], 0x826),
            ("interrupt request 7", INTERRUPT_REQUEST[7], 0x827),
            ("end of interrupt", END_OF_INTERRUPT, 0x80B),
            ("spurious", SPURIOUS_INTERRUPT_VECTOR, 0x80F),
            ("error status", ERROR_STATUS, 0x828),
            ("command", COMMAND, 0x830),
            ("lvt cmci", LVT_CMCI, 0x82F),
            ("lvt timer", LVT_TIMER, 0x832),
            ("lvt thermal", LVT_THERMAL, 0x833),
            ("lvt performance", LVT_PERFORMANCE, 0x834),
            ("lvt lint0", LVT_LINT0, 0x835),
            ("lvt lint1", LVT_LINT1, 0x836),
            ("lvt error", LVT_ERROR, 0x837),
            ("initial count", INITIAL_COUNT, 0x838),
            ("current count", CURRENT_COUNT, 0x839),
            ("divide configuration", DIVIDE_CONFIGURATION, 0x83E),
        ];

        // Tied to `ALL`: a register this list forgets has no manual value
        // behind it, and the loop alone would never say so.
        assert_eq!(
            anchored.len(),
            ALL.len(),
            "a register is defined without an MSR anchored to the manual"
        );
        for (name, register, msr) in anchored {
            assert_eq!(register.msr(), msr, "{name}");
        }
    }
}
