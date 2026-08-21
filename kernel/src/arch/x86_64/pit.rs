//! 8254 interval timer driver — the calibration reference.
//!
//! The kernel touches this chip exactly once, to measure the APIC timer
//! against a crystal whose rate has been fixed since 1981, and then parks it.
//! It is never an interrupt source: counter 0's output goes to a legacy
//! controller slice 2 masked, so it can count without anything hearing it.

use core::fmt;

use wost::arch::current::port::{self, PortBlock};
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
            Self::PortsUnavailable => f.write_str("its ports are already claimed"),
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

    /// Start counter 0 counting down through its full span.
    ///
    /// Mode 0 decrements by one, monotonically, and keeps counting through
    /// zero — no reload to reason about inside a measurement window, and the
    /// single output edge at zero lands on a masked controller.
    pub(crate) fn start_measurement(&mut self) {
        self.registers.write(register::CONTROL, MEASURE.bits());
        let count = FULL_SPAN.to_le_bytes();
        self.registers.write(register::CHANNEL_0, count[0]);
        self.registers.write(register::CHANNEL_0, count[1]);
    }

    /// Where counter 0 stands, latched so the two byte reads cohere.
    pub(crate) fn count(&mut self) -> u16 {
        self.registers
            .write(register::CONTROL, ControlWord::latch(Channel::Zero).bits());
        let low = self.registers.read::<u8>(register::CHANNEL_0);
        let high = self.registers.read::<u8>(register::CHANNEL_0);
        u16::from_le_bytes([low, high])
    }

    /// Halt the counter for good.
    ///
    /// A control word stops the count, and nothing counts again until an
    /// initial count is written — which never comes: this consumes the
    /// driver, and the port claim it held is permanent, so nothing can ever
    /// program the chip again.
    pub(crate) fn park(mut self) {
        self.registers.write(register::CONTROL, MEASURE.bits());
    }
}
