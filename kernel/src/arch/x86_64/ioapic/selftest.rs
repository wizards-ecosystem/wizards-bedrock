use wost::mmio::MmioError;
use x86_apic::io::GsiRange;

use super::{
    IoApic, RegisterAccess, RoutingError, TakeError, TakeFailure, Wiring, entry, index,
    redirection, window,
};

impl<R: RegisterAccess> IoApic<R> {
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

    #[cfg(feature = "selftest")]
    pub(crate) fn entry_of(&mut self, gsi: u32) -> Result<(u32, u32), RoutingError> {
        let line = self.line_for(gsi)?;
        self.entry_words(line).map_err(RoutingError::Registers)
    }
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
