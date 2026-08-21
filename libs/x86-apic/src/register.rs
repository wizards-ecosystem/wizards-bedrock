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

/// Local vector table entry for the APIC's own timer.
pub const LVT_TIMER: Register = Register::new(0x320);

/// Local vector table entry for local interrupt pin 0, which firmware
/// conventionally wires to the legacy controller.
pub const LVT_LINT0: Register = Register::new(0x350);

/// Local vector table entry for local interrupt pin 1, conventionally the
/// non-maskable interrupt.
pub const LVT_LINT1: Register = Register::new(0x360);

/// Local vector table entry for the APIC's own error line.
pub const LVT_ERROR: Register = Register::new(0x370);

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
        ("end of interrupt", END_OF_INTERRUPT),
        ("spurious interrupt vector", SPURIOUS_INTERRUPT_VECTOR),
        ("error status", ERROR_STATUS),
        ("command", COMMAND),
        ("lvt timer", LVT_TIMER),
        ("lvt lint0", LVT_LINT0),
        ("lvt lint1", LVT_LINT1),
        ("lvt error", LVT_ERROR),
        ("initial count", INITIAL_COUNT),
        ("current count", CURRENT_COUNT),
        ("divide configuration", DIVIDE_CONFIGURATION),
    ];

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

    /// The four local vector table entries are consecutive sixteen-byte slots
    /// bar the two this kernel does not touch — thermal at 0x330 and
    /// performance at 0x340 — which is the shape that makes a typo obvious.
    #[test]
    fn the_local_vector_entries_sit_where_the_manual_puts_them() {
        assert_eq!(LVT_TIMER.offset(), 0x320);
        assert_eq!(LVT_LINT0.offset(), LVT_TIMER.offset() + 3 * 16);
        assert_eq!(LVT_LINT1.offset(), LVT_LINT0.offset() + 16);
        assert_eq!(LVT_ERROR.offset(), LVT_LINT1.offset() + 16);
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
        for (name, register, msr) in [
            ("id", ID, 0x802),
            ("version", VERSION, 0x803),
            ("task priority", TASK_PRIORITY, 0x808),
            ("end of interrupt", END_OF_INTERRUPT, 0x80B),
            ("spurious", SPURIOUS_INTERRUPT_VECTOR, 0x80F),
            ("error status", ERROR_STATUS, 0x828),
            ("command", COMMAND, 0x830),
            ("lvt timer", LVT_TIMER, 0x832),
            ("lvt lint0", LVT_LINT0, 0x835),
            ("lvt lint1", LVT_LINT1, 0x836),
            ("lvt error", LVT_ERROR, 0x837),
            ("initial count", INITIAL_COUNT, 0x838),
            ("current count", CURRENT_COUNT, 0x839),
            ("divide configuration", DIVIDE_CONFIGURATION, 0x83E),
        ] {
            assert_eq!(register.msr(), msr, "{name}");
        }
    }
}
