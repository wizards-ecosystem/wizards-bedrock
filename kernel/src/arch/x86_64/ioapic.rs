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
use wost::arch::current::paging::ReleasableDevice;
use wost::mmio::{MmioError, MmioRegion};
use x86_apic::io::{GsiRange, entries_from_version};
use x86_apic::vector::InterruptVector;

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
    /// Separate from an access failure, and checked before the subtraction
    /// rather than after: the arithmetic that finds a line subtracts the base,
    /// which below the base would wrap in release and panic in debug. The
    /// `handles` guard is what stops it reaching either.
    NotOurs(u32),
    /// The registers refused the access.
    Registers(MmioError),
    /// This processor's APIC ID does not fit the redirection entry.
    ///
    /// The entry's destination field is eight bits. This kernel requires
    /// x2APIC, so `apic::id()` is the full thirty-two-bit register rather than
    /// the xAPIC 31:24 form — and shifting one of 256 or more left by
    /// twenty-four silently routed the line to `id & 0xFF` (D-124).
    UnroutableProcessor(u32),
    /// A line did not retain the exact masked entry written during rollback.
    MaskRefused { low: u32, high: u32 },
    /// A line did not retain the complete route written for it.
    RouteRefused {
        expected_low: u32,
        expected_high: u32,
        low: u32,
        high: u32,
    },
}

impl fmt::Display for RoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOurs(gsi) => write!(f, "gsi {gsi} is not this controller's"),
            Self::Registers(error) => write!(f, "{error}"),
            Self::UnroutableProcessor(id) => {
                write!(f, "apic id {id} does not fit a redirection entry")
            }
            Self::MaskRefused { low, high } => write!(
                f,
                "redirection entry refused its masked state ({high:#x}:{low:#x})"
            ),
            Self::RouteRefused {
                expected_low,
                expected_high,
                low,
                high,
            } => write!(
                f,
                "redirection entry retained {high:#x}:{low:#x}, not {expected_high:#x}:{expected_low:#x}"
            ),
        }
    }
}

impl From<MmioError> for RoutingError {
    fn from(error: MmioError) -> Self {
        Self::Registers(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaskError {
    Registers(MmioError),
    Refused { low: u32, high: u32 },
}

impl From<MmioError> for MaskError {
    fn from(error: MmioError) -> Self {
        Self::Registers(error)
    }
}

impl From<MaskError> for RoutingError {
    fn from(error: MaskError) -> Self {
        match error {
            MaskError::Registers(error) => Self::Registers(error),
            MaskError::Refused { low, high } => Self::MaskRefused { low, high },
        }
    }
}

/// Why an I/O APIC could not become a checked, masked controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TakeError {
    /// The register window refused an access.
    Registers(MmioError),
    /// The version register is not one an I/O APIC could have reported.
    ImplausibleVersion(u32),
    /// The ACPI base and hardware entry count leave the GSI namespace.
    InvalidRange { base: u32, entries: u32 },
    /// A line did not retain the exact inert entry written during takeover.
    MaskRefused { line: u32, low: u32, high: u32 },
}

impl fmt::Display for TakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registers(error) => write!(f, "{error}"),
            Self::ImplausibleVersion(version) => {
                write!(f, "version register {version:#010x} is not an i/o apic")
            }
            Self::InvalidRange { base, entries } => {
                write!(f, "{entries} entries from gsi {base} are not representable")
            }
            Self::MaskRefused { line, low, high } => write!(
                f,
                "line {line} retained {high:#x}:{low:#x} while being masked"
            ),
        }
    }
}

/// A failed controller probe together with the mapping it did not consume.
#[derive(Debug)]
pub(crate) struct TakeFailure<R> {
    pub(crate) registers: R,
    pub(crate) reason: TakeError,
}

/// The register operations shared by permanent and releasable mappings.
pub(crate) trait RegisterAccess {
    fn read_u32(&self, offset: usize) -> Result<u32, MmioError>;
    fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), MmioError>;
}

impl RegisterAccess for MmioRegion {
    fn read_u32(&self, offset: usize) -> Result<u32, MmioError> {
        Self::read_u32(self, offset)
    }

    fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), MmioError> {
        Self::write_u32(self, offset, value)
    }
}

impl RegisterAccess for ReleasableDevice {
    fn read_u32(&self, offset: usize) -> Result<u32, MmioError> {
        Self::read_u32(self, offset)
    }

    fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), MmioError> {
        Self::write_u32(self, offset, value)
    }
}

/// An I/O APIC.
#[derive(Debug)]
pub(crate) struct IoApic<R = MmioRegion> {
    registers: R,
    range: GsiRange,
}

impl<R: RegisterAccess> IoApic<R> {
    /// Take over a controller whose registers are at `registers`.
    ///
    /// Masks every line it owns before returning: firmware leaves them in
    /// whatever state its own drivers wanted, and a kernel that unmasks one
    /// line while another is still live from before takes an interrupt it has
    /// no handler for.
    pub(crate) fn take(mut registers: R, gsi_base: u32) -> Result<Self, TakeFailure<R>> {
        let version = match read(&mut registers, index::VERSION) {
            Ok(version) => version,
            Err(error) => {
                return Err(TakeFailure {
                    registers,
                    reason: TakeError::Registers(error),
                });
            }
        };
        // A window firmware pointed at another device reports an implausible
        // version or entry count; refuse it before any redirection write.
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

    /// The lowest global system interrupt this controller routes.
    ///
    /// Reported so a check can say whether "below the base" is even a place —
    /// on a machine whose base is zero it is not, and a check that required a
    /// refusal there would be requiring one for an address that does not exist
    /// (D-121).
    #[cfg(feature = "selftest")]
    pub(crate) const fn base(&self) -> u32 {
        self.range.first()
    }

    /// The global system interrupts this controller routes.
    pub(crate) const fn handles(&self, gsi: u32) -> bool {
        self.range.contains(gsi)
    }

    /// The complete checked GSI range this controller routes.
    pub(crate) const fn range(&self) -> GsiRange {
        self.range
    }

    /// The line `gsi` occupies on this controller.
    ///
    /// Every entry below goes through this rather than subtracting for itself.
    fn line_for(&self, gsi: u32) -> Result<u32, RoutingError> {
        self.handles(gsi)
            .then(|| gsi - self.range.first())
            .ok_or(RoutingError::NotOurs(gsi))
    }

    /// Route `gsi` to `vector` on this processor, and unmask it.
    ///
    /// The line remains masked until the destination has been read back, so a
    /// refused high-word write cannot briefly deliver to a stale processor.
    pub(crate) fn route(
        &mut self,
        gsi: u32,
        vector: InterruptVector,
        wiring: Wiring,
    ) -> Result<(), RoutingError> {
        let line = self.line_for(gsi)?;
        let id = apic::id();
        let destination = u8::try_from(id)
            .map(|id| u32::from(id) << 24)
            .map_err(|_| RoutingError::UnroutableProcessor(id))?;
        self.route_line(line, vector, wiring, destination)
    }

    fn route_line(
        &mut self,
        line: u32,
        vector: InterruptVector,
        wiring: Wiring,
        destination: u32,
    ) -> Result<(), RoutingError> {
        let expected_low = u32::from(vector.get()) | wiring.bits();
        self.mask_line_and_verify(line)
            .map_err(RoutingError::from)?;
        write(&mut self.registers, redirection(line) + 1, destination)?;
        match self.entry_words(line) {
            Ok((low, high)) if low == entry::MASKED && high == destination => {}
            Ok((low, high)) => {
                return self.refuse_route(line, expected_low, destination, low, high);
            }
            Err(error) => {
                self.mask_line_and_verify(line)
                    .map_err(RoutingError::from)?;
                return Err(RoutingError::Registers(error));
            }
        }
        write(&mut self.registers, redirection(line), expected_low)?;

        match self.entry_words(line) {
            Ok((low, high)) if low == expected_low && high == destination => Ok(()),
            Ok((low, high)) => self.refuse_route(line, expected_low, destination, low, high),
            Err(error) => {
                self.mask_line_and_verify(line)
                    .map_err(RoutingError::from)?;
                Err(RoutingError::Registers(error))
            }
        }
    }

    fn refuse_route(
        &mut self,
        line: u32,
        expected_low: u32,
        expected_high: u32,
        low: u32,
        high: u32,
    ) -> Result<(), RoutingError> {
        self.mask_line_and_verify(line)
            .map_err(RoutingError::from)?;
        Err(RoutingError::RouteRefused {
            expected_low,
            expected_high,
            low,
            high,
        })
    }

    /// Mask `gsi` and prove the whole redirection entry is inert.
    pub(crate) fn mask_and_verify(&mut self, gsi: u32) -> Result<(), RoutingError> {
        let line = self.line_for(gsi)?;
        self.mask_line_and_verify(line).map_err(RoutingError::from)
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

    #[cfg(feature = "selftest")]
    pub(crate) fn entry_of(&mut self, gsi: u32) -> Result<(u32, u32), RoutingError> {
        let line = self.line_for(gsi)?;
        self.entry_words(line).map_err(RoutingError::Registers)
    }
}

impl IoApic<ReleasableDevice> {
    /// Make the selected controller permanent.
    pub(crate) fn retain(self) -> IoApic {
        IoApic {
            registers: self.registers.retain(),
            range: self.range,
        }
    }

    /// Return the mapping behind an unselected controller.
    pub(crate) fn into_releasable(self) -> ReleasableDevice {
        self.registers
    }
}

/// The first of the two register indices holding `line`'s redirection entry.
const fn redirection(line: u32) -> u32 {
    index::REDIRECTION + line * 2
}

/// Read the indexed register `index`.
fn read<R: RegisterAccess>(registers: &mut R, index: u32) -> Result<u32, MmioError> {
    <R as RegisterAccess>::write_u32(registers, window::SELECT, index)?;
    <R as RegisterAccess>::read_u32(registers, window::DATA)
}

/// Write the indexed register `index`.
fn write<R: RegisterAccess>(registers: &mut R, index: u32, value: u32) -> Result<(), MmioError> {
    <R as RegisterAccess>::write_u32(registers, window::SELECT, index)?;
    <R as RegisterAccess>::write_u32(registers, window::DATA, value)
}

#[cfg(feature = "selftest")]
#[derive(Debug)]
struct FakeRegisters {
    selected: u32,
    version: u32,
    low: u32,
    high: u32,
    ignore_once: Option<(u32, u32)>,
    unmasked_high: Option<u32>,
    high_write_while_unmasked: Option<u32>,
}

#[cfg(feature = "selftest")]
impl FakeRegisters {
    const fn stale(ignore_once: Option<(u32, u32)>) -> Self {
        // Version 0x11 with a maximum redirection entry of zero: one line.
        Self::reporting(0x0000_0011, ignore_once)
    }

    const fn reporting(version: u32, ignore_once: Option<(u32, u32)>) -> Self {
        Self {
            selected: 0,
            version,
            low: 0x31,
            high: 0xAB00_0000,
            ignore_once,
            unmasked_high: None,
            high_write_while_unmasked: None,
        }
    }
}

#[cfg(feature = "selftest")]
impl RegisterAccess for FakeRegisters {
    fn read_u32(&self, offset: usize) -> Result<u32, MmioError> {
        let value = if self.selected == index::VERSION && offset == window::DATA {
            self.version
        } else if offset != window::DATA {
            0
        } else if self.selected == redirection(0) {
            self.low
        } else if self.selected == redirection(0) + 1 {
            self.high
        } else {
            0
        };
        Ok(value)
    }

    fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), MmioError> {
        if offset == window::SELECT {
            self.selected = value;
        } else if offset == window::DATA {
            if self.ignore_once == Some((self.selected, value)) {
                self.ignore_once = None;
            } else if self.selected == redirection(0) {
                self.low = value;
                if value & entry::MASKED == 0 {
                    self.unmasked_high = Some(self.high);
                }
            } else if self.selected == redirection(0) + 1 {
                if self.low & entry::MASKED == 0 {
                    self.high_write_while_unmasked = Some(value);
                }
                self.high = value;
            }
        }
        Ok(())
    }
}

#[cfg(feature = "selftest")]
pub(crate) fn typed_route_is_encoded_for_test() -> bool {
    let Ok(mut controller) = IoApic::take(FakeRegisters::stale(None), 0) else {
        return false;
    };
    let destination = 0x7D00_0000;
    controller
        .route_line(
            0,
            super::vector::KEYBOARD,
            Wiring::isa_default(),
            destination,
        )
        .is_ok()
        && controller.registers.low == u32::from(super::vector::KEYBOARD.get())
        && controller.registers.high == destination
        && controller.registers.high_write_while_unmasked.is_none()
}

#[cfg(feature = "selftest")]
pub(crate) fn ignored_writes_are_refused_for_test() -> bool {
    let takeover_refused_without_destination_write = match IoApic::take(
        FakeRegisters::stale(Some((redirection(0), entry::MASKED))),
        0,
    ) {
        Err(TakeFailure {
            registers,
            reason: TakeError::MaskRefused { line: 0, .. },
        }) => {
            registers.low == 0x31
                && registers.high == 0xAB00_0000
                && registers.high_write_while_unmasked.is_none()
        }
        _ => false,
    };

    let takeover_refused_without_destination_clear =
        match IoApic::take(FakeRegisters::stale(Some((redirection(0) + 1, 0))), 0) {
            Err(TakeFailure {
                registers,
                reason: TakeError::MaskRefused { line: 0, .. },
            }) => {
                registers.low == entry::MASKED
                    && registers.high == 0xAB00_0000
                    && registers.high_write_while_unmasked.is_none()
            }
            _ => false,
        };

    let Ok(mut controller) = IoApic::take(FakeRegisters::stale(None), 0) else {
        return false;
    };
    let takeover_is_inert = controller.registers.low == entry::MASKED
        && controller.registers.high == 0
        && controller.registers.high_write_while_unmasked.is_none();

    controller.registers.ignore_once = Some((redirection(0), 0xA041));
    let route_refused = matches!(
        controller.route_line(
            0,
            super::vector::TIMER,
            Wiring {
                active_low: true,
                level_triggered: true,
            },
            0x7B00_0000,
        ),
        Err(RoutingError::RouteRefused {
            expected_low: 0xA041,
            expected_high: 0x7B00_0000,
            ..
        })
    ) && controller.registers.low == entry::MASKED
        && controller.registers.high == 0
        && controller.registers.high_write_while_unmasked.is_none();

    controller.registers.ignore_once = Some((redirection(0) + 1, 0x7C00_0000));
    controller.registers.unmasked_high = None;
    let destination_refused_before_unmask = matches!(
        controller.route_line(
            0,
            super::vector::KEYBOARD,
            Wiring {
                active_low: true,
                level_triggered: true,
            },
            0x7C00_0000,
        ),
        Err(RoutingError::RouteRefused {
            expected_low: 0xA042,
            expected_high: 0x7C00_0000,
            ..
        })
    ) && controller.registers.unmasked_high.is_none()
        && controller.registers.low == entry::MASKED
        && controller.registers.high == 0
        && controller.registers.high_write_while_unmasked.is_none();
    takeover_refused_without_destination_write
        && takeover_refused_without_destination_clear
        && takeover_is_inert
        && route_refused
        && destination_refused_before_unmask
}

/// A window that does not report an I/O APIC version is never taken over.
///
/// Firmware controls where the register window points; a version register that
/// is not an I/O APIC's means it points at something else, and no redirection
/// is written to it (D-350).
#[cfg(feature = "selftest")]
pub(crate) fn an_implausible_version_is_refused_for_test() -> bool {
    // A zeroed window, a text-console MMIO byte, and a count no part can hold.
    let refused = [0x0000_0000, 0x0000_0041, 0x00FF_0011]
        .into_iter()
        .all(|version| {
            matches!(
                IoApic::take(FakeRegisters::reporting(version, None), 0),
                Err(TakeFailure {
                    reason: TakeError::ImplausibleVersion(reported),
                    ..
                }) if reported == version
            )
        });
    // The plausible version the other checks rely on is still accepted.
    let accepted = IoApic::take(FakeRegisters::reporting(0x0000_0011, None), 0)
        .is_ok_and(|controller| controller.range == GsiRange::new(0, 1).expect("one entry"));
    refused && accepted
}
