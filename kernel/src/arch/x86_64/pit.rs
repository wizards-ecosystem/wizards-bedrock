//! 8254 interval timer driver: the crystal the APIC timer is calibrated and validated against.
//! It is never an interrupt source: counter 0's output goes to a masked legacy controller.

use core::fmt;

use wost::arch::current::port::{self, PortBlock};
use x86_pit::STATUS_NULL_COUNT;
use x86_pit::command::{Access, Channel, ControlWord, OperatingMode};

/// Ports occupied by the chip: three counters, then the control register.
const REGISTER_COUNT: usize = 4;

/// Register offsets from the port base.
mod register {
    pub(super) const CHANNEL_0: usize = 0;
    pub(super) const CONTROL: usize = 3;
}

/// The longest countdown the sixteen-bit counter holds.
const FULL_SPAN: u16 = 0xFFFF;

/// The one word this driver programs with: counter 0, both count bytes, mode 0, binary.
const MEASURE: ControlWord = ControlWord::binary(
    Channel::Zero,
    Access::LowHigh,
    OperatingMode::InterruptOnTerminalCount,
);

/// Why the timer could not be claimed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

    /// Start counter 0 down its full span, and wait until the count reaches the counting element.
    /// Mode 0 decrements monotonically through zero, with no reload inside a measurement window.
    /// Real chips load the count up to 838 ns after the high byte, and an earlier baseline reads
    /// stale; both hypervisors load instantly, so only hardware needs the wait.
    /// The wait is bounded: a null-count flag that never clears leaves the measurement guards to
    /// judge the baseline.
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

    /// Halt the counter for good: a control word stops it until an initial count is written,
    /// which never comes, because this consumes the driver and its port claim is permanent.
    pub(crate) fn park(mut self) {
        PortBlock::write(&mut self.registers, register::CONTROL, MEASURE.bits());
    }
}
