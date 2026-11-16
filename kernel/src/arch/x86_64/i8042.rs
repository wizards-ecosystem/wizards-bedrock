use core::fmt;
use wost::arch::current::port::{self, Port};

mod status {
    pub(super) const OUTPUT_FULL: u8 = 1 << 0;
    pub(super) const INPUT_FULL: u8 = 1 << 1;
    pub(super) const FROM_SECOND_PORT: u8 = 1 << 5;
}

mod command {
    pub(super) const READ_CONFIG: u8 = 0x20;
    pub(super) const WRITE_CONFIG: u8 = 0x60;
    pub(super) const DISABLE_SECOND: u8 = 0xA7;
    pub(super) const SELF_TEST: u8 = 0xAA;
    pub(super) const TEST_FIRST: u8 = 0xAB;
    pub(super) const DISABLE_FIRST: u8 = 0xAD;
    pub(super) const ENABLE_FIRST: u8 = 0xAE;
}

mod config {
    pub(super) const FIRST_IRQ: u8 = 1 << 0;
    pub(super) const SECOND_IRQ: u8 = 1 << 1;
    pub(super) const TRANSLATE: u8 = 1 << 6;
}

mod device {
    pub(super) const IDENTIFY: u8 = 0xF2;
    pub(super) const ENABLE_SCANNING: u8 = 0xF4;
    pub(super) const RESET: u8 = 0xFF;
}

mod reply {
    pub(super) const INTERFACE_OK: u8 = 0x00;
    pub(super) const SELF_TEST_OK: u8 = 0x55;
    pub(super) const ACK: u8 = 0xFA;
    pub(super) const TEST_PASSED: u8 = 0xAA;
}

// An MF2 keyboard behind a translating controller; VMware sends it with the translate bit clear.
const TRANSLATED_MF2: (u8, u8) = (0xAB, 0x41);

const ABSENT_STATUS: u8 = 0xFF;

const FLUSH_LIMIT: u32 = 32;

const RESPONSE_ATTEMPTS: u32 = 100_000;

// The protocol allows the self-test after a reset hundreds of milliseconds.
const RESET_ATTEMPTS: u32 = 2_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BringUpError {
    PortsUnavailable,
    NotPresent,
    SelfTestFailed,
    InterfaceTestFailed,
    ConfigurationRefused,
    TransportRefused,
    KeyboardMissing,
    CommandRefused,
}

impl fmt::Display for BringUpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PortsUnavailable => "its ports are already claimed",
            Self::NotPresent => "no controller answered on the status port",
            Self::SelfTestFailed => "the controller failed its self-test",
            Self::InterfaceTestFailed => "the keyboard port failed its interface test",
            Self::ConfigurationRefused => "the controller refused its configuration byte",
            Self::TransportRefused => "the controller transport refused a command",
            Self::KeyboardMissing => "no keyboard acknowledged the reset",
            Self::CommandRefused => "the keyboard refused a configuration command",
        })
    }
}

impl core::error::Error for BringUpError {}

#[derive(Debug)]
pub(crate) struct Controller {
    data: Port<u8>,
    control: Port<u8>,
    translated: bool,
}

impl Controller {
    pub(crate) fn new() -> Result<Self, BringUpError> {
        let (data, control) = port::i8042().ok_or(BringUpError::PortsUnavailable)?;
        let mut controller = Self {
            data,
            control,
            translated: false,
        };

        if Port::read(&mut controller.control) == ABSENT_STATUS {
            return Err(BringUpError::NotPresent);
        }
        controller.require_controller_command(command::DISABLE_FIRST)?;
        controller.require_controller_command(command::DISABLE_SECOND)?;
        if !controller.flush() {
            return Err(BringUpError::NotPresent);
        }

        let configured = controller
            .require_controller_query(command::READ_CONFIG, BringUpError::NotPresent)?
            & !(config::FIRST_IRQ | config::SECOND_IRQ | config::TRANSLATE);
        controller.require_config(configured)?;

        if controller.require_controller_query(command::SELF_TEST, BringUpError::SelfTestFailed)?
            != reply::SELF_TEST_OK
        {
            return Err(BringUpError::SelfTestFailed);
        }
        // Some controllers reset their configuration byte during the self-test.
        controller.require_config(configured)?;

        if controller
            .require_controller_query(command::TEST_FIRST, BringUpError::InterfaceTestFailed)?
            != reply::INTERFACE_OK
        {
            return Err(BringUpError::InterfaceTestFailed);
        }
        controller.require_controller_command(command::ENABLE_FIRST)?;

        if !controller.reset_keyboard() {
            return Err(BringUpError::KeyboardMissing);
        }
        // Asked before scanning starts, or the reply would interleave with key data.
        controller.translated = controller.identity() == Some(TRANSLATED_MF2);
        if !controller.device_command(device::ENABLE_SCANNING) {
            return Err(BringUpError::CommandRefused);
        }
        Ok(controller)
    }

