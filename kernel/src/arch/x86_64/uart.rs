use core::fmt;

use wost::arch::current::port::{self, PortBlock};

const REGISTER_COUNT: usize = 8;

mod register {
    pub(super) const DATA: usize = 0;
    pub(super) const DIVISOR_LOW: usize = 0;
    pub(super) const INTERRUPT_ENABLE: usize = 1;
    pub(super) const DIVISOR_HIGH: usize = 1;
    pub(super) const FIFO_CONTROL: usize = 2;
    pub(super) const LINE_CONTROL: usize = 3;
    pub(super) const MODEM_CONTROL: usize = 4;
    pub(super) const LINE_STATUS: usize = 5;
}

mod line_control {
    pub(super) const WORD_8N1: u8 = 0x03;
    pub(super) const DIVISOR_LATCH: u8 = 0x80;
}

mod fifo_control {
    pub(super) const ENABLE: u8 = 0x01;
    pub(super) const CLEAR_RECEIVE: u8 = 0x02;
    pub(super) const CLEAR_TRANSMIT: u8 = 0x04;
    pub(super) const TRIGGER_14_BYTES: u8 = 0xC0;
}

mod modem_control {
    pub(super) const DATA_TERMINAL_READY: u8 = 0x01;
    pub(super) const REQUEST_TO_SEND: u8 = 0x02;
    pub(super) const OUT1: u8 = 0x04;
    pub(super) const OUT2: u8 = 0x08;
    pub(super) const LOOPBACK: u8 = 0x10;
}

mod line_status {
    pub(super) const DATA_READY: u8 = 0x01;
    pub(super) const TRANSMIT_HOLDING_EMPTY: u8 = 0x20;
}

// The 1.8432 MHz reference clock over 16 is 115200 baud.
const DIVISOR_115200: u16 = 1;

const PROBE_BYTE: u8 = 0xAE;

const MODEM_ONLINE: u8 =
    modem_control::DATA_TERMINAL_READY | modem_control::REQUEST_TO_SEND | modem_control::OUT2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum UartError {
    PortsUnavailable,
    NotPresent,
}

impl fmt::Display for UartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PortsUnavailable => "its ports are already claimed",
            Self::NotPresent => "no controller answered the loopback probe",
        })
    }
}

impl core::error::Error for UartError {}

#[derive(Debug)]
pub(crate) struct Uart {
    registers: PortBlock<REGISTER_COUNT>,
}

impl Uart {
    pub(crate) fn new() -> Result<Self, UartError> {
        let registers = port::com1().ok_or(UartError::PortsUnavailable)?;
        let mut uart = Self { registers };
        uart.configure();
        if uart.probe() {
            Ok(uart)
        } else {
            Err(UartError::NotPresent)
        }
    }

    fn configure(&mut self) {
        let divisor = DIVISOR_115200.to_le_bytes();
        PortBlock::write(&mut self.registers, register::INTERRUPT_ENABLE, 0u8);
        PortBlock::write(
            &mut self.registers,
            register::LINE_CONTROL,
            line_control::DIVISOR_LATCH,
        );
        PortBlock::write(&mut self.registers, register::DIVISOR_LOW, divisor[0]);
        PortBlock::write(&mut self.registers, register::DIVISOR_HIGH, divisor[1]);
        PortBlock::write(
            &mut self.registers,
            register::LINE_CONTROL,
            line_control::WORD_8N1,
        );
        PortBlock::write(
            &mut self.registers,
            register::FIFO_CONTROL,
            fifo_control::ENABLE
                | fifo_control::CLEAR_RECEIVE
                | fifo_control::CLEAR_TRANSMIT
                | fifo_control::TRIGGER_14_BYTES,
        );
        PortBlock::write(&mut self.registers, register::MODEM_CONTROL, MODEM_ONLINE);
    }

    fn probe(&mut self) -> bool {
        PortBlock::write(
            &mut self.registers,
            register::MODEM_CONTROL,
            MODEM_ONLINE | modem_control::OUT1 | modem_control::LOOPBACK,
        );
        PortBlock::write(&mut self.registers, register::DATA, PROBE_BYTE);

        let echoed = self.wait_for(line_status::DATA_READY)
            && PortBlock::read::<u8>(&mut self.registers, register::DATA) == PROBE_BYTE;

        PortBlock::write(&mut self.registers, register::MODEM_CONTROL, MODEM_ONLINE);
        echoed
    }

    fn write_byte(&mut self, byte: u8) -> fmt::Result {
        if !self.wait_for(line_status::TRANSMIT_HOLDING_EMPTY) {
            return Err(fmt::Error);
        }
        PortBlock::write(&mut self.registers, register::DATA, byte);
        Ok(())
    }

    fn wait_for(&mut self, flags: u8) -> bool {
        const ATTEMPTS: u32 = 100_000;
        (0..ATTEMPTS)
            .any(|_| PortBlock::read::<u8>(&mut self.registers, register::LINE_STATUS) & flags != 0)
    }
}

impl fmt::Write for Uart {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r')?;
            }
            self.write_byte(byte)?;
        }
        Ok(())
    }
}
