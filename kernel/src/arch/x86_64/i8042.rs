//! Intel 8042 keyboard controller driver — polled, silent, raw set 2.
//!
//! Configured to raise no interrupts at all: IRQ 1's two routes to this CPU
//! are the legacy controllers slice 2 silenced and an I/O APIC that is
//! memory-mapped behind Phase 3's missing page tables, so the tick-woken
//! idle loop drains the controller instead (D-37). Translation is switched
//! off too — the decoder in `ps2-keys` speaks set 2 as the keyboard sends
//! it, not the set 1 the controller would fake for a 1980s BIOS.
//!
//! Every wait is bounded, in the 16550 driver's idiom: a controller that
//! stops answering costs a timeout and a decline, never a hang.

use core::fmt;

use wost::arch::current::port::{self, Port};

/// I/O port the device's bytes arrive on, and device commands leave by.
const DATA: u16 = 0x60;

/// Status when read; commands to the controller itself when written.
const CONTROL: u16 = 0x64;

mod status {
    /// A byte is waiting in the output buffer.
    pub(super) const OUTPUT_FULL: u8 = 1 << 0;
    /// The controller has not yet consumed the last byte written to it.
    pub(super) const INPUT_FULL: u8 = 1 << 1;
    /// The waiting byte came from the second port — the mouse.
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
    /// Raise IRQ 1 when a first-port byte arrives.
    pub(super) const FIRST_IRQ: u8 = 1 << 0;
    /// Raise IRQ 12 when a second-port byte arrives.
    pub(super) const SECOND_IRQ: u8 = 1 << 1;
    /// Translate the keyboard's set 2 into set 1 — the compatibility shim
    /// firmware leaves on.
    pub(super) const TRANSLATE: u8 = 1 << 6;
}

mod device {
    pub(super) const ECHO: u8 = 0xEE;
    pub(super) const SET_SCANCODE_SET: u8 = 0xF0;
    pub(super) const ENABLE_SCANNING: u8 = 0xF4;
    pub(super) const RESET: u8 = 0xFF;
}

mod reply {
    pub(super) const INTERFACE_OK: u8 = 0x00;
    pub(super) const SELF_TEST_OK: u8 = 0x55;
    pub(super) const ACK: u8 = 0xFA;
    /// The keyboard's basic assurance test passed — follows a reset.
    pub(super) const TEST_PASSED: u8 = 0xAA;
    pub(super) const ECHO: u8 = 0xEE;
}

/// The set this kernel decodes, as the argument to the selection command.
const SCANCODE_SET_2: u8 = 0x02;

/// Selection argument that reads the current set back instead of setting one.
#[cfg(feature = "selftest")]
const CURRENT_SET: u8 = 0x00;

/// A missing controller floats the bus high, so the status register reads as
/// this forever.
const ABSENT_STATUS: u8 = 0xFF;

/// Stale bytes to discard before giving up on an output buffer that will not
/// drain.
const FLUSH_LIMIT: u32 = 32;

/// Poll attempts for an ordinary command's acknowledgement.
const RESPONSE_ATTEMPTS: u32 = 100_000;

/// Poll attempts for the keyboard's basic assurance test, which the protocol
/// allows hundreds of milliseconds for after a reset.
const RESET_ATTEMPTS: u32 = 2_000_000;

/// Why the keyboard could not be brought up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum I8042Error {
    /// Another driver already owns these ports.
    PortsUnavailable,
    /// Nothing answers on the status port.
    NotPresent,
    /// The controller failed its self-test.
    SelfTestFailed,
    /// The keyboard port failed its interface test.
    InterfaceTestFailed,
    /// No keyboard acknowledged the reset.
    KeyboardMissing,
    /// The keyboard refused a configuration command.
    CommandRefused,
}

impl fmt::Display for I8042Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortsUnavailable => f.write_str("its ports are already claimed"),
            Self::NotPresent => f.write_str("no controller answered on the status port"),
            Self::SelfTestFailed => f.write_str("the controller failed its self-test"),
            Self::InterfaceTestFailed => f.write_str("the keyboard port failed its interface test"),
            Self::KeyboardMissing => f.write_str("no keyboard acknowledged the reset"),
            Self::CommandRefused => f.write_str("the keyboard refused a configuration command"),
        }
    }
}

impl core::error::Error for I8042Error {}

/// The keyboard controller, claimed and configured for polled raw input.
#[derive(Debug)]
pub(crate) struct Controller {
    data: Port<u8>,
    control: Port<u8>,
}