    // Bounded: a dead controller on a floating bus reads full and from the second port forever.
    pub(crate) fn poll_byte(&mut self) -> Option<u8> {
        for _ in 0..FLUSH_LIMIT {
            let status = Port::read(&mut self.control);
            if status & status::OUTPUT_FULL == 0 {
                return None;
            }
            let byte = Port::read(&mut self.data);
            if status & status::FROM_SECOND_PORT == 0 {
                return Some(byte);
            }
        }
        None
    }

    pub(crate) const fn is_translated(&self) -> bool {
        self.translated
    }

    fn identity(&mut self) -> Option<(u8, u8)> {
        if !self.flush() {
            return None;
        }
        if !self.device_command(device::IDENTIFY) {
            return None;
        }
        let first = self.read_reply(RESPONSE_ATTEMPTS)?;
        let second = self.read_reply(RESPONSE_ATTEMPTS)?;
        Some((first, second))
    }

    // Emulators disagree about an acknowledgement before the self-test result, so both shapes pass.
    fn reset_keyboard(&mut self) -> bool {
        if !self.write_data(device::RESET) {
            return false;
        }
        match self.read_reply(RESET_ATTEMPTS) {
            Some(reply::TEST_PASSED) => true,
            Some(reply::ACK) => self.read_reply(RESET_ATTEMPTS) == Some(reply::TEST_PASSED),
            _ => false,
        }
    }

    fn device_command(&mut self, byte: u8) -> bool {
        self.write_data(byte) && self.read_reply(RESPONSE_ATTEMPTS) == Some(reply::ACK)
    }

    fn controller_command(&mut self, command: u8) -> bool {
        if !self.wait_writable() {
            return false;
        }
        Port::write(&mut self.control, command);
        true
    }

    fn require_controller_command(&mut self, command: u8) -> Result<(), BringUpError> {
        self.controller_command(command)
            .then_some(())
            .ok_or(BringUpError::TransportRefused)
    }

    fn require_controller_query(
        &mut self,
        command: u8,
        no_reply: BringUpError,
    ) -> Result<u8, BringUpError> {
        self.require_controller_command(command)?;
        self.read_reply(RESPONSE_ATTEMPTS).ok_or(no_reply)
    }

    fn read_config(&mut self) -> Option<u8> {
        if !self.controller_command(command::READ_CONFIG) {
            return None;
        }
        self.read_reply(RESPONSE_ATTEMPTS)
    }

    fn require_config(&mut self, value: u8) -> Result<(), BringUpError> {
        let kept = self.controller_command(command::WRITE_CONFIG)
            && self.write_data(value)
            && self.read_config() == Some(value);
        kept.then_some(()).ok_or(BringUpError::ConfigurationRefused)
    }

    fn flush(&mut self) -> bool {
        drains(
            FLUSH_LIMIT,
            || Port::read(&mut self.control) & status::OUTPUT_FULL != 0,
            || {
                let _ = Port::read(&mut self.data);
            },
        )
    }

    fn write_data(&mut self, byte: u8) -> bool {
        if !self.wait_writable() {
            return false;
        }
        Port::write(&mut self.data, byte);
        true
    }

    fn read_reply(&mut self, attempts: u32) -> Option<u8> {
        if self.wait_readable(attempts) {
            Some(Port::read(&mut self.data))
        } else {
            None
        }
    }

    fn wait_readable(&mut self, attempts: u32) -> bool {
        (0..attempts).any(|_| Port::read(&mut self.control) & status::OUTPUT_FULL != 0)
    }

    fn wait_writable(&mut self) -> bool {
        (0..RESPONSE_ATTEMPTS).any(|_| Port::read(&mut self.control) & status::INPUT_FULL == 0)
    }
}

fn drains(limit: u32, mut full: impl FnMut() -> bool, mut discard: impl FnMut()) -> bool {
    for _ in 0..limit {
        if !full() {
            return true;
        }
        discard();
    }
    !full()
}
