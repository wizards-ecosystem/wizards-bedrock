use wost::mmio::MmioError;

use super::{
    IoApic, RegisterAccess, RoutingError, TakeError, TakeFailure, Wiring, entry, index,
    redirection, window,
};

impl<R: RegisterAccess> IoApic<R> {}

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
    /// Version 0x11 with a maximum redirection entry of zero: one line.
    const fn stale(ignore_once: Option<(u32, u32)>) -> Self {
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
