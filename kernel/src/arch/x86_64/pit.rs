//! 8254 interval timer, the calibration reference.
//!
//! Never an interrupt source: counter 0 feeds a masked legacy controller.

use core::fmt;

use wost::arch::current::port::{self, PortBlock};
use x86_pit::STATUS_NULL_COUNT;
use x86_pit::command::{Access, Channel, ControlWord, OperatingMode};

const REGISTER_COUNT: usize = 4;

mod register {
    pub(super) const CHANNEL_0: usize = 0;
    pub(super) const CONTROL: usize = 3;
}

const FULL_SPAN: u16 = 0xFFFF;

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
        let registers = port::pit().ok_or(PitError::PortsUnavailable)?;
        Ok(Self { registers })
    }

    /// Start counter 0 counting down through its full span, and wait until the count has actually
    /// reached the counting element. The wait exists for real hardware: the chip loads a written
    /// count on the crystal edge after the high byte, up to 838 ns later, and a baseline latched
    /// before that reads the stale pre-load value — a corruption both hypervisors are immune to,
    /// because their virtual counters load instantly.
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

    /// Halt the counter for good. A control word stops the count, and nothing counts again until an
    /// initial count is written — which never comes: this consumes the driver, and the port claim
    /// it held is permanent, so nothing can ever program the chip again.
    pub(crate) fn park(mut self) {
        PortBlock::write(&mut self.registers, register::CONTROL, MEASURE.bits());
    }
}
