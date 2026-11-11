//! QEMU's `isa-debug-exit` device: writing to it makes QEMU exit with `(value << 1) | 1`.
//! Present only under `-device isa-debug-exit`, so this is `selftest`-only and never ships.

use wost::arch::current::port::{self, Port};

/// Written when every check passed. QEMU then exits with 33.
const PASSED: u32 = 0x10;

/// Written when any check failed. QEMU then exits with 35.
const FAILED: u32 = 0x11;

/// How a self-test run ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Verdict {
    Passed,
    Failed,
}

impl Verdict {
    pub(crate) const fn from_passed(passed: bool) -> Self {
        if passed { Self::Passed } else { Self::Failed }
    }
}

/// Tell the harness the verdict; under QEMU with the device present this never returns.
/// Anywhere else the write goes nowhere, so a self-test kernel on hardware reaches its idle loop.
pub(crate) fn report(verdict: Verdict) {
    if let Some(mut device) = port::debug_exit() {
        let code = match verdict {
            Verdict::Passed => PASSED,
            Verdict::Failed => FAILED,
        };
        Port::write(&mut device, code);
    }
}
