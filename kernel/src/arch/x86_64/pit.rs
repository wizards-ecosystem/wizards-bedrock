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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ClaimError {
    PortsUnavailable,
}

impl fmt::Display for ClaimError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PortsUnavailable => "its ports are already claimed",
        })
    }
}

impl core::error::Error for ClaimError {}

#[derive(Debug)]
pub(crate) struct Pit {
    registers: PortBlock<REGISTER_COUNT>,
}

impl Pit {
    pub(crate) fn claim() -> Result<Self, ClaimError> {
        let registers = port::pit().ok_or(ClaimError::PortsUnavailable)?;
        Ok(Self { registers })
    }

    // A real 8254 loads the count up to 838 ns after the high byte; an earlier baseline is stale.
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

    // A control word stops the counter until a count is written, and this driver writes none.
    pub(crate) fn park(mut self) {
        PortBlock::write(&mut self.registers, register::CONTROL, MEASURE.bits());
    }
}
