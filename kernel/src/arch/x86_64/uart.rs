//! 16550 UART driver — the kernel's serial console.
//!
//! QEMU maps COM1 to stdio (`-serial stdio`); VMware maps it to `vm/serial.log`.

use core::fmt;

use wost::arch::current::port::{self, PortBlock};

/// I/O port base of the first serial controller.
///
/// `wost::arch::current::port::com1` owns this number for the driver; the
/// self-test keeps it to probe that the driver's ports are exclusively held.
#[cfg(feature = "selftest")]
pub(crate) const COM1: u16 = 0x3F8;

/// Ports occupied by one controller.
const REGISTER_COUNT: usize = 8;

/// Register offsets from the port base.
///
/// Registers 0 and 1 change meaning while [`line_control::DIVISOR_LATCH`] is set.
mod register {
    pub(crate) const DATA: usize = 0;
    pub(crate) const DIVISOR_LOW: usize = 0;
    pub(crate) const INTERRUPT_ENABLE: usize = 1;
    pub(crate) const DIVISOR_HIGH: usize = 1;
    pub(crate) const FIFO_CONTROL: usize = 2;
    pub(crate) const LINE_CONTROL: usize = 3;
    pub(crate) const MODEM_CONTROL: usize = 4;
    pub(crate) const LINE_STATUS: usize = 5;
}

mod line_control {
    /// Eight data bits, no parity, one stop bit.
    pub(crate) const WORD_8N1: u8 = 0x03;
    /// Registers 0 and 1 become the baud-rate divisor while this is set.
    pub(crate) const DIVISOR_LATCH: u8 = 0x80;
}

mod fifo_control {
    pub(crate) const ENABLE: u8 = 0x01;
    pub(crate) const CLEAR_RECEIVE: u8 = 0x02;
    pub(crate) const CLEAR_TRANSMIT: u8 = 0x04;
    pub(crate) const TRIGGER_14_BYTES: u8 = 0xC0;
}

mod modem_control {
    pub(crate) const DATA_TERMINAL_READY: u8 = 0x01;
    pub(crate) const REQUEST_TO_SEND: u8 = 0x02;
    pub(crate) const OUT1: u8 = 0x04;
    /// Gates the controller's interrupt line through to the PIC. Nothing is
    /// actually delivered until the interrupt-enable register is non-zero.
    pub(crate) const OUT2: u8 = 0x08;
    /// Route transmitted bytes straight back into the receiver.
    pub(crate) const LOOPBACK: u8 = 0x10;
}

mod line_status {
    pub(crate) const DATA_READY: u8 = 0x01;
    pub(crate) const TRANSMIT_HOLDING_EMPTY: u8 = 0x20;
}

/// Baud divisor for 115200 baud from the 1.8432 MHz reference clock.
const DIVISOR_115200: u16 = 1;

/// Byte written and read back to prove a controller is present.
const PROBE_BYTE: u8 = 0xAE;

/// Normal operating state of the modem-control register.
const MODEM_ONLINE: u8 =
    modem_control::DATA_TERMINAL_READY | modem_control::REQUEST_TO_SEND | modem_control::OUT2;

/// Why a controller could not be brought up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UartError {
    /// Another driver already owns these ports.
    PortsUnavailable,
    /// The loopback probe came back wrong, or not at all.
    NotPresent,
}

impl fmt::Display for UartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortsUnavailable => {
                core::fmt::Write::write_str(f, "its ports are already claimed")
            }
            Self::NotPresent => {
                core::fmt::Write::write_str(f, "no controller answered the loopback probe")
            }
        }
    }
}

impl core::error::Error for UartError {}

/// A 16550-compatible serial controller.
#[derive(Debug)]
pub(crate) struct Uart {
    registers: PortBlock<REGISTER_COUNT>,
}

impl Uart {
    /// Claim and configure the first serial controller for 115200 baud, 8N1.
    ///
    /// Interrupts stay masked: this is a polled driver until the kernel has an
    /// IDT. The probe runs after configuration, so a machine without a
    /// controller reports [`UartError::NotPresent`] rather than silently
    /// swallowing every log line.
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

    /// Loop the transmitter back to the receiver and check a byte survives.
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

    /// Poll the line-status register until `flags` are set, giving up rather
    /// than hanging the kernel on a controller that has stopped answering.
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
