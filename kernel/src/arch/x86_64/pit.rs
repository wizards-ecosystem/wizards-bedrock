//! 8254 interval timer driver — the calibration reference.
//!
//! The kernel measures the APIC timer against a crystal whose rate has been
//! fixed since 1981, then retains the claimed chip to validate APIC-delivered
//! clock windows. It is never an interrupt source: counter 0's output goes to
//! a masked legacy controller, so each bounded window can count without
//! generating an interrupt.

use core::fmt;

use wost::arch::current::port::{self, PortBlock};
use x86_pit::STATUS_NULL_COUNT;
use x86_pit::command::{Access, Channel, ControlWord, OperatingMode};

/// I/O port base: three counters, then the control register.
const CHANNEL_BASE: u16 = 0x40;

/// Ports occupied by the chip.
const REGISTER_COUNT: usize = 4;

/// Register offsets from the port base.
mod register {
    pub(super) const CHANNEL_0: usize = 0;
    pub(super) const CONTROL: usize = 3;
}

/// The longest countdown the sixteen-bit counter holds.
const FULL_SPAN: u16 = 0xFFFF;

/// The one word this driver programs with: counter 0, both count bytes,
/// mode 0, binary.
const MEASURE: ControlWord = ControlWord::binary(
    Channel::Zero,
    Access::LowHigh,
    OperatingMode::InterruptOnTerminalCount,
);

/// Why the timer could not be claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PitError {
    /// Another driver already owns these ports.
    PortsUnavailable,
}

impl fmt::Display for PitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortsUnavailable => {
                core::fmt::Write::write_str(f, "its ports are already claimed")
            }
        }
    }
}

impl core::error::Error for PitError {}

/// The interval timer, claimed for calibration.
#[derive(Debug)]
pub(crate) struct Pit {
    registers: PortBlock<REGISTER_COUNT>,
}

impl Pit {
    /// Claim the chip's ports.
    pub(crate) fn claim() -> Result<Self, PitError> {
        let registers = port::claim_block(CHANNEL_BASE).ok_or(PitError::PortsUnavailable)?;
        Ok(Self { registers })
    }

    /// Start counter 0 counting down through its full span, and wait until
    /// the count has actually reached the counting element.
    ///
    /// Mode 0 decrements by one, monotonically, and keeps counting through
    /// zero — no reload to reason about inside a measurement window, and the
    /// single output edge at zero lands on a masked controller.
    ///
    /// The wait exists for real hardware: the chip loads a written count on
    /// the crystal edge after the high byte, up to 838 ns later, and a
    /// baseline latched before that reads the stale pre-load value — a
    /// corruption both hypervisors are immune to, because their virtual
    /// counters load instantly. Bounded, like every wait in this kernel: a
    /// chip whose null-count flag never clears simply yields a baseline the
    /// measurement guards will judge.
    pub(crate) fn start_measurement(&mut self) {
        const LOAD_ATTEMPTS: u32 = 1_000;

        PortBlock::write(&mut self.registers, register::CONTROL, MEASURE.bits());
        let count = FULL_SPAN.to_le_bytes();
        PortBlock::write(&mut self.registers, register::CHANNEL_0, count[0]);
        PortBlock::write(&mut self.registers, register::CHANNEL_0, count[1]);

        for _ in 0..LOAD_ATTEMPTS {
            PortBlock::write(
                &mut self.registers,
                register::CONTROL,
                ControlWord::read_back_status(Channel::Zero).bits(),
            );
            let status = PortBlock::read::<u8>(&mut self.registers, register::CHANNEL_0);
            if status & STATUS_NULL_COUNT == 0 {
                return;
            }
        }
    }

    /// Where counter 0 stands, latched so the two byte reads cohere.
    pub(crate) fn count(&mut self) -> u16 {
        PortBlock::write(
            &mut self.registers,
            register::CONTROL,
            ControlWord::latch(Channel::Zero).bits(),
        );
        let low = PortBlock::read::<u8>(&mut self.registers, register::CHANNEL_0);
        let high = PortBlock::read::<u8>(&mut self.registers, register::CHANNEL_0);
        u16::from_le_bytes([low, high])
    }

    /// Halt the counter for good.
    ///
    /// A control word stops the count, and nothing counts again until an
    /// initial count is written — which never comes: this consumes the
    /// driver, and the port claim it held is permanent, so nothing can ever
    /// program the chip again.
    pub(crate) fn park(mut self) {
        PortBlock::write(&mut self.registers, register::CONTROL, MEASURE.bits());
    }
}
