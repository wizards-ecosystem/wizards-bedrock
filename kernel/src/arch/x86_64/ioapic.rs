use core::fmt;
use wost::arch::current::paging::ReleasableDevice;
use wost::mmio::MmioError;
use x86_apic::io::{GsiRange, entries_from_version};

use crate::debug;

mod window {
    pub(super) const SELECT: usize = 0x00;
    pub(super) const DATA: usize = 0x10;
}

mod index {
    pub(super) const VERSION: u32 = 0x01;
    // Each redirection entry takes two consecutive indices from here.
    pub(super) const REDIRECTION: u32 = 0x10;
}

mod entry {
    pub(super) const MASKED: u32 = 1 << 16;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MaskError {
    Registers(MmioError),
    Refused { low: u32, high: u32 },
}

impl From<MmioError> for MaskError {
    fn from(error: MmioError) -> Self {
        Self::Registers(error)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TakeError {
    Registers(MmioError),
    ImplausibleVersion(u32),
    InvalidRange { base: u32, entries: u32 },
    MaskRefused { line: u32, low: u32, high: u32 },
}

impl fmt::Display for TakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registers(error) => write!(formatter, "{error}"),
            Self::ImplausibleVersion(version) => {
                write!(
                    formatter,
                    "version register {version:#010x} is not an i/o apic"
                )
            }
            Self::InvalidRange { base, entries } => {
                write!(
                    formatter,
                    "{entries} entries from gsi {base} are not representable"
                )
            }
            Self::MaskRefused { line, low, high } => write!(
                formatter,
                "line {line} retained {high:#x}:{low:#x} while being masked"
            ),
        }
    }
}

impl core::error::Error for TakeError {}

#[derive(Debug)]
pub(crate) struct TakeFailure {
    pub(crate) registers: ReleasableDevice,
    pub(crate) reason: TakeError,
}

#[derive(Debug)]
pub(crate) struct IoApic {
    registers: ReleasableDevice,
    range: GsiRange,
}

impl IoApic {
    // A window firmware pointed at another device reports an implausible version or entry count,
    // refused before any redirection write.
    pub(crate) fn take(
        mut registers: ReleasableDevice,
        gsi_base: u32,
    ) -> Result<Self, TakeFailure> {
        let version = match read(&mut registers, index::VERSION) {
            Ok(version) => version,
            Err(reason) => {
                return Err(TakeFailure {
                    registers,
                    reason: TakeError::Registers(reason),
                });
            }
        };
        let Some(entries) = entries_from_version(version) else {
            return Err(TakeFailure {
                registers,
                reason: TakeError::ImplausibleVersion(version),
            });
        };
        let Some(range) = GsiRange::new(gsi_base, entries) else {
            return Err(TakeFailure {
                registers,
                reason: TakeError::InvalidRange {
                    base: gsi_base,
                    entries,
                },
            });
        };

        let mut io_apic = Self { registers, range };
        for line in 0..entries {
            let refusal = match io_apic.mask_line_and_verify(line) {
                Ok(()) => continue,
                Err(MaskError::Registers(error)) => TakeError::Registers(error),
                Err(MaskError::Refused { low, high }) => TakeError::MaskRefused { line, low, high },
            };
            return Err(TakeFailure {
                registers: io_apic.registers,
                reason: refusal,
            });
        }
        debug!("ioapic: {entries} entries from gsi {gsi_base}, all masked");
        Ok(io_apic)
    }

    pub(crate) const fn range(&self) -> GsiRange {
        self.range
    }

    fn mask_line_and_verify(&mut self, line: u32) -> Result<(), MaskError> {
        let low = redirection(line);
        write(&mut self.registers, low, entry::MASKED)?;
        let (observed_low, observed_high) = self.entry_words(line)?;
        if observed_low != entry::MASKED {
            return Err(MaskError::Refused {
                low: observed_low,
                high: observed_high,
            });
        }
        write(&mut self.registers, low + 1, 0)?;
        let (low, high) = self.entry_words(line)?;
        if low != entry::MASKED || high != 0 {
            return Err(MaskError::Refused { low, high });
        }
        Ok(())
    }

    fn entry_words(&mut self, line: u32) -> Result<(u32, u32), MmioError> {
        let low = read(&mut self.registers, redirection(line))?;
        let high = read(&mut self.registers, redirection(line) + 1)?;
        Ok((low, high))
    }

    pub(crate) fn into_releasable(self) -> ReleasableDevice {
        self.registers
    }
}

const fn redirection(line: u32) -> u32 {
    index::REDIRECTION + line * 2
}

fn read(registers: &mut ReleasableDevice, index: u32) -> Result<u32, MmioError> {
    registers.write_u32(window::SELECT, index)?;
    registers.read_u32(window::DATA)
}

fn write(registers: &mut ReleasableDevice, index: u32, value: u32) -> Result<(), MmioError> {
    registers.write_u32(window::SELECT, index)?;
    registers.write_u32(window::DATA, value)
}