impl Controller {
    /// Claim the controller and walk it into the polled configuration.
    ///
    /// The sequence is the standard bring-up with one quirk honoured: some
    /// controllers reset their configuration during the `0xAA` self-test, so
    /// the interrupts-off, translation-off byte is written again afterwards.
    /// A refused set 2 selection is tolerated — set 2 is the power-on
    /// default, and the self-test's readback is the arbiter — but a refused
    /// scanning enable is not, because a keyboard that will not scan is not
    /// an input device.
    pub(crate) fn new() -> Result<Self, I8042Error> {
        let data = port::claim(DATA).ok_or(I8042Error::PortsUnavailable)?;
        let control = port::claim(CONTROL).ok_or(I8042Error::PortsUnavailable)?;
        let mut controller = Self { data, control };

        if controller.control.read() == ABSENT_STATUS {
            return Err(I8042Error::NotPresent);
        }
        controller.controller_command(command::DISABLE_FIRST);
        controller.controller_command(command::DISABLE_SECOND);
        if !controller.flush() {
            return Err(I8042Error::NotPresent);
        }

        let configured = controller.read_config().ok_or(I8042Error::NotPresent)?
            & !(config::FIRST_IRQ | config::SECOND_IRQ | config::TRANSLATE);
        controller.write_config(configured);

        if controller.controller_query(command::SELF_TEST) != Some(reply::SELF_TEST_OK) {
            return Err(I8042Error::SelfTestFailed);
        }
        controller.write_config(configured);

        if controller.controller_query(command::TEST_FIRST) != Some(reply::INTERFACE_OK) {
            return Err(I8042Error::InterfaceTestFailed);
        }
        controller.controller_command(command::ENABLE_FIRST);

        if !controller.reset_keyboard() {
            return Err(I8042Error::KeyboardMissing);
        }
        let _ = controller.select_set_2();
        if !controller.device_command(device::ENABLE_SCANNING) {
            return Err(I8042Error::CommandRefused);
        }
        Ok(controller)
    }

    /// The next byte from the keyboard, or `None` when the buffer is empty.
    ///
    /// Bytes flagged as coming from the disabled second port are discarded —
    /// the port is off, but a defensive discard costs one branch. The
    /// discard is bounded like every wait here, because callers hold a lock
    /// with interrupts excluded: a controller that dies into a floating bus
    /// reads as endlessly full *and* from the second port, and an unbounded
    /// discard would wedge the machine silently.
    pub(crate) fn poll_byte(&mut self) -> Option<u8> {
        for _ in 0..FLUSH_LIMIT {
            let status = self.control.read();
            if status & status::OUTPUT_FULL == 0 {
                return None;
            }
            let byte = self.data.read();
            if status & status::FROM_SECOND_PORT == 0 {
                return Some(byte);
            }
        }
        None
    }

    /// The configuration byte, read live from the controller.
    #[cfg(feature = "selftest")]
    pub(crate) fn config(&mut self) -> Option<u8> {
        let _ = self.flush();
        self.read_config()
    }

    /// Send the echo command and report whether the echo came back.
    #[cfg(feature = "selftest")]
    pub(crate) fn echo_roundtrips(&mut self) -> bool {
        let _ = self.flush();
        self.write_data(device::ECHO) && self.read_reply(RESPONSE_ATTEMPTS) == Some(reply::ECHO)
    }

    /// The scancode set the keyboard reports itself speaking.
    #[cfg(feature = "selftest")]
    pub(crate) fn scancode_set(&mut self) -> Option<u8> {
        let _ = self.flush();
        if !self.device_command(device::SET_SCANCODE_SET) || !self.device_command(CURRENT_SET) {
            return None;
        }
        self.read_reply(RESPONSE_ATTEMPTS)
    }

    /// Reset the keyboard and wait for its basic assurance test.
    ///
    /// The protocol answers with an acknowledgement and then the test result,
    /// but emulators disagree about the acknowledgement, so either shape is
    /// accepted: what matters is that `0xAA` arrives.
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

    /// Ask for set 2. A refusal is survivable: the power-on default is set 2.
    fn select_set_2(&mut self) -> bool {
        self.device_command(device::SET_SCANCODE_SET) && self.device_command(SCANCODE_SET_2)
    }

    /// Send one byte to the keyboard and require its acknowledgement.
    fn device_command(&mut self, byte: u8) -> bool {
        self.write_data(byte) && self.read_reply(RESPONSE_ATTEMPTS) == Some(reply::ACK)
    }

    /// Send one command to the controller itself. Controller commands are
    /// not acknowledged; ones with a reply go through `controller_query`.
    fn controller_command(&mut self, command: u8) {
        if self.wait_writable() {
            self.control.write(command);
        }
    }

    /// Send one controller command and read its reply byte.
    fn controller_query(&mut self, command: u8) -> Option<u8> {
        self.controller_command(command);
        self.read_reply(RESPONSE_ATTEMPTS)
    }

    fn read_config(&mut self) -> Option<u8> {
        self.controller_query(command::READ_CONFIG)
    }

    fn write_config(&mut self, value: u8) {
        self.controller_command(command::WRITE_CONFIG);
        let _ = self.write_data(value);
    }

    /// Discard whatever is sitting in the output buffer.
    ///
    /// False if it never drains, which means nothing real is on the other
    /// end of these ports.
    fn flush(&mut self) -> bool {
        (0..FLUSH_LIMIT).any(|_| {
            if self.control.read() & status::OUTPUT_FULL == 0 {
                return true;
            }
            let _ = self.data.read();
            false
        })
    }

    fn write_data(&mut self, byte: u8) -> bool {
        if !self.wait_writable() {
            return false;
        }
        self.data.write(byte);
        true
    }

    fn read_reply(&mut self, attempts: u32) -> Option<u8> {
        if self.wait_readable(attempts) {
            Some(self.data.read())
        } else {
            None
        }
    }

    fn wait_readable(&mut self, attempts: u32) -> bool {
        (0..attempts).any(|_| self.control.read() & status::OUTPUT_FULL != 0)
    }

    fn wait_writable(&mut self) -> bool {
        (0..RESPONSE_ATTEMPTS).any(|_| self.control.read() & status::INPUT_FULL == 0)
    }
}
