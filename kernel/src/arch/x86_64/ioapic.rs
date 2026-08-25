//! The I/O APIC: where a device's interrupt line becomes a vector.
//!
//! A redirection entry says which vector a device's line becomes, which
//! processor takes it, and how the line is read electrically.
//!
//! Safe kernel code over a `wost::MmioRegion` mapped during table construction
//! (D-49). The registers are an index/data pair, so every access is two writes
//! deep and the driver lives behind one lock.

use core::fmt;
use wost::arch::current::apic;

use wost::mmio::{MmioError, MmioRegion};

use crate::debug;

/// Offsets into the register window.
mod window {
    /// Selects which indexed register [`DATA`] addresses.
    pub(super) const SELECT: usize = 0x00;
    /// Reads or writes the register [`SELECT`] names.
    pub(super) const DATA: usize = 0x10;
}

/// Indices of the registers behind that window.
mod index {
    /// Version, and the number of redirection entries this controller has.
    pub(super) const VERSION: u32 = 0x01;
    /// The first redirection entry. Each takes two consecutive indices.
    pub(super) const REDIRECTION: u32 = 0x10;
}

/// Bits of a redirection entry's low word.
mod entry {
    /// Interrupt is read as active-low rather than active-high.
    pub(super) const ACTIVE_LOW: u32 = 1 << 13;
    /// Interrupt is level-triggered rather than edge-triggered.
    pub(super) const LEVEL_TRIGGERED: u32 = 1 << 15;
    /// Nothing is delivered while this is set.
    pub(super) const MASKED: u32 = 1 << 16;
}

/// How firmware said a legacy interrupt is wired.
///
/// The MPS INTI encoding an interrupt source override carries: two bits of
/// polarity and two of trigger mode, each of which may say "conforms to the
/// bus", and the bus here is always ISA — active-high and edge-triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Wiring {
    active_low: bool,
    level_triggered: bool,
}

impl Wiring {
    /// Read the MPS INTI flags of an interrupt source override.
    pub(crate) const fn from_flags(flags: u16) -> Self {
        Self {
            // 00 conforms to the bus (ISA: active high), 01 active high,
            // 11 active low.
            active_low: flags & 0b11 == 0b11,
            // 00 conforms (ISA: edge), 01 edge, 11 level.
            level_triggered: (flags >> 2) & 0b11 == 0b11,
        }
    }

    /// How an ISA device is wired when firmware says nothing.
    pub(crate) const fn isa_default() -> Self {
        Self {
            active_low: false,
            level_triggered: false,
        }
    }

    /// The entry bits this wiring contributes.
    const fn bits(self) -> u32 {
        let mut bits = 0;
        if self.active_low {
            bits |= entry::ACTIVE_LOW;
        }
        if self.level_triggered {
            bits |= entry::LEVEL_TRIGGERED;
        }
        bits
    }
}

/// Why a redirection entry could not be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoutingError {
    /// The interrupt is not one this controller routes.
    ///
    /// Separate from an access failure: the arithmetic that finds a line
    /// subtracts the base, and below it that wraps.
    NotOurs(u32),
    /// The registers refused the access.
    Registers(MmioError),
}

impl fmt::Display for RoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOurs(gsi) => write!(f, "gsi {gsi} is not this controller's"),
            Self::Registers(error) => write!(f, "{error}"),
        }
    }
}

impl From<MmioError> for RoutingError {
    fn from(error: MmioError) -> Self {
        Self::Registers(error)
    }
}

/// An I/O APIC.
#[derive(Debug)]
pub(crate) struct IoApic {
    registers: MmioRegion,
    gsi_base: u32,
    entries: u32,
}

impl IoApic {
    /// Take over a controller whose registers are at `registers`.
    ///
    /// Masks every line it owns before returning: firmware leaves them in
    /// whatever state its own drivers wanted, and a kernel that unmasks one
    /// line while another is still live from before takes an interrupt it has
    /// no handler for.
    pub(crate) fn take(mut registers: MmioRegion, gsi_base: u32) -> Result<Self, MmioError> {
        let version = read(&mut registers, index::VERSION)?;
        // Bits 23:16 hold the highest entry index, so the count is one more.
        let entries = ((version >> 16) & 0xFF) + 1;

        let mut io_apic = Self {
            registers,
            gsi_base,
            entries,
        };
        for line in 0..entries {
            io_apic.mask_line(line)?;
        }
        debug!("ioapic: {entries} entries from gsi {gsi_base}, all masked");
        Ok(io_apic)
    }

    /// The lowest global system interrupt this controller routes.
    ///
    /// Reported so a check can say whether "below the base" is even a place —
    /// on a machine whose base is zero it is not, and a check that required a
    /// refusal there would be requiring one for an address that does not exist
    /// (D-121).
    #[cfg(feature = "selftest")]
    pub(crate) const fn base(&self) -> u32 {
        self.gsi_base
    }

    /// The global system interrupts this controller routes.
    pub(crate) const fn handles(&self, gsi: u32) -> bool {
        gsi >= self.gsi_base && gsi - self.gsi_base < self.entries
    }

    /// The line `gsi` occupies on this controller.
    ///
    /// Every entry below goes through this rather than subtracting for itself.
    fn line_for(&self, gsi: u32) -> Result<u32, RoutingError> {
        self.handles(gsi)
            .then(|| gsi - self.gsi_base)
            .ok_or(RoutingError::NotOurs(gsi))
    }

    /// Route `gsi` to `vector` on this processor, and unmask it.
    ///
    /// The high word is written before the low one on purpose: the low word
    /// holds the mask bit, so writing it last means the line becomes live only
    /// once its destination is already correct.
    pub(crate) fn route(
        &mut self,
        gsi: u32,
        vector: u8,
        wiring: Wiring,
    ) -> Result<(), RoutingError> {
        let line = self.line_for(gsi)?;
        let destination = apic::id() << 24;
        write(&mut self.registers, redirection(line) + 1, destination)?;
        write(
            &mut self.registers,
            redirection(line),
            u32::from(vector) | wiring.bits(),
        )?;
        Ok(())
    }

    /// Whether `gsi` is currently masked.
    ///
    /// Read by the boot-time check, so it exists only where that does.
    #[cfg(feature = "selftest")]
    pub(crate) fn is_masked(&mut self, gsi: u32) -> Result<bool, RoutingError> {
        let line = self.line_for(gsi)?;
        Ok(read(&mut self.registers, redirection(line))? & entry::MASKED != 0)
    }

    /// The vector `gsi` is routed to, read back from the controller.
    ///
    /// Read by the boot-time check, so it exists only where that does.
    #[cfg(feature = "selftest")]
    pub(crate) fn vector_of(&mut self, gsi: u32) -> Result<u8, RoutingError> {
        let line = self.line_for(gsi)?;
        Ok(read(&mut self.registers, redirection(line))? as u8)
    }

    /// Stop `line` delivering anything.
    fn mask_line(&mut self, line: u32) -> Result<(), MmioError> {
        write(&mut self.registers, redirection(line), entry::MASKED)?;
        write(&mut self.registers, redirection(line) + 1, 0)
    }
}

/// The first of the two register indices holding `line`'s redirection entry.
const fn redirection(line: u32) -> u32 {
    index::REDIRECTION + line * 2
}

/// Read the indexed register `index`.
fn read(registers: &mut MmioRegion, index: u32) -> Result<u32, MmioError> {
    registers.write_u32(window::SELECT, index)?;
    registers.read_u32(window::DATA)
}

/// Write the indexed register `index`.
fn write(registers: &mut MmioRegion, index: u32, value: u32) -> Result<(), MmioError> {
    registers.write_u32(window::SELECT, index)?;
    registers.write_u32(window::DATA, value)
}
